//! `openrage-cli rpf scan-open` over a synthetic RPF9 archive: a PSO body in GTA VI's table
//! form, a headerless RBM resource, a registration document and a `KB2n` movie each print what
//! their reader reads, and a resource the scan cannot give a reader with certainty is refused.
//! `dlc inspect`, which reads the registration document, still prints a GTA V pack in its order.
//! `rpf list` and `rpf extract` read such an archive as `rpf scan` and `rpf scan-extract` do, by
//! family, as JSON, stored or inflated, and still read a GTA V pack as they did. `rpf extract --as
//! converted` writes each body as the verb that exports one of its kind does.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its own process, so it
//! runs in a child of this test binary that calls `or_cli::run` between two markers.

use std::io::Write;
use std::process::{Command, ExitCode};

const CHILD_ENV: &str = "OPENRAGE_SCAN_OPEN_ARGS";
const CHILD_TEST: &str = "run_scan_open_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_scan_open_for_a_parent() {
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
    let digit = match code {
        code if code == ExitCode::SUCCESS => 0,
        code if code == ExitCode::from(2) => 2,
        _ => 1,
    };
    let _ = write!(out, "{END}{digit}");
    let _ = out.flush();
}

/// What `openrage-cli <args>` prints, and whether it succeeded.
fn run(args: &[&str]) -> (String, bool) {
    let (printed, _, code) = run_whole(args);
    (printed, code == 0)
}

/// What `openrage-cli <args>` prints, what it says on stderr, and its exit code.
fn run_whole(args: &[&str]) -> (String, String, u8) {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .env("OPENRAGE_SCHEMA", "none")
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}"));
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    let (printed, tail) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the command panicked: {rest}{said}"));
    let code = tail.bytes().next().map_or(1, |digit| digit - b'0');
    (printed.to_string(), said, code)
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// An RPF9 head of layout 4 that holds every measured rule, and its table to 0x1A0.
fn rpf9_head() -> Vec<u8> {
    let toc_end = 0x1A0u32;
    let mut bytes = vec![0x5Au8; toc_end as usize];
    bytes[..0x40].fill(0);
    bytes[..4].copy_from_slice(b"9FPR");
    bytes[0x04..0x08].copy_from_slice(&le32(toc_end));
    bytes[0x08..0x0C].copy_from_slice(&le32(toc_end - 0x10));
    bytes[0x0C..0x10].copy_from_slice(&le32(0x40));
    bytes[0x1C..0x20].copy_from_slice(&le32(0x150));
    bytes[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x24..0x28].copy_from_slice(&le32(1));
    bytes[0x30..0x34].copy_from_slice(&le32(0x10));
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

fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

const ROOT: u32 = 0x1111_0001;
const FIELD: u32 = 0x2222_0000;
/// A u32 member.
const U32: u8 = 0x15;

/// A 32-byte class record of one u32 member whose member table is at `members`.
fn class(members: u64, size: u32) -> [u8; 32] {
    let mut c = [0u8; 32];
    c[0..4].copy_from_slice(&le32(ROOT));
    c[4..8].copy_from_slice(&le32(0x5151_5151));
    c[9] = 3;
    c[16..24].copy_from_slice(&members.to_le_bytes());
    c[24..28].copy_from_slice(&le32(size));
    c[30..32].copy_from_slice(&1u16.to_le_bytes());
    c
}

fn member() -> [u8; 16] {
    let mut m = [0u8; 16];
    m[0..4].copy_from_slice(&le32(FIELD));
    m[8] = U32;
    m
}

/// A PSO document in GTA VI's table form whose root holds one u32, 77.
fn table_form_pso() -> Vec<u8> {
    let mut psin = Vec::new();
    psin.extend(le32(ROOT));
    psin.extend(le32(4));
    psin.extend([0xAA; 8]);
    // A block starts on 16 bytes counted from the section's frame, 8 bytes before its content.
    psin.resize(24, 0);
    psin.extend(le32(77));
    psin.resize(40, 0);
    let mut lsch = class(0xAAAA_AAAA_AAAA_AAAA, 4).to_vec();
    lsch.extend(member());
    let mut lrsc = vec![0u8; 0x50];
    lrsc[0..4].copy_from_slice(b"0DRP");
    lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
    lrsc[0x10..0x14].copy_from_slice(&le32(1));
    lrsc[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes());
    let mut out = Vec::new();
    frame(&mut out, b"PSIN", &psin);
    frame(&mut out, b"LSCH", &lsch);
    frame(&mut out, b"LENM", &[]);
    frame(&mut out, b"LSNS", &le32(0));
    frame(&mut out, b"LGID", &[]);
    frame(&mut out, b"LRSC", &lrsc);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend([0; 4]);
    chks.extend(b"zppp");
    frame(&mut out, b"CHKS", &chks);
    out
}

/// The virtual section of a headerless RBM resource whose root holds one u32, 77, in the block
/// at `block`: 0x5 addresses the virtual section, 0x6 the physical.
fn headerless_rbm(block: u64) -> Vec<u8> {
    const VIRTUAL: u64 = 0x5000_0000;
    let mut v = vec![0u8; 0x100];
    let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    // The block map: one virtual block and no physical one.
    put(0x08, &(VIRTUAL + 0xF0).to_le_bytes());
    put(0xF8, &[1, 0]);
    put(0x10, b"0DRP");
    put(0x14, &0x7Au16.to_le_bytes());
    put(0x20, &le32(1));
    put(0x28, &(VIRTUAL + 0x60).to_le_bytes());
    put(0x30, &(VIRTUAL + 0x80).to_le_bytes());
    put(0x38, &(VIRTUAL + 0xC0).to_le_bytes());
    put(0x48, &[1, 0, 0, 0, 1, 0]);
    put(0x60, &class(VIRTUAL + 0xA0, 4));
    put(0xA0, &member());
    put(0xC0, &le32(ROOT));
    put(0xC4, &le32(4));
    put(0xC8, &block.to_le_bytes());
    put(0xE0, &le32(77));
    v
}

/// The bytes the one packet of [`kb2n`] states it decodes to.
const DECODED: u32 = 7680;

/// A `KB2n` movie of one frame with one stereo 16-bit DCT track at 48,000 Hz, after the 4-byte
/// field GTA VI's heads carry.
fn kb2n() -> Vec<u8> {
    let head = 44;
    let start = head + 4 + 12 + 4;
    let mut packet = vec![0u8; 8];
    packet[..4].copy_from_slice(&le32(DECODED));
    let mut body = le32(packet.len() as u32).to_vec();
    body.extend(&packet);
    body.extend(b"picture");
    let total = (start + body.len()) as u32;
    let mut movie = b"KB2n".to_vec();
    for word in [total - 8, 1, 16, 1, 64, 48, 30, 1, 0, 1] {
        movie.extend(le32(word));
    }
    movie.extend([0xA5; 4]);
    movie.extend([0; 4]);
    movie.extend(48_000u16.to_le_bytes());
    movie.extend(0x7000u16.to_le_bytes());
    movie.extend([0; 4]);
    movie.extend(le32(start as u32));
    movie.extend(body);
    movie
}

const CHANGE_SET: &[u8] = b"<?xml version=\"1.0\"?>\n<CDataFileMgr__ContentChangeSetDef>\
    <id>ccs_test</id><fileList><Item><filename>a.rpf</filename><fileType>RPF_FILE</fileType>\
    </Item></fileList></CDataFileMgr__ContentChangeSetDef>\n";

#[test]
fn each_scanned_body_prints_what_its_reader_reads_and_an_uncertain_resource_is_refused() {
    let bodies = [
        zlib(&table_form_pso()),
        zlib(&headerless_rbm(0x5000_00E0)),
        zlib(&headerless_rbm(0x6000_0000)),
        zlib(CHANGE_SET),
        kb2n(),
        zlib(&headerless_rbm(0x5000_00E0)),
    ];
    let mut bytes = rpf9_head();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        // The last resource has neither a zero header slot nor a page start before it.
        if i == 5 {
            bytes.extend([1; 0x20]);
        }
        offsets.push(bytes.len());
        bytes.extend_from_slice(body);
    }
    let dir = tempfile::tempdir().expect("a folder");
    let archive = dir.path().join("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");
    let archive = archive.display().to_string();
    let open = |i: usize| run(&["rpf", "scan-open", &archive, &format!("{:#X}", offsets[i])]);

    let (pso, ok) = open(0);
    assert!(ok, "{pso}");
    assert!(pso.contains("root fields  1"), "{pso}");

    let (rbm, ok) = open(1);
    assert!(ok, "{rbm}");
    assert!(rbm.contains("GTA VI RBM, headerless"), "{rbm}");
    assert!(rbm.contains("root fields  1"), "{rbm}");
    assert!(rbm.contains("1 classes, 0 enums, 1 blocks"), "{rbm}");
    assert!(
        rbm.contains("block map     1 virtual and 0 physical block(s)"),
        "{rbm}"
    );
    assert!(rbm.contains("0 physical block(s) ending at 0x0"), "{rbm}");

    for refused in [2, 5] {
        let (printed, ok) = open(refused);
        assert!(!ok, "the resource at {:#X}: {printed}", offsets[refused]);
        assert!(!printed.contains("root fields"), "{printed}");
    }

    let (set, ok) = open(3);
    assert!(ok, "{set}");
    assert!(set.contains("change set ccs_test"), "{set}");
    assert!(set.contains("1 RPF_FILE"), "{set}");

    let (movie, ok) = open(4);
    assert!(ok, "{movie}");
    assert!(movie.contains("sound tracks  1"), "{movie}");
    let frames = DECODED / (2 * 2);
    assert!(
        movie.contains(&format!(
            "flags state Bink 2 audio, 2 channel(s), 48000 Hz, {frames} sample frames"
        )),
        "{movie}"
    );
}

/// An RBF document whose root holds one value, 41.
fn rbf() -> Vec<u8> {
    use or_io::meta::{BinaryFormat, MetaInstance, Value};
    let mut root = MetaInstance::new("CTestRoot");
    root.set("count", Value::UInt(41));
    or_io::meta::binary::write_meta_binary(BinaryFormat::Rbf, None, &root).expect("written")
}

#[test]
fn a_scanned_body_leaves_stored_or_as_xml_and_a_scan_extracts_the_bodies_of_a_kind() {
    let bodies = [
        zlib(&headerless_rbm(0x5000_00E0)),
        zlib(&table_form_pso()),
        zlib(CHANGE_SET),
        zlib(&rbf()),
    ];
    let mut bytes = rpf9_head();
    let mut offsets = Vec::new();
    for body in &bodies {
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        offsets.push(format!("{:#X}", bytes.len()));
        bytes.extend_from_slice(body);
    }
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let archive = path("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");

    let (said, ok) = run(&[
        "rpf",
        "scan-body",
        &archive,
        &offsets[0],
        &path("r.rsc"),
        "--stored",
    ]);
    assert!(ok, "{said}");
    let stored = std::fs::read(path("r.rsc")).expect("the stored resource");
    assert_eq!(stored[..bodies[0].len()], bodies[0][..]);
    let (rbm, ok) = run(&["rpf", "scan-open", &path("r.rsc"), "0"]);
    assert!(ok && rbm.contains("root fields  1"), "{rbm}");

    for (at, out) in [(0, "r.xml"), (1, "p.xml"), (3, "f.xml")] {
        let (said, ok) = run(&[
            "rpf",
            "scan-body",
            &archive,
            &offsets[at],
            &path(out),
            "--xml",
        ]);
        assert!(ok, "{said}");
        let xml = std::fs::read_to_string(path(out)).expect("the XML");
        let value = if at == 3 { "41" } else { "77" };
        assert!(
            xml.trim_start().starts_with('<') && xml.contains(value),
            "{xml}"
        );
    }
    let (said, ok) = run(&[
        "rpf",
        "scan-body",
        &archive,
        &offsets[2],
        &path("x"),
        "--xml",
    ]);
    assert!(
        !ok && said.is_empty(),
        "a registration document is no metadata body"
    );

    let (rbf, ok) = run(&["rpf", "scan-open", &archive, &offsets[3]]);
    assert!(
        ok && rbf.contains("zlib rbf") && rbf.contains("root fields  1"),
        "{rbf}"
    );

    let out = path("out");
    let (said, ok) = run(&[
        "rpf",
        "scan-extract",
        &archive,
        &out,
        "--kind",
        "zlib xml",
        "--dry-run",
    ]);
    assert!(ok && said.contains("Would extract 1 body(ies)"), "{said}");
    assert!(
        !std::path::Path::new(&out).exists(),
        "a dry run writes nothing"
    );
    let (said, ok) = run(&["rpf", "scan-extract", &archive, &out, "--kind", "zlib xml"]);
    assert!(ok && said.contains("Extracted 1 body(ies)"), "{said}");
    let written: Vec<_> = std::fs::read_dir(&out)
        .expect("the folder")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(written, [format!("{}.xml", offsets[2])]);
    assert_eq!(
        std::fs::read(dir.path().join("out").join(&written[0])).expect("read"),
        CHANGE_SET
    );
    let (said, ok) = run(&[
        "rpf",
        "scan-extract",
        &archive,
        &out,
        "--role",
        "drawable",
        "--dry-run",
    ]);
    assert!(ok && said.contains("Would extract 0 body(ies)"), "{said}");
}

#[test]
fn a_gta_v_pack_prints_where_it_registers_before_its_change_set_groups() {
    let dir = tempfile::tempdir().expect("a folder");
    let src = dir.path().join("pack");
    std::fs::create_dir(&src).expect("a source folder");
    let setup = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<SSetupData>\
        <deviceName>dlc_test</deviceName><nameHash>TEST</nameHash>\
        <contentChangeSetGroups><Item><NameHash>GROUP_STARTUP</NameHash>\
        <ContentChangeSets><Item>TEST_AUTOGEN</Item></ContentChangeSets></Item>\
        </contentChangeSetGroups><type>EXTRACONTENT_COMPAT_PACK</type></SSetupData>";
    std::fs::write(src.join("setup2.xml"), setup).expect("written");
    let pack = dir.path().join("dlc.rpf").display().to_string();
    let src = src.display().to_string();
    let (made, ok) = run(&["rpf", "create", &src, &pack, "--encryption", "open"]);
    assert!(ok, "{made}");
    let (printed, ok) = run(&["dlc", "inspect", &pack]);
    assert!(ok, "{printed}");
    let at = |line: &str| {
        printed
            .find(line)
            .unwrap_or_else(|| panic!("no {line:?} in {printed}"))
    };
    assert!(at("order") < at("registers against"), "{printed}");
    assert!(at("registers against") < at("group"), "{printed}");
}

/// Where the nested archive of [`listed_archive`] starts, and its document between two missing runs.
const NESTED: u64 = 0x6000;
const PAST_MISSING: u64 = 0x20_0000;

/// A GTA VI archive of five bodies a page apart, a nested archive whose names end it after one
/// document, a nested header with no names, and a document between two runs of zeros a scan
/// counts as missing; and each body an extraction writes, as its offset, its family and its file.
fn listed_archive() -> (Vec<u8>, Vec<(u64, &'static str, String)>) {
    let bodies = [
        (zlib(&headerless_rbm(0x5000_00E0)), "other", "rsc"),
        (zlib(&table_form_pso()), "metadata", "pso"),
        (zlib(CHANGE_SET), "text", "xml"),
        (zlib(&rbf()), "metadata", "ymt"),
        (kb2n(), "movies", "bik"),
    ];
    let mut bytes = rpf9_head();
    let mut listed = Vec::new();
    let mut put = |bytes: &mut Vec<u8>, body: &[u8], family: &'static str, ext: &str| {
        let at = bytes.len() as u64;
        listed.push((at, family, format!("{at:#X}.{ext}")));
        bytes.extend_from_slice(body);
    };
    for (body, family, ext) in bodies {
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        put(&mut bytes, &body, family, ext);
    }
    bytes.resize(NESTED as usize, 0);
    bytes.extend(rpf9_head());
    put(&mut bytes, &zlib(CHANGE_SET), "text", "xml");
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    // The names size the head states.
    bytes.extend([0x33; 0x40]);
    bytes.resize(0x7000, 0);
    bytes.extend(rpf9_head());
    bytes.resize(PAST_MISSING as usize, 0);
    put(&mut bytes, &zlib(CHANGE_SET), "text", "xml");
    bytes.resize(2 * PAST_MISSING as usize, 0);
    (bytes, listed)
}

/// The files of `dir`, by name, with their bytes.
fn files(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .expect("the folder")
        .map(|entry| {
            let entry = entry.expect("an entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            (name, std::fs::read(entry.path()).expect("read"))
        })
        .collect()
}

/// The offsets of the body lines among `listed`, as `rpf scan` prints one.
fn line_offsets(listed: &str) -> Vec<u64> {
    (listed.lines())
        .filter_map(|line| line.trim_start().strip_prefix("0x"))
        .filter_map(|line| u64::from_str_radix(line.split(' ').next()?, 16).ok())
        .collect()
}

#[test]
fn list_and_extract_read_a_gta_vi_archive_as_scan_and_scan_extract_do() {
    let (bytes, listed) = listed_archive();
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let archive = path("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");

    let (scanned, ok) = run(&["rpf", "scan", &archive]);
    assert!(ok && scanned.contains("zlib xml"), "{scanned}");
    assert_eq!(run(&["rpf", "list", &archive]), (scanned, true));

    let (said, ok) = run(&["rpf", "scan-extract", &archive, &path("scanned")]);
    assert!(ok, "{said}");
    let (said, ok) = run(&["rpf", "extract", &archive, &path("extracted")]);
    assert!(ok && said.contains("form          stored"), "{said}");
    let written = files(&dir.path().join("extracted"));
    assert_eq!(written, files(&dir.path().join("scanned")));
    let mut names: Vec<&String> = listed.iter().map(|(_, _, file)| file).collect();
    // The nested archive whose names end it, from its header to their end.
    let nested = format!("{NESTED:#X}.rpf");
    names.push(&nested);
    names.sort();
    assert_eq!(written.keys().collect::<Vec<_>>(), names);
    assert_eq!(
        written[&nested],
        bytes[NESTED as usize..nested_end(&listed)]
    );
}

/// Where the nested archive of [`listed_archive`] ends: with the names after its one document.
fn nested_end(listed: &[(u64, &str, String)]) -> usize {
    let document = listed[5].0 as usize;
    (document + zlib(CHANGE_SET).len()).next_multiple_of(16) + 0x40
}

#[test]
fn a_gta_vi_archive_lists_as_json_and_by_family() {
    use serde_json::Value;
    let (bytes, listed) = listed_archive();
    let dir = tempfile::tempdir().expect("a folder");
    let archive = dir.path().join("data.rpf").display().to_string();
    std::fs::write(&archive, &bytes).expect("written");
    let json = |extra: &[&str]| -> Value {
        let (printed, ok) = run(&[&["rpf", "list", &archive, "--json"][..], extra].concat());
        assert!(ok, "{printed}");
        serde_json::from_str(&printed).unwrap_or_else(|e| panic!("{e}: {printed}"))
    };

    let document = json(&[]);
    assert_eq!(document["archive"], archive.as_str());
    assert_eq!(document["length"], bytes.len());
    let bodies = document["bodies"].as_array().expect("the bodies");
    let body = |at: u64| {
        (bodies.iter().find(|b| b["offset"] == at)).unwrap_or_else(|| panic!("no body at {at:#X}"))
    };
    let (extracted, rows): (Vec<&Value>, Vec<&Value>) =
        bodies.iter().partition(|b| !b["file"].is_null());
    let found: Vec<(u64, &str, String)> = (extracted.iter())
        .map(|b| {
            let text = |key: &str| b[key].as_str().expect("a word");
            let offset = b["offset"].as_u64().expect("an offset");
            (offset, text("kind"), text("file").to_string())
        })
        .collect();
    // The nested archive whose names end it is given the file `rpf extract` writes of it.
    let mut written = listed.clone();
    let at = written.partition_point(|(offset, _, _)| *offset < NESTED);
    written.insert(at, (NESTED, "archives", format!("{NESTED:#X}.rpf")));
    assert_eq!(found, written);
    assert!(rows.iter().all(|b| b["kind"] == "archives"), "{rows:?}");
    assert!(rows.iter().any(|b| b["offset"] == 0), "{rows:?}");
    // The nested header with no names is the one `rpf extract` counts as left out; a names row,
    // which ends an archive that is extracted, is not a header left out.
    let unplaced = (rows.iter())
        .filter(|b| b["offset"] != 0 && b["found_as"] == "archive")
        .count();
    assert_eq!(unplaced, 1, "{rows:?}");
    let out = dir.path().join("out").display().to_string();
    let (said, ok) = run(&["rpf", "extract", &archive, &out, "--dry-run"]);
    let left_out = format!("{unplaced} nested archive(s) left out");
    assert!(ok && said.contains(&left_out), "{said}");

    let document_at = listed[2].0;
    assert_eq!(body(document_at)["found_as"], "zlib xml");
    assert_eq!(body(document_at)["inflated"], CHANGE_SET.len());
    assert_eq!(body(document_at)["stored"], zlib(CHANGE_SET).len());
    assert!(body(document_at)["nested_in"].is_null());
    assert_eq!(body(listed[5].0)["nested_in"], NESTED);
    assert!(body(PAST_MISSING)["nested_in"].is_null());

    let missing = document["missing"].as_array().expect("the missing runs");
    let ends: Vec<&Value> = missing.iter().map(|run| &run["to"]).collect();
    assert_eq!(ends, [PAST_MISSING, bytes.len() as u64], "{missing:?}");
    for run in missing {
        assert!(run["from"].as_u64() < run["to"].as_u64(), "{run}");
        assert!(run["bytes"].as_u64() >= Some(1 << 20), "{run}");
    }

    let metadata = [listed[1].0, listed[3].0];
    let offsets: Vec<u64> = (json(&["--kind", "metadata"])["bodies"].as_array())
        .expect("the bodies")
        .iter()
        .filter_map(|b| b["offset"].as_u64())
        .collect();
    assert_eq!(offsets, metadata);
    let (lines, ok) = run(&["rpf", "list", &archive, "--kind", "metadata"]);
    assert!(
        ok && lines.starts_with("  kind          metadata"),
        "{lines}"
    );
    assert_eq!(line_offsets(&lines), metadata);
}

#[test]
fn extract_writes_a_gta_vi_body_inflated_as_scan_body_does_and_the_bodies_of_a_family() {
    use std::collections::BTreeSet;
    let (bytes, listed) = listed_archive();
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let archive = path("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");

    let (said, ok) = run(&["rpf", "extract", &archive, &path("out"), "--as", "inflated"]);
    assert!(ok && said.contains("form          inflated"), "{said}");
    let written = files(&dir.path().join("out"));
    assert_eq!(
        written.len(),
        listed.len() + 1,
        "the bodies and the nested archive"
    );
    let (said, ok) = run(&[
        "rpf",
        "scan-body",
        &archive,
        &format!("{NESTED:#X}"),
        &path("nested"),
    ]);
    assert!(ok, "{said}");
    let nested = std::fs::read(path("nested")).expect("the nested archive");
    assert_eq!(nested, bytes[NESTED as usize..nested_end(&listed)]);
    assert_eq!(written.get(&format!("{NESTED:#X}.rpf")), Some(&nested));
    for (offset, _, file) in &listed {
        let (said, ok) = run(&[
            "rpf",
            "scan-body",
            &archive,
            &format!("{offset:#X}"),
            &path("body"),
        ]);
        assert!(ok, "{said}");
        let body = std::fs::read(path("body")).expect("the body");
        assert_eq!(written.get(file), Some(&body), "{file}");
    }

    let (said, ok) = run(&["rpf", "extract", &archive, &path("text"), "--kind", "text"]);
    assert!(ok, "{said}");
    let text: BTreeSet<String> = (listed.iter().filter(|l| l.1 == "text"))
        .map(|l| l.2.clone())
        .collect();
    assert_eq!(text.len(), 3);
    let written: BTreeSet<String> = files(&dir.path().join("text")).into_keys().collect();
    assert_eq!(written, text);

    let (said, ok) = run(&[
        "rpf",
        "scan-extract",
        &archive,
        &path("none"),
        "--kind",
        "metadata",
        "--dry-run",
    ]);
    assert!(ok && said.contains("Would extract 2 body(ies)"), "{said}");
}

#[test]
fn a_usage_error_of_the_rpf_verbs_exits_2_and_a_gta_v_pack_lists_and_extracts_as_before() {
    let (bytes, _) = listed_archive();
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let archive = path("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");

    let (out, mesh) = (path("out"), path("out.obj"));
    for verb in [
        &["rpf", "list", &archive][..],
        &["rpf", "extract", &archive, &out],
    ] {
        let (printed, said, code) = run_whole(&[verb, &["--kind", "zlib xml"]].concat());
        assert_eq!((printed.as_str(), code), ("", 2), "{said}");
        for family in or_io::container::rpf::Rpf9Family::ALL {
            assert!(said.contains(family.word()), "{said}");
        }
    }
    for usage in [
        &["rpf", "list"][..],
        &["rpf", "extract", &archive, &out, "--as", "zip"],
        &["rpf", "scan", &archive, "--from"],
        &["rpf", "scan-body", &archive],
        &["rpf", "scan-open", &archive, "here"],
        &["rpf", "scan-extract", &archive],
        &["rpf", "scan-export", &archive, "0x1000", &mesh],
        &["rpf", "scan-anim", &archive],
    ] {
        assert_eq!(run_whole(usage).2, 2, "{usage:?}");
    }
    assert!(!std::path::Path::new(&out).exists(), "a usage error writes");
    assert_eq!(run_whole(&["rpf", "scan-open", &archive, "0x10"]).2, 1);
    for help in [&["help"][..], &["rpf", "help"]] {
        let (_, said, code) = run_whole(help);
        assert_eq!(code, 0);
        assert!(
            said.contains("GTA VI archive") && said.contains("scan-extract"),
            "{said}"
        );
        assert!(said.contains("converted writes"), "{said}");
    }

    let src = dir.path().join("pack");
    std::fs::create_dir(&src).expect("a source folder");
    std::fs::write(src.join("setup2.xml"), CHANGE_SET).expect("written");
    let (pack, src) = (path("dlc.rpf"), src.display().to_string());
    let (made, ok) = run(&["rpf", "create", &src, &pack, "--encryption", "open"]);
    assert!(ok, "{made}");
    let (listed, ok) = run(&["rpf", "list", &pack]);
    assert!(
        ok && listed.starts_with("/\n") && listed.contains("setup2.xml"),
        "{listed}"
    );
    let (said, ok) = run(&["rpf", "extract", &pack, &path("files")]);
    assert!(ok && said.contains("Extracted 1 file(s)"), "{said}");
    assert_eq!(files(&dir.path().join("files"))["setup2.xml"], CHANGE_SET);
}

/// A headerless resource's virtual section, laid out record by record on 16 bytes.
#[derive(Default)]
struct Image(Vec<u8>);

impl Image {
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.0.len().next_multiple_of(16);
        self.0.resize(at + len, 0);
        at
    }

    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A pointer at `at` to `target` of this section.
    fn ptr(&mut self, at: usize, target: usize) {
        self.put(at, &(0x5000_0000 + target as u64).to_le_bytes());
    }

    /// The section as an archive stores it with `physical` after it, each a zlib block, under a
    /// block map of one virtual block the root points at from 0x08.
    fn stored(mut self, physical: &[u8]) -> Vec<u8> {
        let map = self.alloc(16);
        self.put(map + 8, &[1, u8::from(!physical.is_empty())]);
        self.ptr(0x08, map);
        let mut out = zlib(&self.0);
        if !physical.is_empty() {
            out.resize(out.len().next_multiple_of(16), 0);
            out.extend(zlib(physical));
        }
        out
    }

    /// A dictionary's root, the first record, over the one `entry`, keyed by the hash of `name`.
    fn dictionary_over(&mut self, entry: usize, name: &str) {
        let codes = self.alloc(4);
        self.put(
            codes,
            &le32(or_io::gta6::hash::name_hash32(name.as_bytes())),
        );
        let entries = self.alloc(8);
        self.ptr(entries, entry);
        // A word that is no pointer at 0x18, as a dictionary's signature has.
        self.put(0x18, &[1]);
        self.ptr(0x20, codes);
        self.ptr(0x30, entries);
        for counts in [0x28, 0x38] {
            self.put(counts, &[1, 0, 1, 0]);
        }
    }

    /// A drawable root named `name` whose one level holds no model; where it lies.
    fn drawable(&mut self, name: &str) -> usize {
        let root = self.alloc(0xE0);
        let group = self.alloc(0x40);
        self.ptr(root + 0x10, group);
        // Floats where its sphere and its box's minimum lie, as a drawable's signature has.
        for at in (0x20..0x40).step_by(4) {
            self.put(root + at, &1.0f32.to_le_bytes());
        }
        let level = self.alloc(0x10);
        self.ptr(root + 0x50, level);
        self.put(root + 0x7C, &9998.0f32.to_le_bytes());
        let named = self.alloc(name.len() + 1);
        self.put(named, name.as_bytes());
        self.ptr(root + 0xA8, named);
        root
    }
}

/// A texture dictionary of one 4x4 BC1 texture, `zq_glow`, all red, in one 4 KiB tile.
fn texture_dictionary() -> Vec<u8> {
    let mut img = Image::default();
    img.alloc(0x40);
    let t = img.alloc(0x70);
    let mut physical = vec![0u8; 0x1000];
    physical[..4].copy_from_slice(&[0x00, 0xF8, 0x1F, 0x00]);
    img.put(t + 0x08, &le32(physical.len() as u32 / 8));
    img.put(t + 0x0C, &[8, 0]);
    img.put(t + 0x18, &0x6000_0000u64.to_le_bytes());
    img.ptr(t + 0x28, t + 0x50);
    // Its name lies behind eight zero bytes.
    let named = img.alloc(8 + "zq_glow".len() + 1);
    img.put(named + 8, b"zq_glow");
    img.ptr(t + 0x30, named);
    // 4 by 4 by 1, 2D, DXGI 71, in 4 KiB tiles, one level, and the two constants.
    img.put(t + 0x38, &[4, 0, 4, 0, 1, 0, 1, 71, 5, 0, 1]);
    img.put(t + 0x46, &[1, 0]);
    img.put(t + 0x4C, &le32(2));
    img.dictionary_over(t, "zq_glow");
    img.stored(&physical)
}

/// A GTA VI audio container of one mono Opus stream, 0x0B01, of 20 one-frame packets.
fn opus_container() -> Vec<u8> {
    // Each packet a u16 length, a TOC byte and nonzero bytes, all in one 2,048-byte packet.
    let mut data = vec![0u8; 2048];
    let mut at = 0;
    for i in 0..20u8 {
        let packet: Vec<u8> = std::iter::once(0xB8).chain(1..=40 + i).collect();
        data[at..at + 2].copy_from_slice(&(packet.len() as u16).to_le_bytes());
        data[at + 2..at + 2 + packet.len()].copy_from_slice(&packet);
        at += 2 + packet.len();
    }
    let samples = 960 * 20 - 2000;
    let mut record = vec![0u8; 32];
    record[0..4].copy_from_slice(&le32(samples));
    record[4..8].copy_from_slice(&(-1i32).to_le_bytes());
    record[8..10].copy_from_slice(&48_000u16.to_le_bytes());
    record[12..16].copy_from_slice(&le32(samples));
    (record[0x1C], record[0x1D]) = (11, 1);
    // The 2,048-byte packet's first sample, then the last frame's.
    let seek = [le32(0), le32(960 * 19)].concat();
    let chunks = [(0x55u64, data), (0x76, record), (0xA3, seek)];
    // The head, the stream's first-chunk index, two words, its chunk count and id, the chunk table.
    let mut out = b"ADAT".to_vec();
    out.extend([1, 0, 0x21, 0xFF]);
    out.extend([le32(1), le32(0)].concat());
    out.extend([0; 2]);
    out.extend([le32(0x0BAD_F00D), le32(0x0BAD_F00D), le32(3 << 29 | 0x0B01)].concat());
    let table = out.len();
    out.resize(table + 8 * chunks.len(), 0);
    let data_offset = le32(out.len() as u32);
    out[12..16].copy_from_slice(&data_offset);
    for (i, (kind, bytes)) in chunks.iter().enumerate() {
        out.resize(out.len().next_multiple_of(16), 0);
        let row = kind << 56 | (bytes.len() as u64) << 28 | out.len() as u64;
        out[table + 8 * i..][..8].copy_from_slice(&row.to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// `bodies` a page apart behind an RPF9 head, written as `data.rpf` in `dir`: its path, and each
/// body's offset as `rpf extract` names it.
fn archive_of(dir: &std::path::Path, bodies: &[Vec<u8>]) -> (String, Vec<String>) {
    let mut bytes = rpf9_head();
    let mut offsets = Vec::new();
    for body in bodies {
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        offsets.push(format!("{:#X}", bytes.len()));
        bytes.extend_from_slice(body);
    }
    let archive = dir.join("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");
    (archive.display().to_string(), offsets)
}

/// The files under `dir`, each by its path from it, with their bytes.
fn tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut found = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("the folder") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            let inside = tree(&entry.path()).into_iter();
            found.extend(inside.map(|(file, bytes)| (format!("{name}/{file}"), bytes)));
        } else {
            found.insert(name, std::fs::read(entry.path()).expect("read"));
        }
    }
    found
}

#[test]
fn extract_converts_each_gta_vi_body_as_the_verb_of_its_kind_does_and_stores_the_others() {
    let mut drawable = Image::default();
    drawable.drawable("zq_box#dr");
    let mut drawables = Image::default();
    drawables.alloc(0x40);
    let entry = drawables.drawable("zq_box#dr");
    drawables.dictionary_over(entry, "zq_box#dr");
    // A fragment with no bound: a name, a word that is no pointer at 0x30 and its drawable.
    let mut fragment = Image::default();
    fragment.alloc(0x100);
    let inner = fragment.drawable("zq_box#dr");
    fragment.ptr(0x18, inner);
    fragment.put(0x30, &[1]);
    let named = fragment.alloc(8);
    fragment.put(named, b"zq_box");
    fragment.ptr(0xA8, named);
    let bodies = [
        texture_dictionary(),
        drawable.stored(&[]),
        zlib(&table_form_pso()),
        zlib(&rbf()),
        zlib(&headerless_rbm(0x5000_00E0)),
        opus_container(),
        drawables.stored(&[]),
        zlib(CHANGE_SET),
        kb2n(),
        // A resource too short for a signature, which is no RBM either.
        Image(vec![0; 16]).stored(&[]),
        fragment.stored(&[]),
    ];
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let (archive, at) = archive_of(dir.path(), &bodies);
    std::fs::create_dir(path("want")).expect("a folder");
    let want = |name: String| path(&format!("want/{name}"));
    let ran = |args: &[&str]| {
        let (printed, said, code) = run_whole(args);
        assert_eq!(code, 0, "{args:?}: {printed}{said}");
    };

    // What the verb of each body's kind writes of it, under the name the extraction gives it.
    let textures = want(at[0].clone());
    ran(&["rpf", "scan-open", &archive, &at[0], "--export", &textures]);
    for body in [1, 10] {
        let glb = want(format!("{}.glb", at[body]));
        ran(&["rpf", "scan-export", &archive, &at[body], &glb]);
    }
    for offset in &at[2..5] {
        let xml = want(format!("{offset}.xml"));
        ran(&["rpf", "scan-body", &archive, offset, &xml, "--xml"]);
    }
    let container = path("a.awc");
    ran(&["rpf", "scan-body", &archive, &at[5], &container, "--stored"]);
    ran(&["awc", "export", &container, &want(at[5].clone())]);
    for (body, extension) in [(6, "rsc"), (7, "xml"), (8, "bik"), (9, "rsc")] {
        let stored = want(format!("{}.{extension}", at[body]));
        ran(&["rpf", "scan-body", &archive, &at[body], &stored, "--stored"]);
    }
    let want = tree(&dir.path().join("want"));
    let names: Vec<&String> = want.keys().collect();
    let png = format!("{}/zq_glow.png", at[0]);
    let opus = format!("{}/00000B01.opus", at[5]);
    assert_eq!(names.len(), 11, "{names:?}");
    assert!(want[&png].starts_with(b"\x89PNG"), "{names:?}");
    assert!(want[&opus].starts_with(b"OggS"), "{names:?}");

    let out = path("out");
    let converted = ["rpf", "extract", &archive, &out, "--as", "converted"];
    let (said, ok) = run(&[&converted[..], &["--dry-run"]].concat());
    assert!(ok && said.contains("Would extract 11 body(ies)"), "{said}");
    assert!(!std::path::Path::new(&out).exists(), "a dry run writes");
    let (said, ok) = run(&converted);
    assert!(ok && said.contains("form          converted"), "{said}");
    assert!(said.contains("Extracted 11 body(ies)"), "{said}");
    assert!(said.contains("7 converted, 4 stored; 0 error(s)"), "{said}");
    let written = tree(&dir.path().join("out"));
    assert_eq!(written.keys().collect::<Vec<_>>(), names);
    for (name, bytes) in &want {
        assert!(
            written[name] == *bytes,
            "{name} is not what its verb writes"
        );
    }

    let some = path("some");
    let (said, ok) = run(&[
        "rpf",
        "extract",
        &archive,
        &some,
        "--as",
        "converted",
        "--kind",
        "dictionaries",
        "--limit",
        "0x100000",
    ]);
    assert!(ok && said.contains("1 converted, 1 stored"), "{said}");
    let some: Vec<String> = tree(&dir.path().join("some")).into_keys().collect();
    assert_eq!(some, [png, format!("{}.rsc", at[6])]);
}

#[test]
fn a_body_that_does_not_convert_is_named_and_counted_and_the_bodies_after_it_are_written() {
    // The first resource's root block is in a physical section it does not have.
    let bodies = [
        zlib(&headerless_rbm(0x6000_0000)),
        zlib(&table_form_pso()),
        kb2n(),
    ];
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let (archive, at) = archive_of(dir.path(), &bodies);

    let out = path("out");
    let (printed, said, code) = run_whole(&["rpf", "extract", &archive, &out, "--as", "converted"]);
    assert_eq!(code, 1, "{printed}{said}");
    assert!(
        printed.contains("1 converted, 1 stored; 1 error(s)"),
        "{printed}"
    );
    let named = (said.lines()).find(|line| line.contains(&format!("{}.rsc", at[0])));
    assert!(
        named.is_some_and(|line| line.contains("RBM reader")),
        "{said}"
    );

    let xml = path("p.xml");
    let (said, ok) = run(&["rpf", "scan-body", &archive, &at[1], &xml, "--xml"]);
    assert!(ok, "{said}");
    let written = tree(&dir.path().join("out"));
    let names: Vec<&String> = written.keys().collect();
    assert_eq!(
        names,
        [&format!("{}.xml", at[1]), &format!("{}.bik", at[2])]
    );
    assert_eq!(written[names[0]], std::fs::read(xml).expect("the XML"));

    // A limit reaches a converted body's read as it does a stored one's.
    let (printed, said, code) = run_whole(&[
        "rpf",
        "extract",
        &archive,
        &path("limited"),
        "--as",
        "converted",
        "--kind",
        "metadata",
        "--limit",
        "16",
    ]);
    assert_eq!(code, 1, "{printed}{said}");
    assert!(
        printed.contains("0 converted, 0 stored; 1 error(s)"),
        "{printed}"
    );
}

#[test]
fn a_scanned_headerless_metadata_body_takes_its_edited_xml_into_a_loose_resource() {
    use or_io::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Sections};
    use or_io::meta::binary::rbm::gta6::open;
    use or_io::meta::Value;
    use std::io::Cursor;

    // Room after the block map for the slot a loose resource of one block writes there.
    let mut body = headerless_rbm(0x5000_00E0);
    body.resize(0x110, 0);
    let mut bytes = rpf9_head();
    bytes.resize(0x1000, 0);
    bytes.extend(zlib(&body));
    let dir = tempfile::tempdir().expect("a folder");
    let path = |name: &str| dir.path().join(name).display().to_string();
    let archive = path("data.rpf");
    std::fs::write(&archive, &bytes).expect("written");
    let sections = |bytes: Vec<u8>, at: u64| {
        let read = read_rpf9_body(Cursor::new(bytes), at, u64::MAX);
        let (find, read) = read.expect("it reads");
        Rpf9Sections::of(&find, &read).expect("a resource")
    };
    let stored = sections(bytes, 0x1000);
    let value = |s: &Rpf9Sections| {
        let (tree, _) = open(&s.virtual_section, &s.physical).expect("it opens");
        tree.fields[0].1.clone()
    };

    let xml = path("r.xml");
    let (said, ok) = run(&["rpf", "scan-body", &archive, "0x1000", &xml, "--xml"]);
    assert!(ok, "{said}");
    let unedited = std::fs::read_to_string(&xml).expect("the XML");
    let edited = unedited.replacen("value=\"77\"", "value=\"78\"", 1);
    assert_ne!(
        edited, unedited,
        "the root's u32 is written as an attribute"
    );
    let typed = path("e.xml");
    std::fs::write(&typed, &edited).expect("written");

    // The unedited XML writes the stored tree, and the edited one reads back with its edit.
    let same = path("same.rsc");
    let (said, ok) = run(&["rpf", "scan-from-xml", &archive, "0x1000", &xml, &same]);
    assert!(ok, "{said}");
    let out = path("edited.rsc");
    let (said, ok) = run(&["rpf", "scan-from-xml", &archive, "0x1000", &typed, &out]);
    assert!(ok, "{said}");
    let read = |file: &str| sections(std::fs::read(file).expect("the resource"), 0);
    let stored_file = write_rpf9_resource(&stored).expect("written");
    assert_eq!(std::fs::read(&same).expect("the resource"), stored_file);
    assert_eq!(value(&read(&same)), Value::UInt(77));
    assert_eq!(value(&read(&out)), Value::UInt(78));

    // A loose resource takes XML at offset 0, and is never written over.
    let again = path("again.rsc");
    let (said, ok) = run(&["rpf", "scan-from-xml", &out, "0", &typed, &again]);
    assert!(ok, "{said}");
    assert_eq!(value(&read(&again)), Value::UInt(78));
    let (said, ok) = run(&["rpf", "scan-from-xml", &out, "0", &typed, &out]);
    assert!(!ok, "the output is the input: {said}");
    let (said, ok) = run(&["rpf", "scan-from-xml", &archive, "0x0", &typed, &again]);
    assert!(!ok, "an archive's header is no metadata body: {said}");
}
