//! A GTA VI archive opened as what a scan without its table finds: its header's facts and the
//! bodies stored in the clear, found a slice at a time on a worker and never read whole.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{
    extract_rpf9_body, open_at, read_rpf9_body, starts_stored_rpf9_resource, BodyForm, Gta6Mount,
    GtaViKeys, Rpf9BodyKind, Rpf9Header, Rpf9Resource, Rpf9Scan, Rpf9ScanCarry, Rpf9ScanError,
    Rpf9ScanFind, Rpf9ZlibRun, RPF9_HEADER_SIZE, RPF9_MAGIC,
};
use or_job::{Cancel, Progress};

use super::document::{DocPayload, HexDocument, MetaDocument};
use super::extract_job::{run_extraction, ExtractItem, OnClash};
use super::jobs::{file_name, JobKind, RunningJob};
use super::rpf9_counted_names::counted_scan;
use super::rpf9_kept::Kept;

/// What one pass of the worker scans before it publishes its rows and looks for a stop.
///
/// ⚠ A run of missing bytes a slice boundary splits is judged in each slice, so up to
/// [`or_io::container::rpf::RPF9_MISSING_RUN`] of it on each side may count as unclaimed.
const SLICE: u64 = 64 << 20;
/// The most a body opened from the list inflates to.
const BODY_LIMIT: u64 = 1 << 30;
/// The most rows a scan keeps: a body can be 16 bytes, so a file of them would hold a row per 16 bytes.
pub(super) const MAX_ROWS: usize = 1 << 20;

/// Said of a body opened from a scan, which no save writes back into its archive.
pub(crate) const READ_ONLY_BODY: &str = tr_key!(
    "This body was found by a scan of a GTA VI archive and is read-only there: OpenRage never writes into the archive, and Save As writes the body, with the edits its view takes, as a file of its own"
);

/// A line of the list: a body the scan found, or bytes a download has not delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Body(Box<Rpf9ScanFind>),
    /// Runs of at least [`or_io::container::rpf::RPF9_MISSING_RUN`] zero bytes no body claims, between two offsets.
    Missing {
        from: u64,
        to: u64,
        bytes: u64,
    },
}

/// How the scan ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Whole,
    Stopped,
    Failed(String),
}

/// What the scan has found so far.
#[derive(Debug, Default)]
pub struct Found {
    pub rows: Vec<Row>,
    pub bodies: usize,
    /// The offset the scan started from: 0, or the header of the nested archive it lists.
    pub start: u64,
    /// The offset the scan has reached.
    pub reached: u64,
    pub missing: u64,
    pub unclaimed: u64,
    pub ended: Option<Ended>,
    /// Where each nested archive whose names the scan placed ends, by its header's offset.
    pub ends: BTreeMap<u64, u64>,
    /// Whether the worker is reading a kept scan, in place of the archive.
    pub reading_kept: bool,
    /// When the scan this list was read from was made; `None` for a list this session scanned.
    pub kept: Option<std::time::SystemTime>,
}

impl Found {
    /// The view the body `find` of this list opens in, or why none does.
    pub fn view(&self, find: &Rpf9ScanFind) -> Result<View, String> {
        match find.kind {
            Rpf9BodyKind::Archive(_) if find.offset == self.start => Err(tr!(
                "This archive's own header, whose facts are given in the archive's tab."
            )
            .to_string()),
            Rpf9BodyKind::Archive(_) if self.ends.contains_key(&find.offset) => Ok(View::Archive),
            _ => view_of(find),
        }
    }
}

pub use super::rpf9_list::{row_at_offset, rows_between, rows_to_extract, ListFilter, RowKind};

pub struct Rpf9ScanDocument {
    /// The archive file; a nested archive's offsets are this file's.
    pub path: PathBuf,
    /// `None` when the head is all zero, as a piece a download has not delivered reads.
    pub header: Option<Rpf9Header>,
    pub file_len: u64,
    /// The offset of the nested archive's header this document lists; 0 for the whole file.
    pub start: u64,
    cancel: Cancel,
    progress: Progress,
    found: Arc<Mutex<Found>>,
    /// The read of an effect container the list opened, which stops when the archive closes.
    effects: Mutex<Option<super::gta6_effects::EffectsRead>>,
    /// Which rows the list shows, and where it last jumped to.
    list: Mutex<ListFilter>,
    /// Where the scan is kept once it has read the whole archive; `None` for a nested archive's.
    kept: Option<Kept>,
}

impl Rpf9ScanDocument {
    /// The archive at `path` listed from its kept scan, or with its scan started, from its head alone; `None` when the head is neither RPF9 nor missing.
    pub fn open(path: &Path) -> Result<Option<Self>, String> {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let head = head_of(&mut file).map_err(|e| format!("{}: {e}", path.display()))?;
        let header = if head.starts_with(&RPF9_MAGIC.to_le_bytes()) {
            Some(Rpf9Header::parse(&head).map_err(|e| e.to_string())?)
        } else if is_missing(&head) {
            None
        } else {
            return Ok(None);
        };
        let kept = Kept::of(path, &meta);
        Ok(Some(Self::scanned(path, header, meta.len(), 0, kept)))
    }

    /// The archive nested at `offset` of the file at `path`, with `header`, its scan started from
    /// its header and ending at the names the scan places after its last entry.
    pub fn open_nested(path: &Path, offset: u64, header: Rpf9Header) -> Result<Self, String> {
        let file_len = std::fs::metadata(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        Ok(Self::scanned(path, Some(header), file_len, offset, None))
    }

    fn scanned(
        path: &Path,
        header: Option<Rpf9Header>,
        file_len: u64,
        start: u64,
        kept: Option<Kept>,
    ) -> Self {
        let found = Found {
            start,
            reading_kept: kept.as_ref().is_some_and(Kept::is_kept),
            ..Found::default()
        };
        let doc = Self {
            path: path.to_path_buf(),
            header,
            file_len,
            start,
            cancel: Cancel::new(),
            progress: Progress::new(),
            found: Arc::new(Mutex::new(found)),
            effects: Mutex::default(),
            list: Mutex::default(),
            kept,
        };
        doc.work();
        doc
    }

    /// Start the worker: it reads the kept scan where the list waits for one, else scans the
    /// archive, and keeps a scan that read the whole of it before the list says it ended.
    fn work(&self) {
        let (cancel, progress, found) = (
            self.cancel.clone(),
            self.progress.clone(),
            Arc::clone(&self.found),
        );
        let (path, start, file_len) = (self.path.clone(), self.start, self.file_len);
        let kept = self.kept.clone();
        std::thread::spawn(move || {
            let reading_kept = lock(&found).reading_kept;
            if reading_kept {
                if let Some(read) = kept.as_ref().and_then(Kept::read) {
                    *lock(&found) = read;
                    return;
                }
                lock(&found).reading_kept = false;
            }
            let ended = counted_scan(&path, start, file_len, &found, &cancel, &progress)
                .unwrap_or_else(Ended::Failed);
            if let (Ended::Whole, Some(kept)) = (&ended, &kept) {
                if let Err(error) = kept.keep(&found) {
                    tracing::warn!(%error, path = %path.display(), "the scan was not kept");
                }
            }
            lock(&found).ended = Some(ended);
        });
    }

    /// Scan the archive anew in place of the kept scan its list was read from, which the new scan replaces.
    pub fn scan_again(&self) {
        let mut found = self.found();
        if found.kept.is_none() {
            return;
        }
        *found = Found {
            start: self.start,
            ..Found::default()
        };
        drop(found);
        let mut list = self.list();
        // The rows it counted and examined are gone; what the person asked of the list stays.
        let text = std::mem::take(&mut list.text);
        let kinds = std::mem::take(&mut list.kinds);
        *list = ListFilter::default();
        (list.text, list.kinds) = (text, kinds);
        drop(list);
        self.work();
    }

    pub fn list(&self) -> MutexGuard<'_, ListFilter> {
        (self.list.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn held_bytes(&self) -> usize {
        self.found().rows.len() * std::mem::size_of::<Row>()
    }

    pub fn found(&self) -> MutexGuard<'_, Found> {
        lock(&self.found)
    }

    pub fn stop(&self) {
        self.cancel.set();
    }

    pub fn stopping(&self) -> bool {
        self.cancel.is_set()
    }

    /// The scan while it runs, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        let found = self.found();
        if found.ended.is_some() || found.reading_kept {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::ArchiveScan,
            doing: tr!("Scanning {path}", path = file_name(&self.path)),
            // A nested archive's end is known only once its names are found.
            fraction: (self.start == 0).then(|| self.progress.snapshot().fraction()),
            detail: tr_n!("{count} body found", "{count} bodies found", found.bodies),
            stopping: self.stopping(),
        })
    }
}

impl Drop for Rpf9ScanDocument {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

fn lock(found: &Mutex<Found>) -> MutexGuard<'_, Found> {
    found
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Scan the file from `start` a slice at a time, publishing each slice's rows, until it ends or is
/// stopped; a scan from a nested archive's header ends with that archive's names.
pub(super) fn scan(
    path: &Path,
    start: u64,
    len: u64,
    found: &Mutex<Found>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Ended, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut pos, mut carry) = (start, Rpf9ScanCarry::default());
    progress.reached(start, len);
    while pos < len {
        if cancel.is_set() {
            return Ok(Ended::Stopped);
        }
        let mut scan = Rpf9Scan::new(&file, pos..pos.saturating_add(SLICE))
            .map_err(|e| e.to_string())?
            .carrying(carry);
        let (mut rows, mut missing, mut after) = (Vec::new(), 0, pos);
        let (mut failed, mut ends, mut closed) = (None, Vec::new(), None);
        let held = lock(found).rows.len();
        while let Some(find) = scan.next() {
            if cancel.is_set() {
                return Ok(Ended::Stopped);
            }
            let find = match find {
                Ok(find) => find,
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            };
            if scan.missing_bytes() > missing {
                rows.push(Row::Missing {
                    from: after,
                    to: find.offset,
                    bytes: scan.missing_bytes() - missing,
                });
                missing = scan.missing_bytes();
            }
            let (kind, offset, end) = (find.kind, find.offset, find.offset + find.span);
            if start > 0 && offset != start && matches!(kind, Rpf9BodyKind::Archive(_)) {
                failed = Some(tr!(
                    "where this archive ends is not known: the scan met another archive's header at {at} before names of its own",
                    at = format!("{offset:#X}")
                ));
                break;
            }
            after = after.max(end);
            rows.push(Row::Body(Box::new(find)));
            if held + rows.len() >= MAX_ROWS {
                failed = Some(tr!("the list holds at most {count} rows", count = MAX_ROWS));
                break;
            }
            if let Rpf9BodyKind::Names { archive } = kind {
                ends.push((archive, end));
                if start > 0 && archive == start {
                    closed = Some(end);
                    break;
                }
            }
        }
        let end = scan.position().min(scan.file_len());
        carry = scan.carry();
        if scan.missing_bytes() > missing {
            rows.push(Row::Missing {
                from: after,
                to: end,
                bytes: scan.missing_bytes() - missing,
            });
        }
        {
            let mut found = lock(found);
            let body =
                |r: &&Row| matches!(r, Row::Body(f) if !matches!(f.kind, Rpf9BodyKind::Archive(_)));
            found.bodies += rows.iter().filter(body).count();
            found.rows.extend(rows);
            found.rows.truncate(MAX_ROWS);
            found.missing += scan.missing_bytes();
            found.unclaimed += scan.unclaimed_bytes();
            found.ends.extend(ends);
            found.reached = closed.unwrap_or(end);
        }
        progress.reached(end, len);
        if let Some(why) = failed {
            return Ok(Ended::Failed(why));
        }
        if closed.is_some() {
            return Ok(Ended::Whole);
        }
        if end <= pos {
            break;
        }
        pos = end;
    }
    if start > 0 {
        return Ok(Ended::Failed(tr!(
            "where this archive ends is not known: the file ends at {at} before names of its own",
            at = format!("{len:#X}")
        )));
    }
    Ok(Ended::Whole)
}

#[cfg(test)]
#[path = "extracting_a_gta_vi_body.rs"]
mod extracting_a_gta_vi_body;

#[path = "rpf9_scan_view.rs"]
mod view;
use view::not_a_start;
pub use view::{view_of, View};

#[path = "rpf9_table_document.rs"]
pub(crate) mod rpf9_table_document;

#[path = "rpf9_table.rs"]
pub(crate) mod rpf9_table;

#[path = "rpf9_scan_ciphered.rs"]
mod ciphered;
#[cfg(test)]
pub(crate) use ciphered::decipher_entry;
pub(crate) use ciphered::remember_ciphered;
use ciphered::{ciphered_payload, take_ciphered};

#[path = "rpf9_scan_payload.rs"]
mod payload;
use payload::body_payload;

/// The path a body at `offset` of `archive` opens under in `view`.
pub fn body_path(archive: &Path, offset: u64, view: View) -> PathBuf {
    archive.join(format!("{offset:#X}.{}", view.ext()))
}

/// The archive and offset of a path [`body_path`] made, or a file of no known kind that holds a
/// resource on its own at offset 0; `None` for any other path.
pub fn scanned_body(path: &Path) -> Option<(&Path, u64)> {
    archived_body(path).or_else(|| {
        if super::DocumentKind::from_path(path).is_some() {
            return None;
        }
        let mut head = Vec::with_capacity(LOOSE_HEAD as usize);
        (File::open(path).ok()?.take(LOOSE_HEAD))
            .read_to_end(&mut head)
            .ok()?;
        starts_stored_rpf9_resource(&head, or_io::gta6::anim::RSC8_VERSION).then_some((path, 0))
    })
}

/// The bytes of a loose file [`scanned_body`] reads to tell whether a resource starts it.
const LOOSE_HEAD: u64 = 4096;

fn archived_body(path: &Path) -> Option<(&Path, u64)> {
    let stem = path.file_stem()?.to_str()?;
    let offset = u64::from_str_radix(stem.strip_prefix("0x")?, 16).ok()?;
    let archive = path.parent()?;
    // ⚠ An entry at the root of another archive on disk has the same path.
    let head = head_of(&mut File::open(archive).ok()?).ok()?;
    if !head.starts_with(&RPF9_MAGIC.to_le_bytes()) && !is_missing(&head) {
        return None;
    }
    archive.is_file().then_some((archive, offset))
}

/// The first [`RPF9_HEADER_SIZE`] bytes of `file`, fewer when it is shorter.
fn head_of(file: &mut File) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    file.take(RPF9_HEADER_SIZE as u64).read_to_end(&mut head)?;
    Ok(head)
}

/// Whether `head` is all zero, as a piece a download has not delivered reads.
fn is_missing(head: &[u8]) -> bool {
    head.len() == RPF9_HEADER_SIZE && head.iter().all(|b| *b == 0)
}

/// The body a path [`body_path`] made names, read and given to the view its kind has, or kept as its bytes where no reader reads it.
pub(super) fn read_body(path: &Path, archive: &Path, offset: u64) -> Result<DocPayload, String> {
    // A body opened anew holds none of the edits a closed document of it kept, through either door.
    super::gta6_gltf_import::forget(path);
    super::gta6_embedded_edit::forget(path);
    // A content-ciphered entry the archive explorer opened left a decrypt request here, keyed by
    // this synthetic path; the decipher runs on this worker, not the UI thread. The clear reader
    // below reads only bytes stored in the clear.
    if let Some(req) = take_ciphered(path) {
        return ciphered_payload(path, archive, req);
    }
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let (find, bytes) = match read_rpf9_body(&file, offset, BODY_LIMIT) {
        Err(Rpf9ScanError::ExtentNotKnown { .. }) if offset > 0 => {
            return nested(archive, &file, offset)
        }
        // A first entry whose end is not stated ends where its list's next row starts.
        Err(e) => match super::gta6_first_entry::opened(archive, &file, offset, BODY_LIMIT, &e) {
            Some(opened) => {
                let (find, hex) = opened?;
                origins().insert(path.to_path_buf(), (find, false));
                return Ok(DocPayload::Hex(hex));
            }
            None => return Err(e.to_string()),
        },
        Ok(read) => read,
    };
    origins().insert(path.to_path_buf(), (find, path == archive));
    body_payload(path, archive, find, bytes)
}

/// The archive nested at `offset` of `file`, the file at `archive`, as a scan of its own.
fn nested(archive: &Path, mut file: &File, offset: u64) -> Result<DocPayload, String> {
    let at = |e: std::io::Error| format!("{}: {e}", archive.display());
    file.seek(SeekFrom::Start(offset)).map_err(at)?;
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    (file.take(RPF9_HEADER_SIZE as u64))
        .read_to_end(&mut head)
        .map_err(at)?;
    let header = Rpf9Header::parse(&head).map_err(|e| e.to_string())?;
    let scan = Rpf9ScanDocument::open_nested(archive, offset, header)?;
    Ok(DocPayload::Rpf9(Arc::new(scan)))
}

/// What the scan found of each body opened from its list, by the path [`body_path`] made, read
/// when it opened, and whether the body is a file of its own.
// ponytail: one map for the whole app, which no close empties; a find is a few dozen bytes.
static ORIGINS: Mutex<BTreeMap<PathBuf, (Rpf9ScanFind, bool)>> = Mutex::new(BTreeMap::new());

fn origins() -> MutexGuard<'static, BTreeMap<PathBuf, (Rpf9ScanFind, bool)>> {
    (ORIGINS.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The find the body at `path` was read from once it has opened, and whether it is a file of its own.
pub(crate) fn scan_origin(path: &Path) -> Option<(Rpf9ScanFind, bool)> {
    origins().get(path).copied()
}

/// Whether the body at `path` opened as the lines OpenRage writes of what its reader reads.
pub(crate) fn opened_as_facts(path: &Path) -> bool {
    scan_origin(path).is_some_and(|(find, _)| matches!(view_of(&find), Ok(View::Facts(_))))
}

/// Whether the body at `path` opened as text it stores, which an edit can be written back as.
pub(crate) fn opened_as_text(path: &Path) -> bool {
    scan_origin(path).is_some_and(|(find, _)| matches!(view_of(&find), Ok(View::Xml | View::Text)))
}

/// Whether the Text panel keeps `doc` from edits: a scanned body takes them only as its own text.
pub(crate) fn text_takes_no_edit(doc: &super::Document) -> bool {
    doc.read_only.is_some_and(|why| {
        why != READ_ONLY_BODY || !doc.path.as_deref().is_some_and(opened_as_text)
    })
}

/// An animation resource through GTA VI's animation reader: a stored RSC8's virtual section, or a
/// headerless resource's.
fn animation(find: &Rpf9ScanFind, bytes: &[u8]) -> Result<DocPayload, String> {
    let stored = matches!(find.kind, Rpf9BodyKind::StoredRsc8 { .. });
    let read = |section: &[u8]| {
        let read = if stored {
            or_io::gta6::anim::read_stored
        } else {
            or_io::gta6::anim::read
        };
        read(section)
            .map(|mut resource| {
                let sections = or_io::container::rpf::Rpf9Sections::of(find, bytes);
                resource.sections = sections.map(|mut sections| {
                    sections.virtual_section = section.to_vec();
                    Arc::new(sections)
                });
                DocPayload::Gta6Anim(Arc::new(resource))
            })
            .map_err(|e| {
                tr!(
                    "GTA VI's animation reader does not read this body: {e}",
                    e = e
                )
            })
    };
    if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    {
        return read(resource.sections(bytes).0);
    }
    if !matches!(find.kind, Rpf9BodyKind::StoredRsc8 { .. }) {
        return read(bytes);
    }
    let rsc = or_io::container::rpf::rsc8::Rsc8::open(bytes).map_err(|e| e.to_string())?;
    read(&rsc.data()[..rsc.virtual_size().min(rsc.data().len())])
}

/// A headerless resource through GTA VI's RBM reader, from `bytes`, a read of `resource`.
fn headerless(path: &Path, resource: &Rpf9Resource, bytes: &[u8]) -> Result<DocPayload, String> {
    if bytes.get(0x10..0x14) != Some(b"0DRP") {
        return Err(tr!("This body starts with four zero bytes but has no 0DRP at 0x10, so what it is and which reader fits it are not known.").to_string());
    }
    let (virtual_section, physical) = resource.sections(bytes);
    let map = super::gta6_map::keep(path, virtual_section, physical);
    super::gta6_world::keep(path, virtual_section, physical);
    let why = |e: String| {
        tr!(
            "GTA VI's RBM reader does not read this body's two sections: {e}",
            e = e
        )
    };
    match MetaDocument::from_gta_vi_headerless(virtual_section, physical) {
        Ok(doc) => Ok(DocPayload::Meta(doc)),
        // The map reader needs the tables alone, so the Map screen shows a map whose tree is not read.
        Err(e) if map => {
            let mut doc = MetaDocument::empty();
            doc.parsed = None;
            doc.parse_error = Some(why(e));
            doc.source_format = or_io::meta::SourceFormat::Rbm;
            Ok(DocPayload::Meta(doc))
        }
        Err(e) => Err(why(e)),
    }
}

impl super::AppState {
    /// The scan whose list holds the tab on screen: that tab's own, or the one of the archive its
    /// path names it a body of.
    pub fn scan_listing_the_active_tab(&self) -> Option<super::DocumentId> {
        let active = self.active_document()?;
        if matches!(active.payload, DocPayload::Rpf9(_)) {
            return Some(active.id);
        }
        let archive = active.path.as_deref()?.parent()?;
        (self.documents.iter())
            .find(|d| {
                matches!(&d.payload, DocPayload::Rpf9(scan) if scan.start == 0)
                    && d.path.as_deref() == Some(archive)
            })
            .map(|d| d.id)
    }

    /// The offset of the row of `scan`'s list on screen: the one selected in its own tab, or the
    /// body of it the tab on screen shows.
    pub fn scan_row_on_screen(&self, scan: super::DocumentId) -> Option<u64> {
        let active = self.active_document()?;
        if active.id == scan {
            return self.selection.selected(scan);
        }
        let stem = active.path.as_deref()?.file_stem()?.to_str()?;
        u64::from_str_radix(stem.strip_prefix("0x")?, 16).ok()
    }

    /// Open row `row` of the scan document `doc` shows, read-only in the view its kind has or as its bytes, or say why it does not open.
    pub fn open_scanned_body(&mut self, doc: super::DocumentId, row: usize) {
        self.open_scanned(doc, row, false);
    }

    /// As [`Self::open_scanned_body`], in the preview tab: the next row previewed takes its place.
    pub fn preview_scanned_row(&mut self, doc: super::DocumentId, row: usize) {
        self.open_scanned(doc, row, true);
    }

    /// Show the body at `offset` of the scan document `doc` in the preview tab, as a step of history
    /// to its row does; nothing for a document of any other kind.
    pub(super) fn show_scanned_row(&mut self, doc: super::DocumentId, offset: u64) {
        let Some(DocPayload::Rpf9(scan)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let row = {
            let found = scan.found();
            row_at_offset(&found.rows, offset)
                .filter(|&row| matches!(&found.rows[row], Row::Body(find) if find.offset == offset))
        };
        if let Some(row) = row {
            self.open_scanned(doc, row, true);
        }
    }

    fn open_scanned(&mut self, doc: super::DocumentId, row: usize, preview: bool) {
        let Some(DocPayload::Rpf9(scan)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let scan = Arc::clone(scan);
        let (find, view) = {
            let found = scan.found();
            let Some(Row::Body(find)) = found.rows.get(row).cloned() else {
                return;
            };
            let ended = super::gta6_first_entry::note(&scan.path, &found, &find);
            let view = match found.view(&find) {
                // Its tab says why no reader fits it, so its row raises no toast.
                Err(_) if opens_as_bytes(&find) && ended => Ok(View::Bytes),
                view => view,
            };
            (find, view)
        };
        match view {
            // A nested archive's list is where its bodies open from, so its tab is kept.
            Ok(View::Archive) => {
                super::rpf9_counted_names::remember(&scan.path, &scan.found(), find.offset);
                self.load_file(body_path(&scan.path, find.offset, View::Archive))
            }
            Ok(view) => {
                // An effect container's effects are learned as a job of its own, with a Stop.
                if matches!(view, View::Effects) {
                    scan.read_effects(*find);
                    self.show_info_toast(tr!("Its effects are read in the background, and a GTA VI drawable then names its shaders and their parameters by them."));
                }
                let path = body_path(&scan.path, find.offset, view);
                // A step of history to its row shows the body again.
                let row = super::navigation::Location {
                    doc,
                    node: find.offset,
                };
                self.navigation.visit(row);
                self.selection.select(doc, find.offset);
                self.open_scanned_path(path, preview);
            }
            Err(why) => self.show_info_toast(why),
        }
    }
}

/// The body at `offset` of `archive` as [`extract_rpf9_body`] writes a file of it.
pub(super) fn extracted_body(archive: &Path, offset: u64) -> Result<Vec<u8>, String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    extract_rpf9_body(&file, offset, BODY_LIMIT)
        .map(|(_, bytes)| bytes)
        .or_else(|e| {
            super::gta6_first_entry::bytes(archive, &file, offset, BODY_LIMIT, &e)
                .unwrap_or_else(|| Err(e.to_string()))
        })
}

/// Whether a read gives a body an end: an archive's header and its names have none, and a first
/// entry whose end is not stated ends where its list's next row starts.
fn extracts(find: &Rpf9ScanFind) -> bool {
    !matches!(
        find.kind,
        Rpf9BodyKind::Archive(_) | Rpf9BodyKind::Names { .. }
    )
}

/// Whether a body no reader fits opens as its bytes: a read gives it an end, and it is no effect container, whose row reads its effects.
pub fn opens_as_bytes(find: &Rpf9ScanFind) -> bool {
    extracts(find) && !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. })
}

impl super::AppState {
    /// Extract rows `rows` of the scan document `doc` into `dest` on a worker, each a file that
    /// opens again, named by its offset and kind; `false` when nothing started.
    pub fn extract_scanned_bodies(
        &mut self,
        doc: super::DocumentId,
        rows: &[usize],
        dest: PathBuf,
    ) -> bool {
        let Some(DocPayload::Rpf9(scan)) = self.document(doc).map(|d| &d.payload) else {
            return false;
        };
        let scan = Arc::clone(scan);
        let items: Vec<ExtractItem> = {
            let found = scan.found();
            // A nested archive ends with its names row; the names themselves have no end.
            let ends = |find: &Rpf9ScanFind| {
                !matches!(find.kind, Rpf9BodyKind::Names { .. })
                    && super::gta6_first_entry::note(&scan.path, &found, find)
            };
            (rows.iter())
                .filter_map(|row| match found.rows.get(*row) {
                    Some(Row::Body(find)) if ends(find) => Some(ExtractItem::scanned(
                        find.file_name(),
                        scan.path.clone(),
                        find.offset,
                    )),
                    _ => None,
                })
                .collect()
        };
        if items.is_empty() {
            self.show_warning_toast(tr!("Nothing to extract: an archive whose names are not placed, its encrypted names and a first entry no row follows have no end a read knows."));
            return false;
        }
        match self.gate_permits(Some(&dest)) {
            super::GateOutcome::Proceed => {}
            super::GateOutcome::Refused => return false,
            super::GateOutcome::Confirm(_) => {
                self.show_warning_toast(tr!("Extract into a folder outside the game's install."));
                return false;
            }
        }
        let label = tr_n!(
            "{count} body of {archive}",
            "{count} bodies of {archive}",
            items.len(),
            archive = file_name(&scan.path)
        );
        let archive = scan.path.clone();
        self.start_scanned_extraction(label, dest, archive, items, OnClash::KeepBoth)
    }

    /// Write `items` read out of `archive` into `dest` as the one extraction, on a worker.
    pub(super) fn start_scanned_extraction(
        &mut self,
        label: String,
        dest: PathBuf,
        archive: PathBuf,
        items: Vec<ExtractItem>,
        on_clash: OnClash,
    ) -> bool {
        if let Some(what) = self.extract_jobs.running_noun() {
            self.show_error_toast(tr!("{what} is already running", what = what));
            return false;
        }
        let refused = (self.no_job_reads(&dest, "extract").err())
            .or_else(|| std::fs::create_dir_all(&dest).err().map(|e| e.to_string()));
        if let Some(why) = refused {
            self.show_error_toast(why);
            return false;
        }
        let worker_dest = dest.clone();
        self.extract_jobs.start(
            JobKind::Extraction,
            label,
            dest,
            vec![archive],
            move |cancel, progress| {
                run_extraction(
                    items,
                    worker_dest,
                    BodyForm::Decoded,
                    false,
                    on_clash,
                    cancel,
                    progress,
                )
            },
        );
        true
    }
}

impl Rpf9ScanDocument {
    /// Read the effects of the `SGD5` container `find` on a worker, in place of any read before it.
    pub(super) fn read_effects(&self, find: Rpf9ScanFind) {
        *self.effects() = Some(super::gta6_effects::EffectsRead::start(
            &self.path, find, BODY_LIMIT,
        ));
    }

    /// The effects read this archive's list started, while it runs, as the job list draws it.
    pub fn effects_job(&self) -> Option<RunningJob> {
        self.effects().as_ref()?.job()
    }

    pub(super) fn stop_effects(&self) {
        if let Some(read) = self.effects().as_ref() {
            read.stop();
        }
    }

    /// How the effects read ended, once, and the read dropped.
    pub(super) fn effects_ended(&self) -> Option<super::gta6_effects::EffectsEnded> {
        let mut read = self.effects();
        let ended = read.as_ref()?.ended()?;
        *read = None;
        Some(ended)
    }

    fn effects(&self) -> MutexGuard<'_, Option<super::gta6_effects::EffectsRead>> {
        (self.effects.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

type Skeleton = or_io::gta6::headerless::Skeleton;

/// What each scanned drawable or fragment opened says of its skeleton, read on the load's worker.
// ponytail: one map for the whole app, which no close empties; skeletons are small.
static SKELETONS: Mutex<BTreeMap<PathBuf, Result<Skeleton, String>>> = Mutex::new(BTreeMap::new());

pub(super) fn skeletons() -> MutexGuard<'static, BTreeMap<PathBuf, Result<Skeleton, String>>> {
    (SKELETONS.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The skeleton of the scanned drawable or fragment a path [`body_path`] made names, read when it opened.
pub(crate) fn scanned_skeleton(path: &Path) -> Result<Skeleton, String> {
    skeletons().get(path).cloned().unwrap_or_else(|| {
        Err(tr!("This document is no drawable or fragment found by a scan of a GTA VI archive, so it holds no GTA VI skeleton.").to_string())
    })
}

/// Keep `skeleton` as the one the scanned body at `path` was read with.
#[cfg(test)]
pub(crate) fn keep_scanned_skeleton(path: &Path, skeleton: Skeleton) {
    skeletons().insert(path.to_path_buf(), Ok(skeleton));
}

fn skeleton_of(virtual_section: &[u8], fragment: bool) -> Result<Skeleton, String> {
    or_io::gta6::headerless::read_skeleton(virtual_section, fragment)
        .map_err(|e| {
            tr!(
                "GTA VI's skeleton reader does not read this body: {e}",
                e = e
            )
        })?
        .ok_or_else(|| {
            tr!("This drawable's skeleton pointer is null, so it has no bones to pose.").to_string()
        })
}
