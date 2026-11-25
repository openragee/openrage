//! `openrage-cli project build` cooks the source folders an import made: a texture dictionary comes
//! back with its pixels, an image saved beside a DDS reaches the next build, a source that is still
//! the game's file is left out and said so, two builds are byte-identical, the folder never ships and
//! a broken image stops the cook with the last build as it was.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use or_io::authoring::TextureBuildOptions;
use or_io::codec::texture::{decode_level, GrcTextureFormat, RgbaImage};
use or_io::container::resource::{Compression, ResourceEnvelope, Rsc7};
use or_io::decl::File;
use or_io::gta5::formats::format;
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::texture_dict::gen8::PROCESSED;
use or_io::gta5::PgDictionary;
use or_io::Generation;
use or_project::{Container, Module, Output, OutputKind, Project, Target};

const CHILD_TEST: &str = "the_child_that_runs_one_command";
const CHILD_ENV: &str = "OPENRAGE_TEST_ONE_COMMAND";
const BEGIN: &str = "<<begin>>";
const END: &str = "<<end>>";
const SEPARATOR: char = '\u{1f}';

/// Run by `stdout_of` in a child process, so that what a command prints can be read.
#[test]
fn the_child_that_runs_one_command() {
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

/// Whether `openrage-cli <args>` succeeded, and what it printed.
fn cli(args: &[&str]) -> (bool, String) {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, args.join(&SEPARATOR.to_string()))
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("{args:?}: no start marker: {text}"));
    let (printed, tail) = rest.split_once(END).unwrap_or((rest, ""));
    (tail.starts_with('1'), printed.to_string())
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// A Legacy dictionary of one block-compressed texture called `bark`.
fn ytd(image: &RgbaImage) -> Vec<u8> {
    let mut texture = GrcTexture {
        name: "bark".to_string(),
        resource_type_and_conversion_flags: PROCESSED,
        extra_flags: 0x10,
        handle_index: 7,
        stereo_rt_mode: 2,
        ..Default::default()
    };
    texture
        .set_pixels(
            GrcTextureFormat::DXT5,
            image,
            &TextureBuildOptions::default(),
        )
        .unwrap();
    texture.set_template_type(0x2000_0054);
    let mut dictionary = PgDictionary::<GrcTexture>::default();
    dictionary.insert(texture);
    dictionary.to_bytes().unwrap()
}

fn ramp(width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set_pixel(x, y, [(x * 7) as u8, (y * 5) as u8, (x + y) as u8, 255]);
        }
    }
    image
}

/// Red, green, blue and a translucent grey, in flat quadrants.
fn quadrants(size: u32) -> RgbaImage {
    let colours = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [128, 128, 128, 64],
    ];
    let mut image = RgbaImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let q = usize::from(x >= size / 2) + 2 * usize::from(y >= size / 2);
            image.set_pixel(x, y, colours[q]);
        }
    }
    image
}

/// The first level of the texture called `bark` in a Legacy dictionary, decoded.
fn pixels(bytes: &[u8]) -> RgbaImage {
    let dictionary = PgDictionary::<GrcTexture>::from_bytes(bytes).expect("a Legacy dictionary");
    let texture = dictionary.get("bark").expect("the texture is there");
    decode_level(texture, 0).expect("it decodes")
}

fn same_pixels(a: &RgbaImage, b: &RgbaImage) -> bool {
    a.width == b.width && a.data == b.data
}

struct Fixture {
    _work: tempfile::TempDir,
    file: PathBuf,
    /// The dictionary as it was imported.
    original: Vec<u8>,
}

/// `bytes` as the game ships a file: the same payload under another compression, so a cook, which
/// writes OpenRage's own, makes other bytes of it, as it does of every file in the corpus.
fn as_the_game_ships_it(bytes: &[u8]) -> Vec<u8> {
    let opened = Rsc7::open(bytes).expect("an RSC7 file");
    let shipped =
        Rsc7::reseal(&opened.data, &opened.seal, Compression::none()).expect("it reseals");
    assert_ne!(shipped, bytes, "the stand-in is the canonical form");
    shipped
}

/// A project file in `work` holding one Files content and one FiveM resource.
fn create_project(work: &Path, target: Option<Target>) -> PathBuf {
    let file = work.join("harbor.openrage-proj");
    let mut output = Output::of(OutputKind::FivemResource, "fivem", "harbor", &["files"]);
    output.target = target;
    let container = Container {
        modules: vec![Module::files("files")],
        outputs: vec![output],
    };
    Project::create(&file, &container, &[]).unwrap();
    file
}

impl Fixture {
    /// A project holding one Files content, with `bark_set.ytd` imported into it.
    fn new(from_game: bool, target: Option<Target>) -> Self {
        let work = tempfile::tempdir().unwrap();
        let file = create_project(work.path(), target);
        let original = ytd(&ramp(16, 16));
        // The game's file is not OpenRage's own form of it, so a cook of it makes other bytes.
        let original = if from_game {
            as_the_game_ships_it(&original)
        } else {
            original
        };
        let incoming = work.path().join("incoming").join("bark_set.ytd");
        std::fs::create_dir_all(incoming.parent().unwrap()).unwrap();
        std::fs::write(&incoming, &original).unwrap();
        let mut args = vec!["project", "import", text(&file), text(&incoming)];
        if from_game {
            args.push("--from-game");
        }
        let (ok, said) = cli(&args);
        assert!(ok, "the import failed: {said}");
        Self {
            _work: work,
            file,
            original,
        }
    }

    fn folder(&self) -> &Path {
        self.file.parent().unwrap()
    }

    fn source(&self) -> PathBuf {
        self.folder().join("sources").join("bark_set.textures.src")
    }

    fn built(&self) -> PathBuf {
        self.folder().join("build").join("harbor")
    }

    fn build(&self) -> (bool, String) {
        cli(&["project", "build", text(&self.file)])
    }
}

/// Every file under `dir` by its `/` path, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap();
                out.insert(
                    rel.to_string_lossy().replace('\\', "/"),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn a_build_over_an_imported_source_ships_a_dictionary_that_reads_back_with_the_same_pixels() {
    let project = Fixture::new(false, None);
    assert!(project.source().join("asset.json").is_file());

    let (ok, said) = project.build();
    assert!(ok, "{said}");
    let written = snapshot(&project.built());
    let cooked = written
        .get("stream/bark_set.ytd")
        .unwrap_or_else(|| panic!("the build shipped {:?}", written.keys().collect::<Vec<_>>()));
    assert!(
        same_pixels(&pixels(cooked), &pixels(&project.original)),
        "the cooked dictionary reads back with other pixels"
    );
    assert!(
        written.keys().all(|p| !p.contains(".src")),
        "the source folder shipped: {:?}",
        written.keys().collect::<Vec<_>>()
    );
    assert!(said.contains("1 source(s) cooked"), "{said}");
    let (ok, said) = cli(&["project", "verify", text(&project.file)]);
    assert!(ok, "{said}");
}

#[test]
fn an_image_saved_beside_the_dds_is_what_the_next_build_ships() {
    let project = Fixture::new(false, None);
    assert!(project.build().0);
    let first = snapshot(&project.built())["stream/bark_set.ytd"].clone();

    let image = quadrants(16);
    let png = project.source().join("textures").join("bark.png");
    or_io::codec::texture::save_png(&png, &image).unwrap();
    let (ok, said) = project.build();
    assert!(ok, "{said}");
    let second = snapshot(&project.built())["stream/bark_set.ytd"].clone();

    assert_ne!(first, second, "the saved image did not reach the build");
    let shipped = pixels(&second);
    for (x, y, want) in [
        (2, 2, [255u8, 0, 0]),
        (13, 2, [0, 255, 0]),
        (2, 13, [0, 0, 255]),
    ] {
        let got = shipped.get_pixel(x, y);
        assert!(
            (0..3).all(|c| got[c].abs_diff(want[c]) <= 8),
            "({x}, {y}) is {got:?}, not {want:?}"
        );
    }
}

#[test]
fn a_source_that_is_still_the_games_file_is_left_out_and_said_so_until_it_changes() {
    let project = Fixture::new(true, None);
    let (planned, said) = cli(&["project", "plan", text(&project.file)]);
    assert!(planned, "{said}");
    assert!(
        said.contains("left out if it is still the game's file"),
        "the plan does not say it: {said}"
    );

    let (ok, said) = project.build();
    assert!(ok, "{said}");
    assert!(
        !snapshot(&project.built()).contains_key("stream/bark_set.ytd"),
        "the game's own file shipped"
    );
    assert!(
        said.contains("stream/bark_set.ytd is unchanged from the game"),
        "{said}"
    );
    let (ok, said) = cli(&["project", "verify", text(&project.file)]);
    assert!(ok, "a build that left it out does not verify: {said}");

    let png = project.source().join("textures").join("bark.png");
    or_io::codec::texture::save_png(&png, &quadrants(16)).unwrap();
    let (ok, said) = project.build();
    assert!(ok, "{said}");
    assert!(
        snapshot(&project.built()).contains_key("stream/bark_set.ytd"),
        "a changed source is the person's and ships"
    );
    assert!(!said.contains("unchanged from the game"), "{said}");
}

#[test]
fn two_builds_of_one_project_are_byte_identical() {
    let project = Fixture::new(false, None);
    // A build never goes inside the project, so the two land in a folder of their own.
    let out = tempfile::tempdir().unwrap();
    let (one, two) = (out.path().join("one"), out.path().join("two"));
    assert!(cli(&["project", "build", text(&project.file), "--out", text(&one)]).0);
    assert!(cli(&["project", "build", text(&project.file), "--out", text(&two)]).0);
    let (a, b) = (snapshot(&one.join("harbor")), snapshot(&two.join("harbor")));
    assert!(a.contains_key("stream/bark_set.ytd"));
    assert_eq!(a, b);
}

#[test]
fn a_broken_image_fails_the_cook_names_itself_and_leaves_the_last_build() {
    let project = Fixture::new(false, None);
    assert!(project.build().0);
    let good = snapshot(&project.built());

    std::fs::write(
        project.source().join("textures").join("bark.png"),
        b"not an image",
    )
    .unwrap();
    let (ok, said) = project.build();
    assert!(!ok, "a build over a broken image succeeded: {said}");
    assert!(
        said.contains("problem:") && said.contains("bark"),
        "the failure is not a problem naming the image: {said}"
    );
    assert_eq!(snapshot(&project.built()), good, "the last build changed");
    let hidden: Vec<_> = std::fs::read_dir(project.built().parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".cook") || n.ends_with(".new"))
        .collect();
    assert!(hidden.is_empty(), "a failed build left {hidden:?}");
}

#[test]
fn a_source_for_another_platform_is_refused_in_one_sentence() {
    let project = Fixture::new(false, None);
    let manifest = project.source().join("asset.json");
    let edited = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace("\"win64\"", "\"xbox360\"");
    std::fs::write(&manifest, edited).unwrap();

    let (ok, said) = cli(&["project", "plan", text(&project.file)]);
    assert!(!ok, "{said}");
    assert!(said.contains("xbox360") && said.contains("win64"), "{said}");
    let (ok, said) = project.build();
    assert!(!ok, "{said}");
    assert!(!project.built().exists(), "a refused build wrote");
}

#[test]
fn an_enhanced_build_ships_the_cooked_dictionary_in_the_enhanced_layout() {
    let project = Fixture::new(false, Some(Target::Enhanced));
    let (ok, said) = project.build();
    assert!(ok, "{said}");
    let cooked = &snapshot(&project.built())["stream/bark_set.ytd"];
    let version = Rsc7::peek(cooked).expect("an RSC7 file").version();
    assert_eq!(
        format("ytd").unwrap().target_of_version(version).unwrap(),
        Generation::Gen9,
        "the cooked dictionary did not move to the target"
    );
}

/// How many files of each extension the corpus check imports, spread over the staged ones.
const SAMPLE: usize = 3;

/// A flat image the size of the first DDS in the textures source `folder`, saved beside it.
fn repaint_the_first_texture(folder: &Path) {
    let dds = std::fs::read_dir(folder.join("textures"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "dds"))
        .min()
        .expect("a textures source holds a DDS");
    let head = std::fs::read(&dds).unwrap();
    let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    // A DDS header states its height and then its width after the magic, the size and the flags.
    let mut image = RgbaImage::new(word(16), word(12));
    for y in 0..image.height {
        for x in 0..image.width {
            image.set_pixel(x, y, [200, 30, 90, 255]);
        }
    }
    or_io::codec::texture::save_png(dds.with_extension("png"), &image).unwrap();
}

#[test]
fn corpus_files_taken_from_the_game_are_left_out_of_a_build_until_they_are_edited() {
    use or_source::game_import::{import, Facts, Input, Options, Verdict};
    let work = tempfile::tempdir().unwrap();
    let file = create_project(work.path(), None);
    let folder = file.parent().unwrap();

    let mut imported: Vec<String> = Vec::new();
    let mut first_ytd = None;
    for extension in ["ytd", "ydr", "ydd", "yft"] {
        let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), extension)
            .all_targets()
            .find_or_skip(&format!(
                "a corpus .{extension} is left out of a build until it is edited"
            ))
        else {
            continue;
        };
        let step = (files.len() / SAMPLE).max(1);
        for path in files.iter().step_by(step).take(SAMPLE) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // Two staged files of one name would cook one output, which a build refuses.
            if imported.contains(&name) {
                continue;
            }
            let facts = Facts {
                from_game: true,
                virtual_path: Some(format!("x64a.rpf/{name}")),
                game_build: None,
                ..Facts::default()
            };
            let bytes = std::fs::read(path).unwrap();
            let report = import(
                folder,
                &[Input::Bytes {
                    name: &name,
                    bytes: &bytes,
                    facts,
                }],
                &Options::default(),
            );
            if let Verdict::Made { folder } = &report.files[0].verdict {
                if extension == "ytd" && first_ytd.is_none() {
                    first_ytd = Some(folder.clone());
                }
                imported.push(name);
            }
        }
    }
    if imported.is_empty() {
        return;
    }
    // A staged fragment is a source of its own kind, and a build leaves it out like any other file of the game.
    let staged_fragments = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "yft")
        .all_targets()
        .find();
    assert!(
        staged_fragments.is_empty() || imported.iter().any(|n| n.ends_with(".yft")),
        "no staged .yft became a source: {imported:?}"
    );

    let (ok, said) = cli(&["project", "build", text(&file)]);
    assert!(ok, "{said}");
    let built = folder.join("build").join("harbor");
    let shipped = snapshot(&built);
    for name in &imported {
        assert!(
            !shipped.contains_key(&format!("stream/{name}")),
            "{name}: the game's own file, imported and untouched, shipped"
        );
        assert!(
            said.contains(&format!("stream/{name} is unchanged from the game")),
            "{name}: the build does not say it left it out: {said}"
        );
    }

    let Some(texture) = first_ytd else {
        return;
    };
    repaint_the_first_texture(&texture);
    let (ok, said) = cli(&["project", "build", text(&file)]);
    assert!(ok, "{said}");
    let shipped = snapshot(&built);
    let edited = imported
        .iter()
        .find(|n| n.ends_with(".ytd"))
        .expect("a texture dictionary was imported");
    assert!(
        shipped.contains_key(&format!("stream/{edited}")),
        "{edited}: an edited source is the person's and ships"
    );
    assert_eq!(
        shipped.keys().filter(|k| k.starts_with("stream/")).count(),
        1,
        "only the edited source ships: {:?}",
        shipped.keys().collect::<Vec<_>>()
    );
}

/// A creature file in RBF, which is its own XML, so the cook of its source is the file it came from.
const PED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CCreatureMetaData>
  <pedCompExpressions>
    <Item>
      <pedCompID value="6"/>
      <name>a string</name>
    </Item>
  </pedCompExpressions>
</CCreatureMetaData>
"#;

#[test]
fn a_metadata_file_taken_from_the_game_is_left_out_of_a_build_until_its_xml_is_edited() {
    a_metadata_file_taken_from_the_game_is_left_out_until_its_xml_is_edited(None);
}

#[test]
fn a_metadata_file_taken_from_the_game_is_left_out_of_an_enhanced_build_too() {
    a_metadata_file_taken_from_the_game_is_left_out_until_its_xml_is_edited(Some(Target::Enhanced));
}

fn a_metadata_file_taken_from_the_game_is_left_out_until_its_xml_is_edited(target: Option<Target>) {
    use or_io::meta::binary::parse_meta_binary_full;
    use or_io::meta::{canonical, BinaryFormat, Diagnostics, SchemaLibrary};

    let work = tempfile::tempdir().unwrap();
    let file = create_project(work.path(), target);
    let folder = file.parent().unwrap();
    let original = canonical::import(
        &SchemaLibrary::default(),
        PED_XML.as_bytes(),
        BinaryFormat::Rbf,
        None,
    )
    .expect("the RBF imports");
    let incoming = work.path().join("incoming").join("ped.ymt");
    std::fs::create_dir_all(incoming.parent().unwrap()).unwrap();
    std::fs::write(&incoming, &original).unwrap();
    let (ok, said) = cli(&[
        "project",
        "import",
        text(&file),
        text(&incoming),
        "--from-game",
    ]);
    assert!(ok, "{said}");
    let source = folder.join("sources").join("ped.metadata.src");
    assert!(source.join("metadata.xml").is_file(), "{said}");

    let (ok, said) = cli(&["project", "build", text(&file)]);
    assert!(ok, "{said}");
    let built = folder.join("build").join("harbor");
    assert!(!snapshot(&built).contains_key("stream/ped.ymt"));
    assert!(
        said.contains("stream/ped.ymt is unchanged from the game"),
        "{said}"
    );

    let xml = source.join("metadata.xml");
    let edited = std::fs::read_to_string(&xml)
        .unwrap()
        .replace("pedCompID value=\"6\"", "pedCompID value=\"7\"");
    std::fs::write(&xml, edited).unwrap();
    let (ok, said) = cli(&["project", "build", text(&file)]);
    assert!(ok, "{said}");
    let shipped = snapshot(&built);
    let ped = shipped
        .get("stream/ped.ymt")
        .unwrap_or_else(|| panic!("an edited source ships: {:?}", shipped.keys()));
    let (root, layout) =
        parse_meta_binary_full(&SchemaLibrary::default(), ped, &mut Diagnostics::lenient())
            .expect("the shipped file reads");
    let shown = canonical::export(&root, &layout).unwrap();
    assert!(shown.contains("pedCompID value=\"7\""), "{shown}");
    let (ok, said) = cli(&["project", "verify", text(&file)]);
    assert!(ok, "{said}");
}

/// Imports `bytes` as the game's file `name` into a new project, and returns the project file and the source folder.
fn import_from_the_game(work: &Path, name: &str, kind: &str, bytes: &[u8]) -> (PathBuf, PathBuf) {
    let file = create_project(work, None);
    let incoming = work.join("incoming").join(name);
    std::fs::create_dir_all(incoming.parent().unwrap()).unwrap();
    std::fs::write(&incoming, bytes).unwrap();
    let (ok, said) = cli(&[
        "project",
        "import",
        text(&file),
        text(&incoming),
        "--from-game",
    ]);
    assert!(ok, "{said}");
    let stem = name.rsplit_once('.').unwrap().0;
    let source = file
        .parent()
        .unwrap()
        .join("sources")
        .join(format!("{stem}.{kind}.src"));
    assert!(source.join("asset.json").is_file(), "{said}");
    (file, source)
}

/// Builds the project and says whether `shipped` is in the build, with what the build printed.
fn built_with(file: &Path, shipped: &str) -> (Option<Vec<u8>>, String) {
    let (ok, said) = cli(&["project", "build", text(file)]);
    assert!(ok, "{said}");
    let built = file.parent().unwrap().join("build").join("harbor");
    (snapshot(&built).remove(shipped), said)
}

#[test]
fn a_bound_taken_from_the_game_is_left_out_of_a_build_until_a_polygon_changes() {
    use or_interchange::gltf::read::load;
    use or_interchange::gltf::{write, Container as Glb};
    use or_io::gta5::bounds::build::{self, BoundMesh};
    use or_io::gta5::bounds::PhBoundAny;

    let corners = vec![
        [10.0, 20.0, 3.0],
        [14.0, 20.0, 3.0],
        [14.0, 24.0, 3.0],
        [10.0, 24.0, 5.0],
    ];
    let mesh = BoundMesh::new(corners.clone(), vec![[0, 1, 2], [0, 2, 3]]);
    let original = PhBoundAny::PhBoundBVH(build::bvh(&mesh).unwrap())
        .to_bytes()
        .unwrap();
    let work = tempfile::tempdir().unwrap();
    let (file, source) = import_from_the_game(work.path(), "hull.ybn", "bounds", &original);
    let glb = source.join("bounds.glb");
    assert!(glb.is_file() && source.join("bounds.json").is_file());

    let (shipped, said) = built_with(&file, "stream/hull.ybn");
    assert!(shipped.is_none(), "the game's own bound shipped");
    assert!(
        said.contains("stream/hull.ybn is unchanged from the game"),
        "{said}"
    );

    // One corner of one triangle moves inside the bound's box: a changed polygon.
    let mut scene = load(&glb, &std::fs::read(&glb).unwrap()).unwrap();
    scene.objects[0].name = "hull".to_string();
    let mesh = &mut scene.objects[0].meshes[0];
    mesh.name = mesh.name.strip_prefix("hull_").unwrap().to_string();
    let moved = mesh.primitives[0]
        .positions
        .iter_mut()
        .find(|p| p[2] > 4.0)
        .expect("the raised corner");
    moved[2] = 4.0;
    std::fs::write(&glb, write(&scene, "hull", Glb::Glb).primary).unwrap();

    let (shipped, said) = built_with(&file, "stream/hull.ybn");
    let shipped = shipped.unwrap_or_else(|| panic!("a changed bound ships: {said}"));
    assert!(!said.contains("unchanged from the game"), "{said}");
    let PhBoundAny::PhBoundBVH(bound) = PhBoundAny::from_bytes(&shipped).unwrap() else {
        panic!("the shipped file is not the tree bound it was");
    };
    let heights: Vec<f32> = (0..bound.base.compressed_vertices.len())
        .map(|i| bound.base.vertex_position(i).unwrap()[2])
        .collect();
    let quantum = bound.base.un_quantize_factor.z;
    assert!(
        heights.iter().any(|z| (z - 4.0).abs() <= quantum) && heights.iter().all(|z| *z < 4.5),
        "{heights:?}"
    );
    let (ok, said) = cli(&["project", "verify", text(&file)]);
    assert!(ok, "{said}");
}

#[test]
fn a_path_region_taken_from_the_game_is_left_out_of_a_build_until_its_document_is_edited() {
    use or_io::gta5::{CNodeAddress, CPathNode, CPathNodeLink, CPathRegion};

    let node = |index: u16, links: usize| {
        let mut node = CPathNode {
            address: CNodeAddress {
                region: 7,
                index,
                ..Default::default()
            },
            coors_x: 100 * index as i16,
            ..Default::default()
        };
        node.set_num_links(links).unwrap();
        node
    };
    let link = |index: u16| CPathNodeLink {
        other_node: CNodeAddress {
            region: 7,
            index,
            ..Default::default()
        },
        ..Default::default()
    };
    let original = CPathRegion {
        nodes: vec![node(0, 1), node(1, 1)],
        num_nodes_car_nodes: 2,
        links: vec![link(1), link(0)],
        ..Default::default()
    }
    .to_bytes()
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let (file, source) = import_from_the_game(work.path(), "nodes7.ynd", "paths", &original);

    let (shipped, said) = built_with(&file, "stream/nodes7.ynd");
    assert!(shipped.is_none(), "the game's own region shipped");
    assert!(
        said.contains("stream/nodes7.ynd is unchanged from the game"),
        "{said}"
    );

    let document = source.join("paths.json");
    let mut stated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&document).unwrap()).unwrap();
    stated["nodes"][1]["coors_x"] = 250.into();
    std::fs::write(&document, serde_json::to_string_pretty(&stated).unwrap()).unwrap();

    let (shipped, said) = built_with(&file, "stream/nodes7.ynd");
    let shipped = shipped.unwrap_or_else(|| panic!("an edited region ships: {said}"));
    let region = CPathRegion::from_bytes(&shipped).unwrap();
    assert_eq!(region.nodes[1].coors_x, 250);
    assert_eq!(region.links.len(), 2);
    let (ok, said) = cli(&["project", "verify", text(&file)]);
    assert!(ok, "{said}");
}
