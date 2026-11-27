//! A GTA V install built by hand as the conformance kit's scene states one: a parent and its two
//! children in two maps, a map by the clock, a map only a request lets in, a map that places an
//! interior over a level floor, and a drawable for each archetype that draws.

#![allow(dead_code)]

use glam::Vec3;
use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexLayout};
use or_io::decl::{File, Gen8, Hash32, Shared};
use or_io::gta5::drawable::gen8;
use or_io::gta5::{
    GrmGeometry, GrmModel, GrmShaderGroup, GtaDrawable, RmcDrawable, RmcLod, SpdAABB,
};
use or_world::contract::WorldSpec;
use or_world::kit::{Fixture, Grounded, HeldBack, InteriorAt, Ladder, Lit, Scene, Timed};
use or_world::model::SwitchId;
use or_world::state::{flags as group_flags, NOT_BY_DEFAULT};
use or_world_gta5::{Gta5Title, LEGACY};

use crate::forest_fixture::{
    arch, ent, install, manifest_bytes, map_around, map_file, types_file, Arch, Ent, Files, MapOf,
    LOD,
};
use crate::world_fixture::{cube, hash, level_square, mesh_bound_bytes};

/// Where the parent of the ladder draws alone: past its children's 100 m.
pub const FAR_EYE: Vec3 = Vec3::new(240.0, 0.0, 0.0);
/// Where its two children draw in its place.
pub const NEAR_EYE: Vec3 = Vec3::new(70.0, 0.0, 0.0);
pub const NIGHT_EYE: Vec3 = Vec3::new(5000.0, 0.0, 0.0);
pub const EVENT_EYE: Vec3 = Vec3::new(0.0, 5000.0, 0.0);
pub const YARD_EYE: Vec3 = Vec3::new(-5000.0, 10.0, 5.0);
pub const NOWHERE: Vec3 = Vec3::new(-50_000.0, -50_000.0, 0.0);
/// The height of the floor under the yard.
pub const FLOOR: f32 = 3.0;
/// The hours the night map streams in: 20 h to 5 h.
pub const NIGHT_HOURS: u32 = 0b1111 << 20 | 0b11_1111;
/// The archetypes that draw, each a drawable of its own name.
pub const DRAWN: [&str; 7] = [
    "lod_p",
    "hd_a",
    "hd_b",
    "chair",
    "table",
    "night_prop",
    "event_prop",
];

/// How far each level of a drawable of [`drawable_bytes`] draws, in metres.
pub const LOD_DISTANCES: [f32; 4] = [20.0, 60.0, 180.0, 540.0];

/// The weather every check is made under.
pub fn weather() -> u64 {
    u64::from(hash("CLEAR"))
}

/// A Legacy drawable of one triangle a level, at `levels` levels of detail, as the bytes of a file of its own.
pub fn drawable_bytes(name: &str, levels: usize) -> Vec<u8> {
    let channels = [
        (GrcFvfChannel::Position, GrcDataSize::Float3),
        (GrcFvfChannel::Normal, GrcDataSize::Float3),
        (GrcFvfChannel::Tex0, GrcDataSize::Float2),
    ];
    let mut fvf = gen8::GrcFvf::default();
    for (channel, size) in channels {
        fvf.fvf |= 1 << channel as u32;
        fvf.channel_sizes |= (size as u64) << (channel as u64 * 4);
    }
    fvf.channel_count = channels.len() as u8;
    fvf.fvf_size = VertexLayout::of(fvf.fvf, fvf.channel_sizes).packed_stride() as u8;
    let mut data = Vec::new();
    for corner in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for number in corner
            .into_iter()
            .chain([0.0, 0.0, 1.0, corner[0], corner[1]])
        {
            data.extend_from_slice(&number.to_le_bytes());
        }
    }
    let mut geometry = GrmGeometry::<Gen8> {
        primitive_type: 3,
        vertex_buffer: Box::new(gen8::GrcVertexBuffer {
            data,
            fvf: Some(Box::new(fvf)),
            lock_ptr: Shared::Same,
            ..Default::default()
        }),
        index_buffer: Box::new(gen8::GrcIndexBuffer {
            indices: vec![0, 1, 2],
            ..Default::default()
        }),
        vertex_data: Shared::Same,
        ..Default::default()
    };
    geometry.take_counts_from_runs();
    let mut drawable = RmcDrawable::<Gen8> {
        name: name.into(),
        shader_group: Some(Box::new(GrmShaderGroup {
            shaders: vec![Box::new(gen8::GrcInstanceData {
                basis_hash: Hash32::of("default"),
                ..Default::default()
            })],
            ..Default::default()
        })),
        ..Default::default()
    };
    drawable.lod_group.lod_distances = LOD_DISTANCES;
    for lod in drawable.lod_group.lods.iter_mut().take(levels) {
        *lod = RmcLod {
            models: vec![Box::new(GrmModel {
                aabbs: vec![SpdAABB::default(); or_io::codec::mesh::aabb_slots(1)],
                shader_indices: vec![0],
                geometries: vec![Box::new(geometry.clone())],
                ..Default::default()
            })],
            ..Default::default()
        }
        .into();
    }
    let whole = GtaDrawable {
        base: drawable,
        ..Default::default()
    };
    whole.to_bytes().expect("a drawable")
}

/// A parent at the origin drawn to 1,000 m that states two children and 100 m for them.
pub fn parent() -> Ent {
    Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 2,
        ..ent("lod_p", [0.0; 3])
    }
}

/// The two children, 300 m apart, each naming entity 0 of the parent map as its parent.
pub fn children() -> Vec<Ent> {
    let child = |archetype, x| Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent(archetype, [x, 0.0, 0.0])
    };
    vec![child("hd_a", 150.0), child("hd_b", -150.0)]
}

/// A map of one entity of `archetype` at `at`, held back until something asks.
fn held_back(archetype: &'static str, at: [f32; 3]) -> MapOf {
    MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(cube(at.map(|v| v - 10.0), 20.0), vec![ent(archetype, at)])
    }
}

/// The files of the one archive of the scene.
pub fn files() -> Files {
    let lod = map_around(cube([-200.0; 3], 400.0), vec![parent()]);
    let hd = MapOf {
        parent: "m_lod",
        ..map_around(cube([-200.0; 3], 400.0), children())
    };
    let shop = Ent {
        interior: true,
        lod_dist: 100.0,
        ..ent("shop", [-5000.0, 0.0, 5.0])
    };
    let yard = map_around(cube([-5010.0, -10.0, 0.0], 20.0), vec![shop]);
    let mut types: Vec<Arch> = DRAWN.iter().map(|name| arch(name, 500.0)).collect();
    types.push(Arch {
        interior: Some(vec![
            ent("chair", [1.0, 0.0, 0.0]),
            ent("table", [2.0, 0.0, 0.0]),
        ]),
        ..arch("shop", 100.0)
    });
    let mut files = vec![
        map_file("m_lod", &lod),
        map_file("m_hd", &hd),
        map_file("m_night", &held_back("night_prop", [5000.0, 0.0, 0.0])),
        map_file("m_event", &held_back("event_prop", [0.0, 5000.0, 0.0])),
        map_file("m_yard", &yard),
        types_file("scene", &types),
        (
            "levels/_manifest.ymf".to_owned(),
            manifest_bytes(&[("m_night", group_flags::TIMED, NIGHT_HOURS, &[])]),
        ),
        (
            "levels/m_yard_0.ybn".to_owned(),
            mesh_bound_bytes(level_square([-5050.0, -50.0, FLOOR], 100.0, 0x21)),
        ),
    ];
    for name in DRAWN {
        let levels = if name == "lod_p" { 1 } else { 2 };
        files.push((format!("models/{name}.ydr"), drawable_bytes(name, levels)));
    }
    files
}

/// The scene the kit checks the install of [`files`] by.
pub fn scene() -> Scene {
    Scene {
        weather: weather(),
        nowhere: NOWHERE,
        ladder: Ladder {
            parent_map: u64::from(hash("m_lod")),
            child_map: u64::from(hash("m_hd")),
            far: FAR_EYE,
            near: NEAR_EYE,
        },
        timed: Timed {
            map: u64::from(hash("m_night")),
            eye: NIGHT_EYE,
            on: 22.0,
            off: 12.0,
        },
        held: HeldBack {
            map: u64::from(hash("m_event")),
            eye: EVENT_EYE,
            switch: SwitchId(hash("m_event")),
        },
        interior: InteriorAt {
            eye: YARD_EYE,
            instances: 2,
        },
        // No map of GTA V is read with a list yet.
        list: None,
        // Nor with its lights or its occluders.
        lit: Lit {
            eye: YARD_EYE,
            lights: 0,
            occluders: 0,
        },
        placed: (u64::from(hash("m_yard")), 1),
        ground: Grounded {
            at: Vec3::new(-5000.0, 0.0, FLOOR + 5.0),
            height: FLOOR,
        },
    }
}

/// The scene written to a scratch folder, which lives as long as the fixture.
pub fn fixture() -> Fixture {
    let dir = install(&[("x64a.rpf", files())]);
    Fixture {
        title: Box::new(Gta5Title),
        world: WorldSpec::of(LEGACY, dir.path()),
        scene: scene(),
        edit: None,
        keeps: Some(Box::new(dir)),
        executable: None,
    }
}
