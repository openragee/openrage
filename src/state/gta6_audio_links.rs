//! What GTA VI audio game data links to among the bodies the scan of its archive found: the audio
//! container each sound's bank stands for, and an audio configuration body's names from the name
//! pool after it, looked for on a worker as a row of the job list with its progress and Cancel.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content};
use or_io::gta6::audio_data::{AudioData, AudioDataKind};
use or_io::gta6::audio_links::{config_names, BankLink, WaveIndex};
use or_job::{Cancel, Progress};

use super::jobs::{JobKind, RunningJob};
use super::rpf9_scan::{row_at_offset, scanned_body, Row, Rpf9ScanDocument};
use super::{AppState, DocPayload, DocumentId};

/// The most streams a container's table is read for: more than any container was measured to hold.
const MAX_STREAMS: u32 = 1 << 20;
/// The bodies after an audio configuration body tried as its name pool.
const POOL_TRIES: usize = 4;
/// The bytes a candidate name pool is read to.
const POOL_LIMIT: u64 = 16 << 20;

/// Audio configuration names a name pool gave, by key, for every audio view while the app runs.
// ponytail: one map for the whole app, which no close empties; a name is checked against its key.
pub(crate) fn pool_names() -> MutexGuard<'static, BTreeMap<u32, String>> {
    static NAMES: Mutex<BTreeMap<u32, String>> = Mutex::new(BTreeMap::new());
    NAMES.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a body's links are looked for among.
enum Wanted {
    /// Each audio container's offset, flags and stream count, and each wave field's bank and stream id.
    Waves {
        containers: Vec<(u64, u16, u32)>,
        fields: Vec<(u32, u32)>,
    },
    /// The bodies that may be the name pool, nearest first, and the body they would name.
    Pool {
        candidates: Vec<u64>,
        data: Box<AudioData>,
    },
}

/// How a look for a body's links ended.
pub enum Linked {
    Waves {
        /// The container each bank stands for, or why none is named.
        banks: BTreeMap<u32, BankLink>,
        /// The stream ids of the body's waves that some container holds.
        found: BTreeSet<u32>,
        containers: usize,
        /// Wave fields whose bank's container holds their stream, of `fields`.
        linked: usize,
        fields: usize,
    },
    Names {
        pool: u64,
        count: usize,
    },
    /// No body tried holds a name for each item that hashes to the item's key.
    NoPool,
    Stopped,
    Failed(String),
}

/// A look for a body's links, on a worker.
pub struct AudioLinking {
    pub archive: PathBuf,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Linked>>>,
}

impl std::fmt::Debug for AudioLinking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioLinking")
            .field("archive", &self.archive)
            .finish_non_exhaustive()
    }
}

impl AudioLinking {
    fn start(archive: PathBuf, wanted: Wanted) -> Self {
        let linking = Self {
            archive,
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            linking.cancel.clone(),
            linking.progress.clone(),
            Arc::clone(&linking.ended),
        );
        let archive = linking.archive.clone();
        std::thread::spawn(move || {
            // A panic still ends the job, or the view waits on it for ever.
            let linked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    match File::open(&archive) {
                        Err(e) => Linked::Failed(format!("{}: {e}", archive.display())),
                        Ok(file) => match wanted {
                            Wanted::Waves { containers, fields } => {
                                waves(&file, &containers, &fields, &cancel, &progress)
                            }
                            Wanted::Pool { candidates, data } => {
                                pool(&file, &data, &candidates, &cancel, &progress)
                            }
                        },
                    }
                }))
                .unwrap_or_else(|_| {
                    Linked::Failed(tr!("It stopped on an internal error").to_string())
                });
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(linked);
        });
        linking
    }

    fn job(&self) -> Option<RunningJob> {
        if self.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::AudioLinks,
            doing: tr!("Looking in the scan for what GTA VI audio game data links to").to_string(),
            fraction: Some(self.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: self.cancel.is_set(),
        })
    }

    /// How the look ended, `None` while it runs.
    pub(crate) fn ended(&self) -> MutexGuard<'_, Option<Linked>> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Drop for AudioLinking {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

fn waves(
    file: &File,
    containers: &[(u64, u16, u32)],
    fields: &[(u32, u32)],
    cancel: &Cancel,
    progress: &Progress,
) -> Linked {
    let mut by_bank: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for &(bank, stream) in fields {
        by_bank.entry(bank).or_default().insert(stream);
    }
    let wanted: BTreeSet<u32> = fields.iter().map(|f| f.1).collect();
    let mut index = WaveIndex::default();
    for (done, &(offset, flags, streams)) in containers.iter().enumerate() {
        if cancel.is_set() {
            return Linked::Stopped;
        }
        progress.reached(done as u64, containers.len() as u64);
        match stream_words(file, offset, flags, streams) {
            Ok(words) => index.add(offset, &words, &wanted),
            Err(why) => return Linked::Failed(why),
        }
    }
    let found: BTreeSet<u32> = (wanted.into_iter()).filter(|s| index.holds(*s)).collect();
    let banks: BTreeMap<u32, BankLink> = (by_bank.iter())
        .map(|(bank, streams)| (*bank, index.bank(streams)))
        .collect();
    let linked = (fields.iter())
        .filter(|(bank, stream)| {
            matches!(banks.get(bank), Some(BankLink::Container(_))) && found.contains(stream)
        })
        .count();
    Linked::Waves {
        banks,
        found,
        containers: containers.len(),
        linked,
        fields: fields.len(),
    }
}

/// The stream table's words of the container at `offset`, which the scan listed with `flags` and
/// `streams`.
pub(super) fn stream_words(
    mut file: &File,
    offset: u64,
    flags: u16,
    streams: u32,
) -> Result<Vec<u32>, String> {
    if streams > MAX_STREAMS {
        return Err(tr!(
            "the audio container at {at} states {count} streams",
            at = format!("{offset:#X}"),
            count = streams
        ));
    }
    let n = streams as usize;
    let at = offset + or_io::awc::stream_table_offset(flags, n) as u64;
    let mut table = vec![0u8; 4 * n];
    (file.seek(SeekFrom::Start(at)))
        .and_then(|_| file.read_exact(&mut table))
        .map_err(|e| format!("{offset:#X}: {e}"))?;
    Ok((table.as_chunks::<4>().0.iter())
        .map(|w| u32::from_le_bytes(*w))
        .collect())
}

fn pool(
    file: &File,
    data: &AudioData,
    candidates: &[u64],
    cancel: &Cancel,
    progress: &Progress,
) -> Linked {
    for (done, offset) in candidates.iter().enumerate() {
        if cancel.is_set() {
            return Linked::Stopped;
        }
        progress.reached(done as u64, candidates.len() as u64);
        let Ok((_, bytes)) = read_rpf9_body(file, *offset, POOL_LIMIT) else {
            continue;
        };
        if let Ok(names) = config_names(data, &bytes) {
            let count = names.len();
            pool_names().extend(names);
            return Linked::Names {
                pool: *offset,
                count,
            };
        }
    }
    Linked::NoPool
}

/// The open scans of `archive`.
fn scans_of(state: &AppState, archive: &Path) -> Vec<(DocumentId, Arc<Rpf9ScanDocument>)> {
    (state.documents.iter())
        .filter_map(|d| match &d.payload {
            DocPayload::Rpf9(scan) if scan.path == archive => Some((d.id, Arc::clone(scan))),
            _ => None,
        })
        .collect()
}

impl AppState {
    /// Look for what the GTA VI audio game data `doc` links to among the bodies the open scans of
    /// its archive found: its sounds' audio containers, or its name pool.
    pub(crate) fn link_gta6_audio(&mut self, doc: DocumentId) -> Result<(), String> {
        let Some(d) = self.document(doc) else {
            return Ok(());
        };
        let DocPayload::Gta6AudioData(audio) = &d.payload else {
            return Ok(());
        };
        let (archive, offset) = (d.path.as_deref())
            .and_then(scanned_body)
            .map(|(archive, offset)| (archive.to_path_buf(), offset))
            .ok_or_else(|| tr!("This audio game data is no body of a scanned archive, so no scan holds what it links to.").to_string())?;
        let scans = scans_of(self, &archive);
        if scans.is_empty() {
            return Err(tr!(
                "Open the scan of {archive}: what this body links to is looked for among the bodies it finds.",
                archive = archive.display()
            ));
        }
        let wanted = match audio.data.kind {
            AudioDataKind::Sounds => {
                let mut containers = BTreeMap::new();
                for (_, scan) in &scans {
                    for row in &scan.found().rows {
                        if let Row::Body(find) = row {
                            if let Rpf9BodyKind::Awc { flags, streams, .. } = find.kind {
                                containers.insert(find.offset, (flags, streams));
                            }
                        }
                    }
                }
                if containers.is_empty() {
                    return Err(tr!(
                        "The scan of {archive} has found no audio container yet.",
                        archive = archive.display()
                    ));
                }
                Wanted::Waves {
                    containers: (containers.into_iter())
                        .map(|(at, (flags, streams))| (at, flags, streams))
                        .collect(),
                    fields: (audio.items.iter())
                        .flat_map(|item| &item.waves)
                        .map(|w| (w.bank, w.stream_id()))
                        .collect(),
                }
            }
            AudioDataKind::AudioConfig => Wanted::Pool {
                candidates: (scans.iter())
                    .find_map(|(_, scan)| pool_candidates(&scan.found().rows, offset))
                    .unwrap_or_default(),
                data: Box::new(audio.data.clone()),
            },
            _ => return Ok(()),
        };
        let linking = Arc::new(AudioLinking::start(archive, wanted));
        if let Some(DocPayload::Gta6AudioData(audio)) =
            self.document_mut(doc).map(|d| &mut d.payload)
        {
            audio.linking = Some(linking);
        }
        Ok(())
    }

    /// Open the audio container at `container` of `archive` from a scan that lists it, saying
    /// which of its streams is `stream`.
    pub(crate) fn open_gta6_stream(&mut self, archive: &Path, container: u64, stream: u32) {
        let at = scans_of(self, archive).into_iter().find_map(|(doc, scan)| {
            let found = scan.found();
            let row = row_at_offset(&found.rows, container)?;
            let listed = matches!(found.rows.get(row), Some(Row::Body(f)) if f.offset == container);
            listed.then_some((doc, row))
        });
        let Some((scan, row)) = at else {
            self.show_error_toast(tr!(
                "No open scan lists the audio container at {at}.",
                at = format!("{container:#X}")
            ));
            return;
        };
        self.open_scanned_body(scan, row);
        self.show_info_toast(tr!(
            "The sound's wave is the stream {stream} of this audio container.",
            stream = format!("0x{stream:08X}")
        ));
    }

    /// The looks still running, as the job list draws them.
    pub(super) fn audio_linking_jobs(&self) -> impl Iterator<Item = RunningJob> + '_ {
        (self.documents.iter()).filter_map(|d| match &d.payload {
            DocPayload::Gta6AudioData(audio) => audio.linking.as_ref()?.job(),
            _ => None,
        })
    }

    /// Ask every look to stop.
    pub(super) fn stop_audio_linking(&self) {
        for d in &self.documents {
            if let DocPayload::Gta6AudioData(audio) = &d.payload {
                if let Some(linking) = &audio.linking {
                    linking.cancel.set();
                }
            }
        }
    }
}

/// The zlib bodies after the body at `offset` of `rows`, up to the next audio game data, that may
/// be its name pool; `None` when `rows` does not list the body.
fn pool_candidates(rows: &[Row], offset: u64) -> Option<Vec<u64>> {
    let at = row_at_offset(rows, offset)?;
    if !matches!(rows.get(at), Some(Row::Body(f)) if f.offset == offset) {
        return None;
    }
    let after = rows.get(at + 1..).unwrap_or_default().iter();
    Some(
        after
            .filter_map(|row| match row {
                Row::Body(f) if f.container.is_none() => Some(f),
                _ => None,
            })
            .map_while(|f| match f.kind {
                Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::AudioData => None,
                kind => Some((f.offset, matches!(kind, Rpf9BodyKind::Zlib(_)))),
            })
            .filter_map(|(at, zlib)| zlib.then_some(at))
            .take(POOL_TRIES)
            .collect(),
    )
}

#[cfg(test)]
#[path = "gta6_audio_links_tests.rs"]
mod tests;
