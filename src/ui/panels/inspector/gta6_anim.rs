use or_io::gta6::anim::{
    dof_signature, known_name, AnimAt, AnimResource, Animation, Clip, ClipPlay, Database, Motion,
    Record, RootKind, Typed, DELTA_SECTION,
};

use crate::state::gta6_clip_finding::{found_clip, ClipAt};

use super::*;

mod cuts;
mod edit;
mod face;
mod matcher;
mod motion_detail;
mod play;
mod timeline;
mod values;

pub(crate) use play::{choice_count, choice_label, draw_timeline, drawn, model_drawn, posed};

/// Rows a DOF table, the clip list or the tag list shows before it scrolls.
const ROWS_IN_VIEW: f32 = 16.0;

/// What GTA VI's animation reader states of a scanned body: its root, its sets and their
/// animations with each DOF's track, id and type, its clips with what each plays and their tags,
/// or its motions.
pub(super) fn draw_gta6_anim(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Anim(resource)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let resource = std::sync::Arc::clone(resource);
    let table_entry = state
        .document(doc_id)
        .is_some_and(crate::state::Document::is_from_rpf9_table);
    ui.heading(tr!("GTA VI Animations"));
    ui.separator();
    ui.label(tr!("What GTA VI's animation reader reads from this body. A bone track's DOF names its bone by the 32-bit hash of the bone's name, which a scanned drawable's or fragment's skeleton states, so an animation plays on the model you pick."));
    ui.add_space(crate::ui::theme::SPACING_TIGHT);
    grid(ui, "gta6_anim_root", |ui| {
        let root = match resource.root {
            RootKind::ClipDictionary => tr!("clip dictionary"),
            RootKind::AnimationSet => tr!("animation set"),
            RootKind::MotionDictionary => tr!("motion dictionary"),
        };
        row(ui, tr!("Root"), root);
        row(ui, tr!("Name"), &resource.name);
        row(ui, tr!("Key"), &format!("{:#010X}", resource.key));
        row(ui, tr!("Word at 0x6C"), &resource.root_word.to_string());
        row(
            ui,
            tr!("Word at 0x60"),
            &format!("{:#018X}", resource.word_at_60),
        );
        if let Some(word) = resource.word_at_98 {
            row(ui, tr!("Word at 0x98"), &format!("{word:#018X}"));
        }
        if let Some([virtual_blocks, physical]) = resource.block_map {
            let blocks = tr!(
                "{virtual_blocks} virtual and {physical} physical blocks",
                virtual_blocks = virtual_blocks,
                physical = physical
            );
            row(ui, tr!("Block map"), &blocks);
        }
        if let Some(region) = &resource.end_region {
            let names = if region.names.is_empty() {
                tr!("none").to_string()
            } else {
                region.names.join(", ")
            };
            row(ui, tr!("End region names"), &names);
            let word = format!("{:#010X}", region.word);
            row(ui, tr!("End region word"), &word);
        }
        row(ui, tr!("Animation sets"), &resource.sets.len().to_string());
        if resource.root == RootKind::ClipDictionary {
            row(
                ui,
                tr!("Sets another body stores"),
                &resource.external_sets.len().to_string(),
            );
            row(ui, tr!("Clips"), &resource.clips.len().to_string());
        }
        if resource.root == RootKind::MotionDictionary {
            row(ui, tr!("Motions"), &resource.motions.len().to_string());
            row(
                ui,
                tr!("Clip dictionaries drawn on"),
                &resource.dictionaries.len().to_string(),
            );
        }
    });
    cuts::draw_camera_cuts(ui, doc_id, &resource);
    play::draw_play(ui, state, doc_id, &resource);
    edit::draw_edit(ui, state, doc_id, &resource);
    for (s, set) in resource.sets.iter().enumerate() {
        egui::CollapsingHeader::new(tr!(
            "Set {name} ({count})",
            name = &set.name,
            count = set.animations.len()
        ))
        .id_salt(("gta6_anim_set", doc_id, s))
        .show(ui, |ui| {
            for (a, animation) in set.animations.iter().enumerate() {
                draw_animation(ui, (doc_id, s, a), animation);
            }
        });
    }
    if !resource.clips.is_empty() {
        egui::CollapsingHeader::new(tr!("Clips ({count})", count = resource.clips.len()))
            .id_salt(("gta6_anim_clips", doc_id))
            .show(ui, |ui| draw_clips(ui, &resource));
        let count: usize = resource.clips.iter().map(|c| c.tags.len()).sum();
        egui::CollapsingHeader::new(tr!("Tags ({count})", count = count))
            .id_salt(("gta6_anim_tags", doc_id))
            .show(ui, |ui| draw_tags(ui, doc_id, &resource));
        egui::CollapsingHeader::new(tr!("Warp points"))
            .id_salt(("gta6_anim_warps", doc_id))
            .show(ui, |ui| draw_warp_points(ui, doc_id, &resource));
    }
    if !table_entry
        && !resource.motions.is_empty()
        && ui
            .button(tr!("Find the clips its motions name in the open scans"))
            .clicked()
    {
        if let Err(why) = state.find_gta6_clips(doc_id) {
            state.show_error_toast(why);
        }
    }
    // A motion names its clips by 64-bit clip key, which a clip of an open dictionary may hold.
    let clip_named = |key: u64| {
        state.documents.iter().find_map(|d| match &d.payload {
            DocPayload::Gta6Anim(r) => {
                (r.clips.iter().find(|c| c.key == key)).map(|c| format!("{} ({})", c.name, r.name))
            }
            _ => None,
        })
    };
    let mut open = None;
    for (m, motion) in resource.motions.iter().enumerate() {
        let shown = egui::CollapsingHeader::new(&motion.name)
            .id_salt(("gta6_anim_motion", doc_id, m))
            .show(ui, |ui| {
                draw_motion(ui, (doc_id, m), motion, &clip_named, !table_entry)
            });
        open = open.or(shown.body_returned.flatten());
    }
    if let Some(at) = open {
        state.open_found_clip(&at);
    }
    if !resource.motions.is_empty() {
        egui::CollapsingHeader::new(tr!("Everything the motion reader reads"))
            .id_salt(("gta6_anim_motion_read", doc_id))
            .show(ui, |ui| motion_detail::draw(ui, doc_id, &resource));
    }
    matcher::draw_matchers(ui, doc_id, &resource);
    ui.add_space(crate::ui::theme::SPACING_TIGHT);
    ui.label(
        egui::RichText::new(tr!("Not read: what the halves of a motion database's rows mean, what a data attribute's bytes hold, and what attributes of types 16, 18, 21 and 22 are for; any engine name. A clip stored beside its audio runs longer than the audio, so its duration is not the audio's length."))
        .color(theme::TEXT_SECONDARY),
    );
}

fn draw_animation(ui: &mut egui::Ui, id: (DocumentId, usize, usize), animation: &Animation) {
    egui::CollapsingHeader::new(&animation.name)
        .id_salt(("gta6_anim_animation", id))
        .show(ui, |ui| {
            grid(ui, &format!("gta6_anim_facts_{id:?}"), |ui| {
                row(ui, tr!("Key"), &format!("{:#010X}", animation.key));
                row(ui, tr!("Frames"), &animation.frames.to_string());
                row(ui, tr!("Duration"), &format!("{} s", animation.duration));
                let rate = animation
                    .rate()
                    .map_or_else(|| tr!("not stated").to_string(), |r| format!("{r} fps"));
                row(ui, tr!("Frame rate"), &rate);
                row(ui, tr!("Flags"), &format!("{:#06X}", animation.flags));
                row(
                    ui,
                    tr!("Project flags"),
                    &format!("{:#06X}", animation.project_flags),
                );
                row(ui, tr!("References"), &animation.references.to_string());
                let stated = format!("{:#010X}", animation.signature);
                let signature = if animation.signature_holds() {
                    tr!("{stated}, the sum of its DOFs", stated = stated)
                } else {
                    let sum = format!("{:#010X}", dof_signature(&animation.dofs));
                    tr!(
                        "{stated}, where the sum of its DOFs is {sum}",
                        stated = stated,
                        sum = sum
                    )
                };
                row(ui, tr!("Signature"), &signature);
                row(ui, tr!("Blocks"), &animation.blocks.len().to_string());
                row(
                    ui,
                    tr!("Largest block"),
                    &tr!("{bytes} bytes", bytes = animation.max_block_size),
                );
            });
            // ⚠ Only the rows in view: an animation may hold thousands of DOFs.
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical()
                .id_salt(("gta6_anim_dofs", id))
                .max_height(row_height * ROWS_IN_VIEW)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, animation.dofs.len() + 1, |ui, range| {
                    egui::Grid::new(("gta6_anim_dofs_grid", id))
                        .striped(true)
                        .show(ui, |ui| {
                            for heading in [tr!("DOF"), tr!("Track"), tr!("Id"), tr!("Type"), ""] {
                                ui.strong(heading);
                            }
                            ui.end_row();
                            let rows = animation.dofs.iter().enumerate();
                            for (d, dof) in rows.skip(range.start).take(range.len()) {
                                ui.monospace(d.to_string());
                                ui.monospace(dof.track_label());
                                ui.monospace(format!("hash_{:08X}", dof.id));
                                ui.monospace(dof.kind.to_string());
                                if animation.in_delta_section(d) {
                                    ui.label(tr!(
                                        "in section {section}, decoded from its delta code",
                                        section = DELTA_SECTION
                                    ));
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });
                });
            egui::CollapsingHeader::new(tr!("Channels per directory section, by block"))
                .id_salt(("gta6_anim_blocks", id))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("gta6_anim_block_rows", id))
                        .max_height(row_height * ROWS_IN_VIEW)
                        .auto_shrink([false, true])
                        .show_rows(ui, row_height, animation.blocks.len(), |ui, range| {
                            let rows = animation.blocks.iter().enumerate();
                            for (b, block) in rows.skip(range.start).take(range.len()) {
                                ui.monospace(format!("{b}: {:?}", block.sections));
                            }
                        });
                });
        });
}

fn draw_clips(ui: &mut egui::Ui, resource: &AnimResource) {
    let name = |at: &AnimAt| {
        resource
            .sets
            .get(at.set)
            .and_then(|s| s.animations.get(at.animation))
            .map_or("", |a| a.name.as_str())
            .to_string()
    };
    let plays = |clip: &Clip| match &clip.play {
        ClipPlay::One {
            animation,
            key,
            start,
            end,
            rate,
        } => tr!(
            "plays {animation} (key {key}) from {start} s to {end} s at rate {rate}",
            animation = name(animation),
            key = format!("{key:#010X}"),
            start = start,
            end = end,
            rate = rate,
        ),
        ClipPlay::Groups { total, groups } => {
            let external = groups
                .iter()
                .flat_map(|g| &g.entries)
                .filter(|e| e.animation.is_none())
                .count();
            let warped = groups
                .iter()
                .flat_map(|g| &g.entries)
                .filter(|e| !e.warp_points.is_empty())
                .count();
            let mut plays = tr_n!(
                "plays {count} group of animations, {total} s in all",
                "plays {count} groups of animations, {total} s in all",
                groups.len(),
                total = total,
            );
            if warped > 0 {
                plays = tr_n!(
                    "{plays}; {count} entry states warp points",
                    "{plays}; {count} entries state warp points",
                    warped,
                    plays = plays,
                );
            }
            if external == 0 {
                plays
            } else {
                tr_n!(
                    "{plays}; {count} of its animations is in a set another body stores, named by key",
                    "{plays}; {count} of its animations are in sets another body stores, named by key",
                    external,
                    plays = plays,
                )
            }
        }
        ClipPlay::Unresolved => {
            tr!("an entry points at no animation of the sets, so what it plays is not known")
                .to_string()
        }
    };
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("gta6_anim_clip_rows")
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, resource.clips.len(), |ui, range| {
            for clip in &resource.clips[range] {
                ui.label(format!("{} — {}", clip.name, plays(clip)));
            }
        });
}

fn named(hash: u32) -> String {
    known_name(hash).map_or_else(|| format!("hash_{hash:08X}"), str::to_string)
}

/// One row for each clip's tag: its clip, its name, its phases and its attributes.
fn draw_tags(ui: &mut egui::Ui, doc_id: DocumentId, resource: &std::sync::Arc<AnimResource>) {
    draw_lines(ui, ("gta6_anim_tag_lines", doc_id), resource, |resource| {
        (resource.clips.iter())
            .flat_map(|c| c.tags.iter().map(move |t| (c, t)))
            .map(|(clip, tag): (&Clip, &Record)| format!("{} — {}", clip.name, describe(tag)))
            .collect()
    });
}

/// One row for each group entry that states warp points: its clip, group and entry, and the points.
fn draw_warp_points(
    ui: &mut egui::Ui,
    doc_id: DocumentId,
    resource: &std::sync::Arc<AnimResource>,
) {
    draw_lines(ui, ("gta6_anim_warp_lines", doc_id), resource, |resource| {
        let mut lines = Vec::new();
        for clip in &resource.clips {
            let ClipPlay::Groups { groups, .. } = &clip.play else {
                continue;
            };
            for (g, group) in groups.iter().enumerate() {
                for (e, entry) in group.entries.iter().enumerate() {
                    if entry.warp_points.is_empty() {
                        continue;
                    }
                    let points: Vec<String> = (entry.warp_points.iter())
                        .map(|[x, y]| format!("({x}, {y})"))
                        .collect();
                    lines.push(tr!(
                        "{clip} — group {group} entry {entry}: {points}",
                        clip = &clip.name,
                        group = g,
                        entry = e,
                        points = points.join(", ")
                    ));
                }
            }
        }
        lines
    });
}

/// The lines `make` describes `resource` in, made once for the resource and kept, so a frame lays
/// out only the rows in view.
fn draw_lines(
    ui: &mut egui::Ui,
    salt: (&'static str, DocumentId),
    resource: &std::sync::Arc<AnimResource>,
    make: impl FnOnce(&AnimResource) -> Vec<String>,
) {
    let id = egui::Id::new(salt);
    let at = std::sync::Arc::as_ptr(resource) as usize;
    type Cached = (usize, std::sync::Arc<Vec<String>>);
    let lines = match ui
        .data(|d| d.get_temp::<Cached>(id))
        .filter(|(p, _)| *p == at)
    {
        Some((_, lines)) => lines,
        None => {
            let lines = std::sync::Arc::new(make(resource));
            ui.data_mut(|d| d.insert_temp(id, (at, lines.clone())));
            lines
        }
    };
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt(salt.0)
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                ui.label(line.as_str());
            }
        });
}

fn describe(record: &Record) -> String {
    let phases = record
        .phases
        .map_or_else(String::new, |[start, end]| format!(" {start}..{end}"));
    let attributes: Vec<String> = record
        .attributes
        .iter()
        .map(|a| values::attribute(record, a))
        .collect();
    format!("{}{phases}: {}", named(record.name), attributes.join(", "))
}

/// A motion's fields, clips and groups; the clip a search found whose Open was clicked.
fn draw_motion(
    ui: &mut egui::Ui,
    id: (DocumentId, usize),
    motion: &Motion,
    clip_named: &dyn Fn(u64) -> Option<String>,
    open_scanned: bool,
) -> Option<ClipAt> {
    grid(ui, &format!("gta6_anim_motion_{id:?}"), |ui| {
        row(ui, tr!("Key"), &format!("{:#010X}", motion.key));
        row(ui, tr!("Kind"), &motion.kind.to_string());
        row(ui, tr!("Clips, by key"), &motion.clips.len().to_string());
        row(
            ui,
            tr!("Weight lists"),
            &motion.weight_lists.len().to_string(),
        );
        row(ui, tr!("Parameters"), &motion.parameters.len().to_string());
    });
    let mut open = None;
    for &clip in &motion.clips {
        let key = format!("{:#010X}:{:#010X}", clip >> 32, clip & 0xFFFF_FFFF);
        if let Some(named) = clip_named(clip) {
            ui.monospace(format!("{key} {named}"));
        } else if let Some(at) = found_clip(clip) {
            ui.horizontal(|ui| {
                ui.monospace(format!("{key} {} ({})", at.clip, at.dictionary));
                if open_scanned && ui.small_button(tr!("Open")).clicked() {
                    open = Some(at.clone());
                }
            });
        } else {
            ui.monospace(format!("{key} {}", tr!("in no open clip dictionary")));
        }
    }
    for (g, group) in motion.groups.iter().enumerate() {
        ui.label(tr!(
            "Group {group}: clips {first} to {last}, a curve of type {curve} and a range of type {range}",
            group = g,
            first = group.first_clip,
            last = group.first_clip.saturating_add(group.clips).saturating_sub(1),
            curve = group.curve.type_word(),
            range = group.range.type_word(),
        ));
        if let Typed::Rows(database) = &group.range {
            draw_database(ui, g, database);
        }
        values::draw_database(ui, g, &group.range, &motion.properties);
        for tag in &group.tags {
            ui.label(describe(tag));
        }
    }
    for parameter in &motion.parameters {
        ui.monospace(format!(
            "{} {}..{}",
            named(parameter.hash),
            parameter.min,
            parameter.max
        ));
    }
    open
}

/// A type-6 database's counts; [`values::draw_database`] shows its columns and rows.
fn draw_database(ui: &mut egui::Ui, group: usize, database: &Database) {
    let rows: usize = database.tables.iter().map(Vec::len).sum();
    ui.label(tr!(
        "Database of group {group}: {tables} tables of {rows} rows in all, {grids} grids; a row is four items of four 16-bit floats, whose meaning is not known",
        group = group,
        tables = database.tables.len(),
        rows = rows,
        grids = database.grids.len(),
    ));
}
