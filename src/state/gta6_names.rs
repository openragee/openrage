//! The names GTA VI archives' bodies spell, as the app keeps and serves them: the vocabulary of
//! every archive harvested, which names a hash a map or a texture pack states, and the name each
//! drawable and fragment states of itself, by its offset, which a scan's list shows beside it. A
//! harvest of a folder's archives runs as a job, and what it finds is served as soon as it ends.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::RPF9_MAGIC;
use or_io::gta6::names::{self, load_labels, Labels, Vocabulary, HARVEST_PREFIX};
use or_job::{Cancel, Progress};

use super::gta6_texture_naming::{NamingEnded, TextureNaming};
use super::jobs::file_name;
use super::AppState;

static HELD: Mutex<BTreeMap<PathBuf, Arc<Labels>>> = Mutex::new(BTreeMap::new());

/// The vocabulary served, and whether the kept one was asked for this session.
#[derive(Default)]
struct Served {
    vocabulary: Arc<Vocabulary>,
    asked: bool,
}

static SERVED: Mutex<Option<Served>> = Mutex::new(None);
/// Whether a harvest runs, which the folder's bar says and one harvest at a time needs.
static HARVESTING: AtomicBool = AtomicBool::new(false);

fn held() -> MutexGuard<'static, BTreeMap<PathBuf, Arc<Labels>>> {
    HELD.lock().unwrap_or_else(|p| p.into_inner())
}

fn served() -> MutexGuard<'static, Option<Served>> {
    SERVED.lock().unwrap_or_else(|p| p.into_inner())
}

/// The names kept of the bodies of `archive`, by offset; none where none were kept.
// Read once an archive until a harvest ends, which forgets them all.
pub(crate) fn labels(archive: &Path) -> Arc<Labels> {
    let mut held = held();
    if let Some(labels) = held.get(archive) {
        return labels.clone();
    }
    let labels =
        super::prefs::config_root().map_or_else(Labels::new, |root| load_labels(&root, archive));
    let labels = Arc::new(labels);
    held.insert(archive.to_path_buf(), labels.clone());
    labels
}

/// The vocabulary of every archive harvested: empty until the kept one is read, on a worker the
/// first ask starts, since it holds every name.
pub(crate) fn vocabulary() -> Arc<Vocabulary> {
    let mut slot = served();
    let served = slot.get_or_insert_with(Served::default);
    if !std::mem::replace(&mut served.asked, true) {
        let root = super::prefs::config_root();
        std::thread::spawn(move || {
            let kept = root.map_or_else(Vocabulary::default, |root| {
                Vocabulary::load(&names::vocabulary_file(&root))
            });
            // A harvest that ended meanwhile serves more than the file read before it.
            let corpus = !corpus().is_empty();
            if (!kept.is_empty() || corpus) && vocabulary().is_empty() {
                serve(kept);
            }
        });
    }
    Arc::clone(&served.vocabulary)
}

/// Where the recovered-names corpus lies: a folder whose `vocabulary.txt` holds one name a line.
const CORPUS: &str = "OPENRAGE_GTA6_NAMES";

/// The recovered-names corpus the folder [`CORPUS`] names, read once: names the game's files state that no harvest of this install gave.
fn corpus() -> &'static Vocabulary {
    static READ: std::sync::OnceLock<Vocabulary> = std::sync::OnceLock::new();
    READ.get_or_init(|| {
        let Some(folder) = std::env::var_os(CORPUS) else {
            return Vocabulary::default();
        };
        let file = Path::new(&folder).join("vocabulary.txt");
        let kept = Vocabulary::load(&file);
        if !kept.is_empty() {
            return kept;
        }
        let mut read = Vocabulary::default();
        for line in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with("//") {
                read.add(line, None);
            }
        }
        read
    })
}

/// Serve `vocabulary` and the corpus from now on, and read each archive's labels again when next asked.
fn serve(mut vocabulary: Vocabulary) {
    vocabulary.merge(corpus());
    let now = Served {
        vocabulary: Arc::new(vocabulary),
        asked: true,
    };
    *served() = Some(now);
    held().clear();
}

/// The name the 32-bit `hash` names alone in the vocabulary.
pub(crate) fn name32(hash: u32) -> Option<String> {
    vocabulary().name32(hash).map(str::to_string)
}

/// Whether a harvest runs now.
pub(crate) fn harvesting() -> bool {
    HARVESTING.load(Ordering::Relaxed)
}

/// Says no harvest runs once its worker ends, however it ends.
struct Harvesting;

impl Drop for Harvesting {
    fn drop(&mut self) {
        HARVESTING.store(false, Ordering::Relaxed);
    }
}

/// Whether the file at `path` starts as a GTA VI archive does.
fn is_rpf9(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head));
    read.is_ok() && u32::from_le_bytes(head) == RPF9_MAGIC
}

/// What a harvest of `archives` gave: the names new to `vocabulary`, the bodies that state their
/// own name, and whether `cancel` stopped it; the labels of each archive read whole are kept
/// under `config`.
fn harvest_into(
    vocabulary: &mut Vocabulary,
    archives: &[PathBuf],
    config: &Path,
    threads: usize,
    cancel: &Cancel,
    progress: &Progress,
) -> (usize, usize, bool) {
    let sizes: Vec<u64> = (archives.iter())
        .map(|path| std::fs::metadata(path).map_or(0, |m| m.len()))
        .collect();
    let total = sizes.iter().sum::<u64>().max(1);
    let (before, mut done, mut labelled) = (vocabulary.len(), 0u64, 0);
    for (path, &size) in archives.iter().zip(&sizes) {
        if cancel.is_set() {
            break;
        }
        progress.now_on(file_name(path));
        if is_rpf9(path) {
            let stop = || cancel.is_set();
            let reached = |at: u64, _: u64| progress.reached(done + at.min(size), total);
            match names::harvest_archive(path, HARVEST_PREFIX, threads, &stop, &reached) {
                Ok(gave) => {
                    vocabulary.merge(&gave.vocabulary);
                    if !gave.stopped {
                        labelled += gave.labels.len();
                        if let Err(error) = names::save_labels(config, path, &gave.labels) {
                            tracing::warn!(%error, archive = %path.display(), "the names its bodies state were not kept");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, archive = %path.display(), "the archive was not harvested for names");
                }
            }
        }
        done += size;
        progress.reached(done, total);
    }
    (vocabulary.len() - before, labelled, cancel.is_set())
}

/// Learn, for each key of `keys`, the name `vocabulary` holds for it, then the names the naming
/// rules give from each name it holds with an ending; how many keys were named, and whether
/// `cancel` stopped it.
fn name_textures(
    vocabulary: &Vocabulary,
    keys: &HashSet<u64>,
    cancel: &Cancel,
    progress: &Progress,
) -> (usize, bool) {
    let spelled =
        (keys.iter()).filter_map(|key| Some((*key, vocabulary.name64(*key)?.to_string())));
    let mut named = super::ttpf_document::learn_named(spelled);
    let stems: Vec<&str> = vocabulary.resource_names().collect();
    for (done, stem) in stems.iter().enumerate() {
        if cancel.is_set() {
            return (named, true);
        }
        progress.reached(done as u64, stems.len() as u64);
        named += super::ttpf_document::learn_rule_names(stem, keys);
    }
    (named, false)
}

impl AppState {
    /// Harvest, as a job, the names the bodies of every GTA VI archive under `root` spell into
    /// the kept vocabulary, serve it, and name the keys of the folder's texture packs from it.
    pub fn harvest_gta6_names(&mut self, root: &Path) {
        let Some(config) = super::prefs::config_root() else {
            self.show_info_toast(tr!(
                "The names cannot be kept: this run has no configuration folder"
            ));
            return;
        };
        if HARVESTING.swap(true, Ordering::Relaxed) {
            return;
        }
        let packs = super::gta6_texture_index::pack_keys(root);
        let doing = tr!(
            "Harvesting the names the GTA VI archives under {folder} spell",
            folder = root.display()
        );
        let root = root.to_path_buf();
        let run = TextureNaming::run(doing, move |cancel, progress| {
            let _running = Harvesting;
            let (archives, _) = super::gta6_install_scan::archives_under(&root);
            let file = names::vocabulary_file(&config);
            let mut vocabulary = Vocabulary::load(&file);
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let (new, labelled, stopped) = harvest_into(
                &mut vocabulary,
                &archives,
                &config,
                threads,
                cancel,
                progress,
            );
            // What the archives read before a stop spell is kept too.
            if let Err(error) = vocabulary.save(&file) {
                tracing::warn!(%error, "the harvested names were not kept");
            }
            let held = vocabulary.len();
            let keys = match stopped {
                true => None,
                false => packs(cancel, progress),
            };
            let named = keys.map(|keys| name_textures(&vocabulary, &keys, cancel, progress));
            serve(vocabulary);
            match named {
                Some((keys, false)) => NamingEnded::Names {
                    names: held,
                    new,
                    labelled,
                    keys,
                },
                _ => NamingEnded::Stopped,
            }
        });
        self.texture_naming.push(run);
    }

    /// Harvest the names of the GTA VI folder open in a tab, the tab in front first; a toast
    /// where no tab holds one.
    pub fn harvest_names_of_the_open_gta6_folder(&mut self) {
        let is_gta6 = |folder: &PathBuf| {
            folder.is_dir()
                && super::gta_vi::opens_archives_individually(or_title::Game::detect_from_root(
                    folder,
                ))
        };
        let tabs = (self.active_document().into_iter()).chain(self.documents.iter());
        let root = tabs.filter_map(|doc| doc.path.clone()).find(is_gta6);
        match root {
            Some(root) => self.harvest_gta6_names(&root),
            None => self.show_info_toast(tr!(
                "Open a GTA VI folder first: its archives are what the names are harvested from"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use or_io::container::rpf::{write_rpf9_resource, Rpf9Form, Rpf9Sections};
    use or_io::gta6::hash::{name_hash32, name_hash64};

    use super::*;

    #[test]
    fn the_names_kept_of_an_archive_are_served_and_an_archive_with_none_has_none() {
        let root = super::super::prefs::config_root().expect("a test has a configuration folder");
        let folder = tempfile::tempdir().expect("a scratch folder");
        let (named, bare) = (
            folder.path().join("zq_named.rpf"),
            folder.path().join("zq_bare.rpf"),
        );
        for archive in [&named, &bare] {
            std::fs::write(archive, [0u8; 32]).expect("a made-up archive");
        }
        let kept = Labels::from([(0x10, "zq_shed_07.#dr".to_string())]);
        or_io::gta6::names::save_labels(&root, &named, &kept).expect("the names are kept");
        assert_eq!(*labels(&named), kept);
        assert!(labels(&bare).is_empty());
    }

    /// A made-up drawable that states `name`, as a loose resource file: a root whose 0x08 points
    /// at its block map, 0x10 and 0x50 at a record, and 0xA8 at the name.
    fn drawable_file(name: &str) -> Vec<u8> {
        let mut v = vec![0u8; 0x100];
        for (at, target) in [(0x08, 0xE0u64), (0x10, 0xC0), (0x50, 0xC0), (0xA8, 0xD0)] {
            v[at..at + 8].copy_from_slice(&(0x5000_0000 + target).to_le_bytes());
        }
        assert!(name.len() < 0x10, "the name lies before the block map");
        v[0xD0..0xD0 + name.len()].copy_from_slice(name.as_bytes());
        let sections = Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: Vec::new(),
        };
        write_rpf9_resource(&sections).expect("a resource")
    }

    /// A made-up archive: an RPF9 magic, then each of `bodies` at a page of its own.
    fn archive_file(path: &Path, bodies: &[Vec<u8>]) -> Vec<u64> {
        let mut bytes = RPF9_MAGIC.to_le_bytes().to_vec();
        let mut offsets = Vec::new();
        for body in bodies {
            bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
            offsets.push(bytes.len() as u64);
            bytes.extend_from_slice(body);
        }
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        std::fs::write(path, bytes).expect("a made-up archive");
        offsets
    }

    #[test]
    fn a_harvest_keeps_the_names_and_labels_of_each_archive_and_a_stop_ends_it() {
        let config = tempfile::tempdir().expect("a configuration folder");
        let folder = tempfile::tempdir().expect("a scratch folder");
        let archive = folder.path().join("zq_models.rpf");
        let other = folder.path().join("zq_other.rpf");
        let offsets = archive_file(
            &archive,
            &[drawable_file("zq_hut_03.#dr"), drawable_file("zq_mast.#dr")],
        );
        // Another archive generation: no scan reads it.
        std::fs::write(&other, b"RPF7 zq_never_read.#dr\0").expect("a made-up file");

        let (cancel, progress) = (Cancel::new(), Progress::new());
        let mut vocabulary = Vocabulary::default();
        let archives = [archive.clone(), other];
        let (new, labelled, stopped) = harvest_into(
            &mut vocabulary,
            &archives,
            config.path(),
            2,
            &cancel,
            &progress,
        );
        assert!(!stopped);
        assert_eq!(labelled, 2);
        assert_eq!(new, vocabulary.len());
        assert_eq!(
            vocabulary.confirmed(name_hash32(b"zq_hut_03"), name_hash64(b"zq_hut_03")),
            Some("zq_hut_03")
        );
        assert_eq!(vocabulary.name64(name_hash64(b"zq_never_read")), None);
        let wanted = Labels::from([
            (offsets[0], "zq_hut_03.#dr".to_string()),
            (offsets[1], "zq_mast.#dr".to_string()),
        ]);
        assert_eq!(load_labels(config.path(), &archive), wanted);
        let now = progress.snapshot();
        assert_eq!(now.attempted, now.total);

        cancel.set();
        let mut none = Vocabulary::default();
        let read = harvest_into(&mut none, &archives, config.path(), 2, &cancel, &progress);
        assert_eq!(read, (0, 0, true));
        assert!(none.is_empty());
    }

    #[test]
    fn the_keys_a_vocabulary_names_are_learned_by_a_name_held_and_by_the_rules() {
        let mut vocabulary = Vocabulary::default();
        vocabulary.add("zq_names_job_wall", Some(*b"dr"));
        vocabulary.add("zq_names_job_decal", None);
        let spelled = name_hash64(b"zq_names_job_decal");
        let ruled = name_hash64(b"zq_names_job_wall_nm");
        let keys = HashSet::from([spelled, ruled, 7]);
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let (named, stopped) = name_textures(&vocabulary, &keys, &cancel, &progress);
        assert_eq!((named, stopped), (2, false));
        let known = crate::state::ttpf_document::known_name;
        assert_eq!(known(spelled).as_deref(), Some("zq_names_job_decal"));
        assert_eq!(known(ruled).as_deref(), Some("zq_names_job_wall_nm"));
        assert_eq!(known(7), None);
    }

    #[test]
    fn a_harvest_from_the_app_serves_its_names_without_a_restart() {
        let config = tempfile::tempdir().expect("a configuration folder");
        let folder = tempfile::tempdir().expect("a scratch folder");
        crate::state::prefs::keep_prefs_in(config.path());
        let archive = folder.path().join("zq_served.rpf");
        let offsets = archive_file(&archive, &[drawable_file("zq_served_7.#dr")]);
        // Asked before the harvest: the row has no name yet.
        assert!(labels(&archive).is_empty());

        let mut state = AppState::new();
        state.harvest_gta6_names(folder.path());
        let until = Instant::now() + Duration::from_secs(60);
        while harvesting() || state.texture_naming_jobs().next().is_some() {
            assert!(Instant::now() < until, "the harvest never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        let hash = name_hash32(b"zq_served_7");
        assert_eq!(name32(hash).as_deref(), Some("zq_served_7"));
        assert_eq!(
            labels(&archive).get(&offsets[0]).map(String::as_str),
            Some("zq_served_7.#dr")
        );
        let kept = Vocabulary::load(&names::vocabulary_file(config.path()));
        assert_eq!(kept.name32(hash), Some("zq_served_7"));
    }
}
