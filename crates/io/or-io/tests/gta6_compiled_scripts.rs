//! Every compiled script of a build (a content-ciphered body of resource type `0x59`, top-level or
//! nested) reads its program header, whose hash and check copy agree, and decodes from code start to
//! code end with as many distinct natives called as the header states. Written back, each is the
//! source byte for byte, and an integer edit changes only its push's operand bytes.
//!
//! ⚠ The file's own native tables (header +0xA8 and +0xB0) are not decoded, so the called set is
//! checked against the header's count only, not against a declared set.
//!
//! Install-gated and game-derived, so it needs the key snapshot and the build and skips cleanly
//! without them (a clean `just test-no-pack` passes):
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the content section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::script::compiled::Compiled;
use or_io::gta6::script::compiled_write::{set_int, write_back};
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
    /// Programs whose every instruction, encoded again, gives back the source body byte for byte.
    written_back: usize,
    /// Programs where the first 4-byte push took another value and re-read it.
    edited: usize,
    string_constants: usize,
    /// The distinct 0x28-byte objects the string constants name.
    string_objects: BTreeSet<Vec<u8>>,
    failures: Vec<String>,
}

/// The size of the object a string constant names, up to where its bytes differ between programs.
const STRING_OBJECT: usize = 0x28;
/// What a body pointer adds to the body offset it names.
const POINTER_BASE: u64 = 0x5000_0000;

/// Set the first 4-byte push to another value; only its operand bytes change and it reads back.
/// `false` where the program has no such push.
fn an_edit_round_trips(body: &[u8], program: &Compiled) -> Result<bool, String> {
    let Some(push) = (program.instructions().flatten()).find(|i| i.opcode == 0x2E) else {
        return Ok(false);
    };
    let was = i128::from(u32::from_le_bytes([
        push.operands[0],
        push.operands[1],
        push.operands[2],
        push.operands[3],
    ]));
    let mut edited = body.to_vec();
    set_int(&mut edited, push.offset, was ^ 1).map_err(|e| format!("set-int: {e}"))?;
    let at = program.header.code_start + push.offset;
    let changed: Vec<usize> = (0..body.len()).filter(|i| body[*i] != edited[*i]).collect();
    if changed != [at + 2] {
        return Err(format!(
            "set-int changed bytes {changed:X?}, not the push's first operand"
        ));
    }
    let again = Compiled::open(&edited).map_err(|e| e.to_string())?;
    let read = (again.instructions().flatten()).find(|i| i.offset == push.offset);
    if read.map(|i| i.operands[0]) != Some(push.operands[0] ^ 1) {
        return Err("the edited push does not read back its new value".to_owned());
    }
    Ok(true)
}

/// How many string constants the code holds, and the bytes of each object they name.
fn string_objects(body: &[u8], program: &Compiled) -> (usize, Vec<Vec<u8>>) {
    let data = u64::from_le_bytes(body[0xF0..0xF8].try_into().unwrap_or_default());
    let data = data.saturating_sub(POINTER_BASE) as usize;
    let mut n = 0;
    let mut objects = Vec::new();
    for ins in program
        .instructions()
        .flatten()
        .filter(|i| i.opcode == 0x34)
    {
        n += 1;
        let o = &ins.operands;
        let at = data + u32::from_le_bytes([o[0], o[1], o[2], o[3]]) as usize;
        objects.push(
            body.get(at..at + STRING_OBJECT)
                .unwrap_or_default()
                .to_vec(),
        );
    }
    (n, objects)
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    for entry in &mount.archive.entries {
        let index = entry.index as usize;
        let script = (mount.archive.entry_extra(index))
            .and_then(|x| x.nested)
            .is_some_and(|row| {
                or_io::container::rpf::gta6_kinds::kind_code(row.resource_type) == SCRIPT_TYPE
            });
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
                    match write_back(&body) {
                        Ok(out) if out == body[..] => t.written_back += 1,
                        Ok(_) => fail(t, "written back, it differs from the source".to_owned()),
                        Err(e) => fail(t, format!("write back: {e}")),
                    }
                    match an_edit_round_trips(&body, &program) {
                        Ok(edited) => t.edited += usize::from(edited),
                        Err(why) => fail(t, why),
                    }
                    let (n, objects) = string_objects(&body, &program);
                    t.string_constants += n;
                    t.string_objects.extend(objects);
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
    println!(
        "written back byte-identical {} of {}, edited and re-read {}",
        t.written_back, t.programs, t.edited
    );
    println!(
        "string constants {}, naming {} distinct {STRING_OBJECT:#X}-byte objects",
        t.string_constants,
        t.string_objects.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "every compiled script must read and decode end to end"
    );
    assert_eq!(
        t.written_back, t.programs,
        "every program written back is byte-identical"
    );
    assert!(t.edited >= 1, "the build's programs push 4-byte values");
    assert!(
        t.string_constants >= 1 && t.string_objects.len() == 1,
        "every string constant names the same object, which `script compiled --set-string` states"
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
