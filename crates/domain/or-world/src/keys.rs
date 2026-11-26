//! The field names this crate reads, as the numbers they are stored under.
//!
//! Each constant is `literal_string_hash` of its own name evaluated at
//! compile time, so there is no second table of numbers to keep in step and
//! a typo changes the constant rather than going unnoticed. The names
//! themselves are [`or_io::meta::binary::public_names`]' business; this module
//! only says which of them the world model needs.

use or_io::hash::literal_string_hash;

macro_rules! keys {
    ($($ident:ident => $name:literal),* $(,)?) => {
        $(pub const $ident: u32 = literal_string_hash($name.as_bytes());)*

        /// Every key here, beside the name it hashes.
        pub const ALL: &[(&str, u32)] = &[$(($name, $ident)),*];
    };
}

keys! {
    // CMapData
    NAME => "name",
    PARENT => "parent",
    FLAGS => "flags",
    CONTENT_FLAGS => "contentFlags",
    STREAMING_EXTENTS_MIN => "streamingExtentsMin",
    STREAMING_EXTENTS_MAX => "streamingExtentsMax",
    ENTITIES_EXTENTS_MIN => "entitiesExtentsMin",
    ENTITIES_EXTENTS_MAX => "entitiesExtentsMax",
    ENTITIES => "entities",
    CONTAINER_LODS => "containerLods",
    BOX_OCCLUDERS => "boxOccluders",
    OCCLUDE_MODELS => "occludeModels",
    PHYSICS_DICTIONARIES => "physicsDictionaries",
    INSTANCED_DATA => "instancedData",
    TIME_CYCLE_MODIFIERS => "timeCycleModifiers",
    CAR_GENERATORS => "carGenerators",
    LOD_LIGHTS_SOA => "LODLightsSOA",
    DISTANT_LOD_LIGHTS_SOA => "DistantLODLightsSOA",
    BLOCK => "block",

    // CEntityDef, and CMloInstanceDef on top of it
    ARCHETYPE_NAME => "archetypeName",
    GUID => "guid",
    POSITION => "position",
    ROTATION => "rotation",
    SCALE_XY => "scaleXY",
    SCALE_Z => "scaleZ",
    PARENT_INDEX => "parentIndex",
    LOD_DIST => "lodDist",
    CHILD_LOD_DIST => "childLodDist",
    LOD_LEVEL => "lodLevel",
    NUM_CHILDREN => "numChildren",
    PRIORITY_LEVEL => "priorityLevel",
    EXTENSIONS => "extensions",
    TINT_VALUE => "tintValue",
    GROUP_ID => "groupId",
    FLOOR_ID => "floorId",
    DEFAULT_ENTITY_SETS => "defaultEntitySets",
    NUM_EXIT_PORTALS => "numExitPortals",
    MLO_INST_FLAGS => "MLOInstflags",

    // CMapTypes and the archetype family
    ARCHETYPES => "archetypes",
    DEPENDENCIES => "dependencies",
    COMPOSITE_ENTITY_TYPES => "compositeEntityTypes",
    BB_MIN => "bbMin",
    BB_MAX => "bbMax",
    BS_CENTRE => "bsCentre",
    BS_RADIUS => "bsRadius",
    HD_TEXTURE_DIST => "hdTextureDist",
    SPECIAL_ATTRIBUTE => "specialAttribute",
    TEXTURE_DICTIONARY => "textureDictionary",
    CLIP_DICTIONARY => "clipDictionary",
    DRAWABLE_DICTIONARY => "drawableDictionary",
    PHYSICS_DICTIONARY => "physicsDictionary",
    ASSET_TYPE => "assetType",
    ASSET_NAME => "assetName",
    TIME_FLAGS => "timeFlags",

    // CMloArchetypeDef and its children
    MLO_FLAGS => "mloFlags",
    ROOMS => "rooms",
    PORTALS => "portals",
    ENTITY_SETS => "entitySets",
    BLEND => "blend",
    TIMECYCLE_NAME => "timecycleName",
    SECONDARY_TIMECYCLE_NAME => "secondaryTimecycleName",
    ATTACHED_OBJECTS => "attachedObjects",
    ROOM_FROM => "roomFrom",
    ROOM_TO => "roomTo",
    OPACITY => "opacity",
    CORNERS => "corners",
    LOCATIONS => "locations",

    // Occluders, generators and instanced data
    I_CENTER_X => "iCenterX",
    I_CENTER_Y => "iCenterY",
    I_CENTER_Z => "iCenterZ",
    I_LENGTH => "iLength",
    I_WIDTH => "iWidth",
    I_HEIGHT => "iHeight",
    BMIN => "bmin",
    BMAX => "bmax",
    PERPENDICULAR_LENGTH => "perpendicularLength",
    ORIENT_X => "orientX",
    ORIENT_Y => "orientY",
    CAR_MODEL => "carModel",
    BODY_COLOR_REMAP_1 => "bodyColorRemap1",
    BODY_COLOR_REMAP_2 => "bodyColorRemap2",
    BODY_COLOR_REMAP_3 => "bodyColorRemap3",
    BODY_COLOR_REMAP_4 => "bodyColorRemap4",
    POP_GROUP => "popGroup",
    LIVERY => "livery",
    GRASS_INSTANCE_LIST => "GrassInstanceList",
    PROP_INSTANCE_LIST => "PropInstanceList",
    INSTANCE_LIST => "InstanceList",
    BATCH_AABB => "BatchAABB",
    MIN => "min",
    MAX => "max",

    // CTimeCycleModifier
    MIN_EXTENTS => "minExtents",
    MAX_EXTENTS => "maxExtents",
    PERCENTAGE => "percentage",
    RANGE => "range",
    START_HOUR => "startHour",
    END_HOUR => "endHour",

    // CLODLight, CDistantLODLight and the FloatXYZ both hold
    DIRECTION => "direction",
    FALLOFF => "falloff",
    FALLOFF_EXPONENT => "falloffExponent",
    TIME_AND_STATE_FLAGS => "timeAndStateFlags",
    HASH => "hash",
    CONE_INNER_ANGLE => "coneInnerAngle",
    CONE_OUTER_ANGLE_OR_CAP_EXT => "coneOuterAngleOrCapExt",
    CORONA_INTENSITY => "coronaIntensity",
    RGBI => "RGBI",
    NUM_STREET_LIGHTS => "numStreetLights",
    CATEGORY => "category",
    X => "x",
    Y => "y",
    Z => "z",

    // CPackFileMetaData: the manifest
    MAP_DATA_GROUPS => "MapDataGroups",
    HD_TXD_BINDING_ARRAY => "HDTxdBindingArray",
    TARGET_ASSET => "targetAsset",
    HD_TXD => "HDTxd",
    IMAP_DEPENDENCIES => "imapDependencies",
    IMAP_DEPENDENCIES_2 => "imapDependencies_2",
    IMAP_NAME => "imapName",
    ITYP_NAME => "itypName",
    ITYP_DEP_ARRAY => "itypDepArray",

    // CMapDataGroup: a world-state group
    GROUP_NAME => "Name",
    GROUP_BOUNDS => "Bounds",
    GROUP_FLAGS => "Flags",
    WEATHER_TYPES => "WeatherTypes",
    HOURS_ON_OFF => "HoursOnOff",
}

/// The type names this crate dispatches on.
pub mod types {
    pub const C_MAP_DATA: &str = "CMapData";
    pub const C_MAP_TYPES: &str = "CMapTypes";
    pub const C_ENTITY_DEF: &str = "CEntityDef";
    pub const C_MLO_INSTANCE_DEF: &str = "CMloInstanceDef";
    pub const C_BASE_ARCHETYPE_DEF: &str = "CBaseArchetypeDef";
    pub const C_TIME_ARCHETYPE_DEF: &str = "CTimeArchetypeDef";
    pub const C_MLO_ARCHETYPE_DEF: &str = "CMloArchetypeDef";
    pub const C_PACK_FILE_META_DATA: &str = "CPackFileMetaData";
    pub const C_MAP_DATA_GROUP: &str = "CMapDataGroup";
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// ⚠ Known answers, so a change to the hash function is caught here
    /// rather than as a `.ymap` that suddenly has no entities.
    #[test]
    fn the_pinned_keys_are_the_numbers_a_map_file_stores() {
        assert_eq!(ENTITIES, 0xCCAB_9307);
        assert_eq!(ARCHETYPES, 0x018A_3B1B);
        assert_eq!(POSITION, 0x0116_6164);
        assert_eq!(ROTATION, 0x77DD_4010);
        assert_eq!(PARENT_INDEX, 0xD892_31BD);
        assert_eq!(STREAMING_EXTENTS_MIN, 0xDD22_821F);
    }

    /// Two keys sharing a number would make one field shadow the other, and
    /// the shadowed one would read as absent rather than as an error.
    #[test]
    fn no_two_keys_collide() {
        let mut seen: HashMap<u32, &str> = HashMap::new();
        for (name, key) in ALL {
            if let Some(other) = seen.insert(*key, name) {
                assert_eq!(other, *name, "{other} and {name} share {key:#010X}");
            }
        }
    }

    /// Every key must be one [`or_io::meta`] can name, or the app shows the
    /// number for a field this crate is reading by name.
    #[test]
    fn every_key_is_a_name_meta_publishes() {
        let index = or_io::meta::binary::hash_index::HashIndex::default().with_public_names();
        for (name, key) in ALL {
            assert_eq!(index.lookup(*key), Some(*name), "{name}");
        }
    }
}
