//! One entity of a map deleted with its chain of far models, in a pack made of the tree of maps the map belongs to, and what is
//! left held to the checks: the body the install suite runs over a mount and a default suite runs over maps built by hand.
//!
//! The samples of one tree share its pack, which is imported once: each deletion is undone, and the pack is held to the bytes it
//! was made with before the next.
//!
//! [`put_back_sampled`] is the same body with two steps more. Before the deletion, the entity chosen is stood a metre off with
//! another flag and put back by `or_maps::put_back`. Before the deletion is undone, the entity is put back the same way. Each
//! time the pack is held to the bytes it was made with, and the inverses of the plan to leave the edit it was made of.
//!
//! ⚠ **A link is judged by identity, not by count**, as `executing_a_plan_keeps_every_link` judges the working set: archetype, place
//! and guid of both ends, before and after.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_io::meta::{MetaDocument, MetaInstance};
use or_maps::delete::{self, Error};
use or_maps::edit::{pose_of, read_entity};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply, apply_all, inverses, At, Operation, PoseData, Record};
use or_maps::put_back;
use or_world::transform::Pose;
use or_world_gta5::execute::{identity, links_by_identity, Key};
use or_world_gta5::{GroupProblem, MapData, MapGroup};

/// How many threads the files and the trees of an install are spread over, as `what_the_rules_say_of_the_install` spreads its files.
const WORKERS: usize = 12;

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
    /// Deletions whose chosen entity was put back, and the entities that came back in all.
    pub put_back: usize,
    pub came_back: usize,
    /// Plans to put an entity back that were refused, by the kind of refusal.
    pub not_put_back: BTreeMap<&'static str, usize>,
    /// Sources a plan to put an entity back wrote whole, as XML, and not by an operation that says what goes in.
    pub put_back_whole: usize,
    /// Entities moved and flagged, and those of them put back by one record with the extents of their file.
    pub moved: usize,
    pub moved_back: usize,
    /// Plans to put a moved entity back that were refused, by the kind of refusal.
    pub not_moved_back: BTreeMap<&'static str, usize>,
    pub failures: Vec<String>,
}

impl Report {
    /// Add what `other` counted of other samples to this report.
    fn merge(&mut self, other: Report) {
        let Report {
            maps,
            sampled,
            not_made,
            refused,
            ambiguous,
            deleted,
            chains,
            across_files,
            entities_gone,
            renumbered,
            renumbered_into_parent,
            lowered,
            notes,
            rewritten,
            put_back,
            came_back,
            not_put_back,
            put_back_whole,
            moved,
            moved_back,
            not_moved_back,
            failures,
        } = other;
        self.maps += maps;
        self.sampled += sampled;
        for (why, count) in not_made {
            *self.not_made.entry(why).or_default() += count;
        }
        for (rule, count) in refused {
            *self.refused.entry(rule).or_default() += count;
        }
        self.ambiguous += ambiguous;
        self.deleted += deleted;
        self.chains += chains;
        self.across_files += across_files;
        self.entities_gone += entities_gone;
        self.renumbered += renumbered;
        self.renumbered_into_parent += renumbered_into_parent;
        self.lowered += lowered;
        self.notes += notes;
        self.rewritten += rewritten;
        self.put_back += put_back;
        self.came_back += came_back;
        for (why, count) in not_put_back {
            *self.not_put_back.entry(why).or_default() += count;
        }
        self.put_back_whole += put_back_whole;
        self.moved += moved;
        self.moved_back += moved_back;
        for (why, count) in not_moved_back {
            *self.not_moved_back.entry(why).or_default() += count;
        }
        self.failures.extend(failures);
    }

    pub fn text(&self, stride: usize) -> String {
        format!(
            "{} maps hold entities; every {stride}th sampled: {}\n  sources not made: {:?}\n  refused: {:?}; \
             left as ambiguous: {}\n  deleted: {} ({} with a chain, {} across files), {} entities in all\n  \
             {} links renumbered inside a file and {} into a parent map, {} counts lowered, {} notes\n  \
             {} inverses were the XML as it was\n  \
             put back: {} ({} entities came back, {} sources written whole); not put back: {:?}\n  \
             moved: {}, put back: {}; not put back: {:?}\n  failures: {}\n    {}",
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
            self.put_back,
            self.came_back,
            self.put_back_whole,
            self.not_put_back,
            self.moved,
            self.moved_back,
            self.not_moved_back,
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
fn problems<'a>(trees: impl Iterator<Item = &'a MetaInstance>) -> BTreeSet<String> {
    let mut group = MapGroup::new();
    let mut found = BTreeSet::new();
    for tree in trees {
        let Some(map) = MapData::new(tree) else {
            continue;
        };
        group.add(&map);
        for problem in or_world_gta5::validate::check(&map) {
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
///
/// ⚠ The files are read several at once and folded in the order of `paths`: every map file of an install is minutes on one thread.
pub fn survey(
    paths: &[String],
    open: &(dyn Fn(&str) -> Option<Vec<u8>> + Sync),
) -> (BTreeMap<u32, Facts>, BTreeMap<u32, Vec<u32>>) {
    // The name, the parent and the count of entities each file states, by its place among the paths.
    let stated: Mutex<Vec<(usize, u32, u32, usize)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(paths.len()) {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(at) else { break };
                let Some(bytes) = open(path) else {
                    continue;
                };
                let Ok(map) = CMapData::from_bytes(&bytes) else {
                    continue;
                };
                let row = (at, map.name.hash, map.parent.hash, map.entities.len());
                stated.lock().expect("no worker panicked").push(row);
            });
        }
    });
    let mut stated = stated.into_inner().expect("no worker panicked");
    stated.sort();
    let mut facts: BTreeMap<u32, Facts> = BTreeMap::new();
    for (at, name, parent, entities) in stated {
        let slot = facts.entry(name).or_default();
        slot.paths.push(paths[at].clone());
        slot.parent = slot.parent.max(parent);
        slot.entities += entities;
    }
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (name, held) in &facts {
        if held.parent != 0 {
            children.entry(held.parent).or_default().push(*name);
        }
    }
    (facts, children)
}

/// The maps of the tree `name` belongs to: the topmost map above it, then every map under that one.
pub fn tree_of(
    name: u32,
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
) -> Vec<u32> {
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
    members
}

/// Why a pack was not made of a tree: a reason the report counts by, or a failure of the suite.
enum Unmade {
    NotMade(String),
    Failure(String),
}

/// What a pack held when it was made, which every deletion in it is undone to.
struct Before {
    files: BTreeMap<String, Vec<u8>>,
    trees: Vec<MetaInstance>,
    was_wrong: BTreeSet<String>,
    links: BTreeMap<Key, Key>,
}

/// A pack made of a tree of maps in a scratch folder, which goes with it.
struct Pack {
    _scratch: tempfile::TempDir,
    folder: PathBuf,
    /// The file name and the bytes of each map file read, and the source made of each, in one order.
    read: Vec<(String, Vec<u8>)>,
    sources: Vec<String>,
    /// Read the first time a deletion asks, since a sample that finds no entity to delete asks for none of it.
    before: OnceCell<Result<Before, String>>,
}

impl Pack {
    /// Import the map files of `members`, read through `open`, as the sources of a pack.
    fn make(
        open: &dyn Fn(&str) -> Option<Vec<u8>>,
        members: &[u32],
        facts: &BTreeMap<u32, Facts>,
    ) -> Result<Self, Unmade> {
        let scratch =
            tempfile::tempdir().map_err(|e| Unmade::Failure(format!("no scratch folder: {e}")))?;
        let folder = scratch.path().join("maps");
        let mut read: Vec<(String, Vec<u8>)> = Vec::new();
        for member in members {
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
        let added = add_files(&folder, &incoming, None).map_err(Unmade::NotMade)?;
        if let Some((_, why)) = added.refused.first() {
            // A tree with a file left out would have links the plan cannot see.
            return Err(Unmade::NotMade(why.clone()));
        }
        // The source made of each file, in the order the files were read: none was refused.
        let sources: Vec<String> = (added.made.iter())
            .filter_map(|dir| Some(dir.file_name()?.to_string_lossy().into_owned()))
            .collect();
        if sources.len() != read.len() {
            return Err(Unmade::Failure(format!(
                "{} sources were made of {} files",
                sources.len(),
                read.len()
            )));
        }
        Ok(Self {
            _scratch: scratch,
            folder,
            read,
            sources,
            before: OnceCell::new(),
        })
    }

    fn before(&self) -> Result<&Before, String> {
        let before = self.before.get_or_init(|| {
            let trees = trees(&self.folder, &self.sources)?;
            Ok(Before {
                files: files(&self.folder),
                was_wrong: problems(trees.iter()),
                links: links_by_identity(trees.iter()),
                trees,
            })
        });
        before.as_ref().map_err(Clone::clone)
    }

    /// Delete one entity of the map `name` with its chain, hold what is left to the checks and undo it, putting the entity back
    /// in between with `put_back`; false when a failure may have left the pack other than it was made.
    fn delete_in(&self, name: u32, put_back: bool, report: &mut Report) -> bool {
        let (folder, sources) = (&self.folder, &self.sources);
        // The file of the map that holds its entities, and the entity to delete in it.
        let found = self
            .read
            .iter()
            .zip(sources)
            .find_map(|((_, bytes), source)| {
                let map = CMapData::from_bytes(bytes).ok()?;
                (map.name.hash == name && !map.entities.is_empty())
                    .then(|| chosen(&map).map(|index| (source.clone(), index)))
                    .flatten()
            });
        let Some((source, index)) = found else {
            *report
                .refused
                .entry("every entity stands for another")
                .or_default() += 1;
            return true;
        };
        let at = At { source, index };

        let fail = |report: &mut Report, why: String| {
            report
                .failures
                .push(format!("{} entity {index}: {why}", at.source));
        };
        let before = match self.before() {
            Ok(before) => before,
            Err(why) => {
                fail(report, why);
                return true;
            }
        };

        if put_back && !self.moved_back(&at, before, report) {
            return false;
        }

        let planned = match delete::plan(folder, std::slice::from_ref(&at)) {
            Ok(planned) => planned,
            Err(Error::ContainerLods(_)) => {
                *report
                    .refused
                    .entry("the map carries container LODs")
                    .or_default() += 1;
                return true;
            }
            Err(Error::SplitMap(_)) => {
                *report
                    .refused
                    .entry("two files hold entities of one map")
                    .or_default() += 1;
                return true;
            }
            Err(other) => {
                fail(report, other.to_string());
                return true;
            }
        };
        if planned.goes.is_empty() {
            report.ambiguous += 1;
            return true;
        }
        // The identity of each entity that goes, read from the files as they were.
        let mut gone: BTreeSet<Key> = BTreeSet::new();
        for going in &planned.goes {
            let tree = sources.iter().position(|s| *s == going.entity.at.source);
            let entity = tree
                .and_then(|tree| MapData::new(&before.trees[tree]))
                .and_then(|map| map.entities().nth(going.entity.at.index));
            match entity {
                Some(entity) => gone.insert(identity(entity.instance())),
                None => {
                    fail(report, "the plan names an entity that is not there".into());
                    return true;
                }
            };
        }

        // ⚠ From here a failure leaves the pack as the deletion left it.
        let applied = match delete::apply(folder, &planned) {
            Ok(applied) => applied,
            Err(e) => {
                fail(report, format!("the deletion did not apply: {e}"));
                return false;
            }
        };
        let after_files = files(folder);
        let written: BTreeSet<&str> = (after_files.iter())
            .filter(|(path, bytes)| before.files.get(*path) != Some(bytes))
            .map(|(path, _)| path.as_str())
            .collect();
        // A source none of whose files changed cooks to the tree it cooked to, so only the others are cooked again.
        let changed: Vec<String> = (sources.iter())
            .filter(|source| {
                let inside = format!("{source}/");
                written.iter().any(|path| path.starts_with(&inside))
            })
            .cloned()
            .collect();
        let cooked = match trees(folder, &changed) {
            Ok(cooked) => cooked,
            Err(why) => {
                fail(report, format!("after the deletion, {why}"));
                return false;
            }
        };
        let after: Vec<&MetaInstance> = (sources.iter().zip(&before.trees))
            .map(|(source, was)| {
                let again = changed.iter().position(|changed| changed == source);
                again.map_or(was, |again| &cooked[again])
            })
            .collect();
        let kept: BTreeMap<Key, Key> = (before.links.iter())
            .filter(|(child, _)| !gone.contains(child))
            .map(|(child, parent)| (*child, *parent))
            .collect();
        if links_by_identity(after.iter().copied()) != kept {
            fail(
                report,
                "a link that stays joins other entities than it joined".into(),
            );
            return false;
        }
        let new: Vec<String> = (problems(after.iter().copied()).into_iter())
            .filter(|problem| !before.was_wrong.contains(problem))
            .collect();
        if let Some(first) = new.first() {
            fail(
                report,
                format!("{} new problem(s), first: {first}", new.len()),
            );
            return false;
        }
        let listed: BTreeSet<&str> = planned.files.iter().map(|f| f.file.as_str()).collect();
        if written != listed {
            fail(
                report,
                format!("the plan lists {listed:?} and the deletion wrote {written:?}"),
            );
            return false;
        }

        if put_back && !self.put_back(&at, planned.goes.len(), before, &after_files, report) {
            return false;
        }

        let back = inverses(&applied);
        report.rewritten += (back.iter())
            .filter(|inverse| inverse.name() == "rewrite_source")
            .count();
        if let Err(e) = apply_all(folder, &back, false) {
            fail(report, format!("the inverses did not apply: {e}"));
            return false;
        }
        if files(folder) != before.files {
            fail(report, "the inverses do not put every file back".into());
            return false;
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
        true
    }
}

impl Pack {
    /// Put the entity `at` back, which a deletion of `gone` entities took out and left the pack holding `deleted`: the pack is
    /// then the bytes it was made with, and the inverses of what did it leave the deletion. False when a failure left the pack
    /// other than the deletion left it.
    fn put_back(
        &self,
        at: &At,
        gone: usize,
        before: &Before,
        deleted: &BTreeMap<String, Vec<u8>>,
        report: &mut Report,
    ) -> bool {
        let folder = &self.folder;
        let fail = |report: &mut Report, why: String| {
            let said = format!("{} entity {}, put back: {why}", at.source, at.index);
            report.failures.push(said);
        };
        // The pack was made of the files the bases are, so the index the deletion named is the index in the base.
        let planned = match put_back::plan(folder, &put_back::Named::Base(at.clone())) {
            Ok(planned) => planned,
            Err(refused) => {
                match earned(&refused) {
                    Some(kind) => *report.not_put_back.entry(kind).or_default() += 1,
                    None => fail(report, format!("no plan: {refused}")),
                }
                return true;
            }
        };
        if planned.returns.len() != gone || planned.restored.is_some() {
            let came = planned.returns.len();
            fail(
                report,
                format!("{came} entities would come back where {gone} went"),
            );
            return true;
        }
        let done = match put_back::apply(folder, &planned) {
            Ok(done) => done,
            Err(e) => {
                fail(report, format!("the plan did not apply: {e}"));
                return true;
            }
        };
        // ⚠ From here a failure leaves the pack other than the deletion left it.
        if files(folder) != before.files {
            fail(report, "the pack is not the bytes it was made with".into());
            return false;
        }
        if let Err(e) = apply_all(folder, &inverses(&done), false) {
            fail(report, format!("its inverses did not apply: {e}"));
            return false;
        }
        if files(folder) != *deleted {
            fail(report, "its inverses do not leave the deletion".into());
            return false;
        }
        report.put_back += 1;
        report.came_back += planned.returns.len();
        report.put_back_whole += (planned.operations.iter())
            .filter(|operation| operation.name() == "rewrite_source")
            .count();
        true
    }
}

/// Why a plan to put an entity back was refused, where the refusal is one the sources earn and no failure of the suite.
fn earned(refused: &put_back::Error) -> Option<&'static str> {
    Some(match refused {
        put_back::Error::Ambiguous { .. } => "it cannot be told from a twin",
        put_back::Error::FarModelAmbiguous { .. } => "its far model cannot be told from a twin",
        put_back::Error::SplitMap(_) => "two files hold entities of one map",
        put_back::Error::Unwritable { .. } => "its file cannot be written with it",
        _ => return None,
    })
}

impl Pack {
    /// Stand the entity `at` a metre off and state another flag of it, then put it back: the pack is then the bytes it was made
    /// with, and the inverses of what did it leave the edit. False when a failure left the pack other than it was made.
    fn moved_back(&self, at: &At, before: &Before, report: &mut Report) -> bool {
        let folder = &self.folder;
        let fail = |report: &mut Report, why: String| {
            let said = format!("{} entity {}, moved: {why}", at.source, at.index);
            report.failures.push(said);
        };
        let dir = folder.join(&at.source);
        let stands = match read_entity(&dir, at.index) {
            Ok(stands) => stands,
            Err(why) => {
                fail(report, format!("the entity is not read: {why}"));
                return true;
            }
        };
        let pose = pose_of(&stands);
        let moved = Operation::PlaceEntity {
            source: at.source.clone(),
            expects: None,
            entity: at.index,
            pose: PoseData::from(Pose {
                position: pose.position + glam::Vec3::X,
                ..pose
            }),
        };
        let first = match apply(folder, &moved) {
            Ok(first) => first,
            Err(e) => {
                fail(report, format!("the move did not apply: {e}"));
                return true;
            }
        };
        // ⚠ From here a failure leaves the pack other than it was made.
        let flagged = read_entity(&dir, at.index).and_then(|mut flagged| {
            // The lowest bit, which says nothing of where the LOD parent is.
            flagged.flags ^= 1;
            Record::entity(&flagged)
        });
        let stated = flagged.and_then(|record| {
            let stated = Operation::ReplaceEntity {
                source: at.source.clone(),
                expects: None,
                entity: at.index,
                record,
                extents: None,
            };
            apply(folder, &stated).map_err(|e| e.to_string())
        });
        let second = match stated {
            Ok(second) => second,
            Err(why) => {
                fail(report, format!("the flag was not stated: {why}"));
                return false;
            }
        };
        report.moved += 1;
        let edited = files(folder);
        if edited == before.files {
            fail(report, "the move and the flag changed no file".into());
            return true;
        }
        // The edit taken back by its own inverses, which leaves the pack as it was made where the entity is not put back.
        let undo = |report: &mut Report| {
            let back = inverses(&[first.clone(), second.clone()]);
            let undone = apply_all(folder, &back, false);
            let whole = undone.is_ok() && files(folder) == before.files;
            if !whole {
                fail(report, "the edit was not taken back".into());
            }
            whole
        };

        let planned = match put_back::plan(folder, &put_back::Named::Now(at.clone())) {
            Ok(planned) => planned,
            Err(refused) => {
                match earned(&refused) {
                    Some(kind) => *report.not_moved_back.entry(kind).or_default() += 1,
                    None => fail(report, format!("no plan: {refused}")),
                }
                return undo(report);
            }
        };
        if planned.restored.is_none() || !planned.returns.is_empty() {
            let came = planned.returns.len();
            fail(
                report,
                format!("{came} entities would come back for one that was moved"),
            );
            return undo(report);
        }
        let done = match put_back::apply(folder, &planned) {
            Ok(done) => done,
            Err(e) => {
                fail(report, format!("the plan did not apply: {e}"));
                return undo(report);
            }
        };
        if files(folder) != before.files {
            fail(report, "the pack is not the bytes it was made with".into());
            return false;
        }
        if let Err(e) = apply_all(folder, &inverses(&done), false) {
            fail(report, format!("its inverses did not apply: {e}"));
            return false;
        }
        if files(folder) != edited {
            fail(report, "its inverses do not leave the edit".into());
            return false;
        }
        // The plan again, which leaves the pack as it was made for the deletion that follows.
        let again = put_back::apply(folder, &planned);
        if again.is_err() || files(folder) != before.files {
            fail(report, "the plan did not apply a second time".into());
            return false;
        }
        report.moved_back += 1;
        true
    }
}

/// Delete one entity of each map of `names`, which are of one tree, and hold what is left to the checks.
///
/// The tree is imported once: each deletion is undone and the pack held to the bytes it was made with before the next, and a pack
/// a failure left changed is made again.
fn delete_each(
    open: &dyn Fn(&str) -> Option<Vec<u8>>,
    names: &[u32],
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
    put_back: bool,
    report: &mut Report,
) {
    let mut pack: Option<Pack> = None;
    for (at, name) in names.iter().enumerate() {
        let made = match pack.take() {
            Some(made) => made,
            None => match Pack::make(open, &tree_of(*name, facts, children), facts) {
                Ok(made) => made,
                // The tree of the samples left is this one, and is refused as it was.
                Err(Unmade::NotMade(why)) => {
                    *report.not_made.entry(why).or_default() += names.len() - at;
                    return;
                }
                Err(Unmade::Failure(why)) => {
                    report.failures.push(format!("{name:#010X}: {why}"));
                    continue;
                }
            },
        };
        if made.delete_in(*name, put_back, report) {
            pack = Some(made);
        }
    }
}

/// Delete one entity of each map of `sampled` in a pack made of its tree, read through `open`, and report what the checks found;
/// the caller states `maps`.
///
/// ⚠ The samples are taken a tree at a time, the trees with the most samples first and several at once: importing its tree is
/// most of what a sample costs.
pub fn delete_sampled(
    open: &(dyn Fn(&str) -> Option<Vec<u8>> + Sync),
    sampled: &[u32],
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
) -> Report {
    sampled_in_packs(open, sampled, facts, children, false)
}

/// [`delete_sampled`], with the entity each deletion chose put back before the deletion is undone.
pub fn put_back_sampled(
    open: &(dyn Fn(&str) -> Option<Vec<u8>> + Sync),
    sampled: &[u32],
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
) -> Report {
    sampled_in_packs(open, sampled, facts, children, true)
}

fn sampled_in_packs(
    open: &(dyn Fn(&str) -> Option<Vec<u8>> + Sync),
    sampled: &[u32],
    facts: &BTreeMap<u32, Facts>,
    children: &BTreeMap<u32, Vec<u32>>,
    put_back: bool,
) -> Report {
    let mut by_tree: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for name in sampled {
        let top = tree_of(*name, facts, children)[0];
        by_tree.entry(top).or_default().push(*name);
    }
    let mut groups: Vec<Vec<u32>> = by_tree.into_values().collect();
    groups.sort_by_key(|names| std::cmp::Reverse(names.len()));

    let next = AtomicUsize::new(0);
    let whole = Mutex::new(Report {
        sampled: sampled.len(),
        ..Report::default()
    });
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(groups.len()) {
            scope.spawn(|| {
                let mut report = Report::default();
                while let Some(names) = groups.get(next.fetch_add(1, Ordering::Relaxed)) {
                    delete_each(open, names, facts, children, put_back, &mut report);
                }
                whole.lock().expect("no worker panicked").merge(report);
            });
        }
    });
    let mut whole = whole.into_inner().expect("no worker panicked");
    whole.failures.sort();
    whole
}
