//! Library backend wrapping aoide's embedded environment.
//!
//! Replaces `db.rs` — owns an aoide [`Environment`] with a connection-pooled
//! `SQLite` database and exposes blocking methods that mirror the old API surface.
//! Internally, async aoide calls are driven by a dedicated tokio runtime.

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
        track::search::{Filter, Params as SearchParams, PhraseFieldFilter},
    },
    backend_embedded::{self, Environment},
    collection::{Collection, MediaSourceConfig},
    media::content::ContentPathConfig,
    storage_sqlite::connection as conn,
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
    fn all_entities(&self) -> Result<Vec<aoide::track::Entity>> {
        self.rt.block_on(async {
            backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search all track entities")
        })
    }

    /// Load all tracks in the collection, flattened into [`TrackView`]s.
    pub fn all_tracks(&self) -> Result<Vec<TrackView>> {
        let flattened = self
            .all_entities()?
            .into_iter()
            .map(|entity| bridge::flatten(&entity))
            .collect();
        Ok(group_track_views(flattened))
    }

    /// Search tracks using aoide's phrase filter.
    ///
    /// An empty query returns all tracks. Non-empty queries are tokenised
    /// by whitespace and matched case-insensitively against title, artist,
    /// album, and other string fields via aoide's `PhraseFieldFilter`.
    pub fn search_tracks(&self, query: &str) -> Result<Vec<TrackView>> {
        let filter = build_phrase_filter(query);
        let params = SearchParams {
            filter,
            ..SearchParams::default()
        };
        let entities = self.rt.block_on(async {
            backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                Pagination::default(),
            )
            .await
            .context("search tracks by query")
        })?;
        let flattened = entities
            .into_iter()
            .map(|entity| bridge::flatten(&entity))
            .collect();
        Ok(group_track_views(flattened))
    }

    /// Return entity UIDs of every track as strings.
    pub fn track_ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();

        for entity in self.all_entities()? {
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
        let track_id = track_id.to_owned();
        self.rt.block_on(async {
            let mut entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search tracks for update")?
            .into_iter()
            .filter(|entity| bridge::track_id(entity) == track_id)
            .collect::<Vec<_>>();

            if entities.is_empty() {
                return Err(anyhow!("track not found: {track_id}"));
            }

            let mut validated_batch = Vec::with_capacity(entities.len());
            for entity in &mut entities {
                bridge::apply_edit(entity, field.clone());

                let track = entity.body.track.clone();
                let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                    .map_err(|e| anyhow!("validation failed: {e:?}"))?;
                validated_batch.push(validated);
            }

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOnly,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: false,
                decode_gigtags: true,
            };

            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated_batch.into_iter(),
            )
            .await
            .context("replace tracks after edit")?;

            Ok(())
        })?;

        // Best-effort: write updated metadata back into audio file tags.
        if let Err(e) = self.export_track_metadata(&track_id) {
            eprintln!("metadata export after edit failed for {track_id}: {e}");
        }

        Ok(())
    }

    /// Delete every file variant of a logical track by its stable track ID.
    pub fn delete_track(&self, track_id: &str) -> Result<()> {
        let content_paths: Vec<_> = self
            .all_entities()?
            .into_iter()
            .filter(|entity| bridge::track_id(entity) == track_id)
            .map(|entity| bridge::content_path(&entity))
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
        use aoide::api::filter::StringPredicate;
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

        let params = aoide::usecases::track::replace::Params {
            mode: aoide::repo::track::ReplaceMode::UpdateOrCreate,
            resolve_path_from_url: false,
            preserve_collected_at: true,
            update_last_synchronized_rev: true,
            decode_gigtags: true,
        };

        // One batch call for all validated tracks.
        let summary = self.rt.block_on(async {
            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated_batch.into_iter(),
            )
            .await
            .context("batch store imported tracks")
        })?;

        append_import_summary_warnings(&mut warnings, &summary);
        let imported = import_summary_count(&summary);

        Ok((imported, warnings))
    }

    /// Overwrite all metadata fields of an existing track from a [`TrackView`].
    ///
    /// Loads the entity once, applies every field, validates, and writes back in one operation.
    pub fn overwrite_track_fields(
        &self,
        track_id: &str,
        view: &crate::bridge::TrackView,
    ) -> Result<()> {
        let track_id = track_id.to_owned();
        let view = view.clone();
        self.rt.block_on(async {
            let mut entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search tracks for overwrite")?
            .into_iter()
            .filter(|entity| bridge::track_id(entity) == track_id)
            .collect::<Vec<_>>();

            if entities.is_empty() {
                return Err(anyhow!("track not found: {track_id}"));
            }

            let mut validated_batch = Vec::with_capacity(entities.len());
            for entity in &mut entities {
                crate::bridge::apply_all_fields(entity, &view);

                let track = entity.body.track.clone();
                let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                    .map_err(|e| anyhow!("validation failed: {e:?}"))?;
                validated_batch.push(validated);
            }

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOnly,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: false,
                decode_gigtags: true,
            };

            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated_batch.into_iter(),
            )
            .await
            .context("replace tracks after overwrite")?;

            Ok(())
        })?;

        // Best-effort: write updated metadata back into audio file tags.
        if let Err(e) = self.export_track_metadata(&track_id) {
            eprintln!("metadata export after overwrite failed for {track_id}: {e}");
        }

        Ok(())
    }

    /// Reassign logical track ID for every file variant in a grouped track.
    pub fn reassign_track_id(&self, current_track_id: &str, new_track_id: &str) -> Result<()> {
        if current_track_id == new_track_id {
            return Ok(());
        }

        let current_track_id = current_track_id.to_owned();
        let new_track_id = new_track_id.to_owned();
        self.rt.block_on(async {
            let mut entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search tracks for ID reassignment")?
            .into_iter()
            .filter(|entity| bridge::track_id(entity) == current_track_id)
            .collect::<Vec<_>>();

            if entities.is_empty() {
                return Ok(());
            }

            let mut validated_batch = Vec::with_capacity(entities.len());
            for entity in &mut entities {
                bridge::set_track_id(&mut entity.body.track, &new_track_id);

                let track = entity.body.track.clone();
                let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                    .map_err(|e| anyhow!("validation failed during ID reassignment: {e:?}"))?;
                validated_batch.push(validated);
            }

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOnly,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: false,
                decode_gigtags: true,
            };

            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated_batch.into_iter(),
            )
            .await
            .context("reassign logical track IDs")?;

            Ok(())
        })
    }

    /// Write DB metadata back into the audio file tags for every file variant
    /// of the logical track identified by `track_id`.
    ///
    /// Returns `(exported_count, warnings)`.
    pub fn export_track_metadata(&self, track_id: &str) -> Result<(u32, Vec<String>)> {
        use aoide::media_file::io::export::{ExportTrackConfig, export_track_to_file_path};

        let entities = self
            .all_entities()?
            .into_iter()
            .filter(|entity| bridge::track_id(entity) == track_id)
            .collect::<Vec<_>>();

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
        self.rt.block_on(async {
            let entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search tracks for ID backfill")?;

            let mut validated_batch = Vec::new();
            for mut entity in entities {
                if bridge::track_id_from_track(&entity.body.track).is_some() {
                    continue;
                }

                let entity_uid = entity.hdr.uid.to_string();
                bridge::set_track_id(&mut entity.body.track, &entity_uid);

                let track = entity.body.track.clone();
                let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                    .map_err(|e| anyhow!("validation failed during ID backfill: {e:?}"))?;
                validated_batch.push(validated);
            }

            if validated_batch.is_empty() {
                return Ok(());
            }

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOnly,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: false,
                decode_gigtags: true,
            };

            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                validated_batch.into_iter(),
            )
            .await
            .context("backfill logical track IDs")?;

            Ok(())
        })
    }
}

// ── Parallel import helpers ──────────────────────────────────────

enum ImportOutcome {
    Validated(Box<aoide::usecases::track::ValidatedInput>),
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

    // Validate; skip this file on failure rather than aborting the whole batch.
    match aoide::usecases::track::validate_input(track) {
        Ok((validated, _)) => ImportOutcome::Validated(Box::new(validated)),
        Err(e) => ImportOutcome::Warning(format!(
            "{}: validation failed ({e:?})",
            canonical.display()
        )),
    }
}

fn group_track_views(flattened: Vec<TrackView>) -> Vec<TrackView> {
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
        track.tempo > 0,
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

fn append_import_summary_warnings(
    warnings: &mut Vec<String>,
    summary: &aoide::api::track::replace::Summary,
) {
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

fn import_summary_count(summary: &aoide::api::track::replace::Summary) -> u32 {
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
