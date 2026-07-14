// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `exportExt.pdb` (tags). The main `export.pdb` is now built by
//! [`rekordcrate::DeviceExportWriter`]; only the extended tag DB remains pino-specific,
//! since the high-level writer doesn't cover it.

use super::SyncError;
use rekordcrate::pdb::ext::{
    ExtPageType, ExtRow, ParentId, TagId, TagOrCategory, TagOrCategoryStrings, TrackTag,
};
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::offset_array::OffsetArrayContainer;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::{DatabaseType, PageType, Row, Subtype, TrackId};
use rekordcrate::util::MaybeCalculated;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// Generate the extended PDB (`exportExt.pdb`) with tag/track-tag associations.
///
/// `track_tags` carries, per exported track, the **real** PDB track id (returned by
/// `DeviceExportWriter::add_track`) and the track's tag labels. A single "My Tags" category
/// is created, with every unique tag label as a child tag.
///
/// No-op (no file written) when no track carries any tag.
pub(super) fn generate_ext_pdb(
    track_tags: &[(TrackId, Vec<String>)],
    dest_dir: &Path,
) -> Result<(), SyncError> {
    // Collect unique tag labels → sequential ids. Id 1 is reserved for the "My Tags" category.
    let mut tag_map: HashMap<String, u32> = HashMap::new();
    let mut next_tag_id = 2u32;
    for (_, labels) in track_tags {
        for label in labels {
            if !label.is_empty() && !tag_map.contains_key(label) {
                tag_map.insert(label.clone(), next_tag_id);
                next_tag_id += 1;
            }
        }
    }

    if tag_map.is_empty() {
        return Ok(());
    }

    let rekordbox_dir = dest_dir.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&rekordbox_dir)?;

    let table_page_types = [
        PageType::Ext(ExtPageType::Tag),
        PageType::Ext(ExtPageType::TrackTag),
    ];
    let ext_path = rekordbox_dir.join("exportExt.pdb");
    let ext_file = File::create(&ext_path)?;
    let mut ext_db = Database::create(ext_file, DatabaseType::Ext, &table_page_types)?;

    // "My Tags" category (id = 1).
    ext_db.add_row(Row::Ext(ExtRow::Tag(TagOrCategory {
        subtype: Subtype(0x0680),
        index_shift: 0,
        unknown1: 0,
        unknown2: 0,
        parent_id: ParentId(None),
        position: 0,
        id: TagId(1),
        raw_is_category: 1,
        offsets: OffsetArrayContainer {
            offsets: MaybeCalculated::Calculated,
            inner: TagOrCategoryStrings {
                name: "My Tags".parse()?,
                unknown: DeviceSQLString::empty(),
            },
        },
    })))?;

    // Individual tags under the category, sorted by id for deterministic output.
    let mut tags_sorted: Vec<_> = tag_map.iter().collect();
    tags_sorted.sort_by_key(|(_, id)| **id);
    for (position, (label, id)) in tags_sorted.iter().enumerate() {
        ext_db.add_row(Row::Ext(ExtRow::Tag(TagOrCategory {
            subtype: Subtype(0x0680),
            index_shift: 0,
            unknown1: 0,
            unknown2: 0,
            parent_id: ParentId(std::num::NonZero::new(1)),
            position: u32::try_from(position).unwrap_or(u32::MAX),
            id: TagId(**id),
            raw_is_category: 0,
            offsets: OffsetArrayContainer {
                offsets: MaybeCalculated::Calculated,
                inner: TagOrCategoryStrings {
                    name: label.parse()?,
                    unknown: DeviceSQLString::empty(),
                },
            },
        })))?;
    }

    // Track-tag junction rows, using the real PDB track ids.
    for (track_id, labels) in track_tags {
        for label in labels {
            if let Some(&tag_id) = tag_map.get(label) {
                ext_db.add_row(Row::Ext(ExtRow::TrackTag(TrackTag {
                    track_id: *track_id,
                    tag_id: TagId(tag_id),
                    unknown_const: 3,
                })))?;
            }
        }
    }

    ext_db.close()?;
    println!("Ext PDB: {} tag(s) in 1 category", tag_map.len());
    Ok(())
}
