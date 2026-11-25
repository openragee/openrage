//! What kind of file a document was loaded from, and what each kind is called.

use std::path::Path;

use super::{meta, meta_schema, DatTable};
use crate::state::format_handlers as formats;

/// Which file format a document was loaded from.
///
/// [`Self::Registered`] is the enum's open end: a format the enum does not
/// name still has a kind, so the editor can name, offer and open it.
///
/// ⚠ **The open end costs exhaustiveness.** A new builtin variant fails to
/// compile only in this module (measured: four errors, all four in it);
/// everywhere else a catch-all absorbs it, so a
/// kind with no table row, no dialog filter and no reader builds. [`Self::NAMED`]
/// and the suites that walk it carry the check instead:
/// `kinds::tests::every_kind_has_exactly_one_row`,
/// `capabilities::tests::every_kind_has_exactly_one_row` and
/// `document_io::tests::a_named_kind_never_falls_through_these_matches`.
///
/// ⚠ `Registered` carries a [`ResourceType`](or_io::container::resource::ResourceType),
/// not a [`FormatId`](crate::state::format_handlers::FormatId): a kind is
/// generation-agnostic, so carrying the target would give a registered format
/// two kinds where every builtin has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentKind {
    Ypt,
    Ytd,
    Ydr,
    Ydd,
    Yft,
    Ybn,
    Yld,
    Ycd,
    Yed,
    Yfd,
    Ynd,
    Ynv,
    Ypdb,
    Fxc,
    Gxt2,
    Bik,
    Awc,
    HeightMap,
    DistantLights,
    WaterMap,
    WorldBlanket,
    NavPrecalc,
    DatTable,
    Rel,
    NameTable,
    Gfx,
    MoveNetwork,
    Dds,
    Ttpf,
    Rpf,
    Rpf9,
    Gta6Anim,
    Gta6Drawable,
    Gta6Text,
    Gta6AudioData,
    Gta6Expressions,
    Gta6Save,
    GameRoot,
    Folder,
    Text,
    Meta,
    Hex,
    Compare,
    Registered(or_io::container::resource::ResourceType),
}

pub const TEXT_DOCUMENT_EXTENSIONS: &[&str] = &[
    "xml", "meta", "ide", "ipl", "ini", "txt", "log", "json", "vdf", "sps", "ugc", "dat", "list",
    "lua",
];

impl DocumentKind {
    /// Every variant the enum names, [`Self::Registered`] excepted.
    ///
    /// ⚠ The one list: `kinds::KINDS` and `capabilities::BUILTIN` are checked
    /// against it, [`Self::from_variant_name`] reads it, and
    /// `doc_counts::the_named_kinds_are_the_variants_the_enum_declares` checks
    /// it against the enum's declaration by parsing the source.
    pub const NAMED: &'static [Self] = &[
        Self::Ypt,
        Self::Ytd,
        Self::Ydr,
        Self::Ydd,
        Self::Yft,
        Self::Ybn,
        Self::Yld,
        Self::Ycd,
        Self::Yed,
        Self::Yfd,
        Self::Ynd,
        Self::Ynv,
        Self::Ypdb,
        Self::Fxc,
        Self::Gxt2,
        Self::Bik,
        Self::Awc,
        Self::HeightMap,
        Self::DistantLights,
        Self::WaterMap,
        Self::WorldBlanket,
        Self::NavPrecalc,
        Self::DatTable,
        Self::Rel,
        Self::NameTable,
        Self::Gfx,
        Self::MoveNetwork,
        Self::Dds,
        Self::Ttpf,
        Self::Rpf,
        Self::Rpf9,
        Self::Gta6Anim,
        Self::Gta6Drawable,
        Self::Gta6Text,
        Self::Gta6AudioData,
        Self::Gta6Expressions,
        Self::Gta6Save,
        Self::GameRoot,
        Self::Folder,
        Self::Text,
        Self::Meta,
        Self::Hex,
        Self::Compare,
    ];

    pub(in crate::state) fn for_resource(
        resource: or_io::container::resource::ResourceType,
    ) -> Self {
        use or_io::container::resource::ResourceType as R;
        match resource {
            R::Ypt => Self::Ypt,
            R::Ytd => Self::Ytd,
            R::Ydr => Self::Ydr,
            R::Ydd => Self::Ydd,
            R::Yft => Self::Yft,
            R::Ybn => Self::Ybn,
            R::Ycd => Self::Ycd,
            R::Yld => Self::Yld,
            R::Yed => Self::Yed,
            R::Yfd => Self::Yfd,
            R::Ynd => Self::Ynd,
            R::Ynv => Self::Ynv,
            // Each stays on the enum's open end for its own reason:
            //
            // `.ypm`: a retail install ships zero, and nothing reads one.
            //
            // `.ybd`: four files, one at a version this build describes. It
            // opens as an owned dictionary, and the built-in description
            // draws its root grid.
            //
            // `.ysc`: 3,022 files across two installs and every one reads. It
            // opens read-only as decompiled source in the Data workspace, with
            // its disassembly listing in the inspector; there is no `#[record]`
            // declaration to describe (OD-YSC).
            //
            // `.yvr` and `.ywr`: each opens owned and shows its described root,
            // and nothing in the app asks for more.
            R::Ypm | R::Ybd | R::Ysc | R::Yvr | R::Ywr => Self::Registered(resource),
        }
    }

    /// ⚠ Text only if an owned root's extension names no resource, which
    /// `every_owned_row_is_its_own_kind` refuses; a text kind has no writer for it.
    pub(super) fn of_asset(asset: &or_io::Asset) -> Self {
        or_io::container::resource::ResourceType::from_extension(asset.ext())
            .map_or(Self::Text, Self::for_resource)
    }

    pub fn resource_type(self) -> Option<or_io::container::resource::ResourceType> {
        use or_io::container::resource::ResourceType as R;
        match self {
            Self::Ypt => Some(R::Ypt),
            Self::Ytd => Some(R::Ytd),
            Self::Ydr => Some(R::Ydr),
            Self::Ydd => Some(R::Ydd),
            Self::Yft => Some(R::Yft),
            Self::Ybn => Some(R::Ybn),
            Self::Ycd => Some(R::Ycd),
            Self::Yld => Some(R::Yld),
            Self::Yed => Some(R::Yed),
            Self::Yfd => Some(R::Yfd),
            Self::Ynd => Some(R::Ynd),
            Self::Ynv => Some(R::Ynv),
            Self::Registered(resource) => Some(resource),
            // `.ypdb` and `.fxc` are not RSC7 resources, so they take GXT2's
            // route rather than `payload_handlers`, which is keyed by
            // `ResourceType`.
            Self::Ypdb | Self::Fxc => None,
            Self::Gxt2 | Self::Bik | Self::Awc | Self::Rpf | Self::Text | Self::Meta => None,
            Self::HeightMap | Self::DistantLights => None,
            Self::WaterMap | Self::WorldBlanket | Self::NavPrecalc => None,
            Self::DatTable => None,
            Self::Rel | Self::NameTable => None,
            Self::Gfx | Self::Dds | Self::Ttpf | Self::Rpf9 | Self::MoveNetwork => None,
            Self::Gta6Anim | Self::Gta6Drawable | Self::Gta6Text | Self::Gta6AudioData => None,
            Self::Gta6Expressions | Self::Gta6Save => None,
            Self::GameRoot | Self::Folder | Self::Hex | Self::Compare => None,
        }
    }

    pub fn registered() -> Vec<Self> {
        let mut out: Vec<Self> = Vec::new();
        for handler in formats::handlers() {
            let kind = Self::for_resource(handler.id.resource);
            if matches!(kind, Self::Registered(_)) && !out.contains(&kind) {
                out.push(kind);
            }
        }
        out.sort_by_key(|kind| kind.resource_type());
        out
    }

    pub fn extensions(self) -> &'static [&'static str] {
        let Self::Registered(resource) = self else {
            return self.resource_type().map(|r| r.extensions()).unwrap_or(&[]);
        };
        formats::handlers()
            .into_iter()
            .find(|h| h.id.resource == resource && !h.extensions.is_empty())
            .map(|h| h.extensions)
            .unwrap_or_else(|| resource.extensions())
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();

        if let Some(handler) = formats::by_extension(&ext).first() {
            return Some(Self::for_resource(handler.id.resource));
        }

        match ext.as_str() {
            "gxt2" => Some(Self::Gxt2),
            "bik" | "bk2" => Some(Self::Bik),
            "awc" => Some(Self::Awc),
            // A DLC ships each `.datNN.rel` beside a `.datNN`, the same container.
            "rel" | "dat4" | "dat10" | "dat15" | "dat54" | "dat149" | "dat150" | "dat151" => {
                Some(Self::Rel)
            }
            "nametable" => Some(Self::NameTable),
            "gfx" => Some(Self::Gfx),
            "mrf" => Some(Self::MoveNetwork),
            "dds" => Some(Self::Dds),
            "ttpf" => Some(Self::Ttpf),
            "ypdb" => Some(Self::Ypdb),
            "fxc" => Some(Self::Fxc),
            "rpf" => Some(Self::Rpf),
            "meta" | "ymt" | "ymf" | "pso" | "ymap" | "ytyp" | "cut" | "srl" => Some(Self::Meta),
            "dat" => Some(world_dat_kind(path).unwrap_or(Self::Text)),
            // `TextDocument::open` refuses a binary `.dat` no reader opens.
            other if TEXT_DOCUMENT_EXTENSIONS.contains(&other) => Some(Self::Text),
            _ => None,
        }
    }

    pub fn refine_with_bytes(self, bytes: &[u8]) -> Self {
        if !matches!(self, Self::Text) {
            return self;
        }
        if or_io::meta::detect_binary_format(bytes).is_some() {
            return Self::Meta;
        }
        if let Some(root) = meta::peek_xml_root(bytes) {
            if or_io::meta::is_known_root(meta_schema(), &root)
                || or_io::meta::instance::parse_schemaless(bytes).is_ok()
            {
                return Self::Meta;
            }
        }
        self
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Ypt => "ypt",
            Self::Ytd => "ytd",
            Self::Ydr => "ydr",
            Self::Ydd => "ydd",
            Self::Yft => "yft",
            Self::Ybn => "ybn",
            Self::Yld => "yld",
            Self::Ycd => "ycd",
            Self::Yed => "yed",
            Self::Yfd => "yfd",
            Self::Ynd => "ynd",
            Self::Ynv => "ynv",
            Self::Ypdb => "ypdb",
            Self::Fxc => "fxc",
            Self::Gxt2 => "gxt2",
            Self::Bik => "bik",
            Self::Awc => "awc",
            Self::HeightMap | Self::DistantLights => "dat",
            Self::WaterMap | Self::WorldBlanket | Self::NavPrecalc => "dat",
            Self::DatTable => "dat",
            Self::Rel => "rel",
            Self::NameTable => "nametable",
            Self::Gfx => "gfx",
            Self::MoveNetwork => "mrf",
            Self::Dds => "dds",
            Self::Ttpf => "ttpf",
            Self::Rpf | Self::Rpf9 => "rpf",
            Self::GameRoot | Self::Gta6Text | Self::Gta6AudioData => "",
            Self::Folder | Self::Gta6Expressions | Self::Gta6Save => "",
            Self::Hex | Self::Compare | Self::Gta6Anim | Self::Gta6Drawable => "",
            Self::Text => "txt",
            Self::Meta => "meta",
            Self::Registered(_) => self.extensions().first().copied().unwrap_or(""),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Ypt => "YPT",
            Self::Ytd => "YTD",
            Self::Ydr => "YDR",
            Self::Ydd => "YDD",
            Self::Yft => "YFT",
            Self::Ybn => "YBN",
            Self::Yld => "YLD",
            Self::Ycd => "YCD",
            Self::Yed => "YED",
            Self::Yfd => "YFD",
            Self::Ynd => "YND",
            Self::Ynv => "YNV",
            Self::Ypdb => "YPDB",
            Self::Fxc => "FXC",
            Self::Gxt2 => "GXT2",
            Self::Bik => "BIK",
            Self::Awc => "AWC",
            Self::HeightMap => "HEIGHTMAP",
            Self::DistantLights => "LIGHTS",
            Self::WaterMap => "WATERMAP",
            Self::WorldBlanket => "BLANKET",
            Self::NavPrecalc => "NAVPRECALC",
            Self::DatTable => "TABLE",
            Self::Rel => "REL",
            Self::NameTable => "NAMETABLE",
            Self::Gfx => "GFX",
            Self::MoveNetwork => "MRF",
            Self::Dds => "DDS",
            Self::Ttpf => "TTPF",
            Self::Rpf => "RPF",
            Self::Rpf9 => "RPF9",
            Self::Gta6Anim => "ANIM",
            Self::Gta6Drawable => "DRAWABLE",
            Self::Gta6Text => "TEXT TABLE",
            Self::Gta6AudioData => "AUDIO DATA",
            Self::Gta6Expressions => "EXPRESSIONS",
            Self::Gta6Save => "SAVE",
            Self::GameRoot => "GAME",
            Self::Folder => "FOLDER",
            Self::Text => "TEXT",
            Self::Meta => "META",
            Self::Hex => "HEX",
            Self::Compare => "COMPARE",
            Self::Registered(resource) => resource.extension(),
        }
    }
}

/// The kind a binary world `.dat` opens as, by its file name: the game names each one.
fn world_dat_kind(path: &Path) -> Option<DocumentKind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.starts_with("heightmap") {
        Some(DocumentKind::HeightMap)
    } else if name.starts_with("distantlights") {
        Some(DocumentKind::DistantLights)
    } else if name.starts_with("waterheight") {
        Some(DocumentKind::WaterMap)
    } else if name.starts_with("worldblanket") {
        Some(DocumentKind::WorldBlanket)
    } else if name.starts_with("navprecalc") {
        Some(DocumentKind::NavPrecalc)
    } else if DatTable::names(&name) {
        Some(DocumentKind::DatTable)
    } else {
        None
    }
}
