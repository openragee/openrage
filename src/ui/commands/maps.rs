//! The commands of the Map screen: giving a project maps, adding map files, adding and taking out entities and archetypes, standing an entity
//! on the ground, and taking the last change back.

use super::table::{a_project, none_if, Action, Spec, BASE};
use crate::platform::file_dialogs;
use crate::state::workbench::PickKind;
use crate::state::AppState;

const NO_MAPS: &str = tr_key!("The project holds no maps: Project ▸ Add a map pack… adds some");

fn has_maps(state: &AppState) -> Result<(), &'static str> {
    a_project(state)?;
    none_if(!state.has_maps(), crate::i18n::tr(NO_MAPS))
}

const LINKED: &str = tr_key!(
    "These maps are linked from another project, which alone edits them: open that project to change them"
);

fn writable_maps(state: &AppState) -> Result<(), &'static str> {
    none_if(
        state.scanned_map().is_some(),
        crate::i18n::tr(crate::state::rpf9_scan::READ_ONLY_BODY),
    )?;
    has_maps(state)?;
    none_if(
        state.maps_pack().is_some_and(|pack| pack.linked),
        crate::i18n::tr(LINKED),
    )
}

fn an_entity_selected(state: &AppState) -> Result<(), &'static str> {
    writable_maps(state)?;
    none_if(
        state.selected_placed().is_none(),
        tr!("Select an entity on the Map screen first"),
    )
}

/// An entity selected to stand on the ground: of a scanned GTA VI map, whose tree takes the edit,
/// or of the project's maps.
fn an_entity_to_stand(state: &AppState) -> Result<(), &'static str> {
    if state.scanned_map().is_none() {
        return an_entity_selected(state);
    }
    none_if(
        !state.places_selected(),
        tr!("Select an entity on the Map screen first"),
    )
}

fn an_archetype_selected(state: &AppState) -> Result<(), &'static str> {
    writable_maps(state)?;
    none_if(
        !matches!(
            state.workbench.maps.selected.as_ref().map(|p| p.kind),
            Some(PickKind::Archetype(_))
        ),
        tr!("Select an archetype on the Map screen first"),
    )
}

/// A map to add an entity to: a scanned GTA VI map, which a job edits with the maps linked to it,
/// or the project's maps.
fn an_entity_to_add(state: &AppState) -> Result<(), &'static str> {
    match state.scanned_map().is_some() {
        true => Ok(()),
        false => writable_maps(state),
    }
}

/// A map types body to add an archetype to: the scanned GTA VI one an archetype edit goes to, or
/// the project's.
fn an_archetype_to_add(state: &AppState) -> Result<(), &'static str> {
    match state.scanned_archetype_target() {
        Some(_) => Ok(()),
        None => writable_maps(state),
    }
}

/// An archetype to take out: the scanned GTA VI one the entity selected places, or the project's
/// archetype selected.
fn an_archetype_to_remove(state: &AppState) -> Result<(), &'static str> {
    match state.scanned_archetype_target() {
        Some((_, Some(_))) => Ok(()),
        _ => an_archetype_selected(state),
    }
}

fn a_change_to_take_back(state: &AppState) -> Result<(), &'static str> {
    has_maps(state)?;
    none_if(
        state.workbench.maps.undo.is_empty(),
        tr!("No change to a map has been made to take back"),
    )
}

fn a_change_to_make_again(state: &AppState) -> Result<(), &'static str> {
    has_maps(state)?;
    none_if(
        state.workbench.maps.redo.is_empty(),
        tr!("No change to a map has been taken back to make again"),
    )
}

pub(super) const ADD_MAP_PACK: Spec = Spec {
    action: Action::AddMapPack,
    id: "add_map_pack",
    label: tr_key!("Add a map pack…"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Give the project maps: an empty pack in a folder of its own, shipped by every build, and the Map screen to fill it. Add map files… puts files into a pack that is there"),
    enabled: a_project,
    run: |state, _| state.add_map_content(),
    ..BASE
};

pub(super) const ADD_MAPS: Spec = Spec {
    action: Action::AddMaps,
    id: "add_maps",
    label: tr_key!("Add map files…"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Imports into the maps the .ymap, .ytyp, models and textures you pick: each game file is made a source of the pack and joins the map set shown. Dropping them on the Map screen does the same"),
    enabled: writable_maps,
    run: |state, _| {
        if let Some(files) = file_dialogs::open_any_files() {
            state.add_maps(files);
        }
    },
    ..BASE
};

pub(super) const ADD_ENTITY: Spec = Spec {
    action: Action::AddEntity,
    id: "add_entity",
    label: tr_key!("Add an entity"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Place another one of the entity selected (else the first the set holds) where the view is looking, as a copy with a guid of its own; move it with the manipulator or type where it goes. Undo takes it back"),
    enabled: an_entity_to_add,
    run: |state, _| match state.scanned_map().is_some() {
        true => state.add_scanned_entity(),
        false => state.add_entity_here(),
    },
    ..BASE
};

pub(super) const REMOVE_ENTITY: Spec = Spec {
    action: Action::RemoveEntity,
    id: "remove_entity",
    label: tr_key!("Remove entity"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Take the entity selected out of the map files of the pack with its chain: a far model left standing for nothing goes with it, and so does what names it as its far model. Undo puts them back"),
    enabled: an_entity_to_stand,
    run: |state, _| match state.scanned_map().is_some() {
        true => state.remove_scanned_entity(),
        false => state.remove_selected_entity(),
    },
    ..BASE
};

pub(super) const DROP_TO_GROUND: Spec = Spec {
    action: Action::DropToGround,
    id: "drop_to_ground",
    label: tr_key!("Stand on the ground"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Lower or raise the entity selected until its box stands on the highest box under it, or on the ground height set in the snapping when there is none. Undo takes it back"),
    enabled: an_entity_to_stand,
    run: |state, _| state.drop_selected_to_ground(),
    ..BASE
};

pub(super) const ADD_ARCHETYPE: Spec = Spec {
    action: Action::AddArchetype,
    id: "add_archetype",
    label: tr_key!("Add an archetype"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Declare another archetype in the type file shown, as a copy of the one selected (else the first plain one) under a name nobody holds yet; give it its real name and box in the selection. Undo takes it back"),
    enabled: an_archetype_to_add,
    run: |state, _| match state.scanned_archetype_target().is_some() {
        true => state.add_scanned_archetype(),
        false => state.add_archetype_here(),
    },
    ..BASE
};

pub(super) const REMOVE_ARCHETYPE: Spec = Spec {
    action: Action::RemoveArchetype,
    id: "remove_archetype",
    label: tr_key!("Remove archetype"),
    category: tr_key!("Project ▸ More ▸ Maps"),
    hover: tr_key!("Take the archetype selected out of its type file. Entities that place it stay, and the rules say they place a name no type file declares. Undo puts it back"),
    enabled: an_archetype_to_remove,
    run: |state, _| match state.scanned_archetype_target() {
        Some((_, Some(_))) => state.remove_scanned_archetype(),
        _ => state.remove_selected_archetype(),
    },
    ..BASE
};

pub(super) const UNDO_MAPS: Spec = Spec {
    action: Action::UndoMaps,
    id: "undo_maps",
    label: tr_key!("Undo map change"),
    category: tr_key!("Project ▸ History"),
    hover: tr_key!("Undo the last change to a map: an entity moved, turned, scaled, added or taken out, or an archetype added, changed or taken out. Adding map files is one step of the project's History, which Undo import takes back"),
    enabled: a_change_to_take_back,
    label_now: Some(|state| match state.workbench.maps.undo.last() {
        Some(last) => tr!("Undo \"{last}\"", last = last.label.as_str()),
        None => tr!("Undo map change").to_string(),
    }),
    run: |state, _| state.undo_maps_change(),
    ..BASE
};

pub(super) const REDO_MAPS: Spec = Spec {
    action: Action::RedoMaps,
    id: "redo_maps",
    label: tr_key!("Redo map change"),
    category: tr_key!("Project ▸ History"),
    hover: tr_key!("Make again the change to a map that Undo took back"),
    enabled: a_change_to_make_again,
    label_now: Some(|state| match state.workbench.maps.redo.last() {
        Some(last) => tr!("Redo \"{last}\"", last = last.label.as_str()),
        None => tr!("Redo map change").to_string(),
    }),
    run: |state, _| state.redo_maps_change(),
    ..BASE
};
