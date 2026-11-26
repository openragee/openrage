//! The models of a mount built by hand as a browser lists them: what each row states, which file holds it, how often it is
//! placed, each filter and sort, the pages, and the catalogue of an index read from its kept file.
//!
//! ⚠ Every expected value is computed from the fixture's own lists: which declaration, map and file wins by the order the
//! archives mount in, a count by adding what the winning maps place, a size from the box a type file was written with.

mod world_fixture;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use glam::Vec3;
use or_depends::find::{find, names_of, place_maps, Asked, Install, Rank, SearchNames};
use or_depends::models::{Model, Query, Sort};
use or_depends::world_index::{refresh, Kept, KeptError, KeptState, RULES, VERSION};
use or_depends::{Kind, ModelCatalogue, MountedWorld, PlacementIndex, WorldIndex, WorldSource};
use or_world::ArchetypeKind;
use world_fixture::{
    bound_bytes, cube, dlc_list_bytes, hash, manifest_bytes, mount_all, placing_map_bytes,
    typed_bytes, write_archive, write_pack, ArchetypeSpec, Counting, MapSpec, Placed, Span,
};

/// A box `x` by `y` by `z` metres about the origin.
fn sized(x: f32, y: f32, z: f32) -> Span {
    ([-x / 2.0, -y / 2.0, -z / 2.0], [x / 2.0, y / 2.0, z / 2.0])
}

/// One type file of the fixture: the archive that holds it, its path there, the device of its pack and what it declares.
struct TypeFileSpec {
    archive: &'static str,
    path: &'static str,
    pack: &'static str,
    archetypes: Vec<ArchetypeSpec>,
}

/// The type files of the fixture, in the order the mount loads them.
///
/// `prop_bench` is declared by the base game and by both packs, and `prop_lamp` twice by the base game, as a time
/// archetype and then as a base one.
fn type_files() -> Vec<TypeFileSpec> {
    let base = ArchetypeSpec::base;
    let interior = ArchetypeSpec {
        kind: ArchetypeKind::Mlo,
        entities: vec!["prop_inside", "prop_bench"],
        set: vec!["prop_inside"],
        ..base("v_int_shop", sized(20.0, 12.0, 4.0))
    };
    vec![
        TypeFileSpec {
            archive: "x64a.rpf",
            path: "types/props.ytyp",
            pack: "platform",
            archetypes: vec![
                // The texture dictionary of a declaration a later one replaces is no model's.
                ArchetypeSpec {
                    texture_dictionary: "bench_txd",
                    lod_dist: 80.0,
                    ..base("prop_bench", sized(2.0, 1.0, 1.0))
                },
                ArchetypeSpec {
                    kind: ArchetypeKind::Time,
                    ..base("prop_lamp", sized(0.5, 0.5, 4.0))
                },
                ArchetypeSpec {
                    kind: ArchetypeKind::Time,
                    ..base("prop_clock", sized(1.0, 1.0, 2.0))
                },
                ArchetypeSpec {
                    drawable_dictionary: "furniture",
                    texture_dictionary: "txd_nowhere",
                    ..base("prop_dict_chair", sized(1.0, 1.0, 1.0))
                },
                ArchetypeSpec {
                    drawable_dictionary: "dict_nowhere",
                    ..base("prop_stool", sized(0.5, 0.5, 0.75))
                },
                ArchetypeSpec {
                    texture_dictionary: "props_txd",
                    ..base("prop_unplaced", sized(10.0, 3.0, 3.0))
                },
                base("prop_inside", sized(0.25, 0.25, 0.5)),
                base("prop_ghost", sized(3.0, 3.0, 3.0)),
                base("prop_col", sized(6.0, 6.0, 1.0)),
                // A box whose least corner is past its greatest is no box, so it has no size.
                base("prop_inverted", ([1.0; 3], [-1.0; 3])),
                // It names no asset, so the dictionary it states and the mount holds has no entry to draw it by.
                ArchetypeSpec {
                    asset_name: "",
                    drawable_dictionary: "furniture",
                    ..base("prop_assetless", sized(1.5, 1.5, 1.5))
                },
                interior,
            ],
        },
        TypeFileSpec {
            archive: "x64b.rpf",
            path: "types/props_b.ytyp",
            pack: "platform",
            archetypes: vec![ArchetypeSpec {
                lod_dist: 150.0,
                ..base("prop_lamp", sized(0.5, 0.5, 5.0))
            }],
        },
        TypeFileSpec {
            archive: "alpha",
            path: "types/alpha_props.ytyp",
            pack: "dlc_alpha",
            archetypes: vec![
                base("prop_bench", sized(4.0, 1.0, 1.0)),
                base("prop_alpha", sized(7.0, 2.0, 2.0)),
            ],
        },
        TypeFileSpec {
            archive: "beta",
            path: "types/beta_props.ytyp",
            pack: "dlc_beta",
            archetypes: vec![ArchetypeSpec {
                lod_dist: 300.0,
                sphere: ([0.5, 0.25, 0.125], 9.0),
                ..base("prop_bench", sized(5.0, 1.0, 1.0))
            }],
        },
    ]
}

/// The maps of the fixture, in the order the mount loads them: the archive, the path, what it places and how often, and
/// the interiors it places. `x64b.rpf`'s `m_town` is the copy the mount loads.
type MapOf = (
    &'static str,
    &'static str,
    Vec<(&'static str, u32)>,
    Vec<&'static str>,
);

fn maps() -> Vec<MapOf> {
    vec![
        (
            "x64a.rpf",
            "levels/m_town.ymap",
            vec![("prop_bench", 7), ("prop_ghost", 4)],
            vec![],
        ),
        (
            "x64a.rpf",
            "levels/m_dock.ymap",
            vec![("prop_bench", 2), ("prop_dict_chair", 1), ("prop_stool", 1)],
            vec![],
        ),
        (
            "x64b.rpf",
            "patch/m_town.ymap",
            vec![("prop_bench", 3), ("prop_lamp", 1)],
            vec!["v_int_shop"],
        ),
        (
            "alpha",
            "levels/m_alpha.ymap",
            vec![("prop_alpha", 2), ("prop_bench", 1)],
            vec![],
        ),
    ]
}

/// The model and texture files of the fixture, in the order the mount lists them. No file is named `prop_ghost`; a bound
/// alone names `prop_col`; `prop_lamp` is a fragment and a drawable; `x64b.rpf` holds the later `prop_bench.ydr`.
const FILES: &[(&str, &str)] = &[
    ("x64a.rpf", "props/prop_bench.ydr"),
    ("x64a.rpf", "props/prop_lamp.ydr"),
    ("x64a.rpf", "props/prop_lamp.yft"),
    ("x64a.rpf", "props/prop_clock.ydr"),
    ("x64a.rpf", "props/furniture.ydd"),
    ("x64a.rpf", "props/prop_dict_chair.ydr"),
    ("x64a.rpf", "props/prop_stool.yft"),
    ("x64a.rpf", "props/prop_unplaced.ydr"),
    ("x64a.rpf", "props/prop_inside.ydr"),
    ("x64a.rpf", "props/prop_inverted.ydr"),
    ("x64a.rpf", "props/prop_assetless.ydr"),
    ("x64a.rpf", "props/props_txd.ytd"),
    ("x64a.rpf", "props/bench_txd.ytd"),
    ("x64b.rpf", "later/prop_bench.ydr"),
    ("alpha", "props/prop_alpha.ydr"),
];

/// The bounds of the fixture, which name a model and hold none.
const BOUNDS: &[(&str, &str)] = &[
    ("x64a.rpf", "props/prop_col.ybn"),
    ("x64a.rpf", "levels/v_int_shop.ybn"),
];

/// The logical path the mount lists `path` of `archive` at.
fn logical(archive: &str, path: &str) -> String {
    match archive {
        "x64a.rpf" | "x64b.rpf" => format!("platform:/{path}"),
        pack => format!("dlc_{pack}:/{path}"),
    }
}

fn stem(path: &str) -> &str {
    let leaf = path.rsplit('/').next().expect("a leaf");
    leaf.rsplit_once('.').expect("an extension").0
}

/// The install of the fixture, and the folder's guard.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-models-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", dlc_list_bytes(&["alpha", "beta"])),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
        ],
    );
    let files_of = |archive: &str| -> Vec<(&'static str, Vec<u8>)> {
        let mut files: Vec<(&'static str, Vec<u8>)> = Vec::new();
        for file in type_files().iter().filter(|file| file.archive == archive) {
            let bytes = typed_bytes(stem(file.path), &file.archetypes);
            files.push((file.path, bytes));
        }
        for (_, path, placed, interiors) in maps().into_iter().filter(|map| map.0 == archive) {
            let spec = MapSpec {
                interiors,
                ..MapSpec::around(cube([0.0; 3], 100.0))
            };
            let entities: Vec<Placed> = (placed.iter())
                .flat_map(|(archetype, count)| {
                    (0..*count).map(|n| Placed::new(archetype, [n as f32, 0.0, 0.0], 0.0, n))
                })
                .collect();
            files.push((path, placing_map_bytes(stem(path), &spec, &entities)));
        }
        let models = FILES.iter().filter(|file| file.0 == archive);
        // A model file is named and never opened, so what it holds is its own path.
        files.extend(models.map(|(_, path)| (*path, path.as_bytes().to_vec())));
        let bounds = BOUNDS.iter().filter(|file| file.0 == archive);
        files.extend(bounds.map(|(_, path)| (*path, bound_bytes(cube([-1.0; 3], 2.0)))));
        files
    };
    write_archive(&root, "x64a.rpf", &files_of("x64a.rpf"));
    write_archive(&root, "x64b.rpf", &files_of("x64b.rpf"));
    write_pack(&root, "alpha", 10, Vec::new(), files_of("alpha"));
    write_pack(&root, "beta", 20, Vec::new(), files_of("beta"));
    (scratch, root)
}

/// What the fixture says of one model: its last declaration, the declarations before it, and how often it is placed.
#[derive(Debug)]
struct Expected {
    spec: ArchetypeSpec,
    type_file: &'static TypeFileOf,
    replaced: Vec<&'static TypeFileOf>,
    placed: u32,
    inside: u32,
    /// Whether a file or a bound of the fixture has the model's name as its stem.
    named: bool,
    /// The kind and the logical path of the file that holds its model.
    held: Option<(Kind, String)>,
    size: Option<f32>,
}

/// A type file of the fixture by its stem, its logical path and its pack.
#[derive(Debug, PartialEq, Eq)]
struct TypeFileOf {
    name: &'static str,
    logical: String,
    pack: &'static str,
}

/// The copy the mount loads of each file of `files`, by leaf: the last in mount order.
fn winning<'a>(
    files: impl Iterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<&'a str, (&'a str, &'a str)> {
    let leaf = |path: &'a str| path.rsplit('/').next().expect("a leaf");
    files
        .map(|(archive, path)| (leaf(path), (archive, path)))
        .collect()
}

fn expected() -> BTreeMap<&'static str, Expected> {
    let declared: &'static Vec<TypeFileOf> = Box::leak(Box::new(
        (type_files().iter())
            .map(|file| TypeFileOf {
                name: stem(file.path),
                logical: logical(file.archive, file.path),
                pack: file.pack,
            })
            .collect(),
    ));
    let mut placed: BTreeMap<&str, u32> = BTreeMap::new();
    let all_maps = maps();
    let loaded = winning(all_maps.iter().map(|map| (map.0, map.1)));
    for (archive, path, places, interiors) in &all_maps {
        if loaded[path.rsplit('/').next().expect("a leaf")] != (*archive, *path) {
            continue;
        }
        for (archetype, count) in places {
            *placed.entry(archetype).or_default() += count;
        }
        for interior in interiors {
            *placed.entry(interior).or_default() += 1;
        }
    }
    let files = winning(FILES.iter().copied());
    let file_named = |name: &str, extension: &str| {
        let (archive, path) = files.get(format!("{name}.{extension}").as_str())?;
        Some(logical(archive, path))
    };
    let named: BTreeSet<&str> = (FILES.iter().chain(BOUNDS))
        .map(|(_, path)| stem(path))
        .chain(type_files().iter().map(|file| stem(file.path)))
        .chain(all_maps.iter().map(|map| stem(map.1)))
        .collect();

    let mut all: BTreeMap<&'static str, Expected> = BTreeMap::new();
    let mut inside: BTreeMap<&str, u32> = BTreeMap::new();
    for (at, types) in type_files().into_iter().enumerate() {
        for spec in types.archetypes {
            for held in spec.entities.iter().chain(&spec.set) {
                *inside.entry(held).or_default() += 1;
            }
            let earlier = all.remove(spec.name);
            let replaced = earlier.map_or_else(Vec::new, |earlier| {
                let mut replaced = earlier.replaced;
                replaced.push(earlier.type_file);
                replaced
            });
            // The dictionary the archetype states and the mount holds, else a fragment, else a drawable.
            let dictionary = Some(spec.drawable_dictionary).filter(|name| !name.is_empty());
            let held = (!spec.asset_name.is_empty())
                .then(|| {
                    let dictionary = dictionary.and_then(|name| file_named(name, "ydd"));
                    let loose = |extension| file_named(spec.asset_name, extension);
                    dictionary
                        .map(|logical| (Kind::DrawableDict, logical))
                        .or_else(|| loose("yft").map(|logical| (Kind::Fragment, logical)))
                        .or_else(|| loose("ydr").map(|logical| (Kind::Drawable, logical)))
                })
                .flatten();
            let (min, max) = (Vec3::from(spec.span.0), Vec3::from(spec.span.1));
            let size = min.cmple(max).all().then(|| (max - min).max_element());
            all.insert(
                spec.name,
                Expected {
                    named: named.contains(spec.name),
                    spec,
                    type_file: &declared[at],
                    replaced,
                    placed: 0,
                    inside: 0,
                    held,
                    size,
                },
            );
        }
    }
    for (name, model) in &mut all {
        model.placed = placed.get(name).copied().unwrap_or(0);
        model.inside = inside.get(name).copied().unwrap_or(0);
    }
    all
}

/// The mounted fixture with its index and its catalogue.
struct Built {
    _scratch: tempfile::TempDir,
    root: PathBuf,
    game: or_title::MountedGame,
}

impl Built {
    fn new() -> Self {
        let (scratch, root) = install();
        let game = mount_all(&root);
        Self {
            _scratch: scratch,
            root,
            game,
        }
    }

    fn source(&self) -> MountedWorld<'_> {
        MountedWorld::new(&self.game, &self.root)
    }

    fn index(&self) -> WorldIndex {
        WorldIndex::build(&self.source(), None).0
    }
}

/// The names of the models of `page`, in its order; a model no file names by its hash.
fn listed(page: &or_depends::models::Page<'_>) -> Vec<String> {
    page.rows.iter().map(|row| spelled(row.model)).collect()
}

fn spelled(model: &Model) -> String {
    (model.name.clone()).unwrap_or_else(|| format!("{:#010x}", model.hash))
}

/// What a model of the fixture is listed as: its name, or its hash where the fixture holds no file of that stem.
fn spelled_as(name: &str, model: &Expected) -> String {
    if model.named {
        name.to_string()
    } else {
        format!("{:#010x}", hash(name))
    }
}

/// The models of the fixture `keep` keeps, as a page lists them, in no order.
fn those(keep: impl Fn(&str, &Expected) -> bool) -> BTreeSet<String> {
    (expected().iter())
        .filter(|(name, model)| keep(name, model))
        .map(|(name, model)| spelled_as(name, model))
        .collect()
}

fn set(page: &or_depends::models::Page<'_>) -> BTreeSet<String> {
    let names = listed(page);
    let set: BTreeSet<String> = names.iter().cloned().collect();
    assert_eq!(set.len(), names.len(), "a model is listed twice: {names:?}");
    assert_eq!(page.total, names.len(), "the total of a whole answer");
    set
}

#[test]
fn every_archetype_is_listed_once_with_what_the_last_type_file_to_declare_it_states() {
    let built = Built::new();
    let catalogue = ModelCatalogue::new(&built.index());
    let expected = expected();
    assert_eq!(expected.len(), 13, "the fixture's models");
    assert_eq!(catalogue.len(), expected.len());
    assert_eq!(catalogue.models().len(), expected.len());

    for (name, want) in &expected {
        let model = catalogue.model(hash(name));
        let model = model.unwrap_or_else(|| panic!("{name} is not listed"));
        assert_eq!(model.hash, hash(name));
        assert_eq!(model.name.as_deref(), want.named.then_some(*name), "{name}");
        assert_eq!(model.kind, want.spec.kind, "{name}");
        let span = (Vec3::from(want.spec.span.0), Vec3::from(want.spec.span.1));
        assert_eq!(model.span, Some(span), "{name}");
        let sphere = (Vec3::from(want.spec.sphere.0), want.spec.sphere.1);
        assert_eq!(model.sphere, Some(sphere), "{name}");
        assert_eq!(model.size, want.size, "{name}");
        assert_eq!(model.lod_dist, want.spec.lod_dist, "{name}");
        assert_eq!(model.asset_name, hash(want.spec.asset_name), "{name}");
        assert_eq!(
            (model.drawable_dictionary, model.texture_dictionary),
            (
                hash(want.spec.drawable_dictionary),
                hash(want.spec.texture_dictionary)
            ),
            "{name}"
        );
        let declared = catalogue.declared(model);
        assert_eq!(
            (
                declared.type_file.name.as_str(),
                declared.type_file.logical.as_str(),
                declared.pack
            ),
            (
                want.type_file.name,
                want.type_file.logical.as_str(),
                want.type_file.pack
            ),
            "{name}"
        );
    }
    // The dictionaries a file of the mount is named as are named, and the others are their hash alone.
    assert_eq!(
        catalogue.dictionary_name(hash("furniture")),
        Some("furniture")
    );
    assert_eq!(
        catalogue.dictionary_name(hash("props_txd")),
        Some("props_txd")
    );
    assert_eq!(catalogue.dictionary_name(hash("txd_nowhere")), None);
    assert_eq!(catalogue.dictionary_name(hash("bench_txd")), None);
    assert_eq!(catalogue.dictionary_name(hash("dict_nowhere")), None);

    // One model of the fixture has no file of its name, and is listed by its hash after the others.
    let unnamed = those(|_, model| !model.named);
    assert_eq!(unnamed.len(), 1, "{unnamed:?}");
    assert_eq!(catalogue.unnamed(), unnamed.len());
    let last = catalogue.models().last().expect("a model");
    assert_eq!(
        (last.name.as_deref(), last.hash),
        (None, hash("prop_ghost"))
    );

    // The counts a browser offers its filters from are the fixture's.
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut packs: BTreeMap<&str, usize> = BTreeMap::new();
    for want in expected.values() {
        *kinds.entry(format!("{:?}", want.spec.kind)).or_default() += 1;
        *packs.entry(want.type_file.pack).or_default() += 1;
    }
    for (kind, count) in catalogue.kinds() {
        let want = kinds.get(&format!("{kind:?}")).copied().unwrap_or(0);
        assert_eq!(count, want, "{kind:?}");
        assert!(count > 0, "the fixture has a model of each kind");
    }
    let said: BTreeMap<&str, usize> = (catalogue.packs().into_iter())
        .filter(|(_, count)| *count > 0)
        .collect();
    assert_eq!(said, packs);
    assert_eq!(
        catalogue
            .packs()
            .iter()
            .map(|(pack, _)| *pack)
            .collect::<Vec<_>>(),
        ["common", "platform", "dlc_alpha", "dlc_beta"],
        "every pack of the mount, in mount order"
    );
}

#[test]
fn an_archetype_several_packs_declare_is_listed_once_under_the_last_with_those_it_replaced() {
    let built = Built::new();
    let index = built.index();
    let catalogue = ModelCatalogue::new(&index);
    let expected = expected();

    let declarations: usize = (type_files().iter())
        .map(|file| file.archetypes.len())
        .sum();
    assert_eq!(
        declarations - expected.len(),
        index.redefined(),
        "each declaration past a model's first replaced one"
    );
    assert_eq!(index.redefined(), 3);
    let mut replaced = 0;
    for (name, want) in &expected {
        let model = catalogue.model(hash(name)).expect("listed");
        let said: Vec<(String, String, &str)> = (catalogue.replaced(model).iter())
            .map(|declared| {
                let file = declared.type_file;
                (file.name.clone(), file.logical.clone(), declared.pack)
            })
            .collect();
        let wanted: Vec<(String, String, &str)> = (want.replaced.iter())
            .map(|file| (file.name.to_string(), file.logical.clone(), file.pack))
            .collect();
        assert_eq!(said, wanted, "{name}");
        replaced += said.len();
    }
    assert_eq!(replaced, index.redefined());

    // The bench three type files declare is one row, under the last pack, with that pack's box.
    let bench = catalogue.model(hash("prop_bench")).expect("the bench");
    let rows = catalogue.models().iter().filter(|m| m.hash == bench.hash);
    assert_eq!(rows.count(), 1);
    assert_eq!(catalogue.declared(bench).pack, "dlc_beta");
    assert_eq!(bench.size, Some(5.0));
    let packs: Vec<&str> = (catalogue.replaced(bench).iter())
        .map(|declared| declared.pack)
        .collect();
    assert_eq!(packs, ["platform", "dlc_alpha"]);
    // And the lamp the base game declares twice is the base archetype of the later file.
    let lamp = catalogue.model(hash("prop_lamp")).expect("the lamp");
    assert_eq!(lamp.kind, ArchetypeKind::Base);
    assert_eq!(catalogue.declared(lamp).type_file.name, "props_b");
}

#[test]
fn a_model_is_held_by_the_dictionary_it_states_else_a_fragment_else_a_drawable_and_the_file_opens()
{
    let built = Built::new();
    let source = built.source();
    let catalogue = ModelCatalogue::new(&built.index());
    let (mut held, mut kinds) = (0, BTreeSet::new());
    for (name, want) in &expected() {
        let model = catalogue.model(hash(name)).expect("listed");
        let said = catalogue.held(model);
        let said_as = said.map(|file| (file.kind, file.logical.to_string()));
        assert_eq!(said_as, want.held, "{name}");
        let Some(file) = said else { continue };
        // The file is the one the mount opens at that path, read from the archive the catalogue names.
        let bytes = source.open(file.archive, file.logical);
        assert!(bytes.is_some(), "{} does not open", file.logical);
        assert_eq!(bytes, built.game.vfs.open(file.logical), "{name}");
        held += 1;
        kinds.insert(format!("{:?}", file.kind));
    }
    assert_eq!(
        kinds.len(),
        3,
        "a dictionary, a fragment and a drawable: {kinds:?}"
    );
    assert_eq!(held, those(|_, model| model.held.is_some()).len());
    assert!(
        held < catalogue.len(),
        "some model of the fixture has no file"
    );

    let held_by = |name: &str| {
        let model = catalogue.model(hash(name)).expect("listed");
        catalogue.held(model).map(|file| file.logical.to_string())
    };
    // The later archive's copy of a file is the one named.
    assert_eq!(
        held_by("prop_bench").as_deref(),
        Some("platform:/later/prop_bench.ydr")
    );
    // A dictionary the mount holds outranks a loose file of the model's stem, and one it lacks does not.
    assert_eq!(
        held_by("prop_dict_chair").as_deref(),
        Some("platform:/props/furniture.ydd")
    );
    assert_eq!(
        held_by("prop_stool").as_deref(),
        Some("platform:/props/prop_stool.yft")
    );
    // An archetype that names no asset is held by no file, though one has its name and the mount holds its dictionary.
    assert_eq!(held_by("prop_assetless"), None);
    assert!(catalogue
        .model(hash("prop_assetless"))
        .is_some_and(|m| m.name.is_some()));
}

#[test]
fn a_model_is_placed_as_often_as_the_maps_the_mount_loads_place_it() {
    let built = Built::new();
    let source = built.source();
    let index = built.index();
    let catalogue = ModelCatalogue::new(&index);
    // The same count by the walk that reads every map again, which keeps nothing in the index.
    let mut walked = PlacementIndex::new();
    assert_eq!(place_maps(&index, &source, &mut walked), 0);

    let (mut placed, mut inside) = (0u64, 0u64);
    for (name, want) in &expected() {
        let model = catalogue.model(hash(name)).expect("listed");
        assert_eq!(model.placed, want.placed, "{name} by the maps");
        assert_eq!(model.inside, want.inside, "{name} inside an interior");
        assert_eq!(model.placed as usize, walked.of(hash(name)).len(), "{name}");
        assert_eq!(model.is_placed(), want.placed + want.inside > 0, "{name}");
        assert_eq!(
            model.times_placed(),
            u64::from(want.placed) + u64::from(want.inside)
        );
        placed += u64::from(model.placed);
        inside += u64::from(model.inside);
    }
    assert_eq!(
        placed,
        walked.seen(),
        "every placement the walk saw is a model's"
    );
    assert_eq!(inside, 3, "two entities of the interior and one of its set");

    let count = |name: &str| catalogue.model(hash(name)).map(|m| (m.placed, m.inside));
    // The copy of the town a later archive replaces places seven benches and a ghost, and none of them counts.
    assert_eq!(count("prop_bench"), Some((6, 1)));
    assert_eq!(count("prop_ghost"), Some((0, 0)));
    // An interior is placed by the map that stands it, and what it holds is placed inside it alone.
    assert_eq!(count("v_int_shop"), Some((1, 0)));
    assert_eq!(count("prop_inside"), Some((0, 2)));
}

#[test]
fn each_filter_lists_the_models_it_names_and_no_other() {
    let built = Built::new();
    let catalogue = ModelCatalogue::new(&built.index());
    let page = |query: Query<'_>| catalogue.page(&query).expect("a question");
    let all = set(&page(Query::default()));
    assert_eq!(all, those(|_, _| true));

    for kind in [ArchetypeKind::Base, ArchetypeKind::Time, ArchetypeKind::Mlo] {
        let said = set(&page(Query {
            kind: Some(kind),
            ..Query::default()
        }));
        assert_eq!(said, those(|_, model| model.spec.kind == kind), "{kind:?}");
    }
    assert_eq!(
        set(&page(Query {
            kind: Some(ArchetypeKind::Mlo),
            ..Query::default()
        })),
        BTreeSet::from(["v_int_shop".to_string()])
    );
    // The lamp a later file declares as a base archetype is no time archetype, which leaves the clock.
    assert_eq!(
        set(&page(Query {
            kind: Some(ArchetypeKind::Time),
            ..Query::default()
        })),
        BTreeSet::from(["prop_clock".to_string()])
    );

    for pack in ["common", "platform", "dlc_alpha", "dlc_beta"] {
        let said = set(&page(Query {
            pack: Some(pack),
            ..Query::default()
        }));
        assert_eq!(
            said,
            those(|_, model| model.type_file.pack == pack),
            "{pack}"
        );
        let shouted = pack.to_ascii_uppercase();
        let other_case = set(&page(Query {
            pack: Some(&shouted),
            ..Query::default()
        }));
        assert_eq!(other_case, said, "{pack} in another case");
    }
    let alpha = page(Query {
        pack: Some("dlc_alpha"),
        ..Query::default()
    });
    assert_eq!(
        listed(&alpha),
        ["prop_alpha"],
        "the bench alpha declares is beta's"
    );
    let nowhere = page(Query {
        pack: Some("dlc_gamma"),
        ..Query::default()
    });
    assert_eq!((nowhere.total, nowhere.rows.len()), (0, 0));

    // A size is the longest side of the box, and each end of a range is in it.
    let ranges = [
        (Some(2.0), Some(5.0)),
        (Some(5.5), None),
        (None, Some(0.75)),
        (Some(6.0), Some(6.0)),
        (Some(100.0), None),
    ];
    for (least, most) in ranges {
        let said = set(&page(Query {
            size: (least, most),
            ..Query::default()
        }));
        let within = |size: f32| {
            least.is_none_or(|least| size >= least) && most.is_none_or(|most| size <= most)
        };
        let wanted = those(|_, model| model.size.is_some_and(within));
        assert_eq!(said, wanted, "{least:?} to {most:?}");
        assert!(
            !said.contains("prop_inverted"),
            "a model with no size is in no range"
        );
    }
    let six = page(Query {
        size: (Some(6.0), Some(6.0)),
        ..Query::default()
    });
    assert_eq!(listed(&six), ["prop_col"]);

    for placed in [true, false] {
        let said = set(&page(Query {
            placed: Some(placed),
            ..Query::default()
        }));
        let wanted = those(|_, model| (model.placed + model.inside > 0) == placed);
        assert_eq!(said, wanted, "placed: {placed}");
        assert!(!said.is_empty());
    }
    let never = set(&page(Query {
        placed: Some(false),
        ..Query::default()
    }));
    assert!(
        never.contains("prop_unplaced") && !never.contains("prop_inside"),
        "{never:?}"
    );

    // Filters together list what each of them lists.
    let both = set(&page(Query {
        pack: Some("platform"),
        placed: Some(true),
        size: (Some(1.0), None),
        ..Query::default()
    }));
    let wanted = those(|_, model| {
        model.type_file.pack == "platform"
            && model.placed + model.inside > 0
            && model.size.is_some_and(|size| size >= 1.0)
    });
    assert_eq!(both, wanted);
    assert!(!both.is_empty() && both.len() < all.len());
}

#[test]
fn a_text_lists_the_models_a_search_of_the_install_lists_for_it_in_its_order() {
    let built = Built::new();
    let source = built.source();
    let index = built.index();
    let catalogue = ModelCatalogue::new(&index);
    // The search of the install, over the names of every file its archives list.
    let (files, names) = names_of(&source);
    let search = SearchNames::new(&index, &names);
    let placements = PlacementIndex::new();
    let found = |text: &str| {
        let install = Install {
            world: &index,
            catalogue: &files,
            names: &names,
            search: &search,
            placements: &placements,
        };
        let asked = Asked {
            near: None,
            limit: usize::MAX - 1,
        };
        let found = find(text, &install, &asked, None).expect("a question");
        let rows = found.models.into_iter();
        rows.map(|model| (model.hash, model.rank))
            .collect::<Vec<_>>()
    };
    let ghost = format!("{:#010x}", hash("prop_ghost"));
    let texts = [
        "bench",
        "prop",
        "prop_bench",
        "PROP_Bench",
        "in",
        "v_int",
        "prop_ghost",
        &ghost,
        "zzz",
    ];
    let mut matched = 0;
    for text in texts {
        let page = catalogue
            .page(&Query {
                text: Some(text),
                ..Query::default()
            })
            .expect("a question");
        let said: Vec<(u32, Rank)> = (page.rows.iter())
            .map(|row| (row.model.hash, row.rank.expect("a text ranks its rows")))
            .collect();
        assert_eq!(said, found(text), "{text}");
        assert_eq!(page.total, said.len());
        matched += said.len();
    }
    assert!(matched > 12, "{matched} row(s) compared");

    let ranks = |text: &str| {
        let query = Query {
            text: Some(text),
            ..Query::default()
        };
        let page = catalogue.page(&query).expect("a question");
        let rows = page.rows.iter();
        rows.map(|row| (spelled(row.model), row.rank.expect("ranked")))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ranks("bench"),
        [("prop_bench".to_string(), Rank::PartPrefix)]
    );
    // A name no file states finds its model typed in full, and by its hash, and by no part of it.
    assert_eq!(ranks("prop_ghost"), [(ghost.clone(), Rank::Exact)]);
    assert_eq!(ranks(&ghost), [(ghost.clone(), Rank::Exact)]);
    assert!(ranks("ghost").is_empty());

    // A text with a filter lists what both list, and a text that has the shape of a hash and is none is refused.
    let query = Query {
        text: Some("prop"),
        placed: Some(false),
        ..Query::default()
    };
    let unplaced = set(&catalogue.page(&query).expect("a question"));
    let wanted = those(|name, model| {
        model.named && name.starts_with("prop") && model.placed + model.inside == 0
    });
    assert_eq!(unplaced, wanted);
    assert!(!unplaced.is_empty());
    let refused = catalogue.page(&Query {
        text: Some("0x1234567890"),
        ..Query::default()
    });
    assert!(refused.is_err(), "{refused:?}");

    // The blanks about a text are no part of it, and a text of blanks alone is no text.
    assert_eq!(ranks("  prop_bench\t"), ranks("prop_bench"));
    assert!(!ranks("prop_bench").is_empty());
    let blank = catalogue.page(&Query {
        text: Some("  "),
        ..Query::default()
    });
    let whole = catalogue.page(&Query::default());
    assert_eq!(blank, whole);
    assert_eq!(whole.map(|page| page.total), Ok(expected().len()));
}

#[test]
fn each_sort_orders_the_models_both_ways_and_a_model_with_no_name_or_no_size_stays_last() {
    let built = Built::new();
    let catalogue = ModelCatalogue::new(&built.index());
    let expected = expected();
    let order = |sort: Option<Sort>, descending: bool, text: Option<&'static str>| {
        let query = Query {
            sort,
            descending,
            text,
            ..Query::default()
        };
        listed(&catalogue.page(&query).expect("a question"))
    };
    let named = |named: bool| -> Vec<String> {
        let mut names: Vec<(u32, String)> = (expected.iter())
            .filter(|(_, model)| model.named == named)
            .map(|(name, model)| (hash(name), spelled_as(name, model)))
            .collect();
        // Named models by name, which is the order of the map they come from, and the others by their hash.
        if !named {
            names.sort();
        }
        names.into_iter().map(|(_, name)| name).collect()
    };
    let by_name: Vec<String> = named(true).into_iter().chain(named(false)).collect();
    assert_eq!(order(None, false, None), by_name);
    assert_eq!(order(Some(Sort::Name), false, None), by_name);
    let backwards: Vec<String> = (named(true).into_iter().rev())
        .chain(named(false).into_iter().rev())
        .collect();
    assert_eq!(order(Some(Sort::Name), true, None), backwards);
    assert_ne!(by_name, backwards);

    // A key of each row of an order, which must not fall (or rise), and by name among rows that tie.
    let checked = |sort: Sort, descending: bool, text: Option<&'static str>| {
        let rows = order(Some(sort), descending, text);
        let key = |spelled: &String| -> (bool, f64) {
            let found = expected.iter();
            let (_, model) = found
                .map(|(name, model)| (spelled_as(name, model), model))
                .find(|(name, _)| name == spelled)
                .expect("a model of the fixture");
            match sort {
                Sort::Size => (model.size.is_none(), f64::from(model.size.unwrap_or(0.0))),
                _ => (false, f64::from(model.placed + model.inside)),
            }
        };
        for pair in rows.windows(2) {
            let (first, second) = (key(&pair[0]), key(&pair[1]));
            let in_order = match (first.0, second.0) {
                (false, false) if descending => first.1 >= second.1,
                (false, false) => first.1 <= second.1,
                // A model with no size comes after every model that has one, whichever way.
                (lacks, other) => !lacks || other,
            };
            assert!(in_order, "{sort:?} descending {descending}: {rows:?}");
            if first == second {
                let place = |name: &String| by_name.iter().position(|n| n == name);
                assert!(
                    place(&pair[0]) < place(&pair[1]),
                    "a tie is by name: {rows:?}"
                );
            }
        }
        rows
    };
    for sort in [Sort::Size, Sort::Placed] {
        let rising = checked(sort, false, None);
        let falling = checked(sort, true, None);
        assert_eq!(rising.len(), expected.len());
        assert_eq!(falling.len(), expected.len());
        assert_ne!(rising, falling, "{sort:?}");
        // With a text the order asked for is kept, over the models the text lists.
        let some = checked(sort, true, Some("prop"));
        assert!(some.len() > 3 && some.len() < expected.len(), "{some:?}");
    }
    let by_size = order(Some(Sort::Size), false, None);
    assert_eq!(by_size.first().map(String::as_str), Some("prop_inside"));
    assert_eq!(by_size.last().map(String::as_str), Some("prop_inverted"));
    let largest = order(Some(Sort::Size), true, None);
    assert_eq!(largest.first().map(String::as_str), Some("v_int_shop"));
    assert_eq!(largest.last().map(String::as_str), Some("prop_inverted"));
    let most_placed = order(Some(Sort::Placed), true, None);
    assert_eq!(most_placed.first().map(String::as_str), Some("prop_bench"));

    // Best first is the order of a search, and last first its reverse.
    let best = order(None, false, Some("prop"));
    let mut worst = order(None, true, Some("prop"));
    worst.reverse();
    assert_eq!(best, worst);
    assert!(best.len() > 3);
}

#[test]
fn the_pages_of_an_answer_are_its_rows_in_order_and_each_states_the_whole_total() {
    let built = Built::new();
    let catalogue = ModelCatalogue::new(&built.index());
    let questions = [
        Query::default(),
        Query {
            sort: Some(Sort::Size),
            descending: true,
            ..Query::default()
        },
        Query {
            text: Some("prop"),
            ..Query::default()
        },
        Query {
            placed: Some(true),
            sort: Some(Sort::Placed),
            ..Query::default()
        },
    ];
    for question in questions {
        let whole = catalogue.page(&question).expect("a question");
        assert_eq!(whole.total, whole.rows.len());
        assert!(whole.total > 4, "{question:?}");
        for count in [1, 3, 5, whole.total, whole.total + 7] {
            let mut rows = Vec::new();
            for offset in (0..whole.total + count).step_by(count) {
                let page = catalogue
                    .page(&Query {
                        offset,
                        count,
                        ..question
                    })
                    .expect("a question");
                assert_eq!(
                    page.total, whole.total,
                    "the total is the answer's, not the page's"
                );
                let most = whole.total.saturating_sub(offset).min(count);
                assert_eq!(page.rows.len(), most, "at {offset}, {count} a page");
                rows.extend(page.rows);
            }
            assert_eq!(rows, whole.rows, "{count} a page");
        }
        let none = catalogue
            .page(&Query {
                count: 0,
                ..question
            })
            .expect("a question");
        assert_eq!((none.total, none.rows.len()), (whole.total, 0));
    }
}

fn kept_file(scratch: &Path) -> PathBuf {
    scratch.join("kept").join("world.index")
}

/// What a catalogue says of every model, in its order.
fn said(catalogue: &ModelCatalogue) -> Vec<String> {
    (catalogue.models().iter())
        .map(|model| {
            format!(
                "{model:?} {:?} {:?} {:?} {:?}",
                catalogue.declared(model),
                catalogue.replaced(model),
                catalogue.held(model),
                [model.drawable_dictionary, model.texture_dictionary]
                    .map(|hash| catalogue.dictionary_name(hash)),
            )
        })
        .collect()
}

#[test]
fn the_catalogue_of_a_kept_index_opens_no_file_and_lists_no_archive() {
    let built = Built::new();
    let file = kept_file(built._scratch.path());
    let source = built.source();
    let counting = Counting::new(&source);
    let (first, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Absent),
        "{:?}",
        report.kept
    );
    let (listed, opened) = counting.counts();
    assert!(listed.iter().all(|count| *count == 1), "{listed:?}");
    // Every map, type file, manifest and bound is opened, and no model or texture file is.
    let read = maps().len() + type_files().len() + 1;
    let opened: u32 = opened.iter().sum();
    assert!(
        opened as usize >= read + BOUNDS.len() && (opened as usize) <= read + 2 * BOUNDS.len(),
        "{opened} file(s) opened for {read} read whole and {} bound(s)",
        BOUNDS.len()
    );
    let before = said(&ModelCatalogue::new(&first));
    assert_eq!(before.len(), expected().len());

    // A second start, by a mount made anew: the kept file, and nothing else, gives the same catalogue and answers.
    let game = mount_all(&built.root);
    let source = MountedWorld::new(&game, &built.root);
    let counting = Counting::new(&source);
    let (again, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
    assert_eq!(report.opened, 0);
    let catalogue = ModelCatalogue::new(&again);
    let page = catalogue
        .page(&Query {
            text: Some("prop"),
            sort: Some(Sort::Placed),
            ..Query::default()
        })
        .expect("a question");
    assert!(page.total > 4);
    assert_eq!(said(&catalogue), before);
    let (listed, opened) = counting.counts();
    assert!(
        listed.iter().chain(&opened).all(|count| *count == 0),
        "listed {listed:?}, opened {opened:?}"
    );
    assert!(catalogue.models().iter().any(|model| model.placed > 0));
    assert!(catalogue.models().iter().any(|model| model.inside > 0));
    assert!((catalogue.models().iter()).any(|model| catalogue.held(model).is_some()));
}

#[test]
fn a_kept_file_of_the_version_before_is_rebuilt_never_trusted() {
    let built = Built::new();
    let file = kept_file(built._scratch.path());
    let source = built.source();
    let (first, _) = refresh(&source, &file);
    let before = said(&ModelCatalogue::new(&first));
    let sealed = std::fs::read(&file).expect("kept");

    // The file as the version before this one headed it: that version kept no count of placements and no model file,
    // so nothing of such a file may answer.
    assert_eq!(sealed[4..8], VERSION.to_le_bytes());
    assert_eq!(sealed[8..12], RULES.to_le_bytes());
    // ⚠ The number the files kept before the list's members are headed with, for the version and for the rules: a
    // build that kept the number would read such a file as its own.
    const BEFORE: u32 = 2;
    let headed = |at: usize| u32::from_le_bytes(sealed[at..at + 4].try_into().expect("four bytes"));
    assert!(
        headed(4) > BEFORE && headed(8) > BEFORE,
        "version {}, rules {}",
        headed(4),
        headed(8)
    );
    let mut older = sealed.clone();
    older[4..8].copy_from_slice(&(VERSION - 1).to_le_bytes());
    older[8..12].copy_from_slice(&(RULES - 1).to_le_bytes());
    let refused = Kept::unseal(&older).expect_err("the version before");
    assert!(
        matches!(refused, KeptError::Version { found } if found == VERSION - 1),
        "{refused}"
    );
    // The rules alone are refused too: what a row holds of a file changed with them.
    let mut older_rules = sealed.clone();
    older_rules[8..12].copy_from_slice(&(RULES - 1).to_le_bytes());
    let refused = Kept::unseal(&older_rules).expect_err("the rules before");
    assert!(
        matches!(refused, KeptError::Rules { found } if found == RULES - 1),
        "{refused}"
    );

    for old in [&older, &older_rules] {
        std::fs::write(&file, old).expect("written");
        let game = mount_all(&built.root);
        let source = MountedWorld::new(&game, &built.root);
        let counting = Counting::new(&source);
        let (index, report) = refresh(&counting, &file);
        assert!(
            matches!(report.kept, KeptState::Refused(_)),
            "{:?}",
            report.kept
        );
        assert_eq!(
            report.read.len(),
            report.archives,
            "every archive is read again"
        );
        let (listed, _) = counting.counts();
        assert!(listed.iter().all(|count| *count == 1), "{listed:?}");
        assert_eq!(said(&ModelCatalogue::new(&index)), before);
        // And it is kept anew at this version, so the start after it reads nothing.
        assert_eq!(std::fs::read(&file).expect("kept anew"), sealed);
        let (_, report) = refresh(&source, &file);
        assert!(
            matches!(report.kept, KeptState::Current),
            "{:?}",
            report.kept
        );
    }
}

/// The kept file `sealed` with `change` made to what it seals, sealed again under the head it had.
fn resealed(sealed: &[u8], change: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut plain = Vec::new();
    let mut stream = flate2::read::ZlibDecoder::new(&sealed[20..]);
    std::io::Read::read_to_end(&mut stream, &mut plain).expect("inflates");
    change(&mut plain);
    let mut out = sealed[..12].to_vec();
    out.extend((plain.len() as u64).to_le_bytes());
    let mut stream = flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::fast());
    std::io::Write::write_all(&mut stream, &plain).expect("deflates");
    stream.finish().expect("deflates");
    out
}

#[test]
fn a_kept_file_that_states_an_archetype_of_no_kind_is_refused_and_read_again() {
    let built = Built::new();
    let file = kept_file(built._scratch.path());
    let source = built.source();
    let (first, _) = refresh(&source, &file);
    let before = said(&ModelCatalogue::new(&first));
    let sealed = std::fs::read(&file).expect("kept");
    assert_eq!(
        Kept::unseal(&resealed(&sealed, |_| ())).expect("sealed again as it was"),
        Kept::unseal(&sealed).expect("whole")
    );

    // An archetype's kind is the byte after its name: the clock is a time archetype, which is 1. No map or interior
    // names it, so its hash stands twice in what the file seals, as its name and then as its asset.
    let name = hash("prop_clock").to_le_bytes();
    let kind_of_the_clock = |plain: &[u8]| {
        let found: Vec<usize> = (plain.windows(4).enumerate())
            .filter(|(_, bytes)| *bytes == name)
            .map(|(at, _)| at + 4)
            .collect();
        assert_eq!(found.len(), 2, "the clock's name and its asset");
        found[0]
    };
    let no_kind = resealed(&sealed, |plain| {
        let at = kind_of_the_clock(plain);
        assert_eq!(plain[at], 1, "a time archetype");
        plain[at] = 3;
    });
    let refused = Kept::unseal(&no_kind).expect_err("a kind no archetype is of");
    assert!(
        matches!(refused, KeptError::Malformed(_)) && refused.to_string().contains("no kind"),
        "{refused}"
    );
    // Each kind there is reads back as itself.
    for (byte, kind) in [(0u8, ArchetypeKind::Base), (2, ArchetypeKind::Mlo)] {
        let other = resealed(&sealed, |plain| {
            let at = kind_of_the_clock(plain);
            plain[at] = byte;
        });
        std::fs::write(&file, &other).expect("written");
        let (index, report) = refresh(&source, &file);
        assert!(
            matches!(report.kept, KeptState::Current),
            "{:?}",
            report.kept
        );
        let clock = index.archetype(hash("prop_clock")).expect("the clock");
        assert_eq!(clock.kind, kind);
    }

    std::fs::write(&file, &no_kind).expect("written");
    let (index, report) = refresh(&source, &file);
    assert!(
        matches!(report.kept, KeptState::Refused(KeptError::Malformed(_))),
        "{:?}",
        report.kept
    );
    assert_eq!(report.read.len(), report.archives);
    assert_eq!(said(&ModelCatalogue::new(&index)), before);
    assert_eq!(std::fs::read(&file).expect("kept anew"), sealed);
}

/// ⚠ The title update mounts before the packs, and a folder of it patches a pack by the table that names the folder: what
/// lies there is the pack's and not the update's, and is opened from the update all the same.
#[test]
fn what_a_folder_that_patches_a_pack_holds_is_listed_under_that_pack_and_opened_from_the_update() {
    use or_title::{ExtraTitleUpdateData, TitleUpdateMount};

    let scratch = tempfile::Builder::new()
        .prefix("or-depends-models-patched-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", dlc_list_bytes(&["beta", "gamma"])),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
        ],
    );
    let base = ArchetypeSpec::base;
    let table = ExtraTitleUpdateData {
        mounts: vec![TitleUpdateMount {
            device_name: "dlc_beta".into(),
            path: "update:/dlc_patch/beta/".into(),
        }],
    };
    let patch = [
        base("prop_beta", sized(2.0, 1.0, 1.0)),
        base("prop_patched", sized(3.0, 1.0, 1.0)),
    ];
    std::fs::create_dir(root.join("update")).expect("mkdir");
    write_archive(
        &root,
        "update/update.rpf",
        &[
            (
                "common/data/ExtraTitleUpdateData.meta",
                table.to_xml().into_bytes(),
            ),
            // A type file of the update that lies in no such folder.
            (
                "x64/types/update_props.ytyp",
                typed_bytes("update_props", &[base("prop_update", sized(1.0, 1.0, 1.0))]),
            ),
            (
                "dlc_patch/beta/types/beta_props.ytyp",
                typed_bytes("beta_props", &patch),
            ),
            (
                "dlc_patch/beta/models/prop_beta.ydr",
                b"the patch's".to_vec(),
            ),
        ],
    );
    write_pack(
        &root,
        "beta",
        10,
        Vec::new(),
        vec![
            (
                "types/beta_props.ytyp",
                typed_bytes("beta_props", &[base("prop_beta", sized(1.0, 1.0, 1.0))]),
            ),
            ("models/prop_beta.ydr", b"the pack's".to_vec()),
        ],
    );
    write_pack(
        &root,
        "gamma",
        20,
        Vec::new(),
        vec![(
            "types/gamma_props.ytyp",
            typed_bytes("gamma_props", &[base("prop_gamma", sized(4.0, 1.0, 1.0))]),
        )],
    );
    let game = mount_all(&root);
    let place = |container: &str| {
        (game.roots.iter()).position(|root| root.container.replace('\\', "/") == container)
    };
    let (update, pack) = (
        place("update/update.rpf"),
        place("x64/dlcpacks/beta/dlc.rpf"),
    );
    assert!(
        update.is_some() && update < pack,
        "the update is mounted before the pack it patches: {update:?} {pack:?}"
    );
    let source = MountedWorld::new(&game, &root);
    let catalogue = ModelCatalogue::new(&WorldIndex::build(&source, None).0);

    let declared = |name: &str| {
        let model = catalogue.model(hash(name));
        let model = model.unwrap_or_else(|| panic!("{name} is not listed"));
        let declared = catalogue.declared(model);
        (
            declared.pack.to_string(),
            declared.type_file.logical.clone(),
            model.size,
            catalogue.replaced(model).len(),
        )
    };
    // The folder's copy of the pack's type file is the one read, and what it declares is the pack's.
    let patched = "update:/dlc_patch/beta/types/beta_props.ytyp".to_string();
    assert_eq!(
        declared("prop_beta"),
        ("dlc_beta".to_string(), patched.clone(), Some(2.0), 0)
    );
    assert_eq!(
        declared("prop_patched"),
        ("dlc_beta".to_string(), patched, Some(3.0), 0)
    );
    // What the update holds in no such folder is the update's own.
    assert_eq!(declared("prop_update").0, "update");
    assert_eq!(declared("prop_gamma").0, "dlc_gamma");
    let of = |pack: &str| -> BTreeSet<u32> {
        let query = Query {
            pack: Some(pack),
            ..Query::default()
        };
        let page = catalogue.page(&query).expect("a question");
        assert_eq!(page.total, page.rows.len());
        page.rows.iter().map(|row| row.model.hash).collect()
    };
    assert_eq!(
        of("dlc_beta"),
        BTreeSet::from([hash("prop_beta"), hash("prop_patched")])
    );
    assert_eq!(of("update"), BTreeSet::from([hash("prop_update")]));
    let counts: BTreeMap<&str, usize> = catalogue.packs().into_iter().collect();
    assert_eq!(
        (counts["update"], counts["dlc_beta"], counts["dlc_gamma"]),
        (1, 2, 1)
    );

    // The model file of the folder is the one the mount opens at the pack's path, read from the update, which lists it.
    let beta = catalogue.model(hash("prop_beta")).expect("listed");
    let held = catalogue.held(beta).expect("a file holds it");
    assert_eq!(
        (held.kind, held.logical),
        (
            Kind::Drawable,
            "update:/dlc_patch/beta/models/prop_beta.ydr"
        )
    );
    assert_eq!(Some(held.archive), update);
    let bytes = source.open(held.archive, held.logical);
    assert_eq!(bytes.as_deref(), Some(&b"the patch's"[..]));
    assert_eq!(bytes, game.vfs.open("dlc_beta:/models/prop_beta.ydr"));
}

/// ⚠ More models than a sort of a short list takes one at a time, and few sizes among them: an order that left its ties
/// to the sort would list them as no two pages could rely on.
#[test]
fn models_that_tie_in_an_order_are_listed_by_name_however_many_there_are() {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-models-tied-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", dlc_list_bytes(&[])),
            ("data/_manifest.ymf", manifest_bytes(&[], &[])),
        ],
    );
    let names: Vec<&'static str> = (0..96)
        .map(|n| &*Box::leak(format!("prop_tied_{n:02}").into_boxed_str()))
        .collect();
    let sides = [3.0, 1.0, 2.0, 1.0, 3.0, 2.0, 2.0];
    let archetypes: Vec<ArchetypeSpec> = (names.iter().enumerate())
        .map(|(n, name)| ArchetypeSpec::base(name, sized(sides[n % sides.len()], 1.0, 1.0)))
        .collect();
    // Every other model has a file of its name, so the named and those listed by their hash both tie.
    let paths: Vec<String> = (names.iter().step_by(2))
        .map(|name| format!("models/{name}.ydr"))
        .collect();
    let mut files = vec![("types/tied.ytyp", typed_bytes("tied", &archetypes))];
    files.extend(
        paths
            .iter()
            .map(|path| (path.as_str(), b"a model".to_vec())),
    );
    write_archive(&root, "x64a.rpf", &files);
    let game = mount_all(&root);
    let source = MountedWorld::new(&game, &root);
    let catalogue = ModelCatalogue::new(&WorldIndex::build(&source, None).0);
    let models = catalogue.models();
    assert_eq!(models.len(), names.len());
    assert_eq!(catalogue.unnamed(), names.len() - paths.len());
    let sizes: BTreeSet<u32> = (models.iter())
        .map(|model| model.size.expect("a size").to_bits())
        .collect();
    assert_eq!(sizes.len(), 3, "the fixture's sizes, so most models tie");

    for sort in [Sort::Size, Sort::Placed] {
        for descending in [false, true] {
            let query = Query {
                sort: Some(sort),
                descending,
                ..Query::default()
            };
            let page = catalogue.page(&query).expect("a question");
            let said: Vec<u32> = page.rows.iter().map(|row| row.model.hash).collect();
            // The catalogue holds its models by name, so a sort of them by the key alone that keeps ties as they stood is
            // the order wanted.
            let key = |model: &Model| match sort {
                Sort::Size => f64::from(model.size.expect("a size")),
                _ => model.times_placed() as f64,
            };
            let mut wanted: Vec<&Model> = models.iter().collect();
            wanted.sort_by(|a, b| {
                let order = key(a).total_cmp(&key(b));
                if descending {
                    order.reverse()
                } else {
                    order
                }
            });
            let wanted: Vec<u32> = wanted.iter().map(|model| model.hash).collect();
            assert_eq!(said, wanted, "{sort:?}, descending: {descending}");
        }
    }
}

/// ⚠ A kept file lies in a folder anything may write to: counts past what an install could hold must not stop the list.
#[test]
fn counts_past_what_a_count_holds_in_a_kept_file_stop_at_the_most_a_count_holds() {
    let built = Built::new();
    let file = kept_file(built._scratch.path());
    let source = built.source();
    refresh(&source, &file);
    let sealed = std::fs::read(&file).expect("kept");

    // A count of the bench stands after its name: one for each map that places it, the copy a later archive replaces
    // among them, and one for the interior that holds it.
    let name = hash("prop_bench").to_le_bytes();
    let mut stated: Vec<u32> = (maps().iter())
        .flat_map(|map| map.2.iter().filter(|placed| placed.0 == "prop_bench"))
        .map(|placed| placed.1)
        .collect();
    let inside = expected()["prop_bench"].inside;
    assert!(inside > 0 && stated.len() > 2);
    stated.push(inside);
    let altered = resealed(&sealed, |plain| {
        let counts: Vec<usize> = (plain.windows(8).enumerate())
            .filter(|(_, bytes)| {
                let after = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
                bytes[..4] == name && stated.contains(&after)
            })
            .map(|(at, _)| at + 4)
            .collect();
        assert_eq!(counts.len(), stated.len(), "the counts of the bench");
        for at in counts {
            plain[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
    });
    std::fs::write(&file, &altered).expect("written");
    let (index, report) = refresh(&source, &file);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );

    // Two maps the mount loads and one pack's place the bench, each now as often as a count holds.
    let catalogue = ModelCatalogue::new(&index);
    let bench = catalogue.model(hash("prop_bench")).expect("the bench");
    assert_eq!((bench.placed, bench.inside), (u32::MAX, u32::MAX));
    assert_eq!(bench.times_placed(), 2 * u64::from(u32::MAX));
    let most = catalogue.page(&Query {
        sort: Some(Sort::Placed),
        descending: true,
        count: 1,
        ..Query::default()
    });
    assert_eq!(listed(&most.expect("a question")), ["prop_bench"]);
}
