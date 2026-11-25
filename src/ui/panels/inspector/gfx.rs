use super::*;

enum Action {
    Export,
    Import,
}

pub(super) fn draw_gfx(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    ui.heading("Scaleform Movie");
    ui.separator();
    let movie = match state.gfx_movie(doc_id) {
        Ok(movie) => movie,
        Err(why) => {
            ui.colored_label(theme::WARNING, why);
            return;
        }
    };
    let writable = !state
        .document(doc_id)
        .is_some_and(crate::state::Document::is_from_rpf9_table);
    let mut action = None;
    ui.label(format!(
        "SWF version {}, {}, {} tags",
        movie.version,
        if movie.compressed {
            "zlib-compressed"
        } else {
            "uncompressed"
        },
        movie.tags.len()
    ));
    ui.horizontal(|ui| {
        if ui
            .button(format!("{} Export SWF", ph::EXPORT))
            .on_hover_text("Write the movie as a standard SWF for an editor such as JPEXS")
            .clicked()
        {
            action = Some(Action::Export);
        }
        if ui
            .add_enabled(
                writable,
                egui::Button::new(format!("{} Replace from SWF", ph::SWAP)),
            )
            .on_hover_text("Replace the movie with an SWF saved from an editor")
            .clicked()
        {
            action = Some(Action::Import);
        }
    });

    let textures = movie.texture_refs();
    egui::CollapsingHeader::new(format!("Texture references ({})", textures.len()))
        .id_salt("gfx_textures")
        .show(ui, |ui| {
            egui::Grid::new("gfx_texture_grid")
                .striped(true)
                .show(ui, |ui| {
                    for heading in ["Tag", "Id", "File", "Export name"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for t in &textures {
                        ui.monospace(t.tag.to_string());
                        ui.monospace(t.id.to_string());
                        ui.monospace(&t.file_name);
                        ui.monospace(t.export_name.as_deref().unwrap_or(""));
                        ui.end_row();
                    }
                });
        });
    let imports = movie.imports();
    egui::CollapsingHeader::new(format!("Imports ({})", imports.len()))
        .id_salt("gfx_imports")
        .show(ui, |ui| {
            egui::Grid::new("gfx_import_grid")
                .striped(true)
                .show(ui, |ui| {
                    for heading in ["Tag", "Id", "Name", "From"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for i in &imports {
                        ui.monospace(i.tag.to_string());
                        ui.monospace(i.id.to_string());
                        ui.monospace(&i.name);
                        ui.monospace(&i.url);
                        ui.end_row();
                    }
                });
        });

    ui.separator();
    ui.strong("Tags");
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical().id_salt("gfx_tags").show_rows(
        ui,
        row_height,
        movie.tags.len(),
        |ui, rows| {
            for i in rows {
                let tag = &movie.tags[i];
                let defines = tag
                    .character_id()
                    .map(|id| format!("  defines {id}"))
                    .unwrap_or_default();
                ui.monospace(format!(
                    "{i:>5}  {:>4} {:<28} {:>9} bytes{defines}",
                    tag.code,
                    tag.name().unwrap_or("?"),
                    tag.body.len()
                ));
            }
        },
    );

    let outcome = match action {
        None => return,
        Some(Action::Export) => export(state, doc_id),
        Some(Action::Import) => import(state, doc_id),
    };
    match outcome {
        Ok(Some(done)) => state.show_toast(done),
        Ok(None) => {}
        Err(why) => state.show_error_toast(why),
    }
}

fn export(state: &mut AppState, doc_id: DocumentId) -> Result<Option<String>, String> {
    let swf = state.gfx_swf(doc_id)?;
    let name = state
        .document(doc_id)
        .and_then(|d| d.path.as_ref())
        .and_then(|p| p.file_stem())
        .map_or_else(|| "movie".to_string(), |s| s.to_string_lossy().into_owned());
    let Some(path) = crate::platform::file_dialogs::ask("Export Movie as SWF", |dialog| {
        dialog
            .set_file_name(format!("{name}.swf"))
            .add_filter("SWF", &["swf"])
            .save_file()
    }) else {
        return Ok(None);
    };
    std::fs::write(&path, swf).map_err(|e| format!("Export failed: {e}"))?;
    Ok(Some(format!("Wrote {}", path.display())))
}

fn import(state: &mut AppState, doc_id: DocumentId) -> Result<Option<String>, String> {
    let Some(path) = crate::platform::file_dialogs::ask("Replace Movie from SWF", |dialog| {
        dialog.add_filter("SWF", &["swf"]).pick_file()
    }) else {
        return Ok(None);
    };
    let swf = std::fs::read(&path).map_err(|e| format!("Read error: {e}"))?;
    state.gfx_import_swf(doc_id, &swf)?;
    Ok(Some(format!("Replaced the movie from {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame: a shape, a frame and the end.
    fn movie() -> or_io::gfx::GfxMovie {
        let mut bytes = b"GFX\x08\x17\x00\x00\x00\x00\x00\x1E\x01\x00".to_vec();
        bytes.extend_from_slice(&[0x84, 0x00, 3, 0, 0xAA, 0xBB, 0x40, 0x00, 0x00, 0x00]);
        or_io::gfx::GfxMovie::open(&bytes).expect("the hand-built movie opens")
    }

    #[test]
    fn the_inspector_draws_a_movies_tags() {
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Text);
        let d = state.document_mut(doc).expect("the blank document");
        d.payload = DocPayload::Gfx(movie());
        d.kind = DocumentKind::Gfx;

        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| draw_gfx(ui, &mut state, doc));
        assert!(!output.shapes.is_empty());
        let text = format!("{:?}", output.shapes);
        assert!(text.contains("DefineShape"), "the tag list names the shape");
    }
}
