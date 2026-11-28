//! The layered materials of the install a GTA VI drawable is in, and the material each weapon
//! part wears: the metadata bodies that state them, found in the scans of the install's archives
//! and read once a session on a worker. Which bodies they are is kept; what they hold is not.
//! An archive not scanned before is scanned for it, which takes minutes the first time.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use or_io::container::rpf::{
    rpf9_scan::Rpf9BodyReader, seal_rpf9_scan, unseal_rpf9_scan, Rpf9BodyKind, Rpf9Content,
};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::texture_names::stem;
use or_io::hash::string_hash_str;
use or_io::meta::{MetaInstance, Value};

use crate::render::gta6_layers::{Definition, Layered, Material, Row, Rows, TextureRow, LAYERS};
use crate::state::gta6_texture_index::{put, stat, stated};
use crate::state::rpf9_kept::word;
use crate::state::rpf9_scan::{Ended, Row as ScanRow, Rpf9ScanDocument};

/// The version of what this file keeps of an index: a change to it takes another.
const VERSION: u64 = 1;
/// The most a kept index inflates to.
const MOST_BYTES: usize = 1 << 24;
/// The largest metadata body read.
const BODY_LIMIT: u64 = 64 << 20;
// ponytail: an archive past this is not scanned for the bodies, so an install that keeps them in
// one is not served; the archives that hold them here are a sixteenth of it. Scan every archive
// when one that large is found to hold a materials file.
const MOST_ARCHIVE: u64 = 8 << 30;
/// How often the worker looks at the scan of the archive in hand.
const LOOK: Duration = Duration::from_millis(50);

// The members read, as the bodies' own schemas name them: hashes whose names are not known.
/// The materials file's maps after `Tiles`: layer definitions, then materials.
const DEFINITIONS: u32 = 0x42FA_4D39;
const MATERIALS: u32 = 0xFD54_0657;
/// A layer definition's tile.
const TILE: u32 = 0x03E5_2760;
/// A material's flags, its layers' definitions and the tiles it states in place of theirs.
const FLAGS: u32 = 0xC897_AF02;
const LAYER_NAMES: u32 = 0x4A1E_BC36;
const TILE_NAMES: u32 = 0x4BEE_7340;
/// The four lists of rows, and the layer of a row.
const FLOATS: u32 = 0x1652_FCE8;
const COLOURS: u32 = 0xFC24_D9C5;
const WORDS: u32 = 0x9F6B_5B03;
const TEXTURES: u32 = 0x0F20_8402;
const LAYER: u32 = 0xAADE_B53E;
/// A weapon part's model, by the hash of its name, and its material's key.
const PART_MODEL: u32 = 0x873E_F899;
const PART_MATERIAL: u32 = 0x72F5_E33E;

/// What the worker found: the materials file's maps, and each weapon part's material by the
/// hash of the part's model name.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Index {
    pub layered: Layered,
    pub parts: BTreeMap<u32, u32>,
}

impl Index {
    /// The material the weapon part named `name` wears, at whatever level of detail.
    pub(crate) fn material_of(&self, name: &str) -> Option<&Material> {
        let name = stem(name);
        let base = ["_hi", "_mid"].iter().find_map(|l| name.strip_suffix(l));
        let mut key = [Some(name.as_str()), base].into_iter().flatten();
        let key = key.find_map(|name| self.parts.get(&string_hash_str(name)))?;
        self.layered.materials.get(key)
    }
}

/// The member of `instance` its schema names by `hash`.
fn member(instance: &MetaInstance, hash: u32) -> Option<&Value> {
    instance.get(&format!("hash_{hash:08X}"))
}

/// A value of at most eight bytes as the file stores it: a float by its bits.
fn bits(value: &Value) -> Option<u64> {
    Some(match value {
        Value::UInt(v) => *v,
        Value::Int(v) => *v as u64,
        Value::Hash(v) => u64::from(v.hash),
        Value::Color(v) => u64::from(*v),
        Value::Float(v) => u64::from((*v as f32).to_bits()),
        Value::Bitset { bits, .. } => *bits as u64,
        Value::Bool(v) => u64::from(*v),
        _ => return None,
    })
}

fn structure(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(instance) | Value::Pointer(Some(instance)) => Some(instance),
        _ => None,
    }
}

fn items(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

/// The rows of the list `list` of `of`: `(name, value, index, layer)`, a row that does not read
/// left out.
fn list(of: &MetaInstance, list: u32) -> impl Iterator<Item = (u32, u64, u8, u8)> + '_ {
    items(member(of, list)).iter().filter_map(|item| {
        let row = structure(item)?;
        let number = |value: Option<&Value>| value.and_then(bits);
        Some((
            number(row.get("Name"))? as u32,
            number(row.get("Value"))?,
            number(row.get("Index"))? as u8,
            number(member(row, LAYER))? as u8,
        ))
    })
}

fn rows_of(of: &MetaInstance) -> Rows {
    let short = |name| {
        list(of, name)
            .map(|(name, value, index, layer)| Row {
                name,
                value: value as u32,
                index,
                layer,
            })
            .collect()
    };
    Rows {
        floats: short(FLOATS),
        colours: short(COLOURS),
        words: short(WORDS),
        textures: list(of, TEXTURES)
            .map(|(name, key, index, layer)| TextureRow {
                name,
                key,
                index,
                layer,
            })
            .collect(),
    }
}

/// Each entry of the map `value` is: its key and the structure it holds.
fn entries(value: Option<&Value>) -> impl Iterator<Item = (u32, &MetaInstance)> {
    let pairs = match value {
        Some(Value::Map(pairs)) => pairs.as_slice(),
        _ => &[],
    };
    (pairs.iter()).filter_map(|(key, value)| Some((bits(key)? as u32, structure(value)?)))
}

/// What the materials file `root` holds, added to `out`; a key `out` holds stays as it is.
pub(crate) fn add_layered(root: &MetaInstance, out: &mut Layered) {
    for (key, tile) in entries(root.get("Tiles")) {
        out.tiles.entry(key).or_insert_with(|| rows_of(tile));
    }
    for (key, definition) in entries(member(root, DEFINITIONS)) {
        out.definitions.entry(key).or_insert_with(|| Definition {
            tile: member(definition, TILE).and_then(bits).unwrap_or(0) as u32,
            rows: rows_of(definition),
        });
    }
    let names = |of: &MetaInstance, name| {
        let mut names = [0; LAYERS];
        for (slot, value) in names.iter_mut().zip(items(member(of, name))) {
            *slot = bits(value).unwrap_or(0) as u32;
        }
        names
    };
    for (key, material) in entries(member(root, MATERIALS)) {
        out.materials.entry(key).or_insert_with(|| Material {
            flags: member(material, FLAGS).and_then(bits).unwrap_or(0) as u32,
            layers: names(material, LAYER_NAMES),
            tiles: names(material, TILE_NAMES),
            rows: rows_of(material),
        });
    }
}

/// The material of each part the weapon definition `root` states, added to `out`.
pub(crate) fn add_parts(root: &MetaInstance, out: &mut BTreeMap<u32, u32>) {
    for item in items(root.get("Data")).iter().filter_map(structure) {
        let hash = |name| member(item, name).and_then(bits).filter(|hash| *hash != 0);
        if let (Some(model), Some(material)) = (hash(PART_MODEL), hash(PART_MATERIAL)) {
            out.entry(model as u32).or_insert(material as u32);
        }
    }
}

/// What a body is to the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Materials = 0,
    Weapon = 1,
}

/// What `bytes`, a metadata body with its schema, is: its schema names the members of a
/// materials file or of a weapon part.
fn kind_of(bytes: &[u8]) -> Option<Kind> {
    let holds = |hash: u32| bytes.windows(4).any(|window| window == hash.to_le_bytes());
    if holds(MATERIALS) && holds(DEFINITIONS) {
        Some(Kind::Materials)
    } else if holds(PART_MODEL) && holds(PART_MATERIAL) {
        Some(Kind::Weapon)
    } else {
        None
    }
}

/// The bodies of one archive: each one's offset, kind and inflated length.
type Bodies = Vec<(u64, Kind, u64)>;
/// An archive of a kept index: its size and modification time when read, and its bodies.
type KeptArchive = ((u64, u64), Bodies);

/// Where the bodies found under one folder are kept in the configuration folder, each archive by
/// its path under the folder with its size and modification time: an archive is read again only
/// once either is not what is kept.
struct Kept {
    file: PathBuf,
    of: Vec<u8>,
}

impl Kept {
    fn of(root: &Path) -> Option<Self> {
        let path = root.to_string_lossy();
        let mut of = VERSION.to_le_bytes().to_vec();
        put(&mut of, path.as_bytes());
        let folder = crate::state::prefs::config_root()?.join("gta6_layer_indexes");
        let file = folder.join(format!("{:016x}.index", name_hash64(path.as_bytes())));
        Some(Self { file, of })
    }

    fn read(&self, root: &Path) -> io::Result<HashMap<PathBuf, KeptArchive>> {
        let plain = unseal_rpf9_scan(&std::fs::read(&self.file)?, MOST_BYTES)?;
        let other = || io::Error::new(io::ErrorKind::InvalidData, "a kept index of another kind");
        let bytes = &mut plain.strip_prefix(self.of.as_slice()).ok_or_else(other)?;
        let mut archives = HashMap::new();
        while !bytes.is_empty() {
            let path = root.join(&*String::from_utf8_lossy(stated(bytes)?));
            let stat = (word(bytes)?, word(bytes)?);
            let mut bodies = Bodies::new();
            for _ in 0..word(bytes)? {
                let (offset, kind, len) = (word(bytes)?, word(bytes)?, word(bytes)?);
                let kind = match kind {
                    0 => Kind::Materials,
                    1 => Kind::Weapon,
                    _ => return Err(other()),
                };
                bodies.push((offset, kind, len));
            }
            archives.insert(path, (stat, bodies));
        }
        Ok(archives)
    }

    fn keep(&self, plain: &[u8]) -> io::Result<()> {
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&self.file, seal_rpf9_scan(plain)?)
    }
}

/// The bodies of the archive at `path` the index reads, from its scan: the one kept of it, else
/// one made now and kept. `None` where the scan does not read the archive whole.
fn bodies_in(path: &Path) -> Option<Bodies> {
    let scan = Rpf9ScanDocument::open(path).ok()??;
    let offsets: Vec<u64> = loop {
        let found = scan.found();
        match &found.ended {
            Some(Ended::Whole) => break (found.rows.iter())
                .filter_map(|row| match row {
                    ScanRow::Body(find) => Some(find),
                    ScanRow::Missing { .. } => None,
                })
                .filter(|find| {
                    matches!(find.kind, Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::Pso)
                        && find.inflated_len.is_none_or(|len| len <= BODY_LIMIT)
                })
                .map(|find| find.offset)
                .collect(),
            Some(_) => return None,
            None => {}
        }
        drop(found);
        std::thread::sleep(LOOK);
    };
    let mut reader = Rpf9BodyReader::new(File::open(path).ok()?).ok()?;
    let read = |offset| {
        let (_, bytes) = reader.read(offset, BODY_LIMIT).ok()?;
        Some((offset, kind_of(&bytes)?, bytes.len() as u64))
    };
    Some(offsets.into_iter().filter_map(read).collect())
}

/// The index of the install or folder at `root`, read from its archives.
fn build(root: &Path, kept: Option<&Kept>) -> Index {
    let archives = or_title::install::gta6_files::list(root).archive_paths();
    let mut was = (kept.and_then(|kept| kept.read(root).ok())).unwrap_or_default();
    let mut now = kept.map(|kept| kept.of.clone()).unwrap_or_default();
    let mut all = Vec::new();
    for path in archives {
        let Some(stat) = stat(&path).filter(|stat| stat.0 <= MOST_ARCHIVE) else {
            continue;
        };
        let unchanged = was.remove(&path).filter(|kept| kept.0 == stat);
        let Some(bodies) = unchanged.map(|kept| kept.1).or_else(|| bodies_in(&path)) else {
            continue;
        };
        let under = path.strip_prefix(root).unwrap_or(&path);
        put(&mut now, under.to_string_lossy().as_bytes());
        now.extend(
            [stat.0, stat.1, bodies.len() as u64]
                .map(u64::to_le_bytes)
                .as_flattened(),
        );
        for (offset, kind, len) in &bodies {
            now.extend(
                [*offset, *kind as u64, *len]
                    .map(u64::to_le_bytes)
                    .as_flattened(),
            );
        }
        all.extend(bodies.into_iter().map(|body| (path.clone(), body)));
    }
    if let Some(Err(error)) = kept.map(|kept| kept.keep(&now)) {
        tracing::warn!(%error, "the index of the layered materials was not kept");
    }
    let schema = crate::state::document::meta_schema();
    let read = |path: &Path, offset: u64| {
        let (_, bytes) = Rpf9BodyReader::new(File::open(path).ok()?)
            .ok()?
            .read(offset, BODY_LIMIT)
            .ok()?;
        or_io::meta::binary::parse_meta_binary(schema, &bytes).ok()
    };
    let mut index = Index::default();
    let of = |kind| all.iter().filter(move |(_, body)| body.1 == kind);
    for (path, (offset, ..)) in of(Kind::Weapon) {
        if let Some(root) = read(path, *offset) {
            add_parts(&root, &mut index.parts);
        }
    }
    // ponytail: where two materials files state one key the first read keeps it, in the order
    // the archives and their bodies are found; which the game keeps is not read.
    for (path, (offset, ..)) in of(Kind::Materials) {
        if let Some(root) = read(path, *offset) {
            add_layered(&root, &mut index.layered);
        }
    }
    tracing::info!(
        parts = index.parts.len(),
        materials = index.layered.materials.len(),
        "read the layered materials of {}",
        root.display()
    );
    index
}

/// The index that serves the active GTA VI drawable and the material it wears, where the
/// index is read; `start` has a worker read one not asked for before.
pub(crate) fn worn(state: &crate::state::AppState, start: bool) -> Option<(Arc<Index>, Material)> {
    let doc = state.active_document()?;
    let crate::state::DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let selection = state.selection.selected(doc.id);
    let drawable = crate::state::gta6_resource::drawable_of(resource, selection)?;
    let root = crate::state::gta6_texture_index::root_of(state, doc)?;
    let index = match start {
        true => under(&root)?,
        false => held(&root)?,
    };
    let material = index.material_of(&drawable.name)?.clone();
    Some((index, material))
}

/// The keys of the textures the layers of the active GTA VI drawable's material name, once the
/// index that serves it is read; none for a drawable that wears no layered material.
pub(in crate::state) fn tile_keys(state: &crate::state::AppState) -> Vec<u64> {
    let Some((index, material)) = worn(state, false) else {
        return Vec::new();
    };
    let layers = (0..LAYERS).flat_map(|layer| index.layered.textures(&material, layer));
    layers.filter(|key| *key != 0).collect()
}

type Slot = Arc<Mutex<Option<Arc<Index>>>>;

static BUILT: Mutex<BTreeMap<PathBuf, Slot>> = Mutex::new(BTreeMap::new());

/// The index of the install or folder at `root` where one is read.
fn held(root: &Path) -> Option<Arc<Index>> {
    let built = BUILT.lock().unwrap_or_else(|p| p.into_inner());
    let slot = Arc::clone(built.get(root)?);
    let index = slot.lock().unwrap_or_else(|p| p.into_inner()).clone();
    index
}

/// The index of the install or folder at `root`: `None` while a worker reads it, which the first
/// ask starts.
// ⚠ Where the index is kept is found here, on the caller's thread: a test's configuration folder
// is its own thread's.
pub(crate) fn under(root: &Path) -> Option<Arc<Index>> {
    let mut built = BUILT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(slot) = built.get(root) {
        return slot.lock().unwrap_or_else(|p| p.into_inner()).clone();
    }
    let slot = Slot::default();
    built.insert(root.to_path_buf(), Arc::clone(&slot));
    let (root, kept) = (root.to_path_buf(), Kept::of(root));
    std::thread::spawn(move || {
        let index = Arc::new(build(&root, kept.as_ref()));
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(index);
    });
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::HashedString;

    fn hash(hash: u32) -> Value {
        Value::Hash(HashedString::from_hash(hash))
    }

    fn named(hash: u32) -> String {
        format!("hash_{hash:08X}")
    }

    fn row(name: u32, value: Value, index: u64, layer: u64) -> Value {
        let mut row = MetaInstance::new("row");
        row.set("Name", hash(name));
        row.set("Value", value);
        row.set("Index", Value::UInt(index));
        row.set(named(LAYER), Value::UInt(layer));
        Value::Struct(Box::new(row))
    }

    fn map(entries: Vec<(u32, MetaInstance)>) -> Value {
        let pair = |(key, value)| (hash(key), Value::Struct(Box::new(value)));
        Value::Map(entries.into_iter().map(pair).collect())
    }

    #[test]
    fn a_materials_file_reads_as_its_tiles_definitions_and_materials() {
        let mut tile = MetaInstance::new("tile");
        tile.set(
            named(TEXTURES),
            Value::Array(vec![row(5, Value::UInt(u64::MAX - 1), 255, 255)]),
        );
        let mut definition = MetaInstance::new("definition");
        definition.set(named(TILE), hash(0x11));
        definition.set(
            named(FLOATS),
            Value::Array(vec![row(6, Value::Float(0.25), 1, 255)]),
        );
        let mut material = MetaInstance::new("material");
        material.set(named(FLAGS), Value::UInt(0x820));
        material.set(named(LAYER_NAMES), Value::Array(vec![hash(0x22), hash(0)]));
        material.set(named(TILE_NAMES), Value::Array(vec![hash(0), hash(0x12)]));
        material.set(
            named(COLOURS),
            Value::Array(vec![row(7, Value::UInt(0xFF28_2725), 0, 4)]),
        );
        let mut root = MetaInstance::new("root");
        root.set("Tiles", map(vec![(0x11, tile)]));
        root.set(named(DEFINITIONS), map(vec![(0x22, definition)]));
        root.set(named(MATERIALS), map(vec![(0x33, material)]));
        let mut layered = Layered::default();
        add_layered(&root, &mut layered);
        assert_eq!(layered.tiles[&0x11].textures[0].key, u64::MAX - 1);
        let definition = &layered.definitions[&0x22];
        assert_eq!(definition.tile, 0x11);
        let float = definition.rows.floats[0];
        assert_eq!((float.value, float.index), (0.25f32.to_bits(), 1));
        let material = &layered.materials[&0x33];
        assert_eq!((material.flags, material.layers[0]), (0x820, 0x22));
        assert_eq!(material.tiles[..2], [0, 0x12]);
        let colour = material.rows.colours[0];
        assert_eq!(
            (colour.name, colour.value, colour.layer),
            (7, 0xFF28_2725, 4)
        );
    }

    #[test]
    fn a_weapon_part_wears_the_material_its_definition_states_at_every_level_of_detail() {
        let part = |model: u32, material: u32| {
            let mut part = MetaInstance::new("part");
            part.set(named(PART_MODEL), hash(model));
            part.set(named(PART_MATERIAL), hash(material));
            Value::Pointer(Some(Box::new(part)))
        };
        let mut root = MetaInstance::new("root");
        let other = Value::Pointer(Some(Box::new(MetaInstance::new("other"))));
        let muzzle = string_hash_str("w_test_muzzle_01");
        root.set(
            "Data",
            Value::Array(vec![other, part(muzzle, 0x33), part(9, 0)]),
        );
        let mut index = Index::default();
        add_parts(&root, &mut index.parts);
        assert_eq!(index.parts, BTreeMap::from([(muzzle, 0x33)]));
        index.layered.materials.insert(0x33, Material::default());
        assert!(index.material_of("w_test_muzzle_01_hi.#dr").is_some());
        assert!(index.material_of("W_Test_Muzzle_01").is_some());
        assert!(index.material_of("w_test_muzzle_02").is_none());
    }

    #[test]
    fn a_body_is_told_by_the_members_its_schema_names() {
        let with = |hashes: &[u32]| -> Vec<u8> {
            let words = hashes.iter().flat_map(|hash| hash.to_le_bytes());
            [1u8, 2, 3].into_iter().chain(words).collect()
        };
        assert_eq!(
            kind_of(&with(&[MATERIALS, DEFINITIONS])),
            Some(Kind::Materials)
        );
        assert_eq!(
            kind_of(&with(&[PART_MATERIAL, 7, PART_MODEL])),
            Some(Kind::Weapon)
        );
        assert_eq!(kind_of(&with(&[PART_MATERIAL, MATERIALS])), None);
    }
}
