//! The ray tracer's selection against every acceleration structure a GTA VI folder ships: the
//! key of each root and the checksum of each of its structures, computed from the drawables and
//! the effects of the same folder.
//!
//! ⚠ Install-gated: `OPENRAGE_GAME_ROOT` names a GTA VI folder. That variable is the list of
//! installs every install suite takes, so only archives of GTA VI's generation are walked, and a
//! folder with none of them, or with neither an acceleration structure nor a drawable the ray
//! tracer draws, is a skip. A folder whose drawables the ray tracer draws ships their structures:
//! where none is found the suite fails, as it does where one is found and does not read. A scan
//! without the table gives no names, so a root and a drawable are paired by the key alone, as the
//! game pairs them: a root whose key no drawable gives is counted, and what its model is stays
//! not known.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use or_io::container::rpf::rpf9::RPF9_MAGIC;
use or_io::container::rpf::rpf9_scan::Rpf9BodyReader;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9Scan};
use or_io::gta5::shaders::sgd5::Sgd5File;
use or_io::gta6::accel::select::{
    level, select_level, triangles, twice_described, Selection, Traced,
};
use or_io::gta6::headerless::{
    read, read_accel_root, trace_words, AccelRoot, Drawable, Resource, TraceWords,
};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use rayon::prelude::*;

/// The bytes of an archive one scan walks; a body is found by the scan its start lies in.
const SHARD: u64 = 1 << 28;
/// Inflated bytes kept of each resource, which hold most virtual sections whole.
const KEEP: usize = 1 << 20;
const LIMIT: u64 = 1 << 31;
/// The machine is shared: the walk takes this many cores and no more.
const THREADS: usize = 4;
/// One in this many of the pairs whose every record holds has its geometry read.
const SAMPLE: usize = 251;

/// Where a body lies: its archive's number and its offset.
type Place = (usize, u64);

#[derive(Default)]
struct Walked {
    /// The bodies that carry an acceleration structure's signature, read or not.
    structures: usize,
    roots: Vec<(Place, AccelRoot)>,
    /// With whether the body holds this drawable alone, as a dictionary does not.
    drawables: Vec<(Place, bool, TraceWords)>,
    effects: usize,
    /// The name hashes of the effects whose geometry is described twice.
    twice: HashSet<u32>,
    /// Why a body, or a drawable of one, was not read.
    refused: BTreeMap<String, usize>,
}

impl Walked {
    fn refuse(&mut self, why: String) {
        *self.refused.entry(why).or_default() += 1;
    }
}

/// Whether the file at `path` opens with the magic of a GTA VI archive.
fn is_gta6_archive(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    let read = File::open(path).and_then(|mut file| file.read_exact(&mut magic));
    read.is_ok() && u32::from_le_bytes(magic) == RPF9_MAGIC
}

/// Every GTA VI archive under `dir`: an archive of another title's install is passed over.
fn archives_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            archives_under(&path, found);
        } else if (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
            && is_gta6_archive(&path)
        {
            found.push(path);
        }
    }
}

/// What the bodies that start in `from..to` of archive `archive` hold of the three kinds read.
fn shard(path: &Path, archive: usize, from: u64, to: u64) -> Walked {
    let mut walked = Walked::default();
    let open = || File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut scan = (Rpf9Scan::new(open(), from..to).map(|scan| scan.keeping(KEEP)))
        .unwrap_or_else(|e| panic!("scan {}: {e}", path.display()));
    let mut bodies = Rpf9BodyReader::new(open()).expect("a body reader");
    while let Some(find) = scan.next() {
        let find = find.unwrap_or_else(|e| panic!("scan {} from {from:#X}: {e}", path.display()));
        let place = (archive, find.offset);
        let run = match find.kind {
            Rpf9BodyKind::Zlib(run) => run,
            Rpf9BodyKind::Sgd5 { .. } => {
                let (_, bytes) = (bodies.read(find.offset, LIMIT)).expect("the container reads");
                let container = Sgd5File::open(&bytes).expect("the effect container opens");
                walked.effects += container.effects.len();
                walked.twice.extend(twice_described(&container));
                continue;
            }
            _ => continue,
        };
        let (Some(resource), Some(role)) = (run.resource, run.signature.and_then(|s| s.role()))
        else {
            continue;
        };
        match role {
            Rpf9Role::Drawable | Rpf9Role::DrawableWithBound | Rpf9Role::Dictionary => {
                let kept = scan.kept(&find);
                let whole;
                let virtual_section = if kept.len() as u64 >= resource.virtual_len {
                    &kept[..resource.virtual_len as usize]
                } else {
                    match bodies.read(find.offset, LIMIT) {
                        Ok((_, bytes)) => {
                            whole = bytes;
                            resource.sections(&whole).0
                        }
                        Err(e) => {
                            walked.refuse(format!("a {} body: {e}", role.label()));
                            continue;
                        }
                    }
                };
                for words in trace_words(role, virtual_section) {
                    match words {
                        Ok(words) => {
                            let alone = role != Rpf9Role::Dictionary;
                            walked.drawables.push((place, alone, words));
                        }
                        Err(e) => walked.refuse(format!("a {}: {e}", role.label())),
                    }
                }
            }
            Rpf9Role::AccelerationStructure => {
                walked.structures += 1;
                let read = (bodies.read(find.offset, LIMIT).map_err(|e| e.to_string())).and_then(
                    |(_, bytes)| {
                        let (v, p) = resource.sections(&bytes);
                        read_accel_root(v, Some(p)).map_err(|e| e.to_string())
                    },
                );
                match read {
                    Ok(root) => walked.roots.push((place, root)),
                    Err(e) => walked.refuse(format!("a root: {e}")),
                }
            }
            _ => {}
        }
    }
    walked
}

/// What a folder's walk is good for.
#[derive(Debug, PartialEq, Eq)]
enum Standing {
    /// Structures were found: the suite runs, and one that did not read fails it.
    Checked,
    /// Neither a structure nor a drawable the ray tracer draws: no build's worth of models.
    Skipped,
    /// Drawables the ray tracer draws and no structure: a build ships them, so they went unseen.
    Unseen,
}

/// How a folder stands that holds `structures` bodies of a structure's signature, and drawables
/// of which the ray tracer draws some where `traced`.
fn standing(structures: usize, traced: bool) -> Standing {
    match (structures, traced) {
        (0, false) => Standing::Skipped,
        (0, true) => Standing::Unseen,
        _ => Standing::Checked,
    }
}

#[test]
fn gta6_accel_a_folder_of_traced_drawables_and_no_structure_is_a_failure_and_no_skip() {
    assert_eq!(standing(0, false), Standing::Skipped);
    assert_eq!(standing(0, true), Standing::Unseen);
    // A structure that was found and did not read is the suite's to fail on, not a skip.
    assert_eq!(standing(1, false), Standing::Checked);
    assert_eq!(standing(3, true), Standing::Checked);
}

/// Every root, drawable and effect of the folders `OPENRAGE_GAME_ROOT` names, with the
/// archives; `None`, with the skip recorded, where they hold no GTA VI archive, or neither an
/// acceleration structure nor a drawable the ray tracer draws.
fn walk(what: &str) -> Option<(Vec<PathBuf>, Walked)> {
    let mut archives = Vec::new();
    for root in game_roots() {
        archives_under(&root, &mut archives);
    }
    archives.sort();
    if archives.is_empty() {
        let why = format_args!("no GTA VI archive under {GAME_ROOT_ENV}");
        skip_install(what, why);
        return None;
    }
    let mut shards = Vec::new();
    for (a, path) in archives.iter().enumerate() {
        let len = std::fs::metadata(path).expect("the archive's length").len();
        shards.extend((0..len).step_by(SHARD as usize).map(|from| (a, path, from)));
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(THREADS).build();
    let parts: Vec<Walked> = pool.expect("a thread pool").install(|| {
        (shards.par_iter())
            .map(|(a, path, from)| shard(path, *a, *from, from + SHARD))
            .collect()
    });
    let mut all = Walked::default();
    for part in parts {
        all.structures += part.structures;
        all.roots.extend(part.roots);
        all.drawables.extend(part.drawables);
        all.effects += part.effects;
        all.twice.extend(part.twice);
        for (why, count) in part.refused {
            *all.refused.entry(why).or_default() += count;
        }
    }
    let kept =
        |selection: &Selection| (selection.structures.iter()).any(|s| !s.geometries.is_empty());
    let traced = (all.drawables.iter()).any(|(_, _, words)| {
        let selected = selections(words, |name| all.twice.contains(&name));
        selected.iter().flatten().any(kept)
    });
    match standing(all.structures, traced) {
        Standing::Checked => Some((archives, all)),
        Standing::Skipped => {
            let why = format_args!(
                "no acceleration structure and no drawable the ray tracer draws in the {} GTA VI \
                 archives under {GAME_ROOT_ENV}",
                archives.len()
            );
            skip_install(what, why);
            None
        }
        Standing::Unseen => panic!(
            "the {} GTA VI archives under {GAME_ROOT_ENV} hold drawables the ray tracer draws and \
             no acceleration structure was found: a build ships them, so either the folder is \
             part of one or a structure's signature is no longer known",
            archives.len()
        ),
    }
}

/// The selection of each level a drawable has.
fn selections(words: &TraceWords, twice: impl Fn(u32) -> bool + Copy) -> [Option<Selection>; 4] {
    std::array::from_fn(|l| select_level(words, l, twice))
}

/// Why the records of `root` are not the structures `selected` gives of `words`; `None` where
/// they are.
fn differs(
    root: &AccelRoot,
    words: &TraceWords,
    selected: &[Option<Selection>; 4],
) -> Option<&'static str> {
    for l in 0..4u8 {
        let mut records = root.records.iter().filter(|r| r.level == l).peekable();
        if records.peek().is_none() {
            continue;
        }
        let Some(selection) = &selected[usize::from(l)] else {
            return Some("a record of a level the drawable lacks");
        };
        let mut structures = selection.structures.iter();
        for record in records {
            let Some(structure) = structures.next() else {
                return Some("more records than models the ray tracer draws");
            };
            if usize::from(record.number) != structure.model % 16 {
                return Some("a record's number is not its model's");
            }
            if usize::from(record.geometries) != structure.geometries.len() {
                return Some("a record's geometry count");
            }
            match record.structure {
                None if structure.geometries.is_empty() => {}
                None => return Some("no structure of a model that keeps geometry"),
                Some(_) if structure.geometries.is_empty() => {
                    return Some("a structure of a model that keeps no geometry")
                }
                Some(head) if head.checksum != structure.checksum => {
                    return Some("a structure's checksum")
                }
                Some(head) => {
                    let models = words.levels[usize::from(l)].as_ref();
                    let geometries =
                        &models.expect("the level selected")[structure.model].geometries;
                    let triangles: u32 = (structure.geometries.iter())
                        .map(|g| geometries[g.geometry].triangle_count)
                        .sum();
                    if head.elements > triangles || 2 * head.elements < triangles {
                        return Some("a structure's leaves are not one triangle or two each");
                    }
                }
            }
        }
        if structures.next().is_some() {
            return Some("fewer records than models the ray tracer draws");
        }
    }
    None
}

/// The body at `offset` of the archive at `path`, read with its geometry.
fn read_at(path: &Path, offset: u64) -> Result<Resource, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let (find, bytes) = read_rpf9_body(file, offset, LIMIT).map_err(|e| e.to_string())?;
    let Rpf9BodyKind::Zlib(run) = find.kind else {
        return Err("no resource".to_string());
    };
    let (resource, signature) = (run.resource.zip(run.signature)).ok_or("no signature")?;
    let (virtual_section, physical) = resource.sections(&bytes);
    read(signature, virtual_section, Some(physical)).map_err(|e| e.to_string())
}

/// How the triangles the builder is given of a structure stand to what its shipped head states.
#[derive(Debug, Default)]
struct Built {
    drawables: usize,
    /// Drawables whose body, or a geometry the selection keeps, did not read whole.
    unread: usize,
    structures: usize,
    /// Structures the builder is given as many triangles of as their build description states.
    described: usize,
    /// Structures whose head states as many elements as the builder is given triangles.
    equal: usize,
    /// Structures that state fewer elements, and no fewer than half.
    fewer: usize,
    triangles: u64,
    elements: u64,
}

/// The structures of one sampled pair, each from the drawable's own triangles as the selection
/// gives them to the builder.
fn build(
    built: &mut Built,
    (path, offset): (&Path, u64),
    words: &TraceWords,
    root: &AccelRoot,
    twice: impl Fn(u32) -> bool + Copy,
) {
    built.drawables += 1;
    let read = read_at(path, offset);
    let drawable: &Drawable = match &read {
        Ok(Resource::Drawable(drawable)) => drawable,
        Ok(Resource::DrawableWithBound(root)) => &root.drawable,
        _ => {
            built.unread += 1;
            return;
        }
    };
    for l in 0..4u8 {
        let heads = (root.records.iter()).filter(|r| r.level == l);
        let mut heads = heads.filter_map(|r| r.structure).peekable();
        if heads.peek().is_none() {
            continue;
        }
        let selection = select_level(words, l.into(), twice);
        let traced = selection.as_ref().and_then(|s| triangles(s, drawable));
        let whole = |traced: &Vec<Traced>| (traced.iter()).all(|t| t.unread + t.left_out == 0);
        let (Some(selection), Some(traced)) = (selection, traced.filter(whole)) else {
            built.unread += 1;
            return;
        };
        let kept = (selection.structures.iter()).filter(|s| !s.geometries.is_empty());
        let models = words.levels[usize::from(l)].as_ref();
        for ((head, traced), structure) in heads.zip(&traced).zip(kept) {
            let geometries = &models.expect("the level selected")[structure.model].geometries;
            let stated: u32 = (structure.geometries.iter())
                .map(|g| geometries[g.geometry].triangle_count)
                .sum();
            let given = traced.triangles.len() as u32;
            built.structures += 1;
            built.described += usize::from(stated == given);
            built.equal += usize::from(head.elements == given);
            built.fewer += usize::from(head.elements < given && 2 * head.elements >= given);
            built.triangles += u64::from(given);
            built.elements += u64::from(head.elements);
        }
    }
}

#[test]
fn gta6_accel_the_selection_gives_the_key_and_the_checksums_of_every_shipped_structure_it_pairs() {
    let Some((paths, walked)) = walk("gta6 acceleration structures") else {
        return;
    };
    let name = |path: &PathBuf| path.file_name().map(|n| n.to_string_lossy().into_owned());
    let archives: Vec<String> = paths.iter().filter_map(name).collect();
    let twice = |name: u32| walked.twice.contains(&name);
    // Roots of one key and the same records are copies in several archives: one is checked.
    let mut by_key: HashMap<u64, Vec<&AccelRoot>> = HashMap::new();
    for (_, root) in &walked.roots {
        let roots = by_key.entry(root.key).or_default();
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    let records: usize = walked.roots.iter().map(|(_, r)| r.records.len()).sum();
    println!(
        "{} archives; {} roots of {} distinct keys and {} records; {} drawables; {} effects, {} described twice",
        archives.len(),
        walked.roots.len(),
        by_key.len(),
        records,
        walked.drawables.len(),
        walked.effects,
        walked.twice.len()
    );
    for (why, count) in &walked.refused {
        println!("not read, {count}: {why}");
    }

    // Keys the rule gives, and keys it gives with the shader filter dropped: every shader then
    // draws in the opaque bucket of the ray tracer's phase.
    let (mut given, mut control) = (HashSet::new(), HashSet::new());
    let (mut pairs, mut held) = (0usize, 0usize);
    let mut causes: BTreeMap<&str, (usize, Vec<String>)> = BTreeMap::new();
    // The level a paired root states against the two the rule gives, and the stated one.
    let mut levels: BTreeMap<&str, usize> = BTreeMap::new();
    // The checksums of every model's structure, for the roots no drawable's key pairs.
    let mut checksums = HashSet::new();
    let mut sample = Vec::new();
    // Two levels of the same models have one key: a root's first record says which it is of.
    let paired = |selection: &Selection| {
        let kept = (selection.structures.iter()).any(|s| !s.geometries.is_empty());
        let roots = by_key.get(&selection.key).filter(|_| kept);
        let level = selection.level;
        (roots.into_iter().flatten().copied())
            .filter(move |root| usize::from(root.records[0].level) == level)
    };
    for ((archive, offset), alone, words) in &walked.drawables {
        let selected = selections(words, twice);
        let mut unfiltered = words.clone();
        (unfiltered.shaders.iter_mut()).for_each(|s| s.bucket_mask |= 0x2001);
        for unfiltered in selections(&unfiltered, twice).iter().flatten() {
            control.extend(paired(unfiltered).map(|root| root.key));
        }
        for selection in selected.iter().flatten() {
            checksums.extend(selection.structures.iter().map(|s| s.checksum));
            for root in paired(selection) {
                given.insert(root.key);
                pairs += 1;
                let stated = (words.level_word >> 9) & 7 != 0;
                let rule = match (level(words, false), level(words, true)) {
                    (plain, _) if stated && plain == Some(selection.level) => "the stated level",
                    _ if stated => "another level than the stated one",
                    (plain, _) if plain == Some(selection.level) => "the level of a plain model",
                    (_, reduced) if reduced == Some(selection.level) => "level 1, as reduced",
                    _ => "a level the rule does not give",
                };
                *levels.entry(rule).or_default() += 1;
                match differs(root, words, &selected) {
                    None => {
                        held += 1;
                        if *alone && held % SAMPLE == 0 {
                            sample.push(((&*paths[*archive], *offset), words, root));
                        }
                    }
                    Some(cause) => {
                        let (count, examples) = causes.entry(cause).or_default();
                        *count += 1;
                        if examples.len() < 3 {
                            examples.push(format!("{} at {offset:#X}", archives[*archive]));
                        }
                    }
                }
            }
        }
    }
    println!(
        "keys: {} of {} distinct shipped keys are the rule's key of a drawable's level; {} with the shader filter dropped",
        given.len(),
        by_key.len(),
        control.len()
    );
    println!("pairs of a root and a drawable: {pairs}; every record holds in {held}");
    for (cause, (count, examples)) in &causes {
        println!("  {count}: {cause} ({})", examples.join(", "));
    }
    for (rule, count) in &levels {
        println!("  {count} roots are of {rule}");
    }
    // A root no drawable's key pairs: whether the folder's models have its descriptions. A root
    // that stops pairing because the key's rule went wrong is one of these: its drawable's
    // checksums do not cover the key.
    let (mut described, mut unpaired) = (0usize, 0usize);
    for (key, roots) in &by_key {
        if given.contains(key) {
            continue;
        }
        unpaired += 1;
        let mut heads = roots[0].records.iter().filter_map(|r| r.structure);
        described += usize::from(heads.all(|head| checksums.contains(&head.checksum)));
    }
    println!(
        "keys no drawable gives: {unpaired}, of which {described} hold only structures whose checksum some model of the folder gives"
    );

    // The builder's own input against the shipped structures: the triangles it is given of each
    // structure of a sampled pair, and the elements the structure's head states.
    let mut built = Built::default();
    for (place, words, root) in sample {
        build(&mut built, place, words, root, twice);
    }
    println!(
        "built: {} structures of {} sampled drawables, {} more not read: the builder is given the triangles the description states of {}; {} heads state as many elements as that, {} fewer and no fewer than half; {} elements to {} triangles",
        built.structures,
        built.drawables - built.unread,
        built.unread,
        built.described,
        built.equal,
        built.fewer,
        built.elements,
        built.triangles
    );

    assert!(walked.refused.is_empty(), "bodies not read");
    // Inferred, from the two counts alone: a head's elements are its structure's triangle nodes,
    // each of one triangle or of two that share an edge. So no more than the triangles, and no
    // fewer than half; no shipped node was read to say so.
    assert!(10 * built.unread <= built.drawables, "{built:?}");
    assert_eq!(
        (built.described, built.equal + built.fewer),
        (built.structures, built.structures),
        "{built:?}"
    );
    assert!(!walked.twice.is_empty(), "no effect is described twice");
    // The game refuses a root whose key is not the rule's, so a rule that pairs few is wrong.
    // The floor is a stand-in for "every root built of one drawable": a root built of several
    // has no one drawable's key, and how many of them a build ships is no property of the rule.
    assert!(
        1000 * given.len() >= 935 * by_key.len(),
        "the rule gives {} of {} shipped keys",
        given.len(),
        by_key.len()
    );
    // Under the floor a wrong key still shows: the root it loses is built of structures the
    // folder's models describe. A root built of several drawables can be one too, so a share of
    // one key in a hundred thousand is let through.
    assert!(
        100_000 * described <= by_key.len(),
        "{described} of {unpaired} roots no drawable's key gives are built of structures the \
         folder's models describe"
    );
    assert!(
        4 * control.len() < 3 * by_key.len(),
        "the shader filter dropped still gives {} of {} shipped keys",
        control.len(),
        by_key.len()
    );
    // A key covers neither a shader's bucket nor a level past the root's first, so a root baked
    // from another revision of its model in one of the two still pairs: such pairs are few.
    assert!(
        10_000 * (pairs - held) <= pairs,
        "the selection gives every record of {held} of {pairs} pairs"
    );
    let off_rule = [
        "a level the rule does not give",
        "another level than the stated one",
    ];
    assert!(
        off_rule.iter().all(|rule| !levels.contains_key(rule)),
        "{levels:?}"
    );
}
