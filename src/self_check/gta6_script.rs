//! The units that draw a GTA VI script: its outline, its source and its facts, each judged
//! against the same panel over a script that has none of them.

use std::sync::Arc;

use or_io::gta6::script::program::{ImportRow, Program};
use or_io::gta6::script::Level;

use super::{draw_unit, Report};
use crate::state::{AppState, DocPayload, DocumentKind, Gta6ScriptDocument};
use crate::ui::dock::{Tab, Workspace};
use crate::ui::panels::registry::ids;

/// Written here; nothing of a game.
const SOURCE: &str = "// program 0x0A0B0C0D\n\nclass Demo\n{\n    void Run(int count)\n    {\n        DEMO.TICK(count);\n    }\n}\n\nvoid main()\n{\n    func_000040(15.0f);\n}\n";

/// A script with a source and an import, or one with neither.
fn script(full: bool) -> Gta6ScriptDocument {
    if !full {
        return Gta6ScriptDocument::of(Program::default(), "", Level::Source);
    }
    let program = Program {
        program: 0x0A0B_0C0D,
        imports: vec![ImportRow {
            program: 0x0E0F_1011,
            types: 0..0,
        }],
        ..Program::default()
    };
    Gta6ScriptDocument::of(program, SOURCE, Level::Source)
}

/// The three panels a script is read in, each over a script and over an empty one: a panel's chrome alone would pass the blank check.
pub(super) fn units(
    state: &mut AppState,
    mut bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    report: &mut Report,
) {
    let doc = state.new_empty_document(DocumentKind::Text);
    let mut drawn = |state: &mut AppState, full: bool, panel: &'static str| {
        if let Some(d) = state.document_mut(doc) {
            d.payload = DocPayload::Gta6Script(Arc::new(script(full)));
            d.kind = DocumentKind::Gta6Script;
        }
        draw_unit(state, bridge.as_deref_mut(), frames, |state| {
            state.set_active_document(doc);
            state.active_workspace = Workspace::Data;
            *state.workspace_docks.get_mut(Workspace::Data) =
                egui_dock::DockState::new(vec![Tab(panel)]);
        })
    };
    let panels = [
        (
            "outline: a GTA VI script's classes and functions",
            ids::OUTLINE,
        ),
        ("text editor: a GTA VI script's source", ids::TEXT_EDITOR),
        ("inspector: a GTA VI script's facts", ids::DETAILS),
    ];
    for (what, panel) in panels {
        let outcome = drawn(state, false, panel).and_then(|without| {
            drawn(state, true, panel).and_then(|painted| {
                if painted.vertices > without.vertices {
                    Ok(painted)
                } else {
                    Err(format!(
                        "painted {} vertices against {} over an empty script: the script drew nothing",
                        painted.vertices, without.vertices
                    ))
                }
            })
        });
        report.record(what, outcome);
    }
    super::discard(state, doc);
}
