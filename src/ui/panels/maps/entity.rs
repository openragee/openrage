//! The selection: the entity or the archetype picked, with every number of it a field that takes what is typed, so that nothing the
//! manipulator does needs the mouse.
//!
//! A field is left when its value is let go (Tab, Enter or a click elsewhere), and a value that parses is written to the map file as one step
//! of Undo; one that does not is put back as it was. The names of the fields are the engine's.

use or_maps::{Name, Role};
use or_world::transform::Pose;

use super::{action_button, shown};
use crate::state::front_door::counted;
use crate::state::workbench::{Pick, PickKind};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::commands::Action;
use crate::ui::theme;

/// The width of a field that takes a number.
const NUMBER: f32 = 74.0;

/// `value` as a person writes it: at most three decimals, none that are noughts.
fn written(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "" | "-" | "-0" => "0".to_string(),
        other => other.to_string(),
    }
}

/// A number field showing `value`; the number typed once the field is left with text that reads as a number other than `value`.
pub(super) fn number(ui: &mut egui::Ui, id: &str, name: &str, value: f32) -> Option<f32> {
    let key = ui.make_persistent_id(("map_field", id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| written(value));
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(NUMBER));
    let response = named(response, name);
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(key));
        let typed: Option<f32> = text.trim().replace(',', ".").parse().ok();
        return typed.filter(|v| v.is_finite() && written(*v) != written(value));
    }
    None
}

/// A text field showing `value`; the text typed once the field is left with text that is not empty and not `value`.
fn text_field(ui: &mut egui::Ui, id: &str, name: &str, value: &str, width: f32) -> Option<String> {
    let key = ui.make_persistent_id(("map_text", id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| value.to_string());
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(width));
    let response = named(response, name);
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(key, text.clone()));
    }
    if response.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(key));
        let typed = text.trim().to_string();
        return (!typed.is_empty() && typed != value).then_some(typed);
    }
    None
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    if state.scanned_map().is_some() {
        return super::scanned::draw_selection(ui, state);
    }
    if !state.has_maps() {
        crate::ui::panels::empty_state::draw(ui, state);
        return;
    }
    let Some(pick) = state.workbench.maps.selected.clone() else {
        ui.add_space(theme::SPACING_DEFAULT);
        ui.label(
            egui::RichText::new(tr!(
                "Select an entity or an archetype in the list, the plan or the scene to see it and change it"
            ))
            .color(theme::TEXT_SECONDARY),
        );
        return;
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match pick.kind {
            PickKind::Entity(index) => entity(ui, state, &pick, index),
            PickKind::Archetype(index) => archetype(ui, state, &pick, index),
        });
}

fn entity(ui: &mut egui::Ui, state: &mut AppState, pick: &Pick, index: usize) {
    let Some(placed) = state.selected_placed() else {
        ui.label(
            egui::RichText::new(tr!("That entity is not in the set any more"))
                .color(theme::TEXT_SECONDARY),
        );
        return;
    };
    let (guid, lod, interior, declared, level, model) = (
        placed.guid,
        placed.lod_dist,
        placed.interior,
        placed.declared,
        placed.lod_level.clone(),
        placed.model.clone(),
    );
    let archetype = shown(state, &placed.archetype);
    let Some(pose) = state.selected_pose() else {
        return;
    };
    ui.label(
        egui::RichText::new(tr!(
            "Entity {index} of {file}",
            index = index,
            file = pick.source.as_str()
        ))
        .strong(),
    );
    if interior {
        ui.label(egui::RichText::new(tr!("It places an interior")).color(theme::TEXT_SECONDARY));
    }
    let editable = state.maps_pack().is_some_and(|p| !p.linked);
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Archetype"));
            if let Some(typed) =
                text_field(ui, "archetype", tr!("Entity archetype"), &archetype, 180.0)
            {
                state.rename_selected_entity_archetype(&typed);
            }
        });
        if !declared {
            ui.label(
                egui::RichText::new(tr!(
                    "No type file of this set declares it: its box is drawn one metre wide"
                ))
                .small()
                .color(theme::TEXT_SECONDARY),
            );
        }
        let mut next = pose;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Position"));
            for (axis, name, value) in [
                ("x", tr!("Entity position X"), pose.position.x),
                ("y", tr!("Entity position Y"), pose.position.y),
                ("z", tr!("Entity position Z"), pose.position.z),
            ] {
                if let Some(v) = number(ui, &format!("pos{axis}"), name, value) {
                    match axis {
                        "x" => next.position.x = v,
                        "y" => next.position.y = v,
                        _ => next.position.z = v,
                    }
                    changed = true;
                }
            }
        });
        let degrees = pose.degrees();
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Rotation"));
            let mut turned = degrees;
            let mut moved = false;
            for (at, name) in [
                (0, tr!("Entity rotation about X in degrees")),
                (1, tr!("Entity rotation about Y in degrees")),
                (2, tr!("Entity rotation about Z in degrees")),
            ] {
                if let Some(v) = number(ui, &format!("rot{at}"), name, degrees[at]) {
                    turned[at] = v;
                    moved = true;
                }
            }
            if moved {
                next.rotation = Pose::rotation_of_degrees(turned);
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Scale"));
            if let Some(v) = number(
                ui,
                "scale_xy",
                tr!("Entity scale of X and Y"),
                pose.scale_xy,
            ) {
                next.scale_xy = v.max(0.01);
                changed = true;
            }
            if let Some(v) = number(ui, "scale_z", tr!("Entity scale of Z"), pose.scale_z) {
                next.scale_z = v.max(0.01);
                changed = true;
            }
        });
        if changed {
            state.place_selected_entity(next, tr!("Set entity {index}", index = index).to_string());
        }
        ui.horizontal_wrapped(|ui| {
            action_button(ui, state, Action::DropToGround, false);
            action_button(ui, state, Action::AddEntity, false);
            action_button(ui, state, Action::RemoveEntity, false);
        });
    });
    ui.add_space(theme::SPACING_DEFAULT);
    for (label, value) in [
        (tr!("Guid"), guid.to_string()),
        (tr!("Stream distance"), written(lod)),
        (
            tr!("Level of detail"),
            level.unwrap_or_else(|| tr!("not stated").to_string()),
        ),
        (
            tr!("Model"),
            model.unwrap_or_else(|| tr!("none in this set").to_string()),
        ),
    ] {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(label).color(theme::TEXT_SECONDARY));
            ui.label(value);
        });
    }
    ui.label(
        egui::RichText::new(tr!(
            "A scale is two numbers: one for X and Y together and one for Z. A turn is read as degrees about X, then Y, then Z"
        ))
        .small()
        .color(theme::TEXT_SECONDARY),
    );
}

fn archetype(ui: &mut egui::Ui, state: &mut AppState, pick: &Pick, index: usize) {
    let found = state
        .maps_pack()
        .and_then(|p| p.read.as_ref().ok())
        .and_then(|read| read.facts.source(&pick.source))
        .filter(|s| s.role() == Some(Role::Types))
        .and_then(|s| s.types())
        .and_then(|t| t.archetypes.get(index))
        .cloned();
    let Some(a) = found else {
        ui.label(
            egui::RichText::new(tr!("That archetype is not in the set any more"))
                .color(theme::TEXT_SECONDARY),
        );
        return;
    };
    let name = shown(state, &a.name);
    let asset = shown(state, &a.asset_name);
    ui.label(
        egui::RichText::new(tr!(
            "Archetype {index} of {file}",
            index = index,
            file = pick.source.as_str()
        ))
        .strong(),
    );
    let editable = state.maps_pack().is_some_and(|p| !p.linked);
    ui.add_enabled_ui(editable, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Name"));
            if let Some(typed) = text_field(ui, "arch_name", tr!("Archetype name"), &name, 180.0) {
                let label = tr!("Renamed the archetype {name}", name = typed.as_str()).to_string();
                let hashed = or_io::meta::instance::HashedString::new(typed.clone());
                state.edit_selected_archetype(label, &[&typed], move |archetype| {
                    archetype.name = hashed;
                });
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("Asset"));
            if let Some(typed) =
                text_field(ui, "arch_asset", tr!("Archetype asset name"), &asset, 180.0)
            {
                let label = tr!(
                    "Named the asset of an archetype {name}",
                    name = typed.as_str()
                )
                .to_string();
                let hashed = or_io::meta::instance::HashedString::new(typed.clone());
                state.edit_selected_archetype(label, &[&typed], move |archetype| {
                    archetype.asset_name = hashed;
                });
            }
        });
        let mut min = a.bb_min;
        let mut max = a.bb_max;
        let mut boxed = false;
        for (label, held, which) in [
            (tr!("Box minimum"), &mut min, 0),
            (tr!("Box maximum"), &mut max, 1),
        ] {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                for (axis, at) in [("X", 0), ("Y", 1), ("Z", 2)] {
                    let field = format!("arch_box{which}{axis}");
                    let described = format!("{label} {axis}");
                    if let Some(v) = number(ui, &field, &described, held[at]) {
                        held[at] = v;
                        boxed = true;
                    }
                }
            });
        }
        if boxed {
            state.edit_selected_archetype(
                tr!("Changed the box of an archetype").to_string(),
                &[],
                move |archetype| {
                    archetype.bb_min = or_io::decl::Vec3 {
                        x: min[0],
                        y: min[1],
                        z: min[2],
                    };
                    archetype.bb_max = or_io::decl::Vec3 {
                        x: max[0],
                        y: max[1],
                        z: max[2],
                    };
                },
            );
        }
        ui.horizontal_wrapped(|ui| {
            action_button(ui, state, Action::AddArchetype, false);
            action_button(ui, state, Action::RemoveArchetype, false);
        });
    });
    ui.add_space(theme::SPACING_DEFAULT);
    for (label, value) in [
        (tr!("Stream distance"), written(a.lod_dist)),
        (tr!("Flags"), format!("{:#x}", a.flags)),
        (
            tr!("Drawable dictionary"),
            dictionary(state, &a.drawable_dictionary),
        ),
        (
            tr!("Texture dictionary"),
            dictionary(state, &a.texture_dictionary),
        ),
    ] {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(label).color(theme::TEXT_SECONDARY));
            ui.label(value);
        });
    }
    if let Some(interior) = &a.interior {
        interior_section(ui, interior);
    }
}

fn dictionary(state: &AppState, name: &Name) -> String {
    match name.is_empty() {
        true => tr!("none").to_string(),
        false => shown(state, name),
    }
}

/// The rooms and portals of an interior, as `or-world` reads them.
fn interior_section(ui: &mut egui::Ui, interior: &or_maps::facts::InteriorFact) {
    egui::CollapsingHeader::new(
        egui::RichText::new(format!(
            "{}  ·  {}, {}, {}",
            tr!("Interior"),
            counted(interior.rooms.len(), "room"),
            counted(interior.portals.len(), "portal"),
            counted(interior.entities, "entity")
        ))
        .strong(),
    )
    .id_salt("map_interior")
    .default_open(true)
    .show(ui, |ui| {
        for (at, room) in interior.rooms.iter().enumerate() {
            ui.label(format!(
                "{at}  {}  ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}), {} {}",
                room.name,
                room.bb_min[0],
                room.bb_min[1],
                room.bb_min[2],
                room.bb_max[0],
                room.bb_max[1],
                room.bb_max[2],
                tr!("flags"),
                room.flags
            ));
        }
        for (at, portal) in interior.portals.iter().enumerate() {
            ui.label(tr!(
                "Portal {at}: room {from} to room {to}, {corners} corners, flags {flags}",
                at = at,
                from = portal.room_from,
                to = portal.room_to,
                corners = portal.corners,
                flags = portal.flags
            ));
        }
        for problem in &interior.problems {
            ui.colored_label(theme::WARNING.get(), problem);
        }
        ui.label(
            egui::RichText::new(tr!(
                "Rooms and portals are read from the file. Change them in the Data screen"
            ))
            .small()
            .color(theme::TEXT_SECONDARY),
        );
    });
}
