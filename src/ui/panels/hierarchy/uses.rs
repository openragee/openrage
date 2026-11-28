//! A model's or a map's row offers Uses, the files its file uses, and a
//! model's Placed by, the maps placing it: each item opens what it names, and
//! Reveal in tree beside it selects it in Files.

use std::path::Path;

use or_world_gta5::Kind;

use super::*;
use crate::state::dependencies::{Answer, Listed};
use crate::state::navigation::Arrival;

/// Uses and Placed by, on a row whose file the resolver answers for.
pub(super) fn submenus(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    node: &RpfSessionNode,
) {
    if node.is_dir() {
        return;
    }
    let kind = Kind::of_leaf(&node.name);
    let model = matches!(kind, Kind::Drawable | Kind::Fragment);
    if model || kind == Kind::MapData {
        ui.menu_button(tr!("Uses"), |ui| {
            let answer = state.uses_of(doc_id, node.id);
            list(
                ui,
                state,
                answer,
                tr!("Looking for what it uses…"),
                tr!("It uses no other file"),
            );
        });
    }
    if model {
        ui.menu_button(tr!("Placed by"), |ui| {
            let answer = state.placed_by(doc_id, node.id);
            let none = tr!("No map the install loads places it");
            list(
                ui,
                state,
                answer,
                tr!("Reading the maps that place it…"),
                none,
            );
        });
    }
}

fn list(ui: &mut egui::Ui, state: &mut AppState, answer: Answer, looking: &str, none: &str) {
    let quiet = |text: &str| egui::RichText::new(text).color(theme::TEXT_SECONDARY);
    match answer {
        Answer::Cannot(why) => {
            // ⚠ Its own layout: a menu's justifies wrapped text, word by word.
            ui.vertical(|ui| {
                ui.set_max_width(320.0);
                ui.add(egui::Label::new(quiet(&why)).wrap());
            });
        }
        Answer::Looking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(looking);
            });
        }
        Answer::Listed { files, .. } if files.is_empty() => {
            ui.label(quiet(none));
        }
        Answer::Listed { install, files } => {
            for file in files.iter() {
                item(ui, state, &install, file);
            }
        }
    }
}

/// One file: its name and what it is, opening it, and Reveal in tree beside it.
fn item(ui: &mut egui::Ui, state: &mut AppState, install: &Path, file: &Listed) {
    let mut atoms = egui::Atoms::new(file.name.as_str());
    atoms.push_right(egui::Atom::grow());
    atoms.push_right(egui::RichText::new(&file.note).weak());
    let Some(at) = &file.at else {
        ui.add_enabled(false, egui::Button::new(atoms));
        return;
    };
    let then = ui
        .horizontal(|ui| {
            let open = ui.button(atoms).on_hover_text(at);
            ui.separator();
            let reveal = ui
                .small_button(tr!("Reveal in tree"))
                .on_hover_text(tr!("Select it in Files, opening every archive on its way"));
            if open.clicked() {
                Some(Arrival::Open)
            } else {
                reveal.clicked().then_some(Arrival::Reveal)
            }
        })
        .inner;
    let Some(then) = then else {
        return;
    };
    ui.close_kind(egui::UiKind::Menu);
    let reveal = matches!(then, Arrival::Reveal);
    state.walk_to(install, vec![at.clone()], then);
    if reveal {
        crate::ui::panels::search::show_tree(state);
    }
}
