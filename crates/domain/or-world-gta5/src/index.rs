//! GTA V's rows through the kept file every title shares: a section an archive, which holds that
//! archive's rows as `or_depends` seals an index of it alone, and a link that joins them.

use std::collections::HashMap;
use std::sync::Arc;

use or_depends::world_forest::read;
use or_depends::world_forest::{map_box, request_of, Request};
use or_depends::world_index::{self, Kept, Map};
use or_depends::{WorldIndex, WorldSource};
use or_world::contract::{WorldError, WorldIndexer};
use or_world::kept::Section;
use or_world::model::{Gate, Index, MapId, MapRow, PlacementRow, SwitchId, EVERY_HOUR};
use or_world::state::Gate as GroupGate;

use crate::install::Gta5World;
use crate::source::One;

/// The version of what this crate writes in a section, over `or_depends`' own of what a row holds.
pub const RULES: u32 = 1 << 16 | world_index::VERSION << 8 | world_index::RULES;

/// What the link makes beside the rows: the index of the mount, and each map's row by its name.
pub struct Linked {
    pub world: WorldIndex,
    by_hash: HashMap<u32, MapId>,
    /// Each drawable that is a file of its own, by the hash of its stem: the root archive that lists the copy the mount would load, and its logical path.
    drawables: HashMap<u32, (usize, String)>,
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

    /// Where the drawable named `stem` lies, where it is a file of its own.
    pub(crate) fn drawable(&self, stem: u32) -> Option<(usize, &str)> {
        let (archive, logical) = self.drawables.get(&stem)?;
        Some((*archive, logical.as_str()))
    }
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
        Ok(())
    }

    fn whole(&self, _archives: &[Section], _out: &mut Section) -> Result<(), WorldError> {
        // What is derived across the archives is made at the link, in memory, from their rows.
        Ok(())
    }

    fn link(&self, archives: &[Section], _whole: &Section) -> Result<Index, WorldError> {
        let (mut parts, mut patches) = (Vec::with_capacity(archives.len()), Vec::new());
        for (at, section) in archives.iter().enumerate() {
            let mut from = section.reader();
            let patch = from.text()?;
            if !patch.is_empty() {
                patches.push((at, patch.to_owned()));
            }
            let sealed = from.bytes()?;
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
        let mut drawables = HashMap::new();
        // In the mount's order, so the last copy of a stem is the one kept.
        for (archive, logical) in world.files() {
            let leaf = logical.rsplit('/').next().unwrap_or(logical);
            let leaf = leaf.to_ascii_lowercase();
            if let Some(stem) = leaf.strip_suffix(".ydr") {
                let name = or_io::string_hash(stem.as_bytes());
                drawables.insert(name, (archive, logical.to_owned()));
            }
        }
        let linked = Linked {
            world,
            by_hash,
            drawables,
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
