//! The Document panel: the active document as its kind draws itself, the start
//! page while nothing is open, or the workspace's empty state while it has
//! nothing of its own; the centre of the Browse and Data workspaces.
//!
//! ⚠ A kind with no editor of its own is drawn from its inspector body, so
//! while this panel is drawn the inspector shows that document's facts
//! instead: one view of a file, not two.

use or_io::container::resource::ResourceType;

use crate::render::RenderBridge;
use crate::state::{AppState, DocumentId, DocumentKind};
use crate::ui::panels;
use crate::ui::panels::registry::ids;

enum View {
    /// An install, a folder or an archive, or nothing open: the Files view of the folder selected in Files.
    Files,
    Text,
    Metadata,
    StringTable,
    Comparison,
    /// The kind's inspector body.
    Body,
}

fn view_of(kind: Option<DocumentKind>) -> View {
    match kind {
        None | Some(DocumentKind::Rpf | DocumentKind::GameRoot | DocumentKind::Folder) => {
            View::Files
        }
        Some(
            DocumentKind::Text
            | DocumentKind::NameTable
            | DocumentKind::Gta6Script
            | DocumentKind::Registered(ResourceType::Ysc),
        ) => View::Text,
        Some(DocumentKind::Meta) => View::Metadata,
        Some(DocumentKind::Gxt2 | DocumentKind::Gta6Text) => View::StringTable,
        Some(DocumentKind::Compare) => View::Comparison,
        Some(_) => View::Body,
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    if state.documents.is_empty() {
        panels::game_picker::draw_start_page(ui, state);
        return;
    }
    if panels::empty_state::has_nothing_to_show(state) {
        panels::empty_state::draw(ui, state);
        return;
    }
    match view_of(state.active_document().map(|d| d.kind)) {
        View::Files => panels::hierarchy::draw_files_view(ui, state, bridge),
        View::Text => panels::text_editor::draw(ui, state),
        View::Metadata => panels::meta_editor::draw(ui, state),
        View::StringTable => panels::gxt2_table::draw(ui, state),
        View::Comparison => panels::compare::draw(ui, state),
        View::Body => panels::inspector::draw_body(ui, state),
    }
}

/// Whether this panel draws `doc`'s inspector body this frame.
pub fn draws_the_body_of(state: &AppState, doc: DocumentId) -> bool {
    state.active_document == Some(doc)
        && matches!(view_of(state.document(doc).map(|d| d.kind)), View::Body)
        && state
            .workspace_docks
            .shows(state.active_workspace, ids::DOCUMENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DocPayload, HexDocument};
    use crate::ui::dock::{Tab, Workspace};

    const BANNER: &str = "no reader for probe.bin";

    /// How often one frame of the workspace on screen paints the banner.
    fn banners(state: &mut AppState) -> usize {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            ..Default::default()
        };
        let output =
            egui::Context::default().run_ui(input, |ui| crate::ui::dock::draw(ui, state, None));
        format!("{:?}", output.shapes).matches(BANNER).count()
    }

    #[test]
    fn a_kind_with_no_editor_of_its_own_is_drawn_once_in_the_centre() {
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Text);
        let d = state.document_mut(doc).expect("the blank document");
        d.payload = DocPayload::Hex(HexDocument {
            bytes: b"RBF0".repeat(16),
            banner: BANNER.to_string(),
        });
        d.kind = DocumentKind::Hex;
        state.set_active_document(doc);
        assert_eq!(state.active_workspace, Workspace::Browse);
        let arrange = |state: &mut AppState, tab: Tab| {
            *state.workspace_docks.get_mut(Workspace::Browse) =
                egui_dock::DockState::new(vec![tab]);
        };

        arrange(&mut state, Tab::DOCUMENT);
        let alone = banners(&mut state);
        assert!(alone > 0, "the Document panel did not draw the bytes");

        state.workspace_docks.reset(Workspace::Browse);
        assert_eq!(
            banners(&mut state),
            alone,
            "Browse drew the bytes in the centre and again in the inspector"
        );

        arrange(&mut state, Tab::DETAILS);
        assert_eq!(
            banners(&mut state),
            alone,
            "with no Document panel to draw them, the inspector must"
        );
    }
}
