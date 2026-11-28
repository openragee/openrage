//! `openrage-cli world new-map` and `world archetypes` with no donor make a map and a type file from nothing: each opens as its
//! typed root, an entity asked for stands where it was put inside the extents, and what cannot be written is refused with no
//! file left behind.

use std::path::Path;
use std::process::ExitCode;

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_io::meta::{detect_binary_format, BinaryFormat, MetaDocument, SchemaLibrary};
use or_world_gta5::archetypes::ArchetypeIndex;
use or_world_gta5::create::{self, Placed};
use or_world_gta5::generate::{AssetKind, Authoring, DrawableFacts};

const BENCH: &str = "prop_bench_01a";

fn world(args: &[&str]) -> ExitCode {
    let mut all = vec!["world".to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

fn vec3(v: or_io::decl::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// A type file on disk that declares the bench, a cube two units on a side.
fn bench_types(dir: &Path) -> std::path::PathBuf {
    let mut types = create::types("hills_types");
    types.archetypes.push(create::archetype(
        &DrawableFacts {
            name: BENCH.into(),
            kind: AssetKind::Drawable,
            dictionary: None,
            bb_min: Vec3::splat(-1.0),
            bb_max: Vec3::splat(1.0),
            bs_centre: Vec3::ZERO,
            bs_radius: 3.0_f32.sqrt(),
        },
        &Authoring::default(),
    ));
    let path = dir.join("hills_types.ytyp");
    std::fs::write(
        &path,
        create::types_file(&types, BinaryFormat::Pso).expect("saves"),
    )
    .unwrap();
    path
}

#[test]
fn an_empty_map_and_an_empty_type_file_are_made_and_open_as_their_roots() {
    let scratch = tempfile::tempdir().unwrap();
    let map = scratch.path().join("hills.ymap");
    assert_eq!(
        world(&["new-map", "--name", "hills", text(&map)]),
        ExitCode::SUCCESS
    );
    let bytes = std::fs::read(&map).unwrap();
    assert_eq!(detect_binary_format(&bytes), Some(BinaryFormat::Pso));
    let opened = CMapData::from_bytes(&bytes).expect("opens as a CMapData");
    assert!(opened.name == "hills");
    assert!(opened.entities.is_empty());

    let types = scratch.path().join("hills_types.ytyp");
    assert_eq!(
        world(&["archetypes", "--name", "hills_types", text(&types)]),
        ExitCode::SUCCESS
    );
    let bytes = std::fs::read(&types).unwrap();
    assert_eq!(detect_binary_format(&bytes), Some(BinaryFormat::Pso));
    let opened = CMapTypes::from_bytes(&bytes).expect("opens as a CMapTypes");
    assert!(opened.name == "hills_types");
    assert!(opened.archetypes.is_empty());
}

#[test]
fn a_map_made_with_an_entity_places_it_where_it_was_put_inside_its_extents() {
    let scratch = tempfile::tempdir().unwrap();
    let types = bench_types(scratch.path());
    let map = scratch.path().join("hills.ymap");

    // ⚠ Made before the first command runs: a command installs the schema snapshot it finds, and under nextest this process has
    // none until then, so the two files below are one made with no snapshot and one made with whatever this machine has.
    let without_a_command = {
        let tree = MetaDocument::parse(&SchemaLibrary::default(), std::fs::read(&types).unwrap())
            .expect("the type file reads")
            .instance;
        let mut index = ArchetypeIndex::new();
        index.add(or_world_gta5::map_types::MapTypes::new(&tree).expect("a CMapTypes"));
        let mut made = create::map("hills");
        let placed = Placed {
            scale_xy: 2.0,
            scale_z: 3.0,
            flags: 0x20,
            ..Placed::at(
                HashedString::new(BENCH),
                Vec3::new(2000.0, -3000.0, 50.0),
                60.0,
            )
        };
        create::place(&mut made, &placed).expect("placed");
        create::map_file(&made, &index, BinaryFormat::Pso).expect("saves")
    };

    assert_eq!(
        world(&[
            "new-map",
            "--name",
            "hills",
            "--entity",
            BENCH,
            "--at",
            "2000,-3000,50",
            "--lod-dist",
            "60",
            "--scale",
            "2,3",
            "--flags",
            "0x20",
            "--types",
            text(&types),
            text(&map),
        ]),
        ExitCode::SUCCESS
    );
    let written = std::fs::read(&map).unwrap();
    assert_eq!(
        written, without_a_command,
        "the command writes the bytes the library writes with no snapshot installed"
    );
    let opened = CMapData::from_bytes(&written).expect("opens");
    assert!(opened.name == "hills");
    assert_eq!(opened.entities.len(), 1);
    let entity = &opened.entities[0];
    assert!(entity.archetype_name == BENCH);
    let at = Vec3::new(2000.0, -3000.0, 50.0);
    assert_eq!(vec3(entity.position), at);
    assert_eq!((entity.scale_xy, entity.scale_z), (2.0, 3.0));
    assert_eq!((entity.lod_dist, entity.flags), (60.0, 0x20));
    assert_eq!((entity.guid, entity.parent_index), (1, -1));
    // The cube is two on a side, scaled by two across and three up.
    assert_eq!(
        vec3(opened.entities_extents_min),
        at - Vec3::new(2.0, 2.0, 3.0)
    );
    assert_eq!(
        vec3(opened.entities_extents_max),
        at + Vec3::new(2.0, 2.0, 3.0)
    );
}

#[test]
fn the_turn_the_scale_and_the_flags_of_an_entity_are_what_was_typed() {
    let scratch = tempfile::tempdir().unwrap();
    let types = bench_types(scratch.path());
    let map = scratch.path().join("hills.ymap");
    assert_eq!(
        world(&[
            "new-map",
            "--name",
            "hills",
            "--entity",
            BENCH,
            "--at",
            "10,20,30",
            "--lod-dist",
            "60",
            "--turn",
            "0,0,90",
            "--scale",
            "2",
            "--flags",
            "32",
            "--types",
            text(&types),
            text(&map),
        ]),
        ExitCode::SUCCESS
    );
    let opened = CMapData::from_bytes(&std::fs::read(&map).unwrap()).expect("opens");
    let entity = &opened.entities[0];
    let stored = entity.rotation;
    let placing = or_world_gta5::transform::Placement::Entity
        .rotation_of([stored.x, stored.y, stored.z, stored.w]);
    let quarter = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    assert!(placing.abs_diff_eq(quarter, 1e-6), "{placing}");
    assert_eq!(
        (entity.scale_xy, entity.scale_z),
        (2.0, 2.0),
        "one number scales every axis"
    );
    assert_eq!(entity.flags, 32, "a number with no 0x is decimal");
}

#[test]
fn an_option_of_an_entity_with_no_entity_and_a_map_with_no_name_are_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let types = bench_types(scratch.path());
    let out = scratch.path().join("refused.ymap");
    // Each of these would make an empty map and say nothing of the option it dropped.
    for option in [
        ["--at", "1,2,3"],
        ["--lod-dist", "60"],
        ["--turn", "0,0,90"],
        ["--scale", "2"],
        ["--flags", "32"],
        ["--types", text(&types)],
    ] {
        assert_eq!(
            world(&[
                "new-map",
                "--name",
                "hills",
                option[0],
                option[1],
                text(&out)
            ]),
            ExitCode::FAILURE,
            "{option:?}"
        );
    }
    assert_eq!(
        world(&["new-map", "--name", "", text(&out)]),
        ExitCode::FAILURE
    );
    assert!(!out.exists(), "no refusal leaves a file behind");
}

#[test]
fn what_cannot_be_written_is_refused_and_leaves_no_file() {
    let scratch = tempfile::tempdir().unwrap();
    let types = bench_types(scratch.path());
    let out = scratch.path().join("refused.ymap");
    let entity = |at: &'static str| {
        vec![
            "new-map",
            "--name",
            "hills",
            "--entity",
            BENCH,
            "--at",
            at,
            "--lod-dist",
            "60",
        ]
    };
    let with = |mut args: Vec<&'static str>, more: &[&str]| -> ExitCode {
        let mut all: Vec<String> = args.drain(..).map(str::to_string).collect();
        all.extend(more.iter().map(|a| a.to_string()));
        all.push(text(&out).to_string());
        let borrowed: Vec<&str> = all.iter().map(String::as_str).collect();
        world(&borrowed)
    };

    // RBM carries a signature word nothing derives.
    assert_eq!(
        with(vec!["new-map", "--name", "hills", "--as", "rbm"], &[]),
        ExitCode::FAILURE
    );
    assert_eq!(
        with(
            vec!["archetypes", "--name", "hills_types", "--as", "rbm"],
            &[]
        ),
        ExitCode::FAILURE
    );
    // A position that is no number, and one past what a number holds.
    for at in ["nan,0,0", "0,inf,0", "0,0,1e39"] {
        assert_eq!(
            with(entity(at), &["--types", text(&types)]),
            ExitCode::FAILURE,
            "{at}"
        );
    }
    // An entity whose box nothing declares would stand outside the extents.
    assert_eq!(with(entity("1,2,3"), &[]), ExitCode::FAILURE);
    let stranger = vec![
        "new-map",
        "--name",
        "hills",
        "--entity",
        "prop_nobody_declares",
        "--at",
        "1,2,3",
        "--lod-dist",
        "60",
    ];
    assert_eq!(
        with(stranger, &["--types", text(&types)]),
        ExitCode::FAILURE
    );
    assert!(!out.exists(), "no refusal leaves a file behind");

    // The same entity with its type file is written.
    assert_eq!(
        with(entity("1,2,3"), &["--types", text(&types)]),
        ExitCode::SUCCESS
    );
    assert!(out.exists());
}

/// ⚠ Reads one staged fragment, so a checkout with no corpus skips it.
#[test]
fn a_type_file_made_from_nothing_declares_a_staged_fragment_with_its_bounds() {
    const WHAT: &str = "a_type_file_made_from_nothing_declares_a_staged_fragment_with_its_bounds";
    let Some(files) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "yft")
        .limit(1)
        .find_or_skip(WHAT)
    else {
        return;
    };
    let fragment = &files[0];
    let stem = fragment
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("a staged file has a name")
        .to_ascii_lowercase();
    let scratch = tempfile::tempdir().unwrap();
    let out = scratch.path().join("staged_types.ytyp");
    assert_eq!(
        world(&[
            "archetypes",
            "--name",
            "staged_types",
            "--lod-dist",
            "120",
            text(fragment),
            text(&out),
        ]),
        ExitCode::SUCCESS
    );
    let written = std::fs::read(&out).unwrap();
    assert_eq!(detect_binary_format(&written), Some(BinaryFormat::Pso));
    let opened = CMapTypes::from_bytes(&written).expect("opens");
    assert!(opened.name == "staged_types");
    assert_eq!(opened.archetypes.len(), 1);
    let declared = &opened.archetypes[0];
    assert!(declared.name == stem.as_str() && declared.asset_name == stem.as_str());
    assert_eq!(declared.lod_dist, 120.0);
    let (lo, hi) = (vec3(declared.bb_min), vec3(declared.bb_max));
    assert!(
        lo.cmple(hi).all() && lo != hi,
        "the fragment's box: {lo} to {hi}"
    );
}
