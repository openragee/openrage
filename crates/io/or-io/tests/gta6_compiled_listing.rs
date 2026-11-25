//! Every compiled script of a build (a content-ciphered body of resource type `0x59`, top-level or
//! nested) prints as a listing without error, and the share of its instructions the opcode table
//! names is reported with the unnamed opcodes met most often.
//!
//! Install-gated and game-derived, so it needs the key snapshot and the build and skips cleanly
//! without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the content section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::script::compiled::{disasm, Compiled};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The resource type a compiled script's 20-byte row states.
const SCRIPT_TYPE: u32 = 0x59;
const MAX_DEPTH: usize = 8;
/// The opcodes the interpreter's table holds.
const TABLE: u16 = 530;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_rpfs = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    push_rpfs(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            push_rpfs(&pack.path());
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Tally {
    programs: usize,
    functions: usize,
    instructions: usize,
    named: usize,
    /// Instructions per opcode the table leaves unnamed.
    unnamed: BTreeMap<u16, usize>,
    failures: Vec<String>,
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    for entry in &mount.archive.entries {
        let index = entry.index as usize;
        let script = (mount.archive.entry_extra(index))
            .and_then(|x| x.nested)
            .is_some_and(|row| row.resource_type == SCRIPT_TYPE);
        let flagged = entry.cipher.map(|c| c.cipher_type) == Some(CONTENT_CIPHER_TYPE);
        if script && flagged {
            let read = (mount.read_entry_decrypted(&mut *file, entry, keys))
                .map_err(|e| e.to_string())
                .and_then(|body| {
                    let program = Compiled::open(&body).map_err(|e| e.to_string())?;
                    for ins in program.instructions().flatten() {
                        if disasm::mnemonic(ins.opcode).is_none() {
                            *t.unnamed.entry(ins.opcode).or_default() += 1;
                        }
                    }
                    disasm::listing(&program, None).map_err(|e| e.to_string())
                });
            match read {
                Ok(listing) => {
                    t.programs += 1;
                    t.functions += listing.functions;
                    t.instructions += listing.instructions;
                    t.named += listing.named;
                }
                Err(e) => t.failures.push(format!("entry {index}: {e}")),
            }
            continue;
        }
        if depth < MAX_DEPTH
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                walk(file, &child, keys, depth + 1, t);
            }
        }
    }
}

#[test]
fn every_compiled_script_prints_as_a_listing() {
    let what = "GTA VI compiled script listings over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} (with content keys) and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_content_keys() {
        skip_install(
            what,
            format_args!("the snapshot carries no content section"),
        );
        return;
    }
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut t);
    }
    let share = 100.0 * t.named as f64 / t.instructions.max(1) as f64;
    let opcodes = (0..TABLE).filter(|op| disasm::op(*op).is_some()).count();
    println!(
        "listed programs {}, functions {}, instructions {}, named {} ({share:.2}%) by {opcodes} of {TABLE} opcodes, failures {}",
        t.programs,
        t.functions,
        t.instructions,
        t.named,
        t.failures.len()
    );
    let mut unnamed: Vec<_> = t.unnamed.iter().collect();
    unnamed.sort_by(|a, b| b.1.cmp(a.1));
    for (op, n) in unnamed.iter().take(12) {
        println!("  unnamed opcode {op:#05X}: {n} instructions");
    }
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "every compiled script must print as a listing"
    );
    assert!(
        t.programs >= 700,
        "the build holds 746 compiled scripts, found {}",
        t.programs
    );
    assert!(
        share >= 99.9,
        "the table names every instruction of 689 of the build's programs, here {share:.2}%"
    );
}
