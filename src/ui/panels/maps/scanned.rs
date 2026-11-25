//! The Map screen's list and selection while a scanned GTA VI map is shown: the map's own fields,
//! its entities and instance lists, and the rows of the one picked, whose archetype is a choice.
//!
//! ⚠ The fields the format leaves open (a prop list's `numChildren`, the archetype each instance of
//! a grass list of several takes, where an interior stands in the world) are shown as stored, under
//! their member names.

use or_maps::Name;

use crate::state::gta6_map::links::{self, Parent, Shown};
use crate::state::gta6_map::{ScannedMap, Source};
use crate::state::workbench::{Pick, PickKind};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::theme;
use or_io::gta6::map::links::LodTarget;
use or_io::gta6::map::MapData;

/// The height of a row of the list.
const ROW: f32 = 20.0;

/// A name for people: the one the names harvested from GTA VI's archives hold for its hash, else
/// as every map shows it.
fn shown(state: &AppState, name: &Name) -> String {
    let harvested = crate::state::gta6_names::name32(name.hash);
    harvested.unwrap_or_else(|| super::shown(state, name))
}

/// A map's name hash, with the name the names harvested hold for it.
fn map_hash(hash: u32) -> String {
    match crate::state::gta6_names::name32(hash) {
        Some(name) => format!("{hash:#010X} ({name})"),
        None => format!("{hash:#010X}"),
    }
}

fn point(p: [f32; 3]) -> String {
    format!("({:.2}, {:.2}, {:.2})", p[0], p[1], p[2])
}

fn row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(label).color(theme::TEXT_SECONDARY));
        ui.label(value);
    });
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .color(theme::TEXT_SECONDARY),
    );
}

fn file(scanned: &ScannedMap) -> String {
    (scanned.path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The map's fields, the search for its archetypes, then a row for each entity and list.
pub fn draw_list(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let Some(m) = scanned.data() else {
        return;
    };
    let (markers, unmodelled) = (scanned.markers(), scanned.unmodelled());
    let dictionaries = links::unresolved_dictionaries(scanned);
    let shown_links = std::sync::Arc::clone(&scanned.links);
    ui.label(egui::RichText::new(file(scanned)).strong());
    row(ui, "name", map_hash(m.name));
    row(ui, "parent", map_hash(m.parent));
    row(
        ui,
        "flags",
        format!("{:#X}, contentFlags {:#X}", m.flags, m.content_flags),
    );
    row(
        ui,
        "entitiesExtents",
        format!(
            "{} .. {}",
            point(m.entities_extents.min),
            point(m.entities_extents.max)
        ),
    );
    row(
        ui,
        "hash_6C0F9C8C",
        format!(
            "{:#010X}, hash_9E8D32A0 {:#010X}, hash_EC16451D {:#010X}, hash_36C898AA {:#010X}",
            m.hash_6c0f9c8c, m.hash_9e8d32a0, m.hash_ec16451d, m.hash_36c898aa
        ),
    );
    let rows: Vec<(usize, String)> = {
        let filter = state.workbench.maps.filter.trim().to_lowercase();
        let entities = m.entities.iter().enumerate().map(|(k, e)| {
            let name = shown(state, &Name::of(e.archetype_name));
            (k, format!("#{k}  {name}  {}", point(e.position)))
        });
        let lists = (m.prop_lists.iter().map(|l| (tr!("prop list"), l)))
            .chain(m.grass_lists.iter().map(|l| (tr!("grass list"), l)))
            .enumerate()
            .map(|(k, (kind, l))| {
                let name = shown(state, &Name::of(l.archetype_name));
                (
                    m.entities.len() + k,
                    format!(
                        "#{k}  {kind}  {name}  · {}",
                        tr_n!("{count} instance", "{count} instances", l.instances)
                    ),
                )
            });
        (entities.chain(lists))
            .filter(|(_, text)| filter.is_empty() || text.to_lowercase().contains(&filter))
            .collect()
    };
    if markers > 0 {
        note(
            ui,
            &tr_n!(
                "{count} entity is drawn as a marker: no map types body open or found in the scan holds its archetype",
                "{count} entities are drawn as markers: no map types body open or found in the scan holds their archetypes",
                markers
            ),
        );
    }
    if unmodelled > 0 {
        note(
            ui,
            &tr_n!(
                "{count} entity's archetype names a model that no body found in the scan states",
                "{count} entities' archetypes name models that no body found in the scan states",
                unmodelled
            ),
        );
    }
    if dictionaries > 0 {
        note(
            ui,
            &tr_n!(
                "{count} entity's archetype names a drawable dictionary that no one dictionary of the scan holds the keys of",
                "{count} entities' archetypes name drawable dictionaries that no one dictionary of the scan holds the keys of",
                dictionaries
            ),
        );
    }
    if markers + unmodelled + dictionaries > 0 {
        let button = ui.button(tr!("Find the archetypes and their models in the scan"));
        if named(
            button,
            tr!("Find the archetypes and their models in the scan"),
        )
        .clicked()
        {
            if let Err(why) = state.find_gta6_archetypes() {
                state.show_error_toast(why);
            }
        }
    }
    in_place(ui, state, &shown_links);
    let field = ui.add(
        egui::TextEdit::singleline(&mut state.workbench.maps.filter)
            .hint_text(tr!("Filter entities and archetypes")),
    );
    named(field, tr!("Filter map entities and archetypes"));
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let source = scanned.path.display().to_string();
    let selected = state.workbench.maps.selected.clone();
    let mut chosen = None;
    let mut opening = false;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW, rows.len(), |ui, range| {
            for (index, text) in &rows[range] {
                let on = selected
                    .as_ref()
                    .is_some_and(|p| p.kind == PickKind::Entity(*index));
                let response = named(
                    ui.selectable_label(on, text),
                    &tr!("Entity {text}", text = text),
                );
                if response.clicked() || response.double_clicked() {
                    chosen = Some(Pick {
                        source: source.clone(),
                        kind: PickKind::Entity(*index),
                    });
                }
                opening |= response.double_clicked();
            }
        });
    if chosen.is_some() {
        state.workbench.maps.selected = chosen;
    }
    if opening {
        state.open_selected_gta6_model();
    }
}

/// What the map links to, counted, and the switch that draws its models in place.
fn in_place(ui: &mut egui::Ui, state: &mut AppState, shown: &Shown) {
    if let Some(why) = &shown.refused {
        note(
            ui,
            &tr!(
                "This map's links to its parents and its instances were not read: {why}",
                why = why
            ),
        );
    }
    let linked = shown.parents.iter().flatten().count();
    if linked > 0 {
        let found = (shown.parents.iter().flatten())
            .filter(|p| p.found.is_some())
            .count();
        note(
            ui,
            &tr!(
                "{linked} elements name a level-of-detail parent; {found} of them are in map bodies open, and a line joins each of those to its parent",
                linked = linked,
                found = found
            ),
        );
    }
    if shown.prop_instances + shown.grass_instances > 0 {
        note(
            ui,
            &tr!(
                "{props} prop and {grass} grass instances; the plan draws at most {most} of them as points",
                props = shown.prop_instances,
                grass = shown.grass_instances,
                most = links::MOST_POINTS
            ),
        );
    }
    if shown.mixed_grass > 0 {
        note(
            ui,
            &tr_n!(
                "{count} grass list names several archetypes, and which one each instance takes is not known",
                "{count} grass lists name several archetypes, and which one each instance takes is not known",
                shown.mixed_grass
            ),
        );
    }
    let place = &mut state.workbench.maps.in_place;
    ui.horizontal_wrapped(|ui| {
        let on = ui.checkbox(&mut place.on, tr!("Draw models in place"));
        named(
            on,
            tr!("Draw the models of the entity selected and of those round it in place"),
        );
        // A drag shows its value each frame and asks for the models once, where it ends.
        let mut radius = place.dragged.unwrap_or(place.radius);
        let control = egui::DragValue::new(&mut radius)
            .range(0.0..=500.0)
            .speed(1.0)
            .suffix(" m");
        let response = ui.add(control);
        if response.dragged() {
            place.dragged = Some(radius);
        } else {
            place.dragged = None;
            place.radius = radius;
        }
        named(
            response,
            tr!("Metres round the entity selected whose models are drawn too"),
        );
    });
    if place.on {
        note(
            ui,
            &tr!(
                "At most the {most} nearest models are drawn, out to the radius from the entity selected; at 0 m its own alone. A double click opens a model.",
                most = links::MOST_MODELS
            ),
        );
    }
}

/// The instances as points and the level-of-detail links as lines on the plan, through `to_screen`.
pub(super) fn draw_on_plan(
    painter: &egui::Painter,
    state: &AppState,
    to_screen: impl Fn(glam::Vec2) -> egui::Pos2,
) {
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let rect = painter.clip_rect();
    let dot = theme::TEXT_SECONDARY.get();
    for p in &scanned.links.points {
        let at = to_screen(glam::Vec2::new(p[0], p[1]));
        if rect.contains(at) {
            painter.circle_filled(at, 1.0, dot);
        }
    }
    let stroke = egui::Stroke::new(
        1.0_f32,
        theme::graphic(egui::Color32::from_rgb(230, 140, 240)),
    );
    for pair in scanned.links.lines.as_chunks::<2>().0 {
        let flat = |v: &or_renderer::LineVertex| glam::Vec2::new(v.position[0], v.position[1]);
        let (a, b) = (to_screen(flat(&pair[0])), to_screen(flat(&pair[1])));
        if rect.intersects(egui::Rect::from_two_pos(a, b)) {
            painter.line_segment([a, b], stroke);
        }
    }
}

/// What a level-of-detail parent names, for people.
fn parent_text(p: &Parent) -> String {
    let map = map_hash(p.map);
    let Some((path, target)) = &p.found else {
        return tr!(
            "map {map}, index {index}: open that map body to follow it",
            map = map,
            index = p.index
        );
    };
    let file = (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match target {
        None => tr!(
            "index {index} names nothing in {file}",
            index = p.index,
            file = file
        ),
        Some(LodTarget::Entity(entity)) => {
            tr!("entity {entity} of {file}", entity = entity, file = file)
        }
        Some(LodTarget::PropList {
            list,
            instance: Some(instance),
        }) => tr!(
            "instance {instance} of prop list {list} of {file}",
            instance = instance,
            list = list,
            file = file
        ),
        Some(LodTarget::PropList { list, .. }) => {
            tr!("prop list {list} of {file}", list = list, file = file)
        }
        Some(LodTarget::GrassList(list)) => {
            tr!("grass list {list} of {file}", list = list, file = file)
        }
    }
}

/// The parent of element `index` (the entities, then the prop lists), where the map's links read.
fn parent_row(ui: &mut egui::Ui, scanned: &ScannedMap, index: usize) {
    let Some(parent) = scanned.links.parents.get(index) else {
        return;
    };
    let text = match parent {
        None => tr!("None").to_string(),
        Some(p) => parent_text(p),
    };
    row(ui, tr!("Level-of-detail parent"), text);
}

/// What the interior entity `index` places holds, or why it is not read.
fn interior_rows(ui: &mut egui::Ui, state: &AppState, scanned: &ScannedMap, index: usize) {
    match links::interior_of(scanned, index) {
        Some(Ok(i)) => {
            row(
                ui,
                tr!("Interior"),
                tr!(
                    "{entities} entities, {rooms} rooms, {portals} portals, {sets} entity sets",
                    entities = i.entities.len(),
                    rooms = i.rooms.len(),
                    portals = i.portals.len(),
                    sets = i.entity_sets.len()
                ),
            );
            note(ui, tr!("Where an interior's rooms and entities stand in the world is not measured, so they are listed and not drawn"));
            for (k, r) in i.rooms.iter().enumerate() {
                row(
                    ui,
                    &tr!("Room {room}", room = k),
                    tr!(
                        "{name}: {entities} entities, {portals} portals, floor {floor}",
                        name = shown(state, &Name::of(r.name)),
                        entities = r.attached_objects.len(),
                        portals = r.portal_count,
                        floor = r.floor_id
                    ),
                );
            }
        }
        Some(Err(why)) => note(ui, &tr!("Its interior was not read: {why}", why = why)),
        None => note(ui, tr!("It places an interior, whose rooms and portals are read once its archetype is found")),
    }
}

/// The rows of the entity or list picked.
pub fn draw_selection(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(PickKind::Entity(index)) = state.workbench.maps.selected.as_ref().map(|p| p.kind)
    else {
        ui.add_space(theme::SPACING_DEFAULT);
        note(
            ui,
            tr!("Select an entity or a list in the list, the plan or the scene to see it"),
        );
        return;
    };
    super::scanned_author::buttons(ui, state);
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let Some(m) = scanned.data() else {
        return;
    };
    let authored = super::scanned_author::draw(ui, state);
    let mut open_model = None;
    let mut retype = None;
    let mut asked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| match m.entities.get(index) {
            Some(e) => {
                let found = scanned.archetypes.get(index).and_then(Option::as_ref);
                ui.label(egui::RichText::new(tr!("Entity {index} of {file}", index = index, file = file(scanned))).strong());
                note(ui, crate::i18n::tr(crate::state::rpf9_scan::READ_ONLY_BODY));
                if e.interior {
                    interior_rows(ui, state, scanned, index);
                }
                retype = archetype_choice(ui, state, m, index);
                asked = state.selected_pose().and_then(|pose| super::scanned_pose::fields(ui, pose));
                let [qx, qy, qz, qw] = e.rotation;
                row(ui, "rotation", format!("({qx:.4}, {qy:.4}, {qz:.4}, {qw:.4})"));
                row(ui, tr!("Level of detail"), e.lod_level_name().to_string());
                let lod = match (e.lod_dist < 0.0, found) {
                    (true, Some((_, a))) => a.lod_dist,
                    _ => e.lod_dist,
                };
                row(ui, tr!("Stream distance"), format!("{lod:.1}"));
                row(ui, "childLodDist", format!("{:.1}", e.child_lod_dist));
                row(ui, "numChildren", e.num_children.to_string());
                row(ui, "parentIndex", e.parent_index.to_string());
                parent_row(ui, scanned, index);
                note(ui, tr!("parentIndex indexes the chosen parent map's entities, then its prop lists, then its grass lists"));
                row(ui, "hash_E74C3C23", e.hash_e74c3c23.to_string());
                row(ui, "flags", format!("{:#010X}", e.flags));
                row(ui, "id", format!("{:#018X}", e.id));
                row(ui, "tintValue", e.tint_value.to_string());
                ui.add_space(theme::SPACING_DEFAULT);
                match found {
                    None => note(ui, tr!("No map types body open or found in the scan holds its archetype: it is drawn as a marker one metre wide")),
                    Some((source, a)) => {
                        note(ui, &match source {
                            Source::Open(path) => tr!("Its archetype is in {file}, which is open", file = path.display()),
                            Source::Scan(offset) => tr!("Its archetype is in the map types body at {offset} of the scan", offset = format!("{offset:#X}")),
                        });
                        row(ui, tr!("Box"), format!("{} .. {}", point(a.bb_min), point(a.bb_max)));
                        row(ui, "assetType", a.asset_type_name().to_string());
                        row(ui, "model", format!("{:#018X}", a.model_word()));
                        row(ui, "textureDictionary", format!("{:#018X}", a.texture_dictionary));
                        if let Some(offset) = scanned.models.get(index).copied().flatten() {
                            note(ui, &tr!("Its model is the body at {offset} of the scan", offset = format!("{offset:#X}")));
                            if named(ui.button(tr!("Open its model")), tr!("Open its model")).clicked() {
                                open_model = Some(offset);
                            }
                        }
                    }
                }
            }
            None => {
                let k = index - m.entities.len();
                let (kind, list) = match m.prop_lists.get(k) {
                    Some(l) => (tr!("prop list"), l),
                    None => match m.grass_lists.get(k - m.prop_lists.len()) {
                        Some(l) => (tr!("grass list"), l),
                        None => return,
                    },
                };
                ui.label(egui::RichText::new(format!("{kind} {k}")).strong());
                row(ui, tr!("Archetype"), shown(state, &Name::of(list.archetype_name)));
                row(ui, "InstanceList", tr_n!("{count} instance", "{count} instances", list.instances));
                row(ui, "BatchAABB", format!("{} .. {}", point(list.batch_aabb.min), point(list.batch_aabb.max)));
                if k < m.prop_lists.len() {
                    parent_row(ui, scanned, index);
                }
                note(ui, tr!("The plan draws its instances as points"));
            }
        });
    if let Some(offset) = open_model {
        state.open_gta6_model(offset);
    }
    if let Some((word, name)) = retype {
        if let Err(why) = state.retype_scanned_entity(index, word, name) {
            state.show_error_toast(why);
        }
    }
    match asked {
        Some(super::scanned_pose::Asked::Pose(pose)) => {
            let label = tr!("Set entity {index}", index = index).to_string();
            state.place_selected_entity(pose, label);
        }
        Some(super::scanned_pose::Asked::Ground) => state.drop_selected_to_ground(),
        None => {}
    }
    if let Some((text, lod_dist, guid, flags)) = authored {
        if let Err(why) = state.author_scanned_archetype(&text, lod_dist, guid, flags) {
            state.show_error_toast(why);
        }
    }
}

/// The entity's archetype, a choice of those [`AppState::retype_choices`] gives; the one picked
/// where it is another.
fn archetype_choice(
    ui: &mut egui::Ui,
    state: &AppState,
    m: &MapData,
    index: usize,
) -> Option<(u64, u32)> {
    let e = m.entities.get(index)?;
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(tr!("Archetype")).color(theme::TEXT_SECONDARY));
        let combo = egui::ComboBox::from_id_salt(("gta6_retype", index))
            .selected_text(shown(state, &Name::of(e.archetype_name)))
            .show_ui(ui, |ui| {
                // ponytail: listed each frame the choice is open, a walk of the map's entities and
                // the known archetypes with nothing copied but their words; only the rows in view
                // are drawn.
                let choices = state.retype_choices(index);
                let row = ui.spacing().interact_size.y;
                let height = ui.spacing().combo_height;
                let list = egui::ScrollArea::vertical().max_height(height);
                list.show_rows(ui, row, choices.len(), |ui, rows| {
                    for &(word, name) in &choices[rows] {
                        let on = (word, name) == (e.archetype_word, e.archetype_name);
                        if ui
                            .selectable_label(on, shown(state, &Name::of(name)))
                            .clicked()
                            && !on
                        {
                            picked = Some((word, name));
                        }
                    }
                });
            });
        named(
            combo.response,
            tr!("Archetype of the entity selected, among those its map names and the map types bodies open or found hold"),
        );
    });
    picked
}
