//! Translations of GTA VI's native programs kept by content, in memory and on disk, so a program
//! is lowered once: [`Cache::get_or_translate`] is the whole of what a caller needs.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::rdna::{
    self, Index, IndexSource, Intersection, Loc, Lowered, Options, Program, RdnaError, Registers,
    Resource, ResourceKind,
};

/// The stored form's version: a change to [`Stored`] takes a new number.
const FORMAT: u32 = 3;
/// A hash of the native front end's sources, so a build never reads another build's lowering.
pub const TRANSLATOR: &str = env!("OR_RDNA_FINGERPRINT");

/// What a translation is kept under: the program's content, the options its lowering was given
/// and the translator that lowered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key([u8; 20]);

impl Key {
    pub fn of(program: &Program, options: &Options) -> Key {
        Self::with_layout(program, options, &rdna::arena_layout())
    }

    /// [`Key::of`] with the arena's layout a lowering is written from stated: the test of a
    /// node reads its offsets from another crate, which [`TRANSLATOR`] does not name.
    fn with_layout(program: &Program, options: &Options, layout: &[u32]) -> Key {
        // Destructured in full, so a field added to a program cannot be left out of its key.
        let Program {
            kind,
            slot,
            words,
            wave32,
            inputs,
            vertex_inputs,
            position_read,
            front_face,
            launch,
        } = program;
        let mut hash = Sha1::new();
        hash.update(format!(
            "rdna {FORMAT} {TRANSLATOR} {kind:?} {slot} {wave32} {inputs:?} {vertex_inputs:?} \
             {position_read} {front_face} {launch:?} {options:?} {layout:?} "
        ));
        for word in words {
            hash.update(word.to_le_bytes());
        }
        Key(hash.finalize().into())
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// What the cache holds of a program: its lowering, or the refusal, which is as final.
#[derive(Debug, Clone)]
pub enum Cached {
    Translated(Box<Lowered>),
    /// `name` is the census word of the refusal and `message` the whole of it.
    Refused {
        name: String,
        message: String,
    },
}

impl Cached {
    fn of(result: Result<Lowered, RdnaError>) -> Cached {
        match result {
            Ok(lowered) => Cached::Translated(Box::new(lowered)),
            Err(why) => Cached::Refused {
                name: match &why {
                    RdnaError::NotTranslated(why) => why.name(),
                    RdnaError::Cfg(_) => "graph".to_owned(),
                    RdnaError::Container(_) => "container".to_owned(),
                },
                message: why.to_string(),
            },
        }
    }
}

/// Translations by [`Key`]: a map in memory, and with a directory a file per key under it.
#[derive(Debug, Default)]
pub struct Cache {
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<Key, Arc<Cached>>>,
}

impl Cache {
    /// A cache that lasts as long as the value.
    pub fn in_memory() -> Cache {
        Cache::default()
    }

    /// A cache that also reads and writes `dir`, which is made when the first entry is written.
    pub fn on_disk(dir: impl Into<PathBuf>) -> Cache {
        Cache {
            dir: Some(dir.into()),
            memory: Mutex::default(),
        }
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Arc<Cached>>> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, key: &Key) -> Option<PathBuf> {
        let hex = key.hex();
        Some(self.dir.as_ref()?.join(&hex[..2]).join(&hex[2..]))
    }

    /// The entry under `key`, from memory and else from disk; translates nothing.
    pub fn get(&self, key: &Key) -> Option<Arc<Cached>> {
        if let Some(found) = self.memory().get(key) {
            return Some(found.clone());
        }
        // A file that does not decode is one a crash cut short or another format wrote: a miss.
        let found = Arc::new(decode(&std::fs::read(self.path(key)?).ok()?)?);
        self.memory().insert(*key, found.clone());
        Some(found)
    }

    /// Keeps `cached` under `key`; a directory that cannot be written leaves it in memory only.
    pub fn put(&self, key: Key, cached: Cached) -> Arc<Cached> {
        if let Some(path) = self.path(&key) {
            let _ = write_whole(&path, &encode(&cached));
        }
        let cached = Arc::new(cached);
        self.memory().insert(key, cached.clone());
        cached
    }

    /// `program`'s translation under `options`: the kept one, else a new one, kept. The flag says
    /// whether it was kept already. A panic of the translator passes through and keeps nothing.
    pub fn get_or_translate(&self, program: &Program, options: &Options) -> (Arc<Cached>, bool) {
        let key = Key::of(program, options);
        match self.get(&key) {
            Some(found) => (found, true),
            None => {
                let cached = Cached::of(rdna::translate_with(program, options));
                (self.put(key, cached), false)
            }
        }
    }
}

/// Written beside its place and moved there, so a reader never sees half a file.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)
}

// Lives for one write or one read, so the size of its larger variant costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize)]
enum Stored<'a> {
    Translated {
        module: Cow<'a, naga::Module>,
        resources: Vec<StoredResource>,
        approximations: Vec<(String, u32)>,
        half_slots: Vec<u8>,
        subgroups: bool,
        /// Each intersection's word, form, function, depth and loops.
        intersections: Vec<(u32, bool, String, u32, Vec<u32>)>,
        /// The registers a walk lowered alone is given, and those it leaves.
        region: Option<(Vec<String>, Vec<String>)>,
    },
    Refused {
        name: String,
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
struct StoredResource {
    kind: StoredKind,
    at: Option<StoredLoc>,
    words: Option<[u32; 4]>,
    group: u32,
    binding: u32,
    count: Option<u32>,
    written: bool,
}

#[derive(Serialize, Deserialize)]
enum StoredKind {
    UserData,
    ConstantBuffer {
        rows: u32,
    },
    Texture {
        dim: u8,
        compared: bool,
    },
    Sampler {
        comparison: bool,
    },
    Buffer {
        stride: Option<u32>,
    },
    TableWords,
    StorageTexture {
        dim: u8,
        format: naga::StorageFormat,
    },
    Arena,
    Registers,
}

#[derive(Serialize, Deserialize)]
struct StoredLoc {
    root: u8,
    path: Vec<u32>,
    offset: u32,
    index: Option<(StoredSource, u8)>,
}

#[derive(Serialize, Deserialize)]
enum StoredSource {
    User(u8),
    Data {
        buffer: Box<StoredLoc>,
        offset: u32,
        plus: Option<u8>,
    },
    Runtime {
        site: u32,
    },
}

fn store_loc(loc: &Loc) -> StoredLoc {
    StoredLoc {
        root: loc.root,
        path: loc.path.clone(),
        offset: loc.offset,
        index: loc.index.as_ref().map(|index| {
            let source = match &index.source {
                IndexSource::User(register) => StoredSource::User(*register),
                IndexSource::Data {
                    buffer,
                    offset,
                    plus,
                } => StoredSource::Data {
                    buffer: Box::new(store_loc(buffer)),
                    offset: *offset,
                    plus: *plus,
                },
                IndexSource::Runtime { site } => StoredSource::Runtime { site: *site },
            };
            (source, index.shift)
        }),
    }
}

fn load_loc(loc: StoredLoc) -> Loc {
    Loc {
        root: loc.root,
        path: loc.path,
        offset: loc.offset,
        index: loc.index.map(|(source, shift)| Index {
            source: match source {
                StoredSource::User(register) => IndexSource::User(register),
                StoredSource::Data {
                    buffer,
                    offset,
                    plus,
                } => IndexSource::Data {
                    buffer: Box::new(load_loc(*buffer)),
                    offset,
                    plus,
                },
                StoredSource::Runtime { site } => IndexSource::Runtime { site },
            },
            shift,
        }),
    }
}

fn store_resource(resource: &Resource) -> StoredResource {
    use ResourceKind as K;
    StoredResource {
        kind: match resource.kind {
            K::UserData => StoredKind::UserData,
            K::ConstantBuffer { rows } => StoredKind::ConstantBuffer { rows },
            K::Texture { dim, compared } => StoredKind::Texture { dim, compared },
            K::Sampler { comparison } => StoredKind::Sampler { comparison },
            K::Buffer { stride } => StoredKind::Buffer { stride },
            K::TableWords => StoredKind::TableWords,
            K::StorageTexture { dim, format } => StoredKind::StorageTexture { dim, format },
            K::Arena => StoredKind::Arena,
            K::Registers => StoredKind::Registers,
        },
        at: resource.at.as_ref().map(store_loc),
        words: resource.words,
        group: resource.group,
        binding: resource.binding,
        count: resource.count,
        written: resource.written,
    }
}

fn load_resource(resource: StoredResource) -> Resource {
    use StoredKind as K;
    Resource {
        kind: match resource.kind {
            K::UserData => ResourceKind::UserData,
            K::ConstantBuffer { rows } => ResourceKind::ConstantBuffer { rows },
            K::Texture { dim, compared } => ResourceKind::Texture { dim, compared },
            K::Sampler { comparison } => ResourceKind::Sampler { comparison },
            K::Buffer { stride } => ResourceKind::Buffer { stride },
            K::TableWords => ResourceKind::TableWords,
            K::StorageTexture { dim, format } => ResourceKind::StorageTexture { dim, format },
            K::Arena => ResourceKind::Arena,
            K::Registers => ResourceKind::Registers,
        },
        at: resource.at.map(load_loc),
        words: resource.words,
        group: resource.group,
        binding: resource.binding,
        count: resource.count,
        written: resource.written,
    }
}

/// An approximation's name for as long as the process runs: a lowering names them with literals,
/// so a loaded one needs each distinct name kept once.
fn interned(name: String) -> &'static str {
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    let names = names.get_or_insert_with(HashSet::new);
    match names.get(name.as_str()) {
        Some(kept) => kept,
        None => {
            let kept: &'static str = Box::leak(name.into_boxed_str());
            names.insert(kept);
            kept
        }
    }
}

/// `cached` as the bytes a cache file holds: the stored form, deflated.
pub fn encode(cached: &Cached) -> Vec<u8> {
    let stored = match cached {
        Cached::Translated(lowered) => Stored::Translated {
            module: Cow::Borrowed(&lowered.module),
            resources: lowered.resources.iter().map(store_resource).collect(),
            approximations: (lowered.approximations.iter())
                .map(|(name, count)| ((*name).to_owned(), *count))
                .collect(),
            half_slots: lowered.half_slots.iter().copied().collect(),
            subgroups: lowered.subgroups,
            intersections: (lowered.intersections.iter().cloned())
                .map(|at| (at.word, at.wide, at.function, at.depth, at.loops))
                .collect(),
            region: (lowered.region.clone()).map(|registers| (registers.given, registers.left)),
        },
        Cached::Refused { name, message } => Stored::Refused {
            name: name.clone(),
            message: message.clone(),
        },
    };
    let mut out = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    // Neither a module's serialisation nor a write to memory fails.
    let _ = bincode::serialize(&(FORMAT, stored)).map(|bytes| out.write_all(&bytes));
    out.finish().unwrap_or_default()
}

/// What [`encode`] wrote, or `None` for bytes of another format or cut short.
pub fn decode(bytes: &[u8]) -> Option<Cached> {
    let mut plain = Vec::new();
    (flate2::read::DeflateDecoder::new(bytes).read_to_end(&mut plain)).ok()?;
    let (format, stored): (u32, Stored<'static>) = bincode::deserialize(&plain).ok()?;
    if format != FORMAT {
        return None;
    }
    Some(match stored {
        Stored::Translated {
            module,
            resources,
            approximations,
            half_slots,
            subgroups,
            intersections,
            region,
        } => Cached::Translated(Box::new(Lowered {
            module: module.into_owned(),
            resources: resources.into_iter().map(load_resource).collect(),
            approximations: (approximations.into_iter())
                .map(|(name, count)| (interned(name), count))
                .collect::<BTreeMap<_, _>>(),
            half_slots: half_slots.into_iter().collect(),
            subgroups,
            intersections: (intersections.into_iter())
                .map(|(word, wide, function, depth, loops)| Intersection {
                    word,
                    wide,
                    function,
                    depth,
                    loops,
                })
                .collect(),
            region: region.map(|(given, left)| Registers { given, left }),
        })),
        Stored::Refused { name, message } => Cached::Refused { name, message },
    })
}

#[cfg(test)]
#[path = "../fingerprint.rs"]
mod fingerprint;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxil::ShaderKind;

    const S_ENDPGM: u32 = 0xBF81_0000;

    fn program(kind: ShaderKind, words: Vec<u32>) -> Program {
        Program {
            kind,
            slot: 0,
            words,
            wave32: false,
            inputs: Vec::new(),
            vertex_inputs: Vec::new(),
            position_read: 0,
            front_face: false,
            launch: None,
        }
    }

    fn text(cached: &Cached) -> String {
        format!("{cached:?}")
    }

    #[test]
    fn a_key_follows_the_program_and_the_options() {
        let pixel = program(ShaderKind::Pixel, vec![S_ENDPGM]);
        let key = Key::of(&pixel, &Options::default());
        assert_eq!(key, Key::of(&pixel.clone(), &Options::default()));
        assert_eq!(key.hex().len(), 40);
        let mut other = pixel.clone();
        other.wave32 = true;
        assert_ne!(key, Key::of(&other, &Options::default()));
        assert_ne!(
            key,
            Key::of(
                &program(ShaderKind::Pixel, vec![S_ENDPGM, S_ENDPGM]),
                &Options::default()
            )
        );
        let halves = Options {
            half_slots: [3].into(),
            ..Options::default()
        };
        assert_ne!(key, Key::of(&pixel, &halves));
    }

    #[test]
    fn a_key_changes_with_any_number_of_the_arena_s_layout() {
        let pixel = program(ShaderKind::Pixel, vec![S_ENDPGM]);
        let options = Options::default();
        let layout = rdna::arena_layout();
        assert_eq!(
            Key::of(&pixel, &options),
            Key::with_layout(&pixel, &options, &layout)
        );
        // Each number alone: an offset moved in the crate that states the layout is another
        // test of a node, and so another translation, under another name.
        for at in 0..layout.len() {
            let mut moved = layout.clone();
            moved[at] ^= 4;
            assert_ne!(
                Key::of(&pixel, &options),
                Key::with_layout(&pixel, &options, &moved),
                "number {at} of the layout"
            );
        }
        assert!(layout.len() >= 24, "{layout:?}");
    }

    #[test]
    fn a_translation_and_a_refusal_come_back_from_their_bytes() {
        let cache = Cache::in_memory();
        let (translated, _) = cache.get_or_translate(
            &program(ShaderKind::Pixel, vec![S_ENDPGM]),
            &Options::default(),
        );
        assert!(matches!(*translated, Cached::Translated(_)));
        let (refused, _) = cache.get_or_translate(
            &program(ShaderKind::Pixel, vec![0xC000_0000]),
            &Options::default(),
        );
        assert!(matches!(&*refused, Cached::Refused { name, .. } if name == "graph"));
        for cached in [translated, refused] {
            let back = decode(&encode(&cached)).expect("what was encoded decodes");
            assert_eq!(text(&back), text(&cached));
        }
        assert!(decode(b"not a cache file").is_none());
    }

    #[test]
    fn every_part_of_a_lowering_is_stored() {
        let at = Loc {
            root: 2,
            path: vec![8, 16],
            offset: 4,
            index: Some(Index {
                source: IndexSource::Data {
                    buffer: Box::new(Loc {
                        root: 4,
                        path: Vec::new(),
                        offset: 0,
                        index: Some(Index {
                            source: IndexSource::User(7),
                            shift: 5,
                        }),
                    }),
                    offset: 12,
                    plus: Some(3),
                },
                shift: 4,
            }),
        };
        let resource = |kind, at: Option<Loc>| Resource {
            kind,
            at,
            words: Some([1, 2, 3, 4]),
            group: 1,
            binding: 9,
            count: Some(6),
            written: true,
        };
        let cached = Cached::Translated(Box::new(Lowered {
            module: naga::Module::default(),
            resources: vec![
                resource(ResourceKind::UserData, None),
                resource(ResourceKind::ConstantBuffer { rows: 3 }, Some(at.clone())),
                resource(
                    ResourceKind::Texture {
                        dim: 5,
                        compared: true,
                    },
                    Some(Loc {
                        index: Some(Index {
                            source: IndexSource::Runtime { site: 40 },
                            shift: 5,
                        }),
                        ..at.clone()
                    }),
                ),
                resource(ResourceKind::Sampler { comparison: true }, None),
                resource(ResourceKind::Buffer { stride: Some(16) }, None),
                resource(ResourceKind::TableWords, None),
                resource(ResourceKind::Arena, None),
                resource(ResourceKind::Registers, None),
                resource(
                    ResourceKind::StorageTexture {
                        dim: 1,
                        format: naga::StorageFormat::Rgba16Float,
                    },
                    Some(at),
                ),
            ],
            approximations: [("a made-up approximation", 2)].into(),
            half_slots: [1, 5].into(),
            subgroups: true,
            intersections: vec![Intersection {
                word: 7001,
                wide: true,
                function: "part3".to_owned(),
                depth: 9,
                loops: vec![11, 6002],
            }],
            region: Some(Registers {
                given: vec!["v9".to_owned(), "s70".to_owned()],
                left: vec!["v9".to_owned()],
            }),
        }));
        let back = decode(&encode(&cached)).expect("what was encoded decodes");
        assert_eq!(text(&back), text(&cached));
    }

    #[test]
    fn the_translator_s_name_changes_with_a_file_of_any_folder_and_not_with_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("to_naga");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(dir.path().join("mod.rs"), "mod to_naga;\n").unwrap();
        std::fs::write(inner.join("trace.rs"), "fn test() {}\n").unwrap();
        std::fs::write(inner.join("notes.txt"), "no source").unwrap();
        let first = fingerprint::tree_fingerprint(dir.path());
        std::fs::write(inner.join("notes.txt"), "still none").unwrap();
        std::fs::write(inner.join("trace.rs"), "fn test() {}\r\n").unwrap();
        assert_eq!(fingerprint::tree_fingerprint(dir.path()), first);
        std::fs::write(inner.join("trace.rs"), "fn test() { }\n").unwrap();
        assert_ne!(fingerprint::tree_fingerprint(dir.path()), first);
        // And this build's is that of the sources it was built from, every folder of them.
        let sources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rdna");
        let now = fingerprint::tree_fingerprint(&sources);
        assert_eq!(TRANSLATOR, format!("{now:016x}"));
    }

    #[test]
    fn a_second_cache_on_the_directory_translates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let pixel = program(ShaderKind::Pixel, vec![S_ENDPGM]);
        let options = Options::default();
        let first = Cache::on_disk(dir.path());
        assert!(first.get(&Key::of(&pixel, &options)).is_none());
        let (written, kept) = first.get_or_translate(&pixel, &options);
        assert!(!kept);
        assert!(first.get_or_translate(&pixel, &options).1);
        // A new cache has nothing in memory: what it answers with was read from the directory.
        let second = Cache::on_disk(dir.path());
        let read = second
            .get(&Key::of(&pixel, &options))
            .expect("the entry is on disk");
        assert_eq!(text(&read), text(&written));
        assert!(second.get_or_translate(&pixel, &options).1);
        // A file cut short is a miss, and the next translation replaces it.
        let path = second.path(&Key::of(&pixel, &options)).unwrap();
        std::fs::write(&path, b"cut").unwrap();
        let third = Cache::on_disk(dir.path());
        assert!(!third.get_or_translate(&pixel, &options).1);
        assert!(
            Cache::on_disk(dir.path())
                .get_or_translate(&pixel, &options)
                .1
        );
    }
}
