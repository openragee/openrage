//! The extensions an entity, an archetype or a `.ytyp` carries, as one typed set.
//!
//! The declaration says `rage::fwExtensionDef` and the document says which of
//! 22 concrete types it holds. Every kind is identified by the hash of its
//! type name, so it reads the same with or without a schema snapshot, and a
//! type no row names is kept as [`ExtensionKind::Other`] with its fields still
//! readable through [`Extension::instance`] rather than dropped.
//!
//! ⚠ **A kind's field list is a claim about the files.**
//! `the_declarations_are_the_schemas_own_fields` holds each row to the schema
//! snapshot where one is staged, and `what_every_extension_in_an_install_carries`
//! holds it to every extension both retail installs ship. A typo in a name
//! changes its hash, so it fails there rather than reading as absent.

use glam::Vec3;
use or_io::hash::literal_string_hash;
use or_io::meta::{MetaInstance, Value};

use crate::field;

/// What a declared field holds, as the reader presents it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Float,
    /// Any unsigned width; `u8` fields read here too.
    U32,
    S32,
    Bool,
    Vec3,
    Vec4,
    /// `atHashString` and `atFinalHashString`.
    Hash,
    /// `atString`, which is text in the file rather than a hash.
    Str,
    /// `Color32`. ⚠ PSO and RBM have no colour type, so a binary document
    /// presents one as a plain unsigned word and only XML produces a colour.
    Color,
    Enum,
    Bitset,
    Array,
}

impl Shape {
    /// Whether `value` is what a field of this shape reads as.
    ///
    /// ⚠ Deliberately strict about the variant. The typed getters coerce, so
    /// a field declared `U32` that the reader produced as `Int` would still
    /// read — and would write back through a signed arm the file never had.
    pub fn admits(self, value: &Value) -> bool {
        matches!(
            (self, value),
            (Shape::Float, Value::Float(_))
                | (Shape::U32, Value::UInt(_))
                | (Shape::S32, Value::Int(_))
                | (Shape::Bool, Value::Bool(_))
                | (Shape::Vec3, Value::Vec3(_))
                | (Shape::Vec4, Value::Vec4(_))
                | (Shape::Hash, Value::Hash(_))
                | (Shape::Str, Value::String(_))
                | (Shape::Color, Value::Color(_) | Value::UInt(_))
                | (Shape::Enum, Value::Enum { .. })
                | (Shape::Bitset, Value::Bitset { .. })
                | (Shape::Array, Value::Array(_))
        )
    }

    /// The value a freshly authored field of this shape starts at.
    pub fn default_value(self) -> Value {
        match self {
            Shape::Float => Value::Float(0.0),
            Shape::U32 => Value::UInt(0),
            Shape::S32 => Value::Int(0),
            Shape::Bool => Value::Bool(false),
            Shape::Vec3 => Value::Vec3([0.0; 3]),
            Shape::Vec4 => Value::Vec4([0.0; 4]),
            Shape::Hash => Value::Hash(or_io::meta::instance::HashedString::from_hash(0)),
            Shape::Str => Value::String(String::new()),
            Shape::Color => Value::UInt(0),
            Shape::Enum => Value::Enum {
                type_name: String::new(),
                variant: String::new(),
            },
            Shape::Bitset => Value::Bitset {
                type_name: String::new(),
                bits: 0,
            },
            Shape::Array => Value::Array(Vec::new()),
        }
    }
}

/// One field a kind declares, beside the hash it is stored under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDecl {
    pub name: &'static str,
    pub hash: u32,
    pub shape: Shape,
}

const fn decl(name: &'static str, shape: Shape) -> FieldDecl {
    FieldDecl {
        name,
        hash: literal_string_hash(name.as_bytes()),
        shape,
    }
}

/// `rage::fwExtensionDef`'s one field, which every kind inherits.
pub const FW_EXTENSION_DEF: &[FieldDecl] = &[decl("name", Shape::Hash)];

/// `CExtensionDef`'s, which all but one kind inherit on top of it.
pub const C_EXTENSION_DEF: &[FieldDecl] = &[
    decl("name", Shape::Hash),
    decl("offsetPosition", Shape::Vec3),
];

macro_rules! getter_type {
    (Float) => { f32 };
    (U32) => { u32 };
    (S32) => { i32 };
    (Bool) => { bool };
    (Vec3) => { Vec3 };
    (Vec4) => { [f32; 4] };
    (Hash) => { u32 };
    (Str) => { String };
    (Color) => { u32 };
    (Enum) => { &'a str };
    (Bitset) => { u32 };
    (Array) => { &'a [Value] };
}

macro_rules! read_as {
    (Float, $inst:expr, $key:expr) => {
        field::f32_at($inst, $key)
    };
    (U32, $inst:expr, $key:expr) => {
        field::u32_at($inst, $key)
    };
    (S32, $inst:expr, $key:expr) => {
        field::i32_at($inst, $key)
    };
    (Bool, $inst:expr, $key:expr) => {
        field::bool_at($inst, $key)
    };
    (Vec3, $inst:expr, $key:expr) => {
        field::vec3_at($inst, $key).map(Vec3::from)
    };
    (Vec4, $inst:expr, $key:expr) => {
        field::vec4_at($inst, $key)
    };
    (Hash, $inst:expr, $key:expr) => {
        field::hash_at($inst, $key)
    };
    (Str, $inst:expr, $key:expr) => {
        field::string_at($inst, $key)
    };
    (Color, $inst:expr, $key:expr) => {
        field::u32_at($inst, $key)
    };
    (Enum, $inst:expr, $key:expr) => {
        match field::get($inst, $key) {
            Some(Value::Enum { variant, .. }) => Some(variant.as_str()),
            _ => None,
        }
    };
    (Bitset, $inst:expr, $key:expr) => {
        field::u32_at($inst, $key)
    };
    (Array, $inst:expr, $key:expr) => {
        match field::get($inst, $key) {
            Some(Value::Array(items)) => Some(items.as_slice()),
            _ => None,
        }
    };
}

/// The hash of a type name spelt as a binary document stores it: every `:`
/// of a namespace as `_`, which is `rage::x` read as `rage__x`.
const fn stored_type_hash(name: &[u8]) -> u32 {
    let mut key = 0u32;
    let mut i = 0;
    while i < name.len() {
        let byte = if name[i] == b':' { b'_' } else { name[i] };
        key = or_io::hash::partial_literal_hash(&[byte], key);
        i += 1;
    }
    or_io::hash::finalize_hash(key)
}

macro_rules! extension_kinds {
    ($(
        $(#[$doc:meta])*
        $kind:ident = $type_name:literal : $base:ident {
            $($getter:ident : $shape:ident = $field:literal),* $(,)?
        }
    ),* $(,)?) => {
        /// Which extension type an instance is.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ExtensionKind {
            $($(#[$doc])* $kind,)*
            /// A type no row names, kept by the hash it is stored under.
            Other(u32),
        }

        impl ExtensionKind {
            /// Every named kind, in declaration order.
            pub const KNOWN: &'static [ExtensionKind] = &[$(ExtensionKind::$kind),*];

            /// The kind a type hash names.
            ///
            /// ⚠ **A namespaced type has two hashes, and both mean it.** The
            /// file stores the hash of `rage__x`; a staged snapshot presents
            /// the type as `rage::x`, and [`field::name_hash`] hashes what it
            /// is shown. Matching only the second read all 60 shipped cloth
            /// bounds as an unnamed type whenever no snapshot was staged.
            pub fn of_type_hash(hash: u32) -> Self {
                $(
                    if hash == literal_string_hash($type_name.as_bytes())
                        || hash == stored_type_hash($type_name.as_bytes())
                    {
                        return ExtensionKind::$kind;
                    }
                )*
                ExtensionKind::Other(hash)
            }

            /// The type's name as the schema spells it, or `None` for [`Self::Other`].
            ///
            /// ⚠ Spelt `rage::x`, the schema's spelling, which is what the
            /// snapshot-free writer and the schema test both resolve.
            /// `rage__x` is the spelling the file hashes — see
            /// [`Self::stored_hash`].
            pub fn type_name(self) -> Option<&'static str> {
                match self {
                    $(ExtensionKind::$kind => Some($type_name),)*
                    ExtensionKind::Other(_) => None,
                }
            }

            pub fn type_hash(self) -> u32 {
                match self {
                    $(ExtensionKind::$kind => literal_string_hash($type_name.as_bytes()),)*
                    ExtensionKind::Other(hash) => hash,
                }
            }

            /// The number a binary document stores for this type.
            pub fn stored_hash(self) -> u32 {
                match self {
                    $(ExtensionKind::$kind => stored_type_hash($type_name.as_bytes()),)*
                    ExtensionKind::Other(hash) => hash,
                }
            }

            /// The fields this kind adds to the base it inherits.
            pub fn own_fields(self) -> &'static [FieldDecl] {
                match self {
                    $(ExtensionKind::$kind => {
                        const OWN: &[FieldDecl] = &[$(decl($field, Shape::$shape)),*];
                        OWN
                    })*
                    ExtensionKind::Other(_) => &[],
                }
            }

            /// The fields inherited from its base, in file order.
            pub fn base_fields(self) -> &'static [FieldDecl] {
                match self {
                    $(ExtensionKind::$kind => extension_kinds!(@base $base),)*
                    ExtensionKind::Other(_) => &[],
                }
            }

            /// Whether this kind inherits `CExtensionDef`'s offset.
            pub fn has_offset(self) -> bool {
                self.base_fields().len() == C_EXTENSION_DEF.len()
            }
        }

        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $kind<'a> {
                inst: &'a MetaInstance,
            }

            impl<'a> $kind<'a> {
                pub fn instance(&self) -> &'a MetaInstance {
                    self.inst
                }

                $(
                    pub fn $getter(&self) -> Option<getter_type!($shape)> {
                        const KEY: u32 = literal_string_hash($field.as_bytes());
                        read_as!($shape, self.inst, KEY)
                    }
                )*
            }
        )*

        /// An extension with its fields reachable by name.
        #[derive(Debug, Clone, Copy)]
        pub enum Typed<'a> {
            $($kind($kind<'a>),)*
            Other(&'a MetaInstance),
        }

        impl<'a> Extension<'a> {
            /// This extension as its own kind's view.
            pub fn typed(&self) -> Typed<'a> {
                match self.kind {
                    $(ExtensionKind::$kind => Typed::$kind($kind { inst: self.inst }),)*
                    ExtensionKind::Other(_) => Typed::Other(self.inst),
                }
            }
        }
    };
    (@base CExtensionDef) => { C_EXTENSION_DEF };
    (@base fwExtensionDef) => { FW_EXTENSION_DEF };
}

extension_kinds! {
    AudioCollisionSettings = "CExtensionDefAudioCollisionSettings" : CExtensionDef {
        settings: Hash = "settings",
    },
    AudioEmitter = "CExtensionDefAudioEmitter" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        effect_hash: U32 = "effectHash",
    },
    Buoyancy = "CExtensionDefBuoyancy" : CExtensionDef {},
    ClimbHandHold = "CExtensionDefClimbHandHold" : CExtensionDef {
        left: Vec3 = "left",
        right: Vec3 = "right",
        normal: Vec3 = "normal",
    },
    Decal = "CExtensionDefDecal" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        decal_name: Str = "decalName",
        decal_type: S32 = "decalType",
        bone_tag: S32 = "boneTag",
        scale: Float = "scale",
        probability: S32 = "probability",
        flags: S32 = "flags",
    },
    Door = "CExtensionDefDoor" : CExtensionDef {
        enable_limit_angle: Bool = "enableLimitAngle",
        starts_locked: Bool = "startsLocked",
        can_break: Bool = "canBreak",
        limit_angle: Float = "limitAngle",
        door_target_ratio: Float = "doorTargetRatio",
        audio_hash: Hash = "audioHash",
    },
    ExplosionEffect = "CExtensionDefExplosionEffect" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        explosion_name: Str = "explosionName",
        bone_tag: S32 = "boneTag",
        explosion_tag: S32 = "explosionTag",
        explosion_type: S32 = "explosionType",
        flags: U32 = "flags",
    },
    Expression = "CExtensionDefExpression" : CExtensionDef {
        expression_dictionary_name: Hash = "expressionDictionaryName",
        expression_name: Hash = "expressionName",
        creature_metadata_name: Hash = "creatureMetadataName",
        initialise_on_collision: Bool = "initialiseOnCollision",
    },
    Ladder = "CExtensionDefLadder" : CExtensionDef {
        bottom: Vec3 = "bottom",
        top: Vec3 = "top",
        normal: Vec3 = "normal",
        material_type: Enum = "materialType",
        template: Hash = "template",
        can_get_off_at_top: Bool = "canGetOffAtTop",
        can_get_off_at_bottom: Bool = "canGetOffAtBottom",
    },
    Light = "CExtensionDefLight" : CExtensionDef {},
    /// The light attributes a model's lights take, one per light.
    LightEffect = "CExtensionDefLightEffect" : CExtensionDef {
        instances: Array = "instances",
    },
    LightShaft = "CExtensionDefLightShaft" : CExtensionDef {
        corner_a: Vec3 = "cornerA",
        corner_b: Vec3 = "cornerB",
        corner_c: Vec3 = "cornerC",
        corner_d: Vec3 = "cornerD",
        direction: Vec3 = "direction",
        direction_amount: Float = "directionAmount",
        length: Float = "length",
        fade_in_time_start: Float = "fadeInTimeStart",
        fade_in_time_end: Float = "fadeInTimeEnd",
        fade_out_time_start: Float = "fadeOutTimeStart",
        fade_out_time_end: Float = "fadeOutTimeEnd",
        fade_distance_start: Float = "fadeDistanceStart",
        fade_distance_end: Float = "fadeDistanceEnd",
        color: Color = "color",
        intensity: Float = "intensity",
        flashiness: U32 = "flashiness",
        flags: U32 = "flags",
        density_type: Enum = "densityType",
        volume_type: Enum = "volumeType",
        softness: Float = "softness",
        scale_by_sun_intensity: Bool = "scaleBySunIntensity",
    },
    ParticleEffect = "CExtensionDefParticleEffect" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        fx_name: Str = "fxName",
        fx_type: S32 = "fxType",
        bone_tag: S32 = "boneTag",
        scale: Float = "scale",
        probability: S32 = "probability",
        flags: S32 = "flags",
        color: Color = "color",
    },
    ProcObject = "CExtensionDefProcObject" : CExtensionDef {
        radius_inner: Float = "radiusInner",
        radius_outer: Float = "radiusOuter",
        spacing: Float = "spacing",
        min_scale: Float = "minScale",
        max_scale: Float = "maxScale",
        min_scale_z: Float = "minScaleZ",
        max_scale_z: Float = "maxScaleZ",
        min_z_offset: Float = "minZOffset",
        max_z_offset: Float = "maxZOffset",
        object_hash: U32 = "objectHash",
        flags: U32 = "flags",
    },
    Script = "CExtensionDefScript" : CExtensionDef {
        script_name: Str = "scriptName",
        children: Array = "children",
    },
    Scrollbars = "CExtensionDefScrollbars" : CExtensionDef {
        height: Float = "height",
        scrollbars_type: S32 = "scrollbarsType",
        points: Array = "points",
    },
    SpawnPoint = "CExtensionDefSpawnPoint" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        spawn_type: Hash = "spawnType",
        ped_type: Hash = "pedType",
        group: Hash = "group",
        interior: Hash = "interior",
        required_imap: Hash = "requiredImap",
        available_in_mp_sp: Enum = "availableInMpSp",
        probability: Float = "probability",
        time_till_ped_leaves: Float = "timeTillPedLeaves",
        radius: Float = "radius",
        start: U32 = "start",
        end: U32 = "end",
        flags: Bitset = "flags",
        high_pri: Bool = "highPri",
        extended_range: Bool = "extendedRange",
        short_range: Bool = "shortRange",
    },
    SpawnPointOverride = "CExtensionDefSpawnPointOverride" : CExtensionDef {
        scenario_type: Hash = "ScenarioType",
        time_start_override: U32 = "iTimeStartOverride",
        time_end_override: U32 = "iTimeEndOverride",
        group: Hash = "Group",
        model_set: Hash = "ModelSet",
        availability_in_mp_sp: Enum = "AvailabilityInMpSp",
        flags: Bitset = "Flags",
        radius: Float = "Radius",
        time_till_ped_leaves: Float = "TimeTillPedLeaves",
    },
    SwayableEffect = "CExtensionDefSwayableEffect" : CExtensionDef {
        bone_tag: S32 = "boneTag",
        low_wind_speed: Float = "lowWindSpeed",
        low_wind_amplitude: Float = "lowWindAmplitude",
        high_wind_speed: Float = "highWindSpeed",
        high_wind_amplitude: Float = "highWindAmplitude",
    },
    WalkDontWalk = "CExtensionDefWalkDontWalk" : CExtensionDef {},
    WindDisturbance = "CExtensionDefWindDisturbance" : CExtensionDef {
        offset_rotation: Vec4 = "offsetRotation",
        disturbance_type: S32 = "disturbanceType",
        bone_tag: S32 = "boneTag",
        size: Vec4 = "size",
        strength: Float = "strength",
        flags: S32 = "flags",
    },
    /// Capsules a cloth collides with, attached to the model that owns it.
    ///
    /// ⚠ The one kind that inherits `rage::fwExtensionDef` directly, so it
    /// has a name and **no** `offsetPosition`.
    ClothCustomBounds = "rage::phVerletClothCustomBounds" : fwExtensionDef {
        collision_data: Array = "CollisionData",
    },
}

/// One extension, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct Extension<'a> {
    inst: &'a MetaInstance,
    kind: ExtensionKind,
}

impl<'a> Extension<'a> {
    pub fn new(inst: &'a MetaInstance) -> Self {
        Self {
            inst,
            kind: ExtensionKind::of_type_hash(field::name_hash(&inst.type_name)),
        }
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.inst
    }

    pub fn kind(&self) -> ExtensionKind {
        self.kind
    }

    pub fn name(&self) -> u32 {
        field::hash_at(self.inst, literal_string_hash(b"name")).unwrap_or(0)
    }

    /// Where the extension sits relative to what carries it, when its kind
    /// has an offset at all.
    pub fn offset_position(&self) -> Option<Vec3> {
        field::vec3_at(self.inst, literal_string_hash(b"offsetPosition")).map(Vec3::from)
    }

    /// How this instance departs from its kind's declaration.
    ///
    /// Empty for an [`ExtensionKind::Other`], which declares nothing.
    pub fn conformance(&self) -> Conformance {
        let mut out = Conformance::default();
        if matches!(self.kind, ExtensionKind::Other(_)) {
            return out;
        }
        let declared = || self.kind.base_fields().iter().chain(self.kind.own_fields());
        for decl in declared() {
            match field::get(self.inst, decl.hash) {
                None => out.missing.push(decl.name),
                Some(value) if !decl.shape.admits(value) => out.misshapen.push(decl.name),
                Some(_) => {}
            }
        }
        for (name, _) in &self.inst.fields {
            let hash = field::name_hash(name);
            if !declared().any(|d| d.hash == hash) {
                out.undeclared.push(hash);
            }
        }
        out
    }
}

/// Where one instance and its kind's declaration disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conformance {
    pub missing: Vec<&'static str>,
    pub misshapen: Vec<&'static str>,
    /// Fields present that the kind does not declare, by hash.
    pub undeclared: Vec<u32>,
}

impl Conformance {
    pub fn is_exact(&self) -> bool {
        self.missing.is_empty() && self.misshapen.is_empty() && self.undeclared.is_empty()
    }
}

impl ExtensionKind {
    /// A fresh instance of this kind with every declared field at its shape's default.
    ///
    /// ⚠ **Adding one to a document that holds no instance of the kind
    /// reaches the one word synthesis cannot produce.** The document's own
    /// layout has no class for it, so the write falls back to synthesis,
    /// and a class no donor names is written with a zero `signature` that no
    /// shipped file carries. Whether the game minds is unverified here.
    pub fn template(self) -> Option<MetaInstance> {
        let type_name = self.type_name()?;
        let mut inst = MetaInstance::new(type_name);
        for decl in self.base_fields().iter().chain(self.own_fields()) {
            inst.set(decl.name, decl.shape.default_value());
        }
        Some(inst)
    }
}

/// The extensions stored under `extensions` on any carrier.
pub(crate) fn extensions_of(inst: &MetaInstance) -> impl Iterator<Item = Extension<'_>> {
    field::structs_at(inst, crate::keys::EXTENSIONS).map(Extension::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::HashedString;
    use std::collections::HashMap;

    #[test]
    fn there_are_twenty_two_named_kinds_and_no_two_share_a_hash() {
        assert_eq!(ExtensionKind::KNOWN.len(), 22);
        let mut seen: HashMap<u32, ExtensionKind> = HashMap::new();
        for kind in ExtensionKind::KNOWN {
            if let Some(other) = seen.insert(kind.type_hash(), *kind) {
                panic!("{kind:?} and {other:?} share {:#010X}", kind.type_hash());
            }
            assert_eq!(ExtensionKind::of_type_hash(kind.type_hash()), *kind);
        }
    }

    #[test]
    fn a_kind_reads_under_either_spelling_of_its_type() {
        let named = MetaInstance::new("CExtensionDefDoor");
        let numbered = MetaInstance::new(format!(
            "0x{:08X}",
            literal_string_hash(b"CExtensionDefDoor")
        ));
        assert_eq!(Extension::new(&named).kind(), ExtensionKind::Door);
        assert_eq!(Extension::new(&numbered).kind(), ExtensionKind::Door);
    }

    /// ⚠ With no snapshot staged the reader names the cloth bounds by the
    /// spelling its hash is of, `rage__x`, and that has to be the same kind
    /// as the `rage::x` a snapshot presents. The stored hash is the one the
    /// snapshot-free name table attaches, so it is a number files carry.
    #[test]
    fn a_namespaced_kind_reads_under_the_spelling_the_file_hashes() {
        let kind = ExtensionKind::ClothCustomBounds;
        let public = or_io::meta::binary::hash_index::HashIndex::default().with_public_names();
        let presented = public
            .lookup(kind.stored_hash())
            .expect("the public table names the stored hash");
        assert_ne!(kind.stored_hash(), kind.type_hash());

        for spelling in [
            presented.to_string(),
            kind.type_name().unwrap().to_string(),
            format!("0x{:08X}", kind.stored_hash()),
        ] {
            let inst = MetaInstance::new(spelling.clone());
            assert_eq!(Extension::new(&inst).kind(), kind, "{spelling}");
        }

        for other in ExtensionKind::KNOWN.iter().filter(|k| **k != kind) {
            assert_eq!(other.stored_hash(), other.type_hash(), "{other:?}");
        }
    }

    #[test]
    fn an_unnamed_type_is_kept_by_its_hash() {
        let mut inst = MetaInstance::new("0x12345678");
        inst.set("0xAABBCCDD", Value::UInt(7));
        let ext = Extension::new(&inst);
        assert_eq!(ext.kind(), ExtensionKind::Other(0x1234_5678));
        assert!(matches!(ext.typed(), Typed::Other(_)));
        assert!(ext.conformance().is_exact());
        assert_eq!(ext.kind().template(), None);
    }

    #[test]
    fn a_typed_view_reads_its_own_fields() {
        let mut inst = MetaInstance::new("CExtensionDefDoor");
        inst.set("name", Value::Hash(HashedString::from_hash(0x42)));
        inst.set("offsetPosition", Value::Vec3([1.0, 2.0, 3.0]));
        inst.set("limitAngle", Value::Float(1.5));
        inst.set("startsLocked", Value::Bool(true));
        let ext = Extension::new(&inst);
        assert_eq!(ext.name(), 0x42);
        assert_eq!(ext.offset_position(), Some(Vec3::new(1.0, 2.0, 3.0)));
        let Typed::Door(door) = ext.typed() else {
            panic!("not a door");
        };
        assert_eq!(door.limit_angle(), Some(1.5));
        assert_eq!(door.starts_locked(), Some(true));
        assert_eq!(door.can_break(), None);
    }

    /// The census's instrument, falsified: each of its three findings fires.
    #[test]
    fn conformance_reports_missing_misshapen_and_undeclared_fields() {
        let mut inst = ExtensionKind::Door.template().unwrap();
        assert!(Extension::new(&inst).conformance().is_exact());

        inst.fields.retain(|(n, _)| n != "canBreak");
        inst.set("limitAngle", Value::Int(3));
        inst.set("somethingElse", Value::Bool(true));
        let c = Extension::new(&inst).conformance();
        assert_eq!(c.missing, vec!["canBreak"]);
        assert_eq!(c.misshapen, vec!["limitAngle"]);
        assert_eq!(c.undeclared, vec![literal_string_hash(b"somethingElse")]);
    }

    #[test]
    fn every_template_conforms_to_its_own_declaration() {
        for kind in ExtensionKind::KNOWN {
            let inst = kind.template().unwrap();
            assert!(Extension::new(&inst).conformance().is_exact(), "{kind:?}");
            assert_eq!(Extension::new(&inst).kind(), *kind);
        }
    }

    #[test]
    fn only_the_cloth_bounds_has_no_offset() {
        let without: Vec<_> = ExtensionKind::KNOWN
            .iter()
            .filter(|k| !k.has_offset())
            .collect();
        assert_eq!(without, vec![&ExtensionKind::ClothCustomBounds]);
    }

    /// ⚠ The declarations are the schema's, or this module is a second model
    /// of the file. Skipped, and recorded, where no snapshot is staged.
    #[test]
    fn the_declarations_are_the_schemas_own_fields() {
        let lib = crate::test_schema();
        let Some(base) = lib.resolve("rage::fwExtensionDef") else {
            or_test_support::skip_corpus(
                "the_declarations_are_the_schemas_own_fields",
                "the installed schema library has no rage::fwExtensionDef; \
                 see crates/io/or-io/src/meta/README.md",
            );
            return;
        };

        let mut schema_kinds: Vec<String> = lib
            .descendants_of(&base.def.name)
            .into_iter()
            .filter(|s| s.name != "CExtensionDef")
            .map(|s| s.name.clone())
            .collect();
        schema_kinds.sort();
        let mut ours: Vec<String> = ExtensionKind::KNOWN
            .iter()
            .map(|k| k.type_name().unwrap().to_string())
            .collect();
        ours.sort();
        assert_eq!(ours, schema_kinds, "the set is not the schema's set");

        for kind in ExtensionKind::KNOWN {
            let name = kind.type_name().unwrap();
            let resolved = lib.resolve(name).expect(name);
            let schema: Vec<(&str, Shape)> = resolved
                .fields
                .iter()
                .filter(|f| !matches!(f.ty, or_io::meta::schema::FieldType::Pad(_)))
                .map(|f| (f.name.as_str(), shape_of(&f.ty)))
                .collect();
            let declared: Vec<(&str, Shape)> = kind
                .base_fields()
                .iter()
                .chain(kind.own_fields())
                .map(|d| (d.name, d.shape))
                .collect();
            assert_eq!(declared, schema, "{kind:?}");
        }
    }

    fn shape_of(ty: &or_io::meta::schema::FieldType) -> Shape {
        use or_io::meta::schema::{FieldType as T, StringKind};
        match ty {
            T::Float | T::Float16 | T::Double => Shape::Float,
            T::U8 | T::U16 | T::U32 | T::U64 => Shape::U32,
            T::S8 | T::S16 | T::S32 | T::S64 => Shape::S32,
            T::Bool => Shape::Bool,
            T::Vector3 | T::Vec3V => Shape::Vec3,
            T::Vector4 | T::Vec4V => Shape::Vec4,
            T::String(StringKind::Plain) | T::String(StringKind::AtString) => Shape::Str,
            T::String(_) => Shape::Hash,
            T::Color32 => Shape::Color,
            T::Enum(_) => Shape::Enum,
            T::Bitset { .. } => Shape::Bitset,
            T::Array { .. } => Shape::Array,
            other => panic!("no shape for {other:?}"),
        }
    }
}
