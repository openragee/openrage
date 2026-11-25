//! `rpf scan-names`: the names GTA VI archives' bodies spell, harvested into a vocabulary kept in
//! the configuration folder, with the name each drawable and fragment states of itself. The
//! other GTA VI commands show a hash by the name the kept vocabulary holds for it.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use or_io::gta6::names::{self, ArchiveHarvest, TextureUses, Vocabulary, HARVEST_PREFIX};
use or_io::gta6::texture_names::{name_keys, name_keys_of_plain};
use or_io::gta6::TexturePack;

const USAGE: &str =
    "usage: rpf scan-names [<archive.rpf>...] [--prefix <bytes>] [--threads <count>] [--names <file>]... [--pack <file.ttpf or folder>]...";

/// The vocabulary kept in the configuration folder, read once; empty where none is kept.
pub(crate) fn vocabulary() -> &'static Vocabulary {
    static HELD: OnceLock<Vocabulary> = OnceLock::new();
    HELD.get_or_init(|| {
        // A test prints what its own bodies state, whatever this machine keeps.
        if cfg!(test) {
            return Vocabulary::default();
        }
        or_title::config::config_dir()
            .map(|root| Vocabulary::load(&names::vocabulary_file(&root)))
            .unwrap_or_default()
    })
}

/// The textures the models of the archives harvested name by key, as kept in the configuration
/// folder, read once; none where none are kept.
pub(crate) fn texture_uses() -> &'static TextureUses {
    static HELD: OnceLock<TextureUses> = OnceLock::new();
    HELD.get_or_init(|| {
        // A test prints what its own pack states, whatever this machine keeps.
        if cfg!(test) {
            return TextureUses::new();
        }
        or_title::config::config_dir()
            .map(|root| names::load_texture_uses(&root))
            .unwrap_or_default()
    })
}

/// ` (name)` where the kept vocabulary holds the one name the 32-bit `hash` names, else nothing.
pub(crate) fn said32(hash: u32) -> String {
    said(vocabulary().name32(hash))
}

/// ` (name)` where the kept vocabulary holds a name both `hash` and the 64-bit `word` name.
pub(crate) fn said_both(hash: u32, word: u64) -> String {
    said(vocabulary().confirmed(hash, word))
}

fn said(name: Option<&str>) -> String {
    name.map_or_else(String::new, |name| format!(" ({name})"))
}

/// Holds `names` as plain names in the vocabulary kept in the configuration folder, and says how
/// many were new.
pub(crate) fn keep<'a>(names: impl IntoIterator<Item = &'a String>) -> Result<String, String> {
    let root = or_title::config::config_dir().ok_or("no configuration folder to keep names in")?;
    let file = names::vocabulary_file(&root);
    let mut kept = Vocabulary::load(&file);
    let new = names.into_iter().filter(|n| kept.add(n, None)).count();
    kept.save(&file)
        .map_err(|e| format!("write({}): {e}", file.display()))?;
    Ok(format!(
        "  kept          {new} names new of {} held in {}",
        kept.len(),
        file.display()
    ))
}

/// Harvests every body of the archive at `path` a harvest reads, the first `prefix` bytes of
/// each, on `threads` threads, with a line a second on stderr that says how far it has got.
fn harvest_archive(
    path: &str,
    prefix: usize,
    threads: usize,
    cancelled: &or_job::Cancel,
) -> Result<ArchiveHarvest, String> {
    let (began, last) = (Instant::now(), Mutex::new(Instant::now()));
    let reached = |done: u64, len: u64| {
        let Ok(mut last) = last.lock() else {
            return;
        };
        if last.elapsed() < Duration::from_secs(1) && done < len {
            return;
        }
        *last = Instant::now();
        let secs = began.elapsed().as_secs_f64();
        let mut err = std::io::stderr().lock();
        let _ = writeln!(
            err,
            "  {path}: {} of {} MiB, {secs:.0} s",
            done >> 20,
            len.div_ceil(1 << 20)
        );
    };
    let stop = || cancelled.is_set();
    let gave = names::harvest_archive(Path::new(path), prefix, threads, &stop, &reached)
        .map_err(|e| format!("{path}: {e}"))?;
    match gave.stopped {
        true => Err("cancelled".to_string()),
        false => Ok(gave),
    }
}

/// The lines that count what `vocabulary` holds.
fn count_lines(vocabulary: &Vocabulary) -> Vec<String> {
    let counts = vocabulary.counts();
    let kinds: Vec<String> = (counts.kinds.iter())
        .map(|(kind, n)| format!("{n} .#{}{}", char::from(kind[0]), char::from(kind[1])))
        .collect();
    let resource = counts.names - counts.plain;
    vec![
        format!(
            "  names         {} held: {resource} spelled with an ending{}{}, {} plain",
            counts.names,
            if kinds.is_empty() { "" } else { ": " },
            kinds.join(", "),
            counts.plain
        ),
        format!(
            "  32-bit        {} hashes, {} of them shared by two names or more, which name nothing alone",
            counts.hashes32, counts.shared32
        ),
    ]
}

/// The texture packs `given` names: itself where it is a file, every `.ttpf` under it where it is
/// a folder, in the order of their paths.
fn packs_at(given: &Path, into: &mut Vec<PathBuf>) {
    let Ok(listed) = std::fs::read_dir(given) else {
        into.push(given.to_path_buf());
        return;
    };
    let mut paths: Vec<PathBuf> = listed.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            packs_at(&path, into);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ttpf"))
        {
            into.push(path);
        }
    }
}

/// The lines that count, for each of `packs`, a label and its keys, the keys the vocabulary names:
/// by a name it holds, by the naming rules from each name it holds with an ending, then by the
/// map kind and level-of-detail rules from each name it holds with none; then those of the rest
/// that a model of `uses` names by key, which have the model's name and no name of their own.
/// Several packs end with a line for their distinct keys.
// The rules run once over the vocabulary for the keys of every pack together: their cost is the
// vocabulary's, whatever the packs.
fn pack_lines(
    packs: &[(String, HashSet<u64>)],
    vocabulary: &Vocabulary,
    uses: &TextureUses,
) -> Vec<String> {
    let all: HashSet<u64> = packs.iter().flat_map(|(_, keys)| keys).copied().collect();
    let left: HashSet<u64> = (all.iter().copied())
        .filter(|key| vocabulary.name64(*key).is_none())
        .collect();
    let ruled = name_keys(vocabulary.resource_names(), &left);
    let left: HashSet<u64> = (left.into_iter())
        .filter(|key| !ruled.contains_key(key))
        .collect();
    let plain = name_keys_of_plain(vocabulary.plain_names(), &left);
    let line = |label: &str, keys: &HashSet<u64>| {
        let count = |named: &dyn Fn(&u64) -> bool| keys.iter().filter(|key| named(key)).count();
        let held = count(&|key| vocabulary.name64(*key).is_some());
        let by_rule = count(&|key| ruled.contains_key(key));
        let by_plain = count(&|key| plain.contains_key(key));
        let of_a_model =
            count(&|key| left.contains(key) && !plain.contains_key(key) && uses.contains_key(key));
        let named = held + by_rule + by_plain;
        format!(
            "  pack          {named} of {} keys named: {held} by a name held, {by_rule} by a rule, {by_plain} by a rule from a name spelled with no ending; {of_a_model} more are named by no name and are a texture a model harvested names by its key, {} are neither: {label}",
            keys.len(),
            keys.len() - named - of_a_model
        )
    };
    let mut lines: Vec<String> = (packs.iter())
        .map(|(label, keys)| line(label, keys))
        .collect();
    if packs.len() > 1 {
        let label = format!("the distinct keys of the {} packs", packs.len());
        lines.push(line(&label, &all));
    }
    let mut by_rule = BTreeMap::new();
    for (_, rule) in ruled.values() {
        *by_rule.entry(rule.label()).or_insert(0usize) += 1;
    }
    lines.extend((by_rule.iter()).map(|(rule, n)| format!("    {n} by the rule \"{rule}\"")));
    lines
}

pub(crate) fn cmd_scan_names(args: &[String]) -> Result<(), String> {
    let (mut archives, mut prefix, mut packs) = (Vec::new(), HARVEST_PREFIX, Vec::new());
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut lists = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--prefix" => {
                let bytes = words.next().ok_or("--prefix needs a byte count")?;
                prefix = super::rpf::parse_offset(bytes)? as usize;
            }
            "--threads" => {
                let count = words.next().ok_or("--threads needs a count")?;
                threads =
                    (count.parse().ok().filter(|n| *n > 0)).ok_or("--threads needs a count")?;
            }
            // Once the most a body inflated to; a harvest holds only a body's first bytes.
            "--limit" => {
                words.next().ok_or("--limit needs a byte count")?;
            }
            "--pack" => {
                let given = words.next().ok_or("--pack needs a file or a folder")?;
                packs_at(Path::new(given), &mut packs);
            }
            "--names" => lists.push(words.next().ok_or("--names needs a file")?),
            other if other.starts_with('-') => return Err(USAGE.to_string()),
            path => archives.push(path),
        }
    }
    let root = or_title::config::config_dir();
    let kept = root.as_deref().map(names::vocabulary_file);
    let mut vocabulary = kept.as_deref().map(Vocabulary::load).unwrap_or_default();
    for file in lists {
        let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let new = vocabulary.add_lines(&String::from_utf8_lossy(&bytes));
        println!("  {file}: {new} names new");
    }
    let cancelled = super::jobs::on_interrupt();
    for path in archives {
        let before = vocabulary.len();
        let gave = harvest_archive(path, prefix, threads, &cancelled)?;
        vocabulary.merge(&gave.vocabulary);
        println!(
            "  {path}: {} bodies read, {} names new, {} bodies state their own name",
            gave.bodies,
            vocabulary.len() - before,
            gave.labels.len()
        );
        println!(
            "  {path}: {} textures named by key in its models' materials",
            gave.textures.len()
        );
        if let Some(root) = &root {
            names::save_labels(root, Path::new(path), &gave.labels)
                .map_err(|e| format!("keeping the names of {path}: {e}"))?;
            names::save_texture_uses(root, Path::new(path), &gave.textures)
                .map_err(|e| format!("keeping the textures of {path}: {e}"))?;
        }
    }
    count_lines(&vocabulary)
        .iter()
        .for_each(|l| println!("{l}"));
    match &kept {
        Some(file) => {
            vocabulary
                .save(file)
                .map_err(|e| format!("{}: {e}", file.display()))?;
            println!("  kept          {}", file.display());
        }
        None => println!("  kept          nowhere: no configuration directory"),
    }
    if !packs.is_empty() {
        let mut opened = Vec::new();
        for file in &packs {
            let pack = TexturePack::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
            // A zero key keys no texture.
            let keys = (pack.entries.iter().map(|e| e.key)).filter(|key| *key != 0);
            opened.push((file.display().to_string(), keys.collect()));
        }
        let uses = root.as_deref().map(names::load_texture_uses);
        pack_lines(&opened, &vocabulary, &uses.unwrap_or_default())
            .iter()
            .for_each(|l| println!("{l}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counts_say_each_ending_and_the_plain_names() {
        let mut vocabulary = Vocabulary::default();
        assert!(count_lines(&vocabulary)[0].contains("0 held: 0 spelled with an ending, 0 plain"));
        vocabulary.harvest(
            b"\0zq_shed_07.#dr\0zq_crane.#ft\0zq_hut.#dr\0zq_plain\0",
            1 << 10,
        );
        let lines = count_lines(&vocabulary);
        assert!(
            lines[0].contains("4 held: 3 spelled with an ending: 2 .#dr, 1 .#ft, 1 plain"),
            "{lines:?}"
        );
        assert!(lines[1].contains("4 hashes, 0 of them shared"), "{lines:?}");
    }

    #[test]
    fn the_keys_of_several_packs_are_counted_by_what_names_them() {
        use or_io::gta6::hash::name_hash64;
        let mut vocabulary = Vocabulary::default();
        vocabulary.add("zq_cli_wall", Some(*b"dr"));
        vocabulary.add("zq_cli_decal", None);
        let (held, ruled, of_plain) = (
            name_hash64(b"zq_cli_decal"),
            name_hash64(b"zq_cli_wall_nm"),
            name_hash64(b"zq_cli_decal_ab"),
        );
        // A model names 11, and a key a rule names too, which is counted by its name.
        let mut uses = TextureUses::new();
        names::add_texture_uses(&mut uses, "zq_cli_hut", &[(11, 0xA1), (ruled, 0xB2)]);
        let packs = [
            (
                "first".to_string(),
                HashSet::from([held, ruled, of_plain, 11, 13]),
            ),
            ("second".to_string(), HashSet::from([ruled, 17])),
        ];
        let lines = pack_lines(&packs, &vocabulary, &uses);
        let wanted = [
            "3 of 5 keys named: 1 by a name held, 1 by a rule, 1 by a rule from a name spelled with no ending; 1 more are named by no name and are a texture a model harvested names by its key, 1 are neither: first",
            "1 of 2 keys named: 0 by a name held, 1 by a rule, 0 by a rule from a name spelled with no ending; 0 more are named by no name and are a texture a model harvested names by its key, 1 are neither: second",
            "3 of 6 keys named: 1 by a name held, 1 by a rule, 1 by a rule from a name spelled with no ending; 1 more are named by no name and are a texture a model harvested names by its key, 2 are neither: the distinct keys of the 2 packs",
            "1 by the rule \"stem and map kind\"",
        ];
        assert_eq!(lines.len(), wanted.len(), "{lines:?}");
        for (line, wanted) in lines.iter().zip(wanted) {
            assert!(line.ends_with(wanted), "{line}");
        }
        // One pack has no line for the packs together.
        assert_eq!(pack_lines(&packs[..1], &vocabulary, &uses).len(), 2);
    }

    #[test]
    fn a_pack_option_names_a_file_or_every_pack_under_a_folder() {
        let folder = tempfile::tempdir().expect("a scratch folder");
        let inner = folder.path().join("zq_inner");
        std::fs::create_dir(&inner).expect("a folder");
        for file in ["zq_b.ttpf", "zq_a.TTPF", "zq_other.rpf"] {
            std::fs::write(folder.path().join(file), b"").expect("a made-up file");
        }
        std::fs::write(inner.join("zq_c.ttpf"), b"").expect("a made-up file");
        let mut found = Vec::new();
        packs_at(folder.path(), &mut found);
        let names: Vec<_> = found
            .iter()
            .filter_map(|p| p.file_name()?.to_str())
            .collect();
        assert_eq!(names, ["zq_a.TTPF", "zq_b.ttpf", "zq_c.ttpf"]);
        // A file is taken as it is given, whatever its ending.
        let file = folder.path().join("zq_other.rpf");
        let mut given = Vec::new();
        packs_at(&file, &mut given);
        assert_eq!(given, [file]);
    }

    #[test]
    fn a_hash_no_kept_name_names_is_said_as_before() {
        assert_eq!(said(None), "");
        assert_eq!(said(Some("zq_shed_07")), " (zq_shed_07)");
        // A test reads no kept vocabulary.
        assert!(vocabulary().is_empty());
        assert_eq!(said32(or_io::gta6::hash::name_hash32(b"zq_shed_07")), "");
    }
}
