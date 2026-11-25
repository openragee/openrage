use super::*;

pub(super) fn draw_rpf(
    ui: &mut egui::Ui,
    state: &AppState,
    doc_id: crate::state::DocumentId,
    selection: Option<u64>,
) {
    let Some((noun, rpf)) = state.document(doc_id).and_then(|d| match &d.payload {
        DocPayload::Rpf(r) => Some((d.kind.info().noun, r)),
        _ => None,
    }) else {
        return;
    };
    let session = &rpf.session;

    ui.heading(noun);
    ui.separator();

    grid(ui, "rpf_archive", |ui| {
        row(ui, "Files", &session.file_count().to_string());
        row(
            ui,
            "Folders",
            &session.directory_count().saturating_sub(1).to_string(),
        );
        let dirty = state.document(doc_id).is_some_and(|d| d.dirty);
        row(ui, "Unsaved changes", if dirty { "yes" } else { "no" });
    });

    let node_id = selection
        .filter(|&s| s != DOC_ROOT_SEL)
        .filter(|&s| s != u64::MAX);

    let Some(node_id) = node_id else {
        return;
    };
    let Some(node) = session.node(node_id) else {
        return;
    };

    ui.add_space(crate::ui::theme::SPACING_DEFAULT);
    let unopened = matches!(
        node.backing,
        crate::state::RpfNodeBacking::Archive { scanned: false, .. }
            | crate::state::RpfNodeBacking::DiskFolder { scanned: false, .. }
    );
    // What the file is, in the app's words; how an archive stores it is for Show internals.
    let kind_label = match (&node.backing, node.kind) {
        (crate::state::RpfNodeBacking::Archive { .. }, _) => "Archive",
        (_, crate::state::RpfSessionNodeKind::Directory) => "Folder",
        _ => DocumentKind::from_path(std::path::Path::new(&node.name))
            .map_or("File", |kind| kind.info().noun),
    };
    ui.label(
        egui::RichText::new(kind_label)
            .strong()
            .color(theme::TEXT_PRIMARY),
    );
    ui.separator();

    let internals = shows_internals(state);
    grid(ui, "rpf_entry", |ui| {
        row(ui, "Name", &node.name);
        let path = session.path_of(node_id).unwrap_or_default();
        row(ui, "Path", &format!("/{}", path));
        if let Some(size) = session.archive_size(node_id) {
            row(ui, "Size", &or_job::human_bytes(size));
        }
        if unopened {
            row(ui, "Holds", "not read until it is opened");
            return;
        }
        if node.is_dir() {
            row(ui, "Holds", &plural(node.children.len(), "item"));
            return;
        }
        let size = session.size_for(node_id).unwrap_or(0);
        row(ui, "Size", &or_job::human_bytes(size));
        if internals
            && matches!(
                node.backing,
                crate::state::RpfNodeBacking::ArchiveFile { .. }
            )
        {
            let stored = match node.kind {
                crate::state::RpfSessionNodeKind::ResourceFile => "A resource",
                _ => "Plain bytes",
            };
            row(ui, "Stored as", stored);
        }
    });
    super::rpf9_entry::draw(ui, session, &node.backing);
}
