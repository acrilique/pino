//! Bridge between aoide track entities and Pino's flat view model.
//!
//! UI never sees aoide types directly. [`TrackView`] provides the same
//! flat shape as the old `Track + TrackFile` structs so the UI layer
//! needs minimal changes during migration.

use std::path::Path;

use aoide::{
    media::artwork::Artwork,
    media::content::ContentMetadata,
    music::key::{KeyCode, KeySignature},
    music::tempo::TempoBpm,
    prelude::*,
    tag::{FacetId, FacetedTags, PlainTag, Score, label::Label},
    track::{
        self, Actor, Actors, Entity, Titles,
        actor::{self, Role},
        tag::{FACET_ID_COMMENT, FACET_ID_GENRE, FACET_ID_ISRC},
        title,
    },
    util::{
        clock::{DateOrDateTime, YyyyMmDdDate},
        color::Color,
    },
};
use chrono::{DateTime, Utc};

const PINO_TRACK_ID_FACET: &str = "pino.track_id";
const PINO_RATING_FACET: &str = "pino.rating";

// ── View model ───────────────────────────────────────────────────

/// Flat track representation for the UI layer.
///
/// Same shape as the old `Track + TrackFile` so UI components need
/// minimal changes during migration.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct TrackView {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub composer: String,
    pub label: String,
    pub remixer: String,
    pub key: String,
    pub comment: String,
    pub isrc: String,
    pub lyricist: String,
    pub mix_name: String,
    pub release_date: String,
    pub duration_secs: u16,
    pub tempo: u32,
    pub year: u16,
    pub track_number: u32,
    pub disc_number: u16,
    pub rating: u8,
    pub color: u8,
    pub artwork_path: String,
    pub added_at: String,
    pub files: Vec<TrackFileView>,
}

/// Flat file metadata for one audio file.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackFileView {
    pub format: String,
    pub file_path: String,
    pub file_size: u32,
    pub sample_rate: u32,
    pub bitrate: u32,
}

/// Editable field + new value, used for UI → aoide mutations.
#[derive(Clone)]
pub enum TrackField {
    Title(String),
    Artist(String),
    Album(String),
    Genre(String),
    Composer(String),
    Label(String),
    Remixer(String),
    Key(String),
    Comment(String),
    Isrc(String),
    Lyricist(String),
    MixName(String),
    ReleaseDate(String),
    Tempo(u32),
    Year(u16),
    TrackNumber(u32),
    DiscNumber(u16),
    Rating(u8),
    Color(u8),
}

// ── Flatten: Entity → TrackView ──────────────────────────────────

/// Flatten an aoide track entity into a [`TrackView`].
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn flatten(entity: &Entity) -> TrackView {
    let track = &entity.body.track;
    let source = &track.media_source;
    let ContentMetadata::Audio(audio) = &source.content.metadata;
    let file_path = source.content.link.path.as_str();

    let added_at = DateTime::<Utc>::from_timestamp_millis(
        track.media_source.collected_at.unix_timestamp_millis(),
    )
    .map_or_else(String::new, |dt| dt.to_rfc3339());

    TrackView {
        id: track_id(entity),
        title: track.track_title().unwrap_or_default().to_owned(),
        artist: track.track_artist().unwrap_or_default().to_owned(),
        album: track.album_title().unwrap_or_default().to_owned(),
        genre: facet_label(track, FACET_ID_GENRE),
        composer: track.track_composer().unwrap_or_default().to_owned(),
        label: track.publisher.clone().unwrap_or_default(),
        remixer: summary_actor_name(track, Role::Remixer),
        key: track
            .metrics
            .key_signature
            .map(|ks| ks.code().as_canonical_str().to_owned())
            .unwrap_or_default(),
        comment: facet_label(track, FACET_ID_COMMENT),
        isrc: facet_label(track, FACET_ID_ISRC),
        lyricist: summary_actor_name(track, Role::Lyricist),
        mix_name: Titles::kind_title(track.titles.as_ref(), title::Kind::Sub)
            .map_or_else(String::new, |t| t.name.clone()),
        release_date: track
            .released_at
            .map_or_else(String::new, |d| d.to_string()),
        duration_secs: audio.duration.map_or(0, |d| (d.value() / 1000.0) as u16),
        tempo: track
            .metrics
            .tempo_bpm
            .map_or(0, |t| (t.value() * 100.0) as u32),
        year: track.recorded_at.map_or(0, |d| {
            let y = d.year();
            if y > 0 { y as u16 } else { 0 }
        }),
        track_number: track.indexes.track.number.map_or(0, u32::from),
        disc_number: track.indexes.disc.number.unwrap_or(0),
        rating: rating(track),
        color: match track.color {
            Some(Color::Index(i)) => i.clamp(0, 8) as u8,
            _ => 0,
        },
        artwork_path: match &source.artwork {
            Some(Artwork::Linked(linked)) => linked.uri.clone(),
            _ => String::new(),
        },
        added_at,
        files: vec![TrackFileView {
            format: format_from_path(file_path),
            file_size: std::fs::metadata(file_path)
                .map_or(0, |m| m.len().try_into().unwrap_or(u32::MAX)),
            file_path: file_path.to_owned(),
            sample_rate: audio.sample_rate.map_or(0, |sr| sr.value() as u32),
            bitrate: audio.bitrate.map_or(0, |br| br.value() as u32),
        }],
    }
}

// ── Apply edit: TrackField → Entity mutation ─────────────────────

/// Apply a UI edit to the track inside an entity.
#[allow(clippy::cast_possible_truncation)]
pub fn apply_edit(entity: &mut Entity, field: TrackField) {
    apply_track_field(&mut entity.body.track, field);
}

/// Apply every user-editable metadata field from a [`TrackView`] to a track.
pub fn apply_view(track: &mut track::Track, view: &TrackView) {
    apply_track_field(track, TrackField::Title(view.title.clone()));
    apply_track_field(track, TrackField::Artist(view.artist.clone()));
    apply_track_field(track, TrackField::Album(view.album.clone()));
    apply_track_field(track, TrackField::Genre(view.genre.clone()));
    apply_track_field(track, TrackField::Composer(view.composer.clone()));
    apply_track_field(track, TrackField::Label(view.label.clone()));
    apply_track_field(track, TrackField::Remixer(view.remixer.clone()));
    apply_track_field(track, TrackField::Key(view.key.clone()));
    apply_track_field(track, TrackField::Comment(view.comment.clone()));
    apply_track_field(track, TrackField::Isrc(view.isrc.clone()));
    apply_track_field(track, TrackField::Lyricist(view.lyricist.clone()));
    apply_track_field(track, TrackField::MixName(view.mix_name.clone()));
    apply_track_field(track, TrackField::ReleaseDate(view.release_date.clone()));
    apply_track_field(track, TrackField::Tempo(view.tempo));
    apply_track_field(track, TrackField::Year(view.year));
    apply_track_field(track, TrackField::TrackNumber(view.track_number));
    apply_track_field(track, TrackField::DiscNumber(view.disc_number));
    apply_track_field(track, TrackField::Rating(view.rating));
    apply_track_field(track, TrackField::Color(view.color));
}

/// Return logical Pino track ID, falling back to aoide's entity UID.
pub fn track_id(entity: &Entity) -> String {
    track_id_from_track(&entity.body.track).unwrap_or_else(|| entity.hdr.uid.to_string())
}

/// Return stored logical Pino track ID, if any.
pub fn track_id_from_track(track: &track::Track) -> Option<String> {
    custom_facet_value(track, &pino_track_id_facet())
}

/// Persist logical Pino track ID inside custom track tags.
pub fn set_track_id(track: &mut track::Track, id: &str) {
    set_facet_label(track, &pino_track_id_facet(), id.to_owned());
}

/// Return the content path stored for a track entity.
pub fn content_path(entity: &Entity) -> String {
    entity
        .body
        .track
        .media_source
        .content
        .link
        .path
        .as_str()
        .to_owned()
}

fn apply_track_field(track: &mut track::Track, field: TrackField) {
    match field {
        TrackField::Title(v) => {
            track.set_track_title(v);
        }
        TrackField::Album(v) => {
            track.set_album_title(v);
        }
        TrackField::Label(v) => {
            track.publisher = if v.is_empty() { None } else { Some(v) };
        }
        TrackField::Artist(v) => set_summary_actor(track, Role::Artist, v),
        TrackField::Composer(v) => set_summary_actor(track, Role::Composer, v),
        TrackField::Remixer(v) => set_summary_actor(track, Role::Remixer, v),
        TrackField::Lyricist(v) => set_summary_actor(track, Role::Lyricist, v),
        TrackField::Genre(v) => set_facet_label(track, FACET_ID_GENRE, v),
        TrackField::Comment(v) => set_facet_label(track, FACET_ID_COMMENT, v),
        TrackField::Isrc(v) => set_facet_label(track, FACET_ID_ISRC, v),
        TrackField::Key(v) => {
            track.metrics.key_signature = if v.is_empty() {
                None
            } else {
                KeyCode::try_from_canonical_str(&v).map(KeySignature::new)
            };
        }
        TrackField::Tempo(v) => {
            track.metrics.tempo_bpm = if v == 0 {
                None
            } else {
                Some(TempoBpm::new(f64::from(v) / 100.0))
            };
        }
        TrackField::Year(v) => {
            track.recorded_at = if v == 0 {
                None
            } else {
                Some(DateOrDateTime::from(YyyyMmDdDate::from_year(
                    v.cast_signed(),
                )))
            };
        }
        TrackField::ReleaseDate(v) => {
            track.released_at = parse_date_or_datetime(&v);
        }
        TrackField::TrackNumber(v) => {
            track.indexes.track.number = if v == 0 {
                None
            } else {
                Some(u16::try_from(v).unwrap_or(u16::MAX))
            };
        }
        TrackField::DiscNumber(v) => {
            track.indexes.disc.number = if v == 0 { None } else { Some(v) };
        }
        TrackField::Rating(v) => {
            set_rating(track, v);
        }
        TrackField::Color(v) => {
            track.color = if v == 0 {
                None
            } else {
                Some(Color::Index(i16::from(v)))
            };
        }
        TrackField::MixName(v) => set_sub_title(track, v),
    }
}

// ── Helpers ──────────────────────────────────────────────────────

/// Extract first label from a faceted tag group.
fn facet_label(track: &track::Track, facet_id: &FacetId) -> String {
    track
        .tags
        .facets
        .iter()
        .find(|ft| ft.facet_id == *facet_id)
        .and_then(|ft| ft.tags.first())
        .and_then(|pt| pt.label.as_ref())
        .map_or_else(String::new, |l| l.as_str().to_owned())
}

fn custom_facet_value(track: &track::Track, facet_id: &FacetId) -> Option<String> {
    track
        .tags
        .facets
        .iter()
        .find(|ft| ft.facet_id == *facet_id)
        .and_then(|ft| ft.tags.first())
        .and_then(|pt| pt.label.as_ref())
        .map(|label| label.as_str().to_owned())
}

/// Get the name of a summary actor for a given role.
fn summary_actor_name(track: &track::Track, role: Role) -> String {
    Actors::summary_actor(track.actors.iter(), role).map_or_else(String::new, |a| a.name.clone())
}

/// Derive audio format from file extension.
fn format_from_path(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

/// Set or remove a summary actor for a given role.
fn set_summary_actor(track: &mut track::Track, role: Role, name: String) {
    let mut actors = std::mem::take(&mut track.actors).untie();
    actors.retain(|a| !(a.role == role && a.kind == actor::Kind::Summary));
    if !name.is_empty() {
        actors.push(Actor {
            role,
            kind: actor::Kind::Summary,
            name,
            role_notes: None,
        });
    }
    track.actors = actors.canonicalize_into();
}

/// Set or remove the first label of a faceted tag group.
fn set_facet_label(track: &mut track::Track, facet_id: &FacetId, value: String) {
    let mut tags = std::mem::take(&mut track.tags).untie();
    if value.is_empty() {
        tags.facets.retain(|ft| ft.facet_id != *facet_id);
    } else if let Some(ft) = tags.facets.iter_mut().find(|ft| ft.facet_id == *facet_id) {
        if let Some(pt) = ft.tags.first_mut() {
            pt.label = Some(Label::from_unchecked(value));
        } else {
            ft.tags.push(PlainTag {
                label: Some(Label::from_unchecked(value)),
                score: Score::default(),
            });
        }
    } else {
        tags.facets.push(FacetedTags {
            facet_id: facet_id.clone(),
            tags: vec![PlainTag {
                label: Some(Label::from_unchecked(value)),
                score: Score::default(),
            }],
        });
    }
    track.tags = tags.canonicalize_into();
}

/// Set or remove the Sub-kind title (mix name).
fn set_sub_title(track: &mut track::Track, name: String) {
    let mut titles = std::mem::take(&mut track.titles).untie();
    titles.retain(|t| t.kind != title::Kind::Sub);
    if !name.is_empty() {
        titles.push(track::Title {
            kind: title::Kind::Sub,
            name,
        });
    }
    track.titles = titles.canonicalize_into();
}

fn rating(track: &track::Track) -> u8 {
    custom_facet_value(track, &pino_rating_facet())
        .and_then(|value| value.parse::<u8>().ok())
        .map_or(0, |value| value.min(5))
}

fn set_rating(track: &mut track::Track, value: u8) {
    let value = value.min(5);
    if value == 0 {
        set_facet_label(track, &pino_rating_facet(), String::new());
    } else {
        set_facet_label(track, &pino_rating_facet(), value.to_string());
    }
}

fn pino_track_id_facet() -> FacetId {
    FacetId::from_unchecked(PINO_TRACK_ID_FACET)
}

fn pino_rating_facet() -> FacetId {
    FacetId::from_unchecked(PINO_RATING_FACET)
}

/// Apply every field from a [`TrackView`] to the track inside an entity.
///
/// Used when overwriting metadata on the remote library from a local [`TrackView`].
pub fn apply_all_fields(entity: &mut Entity, view: &TrackView) {
    apply_view(&mut entity.body.track, view);
}

/// Parse a string into `DateOrDateTime`, trying year-only and full date formats.
fn parse_date_or_datetime(s: &str) -> Option<DateOrDateTime> {
    if s.is_empty() {
        return None;
    }
    // Try year-only (e.g. "2024")
    if let Ok(year) = s.parse::<i16>()
        && year > 0
    {
        return Some(DateOrDateTime::from(YyyyMmDdDate::from_year(year)));
    }
    // Try YYYYMMDD numeric (e.g. "20240115")
    if s.len() == 8
        && let Ok(val) = s.parse::<i32>()
    {
        return Some(DateOrDateTime::from(YyyyMmDdDate::new_unchecked(val)));
    }
    // Try YYYY-MM-DD (e.g. "2024-01-15")
    if s.len() == 10 {
        let stripped: String = s.chars().filter(char::is_ascii_digit).collect();
        if let Ok(val) = stripped.parse::<i32>() {
            return Some(DateOrDateTime::from(YyyyMmDdDate::new_unchecked(val)));
        }
    }
    None
}
