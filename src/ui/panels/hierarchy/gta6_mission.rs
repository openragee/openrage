//! The mission state tables of a GTA VI script body in the outline of its metadata document:
//! a row per state, by its name where one is known, opening into the records or the placed
//! elements it owns. Read from the document's current form, and read only.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use or_io::gta6::mission_tables::{Body, Run, StateTable, RECORD};

mod other;

use super::*;
use crate::state::MetaDocument;

/// The document last drawn, its tree and text revisions and its current form, `None` where that
/// does not write.
// ponytail: one document kept, so two edited outlines drawn by turns each encode every frame.
#[allow(clippy::type_complexity)]
static CURRENT: Mutex<Option<(DocumentId, u64, u64, Option<Arc<[u8]>>)>> = Mutex::new(None);

/// The edited document's current form, encoded once per revision; `None` where it does not write.
fn current_form(doc_id: DocumentId, meta: &MetaDocument) -> Option<Arc<[u8]>> {
    let key = (doc_id, meta.revision(), meta.text_revision);
    let mut kept = CURRENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((id, revision, text, form)) = kept.as_ref() {
        if (*id, *revision, *text) == key {
            return form.clone();
        }
    }
    let form: Option<Arc<[u8]>> = meta.to_bytes().ok().map(Arc::from);
    *kept = Some((key.0, key.1, key.2, form.clone()));
    form
}

/// The tables a state's run is followed through before the outline stops, against a table that owns itself.
const DEPTH: usize = 8;

/// The bytes a metadata document was read from.
fn raw(payload: &DocPayload) -> Option<&[u8]> {
    match payload {
        DocPayload::Meta(meta) => Some(&meta.raw_bytes),
        _ => None,
    }
}

/// Whether the document is a GTA VI body whose type table declares a state table; no record is read.
pub(crate) fn has_tables(payload: &DocPayload) -> bool {
    raw(payload)
        .and_then(|bytes| Body::open(bytes).ok())
        .is_some_and(|body| body.holds_tables())
}

/// A body's tables while they are drawn.
struct Tables<'a> {
    bytes: &'a [u8],
    body: &'a Body,
    tables: &'a [StateTable],
}

impl Tables<'_> {
    /// The table a state's run is records of.
    fn owned(&self, record: &or_io::gta6::mission_tables::StateRecord) -> Option<usize> {
        match self.body.run(self.bytes, self.tables, record)? {
            Run::Table(table) => Some(table),
            _ => None,
        }
    }

    /// A row per state of `rows` of table `table`, each opening into what it owns.
    fn states(
        &self,
        ui: &mut egui::Ui,
        id: egui::Id,
        table: usize,
        rows: Range<usize>,
        depth: usize,
    ) {
        let Some(records) = self.tables.get(table).and_then(|t| t.records.get(rows)) else {
            return;
        };
        for (k, record) in records.iter().enumerate() {
            let name = {
                // GTA VI's name snapshot names a key before the text does.
                let _names = or_io::meta::binary::hash_index::title_names();
                crate::state::hash_name_or_text(record.key)
            };
            let detail = match record.run {
                Some((block, offset)) => tr_n!(
                    "{count} element at {offset} of block {block}",
                    "{count} elements at {offset} of block {block}",
                    usize::from(record.count),
                    offset = format!("{offset:#X}"),
                    block = block + 1
                ),
                None => tr!("no element").to_string(),
            };
            let id = id.with(k);
            collapsing_row(
                ui,
                id,
                false,
                |ui, _| {
                    ui.label(name.as_str());
                    ui.add(detail_label(detail));
                },
                |ui| match self.body.run(self.bytes, self.tables, record) {
                    Some(Run::Table(to)) if depth < DEPTH => {
                        let from = record.run.map_or(0, |run| run.1 / RECORD);
                        let rows = from..from + usize::from(record.count);
                        self.states(ui, id, to, rows, depth + 1);
                    }
                    Some(Run::Placed(elements)) => {
                        for (at, element) in elements {
                            let [x, y, z] = element.position;
                            let [a, b, c] = element.rotation;
                            leaf_row(ui, |ui| {
                                ui.label(crate::state::gta6_names::said32(element.archetype));
                                ui.add(detail_label(tr!(
                                    "at {at}: position {position}, rotation {rotation}, link {link}",
                                    at = format!("{at:#X}"),
                                    position = format!("{x:.3} {y:.3} {z:.3}"),
                                    rotation = format!("{a:.3} {b:.3} {c:.3}"),
                                    link = crate::state::gta6_names::said32(element.link)
                                )));
                            });
                        }
                    }
                    Some(Run::Other { element }) => {
                        ui.add(detail_label(tr_n!(
                            "{count} element of {size} bytes that is not placed in the world",
                            "{count} elements of {size} bytes each that are not placed in the world",
                            usize::from(record.count),
                            size = element
                        )));
                        other::rows(ui, id, self.bytes, self.body, record);
                    }
                    _ => {}
                },
            );
        }
    }
}

/// The body's state tables, each a collapsing row of its states, a table a state owns drawn
/// under that state; `false`, drawing nothing, for a document that declares no state table.
pub(crate) fn draw(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) -> bool {
    let Some(DocPayload::Meta(meta)) = state.document(doc_id).map(|d| &d.payload) else {
        return false;
    };
    // Every metadata document reaches here: only a body read with tables is encoded again.
    if !Body::open(&meta.raw_bytes).is_ok_and(|b| b.holds_tables()) {
        return false;
    }
    // An edit shows once the document writes; one that does not is drawn as the body was read.
    let current = meta.is_edited().then(|| current_form(doc_id, meta));
    let (bytes, as_read) = match &current {
        Some(Some(form)) => (&form[..], false),
        Some(None) => (meta.raw_bytes.as_slice(), true),
        None => (meta.raw_bytes.as_slice(), false),
    };
    let Some(body) = Body::open(bytes).ok().filter(Body::holds_tables) else {
        return false;
    };
    let tables = match body.tables(bytes) {
        Ok(tables) => tables,
        Err(e) => {
            ui.add(detail_label(tr!(
                "The mission state tables do not read: {e}",
                e = e
            )));
            return true;
        }
    };
    let drawn = Tables {
        bytes,
        body: &body,
        tables: &tables,
    };
    let heading = if as_read {
        tr_n!(
            "{count} mission state table, as the body was read: an edit of the document is not shown here",
            "{count} mission state tables, as the body was read: an edit of the document is not shown here",
            tables.len()
        )
    } else {
        tr_n!(
            "{count} mission state table, as the document now stands",
            "{count} mission state tables, as the document now stands",
            tables.len()
        )
    };
    ui.add(detail_label(heading));
    let owned: HashSet<usize> = (tables.iter().flat_map(|t| &t.records))
        .filter_map(|record| drawn.owned(record))
        .collect();
    let id_root = ui.make_persistent_id(("hierarchy_gta6_mission", doc_id));
    for (k, table) in tables.iter().enumerate() {
        if owned.contains(&k) {
            continue;
        }
        let id = id_root.with(k);
        collapsing_row(
            ui,
            id,
            true,
            |ui, _| {
                ui.label(tr!(
                    "State table at {offset}",
                    offset = format!("{:#X}", table.offset)
                ));
                ui.add(detail_label(tr_n!(
                    "{count} state",
                    "{count} states",
                    table.records.len()
                )));
            },
            |ui| drawn.states(ui, id, k, 0..table.records.len(), 0),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(tag);
        out.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(data);
    }

    /// A body of one block of `class`, 32 bytes of zero, whose type states the flag byte `flags`.
    fn body(flags: u8) -> Vec<u8> {
        let class = 0x101u32;
        let mut psin = Vec::new();
        psin.extend_from_slice(&class.to_le_bytes());
        psin.extend_from_slice(&0x20u32.to_le_bytes());
        psin.extend_from_slice(&[0xAA; 8]);
        // One row ends at 0x18 of the body, so the block starts at 0x20.
        psin.extend_from_slice(&[0x70; 8]);
        psin.extend_from_slice(&[0; 0x20]);
        let mut lsch = [0u8; 32];
        lsch[..4].copy_from_slice(&class.to_le_bytes());
        lsch[8] = flags;
        lsch[24] = 0x20;
        let mut lrsc = vec![0u8; 0x40];
        lrsc[0x38] = 1;
        lrsc[0x3C] = 1;
        let mut out = Vec::new();
        frame(&mut out, b"PSIN", &psin);
        frame(&mut out, b"LSCH", &lsch);
        for tag in [b"LENM", b"LSNS", b"LGID"] {
            frame(&mut out, tag, &[]);
        }
        frame(&mut out, b"LRSC", &lrsc);
        frame(&mut out, b"CHKS", &[0; 12]);
        out
    }

    fn document(raw_bytes: Vec<u8>) -> DocPayload {
        let mut meta = MetaDocument::empty();
        meta.raw_bytes = raw_bytes;
        DocPayload::Meta(meta)
    }

    #[test]
    fn only_a_body_that_declares_a_state_table_has_the_outline() {
        // The array's element type has a table; the same bytes under a plain structure have none.
        assert!(has_tables(&document(body(0x0C))));
        assert!(!has_tables(&document(body(0))));
        assert!(!has_tables(&document(b"<Root/>".to_vec())));
        assert!(!has_tables(&document(Vec::new())));
        let bytes = body(0x0C);
        let read = Body::open(&bytes).expect("a body");
        let tables = read.tables(&bytes).expect("a table");
        // The one state owns nothing, so no table is drawn under another.
        let drawn = Tables {
            bytes: &bytes,
            body: &read,
            tables: &tables,
        };
        assert_eq!((tables.len(), tables[0].offset), (1, 0x20));
        assert_eq!(drawn.owned(&tables[0].records[0]), None);
    }

    #[test]
    fn an_edited_document_s_current_form_is_encoded_once_per_revision() {
        let mut meta = MetaDocument::empty();
        meta.raw_bytes = body(0x0C);
        let doc_id = DocumentId::MAX - 0x215;
        let first = current_form(doc_id, &meta).expect("an XML document writes");
        assert_eq!(&first[..], &meta.to_bytes().expect("it writes")[..]);
        let again = current_form(doc_id, &meta).expect("it writes");
        assert!(Arc::ptr_eq(&first, &again));
        meta.text_revision += 1;
        let edited = current_form(doc_id, &meta).expect("it writes");
        assert!(!Arc::ptr_eq(&first, &edited));
    }
}
