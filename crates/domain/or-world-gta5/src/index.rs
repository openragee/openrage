//! GTA V's rows through the kept file every title shares: a section an archive, which holds that
//! archive's rows as `world_index` seals an index of it alone, and a link that joins them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::state::Gate as GroupGate;
use crate::tables::is_txd_relationships;
use crate::world_forest::read;
use crate::world_forest::{map_box, request_of, Request};
use crate::world_index::{self, Kept, Map};
use crate::{InstallTables, WorldIndex, WorldSource};
use or_io::meta::instance::parse_schemaless;
use or_io::meta::MetaInstance;
use or_world::contract::{WorldError, WorldIndexer};
use or_world::kept::Section;
use or_world::model::{Gate, Index, MapId, MapRow, PlacementRow, SwitchId, EVERY_HOUR};

use crate::install::Gta5World;
use crate::source::One;

/// The version of what this crate writes in a section, over `world_index`'s own of what a row holds.
pub const RULES: u32 = 2 << 16 | world_index::VERSION << 8 | world_index::RULES;

/// How a file of the mount holds what an archetype draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Holding {
    /// A drawable that is a file of its own.
    File,
    /// An entry of a drawable dictionary.
    Dictionary,
    /// The drawable of a fragment.
    Fragment,
}

impl Holding {
    pub const ALL: [Self; 3] = [Self::File, Self::Dictionary, Self::Fragment];

    /// The extension of the files that hold a model so.
    pub fn extension(self) -> &'static str {
        match self {
            Self::File => "ydr",
            Self::Dictionary => "ydd",
            Self::Fragment => "yft",
        }
    }
}

/// A file of the mount: the root archive that lists the copy the mount would load, and its logical path.
pub(crate) type FileAt = (usize, String);

/// What the link makes beside the rows: the index of the mount, and each map's row by its name.
pub struct Linked {
    pub world: WorldIndex,
    by_hash: HashMap<u32, MapId>,
    /// Each file that holds models, by how it holds them and the hash of its stem.
    models: HashMap<(Holding, u32), FileAt>,
    /// Each texture dictionary that is a file of its own, by the hash of its stem.
    textures: HashMap<u32, FileAt>,
    /// Which texture dictionary each falls through to, as the install's relationship files state it.
    relations: InstallTables,
}

impl Linked {
    /// The row of the map named `hash`.
    pub fn row_of(&self, hash: u32) -> Option<MapId> {
        self.by_hash.get(&hash).copied()
    }

    /// The map of the row `id`.
    pub fn map(&self, id: MapId) -> Option<&Map> {
        self.world.maps().get(id.0 as usize)
    }

    /// Where the file named `stem` that holds models as `holding` lies.
    pub(crate) fn model_file(&self, holding: Holding, stem: u32) -> Option<(usize, &str)> {
        let (archive, logical) = self.models.get(&(holding, stem))?;
        Some((*archive, logical.as_str()))
    }

    /// Where the texture dictionary named `stem` lies, where it is a file of its own.
    pub(crate) fn texture_file(&self, stem: u32) -> Option<(usize, &str)> {
        let (archive, logical) = self.textures.get(&stem)?;
        Some((*archive, logical.as_str()))
    }

    /// The texture dictionary the one named `dictionary` falls through to.
    pub fn texture_parent(&self, dictionary: u32) -> Option<u32> {
        self.relations.txd_parent_of(dictionary)
    }
}

/// A relationship file as its value tree, whichever of its two forms it is written in.
fn tree_of(bytes: &[u8]) -> Option<MetaInstance> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<') {
        // ⚠ By the schema where the host staged one that states the file's class, and as the
        // file's own elements state it where none does: a clean checkout stages none.
        let schema = or_io::meta::default_library();
        let typed = or_io::meta::instance::parser::parse_meta(schema, body);
        return typed.or_else(|_| parse_schemaless(body)).ok();
    }
    world_index::parse(bytes)
}

/// What the link of GTA V made beside the rows of `index`.
pub(crate) fn linked(index: &Index) -> Result<&Linked, WorldError> {
    index.title::<Linked>().ok_or(WorldError::Lacks(
        "tables of a GTA V link: the index is another title's",
    ))
}

/// The map of the row `id` of `index`, with what the link made.
pub(crate) fn map_of(index: &Index, id: MapId) -> Result<(&Linked, &Map), WorldError> {
    let linked = linked(index)?;
    let map = linked.map(id).ok_or(WorldError::NoSuchMap(id))?;
    Ok((linked, map))
}

/// What lets `map` in, as the row of it states.
// ⚠ A group switched by the weather that names none lets its map in under no weather, where a
// gate of the core that names none lets it in under any: such a map is held back.
fn gate_of(world: &WorldIndex, map: &Map) -> Gate {
    let held = Gate::Held {
        switch: Some(SwitchId(map.hash)),
    };
    let weathers = |types: &[u32]| types.iter().map(|name| u64::from(*name)).collect();
    match request_of(world, map) {
        Request::Always => Gate::Always,
        Request::Script | Request::Group(GroupGate::Scripted) => held,
        Request::Group(GroupGate::Timed { hours }) => Gate::When {
            hours: *hours & EVERY_HOUR,
            weathers: Vec::new(),
        },
        Request::Group(GroupGate::Weather { types } | GroupGate::TimedWeather { types, .. })
            if types.is_empty() =>
        {
            held
        }
        Request::Group(GroupGate::Weather { types }) => Gate::When {
            hours: EVERY_HOUR,
            weathers: weathers(types),
        },
        Request::Group(GroupGate::TimedWeather { hours, types }) => Gate::When {
            hours: *hours & EVERY_HOUR,
            weathers: weathers(types),
        },
    }
}

fn unread(what: &str, why: impl std::fmt::Display) -> WorldError {
    WorldError::Unread {
        what: what.to_owned(),
        why: why.to_string(),
    }
}

impl WorldIndexer for Gta5World {
    fn rules(&self) -> u32 {
        RULES
    }

    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError> {
        let one = One {
            source: &self.source,
            archive,
            relationships: Mutex::default(),
        };
        let (rows, report) = WorldIndex::build(&one, None);
        // ⚠ A file of the archive that did not open leaves its row unread, and the archive's
        // stamp stands all the same: the rows are kept until the archive changes.
        if rows.archives().any(|listed| listed.stamp.is_none()) && self.stamped(archive) {
            tracing::warn!(
                archive,
                opened = report.opened,
                "an archive's rows were read with a file of it unopened"
            );
        }
        let sealed = rows.seal().map_err(|source| WorldError::Io {
            what: format!("the rows of archive {archive}"),
            source,
        })?;
        out.put_text(&self.source.patch_of(archive).unwrap_or_default());
        out.put_bytes(&sealed);
        // The relationship files of the archive as they are, in the order the walk met them: the
        // link reads them, so a second start opens none.
        let met = one.relationships.into_inner().unwrap_or_default();
        let files: Vec<Vec<u8>> = (met.iter())
            .filter_map(|logical| self.source.open(archive, logical))
            .filter(|bytes| tree_of(bytes).is_some_and(|root| is_txd_relationships(&root)))
            .collect();
        out.put_u32(files.len() as u32);
        for file in &files {
            out.put_bytes(file);
        }
        Ok(())
    }

    fn whole(&self, _archives: &[Section], _out: &mut Section) -> Result<(), WorldError> {
        // What is derived across the archives is made at the link, in memory, from their rows.
        Ok(())
    }

    fn link(&self, archives: &[Section], _whole: &Section) -> Result<Index, WorldError> {
        let (mut parts, mut patches) = (Vec::with_capacity(archives.len()), Vec::new());
        let mut relations = InstallTables::new();
        for (at, section) in archives.iter().enumerate() {
            let mut from = section.reader();
            let patch = from.text()?;
            if !patch.is_empty() {
                patches.push((at, patch.to_owned()));
            }
            let sealed = from.bytes()?;
            // In the mount's order, so a later archive's statement of a dictionary is the one kept.
            for _ in 0..from.u32()? {
                if let Some(root) = tree_of(from.bytes()?) {
                    relations.add_relationships(&root);
                }
            }
            from.end()?;
            let what = format!("the kept rows of archive {at}");
            parts.push(Kept::unseal(sealed).map_err(|e| unread(&what, e))?);
        }
        let world = Kept::joined(parts, &patches);
        let places = (0u32..).map(MapId).zip(world.maps());
        let by_hash: HashMap<u32, MapId> = places.map(|(id, map)| (map.hash, id)).collect();
        let rows = (world.maps().iter())
            .map(|map| MapRow {
                name: u64::from(map.hash),
                label: map.name.clone(),
                // A map whose file did not read streams by no box: only a child asks for it.
                boxes: map_box(map).filter(|_| map.read),
                parents: (world.parent_of(map))
                    .filter(|parent| parent.hash != map.hash)
                    .and_then(|parent| by_hash.get(&parent.hash).copied())
                    .into_iter()
                    .collect(),
                gate: gate_of(&world, map),
                archive: map.archive as u32,
            })
            .collect();
        let (mut models, mut textures) = (HashMap::new(), HashMap::new());
        // In the mount's order, so the last copy of a stem is the one kept.
        for (archive, logical) in world.files() {
            let leaf = logical.rsplit('/').next().unwrap_or(logical);
            let leaf = leaf.to_ascii_lowercase();
            let Some((stem, extension)) = leaf.rsplit_once('.') else {
                continue;
            };
            let name = or_io::string_hash(stem.as_bytes());
            let at = (archive, logical.to_owned());
            match Holding::ALL
                .into_iter()
                .find(|h| h.extension() == extension)
            {
                Some(holding) => {
                    models.insert((holding, name), at);
                }
                None if extension == "ytd" => {
                    textures.insert(name, at);
                }
                None => {}
            }
        }
        let linked = Linked {
            world,
            by_hash,
            models,
            textures,
            relations,
        };
        Ok(Index::new(rows)?.with_title(Arc::new(linked)))
    }

    fn placements(
        &self,
        index: &Index,
        map: MapId,
        out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError> {
        let (linked, map) = map_of(index, map)?;
        let read =
            read::map(&linked.world, &self.source, map).map_err(|why| unread(&map.logical, why))?;
        out.extend(
            (0u32..)
                .zip(&read.placed)
                .map(|(node, placed)| PlacementRow {
                    node,
                    archetype: u64::from(placed.archetype),
                    position: placed.to_world.translation.into(),
                }),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relationship_file_in_xml_reads_with_no_schema_of_its_class() {
        let xml = b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CMapParentTxds><txdRelationships><Item><parent>b</parent><child>a</child></Item><Item><parent>c</parent><child>b</child></Item></txdRelationships></CMapParentTxds>";
        let root = tree_of(xml).expect("the file reads");
        assert!(is_txd_relationships(&root));
        let mut relations = InstallTables::new();
        assert_eq!(relations.add_relationships(&root), 2);
        let name = |name: &str| or_io::string_hash(name.as_bytes());
        assert_eq!(relations.txd_parent_of(name("a")), Some(name("b")));
        assert_eq!(relations.txd_parent_of(name("b")), Some(name("c")));
        assert_eq!(relations.txd_parent_of(name("c")), None);
        assert!(tree_of(b"<not closed").is_none());
    }
}
