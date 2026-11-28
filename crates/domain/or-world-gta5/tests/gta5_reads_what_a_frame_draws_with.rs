//! What a frame of GTA V's world is handed beside its instances, over installs built by hand: a
//! model however its file holds it and for either target, the reason of one that does not read,
//! a texture through the dictionaries its own falls through to, and a map's far lights and
//! occluders.

mod forest_fixture;
mod hand_built;
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
    // Asked with its reasons, the same reader answers every ask once, each refusal as the sentence of its reason and what the file said.
    let mut answered = Vec::new();
    (install.models()).models_or_why(&index, &asks, &mut Worker::new(), &mut answered);
    assert_eq!(answered.len(), asks.len());
    for (ask, model) in &answered {
        let said = model
            .as_ref()
            .err()
            .map(|refused| (refused.why, &refused.detail[..]));
        let reason = why(*ask);
        let stated = reason.as_ref().map(|why| (why.said(), why.to_string()));
        assert_eq!(
            said,
            stated.as_ref().map(|(said, detail)| (*said, &detail[..]))
        );
    }
    let nowhere = or_world::model::TextureAsk {
        scope: AssetKey(0),
        name: 1,
        pixels: 64,
    };
    let mut textures = Vec::new();
    (install.models()).textures_or_why(&index, &[nowhere], &mut Worker::new(), &mut textures);
    let refused = textures.pop().and_then(|(_, texture)| texture.err());
    assert_eq!(
        refused.map(|refused| refused.why),
        Some(or_world_gta5::NoTexture::NoDictionary.said())
    );
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
    // Of the other two, one states more triangles than its bytes hold and one names a point it lacks.
    assert_eq!(own(&*hides).beside.occluders_unread, 2);
    // Its lists are counted, grass and props alike, and none is laid out.
    assert_eq!(
        own(&*hides).beside.lists as usize,
        hand_built::LISTS.0 + hand_built::LISTS.1
    );
    assert_eq!(own(&*lit).beside.lists, 0);
    // The body says so through the contract, for a panel that knows no game to count.
    assert_eq!(
        hides.unlaid_lists() as usize,
        hand_built::LISTS.0 + hand_built::LISTS.1
    );
    assert_eq!(lit.unlaid_lists(), 0);
    assert!(lit.bytes() > 0 && hides.bytes() > far.bytes());
}

#[test]
fn a_channel_an_enhanced_file_keeps_in_a_run_of_its_own_is_gathered_a_vertex_at_a_time() {
    use or_io::codec::mesh::elements_g9;
    use or_io::decl::{File, Gen9};
    use or_io::gta5::GtaDrawable;
    let legacy = hand_built::drawable_bytes("runs9", 1);
    let woven = hand_built::enhanced("runs9.ydr", &legacy);
    // The same vertices with each channel in a run of its own: every position, then every normal, then every coordinate.
    let mut apart = <GtaDrawable<Gen9> as File>::from_bytes(&woven).expect("the file reads");
    let lod = &mut apart.base.lod_group.lods[0];
    let buffer = &mut *lod.models[0].geometries[0].vertex_buffer;
    let together = std::mem::take(&mut buffer.data);
    let stride = together.len() / CORNERS.len();
    let info = buffer.info.as_deref_mut().expect("a declaration");
    let mut sizes = Vec::new();
    for element in elements_g9(&info.offsets, &info.sizes, &info.types) {
        let size = element.format.byte_size().expect("a size");
        info.offsets[element.slot] = buffer.data.len() as u32;
        info.sizes[element.slot] = size as u8;
        for vertex in 0..CORNERS.len() {
            let at = vertex * stride + element.offset as usize;
            buffer.data.extend_from_slice(&together[at..at + size]);
        }
        sizes.push(size as u32);
    }
    assert_ne!(buffer.data, together, "the runs are the woven bytes");
    let files: Files = vec![
        types_file("props", &[held("woven9", ""), held("runs9", "")]),
        ("models/woven9.ydr".to_owned(), woven),
        (
            "models/runs9.ydr".to_owned(),
            apart.to_bytes().expect("a drawable"),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let mut read = Vec::new();
    world.read_models(&index, &[key("woven9"), key("runs9")], &mut read);
    let mesh_of = |name: &str| {
        let outcome = (read.iter().find(|outcome| outcome.ask == key(name))).expect("an outcome");
        assert_eq!(outcome.target, Some(Generation::Gen9));
        match &outcome.model {
            Ok(model) => model.lods[0].meshes[0].clone(),
            Err(why) => panic!("{name} does not read: {why}"),
        }
    };
    let (woven, runs) = (mesh_of("woven9"), mesh_of("runs9"));
    // A vertex of the gathered mesh is its kept elements end to end, in the order of the file's channels.
    let mut at = 0;
    for (element, size) in runs.layout.elements.iter().zip(&sizes) {
        assert_eq!((element.offset, element.format.size()), (at, *size));
        at += size;
    }
    assert_eq!(runs.layout.elements.len(), sizes.len());
    assert_eq!(runs.layout.stride, at);
    assert_eq!(runs.vertices.len(), CORNERS.len() * at as usize);
    for semantic in [Semantic::Position, Semantic::Normal, Semantic::TexCoord(0)] {
        assert_eq!(numbers(&runs, semantic), numbers(&woven, semantic));
    }
    assert_eq!(numbers(&runs, Semantic::Position), CORNERS);
    assert_eq!(runs.indices, woven.indices);
}

#[test]
fn a_model_two_files_hold_is_read_from_the_dictionary_then_the_fragment_and_a_surface_blends_by_its_bucket(
) {
    use or_io::decl::File;
    let bucket = |name: &str, bucket: u8| {
        let mut built = hand_built::drawable(name, 1);
        let group = built.base.shader_group.as_deref_mut().expect("a group");
        group.shaders[0].draw_bucket = bucket;
        (
            format!("models/{name}.ydr"),
            built.to_bytes().expect("a drawable"),
        )
    };
    let mut files: Files = vec![
        types_file(
            "props",
            &[
                held("both", "pack_both"),
                held("twice", ""),
                held("solid", ""),
                held("glass", ""),
                held("decal", ""),
                held("water", ""),
            ],
        ),
        // Each loose drawable has one level, the dictionary's entry three and the fragment two.
        (
            "models/both.ydr".to_owned(),
            hand_built::drawable_bytes("both", 1),
        ),
        (
            "models/pack_both.ydd".to_owned(),
            hand_built::dictionary_bytes(&[("both", 3)]),
        ),
        (
            "models/twice.ydr".to_owned(),
            hand_built::drawable_bytes("twice", 1),
        ),
        (
            "models/twice.yft".to_owned(),
            hand_built::fragment_bytes("twice", 2),
        ),
    ];
    let buckets = [("solid", 0), ("glass", 1), ("decal", 2), ("water", 6)];
    files.extend(buckets.map(|(name, at)| bucket(name, at)));
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let names = ["both", "twice", "solid", "glass", "decal", "water"];
    let asks: Vec<AssetKey> = names.iter().map(|name| key(name)).collect();
    let mut read = Vec::new();
    world.read_models(&index, &asks, &mut read);
    let of = |name: &str| {
        let outcome = (read.iter().find(|outcome| outcome.ask == key(name))).expect("an outcome");
        let model = outcome.model.as_ref();
        let model = model.unwrap_or_else(|why| panic!("{name} does not read: {why}"));
        (
            outcome.holding,
            model.lods.len(),
            model.lods[0].meshes[0].surface.alpha,
        )
    };
    assert_eq!(of("both").0, Some(Holding::Dictionary));
    assert_eq!(of("both").1, 3);
    assert_eq!(of("twice").0, Some(Holding::Fragment));
    assert_eq!(of("twice").1, 2);
    assert_eq!(of("solid").2, Alpha::Opaque);
    for blended in ["glass", "decal", "water"] {
        assert_eq!(of(blended).2, Alpha::Blend, "{blended}");
    }
}

#[test]
fn a_model_with_no_level_no_drawable_or_no_triangle_list_states_which() {
    use or_io::decl::{File, Gen8};
    use or_io::gta5::{FragType, GtaDrawable, GtaFragType, RmcDrawable};
    // A triangle strip, where the model's triangles are three indices each.
    let mut strip = hand_built::drawable("strip", 1);
    let lod = &mut strip.base.lod_group.lods[0];
    lod.models[0].geometries[0].primitive_type = 4;
    let level_less = GtaDrawable::<Gen8> {
        base: RmcDrawable {
            name: "level_less".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut hollow = GtaFragType::<Gen8>::default();
    *hollow = FragType::default();
    let ydr = |name: &str, built: &GtaDrawable<Gen8>| {
        (
            format!("models/{name}.ydr"),
            built.to_bytes().expect("a drawable"),
        )
    };
    let names = ["strip", "level_less", "hollow"];
    let types: Vec<Arch> = names.iter().map(|name| held(name, "")).collect();
    let files: Files = vec![
        types_file("props", &types),
        ydr("strip", &strip),
        ydr("level_less", &level_less),
        (
            "models/hollow.yft".to_owned(),
            hollow.to_bytes().expect("a fragment"),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let asks: Vec<AssetKey> = names.iter().map(|name| key(name)).collect();
    let mut read = Vec::new();
    world.read_models(&index, &asks, &mut read);
    let why = |name: &str| {
        let outcome = (read.iter().find(|outcome| outcome.ask == key(name))).expect("an outcome");
        outcome.model.as_ref().err().cloned()
    };
    assert_eq!(why("strip"), Some(NoModel::NotTriangles));
    assert_eq!(why("level_less"), Some(NoModel::NoLevel));
    assert_eq!(why("hollow"), Some(NoModel::NoDrawable));
}

#[test]
fn a_texture_reads_in_the_format_its_file_states_and_one_that_cannot_says_why() {
    use or_io::decl::File;
    use or_io::gta5::gen8::GrcTexture;
    use or_io::gta5::gen9::{GrcTextureG9, TileMode};
    use or_io::gta5::PgDictionary;
    let four_cc = |code: &[u8; 4]| u32::from_le_bytes(*code);
    let made = |name: &str, change: &dyn Fn(&mut GrcTexture)| {
        let mut texture = hand_built::texture(name, 11);
        change(&mut texture);
        texture
    };
    let mut formats = PgDictionary::<GrcTexture>::default();
    // One level each, the first of colours the file says are sRGB; the last is a format's number and no four letters.
    let stated = [
        ("three", four_cc(b"DXT3"), TextureFormat::Bc2),
        ("five", four_cc(b"DXT5"), TextureFormat::Bc3),
        ("one_channel", four_cc(b"ATI1"), TextureFormat::Bc4),
        ("two_channels", four_cc(b"ATI2"), TextureFormat::Bc5),
        ("seven", four_cc(b"BC7 "), TextureFormat::Bc7),
        ("plain", 32, TextureFormat::Rgba8),
        ("coverage", 28, TextureFormat::A8),
    ];
    for (name, format, _) in stated {
        formats.insert(made(name, &|texture| {
            texture.format = format;
            texture.mip_count = 1;
            texture.info_bits |= u8::from(name == "five");
            texture.backing_store = (0..=255).collect();
        }));
    }
    // Luminance alone, by its number: a format the core's texture has no word for.
    formats.insert(made("grey", &|texture| texture.format = 50));
    formats.insert(made("cube", &|texture| texture.image_type = 1));
    formats.insert(made("deep", &|texture| texture.depth = 4));
    // Its first level alone is more bytes than its store holds.
    formats.insert(made("short", &|texture| {
        texture.width = 64;
        texture.height = 64;
        texture.mip_count = 1;
    }));
    let plain = hand_built::enhanced("tiles9.ytd", &hand_built::textures_bytes(&[("tile", 3)]));
    let mut tiled =
        <PgDictionary<GrcTextureG9> as File>::from_bytes(&plain).expect("the dictionary reads");
    let tile = tiled.entries[0].as_deref_mut().expect("a texture");
    tile.tile_mode = or_io::decl::Open::new(TileMode::Standard);
    let files: Files = vec![
        (
            "textures/formats.ytd".to_owned(),
            formats.to_bytes().expect("a dictionary"),
        ),
        ("textures/plain9.ytd".to_owned(), plain),
        (
            "textures/tiles9.ytd".to_owned(),
            tiled.to_bytes().expect("a dictionary"),
        ),
        (
            "textures/ring_a.ytd".to_owned(),
            hand_built::textures_bytes(&[(MOSS, MOSS_SEED)]),
        ),
        // Two dictionaries that each fall through to the other.
        (
            "data/gtxd.meta".to_owned(),
            hand_built::relationships_bytes(&[("ring_a", "ring_b"), ("ring_b", "ring_a")]),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let ask = |scope: &str, name: &str| TextureAsk {
        scope: key(scope),
        name: u64::from(hash(name)),
        pixels: u32::MAX,
    };
    let mut asks = vec![
        ask("formats", "five"),
        ask("formats", "grey"),
        ask("formats", "cube"),
        ask("formats", "deep"),
        ask("formats", "short"),
        ask("plain9", "tile"),
        ask("tiles9", "tile"),
        ask("ring_a", "no_such_texture"),
        ask("ring_b", MOSS),
    ];
    asks.extend(stated.map(|(name, ..)| ask("formats", name)));
    let mut read: Vec<(TextureAsk, Result<Texture, NoTexture>)> = Vec::new();
    let opened = world.read_textures(&index, &asks, &mut read);
    assert_eq!(opened.files, 4);
    let of = |at: usize| {
        let (given, texture) = &read[at];
        assert_eq!(*given, asks[at]);
        texture
    };
    let store: Vec<u8> = (0..=255).collect();
    for (at, (name, _, format)) in stated.into_iter().enumerate() {
        assert_eq!(
            of(9 + at),
            &Ok(Texture {
                width: 8,
                height: 8,
                format,
                srgb: name == "five",
                levels: vec![store[..format.level_bytes(8, 8)].to_vec()],
            }),
            "{name}"
        );
    }
    assert_eq!(of(0), of(10));
    assert_eq!(of(1), &Err(NoTexture::Format("L8".to_owned())));
    assert_eq!(of(2), &Err(NoTexture::NotFlat));
    assert_eq!(of(3), &Err(NoTexture::NotFlat));
    assert_eq!(of(4), &Err(NoTexture::PartLevel));
    assert!(of(5).is_ok(), "{:?}", of(5));
    assert_eq!(of(6), &Err(NoTexture::Tiled));
    // A chain that comes back to where it began is walked once round and no further.
    assert_eq!(
        of(7),
        &Err(NoTexture::NotInChain {
            walked: 2,
            absent: 1,
            unread: 0
        })
    );
    assert!(of(8).is_ok(), "{:?}", of(8));
}

/// One item of a water file's quads: its corners, its type, whether it is hidden and its height.
fn water_item([west, east, south, north]: [i32; 4], kind: u8, hidden: bool, z: f32) -> String {
    format!(
        "<Item><minX value=\"{west}\" /><maxX value=\"{east}\" /><minY value=\"{south}\" />\
         <maxY value=\"{north}\" /><Type value=\"{kind}\" /><IsInvisible value=\"{hidden}\" />\
         <z value=\"{z:.2}\" /></Item>"
    )
}

#[test]
fn the_water_file_of_an_install_reaches_the_world_as_sheets_at_their_heights() {
    use or_world::caps::Capability;
    use or_world::model::{Corner, WaterQuad};
    // A whole rectangle, a half that keeps its north-west corner, one hidden and one the game leaves out.
    let items = [
        water_item([-8, 8, 0, 4], 0, false, 1.5),
        water_item([10, 20, -4, 6], 2, false, 30.25),
        water_item([30, 40, 0, 4], 0, true, 2.0),
        water_item([50, 60, 0, 4], 5, false, 3.0),
    ];
    let file = format!(
        "<WaterData><WaterQuads>{}</WaterQuads></WaterData>",
        items.concat()
    );
    let water: Files = vec![("data/levels/gta5/water.xml".to_owned(), file.into_bytes())];
    let dir = install(&[("common.rpf", water), ("x64a.rpf", hand_built::files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    assert!(world.caps().has(Capability::Water));
    let sheets = world.water(&index).expect("the water file reads");
    assert_eq!(
        sheets,
        [
            WaterQuad {
                min: [-8.0, 0.0],
                max: [8.0, 4.0],
                height: 1.5,
                half: None,
            },
            WaterQuad {
                min: [10.0, -4.0],
                max: [20.0, 6.0],
                height: 30.25,
                half: Some(Corner::NorthWest),
            },
        ]
    );
    // Every corner of a sheet's surface stands at its height, and a half is one triangle that keeps its corner.
    let half = sheets[1].triangles();
    assert_eq!(half.len(), 1);
    assert!(half[0].contains(&Vec3::new(10.0, 6.0, 30.25)));
    assert!(!half[0].contains(&Vec3::new(20.0, -4.0, 30.25)));
    for corner in sheets.iter().flat_map(|sheet| {
        let height = sheet.height;
        (sheet.triangles().into_iter().flatten()).map(move |corner| (corner, height))
    }) {
        assert_eq!(corner.0.z, corner.1);
    }

    // An install with no water file has no water, and a file that is no water file is refused.
    let dry = install(&[("x64a.rpf", hand_built::files())]);
    let (world, index) = opened(dry.path(), scratch.path());
    assert_eq!(world.water(&index).expect("no file is no fault"), []);
    let broken: Files = vec![(
        "data/levels/gta5/water.xml".to_owned(),
        b"<Other />".to_vec(),
    )];
    let bad = install(&[("common.rpf", broken), ("x64a.rpf", hand_built::files())]);
    let (world, index) = opened(bad.path(), scratch.path());
    assert!(world.water(&index).is_err());
}

#[test]
fn the_world_names_its_weathers_the_clear_day_first_and_its_noon_sun_lights_ground_and_wall() {
    use or_world::model::Moment;
    let dir = install(&[("x64a.rpf", hand_built::files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index) = opened(dir.path(), scratch.path());
    let weathers = world.look().weathers(&index);
    assert_eq!(weathers[0].name, "CLEAR");
    assert_eq!(weathers[0].id, u64::from(hash("CLEAR")));
    assert_eq!(weathers[0].id, hand_built::weather());
    let mut ids: Vec<u64> = weathers.iter().map(|weather| weather.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), weathers.len(), "each weather is named once");
    assert!(weathers.iter().any(|weather| weather.name == "RAIN"));

    struct Unfed;
    impl or_world::model::Feed for Unfed {
        fn set(&mut self, _: or_world::model::FeedName<'_>, _: &[f32]) {}
    }
    let at = |hour: f32| {
        let moment = Moment {
            hour,
            weather: weathers[0].id,
        };
        world.look().frame(&moment, &mut Unfed)
    };
    let noon = at(12.0).sun;
    assert!((noon.length() - 1.0).abs() < 1e-5);
    // At noon the ground takes most of the sun and a wall that faces south a good share of it; none comes from the north.
    assert!(noon.dot(Vec3::Z) >= 0.7, "{noon}");
    assert!(noon.dot(Vec3::NEG_Y) >= 0.4, "{noon}");
    assert!(noon.x.abs() < 1e-5);
    // It rises in the east and sets in the west, level with the ground.
    let (dawn, dusk) = (at(6.0).sun, at(18.0).sun);
    assert!(dawn.x > 0.99 && dusk.x < -0.99, "{dawn} {dusk}");
    assert!(dawn.z.abs() < 1e-5 && dusk.z.abs() < 1e-5);
    assert!(at(0.0).sun.z < 0.0 && at(0.0).key == [0.0; 3]);
}
