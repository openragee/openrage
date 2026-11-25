use super::*;

fn copied_paths(state: &mut AppState, doc: DocumentId, rows: &[RpfNodeId], centre: bool) -> String {
    let target = crate::ui::commands::Target::Rows {
        doc,
        rows: rows.to_vec(),
        into: session(state, doc).root_id(),
        centre,
    };
    let command = crate::ui::commands::on(state, crate::ui::commands::Action::CopyPath, target)
        .expect("the Files rows offer Copy path");
    assert!(command.enabled.is_ok());
    let output = egui::Context::default().run_ui(Default::default(), |ui| {
        crate::ui::commands::run(state, ui.ctx(), &command);
    });
    output
        .platform_output
        .commands
        .into_iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .expect("Copy path writes to the clipboard")
}

#[test]
fn rpf9_copy_paths_include_each_display_extension_and_still_reach_the_same_rows() {
    for (child_name, file_name) in [
        (None, None),
        (Some("packs/child"), Some("models/surface")),
        (Some("packs/child.RPF"), Some("models/surface.YTD")),
    ] {
        let fixture = Fixture::new(false);
        let base = fixture.table.expand_table(0, 0).unwrap();
        let file_hash = {
            let mut tree = fixture.table.tree();
            tree.nodes.get_mut(&0).unwrap().mount.archive.entries[0].name =
                child_name.map(str::to_string);
            let entry = &mut tree.nodes.get_mut(&base).unwrap().mount.archive.entries[0];
            entry.name = file_name.map(str::to_string);
            entry.name64
        };
        let (mut state, doc, archive) = folder(&fixture, false);
        state.request_expand(doc, archive);
        settle(&mut state, doc);
        let child = session(&state, doc)
            .all_nodes()
            .into_iter()
            .find(|&id| {
                session(&state, doc).node(id).unwrap().rpf9_entry()
                    == Some((0, name_hash64(CHILD_NAME)))
            })
            .unwrap();
        state.request_expand(doc, child);
        settle(&mut state, doc);
        let file = session(&state, doc)
            .all_nodes()
            .into_iter()
            .find(|&id| {
                matches!(
                    &session(&state, doc).node(id).unwrap().backing,
                    RpfNodeBacking::Rpf9File { base: at, index: 0, .. } if *at == base
                )
            })
            .unwrap();
        let (child_path, file_path) = match (child_name, file_name) {
            (None, None) => (
                format!("hash_{:016x}.rpf", name_hash64(CHILD_NAME)),
                format!("hash_{file_hash:016x}.ytd"),
            ),
            (Some("packs/child"), Some("models/surface")) => {
                ("packs/child.rpf".into(), "models/surface.ytd".into())
            }
            (Some(child), Some(file)) => (child.to_string(), file.to_string()),
            _ => unreachable!(),
        };
        let expected = [
            format!("/textures.rpf/{child_path}/{file_path}"),
            format!("/textures.rpf/{child_path}"),
            "/textures.rpf".to_string(),
        ];
        let rows = [file, child, archive];
        let before = rows.map(|id| {
            let tree = session(&state, doc);
            let node = tree.node(id).unwrap();
            (
                node.name.clone(),
                node.rpf9_entry(),
                tree.path_of(id).unwrap(),
            )
        });
        for centre in [false, true] {
            let copied = copied_paths(&mut state, doc, &rows, centre);
            assert_eq!(copied, expected.join("\n"));
            for (path, &id) in copied.lines().zip(&rows) {
                state.go_to_address(doc, path).unwrap();
                assert_eq!(
                    state.navigation.current(),
                    Some(crate::state::navigation::Location { doc, node: id })
                );
            }
        }
        for (&id, (name, entry, raw_path)) in rows.iter().zip(before) {
            let node = session(&state, doc).node(id).unwrap();
            assert_eq!(node.name, name);
            assert_eq!(node.rpf9_entry(), entry);
            assert_eq!(session(&state, doc).path_of(id), Some(raw_path.clone()));
            assert_eq!(
                crate::state::navigation::resolve_path(session(&state, doc), &raw_path),
                crate::state::navigation::Resolved::Found(id)
            );
        }
        fixture.unchanged();
    }
}

#[test]
fn rpf9_copy_paths_with_visual_suffixes_resolve_case_insensitively() {
    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let file = session(&state, doc).children_of(child)[0];
    let file_hash = session(&state, doc)
        .node(file)
        .unwrap()
        .rpf9_entry()
        .unwrap()
        .1;
    let path = format!(
        "/textures.rpf/hash_{:016x}.rpf/hash_{file_hash:016x}.ytd",
        name_hash64(CHILD_NAME)
    );
    state
        .go_to_address(doc, &path.to_ascii_uppercase())
        .unwrap();
    assert_eq!(
        state.navigation.current(),
        Some(crate::state::navigation::Location { doc, node: file })
    );
    fixture.unchanged();
}
