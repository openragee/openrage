use glam::Quat;
use or_io::gta6::map::links::{LodLink, LodParentMap};
use or_io::gta6::map::{Aabb, Entity};
use or_maps::{Name, Placed, Scene};
use or_world::gizmo::Obb;

use super::*;

fn entity(position: [f32; 3]) -> Entity {
    Entity {
        interior: false,
        id: 0x51,
        rotation: [0.0, 0.0, 0.0, 1.0],
        archetype_word: 0x52,
        position,
        archetype_name: 0x53,
        flags: 0,
        scale_xy: 1.0,
        scale_z: 1.0,
        parent_index: -1,
        hash_e74c3c23: -1,
        lod_dist: 100.0,
        child_lod_dist: 0.0,
        num_children: 0,
        tint_value: 0,
        priority_level: 0,
        lod_level: 0,
    }
}

fn list(min: [f32; 3], max: [f32; 3]) -> InstanceList {
    InstanceList {
        archetype_name: 0x61,
        instances: 2,
        batch_aabb: Aabb { min, max },
    }
}

fn map(name: u32, entities: Vec<Entity>, prop_lists: Vec<InstanceList>) -> MapData {
    let none = Aabb {
        min: [0.0; 3],
        max: [0.0; 3],
    };
    MapData {
        name,
        name_word: u64::from(name) << 8,
        parent: 0,
        hash_6c0f9c8c: 0,
        hash_9e8d32a0: 0,
        hash_ec16451d: 0,
        hash_36c898aa: 0,
        flags: 0,
        content_flags: 0,
        streaming_extents: none,
        entities_extents: none,
        entities,
        prop_lists,
        grass_lists: Vec::new(),
    }
}

fn map_links(
    lod_parents: Vec<LodParentMap>,
    entities: Vec<LodLink>,
    prop_lists: Vec<LodLink>,
    prop_points: Vec<Vec<[f32; 3]>>,
) -> Links {
    Links::Data(MapLinks {
        lod_parents,
        entities,
        prop_lists,
        prop_points,
        grass_lists: Vec::new(),
    })
}

#[test]
fn a_child_finds_its_parent_in_a_map_body_open_and_a_line_joins_them() {
    const PARENT: u32 = 0xB0D1_E5ED;
    let (child_at, parent_at) = (
        PathBuf::from("lod-child-test.rpf/0x10.ymap"),
        PathBuf::from("lod-parent-test.rpf/0x20.ymap"),
    );
    let child = map(
        0xC41D,
        vec![entity([1.0, 2.0, 3.0])],
        vec![list([0.0; 3], [2.0, 4.0, 6.0])],
    );
    let parent = map(
        PARENT,
        vec![entity([10.0, 0.0, 0.0]), entity([20.0, 0.0, 0.0])],
        vec![list([0.0; 3], [1.0; 3])],
    );
    let link = |map, parent_index, instance| LodLink {
        map,
        parent_index,
        instance,
    };
    let lod_parents = vec![LodParentMap {
        name: PARENT,
        word: 7,
    }];
    let child_links = map_links(
        lod_parents,
        vec![link(Some(0), 1, -1)],
        vec![link(Some(0), 2, 1)],
        vec![vec![[0.5, 0.5, 0.5]; 3]],
    );
    let parent_links = map_links(
        Vec::new(),
        vec![link(None, -1, -1); 2],
        vec![link(None, -1, -1)],
        vec![vec![[5.0; 3], [6.0, 7.0, 8.0]]],
    );
    opened().insert(parent_at.clone(), Ok(Arc::new(MapBody::Data(parent))));
    links().insert(child_at.clone(), Ok(Arc::new(child_links)));
    links().insert(parent_at.clone(), Ok(Arc::new(parent_links)));

    let shown = shown(&child_at, &child);
    let found: Vec<_> = (shown.parents.iter())
        .map(|p| p.as_ref().and_then(|p| p.found.clone()))
        .collect();
    assert_eq!(
        found,
        [
            Some((parent_at.clone(), Some(LodTarget::Entity(1)))),
            Some((
                parent_at.clone(),
                Some(LodTarget::PropList {
                    list: 0,
                    instance: Some(1)
                })
            )),
        ]
    );
    let ends: Vec<[f32; 3]> = shown.lines.iter().map(|v| v.position).collect();
    assert_eq!(
        ends,
        [[1.0, 2.0, 3.0], [20.0, 0.0, 0.0], [1.0, 2.0, 3.0], [6.0, 7.0, 8.0]],
        "the entity to the parent's entity 1, the prop list's centre to the parent list's instance 1"
    );
    assert_eq!((shown.prop_instances, shown.grass_instances), (3, 0));
    assert_eq!(shown.points.len(), 3);

    let none = super::shown(
        Path::new("no-links-test.rpf/0x30.ymap"),
        &map(1, Vec::new(), Vec::new()),
    );
    assert!(none.parents.is_empty() && none.refused.is_none() && none.lines.is_empty());
}

#[test]
fn a_dictionary_resolves_where_exactly_one_body_holds_every_key_of_its_archetypes() {
    let keys: HashMap<u32, Vec<u64>> = [
        (0xA1, vec![0x100, 0x200]),
        (0xA2, vec![0x200]),
        (0xA3, vec![0x100, 0x200]),
    ]
    .into();
    let names = |n: &[u32]| n.iter().copied().collect::<HashSet<u32>>();
    assert_eq!(sole_holder(&keys, &names(&[0xA1, 0xA2])), Some(0x200));
    assert_eq!(
        sole_holder(&keys, &names(&[0xA1, 0xA3])),
        None,
        "two bodies hold both"
    );
    assert_eq!(
        sole_holder(&keys, &names(&[0xA1, 0xA4])),
        None,
        "no body holds 0xA4"
    );
    assert_eq!(sole_holder(&keys, &names(&[])), None);

    let archive = PathBuf::from("dictionary-models-test.rpf");
    dictionary_keys().insert(archive.clone(), keys);
    let a = |name, dictionary| Some((Source::Scan(0x40), archetype(name, dictionary)));
    let archetypes = vec![
        a(0xA1, 0xD1),
        a(0xA2, 0xD1),
        a(0xA3, 0xD2),
        a(0xA9, 0),
        None,
    ];
    let mut models = vec![None, None, None, Some(0x900), None];
    dictionary_models(Some(&archive), &archetypes, &mut models);
    assert_eq!(models, [Some(0x200), Some(0x200), None, Some(0x900), None]);
}

fn archetype(name: u32, drawable_dictionary: u64) -> Archetype {
    Archetype {
        class: ArchetypeClass::Base,
        name,
        name_word: u64::from(name) << 4,
        texture_dictionary: 0,
        drawable_dictionary,
        drawable: if drawable_dictionary == 0 {
            u64::from(name) << 4
        } else {
            0
        },
        fragment: 0,
        lod_dist: 100.0,
        special_attribute: 0,
        bb_min: [-1.0; 3],
        bb_max: [1.0; 3],
        bs_centre: [0.0; 3],
        bs_radius: 1.7,
        flags: 0,
        hd_texture_dist: 0.0,
        clip_dictionary: 0,
        physics_dictionary: 0,
        asset_type: if drawable_dictionary == 0 { 2 } else { 3 },
        asset_name: 0,
    }
}

fn placed_at(index: usize, x: f32) -> Placed {
    Placed {
        source: "in-place-test".into(),
        index,
        archetype: Name::of(0x53),
        declared: true,
        model: None,
        obb: Obb {
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 2.0, 3.0),
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        },
        interior: false,
        lod_dist: 100.0,
        parent_index: -1,
        guid: 0,
        lod_level: None,
    }
}

/// A row of `count` entities a metre apart along x, each with a model at `0x1000 + index` but the
/// second, whose archetype is a drawable dictionary's from the third on.
fn row_of(count: usize) -> ScannedMap {
    let positions: Vec<f32> = (0..count).map(|i| i as f32).collect();
    let body = map(
        1,
        positions.iter().map(|x| entity([*x, 0.0, 0.0])).collect(),
        Vec::new(),
    );
    ScannedMap {
        path: PathBuf::from("in-place-test.rpf/0x0.ymap"),
        map: Arc::new(MapBody::Data(body)),
        scene: Scene {
            placed: positions
                .iter()
                .enumerate()
                .map(|(i, x)| placed_at(i, *x))
                .collect(),
            bounds: None,
        },
        archetypes: (0..count)
            .map(|i| {
                Some((
                    Source::Scan(0),
                    archetype(0xA0 + i as u32, if i >= 2 { 0xD0 } else { 0 }),
                ))
            })
            .collect(),
        models: (0..count)
            .map(|i| (i != 1).then_some(0x1000 + i as u64))
            .collect(),
        key: (0, 0),
        links: Arc::new(Shown::default()),
    }
}

#[test]
fn the_models_drawn_in_place_are_the_nearest_within_the_radius_and_never_more_than_the_most() {
    let scanned = row_of(5);
    let near = targets(&scanned, 0, 2.5);
    let offsets: Vec<u64> = near.iter().map(|t| t.offset).collect();
    assert_eq!(
        offsets,
        [0x1000, 0x1002],
        "entity 1 has no model and 3 is past the radius"
    );
    assert_eq!(near[0].entry, None);
    assert_eq!(
        near[1].entry,
        Some(0xA2),
        "a dictionary's drawable is the entry its archetype names"
    );
    let placed = near[1].place.transform_point3(Vec3::new(1.0, 1.0, 1.0));
    assert!(
        (placed - Vec3::new(4.0, 2.0, 3.0)).length() < 1e-5,
        "{placed}"
    );
    let alone: Vec<u64> = targets(&scanned, 3, 0.0).iter().map(|t| t.offset).collect();
    assert_eq!(alone, [0x1003], "at 0 m the entity selected alone");

    let many = row_of(MOST_MODELS * 2);
    let far = targets(&many, MOST_MODELS, 1e6);
    assert_eq!(far.len(), MOST_MODELS);
    assert_eq!(
        far[0].offset,
        0x1000 + MOST_MODELS as u64,
        "the selected one first"
    );
}
