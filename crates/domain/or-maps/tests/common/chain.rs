//! One entity of a map deleted with its chain of far models, in a pack made of the tree of maps the map belongs to, and what is
//! left held to the checks: the body the install suite runs over a mount and a default suite runs over maps built by hand.
//!
//! ⚠ **A link is judged by identity, not by count**, as `executing_a_plan_keeps_every_link` judges the working set: archetype, place
//! and guid of both ends, before and after.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_io::meta::{MetaDocument, MetaInstance};
use or_maps::delete::{self, Error};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply_all, inverses, At};
use or_world::execute::{identity, links_by_identity, Key};
use or_world::{GroupProblem, MapData, MapGroup};

/// What the first pass keeps of a map name: a few numbers, never the tree.
#[derive(Default, Clone)]
pub struct Facts {
    pub paths: Vec<String>,
    pub parent: u32,
    pub entities: usize,
}

#[derive(Default)]
pub struct Report {
    pub maps: usize,
    pub sampled: usize,
    /// Samples a source could not be made of, by the reason.
    pub not_made: BTreeMap<String, usize>,
    /// Plans refused by a rule of the deletion, by the rule.
    pub refused: BTreeMap<&'static str, usize>,
    pub ambiguous: usize,
    pub deleted: usize,
    /// Deletions that took more than the entity chosen.
    pub chains: usize,
    /// Deletions that wrote more than one file.
    pub across_files: usize,
    pub entities_gone: usize,
    pub renumbered: usize,
    pub renumbered_into_parent: usize,
    pub lowered: usize,
    pub notes: usize,
    /// Inverses that were the XML as it was and not the operation that takes one back.
    pub rewritten: usize,
    pub failures: Vec<String>,
}

impl Report {
    pub fn text(&self, stride: usize) -> String {
        format!(
            "{} maps hold entities; every {stride}th sampled: {}\n  sources not made: {:?}\n  refused: {:?}; \
             left as ambiguous: {}\n  deleted: {} ({} with a chain, {} across files), {} entities in all\n  \
             {} links renumbered inside a file and {} into a parent map, {} counts lowered, {} notes\n  \
             {} inverses were the XML as it was\n  failures: {}\n    {}",
            self.maps,
            self.sampled,
            self.not_made,
            self.refused,
            self.ambiguous,
            self.deleted,
            self.chains,
            self.across_files,
            self.entities_gone,
            self.renumbered,
            self.renumbered_into_parent,
            self.lowered,
            self.notes,
            self.rewritten,
            self.failures.len(),
            self.failures.join("\n    ")
        )
    }
}

/// Every file under `folder`, by its path inside it with `/`.
fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, into);
            } else if let (Ok(inside), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path))
            {
                into.insert(inside.to_string_lossy().replace('\\', "/"), bytes);
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// The value tree each of `sources` cooks to.
fn trees(folder: &Path, sources: &[String]) -> Result<Vec<MetaInstance>, String> {
    sources
        .iter()
        .map(|source| {
            let cooked = or_maps::facts::cooked(&folder.join(source))?;
            MetaDocument::parse(or_io::meta::default_library(), cooked)
                .map(|document| document.instance)
                .map_err(|e| format!("{source}: {e}"))
        })
        .collect()
}

/// What `or_world` says is wrong with the files: the checks of each file, and the checks of the group but for a count that is too high, which the game ships.
fn problems(trees: &[MetaInstance]) -> BTreeSet<String> {
    let mut group = MapGroup::new();
    let mut found = BTreeSet::new();
    for tree in trees {
        let Some(map) = MapData::new(tree) else {
            continue;
        };
        group.add(&map);
        for problem in or_world::validate::check(&map) {
            found.insert(format!("{:#010X}: {problem}", map.name()));
        }
    }
    for problem in group.check() {
        if !matches!(problem, GroupProblem::ChildCountOverstated { .. }) {
            found.insert(problem.to_string());
        }
    }
    found
}

/// The entity of `map` to delete: one under a far model that stands for nothing itself where there is one, else one that stands for nothing.
fn chosen(map: &CMapData) -> Option<usize> {
    let leaf = |with_parent: bool| {
        (map.entities.iter())
            .position(|e| e.num_children == 0 && (!with_parent || e.parent_index >= 0))
    };
    leaf(true).or_else(|| leaf(false))
}

/// What each map name among the files at `paths` holds, and the names of the maps under each, with `open` giving the bytes of a path.
pub fn survey(
    paths: &[String],
    open: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> (BTreeMap<u32, Facts>, BTreeMap<u32, Vec<u32>>) {
    let mut facts: BTreeMap<u32, Facts> = BTreeMap::new();
    for path in paths {
        let Some(bytes) = open(path) else {
            continue;
        };
        let Ok(map) = CMapData::from_bytes(&bytes) else {
            continue;
        };
        let slot = facts.entry(map.name.hash).or_default();
        slot.paths.push(path.clone());
        slot.parent = slot.parent.max(map.parent.hash);
        slot.entities += map.entities.len();
    }
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (name, held) in &facts {
        if held.parent != 0 {
            children.entry(held.parent).or_default().push(*name);
        }
    }
    (facts, children)
}

/// Delete one entity of the map `name` in a pack made of its tree, read through `open`, and hold what is left to the checks.
pub fn delete_one(
    open: &dyn Fn(&str) -> Option<Vec<u8>>,
    name: u32,
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
    report: &mut Report,
) {
    // The topmost map above this one, then every map under it.
    let mut top = name;
    for _ in 0..facts.len() {
        match facts.get(&top).map(|f| f.parent) {
            Some(parent) if parent != 0 && parent != top && facts.contains_key(&parent) => {
                top = parent
            }
            _ => break,
        }
    }
    let mut members = vec![top];
    let mut next = 0;
    while let Some(member) = members.get(next).copied() {
        next += 1;
        for child in children.get(&member).into_iter().flatten() {
            if !members.contains(child) {
                members.push(*child);
            }
        }
    }

    let scratch = match tempfile::tempdir() {
        Ok(scratch) => scratch,
        Err(e) => {
            report
                .failures
                .push(format!("{name:#010X}: no scratch folder: {e}"));
            return;
        }
    };
    let folder = scratch.path().join("maps");
    let mut read: Vec<(String, Vec<u8>)> = Vec::new();
    for member in &members {
        for path in &facts[member].paths {
            if let Some(bytes) = open(path) {
                let leaf = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
                read.push((leaf, bytes));
            }
        }
    }
    let incoming: Vec<Incoming<'_>> = (read.iter())
        .map(|(leaf, bytes)| Incoming {
            name: leaf,
            bytes,
            from_game: true,
            virtual_path: None,
        })
        .collect();
    let added = match add_files(&folder, &incoming, None) {
        Ok(added) => added,
        Err(why) => {
            *report.not_made.entry(why).or_default() += 1;
            return;
        }
    };
    if let Some((_, why)) = added.refused.first() {
        // A tree with a file left out would have links the plan cannot see.
        *report.not_made.entry(why.clone()).or_default() += 1;
        return;
    }
    // The source made of each file, in the order the files were read: none was refused.
    let sources: Vec<String> = (added.made.iter())
        .filter_map(|dir| Some(dir.file_name()?.to_string_lossy().into_owned()))
        .collect();
    if sources.len() != read.len() {
        report.failures.push(format!(
            "{name:#010X}: {} sources were made of {} files",
            sources.len(),
            read.len()
        ));
        return;
    }

    // The file of the map that holds its entities, and the entity to delete in it.
    let Some((source, index)) = read.iter().zip(&sources).find_map(|((_, bytes), source)| {
        let map = CMapData::from_bytes(bytes).ok()?;
        (map.name.hash == name && !map.entities.is_empty())
            .then(|| chosen(&map).map(|index| (source.clone(), index)))
            .flatten()
    }) else {
        *report
            .refused
            .entry("every entity stands for another")
            .or_default() += 1;
        return;
    };
    let at = At { source, index };

    let fail = |report: &mut Report, why: String| {
        report
            .failures
            .push(format!("{} entity {index}: {why}", at.source));
    };
    let before_files = files(&folder);
    let before = match trees(&folder, &sources) {
        Ok(before) => before,
        Err(why) => return fail(report, why),
    };
    let was_wrong = problems(&before);
    let links = links_by_identity(before.iter());

    let planned = match delete::plan(&folder, std::slice::from_ref(&at)) {
        Ok(planned) => planned,
        Err(Error::ContainerLods(_)) => {
            *report
                .refused
                .entry("the map carries container LODs")
                .or_default() += 1;
            return;
        }
        Err(Error::SplitMap(_)) => {
            *report
                .refused
                .entry("two files hold entities of one map")
                .or_default() += 1;
            return;
        }
        Err(other) => return fail(report, other.to_string()),
    };
    if planned.goes.is_empty() {
        report.ambiguous += 1;
        return;
    }
    // The identity of each entity that goes, read from the files as they were.
    let mut gone: BTreeSet<Key> = BTreeSet::new();
    for going in &planned.goes {
        let tree = sources.iter().position(|s| *s == going.entity.at.source);
        let entity = tree
            .and_then(|tree| MapData::new(&before[tree]))
            .and_then(|map| map.entities().nth(going.entity.at.index));
        match entity {
            Some(entity) => gone.insert(identity(entity.instance())),
            None => return fail(report, "the plan names an entity that is not there".into()),
        };
    }

    let applied = match delete::apply(&folder, &planned) {
        Ok(applied) => applied,
        Err(e) => return fail(report, format!("the deletion did not apply: {e}")),
    };
    let after_files = files(&folder);
    let after = match trees(&folder, &sources) {
        Ok(after) => after,
        Err(why) => return fail(report, format!("after the deletion, {why}")),
    };
    let kept: BTreeMap<Key, Key> = (links.iter())
        .filter(|(child, _)| !gone.contains(child))
        .map(|(child, parent)| (*child, *parent))
        .collect();
    if links_by_identity(after.iter()) != kept {
        return fail(
            report,
            "a link that stays joins other entities than it joined".into(),
        );
    }
    let new: Vec<String> = (problems(&after).into_iter())
        .filter(|problem| !was_wrong.contains(problem))
        .collect();
    if let Some(first) = new.first() {
        return fail(
            report,
            format!("{} new problem(s), first: {first}", new.len()),
        );
    }
    let written: BTreeSet<&str> = (after_files.iter())
        .filter(|(path, bytes)| before_files.get(*path) != Some(bytes))
        .map(|(path, _)| path.as_str())
        .collect();
    let listed: BTreeSet<&str> = planned.files.iter().map(|f| f.file.as_str()).collect();
    if written != listed {
        return fail(
            report,
            format!("the plan lists {listed:?} and the deletion wrote {written:?}"),
        );
    }

    let back = inverses(&applied);
    report.rewritten += (back.iter())
        .filter(|inverse| inverse.name() == "rewrite_source")
        .count();
    if let Err(e) = apply_all(&folder, &back, false) {
        return fail(report, format!("the inverses did not apply: {e}"));
    }
    if files(&folder) != before_files {
        return fail(report, "the inverses do not put every file back".into());
    }

    report.deleted += 1;
    report.entities_gone += planned.goes.len();
    report.chains += usize::from(planned.goes.len() > 1);
    report.across_files += usize::from(planned.files.len() > 1);
    report.notes += planned.notes.len();
    for file in &planned.files {
        report.renumbered += file.renumbered;
        report.renumbered_into_parent += file.renumbered_into_parent;
        report.lowered += file.lowered.len();
    }
}
