//! R72i's script-program parity: whether GTA VI's compiled scripts are the same package and the
//! same instruction set on the two builds, measured program by program.
//!
//! A script program is a content-ciphered RPF9 entry whose deciphered body is a managed-VM program
//! (eight zero bytes, then the container the [`Program`] reader reads). For each build
//! `OPENRAGE_GAME_ROOT` names, every `*script*.rpf` and every archive nested in it is mounted with
//! the keys read out of that build's own executable (D18: the Xbox PE, or the PlayStation 5
//! `eboot.bin`), and every such body is taken to three points, the first two the same code on both
//! builds:
//!
//!  * **container** — [`Program::read`] parses the header and the tables. It hard-codes the Xbox
//!    build id and native-anchor, so it refuses a PlayStation 5 program by name at byte 0x24 (the
//!    check copy); that is a named refusal, never plausible nonsense.
//!  * **instruction set** — the code region (from the header's own code pointer and length) is
//!    decoded with the shared opcode table ([`compiled`]), the per-executable check-copy stamp
//!    first set to what the Xbox reader expects so the stamp does not gate the decode. Every
//!    instruction decoding means the bytecode, the opcode table and the word/operand ciphers are
//!    the ones both builds share.
//!  * **the stamp** — each program's check copy is `(hash ^ anchor).rotate_left(12)`; the anchor it
//!    implies is collected, and a build whose programs all imply one anchor proves the container
//!    format is the same and only that one per-executable constant differs.
//!
//! The package (which archives, how many entries, each body's size) and the opcodes used are
//! reported so a difference in either would show. A build that holds fewer programs, or whose code
//! does not decode, or whose programs imply more than one anchor, fails.
//!
//! ⚠ Install-gated and slow-tier (it walks the script archives of whole installs), so it skips
//! cleanly without:
//!   `OPENRAGE_GAME_ROOT`   the build roots, `;`-separated, each holding `*script*.rpf` and its
//!                          executable (`game_scarlett_master.exe`, or `eboot.bin` for a PS5 build)

#![cfg(feature = "gta6")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_extract::extract_snapshot;
use or_io::container::rpf::{Gta6Mount, GtaViKeys};
use or_io::gta6::script::compiled::{self, check_copy};
use or_io::gta6::script::opcode;
use or_io::gta6::script::program::Program;
use or_io::Platform;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

/// The content cipher's type: an entry whose body is deciphered before its head is read. A script
/// body is one of these, as the reader census and the CLI's export walk both hold.
const CONTENT_CIPHER_TYPE: u16 = 17;
/// Real nesting is shallow; this stops a cyclic mount recursing forever.
const MAX_DEPTH: usize = 8;
/// A program header is at least this long; a shorter body is no program.
const HEADER_LEN: usize = 0x180;

/// What one build's script archives yielded.
#[derive(Default)]
struct Measure {
    archives: usize,
    /// Content-ciphered entries met.
    ciphered: usize,
    /// Those whose deciphered body begins as a program does (eight zero bytes): script programs.
    programs: usize,
    /// Programs the container reader ([`Program::read`]) parsed.
    read: usize,
    /// Programs the container reader refused by name (it hard-codes the Xbox build's stamp).
    refused: usize,
    /// Programs whose whole code region decoded under the one opcode table (stamp neutralised).
    decoded: usize,
    /// The distinct opcodes those decoded code regions use.
    opcodes: BTreeSet<u16>,
    /// Instructions decoded across all programs.
    instructions: usize,
    /// The native anchor each program's check copy implies; one entry per distinct anchor, counted.
    anchors: BTreeMap<u32, usize>,
}

/// Whether the folder is a PlayStation 5 build, by the `eboot.bin` beside its archives.
fn platform_of(root: &Path) -> Platform {
    if root.join("eboot.bin").is_file() {
        Platform::Ps5
    } else {
        Platform::XboxSeries
    }
}

/// The build's own executable, from which its keys are read (D18).
fn executable(root: &Path, platform: Platform) -> Option<PathBuf> {
    let name = match platform {
        Platform::Ps5 => "eboot.bin",
        _ => "game_scarlett_master.exe",
    };
    let path = root.join(name);
    path.is_file().then_some(path)
}

/// The keys of the build at `root`, read out of its own executable; `None` when this extractor
/// knows no such build, so it is reported and skipped rather than read with another build's keys.
fn keys_of(root: &Path, platform: Platform) -> Option<GtaViKeys> {
    let exe = executable(root, platform)?;
    let bytes = std::fs::read(&exe).ok()?;
    match extract_snapshot(&bytes) {
        Ok(plain) => GtaViKeys::parse(&plain).ok(),
        Err(e) => {
            eprintln!("script parity: {} keys unread: {e}", platform.label());
            None
        }
    }
}

/// Every archive whose name holds `script` under `root` and under each `pack/<name>/`.
fn script_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut scripts_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains("script") && name.ends_with(".rpf") {
                out.push(path);
            }
        }
    };
    scripts_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            scripts_in(&pack.path());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Take one deciphered body to the three points and fold it into `m`.
fn body(bytes: &[u8], m: &mut Measure) {
    // A program begins with eight zero bytes; a shorter or otherwise-headed body is no script.
    if bytes.len() < HEADER_LEN || bytes[..8].iter().any(|&b| b != 0) {
        return;
    }
    m.programs += 1;

    // The container reader: Xbox reads, PlayStation 5 is refused by name at the check copy (0x24).
    match Program::read(bytes) {
        Ok(_) => m.read += 1,
        Err(_) => m.refused += 1,
    }

    // The anchor the program's own check copy implies: check = (hash ^ anchor).rotate_left(12).
    let hash = u32_at(bytes, 0x10);
    let check = u32_at(bytes, 0x24);
    let anchor = check.rotate_right(12) ^ hash;
    *m.anchors.entry(anchor).or_default() += 1;

    // The instruction set, free of the per-executable stamp: set the check copy to what the shared
    // header reader expects for this hash, leaving the code region untouched, and decode it whole.
    let mut neutral = bytes.to_vec();
    neutral[0x24..0x28].copy_from_slice(&check_copy(hash).to_le_bytes());
    let Ok(program) = compiled::Compiled::open(&neutral) else {
        return;
    };
    let mut opcodes = BTreeSet::new();
    let mut count = 0usize;
    for insn in program.instructions() {
        match insn {
            Ok(insn) => {
                opcodes.insert(insn.opcode);
                count += 1;
            }
            // An opcode past the one table, or a run off the code region: the instruction set differs.
            Err(e) => {
                eprintln!("script parity: a code region of 0x{hash:08X} did not decode: {e}");
                return;
            }
        }
    }
    m.decoded += 1;
    m.instructions += count;
    m.opcodes.extend(opcodes);
}

/// Walk one mounted archive and every archive nested in it, folding every script body into `m`.
fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, m: &mut Measure) {
    let entries = mount.archive.entries.clone();
    for entry in &entries {
        if depth < MAX_DEPTH
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                m.archives += 1;
                walk(file, &child, keys, depth + 1, m);
            }
            continue;
        }
        let Some(cipher) = entry.cipher else { continue };
        if cipher.cipher_type != CONTENT_CIPHER_TYPE {
            continue;
        }
        m.ciphered += 1;
        if let Ok(bytes) = mount.read_entry_decrypted(&mut *file, entry, keys) {
            body(&bytes, m);
        }
    }
}

/// The script programs of one build measured: every `*script*.rpf` mounted and walked.
fn measure(root: &Path, keys: &GtaViKeys) -> Measure {
    let mut m = Measure::default();
    for path in script_archives(root) {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        match Gta6Mount::open_root(&mut file, keys) {
            Ok(mount) => {
                m.archives += 1;
                walk(&mut file, &mount, keys, 0, &mut m);
            }
            Err(e) => eprintln!("script parity: open {}: {e}", path.display()),
        }
    }
    m
}

/// One build's measurement printed.
fn report(platform: Platform, m: &Measure) {
    println!(
        "\n{} — {} script archive(s), {} content-ciphered entr(ies)",
        platform.label(),
        m.archives,
        m.ciphered,
    );
    println!(
        "  programs: {} script-headed; container: {} read, {} refused by name",
        m.programs, m.read, m.refused,
    );
    println!(
        "  instruction set: {} of {} programs decoded whole, {} instruction(s), {} of {} opcodes used",
        m.decoded,
        m.programs,
        m.instructions,
        m.opcodes.len(),
        opcode::COUNT,
    );
    let anchors: Vec<String> = m
        .anchors
        .iter()
        .map(|(a, n)| format!("0x{a:08X} ({n})"))
        .collect();
    println!(
        "  native anchor(s) the check copies imply: {}",
        anchors.join(", ")
    );
}

#[test]
fn a_build_that_holds_or_decodes_fewer_programs_or_implies_two_anchors_is_named() {
    // Equal program counts and a whole decode on each side is parity.
    let full = |programs, decoded, anchor| Measure {
        programs,
        decoded,
        anchors: BTreeMap::from([(anchor, programs)]),
        ..Measure::default()
    };
    let xbox = full(10, 10, 0x01D1_09B0);
    let ps5 = full(10, 10, 0x0300_0000);
    assert_eq!(
        xbox.programs, ps5.programs,
        "equal counts is packing parity"
    );
    assert_eq!(ps5.decoded, ps5.programs, "every program decoded");
    assert_eq!(ps5.anchors.len(), 1, "one anchor per build");
    // A build that decodes fewer, or implies two anchors, is not parity.
    let short = full(10, 9, 0x0300_0000);
    assert_ne!(
        short.decoded, short.programs,
        "a code region that did not decode"
    );
    let mut split = full(10, 10, 0x0300_0000);
    *split.anchors.entry(0x0400_0000).or_default() += 1;
    assert!(
        split.anchors.len() > 1,
        "two anchors is a format difference"
    );
}

#[test]
fn ps5_script_programs_are_the_same_package_and_instruction_set_as_xbox() {
    let what = "GTA VI script-program parity over the builds";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("{GAME_ROOT_ENV} names no GTA VI folder"));
        return;
    }

    let mut by_platform: Vec<(Platform, Measure)> = Vec::new();
    for root in &roots {
        let platform = platform_of(root);
        if script_archives(root).is_empty() {
            continue;
        }
        let Some(keys) = keys_of(root, platform) else {
            eprintln!(
                "script parity: {} at {} has no keys this extractor knows; skipped",
                platform.label(),
                root.display(),
            );
            continue;
        };
        let m = measure(root, &keys);
        report(platform, &m);
        match by_platform.iter_mut().find(|(p, _)| *p == platform) {
            Some((_, into)) => fold(into, m),
            None => by_platform.push((platform, m)),
        }
    }

    if by_platform.iter().all(|(_, m)| m.programs == 0) {
        skip_install(
            what,
            format_args!("no script program under the folders {GAME_ROOT_ENV} names"),
        );
        return;
    }

    // Each build present: every script program's code region decodes whole under the one opcode
    // table (the instruction set is the shared code), and all its programs imply one native anchor
    // (the container format is the shared code, with the anchor its one per-executable constant).
    for (platform, m) in &by_platform {
        if m.programs == 0 {
            continue;
        }
        assert_eq!(
            m.decoded,
            m.programs,
            "{}: {} of {} script programs did not decode under the one opcode table; the \
             instruction set differs from the shared decoder",
            platform.label(),
            m.programs - m.decoded,
            m.programs,
        );
        assert_eq!(
            m.anchors.len(),
            1,
            "{}: its programs imply {} native anchors ({:?}); the check-copy format is not one \
             constant per build",
            platform.label(),
            m.anchors.len(),
            m.anchors
                .keys()
                .map(|a| format!("0x{a:08X}"))
                .collect::<Vec<_>>(),
        );
    }

    let xbox = by_platform.iter().find(|(p, _)| *p == Platform::XboxSeries);
    let ps5 = by_platform.iter().find(|(p, _)| *p == Platform::Ps5);
    let (Some((_, xbox)), Some((_, ps5))) = (xbox, ps5) else {
        eprintln!(
            "script parity: only one build measured, so no cross-build comparison; name both builds \
             in {GAME_ROOT_ENV} to compare the packages and instruction sets",
        );
        return;
    };

    // Packing parity: both builds hold the same script programs.
    assert_eq!(
        xbox.programs, ps5.programs,
        "the builds hold a different number of script programs ({} vs {}); the package differs",
        xbox.programs, ps5.programs,
    );

    // The instruction set is the same: the opcodes PlayStation 5 programs use are a subset of the
    // one table both decode under (a PS5-only opcode would already have failed the whole-decode
    // assertion). Reported for judgement.
    let only_ps5: Vec<u16> = ps5.opcodes.difference(&xbox.opcodes).copied().collect();
    if !only_ps5.is_empty() {
        eprintln!("script parity: opcodes only PlayStation 5 programs used: {only_ps5:?}");
    }
    eprintln!(
        "script parity: Xbox anchor {:?}, PlayStation 5 anchor {:?} — the container differs only in \
         this per-executable constant and the build id beside it",
        xbox.anchors.keys().map(|a| format!("0x{a:08X}")).collect::<Vec<_>>(),
        ps5.anchors.keys().map(|a| format!("0x{a:08X}")).collect::<Vec<_>>(),
    );
}

/// Fold `from` into `into`: a second folder of the same platform is one measurement.
fn fold(into: &mut Measure, from: Measure) {
    into.archives += from.archives;
    into.ciphered += from.ciphered;
    into.programs += from.programs;
    into.read += from.read;
    into.refused += from.refused;
    into.decoded += from.decoded;
    into.instructions += from.instructions;
    into.opcodes.extend(from.opcodes);
    for (anchor, n) in from.anchors {
        *into.anchors.entry(anchor).or_default() += n;
    }
}
