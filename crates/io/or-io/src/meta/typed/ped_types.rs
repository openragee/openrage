//! The ped variation family's types, declared here so a record built from nothing saves where no
//! schema snapshot is installed, which is every user's machine.
//!
//! Transcribed in the PSC's dialect from the type tables of the staged documents, and
//! `every_declared_ped_class_is_the_table_the_documents_state` holds every class, member, offset,
//! size and signature, and every enumerator, to the 1,832 variation and 9 creature documents that
//! state them. Every staged document states one table for each class.
//!
//! ⚠ A `<pad>` is bytes of a class the table gives no member to: the tails of
//! `CPVDrawblData::CPVClothComponentData` (23) and `CPVTextureData` (1). Without them a synthesised
//! table is a smaller one that nothing shipped. The four classes that are not `simple` start with
//! the vtable word the synthesiser adds, so they declare no pad for it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::meta::binary::rbm::schema::{RbmClass, RbmEnum, RbmModel};
use crate::meta::binary::synth::name_hash;
use crate::meta::schema::{parse_psc, Field, FieldType, SchemaItem, SchemaLibrary, StructDef};

pub const PED_TYPES: &str = r#"<ParserSchema>
  <enumdef type="eAnchorPoints">
    <enumval name="ANCHOR_HEAD" value="0"/>
    <enumval name="ANCHOR_EYES" value="1"/>
    <enumval name="ANCHOR_EARS" value="2"/>
    <enumval name="ANCHOR_MOUTH" value="3"/>
    <enumval name="ANCHOR_LEFT_HAND" value="4"/>
    <enumval name="ANCHOR_RIGHT_HAND" value="5"/>
    <enumval name="ANCHOR_LEFT_WRIST" value="6"/>
    <enumval name="ANCHOR_RIGHT_WRIST" value="7"/>
    <enumval name="ANCHOR_HIP" value="8"/>
    <enumval name="ANCHOR_LEFT_FOOT" value="9"/>
    <enumval name="ANCHOR_RIGHT_FOOT" value="10"/>
    <enumval name="ANCHOR_PH_L_HAND" value="11"/>
    <enumval name="ANCHOR_PH_R_HAND" value="12"/>
    <enumval name="NUM_ANCHORS" value="13"/>
  </enumdef>
  <enumdef type="ePedVarComp">
    <enumval name="PV_COMP_INVALID" value="-1"/>
    <enumval name="PV_COMP_HEAD" value="0"/>
    <enumval name="PV_COMP_BERD" value="1"/>
    <enumval name="PV_COMP_HAIR" value="2"/>
    <enumval name="PV_COMP_UPPR" value="3"/>
    <enumval name="PV_COMP_LOWR" value="4"/>
    <enumval name="PV_COMP_HAND" value="5"/>
    <enumval name="PV_COMP_FEET" value="6"/>
    <enumval name="PV_COMP_TEEF" value="7"/>
    <enumval name="PV_COMP_ACCS" value="8"/>
    <enumval name="PV_COMP_TASK" value="9"/>
    <enumval name="PV_COMP_DECL" value="10"/>
    <enumval name="PV_COMP_JBIB" value="11"/>
    <enumval name="PV_COMP_MAX" value="12"/>
  </enumdef>
  <enumdef type="ePropRenderFlags">
    <enumval name="PRF_ALPHA" value="0"/>
    <enumval name="PRF_DECAL" value="1"/>
    <enumval name="PRF_CUTOUT" value="2"/>
  </enumdef>

  <structdef type="CPVTextureData" simple="true">
    <u8 name="texId"/>
    <u8 name="distribution"/>
    <pad bytes="1"/>
  </structdef>
  <structdef type="CPVDrawblData__CPVClothComponentData" simple="true">
    <bool name="ownsCloth"/>
    <pad bytes="23"/>
  </structdef>
  <structdef type="CPVDrawblData" simple="true">
    <u8 name="propMask"/>
    <u8 name="numAlternatives"/>
    <array name="aTexData" type="atArray"><struct type="CPVTextureData"/></array>
    <struct name="clothData" type="CPVDrawblData__CPVClothComponentData"/>
  </structdef>
  <structdef type="CPVComponentData" simple="true">
    <u8 name="numAvailTex"/>
    <array name="aDrawblData3" type="atArray"><struct type="CPVDrawblData"/></array>
  </structdef>
  <structdef type="CPedSelectionSet" simple="true">
    <string name="name" type="atHashString"/>
    <array name="compDrawableId" type="member" size="12"><u8/></array>
    <array name="compTexId" type="member" size="12"><u8/></array>
    <array name="propAnchorId" type="member" size="6"><u8/></array>
    <array name="propDrawableId" type="member" size="6"><u8/></array>
    <array name="propTexId" type="member" size="6"><u8/></array>
  </structdef>
  <structdef type="CComponentInfo" simple="true">
    <string name="pedXml_audioID" type="atHashString"/>
    <string name="pedXml_audioID2" type="atHashString"/>
    <array name="pedXml_expressionMods" type="member" size="5"><float/></array>
    <u32 name="flags"/>
    <bitset name="inclusions" type="fixed32"/>
    <bitset name="exclusions" type="fixed32"/>
    <bitset name="pedXml_vfxComps" type="fixed16" values="ePedVarComp"/>
    <u16 name="pedXml_flags"/>
    <u8 name="pedXml_compIdx"/>
    <u8 name="pedXml_drawblIdx"/>
  </structdef>
  <structdef type="CPedPropTexData" simple="true">
    <bitset name="inclusions" type="fixed32"/>
    <bitset name="exclusions" type="fixed32"/>
    <u8 name="texId"/>
    <u8 name="inclusionId"/>
    <u8 name="exclusionId"/>
    <u8 name="distribution"/>
  </structdef>
  <structdef type="CPedPropMetaData" simple="true">
    <string name="audioId" type="atHashString"/>
    <array name="expressionMods" type="member" size="5"><float/></array>
    <array name="texData" type="atArray"><struct type="CPedPropTexData"/></array>
    <bitset name="renderFlags" type="fixed8" values="ePropRenderFlags"/>
    <u32 name="propFlags"/>
    <u16 name="flags"/>
    <u8 name="anchorId"/>
    <u8 name="propId"/>
    <u8 name="stickyness"/>
  </structdef>
  <structdef type="CAnchorProps" simple="true">
    <array name="props" type="atArray"><u8/></array>
    <enum name="anchor" type="eAnchorPoints"/>
  </structdef>
  <structdef type="CPedPropInfo" simple="true">
    <u8 name="numAvailProps"/>
    <array name="aPropMetaData" type="atArray"><struct type="CPedPropMetaData"/></array>
    <array name="aAnchors" type="atArray"><struct type="CAnchorProps"/></array>
  </structdef>
  <structdef type="CPedVariationInfo" simple="true">
    <bool name="bHasTexVariations"/>
    <bool name="bHasDrawblVariations"/>
    <bool name="bHasLowLODs"/>
    <bool name="bIsSuperLOD"/>
    <array name="availComp" type="member" size="12"><u8/></array>
    <array name="aComponentData3" type="atArray"><struct type="CPVComponentData"/></array>
    <array name="aSelectionSets" type="atArray"><struct type="CPedSelectionSet"/></array>
    <array name="compInfos" type="atArray"><struct type="CComponentInfo"/></array>
    <struct name="propInfo" type="CPedPropInfo"/>
    <string name="dlcName" type="atHashString"/>
  </structdef>

  <structdef type="CShaderVariableComponent">
    <u32 name="pedcompID"/>
    <u32 name="maskID"/>
    <string name="shaderVariableHashString" type="atHashString"/>
    <array name="tracks" type="atArray"><u8/></array>
    <array name="ids" type="atArray"><u16/></array>
    <array name="components" type="atArray"><u8/></array>
  </structdef>
  <structdef type="CPedPropExpressionData">
    <u32 name="pedPropID"/>
    <s32 name="pedPropVarIndex"/>
    <u32 name="pedPropExpressionIndex"/>
    <array name="tracks" type="atArray"><u8/></array>
    <array name="ids" type="atArray"><u16/></array>
    <array name="types" type="atArray"><u8/></array>
    <array name="components" type="atArray"><u8/></array>
  </structdef>
  <structdef type="CPedCompExpressionData">
    <u32 name="pedCompID"/>
    <s32 name="pedCompVarIndex"/>
    <u32 name="pedCompExpressionIndex"/>
    <array name="tracks" type="atArray"><u8/></array>
    <array name="ids" type="atArray"><u16/></array>
    <array name="types" type="atArray"><u8/></array>
    <array name="components" type="atArray"><u8/></array>
  </structdef>
  <structdef type="CCreatureMetaData">
    <array name="shaderVariableComponents" type="atArray"><struct type="CShaderVariableComponent"/></array>
    <array name="pedPropExpressions" type="atArray"><struct type="CPedPropExpressionData"/></array>
    <array name="pedCompExpressions" type="atArray"><struct type="CPedCompExpressionData"/></array>
  </structdef>
</ParserSchema>"#;

/// The `signature` word the staged documents state for each class and enum of the family, which
/// nothing in this tree can compute and the game's parser holds.
///
/// ⚠ Every staged document states the same word for a class, so a synthesised table carries the
/// shipped one and not a zero.
pub const SIGNATURES: &[(&str, u32)] = &[
    ("CAnchorProps", 0x180E_0DA4),
    ("CComponentInfo", 0xDC2B_A2D2),
    ("CCreatureMetaData", 0x8209_6444),
    ("CPVComponentData", 0x78A5_141F),
    ("CPVDrawblData", 0x0765_36BE),
    ("CPVDrawblData::CPVClothComponentData", 0x1E55_BE07),
    ("CPVTextureData", 0xFEAC_7FE2),
    ("CPedCompExpressionData", 0xCE1F_6809),
    ("CPedPropExpressionData", 0x50C5_BB42),
    ("CPedPropInfo", 0x6AD7_318B),
    ("CPedPropMetaData", 0x78FB_596E),
    ("CPedPropTexData", 0xA4F1_9689),
    ("CPedSelectionSet", 0xB9FB_C547),
    ("CPedVariationInfo", 0xF042_3679),
    ("CShaderVariableComponent", 0xB7EE_0E2D),
    ("eAnchorPoints", 0x4E0B_7113),
    ("ePedVarComp", 0xCEF3_CD96),
    ("ePropRenderFlags", 0x5C80_52A1),
];

/// [`PED_TYPES`] as a library.
pub fn declared() -> &'static SchemaLibrary {
    static LIBRARY: OnceLock<SchemaLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut library = SchemaLibrary::new();
        let items =
            parse_psc(PED_TYPES.as_bytes(), "ped").expect("the ped types are well-formed PSC");
        for item in items {
            match item {
                SchemaItem::Struct(def) => library.insert_struct(def),
                SchemaItem::Enum(def) => library.insert_enum(def),
                SchemaItem::Skipped => {}
            }
        }
        library
    })
}

/// The types declared in code, which a save lays out as declared whatever a snapshot or a document's own table states.
pub(super) fn family() -> &'static SchemaLibrary {
    // The alternate-variation types go beside it: a root of that family is built and saved the same way.
    static FAMILY: OnceLock<SchemaLibrary> = OnceLock::new();
    FAMILY.get_or_init(|| {
        let mut lib = declared().clone();
        let alternate = super::alternate_types::declared();
        lib.structs.extend(
            alternate
                .structs
                .iter()
                .map(|(n, d)| (n.clone(), d.clone())),
        );
        // A record of the weapon family built here is conformed to its own declaration, which is the same library.
        lib.structs.extend(
            super::weapon_types::declared()
                .structs
                .iter()
                .map(|(n, d)| (n.clone(), d.clone())),
        );
        lib
    })
}

/// `base` with the family's types.
///
/// ⚠ **The family's definition wins over a snapshot's**, unlike every other shipped transcription:
/// a snapshot states a class by its members, so it leaves out the bytes of a class that no member
/// names, and a table synthesised from it is smaller than any shipped one. A suite holds the
/// transcription to the tables the documents state.
pub fn library(base: &'static SchemaLibrary) -> Cow<'static, SchemaLibrary> {
    let family = family();
    if base.is_empty() {
        return Cow::Borrowed(family);
    }
    let merged = || {
        let mut lib = base.clone();
        lib.structs
            .extend(family.structs.iter().map(|(n, d)| (n.clone(), d.clone())));
        lib.enums
            .extend(family.enums.iter().map(|(n, d)| (n.clone(), d.clone())));
        lib
    };
    // Built once for the installed snapshot: a record built here asks for it once a member.
    static INSTALLED: OnceLock<SchemaLibrary> = OnceLock::new();
    match crate::meta::snapshot::installed().filter(|lib| std::ptr::eq(base, *lib)) {
        Some(_) => Cow::Borrowed(INSTALLED.get_or_init(merged)),
        None => Cow::Owned(merged()),
    }
}

/// Why a document whose own type table is `tables` is not saved off the layout it arrived with: the first class of the
/// families the table states with a member its declaration lacks, or without one it has; `None` where there is none.
pub(super) fn parts_from_its_declaration(tables: &SchemaLibrary) -> Option<String> {
    // ⚠ By the hash of its name, never by the name: a table gives a class the name the index of names has for its hash,
    // which with no snapshot installed is `hash_XXXXXXXX` for every class of a variation but its root.
    static DECLARED: OnceLock<HashMap<u32, &'static str>> = OnceLock::new();
    let declared = DECLARED.get_or_init(|| {
        let named = |name: &'static String| (name_hash(name), name.as_str());
        family().structs.keys().map(named).collect()
    });
    let holds = |members: &[&Field], member: &Field| {
        let key = name_hash(&member.name);
        members.iter().any(|held| name_hash(&held.name) == key)
    };
    // ⚠ In the order of the hashes: a map of classes is walked in no order, and the sentence is the same every time.
    let mut stated: Vec<(u32, &StructDef)> = (tables.structs.values())
        .map(|class| (name_hash(&class.name), class))
        .collect();
    stated.sort_by_key(|(key, _)| *key);
    stated.into_iter().find_map(|(key, stated)| {
        let class = *declared.get(&key)?;
        // The bytes of a class no member names are no member a table states.
        let named = |field: &&Field| !matches!(field.ty, FieldType::Pad(_));
        let declared: Vec<&Field> = (family().resolve(class)?.fields.into_iter())
            .filter(named)
            .collect();
        let own: Vec<&Field> = stated.fields.iter().collect();
        let laid_out = "an edit that changes the file's shape is laid out by the declaration";
        if let Some(member) = own.iter().find(|member| !holds(&declared, member)) {
            let member = &member.name;
            return Some(format!(
                "this file's own type table states `{class}` with `{member}`, a member the `{class}` declared here lacks: {laid_out}, and the save would drop `{member}` from every `{class}`; only an edit that keeps the shape the file arrived with saves here"
            ));
        }
        let member = &declared.iter().find(|member| !holds(&own, member))?.name;
        Some(format!(
            "this file's own type table states `{class}` without `{member}`, a member the `{class}` declared here has: {laid_out}, and no `{class}` of this file holds a `{member}` to write; only an edit that keeps the shape the file arrived with saves here"
        ))
    })
}

/// A model that holds only the signature of every class and enum the family has, and of `arrival`'s own.
///
/// The synthesiser reaches nothing of a donor but these words.
pub fn donor(arrival: Option<&RbmModel>) -> RbmModel {
    let mut classes: HashMap<u32, RbmClass> = HashMap::new();
    let mut enums: HashMap<u32, RbmEnum> = HashMap::new();
    for (name, signature) in SIGNATURES {
        let hash = name_hash(name);
        if declared().enums.contains_key(*name) {
            enums.insert(
                hash,
                RbmEnum {
                    name_hash: hash,
                    signature: *signature,
                    flags: 0,
                    values: Vec::new(),
                },
            );
        } else {
            classes.insert(
                hash,
                RbmClass {
                    name_hash: hash,
                    signature: *signature,
                    flags: 0,
                    align_power: 0,
                    data_size: 0,
                    major_version: 0,
                    entries: Vec::new(),
                },
            );
        }
    }
    if let Some(own) = arrival {
        classes.extend(own.classes.iter().map(|(h, c)| (*h, c.clone())));
        enums.extend(own.enums.iter().map(|(h, e)| (*h, e.clone())));
    }
    RbmModel {
        classes,
        enums,
        data_blocks: Vec::new(),
        root_block_id: 0,
        class_order: Vec::new(),
        enum_order: Vec::new(),
    }
}
