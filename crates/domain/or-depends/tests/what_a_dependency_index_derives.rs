//! What the resolver derives, and how much of it a run actually touched.
//!
//! ⚠ A check that resolves nothing passes, so each test asserts how many
//! edges it considered, computed from the fixture rather than transcribed.
//! The synthetic tests build real `.rpf` archives and mount them, so the path
//! from an archive entry to a resolved dependency runs with no game present.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_depends::{Census, ExportError, Exported, Found, Resolver};
use or_io::container::rpf::crypto::NoOpCryptoProvider;
use or_io::container::rpf::RpfBuilder;
use or_io::decl::{File as _, Gen8};
use or_io::gta5::shared::Vec4V;
use or_io::gta5::{GtaDrawable, PgDictionary, PhBoundAny, PhBoundSphere};
use or_io::meta::instance::value::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::{GtaInstall, MountedGame};
use or_world_gta5::{
    ArchetypeAssetIndex, Catalogue, InstallTables, Kind, ModelContents, PlacementIndex, Via,
};

fn hash(name: &str) -> u32 {
    or_io::string_hash(name.as_bytes())
}

/// An install root holding one `common.rpf` with the named leaves in it, and the folder's guard.
fn synthetic_install(tag: &str, leaves: &[&str]) -> (tempfile::TempDir, PathBuf) {
    install_of(
        tag,
        leaves.iter().map(|leaf| (*leaf, vec![0u8; 32])).collect(),
    )
}

/// An install root holding one `common.rpf` with these files in it, and the folder's guard.
fn install_of(tag: &str, files: Vec<(&str, Vec<u8>)>) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix(&format!("or-depends-{tag}-"))
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().to_path_buf();
    let mut builder = RpfBuilder::default();
    for (leaf, bytes) in files {
        builder
            .insert_with(leaf, bytes, false)
            .unwrap_or_else(|e| panic!("add {leaf}: {e}"));
    }
    builder.save(root.join("common.rpf")).expect("save");
    (scratch, root)
}

fn mount(root: &Path) -> MountedGame {
    build_mounted_game(
        &GtaInstall {
            root: root.to_path_buf(),
        },
        Arc::new(NoOpCryptoProvider::new()),
        DlcCutoff::BaseGame,
    )
    .expect("the synthetic install mounts")
}

/// `(archetype, assetName, textureDictionary, physicsDictionary)`.
const FIXTURE: &[(&str, &str, &str, &str)] = &[
    ("prop_a", "prop_a", "shared_txd", "prop_a_bound"),
    ("prop_b", "prop_b", "shared_txd", ""),
    ("prop_c", "prop_c", "absent_txd", ""),
];

fn fixture_types() -> MetaInstance {
    let items = FIXTURE
        .iter()
        .map(|(name, asset, txd, ybn)| {
            let mut arch = MetaInstance::new("CBaseArchetypeDef");
            arch.set("name", Value::Hash(HashedString::new(*name)));
            arch.set("assetName", Value::Hash(HashedString::new(*asset)));
            arch.set(
                "assetType",
                Value::Enum {
                    type_name: "assetType".into(),
                    variant: "ASSET_TYPE_DRAWABLE".into(),
                },
            );
            arch.set("textureDictionary", Value::Hash(HashedString::new(*txd)));
            if !ybn.is_empty() {
                arch.set("physicsDictionary", Value::Hash(HashedString::new(*ybn)));
            }
            arch.set("bbMin", Value::Vec3([-1.0; 3]));
            arch.set("bbMax", Value::Vec3([1.0; 3]));
            Value::Pointer(Some(Box::new(arch)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new("fixture_types")));
    root.set("archetypes", Value::Array(items));
    root
}

/// How many edges the fixture must produce, derived from the fixture.
fn expected_edges() -> u64 {
    FIXTURE
        .iter()
        .map(|(_, asset, txd, ybn)| {
            // one declaring type file, plus each non-empty field
            1 + u64::from(!asset.is_empty())
                + u64::from(!txd.is_empty())
                + u64::from(!ybn.is_empty())
        })
        .sum()
}

fn census_over(catalogue: &Catalogue) -> Census {
    let tree = fixture_types();
    let mut index = ArchetypeAssetIndex::new();
    index.add(or_world_gta5::MapTypes::new(&tree).expect("a CMapTypes"));
    let tables = InstallTables::default();
    let resolver = Resolver::new(catalogue, &index, &tables);
    let mut census = Census::new();
    for name in index.names() {
        match resolver.of_archetype(name) {
            Some(set) => census.record_probing(&set, Some(catalogue)),
            None => census.miss(),
        }
    }
    census
}

fn catalogue_over(game: &MountedGame) -> Catalogue {
    let mut catalogue = Catalogue::new();
    game.for_each_entry(|facts| catalogue.index(facts));
    catalogue
}

#[test]
fn a_synthetic_install_resolves_every_edge_its_files_can_answer() {
    let (_scratch, root) = synthetic_install(
        "complete",
        &[
            "props/prop_a.ydr",
            "props/prop_b.ydr",
            "props/prop_c.ydr",
            "props/shared_txd.ytd",
            "props/prop_a_bound.ybn",
            "types/fixture_types.ytyp",
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let census = census_over(&catalogue);

    assert!(!census.is_vacuous(), "nothing was exercised");
    assert_eq!(
        census.exercised(),
        expected_edges(),
        "the fixture states {} edges; a run considering another number is \
         resolving something other than what this test set up",
        expected_edges()
    );
    assert_eq!(census.asked, FIXTURE.len() as u64);
    assert_eq!(census.answered, FIXTURE.len() as u64);

    // `absent_txd` is the one thing the install deliberately does not hold.
    assert_eq!(census.edges.unresolved(), 1);
    assert_eq!(census.complete, 2);
    assert_eq!(
        census.coverage_of(Kind::TextureDict).considered,
        3,
        "one texture dictionary per archetype"
    );
    assert_eq!(census.coverage_of(Kind::TextureDict).resolved, 2);
    assert_eq!(census.coverage_via(Via::DeclaringTypeFile).resolved, 3);
    assert_eq!(census.asset_type_agreed, 3);
    assert_eq!(census.asset_type_disagreed, 0);
    assert_eq!(
        *census
            .missing_entirely
            .get(&Via::ArchetypeTextureDictionary)
            .unwrap_or(&0),
        1
    );
}

/// ⚠ An index that quietly stopped looking would show the same coverage with
/// a smaller denominator.
#[test]
fn removing_one_file_moves_the_coverage_and_not_the_denominator() {
    let (_full_scratch, full) = synthetic_install(
        "full",
        &[
            "props/prop_a.ydr",
            "props/prop_b.ydr",
            "props/prop_c.ydr",
            "props/shared_txd.ytd",
            "props/prop_a_bound.ybn",
            "types/fixture_types.ytyp",
        ],
    );
    let (_thinner_scratch, thinner) = synthetic_install(
        "thin",
        &[
            "props/prop_a.ydr",
            "props/prop_b.ydr",
            "props/prop_c.ydr",
            "props/prop_a_bound.ybn",
            "types/fixture_types.ytyp",
        ],
    );

    let before = census_over(&catalogue_over(&mount(&full)));
    let after = census_over(&catalogue_over(&mount(&thinner)));

    assert_eq!(
        before.exercised(),
        after.exercised(),
        "the same archetypes state the same edges either way"
    );
    assert_eq!(before.edges.unresolved(), 1);
    assert_eq!(
        after.edges.unresolved(),
        3,
        "two archetypes named `shared_txd` and it is gone"
    );
    assert!(after.edges.resolved < before.edges.resolved);
}

/// A placement query over maps built here, so the walk is exercised even
/// where no install is.
#[test]
fn a_placement_query_counts_what_it_walked_as_well_as_what_it_found() {
    let maps: Vec<MetaInstance> = ["m_north", "m_south"]
        .iter()
        .map(|name| {
            let entities = ["prop_a", "prop_b", "prop_a"]
                .iter()
                .map(|a| {
                    let mut ent = MetaInstance::new("CEntityDef");
                    ent.set("archetypeName", Value::Hash(HashedString::new(*a)));
                    ent.set("position", Value::Vec3([1.0, 2.0, 3.0]));
                    Value::Pointer(Some(Box::new(ent)))
                })
                .collect();
            let mut root = MetaInstance::new("CMapData");
            root.set("name", Value::Hash(HashedString::new(*name)));
            root.set("entities", Value::Array(entities));
            root
        })
        .collect();

    let mut index = PlacementIndex::watching([hash("prop_a")]);
    for tree in &maps {
        index.add(&or_world_gta5::MapData::new(tree).expect("a CMapData"));
    }

    assert_eq!(index.seen(), 6, "every placement walked, watched or not");
    assert_eq!(index.held(), 4);
    assert_eq!(index.maps_placing(hash("prop_a")).len(), 2);
    assert_eq!(index.of(hash("prop_b")).len(), 0, "not watched");
}

/// A `CMapTypes` of `(name, assetName, drawableDictionary, physicsDictionary)`.
fn model_types(archetypes: &[(&str, &str, &str, &str)]) -> MetaInstance {
    let items = archetypes
        .iter()
        .map(|(name, asset, ydd, physics)| {
            let mut arch = MetaInstance::new("CBaseArchetypeDef");
            arch.set("name", Value::Hash(HashedString::new(*name)));
            arch.set("assetName", Value::Hash(HashedString::new(*asset)));
            if !ydd.is_empty() {
                arch.set("drawableDictionary", Value::Hash(HashedString::new(*ydd)));
            }
            if !physics.is_empty() {
                arch.set(
                    "physicsDictionary",
                    Value::Hash(HashedString::new(*physics)),
                );
            }
            Value::Pointer(Some(Box::new(arch)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new("model_types")));
    root.set("archetypes", Value::Array(items));
    root
}

/// ⚠ A `physicsDictionary` names no file in nearly every case: an archetype
/// stating one has a model carrying its own bound, and that bound is the
/// edge. A `.ydd` entry is confirmed by reading the `.ydd`, never by its name.
#[test]
fn a_model_body_settles_its_collision_and_its_dictionary_entries() {
    let with_bound = GtaDrawable::<Gen8> {
        bound: Some(Box::new(PhBoundAny::PhBoundSphere(Default::default()))),
        ..Default::default()
    };
    let dictionary = PgDictionary::<GtaDrawable<Gen8>> {
        codes: vec![hash("prop_in")],
        entries: vec![Some(Box::new(GtaDrawable::default()))],
        ..Default::default()
    };
    let (_scratch, root) = install_of(
        "bodies",
        vec![
            (
                "props/prop_solid.ydr",
                with_bound.to_bytes().expect("a drawable"),
            ),
            (
                "props/prop_hollow.ydr",
                GtaDrawable::<Gen8>::default()
                    .to_bytes()
                    .expect("a drawable"),
            ),
            (
                "props/pack.ydd",
                dictionary.to_bytes().expect("a dictionary"),
            ),
            ("types/model_types.ytyp", vec![0u8; 32]),
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let tree = model_types(&[
        ("prop_solid", "prop_solid", "", "model_types"),
        ("prop_hollow", "prop_hollow", "", "model_types"),
        ("prop_in", "prop_in", "pack", ""),
        ("prop_gone", "prop_gone", "pack", ""),
    ]);
    let mut index = ArchetypeAssetIndex::new();
    index.add(or_world_gta5::MapTypes::new(&tree).expect("a CMapTypes"));
    let contents = ModelContents::read(&game, &catalogue, &index);
    assert_eq!((contents.dictionaries(), contents.drawables()), (1, 2));
    assert_eq!(contents.unreadable(), 0);

    let tables = InstallTables::default();
    let resolver = Resolver::new(&catalogue, &index, &tables).reading(&contents);
    let edge = |archetype: &str, via: Via| {
        let set = resolver.of_archetype(hash(archetype)).expect("declared");
        set.edges
            .into_iter()
            .find(|e| e.via == via)
            .expect("the edge")
    };

    let solid = edge("prop_solid", Via::ArchetypePhysicsDictionary);
    assert_eq!(solid.name, hash("model_types"));
    assert!(
        matches!(&solid.found, Found::Entry(p) if p.ends_with("prop_solid.ydr")),
        "the bound inside the model, not a file named `model_types`: {:?}",
        solid.found
    );
    assert_eq!(
        edge("prop_hollow", Via::ArchetypePhysicsDictionary).found,
        Found::Missing,
        "a model with no bound of its own leaves the collision unresolved"
    );

    assert!(matches!(
        &edge("prop_in", Via::ArchetypeAsset).found,
        Found::Entry(p) if p.ends_with("pack.ydd")
    ));
    assert_eq!(
        edge("prop_gone", Via::ArchetypeAsset).found,
        Found::Missing,
        "a `.ydd` that does not hold the entry does not resolve it"
    );
}

/// An archetype whose `assetName` the install holds only as a `.ybn` is a
/// collision-only archetype: that `.ybn` is its asset, and its collision
/// where its `physicsDictionary` names no file.
#[test]
fn an_archetype_whose_asset_is_only_a_bound_resolves_to_that_bound() {
    let (_scratch, root) = synthetic_install(
        "bound-only",
        &[
            "props/col_only.ybn",
            "props/col_marked.ybn",
            "types/model_types.ytyp",
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let tree = model_types(&[
        ("col_only", "col_only", "", ""),
        ("col_marked", "col_marked", "", "names_no_file"),
    ]);
    let mut index = ArchetypeAssetIndex::new();
    index.add(or_world_gta5::MapTypes::new(&tree).expect("a CMapTypes"));
    let tables = InstallTables::default();
    let resolver = Resolver::new(&catalogue, &index, &tables);

    let set = resolver.of_archetype(hash("col_only")).expect("declared");
    let asset = set.asset_edge().expect("an asset edge");
    assert_eq!(asset.kind, Kind::Bounds);
    assert!(
        matches!(&asset.found, Found::File(p) if p.ends_with("col_only.ybn")),
        "{:?}",
        asset.found
    );
    assert_eq!(set.found_asset_kind, Some(Kind::Bounds));
    assert_eq!(set.unresolved(), 0, "{:#?}", set.edges);

    let set = resolver.of_archetype(hash("col_marked")).expect("declared");
    let physics = set
        .edges
        .iter()
        .find(|e| e.via == Via::ArchetypePhysicsDictionary)
        .expect("the edge");
    assert_eq!(physics.name, hash("names_no_file"));
    assert!(
        matches!(&physics.found, Found::File(p) if p.ends_with("col_marked.ybn")),
        "{:?}",
        physics.found
    );
    assert_eq!(set.unresolved(), 0, "{:#?}", set.edges);
    let mut census = Census::new();
    census.record(&set);
    assert_eq!(census.physics.the_assets_own_bound, 1);
    assert_eq!(census.physics.total(), 1);
}

/// A `CPackFileMetaData` whose `field` array names each item's bounds.
fn manifest_naming(field: &str, item: &str, groups: &[(&str, &[&str])]) -> MetaInstance {
    let items = groups
        .iter()
        .map(|(name, bounds)| {
            let mut group = MetaInstance::new(item);
            group.set("Name", Value::Hash(HashedString::new(*name)));
            group.set(
                "Bounds",
                Value::Array(
                    (bounds.iter())
                        .map(|b| Value::Hash(HashedString::new(*b)))
                        .collect(),
                ),
            );
            Value::Struct(Box::new(group))
        })
        .collect();
    let mut root = MetaInstance::new("CPackFileMetaData");
    root.set(field, Value::Array(items));
    root
}

/// ⚠ A map's static bounds are named only by the manifest's `MapDataGroups`,
/// for a map that is an imap group: a `.ybn` sharing a map's stem is not
/// that map's collision by any reference.
#[test]
fn a_maps_static_bounds_resolve_through_its_manifest_group() {
    let (_scratch, root) = synthetic_install(
        "static-bounds",
        &[
            "maps/m_group.ymap",
            "maps/m_group_0.ybn",
            "maps/hi@m_group_0.ybn",
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let stated: &[&str] = &["m_group_0", "hi@m_group_0", "m_group_1"];
    let manifest = manifest_naming("MapDataGroups", "CMapDataGroup", &[("m_group", stated)]);
    let mut tables = InstallTables::new();
    tables.add_manifest(or_world_gta5::Manifest::new(&manifest).expect("a CPackFileMetaData"));
    let archetypes = ArchetypeAssetIndex::new();
    let resolver = Resolver::new(&catalogue, &archetypes, &tables);

    let set = resolver
        .of_map(hash("m_group"))
        .expect("the install holds it");
    let names: Vec<u32> = set.of_kind(Kind::Bounds).map(|e| e.name).collect();
    assert_eq!(
        names,
        stated.iter().map(|b| hash(b)).collect::<Vec<_>>(),
        "every bound the group states, in its order"
    );
    assert!(set.edges.iter().all(|e| e.via == Via::ManifestMapDataGroup));
    assert_eq!(set.resolved(), 2);
    assert!(
        !set.edges[2].is_resolved(),
        "`m_group_1` is stated and not installed, so it is reported"
    );
    assert!(matches!(&set.edges[1].found, Found::File(p) if p.ends_with("hi@m_group_0.ybn")));
    assert!(resolver.of_map(hash("m_absent")).is_none());
}

/// A `.ybn` whose header box spans `min` to `max`.
fn bound_spanning(min: [f32; 3], max: [f32; 3]) -> Vec<u8> {
    let mut sphere = PhBoundSphere::default();
    sphere.base.bounding_box_max_xyz_margin_w = Vec4V {
        x: max[0],
        y: max[1],
        z: max[2],
        ..Default::default()
    };
    sphere.base.set_bounding_box_min(min);
    PhBoundAny::PhBoundSphere(sphere)
        .to_bytes()
        .expect("a bound")
}

/// ⚠ A map no imap group names takes its static bounds from the filename
/// stem, which no file states: each edge is labelled a convention, and one
/// whose box leaves the map's streaming extents is kept and flagged.
#[test]
fn a_non_group_maps_static_bounds_resolve_by_filename_stem() {
    let inside = || bound_spanning([0.0; 3], [10.0; 3]);
    let (_scratch, root) = install_of(
        "stem-bounds",
        vec![
            ("maps/m_plain.ymap", vec![0u8; 32]),
            ("maps/m_plain_0.ybn", inside()),
            ("maps/hi@m_plain_0.ybn", inside()),
            (
                "maps/ma@m_plain_1.ybn",
                bound_spanning([500.0; 3], [510.0; 3]),
            ),
            ("maps/m_plainer_0.ybn", inside()),
            ("maps/m_plain_lod.ybn", inside()),
            ("maps/m_group.ymap", vec![0u8; 32]),
            ("maps/m_group_0.ybn", inside()),
            ("maps/m_group_7.ybn", inside()),
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let manifest = manifest_naming(
        "MapDataGroups",
        "CMapDataGroup",
        &[("m_group", &["m_group_0"])],
    );
    let mut tables = InstallTables::new();
    tables.add_manifest(or_world_gta5::Manifest::new(&manifest).expect("a CPackFileMetaData"));
    let mut map = MetaInstance::new("CMapData");
    map.set("streamingExtentsMin", Value::Vec3([-100.0; 3]));
    map.set("streamingExtentsMax", Value::Vec3([100.0; 3]));
    let mut contents = ModelContents::new();
    assert!(contents.add_map(
        hash("m_plain"),
        or_world_gta5::MapData::new(&map).expect("a CMapData")
    ));
    let stem: &[&str] = &["m_plain_0", "hi@m_plain_0", "ma@m_plain_1"];
    for bound in stem {
        let bytes = (game.vfs)
            .open(&catalogue.find_named(bound, Kind::Bounds).unwrap().logical)
            .expect("installed");
        assert!(contents.add_bound(hash(bound), &bytes), "{bound}");
    }
    let archetypes = ArchetypeAssetIndex::new();

    let unread = Resolver::new(&catalogue, &archetypes, &tables)
        .of_map(hash("m_plain"))
        .expect("the install holds it");
    let mut names: Vec<u32> = unread.edges.iter().map(|e| e.name).collect();
    names.sort_unstable();
    let mut expected: Vec<u32> = stem.iter().map(|b| hash(b)).collect();
    expected.sort_unstable();
    assert_eq!(
        names, expected,
        "`<map>_N`, `hi@<map>_N` and `ma@<map>_N`, and no other stem: {:#?}",
        unread.edges
    );
    assert_eq!(unread.resolved(), stem.len());
    assert!(unread.edges.iter().all(|e| !e.via.is_stated()
        && e.via
            == Via::MapStemConvention {
                inside_streaming_box: None
            }));

    let resolver = Resolver::new(&catalogue, &archetypes, &tables).reading(&contents);
    let set = resolver
        .of_map(hash("m_plain"))
        .expect("the install holds it");
    let verdict = |bound: &str| {
        (set.edges.iter())
            .find(|e| e.name == hash(bound))
            .map(|e| e.via)
            .expect("the edge")
    };
    for (bound, inside) in [
        ("m_plain_0", true),
        ("hi@m_plain_0", true),
        ("ma@m_plain_1", false),
    ] {
        assert_eq!(
            verdict(bound),
            Via::MapStemConvention {
                inside_streaming_box: Some(inside)
            },
            "{bound}"
        );
    }
    assert_eq!(set.unresolved(), 0, "a flagged bound is still reported");

    let group = resolver
        .of_map(hash("m_group"))
        .expect("the install holds it");
    assert_eq!(
        group
            .edges
            .iter()
            .map(|e| (e.via, e.name))
            .collect::<Vec<_>>(),
        vec![(Via::ManifestMapDataGroup, hash("m_group_0"))],
        "a group map's bounds are the manifest's, not its stem's"
    );
}

/// ⚠ An interior's collision is named only by the manifest's `Interiors`,
/// keyed by the interior archetype: a `.ybn` sharing its stem is not its
/// collision by any reference.
#[test]
fn an_interior_archetypes_bounds_resolve_through_the_manifest() {
    let (_scratch, root) = synthetic_install(
        "interior-bounds",
        &[
            "int/v_int_test.ytyp",
            "int/v_int_test_0.ybn",
            "int/hi@v_int_test_0.ybn",
            "int/v_int_test_9.ybn",
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let stated: &[&str] = &["v_int_test_0", "hi@v_int_test_0", "v_int_test_1"];
    let manifest = manifest_naming(
        "Interiors",
        "CInteriorBoundsFiles",
        &[("v_int_test", stated)],
    );
    let mut tables = InstallTables::new();
    tables.add_manifest(or_world_gta5::Manifest::new(&manifest).expect("a CPackFileMetaData"));
    let mut interior = MetaInstance::new("CMloArchetypeDef");
    interior.set("name", Value::Hash(HashedString::new("v_int_test")));
    let mut types = MetaInstance::new("CMapTypes");
    types.set("name", Value::Hash(HashedString::new("v_int_test")));
    types.set(
        "archetypes",
        Value::Array(vec![Value::Pointer(Some(Box::new(interior)))]),
    );
    let mut archetypes = ArchetypeAssetIndex::new();
    archetypes.add(or_world_gta5::MapTypes::new(&types).expect("a CMapTypes"));
    let resolver = Resolver::new(&catalogue, &archetypes, &tables);

    let set = resolver
        .of_archetype(hash("v_int_test"))
        .expect("the archetype is declared");
    let bounds: Vec<_> = set.of_kind(Kind::Bounds).collect();
    assert_eq!(
        bounds.iter().map(|e| e.name).collect::<Vec<_>>(),
        stated.iter().map(|b| hash(b)).collect::<Vec<_>>(),
        "every bound the manifest states for the interior, in its order: {:#?}",
        set.edges
    );
    assert!(bounds.iter().all(|e| e.via == Via::ManifestInterior));
    assert_eq!(bounds.iter().filter(|e| e.is_resolved()).count(), 2);
    assert!(
        !bounds[2].is_resolved(),
        "`v_int_test_1` is stated and not installed, so it is reported"
    );
    assert!(matches!(&bounds[1].found, Found::File(p) if p.ends_with("hi@v_int_test_0.ybn")));
}

/// The files `prop_a` needs as the fixture installs them, each with its own
/// fill: the model (which holds its skeleton), its texture dictionary, its
/// collision and its type file.
const PROP_A_SET: &[(&str, u8)] = &[
    ("props/prop_a.ydr", 1),
    ("props/shared_txd.ytd", 2),
    ("props/prop_a_bound.ybn", 3),
    ("types/fixture_types.ytyp", 4),
];

fn export_of(
    tag: &str,
    files: &[(&str, u8)],
) -> (
    tempfile::TempDir,
    PathBuf,
    Result<Vec<Exported>, ExportError>,
) {
    let (scratch, root) = install_of(
        tag,
        files
            .iter()
            .map(|(leaf, fill)| (*leaf, vec![*fill; 48]))
            .collect(),
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let tree = fixture_types();
    let mut index = ArchetypeAssetIndex::new();
    index.add(or_world_gta5::MapTypes::new(&tree).expect("a CMapTypes"));
    let tables = InstallTables::default();
    let resolver = Resolver::new(&catalogue, &index, &tables);
    let set = resolver.of_archetype(hash("prop_a")).expect("declared");
    let result = or_depends::export(&set, &game, &root.join("export"));
    (scratch, root, result)
}

#[test]
fn an_export_writes_the_whole_set_with_a_manifest() {
    let (_scratch, root, result) = export_of("export-whole", PROP_A_SET);
    let exported = result.expect("every piece is in the install");
    let out = root.join("export");

    assert_eq!(exported.len(), PROP_A_SET.len(), "{exported:#?}");
    let manifest = std::fs::read_to_string(out.join(or_depends::MANIFEST)).expect("a manifest");
    for (path, fill) in PROP_A_SET {
        let leaf = path.rsplit('/').next().expect("a leaf");
        assert_eq!(
            std::fs::read(out.join(leaf)).expect(leaf),
            vec![*fill; 48],
            "{leaf} is the install's bytes"
        );
        assert!(
            manifest
                .lines()
                .any(|l| l.starts_with(&format!("{leaf}\t"))),
            "the manifest lists {leaf}:\n{manifest}"
        );
    }
    for via in [
        Via::ArchetypeAsset,
        Via::ArchetypeTextureDictionary,
        Via::ArchetypePhysicsDictionary,
        Via::DeclaringTypeFile,
    ] {
        assert!(
            manifest.contains(&format!("\t{via:?}\t")),
            "the manifest says why each file is there; no {via:?} in\n{manifest}"
        );
    }
}

/// ⚠ An export missing one piece looks like it worked, so a set the install
/// cannot complete writes nothing and names what it lacks.
#[test]
fn an_export_missing_one_piece_refuses_and_names_it() {
    let without_collision: Vec<(&str, u8)> = PROP_A_SET
        .iter()
        .copied()
        .filter(|(path, _)| !path.ends_with(".ybn"))
        .collect();
    let (_scratch, root, result) = export_of("export-short", &without_collision);

    let message = result.expect_err("the collision is missing").to_string();
    assert!(
        message.contains("ArchetypePhysicsDictionary")
            && message.contains(&format!("{:#010X}", hash("prop_a_bound"))),
        "the refusal names the missing piece: {message}"
    );
    assert!(
        !root.join("export").exists(),
        "a refused export writes nothing"
    );
}

/// An asset is asked for by its model's name as readily as by its archetype's.
#[test]
fn an_archetype_is_found_by_the_asset_it_names() {
    let tree = model_types(&[("arch_x", "model_x", "", "")]);
    let mut index = ArchetypeAssetIndex::new();
    index.add(or_world_gta5::MapTypes::new(&tree).expect("a CMapTypes"));
    assert_eq!(index.archetype_of(hash("arch_x")), Some(hash("arch_x")));
    assert_eq!(index.archetype_of(hash("model_x")), Some(hash("arch_x")));
    assert_eq!(index.archetype_of(hash("nothing")), None);
}

const WHAT: &str = "a_real_install_resolves_what_its_type_files_declare";

/// Floors rather than the measured number, which grows with every title
/// update: retail Legacy measured 159,153 archetypes over 695,838 edges, and
/// these are roughly a tenth of that.
const MIN_ARCHETYPES: u64 = 10_000;
const MIN_EDGES: u64 = 40_000;

#[test]
fn a_real_install_resolves_what_its_type_files_declare() {
    let roots = or_test_support::game_roots();
    let Some(root) = roots.first() else {
        or_test_support::skip_install(
            WHAT,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return;
    };
    let exe = or_title::Game::detect_from_root(root).and_then(|game| game.find_executable(root));
    let Some(exe) = exe else {
        or_test_support::skip_install(WHAT, "no executable, so no keys");
        return;
    };
    let Ok(keys) = or_io::container::rpf::crypto::extract_keys(&exe, &|_| {}) else {
        or_test_support::skip_install(WHAT, "key extraction failed");
        return;
    };
    let crypto: Arc<dyn or_io::container::rpf::crypto::RpfCryptoProvider> =
        Arc::new(or_io::container::rpf::crypto::Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let game = build_mounted_game(&GtaInstall { root: root.clone() }, crypto, DlcCutoff::All)
        .expect("the install mounts");

    let catalogue = catalogue_over(&game);

    // One winner per leaf, as the engine resolves it.
    let mut winners: std::collections::BTreeMap<String, String> = Default::default();
    game.for_each_file(|logical, _c, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ytyp") {
            winners.insert(lower, logical.to_string());
        }
    });

    let mut archetypes = ArchetypeAssetIndex::new();
    // ⚠ One tree at a time. A retail install's `.ytyp` trees held together
    // are the shape of run that reached 38 GB and was killed.
    for path in winners.values() {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let mut diags = or_io::meta::Diagnostics::new(or_io::meta::ErrorMode::Lenient);
        let Ok((tree, _)) =
            or_io::meta::binary::parse_meta_binary_full(schema(), &bytes, &mut diags)
        else {
            continue;
        };
        if let Some(types) = or_world_gta5::MapTypes::new(&tree) {
            archetypes.add(types);
        }
    }

    let tables = InstallTables::default();
    let resolver = Resolver::new(&catalogue, &archetypes, &tables);
    let mut census = Census::new();
    for name in archetypes.names() {
        match resolver.of_archetype(name) {
            Some(set) => census.record(&set),
            None => census.miss(),
        }
    }

    eprintln!(
        "\n=== {} archetype(s), {} edge(s), {} resolved ===",
        census.asked,
        census.exercised(),
        census.edges.resolved
    );

    assert!(
        !census.is_vacuous(),
        "an install was present and nothing was exercised"
    );
    assert!(
        census.asked >= MIN_ARCHETYPES,
        "{} archetypes is below the floor of {MIN_ARCHETYPES}; either the type \
         files stopped being read or this is not a full install",
        census.asked
    );
    assert!(
        census.exercised() >= MIN_EDGES,
        "{} edges is below the floor of {MIN_EDGES}",
        census.exercised()
    );

    // ⚠ The one hard invariant. `assetType` is the file's own claim about
    // which family `assetName` belongs to, and the resolver reaches its
    // answer from the install instead; a single disagreement means one of
    // the two is wrong about a real archetype.
    assert_eq!(
        census.asset_type_disagreed, 0,
        "the install and the stated assetType disagree for {} archetype(s)",
        census.asset_type_disagreed
    );
    assert!(
        census.asset_type_agreed >= MIN_ARCHETYPES,
        "only {} archetypes had a type to corroborate, which is too few to \
         mean anything",
        census.asset_type_agreed
    );

    // Every archetype comes from a type file the mount just read, so this
    // route resolving anything less than all of it is a catalogue defect.
    let declaring = census.coverage_via(Via::DeclaringTypeFile);
    assert_eq!(
        declaring.unresolved(),
        0,
        "{} archetype(s) name a declaring `.ytyp` the catalogue cannot find, \
         though the index was built from those same files",
        declaring.unresolved()
    );
    assert!(declaring.considered >= MIN_ARCHETYPES);
}

/// The default library, with the staged snapshot installed when there is one.
fn schema() -> &'static or_io::meta::SchemaLibrary {
    if let Some(path) = or_test_support::schema_snapshot() {
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            panic!(
                "the staged schema snapshot {} is unusable: {e}",
                path.display()
            );
        }
    }
    or_io::meta::default_library()
}

/// A bound whose front gives no root is read whole, and one whose front gives it is not.
#[test]
fn a_bound_whose_front_gives_no_root_is_read_whole() {
    let bound = bound_spanning([0.0; 3], [10.0; 3]);
    let (map_min, map_max) = ([-100.0; 3], [100.0; 3]);
    let mut map = MetaInstance::new("CMapData");
    map.set("streamingExtentsMin", Value::Vec3(map_min));
    map.set("streamingExtentsMax", Value::Vec3(map_max));
    let read = |front: Option<Vec<u8>>, whole: Option<Vec<u8>>| {
        let mut contents = ModelContents::new();
        assert!(contents.add_map(
            hash("m_plain"),
            or_world_gta5::MapData::new(&map).expect("a CMapData")
        ));
        let asked = std::cell::Cell::new(false);
        let ok = contents.read_bound_from(
            hash("m_plain_0"),
            || front,
            || {
                asked.set(true);
                whole
            },
        );
        (ok, asked.get(), contents)
    };

    let (ok, asked, contents) = read(Some(bound.clone()), Some(bound.clone()));
    assert!(ok && !asked, "a front that gives the root is enough");
    assert_eq!(
        contents.bound_inside_map(hash("m_plain_0"), hash("m_plain")),
        Some(true)
    );

    // Sixteen bytes are a header and no stream: no root, as a stored payload cut short gives none
    let (ok, asked, contents) = read(Some(bound[..16].to_vec()), Some(bound.clone()));
    assert!(
        ok && asked,
        "a front with no root falls back to the whole file"
    );
    assert_eq!(
        contents.bound_inside_map(hash("m_plain_0"), hash("m_plain")),
        Some(true),
        "the whole file gives the bound its verdict"
    );
    assert_eq!(contents.unreadable(), 0);

    let (ok, _, contents) = read(Some(bound[..16].to_vec()), Some(bound[..16].to_vec()));
    assert!(!ok, "bytes with no root anywhere give none");
    assert_eq!(
        contents.unreadable(),
        1,
        "and the bound is counted unreadable"
    );

    let (ok, _, contents) = read(None, None);
    assert!(!ok);
    assert_eq!(
        contents.unreadable(),
        0,
        "a file nothing opened is not unreadable"
    );
}

/// A `.ybn` on a folder device, which reads the whole file for its front, is given its verdict as one in an archive is.
#[test]
fn a_loose_bound_is_read_through_a_folder_device_and_given_its_verdict() {
    let (_scratch, root) = install_of(
        "loose-bound",
        vec![
            ("maps/m_loose.ymap", vec![0u8; 32]),
            ("maps/m_loose_0.ybn", bound_spanning([0.0; 3], [10.0; 3])),
            (
                "maps/ma@m_loose_1.ybn",
                bound_spanning([500.0; 3], [510.0; 3]),
            ),
        ],
    );
    let game = mount(&root);
    let catalogue = catalogue_over(&game);
    let mut map = MetaInstance::new("CMapData");
    map.set("streamingExtentsMin", Value::Vec3([-100.0; 3]));
    map.set("streamingExtentsMax", Value::Vec3([100.0; 3]));
    let mut contents = ModelContents::new();
    assert!(contents.add_map(
        hash("m_loose"),
        or_world_gta5::MapData::new(&map).expect("a CMapData")
    ));
    for bound in ["m_loose_0", "ma@m_loose_1"] {
        assert!(
            contents.read_bound(&game, &catalogue, hash(bound)),
            "{bound}"
        );
    }
    assert_eq!(
        contents.bound_inside_map(hash("m_loose_0"), hash("m_loose")),
        Some(true)
    );
    assert_eq!(
        contents.bound_inside_map(hash("ma@m_loose_1"), hash("m_loose")),
        Some(false)
    );
    assert_eq!(contents.unreadable(), 0);
}
