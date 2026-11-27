//! The Statistics panel counts the app's other long-lived CPU state on a line each: the name
//! index, the file index, search results, an archive tree and its tables of contents, the undo
//! histories, the simulation and what Uses and Placed by read.
//!
//! ⚠ Each test grows one structure through the app's own API and reads the figure through the
//! panel's `readings`, so a structure left out of the sum fails here rather than in a profiler.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::RpfArchive;
use or_io::decl::{Gen8, Heap};
use or_io::gta5::particle::PtxEventEmitter;
use or_io::gta5::PtxFxList;
use or_io::names::HashFlavour;
use or_particle::keyframe::constant;
use or_particle::list;
use or_simulation::EmitterInst;

use super::document::{ResourceImage, TextDocument};
use super::memory::{vec_bytes, CpuState};
use super::name_index::NameIndexCategory;
use super::nested_archives_survive_a_save::archive;
use super::opening_a_ydd::{finish_load, node_at};
use super::rpf_session::RpfEditSession;
use super::sim::SerialisableTarget;
use super::uses_and_used_by_in_the_app::{cold, install};
use super::{AppState, ArchiveSource, Change, DocPayload, Document, RpfDocument, RpfNodeBacking};

/// What the panel reads for the app's other CPU state.
fn panel(state: &AppState) -> CpuState {
    crate::ui::panels::statistics::readings(state, &egui::Context::default()).cpu
}

/// `bytes` written to `dir/name` and opened as an archive document, as File ▸ Open does.
fn open_archive(state: &mut AppState, dir: &Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("the archive is written");
    state.load_file(path);
    finish_load(state).expect("the archive opens as a document");
}

/// An archive of `n` drawables, each in a folder of its own.
fn archive_of(n: usize) -> Vec<u8> {
    let names: Vec<String> = (0..n)
        .map(|i| format!("props/set_{i:04}/prop.ydr"))
        .collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    archive(&entries)
}

#[test]
fn the_name_index_is_counted_as_its_names_and_its_snapshot_grow() {
    let mut state = AppState::new();
    let empty = panel(&state).name_index;
    for i in 0..1_000 {
        let name = format!("prop_bench_{i:04}");
        state.name_index.add(NameIndexCategory::Model, name);
    }
    let named = panel(&state).name_index;
    assert!(
        named >= empty + 1_000 * "prop_bench_0000".len(),
        "1,000 names: {empty} -> {named} bytes"
    );
    state.name_index.snapshot();
    assert!(
        panel(&state).name_index > named,
        "the sorted snapshot the pickers read is a second copy of every name"
    );
}

#[test]
fn the_hash_index_over_the_names_is_counted_until_another_title_discards_it() {
    let mut state = AppState::new();
    for i in 0..2_000 {
        let name = format!("prop_bench_{i:04}");
        state.name_index.add(NameIndexCategory::Model, name);
    }
    let indexed = panel(&state).name_index;
    state.name_index.retarget("another title");
    let sets = panel(&state).name_index;
    let entry = std::mem::size_of::<((HashFlavour, u32), String)>() + "prop_bench_0000".len();
    assert!(
        indexed >= sets + 2 * 2_000 * entry,
        "each name under both hash functions, a slot and a copy of it for each: {sets} -> {indexed} bytes"
    );
}

#[test]
fn the_file_index_is_counted_as_its_listing_grows() {
    let mut state = AppState::new();
    state.file_index.walk_with("install".into(), |_| {});
    let empty = panel(&state).file_index;
    let path = |i: usize| format!("x64a.rpf/textures/file_{i:04}.ytd");
    let files = (0..1_000).map(|i| path(i).into_boxed_str());
    Arc::make_mut(&mut state.file_index.roots[0].listing)
        .files
        .extend(files);
    let listed = panel(&state).file_index;
    assert!(
        listed >= empty + 1_000 * path(0).len(),
        "1,000 paths: {empty} -> {listed} bytes"
    );
}

#[test]
fn an_archives_tree_and_tables_of_contents_are_counted_once_it_is_open() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let mut state = AppState::new();
    let none = panel(&state);
    assert_eq!((none.archive_tree, none.archive_tables), (0, 0));

    open_archive(&mut state, dir.path(), "small.rpf", &archive_of(2));
    let small = panel(&state);
    assert!(small.archive_tree > 0, "the rows of an open archive");
    assert!(
        small.archive_tables > 0,
        "the table of contents of an open archive"
    );

    open_archive(&mut state, dir.path(), "large.rpf", &archive_of(300));
    let large = panel(&state);
    assert!(
        large.archive_tables >= small.archive_tables + 300 * 16,
        "300 entries: {} -> {} bytes of tables",
        small.archive_tables,
        large.archive_tables
    );
    assert!(
        large.archive_tree >= small.archive_tree + 300 * 16,
        "300 rows: {} -> {} bytes of tree",
        small.archive_tree,
        large.archive_tree
    );
}

#[test]
fn an_archive_shared_by_two_slots_is_counted_once() {
    let bytes = archive_of(50);
    let shared = Arc::new(RpfArchive::from_bytes(bytes.clone()).expect("the archive opens"));
    let mut session = RpfEditSession::empty();
    session
        .register_archive(Arc::clone(&shared), None)
        .expect("registers");
    let once = session.table_bytes(&mut Default::default());
    session
        .register_archive(Arc::clone(&shared), None)
        .expect("registers");
    assert_eq!(
        session.table_bytes(&mut Default::default()),
        once,
        "the same archive under two slots"
    );

    let mut other = RpfEditSession::empty();
    other
        .register_archive(Arc::clone(&shared), None)
        .expect("registers");
    let mut seen = Default::default();
    assert_eq!(session.table_bytes(&mut seen), once);
    assert_eq!(
        other.table_bytes(&mut seen),
        0,
        "a second tree holding it adds nothing"
    );
    let copy = Arc::new(RpfArchive::from_bytes(bytes).expect("a second copy"));
    other.register_archive(copy, None).expect("registers");
    assert!(
        other.table_bytes(&mut seen) > 0,
        "a copy is a table of its own"
    );
}

/// A session over `outer.rpf`, which holds `mid.rpf` of `n` drawables: the outer archive its
/// slot holds and the middle one its row holds opened, not yet expanded.
fn nested_session(n: usize) -> (RpfEditSession, Arc<RpfArchive>, Arc<RpfArchive>) {
    let mid = archive_of(n);
    let outer = archive(&[("top.ydr", b"x"), ("mid.rpf", &mid)]);
    let opened = RpfArchive::from_bytes(outer).expect("the outer archive opens");
    let mut session = RpfEditSession::from_archive(opened).expect("a session");
    let Some(RpfNodeBacking::ArchiveDir { archive_id }) =
        session.node(session.root_id()).map(|root| &root.backing)
    else {
        unreachable!("the root of an archive's tree is the archive");
    };
    let outer = Arc::clone(&session.archive(*archive_id).expect("its slot").archive);
    let row = (session.all_nodes().into_iter())
        .find(|&id| session.node(id).is_some_and(|n| n.name == "mid.rpf"))
        .expect("the nested archive's row");
    let Some(RpfNodeBacking::Archive {
        source:
            ArchiveSource::Nested {
                parent_entry_index,
                opened,
                ..
            },
        ..
    }) = session.node_mut(row).map(|n| &mut n.backing)
    else {
        unreachable!("a nested archive's row");
    };
    let device = or_title::vfs::ArchiveDevice::new(Arc::clone(&outer));
    let mid = device.open_nested(*parent_entry_index).expect("mid opens");
    *opened = Some(Arc::clone(&mid));
    (session, outer, mid)
}

#[test]
fn a_nested_archive_counts_from_the_listing_of_its_parent() {
    let (session, outer, mid) = nested_session(300);
    assert!(
        mid.table_bytes() > 300 * 16,
        "the nested archive lists 300 files"
    );
    assert_eq!(
        session.table_bytes(&mut Default::default()),
        outer.table_bytes() + mid.table_bytes(),
        "the archive in a slot and the one its listing opened, each once"
    );
}

#[test]
fn a_nested_archive_keeps_its_parents_table_counted_once() {
    let (session, outer, mid) = nested_session(50);
    let mut alone = RpfEditSession::empty();
    alone
        .register_archive(Arc::clone(&mid), None)
        .expect("registers");
    assert_eq!(
        alone.table_bytes(&mut Default::default()),
        mid.table_bytes() + outer.table_bytes(),
        "a nested archive's parent is a table the tree holds alive though no slot has it"
    );

    let mut seen = Default::default();
    session.table_bytes(&mut seen);
    assert_eq!(
        alone.table_bytes(&mut seen),
        0,
        "both archives counted by the first tree"
    );
}

#[test]
fn the_session_byte_cache_is_counted_with_its_archive_document() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    // ⚠ Written first: opening the folder lists it on a worker that may run before a later write.
    std::fs::write(dir.path().join("big.bin"), vec![7u8; 64 * 1024]).expect("a file to read");
    let mut state = AppState::new();
    state.open_folder(dir.path().to_path_buf());
    let tree = state.active_document.expect("the folder's tree");
    let before = state.document(tree).expect("the tree").held_bytes();

    let row = node_at(&mut state, tree, "big.bin");
    let DocPayload::Rpf(rpf) = &state.document(tree).expect("the tree").payload else {
        unreachable!("a folder's tree is an archive document");
    };
    let bytes = rpf.session.bytes_for(row).expect("the file reads");
    assert_eq!(bytes.len(), 64 * 1024);
    assert!(
        state.document(tree).expect("the tree").held_bytes() >= before + 64 * 1024,
        "a row's bytes read once stay in the session until it evicts them"
    );
}

#[test]
fn the_undo_history_is_counted_with_the_bytes_and_images_its_steps_keep() {
    let image = Arc::new(ResourceImage {
        data: vec![0u8; 1 << 20],
        virtual_size: 32,
        seal: {
            use or_io::container::resource::{ResourceEnvelope, Rsc7};
            Rsc7::peek(&Rsc7::bare_header(165)).expect("a bare header parses")
        },
    });
    let doc = Document::new(
        1,
        Some("probe.txt".into()),
        DocPayload::Text(TextDocument::decode(b"x")),
    )
    .with_resource(Some(image));
    let mut state = AppState::new();
    state.documents.push(doc);
    let untouched = panel(&state).undo_history;

    let doc = &mut state.documents[0];
    doc.record(
        "patch".into(),
        Change::Bytes {
            at: 0,
            other: vec![0u8; 4096],
        },
    );
    let patched = panel(&state).undo_history;
    assert!(
        patched >= untouched + 4096,
        "a step holding 4 KiB of the image's bytes: {untouched} -> {patched}"
    );

    let held = state.documents[0].held_bytes();
    state.documents[0].record(
        "rebuild".into(),
        Change::Part {
            key: None,
            parts: Vec::new(),
        },
    );
    assert!(
        state.documents[0].held_bytes() + (1 << 20) <= held + 64,
        "an edit of the contents hands the image to the step"
    );
    assert!(
        panel(&state).undo_history >= patched + (1 << 20),
        "the step keeps the 1 MiB image the document let go of"
    );
}

#[test]
fn the_rows_a_delete_took_out_are_counted_with_their_bytes_while_the_step_keeps_them() {
    let mut session = RpfEditSession::empty();
    let root = session.root_id();
    let row = session
        .add_file_copy(root, "texture.ytd", vec![5; 64 * 1024])
        .expect("a row");
    let rows = session.take_rows(row).expect("the row comes out");
    let mut doc = Document::new(
        1,
        None,
        DocPayload::Rpf(RpfDocument {
            session,
            expanded: Default::default(),
            focused_node: None,
            is_game_root: false,
            entry_documents: Default::default(),
        }),
    );
    let before = doc.history_bytes();
    doc.record(
        "Delete texture.ytd".into(),
        super::rpf_undo::deleted(row, rows),
    );
    assert!(
        doc.history_bytes() >= before + 64 * 1024,
        "the 64 KiB file a deleted row holds: {before} -> {} bytes",
        doc.history_bytes()
    );
}

#[test]
fn a_texture_a_step_swaps_back_is_counted_with_its_pixels() {
    let texture = or_io::gta5::gen8::GrcTexture {
        backing_store: vec![0; 64 * 1024],
        ..Default::default()
    };
    let payload = DocPayload::Text(TextDocument::decode(b"x"));
    let mut doc = Document::new(1, Some("paint.ytd".into()), payload);
    let before = doc.history_bytes();
    doc.record(
        "Replace texture".into(),
        Change::part("texture", texture, |_| None),
    );
    assert!(
        doc.history_bytes() >= before + 64 * 1024,
        "the 64 KiB of pixels the step swaps back: {before} -> {} bytes",
        doc.history_bytes()
    );
}

/// One effect of `events` events, each spawning `rate` particles a second that live a second.
fn fx_of(events: usize, rate: f32) -> PtxFxList {
    let mut asset = PtxFxList::default();
    list::add_effect(&mut asset, "fx_probe");
    list::add_emitter_rule(&mut asset, "em_probe");
    list::add_particle_rule(&mut asset, "pt_probe");
    for rule in list::emitter_rules_mut(&mut asset) {
        rule.spawn_rate_over_time_kfp = constant([rate, 0.0, 0.0, 0.0]);
        rule.particle_life_kfp = constant([1.0, 0.0, 0.0, 0.0]);
    }
    let effect = list::effect_rules_mut(&mut asset)
        .next()
        .expect("the list holds its effect");
    (effect.duration_min, effect.duration_max) = (600.0, 600.0);
    for i in 0..events {
        let mut event = PtxEventEmitter::<Gen8>::default();
        event.base.index = i as i32;
        event.base.end_ratio = 1.0;
        event.emitter_rule_name = "em_probe".into();
        event.particle_rule_name = "pt_probe".into();
        (
            event.playback_rate_scalar_min,
            event.playback_rate_scalar_max,
        ) = (1.0, 1.0);
        (event.zoom_scalar_min, event.zoom_scalar_max) = (1.0, 1.0);
        (event.colour_tint_min, event.colour_tint_max) = (0xFFFF_FFFF, 0xFFFF_FFFF);
        effect.timeline.events.push(Some(Box::new(event)));
    }
    asset
}

/// A state whose simulation has built `asset`'s effect and played it for `seconds`, taking no
/// snapshot on the way.
fn played(asset: &PtxFxList, seconds: usize) -> AppState {
    let mut state = AppState::new();
    state.sim.rebuild(1, asset, SerialisableTarget::Effect(0));
    state.sim.manager.snapshot_interval = f32::MAX;
    for _ in 0..seconds * 60 {
        state.sim.manager.tick(1.0 / 60.0);
    }
    state
}

/// What the simulation holds in the columns of every live emitter's particles.
fn live_positions(state: &AppState) -> usize {
    let effect = state.sim.manager.effect.as_ref().expect("an effect plays");
    (effect.emitters.iter())
        .map(|emitter| vec_bytes(&emitter.points.curr_pos) + vec_bytes(&emitter.points.curr_vel))
        .sum()
}

#[test]
fn the_simulation_counts_each_emitter_the_effect_is_built_with() {
    let idle = panel(&AppState::new()).simulation;
    let one = panel(&played(&fx_of(1, 50.0), 0)).simulation;
    let seven = panel(&played(&fx_of(7, 50.0), 0)).simulation;
    assert!(
        one > idle,
        "the effect and the rules it runs from: {idle} -> {one}"
    );
    assert!(
        seven >= one + 6 * std::mem::size_of::<EmitterInst>(),
        "six more emitters: {one} -> {seven} bytes"
    );
}

#[test]
fn the_simulation_counts_the_rules_it_runs_from() {
    let small = fx_of(1, 50.0);
    let mut large = small.clone();
    for i in 0..300 {
        list::add_particle_rule(&mut large, &format!("unused_rule_{i:03}"));
    }
    let (grown, base) = (large.heap_bytes(), small.heap_bytes());
    let (small_sim, large_sim) = (
        panel(&played(&small, 0)).simulation,
        panel(&played(&large, 0)).simulation,
    );
    assert!(
        large_sim >= small_sim + (grown - base),
        "300 rules the effect does not play: {small_sim} -> {large_sim} bytes against {base} -> {grown} of heap"
    );
}

#[test]
fn the_simulation_counts_the_particles_its_emitters_spawn() {
    let mut state = played(&fx_of(2, 2_000.0), 0);
    let idle = panel(&state).simulation;
    for _ in 0..60 {
        state.sim.manager.tick(1.0 / 60.0);
    }
    let live = live_positions(&state);
    assert!(
        live > 2 * 1_000 * 12,
        "two emitters at 2,000 a second: {live} bytes"
    );
    assert!(
        panel(&state).simulation >= idle + live,
        "the columns of the particles alive: {idle} -> {} bytes against {live}",
        panel(&state).simulation
    );
}

#[test]
fn the_simulation_counts_each_snapshot_a_seek_can_replay_from() {
    let mut state = played(&fx_of(2, 2_000.0), 1);
    let before = panel(&state).simulation;
    for _ in 0..20 {
        state.sim.manager.capture_snapshot();
    }
    let kept: usize = (state.sim.manager.snapshots.iter())
        .flat_map(|frame| &frame.emitter_states)
        .map(|emitter| vec_bytes(&emitter.points.curr_pos))
        .sum();
    assert!(
        kept > 20 * 2 * 1_000 * 12,
        "20 snapshots of two emitters: {kept} bytes"
    );
    assert!(
        panel(&state).simulation >= before + kept,
        "each snapshot keeps every emitter's particles: {before} -> {} bytes against {kept}",
        panel(&state).simulation
    );
}

#[test]
fn search_results_are_counted_as_hits_and_matches_arrive() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let mut state = AppState::new();
    let empty = panel(&state).search_results;

    let hit = or_title::install::content::ContentHit {
        path: "x64a.rpf/data/a_long_path_to_a_hit.dat".repeat(4),
        container: "x64a.rpf".into(),
        offsets: vec![0; 32],
        file_size: 10,
        preview: Some("a line of context".into()),
    };
    state.search.hits.extend(std::iter::repeat_n(hit, 100));
    let searched = panel(&state).search_results;
    assert!(
        searched > empty + 100 * 100,
        "100 hits: {empty} -> {searched}"
    );

    open_archive(&mut state, dir.path(), "found.rpf", &archive_of(200));
    let doc = state.active_document.expect("the archive's document");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        state.deep_find_tick(doc, "**/*.ydr");
        let found = state.deep_find.found_for(doc, "**/*.ydr");
        if found.is_some_and(|found| !found.indexing && found.matches().len() == 200) {
            break;
        }
        assert!(Instant::now() < deadline, "the find never came back");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        panel(&state).search_results > searched + 200 * "doc:/props/set_0000/prop.ydr".len(),
        "200 matches of the find through the archive"
    );
}

#[test]
fn what_uses_and_placed_by_read_is_counted_once_the_install_is_read() {
    let dir = install();
    let (mut state, tree) = cold(dir.path());
    assert_eq!(panel(&state).dependency_index, 0);

    let bench = "x64e.rpf/levels/gta5/props.rpf/prop_bench.ydr";
    let row = node_at(&mut state, tree, bench);
    let deadline = Instant::now() + Duration::from_secs(120);
    let answer = |state: &mut AppState, placing: bool| loop {
        state.poll_jobs();
        let answer = match placing {
            true => state.placed_by(tree, row),
            false => state.uses_of(tree, row),
        };
        if !matches!(answer, super::dependencies::Answer::Looking) {
            return;
        }
        assert!(Instant::now() < deadline, "the install was never read");
        std::thread::sleep(Duration::from_millis(5));
    };
    answer(&mut state, false);
    let uses = panel(&state).dependency_index;
    let mount = state.dependencies.mount_bytes();
    assert!(
        uses > mount,
        "the catalogue beside the mount: {uses} bytes against the mount's {mount}"
    );

    answer(&mut state, true);
    assert!(
        panel(&state).dependency_index > uses,
        "the maps Placed by folds in are counted beside them"
    );
}

#[test]
fn the_panel_reads_the_same_figures_the_state_counts() {
    let mut state = AppState::new();
    state
        .name_index
        .add(NameIndexCategory::Texture, "a_texture");
    let ctx = egui::Context::default();
    assert_eq!(
        crate::ui::panels::statistics::readings(&state, &ctx).cpu,
        state.cpu_state()
    );
}
