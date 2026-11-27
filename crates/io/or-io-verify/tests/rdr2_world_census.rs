//! What an RDR2 install holds of a world and which of it a reader of the tree opens, as counts:
//! every archive mounted with the keys the user's executable unlocks, every file counted by its
//! extension, a sample of each world extension put to each reader that could take it with the
//! first refusal of each, the names a map and a type body state of what a world asks, and what
//! the shader containers hold. It prints counts, extensions, magics and names GTA V's own
//! declarations already spell, and no name of a file.
//!
//! ⚠ Install-gated: `OPENRAGE_GAME_ROOT` names an RDR2 install, whose `RDR2.exe` unlocks the
//! bundled snapshot of the cipher tables.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::container::rpf::rsc8::Rsc8;
use or_io::container::rpf::{Rdr2CryptoProvider, RpfArchive, RpfEntry, RpfVersion};
use or_io::decl::File;
use or_io::hash::literal_string_hash;
use or_io::meta::binary::{parse_meta_binary_full, rbm_sentinel_present};
use or_io::meta::{Diagnostics, SchemaLibrary};
use or_io::rdr2::TextureDictionary;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

/// The extensions a world is made of, by what the contract calls each.
const WORLD: [(&str, &str); 7] = [
    ("ymap", "map"),
    ("ytyp", "types"),
    ("ydr", "model"),
    ("yft", "model"),
    ("ydd", "model"),
    ("ytd", "textures"),
    ("ybn", "bounds"),
];
/// The extension of a shader container.
const SHADERS: &str = "fxdb";
/// The bodies of each world extension put to the readers, spread evenly over those found.
const SAMPLE: usize = 240;
/// How deep an archive inside an archive is followed.
const DEPTH: usize = 4;

/// What a map body would state for a world, by the names GTA V's own map declares.
const MAP_NAMES: [(&str, &[&str]); 7] = [
    (
        "the map",
        &["CMapData", "name", "parent", "flags", "contentFlags"],
    ),
    (
        "boxes",
        &[
            "streamingExtentsMin",
            "streamingExtentsMax",
            "entitiesExtentsMin",
            "entitiesExtentsMax",
        ],
    ),
    (
        "entities",
        &[
            "entities",
            "CEntityDef",
            "archetypeName",
            "position",
            "rotation",
            "scaleXY",
        ],
    ),
    (
        "levels of detail",
        &[
            "parentIndex",
            "lodLevel",
            "lodDist",
            "childLodDist",
            "numChildren",
            "priorityLevel",
        ],
    ),
    (
        "interiors",
        &["CMloInstanceDef", "groupId", "floorId", "MLOInstflags"],
    ),
    (
        "occluders and lights",
        &[
            "boxOccluders",
            "occludeModels",
            "LODLightsSOA",
            "DistantLODLightsSOA",
        ],
    ),
    (
        "lists and the rest",
        &[
            "instancedData",
            "containerLods",
            "physicsDictionaries",
            "carGenerators",
            "timeCycleModifiers",
        ],
    ),
];
/// What a type body would state, by the names GTA V's own type file declares.
const TYPE_NAMES: [(&str, &[&str]); 4] = [
    (
        "the types",
        &["CMapTypes", "name", "archetypes", "compositeEntityTypes"],
    ),
    (
        "an archetype",
        &[
            "CBaseArchetypeDef",
            "CTimeArchetypeDef",
            "assetName",
            "assetType",
            "drawableDictionary",
            "textureDictionary",
            "physicsDictionary",
            "clipDictionary",
            "specialAttribute",
            "hdTextureDist",
        ],
    ),
    (
        "boxes",
        &["bbMin", "bbMax", "bsCentre", "bsRadius", "lodDist"],
    ),
    (
        "interiors",
        &[
            "CMloArchetypeDef",
            "CMloRoomDef",
            "CMloPortalDef",
            "rooms",
            "portals",
            "entitySets",
        ],
    ),
];
/// A name no file states, whose hash is looked for as every other is: what chance alone finds.
const NO_SUCH_NAME: &str = "aNameNoMapOrTypeFileStates";

/// The magics a shader container's programs begin with: Direct3D's bytecode container and
/// SPIR-V's first word, as their bytes lie in a file.
const PROGRAM_MAGICS: [(&str, [u8; 4]); 2] =
    [("DXBC", *b"DXBC"), ("SPIR-V", 0x0723_0203u32.to_le_bytes())];

/// One file of an archive: which archive, its entry, and the hash its row names it by.
#[derive(Clone, Copy)]
struct At {
    archive: usize,
    entry: u32,
    hash: u32,
}

/// The files a map or a type body may name, by the extensions whose rows it would name them with.
const NAMED: [(&str, &[&str]); 4] = [
    ("models", &["ydr", "yft", "ydd"]),
    ("texture dictionaries", &["ytd"]),
    ("maps", &["ymap"]),
    ("bounds", &["ybn"]),
];
/// What every hash of a set is turned by for the set chance is measured against.
const ELSEWHERE: u32 = 0xA5A5_A5A5;

/// What was counted of one extension.
#[derive(Default)]
struct Extension {
    files: usize,
    resources: usize,
    /// Those whose row spells a name and not a hash alone.
    named: usize,
}

/// What the readers made of the sample of one extension.
#[derive(Default)]
struct Read {
    sampled: usize,
    /// The body came out of its archive, deciphered and inflated.
    bodies: usize,
    /// By the container the body is, as its leading bytes say.
    containers: BTreeMap<String, usize>,
    /// By reader: how many bodies it was put to, how many it opened, and its first refusal.
    readers: BTreeMap<&'static str, (usize, usize, Option<String>)>,
    /// The resource ids the RSC8 bodies carry.
    resource_ids: BTreeSet<u8>,
    /// Bodies whose virtual bytes hold the sentinel a paged metadata body opens with.
    metadata: usize,
    /// The names whose hash a body holds, by how many bodies hold each.
    stated: BTreeMap<&'static str, usize>,
    /// By what is named: the bodies that hold a row's hash of it, the words that are one, and the words that are one of as many hashes found nowhere.
    links: BTreeMap<&'static str, [usize; 3]>,
}

impl Read {
    fn tried(&mut self, reader: &'static str, outcome: Result<(), String>) {
        let slot = self.readers.entry(reader).or_default();
        slot.0 += 1;
        match outcome {
            Ok(()) => slot.1 += 1,
            Err(why) => {
                slot.2.get_or_insert(why);
            }
        }
    }
}

/// The install's root, where `OPENRAGE_GAME_ROOT` names one that holds `RDR2.exe`.
fn install(what: &str) -> Option<PathBuf> {
    let root = game_roots()
        .into_iter()
        .find(|r| r.join("RDR2.exe").is_file());
    if root.is_none() {
        skip_install(what, format_args!("no RDR2.exe under {GAME_ROOT_ENV}"));
    }
    root
}

/// Every `.rpf` under `dir`, however deep its folder.
fn archives_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            archives_under(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
        {
            out.push(path);
        }
    }
}

/// The extension of `name`, lower case; none where it has none.
fn extension(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// An error cut to a line.
fn said(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    let line = text.lines().next().unwrap_or_default();
    line.chars().take(160).collect()
}

/// Archives opened, those refused, and the first refusal.
type Opened = (usize, usize, Option<String>);

/// The install mounted: every archive opened, the nested ones too, and every file counted.
struct Mounted {
    archives: Vec<Arc<RpfArchive>>,
    top: Opened,
    nested: Opened,
    /// Top-level archives that are no RPF8.
    other_versions: usize,
    extensions: BTreeMap<String, Extension>,
    /// The files of each world extension, and the shader containers.
    kept: BTreeMap<&'static str, Vec<At>>,
    /// Every file counted, the archives themselves left out.
    files: usize,
}

fn mount(root: &Path, provider: &Arc<Rdr2CryptoProvider>) -> Mounted {
    let mut paths = Vec::new();
    archives_under(root, &mut paths);
    paths.sort();
    let mut mounted = Mounted {
        archives: Vec::new(),
        top: (0, 0, None),
        nested: (0, 0, None),
        other_versions: 0,
        extensions: BTreeMap::new(),
        kept: BTreeMap::new(),
        files: 0,
    };
    let mut queue: Vec<(Arc<RpfArchive>, usize)> = Vec::new();
    for path in paths {
        let mut head = [0u8; 4];
        let read = std::fs::File::open(&path)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head));
        if read.is_err() || RpfVersion::detect(head) != Some(RpfVersion::Rpf8) {
            mounted.other_versions += 1;
            continue;
        }
        match RpfArchive::open_with(&path, provider.clone()) {
            Ok(archive) => {
                mounted.top.0 += 1;
                queue.push((Arc::new(archive), 0));
            }
            Err(e) => {
                mounted.top.1 += 1;
                mounted.top.2.get_or_insert(said(e));
            }
        }
    }
    let kept = (WORLD.iter().map(|(ext, _)| *ext)).chain([SHADERS]);
    let kept: Vec<&'static str> = kept.collect();
    while let Some((archive, depth)) = queue.pop() {
        let index = mounted.archives.len();
        for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
            let name = node.name();
            let entry = node.entry_index();
            let ext = extension(name).unwrap_or_default();
            if ext == "rpf" {
                if depth < DEPTH {
                    match archive.open_nested(entry) {
                        Ok(nested) => {
                            mounted.nested.0 += 1;
                            queue.push((Arc::new(nested), depth + 1));
                        }
                        Err(e) => {
                            mounted.nested.1 += 1;
                            mounted.nested.2.get_or_insert(said(e));
                        }
                    }
                }
                continue;
            }
            mounted.files += 1;
            let counted = mounted.extensions.entry(ext.clone()).or_default();
            counted.files += 1;
            counted.resources += usize::from(matches!(
                archive.entries[entry as usize],
                RpfEntry::Resource(_)
            ));
            counted.named += usize::from(!name.starts_with("hash_"));
            if let Some(kept) = kept.iter().find(|k| **k == ext) {
                mounted.kept.entry(kept).or_default().push(At {
                    archive: index,
                    entry,
                    hash: archive.entries[entry as usize].name_offset(),
                });
            }
        }
        mounted.archives.push(archive);
    }
    mounted
}

/// At most [`SAMPLE`] of `all`, spread evenly.
fn sample(all: &[At]) -> Vec<At> {
    let step = all.len().div_ceil(SAMPLE).max(1);
    all.iter().step_by(step).copied().collect()
}

/// The container `bytes` are, by their leading bytes: the magic as text where it is text.
fn container(bytes: &[u8]) -> String {
    match bytes.get(..4) {
        Some(magic) if magic.iter().all(|b| b.is_ascii_alphanumeric()) => {
            String::from_utf8_lossy(magic).into_owned()
        }
        Some(_) => "no magic".to_string(),
        None => "under four bytes".to_string(),
    }
}

/// Whether `bytes` hold the hash of `name` as a word at a place a word lies at.
// ⚠ A metadata body names a type and a member by the hash of the name as it is spelled: the hash that folds case finds only the names that are lower case already.
fn holds_hash(bytes: &[u8], name: &str) -> bool {
    let hash = literal_string_hash(name.as_bytes()).to_le_bytes();
    bytes.as_chunks::<4>().0.contains(&hash)
}

/// The names of `groups`, and the one no file states.
fn names_of(groups: &[(&str, &[&'static str])]) -> Vec<&'static str> {
    let names = groups.iter().flat_map(|(_, names)| names.iter().copied());
    names.chain([NO_SUCH_NAME]).collect()
}

/// What a body of `ext` would state for a world, in groups.
fn groups_of(ext: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match ext {
        "ymap" => &MAP_NAMES,
        "ytyp" => &TYPE_NAMES,
        _ => &[],
    }
}

/// The hashes the rows of each of [`NAMED`] carry.
type Rows = Vec<(&'static str, BTreeSet<u32>)>;

fn rows_of(mounted: &Mounted) -> Rows {
    (NAMED.iter())
        .map(|(what, extensions)| {
            let files = extensions.iter().filter_map(|ext| mounted.kept.get(ext));
            (*what, files.flatten().map(|at| at.hash).collect())
        })
        .collect()
}

/// Put one body of `ext` to every reader that could take it.
fn read_one(
    mounted: &Mounted,
    (at, ext): (At, &str),
    (library, rows): (&SchemaLibrary, &Rows),
    read: &mut Read,
) {
    read.sampled += 1;
    let archive = &mounted.archives[at.archive];
    let bytes = match archive.read_entry(at.entry) {
        Ok(bytes) => {
            read.tried("the archive's body", Ok(()));
            bytes
        }
        Err(e) => {
            read.tried("the archive's body", Err(said(e)));
            return;
        }
    };
    read.bodies += 1;
    *read.containers.entry(container(&bytes)).or_default() += 1;

    // GTA V's declarations all open an RSC7 first: their first refusal is that head's.
    read.tried(
        "GTA V's declarations (RSC7)",
        Rsc7::peek(&bytes).map(|_| ()).map_err(said),
    );
    if ext == "ytd" {
        read.tried(
            "RDR2's texture dictionary",
            TextureDictionary::from_bytes(&bytes)
                .map(|_| ())
                .map_err(said),
        );
    }
    let groups = groups_of(ext);
    if !groups.is_empty() {
        let mut diags = Diagnostics::lenient();
        read.tried(
            "the metadata reader",
            parse_meta_binary_full(library, &bytes, &mut diags)
                .map(|_| ())
                .map_err(said),
        );
    }
    if !Rsc8::detect(&bytes) {
        return;
    }
    let rsc = match Rsc8::open(&bytes) {
        Ok(rsc) => {
            read.tried("the RSC8 container", Ok(()));
            rsc
        }
        Err(e) => {
            read.tried("the RSC8 container", Err(said(e)));
            return;
        }
    };
    read.resource_ids.insert(rsc.resource_id());
    if groups.is_empty() {
        return;
    }
    let virtual_bytes = &rsc.data()[..rsc.virtual_size().min(rsc.data().len())];
    read.metadata += usize::from(rbm_sentinel_present(virtual_bytes));
    for name in names_of(groups) {
        if holds_hash(virtual_bytes, name) {
            *read.stated.entry(name).or_default() += 1;
        }
    }
    // A row's own hash is left out: a body that held it would name itself.
    let words = virtual_bytes.as_chunks::<4>().0.iter();
    let words: Vec<u32> = words.map(|w| u32::from_le_bytes(*w)).collect();
    for (what, hashes) in rows {
        let named = (words.iter()).filter(|w| **w != at.hash && hashes.contains(w));
        let named = named.count();
        let chance = (words.iter()).filter(|w| hashes.contains(&(**w ^ ELSEWHERE)));
        let link = read.links.entry(what).or_default();
        link[0] += usize::from(named > 0);
        link[1] += named;
        link[2] += chance.count();
    }
}

fn first(first: &Option<String>) -> String {
    (first.as_ref()).map_or(String::new(), |why| format!(" (first: {why})"))
}

#[test]
fn the_world_files_of_an_rdr2_install_are_counted_and_put_to_every_reader() {
    let what = "the world files of an RDR2 install";
    let Some(root) = install(what) else {
        return;
    };
    let provider = Rdr2CryptoProvider::for_install(root.join("RDR2.exe"), None)
        .map(Arc::new)
        .expect("the executable's AES key unlocks the snapshot");
    let mounted = mount(&root, &provider);
    let (opened, refused, why) = &mounted.top;
    eprintln!(
        "rdr2 mount: {opened} top-level RPF8 archive(s) opened, {refused} refused{}; {} that are no RPF8",
        first(why),
        mounted.other_versions
    );
    let (opened, refused, why) = &mounted.nested;
    eprintln!(
        "rdr2 mount: {opened} nested archive(s) opened to depth {DEPTH}, {refused} refused{}",
        first(why),
    );
    eprintln!(
        "rdr2 files: {} in {} archive(s), by extension (files, resources, named):",
        mounted.files,
        mounted.archives.len()
    );
    for (ext, counted) in &mounted.extensions {
        let ext = if ext.is_empty() { "(none)" } else { ext };
        eprintln!(
            "  {ext:<8} {:>8} {:>8} {:>8}",
            counted.files, counted.resources, counted.named
        );
    }
    assert!(mounted.top.0 > 0, "no archive of the install opened");
    // Every file is counted under one extension.
    let summed: usize = mounted.extensions.values().map(|e| e.files).sum();
    assert_eq!(summed, mounted.files);
    let named: usize = mounted.extensions.values().map(|e| e.named).sum();
    eprintln!(
        "rdr2 files: {named} of {} carry a name, the others a hash alone",
        mounted.files
    );
    // What the reference states of a world's files: the row of every one says it is a resource.
    for (ext, _) in WORLD {
        let counted = mounted.extensions.get(ext);
        let (files, resources) = counted.map_or((0, 0), |c| (c.files, c.resources));
        assert!(files > 0, "{ext}: the install holds none");
        assert_eq!(resources, files, "{ext}: a file whose row is no resource's");
    }

    let library = SchemaLibrary::default();
    let rows = rows_of(&mounted);
    for (ext, role) in WORLD {
        let all = mounted.kept.get(ext).map_or(&[][..], Vec::as_slice);
        let mut read = Read::default();
        for at in sample(all) {
            read_one(&mounted, (at, ext), (&library, &rows), &mut read);
        }
        eprintln!(
            "rdr2 {ext} ({role}): {} found, {} sampled, {} bodies read; containers {:?}; resource ids {:?}",
            all.len(),
            read.sampled,
            read.bodies,
            read.containers,
            read.resource_ids
        );
        for (reader, (tried, opened, why)) in &read.readers {
            eprintln!(
                "  {reader}: {opened} of {tried} opened{}",
                why.as_ref()
                    .map_or(String::new(), |why| format!("; first refusal: {why}")),
            );
        }
        let groups = groups_of(ext);
        if !groups.is_empty() {
            eprintln!(
                "  paged metadata by its sentinel: {} of {} bodies",
                read.metadata, read.bodies
            );
        }
        let held = |name: &str| read.stated.get(name).copied().unwrap_or(0);
        for (group, names) in groups {
            let counts: Vec<String> = (names.iter())
                .map(|name| format!("{name} {}", held(name)))
                .collect();
            eprintln!(
                "  states {group}, by the hash of each name, of {} bodies: {}",
                read.bodies,
                counts.join(", ")
            );
        }
        // Every body read is counted under one container, and a reader that refused one says why.
        assert!(read.bodies <= read.sampled);
        let container_total: usize = read.containers.values().sum();
        assert_eq!(
            container_total, read.bodies,
            "{ext}: a body with no container counted"
        );
        for (reader, (tried, opened, why)) in &read.readers {
            assert!(opened <= tried && *tried <= read.sampled, "{ext}: {reader}");
            assert!(
                opened == tried || why.as_ref().is_some_and(|why| !why.trim().is_empty()),
                "{ext}: {reader} refused a body and gave no reason"
            );
        }
        for (named, [bodies, words, chance]) in &read.links {
            eprintln!(
                "  names {named} by the hash of their row: {words} word(s) in {bodies} of {} bodies; {chance} word(s) of as many hashes found nowhere",
                read.bodies
            );
        }
        if !groups.is_empty() {
            let names = names_of(groups);
            let found = names.iter().filter(|name| held(name) > 0).count();
            eprintln!(
                "  {found} of the {} names are stated by a body of the sample",
                names.len() - 1
            );
        }
        // What the reference states of every world file read, and of the one format with a reader: an RSC8 by its head, and a texture dictionary that opens.
        assert_eq!(
            read.containers.get("RSC8").copied().unwrap_or(0),
            read.bodies,
            "{ext}: a body that is no RSC8"
        );
        if let Some((tried, opened, why)) = read.readers.get("RDR2's texture dictionary") {
            assert_eq!(opened, tried, "{ext}: the declaration refused one: {why:?}");
            assert_eq!(*tried, read.bodies, "{ext}: a body not put to it");
        }
        // What the reference states of a map and a type file: each is paged metadata that names GTA V's own root type.
        if let Some((_, [root, ..])) = groups.first() {
            assert_eq!(
                read.metadata, read.bodies,
                "{ext}: a body that is no paged metadata"
            );
            assert_eq!(
                held(root),
                read.bodies,
                "{ext}: a body that does not name {root}"
            );
        }
        // A hash found by chance alone would be found of the name no file states too.
        assert_eq!(
            held(NO_SUCH_NAME),
            0,
            "{ext}: a name no file states was found"
        );
    }

    let containers = mounted.kept.get(SHADERS).map_or(&[][..], Vec::as_slice);
    for at in containers {
        let archive = &mounted.archives[at.archive];
        let bytes = archive
            .read_entry(at.entry)
            .expect("a shader container reads");
        let counts: Vec<String> = (PROGRAM_MAGICS.iter())
            .map(|(name, magic)| {
                let found = bytes.windows(4).filter(|w| *w == magic).count();
                format!("{found} {name}")
            })
            .collect();
        eprintln!(
            "rdr2 {SHADERS}: {} bytes, head {:?}, program magics: {}",
            bytes.len(),
            container(&bytes),
            counts.join(", ")
        );
    }
}
