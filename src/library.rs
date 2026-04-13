//! Library backend wrapping aoide's embedded environment.
//!
//! Replaces `db.rs` — owns an aoide [`Environment`] with a connection-pooled
//! `SQLite` database and exposes blocking methods that mirror the old API surface.
//! Internally, async aoide calls are driven by a dedicated tokio runtime.

use std::{
    num::{NonZeroU32, NonZeroU64},
    path::Path,
};

use anyhow::{Context as _, anyhow};
use aoide::{
    CollectionUid,
    api::{Pagination, track::search::Params as SearchParams},
    backend_embedded::{self, Environment},
    collection::{Collection, MediaSourceConfig},
    media::content::ContentPathConfig,
    storage_sqlite::connection as conn,
};

use crate::bridge::{self, TrackField, TrackView};

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

        Ok(Self {
            rt,
            env,
            collection_uid,
        })
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        self.env.decommission();
    }
}

// ── Read operations ──────────────────────────────────────────────

impl Library {
    /// Load all tracks in the collection, flattened into [`TrackView`]s.
    pub fn all_tracks(&self) -> Result<Vec<TrackView>> {
        self.rt.block_on(async {
            let entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search all tracks")?;

            Ok(entities.iter().map(bridge::flatten).collect())
        })
    }

    /// Return entity UIDs of every track as strings.
    pub fn track_ids(&self) -> Result<Vec<String>> {
        self.rt.block_on(async {
            let entities = backend_embedded::track::search(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                SearchParams::default(),
                Pagination::default(),
            )
            .await
            .context("search track ids")?;

            Ok(entities.iter().map(|e| e.hdr.uid.to_string()).collect())
        })
    }

    /// Check whether a media source with this content path exists.
    pub fn has_content_path(&self, path: &str) -> Result<bool> {
        let tracks = self.all_tracks()?;
        Ok(tracks
            .iter()
            .any(|t| t.files.iter().any(|f| f.file_path == path)))
    }
}

// ── Write operations ─────────────────────────────────────────────

impl Library {
    /// Apply a field edit to the track identified by `entity_uid_str`.
    ///
    /// Loads the entity, applies the edit via [`bridge::apply_edit`],
    /// validates, and replaces it in the database.
    pub fn update_track(&self, entity_uid_str: &str, field: TrackField) -> Result<()> {
        let entity_uid = entity_uid_str
            .parse()
            .map_err(|_| anyhow!("invalid entity uid: {entity_uid_str}"))?;

        self.rt.block_on(async {
            let mut entity =
                backend_embedded::track::load_one(self.env.db_gatekeeper(), entity_uid)
                    .await
                    .context("load track for update")?;

            bridge::apply_edit(&mut entity, field);

            let track = entity.body.track.clone();
            let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                .map_err(|e| anyhow!("validation failed: {e:?}"))?;

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOnly,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: false,
                decode_gigtags: false,
            };

            backend_embedded::track::replace_many_by_media_source_content_path(
                self.env.db_gatekeeper(),
                self.collection_uid.clone(),
                params,
                std::iter::once(validated),
            )
            .await
            .context("replace track after edit")?;

            Ok(())
        })
    }

    /// Delete a track by its entity UID.
    pub fn delete_track(&self, entity_uid_str: &str) -> Result<()> {
        // Load the track to find its content path, then purge by content path.
        let entity_uid = entity_uid_str
            .parse()
            .map_err(|_| anyhow!("invalid entity uid: {entity_uid_str}"))?;

        self.rt.block_on(async {
            let entity = backend_embedded::track::load_one(self.env.db_gatekeeper(), entity_uid)
                .await
                .context("load track for deletion")?;

            let content_path = entity
                .body
                .track
                .media_source
                .content
                .link
                .path
                .as_str()
                .to_owned();

            self.delete_track_by_path(&content_path)
        })
    }

    /// Delete a track by its media source content path.
    pub fn delete_track_by_path(&self, content_path: &str) -> Result<()> {
        use aoide::api::filter::StringPredicate;
        use std::borrow::Cow;

        let collection_uid = self.collection_uid.clone();
        let content_path = content_path.to_owned();

        self.rt.block_on(async {
            self.env
                .db_gatekeeper()
                .spawn_blocking_write_task(move |mut connection| {
                    let _ = aoide::usecases_sqlite::track::purge::purge_by_media_source_content_path_predicates(
                        &mut connection,
                        &collection_uid,
                        vec![StringPredicate::Equals(Cow::Owned(content_path))],
                    );
                })
                .await
                .context("purge track by path")?;

            Ok(())
        })
    }

    /// Import audio files by reading metadata with aoide-media-file and
    /// storing them via `replace_many_by_media_source_content_path`.
    ///
    /// Skips files whose content path is already in the collection.
    /// Returns `(imported_count, warnings)`.
    pub fn import_files(&self, paths: &[std::path::PathBuf]) -> Result<(u32, Vec<String>)> {
        use aoide::media::content::{ContentLink, ContentPath, ContentRevision};
        use aoide::media_file::{
            io::import::{ImportTrack, ImportTrackConfig, import_into_track},
            util::guess_mime_from_file_path,
        };
        use aoide::util::clock::OffsetDateTimeMs;
        use std::io::BufReader;

        let config = ImportTrackConfig::default();
        let mut imported = 0u32;
        let mut warnings = Vec::new();

        for path in paths {
            let path_str = path.to_string_lossy().to_string();

            // Skip if already in collection.
            if self.has_content_path(&path_str).unwrap_or(false) {
                continue;
            }

            // Canonicalize and open.
            let canonical = match path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    warnings.push(format!("{path_str}: {e}"));
                    continue;
                }
            };
            let file = match std::fs::File::open(&canonical) {
                Ok(f) => f,
                Err(e) => {
                    warnings.push(format!("{path_str}: {e}"));
                    continue;
                }
            };

            // Content link (absolute path as content path, file mod-time as revision).
            let content_rev = ContentRevision::try_from_file(&file).ok().flatten();
            let content_path = ContentPath::new(std::borrow::Cow::Owned(path_str.clone()));
            let content_link = ContentLink {
                path: content_path,
                rev: content_rev,
            };

            // MIME type.
            let content_type = match guess_mime_from_file_path(&canonical) {
                Ok(m) => m,
                Err(e) => {
                    warnings.push(format!("{path_str}: unsupported format ({e})"));
                    continue;
                }
            };

            // Build a new Track skeleton, then import metadata from the file.
            let import_track = ImportTrack::NewTrack {
                collected_at: OffsetDateTimeMs::now_local(),
            };
            let mut track = import_track.with_content(content_link, content_type);

            let mut reader: Box<dyn aoide::media_file::io::import::Reader> =
                Box::new(BufReader::new(file));
            if let Err(e) = import_into_track(&mut reader, &config, &mut track) {
                warnings.push(format!("{path_str}: metadata import failed ({e})"));
                continue;
            }

            // Validate and store.
            let (validated, _invalidities) = aoide::usecases::track::validate_input(track)
                .map_err(|e| anyhow!("validation failed for {path_str}: {e:?}"))?;

            let params = aoide::usecases::track::replace::Params {
                mode: aoide::repo::track::ReplaceMode::UpdateOrCreate,
                resolve_path_from_url: false,
                preserve_collected_at: true,
                update_last_synchronized_rev: true,
                decode_gigtags: false,
            };

            self.rt.block_on(async {
                backend_embedded::track::replace_many_by_media_source_content_path(
                    self.env.db_gatekeeper(),
                    self.collection_uid.clone(),
                    params,
                    std::iter::once(validated),
                )
                .await
                .context("store imported track")
            })?;

            imported += 1;
        }

        Ok((imported, warnings))
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
