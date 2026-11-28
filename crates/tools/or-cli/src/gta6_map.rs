//! GTA VI's map bodies on the command line: `rpf scan-open` on one prints its structure, and
//! `rpf scan-maps` counts every one a scan of an archive finds, lists each entity and archetype,
//! checks the level-of-detail links across them and recomputes their entities' extents. Nothing
//! is written.

use std::collections::{BTreeMap, HashMap};
use std::fs;

use glam::{Quat, Vec3};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Scan, Rpf9ZlibRun};
use or_io::gta6::map::links::{Links, LodLink, LodTarget};
use or_io::gta6::map::{
    self, ArchetypeClass, InstanceList, MapBody, MapData, MapTypes, ASSET_TYPES, LOD_LEVELS,
};
use or_io::gta6::names::Vocabulary;
use or_world::aabb::agreement;
use or_world::transform::{placed_corners, scale_of};
use or_world::Aabb as Box3;

const USAGE: &str =
    "usage: rpf scan-maps <archive.rpf> [--from <offset>] [--to <offset>] [--limit <bytes>] [--list] [--check] [--extents] [--names]";

/// The entity, list and archetype rows `scan-open` prints of a map body.
const ROWS: usize = 8;

/// Print the structure of the map body a headerless resource's sections hold, or say why the map
/// reader refuses it; `None` when its root is no map class.
pub(crate) fn print_body(virtual_section: &[u8], physical: &[u8]) -> Option<Result<(), String>> {
    if !is_map(map::root_type(virtual_section)?) {
        return None;
    }
    Some(match map::open(virtual_section, physical) {
        Ok(body) => {
            lines(&body, ROWS).iter().for_each(|l| println!("{l}"));
            Ok(())
        }
        Err(e) => Err(format!("GTA VI's map reader refuses this body: {e}")),
    })
}

fn is_map(root: u32) -> bool {
    [map::C_MAP_DATA, map::C_MAP_TYPES].contains(&root)
}

/// The inflated bytes a census keeps of each body the scan finds, to judge its root by.
const HEAD: usize = 1 << 16;

/// Whether a resource whose virtual section starts with `head` may hold a map's root: `false`
/// only where `head` holds the root's class entry and it names no map class.
// ⚠ Reading every resource again to look at its root costs twice the scan over an archive.
fn may_be_map(head: &[u8]) -> bool {
    if let Some(root) = map::root_type(head) {
        return is_map(root);
    }
    let word = |at: usize| {
        let bytes: [u8; 8] = head.get(at..at + 8)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes))
    };
    let (Some(root), Some(table), true) = (word(0x20), word(0x38), head.len() >= 0x4E) else {
        return true;
    };
    let root = root & 0xFFFF_FFFF;
    table >> 28 == 5 && root != 0 && (table & 0x0FFF_FFFF) + 16 * root > head.len() as u64
}

fn point(p: [f32; 3]) -> String {
    format!("({:.2}, {:.2}, {:.2})", p[0], p[1], p[2])
}

fn list_line(kind: &str, k: usize, l: &InstanceList) -> String {
    format!(
        "    {kind} {k} arch {:#010X}{} instances {} box {} .. {}",
        l.archetype_name,
        super::gta6_names::said32(l.archetype_name),
        l.instances,
        point(l.batch_aabb.min),
        point(l.batch_aabb.max)
    )
}

/// What a map body states, with at most `rows` entities, lists and archetypes listed.
fn lines(body: &MapBody, rows: usize) -> Vec<String> {
    match body {
        MapBody::Data(m) => data_lines(m, rows),
        MapBody::Types(t) => types_lines(t, rows),
    }
}

fn data_lines(m: &MapData, rows: usize) -> Vec<String> {
    let interiors = m.entities.iter().filter(|e| e.interior).count();
    let mut out = vec![
        format!(
            "  CMapData      name {:#010X}{}, 64-bit {:#018X}, parent {:#010X}{}, flags {:#X}, contentFlags {:#X}",
            m.name,
            super::gta6_names::said_both(m.name, m.name_word),
            m.name_word,
            m.parent,
            super::gta6_names::said32(m.parent),
            m.flags,
            m.content_flags
        ),
        format!(
            "  streaming     {} .. {}",
            point(m.streaming_extents.min),
            point(m.streaming_extents.max)
        ),
        format!(
            "  entities box  {} .. {}",
            point(m.entities_extents.min),
            point(m.entities_extents.max)
        ),
        format!(
            "  other maps    hash_6C0F9C8C {:#010X}, hash_9E8D32A0 {:#010X}, hash_EC16451D {:#010X}, hash_36C898AA {:#010X}: each names a map body, whose role is not known",
            m.hash_6c0f9c8c, m.hash_9e8d32a0, m.hash_ec16451d, m.hash_36c898aa
        ),
        format!(
            "  entities      {} (CEntityDef {}, CMloInstanceDef {interiors}); parentIndex {}",
            m.entities.len(),
            m.entities.len() - interiors,
            match m.parent {
                0 => "indexes a space not known, as the map has no parent",
                _ => "indexes the parent's entities, then its prop lists",
            }
        ),
    ];
    for (k, e) in m.entities.iter().enumerate().take(rows) {
        let [x, y, z, w] = e.rotation;
        out.push(format!(
            "    [{k}] {} arch {:#010X}{} pos {} rot ({x:.4}, {y:.4}, {z:.4}, {w:.4}) scale {:.3}/{:.3} parent {} lod {:.1} child {:.1} children {} level {}",
            if e.interior { "mlo" } else { "ent" },
            e.archetype_name,
            super::gta6_names::said_both(e.archetype_name, e.archetype_word),
            point(e.position),
            e.scale_xy,
            e.scale_z,
            e.parent_index,
            e.lod_dist,
            e.child_lod_dist,
            e.num_children,
            e.lod_level
        ));
    }
    let instances = |lists: &[InstanceList]| lists.iter().map(|l| l.instances).sum::<usize>();
    out.push(format!(
        "  instancedData {} prop lists ({} instances), {} grass lists ({} instances), counted and not expanded",
        m.prop_lists.len(),
        instances(&m.prop_lists),
        m.grass_lists.len(),
        instances(&m.grass_lists)
    ));
    let props = m.prop_lists.iter().map(|l| ("prop", l));
    let grass = m.grass_lists.iter().map(|l| ("grass", l));
    for (k, (kind, l)) in props.chain(grass).enumerate().take(rows) {
        out.push(list_line(kind, k, l));
    }
    out
}

fn types_lines(t: &MapTypes, rows: usize) -> Vec<String> {
    let of = |c: ArchetypeClass| t.archetypes.iter().filter(|a| a.class == c).count();
    let mut out = vec![format!(
        "  CMapTypes     name {:#010X}{}, archetypes {} (CBaseArchetypeDef {}, CTimeArchetypeDef {}, CMloArchetypeDef {})",
        t.name,
        super::gta6_names::said32(t.name),
        t.archetypes.len(),
        of(ArchetypeClass::Base),
        of(ArchetypeClass::Time),
        of(ArchetypeClass::Interior)
    )];
    for (k, a) in t.archetypes.iter().enumerate().take(rows) {
        out.push(format!(
            "    [{k}] name {:#010X}{} 64-bit {:#018X} lodDist {:.1} bb {} .. {} bs r {:.2} assetType {} model {:#018X} txd {:#018X}",
            a.name,
            super::gta6_names::said_both(a.name, a.name_word),
            a.name_word,
            a.lod_dist,
            point(a.bb_min),
            point(a.bb_max),
            a.bs_radius,
            a.asset_type,
            a.model_word(),
            a.texture_dictionary
        ));
    }
    out
}

/// What a census of map bodies counted.
#[derive(Default, Debug, PartialEq)]
struct Tally {
    maps: u64,
    types: u64,
    refused: u64,
    parented: u64,
    with_entities: u64,
    entities: u64,
    interiors: u64,
    levels: [u64; LOD_LEVELS.len()],
    prop_lists: u64,
    props: u64,
    grass_lists: u64,
    grass: u64,
    archetypes: [u64; 3],
    assets: [u64; ASSET_TYPES.len()],
}

impl Tally {
    fn add(&mut self, body: &MapBody) {
        match body {
            MapBody::Data(m) => {
                self.maps += 1;
                self.parented += u64::from(m.parent != 0);
                self.with_entities += u64::from(!m.entities.is_empty());
                for e in &m.entities {
                    self.entities += 1;
                    self.interiors += u64::from(e.interior);
                    self.levels[usize::from(e.lod_level)] += 1;
                }
                self.prop_lists += m.prop_lists.len() as u64;
                self.props += m.prop_lists.iter().map(|l| l.instances as u64).sum::<u64>();
                self.grass_lists += m.grass_lists.len() as u64;
                self.grass += m
                    .grass_lists
                    .iter()
                    .map(|l| l.instances as u64)
                    .sum::<u64>();
            }
            MapBody::Types(t) => {
                self.types += 1;
                for a in &t.archetypes {
                    self.archetypes[a.class as usize] += 1;
                    self.assets[a.asset_type as usize] += 1;
                }
            }
        }
    }

    fn summary(&self) -> Vec<String> {
        let named = |names: &[&str], counts: &[u64]| {
            (names.iter().zip(counts))
                .map(|(n, c)| format!("{n} {c}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        vec![
            format!(
                "summary: {} CMapData, {} CMapTypes, {} refused",
                self.maps, self.types, self.refused
            ),
            format!(
                "  maps          {} with a parent, {} with entities",
                self.parented, self.with_entities
            ),
            format!(
                "  entities      {} (CMloInstanceDef {}); {}",
                self.entities,
                self.interiors,
                named(&LOD_LEVELS, &self.levels)
            ),
            format!(
                "  lists         {} prop lists of {} instances, {} grass lists of {} instances",
                self.prop_lists, self.props, self.grass_lists, self.grass
            ),
            format!(
                "  archetypes    {}; {}",
                named(
                    &[
                        ArchetypeClass::Base.name(),
                        ArchetypeClass::Time.name(),
                        ArchetypeClass::Interior.name()
                    ],
                    &self.archetypes
                ),
                named(&ASSET_TYPES, &self.assets)
            ),
        ]
    }
}

/// How many of a census's name hashes `vocabulary` names: each pair is named, then counted.
struct Named<'a> {
    vocabulary: &'a Vocabulary,
    maps: [u64; 2],
    archetypes: [u64; 2],
    entities: [u64; 2],
    lists: [u64; 2],
    /// Each archetype word a map types body or an entity states, with whether it is named.
    words: HashMap<u64, bool>,
    /// The names of the maps named, and the words of the others, a row each.
    map_names: Vec<&'a str>,
    unnamed_maps: Vec<u64>,
}

impl<'a> Named<'a> {
    fn new(vocabulary: &'a Vocabulary) -> Self {
        Self {
            vocabulary,
            maps: [0; 2],
            archetypes: [0; 2],
            entities: [0; 2],
            lists: [0; 2],
            words: HashMap::new(),
            map_names: Vec::new(),
            unnamed_maps: Vec::new(),
        }
    }

    fn add(&mut self, body: &MapBody) {
        let count = |pair: &mut [u64; 2], named: bool| {
            pair[0] += u64::from(named);
            pair[1] += 1;
        };
        match body {
            MapBody::Data(m) => {
                let name = m.name_in(self.vocabulary);
                count(&mut self.maps, name.is_some());
                match name {
                    Some(name) => self.map_names.push(name),
                    None => self.unnamed_maps.push(m.name_word),
                }
                for e in &m.entities {
                    let named = e.archetype_in(self.vocabulary).is_some();
                    count(&mut self.entities, named);
                    self.words.insert(e.archetype_word, named);
                }
                // An instance list states its archetype's 32-bit name alone.
                for l in m.prop_lists.iter().chain(&m.grass_lists) {
                    let named = self.vocabulary.name32(l.archetype_name).is_some();
                    count(&mut self.lists, named);
                }
            }
            MapBody::Types(t) => {
                for a in &t.archetypes {
                    let named = a.name_in(self.vocabulary).is_some();
                    count(&mut self.archetypes, named);
                    self.words.insert(a.name_word, named);
                }
            }
        }
    }

    /// The names the naming rules give the maps no kept name names, by their words.
    fn by_rule(&self) -> HashMap<u64, String> {
        let wanted = self.unnamed_maps.iter().copied().collect();
        or_io::gta6::map_names::name_maps(self.map_names.iter().copied(), &wanted)
    }

    /// How many maps the rules name from the kept names, and from those names reversed: a control
    /// that names a map only by chance.
    fn rule_line(&self, by_rule: &HashMap<u64, String>) -> String {
        let rows = |named: &HashMap<u64, String>| {
            (self.unnamed_maps.iter())
                .filter(|word| named.contains_key(word))
                .count()
        };
        let wanted = self.unnamed_maps.iter().copied().collect();
        let reversed: Vec<String> = (self.map_names.iter())
            .map(|name| name.chars().rev().collect())
            .collect();
        let control =
            or_io::gta6::map_names::name_maps(reversed.iter().map(String::as_str), &wanted);
        format!(
            "  named maps    {} more of the {} by the naming rules, from the names of the maps named; {} from those names reversed",
            rows(by_rule),
            self.maps[1],
            rows(&control)
        )
    }

    fn summary(&self) -> String {
        let share = |[named, of]: [u64; 2]| match of {
            0 => "0 of 0".to_string(),
            _ => format!("{named} of {of} ({:.2}%)", 100.0 * named as f64 / of as f64),
        };
        let distinct = self.words.values().filter(|named| **named).count() as u64;
        format!(
            "  named         by the {} names kept, a name being one both its hashes agree on: maps {}, archetypes {}, entities' archetypes {}, distinct archetype words {}, instance lists by the 32-bit hash alone {}",
            self.vocabulary.len(),
            share(self.maps),
            share(self.archetypes),
            share(self.entities),
            share([distinct, self.words.len() as u64]),
            share(self.lists),
        )
    }
}

/// What a census is asked for beside its counts.
#[derive(Default, Clone, Copy)]
struct Asked {
    /// Every entity, list and archetype of each body.
    list: bool,
    /// The level-of-detail links across the bodies.
    check: bool,
    /// Each map's entities' box against the one it states.
    extents: bool,
}

/// The `--check` and `--extents` rows a census prints of each kind, the rest counted.
const DETAIL: usize = 40;

/// Each entity's archetype word and name, its origin, the quaternion that turns its model, and
/// its two scales.
type Placement = (u64, u32, [f32; 3], [f32; 4], [f32; 2]);

/// A map body as `--check` and `--extents` keep it once its tree is dropped.
struct Kept {
    offset: u64,
    name: u32,
    /// Its entities, prop lists and grass lists, counted.
    counts: [usize; 3],
    num_children: Vec<u32>,
    /// Each entity's and prop list's level-of-detail parent: that map's name, and the link.
    parents: Vec<(u32, LodLink)>,
    /// Empty without `--extents`.
    placed: Vec<Placement>,
    lists: Vec<map::Aabb>,
    stored: map::Aabb,
}

/// The map bodies of a census and the archetypes' boxes, by 64-bit word and by 32-bit name.
#[derive(Default)]
struct Group {
    maps: Vec<Kept>,
    /// The maps whose links did not read, so their parents are not known.
    unlinked: u64,
    by_word: HashMap<u64, ([f32; 3], [f32; 3])>,
    by_name: HashMap<u32, ([f32; 3], [f32; 3])>,
}

impl Group {
    fn add(&mut self, offset: u64, body: &MapBody, links: Option<&Links>, extents: bool) {
        let m = match body {
            MapBody::Types(t) => {
                // The first to state a word or a name wins, as on the Map screen.
                for a in &t.archetypes {
                    let bb = (a.bb_min, a.bb_max);
                    self.by_word.entry(a.name_word).or_insert(bb);
                    self.by_name.entry(a.name).or_insert(bb);
                }
                return;
            }
            MapBody::Data(m) => m,
        };
        let parents = match links {
            Some(Links::Data(l)) => (l.entities.iter().chain(&l.prop_lists))
                .filter_map(|k| Some((l.lod_parents.get(k.map?)?.name, *k)))
                .collect(),
            _ => {
                self.unlinked += 1;
                Vec::new()
            }
        };
        let placed = match extents {
            true => (m.entities.iter())
                .map(|e| {
                    let scale = [e.scale_xy, e.scale_z];
                    let turn = e.model_rotation();
                    (e.archetype_word, e.archetype_name, e.position, turn, scale)
                })
                .collect(),
            false => Vec::new(),
        };
        self.maps.push(Kept {
            offset,
            name: m.name,
            counts: [m.entities.len(), m.prop_lists.len(), m.grass_lists.len()],
            num_children: m.entities.iter().map(|e| e.num_children).collect(),
            parents,
            placed,
            lists: (m.prop_lists.iter().chain(&m.grass_lists))
                .map(|l| l.batch_aabb)
                .collect(),
            stored: m.entities_extents,
        });
    }

    /// Each level-of-detail link resolved among the census's maps by name, and each entity's
    /// `numChildren` against the children found naming it; with whether a link is past its
    /// parent's elements or an entity is named by more children than it counts.
    fn check(&self) -> (Vec<String>, bool) {
        // ponytail: of two bodies of one name the first is the parent; tell them apart if a census needs it.
        let mut by_name: HashMap<u32, usize> = HashMap::new();
        for (i, k) in self.maps.iter().enumerate() {
            by_name.entry(k.name).or_insert(i);
        }
        let (mut links, mut outside, mut past) = (0u64, 0u64, Vec::new());
        let mut found: HashMap<(usize, usize), u32> = HashMap::new();
        for k in &self.maps {
            for (parent, link) in &k.parents {
                links += 1;
                let Some(&p) = by_name.get(parent) else {
                    outside += 1;
                    continue;
                };
                let [e, props, grass] = self.maps[p].counts;
                match link.target(e, props, grass) {
                    Some(LodTarget::Entity(at)) => *found.entry((p, at)).or_default() += 1,
                    Some(_) => {}
                    None => past.push(format!(
                        "    {:#012X}  parentIndex {} is past the {} elements of map {parent:#010X}",
                        k.offset,
                        link.parent_index,
                        e + props + grass
                    )),
                }
            }
        }
        let mut under: Vec<String> = (found.iter())
            .filter_map(|((p, at), n)| {
                let k = &self.maps[*p];
                let stated = k.num_children.get(*at).copied().unwrap_or(0);
                (*n > stated).then(|| {
                    format!(
                        "    {:#012X}  entity {at} counts {stated} children, and {n} elements name it",
                        k.offset
                    )
                })
            })
            .collect();
        under.sort();
        let failed = !past.is_empty() || !under.is_empty();
        let mut out = vec![
            format!(
                "check: {links} level-of-detail links; {outside} name a map outside this census, and {} maps' links did not read",
                self.unlinked
            ),
            format!("  {:>8}  past the elements of the map they select", past.len()),
            format!(
                "  {:>8}  entities named by more elements than their numChildren",
                under.len()
            ),
        ];
        out.extend(past.into_iter().take(DETAIL));
        out.extend(under.into_iter().take(DETAIL));
        (out, failed)
    }

    /// Each map's entities' archetype boxes, scaled, turned and placed, and its lists' boxes,
    /// against the entities' box it states; a map with an archetype the census lacks is counted.
    fn extents(&self) -> Vec<String> {
        let (mut complete, mut incomplete) = (0usize, 0usize);
        let mut buckets: BTreeMap<&str, usize> = BTreeMap::new();
        let mut worst: Option<(f32, u64)> = None;
        for k in self.maps.iter().filter(|k| !k.placed.is_empty()) {
            let Some(computed) = self.entities_box(k) else {
                incomplete += 1;
                continue;
            };
            complete += 1;
            let stored = (Vec3::from(k.stored.min), Vec3::from(k.stored.max));
            let delta = agreement(&computed, stored);
            *buckets.entry(super::world::bucket(delta)).or_default() += 1;
            if worst.is_none_or(|(w, _)| delta > w) {
                worst = Some((delta, k.offset));
            }
        }
        let mut out = vec![
            format!(
                "extents: {complete} maps with every archetype in this census, {incomplete} without"
            ),
            "  agreement of the recomputed entities' box with the stored one:".to_string(),
        ];
        out.extend((buckets.iter()).map(|(label, n)| format!("  {n:>8}  {label}")));
        out.extend(worst.map(|(delta, at)| format!("  worst {delta:.3} m at {at:#012X}")));
        out
    }

    /// The box of `k`'s entities and lists; `None` where an entity's archetype is not held.
    fn entities_box(&self, k: &Kept) -> Option<Box3> {
        let mut computed = Box3::default();
        for (word, name, at, turn, [xy, z]) in &k.placed {
            let (lo, hi) = self.by_word.get(word).or_else(|| self.by_name.get(name))?;
            let scale = scale_of(*xy, *z);
            let rotation = Quat::from_array(*turn);
            let (lo, hi) = (Vec3::from(*lo) * scale, Vec3::from(*hi) * scale);
            for c in placed_corners(lo, hi, rotation, Vec3::from(*at)) {
                computed.add_point(c);
            }
        }
        for l in &k.lists {
            computed.add_box(Vec3::from(l.min), Vec3::from(l.max));
        }
        Some(computed)
    }
}

/// One line on the body at `offset`, and every row of it where `asked.list`; `None` for a body
/// that is no map body.
fn census_line(
    file: &fs::File,
    offset: u64,
    limit: u64,
    tally: &mut Tally,
    named: &mut Named,
    asked: Asked,
    group: &mut Group,
) -> Option<String> {
    let (find, bytes) = match read_rpf9_body(file, offset, limit) {
        Ok(read) => read,
        Err(e) => return Some(format!("  {offset:#012X}  not read: {e}")),
    };
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    if !is_map(map::root_type(virtual_section)?) {
        return None;
    }
    let opened = match asked.check {
        true => map::links::open(virtual_section, physical).map(|(body, l)| (body, l.ok())),
        false => map::open(virtual_section, physical).map(|body| (body, None)),
    };
    Some(match opened {
        Ok((body, links)) => {
            tally.add(&body);
            named.add(&body);
            if asked.check || asked.extents {
                group.add(offset, &body, links.as_ref(), asked.extents);
            }
            let said = match &body {
                MapBody::Data(m) => format!(
                    "CMapData   {} entities, {} prop lists, {} grass lists",
                    m.entities.len(),
                    m.prop_lists.len(),
                    m.grass_lists.len()
                ),
                MapBody::Types(t) => format!("CMapTypes  {} archetypes", t.archetypes.len()),
            };
            let mut line = format!("  {offset:#012X}  {said}");
            if asked.list {
                for row in lines(&body, usize::MAX) {
                    line.push('\n');
                    line.push_str(&row);
                }
            }
            line
        }
        Err(e) => {
            tally.refused += 1;
            format!("  {offset:#012X}  refused: {e}")
        }
    })
}

pub(crate) fn cmd_scan_maps(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let (from, args) = super::rpf::split_offset_flag(&args, "--from")?;
    let (to, args) = super::rpf::split_offset_flag(&args, "--to")?;
    let flag = |name: &str| args.iter().any(|a| a == name);
    let asked = Asked {
        list: flag("--list"),
        check: flag("--check"),
        extents: flag("--extents"),
    };
    let keep_names = flag("--names");
    let rest: Vec<&String> = (args.iter())
        .filter(|a| !matches!(a.as_str(), "--list" | "--check" | "--extents" | "--names"))
        .collect();
    let [path] = rest.as_slice() else {
        return Err(USAGE.to_string());
    };
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let cancelled = super::jobs::on_interrupt();
    let mut tally = Tally::default();
    let mut named = Named::new(super::gta6_names::vocabulary());
    let mut group = Group::default();
    let mut scan = Rpf9Scan::new(&file, from.unwrap_or(0)..to.unwrap_or(u64::MAX))
        .map_err(|e| e.to_string())?
        .keeping(HEAD);
    while let Some(find) = scan.next() {
        if cancelled.is_set() {
            return Err("cancelled".to_string());
        }
        let find = find.map_err(|e| e.to_string())?;
        let starts = matches!(
            find.kind,
            Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(_),
                signature: None,
                ..
            })
        );
        if starts && find.stored_len.is_some() && may_be_map(scan.kept(&find)) {
            let line = census_line(
                &file,
                find.offset,
                limit,
                &mut tally,
                &mut named,
                asked,
                &mut group,
            );
            if let Some(line) = line {
                println!("{line}");
            }
        }
    }
    tally.summary().iter().for_each(|l| println!("{l}"));
    println!("{}", named.summary());
    let by_rule = named.by_rule();
    println!("{}", named.rule_line(&by_rule));
    if keep_names {
        println!("{}", super::gta6_names::keep(by_rule.values())?);
    }
    if asked.extents {
        group.extents().iter().for_each(|l| println!("{l}"));
    }
    if asked.check {
        let (said, failed) = group.check();
        said.iter().for_each(|l| println!("{l}"));
        if failed {
            return Err("the check found links that do not hold".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::map::{Aabb, Archetype, Entity};

    fn entity(interior: bool, lod_level: u8) -> Entity {
        Entity {
            interior,
            id: 1,
            rotation: [0.0, 0.0, 0.0, 1.0],
            archetype_word: 2,
            position: [1.0, 2.0, 3.0],
            archetype_name: 0x0BAD_CAFE,
            flags: 0,
            scale_xy: 1.0,
            scale_z: 1.0,
            parent_index: -1,
            hash_e74c3c23: 0,
            lod_dist: -1.0,
            child_lod_dist: 0.0,
            num_children: 0,
            tint_value: 0,
            priority_level: 0,
            lod_level,
        }
    }

    fn list(instances: usize) -> InstanceList {
        InstanceList {
            archetype_name: 0x1234_5678,
            instances,
            batch_aabb: Aabb {
                min: [0.0; 3],
                max: [1.0; 3],
            },
        }
    }

    fn map(parent: u32) -> MapBody {
        MapBody::Data(MapData {
            name: 0x1111_1111,
            name_word: 0x2222,
            parent,
            hash_6c0f9c8c: 0,
            hash_9e8d32a0: 0,
            hash_ec16451d: 0,
            hash_36c898aa: 0,
            flags: 0,
            content_flags: 0,
            streaming_extents: Aabb {
                min: [-1.0; 3],
                max: [1.0; 3],
            },
            entities_extents: Aabb {
                min: [0.0; 3],
                max: [0.5; 3],
            },
            entities: vec![entity(false, 0), entity(true, 5), entity(false, 5)],
            prop_lists: vec![list(3), list(4)],
            grass_lists: vec![list(11)],
        })
    }

    #[test]
    fn the_named_share_counts_what_the_kept_names_name_and_what_the_rules_add() {
        use or_io::gta6::hash::{name_hash32, name_hash64};
        let mut vocabulary = Vocabulary::default();
        vocabulary.add("zq_07", None);
        vocabulary.add("zq_bench_01a", None);
        let mut named = Named::new(&vocabulary);
        for name in ["zq_07", "zq_07_instances_0", "zr_01"] {
            let MapBody::Data(mut m) = map(0) else {
                unreachable!()
            };
            (m.name, m.name_word) = (name_hash32(name.as_bytes()), name_hash64(name.as_bytes()));
            // One entity of an archetype the vocabulary names, by both hashes; two it does not.
            m.entities[0].archetype_name = name_hash32(b"zq_bench_01a");
            m.entities[0].archetype_word = name_hash64(b"zq_bench_01a");
            named.add(&MapBody::Data(m));
        }
        assert_eq!((named.maps, named.entities), ([1, 3], [3, 9]));
        let by_rule = named.by_rule();
        let given: Vec<&String> = by_rule.values().collect();
        assert_eq!(given, ["zq_07_instances_0"]);
        let line = named.rule_line(&by_rule);
        assert!(line.contains("1 more of the 3"), "{line}");
        assert!(line.contains("; 0 from those names reversed"), "{line}");
        assert!(named.summary().contains("maps 1 of 3 (33.33%)"));
    }

    fn types() -> MapBody {
        let archetype = |class, asset_type| Archetype {
            class,
            name: 7,
            name_word: 8,
            texture_dictionary: 0,
            drawable_dictionary: 0,
            drawable: 0,
            fragment: 0,
            lod_dist: 1.0,
            special_attribute: 0,
            bb_min: [0.0; 3],
            bb_max: [1.0; 3],
            bs_centre: [0.5; 3],
            bs_radius: 1.0,
            flags: 0,
            hd_texture_dist: 0.0,
            clip_dictionary: 0,
            physics_dictionary: 0,
            asset_type,
            asset_name: 7,
        };
        MapBody::Types(MapTypes {
            name: 9,
            archetypes: vec![
                archetype(ArchetypeClass::Base, 2),
                archetype(ArchetypeClass::Base, 1),
                archetype(ArchetypeClass::Interior, 0),
            ],
        })
    }

    #[test]
    fn a_census_counts_maps_entities_by_level_lists_and_archetypes_by_class_and_asset_type() {
        let mut tally = Tally::default();
        for body in [map(0), map(5), types()] {
            tally.add(&body);
        }
        let expected = Tally {
            maps: 2,
            types: 1,
            refused: 0,
            parented: 1,
            with_entities: 2,
            entities: 6,
            interiors: 2,
            levels: [2, 0, 0, 0, 0, 4, 0],
            prop_lists: 4,
            props: 14,
            grass_lists: 2,
            grass: 22,
            archetypes: [2, 0, 1],
            assets: [1, 1, 1, 0],
        };
        assert_eq!(tally, expected);
        let summary = tally.summary().join("\n");
        assert!(
            summary.contains("2 CMapData, 1 CMapTypes, 0 refused"),
            "{summary}"
        );
        assert!(summary.contains("LODTYPES_DEPTH_HD 2, "), "{summary}");
        assert!(summary.contains("LODTYPES_DEPTH_ORPHANHD 4"), "{summary}");
        assert!(
            summary.contains("4 prop lists of 14 instances, 2 grass lists of 22"),
            "{summary}"
        );
    }

    #[test]
    fn scan_open_lists_at_most_its_rows_and_says_what_parent_index_indexes() {
        let text = lines(&map(0), 2).join("\n");
        assert!(
            text.contains("3 (CEntityDef 2, CMloInstanceDef 1)"),
            "{text}"
        );
        assert!(text.contains("a space not known"), "{text}");
        assert_eq!(text.matches("arch 0x0BADCAFE").count(), 2, "{text}");
        assert!(
            text.contains("2 prop lists (7 instances), 1 grass lists (11 instances)"),
            "{text}"
        );
        assert!(lines(&map(5), 8)
            .join("\n")
            .contains("the parent's entities"));
        let text = lines(&types(), 8).join("\n");
        assert!(
            text.contains(
                "archetypes 3 (CBaseArchetypeDef 2, CTimeArchetypeDef 0, CMloArchetypeDef 1)"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_census_reads_again_only_a_body_whose_head_does_not_rule_out_a_map_root() {
        // Root 1 of a class table of one entry at 0x80, whose class word is `class`.
        let head = |class: u32, table: u64| {
            let mut h = vec![0u8; 0x100];
            h[0x20..0x24].copy_from_slice(&1u32.to_le_bytes());
            h[0x38..0x40].copy_from_slice(&table.to_le_bytes());
            h[0x4C..0x4E].copy_from_slice(&1u16.to_le_bytes());
            h[0x80..0x84].copy_from_slice(&class.to_le_bytes());
            h
        };
        assert!(may_be_map(&head(map::C_MAP_DATA, 0x5000_0080)));
        assert!(may_be_map(&head(map::C_MAP_TYPES, 0x5000_0080)));
        assert!(!may_be_map(&head(map::C_MAP_DATA ^ 1, 0x5000_0080)));
        assert!(!may_be_map(&head(map::C_MAP_DATA, 0x6000_0080)));
        // A table past the kept bytes, and a head too short to say, are read whole.
        assert!(may_be_map(&head(0, 0x5000_1000)));
        assert!(may_be_map(&head(map::C_MAP_DATA, 0x5000_0080)[..0x40]));
        assert!(may_be_map(&[]));
    }

    #[test]
    fn scan_maps_takes_one_archive() {
        assert_eq!(cmd_scan_maps(&[]), Err(USAGE.to_string()));
        let flags = ["--list", "--check", "--extents"].map(String::from);
        assert_eq!(cmd_scan_maps(&flags), Err(USAGE.to_string()));
    }

    /// A map named `name` of `entities` and no lists, which states `stored` as its entities' box.
    fn data(name: u32, entities: Vec<Entity>, stored: Aabb) -> MapBody {
        let MapBody::Data(mut m) = map(0) else {
            panic!("a map");
        };
        m.name = name;
        m.entities = entities;
        m.prop_lists.clear();
        m.grass_lists.clear();
        m.entities_extents = stored;
        MapBody::Data(m)
    }

    /// Links to the maps `parents` names, one per entity as `(selector, parentIndex)`.
    fn linked(parents: &[u32], links: &[(Option<usize>, i16)]) -> Links {
        use or_io::gta6::map::links::{LodParentMap, MapLinks};
        Links::Data(MapLinks {
            lod_parents: (parents.iter())
                .map(|&name| LodParentMap { name, word: 0 })
                .collect(),
            entities: (links.iter())
                .map(|&(map, parent_index)| LodLink {
                    map,
                    parent_index,
                    instance: -1,
                })
                .collect(),
            prop_lists: Vec::new(),
            prop_points: Vec::new(),
            grass_lists: Vec::new(),
        })
    }

    const UNIT: Aabb = Aabb {
        min: [0.0; 3],
        max: [1.0; 3],
    };

    #[test]
    fn a_check_resolves_each_link_among_the_census_s_maps_and_counts_children_against_their_num() {
        let mut parent = entity(false, 1);
        parent.num_children = 2;
        let mut g = Group::default();
        let lod = data(0xA, vec![parent, entity(false, 0)], UNIT);
        g.add(
            0x100,
            &lod,
            Some(&linked(&[], &[(None, -1), (None, -1)])),
            false,
        );
        let hd = data(0xB, vec![entity(false, 0); 2], UNIT);
        let two = linked(&[0xA], &[(Some(0), 0), (Some(0), 0)]);
        g.add(0x200, &hd, Some(&two), false);
        let (said, failed) = g.check();
        assert!(!failed, "{said:?}");
        assert!(
            said[0].contains("2 level-of-detail links; 0 name a map outside"),
            "{said:?}"
        );

        // A third child, an index past the parent's two entities, a map outside the census and
        // a map whose links did not read.
        let more = data(0xC, vec![entity(false, 0); 3], UNIT);
        let three = linked(&[0xA, 0xD], &[(Some(0), 0), (Some(0), 2), (Some(1), 0)]);
        g.add(0x300, &more, Some(&three), false);
        g.add(0x400, &data(0xE, Vec::new(), UNIT), None, false);
        let (said, failed) = g.check();
        assert!(failed);
        let text = said.join("\n");
        for wanted in [
            "5 level-of-detail links; 1 name a map outside this census, and 1 maps' links did not read",
            "parentIndex 2 is past the 2 elements of map 0x0000000A",
            "0x0000000100  entity 0 counts 2 children, and 3 elements name it",
        ] {
            assert!(text.contains(wanted), "{wanted}\n{text}");
        }
    }

    #[test]
    fn extents_place_each_archetype_box_scaled_and_turned_and_a_list_shows_every_entity() {
        let mut g = Group::default();
        // Its one archetype: word 8, name 7, the unit box.
        g.add(0x10, &types(), None, true);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let mut e = entity(false, 0);
        (e.archetype_word, e.archetype_name) = (8, 7);
        (e.position, e.scale_xy, e.scale_z) = ([10.0, 0.0, 0.0], 2.0, 3.0);
        // Stored as the conjugate: the model turns a quarter about +z, x onto y.
        e.rotation = [0.0, 0.0, -s, s];
        let stored = Aabb {
            min: [8.0, 0.0, 0.0],
            max: [10.0, 2.0, 3.0],
        };
        g.add(0x20, &data(1, vec![e], stored), None, true);
        g.add(0x30, &data(2, vec![entity(false, 0)], stored), None, true);
        let computed = g.entities_box(&g.maps[0]).expect("its archetype is held");
        let off = |a: Vec3, b: [f32; 3]| (a - Vec3::from(b)).abs().max_element();
        assert!(off(computed.min, stored.min) < 1e-5, "{computed:?}");
        assert!(off(computed.max, stored.max) < 1e-5, "{computed:?}");
        assert!(
            g.entities_box(&g.maps[1]).is_none(),
            "an archetype no body holds"
        );
        let text = g.extents().join("\n");
        assert!(
            text.contains("1 maps with every archetype in this census, 1 without"),
            "{text}"
        );

        let listed = lines(&map(0), usize::MAX).join("\n");
        assert_eq!(listed.matches("arch 0x0BADCAFE").count(), 3, "{listed}");
    }
}
