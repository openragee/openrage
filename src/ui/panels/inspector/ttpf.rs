use super::*;

/// The pack's facts, then the selected texture's entry and why it does not read, if it does not.
pub(super) fn draw_ttpf(
    ui: &mut egui::Ui,
    state: &AppState,
    doc_id: crate::state::DocumentId,
    selection: Option<u64>,
) {
    let Some(DocPayload::Ttpf(ttpf)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let pack = &ttpf.pack;
    let stated = pack.rows.last().map_or(0, |sentinel| sentinel.offset);
    ui.heading(tr!("GTA VI Texture Pack"));
    ui.separator();
    grid(ui, "ttpf_pack", |ui| {
        row(ui, tr!("Textures"), &pack.entries.len().to_string());
        row(ui, tr!("Table rows"), &pack.rows.len().to_string());
        row(ui, tr!("File"), &or_job::human_bytes(stated));
        if !pack.is_complete() {
            let on_disk = if pack.size_on_disk() == stated {
                tr!("full length, but sparse: pieces may be missing").to_string()
            } else {
                let percent = 100.0 * pack.size_on_disk() as f64 / stated.max(1) as f64;
                format!("{percent:.1} %")
            };
            row(ui, tr!("On disk"), &on_disk);
        }
        row(
            ui,
            tr!("Decoded"),
            &or_job::human_bytes(pack.header.virtual_size),
        );
    });

    let Some(index) = selection
        .map(|s| s as usize)
        .filter(|&i| i < pack.entries.len())
    else {
        ui.add_space(crate::ui::theme::SPACING_DEFAULT);
        ui.label(tr!("Select a texture to see its entry"));
        return;
    };
    let entry = pack.entries[index];
    let format = match entry.format() {
        Some(format) => format!("{} ({:#05x})", format.name(), entry.format_code),
        None => tr!(
            "{code}, not a measured format",
            code = format!("{:#05x}", entry.format_code)
        ),
    };
    let refusal = ttpf.refusal(index);
    let readable = match refusal {
        None => tr!("yes, read when it is shown"),
        Some(_) => tr!("no"),
    };
    ui.add_space(crate::ui::theme::SPACING_DEFAULT);
    ui.label(
        egui::RichText::new(tr!("Texture {name}", name = ttpf.name(index)))
            .strong()
            .color(theme::TEXT_PRIMARY),
    );
    grid(ui, "ttpf_texture", |ui| {
        row(
            ui,
            tr!("Dimensions"),
            &format!("{} \u{00d7} {} px", entry.width, entry.height),
        );
        row(ui, tr!("Format"), &format);
        let srgb = entry.format().is_some_and(|f| f.is_srgb());
        row(ui, "sRGB", if srgb { tr!("yes") } else { tr!("no") });
        row(ui, tr!("Shape"), &shape(&entry));
        row(ui, tr!("Key"), &format!("{:016X}", entry.key));
        // The model a names harvest found to bind the key, which says what a texture with no name is.
        if let Some(used) = crate::state::gta6_names::texture_use_label(entry.key) {
            row(ui, tr!("Use"), &used);
        }
        let mips = match entry.mip_count {
            0 => tr!("not stated").to_string(),
            n => n.to_string(),
        };
        row(ui, tr!("Mips stated"), &mips);
        if refusal.is_none() {
            let read = ttpf.texture(index).map(|t| t.mip_count);
            row(
                ui,
                tr!("Mips read"),
                &read.map_or(String::new(), |n| n.to_string()),
            );
        }
        if let Some(rows) = pack.rows_of(index) {
            let at = |row: usize| pack.rows.get(row).map_or(0, |r| r.virtual_address);
            let memory = at(rows.end).saturating_sub(at(rows.start));
            row(ui, tr!("Memory"), &or_job::human_bytes(memory));
            let mut codecs: Vec<String> = Vec::new();
            for kind in rows.map(|row| pack.rows[row].kind) {
                let name = codec(kind);
                if !codecs.contains(&name) {
                    codecs.push(name);
                }
            }
            row(ui, tr!("Row codecs"), &codecs.join(", "));
        }
        row(
            ui,
            tr!("Tiles"),
            if entry.tile_mode == 0 {
                "64 KiB"
            } else {
                "4 KiB"
            },
        );
        let bodies = pack.rows_of(index).map_or(0, |rows| rows.len());
        row(ui, tr!("Bodies"), &bodies.to_string());
        row(ui, tr!("Readable"), readable);
    });
    // What the read of its bodies through the coder may be called, and what that word rests on.
    // ⚠ Under the grid, where a long sentence wraps: as a row it widens the grid past the panel.
    if let Some(decode) = ttpf.decode_label(index).filter(|_| refusal.is_none()) {
        ui.label(format!("{}: {decode}", tr!("Decode")));
        for basis in ttpf.decode_basis(index) {
            ui.label(egui::RichText::new(basis).color(theme::TEXT_SECONDARY));
        }
    }
    if let Some(why) = refusal {
        ui.label(egui::RichText::new(why).color(theme::TEXT_SECONDARY));
    }
}

/// A texture's shape as its entry states it: 2D, a cube, a volume of its depth, an array of its slices.
fn shape(entry: &or_io::gta6::ttpf::Entry) -> String {
    match entry.dimension {
        1 => tr!("2D").to_string(),
        2 => tr!("cube").to_string(),
        3 => tr!("volume, {depth} deep", depth = entry.slices),
        5 => tr_n!(
            "array of {count} slice",
            "array of {count} slices",
            entry.slices
        ),
        other => tr!("dimension {other}", other = other),
    }
}

/// A body's codec kind: stored, zlib, or BC blocks through an entropy coder that is not known.
fn codec(kind: u8) -> String {
    match kind {
        or_io::gta6::ttpf::KIND_STORED => tr!("stored").to_string(),
        or_io::gta6::ttpf::KIND_ZLIB => "zlib".to_string(),
        1 | 3..=7 => tr!("BC{kind} coder", kind = kind),
        other => tr!("kind {other}", other = other),
    }
}
