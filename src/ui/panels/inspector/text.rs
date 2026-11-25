use super::*;

pub(super) fn draw_gxt2(ui: &mut egui::Ui, state: &AppState, doc_id: crate::state::DocumentId) {
    let table = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Gxt2(t)) => t,
        _ => return,
    };

    ui.heading("Global Text Table");
    ui.separator();

    grid(ui, "gxt2_summary", |ui| {
        row(ui, "Entries", &table.len().to_string());
    });

    ui.add_space(crate::ui::theme::SPACING_SNUG);
    ui.label(
        egui::RichText::new(
            "Keys are stored as case-insensitive string hashes. Edit values \
             and search the full table in the center panel.",
        )
        .small()
        .color(theme::TEXT_SECONDARY),
    );
}

pub(super) fn draw_text(ui: &mut egui::Ui, state: &AppState, doc_id: crate::state::DocumentId) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    let DocPayload::Text(text) = &doc.payload else {
        return;
    };
    if super::gta6_move::draw_scanned(ui, doc.path.as_deref()) {
        return;
    }
    if (doc.path.as_deref()).is_some_and(crate::state::rpf9_scan::opened_as_facts) {
        ui.label(tr!("The Document panel lists what GTA VI's headerless reader reads from this binary body, in lines OpenRage writes."));
        return;
    }

    ui.heading("Text Document");
    ui.separator();

    grid(ui, "text_doc_basic", |ui| {
        row(ui, "Name", &doc.name);
        if let Some(path) = &doc.path {
            row(ui, "Path", &path.display().to_string());
            if let Some(ext) = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_uppercase)
            {
                row(ui, "Extension", &ext);
            }
        }
        row(ui, "Encoding", text.encoding.label());
        row(ui, "Line endings", text.line_ending.label());
        row(ui, "Characters", &text.text.len_chars().to_string());
        row(ui, "Lines", &text.text.len_lines().to_string());
        row(ui, "Bytes (decoded)", &text.text.len_bytes().to_string());
    });
}

pub(super) fn draw_name_table(
    ui: &mut egui::Ui,
    state: &AppState,
    doc_id: crate::state::DocumentId,
) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    let DocPayload::NameTable(text) = &doc.payload else {
        return;
    };

    ui.heading("Name Table");
    ui.separator();

    grid(ui, "name_table_basic", |ui| {
        row(ui, "Name", &doc.name);
        if let Some(path) = &doc.path {
            row(ui, "Path", &path.display().to_string());
        }
        let names = text
            .text
            .to_string()
            .lines()
            .filter(|l| !l.is_empty())
            .count();
        row(ui, "Names", &names.to_string());
    });

    ui.add_space(crate::ui::theme::SPACING_SNUG);
    ui.label(
        egui::RichText::new(
            "One name per line, in the file's order: the sibling .dat holds each \
             as its hash and may address it by index. Edit them in the center panel.",
        )
        .small()
        .color(theme::TEXT_SECONDARY),
    );
}

pub(super) fn draw_meta(
    ui: &mut egui::Ui,
    state: &AppState,
    doc_id: crate::state::DocumentId,
    _selection: Option<u64>,
) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    let DocPayload::Meta(meta) = &doc.payload else {
        return;
    };

    ui.heading("Meta Document");
    ui.separator();

    let lib = meta.schema();
    let type_name = meta.type_name();
    let resolved = lib.resolve_any(type_name);
    let internals = shows_internals(state);

    grid(ui, "meta_doc_basic", |ui| {
        row(ui, "Name", &doc.name);
        if let Some(path) = &doc.path {
            row(ui, "Path", &path.display().to_string());
        }
        row(ui, "Schema type", type_name);
        if let Some(r) = &resolved {
            if internals {
                row(ui, "Defined in", &r.def.source_path);
            }
            row(ui, "Fields", &r.fields.len().to_string());
            if let Some(base) = &r.def.base {
                row(ui, "Base", base);
            }
        }
        row(
            ui,
            "Size",
            &or_job::human_bytes(meta.raw_bytes.len() as u64),
        );
        if internals {
            row(ui, "Revision", &meta.revision().to_string());
        }
        if let Some(err) = &meta.parse_error {
            row(ui, "Parse error", err);
        }
    });
    if meta.keeps_a_checksum() {
        ui.label(tr!("The CHKS checksum is kept as it was read and is not recomputed, so the game may refuse the saved file."));
    }
    if meta.gta6_library {
        draw_untyped(ui, meta);
    }
    super::gta6_meta_ids::draw(ui, doc_id, meta);
}

/// What of a GTA VI data XML GTA VI's library does not type, which is kept as text.
fn draw_untyped(ui: &mut egui::Ui, meta: &crate::state::MetaDocument) {
    if let Some(name) = &meta.gta6_untyped {
        ui.label(tr!("GTA VI's library holds no structure {name}, so the whole document is untyped and kept as text.", name = name));
        return;
    }
    let untyped = meta.gta6_untyped_elements;
    if untyped > 0 {
        ui.label(tr_n!(
            "{count} element GTA VI's library does not hold is untyped and kept as text.",
            "{count} elements GTA VI's library does not hold are untyped and kept as text.",
            untyped
        ));
    }
}
