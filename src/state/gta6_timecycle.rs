//! The look of the day a GTA VI install states: the time table, the weather cycles, the weather
//! set, its modifiers and the tuning values among the nameless bodies of its common archive, read
//! on a worker once a session and held in memory.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Scan};
use or_title::TimecycleData;

use super::AppState;

/// The archive of an install the tables of the day are in.
const ARCHIVE: &str = "common.rpf";
/// The bytes one text is read to: a weather cycle is well under it.
const TEXT_LIMIT: u64 = 16 << 20;

type Read = Arc<OnceLock<Arc<TimecycleData>>>;

#[derive(Default)]
struct Held {
    /// The archive found for the document asked for last, so that a frame does not look again.
    asked: Option<(Option<PathBuf>, Option<PathBuf>)>,
    /// Each archive read or being read.
    read: Vec<(PathBuf, Read)>,
}

// ponytail: read again each session, about a second; kept beside the effects if it shows.
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// The install's folder where one is set, else the folder above the active document that holds
/// the archive.
fn archive_of(state: &AppState, document: Option<&Path>) -> Option<PathBuf> {
    let set = (state.prefs.game_paths.get(or_title::Game::GtaVI)).map(|root| root.join(ARCHIVE));
    set.filter(|archive| archive.is_file()).or_else(|| {
        (document?.ancestors().skip(1))
            .map(|folder| folder.join(ARCHIVE))
            .find(|archive| archive.is_file())
    })
}

/// Every text a scan of `archive` finds, XML or plain, as the tables of the day they state.
fn read(archive: &Path) -> Result<TimecycleData, String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    let mut texts = Vec::new();
    for find in Rpf9Scan::new(&file, 0..u64::MAX).map_err(|e| e.to_string())? {
        let find = find.map_err(|e| e.to_string())?;
        let text = matches!(find.kind, Rpf9BodyKind::Zlib(run)
            if matches!(run.content, Rpf9Content::Xml | Rpf9Content::Text));
        if !text {
            continue;
        }
        match read_rpf9_body(&file, find.offset, TEXT_LIMIT) {
            Ok((_, bytes)) => texts.push(String::from_utf8_lossy(&bytes).into_owned()),
            Err(error) => tracing::debug!(offset = find.offset, %error, "a text is not read"),
        }
    }
    Ok(or_title::timecycle_of_documents(
        texts.iter().map(String::as_str),
    ))
}

/// The tables of the day of the GTA VI install the active document is of, once a worker has read
/// them; `None` before, and where no install is found or it states none.
pub(crate) fn timecycle(state: &AppState) -> Option<Arc<TimecycleData>> {
    let document = state.active_document().and_then(|d| d.path.clone());
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    let held = held.get_or_insert_with(Held::default);
    if held.asked.as_ref().map(|(of, _)| of) != Some(&document) {
        let archive = archive_of(state, document.as_deref());
        held.asked = Some((document, archive));
    }
    let archive = held.asked.as_ref()?.1.clone()?;
    let slot = match held.read.iter().find(|(path, _)| *path == archive) {
        Some((_, slot)) => Arc::clone(slot),
        None => {
            let slot = Read::default();
            held.read.push((archive.clone(), Arc::clone(&slot)));
            let filled = Arc::clone(&slot);
            std::thread::spawn(move || {
                let data = read(&archive).unwrap_or_else(|why| {
                    tracing::warn!(archive = %archive.display(), %why, "GTA VI's tables of the day are not read");
                    TimecycleData::default()
                });
                tracing::info!(
                    cycles = data.timecycle.cycles.len(),
                    "GTA VI's tables of the day were read"
                );
                let _ = filled.set(Arc::new(data));
            });
            slot
        }
    };
    let data = slot.get()?;
    (!data.timecycle.cycles.is_empty()).then(|| Arc::clone(data))
}
