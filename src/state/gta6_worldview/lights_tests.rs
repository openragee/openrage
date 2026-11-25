use super::*;
use lights_lod::Distant;
use or_io::gta6::map::light_ext::{Light, LightKind};

const ALL_HOURS: u32 = 0x00FF_FFFF;

const REACH: LodReach = LodReach {
    range: [500.0; 3],
    fadeout: [100.0; 3],
};

fn placed_at(x: f32) -> Lamp {
    Lamp {
        light: Light {
            kind: LightKind::Point,
            position: [x, 0.0, 0.0],
            direction: [0.0, 0.0, -1.0],
            colour: [1.0; 3],
            intensity: 10.0,
            radius: 5.0,
            falloff_exponent: 8.0,
            hours: ALL_HOURS,
            flags: 0,
            cone_inner: 0.0,
            cone_outer: 0.0,
            length: 0.0,
            corona: [0.0; 3],
        },
        reach: None,
    }
}

fn lod_map(parent: u32, falloff: &[f32]) -> MapLights {
    let n = falloff.len();
    MapLights {
        parent,
        placed: Vec::new(),
        inside: Vec::new(),
        lod: MapLod {
            direction: vec![[0.0, 0.0, -1.0]; n],
            falloff: falloff.to_vec(),
            falloff_exponent: vec![1.0; n],
            cone_inner: vec![0; n],
            cone_outer: vec![0; n],
            ..MapLod::default()
        },
    }
}

/// A map of distant lights of type point at `xs`, which also places a light of its own at 200.
fn distant_map(xs: &[f32]) -> MapLights {
    let light = |x: &f32| Distant {
        position: [*x, 0.0, 0.0],
        rgb: [255, 0, 51],
        intensity: 80.0,
        light_type: 1,
    };
    MapLights {
        lod: MapLod {
            distant: xs.iter().map(light).collect(),
            ..MapLod::default()
        },
        placed: vec![placed_at(200.0)],
        ..MapLights::default()
    }
}

#[test]
fn a_level_of_detail_light_stands_at_its_parents_distant_light_unless_a_placed_one_does() {
    let loaded = HashMap::from([
        (7, lod_map(8, &[3.0, 12.0])),
        (8, distant_map(&[100.0, 200.0])),
    ]);
    let lamps = lamps_of(&loaded, &REACH, &mut HashMap::new());
    let mut xs: Vec<(f32, bool)> = (lamps.iter())
        .map(|l| (l.light.position[0], l.reach.is_some()))
        .collect();
    xs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        xs,
        [(100.0, true), (200.0, false)],
        "the placed light at 200 stands for its copy"
    );
    let copy = lamps.iter().find(|l| l.reach.is_some()).map(|l| l.light);
    assert_eq!(copy.map(|l| (l.radius, l.hours)), Some((3.0, ALL_HOURS)));
}

#[test]
fn the_lists_kept_between_changes_give_the_lamps_made_afresh() {
    let mut loaded = HashMap::from([
        (7, lod_map(8, &[3.0, 12.0])),
        (8, distant_map(&[100.0, 200.0])),
    ]);
    let mut lists = HashMap::new();
    let positions =
        |lamps: Vec<Lamp>| -> Vec<[f32; 3]> { lamps.iter().map(|l| l.light.position).collect() };
    let mut check = |loaded: &HashMap<u32, MapLights>| {
        let kept = positions(lamps_of(loaded, &REACH, &mut lists));
        assert_eq!(
            kept,
            positions(lamps_of(loaded, &REACH, &mut HashMap::new()))
        );
        kept.len()
    };
    assert_eq!(check(&loaded), 2);
    // The parent goes: its child's list draws nothing; it comes back: the list is made again.
    let parent = loaded.remove(&8);
    assert_eq!(check(&loaded), 0);
    loaded.extend(parent.map(|p| (8, p)));
    assert_eq!(check(&loaded), 2);
}

#[test]
fn lists_that_do_not_pair_light_for_light_draw_nothing() {
    let alone = HashMap::from([(7, lod_map(8, &[3.0]))]);
    assert!(lamps_of(&alone, &REACH, &mut HashMap::new()).is_empty());
    let uneven = HashMap::from([(7, lod_map(8, &[3.0])), (8, distant_map(&[1.0, 2.0]))]);
    assert_eq!(
        lamps_of(&uneven, &REACH, &mut HashMap::new()).len(),
        1,
        "the placed light alone"
    );
}

#[test]
fn a_map_of_light_lists_is_wanted_though_it_places_nothing_and_a_held_map_is_not() {
    let row = |name: u32, content_flags: u32, held: u32| super::super::index::MapRow {
        name,
        content_flags,
        held,
        modes: 1,
        boxes: [
            -100.0, -100.0, -100.0, 100.0, 100.0, 100.0, -1.0, -1.0, -1.0, 1.0, 1.0, 1.0,
        ],
        ..bytemuck::Zeroable::zeroed()
    };
    let mut index = WorldIndex::default();
    index.maps = vec![
        row(1, LOD_LIGHTS, HELD_EMPTY),
        row(2, DISTANT_LIGHTS, HELD_EMPTY),
        row(3, LOD_LIGHTS, 2),
        row(4, HIGH_DETAIL, HELD_EMPTY),
        row(5, HIGH_DETAIL, 0),
    ];
    let mut lights = Lights::new(Arc::new(index));
    lights.rewant(Vec3::ZERO);
    let mut wanted: Vec<u32> = lights.wanted.keys().copied().collect();
    wanted.sort_unstable();
    assert_eq!(
        wanted,
        [1, 2, 5],
        "a script-managed list follows its group; an empty map of entities stays out"
    );
}

#[test]
fn an_interior_s_lamp_lights_while_its_contents_draw_and_its_room_is_seen_alone() {
    let mut lights = Lights::new(Arc::new(WorldIndex::default()));
    let map = MapLights {
        inside: vec![(placed_at(1.0), [4, 2])],
        ..MapLights::default()
    };
    lights.loaded.insert(7, map);
    lights.relamp();
    let view = View {
        eye: Vec3::ZERO,
        forward: Vec3::X,
        tan_half: 0.5,
    };
    let mut drawn = |rooms: &RoomsSeen| lights.lights(&view, (12.0, 1.0, MOST), rooms).lights;
    assert_eq!(
        drawn(&RoomsSeen::new()),
        0,
        "an interior whose contents do not draw"
    );
    assert_eq!(
        drawn(&RoomsSeen::from([((7, 4), vec![true, true, false])])),
        0
    );
    assert_eq!(
        drawn(&RoomsSeen::from([((7, 4), vec![false, false, true])])),
        1
    );
    assert_eq!(
        drawn(&RoomsSeen::from([((8, 4), vec![true, true, false])])),
        0,
        "another interior's contents draw"
    );
}

#[test]
fn the_lights_of_a_body_that_is_no_map_are_none() {
    let mut model = lights_lod::tests::model(&[], &[]);
    model.data_blocks[0].data_type = 1;
    assert_eq!(lights_of(&model, &WorldIndex::default()), None);
}
