//! Nothing draws over what it belongs to: the text editor's footer sits below
//! its lines and its find bar inside it, the texture view's header and encode
//! bar stay within the panel, the viewport's overlay within its box, and
//! toasts clear the job list. Each is drawn where the default layout puts its
//! panel on the default window, and again as narrow as the default layouts
//! make a panel.

use std::path::PathBuf;

use egui::{Rect, Vec2};
use or_io::codec::texture::GrcTextureFormat;
use or_io::gta5::{gen8, PgDictionary};

use super::jobs::{JobKind, Outcome, Then};
use super::{
    textures, AppState, DocPayload, Document, DocumentId, Prefs, TextDocument, TextureListing,
};
use crate::state::AppearancePrefs;
use crate::ui::dock::{Tab, Workspace};

pub(super) fn window((width, height): (u32, u32)) -> Vec2 {
    egui::vec2(width as f32, height as f32)
}

pub(super) fn input(screen: Vec2, time: f64, events: Vec<egui::Event>) -> egui::RawInput {
    let modifiers = events
        .iter()
        .find_map(|event| match event {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, screen)),
        time: Some(time),
        modifiers,
        events,
        ..Default::default()
    }
}

/// The context the editor draws with: its fonts, its spacing and its theme.
pub(super) fn app_context() -> egui::Context {
    crate::app::gpu::configured_egui_context(None, &AppearancePrefs::default())
}

/// Where the default layout of `workspace` puts `tab` on a `screen`-sized window.
fn panel(workspace: Workspace, tab: Tab, screen: Vec2) -> Rect {
    let mut state = AppState::from_prefs(Prefs::default());
    state.active_workspace = workspace;
    let ctx = app_context();
    for frame in 0..2 {
        ctx.begin_pass(input(screen, f64::from(frame), Vec::new()));
        crate::ui::draw(&ctx, &mut state, None);
        let _ = ctx.end_pass();
    }
    let dock = state.workspace_docks.get_mut(workspace);
    let found = dock
        .iter_leaves()
        .find(|(_, leaf)| leaf.tabs.contains(&tab));
    found
        .map(|(_, leaf)| leaf.viewport)
        .unwrap_or_else(|| panic!("the default {workspace:?} layout has no {tab:?}"))
}

/// The panel at the default window, and the same panel as narrow as the
/// Inspector's column, the narrowest the default layouts make, at the smallest window.
pub(super) fn default_and_narrow(workspace: Workspace, tab: Tab) -> [Rect; 2] {
    let default = panel(workspace, tab, window(crate::app::EDITOR_SIZE));
    let narrowest = panel(
        Workspace::Particle,
        Tab::DETAILS,
        window(crate::app::EDITOR_MIN_SIZE),
    );
    let narrow = Rect::from_min_size(default.min, egui::vec2(narrowest.width(), default.height()));
    [default, narrow]
}

/// Frames of one panel given `rect`, as a dock tab gives it, on a screen the size of the default window.
pub(super) struct Panel {
    pub(super) ctx: egui::Context,
    pub(super) rect: Rect,
    time: f64,
}

impl Panel {
    pub(super) fn new(rect: Rect) -> Self {
        let ctx = app_context();
        ctx.enable_accesskit();
        Self {
            ctx,
            rect,
            time: 0.0,
        }
    }

    pub(super) fn frame(
        &mut self,
        events: Vec<egui::Event>,
        mut draw: impl FnMut(&mut egui::Ui),
    ) -> egui::FullOutput {
        self.time += 1.0 / 30.0;
        let screen = window(crate::app::EDITOR_SIZE);
        let rect = self.rect;
        self.ctx.run_ui(input(screen, self.time, events), |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                draw(ui);
            });
        })
    }
}

/// A text a frame painted: what it says, where, and the rectangle it was clipped to.
#[derive(Debug)]
pub(super) struct Painted {
    pub(super) text: String,
    pub(super) rect: Rect,
    pub(super) clip: Rect,
}

pub(super) fn painted(output: &egui::FullOutput) -> Vec<Painted> {
    fn walk(shape: &egui::Shape, clip: Rect, out: &mut Vec<Painted>) {
        match shape {
            egui::Shape::Text(text) => out.push(Painted {
                text: text.galley.text().to_string(),
                rect: text.galley.rect.translate(text.pos.to_vec2()),
                clip,
            }),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, clipped.clip_rect, &mut out);
    }
    out
}

/// Every widget a frame drew that says something, and its rectangle.
pub(super) fn widgets(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let Some(update) = &output.platform_output.accesskit_update else {
        return Vec::new();
    };
    (update.nodes.iter())
        .filter_map(|(_, node)| {
            let text = node.label().or_else(|| node.value())?;
            let b = node.bounds()?;
            let rect = Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            );
            Some((text.to_string(), rect))
        })
        .collect()
}

/// Where each area drawn over the panels sat in the last frame.
pub(super) fn areas_over_the_panels(ctx: &egui::Context) -> Vec<Rect> {
    ctx.memory(|m| {
        (m.areas().visible_layer_ids().into_iter())
            .filter(|layer| layer.order == egui::Order::Foreground)
            .filter_map(|layer| m.area_rect(layer.id))
            .collect()
    })
}

/// Whether two rectangles share any area; touching edges share none.
pub(super) fn overlap(a: Rect, b: Rect) -> bool {
    let shared = a.intersect(b);
    shared.width() > 0.01 && shared.height() > 0.01
}

pub(super) fn within(outer: Rect, inner: Rect) -> bool {
    outer.expand(0.01).contains_rect(inner)
}

/// Forty lines, each wider than any panel, so whatever sits on one covers its text.
pub(super) fn long_lines() -> String {
    (1..=40)
        .map(|n| format!("line {n}: {}", "a word that runs on ".repeat(12)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` open in the editor as `C:/mods/<name>`, in the Data workspace.
pub(super) fn editing(name: &str, text: &str) -> AppState {
    let mut state = AppState::from_prefs(Prefs::default());
    state.documents.clear();
    let id: DocumentId = 4243;
    state.documents.push(Document::new(
        id,
        Some(PathBuf::from(format!("C:/mods/{name}"))),
        DocPayload::Text(TextDocument::decode(text.as_bytes())),
    ));
    state.active_document = Some(id);
    state.active_workspace = Workspace::Data;
    state
}

/// The editor's own lines (their numbers and their text) and every other text it painted.
pub(super) fn lines_and_the_rest(
    output: &egui::FullOutput,
    text: &str,
) -> (Vec<Painted>, Vec<Painted>) {
    painted(output)
        .into_iter()
        .partition(|p| p.text.parse::<usize>().is_ok() || text.lines().any(|line| line == p.text))
}

pub(super) fn draw_editor(
    panel: &mut Panel,
    state: &mut AppState,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    panel.frame(events, |ui| crate::ui::panels::text_editor::draw(ui, state))
}

#[test]
fn the_text_editors_footer_sits_below_its_lines() {
    let _serial = crate::ui::theme::serial();
    let text = long_lines();
    for rect in default_and_narrow(Workspace::Data, Tab::TEXT_EDITOR) {
        let mut state = editing("a_settings_file_with_a_long_name.txt", &text);
        let mut panel = Panel::new(rect);
        let mut output = draw_editor(&mut panel, &mut state, Vec::new());
        for _ in 0..2 {
            output = draw_editor(&mut panel, &mut state, Vec::new());
        }
        let (lines, footer) = lines_and_the_rest(&output, &text);
        assert!(
            footer.iter().any(|p| p.text.starts_with("Ln ")),
            "no footer in {rect:?}: {footer:?}"
        );
        for (i, part) in footer.iter().enumerate() {
            assert!(
                within(rect, part.rect),
                "the footer's `{}` at {:?} leaves the editor at {rect:?}",
                part.text,
                part.rect
            );
            for line in &lines {
                assert!(
                    !overlap(part.rect, line.rect.intersect(line.clip)),
                    "the footer's `{}` at {:?} is drawn over `{}` at {:?}",
                    part.text,
                    part.rect,
                    line.text,
                    line.rect
                );
            }
            for other in &footer[i + 1..] {
                assert!(
                    !overlap(part.rect, other.rect),
                    "the footer's `{}` and `{}` overlap in {rect:?}",
                    part.text,
                    other.text
                );
            }
        }
    }
}

#[test]
fn the_find_bar_sits_inside_the_editor() {
    let _serial = crate::ui::theme::serial();
    let text = long_lines();
    for rect in default_and_narrow(Workspace::Data, Tab::TEXT_EDITOR) {
        let mut state = editing("notes.txt", &text);
        let mut panel = Panel::new(rect);
        let at = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let find = egui::Event::Key {
            key: egui::Key::F,
            physical_key: Some(egui::Key::F),
            pressed: true,
            repeat: false,
            modifiers: crate::ui::shortcuts::PRIMARY_EGUI,
        };
        for events in [
            Vec::new(),
            vec![egui::Event::PointerMoved(at), press(true)],
            vec![press(false)],
            vec![find],
            Vec::new(),
        ] {
            draw_editor(&mut panel, &mut state, events);
        }
        let output = draw_editor(&mut panel, &mut state, Vec::new());
        let (lines, rest) = lines_and_the_rest(&output, &text);
        let editor = (lines.iter().find(|p| p.text == "1"))
            .map(|p| p.clip)
            .unwrap_or_else(|| panic!("the editor drew no first line in {rect:?}"));
        let bars = areas_over_the_panels(&panel.ctx);
        assert!(!bars.is_empty(), "Ctrl+F opened no find bar in {rect:?}");
        for bar in bars {
            assert!(
                within(editor, bar),
                "the find bar at {bar:?} is not inside the editor's lines at {editor:?}"
            );
            for part in rest.iter().filter(|p| !within(bar, p.rect)) {
                assert!(
                    !overlap(bar, part.rect),
                    "the find bar at {bar:?} covers `{}` at {:?}",
                    part.text,
                    part.rect
                );
            }
        }
    }
}

/// A dictionary of one DXT1 texture with six mips and a long name, selected in the texture view.
pub(super) fn a_texture_on_screen() -> AppState {
    let mut dictionary = PgDictionary::default();
    // 128×128 down to 4×4 in eight-byte 4×4 blocks.
    let bytes: usize = (0..6).map(|level| ((32usize >> level).pow(2)) * 8).sum();
    dictionary.insert(gen8::GrcTexture {
        name: "prop_vehicle_generic_doorshut_n".to_string(),
        width: 128,
        height: 128,
        depth: 1,
        format: GrcTextureFormat::DXT1.to_raw_format(),
        mip_count: 6,
        backing_store: vec![0; bytes],
        ..Default::default()
    });
    let mut state = AppState::from_prefs(Prefs::default());
    state.documents.clear();
    let id: DocumentId = 4242;
    state.documents.push(Document::new(
        id,
        Some(PathBuf::from("C:/mods/props.ytd")),
        DocPayload::Resource(or_io::Asset::TextureDict(dictionary)),
    ));
    state.active_document = Some(id);
    state.active_workspace = Workspace::Texture;
    let payload = &state.document(id).expect("open").payload;
    let TextureListing::Textures { textures, .. } = textures::textures_in(payload, None) else {
        panic!("the dictionary lists no textures");
    };
    let selected = textures.first().map(|t| t.selection_id).expect("listed");
    state.selection.select(id, selected);
    state
}

#[test]
fn the_texture_views_header_and_encode_bar_stay_within_the_panel() {
    let _serial = crate::ui::theme::serial();
    for rect in default_and_narrow(Workspace::Texture, Tab::TEXTURE_PREVIEW) {
        let mut state = a_texture_on_screen();
        let mut panel = Panel::new(rect);
        let mut output = None;
        for _ in 0..3 {
            output = Some(panel.frame(Vec::new(), |ui| {
                crate::ui::panels::texture_preview::draw(ui, &mut state)
            }));
        }
        let parts = widgets(&output.expect("drawn"));
        for wanted in [
            "prop_vehicle_generic_doorshut_n",
            "Reset",
            "Apply",
            "Unembed",
        ] {
            assert!(
                parts.iter().any(|(text, _)| text == wanted),
                "no `{wanted}` in {rect:?}: {parts:?}"
            );
        }
        for (i, (text, part)) in parts.iter().enumerate() {
            assert!(
                within(rect, *part),
                "`{text}` at {part:?} leaves the texture view at {rect:?}"
            );
            for (other, at) in &parts[i + 1..] {
                assert!(
                    !overlap(*part, *at),
                    "`{text}` at {part:?} and `{other}` at {at:?} overlap in {rect:?}"
                );
            }
        }
    }
}

#[test]
fn a_gallery_tile_s_long_name_wraps_short_of_its_size() {
    let _serial = crate::ui::theme::serial();
    for rect in default_and_narrow(Workspace::Texture, Tab::GALLERY) {
        let mut state = a_texture_on_screen();
        let mut panel = Panel::new(rect);
        let mut output = None;
        for _ in 0..3 {
            output = Some(panel.frame(Vec::new(), |ui| {
                crate::ui::panels::gallery::draw(ui, &mut state, None)
            }));
        }
        let shown = painted(&output.expect("drawn"));
        let at = |wanted: &str| {
            (shown.iter())
                .find(|p| p.text.starts_with(wanted))
                .unwrap_or_else(|| panic!("no `{wanted}` in {rect:?}"))
                .rect
        };
        let (name, size) = (at("prop_vehicle_generic_doorshut_n"), at("128×128"));
        assert!(
            !overlap(name, size),
            "{name:?} and {size:?} overlap in {rect:?}"
        );
    }
}

#[test]
fn the_viewports_overlay_stays_within_its_box() {
    let _serial = crate::ui::theme::serial();
    let mut state = AppState::from_prefs(Prefs::default());
    for rect in default_and_narrow(Workspace::Model, Tab::VIEWPORT_3D) {
        let mut panel = Panel::new(rect);
        let output = panel.frame(Vec::new(), |ui| {
            crate::ui::panels::viewport::draw_viewport_overlay(
                ui,
                rect,
                &mut state,
                None,
                None,
                egui::Rect::NOTHING,
            )
        });
        let boxes: Vec<Rect> = (output.shapes.iter())
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(shape) => Some(shape.rect),
                _ => None,
            })
            .collect();
        let [frame] = boxes[..] else {
            panic!("the overlay drew {} boxes: {boxes:?}", boxes.len());
        };
        assert!(
            within(rect, frame),
            "the overlay's box at {frame:?} leaves the viewport at {rect:?}"
        );
        let texts = painted(&output);
        assert!(!texts.is_empty(), "the overlay says nothing");
        for text in texts {
            assert!(
                within(frame, text.rect),
                "`{}` at {:?} runs out of the overlay's box at {frame:?}",
                text.text,
                text.rect
            );
        }
    }
}

#[test]
fn toasts_clear_the_job_list() {
    let _serial = crate::ui::theme::serial();
    for screen in [crate::app::EDITOR_SIZE, crate::app::EDITOR_MIN_SIZE].map(window) {
        let mut state = AppState::from_prefs(Prefs::default());
        let folder = Then::OpenFolder(PathBuf::from("C:/mods/out"));
        state.jobs.record(
            JobKind::Extraction,
            Outcome::Done(folder),
            "Extracted 3 files into C:\\mods\\out",
        );
        state.jobs.record(
            JobKind::Search,
            Outcome::Done(Then::Nothing),
            "12 matches in 3 files",
        );
        crate::ui::panels::jobs::open(&mut state);
        state.show_toast("Folder opened");
        state.show_error_toast("Could not save props.ytd: another program holds the file open");
        let ctx = app_context();
        for frame in 0..3 {
            ctx.begin_pass(input(screen, f64::from(frame), Vec::new()));
            crate::ui::draw(&ctx, &mut state, None);
            crate::app::modals::draw_editor_overlays(&ctx, &mut state);
            let _ = ctx.end_pass();
        }
        let areas = areas_over_the_panels(&ctx);
        assert!(
            areas.len() >= 3,
            "the list and two toasts should be over the panels, found {areas:?}"
        );
        let window_rect = Rect::from_min_size(egui::Pos2::ZERO, screen);
        for (i, area) in areas.iter().enumerate() {
            assert!(
                within(window_rect, *area),
                "{area:?} leaves the {screen:?} window"
            );
            for other in &areas[i + 1..] {
                assert!(
                    !overlap(*area, *other),
                    "{area:?} and {other:?} overlap on the {screen:?} window"
                );
            }
        }
    }
}
