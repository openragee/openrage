//! Names for the world metadata family, attached to hashes real files carry.
//!
//! PSO and RBM store every type and every field as a hash of its C++ name, and
//! resolving one needs the gitignored schema snapshot: without it a `.ymap`
//! decodes correctly and reads as `<0xCCAB9307>` rather than `<entities>`.
//!
//! ⚠ A hand-written list is safe because a hash is a name's fingerprint: an
//! entry attaches only when `literal_string_hash(string) == number`, so a
//! wrong string matches nothing rather than misleading. Every entry is a name
//! CodeWalker publishes in `MetaTypes/MetaNames.cs` whose hash a retail
//! install ships. The residual risk is a genuine 32-bit collision against a
//! name the list does not hold; `no_two_names_claim_the_same_hash` rules one
//! out only within the list.
//!
//! [`crate::meta::snapshot`]'s library always wins, so staging a snapshot replaces
//! these rather than competing with them. A 20,186-name table from the same
//! source was deleted: with it a build with no snapshot resolved 8,525 of the
//! 9,629 name-hashes a 6,071-document corpus carries, and without it 122 —
//! the world family and nothing else. The five hashes only that table named
//! are in the list below; the 505 it spelled differently from the library
//! did not move, because the library already won every disagreement.
//!
//! The manifest keys are here because `or_world_gta5::keys` resolves without a
//! library; `every_key_is_a_name_meta_publishes` breaks if one goes.
//!
//! These names reach the write path: the binary writers resolve a field's
//! name from the same index the reader used and look it up in the instance by
//! that name, so naming only one side would be the bug.

use std::collections::HashSet;

/// Names for the `CMapData` / `CMapTypes` families and everything they reach,
/// in byte order of the string.
pub const KNOWN: &[&str] = &[
    "AnimDict",
    "AnimName",
    "AnimatedModel",
    "Animations",
    "Ao",
    "AvailabilityInMpSp",
    "BatchAABB",
    "Bounds",
    "BoxOccluder",
    "CBaseArchetypeDef",
    "CBlockDesc",
    "CCarGen",
    "CCompEntityAnims",
    "CCompEntityEffectsData",
    "CCompositeEntityType",
    "CDistantLODLight",
    "CEntityDef",
    "CExtensionDefAudioCollisionSettings",
    "CExtensionDefAudioEmitter",
    "CExtensionDefBuoyancy",
    "CExtensionDefDoor",
    "CExtensionDefExplosionEffect",
    "CExtensionDefExpression",
    "CExtensionDefLadder",
    "CExtensionDefLightEffect",
    "CExtensionDefLightShaft",
    "CExtensionDefParticleEffect",
    "CExtensionDefProcObject",
    "CExtensionDefSpawnPoint",
    "CExtensionDefSpawnPointOverride",
    "CExtensionDefWindDisturbance",
    "CLODLight",
    "CLightAttrDef",
    "CMapData",
    "CMapDataGroup",
    "CMapTypes",
    "CMloArchetypeDef",
    "CMloEntitySet",
    "CMloInstanceDef",
    "CMloPortalDef",
    "CMloRoomDef",
    "CMloTimeCycleModifier",
    "CTimeArchetypeDef",
    "CTimeCycleModifier",
    "CapsuleHalfHeight",
    "CapsuleHalfWidth",
    "CapsuleLen",
    "CapsuleRadius",
    "CollisionData",
    "Color",
    "Content",
    "DistantLODLightsSOA",
    "EndImapFile",
    "EndModel",
    "Flags",
    "FloatXYZ",
    "GrassInstanceList",
    "Group",
    "HDTxd",
    "HDTxdBindingArray",
    "HoursOnOff",
    "ImapLink",
    "InstanceList",
    "Interiors",
    "LODLights",
    "LODLightsSOA",
    "LodFadeStartDist",
    "LodInstFadeRange",
    "MLOInstflags",
    "MapDataGroups",
    "ModelSet",
    "Name",
    "Normal",
    "NormalX",
    "NormalY",
    "OccludeModel",
    "OrientToTerrain",
    "OwnerName",
    "Pad",
    "PlateProbabilities",
    "Position",
    "PropInstanceList",
    "PtFxAssetName",
    "RGBI",
    "Radius",
    "Rotation",
    "Scale",
    "ScaleRange",
    "ScenarioType",
    "StartImapFile",
    "StartModel",
    "TimeTillPedLeaves",
    "WeatherTypes",
    "ambientOcclusionMultiplier",
    "archetypeName",
    "archetypes",
    "artificialAmbientOcclusion",
    "assetName",
    "assetType",
    "attachedObjects",
    "audioHash",
    "audioOcclusion",
    "availableInMpSp",
    "bbMax",
    "bbMin",
    "blend",
    "block",
    "bmax",
    "bmin",
    "bodyColorRemap1",
    "bodyColorRemap2",
    "bodyColorRemap3",
    "bodyColorRemap4",
    "boneTag",
    "bottom",
    "boxOccluders",
    "bsCentre",
    "bsRadius",
    "canBreak",
    "canGetOffAtBottom",
    "canGetOffAtTop",
    "carGenerators",
    "carModel",
    "category",
    "childLodDist",
    "clipDictionary",
    "color",
    "colour",
    "compositeEntityTypes",
    "coneInnerAngle",
    "coneOuterAngle",
    "coneOuterAngleOrCapExt",
    "containerLods",
    "contentFlags",
    "cornerA",
    "cornerB",
    "cornerC",
    "cornerD",
    "corners",
    "coronaIntensity",
    "coronaSize",
    "coronaZBias",
    "creatureMetadataName",
    "cullingPlane",
    "dataSize",
    "defaultEntitySets",
    "densityType",
    "dependencies",
    "direction",
    "directionAmount",
    "disturbanceType",
    "doorTargetRatio",
    "drawableDictionary",
    "effectHash",
    "effectsData",
    "enableLimitAngle",
    "end",
    "endHour",
    "endPhase",
    "entities",
    "entitiesExtentsMax",
    "entitiesExtentsMin",
    "entitySets",
    "explosionName",
    "explosionTag",
    "explosionType",
    "exportedBy",
    "expressionDictionaryName",
    "expressionName",
    "extendedRange",
    "extensions",
    "extents",
    "exteriorVisibiltyDepth",
    "fadeDistanceEnd",
    "fadeDistanceStart",
    "fadeInTimeEnd",
    "fadeInTimeStart",
    "fadeOutTimeEnd",
    "fadeOutTimeStart",
    "falloff",
    "falloffExponent",
    "flags",
    "flashiness",
    "floorId",
    "fxName",
    "fxOffsetPos",
    "fxOffsetRot",
    "fxType",
    "group",
    "groupId",
    "guid",
    "hash",
    "hdTextureDist",
    "highPri",
    "iCenterX",
    "iCenterY",
    "iCenterZ",
    "iCosZ",
    "iHeight",
    "iLength",
    "iSinZ",
    "iTimeEndOverride",
    "iTimeStartOverride",
    "iWidth",
    "imapDependencies",
    "imapDependencies_2",
    "imapName",
    "initialiseOnCollision",
    "instancedData",
    "instances",
    "intensity",
    "interior",
    "itypDepArray",
    "itypDependencies_2",
    "itypName",
    "length",
    "lightFadeDistance",
    "lightGroup",
    "lightHash",
    "lightType",
    "limitAngle",
    "livery",
    "locations",
    "lodDist",
    "lodLevel",
    "materialType",
    "max",
    "maxExtents",
    "maxScale",
    "maxScaleZ",
    "maxZOffset",
    "min",
    "minExtents",
    "minScale",
    "minScaleZ",
    "minZOffset",
    "mirrorPriority",
    "mloFlags",
    "name",
    "normal",
    "numChildren",
    "numExitPortals",
    "numStreetLights",
    "numTris",
    "numVertsInBytes",
    "objectHash",
    "occludeModels",
    "offsetPosition",
    "offsetRotation",
    "opacity",
    "orientX",
    "orientY",
    "owner",
    "padding1",
    "padding2",
    "padding3",
    "parent",
    "parentIndex",
    "pedType",
    "percentage",
    "perpendicularLength",
    "physicsDictionaries",
    "physicsDictionary",
    "popGroup",
    "portalCount",
    "portals",
    "position",
    "posn",
    "priorityLevel",
    "probability",
    "projectedTextureKey",
    "ptFxHasTint",
    "ptFxIsTriggered",
    "ptFxProbability",
    "ptFxScale",
    "ptFxSize",
    "ptFxTag",
    "ptFxTintB",
    "ptFxTintG",
    "ptFxTintR",
    "punchInPhase",
    "punchOutPhase",
    "radius",
    "radiusInner",
    "radiusOuter",
    "rage__fwGrassInstanceListDef",
    "rage__fwGrassInstanceListDef__InstanceData",
    "rage__fwInstancedMapData",
    "rage__phCapsuleBoundDef",
    "rage__phVerletClothCustomBounds",
    "rage__spdAABB",
    "range",
    "requiredImap",
    "roomFrom",
    "roomTo",
    "rooms",
    "rotation",
    "scale",
    "scaleBySunIntensity",
    "scaleXY",
    "scaleZ",
    "secondaryTimecycleName",
    "settings",
    "shadowBlur",
    "shadowFadeDistance",
    "shadowNearClip",
    "shortRange",
    "size",
    "softness",
    "spacing",
    "spawnType",
    "specialAttribute",
    "specularFadeDistance",
    "sphere",
    "start",
    "startHour",
    "startPhase",
    "startsLocked",
    "strRecordedRequest",
    "strRequestRecording",
    "strResourceReference",
    "streamingExtentsMax",
    "streamingExtentsMin",
    "strength",
    "tangent",
    "targetAsset",
    "template",
    "textureDictionary",
    "time",
    "timeAndStateFlags",
    "timeCycleModifiers",
    "timeFlags",
    "timeTillPedLeaves",
    "timecycleName",
    "tintValue",
    "top",
    "txdRelationships",
    "version",
    "verts",
    "volIntensity",
    "volOuterColour",
    "volOuterExponent",
    "volOuterIntensity",
    "volSizeScale",
    "volumeType",
    "volumetricFadeDistance",
    "x",
    "y",
    "z",
];

/// Every publicly-known name, from here and from [`super::root_types`].
pub fn all() -> impl Iterator<Item = &'static str> {
    KNOWN
        .iter()
        .copied()
        .chain(super::root_types::KNOWN.iter().copied())
}

/// Names this list claims that a caller's own hashes do not contain; an
/// unmatched name is harmless, so this is a report rather than a check.
pub fn unmatched_against(hashes: &HashSet<u32>) -> Vec<&'static str> {
    all()
        .filter(|name| !hashes.contains(&crate::hash::literal_string_hash(name.as_bytes())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::literal_string_hash;
    use std::collections::HashMap;

    /// Each number is one a retail Legacy install ships in a `.ymap` or
    /// `.ytyp`, beside the string that hashes to it.
    #[test]
    fn every_pinned_name_hashes_to_the_number_the_game_ships() {
        for (name, expected) in [
            ("CMapData", 0xD359_3FA6u32),
            ("CMapTypes", 0xD98B_B561),
            ("CEntityDef", 0xCE50_1483),
            ("CBaseArchetypeDef", 0x82D6_FC83),
            ("CMloArchetypeDef", 0x1050_6455),
            ("CMloRoomDef", 0x0B18_11F1),
            ("CMloPortalDef", 0x9950_72CA),
            ("CMloEntitySet", 0xD6A7_99F9),
            ("CMloInstanceDef", 0x09CC_28BE),
            ("entities", 0xCCAB_9307),
            ("archetypes", 0x018A_3B1B),
            ("archetypeName", 0xA023_A02C),
            ("position", 0x0116_6164),
            ("rotation", 0x77DD_4010),
            ("streamingExtentsMin", 0xDD22_821F),
            ("streamingExtentsMax", 0xA22E_A335),
            ("entitiesExtentsMin", 0x1C75_BCF1),
            ("entitiesExtentsMax", 0x6D07_4437),
            ("contentFlags", 0x6A67_5035),
            ("parentIndex", 0xD892_31BD),
            ("numChildren", 0xA687_AC89),
            ("lodLevel", 0x6C8F_1715),
            ("lodDist", 0xBF74_DFA7),
            ("childLodDist", 0xCA97_4BCD),
            ("rooms", 0xCD20_D7A8),
            ("portals", 0x89F7_E992),
            ("entitySets", 0x45BC_B930),
            ("corners", 0x4F42_AEB3),
            ("attachedObjects", 0x8E05_312C),
        ] {
            assert_eq!(literal_string_hash(name.as_bytes()), expected, "{name}");
        }
    }

    /// ⚠ The spelling is the game's, typo included.
    ///
    /// `exteriorVisibiltyDepth` is missing an `i`, and the corrected spelling
    /// hashes somewhere else entirely — so "fixing" it silently unnames the
    /// field.
    #[test]
    fn a_misspelt_field_keeps_the_misspelling() {
        assert_eq!(
            literal_string_hash(b"exteriorVisibiltyDepth"),
            0x20F3_D23E,
            "the hash a CMloRoomDef actually carries"
        );
        assert_ne!(literal_string_hash(b"exteriorVisibilityDepth"), 0x20F3_D23E);
        assert!(KNOWN.contains(&"exteriorVisibiltyDepth"));
        assert!(!KNOWN.contains(&"exteriorVisibilityDepth"));
    }

    /// Case matters, and two of these pairs are live in the same corpus:
    /// `CEntityDef.flags` beside `CExtensionDefSpawnPoint.Flags`, and
    /// `color` beside `Color`. A case-insensitive dedupe drops one of each.
    #[test]
    fn names_differing_only_in_case_are_both_kept() {
        for (lower, upper) in [
            ("flags", "Flags"),
            ("color", "Color"),
            ("name", "Name"),
            ("normal", "Normal"),
            ("group", "Group"),
            ("radius", "Radius"),
            ("rotation", "Rotation"),
            ("scale", "Scale"),
            ("position", "Position"),
        ] {
            assert!(KNOWN.contains(&lower), "{lower}");
            assert!(KNOWN.contains(&upper), "{upper}");
            assert_ne!(
                literal_string_hash(lower.as_bytes()),
                literal_string_hash(upper.as_bytes()),
                "{lower} / {upper}"
            );
        }
    }

    #[test]
    fn no_two_names_claim_the_same_hash() {
        let mut seen: HashMap<u32, &str> = HashMap::new();
        for name in all() {
            let hash = literal_string_hash(name.as_bytes());
            if let Some(other) = seen.insert(hash, name) {
                assert_eq!(other, name, "{other} and {name} both hash to {hash:#010X}");
            }
        }
    }

    #[test]
    fn the_list_holds_no_duplicates() {
        let distinct: HashSet<&str> = KNOWN.iter().copied().collect();
        assert_eq!(distinct.len(), KNOWN.len());
    }

    #[test]
    fn the_list_is_in_byte_order() {
        let mut sorted = KNOWN.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KNOWN);
    }

    /// ⚠ The five the retired CodeWalker table was the only source for.
    ///
    /// Four are hashes the metadata corpus carries and the fifth roots
    /// documents outside it. Deleting one unnames a real field, and nothing
    /// else in this crate would fail.
    #[test]
    fn the_residue_of_the_retired_name_table_is_here() {
        for name in [
            "Content",
            "PlateProbabilities",
            "strRecordedRequest",
            "strRequestRecording",
            "strResourceReference",
        ] {
            assert!(KNOWN.contains(&name), "{name}");
        }
    }

    #[test]
    fn unmatched_reports_what_a_caller_did_not_see() {
        let mut hashes = HashSet::new();
        hashes.insert(literal_string_hash(b"entities"));
        let missing = unmatched_against(&hashes);
        assert!(!missing.contains(&"entities"));
        assert!(missing.contains(&"archetypes"));
    }
}
