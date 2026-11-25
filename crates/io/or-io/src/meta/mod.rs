pub mod binary;
pub mod canonical;
pub mod diagnostics;
pub mod error;
pub mod field_hints;
pub mod instance;
pub mod par_tree;
pub mod schema;
pub mod shipped;
pub mod snapshot;
pub mod snapshot_discovery;
pub mod srl;
pub mod typed;

pub use binary::{detect_binary_format, write_meta_binary, BinaryFormat};
pub use diagnostics::{Diagnostics, ErrorMode, Gap, GapKind};
pub use error::{MetaError, Result, UnsupportedShape};
pub use instance::{write_meta, MetaDocument, MetaInstance, SourceFormat, Value};
pub use schema::{
    build_library, consistent_struct_short_name, convert_datatype_to_friendly_name,
    normalize_member_name, ArrayKind, BitsetStorage, EnumDef, EnumValue, Field, FieldHint,
    FieldType, Literal, PointerPolicy, ResolvedStruct, SchemaLibrary, StringKind, StructDef,
    TypeName,
};
pub use snapshot::{
    builder_stamp, decode as decode_snapshot, default_library, default_origin,
    encode as encode_snapshot, has_default_library, has_names_library, install_default_library,
    install_names_library, install_names_snapshot, install_snapshot, load_snapshot, snapshot_age,
    SnapshotAge,
};
pub use typed::{
    Anchor, CAlternateVariations, CAnchorProps, CBaseArchetypeDef, CBaseArchetypeDefAny,
    CComponentInfo, CCreatureMetaData, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
    CMloArchetypeDef, CMloInstanceDef, CPVClothComponentData, CPVComponentData, CPVDrawblData,
    CPVTextureData, CPedCompExpressionData, CPedPropExpressionData, CPedPropInfo, CPedPropMetaData,
    CPedPropTexData, CPedSelectionSet, CPedVariationInfo, CScenarioChain, CScenarioChainingEdge,
    CScenarioChainingGraph, CScenarioChainingNode, CScenarioPoint, CScenarioPointCluster,
    CScenarioPointContainer, CScenarioPointLookUps, CScenarioPointRegion, CShaderVariableComponent,
    CTimeArchetypeDef, FirstPersonAlternate, FirstPersonAlternateData,
    SAlternateVariationComponent, SAlternateVariationPed, SAlternateVariationSwitch,
};

/// Whether `root` names a struct `library` declares.
pub fn is_known_root(library: &SchemaLibrary, root: &str) -> bool {
    library.structs.contains_key(root)
        || library
            .structs
            .contains_key(&convert_datatype_to_friendly_name(root))
}
