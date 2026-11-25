// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Library backend wrapping aoide's embedded environment.
//!
//! Owns an aoide [`Environment`] with a connection-pooled `SQLite` database and
//! exposes blocking methods. Internally, async aoide calls are driven by a
//! dedicated tokio runtime.

use std::{
    collections::{HashMap, HashSet},
    num::{NonZeroU32, NonZeroU64},
    path::Path,
};

use anyhow::{Context, anyhow};
use aoide::{
    CollectionUid,
    api::{
        Pagination,
        filter::StringPredicate,
        tag::search::{FacetsFilter, Filter as TagFilter},
        track::{
            replace::Summary as ReplaceSummary,
            search::{Filter, Params as SearchParams, PhraseFieldFilter},
        },
    },
    backend_embedded::{self, Environment},
    collection::{Collection, MediaSourceConfig},
    media::content::ContentPathConfig,
    repo::track::ReplaceMode,
    storage_sqlite::connection as conn,
    track::Entity,
    usecases::track::{ValidatedInput, replace::Params as ReplaceParams, validate_input},
};

use crate::bridge::{self, TrackField, TrackView};

struct ImportRequest {
    source_path: std::path::PathBuf,
    track_id: String,
    stored_path: Option<String>,
    metadata: Option<TrackView>,
}

// ── Public types ─────────────────────────────────────────────────

/// Result alias for Library operations.
pub type Result<T> = anyhow::Result<T>;

/// Thin wrapper around aoide's embedded backend.
///
/// Thread-safe (`Send + Sync`). Methods are blocking — call from a
/// background thread (via `task::spawn_blocking` or similar).
pub struct Library {
    rt: tokio::runtime::Runtime,
    env: Environment,
    collection_uid: CollectionUid,
}

// ── Lifecycle ────────────────────────────────────────────────────

impl Library {
    /// Open (or create) the library database at `db_dir/aoide.sqlite`.
    ///
    /// Commissions an aoide [`Environment`], applies pending schema
    /// migrations, and ensures a default collection exists.
    pub fn open(db_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(db_dir)
            .with_context(|| format!("create data dir: {}", db_dir.display()))?;

        let db_path = db_dir.join("aoide.sqlite");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create tokio runtime")?;

        let db_config = backend_embedded::storage::DatabaseConfig {
            connection: conn::Config {
                storage: conn::Storage::File { path: db_path },
                pool: conn::pool::Config {
                    max_size: NonZeroU32::new(4).expect("nonzero"),
                    gatekeeper: conn::pool::gatekeeper::Config {
                        acquire_read_timeout_millis: NonZeroU64::new(10_000).expect("nonzero"),
                        acquire_write_timeout_millis: NonZeroU64::new(30_000).expect("nonzero"),
                    },
                },
            },
            migrate_schema: Some(
                backend_embedded::storage::DatabaseSchemaMigrationMode::ApplyPending,
            ),
        };

        let env = Environment::commission(&db_config).context("commission aoide environment")?;

        let collection_uid = rt.block_on(ensure_default_collection(env.db_gatekeeper()))?;

        let lib = Self {
            rt,
            env,
            collection_uid,
        };

        lib.backfill_track_ids()?;

        Ok(lib)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.env.decommission();
    }
}

// ── Read operations ──────────────────────────────────────────────

impl Library {
    /// Load every track entity matching `filter` (`None` = all).
    fn search_entities(
        &self,
        filter: Option<Filter>,
        context: &'static str,
    ) -> Result<Vec<Entity>> {
        let params = SearchParams {
            filter,
            ..SearchParams::default()
        };
        self.rt.block_on(async {
            backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                Pagination::default(),
            )
            .await
            .context(context)
        })
    }

    fn all_entities(&self, context: &'static str) -> Result<Vec<Entity>> {
        self.search_entities(None, context)
    }

    /// Load every file variant of the logical track identified by `track_id`.
    ///
    /// Every track carries its ID as a facet (see `backfill_track_ids`), so the lookup is a
    /// tag filter in the database rather than a scan of the collection.
    fn entities_of_track(&self, track_id: &str, context: &'static str) -> Result<Vec<Entity>> {
        let filter = Filter::Tag(TagFilter {
            facets: Some(FacetsFilter::AnyOf(vec![bridge::track_id_facet().into()])),
            label: Some(StringPredicate::Equals(track_id.to_owned().into())),
            ..TagFilter::default()
        });
        self.search_entities(Some(filter), context)
    }

    /// Load all tracks in the collection, flattened into [`TrackView`]s.
    pub fn all_tracks(&self) -> Result<Vec<TrackView>> {
        let entities = self.all_entities("search all track entities")?;
        Ok(group_track_views(entities.iter().map(bridge::flatten)))
    }

    /// Search tracks using aoide's phrase filter.
    ///
    /// An empty query returns all tracks. Non-empty queries are tokenised
    /// by whitespace and matched case-insensitively against title, artist,
    /// album, and other string fields via aoide's `PhraseFieldFilter`.
    pub fn search_tracks(&self, query: &str) -> Result<Vec<TrackView>> {
        let entities =
            self.search_entities(build_phrase_filter(query), "search tracks by query")?;
        Ok(group_track_views(entities.iter().map(bridge::flatten)))
    }

    /// Return entity UIDs of every track as strings.
    pub fn track_ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();

        for entity in self.all_entities("search all track entities")? {
            let track_id = bridge::track_id(&entity);
            if seen.insert(track_id.clone()) {
                ids.push(track_id);
            }
        }

        Ok(ids)
    }
}

// ── Write operations ─────────────────────────────────────────────

impl Library {
    /// Apply a field edit to every file variant of the logical track identified by `track_id`.
    ///
    /// Loads all entities for that logical track, applies the edit, validates,
    /// and replaces them in the database.
    pub fn update_track(&self, track_id: &str, field: &TrackField) -> Result<()> {
        let mut entities = self.entities_of_track(track_id, "search tracks for update")?;
        if entities.is_empty() {
            return Err(anyhow!("track not found: {track_id}"));
        }

        for entity in &mut entities {
            bridge::apply_field(&mut entity.body.track, field.clone());
        }
        self.update_entities(&entities, "replace tracks after edit")?;

        self.export_track_metadata_best_effort(track_id);
        Ok(())
    }

    /// Delete every file variant of a logical track by its stable track ID.
    pub fn delete_track(&self, track_id: &str) -> Result<()> {
        let content_paths: Vec<_> = self
            .entities_of_track(track_id, "search tracks for deletion")?
            .iter()
            .map(bridge::content_path)
            .collect();

        if content_paths.is_empty() {
            return Err(anyhow!("track not found: {track_id}"));
        }

        for content_path in content_paths {
            self.delete_track_by_path(&content_path)?;
        }

        Ok(())
    }

    /// Delete a track by its media source content path.
    pub fn delete_track_by_path(&self, content_path: &str) -> Result<()> {
        use std::borrow::Cow;

        let collection_uid = self.collection_uid.clone();
        let content_path = content_path.to_owned();

        self.rt.block_on(async {
            let _summary = self
                .env
                .db_gatekeeper()
                .spawn_blocking_write_task(move |mut connection| {
                    aoide::usecases_sqlite::track::purge::purge_by_media_source_content_path_predicates(
                        &mut connection,
                        &collection_uid,
                        vec![StringPredicate::Equals(Cow::Owned(content_path))],
                    )
                    .map_err(|e| anyhow!("purge track by path failed: {e}"))
                })
                .await
                .context("purge track by path task")??;

            Ok(())
        })
    }

    /// Import audio files by reading metadata with aoide-media-file and
    /// storing them via `replace_many_by_media_source_content_path`.
    ///
    /// Skips files whose content path is already in the collection.
    /// Returns `(imported_count, warnings)`.
    /// Import audio files with an optional progress callback `(current, total)`.
    pub fn import_files_with_progress(
        &self,
        paths: &[std::path::PathBuf],
        on_progress: Option<&(dyn Fn(u32, u32) + Sync)>,
    ) -> Result<(u32, Vec<String>)> {
        let requests = paths
            .iter()
            .map(|path| ImportRequest {
                source_path: path.clone(),
                track_id: uuid::Uuid::new_v4().to_string(),
                stored_path: None,
                metadata: None,
            })
            .collect();

        self.import_requests(requests, on_progress)
    }

    /// Import one audio file as another file variant of an existing logical track.
    pub fn import_file_variant(
        &self,
        source_path: &Path,
        track_id: &str,
        stored_path: Option<String>,
        metadata: &TrackView,
    ) -> Result<(u32, Vec<String>)> {
        self.import_requests(
            vec![ImportRequest {
                source_path: source_path.to_path_buf(),
                track_id: track_id.to_owned(),
                stored_path,
                metadata: Some(metadata.clone()),
            }],
            None,
        )
    }

    fn import_requests(
        &self,
        requests: Vec<ImportRequest>,
        on_progress: Option<&(dyn Fn(u32, u32) + Sync)>,
    ) -> Result<(u32, Vec<String>)> {
        use aoide::media_file::io::import::ImportTrackConfig;
        use rayon::iter::{IntoParallelIterator, ParallelIterator};

        let config = ImportTrackConfig::default();

        // Load all existing content paths once to avoid O(n) full-table scans.
        let existing_paths = self.existing_content_paths()?;

        let total = u32::try_from(requests.len()).unwrap_or(u32::MAX);
        let processed = std::sync::atomic::AtomicU32::new(0);

        // Read metadata in parallel across all available cores.
        let results: Vec<_> = requests
            .into_par_iter()
            .map(|request| {
                let outcome = process_single_import(&request, &existing_paths, &config);
                let done = processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if let Some(cb) = on_progress {
                    cb(done, total);
                }
                outcome
            })
            .collect();

        let mut warnings = Vec::new();
        let mut validated_batch = Vec::new();
        for result in results {
            match result {
                ImportOutcome::Validated(track) => validated_batch.push(*track),
                ImportOutcome::Skipped => {}
                ImportOutcome::Warning(msg) => warnings.push(msg),
            }
        }

        if validated_batch.is_empty() {
            return Ok((0, warnings));
        }

        // One batch call for all validated tracks.
        let summary = self.replace_tracks(
            replace_params(ReplaceMode::UpdateOrCreate),
            validated_batch,
            "batch store imported tracks",
        )?;

        append_import_summary_warnings(&mut warnings, &summary);
        let imported = import_summary_count(&summary);

        Ok((imported, warnings))
    }

    /// Overwrite all metadata fields of an existing track from a [`TrackView`].
    ///
    /// Loads the entity once, applies every field, validates, and writes back in one operation.
    pub fn overwrite_track_fields(&self, track_id: &str, view: &TrackView) -> Result<()> {
        let mut entities = self.entities_of_track(track_id, "search tracks for overwrite")?;
        if entities.is_empty() {
            return Err(anyhow!("track not found: {track_id}"));
        }

        for entity in &mut entities {
            bridge::apply_view(&mut entity.body.track, view);
        }
        self.update_entities(&entities, "replace tracks after overwrite")?;

        self.export_track_metadata_best_effort(track_id);
        Ok(())
    }

    /// Reassign logical track ID for every file variant in a grouped track.
    pub fn reassign_track_id(&self, current_track_id: &str, new_track_id: &str) -> Result<()> {
        if current_track_id == new_track_id {
            return Ok(());
        }

        let mut entities =
            self.entities_of_track(current_track_id, "search tracks for ID reassignment")?;
        for entity in &mut entities {
            bridge::set_track_id(&mut entity.body.track, new_track_id);
        }
        self.update_entities(&entities, "reassign logical track IDs")
    }

    /// Write updated metadata back into the audio file tags, logging instead of failing: the
    /// database is the source of truth, so an unwritable file must not undo the edit.
    fn export_track_metadata_best_effort(&self, track_id: &str) {
        match self.export_track_metadata(track_id) {
            Ok((_, warnings)) => {
                for warning in warnings {
                    eprintln!("metadata export: {warning}");
                }
            }
            Err(e) => eprintln!("metadata export failed for {track_id}: {e}"),
        }
    }

    /// Write DB metadata back into the audio file tags for every file variant
    /// of the logical track identified by `track_id`.
    ///
    /// Returns `(exported_count, warnings)`.
    pub fn export_track_metadata(&self, track_id: &str) -> Result<(u32, Vec<String>)> {
        use aoide::media_file::io::export::{ExportTrackConfig, export_track_to_file_path};

        let entities = self.entities_of_track(track_id, "search tracks for metadata export")?;
        if entities.is_empty() {
            return Err(anyhow!("track not found: {track_id}"));
        }

        let config = ExportTrackConfig::default();
        let mut exported = 0u32;
        let mut warnings = Vec::new();

        for entity in entities {
            let mut track = entity.body.track.clone();
            let file_path = track.media_source.content.link.path.as_str().to_owned();
            let path = std::path::PathBuf::from(&file_path);

            if !path.exists() {
                warnings.push(format!("{file_path}: file not found, skipped"));
                continue;
            }

            match export_track_to_file_path(&path, &config, &mut track, None) {
                Ok(()) => exported += 1,
                Err(e) => warnings.push(format!("{file_path}: export failed ({e})")),
            }
        }

        Ok((exported, warnings))
    }
}

// ── Internals ────────────────────────────────────────────────────

const DEFAULT_COLLECTION_TITLE: &str = "Pino Library";
const DEFAULT_COLLECTION_KIND: &str = "pino";

/// Find the default Pino collection, or create one if none exists.
async fn ensure_default_collection(
    gatekeeper: &aoide::storage_sqlite::connection::pool::gatekeeper::Gatekeeper,
) -> Result<CollectionUid> {
    use aoide::repo::collection::KindFilter;
    use std::borrow::Cow;

    // Try to find existing collection by kind.
    let existing = backend_embedded::collection::load_all(
        gatekeeper,
        Some(KindFilter::Equal(Cow::Owned(
            DEFAULT_COLLECTION_KIND.to_owned(),
        ))),
        None,
        aoide::api::collection::LoadScope::Entity,
        None,
    )
    .await
    .context("load collections")?;

    if let Some(first) = existing.first() {
        return Ok(first.entity.hdr.uid.clone());
    }

    // None found — create one with URI-based content paths (no VFS root needed).
    let collection = Collection {
        title: DEFAULT_COLLECTION_TITLE.to_owned(),
        kind: Some(DEFAULT_COLLECTION_KIND.to_owned()),
        notes: None,
        color: None,
        media_source_config: MediaSourceConfig {
            content_path: ContentPathConfig::Uri,
        },
    };

    let entity = backend_embedded::collection::create(gatekeeper, collection)
        .await
        .context("create default collection")?;

    Ok(entity.hdr.uid.clone())
}

impl Library {
    fn existing_content_paths(&self) -> Result<HashSet<String>> {
        Ok(self
            .all_tracks()?
            .into_iter()
            .flat_map(|track| track.files.into_iter().map(|file| file.file_path))
            .collect())
    }

    fn backfill_track_ids(&self) -> Result<()> {
        let mut entities = self.all_entities("search tracks for ID backfill")?;
        entities.retain(|entity| bridge::track_id_from_track(&entity.body.track).is_none());

        for entity in &mut entities {
            let entity_uid = entity.hdr.uid.to_string();
            bridge::set_track_id(&mut entity.body.track, &entity_uid);
        }
        self.update_entities(&entities, "backfill logical track IDs")
    }

    /// Validate and write back already-existing entities after an in-memory change.
    fn update_entities(&self, entities: &[Entity], context: &'static str) -> Result<()> {
        if entities.is_empty() {
            return Ok(());
        }
        let validated = entities
            .iter()
            .map(|entity| validate_input(entity.body.track.clone()).0)
            .collect();
        self.replace_tracks(replace_params(ReplaceMode::UpdateOnly), validated, context)?;
        Ok(())
    }

    fn replace_tracks(
        &self,
        params: ReplaceParams,
        validated: Vec<ValidatedInput>,
        context: &'static str,
    ) -> Result<ReplaceSummary> {
        self.rt.block_on(async {
            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated,
            )
            .await
            .context(context)
        })
    }
}

/// Replace parameters shared by every write: imports create or update and stamp the
/// synchronized revision, edits of existing tracks only update.
fn replace_params(mode: ReplaceMode) -> ReplaceParams {
    let update_last_synchronized_rev = matches!(mode, ReplaceMode::UpdateOrCreate);
    ReplaceParams {
        mode,
        resolve_path_from_url: false,
        preserve_collected_at: true,
        update_last_synchronized_rev,
        decode_gigtags: true,
    }
}

// ── Parallel import helpers ──────────────────────────────────────

enum ImportOutcome {
    Validated(Box<ValidatedInput>),
    Skipped,
    Warning(String),
}

fn process_single_import(
    request: &ImportRequest,
    existing_paths: &HashSet<String>,
    config: &aoide::media_file::io::import::ImportTrackConfig,
) -> ImportOutcome {
    use aoide::media::content::{ContentLink, ContentPath, ContentRevision};
    use aoide::media_file::{
        io::import::{ImportTrack, import_into_track},
        util::guess_mime_from_file_path,
    };
    use aoide::util::clock::OffsetDateTimeMs;
    use std::io::BufReader;

    // Canonicalize and open.
    let canonical = match request.source_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return ImportOutcome::Warning(format!("{}: {e}", request.source_path.display()));
        }
    };
    let stored_path = request
        .stored_path
        .clone()
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    if existing_paths.contains(&stored_path) {
        return ImportOutcome::Skipped;
    }

    let file = match std::fs::File::open(&canonical) {
        Ok(f) => f,
        Err(e) => return ImportOutcome::Warning(format!("{}: {e}", canonical.display())),
    };

    // Content link (absolute path as content path, file mod-time as revision).
    let content_rev = ContentRevision::try_from_file(&file).ok().flatten();
    let content_path = ContentPath::new(std::borrow::Cow::Owned(stored_path));
    let content_link = ContentLink {
        path: content_path,
        rev: content_rev,
    };

    // MIME type.
    let content_type = match guess_mime_from_file_path(&canonical) {
        Ok(m) => m,
        Err(e) => {
            return ImportOutcome::Warning(format!(
                "{}: unsupported format ({e})",
                canonical.display()
            ));
        }
    };

    // Build a new Track skeleton, then import metadata from the file.
    let import_track = ImportTrack::NewTrack {
        collected_at: OffsetDateTimeMs::now_local(),
    };
    let mut track = import_track.with_content(content_link, content_type);

    let mut reader: Box<dyn aoide::media_file::io::import::Reader> = Box::new(BufReader::new(file));
    if let Err(e) = import_into_track(&mut reader, config, &mut track) {
        return ImportOutcome::Warning(format!(
            "{}: metadata import failed ({e})",
            canonical.display()
        ));
    }

    bridge::set_track_id(&mut track, &request.track_id);
    if let Some(metadata) = request.metadata.as_ref() {
        bridge::apply_view(&mut track, metadata);
    }

    let (validated, _invalidities) = validate_input(track);
    ImportOutcome::Validated(Box::new(validated))
}

fn group_track_views(flattened: impl IntoIterator<Item = TrackView>) -> Vec<TrackView> {
    let mut grouped: HashMap<String, Vec<TrackView>> = HashMap::new();
    for track in flattened {
        grouped.entry(track.id.clone()).or_default().push(track);
    }

    let mut grouped: Vec<_> = grouped.into_values().map(merge_track_group).collect();
    grouped.sort_by(|left, right| {
        left.artist
            .cmp(&right.artist)
            .then(left.album.cmp(&right.album))
            .then(left.title.cmp(&right.title))
            .then(left.id.cmp(&right.id))
    });
    grouped
}

fn merge_track_group(mut group: Vec<TrackView>) -> TrackView {
    let best_index = group
        .iter()
        .enumerate()
        .max_by_key(|(_, track)| track_richness(track))
        .map_or(0, |(index, _)| index);

    let mut merged = group.swap_remove(best_index);

    for mut track in group {
        if merged.added_at.is_empty()
            || (!track.added_at.is_empty() && track.added_at < merged.added_at)
        {
            merged.added_at = std::mem::take(&mut track.added_at);
        }
        merged.files.extend(track.files);
    }

    let mut seen_paths = HashSet::new();
    merged
        .files
        .retain(|file| seen_paths.insert(file.file_path.clone()));
    merged
}

fn track_richness(track: &TrackView) -> usize {
    [
        !track.title.is_empty(),
        !track.artist.is_empty(),
        !track.album.is_empty(),
        !track.genre.is_empty(),
        !track.composer.is_empty(),
        !track.label.is_empty(),
        !track.remixer.is_empty(),
        !track.key.is_empty(),
        !track.comment.is_empty(),
        !track.isrc.is_empty(),
        !track.lyricist.is_empty(),
        !track.mix_name.is_empty(),
        !track.release_date.is_empty(),
        !track.artwork_path.is_empty(),
        track.duration_secs > 0,
        track.tempo > 0.0,
        track.year > 0,
        track.track_number > 0,
        track.disc_number > 0,
        track.rating > 0,
        track.color > 0,
    ]
    .into_iter()
    .filter(|present| *present)
    .count()
}

fn append_import_summary_warnings(warnings: &mut Vec<String>, summary: &ReplaceSummary) {
    warnings.extend(
        summary
            .failed
            .iter()
            .map(|path| format!("{path}: import failed")),
    );
    warnings.extend(
        summary
            .not_imported
            .iter()
            .map(|path| format!("{path}: not imported")),
    );
    warnings.extend(summary.not_created.iter().map(|track| {
        format!(
            "{}: not created",
            track.media_source.content.link.path.as_str()
        )
    }));
    warnings.extend(summary.not_updated.iter().map(|track| {
        format!(
            "{}: not updated",
            track.media_source.content.link.path.as_str()
        )
    }));
}

fn import_summary_count(summary: &ReplaceSummary) -> u32 {
    let imported = summary.created.len() + summary.updated.len() + summary.unchanged.len();
    u32::try_from(imported).unwrap_or(u32::MAX)
}

/// Build an aoide phrase filter from a search query string.
///
/// Tokens are split on whitespace. An empty query returns `None` (= match all).
/// With `fields` left empty, aoide matches against all indexed string fields
/// (title, artist, album, genre, comment, etc.).
fn build_phrase_filter(query: &str) -> Option<Filter> {
    let terms: Vec<String> = query.split_whitespace().map(String::from).collect();
    if terms.is_empty() {
        return None;
    }
    Some(Filter::Phrase(PhraseFieldFilter {
        fields: vec![], // empty = search all string fields
        terms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::TrackFileView;

    fn variant(id: &str, title: &str, path: &str, added_at: &str) -> TrackView {
        TrackView {
            id: id.to_owned(),
            title: title.to_owned(),
            added_at: added_at.to_owned(),
            files: vec![TrackFileView {
                file_path: path.to_owned(),
                ..TrackFileView::default()
            }],
            ..TrackView::default()
        }
    }

    #[test]
    fn variants_of_one_track_are_merged() {
        let grouped = group_track_views([
            variant("a", "", "/a.flac", "2024-02-01T00:00:00+00:00"),
            variant("a", "Song", "/a.mp3", "2024-01-01T00:00:00+00:00"),
            variant("a", "Song", "/a.mp3", "2024-03-01T00:00:00+00:00"),
        ]);

        assert_eq!(grouped.len(), 1);
        let merged = &grouped[0];
        // Metadata comes from the richest variant, the date from the oldest one.
        assert_eq!(merged.title, "Song");
        assert_eq!(merged.added_at, "2024-01-01T00:00:00+00:00");
        // Duplicate paths collapse into one file.
        let mut paths: Vec<_> = merged.files.iter().map(|f| f.file_path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["/a.flac", "/a.mp3"]);
    }

    #[test]
    fn groups_are_sorted_by_artist_album_title() {
        let mut b = variant("1", "B", "/b.mp3", "");
        b.artist = "Zed".to_owned();
        let mut a = variant("2", "A", "/a.mp3", "");
        a.artist = "Abe".to_owned();
        let mut a2 = variant("3", "C", "/c.mp3", "");
        a2.artist = "Abe".to_owned();

        let ids: Vec<_> = group_track_views([b, a2, a])
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, ["2", "3", "1"]);
    }

    #[test]
    fn phrase_filter_splits_on_whitespace() {
        assert!(build_phrase_filter("  ").is_none());
        let Some(Filter::Phrase(filter)) = build_phrase_filter(" daft  punk ") else {
            panic!("expected a phrase filter");
        };
        assert_eq!(filter.terms, ["daft", "punk"]);
        assert_eq!(filter.fields, []);
    }
}

/// End-to-end checks against a real (temporary) aoide database.
#[cfg(test)]
pub(crate) mod db_tests {
    use super::*;

    /// A temporary directory holding a library and two silent WAV files.
    struct Fixture {
        dir: std::path::PathBuf,
        lib: Library,
        ids: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("pino-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let files: Vec<_> = ["a.wav", "b.wav"].iter().map(|n| dir.join(n)).collect();
            for file in &files {
                std::fs::write(file, silent_wav()).unwrap();
            }

            let lib = Library::open(&dir.join("db")).unwrap();
            let (imported, warnings) = lib.import_files_with_progress(&files, None).unwrap();
            assert_eq!((imported, warnings), (2, Vec::new()));
            let ids = lib.track_ids().unwrap();
            Self { dir, lib, ids }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// One second of 8 kHz mono 16-bit silence.
    pub(crate) fn silent_wav() -> Vec<u8> {
        let data_len = 16_000u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8_000u32.to_le_bytes()); // sample rate
        wav.extend_from_slice(&16_000u32.to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(wav.len() + data_len as usize, 0);
        wav
    }

    #[test]
    fn edit_changes_only_the_addressed_track() {
        let fx = Fixture::new();
        let (edited, other) = (&fx.ids[0], &fx.ids[1]);

        fx.lib
            .update_track(edited, &TrackField::Title("Edited".to_owned()))
            .unwrap();

        let tracks = fx.lib.all_tracks().unwrap();
        let title = |id: &String| tracks.iter().find(|t| &t.id == id).unwrap().title.clone();
        assert_eq!(title(edited), "Edited");
        assert_ne!(title(other), "Edited");
    }

    #[test]
    fn delete_removes_only_the_addressed_track() {
        let fx = Fixture::new();

        fx.lib.delete_track(&fx.ids[0]).unwrap();

        assert_eq!(fx.lib.track_ids().unwrap(), [fx.ids[1].clone()]);
        assert!(fx.lib.delete_track("no-such-track").is_err());
    }
}
