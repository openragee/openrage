//! `openrage-cli ttpf info` and `ttpf export` over GTA VI texture packs built here, through a
//! made-up tables file with two entries changed over: a listing that counts blocks against the
//! file fails the file for that pack, keeps that, and calls no texture of the pack exact from
//! then on; a check says the largest count it held and one failing check's own count; and with
//! no tables kept, a texture whose coded body was not read is not called exact.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so it
//! runs in a child of this test binary that calls `or_cli::run` between two markers.

use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode};

use or_io::gta6::bcpack::made_up::{bc7_body, bc7_entries_swapped, tables_file};

const CHILD_ENV: &str = "OPENRAGE_TTPF_EXACT_ARGS";
const CHILD_TEST: &str = "run_ttpf_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_ttpf_for_a_parent() {
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

/// What `openrage-cli <args>` prints with its configuration kept under `config`, what it says of
/// an error after it, and whether it succeeded.
fn run(config: &Path, args: &[&str]) -> (String, bool) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_CONFIG_DIR", config)
        .env("OPENRAGE_SCHEMA", "none")
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
    let said = format!("{printed}{}", String::from_utf8_lossy(&output.stderr));
    (said, tail.starts_with('1'))
}

/// A texture of a made-up pack: its size, its format code, its tile mode, its mips, and its
/// bodies in the order of their rows, each a codec kind, the bytes stored and the size they
/// decode to.
struct Spec {
    width: u32,
    height: u32,
    code: u32,
    tile_mode: u32,
    mips: u32,
    bodies: Vec<(u8, Vec<u8>, u64)>,
}

/// A pack of `textures`, laid out as the container is measured to be: the table of rows, the
/// directory in the first row, then each body at a 256-byte boundary and a sentinel row.
fn pack(textures: &[Spec]) -> Vec<u8> {
    let put = |file: &mut [u8], at: usize, word: u32| {
        file[at..at + 4].copy_from_slice(&word.to_le_bytes());
    };
    let rows_n = 2 + textures.iter().map(|t| t.bodies.len()).sum::<usize>();
    let directory_at = (16 + 8 * rows_n).next_multiple_of(0x100);
    let (pool, directory_len) = (0x40, 0x1000);
    let mut file = vec![0u8; directory_at + directory_len];
    file[directory_at..directory_at + 4].copy_from_slice(b"ETPF");
    put(&mut file, directory_at + 4, 0x27A);
    put(&mut file, directory_at + 8, textures.len() as u32);
    put(&mut file, directory_at + 12, pool);
    let keys = directory_at + 16 + 16 * textures.len();
    for k in 0..textures.len() {
        put(&mut file, keys + 8 * k, 0x1234 + k as u32);
    }
    // A row: where it lies in the file, its codec kind, its virtual address and its pad.
    let mut rows: Vec<(usize, u8, u64, usize)> = vec![(directory_at, 0, 0, 0)];
    let mut va = directory_len as u64;
    for (k, texture) in textures.iter().enumerate() {
        let entry = directory_at + 16 + 16 * k;
        let size = (texture.width - 1) | (texture.height - 1) << 14 | texture.mips << 28;
        put(&mut file, entry, size);
        put(
            &mut file,
            entry + 4,
            texture.code | 1 << 21 | texture.tile_mode << 24,
        );
        put(&mut file, entry + 8, (va >> 12) as u32);
        for (kind, stored, decoded) in &texture.bodies {
            let offset = file.len();
            file.extend(stored);
            file.resize(file.len().next_multiple_of(0x100), 0);
            rows.push((offset, *kind, va, file.len() - offset - stored.len()));
            va += decoded;
        }
    }
    let (_, kind, _, pad) = rows[rows.len() - 1];
    rows.push((file.len(), kind, va, pad));
    file[..4].copy_from_slice(b"RDS5");
    put(&mut file, 4, rows_n as u32);
    put(&mut file, 8, va as u32);
    file[15] = 0x16;
    for (i, &(offset, kind, va, pad)) in rows.iter().enumerate() {
        let page = (va >> 12) as u32;
        let low = (offset >> 8) as u32 | u32::from(kind) << 26 | (page & 3) << 30;
        put(&mut file, 16 + 8 * i, low);
        put(&mut file, 20 + 8 * i, page >> 2 | (pad as u32) << 20);
    }
    file
}

/// The blocks of [`bc7`]'s one level.
const BLOCKS: usize = 512;

/// A 128x64 BC7 texture of one level, stored through the coder: [`BLOCKS`] made-up blocks, the
/// first `from_presets` of them each a difference from a preset entry.
fn bc7(from_presets: usize) -> Spec {
    Spec {
        width: 128,
        height: 64,
        code: 0x310,
        tile_mode: 2,
        mips: 1,
        bodies: vec![(7, bc7_body(BLOCKS, from_presets), 16 * BLOCKS as u64)],
    }
}

/// The line of a listing or an export that is texture `index`'s.
fn line_of(printed: &str, index: usize) -> &str {
    let found = printed.lines().find(|line| {
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or_default();
        first == index.to_string() || first.ends_with(&format!("{index}.dds"))
    });
    found.unwrap_or_else(|| panic!("no line for texture {index} in:\n{printed}"))
}

#[test]
fn a_listing_that_counts_blocks_against_the_supplied_tables_fails_them_for_the_pack_and_keeps_it() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let config = dir.path().join("config");
    let text = |path: &Path| path.to_string_lossy().into_owned();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
        std::fs::write(&path, bytes).expect("written");
        text(&path)
    };

    // The tables supplied are a made-up file with the first two entries of BC7's list changed
    // over: a damaged file, which reads, and which only a block that reads one of the two shows.
    let tables = write("damaged.tables", &bc7_entries_swapped(&tables_file()));
    let (printed, ok) = run(&config, &["ttpf", "tables", &tables]);
    assert!(ok, "{printed}");

    // A folder whose one pack reads neither entry: a whole check holds the file's BC7 tables.
    let held = write("held/one.ttpf", &pack(&[bc7(0)]));
    let folder = text(&dir.path().join("held"));
    let (printed, ok) = run(&config, &["ttpf", "tables", "--check", &folder]);
    assert!(ok, "the check holds: {printed}");
    assert!(
        printed.contains(&format!("A check held {BLOCKS} BC7 blocks")),
        "{printed}"
    );
    let (printed, ok) = run(&config, &["ttpf", "info", &held]);
    assert!(ok, "{printed}");
    assert!(line_of(&printed, 0).ends_with(", exact"), "{printed}");

    // A pack never checked: texture 0 reads both entries, and texture 1 reads neither. The
    // listing counts texture 0's two blocks against the file, and calls neither texture exact:
    // texture 1 read through the same file.
    let two = write("other/two.ttpf", &pack(&[bc7(2), bc7(0)]));
    let against = format!("2 of {} coded blocks", 2 * BLOCKS);
    let listed_as_failed = |printed: &str| {
        for index in [0, 1] {
            let line = line_of(printed, index);
            assert!(!line.contains(", exact"), "texture {index}: {printed}");
            assert!(line.contains("NOT exact"), "texture {index}: {printed}");
        }
        assert!(printed.contains(&against), "{printed}");
    };
    let (printed, ok) = run(&config, &["ttpf", "info", &two]);
    assert!(ok, "{printed}");
    listed_as_failed(&printed);

    // That is kept with the pack: a later listing says it before any texture is read, and an
    // export of texture 1 alone, which counts nothing against the file, says it of texture 1.
    let (printed, ok) = run(&config, &["ttpf", "info", &two]);
    assert!(ok, "{printed}");
    listed_as_failed(&printed);
    assert!(
        printed.contains("NOT this coder's tables for this pack"),
        "{printed}"
    );
    let out = text(&dir.path().join("out"));
    let export = [
        "ttpf", "export", &two, &out, "--index", "1", "--format", "dds",
    ];
    let (printed, ok) = run(&config, &export);
    assert!(ok, "{printed}");
    assert!(line_of(&printed, 1).contains("NOT exact"), "{printed}");
    assert!(printed.contains(&against), "{printed}");

    // An export is what counts the blocks of a pack nothing has listed: it words texture 1
    // after texture 0 is counted, and the listing after it says what the export kept.
    let three = write("other/three.ttpf", &pack(&[bc7(2), bc7(0)]));
    let (printed, ok) = run(
        &config,
        &["ttpf", "export", &three, &out, "--format", "dds"],
    );
    assert!(ok, "{printed}");
    for index in [0, 1] {
        assert!(line_of(&printed, index).contains("NOT exact"), "{printed}");
    }
    assert!(printed.contains(&against), "{printed}");
    let (printed, ok) = run(&config, &["ttpf", "info", &three]);
    assert!(ok, "{printed}");
    listed_as_failed(&printed);

    // The verdict is that pack's: the pack the check held still reads as it did, and the
    // check, run again, still holds.
    let (printed, ok) = run(&config, &["ttpf", "info", &held]);
    assert!(ok, "{printed}");
    assert!(line_of(&printed, 0).ends_with(", exact"), "{printed}");
    let (printed, ok) = run(&config, &["ttpf", "tables", "--check", &folder]);
    assert!(ok, "{printed}");
}

#[test]
fn a_check_says_the_largest_count_it_held_and_one_failing_check_s_own_count() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let config = dir.path().join("config");
    let folder = |name: &str, textures: &[Spec]| {
        let folder = dir.path().join(name);
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(folder.join("made_up.ttpf"), pack(textures)).expect("written");
        folder.to_string_lossy().into_owned()
    };
    let tables = dir.path().join("damaged.tables");
    std::fs::write(&tables, bc7_entries_swapped(&tables_file())).expect("written");
    let (said, ok) = run(&config, &["ttpf", "tables", &tables.to_string_lossy()]);
    assert!(ok, "{said}");
    let check = |folder: &str| run(&config, &["ttpf", "tables", "--check", folder]);

    // Two folders that hold, the larger checked first: what is said to be held of BC7 is the
    // largest check of it, not the last.
    let large = folder("large", &[bc7(0), bc7(0)]);
    let small = folder("small", &[bc7(0)]);
    let held = format!("A check held {} BC7 blocks", 2 * BLOCKS);
    for holds in [&large, &small] {
        let (said, ok) = check(holds);
        assert!(ok && said.contains(&held), "{said}");
    }

    // Two folders that fail, the one that counts more checked first: the count said is that
    // check's own, which a check that counts fewer does not replace, and the failure stands
    // for a folder that holds.
    let worse = folder("worse", &[bc7(2), bc7(2)]);
    let bad = folder("bad", &[bc7(2)]);
    let found = format!("a check found 4 of {} coded blocks", 2 * BLOCKS);
    for fails in [&worse, &bad, &small] {
        let (said, ok) = check(fails);
        assert!(!ok && said.contains(&found), "{said}");
    }
}

#[test]
fn a_texture_whose_coded_body_was_not_read_says_so_and_is_not_called_exact() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let config = dir.path().join("config");
    // A 256x128 BC5 texture of two levels: the top one stored plainly, a 4 KiB tile to each of
    // its rows, and the one under it through the coder, which reads through no table kept.
    let plain = Spec {
        width: 256,
        height: 128,
        code: 0x298,
        tile_mode: 2,
        mips: 2,
        bodies: vec![
            (5, vec![0x31; 0x234], 0x2000),
            (0, vec![0x5A; 0x8000], 0x8000),
        ],
    };
    let path = dir.path().join("plain_top.ttpf");
    std::fs::write(&path, pack(&[plain])).expect("written");
    let path = path.to_string_lossy().into_owned();

    let (printed, ok) = run(&config, &["ttpf", "info", &path]);
    assert!(ok, "{printed}");
    let line = line_of(&printed, 0);
    assert!(line.contains("yes, 1 mip(s)"), "{printed}");
    assert!(!line.contains(", exact"), "{printed}");
    assert!(
        line.contains("its plain levels were read and its coded levels were not"),
        "{printed}"
    );

    let out = dir.path().join("out").to_string_lossy().into_owned();
    let (printed, ok) = run(&config, &["ttpf", "export", &path, &out, "--format", "dds"]);
    assert!(ok, "{printed}");
    assert!(
        line_of(&printed, 0).contains("its plain levels were read"),
        "{printed}"
    );
}
