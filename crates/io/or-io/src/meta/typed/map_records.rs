//! What a map holds besides entities, typed: the parked vehicles, box occluders, occlude models, time
//! cycle modifiers and far lights of a [`CMapData`], each with the members the installs state for it
//! and no other.
//!
//! ⚠ **The far lights are two families in two maps.** A map of far lights holds `LODLightsSOA`,
//! eight arrays under one subscript, and names as its `parent` the map that holds
//! `DistantLODLightsSOA`, two arrays under the same subscript: light `i` has its position and colour
//! in the parent and everything else in the child, and no shipped map holds both. The engine trusts
//! every length, so a save refuses a family whose arrays differ ([`broken`]) and [`save_far_lights`]
//! refuses a pair out of step, which the save of one map alone cannot see.
//!
//! ⚠ An occlude model's `verts` is a run of bytes behind a bare pointer, which the file's own type
//! table says `dataSize` counts: a save refuses a model whose two differ. A reader takes the run to
//! the end of its block, so the bytes are the model's whole block.
//!
//! ⚠ No class of these is declared for a file that states none (`world_types`): a record is added as
//! a copy of one the file holds, one built here saves only into a file whose own table states its
//! class, and a map made from nothing, whose `CMapData` states none of the six members that hold
//! them, refuses one.

use super::*;

meta_record! {
    /// `CCarGen`: a parked vehicle, where it stands and what may spawn there.
    CCarGen = "CCarGen" {
        position: Vec3 = "position",
        orient_x: f32 = "orientX",
        orient_y: f32 = "orientY",
        perpendicular_length: f32 = "perpendicularLength",
        car_model: HashedString = "carModel",
        flags: u32 = "flags",
        body_color_remap1: i32 = "bodyColorRemap1",
        body_color_remap2: i32 = "bodyColorRemap2",
        body_color_remap3: i32 = "bodyColorRemap3",
        body_color_remap4: i32 = "bodyColorRemap4",
        pop_group: HashedString = "popGroup",
        livery: i8 = "livery" => -1,
    }
}

meta_record! {
    /// `BoxOccluder`: a box that hides what is behind it, every member a signed 16-bit number.
    BoxOccluder = "BoxOccluder" {
        i_center_x: i16 = "iCenterX",
        i_center_y: i16 = "iCenterY",
        i_center_z: i16 = "iCenterZ",
        i_cos_z: i16 = "iCosZ",
        i_length: i16 = "iLength",
        i_width: i16 = "iWidth",
        i_height: i16 = "iHeight",
        i_sin_z: i16 = "iSinZ",
    }
}

meta_record! {
    /// `OccludeModel`: a mesh that hides what is behind it, its vertices and indices one run of bytes.
    OccludeModel = "OccludeModel" {
        bmin: Vec3 = "bmin",
        bmax: Vec3 = "bmax",
        data_size: u32 = "dataSize",
        verts: Vec<u8> = "verts",
        num_verts_in_bytes: u16 = "numVertsInBytes",
        num_tris: u16 = "numTris",
        flags: u32 = "flags",
    }
}

meta_record! {
    /// `CTimeCycleModifier`: a time cycle modifier applied inside a box of the map, between two hours.
    CTimeCycleModifier = "CTimeCycleModifier" {
        name: HashedString = "name",
        min_extents: Vec3 = "minExtents",
        max_extents: Vec3 = "maxExtents",
        percentage: f32 = "percentage",
        range: f32 = "range",
        start_hour: u32 = "startHour",
        end_hour: u32 = "endHour",
    }
}

meta_record! {
    /// `FloatXYZ`: three floats, the position or the direction of one far light.
    FloatXYZ = "FloatXYZ" {
        x: f32 = "x",
        y: f32 = "y",
        z: f32 = "z",
    }
}

meta_record! {
    /// `CLODLight`: the far lights of a map as eight arrays under one subscript; their positions are in the parent map.
    CLODLight = "CLODLight" {
        direction: Vec<FloatXYZ> = "direction",
        falloff: Vec<f32> = "falloff",
        falloff_exponent: Vec<f32> = "falloffExponent",
        time_and_state_flags: Vec<u32> = "timeAndStateFlags",
        hash: Vec<u32> = "hash",
        cone_inner_angle: Vec<u8> = "coneInnerAngle",
        cone_outer_angle_or_cap_ext: Vec<u8> = "coneOuterAngleOrCapExt",
        corona_intensity: Vec<u8> = "coronaIntensity",
    }
}

meta_record! {
    /// `CDistantLODLight`: the positions and colours of the far lights a child map describes, under its subscript.
    CDistantLODLight = "CDistantLODLight" {
        position: Vec<FloatXYZ> = "position",
        rgbi: Vec<u32> = "RGBI",
        num_street_lights: u16 = "numStreetLights",
        category: u16 = "category",
    }
}

/// The members of [`CMapData`] that hold these records, which the class declared for a map built from nothing does not state.
#[cfg(test)]
pub(super) const ROOT_MEMBERS: [&str; 6] = [
    "boxOccluders",
    "occludeModels",
    "timeCycleModifiers",
    "carGenerators",
    "LODLightsSOA",
    "DistantLODLightsSOA",
];

/// The members each record of a map types, by the class that states them.
pub(super) const MEMBERS: &[(&str, &[&str])] = &[
    ("CCarGen", CCarGen::MEMBERS),
    ("BoxOccluder", BoxOccluder::MEMBERS),
    ("OccludeModel", OccludeModel::MEMBERS),
    ("CTimeCycleModifier", CTimeCycleModifier::MEMBERS),
    ("FloatXYZ", FloatXYZ::MEMBERS),
    ("CLODLight", CLODLight::MEMBERS),
    ("CDistantLODLight", CDistantLODLight::MEMBERS),
];

impl CLODLight {
    /// Each array of the family with its length, in the order the class states them.
    fn lengths(&self) -> [(&'static str, usize); 8] {
        [
            ("direction", self.direction.len()),
            ("falloff", self.falloff.len()),
            ("falloffExponent", self.falloff_exponent.len()),
            ("timeAndStateFlags", self.time_and_state_flags.len()),
            ("hash", self.hash.len()),
            ("coneInnerAngle", self.cone_inner_angle.len()),
            (
                "coneOuterAngleOrCapExt",
                self.cone_outer_angle_or_cap_ext.len(),
            ),
            ("coronaIntensity", self.corona_intensity.len()),
        ]
    }

    /// How many far lights the family holds: the length of its first array, which a save holds the others to.
    pub fn len(&self) -> usize {
        self.direction.len()
    }

    /// Whether every array of the family is empty, as in a map that holds no far light.
    pub fn is_empty(&self) -> bool {
        self.lengths().iter().all(|(_, len)| *len == 0)
    }
}

impl CDistantLODLight {
    fn lengths(&self) -> [(&'static str, usize); 2] {
        [("position", self.position.len()), ("RGBI", self.rgbi.len())]
    }

    /// How many far lights the family holds: the length of its first array, which a save holds the other to.
    pub fn len(&self) -> usize {
        self.position.len()
    }

    /// Whether both arrays of the family are empty, as in a map that holds no far light.
    pub fn is_empty(&self) -> bool {
        self.lengths().iter().all(|(_, len)| *len == 0)
    }
}

/// A length of a map's records that the engine trusts and the map does not keep.
#[derive(Debug, thiserror::Error)]
pub(super) enum Broken {
    #[error("light array {member} holds {len} where its siblings hold {expected}")]
    FarLightArraysDiffer {
        member: &'static str,
        len: usize,
        expected: usize,
    },
    #[error("occlude model {model} states {stated} bytes of `verts` (dataSize) and holds {held}")]
    OccludeModelSizeDiffers {
        model: usize,
        stated: u32,
        held: usize,
    },
}

/// The first array of a family that is shorter than its longest.
fn ragged(lengths: &[(&'static str, usize)]) -> Option<Broken> {
    let expected = lengths.iter().map(|(_, len)| *len).max()?;
    let (member, len) = *lengths.iter().find(|(_, len)| *len != expected)?;
    Some(Broken::FarLightArraysDiffer {
        member,
        len,
        expected,
    })
}

/// The first length of `map` that misses what it counts: the far lights, the distant ones, then the occlude models.
pub(super) fn broken(map: &CMapData) -> Option<Broken> {
    let models = || {
        (map.occlude_models.iter().enumerate()).find_map(|(model, m)| {
            (m.data_size as usize != m.verts.len()).then_some(Broken::OccludeModelSizeDiffers {
                model,
                stated: m.data_size,
                held: m.verts.len(),
            })
        })
    };
    ragged(&map.lod_lights_soa.lengths())
        .or_else(|| ragged(&map.distant_lod_lights_soa.lengths()))
        .or_else(models)
}

/// Whether a record of `map` was built here, or an occlude model's bytes changed length, so no arrival layout has its shape.
pub(super) fn holds_a_record_built_here(map: &CMapData) -> bool {
    let points = |points: &[FloatXYZ]| points.iter().any(|p| built_here(&p.carry, "FloatXYZ"));
    let lights = &map.lod_lights_soa;
    let distant = &map.distant_lod_lights_soa;
    (map.car_generators.iter()).any(|r| built_here(&r.carry, "CCarGen"))
        || (map.box_occluders.iter()).any(|r| built_here(&r.carry, "BoxOccluder"))
        || (map.time_cycle_modifiers.iter()).any(|r| built_here(&r.carry, "CTimeCycleModifier"))
        || (map.occlude_models.iter()).any(|r| built_here(&r.carry, "OccludeModel") || resized(r))
        // ⚠ A map whose class states no far lights reads each family as one built here that holds nothing, and that one is none.
        || (built_here(&lights.carry, "CLODLight") && *lights != CLODLight::default())
        || (built_here(&distant.carry, "CDistantLODLight")
            && *distant != CDistantLODLight::default())
        || points(&lights.direction)
        || points(&distant.position)
}

/// Whether `model` holds another count of bytes than the tree it was read from, whose block the arrival layout sized.
fn resized(model: &OccludeModel) -> bool {
    let arrived = (model.carry.instance.as_deref())
        .and_then(|inst| field(inst, "verts"))
        .and_then(|verts| <Vec<u8> as Member>::read(verts).ok());
    arrived.is_some_and(|arrived| arrived.len() != model.verts.len())
}

/// The far lights of a map and of the map it names as its parent, out of step.
#[derive(Debug, thiserror::Error)]
enum OutOfStep {
    #[error("the map of far lights names the parent {named:#010X}, and the map given with it is {given:#010X}")]
    NotTheParent { named: u32, given: u32 },
    #[error("the map holds {lights} far lights and its parent {distant} distant ones: the two run under one subscript, a light to each")]
    CountsDiffer { lights: usize, distant: usize },
}

/// Saves a map of far lights and the map it names as its parent, refused where the two are out of step.
pub fn save_far_lights(lights: &CMapData, parent: &CMapData) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let (held, distant) = (
        lights.lod_lights_soa.len(),
        parent.distant_lod_lights_soa.len(),
    );
    let out_of_step = if lights.parent.hash != parent.name.hash {
        Some(OutOfStep::NotTheParent {
            named: lights.parent.hash,
            given: parent.name.hash,
        })
    } else {
        (held != distant).then_some(OutOfStep::CountsDiffer {
            lights: held,
            distant,
        })
    };
    if let Some(reason) = out_of_step {
        return Err(Error::Refused {
            ext: CMapData::EXTENSION,
            reason: crate::error::Reason::new(reason),
        });
    }
    Ok((lights.to_bytes()?, parent.to_bytes()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::typed::world_types::WORLD_TYPES;
    use crate::meta::{build_library, canonical, detect_binary_format};

    /// What the installs state for each record of a map, by name
    /// (`docs/reference/Crate_Reference.md`, the table of the members of `CMapData`).
    const STATED: &[(&str, &[&str])] = &[
        (
            "CCarGen",
            &[
                "bodyColorRemap1",
                "bodyColorRemap2",
                "bodyColorRemap3",
                "bodyColorRemap4",
                "carModel",
                "flags",
                "livery",
                "orientX",
                "orientY",
                "perpendicularLength",
                "popGroup",
                "position",
            ],
        ),
        (
            "BoxOccluder",
            &[
                "iCenterX", "iCenterY", "iCenterZ", "iCosZ", "iHeight", "iLength", "iSinZ",
                "iWidth",
            ],
        ),
        (
            "OccludeModel",
            &[
                "bmax",
                "bmin",
                "dataSize",
                "flags",
                "numTris",
                "numVertsInBytes",
                "verts",
            ],
        ),
        (
            "CTimeCycleModifier",
            &[
                "endHour",
                "maxExtents",
                "minExtents",
                "name",
                "percentage",
                "range",
                "startHour",
            ],
        ),
        ("FloatXYZ", &["x", "y", "z"]),
        (
            "CLODLight",
            &[
                "coneInnerAngle",
                "coneOuterAngleOrCapExt",
                "coronaIntensity",
                "direction",
                "falloff",
                "falloffExponent",
                "hash",
                "timeAndStateFlags",
            ],
        ),
        (
            "CDistantLODLight",
            &["RGBI", "category", "numStreetLights", "position"],
        ),
    ];

    /// The classes of a map with these records, each with the members [`STATED`] lists and one more, which no record
    /// types; `VERTS` stands where an occlude model states its bytes.
    const RECORD_TYPES: &str = r#"
  <structdef type="CCarGen">
    <Vector3 name="position"/>
    <float name="orientX"/>
    <float name="orientY"/>
    <float name="perpendicularLength"/>
    <string name="carModel" type="atHashString"/>
    <u32 name="flags"/>
    <s32 name="bodyColorRemap1"/>
    <s32 name="bodyColorRemap2"/>
    <s32 name="bodyColorRemap3"/>
    <s32 name="bodyColorRemap4"/>
    <string name="popGroup" type="atHashString"/>
    <s8 name="livery" init="-1"/>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="BoxOccluder">
    <s16 name="iCenterX"/>
    <s16 name="iCenterY"/>
    <s16 name="iCenterZ"/>
    <s16 name="iCosZ"/>
    <s16 name="iLength"/>
    <s16 name="iWidth"/>
    <s16 name="iHeight"/>
    <s16 name="iSinZ"/>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="OccludeModel">
    <Vector3 name="bmin"/>
    <Vector3 name="bmax"/>
    <u32 name="dataSize"/>
    VERTS
    <u16 name="numVertsInBytes"/>
    <u16 name="numTris"/>
    <u32 name="flags"/>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="CTimeCycleModifier">
    <string name="name" type="atHashString"/>
    <Vector3 name="minExtents"/>
    <Vector3 name="maxExtents"/>
    <float name="percentage"/>
    <float name="range"/>
    <u32 name="startHour"/>
    <u32 name="endHour"/>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="FloatXYZ">
    <float name="x"/>
    <float name="y"/>
    <float name="z"/>
  </structdef>
  <structdef type="CLODLight">
    <array name="direction" type="atArray"><struct type="FloatXYZ"/></array>
    <array name="falloff" type="atArray"><float/></array>
    <array name="falloffExponent" type="atArray"><float/></array>
    <array name="timeAndStateFlags" type="atArray"><u32/></array>
    <array name="hash" type="atArray"><u32/></array>
    <array name="coneInnerAngle" type="atArray"><u8/></array>
    <array name="coneOuterAngleOrCapExt" type="atArray"><u8/></array>
    <array name="coronaIntensity" type="atArray"><u8/></array>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="CDistantLODLight">
    <array name="position" type="atArray"><struct type="FloatXYZ"/></array>
    <array name="RGBI" type="atArray"><u32/></array>
    <u16 name="numStreetLights"/>
    <u16 name="category"/>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
</ParserSchema>"#;

    /// The members of a map that hold these records, in the order the installs state them about `physicsDictionaries`.
    const ROOT_BEFORE: &str = r#"<array name="boxOccluders" type="atArray"><struct type="BoxOccluder"/></array>
    <array name="occludeModels" type="atArray"><struct type="OccludeModel"/></array>
    "#;
    const ROOT_AFTER: &str = r#"
    <array name="timeCycleModifiers" type="atArray"><struct type="CTimeCycleModifier"/></array>
    <array name="carGenerators" type="atArray"><struct type="CCarGen"/></array>
    <struct name="LODLightsSOA" type="CLODLight"/>
    <struct name="DistantLODLightsSOA" type="CDistantLODLight"/>"#;

    /// ⚠ Only an RBM states a run of bytes behind a bare pointer, so the PSO built here states an occlude model without
    /// its bytes; no PSO of either install holds an occlude model.
    fn library(format: BinaryFormat) -> SchemaLibrary {
        let dictionaries = r#"<array name="physicsDictionaries" type="atArray"><string type="atHashString"/></array>"#;
        assert!(WORLD_TYPES.contains(dictionaries));
        let verts = match format {
            BinaryFormat::Rbm => {
                r#"<array name="verts" type="pointer" sizeVar="dataSize" align="16"><u8/></array>"#
            }
            _ => "",
        };
        let psc = WORLD_TYPES
            .replace(
                dictionaries,
                &format!("{ROOT_BEFORE}{dictionaries}{ROOT_AFTER}"),
            )
            .replace("</ParserSchema>", &RECORD_TYPES.replace("VERTS", verts));
        build_library([("map_records.psc", psc.as_bytes())]).expect("the classes of a map parse")
    }

    /// A `class` record of `library` with `members` set, every other member at its empty value.
    fn record(library: &SchemaLibrary, class: &str, members: &[(&str, Value)]) -> MetaInstance {
        let mut inst = MetaInstance::new(class);
        conform(library, &mut inst).expect("the class conforms");
        for (member, value) in members {
            // ⚠ By hash: `conform` spells a member as the index of names does, which lacks some.
            let key = name_hash(member);
            let slot = inst.fields.iter_mut().find(|(n, _)| name_hash(n) == key);
            *slot.map(|(_, v)| v).expect("the class has the member") = value.clone();
        }
        inst
    }

    fn hash(name: &str) -> Value {
        Value::Hash(HashedString::new(name))
    }

    fn structs(items: Vec<MetaInstance>) -> Value {
        let held = |i| Value::Struct(Box::new(i));
        Value::Array(items.into_iter().map(held).collect())
    }

    fn numbers<T: Copy + Into<u64>>(items: &[T]) -> Value {
        Value::Array(items.iter().map(|&i| Value::UInt(i.into())).collect())
    }

    fn floats(items: &[f32]) -> Value {
        let held = |&f| Value::Float(f64::from(f));
        Value::Array(items.iter().map(held).collect())
    }

    const UNTYPED: (&str, Value) = ("aMemberNoRecordTypes", Value::UInt(9));

    fn car_gen(library: &SchemaLibrary, model: &str, x: f32) -> MetaInstance {
        let members = [
            ("position", Value::Vec3([x, -40.25, 30.0])),
            ("orientX", Value::Float(0.5)),
            ("orientY", Value::Float(-0.75)),
            ("perpendicularLength", Value::Float(2.5)),
            ("carModel", hash(model)),
            ("flags", Value::UInt(0x0400_0680)),
            ("bodyColorRemap1", Value::Int(12)),
            ("bodyColorRemap2", Value::Int(-1)),
            ("bodyColorRemap3", Value::Int(3)),
            ("bodyColorRemap4", Value::Int(-1)),
            ("popGroup", hash("a_pop_group")),
            ("livery", Value::Int(-1)),
            UNTYPED,
        ];
        record(library, "CCarGen", &members)
    }

    const BOX: [i16; 8] = [400, -160, 120, 23170, 40, 20, 12, -23170];

    fn box_occluder(library: &SchemaLibrary) -> MetaInstance {
        let names = [
            "iCenterX", "iCenterY", "iCenterZ", "iCosZ", "iLength", "iWidth", "iHeight", "iSinZ",
        ];
        let mut members: Vec<(&str, Value)> = (names.into_iter())
            .zip(BOX.map(|v| Value::Int(i64::from(v))))
            .collect();
        members.push(UNTYPED);
        record(library, "BoxOccluder", &members)
    }

    /// Nine bytes of vertices and one triangle of three: the count of triangles is a word stated whole.
    const VERTS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 1, 2];
    const NUM_TRIS: u16 = 0x8001;

    fn occlude_model(library: &SchemaLibrary, format: BinaryFormat) -> MetaInstance {
        let mut members = vec![
            ("bmin", Value::Vec3([-8.0, -6.0, 0.0])),
            ("bmax", Value::Vec3([8.0, 6.0, 12.0])),
            ("numVertsInBytes", Value::UInt(9)),
            ("numTris", Value::UInt(u64::from(NUM_TRIS))),
            ("flags", Value::UInt(1)),
            UNTYPED,
        ];
        if format == BinaryFormat::Rbm {
            members.push(("dataSize", Value::UInt(VERTS.len() as u64)));
            members.push(("verts", Value::ByteArray(VERTS.to_vec())));
        }
        record(library, "OccludeModel", &members)
    }

    fn modifier(library: &SchemaLibrary) -> MetaInstance {
        let members = [
            ("name", hash("a_warm_evening")),
            ("minExtents", Value::Vec3([-100.0, -50.0, 0.0])),
            ("maxExtents", Value::Vec3([100.0, 50.0, 80.0])),
            ("percentage", Value::Float(80.0)),
            ("range", Value::Float(45.0)),
            ("startHour", Value::UInt(20)),
            ("endHour", Value::UInt(6)),
            UNTYPED,
        ];
        record(library, "CTimeCycleModifier", &members)
    }

    /// The tree of a map with two parked vehicles, a box occluder, an occlude model and a time cycle modifier.
    fn town_tree(library: &SchemaLibrary, format: BinaryFormat) -> MetaInstance {
        let members = [
            ("name", hash("town")),
            ("boxOccluders", structs(vec![box_occluder(library)])),
            (
                "occludeModels",
                structs(vec![occlude_model(library, format)]),
            ),
            ("timeCycleModifiers", structs(vec![modifier(library)])),
            (
                "carGenerators",
                structs(vec![
                    car_gen(library, "a_saloon", 120.5),
                    car_gen(library, "a_van", 124.5),
                ]),
            ),
        ];
        record(library, "CMapData", &members)
    }

    fn points(library: &SchemaLibrary, of: &[[f32; 3]]) -> Value {
        let point = |[x, y, z]: &[f32; 3]| {
            let members = [
                ("x", Value::Float(f64::from(*x))),
                ("y", Value::Float(f64::from(*y))),
                ("z", Value::Float(f64::from(*z))),
            ];
            record(library, "FloatXYZ", &members)
        };
        structs(of.iter().map(point).collect())
    }

    const DIRECTIONS: [[f32; 3]; 3] = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
    const POSITIONS: [[f32; 3]; 3] = [[10.0, 20.0, 30.0], [11.0, 21.0, 31.0], [12.0, 22.0, 32.0]];
    const RGBI: [u32; 3] = [0xFF10_2030, 0x8040_5060, 0x4070_8090];

    /// The tree of a map of far lights: three lights, whose positions and colours are in the map it names as its parent.
    fn lights_tree(library: &SchemaLibrary) -> MetaInstance {
        let members = [
            ("direction", points(library, &DIRECTIONS)),
            ("falloff", floats(&[25.0, 30.0, 35.0])),
            ("falloffExponent", floats(&[8.0, 16.0, 32.0])),
            (
                "timeAndStateFlags",
                numbers(&[0x00FF_FFFFu32, 0x0100_0FFF, 7]),
            ),
            ("hash", numbers(&[0xAAAA_0001u32, 0xAAAA_0002, 0xAAAA_0003])),
            ("coneInnerAngle", numbers(&[10u8, 20, 30])),
            ("coneOuterAngleOrCapExt", numbers(&[40u8, 50, 60])),
            ("coronaIntensity", numbers(&[70u8, 80, 255])),
            UNTYPED,
        ];
        let family = record(library, "CLODLight", &members);
        let members = [
            ("name", hash("town_lodlights")),
            ("parent", hash("town_distantlights")),
            ("LODLightsSOA", Value::Struct(Box::new(family))),
        ];
        record(library, "CMapData", &members)
    }

    /// The tree of the map a map of far lights names as its parent, with the positions and colours of its three lights.
    fn distant_tree(library: &SchemaLibrary) -> MetaInstance {
        let members = [
            ("position", points(library, &POSITIONS)),
            ("RGBI", numbers(&RGBI)),
            ("numStreetLights", Value::UInt(2)),
            ("category", Value::UInt(1)),
            UNTYPED,
        ];
        let family = record(library, "CDistantLODLight", &members);
        let members = [
            ("name", hash("town_distantlights")),
            ("DistantLODLightsSOA", Value::Struct(Box::new(family))),
        ];
        record(library, "CMapData", &members)
    }

    fn encoded(library: &SchemaLibrary, root: &MetaInstance, format: BinaryFormat) -> Vec<u8> {
        let layout = canonical::layout_for(library, root, format, None).expect("lays out");
        write_meta_binary(format, Some(&layout), root).expect("encodes")
    }

    fn town_file(format: BinaryFormat) -> Vec<u8> {
        let library = library(format);
        encoded(&library, &town_tree(&library, format), format)
    }

    /// The map of far lights and the map it names as its parent, as files.
    fn light_files(format: BinaryFormat) -> (Vec<u8>, Vec<u8>) {
        let library = library(format);
        (
            encoded(&library, &lights_tree(&library), format),
            encoded(&library, &distant_tree(&library), format),
        )
    }

    fn opened(bytes: &[u8]) -> CMapData {
        CMapData::from_bytes(bytes).expect("opens typed")
    }

    const FORMATS: [BinaryFormat; 2] = [BinaryFormat::Pso, BinaryFormat::Rbm];

    /// The tree `bytes` reads as, by its own type table and no schema.
    fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
            .expect("the save reads")
            .instance
    }

    /// The `index`th record of `array` in the map `root`.
    fn element<'a>(root: &'a MetaInstance, array: &str, index: usize) -> &'a MetaInstance {
        match field(root, array) {
            Some(Value::Array(items)) => match &items[index] {
                Value::Struct(inst) | Value::Pointer(Some(inst)) => inst,
                other => panic!("`{array}` holds records, and one is {other:?}"),
            },
            other => panic!("the map holds `{array}`, and it is {other:?}"),
        }
    }

    /// The family of far lights `root` holds under `member`.
    fn soa<'a>(root: &'a MetaInstance, member: &str) -> &'a MetaInstance {
        match field(root, member) {
            Some(Value::Struct(inst)) => inst,
            other => panic!("the map holds `{member}`, and it is {other:?}"),
        }
    }

    /// `edited` saved, read back equal to what was saved, and saved again to the same bytes.
    fn saved_to_its_fixed_point(edited: &CMapData, format: BinaryFormat) -> (Vec<u8>, CMapData) {
        let saved = edited.to_bytes().expect("the edited map saves");
        reads_back_at_its_fixed_point(edited, saved, format)
    }

    fn reads_back_at_its_fixed_point(
        edited: &CMapData,
        saved: Vec<u8>,
        format: BinaryFormat,
    ) -> (Vec<u8>, CMapData) {
        assert_eq!(detect_binary_format(&saved), Some(format));
        let back = opened(&saved);
        assert_eq!(&back, edited, "what was saved is what reads back");
        assert_eq!(
            back.to_bytes().expect("saves again"),
            saved,
            "a second save is the first"
        );
        (saved, back)
    }

    fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    fn xyz(points: &[FloatXYZ]) -> Vec<[f32; 3]> {
        points.iter().map(|p| [p.x, p.y, p.z]).collect()
    }

    const UNTYPED_SURVIVES: &str =
        "a member the record does not name survives an edit of one it names";

    #[test]
    fn each_record_types_the_members_the_installs_state_and_no_other() {
        assert_eq!(MEMBERS.len(), STATED.len());
        for ((class, typed), (stated_class, stated)) in MEMBERS.iter().zip(STATED) {
            assert_eq!(class, stated_class);
            let mut typed: Vec<&str> = typed.to_vec();
            typed.sort_unstable();
            assert_eq!(&typed, stated, "`{class}`");
        }
        for member in ROOT_MEMBERS {
            assert!(CMapData::MEMBERS.contains(&member), "`{member}`");
        }
    }

    #[test]
    fn each_record_of_a_map_reads_to_the_values_built_and_saves_to_the_same_bytes() {
        for format in FORMATS {
            let bytes = town_file(format);
            let map = opened(&bytes);

            assert_eq!(map.car_generators.len(), 2);
            let saloon = &map.car_generators[0];
            assert_eq!(saloon.position, vec3(120.5, -40.25, 30.0));
            assert_eq!((saloon.orient_x, saloon.orient_y), (0.5, -0.75));
            assert_eq!(saloon.perpendicular_length, 2.5);
            assert!(saloon.car_model == "a_saloon" && saloon.pop_group == "a_pop_group");
            assert_eq!(saloon.flags, 0x0400_0680);
            let remaps = [
                saloon.body_color_remap1,
                saloon.body_color_remap2,
                saloon.body_color_remap3,
                saloon.body_color_remap4,
            ];
            assert_eq!((remaps, saloon.livery), ([12, -1, 3, -1], -1));
            assert!(map.car_generators[1].car_model == "a_van");
            assert_eq!(map.car_generators[1].position.x, 124.5);

            assert_eq!(map.box_occluders.len(), 1);
            let b = &map.box_occluders[0];
            let read = [
                b.i_center_x,
                b.i_center_y,
                b.i_center_z,
                b.i_cos_z,
                b.i_length,
                b.i_width,
                b.i_height,
                b.i_sin_z,
            ];
            assert_eq!(read, BOX);

            assert_eq!(map.occlude_models.len(), 1);
            let model = &map.occlude_models[0];
            assert_eq!(
                (model.bmin, model.bmax),
                (vec3(-8.0, -6.0, 0.0), vec3(8.0, 6.0, 12.0))
            );
            assert_eq!(
                (model.num_verts_in_bytes, model.num_tris, model.flags),
                (9, NUM_TRIS, 1)
            );
            if format == BinaryFormat::Rbm {
                assert_eq!(model.data_size as usize, VERTS.len());
                assert_eq!(model.verts, VERTS);
            }

            assert_eq!(map.time_cycle_modifiers.len(), 1);
            let warm = &map.time_cycle_modifiers[0];
            assert!(warm.name == "a_warm_evening");
            assert_eq!(
                (warm.min_extents, warm.max_extents),
                (vec3(-100.0, -50.0, 0.0), vec3(100.0, 50.0, 80.0))
            );
            assert_eq!((warm.percentage, warm.range), (80.0, 45.0));
            assert_eq!((warm.start_hour, warm.end_hour), (20, 6));
            assert!(map.lod_lights_soa.is_empty() && map.distant_lod_lights_soa.is_empty());

            assert_eq!(
                map.to_bytes().expect("an untouched save"),
                bytes,
                "{format:?}: an untouched map saves to the bytes it arrived as"
            );

            let (lights_bytes, distant_bytes) = light_files(format);
            let (lights, distant) = (opened(&lights_bytes), opened(&distant_bytes));
            let family = &lights.lod_lights_soa;
            assert_eq!(family.len(), 3);
            assert_eq!(xyz(&family.direction), DIRECTIONS);
            assert_eq!(family.falloff, [25.0, 30.0, 35.0]);
            assert_eq!(family.falloff_exponent, [8.0, 16.0, 32.0]);
            assert_eq!(family.time_and_state_flags, [0x00FF_FFFF, 0x0100_0FFF, 7]);
            assert_eq!(family.hash, [0xAAAA_0001, 0xAAAA_0002, 0xAAAA_0003]);
            assert_eq!(family.cone_inner_angle, [10, 20, 30]);
            assert_eq!(family.cone_outer_angle_or_cap_ext, [40, 50, 60]);
            assert_eq!(family.corona_intensity, [70, 80, 255]);
            assert!(lights.distant_lod_lights_soa.is_empty());
            assert!(lights.parent.hash == distant.name.hash);

            let family = &distant.distant_lod_lights_soa;
            assert_eq!(family.len(), 3);
            assert_eq!(xyz(&family.position), POSITIONS);
            assert_eq!(family.rgbi, RGBI);
            assert_eq!((family.num_street_lights, family.category), (2, 1));
            assert!(distant.lod_lights_soa.is_empty());

            let saved = save_far_lights(&lights, &distant).expect("an untouched pair saves");
            assert_eq!(
                saved,
                (lights_bytes, distant_bytes),
                "{format:?}: an untouched pair of maps saves to the bytes it arrived as"
            );
        }
    }

    #[test]
    fn a_parked_vehicle_moved_and_given_another_model_saves_reads_back_and_keeps_what_no_record_types(
    ) {
        for format in FORMATS {
            let bytes = town_file(format);
            let mut map = opened(&bytes);
            let van = &mut map.car_generators[1];
            van.position = vec3(-300.0, 410.5, 22.25);
            (van.orient_x, van.orient_y) = (1.0, 0.0);
            van.car_model = hashed("a_pickup");
            van.livery = 3;
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            assert_ne!(saved, bytes);
            let van = &back.car_generators[1];
            assert_eq!(van.position, vec3(-300.0, 410.5, 22.25));
            assert_eq!((van.orient_x, van.orient_y, van.livery), (1.0, 0.0, 3));
            assert!(van.car_model == "a_pickup" && van.pop_group == "a_pop_group");
            assert!(back.car_generators[0].car_model == "a_saloon");

            let root = tree(&saved);
            for (array, at) in [
                ("carGenerators", 1),
                ("carGenerators", 0),
                ("boxOccluders", 0),
                ("occludeModels", 0),
                ("timeCycleModifiers", 0),
            ] {
                assert_eq!(
                    field(element(&root, array, at), UNTYPED.0),
                    Some(&UNTYPED.1),
                    "{format:?}, `{array}`: {UNTYPED_SURVIVES}"
                );
            }

            // Put back, the file is the one it was: nothing else moved.
            let mut map = back;
            map.car_generators[1] = opened(&bytes).car_generators[1].clone();
            assert_eq!(map.to_bytes().expect("saves"), bytes);
        }
    }

    #[test]
    fn a_box_occluder_resized_saves_and_reads_back() {
        for format in FORMATS {
            let mut map = opened(&town_file(format));
            let b = &mut map.box_occluders[0];
            (b.i_length, b.i_width, b.i_height) = (64, -32, i16::MAX);
            b.i_center_z = i16::MIN;
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            let b = &back.box_occluders[0];
            assert_eq!((b.i_length, b.i_width, b.i_height), (64, -32, i16::MAX));
            assert_eq!((b.i_center_x, b.i_center_z), (BOX[0], i16::MIN));
            assert_eq!((b.i_cos_z, b.i_sin_z), (BOX[3], BOX[7]));
            let root = tree(&saved);
            assert_eq!(
                field(element(&root, "boxOccluders", 0), UNTYPED.0),
                Some(&UNTYPED.1),
                "{format:?}: {UNTYPED_SURVIVES}"
            );
        }
    }

    #[test]
    fn a_time_cycle_modifier_whose_hours_change_saves_and_reads_back() {
        for format in FORMATS {
            let mut map = opened(&town_file(format));
            let warm = &mut map.time_cycle_modifiers[0];
            (warm.start_hour, warm.end_hour) = (5, 23);
            warm.max_extents.z = 120.0;
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            let warm = &back.time_cycle_modifiers[0];
            assert_eq!((warm.start_hour, warm.end_hour), (5, 23));
            assert_eq!(warm.max_extents, vec3(100.0, 50.0, 120.0));
            assert!(warm.name == "a_warm_evening");
            assert_eq!((warm.percentage, warm.range), (80.0, 45.0));
            let root = tree(&saved);
            assert_eq!(
                field(element(&root, "timeCycleModifiers", 0), UNTYPED.0),
                Some(&UNTYPED.1),
                "{format:?}: {UNTYPED_SURVIVES}"
            );
        }
    }

    #[test]
    fn a_far_light_moved_in_both_families_saves_both_maps_and_reads_back() {
        for format in FORMATS {
            let (lights_bytes, distant_bytes) = light_files(format);
            let (mut lights, mut distant) = (opened(&lights_bytes), opened(&distant_bytes));
            // The light at 1: where it stands is the parent's, and where it points and how far it reaches the child's.
            let moved = FloatXYZ {
                x: -5.0,
                y: 6.5,
                z: 70.0,
                ..distant.distant_lod_lights_soa.position[1].clone()
            };
            distant.distant_lod_lights_soa.position[1] = moved;
            distant.distant_lod_lights_soa.rgbi[1] = 0x1122_3344;
            lights.lod_lights_soa.direction[1].z = -0.5;
            lights.lod_lights_soa.falloff[1] = 44.0;
            lights.lod_lights_soa.cone_outer_angle_or_cap_ext[1] = 99;

            let (saved_lights, saved_distant) =
                save_far_lights(&lights, &distant).expect("the pair saves");
            let (saved_lights, back) = reads_back_at_its_fixed_point(&lights, saved_lights, format);
            let (saved_distant, parent) =
                reads_back_at_its_fixed_point(&distant, saved_distant, format);
            assert_ne!(saved_lights, lights_bytes);
            assert_ne!(saved_distant, distant_bytes);

            let family = &parent.distant_lod_lights_soa;
            let mut positions = POSITIONS;
            positions[1] = [-5.0, 6.5, 70.0];
            assert_eq!(xyz(&family.position), positions);
            assert_eq!(family.rgbi, [RGBI[0], 0x1122_3344, RGBI[2]]);
            assert_eq!((family.num_street_lights, family.category), (2, 1));
            let family = &back.lod_lights_soa;
            let mut directions = DIRECTIONS;
            directions[1][2] = -0.5;
            assert_eq!(xyz(&family.direction), directions);
            assert_eq!(family.falloff, [25.0, 44.0, 35.0]);
            assert_eq!(family.cone_outer_angle_or_cap_ext, [40, 99, 60]);
            assert_eq!(family.hash, [0xAAAA_0001, 0xAAAA_0002, 0xAAAA_0003]);
            assert_eq!(
                back.lod_lights_soa.len(),
                parent.distant_lod_lights_soa.len()
            );

            for (saved, member) in [
                (&saved_lights, "LODLightsSOA"),
                (&saved_distant, "DistantLODLightsSOA"),
            ] {
                let root = tree(saved);
                assert_eq!(
                    field(soa(&root, member), UNTYPED.0),
                    Some(&UNTYPED.1),
                    "{format:?}, `{member}`: {UNTYPED_SURVIVES}"
                );
            }
        }
    }

    /// The far light at `at` taken out of every array of both families.
    fn take_light(lights: &mut CMapData, distant: &mut CMapData, at: usize) {
        let family = &mut lights.lod_lights_soa;
        family.direction.remove(at);
        family.falloff.remove(at);
        family.falloff_exponent.remove(at);
        family.time_and_state_flags.remove(at);
        family.hash.remove(at);
        family.cone_inner_angle.remove(at);
        family.cone_outer_angle_or_cap_ext.remove(at);
        family.corona_intensity.remove(at);
        distant.distant_lod_lights_soa.position.remove(at);
        distant.distant_lod_lights_soa.rgbi.remove(at);
    }

    #[test]
    fn a_far_light_taken_out_of_both_families_and_one_added_as_a_copy_save_and_read_back() {
        for format in FORMATS {
            let (lights_bytes, distant_bytes) = light_files(format);
            let (mut lights, mut distant) = (opened(&lights_bytes), opened(&distant_bytes));
            take_light(&mut lights, &mut distant, 0);
            let (saved_lights, saved_distant) =
                save_far_lights(&lights, &distant).expect("the pair saves");
            let (_, back) = reads_back_at_its_fixed_point(&lights, saved_lights, format);
            let (_, parent) = reads_back_at_its_fixed_point(&distant, saved_distant, format);
            assert_eq!(back.lod_lights_soa.hash, [0xAAAA_0002, 0xAAAA_0003]);
            assert_eq!(xyz(&back.lod_lights_soa.direction), DIRECTIONS[1..]);
            assert_eq!(back.lod_lights_soa.corona_intensity, [80, 255]);
            assert_eq!(xyz(&parent.distant_lod_lights_soa.position), POSITIONS[1..]);
            assert_eq!(parent.distant_lod_lights_soa.rgbi, RGBI[1..]);

            // A light added is a copy of one each family holds, with a place and a hash of its own.
            let (mut lights, mut distant) = (opened(&lights_bytes), opened(&distant_bytes));
            let family = &mut lights.lod_lights_soa;
            family.direction.push(family.direction[2].clone());
            family.falloff.push(family.falloff[2]);
            family.falloff_exponent.push(family.falloff_exponent[2]);
            family
                .time_and_state_flags
                .push(family.time_and_state_flags[2]);
            family.hash.push(0xAAAA_0004);
            family.cone_inner_angle.push(family.cone_inner_angle[2]);
            family
                .cone_outer_angle_or_cap_ext
                .push(family.cone_outer_angle_or_cap_ext[2]);
            family.corona_intensity.push(family.corona_intensity[2]);
            let family = &mut distant.distant_lod_lights_soa;
            let mut at = family.position[2].clone();
            at.x = 13.0;
            family.position.push(at);
            family.rgbi.push(RGBI[2]);
            let (saved_lights, saved_distant) =
                save_far_lights(&lights, &distant).expect("the pair saves");
            let (_, back) = reads_back_at_its_fixed_point(&lights, saved_lights, format);
            let (_, parent) = reads_back_at_its_fixed_point(&distant, saved_distant, format);
            assert_eq!(back.lod_lights_soa.len(), 4);
            assert_eq!(back.lod_lights_soa.hash[3], 0xAAAA_0004);
            assert_eq!(back.lod_lights_soa.corona_intensity, [70, 80, 255, 255]);
            let position = &parent.distant_lod_lights_soa.position;
            assert_eq!(xyz(position)[3], [13.0, 22.0, 32.0]);
            assert_eq!(xyz(position)[..3], POSITIONS);
        }
    }

    /// What `to_bytes` refuses `map` with.
    fn refusal(map: &CMapData) -> String {
        let refused = map.to_bytes().expect_err("a broken map is not saved");
        assert!(matches!(refused, Error::Refused { .. }), "{refused}");
        refused.to_string()
    }

    #[test]
    fn far_lights_whose_arrays_differ_in_length_are_refused_at_the_save_with_their_sentence() {
        for format in FORMATS {
            let (lights_bytes, distant_bytes) = light_files(format);
            let (lights, distant) = (opened(&lights_bytes), opened(&distant_bytes));

            type Shorten = fn(&mut CLODLight);
            let shortened: [(Shorten, &str); 8] = [
                (|f| f.direction.truncate(2), "direction"),
                (|f| f.falloff.truncate(2), "falloff"),
                (|f| f.falloff_exponent.truncate(2), "falloffExponent"),
                (|f| f.time_and_state_flags.truncate(2), "timeAndStateFlags"),
                (|f| f.hash.truncate(2), "hash"),
                (|f| f.cone_inner_angle.truncate(2), "coneInnerAngle"),
                (
                    |f| f.cone_outer_angle_or_cap_ext.truncate(2),
                    "coneOuterAngleOrCapExt",
                ),
                (|f| f.corona_intensity.truncate(2), "coronaIntensity"),
            ];
            for (shorten, member) in shortened {
                let mut map = lights.clone();
                shorten(&mut map.lod_lights_soa);
                let why = refusal(&map);
                let said = format!("light array {member} holds 2 where its siblings hold 3");
                assert!(why.ends_with(&said), "{why}");
                let why = save_far_lights(&map, &distant)
                    .expect_err("nor is it saved with its parent")
                    .to_string();
                assert!(
                    why.contains("light array") || why.contains("far lights"),
                    "{why}"
                );
            }

            // One array longer than the rest: each of the others is the one that is short.
            let mut map = lights.clone();
            map.lod_lights_soa.hash.push(0xAAAA_0004);
            let why = refusal(&map);
            let said = "light array direction holds 3 where its siblings hold 4";
            assert!(why.ends_with(said), "{why}");

            let mut map = distant.clone();
            map.distant_lod_lights_soa.rgbi.pop();
            let why = refusal(&map);
            let said = "light array RGBI holds 2 where its siblings hold 3";
            assert!(why.ends_with(said), "{why}");

            let mut map = distant.clone();
            map.distant_lod_lights_soa.position.clear();
            let why = refusal(&map);
            let said = "light array position holds 0 where its siblings hold 3";
            assert!(why.ends_with(said), "{why}");

            lights.to_bytes().expect("and the lights as they came save");
            distant.to_bytes().expect("and so does their parent");
        }
    }

    #[test]
    fn a_pair_of_far_light_maps_out_of_step_is_refused_at_the_save_with_its_sentence() {
        for format in FORMATS {
            let (lights_bytes, distant_bytes) = light_files(format);
            let (lights, distant) = (opened(&lights_bytes), opened(&distant_bytes));

            // A light taken out of one family whole: each map saves alone, and the pair does not.
            let mut shorter = lights.clone();
            take_light(&mut shorter, &mut distant.clone(), 2);
            shorter.to_bytes().expect("the map alone holds its lengths");
            let refused = save_far_lights(&shorter, &distant).expect_err("the pair is out of step");
            assert!(matches!(refused, Error::Refused { .. }), "{refused}");
            let said = "the map holds 2 far lights and its parent 3 distant ones: the two run under one subscript, a light to each";
            assert!(refused.to_string().ends_with(said), "{refused}");

            let mut parent = distant.clone();
            take_light(&mut lights.clone(), &mut parent, 2);
            parent
                .to_bytes()
                .expect("the parent alone holds its lengths");
            let refused = save_far_lights(&lights, &parent).expect_err("the pair is out of step");
            let said = "the map holds 3 far lights and its parent 2 distant ones: the two run under one subscript, a light to each";
            assert!(refused.to_string().ends_with(said), "{refused}");

            // A map that is not the one the lights name as their parent, though it holds as many.
            let mut stranger = distant.clone();
            stranger.name = HashedString::new("another_town_distantlights");
            let refused = save_far_lights(&lights, &stranger).expect_err("not the parent");
            assert!(matches!(refused, Error::Refused { .. }), "{refused}");
            let said = format!(
                "the map of far lights names the parent {:#010X}, and the map given with it is {:#010X}",
                crate::string_hash(b"town_distantlights"),
                crate::string_hash(b"another_town_distantlights")
            );
            assert!(refused.to_string().ends_with(&said), "{refused}");

            save_far_lights(&lights, &distant).expect("and the pair as it came saves");
        }
    }

    #[test]
    fn an_occlude_models_bytes_are_changed_and_resized_and_a_count_that_misses_them_is_refused() {
        let format = BinaryFormat::Rbm;
        let bytes = town_file(format);

        // As many bytes as it held: the block the file arrived with takes them.
        let mut map = opened(&bytes);
        map.occlude_models[0].verts[4] = 200;
        let (saved, back) = saved_to_its_fixed_point(&map, format);
        let mut changed = VERTS;
        changed[4] = 200;
        assert_eq!(back.occlude_models[0].verts, changed);
        assert_ne!(saved, bytes);

        // More bytes and fewer, with the count that states them.
        for bytes_held in [VERTS.len() + 12, 3, 0] {
            let mut map = opened(&bytes);
            let model = &mut map.occlude_models[0];
            model.verts = (0..bytes_held).map(|b| b as u8 ^ 0x5A).collect();
            model.data_size = bytes_held as u32;
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            let model = &back.occlude_models[0];
            assert_eq!(model.verts, map.occlude_models[0].verts);
            assert_eq!(model.data_size as usize, bytes_held);
            assert_eq!((model.num_tris, model.flags), (NUM_TRIS, 1));
            assert_eq!(back.car_generators.len(), 2);
            let root = tree(&saved);
            assert_eq!(
                field(element(&root, "occludeModels", 0), UNTYPED.0),
                Some(&UNTYPED.1),
                "{bytes_held} bytes: {UNTYPED_SURVIVES}"
            );
        }

        let mut map = opened(&bytes);
        map.occlude_models[0].verts.push(7);
        let why = refusal(&map);
        let said = "occlude model 0 states 12 bytes of `verts` (dataSize) and holds 13";
        assert!(why.ends_with(said), "{why}");

        let mut map = opened(&bytes);
        map.occlude_models[0].data_size = 11;
        let why = refusal(&map);
        let said = "occlude model 0 states 11 bytes of `verts` (dataSize) and holds 12";
        assert!(why.ends_with(said), "{why}");
    }

    /// A town whose second occlude model holds `second` behind its pointer, another count of bytes than the first.
    fn town_of_two_models(second: &[u8]) -> Vec<u8> {
        let format = BinaryFormat::Rbm;
        let library = library(format);
        let mut other = occlude_model(&library, format);
        for (member, value) in &mut other.fields {
            if name_hash(member) == name_hash("verts") {
                *value = Value::ByteArray(second.to_vec());
            } else if name_hash(member) == name_hash("dataSize") {
                *value = Value::UInt(second.len() as u64);
            } else if name_hash(member) == name_hash("flags") {
                *value = Value::UInt(2);
            }
        }
        let mut root = town_tree(&library, format);
        let Some(Value::Array(models)) = root.get_mut("occludeModels") else {
            panic!("the map holds its occlude models");
        };
        models.push(Value::Struct(Box::new(other)));
        encoded(&library, &root, format)
    }

    /// ⚠ The block a run of bytes arrived in is sized for that run and no other: a model that takes the place of one
    /// with another count of bytes would write past that block, or leave the end of a longer run behind its own.
    #[test]
    fn occlude_models_of_other_lengths_that_change_places_save_and_read_back() {
        let format = BinaryFormat::Rbm;
        for second in [&[9u8, 8, 7, 6, 5][..], &[0x5A; 40][..], &[][..]] {
            let bytes = town_of_two_models(second);
            let arrived = opened(&bytes);
            assert_eq!(arrived.occlude_models[0].verts, VERTS);
            assert_eq!(arrived.occlude_models[1].verts, second);
            assert_eq!(arrived.to_bytes().expect("an untouched save"), bytes);

            let mut map = arrived.clone();
            map.occlude_models.swap(0, 1);
            let (_, back) = saved_to_its_fixed_point(&map, format);
            assert_eq!(back.occlude_models[0].verts, second);
            assert_eq!(back.occlude_models[1].verts, VERTS);
            assert_eq!(
                (back.occlude_models[0].flags, back.occlude_models[1].flags),
                (2, 1)
            );
            assert_eq!(back.car_generators, arrived.car_generators);
            assert_eq!(back.box_occluders, arrived.box_occluders);
            assert_eq!(back.time_cycle_modifiers, arrived.time_cycle_modifiers);

            // One taken out and a copy of the other put where it was: as many models as the file arrived with.
            for (from, to) in [(1, 0), (0, 1)] {
                let mut map = arrived.clone();
                map.occlude_models[to] = map.occlude_models[from].clone();
                let (_, back) = saved_to_its_fixed_point(&map, format);
                assert_eq!(back.occlude_models[0], back.occlude_models[1]);
                assert_eq!(
                    back.occlude_models[to].verts,
                    arrived.occlude_models[from].verts
                );
                assert_eq!(back.car_generators, arrived.car_generators);
            }
        }
    }

    #[test]
    fn a_record_added_as_a_copy_of_one_the_file_holds_saves_and_reads_back() {
        for format in FORMATS {
            let mut map = opened(&town_file(format));
            let mut coupe = map.car_generators[0].clone();
            coupe.car_model = hashed("a_coupe");
            coupe.position.x = 128.5;
            map.car_generators.push(coupe);
            let mut second = map.box_occluders[0].clone();
            second.i_center_x = -400;
            map.box_occluders.push(second);
            let mut cold = map.time_cycle_modifiers[0].clone();
            cold.name = hashed("a_cold_morning");
            map.time_cycle_modifiers.insert(0, cold);
            let model = map.occlude_models[0].clone();
            map.occlude_models.push(model);
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            assert_eq!(back.car_generators.len(), 3);
            assert!(back.car_generators[2].car_model == "a_coupe");
            assert_eq!(back.car_generators[2].position.x, 128.5);
            assert!(back.car_generators[2].pop_group == "a_pop_group");
            assert_eq!(back.box_occluders[1].i_center_x, -400);
            assert_eq!(back.box_occluders[1].i_cos_z, BOX[3]);
            assert!(back.time_cycle_modifiers[0].name == "a_cold_morning");
            assert!(back.time_cycle_modifiers[1].name == "a_warm_evening");
            assert_eq!(back.occlude_models[1], back.occlude_models[0]);
            let root = tree(&saved);
            for (array, at) in [
                ("carGenerators", 2),
                ("boxOccluders", 1),
                ("timeCycleModifiers", 0),
                ("occludeModels", 1),
            ] {
                assert_eq!(
                    field(element(&root, array, at), UNTYPED.0),
                    Some(&UNTYPED.1),
                    "{format:?}, `{array}`: the copy keeps what the record it copies states"
                );
            }
        }
    }

    /// The members `inst` states, by name hash, in the order it states them.
    fn stated(inst: &MetaInstance) -> Vec<u32> {
        inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
    }

    /// ⚠ As a hash alone, which is how a name the index lacks reads back: two names are equal when their text is too.
    fn hashed(name: &str) -> HashedString {
        HashedString::from_hash(crate::string_hash(name.as_bytes()))
    }

    /// ⚠ One kind to a save: a save is told that it holds a record built here by any one of them, so two kinds in one
    /// save would each hide that the other went unseen.
    #[test]
    fn each_kind_of_record_built_here_takes_the_members_the_files_own_class_states() {
        type Build = fn(&mut CMapData);
        let builds: [(&str, Build); 4] = [
            ("carGenerators", |map| {
                map.car_generators.push(CCarGen {
                    position: vec3(1.0, 2.0, 3.0),
                    car_model: hashed("a_coupe"),
                    ..Default::default()
                });
            }),
            ("boxOccluders", |map| {
                map.box_occluders.push(BoxOccluder {
                    i_length: 8,
                    i_cos_z: 32767,
                    ..Default::default()
                });
            }),
            ("timeCycleModifiers", |map| {
                map.time_cycle_modifiers.push(CTimeCycleModifier {
                    name: hashed("a_cold_morning"),
                    end_hour: 9,
                    ..Default::default()
                });
            }),
            ("occludeModels", |map| {
                map.occlude_models.push(OccludeModel {
                    bmax: vec3(1.0, 1.0, 1.0),
                    ..Default::default()
                });
            }),
        ];
        for format in FORMATS {
            for (array, build) in builds {
                let mut map = opened(&town_file(format));
                build(&mut map);
                let (saved, back) = saved_to_its_fixed_point(&map, format);
                let held = match array {
                    "carGenerators" => back.car_generators.len(),
                    "boxOccluders" => back.box_occluders.len(),
                    "timeCycleModifiers" => back.time_cycle_modifiers.len(),
                    _ => back.occlude_models.len(),
                };
                let root = tree(&saved);
                assert_eq!(
                    stated(element(&root, array, held - 1)),
                    stated(element(&root, array, 0)),
                    "{format:?}: a record built into `{array}` states what the file's own class states"
                );
            }
            let built = CCarGen::default();
            assert_eq!(built.livery, -1, "the engine's init");

            // A far light whose point is built here, in each family.
            let (lights_bytes, distant_bytes) = light_files(format);
            let (mut lights, mut distant) = (opened(&lights_bytes), opened(&distant_bytes));
            let point = |x| FloatXYZ {
                x,
                ..Default::default()
            };
            lights.lod_lights_soa.direction[0] = point(0.25);
            distant.distant_lod_lights_soa.position[0] = point(99.0);
            let (saved_lights, saved_distant) =
                save_far_lights(&lights, &distant).expect("the pair saves");
            let (_, back) = reads_back_at_its_fixed_point(&lights, saved_lights, format);
            let (_, parent) = reads_back_at_its_fixed_point(&distant, saved_distant, format);
            assert_eq!(xyz(&back.lod_lights_soa.direction)[0], [0.25, 0.0, 0.0]);
            assert_eq!(
                xyz(&parent.distant_lod_lights_soa.position)[0],
                [99.0, 0.0, 0.0]
            );
        }
    }

    #[test]
    fn a_map_made_from_nothing_refuses_a_record_its_class_has_no_member_for() {
        let empty = CMapData {
            name: HashedString::new("hills"),
            ..Default::default()
        };
        let whole = empty.to_bytes().expect("a map with no record saves");
        let donor = opened(&town_file(BinaryFormat::Pso));
        let (lights, distant) = light_files(BinaryFormat::Pso);
        let (lights, distant) = (opened(&lights), opened(&distant));

        type Put<'a> = Box<dyn Fn(&mut CMapData) + 'a>;
        let puts: [(&str, Put<'_>); 8] = [
            (
                "carGenerators",
                Box::new(|map| map.car_generators.push(CCarGen::default())),
            ),
            (
                "carGenerators",
                Box::new(|map| map.car_generators.push(donor.car_generators[0].clone())),
            ),
            (
                "boxOccluders",
                Box::new(|map| map.box_occluders.push(donor.box_occluders[0].clone())),
            ),
            (
                "occludeModels",
                Box::new(|map| map.occlude_models.push(OccludeModel::default())),
            ),
            (
                "timeCycleModifiers",
                Box::new(|map| {
                    map.time_cycle_modifiers
                        .push(donor.time_cycle_modifiers[0].clone())
                }),
            ),
            (
                "LODLightsSOA",
                Box::new(|map| map.lod_lights_soa = lights.lod_lights_soa.clone()),
            ),
            (
                "DistantLODLightsSOA",
                Box::new(|map| map.distant_lod_lights_soa = distant.distant_lod_lights_soa.clone()),
            ),
            (
                "DistantLODLightsSOA",
                Box::new(|map| map.distant_lod_lights_soa.category = 2),
            ),
        ];
        for (member, put) in &puts {
            for mut map in [empty.clone(), opened(&whole)] {
                put(&mut map);
                let refused = map
                    .to_bytes()
                    .expect_err("the class of a map made from nothing states no such member");
                assert!(matches!(refused, Error::Save { .. }), "{refused}");
                let said = format!(
                    "`CMapData::{member}` was set, and the document's own `CMapData` has no such member to hold it"
                );
                assert!(refused.to_string().contains(&said), "{refused}");
            }
        }
        assert_eq!(opened(&whole).to_bytes().expect("saves again"), whole);
    }

    /// A map whose classes state fewer members than the installs' do: a parked vehicle with no population group and no
    /// livery, which is what the four PSO maps of the Legacy install state.
    fn narrower_file(format: BinaryFormat) -> Vec<u8> {
        let wide = library(format);
        let mut narrow = wide.clone();
        let class = narrow.structs.get_mut("CCarGen").expect("declared");
        let dropped = [name_hash("popGroup"), name_hash("livery")];
        class
            .fields
            .retain(|f| !dropped.contains(&name_hash(&f.name)));
        let members = [
            ("name", hash("an_older_town")),
            (
                "carGenerators",
                structs(vec![record(
                    &narrow,
                    "CCarGen",
                    &[("carModel", hash("a_saloon")), UNTYPED],
                )]),
            ),
        ];
        let root = record(&narrow, "CMapData", &members);
        encoded(&narrow, &root, format)
    }

    #[test]
    fn a_member_a_files_own_class_lacks_reads_as_the_engines_init_and_refuses_a_change() {
        for format in FORMATS {
            let bytes = narrower_file(format);
            let mut map = opened(&bytes);
            let saloon = &map.car_generators[0];
            assert!(saloon.car_model == "a_saloon");
            assert_eq!(saloon.livery, -1, "the engine's init");
            assert_eq!(saloon.pop_group, HashedString::default());
            assert_eq!(map.to_bytes().expect("an untouched save"), bytes);

            map.car_generators[0].car_model = hashed("a_van");
            let (saved, back) = saved_to_its_fixed_point(&map, format);
            assert!(back.car_generators[0].car_model == "a_van");
            assert_eq!(
                stated(element(&tree(&saved), "carGenerators", 0)),
                stated(element(&tree(&bytes), "carGenerators", 0)),
                "the class the file states is the class it is saved with"
            );

            map.car_generators[0].livery = 2;
            let refused = map
                .to_bytes()
                .expect_err("the file's own class has no livery to hold 2");
            let why = refused.to_string();
            assert!(
                why.contains("`CCarGen::livery` was set") && why.contains("no such member"),
                "{why}"
            );
        }
    }

    #[test]
    fn an_occlude_model_with_no_bytes_behind_its_pointer_reads_empty_saves_as_it_came_and_takes_bytes(
    ) {
        let format = BinaryFormat::Rbm;
        let library = library(format);
        let mut root = town_tree(&library, format);
        let Some(Value::Array(models)) = root.get_mut("occludeModels") else {
            panic!("the map holds its occlude models");
        };
        let Some(Value::Struct(model)) = models.first_mut() else {
            panic!("the map holds its occlude model");
        };
        for (member, value) in &mut model.fields {
            if name_hash(member) == name_hash("verts") {
                *value = Value::Null;
            } else if name_hash(member) == name_hash("dataSize") {
                *value = Value::UInt(0);
            }
        }
        let bytes = encoded(&library, &root, format);
        assert_eq!(
            field(element(&tree(&bytes), "occludeModels", 0), "verts"),
            Some(&Value::Null),
            "a null pointer reads as nothing"
        );
        let mut map = opened(&bytes);
        assert!(map.occlude_models[0].verts.is_empty());
        assert_eq!(map.to_bytes().expect("an untouched save"), bytes);

        map.occlude_models[0].verts = VERTS.to_vec();
        map.occlude_models[0].data_size = VERTS.len() as u32;
        let (_, back) = saved_to_its_fixed_point(&map, format);
        assert_eq!(back.occlude_models[0].verts, VERTS);
    }

    /// ⚠ A text read with no schema calls a parked vehicle `Item`, the element that holds it, and states the bytes of an
    /// occlude model as text.
    #[test]
    fn a_text_map_read_with_no_schema_saves_its_records_as_they_came_and_takes_an_edit() {
        let mut saloon = MetaInstance::new("Item");
        saloon.set("position", Value::Vec3([1.0, 2.0, 3.0]));
        saloon.set("carModel", Value::String("a_saloon".into()));
        saloon.set("flags", Value::Int(1664));
        saloon.set("livery", Value::Int(-1));
        saloon.set("aMemberNoRecordTypes", Value::Int(9));
        let mut model = MetaInstance::new("Item");
        model.set("dataSize", Value::Int(VERTS.len() as i64));
        model.set("verts", Value::ByteArray(VERTS.to_vec()));
        model.set("numTris", Value::Int(i64::from(NUM_TRIS)));
        let mut lights = MetaInstance::new("LODLightsSOA");
        lights.set("falloff", Value::Array(Vec::new()));
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::String("town".into()));
        root.set("occludeModels", structs(vec![model]));
        root.set("carGenerators", structs(vec![saloon]));
        root.set("LODLightsSOA", Value::Struct(Box::new(lights)));
        let text = write_meta(&root).expect("writes as text").into_bytes();

        let mut map = opened(&text);
        assert!(map.car_generators[0].car_model == "a_saloon");
        assert_eq!(map.car_generators[0].flags, 1664);
        assert_eq!(map.occlude_models[0].verts, VERTS);
        assert_eq!(map.occlude_models[0].num_tris, NUM_TRIS);
        assert!(map.lod_lights_soa.is_empty());
        assert_eq!(
            String::from_utf8(map.to_bytes().expect("an untouched save")),
            String::from_utf8(text),
            "an untouched text saves as it came"
        );

        map.car_generators[0].position.z = 4.5;
        map.car_generators[0].flags = 1665;
        map.occlude_models[0].verts[0] = 77;
        let saved = map.to_bytes().expect("the edited records save");
        let back = opened(&saved);
        assert_eq!(back, map, "what was saved is what reads back");
        assert_eq!(back.car_generators[0].position, vec3(1.0, 2.0, 4.5));
        assert_eq!(back.car_generators[0].flags, 1665);
        assert_eq!(back.occlude_models[0].verts[0], 77);
        assert_eq!(back.to_bytes().expect("saves again"), saved);
        assert_eq!(
            field(element(&tree(&saved), "carGenerators", 0), UNTYPED.0),
            Some(&Value::Int(9))
        );
    }

    /// ⚠ Read with no schema, an element that holds nothing is the empty text of a leaf: it is what a person leaves of a
    /// family of far lights, or of a record, they emptied by hand.
    #[test]
    fn a_text_map_whose_record_is_an_element_that_holds_nothing_opens_and_saves_as_it_came() {
        let text = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<CMapData>\n",
            "  <name>town</name>\n",
            "  <carGenerators>\n",
            "    <Item/>\n",
            "  </carGenerators>\n",
            "  <LODLightsSOA/>\n",
            "  <DistantLODLightsSOA/>\n",
            "</CMapData>\n",
        );
        let mut map = opened(text.as_bytes());
        assert_eq!(map.car_generators.len(), 1);
        assert_eq!(map.car_generators[0].livery, -1, "the engine's init");
        assert!(map.lod_lights_soa.is_empty() && map.distant_lod_lights_soa.is_empty());
        let saved = map.to_bytes().expect("an untouched save");
        assert_eq!(String::from_utf8(saved).as_deref(), Ok(text));

        map.name = HashedString::new("another_town");
        let saved = map.to_bytes().expect("the renamed map saves");
        let back = opened(&saved);
        assert_eq!(back, map, "what was saved is what reads back");
        assert!(back.name == "another_town");
        assert_eq!(back.to_bytes().expect("saves again"), saved);

        // A member such a record states nowhere has no place to be set, as in a class that lacks it.
        map.car_generators[0].livery = 2;
        let why = map
            .to_bytes()
            .expect_err("the record states no livery to hold 2")
            .to_string();
        assert!(
            why.contains("`CCarGen::livery` was set") && why.contains("no such member"),
            "{why}"
        );
    }

    #[test]
    fn the_records_of_a_map_are_judged_typed_by_their_class() {
        let mut inst = MetaInstance::new("CMapData");
        let mut one = MetaInstance::new("CCarGen");
        one.set("aMemberNoRecordTypes", Value::UInt(1));
        inst.set("carGenerators", structs(vec![one.clone()]));
        let first = CMapData::read(&inst).expect("reads");
        one.set("aMemberNoRecordTypes", Value::UInt(2));
        inst.set("carGenerators", structs(vec![one]));
        let other = CMapData::read(&inst).expect("reads");
        assert_ne!(
            first, other,
            "a member no record types still tells two parked vehicles apart"
        );
        assert!(is_typed("CCarGen", name_hash("popGroup")));
        assert!(is_typed("CLODLight", name_hash("hash")));
        assert!(is_typed("FloatXYZ", name_hash("x")));
        assert!(!is_typed("CMapData", name_hash("popGroup")));
        assert!(!is_typed("CCarGen", name_hash("hash")));
        assert!(!is_typed("CDistantLODLight", name_hash("x")));
    }
}
