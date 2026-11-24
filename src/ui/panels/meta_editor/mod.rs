mod bytes;
mod ctx;
mod derived;
mod header;
mod picker;
mod presets;
mod reference;
mod widgets;

use crate::state::{gta_install_root_from_prefs, reference_names, AppState, Change, DocPayload};
use crate::ui::theme;

use ctx::EditorCtx;
pub(crate) use widgets::item_summary;

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(doc_id) = state.active_document else {
        empty(ui);
        return;
    };
    if !matches!(
        state.document(doc_id).map(|d| &d.payload),
        Some(DocPayload::Meta(_))
    ) {
        empty(ui);
        return;
    }

    if let Some(doc) = state.document_mut(doc_id) {
        if let DocPayload::Meta(m) = &mut doc.payload {
            m.ensure_parsed_current();
            m.refresh_validation();
        }
    }

    header::draw(ui, state, doc_id);
    ui.separator();
    if crate::ui::panels::gta6_anim_scene::draw(ui, state, doc_id) {
        ui.separator();
    }
    draw_visual_view(ui, state, doc_id);
}

/// What stands above the fields of a type the schema does not describe: `named` for a GTA VI
/// body while that game's name snapshot is installed, whose names are not the public tables'.
fn undescribed_note(named: bool) -> &'static str {
    match named {
        true => tr_key!(
            "The fields below are the ones this GTA VI body's own table states. Names come from GTA VI's name snapshot; a name the snapshot holds only as a hash shows its hash."
        ),
        false => tr_key!(
            "No schema is staged for this type, so the fields below are the ones the document itself carries. Names come from the public tables; a field the tables do not name shows its hash."
        ),
    }
}

fn empty(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("Open a .meta, a metadata .xml or a .srl to edit its fields here.")
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    });
}

fn draw_visual_view(ui: &mut egui::Ui, state: &mut AppState, doc_id: crate::state::DocumentId) {
    let library = crate::state::meta_schema();
    let name_index = state.name_index.snapshot();
    let places_an_interior = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Meta(meta)) => meta
            .parsed
            .as_ref()
            .is_some_and(|p| reference_names::places_an_interior(&p.instance)),
        _ => false,
    };
    let install = gta_install_root_from_prefs(&state.prefs).filter(|_| state.keys.has_keys());
    let mount = state.prefs.mount_options();
    let tables = match &install {
        Some(root) => {
            let crypto = state.keys.provider();
            state.game_tables.ensure(root.clone(), crypto, mount);
            state.game_tables.get()
        }
        None => Default::default(),
    };
    let interiors = match (places_an_interior, install) {
        (true, Some(root)) => {
            state.interiors.ensure(root, state.keys.provider(), mount);
            state.interiors.get()
        }
        _ => Default::default(),
    };
    state.presets.ensure_loaded();
    let ctx = EditorCtx {
        library,
        name_index,
        references: reference_names::ReferenceNames::placing(interiors, tables),
        path: Vec::new(),
        records: std::rc::Rc::new(presets::RecordUi::of(state, doc_id)),
        edits: Default::default(),
    };

    let Some(doc) = state.document_mut(doc_id) else {
        return;
    };
    // A scanned GTA VI metadata body saves its edits as a copy, so its form edits as any other's.
    let read_only = doc
        .read_only
        .filter(|why| *why != crate::state::rpf9_scan::READ_ONLY_BODY);
    let DocPayload::Meta(meta) = &mut doc.payload else {
        return;
    };
    let meta_is_gta_vi = meta.is_gta_vi_body();

    let mut dirty = false;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if let Some(err) = meta.parse_error.clone() {
                ui.colored_label(
                    theme::WARNING,
                    "The XML in the Text tab failed to parse against the installed schema:",
                );
                ui.add_space(theme::SPACING_TIGHT);
                ui.label(
                    egui::RichText::new(err)
                        .small()
                        .monospace()
                        .color(theme::TEXT_SECONDARY),
                );
                ui.add_space(theme::SPACING_DEFAULT);
                ui.label(
                    egui::RichText::new(
                        "Switch to the Text tab to fix the document; \
                         the visual editor will reload automatically.",
                    )
                    .small()
                    .color(theme::TEXT_DIM),
                );
                return;
            }
            let Some(parsed) = meta.parsed.as_mut() else {
                ui.colored_label(
                    theme::TEXT_SECONDARY,
                    "No parsed instance — start typing XML in the Text tab.",
                );
                return;
            };
            // The schema knows what a field *should* be, so it is asked
            // first; failing that, the decoded instance says what each field
            // *is*. Without the fallback the visual editor is inert for every
            // binary document on a build with no snapshot.
            let resolved = library.resolve(&parsed.instance.type_name);
            let described = resolved.is_some();
            let fields: Vec<or_io::meta::Field> = match resolved {
                Some(resolved) => resolved.fields.iter().map(|f| (*f).clone()).collect(),
                None => derived::fields_of(&parsed.instance),
            };

            if fields.is_empty() {
                ui.colored_label(
                    theme::TEXT_SECONDARY,
                    format!(
                        "`{}` decoded to no fields — switch to the Text tab to view the raw document.",
                        parsed.instance.type_name
                    ),
                );
                return;
            }

            if !described {
                let named = meta_is_gta_vi && or_io::meta::has_names_library();
                ui.label(
                    egui::RichText::new(crate::i18n::tr(undescribed_note(named)))
                        .small()
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(theme::SPACING_SNUG);
            }

            widgets::draw_struct(ui, &ctx, &fields, &mut parsed.instance, &mut dirty);
        });

    // ⚠ The form edits in place, so a read-only document takes back what it changed this frame.
    if let Some(why) = read_only.filter(|_| dirty) {
        let mut edits = ctx.edits.take();
        if let Some(parsed) = meta.parsed.as_mut() {
            crate::state::meta_edit::flip_all(&mut edits, &mut parsed.instance);
        }
        state.show_warning_toast(crate::i18n::tr(why));
        return;
    }
    if dirty {
        meta.after_visual_edit_in_place();
        let edits = ctx.edits.take();
        match edits.first().map(crate::state::MetaEdit::label) {
            Some(label) => state.record(doc_id, label, Change::Meta(edits)),
            None => doc.mark_edited(),
        }
    }
    if read_only.is_none() {
        presets::apply(state, doc_id, ctx.records.take());
    }
}

#[cfg(test)]
mod tests {
    use crate::state::MetaDocument;

    const WHAT: &str = "a_binary_document_opens_with_the_schemas_field_names";

    #[test]
    fn a_named_gta_vi_body_is_not_told_its_names_are_the_public_tables() {
        let named = super::undescribed_note(true);
        assert!(named.contains("GTA VI's name snapshot"), "{named}");
        assert!(!named.contains("public tables"), "{named}");
        assert!(super::undescribed_note(false).contains("public tables"));
    }

    /// ⚠ Nothing is compiled in, so a binary document's field names are the
    /// installed schema's or they are hashes: the editor has to install the
    /// snapshot a suite can see.
    #[test]
    fn a_binary_document_opens_with_the_schemas_field_names() {
        if or_test_support::schema_snapshot().is_none() {
            or_test_support::skip_schema(WHAT);
            return;
        }
        let library = crate::state::meta_schema();
        assert!(
            library.struct_("CAmbientModelSet").is_some(),
            "a snapshot is staged, so the editor must have installed it"
        );
        let xml = br#"<CAmbientModelSet>
  <Name>example_models</Name>
  <Models/>
</CAmbientModelSet>"#;
        let pso =
            or_io::meta::canonical::import(library, xml, or_io::meta::BinaryFormat::Pso, None)
                .expect("the XML imports as PSO");

        let doc = MetaDocument::from_bytes(pso);
        let parsed = doc.parsed.expect("the PSO document parses");
        assert_eq!(parsed.instance.type_name, "CAmbientModelSet");
        let names: Vec<&str> = parsed
            .instance
            .fields
            .iter()
            .map(|(n, _)| n.as_str())
            .collect();
        assert!(names.contains(&"Models"), "got {names:?}");
        assert!(library.resolve(&parsed.instance.type_name).is_some());
    }
}
