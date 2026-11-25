//! What a document holds, by the kind of file it was read from.

use or_io::gta5::pose_db::PoseMatcherDatabase;
use or_io::gta5::shaders::fxc::GrcEffect;
use or_io::gta5::text::Gxt2;

use super::{DatTable, DocumentKind, HexDocument, MetaDocument, RelDocument, TextDocument};
use crate::state::rpf_session::RpfDocument;

/// A `.fxc`, and the bytes its DXBC blobs are located inside.
///
/// ⚠ The effect stores each program's bytecode as an **offset and a length
/// into the file**, not as a copy, so a reader that dropped the bytes would
/// leave every `bytecode_offset` pointing at nothing.
#[derive(Clone)]
pub struct FxcDocument {
    pub effect: GrcEffect,
    pub bytes: Vec<u8>,
}

// An owned root is matched by value in every row, and a box would hide it from those patterns.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum DocPayload {
    Ypdb(PoseMatcherDatabase),
    Fxc(FxcDocument),
    Gxt2(Gxt2),
    Bik(crate::video::BikDocument),
    Awc(crate::audio::AwcDocument),
    HeightMap(or_io::height_map::HeightMap),
    DistantLights(or_io::distant_lights::DistantLights),
    WaterMap(or_io::water_map::WaterMap),
    WorldBlanket(or_io::world_blanket::WorldBlanket),
    NavPrecalc(or_io::nav_precalc::NavPrecalc),
    DatTable(DatTable),
    Rel(RelDocument),
    /// A `.nametable`'s names, one per line.
    NameTable(TextDocument),
    Gfx(or_io::gfx::GfxMovie),
    MoveNetwork(or_io::mrf::MvNetworkDef),
    /// A loose `.dds`, held as the one texture it is.
    Dds(or_io::gta5::gen8::GrcTexture),
    /// A GTA VI texture pack, read a texture at a time.
    Ttpf(std::sync::Arc<crate::state::ttpf_document::TtpfDocument>),
    Rpf(RpfDocument),
    /// A GTA VI archive, as a scan without its table finds it.
    Rpf9(std::sync::Arc<crate::state::rpf9_scan::Rpf9ScanDocument>),
    /// A GTA VI animation resource a scan found, read only.
    Gta6Anim(std::sync::Arc<or_io::gta6::anim::AnimResource>),
    /// A GTA VI drawable, fragment, bound or dictionary of drawables, its edited materials saved as a loose resource.
    Gta6Drawable(std::sync::Arc<or_io::gta6::headerless::Resource>),
    /// A GTA VI text table a scan found, whose texts edit and save as a loose resource.
    Gta6Text(Box<crate::state::gta6_text::Gta6TextDocument>),
    /// GTA VI audio game data a scan found, read only.
    Gta6AudioData(Box<crate::state::gta6_text::Gta6AudioDocument>),
    /// A GTA VI expression set a scan found, read only.
    Gta6Expressions(std::sync::Arc<or_io::gta6::anim::Expressions>),
    /// A GTA VI save, a story save or a record, read from a file.
    Gta6Save(std::sync::Arc<crate::ui::panels::hierarchy::gta6_save::Gta6SaveDocument>),
    /// A compiled GTA VI script as the source it decompiles to, read only.
    Gta6Script(std::sync::Arc<super::Gta6ScriptDocument>),
    Text(TextDocument),
    Meta(MetaDocument),
    Hex(HexDocument),
    /// Two files and how they differ, which no file holds.
    Compare(crate::state::compare::Comparison),
    /// A file opened as the owned value `or_io` declares for it.
    Resource(or_io::Asset),

    // The enum's open end, constructed only by a registered reader: in-tree,
    // `.ysc`'s `ScriptDocument` and `adding_a_format`'s synthetic format.
    Registered {
        format: crate::state::format_handlers::FormatId,
        data: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    },
}

impl DocPayload {
    pub fn kind(&self) -> DocumentKind {
        match self {
            DocPayload::Ypdb(_) => DocumentKind::Ypdb,
            DocPayload::Fxc(_) => DocumentKind::Fxc,
            DocPayload::Gxt2(_) => DocumentKind::Gxt2,
            DocPayload::Bik(_) => DocumentKind::Bik,
            DocPayload::Awc(_) => DocumentKind::Awc,
            DocPayload::HeightMap(_) => DocumentKind::HeightMap,
            DocPayload::DistantLights(_) => DocumentKind::DistantLights,
            DocPayload::WaterMap(_) => DocumentKind::WaterMap,
            DocPayload::WorldBlanket(_) => DocumentKind::WorldBlanket,
            DocPayload::NavPrecalc(_) => DocumentKind::NavPrecalc,
            DocPayload::DatTable(_) => DocumentKind::DatTable,
            DocPayload::Rel(_) => DocumentKind::Rel,
            DocPayload::NameTable(_) => DocumentKind::NameTable,
            DocPayload::Gfx(_) => DocumentKind::Gfx,
            DocPayload::MoveNetwork(_) => DocumentKind::MoveNetwork,
            DocPayload::Dds(_) => DocumentKind::Dds,
            DocPayload::Ttpf(_) => DocumentKind::Ttpf,
            DocPayload::Rpf(_) => DocumentKind::Rpf,
            DocPayload::Rpf9(_) => DocumentKind::Rpf9,
            DocPayload::Gta6Anim(_) => DocumentKind::Gta6Anim,
            DocPayload::Gta6Drawable(_) => DocumentKind::Gta6Drawable,
            DocPayload::Gta6Text(_) => DocumentKind::Gta6Text,
            DocPayload::Gta6AudioData(_) => DocumentKind::Gta6AudioData,
            DocPayload::Gta6Expressions(_) => DocumentKind::Gta6Expressions,
            DocPayload::Gta6Save(_) => DocumentKind::Gta6Save,
            DocPayload::Gta6Script(_) => DocumentKind::Gta6Script,
            DocPayload::Text(_) => DocumentKind::Text,
            DocPayload::Meta(_) => DocumentKind::Meta,
            DocPayload::Hex(_) => DocumentKind::Hex,
            DocPayload::Compare(_) => DocumentKind::Compare,
            DocPayload::Resource(asset) => DocumentKind::of_asset(asset),
            DocPayload::Registered { format, .. } => DocumentKind::Registered(format.resource),
        }
    }

    pub fn generation(&self) -> or_io::Generation {
        match self {
            DocPayload::Resource(asset) => asset.target(),
            DocPayload::Registered { format, .. } => format.target,
            _ => or_io::Generation::Gen8,
        }
    }

    pub fn is_enhanced(&self) -> bool {
        self.generation().is_gen9()
    }

    /// The text a find reads: a text document's, or the source of a GTA VI script.
    pub fn found_in(&self) -> Option<&ropey::Rope> {
        match self {
            DocPayload::Text(text) => Some(&text.text),
            DocPayload::Gta6Script(script) => Some(&script.source),
            _ => None,
        }
    }
}
