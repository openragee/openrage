//! Creating and deleting world data on every map and manifest a retail
//! install ships, carried out end to end and read back from the bytes.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//! `OPENRAGE_EXEC_STRIDE=N` takes every Nth map.
//!
//! ⚠ **An entity delete is judged by identity, not by count.** Every LOD link
//! in the map and its child maps is recorded as *which entity names which* —
//! archetype, position and `guid` — before the delete, and has to be the same
//! relation afterwards, minus only the removed entity's own link. A renumber
//! that was off by one anywhere keeps every file valid and fails here.
//!
//! The set holding a map and its child maps is loaded, edited, encoded, read
//! back and dropped before the next, so one delete never sees another's
//! result and memory is one set at a time.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::{parse_meta_binary_full, MetaLayout};
use or_io::meta::{Diagnostics, MetaInstance};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_world::execute::{identity, links_by_identity, ExecError, Key, WorkingSet};
use or_world::map_data::MapData;
use or_world::{ArchetypeIndex, Manifest, MapTypes};

const WHAT: &str = "executing_a_plan_keeps_every_link";

#[derive(Default)]
struct Report {
    label: String,
    stride: usize,
    maps: u64,
    all_parents: u64,
    attempted: u64,
    subtrees: u64,
    deleted: u64,
    renumbering: u64,
    leaf_moved: u64,
    refused: BTreeMap<String, u64>,
    links_kept: u64,
    bytes_kept: u64,
    write_refused: BTreeMap<String, u64>,
    refreshed_maps: u64,
    extents_recomputed: u64,
    extents_kept: u64,
    local: u64,
    cross_file: u64,
    failures: Vec<String>,
    manifests: u64,
    create_tried: u64,
    created: u64,
    delete_tried: u64,
    map_deleted: u64,
}

impl Report {
    fn text(&self) -> String {
        format!(
            "{} (every {} map)\n  entity delete: {} maps with entities, {} with every entity a LOD \
             parent, {} attempted ({} a parent with its children), {} carried out, refused {:?}\n    links named the same entities \
             after the edit on {}, and after the bytes were read back on {}; the writer refused \
             {:?}\n    {} local and {} \
             cross-file indices renumbered, over {} deletes that moved any ({} of them a leaf's)\n    {} changed maps \
             had flags and extents recomputed: extents rewritten on {}, kept as a superset on {} \
             that name an archetype no type file declares\n  \
             map create: {} of {} manifests stated a clone read back from the bytes\n  \
             map delete: {} of {} manifests stopped stating a deleted map\n  failures: {}\n    {}",
            self.label,
            self.stride,
            self.maps,
            self.all_parents,
            self.attempted,
            self.subtrees,
            self.deleted,
            self.refused,
            self.links_kept,
            self.bytes_kept,
            self.write_refused,
            self.local,
            self.cross_file,
            self.renumbering,
            self.leaf_moved,
            self.refreshed_maps,
            self.extents_recomputed,
            self.extents_kept,
            self.created,
            self.create_tried,
            self.map_deleted,
            self.delete_tried,
            self.failures.len(),
            self.failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n    "),
        )
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn read(game: &or_title::MountedGame, path: &str) -> Option<(MetaInstance, MetaLayout)> {
    let bytes = game.vfs.open(path)?;
    let mut diags = Diagnostics::lenient();
    parse_meta_binary_full(support::schema(), &bytes, &mut diags).ok()
}

fn reread(bytes: &[u8]) -> Option<MetaInstance> {
    let mut diags = Diagnostics::lenient();
    parse_meta_binary_full(support::schema(), bytes, &mut diags)
        .ok()
        .map(|(root, _)| root)
}

/// What the first pass keeps of a map: a few numbers, never the tree.
#[derive(Default, Clone)]
struct Facts {
    paths: Vec<String>,
    parent: u32,
    entities: usize,
    cross_file: bool,
}

fn measure(root: &Path, stride: usize) -> Option<Report> {
    let provider = crypto_for(root)?;
    let install = or_title::GtaInstall::from_path(root)?;
    let game = or_title::build_mounted_game(&install, provider, or_title::DlcCutoff::All).ok()?;

    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    let mut type_files: BTreeMap<String, String> = BTreeMap::new();
    let mut manifest_paths = Vec::new();
    game.for_each_file(|logical, _container, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ymap") {
            winners.insert(lower, logical.to_string());
        } else if lower.ends_with(".ytyp") {
            type_files.insert(lower, logical.to_string());
        } else if lower.ends_with(".ymf") {
            manifest_paths.push(logical.to_string());
        }
    });
    let mut index = ArchetypeIndex::new();
    for path in type_files.values() {
        if let Some((tree, _)) = read(&game, path) {
            if let Some(types) = MapTypes::new(&tree) {
                index.add(types);
            }
        }
    }

    let mut facts: BTreeMap<u32, Facts> = BTreeMap::new();
    for path in winners.values() {
        let Some((tree, _)) = read(&game, path) else {
            continue;
        };
        let Some(map) = MapData::new(&tree) else {
            continue;
        };
        let slot = facts.entry(map.name()).or_default();
        slot.paths.push(path.clone());
        slot.parent = slot.parent.max(map.parent());
        slot.entities += map.entity_count();
        slot.cross_file |= map
            .entities()
            .any(|e| e.lod_in_parent_map() && e.parent_index().is_some());
    }
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (name, f) in &facts {
        if f.parent != 0 {
            children.entry(f.parent).or_default().push(*name);
        }
    }

    let mut report = Report {
        label: root.display().to_string(),
        stride,
        ..Report::default()
    };

    let with_entities: Vec<u32> = facts
        .iter()
        .filter(|(_, f)| f.entities > 0)
        .map(|(n, _)| *n)
        .collect();
    report.maps = with_entities.len() as u64;
    for name in with_entities.iter().step_by(stride) {
        delete_one_entity(&game, *name, &facts, &children, &index, &mut report);
    }

    // Which manifests state which maps, so a delete can reach every one.
    let mut stating: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for path in &manifest_paths {
        let Some((tree, _)) = read(&game, path) else {
            continue;
        };
        let Some(m) = Manifest::new(&tree) else {
            continue;
        };
        for (map, _) in m.imap_dependencies() {
            stating.entry(map).or_default().push(path.clone());
        }
        for (map, _) in m.flat_imap_dependencies() {
            stating.entry(map).or_default().push(path.clone());
        }
    }
    for list in stating.values_mut() {
        list.sort();
        list.dedup();
    }
    for path in &manifest_paths {
        let Some((tree, _)) = read(&game, path) else {
            continue;
        };
        let Some(m) = Manifest::new(&tree) else {
            continue;
        };
        let stated: Vec<(u32, BTreeSet<u32>)> = m.imap_dependencies().collect();
        if stated.iter().all(|(map, _)| !facts.contains_key(map)) {
            continue;
        }
        report.manifests += 1;
        create_one_map(&game, path, &stated, &facts, &mut report);
        delete_one_map(
            &game,
            path,
            &stated,
            &facts,
            &children,
            &stating,
            &mut report,
        );
    }
    Some(report)
}

fn delete_one_entity(
    game: &or_title::MountedGame,
    name: u32,
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
    index_of_types: &ArchetypeIndex,
    report: &mut Report,
) {
    // Grandchild maps too, so no descendant of a deleted entity is outside
    // the set and a link into it cannot go unchecked.
    let mut ws = WorkingSet::new();
    let mut members = vec![name];
    for child in children.get(&name).into_iter().flatten() {
        members.push(*child);
        members.extend(children.get(child).into_iter().flatten().copied());
    }
    for m in &members {
        for path in &facts[m].paths {
            if let Some((tree, layout)) = read(game, path) {
                ws.add(path.clone(), tree, layout);
            }
        }
    }

    // ⚠ A parent whose children are all leaves, where one exists: a map
    // lists its LOD parents ahead of its unnamed entities, so deleting a
    // leaf almost never moves anything and would test almost nothing.
    // Taking a parent with its children shifts the rest of the array.
    let group = ws.group();
    let len = facts[&name].entities;
    let subtree = (0..len).find(|i| {
        group.children_naming(name, *i as u32) > 0 && {
            let kids = ws.lod_children(name, *i);
            kids.len() <= 64
                && kids
                    .iter()
                    .all(|(m, k, _)| group.children_naming(*m, *k as u32) == 0)
        }
    });
    let leaf = (0..len).find(|i| group.children_naming(name, *i as u32) == 0);
    let Some(index) = subtree.or(leaf) else {
        report.all_parents += 1;
        return;
    };
    report.attempted += 1;

    let before = links_by_identity(ws.documents().map(|d| &d.root));
    let mut removed: BTreeSet<Key> = ws
        .lod_children(name, index)
        .into_iter()
        .map(|(_, _, key)| key)
        .collect();
    removed.extend(
        ws.documents()
            .filter_map(|d| MapData::new(&d.root))
            .filter(|m| m.name() == name)
            .find_map(|m| m.entities().nth(index).map(|e| identity(e.instance()))),
    );

    let outcome = if subtree.is_some() {
        report.subtrees += 1;
        ws.delete_subtree(name, index).map(|s| s.renumbered)
    } else {
        ws.delete_entity(name, index)
    };
    let moved = match outcome {
        Ok(moved) => moved,
        Err(e) => {
            let reason = match e {
                ExecError::ContainerLods(_) => "the map carries container LODs".to_string(),
                ExecError::AmbiguousParts(_) => "two parts of the map hold entities".to_string(),
                other => {
                    report.failures.push(format!("{name:#010X}: {other}"));
                    "other".to_string()
                }
            };
            *report.refused.entry(reason).or_default() += 1;
            return;
        }
    };
    report.deleted += 1;
    report.local += moved.local;
    report.cross_file += moved.cross_file;
    if moved.local + moved.cross_file > 0 {
        report.renumbering += 1;
        if subtree.is_none() {
            report.leaf_moved += 1;
        }
    }
    // A single delete is exactly what the plan predicts it moves.
    if subtree.is_none() {
        let predicted = group.indices_above(name, index as u32);
        if predicted != (moved.local, moved.cross_file) {
            report.failures.push(format!(
                "{name:#010X}: the plan predicted {predicted:?} moved indices and the edit moved \
                 ({}, {})",
                moved.local, moved.cross_file
            ));
        }
    }

    let mut expected = before;
    expected.retain(|child, _| !removed.contains(child));
    let after = links_by_identity(ws.documents().filter(|d| !d.is_removed()).map(|d| &d.root));
    if after == expected {
        report.links_kept += 1;
    } else {
        report.failures.push(format!(
            "{name:#010X}: {} links before the delete, {} after",
            expected.len(),
            after.len()
        ));
        return;
    }

    match ws.refresh_derived(index_of_types) {
        Ok(r) => {
            report.refreshed_maps += r.maps;
            report.extents_recomputed += r.extents_recomputed;
            report.extents_kept += r.extents_kept;
        }
        Err(e) => {
            report.failures.push(format!("{name:#010X}: refresh: {e}"));
            return;
        }
    }

    // A writer that cannot lay the edited document out refuses with its
    // own reason and writes nothing; that is counted apart from a failure.
    let written = match ws.finish() {
        Ok(w) => w,
        Err(ExecError::Write(e)) => {
            *report.write_refused.entry(e.to_string()).or_default() += 1;
            return;
        }
        Err(e) => {
            report.failures.push(format!("{name:#010X}: finish: {e}"));
            return;
        }
    };
    let mut roots: BTreeMap<String, MetaInstance> = ws
        .documents()
        .filter(|d| !d.is_removed())
        .map(|d| (d.label.clone(), d.root.clone()))
        .collect();
    for (label, bytes) in &written {
        match reread(bytes) {
            Some(back) => {
                roots.insert(label.clone(), back);
            }
            None => {
                report
                    .failures
                    .push(format!("{name:#010X}: {label} does not read back"));
                return;
            }
        }
    }
    if links_by_identity(roots.values()) == expected {
        report.bytes_kept += 1;
    } else {
        report
            .failures
            .push(format!("{name:#010X}: the links the bytes carry differ"));
    }
}

fn create_one_map(
    game: &or_title::MountedGame,
    manifest: &str,
    stated: &[(u32, BTreeSet<u32>)],
    facts: &BTreeMap<u32, Facts>,
    report: &mut Report,
) {
    // A donor with no cross-file links can stand as a map with no parent.
    let Some((donor, types)) = stated.iter().find(|(map, _)| {
        facts
            .get(map)
            .is_some_and(|f| !f.cross_file && f.paths.len() == 1)
    }) else {
        return;
    };
    report.create_tried += 1;
    let mut ws = WorkingSet::new();
    let Some((mtree, mlayout)) = read(game, manifest) else {
        return;
    };
    let stated_before = {
        let mut s = or_world::StatedDependencies::new();
        s.add(Manifest::new(&mtree).expect("a manifest"));
        s.len()
    };
    ws.add(manifest, mtree, mlayout);
    let Some((dtree, dlayout)) = read(game, &facts[donor].paths[0]) else {
        return;
    };
    let name = or_io::string_hash(format!("or_created_{donor:08x}").as_bytes());
    if let Err(e) = ws.create_map("created.ymap", dtree, dlayout, name, 0, types, manifest) {
        report
            .failures
            .push(format!("create from {donor:#010X}: {e}"));
        return;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            report
                .failures
                .push(format!("create from {donor:#010X}: finish: {e}"));
            return;
        }
    };
    let mut ok_manifest = false;
    let mut ok_map = false;
    for (label, bytes) in &written {
        let Some(back) = reread(bytes) else {
            continue;
        };
        if label == manifest {
            let mut s = or_world::StatedDependencies::new();
            if let Some(m) = Manifest::new(&back) {
                s.add(m);
            }
            ok_manifest = s.of(name) == Some(types) && s.len() == stated_before + 1;
        } else if let Some(m) = MapData::new(&back) {
            ok_map = m.name() == name && m.entity_count() == facts[donor].entities;
        }
    }
    if ok_manifest && ok_map {
        report.created += 1;
    } else {
        report.failures.push(format!(
            "create from {donor:#010X} in {manifest}: manifest {ok_manifest}, map {ok_map}"
        ));
    }
}

fn delete_one_map(
    game: &or_title::MountedGame,
    manifest: &str,
    stated: &[(u32, BTreeSet<u32>)],
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
    stating: &BTreeMap<u32, Vec<String>>,
    report: &mut Report,
) {
    let Some((doomed, _)) = stated
        .iter()
        .find(|(map, _)| facts.contains_key(map) && !children.contains_key(map))
    else {
        return;
    };
    report.delete_tried += 1;
    let mut ws = WorkingSet::new();
    let manifests = stating.get(doomed).cloned().unwrap_or_default();
    for path in &manifests {
        if let Some((t, l)) = read(game, path) {
            ws.add(path.clone(), t, l);
        }
    }
    for path in &facts[doomed].paths {
        if let Some((t, l)) = read(game, path) {
            ws.add(path.clone(), t, l);
        }
    }
    if let Err(e) = ws.delete_map(*doomed) {
        report
            .failures
            .push(format!("delete {doomed:#010X} stated in {manifest}: {e}"));
        return;
    }
    let written = match ws.finish() {
        Ok(w) => w,
        Err(e) => {
            report
                .failures
                .push(format!("delete {doomed:#010X}: finish: {e}"));
            return;
        }
    };
    let mut still = false;
    let mut read_back = 0;
    for (_, bytes) in &written {
        let Some(back) = reread(bytes) else {
            continue;
        };
        read_back += 1;
        if let Some(m) = Manifest::new(&back) {
            still |= m.imap_dependencies().any(|(map, _)| map == *doomed)
                || m.flat_imap_dependencies().any(|(map, _)| map == *doomed);
        }
    }
    if !still && read_back == written.len() && read_back == manifests.len() {
        report.map_deleted += 1;
    } else {
        report.failures.push(format!(
            "delete {doomed:#010X}: still stated {still}, {read_back} of {} manifests read back",
            manifests.len()
        ));
    }
}

#[test]
fn executing_a_plan_keeps_every_link() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install \
                 directories. World files live inside encrypted archives."
            ),
        );
        return;
    }
    if support::schema().struct_("CMapData").is_none() {
        skip_install(
            WHAT,
            "no schema snapshot is staged, and a grown or shrunk array reaches the file only \
             through synthesis, which lays out from the schema; see crates/io/or-io/src/meta/README.md",
        );
        return;
    }
    let stride: usize = std::env::var("OPENRAGE_EXEC_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(1);
    let reports: Vec<Report> = roots.iter().filter_map(|r| measure(r, stride)).collect();
    if reports.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for r in &reports {
        eprintln!("{}", r.text());
    }
    for r in &reports {
        let text = r.text();
        let refused: u64 = r.write_refused.values().sum();
        assert!(r.deleted >= 100 / r.stride as u64, "{text}");
        assert!(
            refused * 100 <= r.deleted,
            "the writer refused more than one delete in a hundred: {text}"
        );
        assert!(
            r.renumbering > 0,
            "no delete moved an index, so nothing was tested: {text}"
        );
        assert!(r.failures.is_empty(), "{text}");
        assert_eq!(r.links_kept, r.deleted, "{text}");
        assert_eq!(r.bytes_kept + refused, r.deleted, "{text}");
        assert!(r.created > 0 && r.map_deleted > 0, "{text}");
        assert_eq!(r.created, r.create_tried, "{text}");
        assert_eq!(r.map_deleted, r.delete_tried, "{text}");
    }
}
