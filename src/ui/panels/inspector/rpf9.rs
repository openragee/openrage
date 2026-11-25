use or_io::container::rpf::{
    Rpf9BodyKind, Rpf9Content, Rpf9Resource, Rpf9ScanFind, Rpf9Walk, Rpf9ZlibRun,
};

use super::*;
use crate::state::rpf9_scan::{
    opens_as_bytes, row_at_offset, rows_between, rows_to_extract, Ended, Found, ListFilter, Row,
    RowKind, View,
};

/// The archive's header facts, what a scan without the table is, its progress and the bodies it
/// found, laying out only the rows in view: a large archive holds hundreds of thousands.
pub(super) fn draw_rpf9(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    if let Some(DocPayload::Rpf9Table(table)) = state.document(doc_id).map(|d| &d.payload) {
        draw_table_facts(ui, table);
        return;
    }
    let Some(DocPayload::Rpf9(scan)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let scan = scan.clone();
    let nested = scan.start > 0;
    match &scan.header {
        Some(header) => {
            ui.heading(tr!("GTA VI Archive"));
            ui.separator();
            ui.label(crate::state::gta_vi::rpf9_why());
            ui.label(crate::state::gta_vi::rpf9_facts(header));
            if nested {
                ui.label(tr!(
                    "It is nested in {file} at {at}, and the offsets below are that file's.",
                    file = crate::state::jobs::file_name(&scan.path),
                    at = format!("{:#X}", scan.start),
                ));
            }
        }
        None => {
            ui.heading(tr!("Archive"));
            ui.separator();
            ui.colored_label(
                theme::WARNING,
                tr!("Its first 64 bytes are zero, as a piece a download has not delivered reads, so whether it is an RPF9 archive and what its header states are not known; the bytes present are scanned."),
            );
        }
    }
    ui.label(tr!("What follows is a scan without the table: the bodies stored in the clear, found by their own bytes. A body has no name and no folder, and its end is given only where its bytes make it certain."));
    ui.add_space(crate::ui::theme::SPACING_SNUG);

    let found = scan.found();
    let reached = or_job::human_bytes(found.reached);
    let total = or_job::human_bytes(scan.file_len);
    let mut again = false;
    match &found.ended {
        None if found.reading_kept => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr!("Reading the kept scan…"));
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        None => {
            ui.horizontal(|ui| {
                if nested {
                    ui.spinner();
                    ui.label(tr!(
                        "Scanned to {at}; the scan ends at this archive's names.",
                        at = format!("{:#X}", found.reached)
                    ));
                } else {
                    let fraction = found.reached as f32 / scan.file_len.max(1) as f32;
                    ui.add(egui::ProgressBar::new(fraction).desired_width(240.0));
                    ui.label(tr!(
                        "Scanned {reached} of {total}",
                        reached = reached,
                        total = total
                    ));
                }
                let stop = ui.add_enabled(!scan.stopping(), egui::Button::new(tr!("Stop")));
                if stop.clicked() {
                    scan.stop();
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        Some(Ended::Whole) if nested => {
            ui.label(tr!(
                "The scan read the whole archive, from {from} to the end of its names at {to}.",
                from = format!("{:#X}", scan.start),
                to = format!("{:#X}", found.reached),
            ));
        }
        Some(Ended::Whole) => {
            ui.label(tr!("The scan read the whole file, {total}.", total = total));
            if let Some(made) = found.kept {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr!(
                        "The list is a scan kept {ago}: the archive was not read again.",
                        ago = crate::state::prefs::ago(made)
                    ));
                    let button = ui.button(tr!("Scan again")).on_hover_text(tr!(
                        "Reads the whole archive again and keeps that scan in place of this one."
                    ));
                    again = button.clicked();
                });
            }
        }
        Some(Ended::Stopped) => {
            ui.label(tr!(
                "The scan was stopped after {reached} of {total}; nothing after that is listed.",
                reached = reached,
                total = total
            ));
        }
        Some(Ended::Failed(why)) => {
            ui.colored_label(
                theme::WARNING,
                tr!(
                    "The scan stopped after {reached} of {total}: {why}",
                    reached = reached,
                    total = total,
                    why = why
                ),
            );
        }
    }
    ui.label(
        egui::RichText::new(tr_n!(
            "{count} body found; {missing} missing, as pieces a download has not delivered; {unclaimed} that no body claims.",
            "{count} bodies found; {missing} missing, as pieces a download has not delivered; {unclaimed} that no body claims.",
            found.bodies,
            missing = or_job::human_bytes(found.missing),
            unclaimed = or_job::human_bytes(found.unclaimed),
        ))
        .color(theme::TEXT_SECONDARY),
    );
    drop(found);
    if again {
        scan.scan_again();
    }
    if !nested && scan.header.is_some() {
        super::gta6_dialogue::draw_dialogue(ui, state);
    }
}

/// An archive listed from its table: the sentence that says so, with its entries and key, and its header's facts.
fn draw_table_facts(
    ui: &mut egui::Ui,
    table: &crate::state::rpf9_scan::rpf9_table_document::Rpf9TableDocument,
) {
    ui.heading(tr!("GTA VI Archive"));
    ui.separator();
    if let Some(root) = &table.tree().root {
        let named = root.rows.iter().filter(|row| row.name.is_some()).count();
        ui.label(tr_n!(
            "Listed from its decrypted table: {count} entry, {named} of them named, under the table key {key}.",
            "Listed from its decrypted table: {count} entries, {named} of them named, under the table key {key}.",
            root.rows.len(),
            named = named,
            key = format!("{:#010X}", root.key_id),
        ));
    }
    ui.label(crate::state::gta_vi::rpf9_facts(&table.header));
}

/// The bodies a scan found, under their filter, laying out only the rows in view: a large archive
/// holds hundreds of thousands. A click opens a body in the preview tab and a double click keeps it.
pub(super) fn draw_scan_rows(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Rpf9(scan)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let scan = scan.clone();
    let on_screen = state.scan_row_on_screen(doc_id);
    // A step of history that leads to a row scrolls the list to it.
    let revealed = (state.navigation.reveal)
        .filter(|at| at.doc == doc_id)
        .map(|at| at.node);
    if revealed.is_some() {
        state.navigation.reveal = None;
    }
    let mut parent = false;
    ui.horizontal(|ui| {
        let file = crate::state::jobs::file_name(&scan.path);
        // The way back to the archive's own tab from a body of it, and to its parent from a nested one.
        parent = ui.link(file).clicked();
        if scan.start > 0 {
            ui.label("›");
            ui.strong(format!("{:#X}", scan.start));
        }
    });
    let found = scan.found();
    let mut list = scan.list();
    if let Some(offset) = revealed {
        list.jumped = row_at_offset(&found.rows, offset);
        list.scroll = true;
    }
    draw_find(ui, &mut list, &found.rows);
    list.seek(&found.rows);
    let filtering = list.filtering(found.rows.len());
    let (jumped, scroll) = (list.jumped, std::mem::take(&mut list.scroll));
    let names = crate::state::gta6_names::labels(&scan.path);
    let name_of = |row: &Row| match row {
        Row::Body(find) => names.get(&find.offset).map(String::as_str),
        Row::Missing { .. } => None,
    };
    let shown = list.shown(&found.rows, |row| named_words(row, name_of(row)));
    let count = shown.map_or(found.rows.len(), <[usize]>::len);
    let selection_id = egui::Id::new(("scanned bodies selected", doc_id));
    let mut selection: std::collections::BTreeSet<usize> =
        ui.data(|d| d.get_temp(selection_id)).unwrap_or_default();
    // The row a Shift+click's range starts from: the one clicked last.
    let anchor_id = egui::Id::new(("scanned bodies anchor", doc_id));
    let mut anchor: Option<usize> = ui.data(|d| d.get_temp(anchor_id)).flatten();
    let mut extract: Option<Vec<usize>> = None;
    ui.horizontal_wrapped(|ui| {
        let label = match selection.len() {
            0 => tr_n!(
                "Extract the {count} row shown…",
                "Extract the {count} rows shown…",
                count
            ),
            selected => tr_n!(
                "Extract the {count} selected body…",
                "Extract the {count} selected bodies…",
                selected
            ),
        };
        // ⚠ A filter still reading the list shows a part of what it will: the rest would be left out.
        let button = ui
            .add_enabled(count > 0 && !filtering, egui::Button::new(label))
            .on_hover_text(tr!("Extracts the rows selected, or every row the list shows where none is. Ctrl+click and Shift+click select rows."));
        if button.clicked() {
            extract = Some(rows_to_extract(&selection, shown, found.rows.len()));
        }
        if let Some(shown) = shown {
            let said = tr_n!(
                "{count} row of {total} matches",
                "{count} rows of {total} match",
                shown.len(),
                total = found.rows.len(),
            );
            ui.label(egui::RichText::new(said).color(theme::TEXT_SECONDARY));
            if filtering {
                ui.spinner();
                ui.ctx().request_repaint();
            }
        }
    });
    let row_of = |at: usize| shown.map_or(at, |s| s[at]);
    let row_height = ui.spacing().interact_size.y;
    let (mut clicked, mut kept) = (None, None);
    let rows = egui::ScrollArea::both()
        .id_salt(("scanned bodies", doc_id))
        .auto_shrink([false, false]);
    rows.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        if let (true, Some(row)) = (scroll, jumped) {
            // The shown row the jump landed on, or the last shown before it.
            let at = shown.map_or(row, |s| s.partition_point(|&r| r <= row).saturating_sub(1));
            let y = top + at as f32 * row_height;
            let rect = egui::Rect::from_min_size(
                egui::pos2(ui.max_rect().left(), y),
                egui::vec2(ui.available_width().max(1.0), row_height),
            );
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
        let clip = ui.clip_rect();
        let at = |y: f32| (((y - top) / row_height).max(0.0) as usize).min(count);
        let (first, end) = (at(clip.top()), (at(clip.bottom()) + 1).min(count));
        let first = first.min(end);
        ui.add_space(first as f32 * row_height);
        for index in first..end {
            let (index, row) = (row_of(index), &found.rows[row_of(index)]);
            let background = ui.painter().add(egui::Shape::Noop);
            let drawn = ui.horizontal(|ui| {
                ui.set_height(row_height);
                // ⚠ A selectable label senses drags, which would take presses meant for the row.
                ui.style_mut().interaction.selectable_labels = false;
                draw_row(ui, row, name_of(row))
            });
            let rect = drawn.response.rect;
            if jumped == Some(index) {
                let fill = ui.visuals().selection.bg_fill;
                ui.painter()
                    .set(background, egui::Shape::rect_filled(rect, 2.0, fill));
            }
            let (Row::Body(find), Some(name)) = (row, drawn.inner) else {
                continue;
            };
            // The whole row is one control, laid over its labels so that a click anywhere opens it.
            let id = ui.id().with(("scanned body", index));
            let response = ui.interact(rect, id, egui::Sense::click());
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
            let row = index;
            let selected = selection.contains(&row) || on_screen == Some(find.offset);
            if response.hovered() || response.has_focus() || selected {
                let fill = match selected {
                    true => ui.visuals().selection.bg_fill,
                    false => ui.visuals().widgets.hovered.weak_bg_fill,
                };
                ui.painter()
                    .set(background, egui::Shape::rect_filled(rect, 2.0, fill));
            }
            let response = response.on_hover_text(hover(&found, find));
            let modifiers = ui.input(|i| i.modifiers);
            if response.clicked() && modifiers.shift {
                selection.extend(rows_between(anchor.unwrap_or(row), row, shown));
            } else if response.clicked() && modifiers.command {
                if !selection.remove(&row) {
                    selection.insert(row);
                }
                anchor = Some(row);
            } else if response.double_clicked() {
                kept = Some(row);
            } else if response.clicked() {
                selection.clear();
                (anchor, clicked) = (Some(row), Some(row));
            }
            response.context_menu(|ui| {
                if ui.button(tr!("Extract…")).clicked() {
                    extract = Some(vec![row]);
                    ui.close_kind(egui::UiKind::Menu);
                }
                let many = tr_n!(
                    "Extract the {count} selected body…",
                    "Extract the {count} selected bodies…",
                    selection.len()
                );
                if selection.len() > 1 && ui.button(many).clicked() {
                    extract = Some(selection.iter().copied().collect());
                    ui.close_kind(egui::UiKind::Menu);
                }
            });
        }
        ui.add_space((count - end) as f32 * row_height);
    });
    drop(list);
    drop(found);
    ui.data_mut(|d| {
        d.insert_temp(selection_id, selection);
        d.insert_temp(anchor_id, anchor);
    });
    if let Some(row) = clicked {
        state.preview_scanned_row(doc_id, row);
    }
    if let Some(row) = kept {
        state.open_scanned_body(doc_id, row);
    }
    if parent {
        state.load_file(scan.path.clone());
    }
    if let Some(rows) = extract {
        if let Some(dest) = crate::platform::file_dialogs::pick_extract_destination() {
            state.extract_scanned_bodies(doc_id, &rows, dest);
        }
    }
}

/// The Find field, which keeps rows by its words or sends the list to an offset, and a toggle for
/// each kind the list holds, with how many rows are of it.
fn draw_find(ui: &mut egui::Ui, list: &mut ListFilter, rows: &[Row]) {
    let find = egui::TextEdit::singleline(&mut list.text)
        .hint_text(tr!("Find by kind or size, or go to 0x offset"))
        .desired_width(f32::INFINITY);
    ui.add(find);
    let counts = list.counts(rows).clone();
    if list.counting(rows.len()) {
        ui.ctx().request_repaint();
    }
    ui.horizontal_wrapped(|ui| {
        for kind in RowKind::ALL {
            let mut on = list.kinds.contains(&kind);
            let count = counts.get(&kind).copied().unwrap_or(0);
            // A kind toggled on stays, so that it can be toggled off.
            if count == 0 && !on {
                continue;
            }
            let label = format!("{} {count}", row_kind_label(kind));
            if ui.toggle_value(&mut on, label).changed() {
                if on {
                    list.kinds.insert(kind);
                } else {
                    list.kinds.remove(&kind);
                }
            }
        }
    });
}

pub(crate) fn row_kind_label(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Model => tr!("Models"),
        RowKind::Dictionary => tr!("Dictionaries"),
        RowKind::Collision => tr!("Collision"),
        RowKind::Navigation => tr!("Navigation"),
        RowKind::Archive => tr!("Archives"),
        RowKind::Text => tr!("Text"),
        RowKind::Metadata => tr!("Metadata"),
        RowKind::Resource => tr!("Other resources"),
        RowKind::Animation => tr!("Animations"),
        RowKind::Movie => tr!("Movies"),
        RowKind::Audio => tr!("Audio"),
        RowKind::Shaders => tr!("Shaders"),
        RowKind::Unknown => tr!("Not known"),
        RowKind::Missing => tr!("Missing"),
    }
}

/// A body's offset, what it is, and for any but an archive's header its size.
fn body_words(find: &Rpf9ScanFind) -> (String, String, Option<String>) {
    let at = format!("{:#012X}", find.offset);
    match find.kind {
        Rpf9BodyKind::Archive(header) => {
            let heading = tr_n!(
                "archive header: {count} entry, table to {toc}, names {names} bytes",
                "archive header: {count} entries, table to {toc}, names {names} bytes",
                header.entry_count,
                toc = format!("{:#X}", find.offset + u64::from(header.toc_end)),
                names = header.names_size,
            );
            (at, heading, None)
        }
        _ => (at, kind_label(find), Some(size_label(find))),
    }
}

/// The words a row shows, which name a body's row and which the filter's text is found in.
fn row_words(row: &Row) -> String {
    match row {
        &Row::Missing { from, to, bytes } => tr!(
            "{bytes} missing between {from} and {to}: pieces a download has not delivered",
            bytes = or_job::human_bytes(bytes),
            from = format!("{from:#X}"),
            to = format!("{to:#X}"),
        ),
        Row::Body(find) => match body_words(find) {
            (at, what, Some(size)) => format!("{at}  {what}  {size}"),
            (at, what, None) => format!("{at}  {what}"),
        },
    }
}

/// A row's words and the name a harvest kept for its body, which the filter's text is found in too.
fn named_words(row: &Row, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{}  {name}", row_words(row)),
        None => row_words(row),
    }
}

/// One line of the list; for a body, the words it shows, which name the row that opens it.
/// ⚠ The name a harvest kept comes right after the offset: after the size it lies past the
/// panel's edge, where nothing says it is.
fn draw_row(ui: &mut egui::Ui, row: &Row, name: Option<&str>) -> Option<String> {
    let Row::Body(find) = row else {
        ui.colored_label(theme::WARNING, row_words(row));
        return None;
    };
    match body_words(find) {
        // A group's heading: the bodies after it, up to the next, follow its header. Its words
        // start in the column a body's kind starts in.
        (at, heading, None) => {
            let strong = |text: &str| {
                egui::RichText::new(text)
                    .monospace()
                    .strong()
                    .color(theme::TEXT_PRIMARY)
            };
            ui.label(strong(&at));
            ui.label(strong(&heading));
        }
        (at, kind, Some(size)) => {
            ui.label(egui::RichText::new(&at).monospace());
            if let Some(name) = name {
                ui.label(egui::RichText::new(name).color(theme::TEXT_PRIMARY));
            }
            ui.label(egui::RichText::new(&kind).monospace());
            ui.label(egui::RichText::new(&size).color(theme::TEXT_SECONDARY));
        }
    }
    Some(named_words(row, name))
}

/// What a body's row says under the pointer: how it opens, or why it does not.
fn hover(found: &Found, find: &Rpf9ScanFind) -> String {
    match found.view(find) {
        Ok(View::Headerless) => tr!("A body that starts with four zero bytes. Click to open it where GTA VI's RBM reader reads it from its own bytes, read-only in this archive; otherwise it opens as its bytes, with the reason.").to_string(),
        Ok(View::Archive) => tr!("Click to open this nested archive as a scan of its own, from its header to the end of its names").to_string(),
        Ok(_) => tr!("Click to open it, read-only in this archive: Save As writes it, with any edit its view takes, as a file of its own").to_string(),
        Err(why) if opens_as_bytes(find) => {
            format!("{why} {}", tr!("Click to open its bytes read-only."))
        }
        Err(why) => why,
    }
}

/// Where a body opened from a scan's list came from: its archive and offset, what the scan found
/// there, for a resource its block map and how far its blocks were followed, and for a body shown
/// as its bytes why no view reads it.
pub(super) fn draw_scan_origin(ui: &mut egui::Ui, state: &AppState, doc_id: DocumentId) {
    let Some(path) = state.document(doc_id).and_then(|d| d.path.as_deref()) else {
        return;
    };
    let Some((find, loose)) = crate::state::rpf9_scan::scan_origin(path) else {
        return;
    };
    let heading = match loose {
        true => tr!("A GTA VI resource in a file of its own"),
        false => tr!("Found by a scan of a GTA VI archive"),
    };
    ui.separator();
    ui.label(egui::RichText::new(heading).strong());
    grid(ui, "scan_origin", |ui| {
        if loose {
            row(ui, tr!("File"), &path.display().to_string());
        } else {
            let archive = path.parent().unwrap_or(path).display().to_string();
            row(ui, tr!("Archive"), &archive);
            row(ui, tr!("Offset"), &format!("{:#X}", find.offset));
        }
        row(ui, tr!("Found as"), &kind_label(&find));
        row(ui, tr!("Size"), &sizes(&find));
        if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = find.kind
        {
            row(ui, tr!("Block map"), &block_map(&resource));
            let walk = walk_label(resource.walk)
                .unwrap_or_else(|| tr!("every block its block map counts was found").to_string());
            row(ui, tr!("Blocks followed"), &walk);
        }
        if let Some(DocPayload::Hex(hex)) = state.document(doc_id).map(|d| &d.payload) {
            row(ui, tr!("Shown as bytes"), &hex.banner);
        }
        if let Some(note) = crate::state::gta6_dialogue::strict_note(path) {
            row(ui, tr!("Strict tree"), &note);
        }
    });
}

fn block_map(resource: &Rpf9Resource) -> String {
    tr!(
        "Its block map counts {virtual_blocks} virtual and {physical_blocks} physical blocks, which hold {virtual_size} and {physical_size}.",
        virtual_blocks = resource.virtual_blocks,
        physical_blocks = resource.physical_blocks,
        virtual_size = or_job::human_bytes(resource.virtual_len),
        physical_size = or_job::human_bytes(resource.physical_len),
    )
}

fn kind_label(find: &Rpf9ScanFind) -> String {
    if let Rpf9BodyKind::Zlib(run) = find.kind {
        if let Some(signature) = run.signature {
            return headerless_label(signature);
        }
    }
    let what = match find.kind {
        Rpf9BodyKind::Archive(_) => tr!("archive"),
        Rpf9BodyKind::Zlib(run) => match run.content {
            Rpf9Content::Xml => tr!("XML"),
            Rpf9Content::Text => tr!("text"),
            Rpf9Content::Pso => tr!("PSO metadata"),
            Rpf9Content::Rbf => tr!("RBF metadata"),
            Rpf9Content::ResourceShaped => tr!("four zero bytes first"),
            Rpf9Content::ClipRoot => tr!("GTA VI animations, headerless"),
            Rpf9Content::PipelineCache => tr!("GTA VI pipeline cache"),
            Rpf9Content::TextTable => tr!("GTA VI text table"),
            Rpf9Content::AudioData => tr!("GTA VI audio game data"),
            Rpf9Content::Identified(body) => crate::state::gta6_other_bodies::label(body),
            Rpf9Content::Other => tr!("zlib, not known"),
        },
        Rpf9BodyKind::StoredRsc8 { .. } => tr!("stored RSC8"),
        Rpf9BodyKind::Bink2 { .. } => tr!("Bink 2 movie"),
        Rpf9BodyKind::Awc { .. } => tr!("audio container"),
        Rpf9BodyKind::Sgd5 { .. } => tr!("SGD5 effect container"),
        Rpf9BodyKind::Names { .. } => tr!("names, encrypted"),
        Rpf9BodyKind::FirstEntry { .. } => tr!("first entry, not known"),
        Rpf9BodyKind::RawDeflate {
            content: Rpf9Content::Pso,
        } => tr!("PSO metadata"),
        Rpf9BodyKind::RawDeflate { .. } => tr!("RBF metadata"),
    };
    match find.container {
        Some(at) => tr!(
            "{what}, in the audio container at {at}",
            what = what,
            at = format!("{at:#X}")
        ),
        None => what.to_string(),
    }
}

/// A headerless resource by its role, or by its signature where no role is known.
fn headerless_label(signature: or_io::container::rpf::Rpf9Signature) -> String {
    match signature.role() {
        Some(role) => tr!(
            "headerless {role}",
            role = crate::state::gta6_resource::role_name(Some(role))
        ),
        None => tr!("headerless, signature {signature}", signature = signature),
    }
}

/// A resource's walk, where it did not find every block; else its sizes.
fn size_label(find: &Rpf9ScanFind) -> String {
    if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    {
        if let Some(walk) = walk_label(resource.walk) {
            return walk;
        }
    }
    sizes(find)
}

/// How far a resource's blocks were followed; `None` when every block was found.
fn walk_label(walk: Rpf9Walk) -> Option<String> {
    Some(match walk {
        Rpf9Walk::Whole => return None,
        Rpf9Walk::NotOnDisk { at } => tr!(
            "a block at {at} is in a piece a download has not delivered",
            at = format!("{at:#X}")
        ),
        Rpf9Walk::Refused { at, why } => tr!(
            "its blocks do not hold to the measured layout at {at}: {why}",
            at = format!("{at:#X}"),
            why = why
        ),
        Rpf9Walk::TailNotDeflate { at } => tr!(
            "its zlib stream stops being deflate data at {at} with no piece missing before it: its tail is replaced or protected",
            at = format!("{at:#X}")
        ),
        Rpf9Walk::PointersPast { section, targets } => tr!(
            "every block was found; {targets} pointer words aim just past its {section} section: values, or a block of another resource",
            targets = targets,
            section = section
        ),
    })
}

/// A body's stored and inflated sizes, where the scan knows its end.
fn sizes(find: &Rpf9ScanFind) -> String {
    match (find.stored_len, find.inflated_len) {
        (Some(stored), Some(inflated)) if stored != inflated => tr!(
            "{stored}, {inflated} inflated",
            stored = or_job::human_bytes(stored),
            inflated = or_job::human_bytes(inflated)
        ),
        (Some(stored), _) => or_job::human_bytes(stored),
        (None, _) => tr!("end not known").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_the_filter_reads_hold_the_name_a_harvest_kept() {
        let row = Row::Missing {
            from: 0,
            to: 16,
            bytes: 16,
        };
        assert!(named_words(&row, Some("a_made_up_model.#dr")).ends_with("a_made_up_model.#dr"));
        assert_eq!(named_words(&row, None), row_words(&row));
    }

    #[test]
    fn an_archive_listed_from_its_table_says_so_in_its_tab() {
        use crate::state::rpf9_scan::rpf9_table_document::tests::{sample_document, state_showing};
        let _serial = crate::ui::theme::serial();
        let (_dir, doc) = sample_document();
        let (mut state, id) = state_showing(doc);
        let output = egui::Context::default().run_ui(Default::default(), |ui| {
            draw_rpf9(ui, &mut state, id);
        });
        let shapes = format!("{:?}", output.shapes);
        assert!(
            shapes.contains("Listed from its decrypted table: 3 entries, 2 of them named"),
            "{shapes}"
        );
        assert!(!shapes.contains("scan without the table"), "{shapes}");
    }
}
