//! GTA VI's dialogue and radio as every view finds them while the app runs: the lines the
//! conversation bodies state, their texts by language, the streamed sounds of the sounds bodies
//! and the audio container that holds each stream. An index of the open scans fills them on a
//! worker, as a row of the job list; a body or a text table opened by hand adds what it holds.

use crate::state::Rpf9Document;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::audio_data::{AudioData, AudioDataKind};
use or_io::gta6::audio_radio::{self, Station, StationList, StreamedSound, TrackText};
use or_io::gta6::dialogue::{subtitles_in, Dialogue, DialogueLine};
use or_io::gta6::text::TextTable;
use or_io::gta6::text_language::{registered, LanguageArchives, Scanned};
use or_job::{Cancel, Progress};

use super::jobs::{JobKind, RunningJob};
use super::rpf9_scan::{body_path, Row, View};
use super::{AppState, DocPayload};

/// The bytes a conversation body, a text table or a sounds body is read to.
const BODY_LIMIT: u64 = 64 << 20;
/// A change set that registers languages is a few kilobytes: a longer XML body is not read for one.
const CHANGE_SET_LIMIT: u64 = 1 << 20;
/// The language the texts of a table opened by hand are kept under: its archive is not looked at.
const OPENED: &str = "";
/// The bits of a stream table's word that are the stream's id.
const STREAM_ID: u32 = 0x1FFF_FFFF;

/// What the dialogue view shows: a speaker's initials, a text a label or a subtitle holds, and
/// the language of the subtitles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    pub speaker: String,
    pub text: String,
    pub language: String,
}

/// The lines found so far and what joins them to their texts and their audio.
#[derive(Default)]
pub(crate) struct Lines {
    dialogue: Dialogue,
    by_label: BTreeMap<u32, DialogueLine>,
    /// Each line's label hash, in the order of its key and number.
    ordered: Vec<u32>,
    /// The label hash of the line each audio stream speaks.
    by_stream: BTreeMap<u32, u32>,
    /// The texts kept of each language: of a line's label, and of a key a view asked for.
    texts: BTreeMap<String, BTreeMap<u32, String>>,
    /// Keys that are no line's label whose texts are kept: a radio station's key names it.
    wanted: BTreeSet<u32>,
    /// The languages a change set registers text archives for, in its order.
    registered: Vec<String>,
    /// The most language archives one scanned file held when the scans were last indexed.
    places: usize,
    sounds: BTreeMap<u32, StreamedSound>,
    /// The archives of `containers`, and the archive and offset of the container of each stream.
    archives: Vec<PathBuf>,
    containers: BTreeMap<u32, (usize, u64)>,
    /// How the last index ended.
    pub said: Option<Result<String, String>>,
    pub filter: Filter,
    /// The lines the filter keeps, and the filter and the state of the lines they were kept for.
    shown: Vec<u32>,
    shown_for: Option<(Filter, u64)>,
    changes: u64,
}

// ponytail: one set of lines for the whole app, which no close empties, and the containers of
// one index replace those of the one before. A set for each install would lift both.
pub(crate) fn lines() -> MutexGuard<'static, Lines> {
    static LINES: LazyLock<Mutex<Lines>> = LazyLock::new(Mutex::default);
    LINES.lock().unwrap_or_else(|p| p.into_inner())
}

/// A language archive's place among its file's, as a table with no registered name is told.
fn place(ordinal: usize) -> String {
    format!("#{}", ordinal + 1)
}

/// The language of a table of the language archive `ordinal` of a file that holds `of`: its
/// registered name where as many are registered, else the archive's place.
fn language_of(registered: &[String], ordinal: usize, of: usize) -> String {
    match registered.get(ordinal) {
        Some(name) if of == registered.len() => name.clone(),
        _ => place(ordinal),
    }
}

impl Lines {
    /// Reads the lines again from the conversation bodies added.
    fn refresh(&mut self) {
        let mut found: Vec<DialogueLine> = self.dialogue.lines().collect();
        self.by_stream = (found.iter())
            .map(|l| (l.stream_id(), l.label_hash()))
            .collect();
        self.by_label = found.drain(..).map(|l| (l.label_hash(), l)).collect();
        let mut ordered: Vec<&DialogueLine> = self.by_label.values().collect();
        ordered.sort_by(|a, b| (&a.key, a.number).cmp(&(&b.key, b.number)));
        self.ordered = ordered.iter().map(|l| l.label_hash()).collect();
        self.changes += 1;
    }

    pub fn count(&self) -> usize {
        self.by_label.len()
    }

    pub fn line(&self, label: u32) -> Option<&DialogueLine> {
        self.by_label.get(&label)
    }

    /// The languages a view offers: those registered, else each place a scanned file held, then
    /// any other whose texts are kept.
    pub fn languages(&self) -> Vec<String> {
        let mut out = self.registered.clone();
        if out.is_empty() {
            out.extend((0..self.places).map(place));
        }
        for kept in self.texts.keys() {
            if kept != OPENED && !out.contains(kept) {
                out.push(kept.clone());
            }
        }
        out
    }

    /// The language read where none is picked: the first registered, else the first archive's.
    pub fn first_language(&self) -> String {
        (self.registered.first().cloned()).unwrap_or_else(|| place(0))
    }

    /// Whether texts of `language` are kept.
    pub fn holds(&self, language: &str) -> bool {
        self.texts.contains_key(language)
    }

    /// The text of `key` in `language`, else in a table opened by hand.
    pub fn text(&self, key: u32, language: &str) -> Option<&str> {
        ([language, OPENED].iter())
            .find_map(|language| self.texts.get(*language)?.get(&key))
            .map(String::as_str)
    }

    /// The text of `key` in the first language that holds one, the registered ones first.
    fn any_text(&self, key: u32) -> Option<&str> {
        let registered = self.registered.iter().filter_map(|l| self.texts.get(l));
        (registered.chain(self.texts.values()))
            .find_map(|texts| texts.get(&key))
            .map(String::as_str)
    }

    pub fn sound(&self, key: u32) -> Option<&StreamedSound> {
        self.sounds.get(&key)
    }

    /// The archive and offset of the audio container that holds `stream`.
    pub fn container(&self, stream: u32) -> Option<(&Path, u64)> {
        let (archive, at) = self.containers.get(&(stream & STREAM_ID))?;
        Some((self.archives.get(*archive)?.as_path(), *at))
    }

    /// The label hashes of the lines the filter kept when [`Self::filter_lines`] last ran.
    pub fn shown(&self) -> &[u32] {
        &self.shown
    }

    /// Keeps the lines the filter keeps, in order, again where the filter or the lines changed.
    pub fn filter_lines(&mut self) {
        let key = (self.filter.clone(), self.changes);
        if self.shown_for.as_ref() != Some(&key) {
            let speaker = self.filter.speaker.trim().to_ascii_uppercase();
            let text = self.filter.text.trim().to_lowercase();
            let keeps = |hash: &u32| {
                let Some(line) = self.by_label.get(hash) else {
                    return false;
                };
                if !speaker.is_empty() && line.speaker() != Some(speaker.as_str()) {
                    return false;
                }
                let held = |t: &str| t.to_lowercase().contains(&text);
                text.is_empty()
                    || held(line.label.as_str())
                    || self.text(*hash, &self.filter.language).is_some_and(held)
            };
            let shown: Vec<u32> = self.ordered.iter().copied().filter(keeps).collect();
            self.shown = shown;
            self.shown_for = Some(key);
        }
    }

    /// Keeps `texts` of `language`, beside those kept of it before.
    fn keep(&mut self, language: &str, texts: BTreeMap<u32, String>) {
        (self.texts.entry(language.to_string()).or_default()).extend(texts);
        self.changes += 1;
    }

    /// The keys whose texts are kept: each line's label and each key a view asked for.
    fn keys(&self) -> BTreeSet<u32> {
        (self.by_label.keys().chain(&self.wanted))
            .copied()
            .collect()
    }
}

/// Why the strict tree of each RBF0 body opened does not read, by the body's path.
fn notes() -> MutexGuard<'static, BTreeMap<PathBuf, String>> {
    static NOTES: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());
    NOTES.lock().unwrap_or_else(|p| p.into_inner())
}

/// What opening the metadata body `find` at `path` adds. An RBF0 body is read as its strict
/// element tree, and where that fails the body still opens as the lenient reader's tree, under
/// a note; a PSO body's dialogue lines are kept.
pub(super) fn opened(path: &Path, find: &Rpf9ScanFind, bytes: &[u8]) {
    let Rpf9BodyKind::Zlib(run) = find.kind else {
        return;
    };
    if run.content != Rpf9Content::Rbf {
        return note_body(bytes);
    }
    match or_io::gta6::ui_tree::read(bytes) {
        Ok(_) => notes().remove(path),
        Err(e) => notes().insert(
            path.to_path_buf(),
            tr!(
                "The strict RBF0 reader does not read this body ({e}), so the tree shown is the lenient reader's alone",
                e = e
            ),
        ),
    };
}

/// Why the tree of the RBF0 body at `path` is the lenient reader's alone; `None` where the strict
/// reader reads it too.
pub(crate) fn strict_note(path: &Path) -> Option<String> {
    notes().get(path).cloned()
}

/// Keeps the dialogue lines `body` states.
fn note_body(body: &[u8]) {
    let mut lines = lines();
    if lines.dialogue.add(body) > 0 {
        lines.refresh();
    }
}

/// Keeps the text `table` holds of each line found so far and of each key a view asked for,
/// where none is kept yet.
pub(super) fn note_table(table: &TextTable) {
    let mut lines = lines();
    let keys = lines.keys();
    let kept = lines.texts.entry(OPENED.to_string()).or_default();
    for (key, text) in subtitles_in(table, &keys) {
        kept.entry(key).or_insert_with(|| text.to_string());
    }
    lines.changes += 1;
}

/// The audio stream's name and the label of the line whose label hashes to `key`.
pub(crate) fn line_of(key: u32) -> Option<(String, String)> {
    let lines = lines();
    let line = lines.by_label.get(&key)?;
    Some((line.wave(), line.label.clone()))
}

/// The comments an exported audio stream `stream` carries: none unless it speaks a line found.
pub(crate) fn comments(stream: u32) -> Vec<(String, String)> {
    let lines = lines();
    let Some(hash) = lines.by_stream.get(&(stream & STREAM_ID)) else {
        return Vec::new();
    };
    let subtitle = lines.any_text(*hash);
    (lines.by_label.get(hash)).map_or_else(Vec::new, |line| line.comments(subtitle))
}

/// The language of the text table opened at `path` from a scan: the one its language archive is
/// registered for, else that archive's place among its file's; `None` where no open scan of the
/// whole archive lists it in a language archive.
// The archives are found again only when the scan's list has grown.
pub(crate) fn table_language(state: &AppState, path: &Path) -> Option<String> {
    static FOUND: Mutex<BTreeMap<PathBuf, (usize, LanguageArchives)>> = Mutex::new(BTreeMap::new());
    let archive = path.parent()?;
    let stem = path.file_stem()?.to_str()?;
    let at = u64::from_str_radix(stem.strip_prefix("0x")?, 16).ok()?;
    let scan = (state.documents.iter()).find_map(|d| match &d.payload {
        DocPayload::Rpf9(Rpf9Document::Scan(scan)) if scan.start == 0 && scan.path == archive => {
            Some(scan)
        }
        _ => None,
    })?;
    let found = scan.found();
    let mut kept = FOUND.lock().unwrap_or_else(|p| p.into_inner());
    let known = kept.entry(archive.to_path_buf()).or_default();
    if known.0 != found.rows.len() {
        let bodies = found.rows.iter().filter_map(|row| match row {
            Row::Body(find) => Some((find.offset, Scanned::of(find))),
            Row::Missing { .. } => None,
        });
        *known = (found.rows.len(), LanguageArchives::of(bodies));
    }
    let ordinal = known.1.ordinal(at)?;
    Some(language_of(&lines().registered, ordinal, known.1.len()))
}

/// What a music body states of the radio: its stations, the lists of them, and its tracks' texts.
pub(crate) struct Radio {
    pub stations: Vec<Station>,
    pub lists: Vec<StationList>,
    pub texts: BTreeMap<u32, TrackText>,
}

/// The radio of the music body `data` opened at `path`, read once; its stations' keys join the
/// keys whose texts are kept, since a station's key names it.
// ponytail: kept by path for the whole app, which no close empties; a body is read-only.
pub(crate) fn radio_of(path: &Path, data: &AudioData) -> Arc<Result<Radio, String>> {
    static RADIOS: Mutex<BTreeMap<PathBuf, Arc<Result<Radio, String>>>> =
        Mutex::new(BTreeMap::new());
    let mut radios = RADIOS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(radio) = radios.get(path) {
        return Arc::clone(radio);
    }
    let read = || -> Result<Radio, audio_radio::RadioError> {
        Ok(Radio {
            stations: audio_radio::stations(data)?,
            lists: audio_radio::station_lists(data)?,
            texts: audio_radio::track_texts(data)?,
        })
    };
    let read = read().map_err(|e| e.to_string());
    if let Ok(radio) = &read {
        let mut lines = lines();
        (lines.wanted).extend(radio.stations.iter().map(|s| s.key));
    }
    let radio = Arc::new(read);
    radios.insert(path.to_path_buf(), Arc::clone(&radio));
    radio
}

/// The stream a view asked to hear, by the path its audio container opens under.
fn asked() -> MutexGuard<'static, Option<(PathBuf, u32)>> {
    static ASKED: Mutex<Option<(PathBuf, u32)>> = Mutex::new(None);
    ASKED.lock().unwrap_or_else(|p| p.into_inner())
}

/// The stream of the audio container opened at `path` that a view asked to hear, once: its
/// place among `rows`, where a row or one of its channels has the stream's id.
pub(crate) fn play_asked(path: &Path, rows: &[crate::audio::AwcStreamRow]) -> Option<usize> {
    let mut asked = asked();
    let stream = match asked.as_ref() {
        Some((container, stream)) if container == path => *stream,
        _ => return None,
    };
    *asked = None;
    let is = |id: u32| id & STREAM_ID == stream;
    (rows.iter())
        .find(|row| is(row.id) || row.opus_channels.iter().any(|c| is(c.0)))
        .map(|row| row.stream)
}

/// What the scan of one archive found that the index reads.
#[derive(Debug, Default, PartialEq, Eq)]
struct Archive {
    path: PathBuf,
    conversations: Vec<u64>,
    /// Each text table of a language archive, and that archive's place among the file's.
    tables: Vec<(u64, usize)>,
    /// How many language archives the file holds.
    languages: usize,
    change_sets: Vec<u64>,
    audio_data: Vec<u64>,
    /// Each audio container's offset, flags and stream count.
    containers: Vec<(u64, u16, u32)>,
}

impl Archive {
    fn of(path: &Path, rows: &[Row]) -> Self {
        let finds: Vec<&Rpf9ScanFind> = (rows.iter())
            .filter_map(|row| match row {
                Row::Body(find) => Some(&**find),
                Row::Missing { .. } => None,
            })
            .collect();
        let languages = LanguageArchives::of(finds.iter().map(|f| (f.offset, Scanned::of(f))));
        let mut out = Self {
            path: path.to_path_buf(),
            languages: languages.len(),
            ..Self::default()
        };
        for find in finds {
            let at = find.offset;
            match find.kind {
                Rpf9BodyKind::Awc { flags, streams, .. } => {
                    out.containers.push((at, flags, streams))
                }
                Rpf9BodyKind::Zlib(run) => match run.content {
                    Rpf9Content::Pso => out.conversations.push(at),
                    Rpf9Content::AudioData => out.audio_data.push(at),
                    Rpf9Content::TextTable => {
                        out.tables.extend(languages.ordinal(at).map(|n| (at, n)))
                    }
                    Rpf9Content::Xml
                        if find.inflated_len.is_none_or(|len| len <= CHANGE_SET_LIMIT) =>
                    {
                        out.change_sets.push(at)
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        out
    }

    /// The bodies an index reads of it, with the tables of one of its languages.
    fn bodies(&self) -> usize {
        self.conversations.len()
            + self.change_sets.len()
            + self.audio_data.len()
            + self.containers.len()
            + self.tables.len() / self.languages.max(1)
    }
}

/// An index of the open scans, running on a worker.
struct Indexing {
    cancel: Cancel,
    progress: Progress,
}

fn indexing() -> MutexGuard<'static, Option<Indexing>> {
    static INDEXING: Mutex<Option<Indexing>> = Mutex::new(None);
    INDEXING.lock().unwrap_or_else(|p| p.into_inner())
}

/// Whether an index is running.
pub(crate) fn running() -> bool {
    indexing().is_some()
}

/// The index still running, as the job list draws it.
pub(super) fn job() -> Option<RunningJob> {
    let indexing = indexing();
    let run = indexing.as_ref()?;
    Some(RunningJob {
        kind: JobKind::AudioLinks,
        doing: tr!("Indexing the dialogue, texts and radio streams of the open GTA VI scans")
            .to_string(),
        fraction: Some(run.progress.snapshot().fraction()),
        detail: String::new(),
        stopping: run.cancel.is_set(),
    })
}

/// Ask the index to stop.
pub(super) fn stop() {
    if let Some(run) = indexing().as_ref() {
        run.cancel.set();
    }
}

/// How far an index is, and whether it was asked to stop.
struct Steps<'a> {
    done: u64,
    total: u64,
    cancel: &'a Cancel,
    progress: &'a Progress,
}

impl Steps<'_> {
    /// One more body read; `Err` once a stop is asked.
    fn step(&mut self) -> Result<(), String> {
        if self.cancel.is_set() {
            return Err(tr!("Stopped before it finished").to_string());
        }
        self.progress.reached(self.done.min(self.total), self.total);
        self.done += 1;
        Ok(())
    }
}

/// Reads what `archives` hold into the lines: the languages a change set registers, every
/// conversation body's lines, the sounds bodies' streamed sounds, the texts of `language` of the
/// lines and of the keys asked for, and the container of each stream. Says what it kept.
fn index(
    archives: &[Archive],
    language: Option<String>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<String, String> {
    let mut steps = Steps {
        done: 0,
        total: archives.iter().map(|a| a.bodies() as u64).sum(),
        cancel,
        progress,
    };
    let open = |a: &Archive| File::open(&a.path).map_err(|e| format!("{}: {e}", a.path.display()));

    if lines().registered.is_empty() {
        'archives: for archive in archives {
            let file = open(archive)?;
            for &at in &archive.change_sets {
                steps.step()?;
                let Ok((_, bytes)) = read_rpf9_body(&file, at, CHANGE_SET_LIMIT) else {
                    continue;
                };
                let names = registered(&String::from_utf8_lossy(&bytes));
                if !names.is_empty() {
                    lines().registered = names;
                    break 'archives;
                }
            }
        }
    }

    let mut bodies = 0;
    for archive in archives {
        let file = open(archive)?;
        for &at in &archive.conversations {
            steps.step()?;
            if let Ok((_, bytes)) = read_rpf9_body(&file, at, BODY_LIMIT) {
                bodies += usize::from(lines().dialogue.add(&bytes) > 0);
            }
        }
        for &at in &archive.audio_data {
            steps.step()?;
            let Ok((_, bytes)) = read_rpf9_body(&file, at, BODY_LIMIT) else {
                continue;
            };
            // Only a sounds body is parsed: the others are as large and hold no streamed sound.
            let schema = bytes.first_chunk::<4>().map(|w| u32::from_le_bytes(*w));
            if schema.and_then(AudioDataKind::of_schema) != Some(AudioDataKind::Sounds) {
                continue;
            }
            let read = AudioData::read(&bytes).map_err(|e| e.to_string());
            match read.and_then(|d| audio_radio::streamed_sounds(&d).map_err(|e| e.to_string())) {
                Ok((sounds, _)) => lines().sounds.extend(sounds),
                Err(e) => tracing::warn!(path = %archive.path.display(), at, "{e}"),
            }
        }
    }
    lines().refresh();

    let (registered, keys) = {
        let lines = lines();
        (lines.registered.clone(), lines.keys())
    };
    let language = language.unwrap_or_else(|| lines().first_language());
    let (mut tables, mut kept) = (0, BTreeMap::new());
    for archive in archives {
        let file = open(archive)?;
        for &(at, ordinal) in &archive.tables {
            if language_of(&registered, ordinal, archive.languages) != language {
                continue;
            }
            steps.step()?;
            let Ok((find, bytes)) = read_rpf9_body(&file, at, BODY_LIMIT) else {
                continue;
            };
            let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(resource),
                ..
            }) = find.kind
            else {
                continue;
            };
            let Ok((table, _)) = or_io::gta6::text::read(resource.sections(&bytes).0) else {
                continue;
            };
            tables += 1;
            kept.extend(subtitles_in(&table, &keys).map(|(key, text)| (key, text.to_string())));
        }
    }
    let texts = kept.len();

    let wanted: BTreeSet<u32> = {
        let lines = lines();
        let sounds = lines.sounds.values().flat_map(|s| s.streams());
        (lines.by_stream.keys().copied().chain(sounds))
            .map(|id| id & STREAM_ID)
            .collect()
    };
    let mut containers: BTreeMap<u32, (usize, u64)> = BTreeMap::new();
    for (n, archive) in archives.iter().enumerate() {
        let file = open(archive)?;
        for &(at, flags, streams) in &archive.containers {
            steps.step()?;
            let Ok(words) = super::gta6_audio_links::stream_words(&file, at, flags, streams) else {
                continue;
            };
            for id in words.iter().map(|w| w & STREAM_ID) {
                if wanted.contains(&id) {
                    containers.entry(id).or_insert((n, at));
                }
            }
        }
    }

    let mut lines = lines();
    lines.keep(&language, kept);
    lines.places = archives.iter().map(|a| a.languages).max().unwrap_or(0);
    lines.archives = archives.iter().map(|a| a.path.clone()).collect();
    let streams = containers.len();
    lines.containers = containers;
    if lines.filter.language.is_empty() {
        lines.filter.language = language.clone();
    }
    Ok(tr!(
        "{lines} dialogue lines in {bodies} conversation bodies; {texts} texts of the language {language} kept from {tables} text tables; {streams} audio streams found in the audio containers of the open scans",
        lines = lines.count(),
        bodies = bodies,
        texts = texts,
        language = language,
        tables = tables,
        streams = streams
    ))
}

impl AppState {
    /// Index the open scans of whole GTA VI archives on a worker: their dialogue lines, the
    /// texts of `language` (the first registered where `None`), and the audio container of each
    /// line's and each streamed sound's stream.
    pub(crate) fn index_gta6_dialogue(&mut self, language: Option<String>) -> Result<(), String> {
        if running() {
            return Ok(());
        }
        let archives: Vec<Archive> = (self.documents.iter())
            .filter_map(|d| match &d.payload {
                DocPayload::Rpf9(Rpf9Document::Scan(scan)) if scan.start == 0 => {
                    Some(Archive::of(&scan.path, &scan.found().rows))
                }
                _ => None,
            })
            .collect();
        if archives.is_empty() {
            return Err(tr!("Open the scan of a GTA VI archive first: the index reads the conversation bodies, text tables, sounds and audio containers the open scans found.").to_string());
        }
        let (cancel, progress) = (Cancel::new(), Progress::new());
        *indexing() = Some(Indexing {
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        lines().said = None;
        std::thread::spawn(move || {
            // A panic still ends the job, or the views wait on it for ever.
            let said = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                index(&archives, language, &cancel, &progress)
            }))
            .unwrap_or_else(|_| Err(tr!("It stopped on an internal error").to_string()));
            lines().said = Some(said);
            *indexing() = None;
        });
        Ok(())
    }

    /// Open the audio container that holds `stream` from the scan that lists it, and play the
    /// stream once the container has opened.
    pub(crate) fn play_gta6_stream(&mut self, stream: u32) {
        let held = lines()
            .container(stream)
            .map(|(a, at)| (a.to_path_buf(), at));
        let Some((archive, container)) = held else {
            self.show_error_toast(tr!("No audio container of the indexed scans holds this stream: index the open scans with the scan of its archive open."));
            return;
        };
        *asked() = Some((
            body_path(&archive, container, View::Audio),
            stream & STREAM_ID,
        ));
        self.open_gta6_stream(&archive, container, stream);
    }
}

#[cfg(test)]
#[path = "gta6_dialogue_tests.rs"]
mod tests;
