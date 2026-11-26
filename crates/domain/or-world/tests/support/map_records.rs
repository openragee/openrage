//! The census of what the maps of an install hold besides entities, and the judgements the install suite makes of it: shared with
//! the binary that holds those judgements to records built by hand, which a gate runs.

#![allow(dead_code)]

use std::collections::BTreeMap;

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::File;
use or_io::hash::literal_string_hash;
use or_io::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry};
use or_io::meta::binary::{type_tables, ContainerLayout, MetaLayout};
use or_io::meta::schema::{ArrayKind, FieldType};
use or_io::meta::{
    save_far_lights, BinaryFormat, BoxOccluder, CCarGen, CDistantLODLight, CLODLight, CMapData,
    CTimeCycleModifier, FloatXYZ, MetaDocument, MetaInstance, OccludeModel, Value,
};
use or_world::field;

/// The seven classes of the records, each with the members its record types.
pub const RECORDS: [(&str, &[&str]); 7] = [
    ("CCarGen", CCarGen::MEMBERS),
    ("BoxOccluder", BoxOccluder::MEMBERS),
    ("OccludeModel", OccludeModel::MEMBERS),
    ("CTimeCycleModifier", CTimeCycleModifier::MEMBERS),
    ("CLODLight", CLODLight::MEMBERS),
    ("CDistantLODLight", CDistantLODLight::MEMBERS),
    ("FloatXYZ", FloatXYZ::MEMBERS),
];

pub const CAR_GEN: usize = 0;
pub const BOX_OCCLUDER: usize = 1;
pub const OCCLUDE_MODEL: usize = 2;
pub const MODIFIER: usize = 3;
pub const LOD_LIGHT: usize = 4;
pub const DISTANT_LIGHT: usize = 5;
pub const POINT: usize = 6;

/// The members of a map that hold the records, in the order of [`RECORDS`].
pub const HELD_UNDER: [&str; 6] = [
    "carGenerators",
    "boxOccluders",
    "occludeModels",
    "timeCycleModifiers",
    "LODLightsSOA",
    "DistantLODLightsSOA",
];

pub fn key(name: &str) -> u32 {
    literal_string_hash(name.as_bytes())
}

/// A member's type, as a type table states it.
pub fn spelt(ty: &FieldType) -> String {
    match ty {
        FieldType::Array { elem, kind } => match kind {
            ArrayKind::AtArray => format!("atArray<{}>", spelt(elem)),
            ArrayKind::Pointer { size_var, align } => format!(
                "pointer to a run of {} counted by {}, aligned to {align}",
                spelt(elem),
                size_var.as_deref().unwrap_or("nothing")
            ),
            other => format!("{other:?}<{}>", spelt(elem)),
        },
        FieldType::Struct(name) => format!("struct {name}"),
        FieldType::Pointer { base, .. } => format!("pointer to {base}"),
        FieldType::String(kind) => format!("{kind:?}"),
        other => format!("{other:?}"),
    }
}

/// How an RBM class states the member `entry`, where a table says more than its type: what counts a run of bytes.
pub fn counted_run(class: &RbmClass, entry: &RbmClassEntry) -> Option<(u32, u8)> {
    let counted = matches!(
        entry.data_type,
        PsoType::ArrayPointer32WithCount | PsoType::ArrayPointer64WithCount
    );
    let by = class.entry(u16::try_from(entry.array_count()).ok()?)?;
    counted.then_some((by.name_hash, entry.array_align_power()))
}

/// The lengths of the arrays a family of far lights holds, in the order the family states them.
pub fn lengths(family: Option<&MetaInstance>) -> Vec<usize> {
    let arrays = family.into_iter().flat_map(|inst| &inst.fields);
    arrays
        .filter_map(|(_, value)| match value {
            Value::Array(items) => Some(items.len()),
            _ => None,
        })
        .collect()
}

/// What one map of far lights holds: where it is, the map it is and the one it names, and its bytes.
pub struct LightMap {
    pub path: String,
    pub name: u32,
    pub parent: u32,
    pub lod: usize,
    pub distant: usize,
    pub bytes: Vec<u8>,
}

/// One class as the files state it, its members with their types: by encoding, the files that state it so and the
/// records those files hold.
#[derive(Default)]
pub struct Signature {
    pub by_format: BTreeMap<&'static str, (usize, usize)>,
}

impl Signature {
    pub fn records(&self) -> usize {
        self.by_format.values().map(|(_, records)| records).sum()
    }
}

/// What the maps of one install hold besides entities.
#[derive(Default)]
pub struct Census {
    pub label: String,
    pub maps: usize,
    pub formats: BTreeMap<&'static str, usize>,
    /// Each member a map states, under the root or one struct below it: the maps that state it, the maps that hold
    /// something under it and how many elements they hold.
    pub held: BTreeMap<String, (usize, usize, usize)>,
    /// The classes of [`RECORDS`], and the root, by the members their files state.
    pub signatures: BTreeMap<&'static str, BTreeMap<String, Signature>>,
    /// The member names the records of each class state in the install, by name hash, with one spelling of each.
    pub stated: [BTreeMap<u32, String>; 7],
    /// How many records of each class the install holds, and in how many maps.
    pub records: [(usize, usize); 7],
    /// Occlude models whose `dataSize` is the count of the bytes read behind `verts`, and those where it is not.
    pub sized: (usize, usize),
    /// Counted runs of bytes that the files' own tables state as `verts` counted by `dataSize`, and those stated otherwise.
    pub counted_runs: (usize, usize),
    /// Maps of distant lights whose `numStreetLights` is no more than the lights they hold, and those where it is more.
    pub street_lights: (usize, usize),
    pub lights: Vec<LightMap>,
    /// Maps that hold a record: those read typed, those saved untouched to the bytes they were read from, and those
    /// saved to their fixed point besides.
    pub with_a_record: usize,
    pub saved_as_read: usize,
    pub saved_fixed: usize,
    pub pairs: usize,
    pub pairs_saved: usize,
    /// Maps whose first two occlude models hold other counts of bytes, and those that read back what was saved once the
    /// two changed places.
    pub swapped: (usize, usize),
    pub problems: Vec<String>,
}

pub fn count(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::ByteArray(bytes) => bytes.len(),
        Value::Map(pairs) => pairs.len(),
        _ => 0,
    }
}

pub fn format_name(format: BinaryFormat) -> &'static str {
    match format {
        BinaryFormat::Rbm => "RBM",
        BinaryFormat::Pso => "PSO",
        BinaryFormat::Rbf => "RBF",
    }
}

/// Whether `saved` is the bytes `read` was: whole, or for an RBM its header and decompressed payload.
pub fn same_bytes(format: BinaryFormat, read: &[u8], saved: &[u8]) -> bool {
    match format {
        BinaryFormat::Rbm => {
            saved.get(..16) == read.get(..16)
                && matches!((Rsc7::open(read), Rsc7::open(saved)), (Ok(a), Ok(b)) if a.data == b.data)
        }
        _ => saved == read,
    }
}

impl Census {
    pub fn fold(&mut self, path: &str, bytes: Vec<u8>) {
        let doc = match MetaDocument::parse(super::support::schema(), bytes.clone()) {
            Ok(doc) => doc,
            Err(e) => return self.problems.push(format!("{path}: reads: {e}")),
        };
        let Some(layout) = &doc.layout else {
            return self.problems.push(format!("{path}: is no binary document"));
        };
        self.maps += 1;
        let format = format_name(layout.format);
        *self.formats.entry(format).or_default() += 1;
        let root = &doc.instance;
        self.walk("", root, 0);

        let held = self.fold_records(root);
        self.fold_signatures(layout, format, &held);
        self.fold_lights(path, root, &bytes);

        let typed = match CMapData::from_bytes(&bytes) {
            Ok(typed) => typed,
            Err(e) => return self.problems.push(format!("{path}: opens typed: {e}")),
        };
        if let Some(differs) = typed_differs(root, &typed) {
            self.problems.push(format!("{path}: {differs}"));
        }
        if held[..POINT].iter().all(|&records| records == 0) {
            return;
        }
        self.with_a_record += 1;
        // ⚠ The fixed point is the save of what the save reads as, which a fresh DEFLATE does not disturb.
        let saved = typed.to_bytes().and_then(|saved| {
            let back = CMapData::from_bytes(&saved)?;
            Ok((
                same_bytes(layout.format, &bytes, &saved),
                back == typed,
                back.to_bytes()? == saved,
            ))
        });
        match saved {
            Ok((as_read, true, true)) => {
                self.saved_fixed += 1;
                self.saved_as_read += usize::from(as_read);
            }
            Ok((_, equal, fixed)) => self.problems.push(format!(
                "{path}: saved, it reads back equal: {equal}, and saves again to the same bytes: {fixed}"
            )),
            Err(e) => self.problems.push(format!("{path}: saves: {e}")),
        }
        self.fold_swap(path, &typed);
    }

    /// Saves `typed` with its first two occlude models in each other's place, where they hold other counts of bytes.
    ///
    /// ⚠ The layout a map arrived with sizes a block for each run of bytes, so this is the edit of as many records that
    /// layout cannot take.
    pub fn fold_swap(&mut self, path: &str, typed: &CMapData) {
        let [first, second, ..] = &typed.occlude_models[..] else {
            return;
        };
        if first.verts.len() == second.verts.len() {
            return;
        }
        self.swapped.0 += 1;
        let mut edited = typed.clone();
        edited.occlude_models.swap(0, 1);
        let saved = edited.to_bytes().and_then(|saved| {
            let back = CMapData::from_bytes(&saved)?;
            Ok((back == edited, back.to_bytes()? == saved))
        });
        match saved {
            Ok((true, true)) => self.swapped.1 += 1,
            Ok((equal, fixed)) => self.problems.push(format!(
                "{path}: with its first two occlude models in each other's place it reads back equal: {equal}, and saves again to the same bytes: {fixed}"
            )),
            Err(e) => self.problems.push(format!(
                "{path}: with its first two occlude models in each other's place it saves: {e}"
            )),
        }
    }

    /// Counts the records `root` holds, by class, and takes the members each states.
    pub fn fold_records(&mut self, root: &MetaInstance) -> [usize; 7] {
        let mut held = [0usize; 7];
        for class in [CAR_GEN, BOX_OCCLUDER, OCCLUDE_MODEL, MODIFIER] {
            for record in field::structs_at(root, key(HELD_UNDER[class])) {
                held[class] += 1;
                self.state(class, record);
            }
        }
        for model in field::structs_at(root, key("occludeModels")) {
            let stated = field::u32_at(model, key("dataSize")).unwrap_or(0) as usize;
            let read = field::get(model, key("verts")).map_or(0, count);
            match stated == read {
                true => self.sized.0 += 1,
                false => self.sized.1 += 1,
            }
        }
        for (class, points) in [(LOD_LIGHT, "direction"), (DISTANT_LIGHT, "position")] {
            let Some(family) = field::struct_at(root, key(HELD_UNDER[class])) else {
                continue;
            };
            self.state(class, family);
            for point in field::structs_at(family, key(points)) {
                held[class] += 1;
                held[POINT] += 1;
                self.state(POINT, point);
            }
        }
        if let Some(family) = field::struct_at(root, key("DistantLODLightsSOA")) {
            if held[DISTANT_LIGHT] > 0 {
                let street = field::u32_at(family, key("numStreetLights")).unwrap_or(0) as usize;
                match street <= held[DISTANT_LIGHT] {
                    true => self.street_lights.0 += 1,
                    false => self.street_lights.1 += 1,
                }
            }
        }
        for (class, &records) in held.iter().enumerate() {
            self.records[class].0 += records;
            self.records[class].1 += usize::from(records > 0);
        }
        held
    }

    pub fn state(&mut self, class: usize, record: &MetaInstance) {
        for (member, _) in &record.fields {
            self.stated[class]
                .entry(field::name_hash(member))
                .or_insert_with(|| member.clone());
        }
    }

    /// Takes each class of [`RECORDS`] and the root as the type table of `layout` states them.
    pub fn fold_signatures(
        &mut self,
        layout: &MetaLayout,
        format: &'static str,
        held: &[usize; 7],
    ) {
        let tables = type_tables::of_layout(layout);
        let classes = RECORDS.iter().map(|(class, _)| *class).zip(held.iter());
        for (class, &records) in classes.chain([("CMapData", &1usize)]) {
            let stated =
                (tables.structs.values()).find(|def| field::name_hash(&def.name) == key(class));
            let Some(def) = stated else { continue };
            let members: Vec<String> = (def.fields.iter())
                .map(|f| format!("{}: {}", f.name, spelt(&f.ty)))
                .collect();
            let signature = self
                .signatures
                .entry(class)
                .or_default()
                .entry(members.join("\n        "))
                .or_default();
            let stated = signature.by_format.entry(format).or_default();
            (stated.0, stated.1) = (stated.0 + 1, stated.1 + records);
        }
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            return;
        };
        let Some(class) = rbm.model.class_for(key("OccludeModel")) else {
            return;
        };
        for entry in &class.entries {
            let Some((by, align_power)) = counted_run(class, entry) else {
                continue;
            };
            let as_measured =
                entry.name_hash == key("verts") && by == key("dataSize") && align_power == 4;
            match as_measured {
                true => self.counted_runs.0 += 1,
                false => self.counted_runs.1 += 1,
            }
        }
    }

    pub fn fold_lights(&mut self, path: &str, root: &MetaInstance, bytes: &[u8]) {
        let lod = lengths(field::struct_at(root, key("LODLightsSOA")));
        let distant = lengths(field::struct_at(root, key("DistantLODLightsSOA")));
        if lod.iter().chain(&distant).all(|&len| len == 0) {
            return;
        }
        for (family, lengths) in [("LODLightsSOA", &lod), ("DistantLODLightsSOA", &distant)] {
            if lengths.iter().any(|len| Some(len) != lengths.first()) {
                self.problems
                    .push(format!("{path}: the arrays of `{family}` hold {lengths:?}"));
            }
        }
        self.lights.push(LightMap {
            path: path.to_string(),
            name: field::hash_at(root, key("name")).unwrap_or(0),
            parent: field::hash_at(root, key("parent")).unwrap_or(0),
            lod: lod.first().copied().unwrap_or(0),
            distant: distant.first().copied().unwrap_or(0),
            bytes: bytes.to_vec(),
        });
    }

    pub fn walk(&mut self, prefix: &str, inst: &MetaInstance, depth: usize) {
        for (name, value) in &inst.fields {
            let at = format!("{prefix}{name}");
            let elements = count(value);
            let entry = self.held.entry(at.clone()).or_default();
            entry.0 += 1;
            entry.1 += usize::from(elements > 0);
            entry.2 += elements;
            if let (0, Value::Struct(inner)) = (depth, value) {
                self.walk(&format!("{at}."), inner, depth + 1);
            }
        }
    }

    pub fn merge(&mut self, other: Census) {
        self.maps += other.maps;
        for (format, maps) in other.formats {
            *self.formats.entry(format).or_default() += maps;
        }
        for (member, (stated, held, elements)) in other.held {
            let mine = self.held.entry(member).or_default();
            (mine.0, mine.1, mine.2) = (mine.0 + stated, mine.1 + held, mine.2 + elements);
        }
        for (class, signatures) in other.signatures {
            let mine = self.signatures.entry(class).or_default();
            for (members, signature) in signatures {
                let mine = mine.entry(members).or_default();
                for (format, (files, records)) in signature.by_format {
                    let mine = mine.by_format.entry(format).or_default();
                    (mine.0, mine.1) = (mine.0 + files, mine.1 + records);
                }
            }
        }
        for (mine, theirs) in self.stated.iter_mut().zip(other.stated) {
            for (hash, spelling) in theirs {
                mine.entry(hash).or_insert(spelling);
            }
        }
        for (mine, theirs) in self.records.iter_mut().zip(other.records) {
            (mine.0, mine.1) = (mine.0 + theirs.0, mine.1 + theirs.1);
        }
        for (mine, theirs) in [
            (&mut self.sized, other.sized),
            (&mut self.counted_runs, other.counted_runs),
            (&mut self.street_lights, other.street_lights),
            (&mut self.swapped, other.swapped),
        ] {
            (mine.0, mine.1) = (mine.0 + theirs.0, mine.1 + theirs.1);
        }
        self.lights.extend(other.lights);
        self.with_a_record += other.with_a_record;
        self.saved_as_read += other.saved_as_read;
        self.saved_fixed += other.saved_fixed;
        self.problems.extend(other.problems);
    }

    /// Pairs each map of far lights with the map it names as its parent, in its own folder, and saves the two together.
    pub fn pair_the_lights(&mut self) {
        let folder = |path: &str| {
            path.rsplit_once('/')
                .map_or("", |(folder, _)| folder)
                .to_string()
        };
        let mut parents: BTreeMap<(String, u32), Vec<usize>> = BTreeMap::new();
        for (at, map) in self.lights.iter().enumerate() {
            if map.distant > 0 {
                parents
                    .entry((folder(&map.path), map.name))
                    .or_default()
                    .push(at);
            }
            if map.lod > 0 && map.distant > 0 {
                let path = &map.path;
                self.problems
                    .push(format!("{path}: holds both families of far lights"));
            }
        }
        let mut named: BTreeMap<usize, usize> = BTreeMap::new();
        for map in self.lights.iter().filter(|map| map.lod > 0) {
            self.pairs += 1;
            let path = &map.path;
            let found = parents.get(&(folder(path), map.parent));
            let Some(&[parent]) = found.map(Vec::as_slice) else {
                let maps = found.map_or(0, Vec::len);
                self.problems.push(format!(
                    "{path}: {maps} maps of its folder are the parent it names and hold distant lights"
                ));
                continue;
            };
            *named.entry(parent).or_default() += 1;
            let parent = &self.lights[parent];
            if parent.distant != map.lod {
                self.problems.push(format!(
                    "{path}: holds {} far lights and its parent {} distant ones",
                    map.lod, parent.distant
                ));
                continue;
            }
            let saved = CMapData::from_bytes(&map.bytes).and_then(|lights| {
                let distant = CMapData::from_bytes(&parent.bytes)?;
                let (lights_saved, distant_saved) = save_far_lights(&lights, &distant)?;
                Ok(lights_saved == lights.to_bytes()? && distant_saved == distant.to_bytes()?)
            });
            match saved {
                Ok(true) => self.pairs_saved += 1,
                Ok(false) => self.problems.push(format!(
                    "{path}: saved with its parent, a map is not what it saves as alone"
                )),
                Err(e) => self
                    .problems
                    .push(format!("{path}: saves with its parent: {e}")),
            }
        }
        let distant = self.lights.iter().filter(|map| map.distant > 0).count();
        let shared = named.values().filter(|&&children| children > 1).count();
        if named.len() != distant || shared > 0 {
            self.problems.push(format!(
                "{distant} maps hold distant lights, {} are named as a parent and {shared} by more than one map",
                named.len()
            ));
        }
    }

    /// The members a record types that no record of the install states, and the members the install states that no record types.
    pub fn members_that_differ(&self) -> Vec<String> {
        let mut differ = Vec::new();
        for (class, (name, typed)) in RECORDS.iter().enumerate() {
            let keys: Vec<u32> = typed.iter().map(|member| key(member)).collect();
            for (member, key) in typed.iter().zip(&keys) {
                if !self.stated[class].contains_key(key) {
                    differ.push(format!("{name}::{member} is typed and never stated"));
                }
            }
            for (key, spelt) in &self.stated[class] {
                if !keys.contains(key) {
                    differ.push(format!("{name}::{spelt} is stated and not typed"));
                }
            }
        }
        differ
    }

    /// The classes a file that holds a record of them states with other members than the record types.
    pub fn signatures_that_differ(&self) -> Vec<String> {
        let mut differ = Vec::new();
        for (class, typed) in RECORDS {
            let mut typed: Vec<u32> = typed.iter().map(|member| key(member)).collect();
            typed.sort_unstable();
            for (members, signature) in self.signatures.get(class).into_iter().flatten() {
                let mut stated: Vec<u32> = (members.lines())
                    .filter_map(|line| line.trim().split_once(": "))
                    .map(|(member, _)| field::name_hash(member))
                    .collect();
                stated.sort_unstable();
                if signature.records() > 0 && stated != typed {
                    differ.push(format!(
                        "{class}: {} records are held by files that state it as\n        {members}",
                        signature.records()
                    ));
                }
            }
        }
        differ
    }

    pub fn report(&self) -> String {
        let [cars, boxes, models, modifiers, lights, distant, _] = self.records;
        format!(
            "{}: {} maps {:?}; {} parked vehicles in {} maps, {} box occluders in {}, {} occlude models in {}, \
             {} time cycle modifiers in {}, {} far lights in {} maps and their {} positions in {}; \
             {} pairs of maps of far lights, {} saved together; {} maps hold such a record, {} saved to their fixed \
             point, {} of them to the bytes they were read from; {} maps whose first two occlude models hold other \
             counts of bytes, {} read back with the two in each other's place; {} problems",
            self.label,
            self.maps,
            self.formats,
            cars.0,
            cars.1,
            boxes.0,
            boxes.1,
            models.0,
            models.1,
            modifiers.0,
            modifiers.1,
            lights.0,
            lights.1,
            distant.0,
            distant.1,
            self.pairs,
            self.pairs_saved,
            self.with_a_record,
            self.saved_fixed,
            self.saved_as_read,
            self.swapped.0,
            self.swapped.1,
            self.problems.len()
        )
    }

    /// The census in full: what a document writes down from.
    pub fn print(&self) {
        eprintln!("== {} ==", self.label);
        for (class, signatures) in &self.signatures {
            eprintln!("  {class} is stated {} way(s):", signatures.len());
            for (members, signature) in signatures {
                let stated: Vec<String> = (signature.by_format.iter())
                    .map(|(format, (files, records))| match *class {
                        "CMapData" => format!("{files} {format} files"),
                        _ => format!("{files} {format} files, which hold {records} of it"),
                    })
                    .collect();
                eprintln!("    by {}\n        {members}", stated.join("; by "));
            }
        }
        eprintln!("  every member a map states, under its root or one struct below:");
        for (member, (stated, held, elements)) in &self.held {
            eprintln!(
                "    {member}: stated by {stated} maps, {held} hold something under it, {elements} elements in all"
            );
        }
        eprintln!(
            "  occlude models whose dataSize is the count of the bytes behind verts: {} of {}",
            self.sized.0,
            self.sized.0 + self.sized.1
        );
        eprintln!(
            "  RBM tables that state verts as a run counted by dataSize and aligned to 16: {} of {}",
            self.counted_runs.0,
            self.counted_runs.0 + self.counted_runs.1
        );
        eprintln!(
            "  maps of distant lights whose numStreetLights is no more than their lights: {} of {}",
            self.street_lights.0,
            self.street_lights.0 + self.street_lights.1
        );
        let lod_only = self.lights.iter().filter(|m| m.lod > 0 && m.distant == 0);
        let distant_only = self.lights.iter().filter(|m| m.lod == 0 && m.distant > 0);
        eprintln!(
            "  maps that hold far lights: {}; {} hold LODLightsSOA alone and {} DistantLODLightsSOA alone",
            self.lights.len(),
            lod_only.count(),
            distant_only.count()
        );
        eprintln!(
            "  members typed and never stated by a record, or stated and not typed: {:?}",
            self.members_that_differ()
        );
        eprintln!(
            "  classes a file that holds a record of them states otherwise: {:?}",
            self.signatures_that_differ()
        );
        eprintln!("{}", self.report());
    }
}

/// How the records `typed` holds differ from the ones the read views of `or_world` give over `root`.
pub fn typed_differs(root: &MetaInstance, typed: &CMapData) -> Option<String> {
    let view = or_world::MapData::new(root)?;
    let at = |v: or_io::decl::Vec3| glam::Vec3::new(v.x, v.y, v.z);
    let point = |p: &FloatXYZ| glam::Vec3::new(p.x, p.y, p.z);
    let counts = [
        (view.car_generator_count(), typed.car_generators.len()),
        (view.box_occluder_count(), typed.box_occluders.len()),
        (view.occlude_model_count(), typed.occlude_models.len()),
        (
            view.time_cycle_modifier_count(),
            typed.time_cycle_modifiers.len(),
        ),
        (
            view.lod_lights().map_or(0, |lights| lights.len()),
            typed.lod_lights_soa.len(),
        ),
        (
            view.distant_lod_lights().map_or(0, |lights| lights.len()),
            typed.distant_lod_lights_soa.len(),
        ),
    ];
    if counts.iter().any(|(viewed, typed)| viewed != typed) {
        return Some(format!(
            "holds (in the tree, typed) {counts:?} parked vehicles, box occluders, occlude models, modifiers, far lights and distant lights"
        ));
    }
    let cars = (view.car_generators().zip(&typed.car_generators)).all(|(car, typed)| {
        car.position() == Some(at(typed.position))
            && car.orient() == (typed.orient_x, typed.orient_y)
            && car.perpendicular_length() == typed.perpendicular_length
            && car.car_model() == typed.car_model.hash
            && car.flags() == typed.flags
            && car.pop_group() == typed.pop_group.hash
            && car.livery() == i32::from(typed.livery)
            && car.body_color_remaps()
                == [
                    typed.body_color_remap1,
                    typed.body_color_remap2,
                    typed.body_color_remap3,
                    typed.body_color_remap4,
                ]
    });
    let quarter = |v: i16| f32::from(v) * 0.25;
    let boxes = (view.box_occluders().zip(&typed.box_occluders)).all(|(viewed, typed)| {
        let centre = (typed.i_center_x, typed.i_center_y, typed.i_center_z);
        let size = (typed.i_length, typed.i_width, typed.i_height);
        viewed.centre == glam::Vec3::new(quarter(centre.0), quarter(centre.1), quarter(centre.2))
            && viewed.size == glam::Vec3::new(quarter(size.0), quarter(size.1), quarter(size.2))
    });
    let models = (view.occlude_model_bounds().zip(&typed.occlude_models))
        .all(|(bounds, typed)| bounds == (at(typed.bmin), at(typed.bmax)));
    let modifiers =
        (view.time_cycle_modifiers().zip(&typed.time_cycle_modifiers)).all(|(modifier, typed)| {
            modifier.name() == typed.name.hash
                && modifier.extents() == Some((at(typed.min_extents), at(typed.max_extents)))
                && modifier.percentage() == typed.percentage
                && modifier.range() == typed.range
                && modifier.hours() == (typed.start_hour, typed.end_hour)
        });
    let family = &typed.lod_lights_soa;
    let lights = view.lod_lights().is_none_or(|lights| {
        lights.iter().count() == family.len()
            && lights.iter().enumerate().all(|(i, light)| {
                light.direction == point(&family.direction[i])
                    && light.falloff == family.falloff[i]
                    && light.falloff_exponent == family.falloff_exponent[i]
                    && light.time_and_state_flags == family.time_and_state_flags[i]
                    && light.hash == family.hash[i]
                    && light.cone_inner_angle == family.cone_inner_angle[i]
                    && light.cone_outer_angle_or_cap_ext == family.cone_outer_angle_or_cap_ext[i]
                    && light.corona_intensity == family.corona_intensity[i]
            })
    });
    let family = &typed.distant_lod_lights_soa;
    let distant = view.distant_lod_lights().is_none_or(|lights| {
        lights.iter().count() == family.len()
            && lights.num_street_lights() == u32::from(family.num_street_lights)
            && lights.category() == u32::from(family.category)
            && lights.iter().enumerate().all(|(i, light)| {
                light.position == point(&family.position[i]) && light.rgbi == family.rgbi[i]
            })
    });
    let kinds = [
        (cars, "parked vehicles"),
        (boxes, "box occluders"),
        (models, "occlude models"),
        (modifiers, "time cycle modifiers"),
        (lights, "far lights"),
        (distant, "distant lights"),
    ];
    let differ: Vec<&str> = (kinds.iter())
        .filter_map(|(same, kind)| (!same).then_some(*kind))
        .collect();
    (!differ.is_empty()).then(|| {
        format!(
            "its {} read otherwise typed than in the tree",
            differ.join(", ")
        )
    })
}

pub fn light_map(path: &str, name: &str, parent: &str, lod: usize, distant: usize) -> LightMap {
    LightMap {
        path: path.to_string(),
        name: key(name),
        parent: key(parent),
        lod,
        distant,
        bytes: Vec::new(),
    }
}

pub fn census_of_lights(lights: Vec<LightMap>) -> Census {
    Census {
        lights,
        ..Census::default()
    }
}
