//! What a map of GTA V states beside its entities, in the contract's terms: its far lights, its
//! box occluders and its occlude models.
//!
//! ⚠ The instance lists of a map, its grass and its props, are counted and not laid out: `or_io`
//! types no member of a map's `instancedData`, so what one instance of a list states is read by
//! nothing in the tree.

use crate::field;
use crate::keys;
use crate::map_data::{DistantLodLight, LodLight};
use crate::MapData;
use glam::Vec3;
use or_io::literal_string_hash;
use or_io::meta::{MetaInstance, Value};
use or_world::model::{Light, LightKind, OccluderBox, OccluderMesh, Occluders};

const I_COS_Z: u32 = literal_string_hash(b"iCosZ");
const I_SIN_Z: u32 = literal_string_hash(b"iSinZ");
const DATA_SIZE: u32 = literal_string_hash(b"dataSize");
const VERTS: u32 = literal_string_hash(b"verts");
const NUM_VERTS_IN_BYTES: u32 = literal_string_hash(b"numVertsInBytes");
const NUM_TRIS: u32 = literal_string_hash(b"numTris");

/// The bits of `numTris` that count triangles: the bit above them is set in every model the installs hold.
const TRIANGLES: u32 = 0x7FFF;
/// The bytes of one point of an occlude model: three floats.
const POINT: usize = 12;
/// The length an occluder's number counts in: a quarter of a metre.
const QUARTER: f32 = 0.25;
/// What a far light's cone bytes count in, in radians: the half turn over the most a byte holds.
const CONE: f32 = std::f32::consts::PI / 255.0;
/// The hours of `timeAndStateFlags`, a bit an hour.
const HOURS: u32 = (1 << 24) - 1;

/// What a map states beside its entities.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Beside {
    pub lights: Vec<Light>,
    pub occluders: Occluders,
    /// Far lights the map states that gave no light: their type is none the contract has, or the map that states where they stand is absent or holds fewer.
    pub lights_unread: u32,
    /// Occlude models the map states whose bytes are no points and triangles of them.
    pub occluders_unread: u32,
    /// The instance lists the map states, its grass and its props, none of which is laid out.
    pub lists: u32,
}

impl Beside {
    /// The bytes it holds.
    pub fn bytes(&self) -> u64 {
        let of = |len: usize, each: usize| (len * each) as u64;
        let meshes = self.occluders.meshes.iter().map(|mesh| {
            of(mesh.vertices.len(), std::mem::size_of::<Vec3>())
                + of(mesh.indices.len(), std::mem::size_of::<u32>())
        });
        of(self.lights.len(), std::mem::size_of::<Light>())
            + of(
                self.occluders.boxes.len(),
                std::mem::size_of::<OccluderBox>(),
            )
            + meshes.sum::<u64>()
    }
}

/// One box occluder as the rows that take the cube of side 1 about the origin to it.
// ⚠ The two numbers of its turn are stored the other way round from their names: the box's own x
// runs along (iSinZ, iCosZ) in the world, and a box that states neither is unturned.
fn box_of(inst: &MetaInstance) -> OccluderBox {
    let number = |key| field::i32_at(inst, key).unwrap_or(0) as f32;
    let quarters = |key| number(key) * QUARTER;
    let centre = [keys::I_CENTER_X, keys::I_CENTER_Y, keys::I_CENTER_Z].map(quarters);
    let size = [keys::I_LENGTH, keys::I_WIDTH, keys::I_HEIGHT].map(quarters);
    let (along, across) = match glam::Vec2::new(number(I_SIN_Z), number(I_COS_Z)).try_normalize() {
        Some(turn) => (turn, turn.perp()),
        None => (glam::Vec2::X, glam::Vec2::Y),
    };
    OccluderBox {
        rows: [
            [along.x * size[0], across.x * size[1], 0.0, centre[0]],
            [along.y * size[0], across.y * size[1], 0.0, centre[1]],
            [0.0, 0.0, size[2], centre[2]],
        ],
    }
}

/// One occlude model as a mesh; `None` where its bytes are no whole points followed by whole triangles of them.
// ⚠ `verts` is one run: `numVertsInBytes` bytes of points, three floats each, then a byte an
// index to the end of `dataSize`.
fn mesh_of(inst: &MetaInstance) -> Option<OccluderMesh> {
    let Some(Value::ByteArray(bytes)) = field::get(inst, VERTS) else {
        return None;
    };
    let stated = field::u32_at(inst, DATA_SIZE)? as usize;
    let points = field::u32_at(inst, NUM_VERTS_IN_BYTES)? as usize;
    let triangles = (field::u32_at(inst, NUM_TRIS)? & TRIANGLES) as usize;
    let (points, indices) = bytes.get(..stated)?.split_at_checked(points)?;
    if !points.len().is_multiple_of(POINT) || indices.len() != 3 * triangles {
        return None;
    }
    let number = |at: &[u8]| Some(f32::from_le_bytes(at.try_into().ok()?));
    let point = |at: &[u8; POINT]| {
        Some(Vec3::new(
            number(&at[0..4])?,
            number(&at[4..8])?,
            number(&at[8..12])?,
        ))
    };
    let vertices: Option<Vec<Vec3>> = points.as_chunks::<POINT>().0.iter().map(point).collect();
    let vertices = vertices?;
    if indices
        .iter()
        .any(|index| usize::from(*index) >= vertices.len())
    {
        return None;
    }
    Some(OccluderMesh {
        vertices,
        indices: indices.iter().map(|index| u32::from(*index)).collect(),
    })
}

/// One far light, which a map states in two halves: everything but where it stands and its colour in `far`, and those in `at`.
// ⚠ The scale of the cone's bytes and of a capsule's, and the reading of the colour and the
// intensity as a byte over the most it holds, are not measured against the game here. A far light
// has no distance of its own to fade by: it is drawn while its map streams.
fn light_of(far: &LodLight, at: &DistantLodLight) -> Option<Light> {
    let byte = |shift: u32| (at.rgbi >> shift & 0xFF) as f32 / 255.0;
    let kind = match far.time_and_state_flags >> 26 & 7 {
        1 => LightKind::Point,
        2 => LightKind::Spot {
            inner: f32::from(far.cone_inner_angle) * CONE,
            outer: f32::from(far.cone_outer_angle_or_cap_ext) * CONE,
        },
        4 => LightKind::Capsule {
            length: f32::from(far.cone_outer_angle_or_cap_ext) * QUARTER,
        },
        _ => return None,
    };
    Some(Light {
        kind,
        position: at.position,
        direction: far.direction.normalize_or_zero(),
        colour: [byte(16), byte(8), byte(0)],
        intensity: byte(24),
        range: far.falloff,
        fade: (f32::MAX, f32::MAX),
        hours: far.time_and_state_flags & HOURS,
    })
}

/// What `map` states beside its entities; `parent` is the map it names as its parent, read, where the mount holds it.
// ⚠ Far light `i` of a map stands where the map it names as its parent says light `i` does, so a
// map of far lights whose parent is absent states none, and the parent states none by itself.
pub(crate) fn beside(map: &MapData<'_>, parent: Option<&MapData<'_>>) -> Beside {
    let root = map.instance();
    let mut out = Beside {
        lists: (map.grass_batches().count() + map.prop_batch_count()) as u32,
        ..Beside::default()
    };
    (out.occluders.boxes).extend(field::structs_at(root, keys::BOX_OCCLUDERS).map(box_of));
    for model in field::structs_at(root, keys::OCCLUDE_MODELS) {
        match mesh_of(model) {
            Some(mesh) => out.occluders.meshes.push(mesh),
            None => out.occluders_unread += 1,
        }
    }
    if let Some(far) = map.lod_lights().filter(|far| !far.is_empty()) {
        let stand = parent.and_then(|parent| parent.distant_lod_lights());
        for at in 0..far.len() {
            let stands = stand.as_ref().and_then(|stand| stand.get(at));
            match far
                .get(at)
                .zip(stands)
                .and_then(|(far, at)| light_of(&far, &at))
            {
                Some(light) => out.lights.push(light),
                None => out.lights_unread += 1,
            }
        }
    }
    out
}
