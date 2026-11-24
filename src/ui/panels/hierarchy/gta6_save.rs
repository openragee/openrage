//! A GTA VI save in the outline: a row per block of a story save, or the one payload of a
//! record, each opening into the tree of its JSON. A story save's values are edited in place,
//! each edit a step of undo; a record is read only. The save's facts are in the centre.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use or_io::gta6::save::{self, Save, SaveError};
use or_io::meta::binary::hash_index::default_hash_index;
use serde_json::Value;

use super::*;
use crate::state::capabilities::FormatCapabilities;
use crate::state::{Change, Swap};

/// The members of one object or array the outline draws before it counts the rest.
// ponytail: a cap, not a virtual list; a typed array of thousands shows its first rows only.
const ROWS: usize = 256;

/// Why a record takes no edit and no save.
const READ_ONLY: &str = tr_key!(
    "a GTA VI save record is read only: its header holds a checksum whose algorithm is not known"
);

/// The save's row of the capability table.
pub(crate) const CAPABILITIES: FormatCapabilities = FormatCapabilities {
    selection_summary: Some(summary),
    outline: Some(draw_rows),
    has_outline_children: Some(has_rows),
    inspector: Some(draw_facts),
    ..FormatCapabilities::none(DocumentKind::Gta6Save)
};

/// A save, and the JSON of each of its rows, parsed when it opens and after each edit.
#[derive(Clone)]
pub(crate) struct Gta6SaveDocument {
    pub save: Save,
    /// A story save's blocks, code blocks first, or a record's one payload where it is JSON.
    rows: Vec<Row>,
}

/// The tree a row shows.
#[derive(Clone)]
struct Row {
    /// A block's group key and its own key, where the tree is what lies under the two.
    under: Vec<String>,
    value: Value,
}

/// One edit of a story save's block, at a path under the block's two outer keys.
#[derive(Clone)]
pub(crate) enum SaveEdit {
    /// The value there replaced by this JSON text.
    Set(String),
    /// This JSON text added to the object there under the key, or to the array there with none.
    Add(Option<String>, String),
    /// The member or element there taken out.
    Remove,
}

/// Whether `bytes` start a GTA VI save container.
pub(crate) fn is_save(bytes: &[u8]) -> bool {
    save::SaveKind::of(bytes).is_some()
}

/// The save `bytes` hold as a document; a packed copy is refused by its title and slot.
pub(crate) fn open(bytes: &[u8]) -> Result<DocPayload, String> {
    let save = save::open(bytes).map_err(|e| match e {
        SaveError::Packed { title, slot } => tr!(
            "A packed copy (SLAYER1) of the save {title} in slot {slot}: its body is compressed or encrypted and is not decoded. Open the plain JSAV beside it, which holds the same save",
            title = title,
            slot = slot
        ),
        other => tr!("The GTA VI save does not read: {e}", e = other),
    })?;
    Ok(DocPayload::Gta6Save(Arc::new(Gta6SaveDocument::new(save))))
}

/// The tree of the JSON `text`: what lies under a block's group key and its own key, else the
/// value as it is. Text that is not JSON keeps its row, with nothing under it.
fn row_of(text: &str) -> Row {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let only = |v: &Value| match v {
        Value::Object(map) if map.len() == 1 => {
            (map.iter().next()).map(|(key, inner)| (key.clone(), inner.clone()))
        }
        _ => None,
    };
    let under =
        only(&value).and_then(|(group, v)| only(&v).map(|(key, inner)| (vec![group, key], inner)));
    match under {
        Some((under, value)) => Row { under, value },
        None => Row {
            under: Vec::new(),
            value,
        },
    }
}

/// The names the metadata names hold, each by the hash a save writes for it, which keeps case.
// ponytail: built once from the names known at the first draw; a schema installed later is not seen.
fn key_names() -> &'static HashMap<u32, String> {
    static NAMES: OnceLock<HashMap<u32, String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (default_hash_index().names())
            .map(|name| (save::key_hash(name), name.to_owned()))
            .collect()
    })
}

/// The known name whose hash the key `key` states.
fn key_name(key: &str) -> Option<&'static str> {
    key_names().get(&save::parse_key(key)?).map(String::as_str)
}

/// The key a member typed as `typed` is written under: a reserved key as it is, a name as its hash.
fn written_key(typed: &str) -> String {
    match typed.starts_with('$') {
        true => typed.to_owned(),
        false => save::key_text(typed),
    }
}

/// A block's text as it stood on the other side of an edit.
struct BlockText {
    row: usize,
    other: String,
}

impl Swap for BlockText {
    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let DocPayload::Gta6Save(doc) = payload else {
            return false;
        };
        Arc::make_mut(doc).swap_text(self.row, &mut self.other)
    }

    fn held_bytes(&self) -> usize {
        self.other.len()
    }
}

impl Gta6SaveDocument {
    fn new(save: Save) -> Self {
        let rows = match &save {
            Save::Story(story) => story.blocks().map(|block| row_of(&block.text)).collect(),
            Save::Record(record) => (record.text())
                .filter(|text| serde_json::from_str::<Value>(text).is_ok())
                .map(row_of)
                .into_iter()
                .collect(),
        };
        Self { save, rows }
    }

    /// Swap the text of story block `row` with `other` and read its tree again; `false`, changing nothing, where there is no such block.
    fn swap_text(&mut self, row: usize, other: &mut String) -> bool {
        let Save::Story(story) = &mut self.save else {
            return false;
        };
        let block = (story.code.iter_mut().chain(&mut story.script)).nth(row);
        let (Some(block), Some(tree)) = (block, self.rows.get_mut(row)) else {
            return false;
        };
        std::mem::swap(&mut block.text, other);
        *tree = row_of(&block.text);
        true
    }

    /// The text, as the file writes it, of the value at `path` under the two outer keys of story block `row`.
    fn text_at(&self, row: usize, path: &[String]) -> Option<&str> {
        let Save::Story(story) = &self.save else {
            return None;
        };
        let under = &self.rows.get(row)?.under;
        let full: Vec<&str> = under.iter().chain(path).map(String::as_str).collect();
        story.blocks().nth(row)?.get(&full)
    }

    /// Make `edit` at `path` of story block `row`: the text the block held before, or `None` where the edit changed nothing.
    fn edit(
        &mut self,
        row: usize,
        path: &[String],
        edit: &SaveEdit,
    ) -> Result<Option<String>, String> {
        let Save::Story(story) = &self.save else {
            return Err(crate::i18n::tr(READ_ONLY).to_string());
        };
        let (Some(block), Some(tree)) = (story.blocks().nth(row), self.rows.get(row)) else {
            return Err(tr!("The save holds no such block").to_string());
        };
        // The edit is made on a copy, so a refusal leaves the block as it was.
        let mut edited = block.clone();
        let full: Vec<&str> = (tree.under.iter().chain(path))
            .map(String::as_str)
            .collect();
        match edit {
            SaveEdit::Set(json) => edited.set(&full, json),
            SaveEdit::Add(key, json) => edited.insert(&full, key.as_deref(), json),
            SaveEdit::Remove => edited.remove(&full),
        }
        .map_err(|e| e.to_string())?;
        if edited.text == block.text {
            return Ok(None);
        }
        let mut text = edited.text;
        Ok(self.swap_text(row, &mut text).then_some(text))
    }

    /// The file a save writes: a story save with every block's text as it is held, a record refused.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        match &self.save {
            Save::Story(story) => story.save().map_err(|e| e.to_string()),
            Save::Record(_) => Err(crate::i18n::tr(READ_ONLY).to_string()),
        }
    }

    pub fn held_bytes(&self) -> usize {
        match &self.save {
            Save::Story(story) => story.blocks().map(|block| block.text.len()).sum(),
            Save::Record(record) => record.payload.len(),
        }
    }

    /// What row `k` is called, and the bytes of its text.
    fn row(&self, k: usize) -> Option<(String, usize)> {
        let hex = |hash: u32| format!("{hash:08X}");
        match &self.save {
            Save::Story(story) => {
                let block = story.blocks().nth(k)?;
                let label = if k < story.code.len() {
                    tr!("Code block {id}", id = hex(block.hash))
                } else {
                    tr!("Script block {id}", id = hex(block.hash))
                };
                Some((label, block.text.len()))
            }
            Save::Record(record) => {
                (k < self.rows.len()).then(|| (tr!("Payload").to_string(), record.payload.len()))
            }
        }
    }

    /// The status line of the row `sel` picks.
    fn picked(&self, sel: u64) -> Option<String> {
        let (label, bytes) = self.row(usize::try_from(sel).ok()?)?;
        Some(tr_n!(
            "{row}: {count} byte of JSON",
            "{row}: {count} bytes of JSON",
            bytes,
            row = label
        ))
    }

    /// What the save is, in a line.
    fn whole(&self) -> String {
        match &self.save {
            Save::Story(story) => tr!(
                "GTA VI story save, code blocks: {code}, script blocks: {script}",
                code = story.code.len(),
                script = story.script.len()
            ),
            Save::Record(record) => tr!(
                "GTA VI save record {slot}: {title}",
                slot = record.header.slot,
                title = record.header.title
            ),
        }
    }

    /// What the save states beyond its rows, a line each.
    fn facts(&self) -> Vec<String> {
        let mut lines = vec![self.whole()];
        match &self.save {
            Save::Story(_) => {
                lines.push(
                    tr!("A save writes every block's text as it is held, so a save nothing edited is written as the same bytes")
                        .to_string(),
                );
                lines.push(
                    tr!("In the outline, a click on a value edits it as JSON, and a right-click on a row adds to it or takes it out")
                        .to_string(),
                );
            }
            Save::Record(record) => {
                let h = &record.header;
                let kind = match h.kind {
                    1 => tr!("a packed story save").to_string(),
                    2 => tr!("settings").to_string(),
                    3 => tr!("a profile record").to_string(),
                    other => tr!("kind {kind}, which was not measured", kind = other),
                };
                lines.push(tr!("Kind: {kind}", kind = kind));
                lines.push(tr!(
                    "Saved {seconds} seconds after 1970, UTC",
                    seconds = h.time
                ));
                lines.push(tr_n!(
                    "Payload: {count} byte",
                    "Payload: {count} bytes",
                    record.payload.len()
                ));
                lines.push(tr!(
                    "Checksum {sum}, of an algorithm that is not known: the record is read only",
                    sum = format!("{:08X}", h.checksum)
                ));
                if self.rows.is_empty() {
                    lines.push(tr!("The payload is not JSON text and is not decoded").to_string());
                }
            }
        }
        lines
    }
}

impl AppState {
    /// Make `edit` at `path` of block `row` of the story save `doc`, as a step of its history.
    pub(crate) fn edit_gta6_save(
        &mut self,
        doc: DocumentId,
        row: usize,
        path: &[String],
        edit: &SaveEdit,
    ) -> Result<(), String> {
        let document = (self.document_mut(doc)).ok_or_else(|| tr!("its document is closed"))?;
        let DocPayload::Gta6Save(save) = &mut document.payload else {
            return Err(tr!("The save holds no such block").to_string());
        };
        let Some(before) = Arc::make_mut(save).edit(row, path, edit)? else {
            return Ok(());
        };
        let label = match edit {
            SaveEdit::Set(_) => tr!("Edit a save value"),
            SaveEdit::Add(..) => tr!("Add to a save"),
            SaveEdit::Remove => tr!("Remove from a save"),
        };
        let change = Change::Part {
            key: None,
            parts: vec![Box::new(BlockText { row, other: before })],
        };
        self.record(doc, label, change);
        Ok(())
    }
}

/// The status line of the row picked, else what the save is.
fn summary(payload: &DocPayload, sel: u64) -> String {
    let DocPayload::Gta6Save(doc) = payload else {
        return String::new();
    };
    doc.picked(sel).unwrap_or_else(|| doc.whole())
}

/// Whether the save has a row.
fn has_rows(payload: &DocPayload) -> bool {
    matches!(payload, DocPayload::Gta6Save(doc) if !doc.rows.is_empty())
}

/// How many members an object or an array holds; `None` for a value with none.
fn member_count(value: &Value) -> Option<usize> {
    match value {
        Value::Object(map) => Some(map.len()),
        Value::Array(items) => Some(items.len()),
        _ => None,
    }
}

/// What the outline keeps between frames.
#[derive(Clone, Default)]
struct Held {
    /// The text a key or its known name must hold for its row to be drawn.
    filter: String,
    /// The value being edited: its block, its path and the text typed so far.
    editing: Option<(usize, Vec<String>, String)>,
    /// Whether the field of the value being edited has still to take the keyboard.
    focus: bool,
    /// The key typed for a member to add.
    key: String,
}

/// The outline of one save while it is drawn: what it filters by, and the edits its rows asked for.
struct Tree<'a> {
    doc: &'a Gta6SaveDocument,
    /// The filter, in lower case.
    filter: String,
    held: &'a mut Held,
    edits: Vec<(usize, Vec<String>, SaveEdit)>,
}

impl Tree<'_> {
    fn editable(&self) -> bool {
        matches!(self.doc.save, Save::Story(_))
    }

    /// Whether `key` as it is written, or the known name it hashes, holds the filter.
    fn names(&self, key: &str) -> bool {
        let holds = |text: &str| text.to_lowercase().contains(&self.filter);
        holds(key) || key_name(key).is_some_and(holds)
    }

    /// Whether a key under `value` holds the filter.
    // ponytail: walks the tree each frame while a filter is typed; index the keys if a save is slow.
    fn holds(&self, value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(key, v)| self.names(key) || self.holds(v)),
            Value::Array(items) => items.iter().any(|v| self.holds(v)),
            _ => false,
        }
    }

    /// A row per member of the object or array `value` whose key, or a key under it, holds the
    /// filter, each opening into its own, up to [`ROWS`]; `all` draws every member.
    fn members(
        &mut self,
        ui: &mut egui::Ui,
        id: egui::Id,
        row: usize,
        path: &mut Vec<String>,
        value: &Value,
        all: bool,
    ) {
        let is_object = value.is_object();
        let members: Vec<(String, &Value)> = match value {
            Value::Object(map) => map.iter().map(|(key, v)| (key.clone(), v)).collect(),
            Value::Array(items) => (items.iter().enumerate())
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            _ => return,
        };
        let (mut drawn, mut more) = (0usize, 0usize);
        for (key, child) in members {
            let hit = all || (is_object && self.names(&key));
            if !hit && !self.holds(child) {
                continue;
            }
            if drawn == ROWS {
                more += 1;
                continue;
            }
            drawn += 1;
            let shown = match key_name(&key).filter(|_| is_object) {
                Some(name) => format!("{key}  {name}"),
                None => key.clone(),
            };
            let label = |ui: &mut egui::Ui| {
                ui.add(egui::Label::new(shown.as_str()).sense(egui::Sense::click()))
            };
            let id = id.with(&key);
            path.push(key);
            let response = match member_count(child) {
                Some(count) => collapsing_row(
                    ui,
                    id,
                    false,
                    |ui, _| {
                        let response = label(ui);
                        ui.add(detail_label(count.to_string()));
                        response
                    },
                    |ui| self.members(ui, id, row, path, child, hit),
                ),
                None => leaf_row(ui, |ui| {
                    let response = label(ui);
                    self.value(ui, row, path, child);
                    response
                }),
            };
            self.menu(&response, row, path, child, true);
            path.pop();
        }
        if more > 0 {
            ui.add(detail_label(tr_n!(
                "{count} more row not shown",
                "{count} more rows not shown",
                more
            )));
        }
    }

    /// A leaf's value: its text, which a click starts editing, or the field it is edited in,
    /// which Enter keeps and Escape leaves.
    fn value(&mut self, ui: &mut egui::Ui, row: usize, path: &[String], value: &Value) {
        let edited = |held: &Held| {
            (held.editing.as_ref()).is_some_and(|(at, of, _)| *at == row && of.as_slice() == path)
        };
        if !edited(self.held) {
            let shown = ui.add(detail_label(value.to_string()).sense(egui::Sense::click()));
            if !self.editable() {
                return;
            }
            let shown = shown.on_hover_text(tr!(
                "Click to edit the value as JSON: Enter keeps it, Escape leaves it"
            ));
            if shown.clicked() {
                // The file's own text, so a number keeps the digits it is written with.
                let text =
                    (self.doc.text_at(row, path)).map_or_else(|| value.to_string(), str::to_owned);
                self.held.editing = Some((row, path.to_vec(), text));
                self.held.focus = true;
            }
            return;
        }
        let Some((_, _, text)) = self.held.editing.as_mut() else {
            return;
        };
        let field = ui.text_edit_singleline(text);
        if std::mem::take(&mut self.held.focus) {
            field.request_focus();
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if field.lost_focus() && enter {
            let json = std::mem::take(text);
            self.edits.push((row, path.to_vec(), SaveEdit::Set(json)));
            self.held.editing = None;
        } else if escape || field.lost_focus() {
            self.held.editing = None;
        }
    }

    /// The menu of a row of a story save: add to the object or array it is, or take it out.
    fn menu(
        &mut self,
        response: &egui::Response,
        row: usize,
        path: &[String],
        value: &Value,
        removable: bool,
    ) {
        if !self.editable() {
            return;
        }
        response.context_menu(|ui| {
            let mut edit = None;
            match value {
                Value::Array(_) => {
                    if ui.button(tr!("Add an element")).clicked() {
                        edit = Some(SaveEdit::Add(None, "null".to_owned()));
                    }
                }
                Value::Object(_) => {
                    ui.horizontal(|ui| {
                        ui.label(tr!("Key"));
                        ui.text_edit_singleline(&mut self.held.key)
                            .on_hover_text(tr!(
                            "A name is written as its hash, and a key that starts with $ as it is"
                        ));
                    });
                    let key = self.held.key.trim();
                    let add = egui::Button::new(tr!("Add a member"));
                    if ui.add_enabled(!key.is_empty(), add).clicked() {
                        edit = Some(SaveEdit::Add(Some(written_key(key)), "null".to_owned()));
                    }
                }
                _ => {}
            }
            if removable && ui.button(tr!("Remove")).clicked() {
                edit = Some(SaveEdit::Remove);
            }
            if let Some(edit) = edit {
                self.edits.push((row, path.to_vec(), edit));
                ui.close_kind(egui::UiKind::Menu);
            }
        });
    }
}

/// A filter over keys, then a collapsing row per block or payload with its JSON beneath it; a
/// click selects the row, and the edits the rows asked for are made once they are drawn.
fn draw_rows(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Save(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let doc = Arc::clone(doc);
    let id_root = ui.make_persistent_id(("hierarchy_gta6_save", doc_id));
    let mut held: Held = ui.data(|d| d.get_temp(id_root)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(tr!("Filter keys"));
        ui.text_edit_singleline(&mut held.filter).on_hover_text(tr!(
            "Keeps the rows whose key, or the known name it hashes, holds this text"
        ));
    });
    let mut tree = Tree {
        doc: &doc,
        filter: held.filter.trim().to_lowercase(),
        held: &mut held,
        edits: Vec::new(),
    };
    if !tree.editable() {
        ui.add(detail_label(crate::i18n::tr(READ_ONLY)));
    }
    let all = tree.filter.is_empty();
    let mut clicked = None;
    for (k, row) in doc.rows.iter().enumerate() {
        let Some((label, _)) = doc.row(k) else {
            continue;
        };
        if !all && !tree.holds(&row.value) {
            continue;
        }
        let selected = state.selection.is_selected(doc_id, k as u64);
        let id = id_root.with(k);
        let response = collapsing_row(
            ui,
            id,
            false,
            |ui, _| ui.selectable_label(selected, label),
            |ui| tree.members(ui, id, k, &mut Vec::new(), &row.value, all),
        );
        if response.clicked() {
            clicked = Some(k as u64);
        }
        tree.menu(&response, k, &[], &row.value, false);
    }
    let edits = std::mem::take(&mut tree.edits);
    // The outline's own hold on the save is let go first, so an edit changes it in place.
    drop(doc);
    ui.data_mut(|d| d.insert_temp(id_root, held));
    if let Some(sel) = clicked {
        state.selection.select(doc_id, sel);
    }
    for (row, path, edit) in edits {
        if let Err(why) = state.edit_gta6_save(doc_id, row, &path, &edit) {
            state.show_error_toast(why);
        }
    }
}

/// The save's facts in the centre, under the status line of the row picked.
fn draw_facts(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId, selection: Option<u64>) {
    let Some(DocPayload::Gta6Save(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    if let Some(said) = selection.and_then(|sel| doc.picked(sel)) {
        ui.strong(said);
        ui.separator();
    }
    for line in doc.facts() {
        ui.label(line);
    }
    super::gta6_save_text::draw(ui, &doc.save);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A story save of one code block and one script block, laid out by hand.
    fn story() -> Vec<u8> {
        let texts = [
            "{\"$0x00000001\":{\"$0x000000aa\":{\"$0x000000b1\":[1,2,3],\"$x\":1.500000}}}",
            "{\"$0x00000002\":{\"$0x000000bb\":{}}}",
        ];
        let mut bytes = b"JSAV".to_vec();
        for word in [3u32, 1, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        // Two entries end at 0x38, so the texts start at 0x40, a CRLF between them.
        let offsets = [0x40, 0x40 + texts[0].len() + 2];
        for (k, hash) in [0xAAu32, 0xBB].into_iter().enumerate() {
            for word in [hash, 1, 0, offsets[k] as u32, texts[k].len() as u32] {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        }
        bytes.resize(0x40, 0);
        bytes.extend_from_slice(format!("{}\r\n{}", texts[0], texts[1]).as_bytes());
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        bytes
    }

    fn document(bytes: &[u8]) -> Arc<Gta6SaveDocument> {
        match open(bytes) {
            Ok(DocPayload::Gta6Save(doc)) => doc,
            Ok(other) => panic!("opened as {:?}", other.kind()),
            Err(e) => panic!("{e}"),
        }
    }

    fn path(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    /// A record: a header alone, whose payload starts where the header ends.
    fn record() -> Vec<u8> {
        let mut header = vec![0u8; save::ASAV_HEADER];
        header[..4].copy_from_slice(b"ASAV");
        header[4] = 1;
        header[0x14..0x18].copy_from_slice(&(save::ASAV_HEADER as u32).to_le_bytes());
        header[0x128..0x12E].copy_from_slice(b"SAVE03");
        header
    }

    #[test]
    fn a_story_save_opens_with_a_row_per_block_and_saves_as_the_same_bytes() {
        let bytes = story();
        assert!(is_save(&bytes));
        let doc = document(&bytes);
        let payload = DocPayload::Gta6Save(Arc::clone(&doc));
        assert_eq!(payload.kind(), DocumentKind::Gta6Save);
        assert!(has_rows(&payload));
        // A row holds what lies under the block's two outer keys.
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[0].under, path(&["$0x00000001", "$0x000000aa"]));
        assert_eq!(doc.rows[0].value["$x"], 1.5);
        assert_eq!(member_count(&doc.rows[0].value["$0x000000b1"]), Some(3));
        assert_eq!(member_count(&doc.rows[0].value["$x"]), None);
        assert_eq!(doc.rows[1].value, serde_json::json!({}));
        assert!(doc
            .row(0)
            .is_some_and(|(label, _)| label.contains("000000AA")));
        assert!(doc
            .row(1)
            .is_some_and(|(label, _)| label.contains("000000BB")));
        assert_ne!(
            doc.row(0).map(|r| r.0.replace("AA", "BB")),
            doc.row(1).map(|r| r.0)
        );
        assert_eq!(doc.row(2), None);
        assert_eq!(summary(&payload, 9), doc.whole());
        assert_ne!(summary(&payload, 1), doc.whole());
        assert_eq!(doc.to_bytes().as_deref(), Ok(bytes.as_slice()));
    }

    #[test]
    fn a_packed_copy_and_a_file_that_is_no_save_are_told_apart() {
        assert!(!is_save(b"RSC7"));
        // A packed copy's head, its name empty, then the header of the save it packs.
        let mut packed = b"SLAYER1\0".to_vec();
        packed.extend_from_slice(&1u32.to_le_bytes());
        packed.extend_from_slice(&0u32.to_le_bytes());
        packed.extend_from_slice(&0u64.to_le_bytes());
        let mut header = record();
        header[0x14..0x18].fill(0);
        packed.extend_from_slice(&header);
        assert!(is_save(&packed));
        let refused = open(&packed).err().expect("a packed copy is refused");
        assert!(refused.contains("SAVE03"), "{refused}");
        // The same header alone is a record: read only, and with no JSON no row.
        let record = document(&record());
        assert!(!has_rows(&DocPayload::Gta6Save(Arc::clone(&record))));
        assert!(record.to_bytes().is_err());
        assert_eq!(record.facts().len(), 6);
        let mut held = (*record).clone();
        assert_eq!(
            held.edit(0, &[], &SaveEdit::Remove),
            Err(crate::i18n::tr(READ_ONLY).to_string())
        );
    }

    #[test]
    fn a_value_is_replaced_added_and_removed_and_the_save_written_opens_as_the_same_trees() {
        let bytes = story();
        let mut doc = (*document(&bytes)).clone();
        let x = path(&["$x"]);
        let list = path(&["$0x000000b1"]);
        // The digits typed are the digits written.
        let before = (doc.edit(0, &x, &SaveEdit::Set(" 2.250000 ".into())))
            .expect("a number")
            .expect("a changed block");
        assert_eq!(doc.text_at(0, &x), Some("2.250000"));
        assert_eq!(doc.rows[0].value["$x"], 2.25);
        for json in ["\"text\"", "true", "null"] {
            (doc.edit(0, &x, &SaveEdit::Set(json.into()))).expect("a value");
            assert_eq!(doc.text_at(0, &x), Some(json));
        }
        // The same text again is no edit, and text that is not JSON is refused and changes nothing.
        assert_eq!(doc.edit(0, &x, &SaveEdit::Set("null".into())), Ok(None));
        assert!(doc.edit(0, &x, &SaveEdit::Set("nul".into())).is_err());
        assert_eq!(doc.text_at(0, &x), Some("null"));
        (doc.edit(0, &list, &SaveEdit::Add(None, "null".into()))).expect("an element");
        let key = written_key("Health");
        assert_eq!(key, save::key_text("Health"));
        assert_eq!(written_key("$data"), "$data");
        (doc.edit(0, &[], &SaveEdit::Add(Some(key.clone()), "\"full\"".into()))).expect("a member");
        (doc.edit(0, &path(&["$0x000000b1", "0"]), &SaveEdit::Remove)).expect("an element out");
        (doc.edit(0, &x, &SaveEdit::Remove)).expect("a member out");
        let mut expected = serde_json::Map::new();
        expected.insert("$0x000000b1".to_owned(), serde_json::json!([2, 3, null]));
        expected.insert(key, Value::from("full"));
        let expected = Value::Object(expected);
        assert_eq!(doc.rows[0].value, expected);
        assert!(doc.edit(0, &x, &SaveEdit::Remove).is_err());
        assert!(doc.edit(9, &[], &SaveEdit::Remove).is_err());
        // What is written opens as the same trees, the block no edit touched as the same text.
        let again = document(&doc.to_bytes().expect("a story save is written"));
        assert_eq!(again.rows[0].value, expected);
        assert_eq!(again.rows[0].under, doc.rows[0].under);
        assert_eq!(again.text_at(1, &[]), Some("{}"));
        // The first text swapped back is the file as it was read.
        let mut payload = DocPayload::Gta6Save(Arc::new(doc));
        let mut step = BlockText {
            row: 0,
            other: before,
        };
        assert!(step.swap(&mut payload));
        let DocPayload::Gta6Save(undone) = &payload else {
            panic!("a save");
        };
        assert_eq!(undone.to_bytes().as_deref(), Ok(bytes.as_slice()));
        assert_eq!(undone.rows[0].value["$x"], 1.5);
        let mut gone = BlockText {
            row: 9,
            other: String::new(),
        };
        assert!(!gone.swap(&mut payload));
    }

    #[test]
    fn an_edit_in_the_app_is_a_step_of_undo_and_marks_the_save_changed() {
        let bytes = story();
        let mut state = AppState::new();
        let payload = open(&bytes).expect("a save");
        let file = Some(PathBuf::from("SAVE01"));
        state.documents.push(Document::new(1, file, payload));
        state.active_document = Some(1);
        let x = path(&["$x"]);
        let text = |state: &AppState| match state.document(1).map(|d| &d.payload) {
            Some(DocPayload::Gta6Save(doc)) => doc.text_at(0, &x).map(str::to_owned),
            _ => None,
        };
        let dirty = |state: &AppState| state.document(1).is_some_and(|d| d.dirty);
        (state.edit_gta6_save(1, 0, &x, &SaveEdit::Set("7".into()))).expect("an edit");
        assert_eq!(text(&state).as_deref(), Some("7"));
        assert!(dirty(&state));
        // An edit that changes nothing is no step, and a refused one says why.
        (state.edit_gta6_save(1, 0, &x, &SaveEdit::Set("7".into()))).expect("no edit");
        assert!(state
            .edit_gta6_save(1, 0, &path(&["$nope"]), &SaveEdit::Remove)
            .is_err());
        assert!(state.edit_gta6_save(2, 0, &x, &SaveEdit::Remove).is_err());
        assert!(state.undo().is_some());
        assert_eq!(text(&state).as_deref(), Some("1.500000"));
        assert!(!dirty(&state));
        assert!(state.undo().is_none());
        assert!(state.redo().is_some());
        assert_eq!(text(&state).as_deref(), Some("7"));
    }

    #[test]
    fn a_filter_keeps_the_rows_whose_key_or_known_name_holds_it() {
        let doc = document(&story());
        let mut held = Held::default();
        let mut tree = Tree {
            doc: &doc,
            filter: "b1".to_owned(),
            held: &mut held,
            edits: Vec::new(),
        };
        assert!(tree.editable());
        assert!(tree.names("$0x000000B1"));
        assert!(!tree.names("$x"));
        // The first block holds the key under its two outer keys; the second holds no key at all.
        assert!(tree.holds(&doc.rows[0].value));
        assert!(!tree.holds(&doc.rows[1].value));
        // A key is named where a known name hashes to it with its case kept.
        let known = (default_hash_index().names().next()).map(str::to_owned);
        if let Some(name) = known {
            let key = save::key_text(&name);
            assert_eq!(key_name(&key), Some(name.as_str()));
            tree.filter = name.to_lowercase();
            assert!(tree.names(&key));
        }
        assert_eq!(key_name("$x"), None);
    }
}
