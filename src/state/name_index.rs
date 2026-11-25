//! Names the metadata editor offers as an autocomplete, by category.
//!
//! ⚠ **Every category has to name a producer, and [`PRODUCER_OF`] is where.**
//! A dead-code lint cannot see a category nothing fills: `drain_bg` is live
//! from `poll_jobs` whether or not anything ever sets `bg_rx`, and the
//! picker opens empty forever.
//!
//! An install listing sees file names, which is exactly a model, a texture
//! dictionary or a clip dictionary; an effect **rule** name is a string
//! inside a `.ypt` and no listing can reach it, so that category is fed by
//! the documents the editor has open.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;

use or_io::container::RpfCryptoProvider;
use or_io::names::{Collision, HashFlavour, NameIndex as HashNames};
use or_title::vfs::{build_mounted_game_with, DlcCutoff};
use or_title::{scan_install_names, GtaInstall, InstalledNameKind};

use super::document::DocPayload;
use super::jobs::{JobKind, RunningJob};
use super::memory::{hash_table_bytes, hash_table_bytes_for_len};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum NameIndexCategory {
    Model = 0,
    Texture = 1,
    ClipSet = 2,
    Effect = 3,
}

impl NameIndexCategory {
    pub const ALL: &'static [Self] = &[Self::Model, Self::Texture, Self::ClipSet, Self::Effect];

    pub fn label(self) -> &'static str {
        match self {
            Self::Model => "Models",
            Self::Texture => "Textures",
            Self::ClipSet => "Clip sets",
            Self::Effect => "Effect rules",
        }
    }

    /// What an empty picker should say is going to fill it.
    pub fn producer_hint(self) -> &'static str {
        match producer_of(self) {
            Some(NameProducer::InstallScan) => {
                "(nothing indexed yet — choose a game install and extract its \
                 keys, and the background scan fills this)"
            }
            Some(NameProducer::OpenDocument) => {
                "(nothing indexed yet — open a .ypt and its effect rules appear here)"
            }
            None => "(nothing produces this category)",
        }
    }
}

/// What fills a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProducer {
    /// The background walk of a mounted install, classifying leaf names.
    InstallScan,
    /// A document the editor has open, read for the names it carries.
    OpenDocument,
}

/// Every category, against what supplies it.
pub const PRODUCER_OF: &[(NameIndexCategory, NameProducer)] = &[
    (NameIndexCategory::Model, NameProducer::InstallScan),
    (NameIndexCategory::Texture, NameProducer::InstallScan),
    (NameIndexCategory::ClipSet, NameProducer::InstallScan),
    (NameIndexCategory::Effect, NameProducer::OpenDocument),
];

/// ⚠ **A category with no producer does not compile**, because a test is
/// skippable.
const _: () = {
    let mut i = 0;
    while i < NameIndexCategory::ALL.len() {
        let mut found = false;
        let mut j = 0;
        while j < PRODUCER_OF.len() {
            if PRODUCER_OF[j].0 as u8 == NameIndexCategory::ALL[i] as u8 {
                found = true;
            }
            j += 1;
        }
        assert!(
            found,
            "a picker category with nothing to fill it opens empty forever"
        );
        i += 1;
    }
    assert!(
        PRODUCER_OF.len() == NameIndexCategory::ALL.len(),
        "PRODUCER_OF names a category twice, or one the enum does not"
    );
};

const fn producer_of(category: NameIndexCategory) -> Option<NameProducer> {
    let mut i = 0;
    while i < PRODUCER_OF.len() {
        if PRODUCER_OF[i].0 as u8 == category as u8 {
            return Some(PRODUCER_OF[i].1);
        }
        i += 1;
    }
    None
}

impl From<InstalledNameKind> for NameIndexCategory {
    fn from(kind: InstalledNameKind) -> Self {
        match kind {
            InstalledNameKind::Model => Self::Model,
            InstalledNameKind::Texture => Self::Texture,
            InstalledNameKind::ClipSet => Self::ClipSet,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NameIndexSnapshot {
    pub models: Arc<Vec<String>>,
    pub textures: Arc<Vec<String>>,
    pub clip_sets: Arc<Vec<String>>,
    pub effects: Arc<Vec<String>>,
}

impl NameIndexSnapshot {
    pub fn names(&self, category: NameIndexCategory) -> &[String] {
        match category {
            NameIndexCategory::Model => &self.models,
            NameIndexCategory::Texture => &self.textures,
            NameIndexCategory::ClipSet => &self.clip_sets,
            NameIndexCategory::Effect => &self.effects,
        }
    }
}

pub type NameBatch = Vec<(NameIndexCategory, String)>;

/// The names a payload carries, for the categories no install listing can
/// supply.
pub fn names_in(payload: &DocPayload) -> NameBatch {
    match payload {
        DocPayload::Resource(or_io::Asset::Particles(list)) => {
            effect_rule_names(or_particle::list::effect_rules(list).map(|e| &e.name))
        }
        DocPayload::Resource(or_io::Asset::ParticlesGen9(list)) => {
            effect_rule_names(or_particle::list::effect_rules(list).map(|e| &e.name))
        }
        _ => NameBatch::new(),
    }
}

fn effect_rule_names<'a, I>(rules: I) -> NameBatch
where
    I: Iterator<Item = &'a String>,
{
    rules
        .filter(|name| !name.is_empty())
        .map(|name| (NameIndexCategory::Effect, name.clone()))
        .collect()
}

pub struct NameIndex {
    models: HashSet<String>,
    textures: HashSet<String>,
    clip_sets: HashSet<String>,
    effects: HashSet<String>,
    cached_snapshot: Option<NameIndexSnapshot>,
    bg_rx: Option<Receiver<NameBatch>>,
    bg_handle: Option<JoinHandle<()>>,
    bg_cancel: Option<Arc<AtomicBool>>,
    scanned_root: Option<PathBuf>,
    /// Names the scan in flight has sent.
    scan_found: usize,
    /// The same names, reachable from their hash.
    ///
    /// ⚠ Per title, and switching titles replaces it rather than adding to
    /// it: one game's name answering another's hash looks exactly like a
    /// correct answer.
    hashes: HashNames,
}

impl Default for NameIndex {
    fn default() -> Self {
        Self {
            models: HashSet::new(),
            textures: HashSet::new(),
            clip_sets: HashSet::new(),
            effects: HashSet::new(),
            cached_snapshot: None,
            bg_rx: None,
            bg_handle: None,
            bg_cancel: None,
            scanned_root: None,
            scan_found: 0,
            hashes: HashNames::with_bundled(""),
        }
    }
}

impl NameIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&mut self) -> Arc<NameIndexSnapshot> {
        if let Some(snap) = &self.cached_snapshot {
            return Arc::new(snap.clone());
        }
        let mut models: Vec<String> = self.models.iter().cloned().collect();
        let mut textures: Vec<String> = self.textures.iter().cloned().collect();
        let mut clip_sets: Vec<String> = self.clip_sets.iter().cloned().collect();
        let mut effects: Vec<String> = self.effects.iter().cloned().collect();
        models.sort();
        textures.sort();
        clip_sets.sort();
        effects.sort();
        let snap = NameIndexSnapshot {
            models: Arc::new(models),
            textures: Arc::new(textures),
            clip_sets: Arc::new(clip_sets),
            effects: Arc::new(effects),
        };
        self.cached_snapshot = Some(snap.clone());
        Arc::new(snap)
    }

    pub fn add(&mut self, category: NameIndexCategory, name: impl Into<String>) {
        let set = match category {
            NameIndexCategory::Model => &mut self.models,
            NameIndexCategory::Texture => &mut self.textures,
            NameIndexCategory::ClipSet => &mut self.clip_sets,
            NameIndexCategory::Effect => &mut self.effects,
        };
        let name = normalise(name);
        self.hashes.insert(&name);
        if set.insert(name) {
            self.cached_snapshot = None;
        }
    }

    /// The name for a hash, or `hash_<hex>` when nothing here knows it.
    pub fn display(&self, hash: u32) -> String {
        self.hashes.display(hash)
    }

    /// The name `hash` was made from, where this title's names hold it.
    pub fn name_of(&self, hash: u32) -> Option<&str> {
        self.hashes.resolve(hash).map(|found| found.name)
    }

    /// Point the reverse index at a title, discarding whatever the last one
    /// put in it.
    pub fn retarget(&mut self, title: &str) {
        if self.hashes.title() != title {
            self.hashes = HashNames::with_bundled(title);
        }
    }

    /// Take whatever names a newly opened document carries.
    pub fn absorb(&mut self, payload: &DocPayload) {
        for (category, name) in names_in(payload) {
            self.add(category, name);
        }
    }

    /// Walk an install in the background and stream its names in.
    ///
    /// Idempotent per install root: re-opening the same game does not start a
    /// second walk over the same archives.
    pub fn start_scan(
        &mut self,
        install_root: PathBuf,
        crypto: Arc<dyn RpfCryptoProvider>,
        mount: or_title::MountOptions,
    ) {
        if self.scanned_root.as_deref() == Some(install_root.as_path()) {
            return;
        }
        self.stop_bg_scan();

        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx): (Sender<NameBatch>, Receiver<NameBatch>) = channel();
        self.bg_rx = Some(rx);
        self.bg_cancel = Some(Arc::clone(&cancel));
        self.scanned_root = Some(install_root.clone());
        self.scan_found = 0;

        let handle = std::thread::spawn(move || {
            let install = GtaInstall {
                root: install_root.clone(),
            };
            let game = match build_mounted_game_with(&install, crypto, DlcCutoff::All, mount) {
                Ok(game) => game,
                Err(e) => {
                    tracing::warn!(
                        install = %install_root.display(),
                        error = %e,
                        "Name scan could not mount the install; the pickers stay empty"
                    );
                    return;
                }
            };
            // ⚠ The mount report has nowhere else to go. Nothing in `src/`
            // reads `Diagnostics` — the CLI is the only consumer that prints
            // them — so the log is where a user can see which archives came
            // from their overlay and paste it into a bug report.
            for finding in game.diagnostics.messages() {
                tracing::info!(mount = %finding, "Install mount");
            }
            let mut sent = 0usize;
            scan_install_names(
                &game,
                &cancel,
                or_title::install::names::DEFAULT_BATCH,
                |batch| {
                    sent += batch.len();
                    let converted: NameBatch = batch
                        .into_iter()
                        .map(|(kind, name)| (NameIndexCategory::from(kind), name))
                        .collect();
                    let _ = tx.send(converted);
                },
            );
            tracing::info!(
                install = %install_root.display(),
                names = sent,
                "Name scan finished"
            );
        });
        self.bg_handle = Some(handle);
    }

    /// Take the names the scan has sent; once it has ended, how many it sent in all.
    pub fn drain_bg(&mut self) -> Option<usize> {
        let mut pending: Vec<NameBatch> = Vec::new();
        let mut ended = false;
        if let Some(rx) = self.bg_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(batch) => pending.push(batch),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended = true;
                        break;
                    }
                }
            }
        }
        for batch in pending {
            self.scan_found += batch.len();
            for (category, name) in batch {
                self.add(category, name);
            }
        }
        if !ended {
            return None;
        }
        self.bg_rx = None;
        self.bg_cancel = None;
        let _ = self.bg_handle.take();
        Some(self.scan_found)
    }

    /// The names the pickers offer, the snapshot they read and the hash index over them.
    ///
    /// ⚠ The hash index is counted from its names, which its crate gives no size for: two entries
    /// for each, in a table grown one entry at a time, holding each name twice at the average
    /// length of the names here.
    pub fn held_bytes(&self) -> usize {
        let sets = [&self.models, &self.textures, &self.clip_sets, &self.effects];
        let (mut count, mut text, mut own) = (0, 0, std::mem::size_of::<Self>());
        for set in sets {
            own += hash_table_bytes::<String>(set.capacity());
            for name in set {
                (count, text, own) = (count + 1, text + name.len(), own + name.capacity());
            }
        }
        let snapshot = |names: &Arc<Vec<String>>| {
            let strings: usize = names.iter().map(String::capacity).sum();
            names.capacity() * std::mem::size_of::<String>() + strings
        };
        let snapshots = self.cached_snapshot.as_ref().map_or(0, |snap| {
            [&snap.models, &snap.textures, &snap.clip_sets, &snap.effects]
                .map(snapshot)
                .iter()
                .sum()
        });
        let average = text / count.max(1);
        let entries = 2 * self.hashes.names();
        let collisions = self.hashes.collisions().len();
        own + snapshots
            + hash_table_bytes_for_len::<((HashFlavour, u32), String)>(entries)
            + entries * average
            + collisions * (std::mem::size_of::<Collision>() + 2 * average)
    }

    pub fn is_scanning(&self) -> bool {
        self.bg_rx.is_some()
    }

    /// The scan in flight, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        self.is_scanning().then(|| RunningJob {
            kind: JobKind::NameScan,
            doing: "Reading the install's names for the pickers".into(),
            fraction: None,
            detail: format!("{} name(s)", self.scan_found),
            stopping: false,
        })
    }

    pub fn stop_bg_scan(&mut self) {
        if let Some(c) = self.bg_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = self.bg_handle.take();
        self.bg_rx = None;
        self.scanned_root = None;
    }
}

fn normalise(name: impl Into<String>) -> String {
    let mut s = name.into();
    if let Some((stem, _)) = s.rsplit_once('.') {
        s = stem.to_string();
    }
    s
}

impl Drop for NameIndex {
    fn drop(&mut self) {
        self.stop_bg_scan();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// The install scan's categories map onto the picker's, one for one.
    #[test]
    fn every_kind_the_install_scan_emits_lands_in_a_scanned_category() {
        for kind in InstalledNameKind::ALL {
            let category = NameIndexCategory::from(*kind);
            let producer = PRODUCER_OF
                .iter()
                .find(|(c, _)| *c == category)
                .map(|(_, p)| *p);
            assert_eq!(
                producer,
                Some(NameProducer::InstallScan),
                "{kind:?} arrives from the install walk and lands in \
                 {category:?}, which PRODUCER_OF says comes from somewhere else"
            );
        }
    }

    /// And the converse: a category the scan cannot reach must not claim to
    /// come from it.
    #[test]
    fn a_category_the_install_scan_cannot_reach_is_fed_by_a_document() {
        let scanned: Vec<NameIndexCategory> = InstalledNameKind::ALL
            .iter()
            .map(|k| NameIndexCategory::from(*k))
            .collect();
        for (category, producer) in PRODUCER_OF {
            if scanned.contains(category) {
                continue;
            }
            assert_eq!(
                *producer,
                NameProducer::OpenDocument,
                "{category:?} is not something an archive listing can see"
            );
        }
    }

    fn ypt_with_rules(names: &[&str]) -> DocPayload {
        let mut list = or_io::gta5::PtxFxList::default();
        for name in names {
            or_particle::list::add_effect(&mut list, name);
        }
        DocPayload::Resource(or_io::Asset::Particles(list))
    }

    #[test]
    fn opening_a_particle_file_supplies_the_effect_rules_it_carries() {
        let mut index = NameIndex::new();
        assert!(index.snapshot().names(NameIndexCategory::Effect).is_empty());

        index.absorb(&ypt_with_rules(&["exp_grd_bzgas", "", "veh_exhaust"]));

        let snapshot = index.snapshot();
        assert_eq!(
            snapshot.names(NameIndexCategory::Effect),
            ["exp_grd_bzgas", "veh_exhaust"],
            "an unnamed rule is not a name"
        );
    }

    #[test]
    fn a_payload_with_no_names_contributes_none() {
        let mut index = NameIndex::new();
        index.absorb(&DocPayload::Gxt2(Default::default()));
        for category in NameIndexCategory::ALL {
            assert!(index.snapshot().names(*category).is_empty());
        }
    }

    /// The receiver `drain_bg` reads is the one the scan's sender writes, and
    /// nothing else in this file constructs one.
    #[test]
    fn a_drained_batch_reaches_the_snapshot() {
        let mut index = NameIndex::new();
        let (tx, rx) = channel();
        index.bg_rx = Some(rx);

        tx.send(vec![
            (NameIndexCategory::Model, "prop_bench_01a".to_string()),
            (NameIndexCategory::Texture, "vehshare".to_string()),
            (NameIndexCategory::ClipSet, "move_m@generic".to_string()),
        ])
        .expect("the receiver is alive");
        index.drain_bg();

        let snapshot = index.snapshot();
        assert_eq!(snapshot.names(NameIndexCategory::Model), ["prop_bench_01a"]);
        assert_eq!(snapshot.names(NameIndexCategory::Texture), ["vehshare"]);
        assert_eq!(
            snapshot.names(NameIndexCategory::ClipSet),
            ["move_m@generic"]
        );
    }

    #[test]
    fn a_name_is_stored_without_its_extension_and_only_once() {
        let mut index = NameIndex::new();
        index.add(NameIndexCategory::Model, "prop_bench_01a.ydr");
        index.add(NameIndexCategory::Model, "prop_bench_01a");
        assert_eq!(index.snapshot().names(NameIndexCategory::Model).len(), 1);
    }

    #[test]
    fn stopping_a_scan_cancels_it_and_forgets_the_root() {
        let mut index = NameIndex::new();
        let cancel = Arc::new(AtomicBool::new(false));
        index.bg_cancel = Some(Arc::clone(&cancel));
        index.scanned_root = Some(PathBuf::from("D:/whatever"));
        let (_tx, rx) = channel::<NameBatch>();
        index.bg_rx = Some(rx);

        index.stop_bg_scan();

        assert!(cancel.load(Ordering::Relaxed), "the worker is told to stop");
        assert!(index.bg_rx.is_none());
        assert!(index.scanned_root.is_none());
    }
    #[test]
    fn shared_words_resolve_before_an_install_or_document_is_opened() {
        let mut names = NameIndex::new();
        let hash = or_io::hash::string_hash(b"diffuse");
        assert_eq!(names.name_of(hash), Some("diffuse"));
        names.retarget("gta5");
        assert_eq!(names.name_of(hash), Some("diffuse"));
    }
}
