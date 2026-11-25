//! A `.ymt`, `.ymap`, `.ytyp` or `.meta` becomes OpenRage's own canonical XML beside the file it came
//! from, and compiles back through the type table of that file; one whose XML would not come back to
//! a fixed point with every value is kept as a compiled file, with the reason.

use std::fs;
use std::path::{Path, PathBuf};

use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::canonical;
use or_io::meta::instance::HashedString;
use or_io::meta::{
    build_library, detect_binary_format, write_meta_binary, BinaryFormat, Diagnostics,
    MetaInstance, SchemaLibrary, Value,
};
use or_source::asset::{sha256_hex, Source};
use or_source::cook::{cook_source, describe, Cooker, Recook};
use or_source::game_import::{import, kind_of, Facts, Input, Options, Report, Verdict};
use or_source::metadata;
use or_source::models::Extraction;
use or_test_support::{skip_corpus, skip_schema};

/// Every name here is one the default hash index carries, so a read resolves it and the synthetic
/// file decodes to the tree it was written from.
const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <Vector3 name="position"/>
    <float name="scaleXY"/>
    <s32 name="parentIndex"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <u32 name="contentFlags"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;

const RBF_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CCreatureMetaData>
  <pedCompExpressions>
    <Item>
      <pedCompID value="6"/>
      <name>a string</name>
      <offset x="0.0" y="0.5" z="1.0"/>
      <enabled value="true"/>
    </Item>
  </pedCompExpressions>
</CCreatureMetaData>
"#;

fn entity(archetype: &str) -> MetaInstance {
    let mut e = MetaInstance::new("CEntityDef");
    e.set("archetypeName", Value::Hash(HashedString::new(archetype)));
    e.set("flags", Value::UInt(32));
    e.set("position", Value::Vec3([1.0, -2.5, 3.25]));
    e.set("scaleXY", Value::Float(1.0));
    e.set("parentIndex", Value::Int(-1));
    e
}

/// A map in `format`, laid out as an import lays out.
fn synthetic(format: BinaryFormat) -> Vec<u8> {
    if format == BinaryFormat::Rbf {
        return canonical::import(&SchemaLibrary::default(), RBF_XML.as_bytes(), format, None)
            .expect("the RBF imports");
    }
    let library = build_library([("inline.psc", PSC.as_bytes())]).expect("the schema parses");
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new("cs1_02")));
    root.set("contentFlags", Value::UInt(65));
    root.set(
        "entities",
        Value::Array(vec![
            Value::Pointer(Some(Box::new(entity("prop_bin_01a")))),
            Value::Pointer(Some(Box::new(entity("prop_bin_02a")))),
        ]),
    );
    let layout = canonical::layout_for(&library, &root, format, None).expect("lays out");
    write_meta_binary(format, Some(&layout), &root).expect("encodes")
}

fn how(file: &str) -> Extraction<'_> {
    Extraction {
        from_game: true,
        virtual_path: Some(file),
        textures: false,
        skeleton: None,
    }
}

fn extract(parent: &Path, bytes: &[u8], name: &str, extension: &str) -> PathBuf {
    metadata::extract(bytes, name, extension, parent, &how("x64a.rpf/x"))
        .unwrap_or_else(|problems| panic!("{name}.{extension}: {problems:#?}"))
}

fn compile(dir: &Path) -> Vec<u8> {
    metadata::compile(&Source::open(dir).unwrap())
        .unwrap_or_else(|problems| panic!("{}: {problems:#?}", dir.display()))
        .bytes
}

/// The `contentFlags` a map holds.
fn content_flags(bytes: &[u8]) -> u64 {
    let mut diagnostics = Diagnostics::lenient();
    let (root, _) = parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut diagnostics)
        .expect("the file reads");
    match root.get("contentFlags") {
        Some(Value::UInt(flags)) => *flags,
        other => panic!("contentFlags is {other:?}"),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn made(report: &Report) -> PathBuf {
    assert_eq!(report.files.len(), 1, "{report:#?}");
    match &report.files[0].verdict {
        Verdict::Made { folder } => folder.clone(),
        other => panic!("expected a source to be made, got {other:?}: {report:#?}"),
    }
}

fn imported(project: &Path, name: &str, bytes: &[u8]) -> Report {
    import(
        project,
        &[Input::Bytes {
            name,
            bytes,
            facts: Facts {
                from_game: true,
                virtual_path: Some(format!("x64a.rpf/{name}")),
                game_build: None,
                ..Facts::default()
            },
        }],
        &Options::default(),
    )
}

#[test]
fn the_binary_metadata_extensions_have_a_row_that_makes_a_metadata_source_and_a_text_meta_has_none()
{
    assert_eq!(metadata::KIND, "metadata");
    for extension in ["ymt", "ymap", "ytyp", "YMAP"] {
        assert_eq!(kind_of(extension).map(|r| r.kind), Some(metadata::KIND));
    }
    assert!(kind_of("meta").is_none());
}

#[test]
fn a_ymap_in_each_binary_container_becomes_xml_and_an_edit_to_the_xml_compiles_into_it() {
    for (format, extension) in [
        (BinaryFormat::Rbm, "ymap"),
        (BinaryFormat::Pso, "ytyp"),
        (BinaryFormat::Rbf, "ymt"),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let original = synthetic(format);
        assert_eq!(detect_binary_format(&original), Some(format));
        let dir = extract(scratch.path(), &original, "cs1_02", extension);
        assert_eq!(dir.file_name().unwrap(), "cs1_02.metadata.src");
        assert_eq!(
            names(&dir),
            [
                "asset.json".to_string(),
                format!("base.{extension}"),
                "metadata.xml".to_string()
            ]
        );
        let source = Source::open(&dir).unwrap();
        assert_eq!(source.manifest.kind, "metadata");
        assert_eq!(source.manifest.origin.sha256, Some(sha256_hex(&original)));

        // Written by the canonical layout, the file is its own first export.
        let first = compile(&dir);
        assert_eq!(first, compile(&dir), "{format:?}: two compiles differ");
        assert_eq!(
            first, original,
            "{format:?}: the first export is not the file"
        );
        assert_eq!(metadata::differs_from_the_original(&dir), Some(false));

        // The XML a person edits.
        let xml = fs::read_to_string(dir.join("metadata.xml")).unwrap();
        if format == BinaryFormat::Rbf {
            let edited = xml.replace("a string", "another string");
            assert_ne!(xml, edited, "{xml}");
            fs::write(dir.join("metadata.xml"), edited).unwrap();
            let after = compile(&dir);
            assert_ne!(after, original, "the edit did not reach the file");
            continue;
        }
        assert!(xml.contains("65"), "{xml}");
        fs::write(
            dir.join("metadata.xml"),
            xml.replace("contentFlags value=\"65\"", "contentFlags value=\"66\""),
        )
        .unwrap();
        let after = compile(&dir);
        assert_eq!(
            content_flags(&after),
            66,
            "{format:?}: the edit is not in the file"
        );
        assert_eq!(content_flags(&original), 65);
    }
}

#[test]
fn a_text_meta_is_its_own_xml_and_compiles_to_the_bytes_it_came_from() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = extract(scratch.path(), RBF_XML.as_bytes(), "peds", "meta");
    assert_eq!(
        fs::read(dir.join("metadata.xml")).unwrap(),
        RBF_XML.as_bytes()
    );
    assert_eq!(compile(&dir), RBF_XML.as_bytes());
    assert_eq!(metadata::differs_from_the_original(&dir), Some(false));
}

#[test]
fn an_edit_that_leaves_a_text_meta_no_xml_document_is_refused_by_the_compile_and_the_cook() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = extract(scratch.path(), RBF_XML.as_bytes(), "peds", "meta");
    let shadow = tempfile::tempdir().unwrap();
    let why = "no metadata document";
    for edit in [
        "<CCreatureMetaData><oops></CCreatureMetaData>",
        "this is not xml at all <<<",
        "",
    ] {
        fs::write(dir.join("metadata.xml"), edit).unwrap();
        let problems = metadata::compile(&Source::open(&dir).unwrap())
            .expect_err(&format!("{edit:?} compiled"));
        assert_eq!(problems[0].file.as_deref(), Some("metadata.xml"));
        assert!(problems[0].message.contains(why), "{edit:?}: {problems:#?}");
        let cooked = cook_source(&dir, shadow.path()).expect_err(&format!("{edit:?} cooked"));
        assert!(cooked.iter().any(|p| p.contains(why)), "{cooked:#?}");
    }
    // An edit that is still a document compiles, as itself.
    let edited = RBF_XML.replace("a string", "another string");
    fs::write(dir.join("metadata.xml"), &edited).unwrap();
    assert_eq!(compile(&dir), edited.as_bytes());
}

#[test]
fn a_file_that_is_no_metadata_document_is_refused_with_the_reason_and_leaves_no_folder() {
    let scratch = tempfile::tempdir().unwrap();
    for (bytes, extension, why) in [
        (
            b"not metadata at all".as_slice(),
            "ymap",
            "is not a binary metadata file",
        ),
        (
            b"not metadata at all".as_slice(),
            "meta",
            "no metadata document",
        ),
        (b"PSINbroken".as_slice(), "ytyp", ""),
        (RBF_XML.as_bytes(), "ydr", "holds one of"),
    ] {
        let problems =
            metadata::extract(bytes, "x", extension, scratch.path(), &how("x")).unwrap_err();
        assert!(
            !problems.is_empty() && problems[0].message.contains(why),
            "{extension}: {problems:#?}"
        );
    }
    assert_eq!(names(scratch.path()), Vec::<String>::new());
}

#[test]
fn the_base_of_a_source_cooks_to_what_the_source_cooks_to_until_its_xml_is_changed() {
    for (format, extension) in [(BinaryFormat::Rbm, "ymap"), (BinaryFormat::Pso, "ytyp")] {
        let scratch = tempfile::tempdir().unwrap();
        let dir = extract(scratch.path(), &synthetic(format), "cs1_02", extension);
        let base = |dir: &Path| {
            metadata::compile_base(&Source::open(dir).unwrap())
                .unwrap_or_else(|problems| panic!("{format:?}: {problems:#?}"))
        };
        let untouched = compile(&dir);
        assert_eq!(
            base(&dir).bytes,
            untouched,
            "{format:?}: an untouched source"
        );
        assert_eq!(base(&dir).file_name, format!("cs1_02.{extension}"));

        let xml = fs::read_to_string(dir.join("metadata.xml")).unwrap();
        let edited = xml.replace("contentFlags value=\"65\"", "contentFlags value=\"66\"");
        assert_ne!(xml, edited, "{xml}");
        fs::write(dir.join("metadata.xml"), edited).unwrap();
        assert_eq!(content_flags(&compile(&dir)), 66);
        assert_eq!(
            base(&dir).bytes,
            untouched,
            "{format:?}: the base does not follow an edit of the XML"
        );
    }

    // A base that is not the file the XML was exported from is refused, as the compile refuses it.
    let scratch = tempfile::tempdir().unwrap();
    let dir = extract(
        scratch.path(),
        &synthetic(BinaryFormat::Rbm),
        "cs1_02",
        "ymap",
    );
    fs::write(dir.join("base.ymap"), synthetic(BinaryFormat::Pso)).unwrap();
    assert!(metadata::compile_base(&Source::open(&dir).unwrap()).is_err());

    // A base that is no metadata file is refused, and the refusal names the base and not the XML.
    fs::write(dir.join("base.ymap"), b"not a map").unwrap();
    let problems = metadata::compile_base(&Source::open(&dir).unwrap()).unwrap_err();
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(problems[0].file.as_deref(), Some("base.ymap"));
}

#[test]
fn a_changed_base_is_refused_as_not_the_file_the_xml_was_exported_from() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = extract(
        scratch.path(),
        &synthetic(BinaryFormat::Rbm),
        "cs1_02",
        "ymap",
    );
    fs::write(dir.join("base.ymap"), synthetic(BinaryFormat::Pso)).unwrap();
    let problems = metadata::compile(&Source::open(&dir).unwrap()).unwrap_err();
    assert!(
        problems[0]
            .message
            .contains("not the file the XML was exported from"),
        "{problems:#?}"
    );
}

#[test]
fn importing_a_ymap_makes_a_metadata_source_and_says_the_first_export_is_the_files() {
    let project = tempfile::tempdir().unwrap();
    let original = synthetic(BinaryFormat::Rbm);
    let report = imported(project.path(), "cs1_02.ymap", &original);
    let folder = made(&report);
    assert_eq!(folder, project.path().join("sources/cs1_02.metadata.src"));
    assert!(
        report.files[0]
            .reason
            .contains("first export is the original's bytes"),
        "{report:#?}"
    );
    let source = Source::open(&folder).unwrap();
    assert!(source.manifest.origin.from_game);
    assert_eq!(
        source.manifest.origin.cooked_sha256,
        Some(sha256_hex(&compile(&folder))),
        "an import records what the cook made, so a build can tell the file is still the game's"
    );
    assert!(!project.path().join("files").exists());
}

#[test]
fn a_metadata_file_kept_as_a_compiled_file_says_why_and_the_report_names_the_kind() {
    let project = tempfile::tempdir().unwrap();
    let report = imported(project.path(), "weird.ymap", b"PSINbroken");
    assert!(
        matches!(
            report.files[0].verdict,
            Verdict::Kept { copied_to: Some(_) }
        ),
        "{report:#?}"
    );
    assert!(
        report.files[0]
            .reason
            .contains("has no source form this import could make"),
        "{report:#?}"
    );
    assert_eq!(
        fs::read(project.path().join("files/weird.ymap")).unwrap(),
        b"PSINbroken"
    );
    assert_eq!(
        report.notes,
        ["Kept as compiled files, not sources: 1 .ymap"],
        "{report:#?}"
    );
}

#[test]
fn a_metadata_source_cooks_to_the_game_extension_a_build_and_a_saved_xml_ask_for() {
    let project = tempfile::tempdir().unwrap();
    let original = synthetic(BinaryFormat::Rbm);
    let dir = extract(project.path(), &original, "cs1_02", "ymap");
    let said = describe(&dir).expect("a metadata source is described");
    assert_eq!(said.kind, "metadata");
    assert_eq!(said.name, "cs1_02");
    assert_eq!(said.extension, "ymap");
    assert!(said.bytes > original.len() as u64);

    let scratch = tempfile::tempdir().unwrap();
    let cooked = cook_source(&dir, scratch.path()).unwrap();
    assert_eq!(cooked.file_name().unwrap(), "cs1_02.ymap");
    assert_eq!(fs::read(&cooked).unwrap(), original);

    let shadow = project.path().join("build/shadow");
    let id = Source::open(&dir).unwrap().manifest.id;
    let mut cooker = Cooker::new(&shadow);
    let first = cooker.recook(&dir.join("metadata.xml"));
    assert!(matches!(first, Recook::Cooked { .. }), "{first:?}");
    assert_eq!(
        fs::read(shadow.join(&id).join("cs1_02.ymap")).unwrap(),
        original
    );

    let xml = fs::read_to_string(dir.join("metadata.xml")).unwrap();
    fs::write(
        dir.join("metadata.xml"),
        xml.replace("contentFlags value=\"65\"", "contentFlags value=\"66\""),
    )
    .unwrap();
    let saved = cooker.recook(&dir.join("metadata.xml"));
    assert!(matches!(saved, Recook::Cooked { .. }), "{saved:?}");
    let now = fs::read(shadow.join(&id).join("cs1_02.ymap")).unwrap();
    assert_eq!(content_flags(&now), 66);
}

#[test]
fn the_xml_for_a_changed_file_is_given_with_nothing_written_and_xml_typed_by_hand_is_no_export() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = extract(
        scratch.path(),
        &synthetic(BinaryFormat::Rbm),
        "cs1_02",
        "ymap",
    );
    let inputs = metadata::Inputs::read(&Source::open(&dir).unwrap()).unwrap();
    let held = fs::read(dir.join("metadata.xml")).unwrap();
    assert_eq!(inputs.xml(), held);
    assert_eq!(inputs.extension(), "ymap");
    let cooked = inputs.compile().unwrap().bytes;
    assert!(inputs.is_the_export_of(&cooked));
    assert_eq!(
        inputs.xml_of(&cooked).unwrap(),
        held,
        "the file the source cooks to asks for the XML it holds"
    );

    // Another file of its kind: the XML for it is given, and the source is as it was.
    let typed = String::from_utf8(held.clone()).unwrap();
    let mut edited = inputs.clone();
    edited.set_xml(
        typed
            .replace("contentFlags value=\"65\"", "contentFlags value=\"66\"")
            .into_bytes(),
    );
    let other = edited.compile().unwrap().bytes;
    assert_eq!(content_flags(&other), 66);
    let xml = inputs.xml_of(&other).unwrap();
    assert_eq!(xml, edited.xml());
    assert!(!inputs.is_the_export_of(&other));
    assert_eq!(fs::read(dir.join("metadata.xml")).unwrap(), held);
    let before = names(&dir);
    metadata::write_xml(&dir, &xml).unwrap();
    assert_eq!(compile(&dir), other);
    assert_eq!(names(&dir), before, "the part file is gone");

    // The same document spaced as no export spaces it cooks to the same file, and is not that file's export.
    let mut spaced = inputs.clone();
    spaced.set_xml(
        typed
            .replacen("<entities", "\n\n   <entities", 1)
            .into_bytes(),
    );
    assert_ne!(spaced.xml(), held);
    assert_eq!(spaced.compile().unwrap().bytes, cooked);
    assert!(!spaced.is_the_export_of(&cooked));
    assert_eq!(spaced.xml_of(&cooked).unwrap(), held);
}

/// Where the staged binary metadata documents are, as the R37 suites find them.
fn metadata_corpus() -> Option<Vec<PathBuf>> {
    let origin = or_test_support::owned_corpus_origin(
        "or-meta",
        env!("CARGO_MANIFEST_DIR"),
        "OPENRAGE_META_CORPUS",
        "binary",
    );
    let mut files: Vec<PathBuf> = fs::read_dir(origin.path())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ymt")))
        .collect();
    files.sort();
    if files.is_empty() {
        skip_corpus(
            "every corpus .ymt becomes a metadata source or is kept with the reason",
            format!("no .ymt staged in {}", origin.describe()),
        );
        return None;
    }
    Some(files)
}

#[derive(Default)]
struct Tally {
    population: usize,
    sources: usize,
    differ_from_the_original: usize,
    refused: Vec<String>,
}

/// One document in this many of the corpus is made a source, and one in this many of those a second time.
const SAMPLE: usize = 16;
const AGAIN: usize = 4;

/// How many documents a failing run names.
const REPORTED: usize = 12;

/// Per mille of a container's population that may be refused before the run stops being evidence.
const ALLOWANCE: usize = 20;

#[test]
fn a_sample_of_the_corpus_ymt_reaches_its_canonical_fixed_point_through_its_xml_or_is_refused_with_the_reason(
) {
    let Some(files) = metadata_corpus() else {
        return;
    };
    // ⚠ One document in `SAMPLE`, spread over the sorted corpus: a source costs a few cycles of the
    // conversion, and `xml_converges_without_a_snapshot` in `or-io-verify` holds the conversion
    // itself over every document.
    let total = files.len();
    let files: Vec<PathBuf> = files.into_iter().step_by(SAMPLE).collect();
    let scratch = tempfile::tempdir().unwrap();
    let mut tallies: std::collections::BTreeMap<&'static str, Tally> = Default::default();
    for path in files {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let name = file.trim_end_matches(".ymt");
        let bytes = fs::read(&path).unwrap();
        let format = detect_binary_format(&bytes).expect("a staged document is binary");
        let tally = tallies.entry(format.label()).or_default();
        tally.population += 1;

        let dir = match metadata::extract(&bytes, name, "ymt", scratch.path(), &how(&file)) {
            Ok(dir) => dir,
            Err(problems) => {
                assert!(
                    !problems.is_empty() && !problems[0].message.is_empty(),
                    "{file}"
                );
                tally
                    .refused
                    .push(format!("{file}: {}", problems[0].message));
                continue;
            }
        };
        tally.sources += 1;
        let compiled = compile(&dir);
        assert_eq!(compiled, compile(&dir), "{file}: two compiles differ");
        if compiled != bytes {
            tally.differ_from_the_original += 1;
        }
        fs::remove_dir_all(&dir).unwrap();

        // The canonical fixed point, through the source itself: the file a compile wrote, made a
        // source in its turn, compiles to itself.
        if tally.sources.is_multiple_of(AGAIN) {
            let again = extract(scratch.path(), &compiled, name, "ymt");
            assert_eq!(
                compile(&again),
                compiled,
                "{file}: its canonical form is not a fixed point"
            );
            fs::remove_dir_all(&again).unwrap();
        }
    }
    for (label, tally) in &tallies {
        println!(
            "{label}: {} documents sampled of {total} .ymt, {} became sources ({} compile to \
             other bytes than the file they came from), {} kept with a reason",
            tally.population,
            tally.sources,
            tally.differ_from_the_original,
            tally.refused.len()
        );
        assert!(
            tally.refused.len() * 1000 <= tally.population * ALLOWANCE,
            "{label}: {} of {} refused:\n    {}",
            tally.refused.len(),
            tally.population,
            tally
                .refused
                .iter()
                .take(REPORTED)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n    ")
        );
    }
}

const CHILD_TEST: &str = "the_child_that_makes_sources_with_no_snapshot";
const CHILD_ENV: &str = "OPENRAGE_TEST_METADATA_LIST";

/// The corpus documents both processes of the test below make a source of, one in `STEP`.
fn spread(files: &[PathBuf]) -> Vec<&PathBuf> {
    files.iter().step_by((files.len() / 60).max(1)).collect()
}

/// What a document made into a source in `parent` gives: the digest of its XML and of its compile, or why it was kept.
fn digests(parent: &Path, n: usize, file: &Path) -> String {
    let label = file.file_name().unwrap().to_string_lossy().into_owned();
    let bytes = fs::read(file).unwrap();
    let name = format!("{n:03}");
    match metadata::extract(&bytes, &name, "ymt", parent, &how(&label)) {
        Ok(dir) => {
            let xml = fs::read(dir.join("metadata.xml")).unwrap();
            format!("{name} {} {}", sha256_hex(&xml), sha256_hex(&compile(&dir)))
        }
        Err(problems) => format!("{name} kept {}", problems[0].message),
    }
}

/// Run by the test below in a process that installs no snapshot: makes a source of each document it is told of.
#[test]
fn the_child_that_makes_sources_with_no_snapshot() {
    let Some(list) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    assert!(
        !or_io::meta::has_default_library(),
        "the child has a snapshot installed"
    );
    let scratch = tempfile::tempdir().unwrap();
    let list = fs::read_to_string(list).unwrap();
    for (n, file) in list.lines().enumerate() {
        println!("RESULT {}", digests(scratch.path(), n, Path::new(file)));
    }
}

#[test]
fn a_host_with_a_schema_snapshot_makes_the_same_sources_as_one_without() {
    let Some(files) = metadata_corpus() else {
        return;
    };
    let Some(path) = or_test_support::schema_snapshot() else {
        skip_schema("a host with a snapshot makes the same sources as one without");
        return;
    };
    let chosen = spread(&files);
    let scratch = tempfile::tempdir().unwrap();
    let list = scratch.path().join("list.txt");
    fs::write(
        &list,
        chosen
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &list)
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&child.stdout);
    let without: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split_once("RESULT ").map(|(_, rest)| rest))
        .collect();
    assert_eq!(without.len(), chosen.len(), "{text}");

    // The way a host installs it: the names its snapshot gives come from the process's default library.
    or_io::meta::install_snapshot(&path).expect("the snapshot installs");
    assert!(or_io::meta::has_default_library());
    let here = scratch.path().join("here");
    let differing: Vec<String> = chosen
        .iter()
        .enumerate()
        .map(|(n, file)| digests(&here, n, file))
        .zip(&without)
        .filter(|(with, without)| with != *without)
        .map(|(with, without)| format!("with a snapshot {with}, without {without}"))
        .collect();
    let kept = without.iter().filter(|r| r.contains(" kept ")).count();
    println!(
        "{} documents: {} sources and {kept} kept, the same with a snapshot installed and without",
        without.len(),
        without.len() - kept
    );
    assert!(differing.is_empty(), "{}", differing.join("\n"));
}
