//! `openrage-cli formats` and `search --verify` over one synthetic file per
//! reader `or_io` has for a GTA V file: each extension's `reads` names the
//! reader that parsed it, rather than `none` or `detected only` for a format
//! the tree reads, and the verify sweep parses every file that is not metadata.
//!
//! ⚠ The census does not try RDR2's readers, GTA VI's `.ttpf` reader or the
//! RPF9 header reader, so it names no reader for those files.
//!
//! ⚠ The command prints with `println!`, which a test cannot read back in its
//! own process, so it runs in a child of this test binary that calls
//! `or_cli::run` between two markers.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode};

use or_io::audio_world_sectors::AudioWorldSectors;
use or_io::cache_loader::CacheFile;
use or_io::container::resource::{Compression, ResourceEnvelope, Rsc7, YSC_FILE_VERSION};
use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::formats::{format, FORMATS};
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::gen9::GrcTextureG9;
use or_io::gta5::pose_db::PoseMatcherDatabase;
use or_io::gta5::{
    CNavMesh, CPathRegion, CVehicleRecording, CharacterCloth, CrClipDictionary,
    CrExpressionsDictionary, CrFrameFilterDictionary, FwWaypointRecordingRoute, GtaDrawable,
    GtaFragType, Gxt2, PgDictionary, PhBoundAny, PtxFxList,
};
use or_io::meta::{write_meta_binary, BinaryFormat, SchemaLibrary};
use or_io::nav_precalc::NavPrecalc;
use or_io::rel::{RelFile, RelIndex};
use or_io::social_club_metadata::SocialClubMetadata;
use or_io::water_map::WaterMap;
use or_io::world_blanket::WorldBlanket;
use or_io::Generation;
use serde_json::Value;

const CHILD_ENV: &str = "OPENRAGE_FORMATS_CENSUS_ARGS";
const CHILD_TEST: &str = "run_the_census_for_a_parent";
const SEPARATOR: char = '\u{1f}';
const BEGIN: &str = "\u{1e}stdout begins\u{1e}";
const END: &str = "\u{1e}stdout ends\u{1e}";

const PSC: &str = r#"<ParserSchema>
  <structdef type="CEntityDef"><u32 name="flags"/></structdef>
</ParserSchema>"#;

/// Not a test of its own: the body a parent's child process runs.
#[test]
fn run_the_census_for_a_parent() {
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

/// `openrage-cli formats --json` over `game`, with `schema` as the snapshot.
fn census(game: &Path, schema: &Path) -> Value {
    json_of(
        &["formats", "--json", "--game", &game.display().to_string()],
        schema,
    )
}

/// `openrage-cli <args>`, whose stdout is JSON, with `schema` as the snapshot.
fn json_of(args: &[&str], schema: &Path) -> Value {
    let args = args.join(&SEPARATOR.to_string());
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args)
        .env("OPENRAGE_SCHEMA", schema)
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
    assert!(tail.starts_with('1'), "the command failed: {printed}");
    serde_json::from_str(printed).unwrap_or_else(|e| panic!("{e}: {printed}"))
}

fn seal(version: u32, payload: &[u8]) -> Vec<u8> {
    let seal = Rsc7::invent_single_chunk_seal(version, payload.len()).expect("a seal");
    let mut payload = payload.to_vec();
    payload.resize(seal.total_size() as usize, 0);
    Rsc7::reseal(&payload, &seal, Compression::fast()).expect("sealed")
}

fn fxc_string(s: &str) -> Vec<u8> {
    let mut out = vec![s.len() as u8 + 1];
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    out
}

/// A `.fxc` with its header and six empty program banks.
fn fxc() -> Vec<u8> {
    let mut out = b"rgxe".to_vec();
    out.extend_from_slice(&0x59u32.to_le_bytes());
    out.push(0);
    for _ in 0..6 {
        out.push(1);
        out.extend_from_slice(&fxc_string("NULL"));
        out.extend_from_slice(&[0; 6]);
    }
    out.extend_from_slice(&[0; 5]);
    out
}

/// An `.awc` with no stream, in the clear.
fn awc() -> Vec<u8> {
    let mut out = b"ADAT".to_vec();
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes());
    out
}

/// Enhanced's shader container, which ships under `.awc`: no program, no effect.
fn sga() -> Vec<u8> {
    let mut out = b"SGD2".to_vec();
    out.extend_from_slice(&[0; 4 * 9]);
    out
}

/// A script with no code, statics, natives, name or strings.
fn ysc() -> Vec<u8> {
    seal(YSC_FILE_VERSION, &[0; 0x80])
}

fn rel(rel_type: u32) -> Vec<u8> {
    RelFile {
        rel_type,
        data: Vec::new(),
        names: Vec::new(),
        index: RelIndex::Hashed(Vec::new()),
        hash_offsets: Vec::new(),
        pack_offsets: Vec::new(),
    }
    .save()
    .expect("an empty .rel saves")
}

/// A `.gfx` of one empty frame: a zero-bit frame rectangle, 30 fps, then the end tag.
fn gfx() -> Vec<u8> {
    let body = [0x00, 0x00, 0x1E, 0x01, 0x00, 0x00, 0x00];
    let mut out = b"GFX\x08".to_vec();
    out.extend_from_slice(&(8 + body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// A binary `.srl` of one frame that requests one name and releases none.
fn srl() -> Vec<u8> {
    [1u32, 0x1234_5678, 1, 0, 0, 1, 0]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect()
}

/// A one-cell `heightmap.dat` stored as the whole grid.
fn height_map() -> Vec<u8> {
    or_io::height_map::HeightMap {
        version: 1,
        flags: 0,
        num_cols: 1,
        num_rows: 1,
        bounds_min: [0.0; 3],
        bounds_max: [1.0; 3],
        max: vec![9],
        min: vec![1],
        water_mask: Vec::new(),
    }
    .save()
    .expect("a height map saves")
}

/// A `distantlights.dat` of the 16-block grid with no group.
fn distant_lights() -> Vec<u8> {
    use or_io::distant_lights::{DistantLights, BLOCKS};
    let cells = BLOCKS * BLOCKS;
    DistantLights {
        block_first_group: vec![0; cells],
        block_num_coastal_groups: vec![0; cells],
        block_num_inland_groups: vec![0; cells],
        points: Vec::new(),
        groups: Vec::new(),
    }
    .save()
    .expect("distant lights save")
}

/// A `.mrf` of no nodes: the header and the empty request and flag lists.
fn mrf() -> Vec<u8> {
    or_io::mrf::MvNetworkDef {
        version: or_io::mrf::VERSION,
        xrefs: Vec::new(),
        requests: Vec::new(),
        flags: Vec::new(),
        nodes: Vec::new(),
        string_table: Vec::new(),
    }
    .save()
    .expect("an empty network saves")
}

fn saved_dat(bytes: or_io::Result<Vec<u8>>) -> Vec<u8> {
    bytes.expect("a default table saves")
}

fn saved<T: File>(value: T) -> Vec<u8> {
    value.to_bytes().expect("a default root saves")
}

/// The reader name the census gives a declared format's owned root at `target`.
fn root(ext: &str, target: Generation) -> String {
    let root = format(ext)
        .expect("declared")
        .root_name(target)
        .expect("a root at that target");
    format!("{root} ({})", target.edition_label())
}

/// A `CEntityDef` in binary metadata's `format`.
fn binary_meta(library: &SchemaLibrary, format: BinaryFormat) -> Vec<u8> {
    let instance = or_io::meta::instance::parse_meta(
        library,
        br#"<CEntityDef><flags value="3"/></CEntityDef>"#,
    )
    .expect("an instance");
    let layout =
        or_io::meta::canonical::layout_for(library, &instance, format, None).expect("a layout");
    write_meta_binary(format, Some(&layout), &instance).expect("binary metadata")
}

/// Every synthetic file, and per extension each reader the census must name
/// with the count of files it parsed.
fn staged(dir: &Path, library: &SchemaLibrary) -> BTreeMap<&'static str, BTreeMap<String, u64>> {
    let gen8 = Generation::Gen8;
    let gen9 = Generation::Gen9;
    let files: Vec<(&str, Vec<u8>, String)> =
        vec![
        ("a.yfd", saved(CrFrameFilterDictionary::default()), root("yfd", gen8)),
        ("a.yed", saved(CrExpressionsDictionary::default()), root("yed", gen8)),
        ("a.ynd", saved(CPathRegion::default()), root("ynd", gen8)),
        ("a.ynv", saved(CNavMesh::default()), root("ynv", gen8)),
        ("a.ytd", saved(PgDictionary::<GrcTexture>::default()), root("ytd", gen8)),
        ("b.ytd", saved(PgDictionary::<GrcTextureG9>::default()), root("ytd", gen9)),
        ("a.ybn", saved(PhBoundAny::PhBoundSphere(Default::default())), root("ybn", gen8)),
        ("a.ybd", saved(PgDictionary::<PhBoundAny>::default()), root("ybd", gen8)),
        ("a.yld", saved(PgDictionary::<CharacterCloth>::default()), root("yld", gen8)),
        ("a.ycd", saved(CrClipDictionary::default()), root("ycd", gen8)),
        ("a.ydr", saved(GtaDrawable::<Gen8>::default()), root("ydr", gen8)),
        ("b.ydr", saved(GtaDrawable::<Gen9>::default()), root("ydr", gen9)),
        ("a.ydd", saved(PgDictionary::<GtaDrawable<Gen8>>::default()), root("ydd", gen8)),
        ("b.ydd", saved(PgDictionary::<GtaDrawable<Gen9>>::default()), root("ydd", gen9)),
        ("a.yft", saved(GtaFragType::<Gen8>::default()), root("yft", gen8)),
        ("b.yft", saved(GtaFragType::<Gen9>::default()), root("yft", gen9)),
        ("a.ypt", saved(PtxFxList::<Gen8>::default()), root("ypt", gen8)),
        ("b.ypt", saved(PtxFxList::<Gen9>::default()), root("ypt", gen9)),
        ("a.yvr", saved(CVehicleRecording::default()), root("yvr", gen8)),
        ("a.ywr", saved(FwWaypointRecordingRoute::default()), root("ywr", gen8)),
        ("a.awc", awc(), "AwcFile".into()),
        ("sga_win32_60_final.awc", sga(), "SgaFile".into()),
        ("a.rel", rel(151), "RelFile".into()),
        ("a.dat4", rel(4), "RelFile".into()),
        ("a.dat10", rel(10), "RelFile".into()),
        ("a.dat15", rel(15), "RelFile".into()),
        ("a.dat16", rel(16), "RelFile".into()),
        ("a.dat22", rel(22), "RelFile".into()),
        ("a.dat54", rel(54), "RelFile".into()),
        ("a.dat149", rel(149), "RelFile".into()),
        ("a.dat150", rel(150), "RelFile".into()),
        ("a.dat151", rel(151), "RelFile".into()),
        ("a.gfx", gfx(), "GfxMovie".into()),
        ("a.srl", srl(), "StreamingRequestList".into()),
        ("heightmap.dat", height_map(), "HeightMap".into()),
        ("distantlights.dat", distant_lights(), "DistantLights".into()),
        ("waterheight.dat", WaterMap::default().save().expect("saves"), "WaterMap".into()),
        (
            "worldblanket.dat",
            WorldBlanket { cells: vec![0, 1, 0, 0] }.save(),
            "WorldBlanket".into(),
        ),
        ("navprecalc.dat", NavPrecalc::default().save().expect("saves"), "NavPrecalc".into()),
        ("a.mrf", mrf(), "MvNetworkDef".into()),
        // A `.dat` is many formats, so these three are named by file name.
        ("gta5_cache_y.dat", saved_dat(CacheFile::default().save()), "CacheFile".into()),
        (
            "mp2023_01_3100635916_cache_y.dat",
            saved_dat(CacheFile::default().save()),
            "CacheFile".into(),
        ),
        (
            "metadata.dat",
            saved_dat(
                SocialClubMetadata {
                    format_word: 1,
                    built: "2024-02-01T15:54:51".into(),
                    body: vec![2, 4, 4, 0],
                }
                .save(),
            ),
            "SocialClubMetadata".into(),
        ),
        (
            "audioworldsectorsinfo.dat",
            saved_dat(AudioWorldSectors::default().save()),
            "AudioWorldSectors".into(),
        ),
        ("a.dds", or_io::dds::Dds::default().to_bytes().expect("a DDS"), "Dds".into()),
        (
            "a.dat151.nametable",
            or_io::gta5::data::nametable::write(&["one", "two"]),
            "NameTable".into(),
        ),
        (
            "a.sps",
            b"shader normal.fxc\n__rage_drawbucket {\n  int 0\n}\n".to_vec(),
            "SpsPreset".into(),
        ),
        ("a.gxt2", saved(Gxt2::new()), "Gxt2".into()),
        ("a.ypdb", saved(PoseMatcherDatabase::default()), "PoseMatcherDatabase".into()),
        ("a.fxc", fxc(), "GrcEffect".into()),
        ("a.ysc", ysc(), "ScrProgram".into()),
        (
            "a.meta",
            br#"<?xml version="1.0" encoding="UTF-8"?><CEntityDef><flags value="1"/></CEntityDef>"#
                .to_vec(),
            "meta XML".into(),
        ),
        (
            "a.xml",
            br#"<CEntityDef><flags value="2"/></CEntityDef>"#.to_vec(),
            "meta XML".into(),
        ),
        // Known by its name among the `.xml` that is metadata.
        (
            "water.xml",
            br#"<WaterData><WaterQuads><Item><minX value="0" /><maxX value="8" /><minY value="0" /><maxY value="4" /></Item></WaterQuads></WaterData>"#
                .to_vec(),
            "CWaterData".into(),
        ),
        ("a.ymt", binary_meta(library, BinaryFormat::Pso), "meta PSO".into()),
        ("b.ymt", binary_meta(library, BinaryFormat::Rbm), "meta RBM".into()),
        ("c.ymt", binary_meta(library, BinaryFormat::Rbf), "meta RBF".into()),
    ];

    let mut expected: BTreeMap<&'static str, BTreeMap<String, u64>> = BTreeMap::new();
    for (name, bytes, reader) in files {
        std::fs::write(dir.join(name), bytes).expect("staged");
        let ext = name.rsplit_once('.').expect("an extension").1;
        *expected.entry(ext).or_default().entry(reader).or_default() += 1;
    }
    expected
}

#[test]
fn the_census_names_the_reader_that_parsed_each_synthetic_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let game = dir.path().join("game");
    std::fs::create_dir(&game).expect("game dir");
    let library = or_io::meta::build_library([("t.psc", PSC.as_bytes())]).expect("a library");
    let schema = dir.path().join("schema_snapshot.bin");
    std::fs::write(
        &schema,
        or_io::meta::encode_snapshot(&library).expect("encoded"),
    )
    .expect("snapshot");

    let expected = staged(&game, &library);
    for declared in FORMATS {
        for target in declared.targets() {
            let reader = root(declared.ext, target);
            assert!(
                expected
                    .get(declared.ext)
                    .is_some_and(|readers| readers.contains_key(&reader)),
                "no synthetic .{} stages {reader}",
                declared.ext
            );
        }
    }
    std::fs::write(game.join("a.bik"), b"BIKi\0\0\0\0").expect("staged");

    let report = census(&game, &schema);
    let rows: BTreeMap<String, &Value> = report["extensions"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| (row["ext"].as_str().expect("ext").to_string(), row))
        .collect();

    let mut wrong = Vec::new();
    for (ext, readers) in &expected {
        let Some(row) = rows.get(*ext) else {
            wrong.push(format!(".{ext}: no row"));
            continue;
        };
        let got: BTreeMap<String, u64> =
            serde_json::from_value(row["readers"].clone()).unwrap_or_default();
        if &got != readers || row["refused"] != 0 {
            wrong.push(format!(
                ".{ext}: reads `{}`, readers {got:?}, refused {}; expected {readers:?}",
                row["reads"], row["refused"]
            ));
        }
    }
    let bik = &rows["bik"]["reads"];
    if bik != "detected only" {
        wrong.push(format!(".bik: reads {bik}, expected \"detected only\""));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Metadata, `.srl` among it, is the `meta` command's to sweep, not `verify`'s.
const METADATA: &[&str] = &["meta", "xml", "ymt", "srl"];

#[test]
fn the_verify_sweep_parses_every_synthetic_file_but_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let game = dir.path().join("game");
    std::fs::create_dir(&game).expect("game dir");
    let library = or_io::meta::build_library([("t.psc", PSC.as_bytes())]).expect("a library");
    let expected = staged(&game, &library);
    std::fs::write(game.join("a.bik"), b"BIKi\0\0\0\0").expect("staged");

    let game = game.display().to_string();
    let report = json_of(
        &["search", "--verify", "--json", "--game", &game],
        Path::new("none"),
    );
    let swept: BTreeMap<String, (u64, u64)> = report["byExtension"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| {
            let count = |key: &str| row[key].as_u64().expect("a count");
            let ext = row["ext"].as_str().expect("ext").to_string();
            (ext, (count("attempted"), count("refused")))
        })
        .collect();

    let mut wrong = Vec::new();
    for (ext, readers) in &expected {
        // A water file is the one `.xml` the sweep parses.
        let swept_here = |reader: &str| !METADATA.contains(ext) || reader == "CWaterData";
        let files: u64 = (readers.iter())
            .filter(|(reader, _)| swept_here(reader))
            .map(|(_, count)| count)
            .sum();
        let want = (files > 0).then_some((files, 0));
        let got = swept.get(*ext).copied();
        if got != want {
            wrong.push(format!(
                ".{ext}: swept {got:?} (attempted, refused); expected {want:?}"
            ));
        }
    }
    if let Some(bik) = swept.get("bik") {
        wrong.push(format!(".bik: swept {bik:?}, though nothing reads it"));
    }
    assert!(wrong.is_empty(), "{}\n{report}", wrong.join("\n"));
}
