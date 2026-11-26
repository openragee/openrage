//! GTA VI text tables and audio game data opened from a scan: a table's texts edit and save as a
//! loose resource or export as GTA V's text table; audio game data lists its items read-only.

use std::collections::BTreeMap;
use std::sync::Arc;

use or_io::container::rpf::{Rpf9Form, Rpf9Sections};
use or_io::gta5::text::Gxt2;
use or_io::gta5::File as _;
use or_io::gta6::audio_data::{AudioData, AudioFields, AudioItem};
use or_io::gta6::audio_links::{layout, ItemLayout};
use or_io::gta6::text::{TextItem, TextTable};

use super::document::DocPayload;

/// A text table and the row each text has in the view: its entry, and its form in a variant entry.
#[derive(Debug, Clone)]
pub struct Gta6TextDocument {
    pub table: TextTable,
    pub rows: Vec<(usize, Option<usize>)>,
    /// Whether a text was taken to be changed since the table was read.
    pub edited: bool,
    /// The virtual section the table was read from, which a save writes over.
    pub section: Vec<u8>,
}

impl Gta6TextDocument {
    pub fn open(virtual_section: &[u8]) -> Result<Self, String> {
        let (table, _) = or_io::gta6::text::read(virtual_section).map_err(|e| {
            tr!(
                "GTA VI's text table reader does not read this body: {e}",
                e = e
            )
        })?;
        let rows = (table.entries.iter().enumerate())
            .flat_map(|(i, e)| match &e.item {
                TextItem::Text(_) => vec![(i, None)],
                TextItem::Variants(v) => (0..v.forms.len()).map(|f| (i, Some(f))).collect(),
            })
            .collect();
        Ok(Self {
            table,
            rows,
            edited: false,
            section: virtual_section.to_vec(),
        })
    }

    /// The table over the section it was read from, at its length; else laid out again, and why.
    pub fn written(&self) -> Result<(Vec<u8>, Option<String>), String> {
        let why = match self.table.to_section_over(&self.section) {
            Ok(section) => return Ok((section, None)),
            Err(why) => why.to_string(),
        };
        let section = self.table.to_section().map_err(|e| e.to_string())?;
        let said = tr!(
            "The text table is laid out again at another length ({reason}), so a save into its archive writes a rebuilt archive, which the game refuses because only its maker can sign the table.",
            reason = why
        );
        Ok((section, Some(said)))
    }

    /// The key and text of row `row`.
    pub fn row(&self, row: usize) -> Option<(u32, Option<usize>, &str)> {
        let (entry, form) = *self.rows.get(row)?;
        let e = self.table.entries.get(entry)?;
        let text = match (&e.item, form) {
            (TextItem::Text(t), None) => t,
            (TextItem::Variants(v), Some(f)) => &v.forms.get(f)?.text,
            _ => return None,
        };
        Some((e.key, form, text))
    }

    /// The text of row `row`, to change: the table then saves its texts as edited.
    pub fn text_mut(&mut self, row: usize) -> Option<&mut String> {
        let (entry, form) = *self.rows.get(row)?;
        self.edited = true;
        match (&mut self.table.entries.get_mut(entry)?.item, form) {
            (TextItem::Text(t), None) => Some(t),
            (TextItem::Variants(v), Some(f)) => Some(&mut v.forms.get_mut(f)?.text),
            _ => None,
        }
    }

    pub fn held_bytes(&self) -> usize {
        let texts: usize = self.table.texts().map(|(_, t)| t.len()).sum();
        texts + self.section.len() + self.rows.len() * std::mem::size_of::<(usize, Option<usize>)>()
    }

    /// The table's plain texts as GTA V's text table; refused while it holds grammatical variants,
    /// which a GTA V table has no place for.
    pub fn to_gxt2(&self) -> Result<Vec<u8>, String> {
        let mut gxt2 = Gxt2::new();
        let mut variants = 0;
        for e in &self.table.entries {
            match &e.item {
                TextItem::Text(t) => {
                    gxt2.insert(e.key, t.clone());
                }
                TextItem::Variants(_) => variants += 1,
            }
        }
        if variants > 0 {
            return Err(tr_n!(
                "GTA V's text table holds one text per key, and {count} key of this table holds grammatical variants",
                "GTA V's text table holds one text per key, and {count} keys of this table hold grammatical variants",
                variants
            ));
        }
        gxt2.to_bytes().map_err(|e| e.to_string())
    }
}

/// The text table row's `edited_sections`: the table over its own section, once a text was changed.
pub(super) fn edited_sections(payload: &DocPayload) -> Option<Result<Rpf9Sections, String>> {
    let DocPayload::Gta6Text(doc) = payload else {
        return None;
    };
    doc.edited.then(|| {
        let (virtual_section, rebuilt) = doc.written()?;
        if let Some(said) = rebuilt {
            tracing::info!("{said}");
        }
        Ok(Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section,
            physical: Vec::new(),
        })
    })
}

/// Audio game data, its items, and the items of each class the inspector groups.
#[derive(Debug, Clone)]
pub struct Gta6AudioDocument {
    pub data: AudioData,
    pub items: Vec<AudioItem>,
    /// Each item's layout, where its kind and class were laid out.
    pub layouts: Vec<Option<ItemLayout>>,
    /// Each item's position, by its key.
    pub by_key: BTreeMap<u32, usize>,
    /// Each class, or every item where the kind has no class byte, with its items' positions.
    pub groups: Vec<(Option<u8>, Vec<usize>)>,
    pub selected: Option<usize>,
    /// The last look for what the body links to in the scan of its archive.
    pub linking: Option<Arc<super::gta6_audio_links::AudioLinking>>,
}

impl Gta6AudioDocument {
    pub fn open(body: &[u8]) -> Result<Self, String> {
        let data = AudioData::read(body).map_err(|e| {
            tr!(
                "GTA VI's audio game data reader does not read this body: {e}",
                e = e
            )
        })?;
        let items = data.items();
        let layouts = items.iter().map(|item| layout(&data, item)).collect();
        let by_key = (items.iter().enumerate())
            .map(|(i, item)| (item.key, i))
            .collect();
        let mut groups: BTreeMap<Option<u8>, Vec<usize>> = BTreeMap::new();
        for (i, item) in items.iter().enumerate() {
            groups.entry(item.class).or_default().push(i);
        }
        Ok(Self {
            data,
            items,
            layouts,
            by_key,
            groups: groups.into_iter().collect(),
            selected: None,
            linking: None,
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.data.data.len()
            + self.items.len()
                * (std::mem::size_of::<AudioItem>() + std::mem::size_of::<Option<ItemLayout>>())
    }

    /// How many of a class's items have their fields laid out.
    pub fn laid_out(&self, items: &[usize]) -> usize {
        (items.iter())
            .filter(|i| {
                let fields = self.items.get(**i).map(|it| &it.fields);
                let layout = self.layouts.get(**i).and_then(Option::as_ref);
                layout.is_some() || fields.is_some_and(|f| *f != AudioFields::Opaque)
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::text::{TextEntry, Variant, Variants};

    fn table(with_variants: bool) -> TextTable {
        let mut entries = vec![
            TextEntry {
                key: 0x0000_0101,
                group: 0,
                slot: 0,
                item: TextItem::Text("une ligne".into()),
            },
            TextEntry {
                key: 0x0000_0182,
                group: 1,
                slot: 0,
                item: TextItem::Text("another line".into()),
            },
        ];
        if with_variants {
            let form = |text: &str| Variant {
                text: text.into(),
                gender: 1,
                plurality: 2,
                case: 3,
                hash_34a643c5: [[4, 5]; 9],
            };
            entries.push(TextEntry {
                key: 0x0000_0183,
                group: 1,
                slot: 1,
                item: TextItem::Variants(Variants {
                    gender: 0,
                    plurality: 1,
                    hash_27d7357f: 2,
                    hash_e0237fcf: 0,
                    forms: vec![form("one form"), form("another form")],
                }),
            });
        }
        TextTable { groups: 2, entries }
    }

    #[test]
    fn an_edited_text_saves_as_a_table_that_reads_back_with_the_edit() {
        let section = table(true).to_section().expect("written");
        let mut doc = Gta6TextDocument::open(&section).expect("it opens");
        assert_eq!(doc.rows, [(0, None), (1, None), (2, Some(0)), (2, Some(1))]);
        assert_eq!(doc.row(3), Some((0x183, Some(1), "another form")));
        let payload = |doc: &Gta6TextDocument| DocPayload::Gta6Text(Box::new(doc.clone()));
        assert!(edited_sections(&payload(&doc)).is_none(), "nothing edited");

        *doc.text_mut(3).expect("a text") = "a changed form".into();
        let sections = edited_sections(&payload(&doc))
            .expect("edited")
            .expect("written");
        let again = Gta6TextDocument::open(&sections.virtual_section).expect("it opens");
        assert_eq!(again.row(3), Some((0x183, Some(1), "a changed form")));
        assert_eq!(again.table.entries[..2], doc.table.entries[..2]);

        *doc.text_mut(0).expect("a text") = "holds a \0 NUL".into();
        let refused = edited_sections(&payload(&doc)).expect("edited");
        assert!(refused.expect_err("refused").contains("NUL"));
    }

    #[test]
    fn a_same_length_edit_saves_at_the_section_s_length_and_a_longer_one_is_laid_out_again() {
        let section = table(true).to_section().expect("written");
        let payload = |doc: &Gta6TextDocument| DocPayload::Gta6Text(Box::new(doc.clone()));
        let mut doc = Gta6TextDocument::open(&section).expect("it opens");
        *doc.text_mut(1).expect("a text") = "anOther line".into();
        let (same, said) = doc.written().expect("written");
        assert_eq!(said, None, "nothing is laid out again");
        assert_eq!(same.len(), section.len());
        assert_ne!(same, section);
        let sections = edited_sections(&payload(&doc))
            .expect("edited")
            .expect("written");
        assert_eq!(sections.virtual_section, same);
        let again = Gta6TextDocument::open(&same).expect("it opens");
        assert_eq!(again.row(1), Some((0x182, None, "anOther line")));

        *doc.text_mut(1).expect("a text") = "a line longer than the one it was".into();
        let (longer, said) = doc.written().expect("written");
        assert_ne!(longer.len(), section.len());
        let said = said.expect("it says why");
        assert!(said.contains("rebuilt archive"), "{said}");
        let again = Gta6TextDocument::open(&longer).expect("it opens");
        assert_eq!(
            again.row(1),
            Some((0x182, None, "a line longer than the one it was"))
        );
    }

    fn finished(state: &mut crate::state::AppState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(std::time::Instant::now() < deadline, "the save never ended");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// An archive with `body` deflated after its table, and the document its scan opens it in.
    fn opened(
        dir: &std::path::Path,
        body: &[u8],
    ) -> (crate::state::AppState, crate::state::DocumentId) {
        use crate::state::opening_an_rpf9_archive::{
            row_at, rows, rpf9_head, scanned, settle, written, zlib,
        };
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1010, 0);
        bytes.extend(zlib(body));
        let mut state = crate::state::AppState::new();
        let doc = scanned(&mut state, written(dir, &bytes));
        state.open_scanned_body(doc, row_at(&rows(&state, doc), 0x1010));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != doc)
            .unwrap_or_else(|| panic!("the body: {}", state.error_dialog_message));
        (state, body)
    }

    #[test]
    fn a_scanned_text_table_saves_its_edit_as_a_loose_resource_that_opens_again() {
        use crate::state::DocumentKind;
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (mut state, body) = opened(dir.path(), &table(true).to_section().expect("written"));
        assert_eq!(
            state.document(body).map(|d| d.kind),
            Some(DocumentKind::Gta6Text)
        );
        if let Some(DocPayload::Gta6Text(doc)) = state.document_mut(body).map(|d| &mut d.payload) {
            *doc.text_mut(1).expect("a text") = "an edited line".into();
        }
        let saved = dir.path().join("saved.rsc");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        state.load_file(saved);
        crate::state::opening_an_rpf9_archive::settle(&mut state);
        let again = state
            .active_document
            .filter(|&d| d != body)
            .expect("it opened");
        let Some(DocPayload::Gta6Text(doc)) = state.document(again).map(|d| &d.payload) else {
            panic!("a text table: {}", state.error_dialog_message);
        };
        assert_eq!(doc.row(1), Some((0x182, None, "an edited line")));
        // A table with variants has no GTA V form.
        assert!(!state.save_active_document_as(dir.path().join("t.gxt2")));
    }

    #[test]
    fn scanned_audio_game_data_lists_its_items_with_their_waves() {
        use crate::state::DocumentKind;
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|w| w.to_le_bytes()).collect() };
        let mut body = words(&[166, 15, or_io::gta6::audio_data::STAMP]);
        body.extend([13, 0, 0]);
        body.extend(words(&[0xB0B0_0001, 0xE000_0042]));
        // No groups, one encrypted name, one item, one wave field, no other field.
        body.extend(words(&[4, 0, 8, 1, 0x5A5A_5A5A, 1, 0x1234_5678, 4]));
        body.extend(words(&[0, 1, 8 + 4 + 3, 0, 0, 0]));
        let (state, doc) = opened(dir.path(), &body);
        assert_eq!(
            state.document(doc).map(|d| d.kind),
            Some(DocumentKind::Gta6AudioData)
        );
        let Some(DocPayload::Gta6AudioData(audio)) = state.document(doc).map(|d| &d.payload) else {
            panic!("audio game data");
        };
        assert_eq!(audio.groups, [(Some(13), vec![0])]);
        assert_eq!(audio.items[0].key, 0x1234_5678);
        assert_eq!(audio.items[0].waves[0].stream_id(), 0x42);
        assert_eq!(audio.laid_out(&[0]), 1);
    }

    #[test]
    fn plain_texts_export_as_gta_v_s_text_table_and_variants_refuse() {
        let doc =
            Gta6TextDocument::open(&table(false).to_section().expect("written")).expect("it opens");
        let gxt2 = Gxt2::from_bytes(&doc.to_gxt2().expect("exported")).expect("a GXT2");
        assert_eq!(gxt2.get(0x101), Some("une ligne"));
        assert_eq!(gxt2.get(0x182), Some("another line"));
        assert_eq!(gxt2.len(), 2);
        let doc =
            Gta6TextDocument::open(&table(true).to_section().expect("written")).expect("it opens");
        assert!(doc
            .to_gxt2()
            .expect_err("refused")
            .contains("grammatical variants"));
    }
}
