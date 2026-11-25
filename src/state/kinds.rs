use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use egui::Color32;
use egui_phosphor::regular as ph;

use super::format_handlers::Tier;
use super::DocumentKind;
use crate::ui::dock::Workspace;
use crate::ui::theme;

#[derive(Debug, Clone, Copy)]
pub struct KindInfo {
    pub kind: DocumentKind,

    pub noun: &'static str,

    pub tooltip: &'static str,

    pub glyph: &'static str,

    pub color: Color32,

    pub workspace: Workspace,

    pub keeps_source_extension: bool,

    own_extensions: &'static [&'static str],
}

impl KindInfo {
    /// This row with its noun and tooltip read into the language in use.
    fn in_use(self) -> Self {
        Self {
            noun: crate::i18n::tr(self.noun),
            tooltip: crate::i18n::tr(self.tooltip),
            ..self
        }
    }

    pub fn provisional(kind: DocumentKind) -> Self {
        let noun = match kind.resource_type() {
            Some(resource) => resource.description(),
            None => tr!("Unrecognised"),
        };
        Self {
            kind,
            noun,
            tooltip: noun,
            glyph: ph::PUZZLE_PIECE,
            color: theme::KIND_UNKNOWN,
            workspace: Workspace::Browse,
            keeps_source_extension: false,
            own_extensions: &[],
        }
    }

    pub fn extensions(&self) -> &'static [&'static str] {
        match self.kind.resource_type() {
            Some(_) => self.kind.extensions(),
            None => self.own_extensions,
        }
    }

    pub fn default_extension(&self) -> Option<&'static str> {
        self.extensions().first().copied()
    }

    pub fn save_title(&self) -> String {
        tr!("Save {noun}", noun = self.noun)
    }

    /// The New menu's row: the noun in sentence case, then the extension, as in `Particle effects (.ypt)`.
    pub fn menu_label(&self) -> String {
        let words: Vec<String> = self
            .noun
            .split(' ')
            .enumerate()
            .map(|(i, word)| {
                // An acronym keeps its capitals.
                if i == 0 || !word.contains(char::is_lowercase) {
                    word.to_string()
                } else {
                    word.to_lowercase()
                }
            })
            .collect();
        format!("{} (.{})", words.join(" "), self.kind.extension())
    }

    pub fn filter_label(&self) -> String {
        const SHOWN: usize = 4;
        let exts = self.extensions();
        let listed: Vec<String> = exts.iter().take(SHOWN).map(|e| format!(".{e}")).collect();
        let tail = if exts.len() > SHOWN { ", …" } else { "" };
        format!("{} ({}{})", self.noun, listed.join(", "), tail)
    }
}

pub fn all() -> impl Iterator<Item = KindInfo> {
    KINDS
        .iter()
        .copied()
        .map(KindInfo::in_use)
        .chain(DocumentKind::registered().into_iter().map(|k| k.info()))
}

/// A row in the create-new menu, and whether this build can actually make one.
#[derive(Debug, Clone)]
pub struct Creatable {
    pub info: KindInfo,
    /// `None` when it can be authored; otherwise why not, for the tooltip.
    pub unavailable: Option<String>,
}

impl Creatable {
    pub fn available(&self) -> bool {
        self.unavailable.is_none()
    }
}

/// Every kind the menu shows, each saying whether this build can author it.
///
/// ⚠ The question is asked of
/// [`why_not_authorable`](super::super::document_io::why_not_authorable), the
/// same function the attempt asks, so an offer and its outcome cannot disagree.
///
/// ⚠ **Shown-and-disabled rather than hidden.** A row that vanishes teaches
/// nothing; a greyed one says why on hover.
pub fn creatable() -> Vec<Creatable> {
    all()
        .filter(|info| info.default_extension().is_some() && !info.keeps_source_extension)
        .map(|info| Creatable {
            unavailable: super::document_io::why_not_authorable(info.kind),
            info,
        })
        .collect()
}

pub fn openable_extensions() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = all().flat_map(|info| info.extensions()).copied().collect();
    out.sort_unstable();
    out.dedup();
    out
}

static KINDS: &[KindInfo] = &[
    KindInfo {
        kind: DocumentKind::Ypt,
        noun: tr_key!("Particle Effects"),
        tooltip: tr_key!("YPT — particle effects"),
        glyph: ph::LIGHTNING,
        color: theme::KIND_YPT,
        workspace: Workspace::Particle,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ytd,
        noun: tr_key!("Texture Dictionary"),
        tooltip: tr_key!("YTD — texture dictionary"),
        glyph: ph::IMAGES,
        color: theme::KIND_YTD,
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ydr,
        noun: tr_key!("Drawable"),
        tooltip: tr_key!("YDR — drawable model"),
        glyph: ph::CUBE,
        color: theme::KIND_YDR,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ydd,
        noun: tr_key!("Drawable Dictionary"),
        tooltip: tr_key!("YDD — drawable dictionary"),
        glyph: ph::STACK,
        color: theme::KIND_YDD,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Yft,
        noun: tr_key!("Fragment"),
        tooltip: tr_key!("YFT — fragment"),
        glyph: ph::SHARE_NETWORK,
        color: theme::KIND_YFT,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ybn,
        noun: tr_key!("Physics Bound"),
        tooltip: tr_key!("YBN — collision bound"),
        glyph: ph::POLYGON,
        color: theme::KIND_YBN,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Yld,
        noun: tr_key!("Cloth Dictionary"),
        tooltip: tr_key!("YLD — cloth dictionary"),
        glyph: ph::WAVE_SAWTOOTH,
        color: theme::KIND_YLD,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ycd,
        noun: tr_key!("Clip Dictionary"),
        tooltip: tr_key!("YCD — animation clip dictionary"),
        glyph: ph::PERSON_SIMPLE_RUN,
        color: theme::KIND_YCD,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Yed,
        noun: tr_key!("Expression Dictionary"),
        tooltip: tr_key!("YED — procedural-bone expressions"),
        glyph: ph::FUNCTION,
        color: theme::KIND_YED,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Yfd,
        noun: tr_key!("Frame Filter Dictionary"),
        tooltip: tr_key!("YFD — animation blend masks"),
        glyph: ph::FUNNEL,
        color: theme::KIND_YFD,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    // ⚠ No geometry row draws either yet, so the Model viewport stays empty for them.
    KindInfo {
        kind: DocumentKind::Ynd,
        noun: tr_key!("Path Region"),
        tooltip: tr_key!("YND — traffic path nodes"),
        glyph: ph::PATH,
        color: theme::KIND_YND,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ynv,
        noun: tr_key!("Navigation Mesh"),
        tooltip: tr_key!("YNV — navmesh cell"),
        glyph: ph::MAP_TRIFOLD,
        color: theme::KIND_YNV,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Ypdb,
        noun: tr_key!("Pose Matcher Database"),
        tooltip: tr_key!("YPDB — pose matcher database"),
        glyph: ph::CROSSHAIR,
        color: theme::KIND_YPDB,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &["ypdb"],
    },
    KindInfo {
        kind: DocumentKind::Fxc,
        noun: tr_key!("Shader Effect"),
        tooltip: tr_key!("FXC — compiled shader effect"),
        glyph: ph::CIRCUITRY,
        color: theme::KIND_FXC,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &["fxc"],
    },
    KindInfo {
        kind: DocumentKind::Gxt2,
        noun: tr_key!("Global Text Table"),
        tooltip: tr_key!("GXT2 — global text table"),
        glyph: ph::TRANSLATE,
        color: theme::KIND_GXT2,
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["gxt2"],
    },
    KindInfo {
        kind: DocumentKind::Bik,
        noun: tr_key!("Bink Video"),
        tooltip: tr_key!("BIK — Bink video, played in the preview"),
        glyph: ph::FILM_STRIP,
        color: Color32::from_rgb(0xe0, 0x6a, 0x8f),
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &["bik"],
    },
    KindInfo {
        kind: DocumentKind::Awc,
        noun: tr_key!("Audio Wave Container"),
        tooltip: tr_key!("AWC — audio wave container"),
        glyph: ph::SPEAKER_HIGH,
        color: Color32::from_rgb(0x8f, 0xd0, 0x6a),
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["awc"],
    },
    KindInfo {
        kind: DocumentKind::Rel,
        noun: tr_key!("Audio Metadata"),
        tooltip: tr_key!("REL — audio metadata: sounds, game objects, speech, curves and mixes"),
        glyph: ph::WAVEFORM,
        color: Color32::from_rgb(0x6a, 0xb8, 0xd0),
        workspace: Workspace::Data,
        // A `.dat54` saved as `.rel` would hold three bytes per item its `.rel` sibling lacks.
        keeps_source_extension: true,
        own_extensions: &[
            "rel", "dat4", "dat10", "dat15", "dat54", "dat149", "dat150", "dat151",
        ],
    },
    KindInfo {
        kind: DocumentKind::NameTable,
        noun: tr_key!("Name Table"),
        tooltip: tr_key!("NAMETABLE — the names a sibling .dat holds only as hashes, one per line"),
        glyph: ph::LIST_BULLETS,
        color: theme::KIND_TEXT,
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["nametable"],
    },
    KindInfo {
        kind: DocumentKind::Gfx,
        noun: tr_key!("Scaleform Movie"),
        tooltip: tr_key!("GFX — Scaleform movie, exported to and imported from SWF"),
        glyph: ph::MONITOR_PLAY,
        color: Color32::from_rgb(0xd0, 0x9a, 0x4a),
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["gfx"],
    },
    KindInfo {
        kind: DocumentKind::MoveNetwork,
        noun: tr_key!("Move Network"),
        tooltip: tr_key!("MRF — move network definition: state machines, states, transitions and nodes"),
        glyph: ph::FLOW_ARROW,
        color: theme::KIND_YCD,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &["mrf"],
    },
    KindInfo {
        kind: DocumentKind::Dds,
        noun: tr_key!("DDS Texture"),
        tooltip: tr_key!("DDS — a loose texture, viewed and replaced like a .ytd's"),
        glyph: ph::IMAGE_SQUARE,
        color: theme::KIND_YTD,
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["dds"],
    },
    KindInfo {
        kind: DocumentKind::Ttpf,
        noun: tr_key!("GTA VI Texture Pack"),
        tooltip: tr_key!("TTPF — a GTA VI texture pack, listed and viewed one texture at a time; a texture is replaced from an image and the pack saved"),
        glyph: ph::IMAGES_SQUARE,
        color: theme::KIND_YTD,
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["ttpf"],
    },
    KindInfo {
        kind: DocumentKind::Rpf,
        noun: tr_key!("Archive"),
        tooltip: tr_key!("RPF — RAGE archive"),
        glyph: ph::ARCHIVE,
        color: theme::KIND_RPF,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &["rpf"],
    },
    KindInfo {
        kind: DocumentKind::Rpf9,
        noun: tr_key!("GTA VI Archive"),
        tooltip: tr_key!("RPF9 — a GTA VI archive, shown as what a scan without its table finds, read-only"),
        glyph: ph::SCAN,
        color: theme::KIND_RPF,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Anim,
        noun: tr_key!("GTA VI Animations"),
        tooltip: tr_key!("A GTA VI animation resource found by a scan: its clips, animations and tracks, whose keys and clip windows edit and save as a loose resource"),
        glyph: ph::PERSON_SIMPLE_WALK,
        color: theme::KIND_YCD,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Drawable,
        noun: tr_key!("GTA VI Drawable"),
        tooltip: tr_key!("A GTA VI drawable, fragment, bound or dictionary of drawables found by a scan: its geometry drawn with the textures its keys resolve to, and a drawable's materials, which edit and save as a loose resource"),
        glyph: ph::CUBE_TRANSPARENT,
        color: theme::KIND_YDR,
        workspace: Workspace::Model,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Text,
        noun: tr_key!("GTA VI Text Table"),
        tooltip: tr_key!("A GTA VI text table found by a scan: its keys and texts, which edit and save as a loose resource"),
        glyph: ph::CHAT_TEXT,
        color: theme::KIND_GXT2,
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6AudioData,
        noun: tr_key!("GTA VI Audio Game Data"),
        tooltip: tr_key!("GTA VI audio game data found by a scan: its items, their fields, waves and references, read-only"),
        glyph: ph::MUSIC_NOTES,
        color: Color32::from_rgb(0x6a, 0xb8, 0xd0),
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Expressions,
        noun: tr_key!("GTA VI Expression Set"),
        tooltip: tr_key!("A GTA VI expression set found by a scan: the facial controls and bones it reads and those it drives, read-only"),
        glyph: ph::SMILEY,
        color: theme::KIND_YED,
        workspace: Workspace::Animation,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Save,
        noun: tr_key!("GTA VI Save"),
        tooltip: tr_key!("A GTA VI save: a story save's blocks or a record's payload, each as the tree of its JSON; a packed copy is not decoded"),
        glyph: ph::FLOPPY_DISK_BACK,
        color: theme::KIND_GXT2,
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Gta6Script,
        noun: tr_key!("GTA VI Script"),
        tooltip: tr_key!("A compiled GTA VI script, read from the archive that holds it or from a deciphered file: the source it decompiles to and the classes and functions that source lists, read-only"),
        glyph: ph::CODE,
        color: theme::KIND_TEXT,
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::GameRoot,
        noun: tr_key!("Game Install"),
        tooltip: tr_key!("Game install — the folder a game was installed to"),
        glyph: ph::TREE_STRUCTURE,
        color: theme::KIND_GAME_ROOT,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Folder,
        noun: tr_key!("Folder"),
        tooltip: tr_key!("Folder — opened from disk"),
        glyph: ph::FOLDER_OPEN,
        color: theme::KIND_FOLDER,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Text,
        noun: tr_key!("Text Document"),
        tooltip: tr_key!("Text document"),
        glyph: ph::FILE_TEXT,
        color: theme::KIND_TEXT,
        workspace: Workspace::Data,
        keeps_source_extension: true,
        own_extensions: super::document::TEXT_DOCUMENT_EXTENSIONS,
    },
    KindInfo {
        kind: DocumentKind::Meta,
        noun: tr_key!("Metadata"),
        tooltip: tr_key!("Meta — schema-driven metadata"),
        glyph: ph::TREE_VIEW,
        color: theme::KIND_META,
        workspace: Workspace::Data,
        keeps_source_extension: true,
        own_extensions: &["ymap", "ytyp", "meta", "ymt", "ymf", "pso", "cut", "srl"],
    },
    // After `Text`: the first row claiming an extension names its file association.
    KindInfo {
        kind: DocumentKind::HeightMap,
        noun: tr_key!("Terrain Height Map"),
        tooltip: tr_key!("heightmap.dat — the world's coarse terrain height grid"),
        glyph: ph::MOUNTAINS,
        color: Color32::from_rgb(0xb0, 0x9a, 0x6a),
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
    KindInfo {
        kind: DocumentKind::DistantLights,
        noun: tr_key!("Distant Lights"),
        tooltip: tr_key!("distantlights.dat — far-off traffic and street lights"),
        glyph: ph::LIGHTBULB,
        color: Color32::from_rgb(0xf0, 0xc8, 0x5a),
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
    KindInfo {
        kind: DocumentKind::WaterMap,
        noun: tr_key!("Water Height Map"),
        tooltip: tr_key!("waterheight.dat — which rivers, lakes and pools lie under each world tile"),
        glyph: ph::DROP,
        color: Color32::from_rgb(0x5a, 0x9e, 0xe0),
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
    KindInfo {
        kind: DocumentKind::WorldBlanket,
        noun: tr_key!("World Blanket"),
        tooltip:
            tr_key!("worldblanket.dat — a square grid of 16-bit cells whose meaning the file does not state"),
        glyph: ph::GRID_FOUR,
        color: Color32::from_rgb(0x9a, 0xa8, 0xb8),
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
    KindInfo {
        kind: DocumentKind::NavPrecalc,
        noun: tr_key!("Nav Precalc"),
        tooltip: tr_key!("navprecalc.dat — NAVP cells of unmeasured entries and a BITS block"),
        glyph: ph::NAVIGATION_ARROW,
        color: Color32::from_rgb(0x7a, 0xc0, 0x8a),
        workspace: Workspace::Texture,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
    KindInfo {
        kind: DocumentKind::Hex,
        noun: tr_key!("Bytes"),
        tooltip: tr_key!("Hex view — a file no reader opens, read-only"),
        glyph: ph::BINARY,
        color: theme::KIND_UNKNOWN,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::Compare,
        noun: tr_key!("Comparison"),
        tooltip:
            tr_key!("Comparison — how two files differ, or a file in the mods overlay and the game's own"),
        glyph: ph::GIT_DIFF,
        color: theme::KIND_TEXT,
        workspace: Workspace::Browse,
        keeps_source_extension: false,
        own_extensions: &[],
    },
    KindInfo {
        kind: DocumentKind::DatTable,
        noun: tr_key!("Game Data Table"),
        tooltip: tr_key!("*_cache_y.dat, metadata.dat or audioworldsectorsinfo.dat"),
        glyph: ph::DATABASE,
        color: Color32::from_rgb(0x8a, 0xa8, 0xc8),
        workspace: Workspace::Data,
        keeps_source_extension: false,
        own_extensions: &["dat"],
    },
];

#[derive(Default)]
pub struct KindRegistry {
    tiers: HashMap<Tier, HashMap<DocumentKind, KindInfo>>,
}

impl KindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_at(&mut self, tier: Tier, info: KindInfo) {
        self.tiers.entry(tier).or_default().insert(info.kind, info);
    }

    pub fn get(&self, kind: DocumentKind) -> Option<KindInfo> {
        for tier in [Tier::Preferred, Tier::Standard, Tier::Fallback] {
            if let Some(found) = self.tiers.get(&tier).and_then(|m| m.get(&kind)) {
                return Some(*found);
            }
        }
        None
    }
}

fn live() -> &'static RwLock<KindRegistry> {
    static LIVE: OnceLock<RwLock<KindRegistry>> = OnceLock::new();
    LIVE.get_or_init(|| {
        let mut registry = KindRegistry::new();
        // `.ysc` has no named kind, and it opens as decompiled source, which
        // the Data workspace shows.
        let ysc = DocumentKind::Registered(or_io::container::resource::ResourceType::Ysc);
        registry.register_at(
            Tier::Fallback,
            KindInfo {
                workspace: Workspace::Data,
                ..KindInfo::provisional(ysc)
            },
        );
        RwLock::new(registry)
    })
}

// A registration seam, called by nothing in-tree by definition. ⚠ Not the
// same item as `KindRegistry::register`, which tests reach: a rule keyed on
// the name treats the two as one.
#[allow(dead_code)]
pub fn register(info: KindInfo) {
    register_at(Tier::Standard, info);
}

pub fn register_at(tier: Tier, info: KindInfo) {
    live()
        .write()
        .expect("kind table poisoned")
        .register_at(tier, info);
}

impl DocumentKind {
    pub fn info(self) -> KindInfo {
        if let Some(registered) = live().read().expect("kind table poisoned").get(self) {
            return registered;
        }
        KINDS
            .iter()
            .find(|info| info.kind == self)
            .copied()
            .map_or_else(|| KindInfo::provisional(self), KindInfo::in_use)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_exactly_one_row() {
        for kind in DocumentKind::NAMED {
            let rows = KINDS.iter().filter(|i| i.kind == *kind).count();
            assert_eq!(
                rows, 1,
                "{kind:?} has {rows} rows in KINDS, expected exactly 1. A kind \
                 with no row falls back to KindInfo::provisional, which is \
                 deliberately unattractive: the noun reads \"Unrecognised\", the \
                 glyph is a puzzle piece and the workspace is whichever one is \
                 first."
            );
        }
        assert_eq!(
            KINDS.len(),
            DocumentKind::NAMED.len(),
            "the table has a row for a kind the enum does not name"
        );
    }

    #[test]
    fn a_resource_kind_does_not_keep_its_own_extensions() {
        for info in KINDS {
            if info.kind.resource_type().is_some() {
                assert!(
                    info.own_extensions.is_empty(),
                    "{:?} is an RSC7 resource, so its extensions come from \
                     ResourceType::extensions() -- remove the copy",
                    info.kind
                );
            }
        }
    }

    #[test]
    fn every_extension_in_the_table_resolves_to_a_kind() {
        for info in KINDS {
            for ext in info.extensions() {
                let path = std::path::PathBuf::from(format!("probe.{ext}"));
                assert!(
                    DocumentKind::from_path(&path).is_some(),
                    "{:?} claims .{ext} but from_path does not recognise it",
                    info.kind
                );
            }
        }
    }

    #[test]
    fn a_resource_extension_resolves_to_its_own_kind() {
        for info in KINDS {
            if info.kind.resource_type().is_none() {
                continue;
            }
            for ext in info.extensions() {
                let path = std::path::PathBuf::from(format!("probe.{ext}"));
                assert_eq!(
                    DocumentKind::from_path(&path),
                    Some(info.kind),
                    ".{ext} should open as {:?}",
                    info.kind
                );
            }
        }
    }

    #[test]
    fn only_the_tree_roots_the_hex_view_a_comparison_and_gta_vi_s_scanned_kinds_have_no_extension()
    {
        for info in KINDS {
            let has = info.default_extension().is_some();
            // A GTA VI archive is an `.rpf` its head names, an animation resource opens only from
            // a scan, and both save as nothing.
            let expected = !matches!(
                info.kind,
                DocumentKind::GameRoot
                    | DocumentKind::Folder
                    | DocumentKind::Hex
                    | DocumentKind::Compare
                    | DocumentKind::Rpf9
                    | DocumentKind::Gta6Anim
                    | DocumentKind::Gta6Drawable
                    | DocumentKind::Gta6Text
                    | DocumentKind::Gta6AudioData
                    | DocumentKind::Gta6Expressions
                    | DocumentKind::Gta6Save
                    | DocumentKind::Gta6Script
            );
            assert_eq!(
                has,
                expected,
                "{:?}: default_extension() = {:?}",
                info.kind,
                info.default_extension()
            );
        }
    }

    #[test]
    fn the_catch_all_admits_everything_the_per_format_filters_do() {
        let everything = openable_extensions();
        for info in all() {
            for ext in info.extensions() {
                assert!(
                    everything.contains(ext),
                    "{:?} offers .{ext} but the catch-all filter does not",
                    info.kind
                );
            }
        }
    }

    #[test]
    fn nothing_the_open_dialog_offers_is_unopenable() {
        for ext in openable_extensions() {
            let path = std::path::PathBuf::from(format!("probe.{ext}"));
            assert!(
                DocumentKind::from_path(&path).is_some(),
                "the Open dialog offers .{ext}, which from_path does not recognise"
            );
        }
    }

    #[test]
    fn only_the_tree_roots_the_hex_view_a_comparison_and_gta_vi_s_scanned_kinds_are_absent_from_the_open_dialog(
    ) {
        let offered: Vec<DocumentKind> = KINDS
            .iter()
            .filter(|i| !i.extensions().is_empty())
            .map(|i| i.kind)
            .collect();
        assert!(!offered.contains(&DocumentKind::GameRoot));
        assert!(!offered.contains(&DocumentKind::Folder));
        assert!(!offered.contains(&DocumentKind::Hex));
        assert!(!offered.contains(&DocumentKind::Compare));
        assert!(!offered.contains(&DocumentKind::Rpf9));
        assert!(!offered.contains(&DocumentKind::Gta6Anim));
        assert!(!offered.contains(&DocumentKind::Gta6Drawable));
        assert!(!offered.contains(&DocumentKind::Gta6Text));
        assert!(!offered.contains(&DocumentKind::Gta6AudioData));
        assert!(!offered.contains(&DocumentKind::Gta6Expressions));
        assert!(!offered.contains(&DocumentKind::Gta6Save));
        assert!(!offered.contains(&DocumentKind::Gta6Script));
        assert_eq!(offered.len(), KINDS.len() - 12);
    }

    #[test]
    fn a_filter_label_stays_narrow_enough_to_read() {
        assert_eq!(
            DocumentKind::Ydd.info().filter_label(),
            "Drawable Dictionary (.ydd)"
        );
        assert_eq!(
            DocumentKind::Meta.info().filter_label(),
            "Metadata (.ymap, .ytyp, .meta, .ymt, …)"
        );
        let text = DocumentKind::Text.info().filter_label();
        assert!(text.ends_with(", …)"), "{text}");
        assert!(text.len() < 60, "still too wide: {text}");
    }

    #[test]
    fn a_format_nobody_described_gets_an_obviously_provisional_row() {
        use or_io::container::resource::ResourceType;
        let kind = DocumentKind::Registered(ResourceType::Yed);
        let info = KindInfo::provisional(kind);

        assert_eq!(info.kind, kind);
        assert_eq!(info.noun, ResourceType::Yed.description());
        assert_eq!(
            info.color,
            theme::KIND_UNKNOWN,
            "a provisional row must not borrow a palette entry that says a \
             person chose it"
        );
        assert!(info.own_extensions.is_empty());
        assert!(!info.keeps_source_extension);
    }

    #[test]
    fn describing_a_format_replaces_its_provisional_row() {
        use or_io::container::resource::ResourceType;
        let kind = DocumentKind::Registered(ResourceType::Ypm);

        let mut registry = KindRegistry::new();
        assert!(registry.get(kind).is_none());
        registry.register_at(
            Tier::Standard,
            KindInfo {
                noun: "Parameterised Motion",
                glyph: ph::LIGHTNING,
                color: theme::KIND_YPT,
                ..KindInfo::provisional(kind)
            },
        );

        let row = registry.get(kind).expect("registered above");
        assert_eq!(row.noun, "Parameterised Motion");
        assert_ne!(row.color, theme::KIND_UNKNOWN);
    }

    #[test]
    fn a_preferred_kind_row_beats_a_standard_one_registered_after_it() {
        use or_io::container::resource::ResourceType;
        let kind = DocumentKind::Registered(ResourceType::Ypm);

        let mut registry = KindRegistry::new();
        registry.register_at(
            Tier::Preferred,
            KindInfo {
                noun: "preferred",
                ..KindInfo::provisional(kind)
            },
        );
        registry.register_at(
            Tier::Standard,
            KindInfo {
                noun: "standard",
                ..KindInfo::provisional(kind)
            },
        );

        assert_eq!(registry.get(kind).expect("a row").noun, "preferred");
    }

    /// `.ysc` opens as decompiled source, so it opens in the Data workspace,
    /// which shows it, though it has no named kind.
    #[test]
    fn a_script_opens_in_the_data_workspace() {
        let ysc = DocumentKind::Registered(or_io::container::resource::ResourceType::Ysc);
        let info = ysc.info();
        assert_eq!(info.workspace, Workspace::Data);
        assert!(Workspace::Data.handles_kind(ysc));
        assert_eq!(
            info.noun,
            KindInfo::provisional(ysc).noun,
            "only the workspace differs"
        );
    }

    /// `.ypm`: a retail install ships zero, and nothing reads one. A kind
    /// would be a menu entry nothing can exercise.
    ///
    /// `.ybd`: four files, one at a version this build describes, and it
    /// *does* have a `PayloadHandler` — so it opens and shows the
    /// pack-described root. What it declines is the chrome.
    ///
    /// `.ysc`: 3,022 files across two installs and every one reads. It opens
    /// read-only as the source `openrage-cli script decompile` prints, and has
    /// no declaration a built-in description could draw (OD-YSC).
    ///
    /// `.yvr` and `.ywr`: every shipped file opens owned, and nothing in the
    /// app asks for more than the described root.
    #[test]
    fn the_unclaimed_resources_are_unclaimed_for_different_reasons() {
        use or_io::container::resource::ResourceType;
        let registered = DocumentKind::registered();
        assert_eq!(
            registered,
            vec![
                DocumentKind::Registered(ResourceType::Ypm),
                DocumentKind::Registered(ResourceType::Ybd),
                DocumentKind::Registered(ResourceType::Ysc),
                DocumentKind::Registered(ResourceType::Yvr),
                DocumentKind::Registered(ResourceType::Ywr),
            ],
            "a resource with no named kind is unread, deliberately plain, or has \
             nothing a panel could show"
        );

        let handlers = super::super::payload_handlers::HANDLERS;
        assert!(
            !handlers.iter().any(|h| h.id.resource == ResourceType::Ypm),
            "nothing reads a .ypm"
        );
        for readable in [
            ResourceType::Ybd,
            ResourceType::Ysc,
            ResourceType::Yvr,
            ResourceType::Ywr,
        ] {
            assert!(
                handlers.iter().any(|h| h.id.resource == readable),
                "{readable} opens through the registered path, so it needs a handler"
            );
        }

        let mut sorted = registered.clone();
        sorted.sort_by_key(|k| k.resource_type());
        assert_eq!(registered, sorted);

        for extension in ["ypm", "ybd", "ysc", "yvr", "ywr"] {
            assert!(openable_extensions().contains(&extension));
        }
    }

    /// `.fxc` is a shipped format that can never be created: it holds compiled
    /// shader bytecode and OpenRage bakes rather than writes one. ⚠ Not
    /// `.ypm`: `adding_a_format` takes it as its synthetic stand-in, so in a
    /// process where that ran first `.ypm` *can* be created.
    ///
    /// It also covers a kind with no resource type, which
    /// [`why_not_authorable`](super::super::document_io::why_not_authorable)
    /// must ask about rather than wave through, or the menu offers a row whose
    /// click refuses.
    #[test]
    fn a_kind_nothing_can_write_is_not_offered_in_the_create_menu() {
        let info = DocumentKind::Fxc.info();

        assert!(info.default_extension().is_some());
        assert!(!info.keeps_source_extension);
        assert!(
            super::super::document_io::why_not_authorable(DocumentKind::Fxc).is_some(),
            "nothing can write a .fxc, so the menu must not offer to create one"
        );
        assert!(!creatable()
            .iter()
            .any(|r| r.info.kind == DocumentKind::Fxc && r.available()));
    }

    /// ⚠ A binary world `.dat` is routed by its file name, never its extension,
    /// so its row claims `.dat` only to name the open dialog's filter. `Text`
    /// must claim it first: the first claimant names the file association, and
    /// every other `.dat` is a text table.
    #[test]
    fn the_binary_world_dats_claim_dat_after_text() {
        let claimants: Vec<DocumentKind> = all()
            .filter(|info| info.extensions().contains(&"dat"))
            .map(|info| info.kind)
            .collect();
        assert_eq!(
            claimants,
            vec![
                DocumentKind::Text,
                DocumentKind::HeightMap,
                DocumentKind::DistantLights,
                DocumentKind::WaterMap,
                DocumentKind::WorldBlanket,
                DocumentKind::NavPrecalc,
                DocumentKind::DatTable,
            ]
        );
        for kind in &claimants[1..] {
            let info = kind.info();
            assert_eq!(info.extensions(), ["dat"], "{kind:?}");
            assert!(!info.keeps_source_extension, "{kind:?}");
        }
    }

    /// Opening a kind switches to its home unless the workspace on screen
    /// handles it, so Particle must handle nothing else either.
    #[test]
    fn no_kind_but_particle_effects_opens_into_the_particle_workspace() {
        for kind in all().map(|info| info.kind) {
            let particle = kind == DocumentKind::Ypt;
            assert_eq!(
                Workspace::for_kind(kind) == Workspace::Particle,
                particle,
                "{kind:?} opens into {:?}",
                Workspace::for_kind(kind)
            );
            assert_eq!(
                Workspace::Particle.handles_kind(kind),
                particle,
                "{kind:?} opened while Particle is on screen would stay there"
            );
        }
    }

    #[test]
    fn a_kinds_default_workspace_can_show_it() {
        for info in all() {
            assert!(
                info.workspace.handles_kind(info.kind),
                "{:?} opens in {:?}, which does not handle it",
                info.kind,
                info.workspace
            );
        }
    }

    #[test]
    fn the_new_menu_offers_what_it_always_offered() {
        // The *rows*; which of them are offerable is `what_the_create_menu_offers.rs`.
        let offered: Vec<DocumentKind> = creatable()
            .into_iter()
            .map(|row| row.info.kind)
            .filter(|kind| !matches!(kind, DocumentKind::Registered(_)))
            .collect();
        assert_eq!(
            offered,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ytd,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Ybn,
                DocumentKind::Yld,
                DocumentKind::Ycd,
                DocumentKind::Yed,
                DocumentKind::Yfd,
                DocumentKind::Ynd,
                DocumentKind::Ynv,
                DocumentKind::Ypdb,
                DocumentKind::Fxc,
                DocumentKind::Gxt2,
                DocumentKind::Bik,
                DocumentKind::Awc,
                DocumentKind::NameTable,
                DocumentKind::Gfx,
                DocumentKind::MoveNetwork,
                DocumentKind::Dds,
                DocumentKind::Ttpf,
                DocumentKind::Rpf,
                DocumentKind::HeightMap,
                DocumentKind::DistantLights,
                DocumentKind::WaterMap,
                DocumentKind::WorldBlanket,
                DocumentKind::NavPrecalc,
                DocumentKind::DatTable,
            ]
        );
    }

    #[test]
    fn every_creatable_kind_has_an_extension_to_offer() {
        for row in creatable() {
            let info = row.info;
            assert!(
                info.default_extension().is_some(),
                "{:?} is offered in the New menu with no extension",
                info.kind
            );
            assert!(!info.kind.extension().is_empty());
        }
    }

    #[test]
    fn only_the_multi_extension_kinds_keep_their_source_extension() {
        for info in KINDS {
            let expected = matches!(
                info.kind,
                DocumentKind::Text | DocumentKind::Meta | DocumentKind::Rel
            );
            assert_eq!(
                info.keeps_source_extension, expected,
                "{:?}: keeps_source_extension",
                info.kind
            );
        }
    }

    #[test]
    fn keeping_the_source_extension_implies_there_is_a_choice() {
        for info in KINDS {
            if info.keeps_source_extension {
                assert!(
                    info.extensions().len() > 1,
                    "{:?} keeps its source extension but answers to only {:?}",
                    info.kind,
                    info.extensions()
                );
            }
        }
    }

    #[test]
    fn the_save_title_reads_properly() {
        assert_eq!(
            DocumentKind::Ydd.info().save_title(),
            "Save Drawable Dictionary"
        );
        assert_eq!(DocumentKind::Ybn.info().save_title(), "Save Physics Bound");
    }
}
