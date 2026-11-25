//! Every compiled script of a build (a content-ciphered body of resource type `0x59`, top-level or
//! nested) reads its program header, whose hash and check copy agree, and decodes from code start to
//! code end with as many distinct natives called as the header states.
//!
//! ⚠ The file's own native tables (header +0xA8 and +0xB0) are not decoded, so the called set is
//! checked against the header's count only, not against a declared set.
//!
//! Install-gated and game-derived, so it needs the key snapshot and the build and skips cleanly
//! without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the content section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::script::compiled::Compiled;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The resource type a compiled script's 20-byte row states.
const SCRIPT_TYPE: u32 = 0x59;
const MAX_DEPTH: usize = 8;

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
    instructions: usize,
    native_calls: usize,
    /// Programs that use 0x1D3 or 0x1D4, the two opcodes one handler sizes apart.
    count_users: usize,
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
            let fail =
                |t: &mut Tally, why: String| t.failures.push(format!("entry {index}: {why}"));
            let body = match mount.read_entry_decrypted(&mut *file, entry, keys) {
                Ok(body) => body,
                Err(e) => {
                    fail(t, format!("read: {e}"));
                    continue;
                }
            };
            let read = Compiled::open(&body).and_then(|p| Ok((p.facts()?, p)));
            match read {
                Ok((facts, program)) => {
                    t.programs += 1;
                    t.instructions += facts.instructions;
                    t.native_calls += facts.native_calls;
                    let mut ops = program.instructions().flatten().map(|i| i.opcode);
                    if ops.any(|op| op == 0x1D3 || op == 0x1D4) {
                        t.count_users += 1;
                    }
                    if facts.natives_found != facts.header.natives as usize {
                        fail(
                            t,
                            format!(
                                "{} distinct natives called, the header states {}",
                                facts.natives_found, facts.header.natives
                            ),
                        );
                    }
                }
                Err(e) => fail(t, e.to_string()),
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
fn every_compiled_script_reads_its_header_and_decodes() {
    let what = "GTA VI compiled scripts over a build";
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
    println!(
        "compiled scripts {}, instructions {}, native calls {}, failures {}",
        t.programs,
        t.instructions,
        t.native_calls,
        t.failures.len()
    );
    println!(
        "still open: the native tables at header +0xA8 and +0xB0 are not decoded, so the called \
         natives are checked against the header's count, not a declared set"
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "every compiled script must read and decode end to end"
    );
    assert!(
        t.count_users >= 1,
        "the build has a program using 0x1D3 or 0x1D4, which must decode through them"
    );
    assert!(
        t.programs >= 700,
        "the build holds 746 compiled scripts, found {}",
        t.programs
    );
}
