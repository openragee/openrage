//! Every `.ypt` of a mounted install becomes a particles source, compiles, and imports again to the same source: the fixed point of
//! the source form, and a count of the files it holds and the ones it cannot, by target.
//!
//! Point `OPENRAGE_GAME_ROOT` at install roots (several are separated by `;`). Without one this skips, which is not a pass.
//!
//! `OPENRAGE_YPT_FILTER` (names or parts of names, separated by `,`) narrows a run to the files whose name holds one, for a look at a few.
//!
//! ⚠ The same bytes found in several archives (a pack, its `hi`, `lo` and `g9` twins, a DLC patch) are made once and counted by the
//! files they stand for. A file the source form cannot hold is listed with the sentence it was refused with: it is an error, and
//! nothing of it is kept.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use or_source::asset::Source;
use or_source::game_import::{decompile, Facts};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

const WHAT: &str = "every_ypt_of_the_install_reaches_a_fixed_point";

/// Every file of the folder `dir` below it, by its path with `/` and its bytes.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let shown = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(shown, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// What a file's source is made of, the manifest left to its id and its origin.
fn without_the_manifest(mut files: BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    files.remove("asset.json");
    files
}

/// What one file came to: the target of its source, or why it has none, and whether the source it imports to again is the same.
enum Outcome {
    Held {
        target: String,
        textures: bool,
        models: bool,
    },
    NotAFixedPoint {
        target: String,
        what: String,
    },
    Refused(String),
}

fn through(logical: &str, bytes: &[u8]) -> Outcome {
    let name = logical.rsplit('/').next().unwrap_or(logical);
    let facts = Facts {
        from_game: true,
        virtual_path: Some(logical.to_string()),
        ..Facts::default()
    };
    let first_dir = tempfile::tempdir().unwrap();
    let first = match decompile(bytes, name, first_dir.path(), &facts) {
        Ok(folder) => folder,
        Err(why) => return Outcome::Refused(why),
    };
    let source = Source::open(&first).unwrap();
    let target = source.manifest.target.clone();
    let not_fixed = |what: String| Outcome::NotAFixedPoint {
        target: target.clone(),
        what,
    };
    let compiled = match or_source::particles::compile(&source) {
        Ok(compiled) => compiled,
        Err(problems) => return not_fixed(format!("does not compile: {problems:?}")),
    };
    let second_dir = tempfile::tempdir().unwrap();
    let second = match decompile(&compiled.bytes, name, second_dir.path(), &facts) {
        Ok(folder) => folder,
        Err(why) => return not_fixed(format!("its compiled file does not import: {why}")),
    };
    let (a, b) = (
        without_the_manifest(tree(&first)),
        without_the_manifest(tree(&second)),
    );
    if a.keys().ne(b.keys()) {
        return not_fixed(format!(
            "imports to other files: {:?} then {:?}",
            a.keys().collect::<Vec<_>>(),
            b.keys().collect::<Vec<_>>()
        ));
    }
    match a.iter().find(|(file, bytes)| b[*file] != **bytes) {
        Some((file, _)) => not_fixed(format!("{file} is not the same the second time")),
        None => Outcome::Held {
            target,
            textures: a.contains_key("textures.json"),
            models: a.contains_key("models.ypt"),
        },
    }
}

#[test]
fn every_ypt_of_a_legacy_install_becomes_a_source_that_compiles_and_imports_to_the_same_source() {
    measure(false);
}

#[test]
fn every_ypt_of_an_enhanced_install_becomes_a_source_that_compiles_and_imports_to_the_same_source()
{
    measure(true);
}

/// The `.ypt` files of the installs of one edition among those the environment names.
fn measure(enhanced: bool) {
    let roots: Vec<_> = game_roots()
        .into_iter()
        .filter(|root| {
            or_title::game::Game::detect_from_root(root)
                .is_some_and(|game| game.loads_gen9_content() == enhanced)
        })
        .collect();
    if roots.is_empty() {
        let edition = if enhanced { "Enhanced" } else { "Legacy" };
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no {edition} install"));
        return;
    }
    let mut held: BTreeMap<String, usize> = BTreeMap::new();
    let (mut with_textures, mut with_models) = (0usize, 0usize);
    let mut files = 0usize;
    let mut unique = 0usize;
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut not_fixed: Vec<String> = Vec::new();
    for root in roots {
        let Some(game) = common::mount(&root) else {
            panic!("{} does not mount", root.display());
        };
        let mut logicals = Vec::new();
        let only = std::env::var("OPENRAGE_YPT_FILTER").unwrap_or_default();
        game.for_each_file(|logical, _, leaf| {
            let leaf = leaf.to_ascii_lowercase();
            let wanted = only.is_empty() || only.split(',').any(|part| leaf.contains(part));
            if wanted && leaf.ends_with(".ypt") {
                logicals.push(logical.to_string());
            }
        });
        // The same bytes are made once, and stand for every file that holds them.
        let mut by_bytes: BTreeMap<Vec<u8>, Vec<String>> = BTreeMap::new();
        for logical in logicals {
            files += 1;
            if let Some(bytes) = game.vfs.open(&logical) {
                by_bytes.entry(bytes).or_default().push(logical);
            }
        }
        unique += by_bytes.len();
        // The largest first, so that the last to finish is not the slowest.
        let mut work: Vec<(Vec<u8>, Vec<String>)> = by_bytes.into_iter().collect();
        work.sort_by_key(|(bytes, _)| std::cmp::Reverse(bytes.len()));
        let outcomes = std::sync::Mutex::new(Vec::new());
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| loop {
                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((bytes, stands_for)) = work.get(at) else {
                        break;
                    };
                    let outcome = through(&stands_for[0], bytes);
                    outcomes.lock().unwrap().push((stands_for.clone(), outcome));
                });
            }
        });
        for (stands_for, outcome) in outcomes.into_inner().unwrap() {
            match outcome {
                Outcome::Held {
                    target,
                    textures,
                    models,
                } => {
                    *held.entry(target).or_default() += stands_for.len();
                    with_textures += usize::from(textures) * stands_for.len();
                    with_models += usize::from(models) * stands_for.len();
                }
                Outcome::NotAFixedPoint { target, what } => {
                    not_fixed.push(format!("{} ({target}): {what}", stands_for[0]))
                }
                Outcome::Refused(why) => refused
                    .entry(why)
                    .or_default()
                    .extend(stands_for.iter().cloned()),
            }
        }
    }
    println!("{files} .ypt files, {unique} different ones");
    println!("  {with_textures} of them hold textures and {with_models} hold models");
    for (target, count) in &held {
        println!("  {target}: {count} files are sources that reach their fixed point");
    }
    for (why, who) in &refused {
        println!("  refused ({}): {why}", who.len());
        for logical in who.iter().take(3) {
            println!("    {logical}");
        }
    }
    assert!(
        not_fixed.is_empty(),
        "{} source(s) are not a fixed point:\n{}",
        not_fixed.len(),
        not_fixed
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
