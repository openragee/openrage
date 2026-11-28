//! What a frame of GTA V's world is handed beside its instances, over installs built by hand: a
//! model however its file holds it and for either target, the reason of one that does not read,
//! a texture through the dictionaries its own falls through to, and a map's far lights and
//! occluders.

#[path = "../../or-depends/tests/forest_fixture/mod.rs"]
mod forest_fixture;
mod hand_built;
#[path = "../../or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::any::Any;
use std::path::Path;
use std::sync::Arc;

use glam::Vec3;
use or_io::Generation;
use or_world::contract::{MapBody, Worker, WorldInstall, WorldSpec};
use or_world::kept;
use or_world::model::{
    Alpha, AssetKey, ElementFormat, Index, LightKind, Mesh, Model, Semantic, Texture, TextureAsk,
    TextureFormat,
};
use or_world_gta5::{
    Body, Gta5World, Holding, Materials, NoModel, NoTexture, Outcome, BOX, EMBEDDED, LEGACY, TITLE,
};

use forest_fixture::{arch, install, types_file, Arch, Files};
use hand_built::{
    badge, BADGE_SEED, CUTOUT_BUCKET, LEVELS, MID_TXD, MOSS, MOSS_SEED, OWN_TXD, ROOT_TXD, WALL,
    WALL_SEED,
};
use world_fixture::{hash, mount};

/// The world of the install at `root`, its index through a kept file under `scratch`.
fn opened(root: &Path, scratch: &Path) -> (Arc<Gta5World>, Arc<Index>) {
    let world = Arc::new(Gta5World::over(mount(root), root.to_path_buf()));
    let spec = WorldSpec::of(LEGACY, root);
    let file = kept::file_for(scratch, TITLE, &spec);
    let install: &dyn WorldInstall = &*world;
    let (index, _) = kept::refresh(TITLE, &spec, install, &file).expect("the index builds");
    (world, Arc::new(index))
}

fn key(name: &str) -> AssetKey {
    AssetKey(u64::from(hash(name)))
}

/// An archetype named `name` whose asset is of its own name, in the drawable dictionary `dictionary` where it names one, with the scene's texture dictionary.
fn held(name: &'static str, dictionary: &'static str) -> Arch {
    Arch {
        files: (name, dictionary, OWN_TXD),
        ..arch(name, 80.0)
    }
}

/// The numbers of the element `semantic` of each vertex of `mesh`, read at the offset its layout states.
fn numbers(mesh: &Mesh, semantic: Semantic) -> Vec<Vec<f32>> {
    let element = (mesh.layout.elements.iter())
        .find(|element| element.semantic == semantic)
        .unwrap_or_else(|| panic!("no {semantic:?} among {:?}", mesh.layout.elements));
    let count = match element.format {
        ElementFormat::F32x2 => 2,
        ElementFormat::F32x3 => 3,
        other => panic!("{semantic:?} is stored as {other:?}"),
    };
    let stride = mesh.layout.stride as usize;
    (mesh.vertices.chunks_exact(stride))
        .map(|vertex| {
            let at = &vertex[element.offset as usize..][..4 * count];
            (at.as_chunks::<4>().0.iter())
                .map(|four| f32::from_le_bytes(*four))
                .collect()
        })
        .collect()
}

/// The corners, the one normal and the coordinates `hand_built::drawable` gives its triangle.
const CORNERS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

/// The files every holding and both targets are read from, and each archetype with how it is held and for which target.
fn holdings() -> (Files, Vec<(&'static str, Holding, Generation, usize)>) {
    let ydr = |name: &str, levels| {
        (
            format!("{name}.ydr"),
            hand_built::drawable_bytes(name, levels),
        )
    };
    let yft = |name: &str, levels| {
        (
            format!("{name}.yft"),
            hand_built::fragment_bytes(name, levels),
        )
    };
    let ydd = |name: &str, held: &[(&str, usize)]| {
        (format!("{name}.ydd"), hand_built::dictionary_bytes(held))
    };
    let legacy = [
        ydr("loose", 2),
        ydd("pack", &[("entry_a", 1), ("entry_b", 3)]),
        yft("frag", 2),
    ];
    let enhanced = [
        ydr("loose9", 2),
        ydd("pack9", &[("entry9_a", 1), ("entry9_b", 3)]),
        yft("frag9", 2),
    ]
    .map(|(leaf, bytes)| {
        let made = hand_built::enhanced(&leaf, &bytes);
        (leaf, made)
    });
    let types = [
        held("loose", ""),
        held("entry_a", "pack"),
        held("entry_b", "pack"),
        held("frag", ""),
        held("loose9", ""),
        held("entry9_a", "pack9"),
        held("entry9_b", "pack9"),
        held("frag9", ""),
    ];
    let mut files: Files = vec![types_file("props", &types)];
    for (leaf, bytes) in legacy.into_iter().chain(enhanced) {
        files.push((format!("models/{leaf}"), bytes));
    }
    let (g8, g9) = (Generation::Gen8, Generation::Gen9);
    let stated = vec![
        ("loose", Holding::File, g8, 2),
        ("entry_a", Holding::Dictionary, g8, 1),
        ("entry_b", Holding::Dictionary, g8, 3),
        ("frag", Holding::Fragment, g8, 2),
        ("loose9", Holding::File, g9, 2),
        ("entry9_a", Holding::Dictionary, g9, 1),
        ("entry9_b", Holding::Dictionary, g9, 3),
        ("frag9", Holding::Fragment, g9, 2),
    ];
    (files, stated)
}

#[test]
fn a_drawable_of_each_holding_and_each_target_reads_to_the_vertices_and_the_layout_built() {
    let (files, stated) = holdings();
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let asks: Vec<AssetKey> = stated.iter().map(|(name, ..)| key(name)).collect();
    let mut read = Vec::new();
    let opened = world.read_models(&index, &asks, &mut read);
    // Two drawables, two fragments and two dictionaries, each dictionary once for its two entries.
    assert_eq!(opened.files, 6);
    assert_eq!(read.len(), stated.len());
    for (name, holding, target, levels) in stated {
        let outcome = (read.iter().find(|outcome| outcome.ask == key(name))).expect("an outcome");
        assert_eq!(
            (outcome.holding, outcome.target),
            (Some(holding), Some(target)),
            "{name}"
        );
        let model = match &outcome.model {
            Ok(model) => model,
            Err(why) => panic!("{name} does not read: {why}"),
        };
        assert_eq!(model.lods.len(), levels, "{name}");
        for (level, lod) in model.lods.iter().enumerate() {
            assert_eq!(lod.distance, hand_built::LOD_DISTANCES[level], "{name}");
            let [mesh] = &lod.meshes[..] else {
                panic!("{name}: {} meshes at level {level}", lod.meshes.len());
            };
            assert_eq!(mesh.indices, [0, 1, 2], "{name}");
            assert_eq!(
                mesh.vertices.len(),
                CORNERS.len() * mesh.layout.stride as usize
            );
            assert_eq!(numbers(mesh, Semantic::Position), CORNERS, "{name}");
            let uvs: Vec<Vec<f32>> = CORNERS.iter().map(|c| vec![c[0], c[1]]).collect();
            assert_eq!(numbers(mesh, Semantic::TexCoord(0)), uvs, "{name}");
            if target == Generation::Gen8 {
                // The file's own bytes, whole: what the drawable was built with.
                let built = hand_built::drawable(name, levels);
                let geometry = &built.base.lod_group.lods[level].models[0].geometries[0];
                assert_eq!(mesh.vertices, geometry.vertex_buffer.data, "{name}");
                assert_eq!(mesh.layout.stride, 32);
                assert_eq!(numbers(mesh, Semantic::Normal), [[0.0, 0.0, 1.0]; 3]);
            }
            // The colour is the texture bound by name as the diffuse, never the first binding,
            // asked for in the archetype's dictionary; the bump is asked for in the model's own.
            let (colour, normal) = (mesh.surface.colour, mesh.surface.normal);
            let colour = colour.unwrap_or_else(|| panic!("{name} asks for no colour"));
            assert_eq!(
                (colour.scope, colour.name),
                (key(OWN_TXD), u64::from(hash(WALL))),
                "{name}"
            );
            let normal = normal.unwrap_or_else(|| panic!("{name} asks for no bump"));
            assert_eq!(
                (normal.scope, normal.name),
                (
                    AssetKey(EMBEDDED | key(name).0),
                    u64::from(hash(&badge(name)))
                ),
                "{name}"
            );
            assert!(matches!(mesh.surface.alpha, Alpha::Cutout(_)), "{name}");
        }
        let title = model.title.as_ref().expect("the materials of the model");
        let materials = title.downcast_ref::<Materials>().expect("GTA V's record");
        let [material] = &materials.materials[..] else {
            panic!("{name}: {} materials", materials.materials.len());
        };
        assert_eq!(material.draw_bucket, CUTOUT_BUCKET);
        assert_eq!(material.shader, hash("default"));
        let bound: Vec<(&str, bool)> = (material.textures.iter())
            .map(|bound| (bound.texture.as_str(), bound.embedded))
            .collect();
        assert_eq!(
            bound,
            [(badge(name).as_str(), true), (WALL, false)],
            "{name}"
        );
        assert_eq!(materials.of, vec![vec![Some(0)]; levels], "{name}");
    }
    // What a model carries reads from its own file, of either target.
    let carried = |name: &str| TextureAsk {
        scope: AssetKey(EMBEDDED | key(name).0),
        name: u64::from(hash(&badge(name))),
        pixels: u32::MAX,
    };
    let asks = [
        "loose", "entry_a", "entry_b", "frag", "loose9", "entry9_b", "frag9",
    ]
    .map(carried);
    let mut textures = Vec::new();
    let opened = world.read_textures(&index, &asks, &mut textures);
    assert_eq!(opened.files, 6);
    let store = hand_built::texture("any", BADGE_SEED).backing_store;
    for (ask, texture) in &textures {
        let texture = texture
            .as_ref()
            .unwrap_or_else(|why| panic!("{ask:?}: {why}"));
        assert_eq!(
            (texture.width, texture.height, texture.format),
            (8, 8, TextureFormat::Bc1)
        );
        assert_eq!(texture.levels[0], store[..LEVELS[0]]);
    }
    assert_eq!(textures.len(), asks.len());
}

#[test]
fn a_texture_bound_by_name_that_the_models_own_dictionary_holds_is_asked_for_there() {
    use or_io::decl::File;
    use or_io::gta5::drawable::gen8::GrcTextureReference;
    use or_io::gta5::gen8::EntryData;
    use or_io::gta5::TextureSlot;
    // The scene's drawable carries its badge; here its shader binds the badge by name instead.
    let mut built = hand_built::drawable("by_name", 1);
    let group = built.base.shader_group.as_deref_mut().expect("a group");
    let named = GrcTextureReference::named(badge("by_name"));
    group.shaders[0].entries[0].data = EntryData::Texture(TextureSlot::Reference(Box::new(named)));
    let legacy = built.to_bytes().expect("a drawable");
    let files: Files = vec![
        types_file("props", &[held("by_name", ""), held("by_name9", "")]),
        ("models/by_name.ydr".to_owned(), legacy.clone()),
        (
            "models/by_name9.ydr".to_owned(),
            hand_built::enhanced("by_name9.ydr", &legacy),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let asks = [key("by_name"), key("by_name9")];
    let mut read = Vec::new();
    world.read_models(&index, &asks, &mut read);
    let mut wanted = Vec::new();
    for (outcome, target) in read.iter().zip([Generation::Gen8, Generation::Gen9]) {
        assert_eq!(outcome.target, Some(target));
        let model = outcome.model.as_ref().expect("the model reads");
        let materials = model.title.as_ref().expect("its materials");
        let materials = materials
            .downcast_ref::<Materials>()
            .expect("GTA V's record");
        let bound: Vec<(&str, bool)> = (materials.materials[0].textures.iter())
            .map(|bound| (bound.texture.as_str(), bound.embedded))
            .collect();
        // The badge is the model's own though bound by name; the wall, which it does not carry, is the archetype's dictionary's.
        assert_eq!(bound, [(badge("by_name").as_str(), true), (WALL, false)]);
        let surface = &model.lods[0].meshes[0].surface;
        let normal = surface.normal.expect("a bump");
        assert_eq!(normal.scope, AssetKey(EMBEDDED | outcome.ask.0));
        assert_eq!(surface.colour.expect("a colour").scope, key(OWN_TXD));
        wanted.push(normal);
    }
    let mut textures = Vec::new();
    world.read_textures(&index, &wanted, &mut textures);
    let store = hand_built::texture("any", BADGE_SEED).backing_store;
    assert_eq!(textures.len(), 2);
    for (ask, texture) in &textures {
        let texture = (texture.as_ref()).unwrap_or_else(|why| panic!("{ask:?}: {why}"));
        assert_eq!(texture.levels[0], store[..LEVELS[0]]);
    }
}

/// A file that opens as a resource and holds too little to be the drawable its header says.
fn cut_short() -> Vec<u8> {
    let whole = hand_built::drawable_bytes("cut", 1);
    whole[..whole.len() / 2].to_vec()
}

#[test]
fn a_model_that_cannot_be_read_states_its_reason_and_the_contracts_reader_hands_back_the_rest() {
    let no_asset = Arch {
        files: ("", "", ""),
        ..arch("no_asset", 80.0)
    };
    let files: Files = vec![
        types_file(
            "props",
            &[
                arch("whole", 80.0),
                arch("no_file", 80.0),
                arch("garbage", 80.0),
                arch("cut", 80.0),
                no_asset,
                held("absent_entry", "pack"),
                held("fallen_back", "no_such_pack"),
            ],
        ),
        (
            "models/whole.ydr".to_owned(),
            hand_built::drawable_bytes("whole", 1),
        ),
        ("models/garbage.ydr".to_owned(), vec![7; 64]),
        ("models/cut.ydr".to_owned(), cut_short()),
        (
            "models/pack.ydd".to_owned(),
            hand_built::dictionary_bytes(&[("another", 1)]),
        ),
        // A loose file of an archetype whose dictionary the mount lacks draws it.
        (
            "models/fallen_back.ydr".to_owned(),
            hand_built::drawable_bytes("fallen_back", 1),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let past = AssetKey(BOX.0 + 1);
    let names = [
        "whole",
        "no_file",
        "garbage",
        "cut",
        "no_asset",
        "absent_entry",
        "fallen_back",
        "nowhere_declared",
    ];
    let mut asks: Vec<AssetKey> = names.iter().map(|name| key(name)).collect();
    asks.extend([past, BOX]);
    let mut read: Vec<Outcome> = Vec::new();
    world.read_models(&index, &asks, &mut read);
    assert_eq!(read.len(), asks.len());
    let why = |ask: AssetKey| {
        let outcome = read.iter().find(|outcome| outcome.ask == ask);
        outcome.expect("an outcome").model.as_ref().err().cloned()
    };
    assert_eq!(why(key("whole")), None);
    assert_eq!(why(key("fallen_back")), None);
    assert_eq!(why(BOX), None);
    assert_eq!(why(key("no_file")), Some(NoModel::NoFile));
    assert_eq!(why(key("no_asset")), Some(NoModel::NoAsset));
    assert_eq!(why(key("absent_entry")), Some(NoModel::NotInDictionary));
    assert_eq!(why(key("nowhere_declared")), Some(NoModel::Undeclared));
    assert_eq!(why(past), Some(NoModel::NotAnArchetype));
    assert!(
        matches!(why(key("garbage")), Some(NoModel::Version(_))),
        "{:?}",
        why(key("garbage"))
    );
    assert!(
        matches!(why(key("cut")), Some(NoModel::Refused(_))),
        "{:?}",
        why(key("cut"))
    );
    // A reason for each ask that gave no model, each in words of its own.
    let reasons: Vec<&'static str> = (read.iter())
        .filter_map(|outcome| outcome.model.as_ref().err().map(NoModel::kind))
        .collect();
    assert_eq!(reasons.len(), 7);
    assert!(reasons.iter().all(|reason| !reason.is_empty()));
    // The contract's reader hands back exactly those that read.
    let (install, mut models): (&dyn WorldInstall, Vec<(AssetKey, Model)>) = (&*world, Vec::new());
    (install.models()).models(&index, &asks, &mut Worker::new(), &mut models);
    let mut given: Vec<AssetKey> = models.iter().map(|(ask, _)| *ask).collect();
    given.sort();
    let mut wanted = vec![key("whole"), key("fallen_back"), BOX];
    wanted.sort();
    assert_eq!(given, wanted);
}

/// The scene of the kit with a texture dictionary of the Enhanced game beside it.
fn scene() -> (
    tempfile::TempDir,
    Arc<Gta5World>,
    Arc<Index>,
    tempfile::TempDir,
) {
    let mut files = hand_built::files();
    let nine = hand_built::textures_bytes(&[("slate", 9)]);
    files.push((
        "textures/nine.ytd".to_owned(),
        hand_built::enhanced("nine.ytd", &nine),
    ));
    let mut torn = hand_built::textures_bytes(&[("wall", 1)]);
    torn.truncate(torn.len() / 2);
    files.push(("textures/torn.ytd".to_owned(), torn));
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    (dir, world, index, scratch)
}

#[test]
fn a_texture_is_found_through_the_dictionaries_its_own_falls_through_to_and_each_is_opened_once() {
    let (_dir, world, index, _scratch) = scene();
    let ask = |scope: &str, name: &str, pixels| TextureAsk {
        scope: key(scope),
        name: u64::from(hash(name)),
        pixels,
    };
    let asks = [
        ask(OWN_TXD, WALL, u32::MAX),
        ask(OWN_TXD, WALL, 4),
        ask(OWN_TXD, MOSS, u32::MAX),
        ask(MID_TXD, WALL, u32::MAX),
        ask(ROOT_TXD, WALL, 1),
        ask(OWN_TXD, "no_such_texture", u32::MAX),
        ask(ROOT_TXD, MOSS, u32::MAX),
        ask("nine", "slate", u32::MAX),
        TextureAsk {
            scope: AssetKey(0),
            ..ask(OWN_TXD, WALL, u32::MAX)
        },
        ask("torn", WALL, u32::MAX),
    ];
    let mut read: Vec<(TextureAsk, Result<Texture, NoTexture>)> = Vec::new();
    let opened = world.read_textures(&index, &asks, &mut read);
    // The scene's own dictionary, the one at the end of its chain, the Enhanced one and the one
    // cut short: the middle of the chain has no file, and five asks walk the same two.
    assert_eq!(opened.files, 4);
    assert_eq!(read.len(), asks.len());
    let of = |at: usize| {
        let (given, texture) = &read[at];
        assert_eq!(*given, asks[at]);
        texture
    };
    let wall = hand_built::texture(WALL, WALL_SEED).backing_store;
    let moss = hand_built::texture(MOSS, MOSS_SEED).backing_store;
    let levels = |store: &[u8]| {
        let (first, rest) = store.split_at(LEVELS[0]);
        vec![first.to_vec(), rest[..LEVELS[1]].to_vec()]
    };
    let whole = |store: &[u8]| Texture {
        width: 8,
        height: 8,
        format: TextureFormat::Bc1,
        srgb: false,
        levels: levels(store),
    };
    // The level below the first, alone, where no more than four pixels a side are wanted, and the
    // smallest stored where fewer are wanted than any level has.
    let small = Texture {
        width: 4,
        height: 4,
        levels: levels(&wall).split_off(1),
        ..whole(&wall)
    };
    assert_eq!(of(0), &Ok(whole(&wall)));
    assert_eq!(of(1), &Ok(small.clone()));
    assert_eq!(of(2), &Ok(whole(&moss)));
    assert_eq!(of(3), &Ok(whole(&wall)));
    assert_eq!(of(4), &Ok(small));
    assert_eq!(
        of(5),
        &Err(NoTexture::NotInChain {
            walked: 3,
            absent: 1,
            unread: 0
        })
    );
    // A chain is walked towards what a dictionary falls through to, never back.
    assert_eq!(
        of(6),
        &Err(NoTexture::NotInChain {
            walked: 1,
            absent: 0,
            unread: 0
        })
    );
    assert_eq!(
        of(9),
        &Err(NoTexture::NotInChain {
            walked: 1,
            absent: 0,
            unread: 1
        })
    );
    let slate = of(7).as_ref().expect("the Enhanced dictionary's texture");
    assert_eq!(
        (slate.width, slate.height, slate.format),
        (8, 8, TextureFormat::Bc1)
    );
    assert_eq!(
        slate.levels[0],
        hand_built::texture("slate", 9).backing_store[..LEVELS[0]]
    );
    assert_eq!(of(8), &Err(NoTexture::NoDictionary));
}

/// The body of the map named `name`, as GTA V's own.
fn body_of(world: &Gta5World, index: &Index, name: &str) -> Arc<dyn MapBody> {
    let install: &dyn WorldInstall = world;
    let map = index.named(u64::from(hash(name))).expect("the map's row");
    let read = install.maps().read(index, map, &mut Worker::new());
    read.unwrap_or_else(|why| panic!("{name} does not read: {why}"))
}

fn own(body: &dyn MapBody) -> &Body {
    let body: &dyn Any = body;
    body.downcast_ref().expect("a body of GTA V")
}

#[test]
fn a_body_states_the_far_lights_and_the_occluders_of_its_map() {
    let (_dir, world, index, _scratch) = scene();
    let lit = body_of(&world, &index, "m_lights");
    let stated = hand_built::far_lights();
    let byte = |word: u32, shift: u32| (word >> shift & 0xFF) as f32 / 255.0;
    let lights = lit.lights();
    assert_eq!(lights.len(), 3);
    for (light, far) in lights.iter().zip(&stated) {
        assert_eq!(light.position, Vec3::from(far.at));
        assert_eq!(light.direction, Vec3::NEG_Z);
        assert_eq!(light.range, far.falloff);
        assert_eq!(light.hours, far.flags & ((1 << 24) - 1));
        assert_eq!(
            light.colour,
            [byte(far.rgbi, 16), byte(far.rgbi, 8), byte(far.rgbi, 0)]
        );
        assert_eq!(light.intensity, byte(far.rgbi, 24));
    }
    assert_eq!(lights[0].kind, LightKind::Point);
    let LightKind::Spot { inner, outer } = lights[1].kind else {
        panic!("{:?}", lights[1].kind);
    };
    // 51 and 102 of the 255 a half turn is counted in: a fifth and two fifths of it.
    let fifth = std::f32::consts::PI / 5.0;
    assert!((inner - fifth).abs() < 1e-6 && (outer - 2.0 * fifth).abs() < 1e-6);
    assert_eq!(lights[2].kind, LightKind::Capsule { length: 3.0 });
    assert_eq!(lights[1].hours, hand_built::NIGHT_HOURS);
    // The fourth is of a type the contract has no light for.
    assert_eq!(own(&*lit).beside.lights_unread, 1);
    assert!(lit.occluders().boxes.is_empty() && lit.occluders().meshes.is_empty());
    // The map that says where they stand states none by itself.
    let far = body_of(&world, &index, "m_lights_far");
    assert!(far.lights().is_empty());
    assert_eq!(own(&*far).beside.lights_unread, 0);

    let hides = body_of(&world, &index, "m_hides");
    assert!(hides.lights().is_empty());
    let occluders = hides.occluders();
    assert_eq!(occluders.boxes.len(), 2);
    for (held, stated) in occluders.boxes.iter().zip(hand_built::boxes()) {
        let [x, y, z, cos, length, width, height, sin] = stated.map(f32::from);
        // The corner of the box at the high end of its own x, y and z, by the angle of its turn.
        let angle = if (cos, sin) == (0.0, 0.0) {
            0.0
        } else {
            cos.atan2(sin)
        };
        let half = Vec3::new(length, width, height) / 8.0;
        let turned = glam::Quat::from_rotation_z(angle) * half;
        let corner = Vec3::new(x, y, z) / 4.0 + turned;
        let rows = held.rows;
        let at = |row: [f32; 4]| row[0] * 0.5 + row[1] * 0.5 + row[2] * 0.5 + row[3];
        let given = Vec3::new(at(rows[0]), at(rows[1]), at(rows[2]));
        assert!(given.distance(corner) < 1e-3, "{given} for {corner}");
    }
    // The second box is turned: its own x does not run along the world's.
    assert!(occluders.boxes[1].rows[1][0].abs() > 1.0);
    let [mesh] = &occluders.meshes[..] else {
        panic!("{} meshes", occluders.meshes.len());
    };
    let square = &hand_built::occlude_models()[0];
    let points: Vec<Vec3> = square
        .points
        .iter()
        .map(|point| Vec3::from(*point))
        .collect();
    assert_eq!(mesh.vertices, points);
    let indices: Vec<u32> = (square.triangles.iter().flatten())
        .map(|index| u32::from(*index))
        .collect();
    assert_eq!(mesh.indices, indices);
    // The other model states more triangles than its bytes hold.
    assert_eq!(own(&*hides).beside.occluders_unread, 1);
    assert!(lit.bytes() > 0 && hides.bytes() > far.bytes());
}
