//! `openrage-cli rpf scan-open` of a headerless GTA VI metadata body names a hash value no table
//! names by the vocabulary kept in the configuration folder, marked as from the text.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so it
//! runs in a child of this test binary that calls `or_cli::run` between two markers.

use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode};

use or_io::gta6::hash::name_hash32;
use or_io::gta6::names::{vocabulary_file, Vocabulary};

const CHILD_ENV: &str = "OPENRAGE_HEADERLESS_NAMES_ARGS";
const CHILD_TEST: &str = "run_headerless_names_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_headerless_names_for_a_parent() {
    let Some(args) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = args
        .to_string_lossy()
        .split(SEPARATOR)
        .map(str::to_string)
        .collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}");
    let _ = out.flush();
    let code = or_cli::run(&args);
    let _ = out.flush();
    let _ = write!(out, "{END}{}", u8::from(code == ExitCode::SUCCESS));
    let _ = out.flush();
}

/// What `openrage-cli <args>` prints with `config` as its configuration folder, and whether it succeeded.
fn run(config: &Path, args: &[&str]) -> (String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .env(or_title::config::CONFIG_DIR_VARIABLE, config)
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let (printed, tail) = rest.split_once(END).unwrap_or_else(|| {
        panic!(
            "the command panicked: {rest}{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (printed.to_string(), tail.starts_with('1'))
}

/// An RPF9 head of layout 4 that holds every measured rule, and its table to 0x1A0.
fn rpf9_head() -> Vec<u8> {
    let toc_end = 0x1A0u32;
    let mut bytes = vec![0x5Au8; toc_end as usize];
    bytes[..0x40].fill(0);
    bytes[..4].copy_from_slice(b"9FPR");
    bytes[0x04..0x08].copy_from_slice(&toc_end.to_le_bytes());
    bytes[0x08..0x0C].copy_from_slice(&(toc_end - 0x10).to_le_bytes());
    bytes[0x0C..0x10].copy_from_slice(&0x40u32.to_le_bytes());
    bytes[0x1C..0x20].copy_from_slice(&0x150u32.to_le_bytes());
    bytes[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bytes[0x30..0x34].copy_from_slice(&0x10u32.to_le_bytes());
    bytes[0x2C] = 1;
    (bytes[0x19], bytes[0x1A], bytes[0x20]) = (0x7A, 4, 5);
    (bytes[0x2D], bytes[0x2E]) = (0x0F, 0x09);
    bytes
}

/// `data` as a zlib stream of one stored deflate block, under the `78 DA` head a scan looks for.
fn zlib(data: &[u8]) -> Vec<u8> {
    let len = u16::try_from(data.len()).expect("one stored block");
    let mut out = vec![0x78, 0xDA, 0x01];
    out.extend(len.to_le_bytes());
    out.extend((!len).to_le_bytes());
    out.extend_from_slice(data);
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), &c| {
        let a = (a + u32::from(c)) % 65_521;
        (a, (b + a) % 65_521)
    });
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

const ROOT: u32 = 0x1111_0001;
const FIELD: u32 = 0x2222_0000;
/// A 32-bit string hash member.
const STRING_HASH: u8 = 0x4A;

/// The virtual section of a headerless RBM resource whose root holds one string hash, `hash`.
fn headerless_rbm(hash: u32) -> Vec<u8> {
    const VIRTUAL: u64 = 0x5000_0000;
    let mut v = vec![0u8; 0x100];
    let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    // The block map: one virtual block and no physical one.
    put(0x08, &(VIRTUAL + 0xF0).to_le_bytes());
    put(0xF8, &[1, 0]);
    put(0x10, b"0DRP");
    put(0x14, &0x7Au16.to_le_bytes());
    put(0x20, &1u32.to_le_bytes());
    put(0x28, &(VIRTUAL + 0x60).to_le_bytes());
    put(0x30, &(VIRTUAL + 0x80).to_le_bytes());
    put(0x38, &(VIRTUAL + 0xC0).to_le_bytes());
    put(0x48, &[1, 0, 0, 0, 1, 0]);
    // The root's class: one member, its table at 0xA0.
    put(0x60, &ROOT.to_le_bytes());
    put(0x64, &0x5151_5151u32.to_le_bytes());
    put(0x69, &[3]);
    put(0x70, &(VIRTUAL + 0xA0).to_le_bytes());
    put(0x78, &4u32.to_le_bytes());
    put(0x7E, &1u16.to_le_bytes());
    put(0xA0, &FIELD.to_le_bytes());
    put(0xA8, &[STRING_HASH]);
    // The one block, of the root's type, at 0xE0.
    put(0xC0, &ROOT.to_le_bytes());
    put(0xC4, &4u32.to_le_bytes());
    put(0xC8, &(VIRTUAL + 0xE0).to_le_bytes());
    put(0xE0, &hash.to_le_bytes());
    v
}

#[test]
fn a_headerless_body_s_hash_the_kept_vocabulary_holds_prints_its_name() {
    let name = "zq_cli_headerless_key";
    let dir = tempfile::tempdir().expect("a folder");
    let config = dir.path().join("config");
    let mut vocabulary = Vocabulary::default();
    vocabulary.add(name, None);
    vocabulary
        .save(&vocabulary_file(&config))
        .expect("the vocabulary is kept");

    let open = |hash: u32| {
        let mut bytes = rpf9_head();
        bytes.resize(0x1000, 0);
        bytes.extend(zlib(&headerless_rbm(hash)));
        let archive = dir.path().join(format!("{hash:08x}.rpf"));
        std::fs::write(&archive, &bytes).expect("written");
        let archive = archive.display().to_string();
        run(&config, &["rpf", "scan-open", &archive, "0x1000"])
    };

    let hash = name_hash32(name.as_bytes());
    let (printed, ok) = open(hash);
    assert!(ok, "{printed}");
    assert!(printed.contains("GTA VI RBM, headerless"), "{printed}");
    assert!(
        printed.contains(&format!("hash_{hash:08X}  {name} (from the text)")),
        "{printed}"
    );
    assert!(printed.contains("1 of 1 hash values"), "{printed}");

    // A hash the vocabulary does not hold is counted and named by nothing.
    let (printed, ok) = open(hash ^ 1);
    assert!(ok, "{printed}");
    assert!(printed.contains("0 of 1 hash values"), "{printed}");
    assert!(!printed.contains(name), "{printed}");
}
