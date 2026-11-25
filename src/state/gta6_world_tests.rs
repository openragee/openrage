use or_io::gta6::map::lanes::Phase;
use or_io::gta6::map::lights::{LightAttr, Pose};
use or_io::gta6::map::world::AreaBody;
use or_io::gta6::map::{Aabb, Archetype, ArchetypeClass, Entity};

use super::*;

fn node(k: u16, position: [f32; 3], first_link: u32) -> PathNode {
    PathNode {
        position,
        flags: 0,
        address: NodeAddress(0, k),
        hash_2522b86a: NodeAddress(0, 0),
        street_name: 0,
        junction_address: NodeAddress(5, k),
        first_link,
    }
}

fn link(to: u16) -> Link {
    Link {
        target: NodeAddress(0, to),
        distance: 10,
        lanes_from_other_node: 1,
        lanes_to_other_node: 1,
        flags: 0,
    }
}

fn junction(k: u16, phases: usize) -> Junction {
    let phase = Phase {
        duration: 10.0,
        light_boxes: Vec::new(),
    };
    Junction {
        node: NodeAddress(5, k),
        entrances: Vec::new(),
        phases: vec![phase; phases],
    }
}

#[test]
fn the_lane_graph_is_drawn_from_whichever_of_its_lists_are_read() {
    let mut lanes = Lanes::default();
    lanes.keep(LaneBody::Links(vec![link(1), link(0), link(2)]));
    assert!(lanes.points.is_empty() && lanes.segments.is_empty());

    // Node 2 stands nowhere: it is no point and no end of a segment.
    let nodes = vec![
        node(0, [0.0, 0.0, 1.0], 0),
        node(1, [10.0, 0.0, 1.0], 1),
        node(2, [f32::NAN, 0.0, 1.0], 3),
    ];
    lanes.keep(LaneBody::Nodes(nodes));
    assert_eq!(lanes.points, [[0.0, 0.0], [10.0, 0.0]]);
    assert_eq!(
        lanes.segments,
        [[[0.0, 0.0], [10.0, 0.0]]],
        "a link and the link back are one segment"
    );
    assert!(lanes.signals.is_empty());

    lanes.keep(LaneBody::Junctions(vec![
        junction(1, 2),
        junction(0, 0),
        junction(9, 1),
    ]));
    assert_eq!(
        lanes.signals,
        [[10.0, 0.0]],
        "the one signalised junction whose node is there"
    );
}

fn entity(word: u64, position: [f32; 3]) -> Entity {
    Entity {
        interior: false,
        id: 1,
        rotation: [0.0, 0.0, 0.0, 1.0],
        archetype_word: word,
        position,
        archetype_name: word as u32,
        flags: 0,
        scale_xy: 1.0,
        scale_z: 1.0,
        parent_index: -1,
        hash_e74c3c23: 0,
        lod_dist: -1.0,
        child_lod_dist: 0.0,
        num_children: 0,
        tint_value: 0,
        priority_level: 0,
        lod_level: 5,
    }
}

fn map_of(name: u32, lo: f32, hi: f32, entities: Vec<Entity>) -> MapData {
    let extents = Aabb {
        min: [lo, lo, -10.0],
        max: [hi, hi, 10.0],
    };
    MapData {
        name,
        name_word: u64::from(name),
        parent: 0,
        hash_6c0f9c8c: 0,
        hash_9e8d32a0: 0,
        hash_ec16451d: 0,
        hash_36c898aa: 0,
        flags: 0,
        content_flags: 0,
        streaming_extents: extents,
        entities_extents: extents,
        entities,
        prop_lists: Vec::new(),
        grass_lists: Vec::new(),
    }
}

#[test]
fn every_map_opened_is_indexed_once_and_the_plan_gets_the_points_of_the_others() {
    let mut world = World::default();
    let (a, b) = (Path::new("a/0x10.rbm"), Path::new("a/0x20.rbm"));
    let near = map_of(1, 0.0, 100.0, vec![entity(7, [5.0, 6.0, 0.0])]);
    world.index(a, &near);
    world.index(a, &near);
    let far = vec![entity(7, [50.0, 60.0, 0.0]), entity(7, [500.0, 600.0, 0.0])];
    world.index(b, &map_of(2, 0.0, 1000.0, far));
    world.index(Path::new("a/0x30.rbm"), &map_of(3, 0.0, 1.0, Vec::new()));
    assert_eq!(world.indexed(), (2, 3));

    let all = |except| -> Vec<[f32; 2]> {
        world
            .placements([0.0, 0.0], [100.0, 100.0], except)
            .collect()
    };
    assert_eq!(all(None), [[5.0, 6.0], [50.0, 60.0]]);
    assert_eq!(
        all(Some(a)),
        [[50.0, 60.0]],
        "the map shown is drawn as boxes"
    );
}

fn light(posn: [f32; 3], colour: [u8; 3], falloff: f32) -> LightAttr {
    LightAttr {
        hash_30eca388: posn,
        posn,
        colour,
        vol_outer_colour: [0; 3],
        intensity: 1.0,
        flags: 0,
        time_flags: 0xFF_FFFF,
        falloff,
        falloff_exponent: 8.0,
        vol_intensity: 0.0,
        vol_size_scale: 0.0,
        vol_outer_intensity: 0.0,
        vol_outer_exponent: 0.0,
        corona_size: 0.0,
        corona_intensity: 0.0,
        corona_z_bias: 0.0,
        shadow_near_clip: 0.0,
        direction: [0.0, 0.0, -1.0],
        tangent: [1.0, 0.0, 0.0],
        cone_inner_angle: 0.0,
        cone_outer_angle: 0.0,
        extents: [0.0; 3],
        flashiness: 0,
        light_fade_distance: 0,
        shadow_fade_distance: 0,
        specular_fade_distance: 0,
        volumetric_fade_distance: 0,
        light_type: 1,
        group_id: 0,
        shadow_blur: 0,
        light_hash: 0,
    }
}

fn effect(posn: [f32; 3], colour: [u8; 3]) -> LightEffect {
    LightEffect {
        name: 1,
        offset_position: [0.0; 3],
        instances: vec![light(posn, colour, 12.0)],
        hash_c4e96a00: Vec::new(),
    }
}

fn pose_at(position: [f32; 3]) -> Pose {
    Pose {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
    }
}

fn archetype(name_word: u64) -> Archetype {
    Archetype {
        class: ArchetypeClass::Base,
        name: name_word as u32,
        name_word,
        texture_dictionary: 0,
        drawable_dictionary: 0,
        drawable: name_word,
        fragment: 0,
        lod_dist: 100.0,
        special_attribute: 0,
        bb_min: [0.0; 3],
        bb_max: [1.0; 3],
        bs_centre: [0.5; 3],
        bs_radius: 1.0,
        flags: 0,
        hd_texture_dist: 0.0,
        clip_dictionary: 0,
        physics_dictionary: 0,
        asset_type: 2,
        asset_name: name_word as u32,
    }
}

#[test]
fn a_map_s_lights_are_its_entities_own_and_those_of_the_archetypes_they_place() {
    let mut world = World::default();
    let path = Path::new("a/0x10.rbm");
    let own = MapLights {
        effects: vec![
            (LightOwner::Entity(1), effect([1.0, 0.0, 0.0], [255, 0, 0])),
            // An interior's light stands in the interior's space and is not drawn.
            (
                LightOwner::InteriorEntity {
                    archetype: 0,
                    entity: 0,
                    pose: pose_at([0.0; 3]),
                },
                effect([0.0; 3], [1, 1, 1]),
            ),
        ],
        other_extensions: 0,
    };
    world.lights.insert(path.to_path_buf(), Arc::new(own));
    let types = MapTypes {
        name: 9,
        archetypes: vec![archetype(7), archetype(8)],
    };
    let of_types = MapLights {
        effects: vec![(
            LightOwner::Archetype(0),
            effect([0.0, 2.0, 0.0], [0, 255, 0]),
        )],
        other_extensions: 0,
    };
    world.keep_archetype_lights(&types, &of_types);
    let version = world.version;
    world.keep_archetype_lights(&types, &of_types);
    assert_eq!(
        world.version, version,
        "a body read twice adds its lights once"
    );

    // Entities 0 and 2 place archetype 7, which has a light; entity 1 has one of its own.
    let map = map_of(
        1,
        0.0,
        100.0,
        vec![
            entity(7, [10.0, 10.0, 0.0]),
            entity(8, [20.0, 20.0, 0.0]),
            entity(7, [30.0, 30.0, 0.0]),
        ],
    );
    let lights = world.place(path, &map);
    let red = Light {
        at: [21.0, 20.0],
        colour: [255, 0, 0],
        range: 12.0,
    };
    let green = |at| Light {
        at,
        colour: [0, 255, 0],
        range: 12.0,
    };
    assert_eq!(lights, [red, green([10.0, 12.0]), green([30.0, 32.0])]);
    assert!(world
        .place(Path::new("a/0x20.rbm"), &map_of(2, 0.0, 1.0, Vec::new()))
        .is_empty());
}

/// A fill of two maps, a map types body that states a lit archetype and a lit interior, and the
/// three lane lists; `region` keeps the maps that meet it.
fn gathered(region: Option<&fill::Region>) -> Filled {
    let mut read = fill::Gathered::default();
    // The map at 0x100: an entity of archetype 7, one of archetype 8 with a light of its own, and
    // an instance of interior 9 that turns it a quarter about +z, x onto y.
    let s = std::f32::consts::FRAC_1_SQRT_2;
    let mut instance = entity(9, [100.0, 100.0, 0.0]);
    (instance.interior, instance.rotation) = (true, [0.0, 0.0, s, s]);
    let near = vec![
        entity(7, [10.0, 10.0, 0.0]),
        entity(8, [20.0, 20.0, 0.0]),
        instance,
    ];
    let own = MapLights {
        effects: vec![(LightOwner::Entity(1), effect([1.0, 0.0, 0.0], [255, 0, 0]))],
        other_extensions: 0,
    };
    let near = MapBody::Data(map_of(1, 0.0, 200.0, near));
    read.add(0x100, AreaBody::Map(near, Some(own)), region);
    // The map at 0x200, far off: another entity of archetype 7, and lights that did not read.
    let far = vec![entity(7, [5050.0, 5050.0, 0.0])];
    let far = MapBody::Data(map_of(2, 5000.0, 5100.0, far));
    read.add(0x200, AreaBody::Map(far, None), region);

    let mut interior = archetype(9);
    interior.class = ArchetypeClass::Interior;
    let types = MapTypes {
        name: 3,
        archetypes: vec![archetype(7), interior],
    };
    let inside = LightOwner::InteriorEntity {
        archetype: 1,
        entity: 0,
        pose: pose_at([5.0, 0.0, 0.0]),
    };
    let of_a_set = LightOwner::EntitySetEntity {
        archetype: 1,
        set: 0,
        entity: 0,
        pose: pose_at([0.0, 3.0, 0.0]),
    };
    let of_types = MapLights {
        effects: vec![
            (
                LightOwner::Archetype(0),
                effect([0.0, 2.0, 0.0], [0, 255, 0]),
            ),
            (inside, effect([1.0, 0.0, 0.0], [0, 0, 255])),
            (of_a_set, effect([0.0; 3], [9, 9, 9])),
        ],
        other_extensions: 0,
    };
    // A body read twice states its archetypes and its interiors once.
    for _ in 0..2 {
        let body = MapBody::Types(types.clone());
        read.add(0x300, AreaBody::Map(body, Some(of_types.clone())), region);
    }
    let nodes = vec![node(0, [0.0, 0.0, 1.0], 0), node(1, [10.0, 0.0, 1.0], 1)];
    read.add(0x400, AreaBody::Lanes(LaneBody::Nodes(nodes)), region);
    let links = vec![link(1), link(0)];
    read.add(0x500, AreaBody::Lanes(LaneBody::Links(links)), region);
    let junctions = vec![junction(1, 1)];
    read.add(
        0x600,
        AreaBody::Lanes(LaneBody::Junctions(junctions)),
        region,
    );
    read.finish()
}

/// Whether `lights` are those `wanted`, each at its place to a millimetre.
fn are(lights: &[Light], wanted: &[([f32; 2], [u8; 3])]) -> bool {
    lights.len() == wanted.len()
        && (lights.iter().zip(wanted)).all(|(l, (at, colour))| {
            (0..2).all(|i| (l.at[i] - at[i]).abs() < 1e-3) && l.colour == *colour && l.range == 12.0
        })
}

#[test]
fn a_fill_places_each_archetype_s_lights_at_its_entities_and_an_interior_s_through_its_instance() {
    let filled = gathered(None);
    assert_eq!((filled.index.maps.len(), filled.index.rows.len()), (2, 4));
    assert_eq!(filled.boxes.len(), 2);
    assert_eq!(filled.boxes[&7], [[0.0; 3], [1.0; 3]]);
    let wanted = [
        // The light of the entity's own, read with its map.
        ([21.0, 20.0], [255, 0, 0]),
        // Archetype 7's, at its first entity.
        ([10.0, 12.0], [0, 255, 0]),
        // The interior's: its entity at (5, 0) holds a light at (1, 0), and the instance turns
        // (6, 0) to (0, 6); then the entity set's, at (0, 3) turned to (-3, 0).
        ([100.0, 106.0], [0, 0, 255]),
        ([97.0, 100.0], [9, 9, 9]),
        // Archetype 7's again, at its entity of the other map.
        ([5050.0, 5052.0], [0, 255, 0]),
    ];
    assert!(are(&filled.lights, &wanted), "{:?}", filled.lights);
    assert_eq!(filled.lane_points, [[0.0, 0.0], [10.0, 0.0]]);
    assert_eq!(filled.lane_segments, [[[0.0, 0.0], [10.0, 0.0]]]);
    assert_eq!(filled.lane_signals, [[10.0, 0.0]]);

    // A region keeps the maps whose streaming box meets it, and every archetype and lane list.
    let part = gathered(Some(&([0.0, 0.0], [300.0, 300.0])));
    assert_eq!((part.index.maps.len(), part.index.rows.len()), (1, 3));
    assert!(are(&part.lights, &wanted[..4]), "{:?}", part.lights);
    assert_eq!(part.lane_segments.len(), 1);
}

#[test]
fn what_a_fill_read_is_kept_by_the_archive_s_path_size_and_time_and_read_again() {
    let dir = tempfile::tempdir().expect("a folder");
    let archive = dir.path().join("zq_levels.rpf");
    std::fs::write(&archive, [0u8; 64]).expect("a made-up archive");
    let filled = gathered(None);
    let kept = fill::Kept::of(dir.path(), &archive).expect("the archive states its size and time");
    assert_eq!(kept.read(), None, "nothing is kept yet");
    kept.keep(&filled).expect("kept");
    assert_eq!(kept.read().as_ref(), Some(&filled));
    assert_eq!(
        fill::Kept::of(dir.path(), &archive).expect("stated").read(),
        Some(filled),
        "another session reads it"
    );

    // An archive at another path, and the same archive once it has grown, have no fill kept.
    let other = dir.path().join("zq_other.rpf");
    std::fs::write(&other, [0u8; 64]).expect("written");
    assert_eq!(
        fill::Kept::of(dir.path(), &other).expect("stated").read(),
        None
    );
    std::fs::write(&archive, [0u8; 65]).expect("grown");
    assert_eq!(
        fill::Kept::of(dir.path(), &archive).expect("stated").read(),
        None
    );
    assert!(fill::Kept::of(dir.path(), &dir.path().join("zq_none.rpf")).is_none());
}

#[test]
fn a_kept_fill_cut_short_or_whose_rows_are_no_rows_of_its_index_is_refused() {
    let filled = gathered(None);
    let mut bytes = Vec::new();
    filled.write(&mut bytes).expect("written");
    assert_eq!(
        Filled::read(&mut bytes.as_slice()).expect("it reads"),
        filled
    );
    assert!(Filled::read(&mut &bytes[..bytes.len() - 1]).is_err());
    assert!(Filled::read(&mut &bytes[..4]).is_err());
    // The first map is 56 bytes after the count of maps, and its last eight say where its rows end.
    let mut wrong = bytes.clone();
    wrong[8 + 48..8 + 56].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Filled::read(&mut wrong.as_slice()).is_err());
    // A count of maps the bytes do not bear out.
    let mut wrong = bytes.clone();
    wrong[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(Filled::read(&mut wrong.as_slice()).is_err());
}

#[test]
fn an_area_finds_the_entity_under_the_pointer_and_the_map_that_places_it() {
    let area = Area::new(PathBuf::from("a/levels.rpf"), gathered(None));
    assert!(area.holds(Path::new("a/levels.rpf/0x100.rbm")));
    assert!(!area.holds(Path::new("b/levels.rpf/0x100.rbm")));
    assert_eq!(area.pick([10.4, 10.0], 1.0), Some(0));
    assert_eq!(area.pick([15.0, 15.0], 1.0), None);
    assert_eq!(area.pick([14.0, 14.0], 30.0), Some(0), "the nearest of two");
    assert_eq!(area.pick([5049.0, 5050.0], 2.0), Some(3));
    assert_eq!(area.map_of(0).map(|m| m.offset), Some(0x100));
    assert_eq!(area.map_of(2).map(|m| m.offset), Some(0x100));
    assert_eq!(area.map_of(3).map(|m| m.offset), Some(0x200));
    assert_eq!(area.map_of(4).map(|m| m.offset), None);
    // Every row here is an HD with no parent: three cells of 64 m, and two of 4096 m.
    assert_eq!((area.full.cells(), area.lower.cells()), (3, 0));
    assert_eq!((area.far[0].cells(), area.far[1].cells()), (2, 2));
    assert_eq!(area.segments.cells(), 1);
    assert!(is_full(0) && is_full(5) && !is_full(1) && !is_full(6));
    assert!(archetype_name(&area.filled.index.rows[0]).starts_with("0x"));
}

#[test]
fn an_interior_s_lights_stand_at_each_instance_of_it_the_map_shown_places() {
    let mut world = World::default();
    let mut interior = archetype(9);
    interior.class = ArchetypeClass::Interior;
    let types = MapTypes {
        name: 1,
        archetypes: vec![interior],
    };
    let inside = LightOwner::InteriorEntity {
        archetype: 0,
        entity: 0,
        pose: pose_at([5.0, 0.0, 0.0]),
    };
    let found = MapLights {
        effects: vec![(inside, effect([1.0, 0.0, 0.0], [0, 0, 255]))],
        other_extensions: 0,
    };
    world.keep_archetype_lights(&types, &found);
    let version = world.version;
    world.keep_archetype_lights(&types, &found);
    assert_eq!(
        world.version, version,
        "a body read twice adds its lights once"
    );

    // The first entity places the interior; the second names the same word and places none.
    let mut instance = entity(9, [100.0, 100.0, 0.0]);
    instance.interior = true;
    let map = map_of(1, 0.0, 200.0, vec![instance, entity(9, [0.0, 0.0, 0.0])]);
    let lights = world.place(Path::new("a/0x10.rbm"), &map);
    let blue = Light {
        at: [106.0, 100.0],
        colour: [0, 0, 255],
        range: 12.0,
    };
    assert_eq!(lights, [blue]);
}

#[test]
fn a_region_reads_the_maps_that_meet_it_once_a_pass_has_kept_where_every_body_is() {
    use fill::{bodies_of, Extent};
    // A pass over a near map, a far one, a map that places nothing, archetypes and a lane list;
    // a pass for a region keeps where the maps outside it are too.
    let passed = |region: Option<&fill::Region>| {
        let map = |name, lo, hi, entities| {
            AreaBody::Map(MapBody::Data(map_of(name, lo, hi, entities)), None)
        };
        let mut read = fill::Gathered::default();
        read.add(
            0x100,
            map(1, 0.0, 200.0, vec![entity(7, [10.0, 10.0, 0.0])]),
            region,
        );
        read.add(
            0x200,
            map(2, 5000.0, 5100.0, vec![entity(7, [5050.0, 5050.0, 0.0])]),
            region,
        );
        read.add(0x280, map(4, 0.0, 200.0, Vec::new()), region);
        let types = MapTypes {
            name: 3,
            archetypes: vec![archetype(7)],
        };
        read.add(0x300, AreaBody::Map(MapBody::Types(types), None), region);
        let nodes = vec![node(0, [0.0, 0.0, 1.0], 0)];
        read.add(0x400, AreaBody::Lanes(LaneBody::Nodes(nodes)), region);
        read.bodies().to_vec()
    };
    let near: fill::Region = ([0.0, 0.0], [300.0, 300.0]);
    let bodies = passed(None);
    assert_eq!(bodies, passed(Some(&near)));
    let starts: Vec<u64> = bodies.iter().map(|(offset, _)| *offset).collect();
    assert_eq!(
        starts,
        [0x100, 0x200, 0x300, 0x400],
        "a map that places nothing is no body of a fill"
    );
    assert!(matches!(bodies[0].1, Extent::Map(b) if b.max[0] == 200.0));
    assert_eq!(bodies[2].1, Extent::Everywhere);

    assert_eq!(bodies_of(&bodies, &near), [0x100, 0x300, 0x400]);
    assert_eq!(
        bodies_of(&bodies, &([5090.0, 5090.0], [6000.0, 6000.0])),
        [0x200, 0x300, 0x400]
    );
    assert_eq!(
        bodies_of(&bodies, &([300.0, 300.0], [4000.0, 4000.0])),
        [0x300, 0x400]
    );
    assert_eq!(bodies_of(&bodies, &([-1e6, -1e6], [1e6, 1e6])), starts);

    // The bodies are kept by the archive's path, size and time, as a fill is.
    let dir = tempfile::tempdir().expect("a folder");
    let archive = dir.path().join("zq_levels.rpf");
    std::fs::write(&archive, [0u8; 64]).expect("a made-up archive");
    let kept = fill::Kept::of(dir.path(), &archive).expect("the archive states its size and time");
    assert_eq!(kept.read_bodies(), None, "nothing is kept yet");
    kept.keep_bodies(&bodies).expect("kept");
    assert_eq!(kept.read_bodies().as_ref(), Some(&bodies));
    assert_eq!(kept.read(), None, "the bodies are no fill");
    std::fs::write(&archive, [0u8; 65]).expect("grown");
    let grown = fill::Kept::of(dir.path(), &archive).expect("stated");
    assert_eq!(grown.read_bodies(), None);
}
