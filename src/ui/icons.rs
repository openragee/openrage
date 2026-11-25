use egui::Color32;
use egui_phosphor::regular as ph;

use crate::state::{DocumentKind, RpfNodeBacking, RpfSessionNode, RpfSessionNodeKind};
use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Dirty,
    Loading,
    Error,
    Recovered,
    New,
    Unsupported,
}

impl BadgeKind {
    fn glyph(self) -> &'static str {
        match self {
            BadgeKind::Dirty => "●",
            BadgeKind::Loading => ph::SPINNER,
            BadgeKind::Error => ph::WARNING,
            BadgeKind::Recovered => ph::SHIELD_CHECK,
            BadgeKind::New => ph::PLUS_CIRCLE,
            BadgeKind::Unsupported => ph::PROHIBIT,
        }
    }

    fn color(self) -> Color32 {
        match self {
            BadgeKind::Dirty => theme::STATE_DIRTY.get(),
            BadgeKind::Loading => theme::STATE_LOADING.get(),
            BadgeKind::Error => theme::STATE_ERROR.get(),
            BadgeKind::Recovered => theme::STATE_RECOVERED.get(),
            BadgeKind::New => theme::STATE_NEW.get(),
            BadgeKind::Unsupported => theme::TEXT_SECONDARY.get(),
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            BadgeKind::Dirty => tr!("Unsaved changes"),
            BadgeKind::Loading => tr!("Loading…"),
            BadgeKind::Error => tr!("Failed to open"),
            BadgeKind::Recovered => tr!("Recovered via fallback path"),
            BadgeKind::New => tr!("New (in-memory)"),
            BadgeKind::Unsupported => tr!("OpenRage cannot open this file"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IconBadge {
    pub glyph: &'static str,
    pub color: Color32,
    pub badge: Option<BadgeKind>,
    pub type_tooltip: &'static str,
}

impl IconBadge {
    fn new(glyph: &'static str, color: Color32, type_tooltip: &'static str) -> Self {
        Self {
            glyph,
            color: theme::legible(color, theme::GRAPHIC),
            badge: None,
            type_tooltip,
        }
    }

    pub fn with_badge(mut self, badge: BadgeKind) -> Self {
        self.badge = Some(badge);
        self
    }

    /// A file OpenRage cannot open: dimmed, and badged unless a badge says more, since the dimmed colour is the generic file icon's own.
    pub fn dimmed(mut self) -> Self {
        self.color = theme::TEXT_DISABLED.get();
        self.badge = self.badge.or(Some(BadgeKind::Unsupported));
        self
    }

    pub fn tooltip(self) -> String {
        match self.badge {
            Some(b) => format!("{}\n{}", self.type_tooltip, b.tooltip()),
            None => self.type_tooltip.to_string(),
        }
    }
}

pub fn doc_kind_icon(kind: DocumentKind, dirty: bool) -> IconBadge {
    let info = kind.info();
    let mut icon = IconBadge::new(info.glyph, info.color, info.tooltip);
    if dirty {
        icon = icon.with_badge(BadgeKind::Dirty);
    }
    icon
}

#[derive(Debug, Clone, Copy)]
pub enum YptItem {
    EffectsSection,
    Effect,
    Emitter,
    Particle,
    EmitterRuleLibrary,
    ParticleRuleLibrary,
    TextureDict,
    ModelDict,
    Domain,
    Evolution,
    Behaviour,
}

pub fn ypt_icon(item: YptItem) -> IconBadge {
    match item {
        YptItem::EffectsSection => IconBadge::new(
            ph::STACK_SIMPLE,
            theme::KIND_YPT,
            tr!("Particle effect rules"),
        ),
        YptItem::Effect => {
            IconBadge::new(ph::LIGHTNING, theme::KIND_YPT, tr!("Particle effect rule"))
        }
        YptItem::Emitter => {
            IconBadge::new(ph::DROP_HALF, theme::KIND_YPT, tr!("Event emitter slot"))
        }
        YptItem::Particle => {
            IconBadge::new(ph::SPARKLE, theme::KIND_YPT, tr!("Particle rule reference"))
        }
        YptItem::EmitterRuleLibrary => {
            IconBadge::new(ph::DATABASE, theme::KIND_YPT, tr!("Emitter rule library"))
        }
        YptItem::ParticleRuleLibrary => {
            IconBadge::new(ph::BOOKS, theme::KIND_YPT, tr!("Particle rule library"))
        }
        YptItem::TextureDict => IconBadge::new(
            ph::IMAGES_SQUARE,
            theme::KIND_YTD,
            tr!("Embedded texture dictionary"),
        ),
        YptItem::ModelDict => IconBadge::new(
            ph::CUBE_FOCUS,
            theme::KIND_YDR,
            tr!("Embedded model / drawable dictionary"),
        ),
        YptItem::Domain => IconBadge::new(
            ph::TARGET,
            theme::TEXT_SECONDARY.get(),
            tr!("Spawn / target / attractor domain"),
        ),
        YptItem::Evolution => IconBadge::new(
            ph::FUNCTION,
            theme::TEXT_SECONDARY.get(),
            tr!("Evolution / parameter curve"),
        ),
        YptItem::Behaviour => IconBadge::new(
            ph::FADERS,
            theme::TEXT_SECONDARY.get(),
            tr!("Particle behaviour"),
        ),
    }
}

#[derive(Debug, Clone, Copy)]
pub enum YdrSection {
    Lods,
    Lod,
    Model,
    Geometry,
    Shaders,
    Shader,
    Skeleton,
    Bone,
    Lights,
    Light,
    Bound,
}

pub fn ydr_section_icon(section: YdrSection) -> IconBadge {
    match section {
        YdrSection::Lods => {
            IconBadge::new(ph::STACK, theme::KIND_YDR, tr!("Level-of-detail groups"))
        }
        YdrSection::Lod => IconBadge::new(ph::CIRCLES_THREE, theme::KIND_YDR, tr!("LOD bucket")),
        YdrSection::Model => IconBadge::new(ph::CUBE_FOCUS, theme::KIND_YDR, tr!("Model")),
        YdrSection::Geometry => IconBadge::new(ph::POLYGON, theme::KIND_YDR, tr!("Geometry")),
        YdrSection::Shaders => IconBadge::new(ph::PALETTE, theme::KIND_YDR, tr!("Shader group")),
        YdrSection::Shader => IconBadge::new(ph::PAINT_BRUSH, theme::KIND_YDR, tr!("Shader")),
        YdrSection::Skeleton => {
            IconBadge::new(ph::TREE_STRUCTURE, theme::KIND_YDR, tr!("Skeleton"))
        }
        YdrSection::Bone => IconBadge::new(ph::TREE_VIEW, theme::KIND_YDR, tr!("Bone")),
        YdrSection::Lights => IconBadge::new(ph::LIGHTBULB, theme::KIND_YDR, tr!("Lights")),
        YdrSection::Light => IconBadge::new(ph::LIGHTBULB_FILAMENT, theme::KIND_YDR, tr!("Light")),
        YdrSection::Bound => IconBadge::new(ph::CUBE_TRANSPARENT, theme::KIND_YBN, tr!("Bound")),
    }
}

/// A bound's badge: a container, a polygon mesh, or an analytical primitive.
pub fn bound_icon(composite: bool, polygonal: bool) -> IconBadge {
    if composite {
        return IconBadge::new(
            ph::CUBE_TRANSPARENT,
            theme::KIND_YBN,
            tr!("Composite bound (container)"),
        );
    }
    if polygonal {
        IconBadge::new(ph::TRIANGLE, theme::KIND_YBN, tr!("Polygonal bound (mesh)"))
    } else {
        IconBadge::new(
            ph::SPHERE,
            theme::KIND_YBN,
            tr!("Primitive bound (analytical)"),
        )
    }
}

pub fn ytd_texture_icon() -> IconBadge {
    IconBadge::new(ph::IMAGE_SQUARE, theme::KIND_YTD, tr!("Texture"))
}

pub fn folder_icon(open: bool) -> IconBadge {
    let glyph = if open { ph::FOLDER_OPEN } else { ph::FOLDER };
    IconBadge::new(glyph, theme::KIND_FOLDER, tr!("Folder"))
}

/// A Files row's icon; `unsaved` marks an archive row whose archive holds an edit no save has written.
pub fn rpf_node_icon(
    node: &RpfSessionNode,
    unsaved: bool,
    in_flight: bool,
    supported: bool,
    open: bool,
) -> IconBadge {
    let icon = match &node.backing {
        RpfNodeBacking::Rpf9Archive {
            open_error: Some(_),
            ..
        } => IconBadge::new(
            ph::ARCHIVE,
            theme::STATE_ERROR.get(),
            tr!("RPF archive (failed to open)"),
        )
        .with_badge(BadgeKind::Error),
        RpfNodeBacking::Rpf9Archive { .. } => {
            IconBadge::new(ph::ARCHIVE, theme::KIND_RPF, tr!("RPF archive"))
        }
        RpfNodeBacking::Rpf9Dir { .. } => folder_icon(open),
        RpfNodeBacking::Rpf9File { .. } => file_icon_for_node(node),
        RpfNodeBacking::Root => folder_icon(open),
        RpfNodeBacking::DiskFolder { .. } => folder_icon(open),
        RpfNodeBacking::InMemoryDir => folder_icon(open).with_badge(BadgeKind::New),
        RpfNodeBacking::Archive {
            open_error: Some(_),
            ..
        } => IconBadge::new(
            ph::ARCHIVE,
            theme::STATE_ERROR.get(),
            tr!("RPF archive (failed to open)"),
        )
        .with_badge(BadgeKind::Error),
        RpfNodeBacking::Archive { recovered_hint, .. } => {
            let mut icon = IconBadge::new(ph::ARCHIVE, theme::KIND_RPF, tr!("RPF archive"));
            if unsaved {
                icon = icon.with_badge(BadgeKind::Dirty);
            } else if recovered_hint.is_some() {
                icon = icon.with_badge(BadgeKind::Recovered);
            }
            icon
        }
        RpfNodeBacking::ArchiveDir { .. } => {
            let glyph = if open {
                ph::FOLDER_NOTCH_OPEN
            } else {
                ph::FOLDER_NOTCH
            };
            IconBadge::new(glyph, theme::KIND_FOLDER, tr!("Folder inside RPF"))
        }
        RpfNodeBacking::ArchiveFile { override_bytes, .. } => {
            let mut icon = file_icon_for_node(node);
            if override_bytes.is_some() {
                icon = icon.with_badge(BadgeKind::Dirty);
            }
            icon
        }
        RpfNodeBacking::InMemoryFile { .. } => file_icon_for_node(node).with_badge(BadgeKind::New),
        RpfNodeBacking::DiskFile { override_bytes, .. } => {
            let mut icon = file_icon_for_node(node);
            if override_bytes.is_some() {
                icon = icon.with_badge(BadgeKind::Dirty);
            }
            icon
        }
    };

    let mut icon = if in_flight {
        IconBadge {
            badge: Some(BadgeKind::Loading),
            ..icon
        }
    } else {
        icon
    };
    if !supported && !node.is_dir() {
        icon = icon.dimmed();
    }
    icon
}

fn file_icon_for_node(node: &RpfSessionNode) -> IconBadge {
    if let Some(kind) = node.rpf9_type() {
        return rpf9_icon(kind);
    }
    let ext = std::path::Path::new(&node.name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    if let Some(ext) = ext.as_deref() {
        if let Some(kind) = DocumentKind::from_path(std::path::Path::new(&node.name)) {
            if matches!(kind, DocumentKind::Rpf) {
                return IconBadge::new(ph::ARCHIVE, theme::KIND_RPF, tr!("RPF archive"));
            }
            return doc_kind_icon(kind, false);
        }
        if matches!(
            ext,
            "xml" | "meta" | "ide" | "ipl" | "ini" | "txt" | "log" | "json" | "vdf" | "sps" | "ugc"
        ) {
            return IconBadge::new(ph::FILE_TEXT, theme::KIND_TEXT, tr!("Text document"));
        }
    }
    match node.kind {
        RpfSessionNodeKind::ResourceFile => IconBadge::new(
            ph::FILE_CODE,
            theme::TEXT_PRIMARY.get(),
            tr!("RAGE resource file"),
        ),
        _ => IconBadge::new(ph::FILE, theme::TEXT_SECONDARY.get(), tr!("Binary file")),
    }
}

fn rpf9_icon(kind: or_io::container::rpf::Gta6EntryType) -> IconBadge {
    use or_io::container::rpf::{Gta6EntryType, Rpf9Role};
    use or_io::gta6::other_bodies::OtherBody;
    let document = match kind {
        Gta6EntryType::Archive => Some(DocumentKind::Rpf9),
        Gta6EntryType::TextureDictionary => Some(DocumentKind::Ytd),
        Gta6EntryType::Awc => Some(DocumentKind::Awc),
        Gta6EntryType::Resource(Rpf9Role::Drawable) => Some(DocumentKind::Ydr),
        Gta6EntryType::Resource(Rpf9Role::DrawableWithBound) => Some(DocumentKind::Yft),
        Gta6EntryType::Resource(Rpf9Role::Bound) => Some(DocumentKind::Ybn),
        Gta6EntryType::Resource(Rpf9Role::NavMesh) => Some(DocumentKind::Ynv),
        Gta6EntryType::Resource(Rpf9Role::Cloth) => Some(DocumentKind::Yld),
        Gta6EntryType::Resource(Rpf9Role::ParticleEffects) => Some(DocumentKind::Ypt),
        Gta6EntryType::Body(OtherBody::Gfx) => Some(DocumentKind::Gfx),
        Gta6EntryType::Body(OtherBody::HeightMap) => Some(DocumentKind::HeightMap),
        Gta6EntryType::Body(OtherBody::MoveNetwork) => Some(DocumentKind::MoveNetwork),
        Gta6EntryType::Rbf | Gta6EntryType::Pso => Some(DocumentKind::Meta),
        _ => None,
    };
    let mut icon = document
        .map(|kind| doc_kind_icon(kind, false))
        .unwrap_or_else(|| {
            let glyph = if matches!(kind, Gta6EntryType::Unknown) {
                ph::FILE
            } else {
                ph::FILE_CODE
            };
            IconBadge::new(glyph, theme::TEXT_SECONDARY.get(), tr!("Binary file"))
        });
    icon.type_tooltip = crate::state::files_view::rpf9_type_name(kind);
    icon
}

pub fn icon_label(
    icon: &IconBadge,
    label: impl Into<String>,
    label_color: Color32,
) -> egui::text::LayoutJob {
    icon_label_sized(icon, label, label_color, None)
}

pub fn icon_label_small(
    icon: &IconBadge,
    label: impl Into<String>,
    label_color: Color32,
) -> egui::text::LayoutJob {
    icon_label_sized(icon, label, label_color, Some(SMALL_SIZE))
}

const SMALL_SIZE: f32 = 10.0;

fn icon_label_sized(
    icon: &IconBadge,
    label: impl Into<String>,
    label_color: Color32,
    label_size: Option<f32>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let body_font = match label_size {
        Some(sz) => egui::FontId::new(sz, egui::FontFamily::Proportional),
        None => egui::FontId::default(),
    };
    let icon_font = match label_size {
        Some(sz) => egui::FontId::new(sz, egui::FontFamily::Proportional),
        None => egui::FontId::default(),
    };
    let badge_font = egui::FontId::new(body_font.size * 0.85, egui::FontFamily::Proportional);

    job.append(
        icon.glyph,
        0.0,
        egui::TextFormat {
            color: icon.color,
            font_id: icon_font,
            ..Default::default()
        },
    );
    if let Some(badge) = icon.badge {
        job.append(
            badge.glyph(),
            2.0,
            egui::TextFormat {
                color: badge.color(),
                font_id: badge_font,
                valign: egui::Align::TOP,
                ..Default::default()
            },
        );
    }
    let label_text = label.into();
    if !label_text.is_empty() {
        job.append(
            &label_text,
            theme::ICON_GUTTER,
            egui::TextFormat {
                color: label_color,
                font_id: body_font,
                ..Default::default()
            },
        );
    }
    job
}

pub fn glyph_with_label(icon: &IconBadge, label: &str) -> egui::text::LayoutJob {
    icon_label(icon, label, theme::TEXT_PRIMARY.get())
}
