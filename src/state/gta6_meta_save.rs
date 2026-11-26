//! An edited GTA VI metadata document saved as a file of its own: a PSO or RBF body as its bytes,
//! a headerless resource as a loose resource of its edited sections. A GTA VI `zppp` PSO's `CHKS`
//! is recomputed on save; a frame whose rule is not reproduced is left carried. A table-form PSO's
//! new texts go to its enciphered `LSNS` runs, enciphered with the held key snapshot.

use std::sync::Arc;

use or_io::container::rpf::{GtaViKeys, Rpf9Sections};
use or_io::meta::binary::pso::table_strings;
use or_io::meta::binary::ContainerLayout;

use super::document::{meta_schema, DocPayload, MetaDocument};
use super::extract_job::{ExtractItem, OnClash};
use super::{AppState, DocumentId};

#[cfg(test)]
#[path = "../../crates/tools/or-cli/src/gta6_string_fixture.rs"]
mod string_fixture;

#[path = "gta6_data_xml_splice.rs"]
mod splice;

/// The bytes `meta` saves as, a table-form PSO's new texts enciphered with the keys `keys` loads.
// `keys` is called only when the write needs to encipher a text, which only the writer knows.
pub(crate) fn meta_bytes(
    meta: &MetaDocument,
    keys: impl FnOnce() -> Option<Arc<GtaViKeys>>,
) -> Result<Vec<u8>, String> {
    let layout = (meta.layout.as_ref()).filter(|l| match &l.container {
        ContainerLayout::Pso(p) => p.is_table_form(),
        _ => false,
    });
    let Some(layout) = layout else {
        // A GTA VI data XML edited only in its values keeps every other byte it was read with.
        return splice::spliced(meta).map_or_else(|| meta.to_bytes(), Ok);
    };
    let instance = if meta.parsed_at_text_revision < meta.text_revision {
        let xml = meta.text.text.to_string().into_bytes();
        or_io::meta::MetaDocument::parse(meta_schema(), xml)
            .map(|d| d.instance)
            .map_err(|e| format!("the edited XML does not parse, so it cannot be saved: {e}"))?
    } else {
        match &meta.parsed {
            Some(parsed) => parsed.instance.clone(),
            None => return meta.to_bytes(),
        }
    };
    table_strings::write_adding_strings(layout, &instance, None).or_else(|first| {
        match keys().filter(|k| k.has_string_keys()) {
            Some(keys) => table_strings::write_adding_strings(layout, &instance, Some(&keys)),
            None => Err(first),
        }
        .map_err(|e| e.to_string())
    })
}

/// The metadata row's `edited_sections`: a headerless resource's, once it is edited.
pub(super) fn edited_sections(payload: &DocPayload) -> Option<Result<Rpf9Sections, String>> {
    let DocPayload::Meta(meta) = payload else {
        return None;
    };
    meta.is_edited()
        .then(|| meta.headerless_sections())
        .flatten()
}

impl AppState {
    /// Says, as an edited PSO document whose `CHKS` a save carries (a GTA V/RDR2 or tagless frame,
    /// not a recomputed GTA VI `zppp` one) is saved, that its checksum is the one it was read with.
    pub(super) fn say_the_checksum_is_kept(&mut self, doc: DocumentId) {
        if matches!(self.document(doc).map(|d| &d.payload), Some(DocPayload::Meta(m)) if m.keeps_a_checksum() && m.is_edited())
        {
            self.show_warning_toast(tr!("The CHKS checksum is kept as it was read and is not recomputed, so the game may refuse the saved file."));
        }
        self.say_the_animation_hashes_are_kept(doc);
    }

    /// `name` in `dest`, the bytes of the edited PSO or RBF body `doc` shows, written on a worker;
    /// `None` for any other document, which saves as it did.
    pub(super) fn save_edited_meta(
        &mut self,
        doc: DocumentId,
        dest: std::path::PathBuf,
        name: String,
        reads: std::path::PathBuf,
    ) -> Option<bool> {
        let DocPayload::Meta(meta) = &self.document(doc)?.payload else {
            return None;
        };
        if !meta.is_edited() || meta.headerless_sections().is_some() {
            return None;
        }
        match meta_bytes(meta, || super::rpf9_scan::rpf9_table::keys().ok()) {
            Ok(bytes) => {
                let item = ExtractItem::bytes(name.clone(), bytes);
                Some(self.start_scanned_extraction(name, dest, reads, vec![item], OnClash::Save))
            }
            Err(e) => {
                self.show_error_toast(tr!("Cannot save: {reason}", reason = e));
                Some(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use or_io::container::rpf::{read_rpf9_body, GtaViKeys, Rpf9Sections};
    use or_io::meta::Value;

    use crate::state::opening_an_rpf9_archive::{
        headerless_rbm, row_at, rows, rpf9_head, said, scanned, settle, written, zlib,
    };
    use crate::state::{AppState, DocPayload, DocumentId, ToastType};

    const CHECKSUM: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

    fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
    }

    /// A PSO body in GTA VI's table form whose root holds one u32, laid out as the rules lay one
    /// out, with a checksum word.
    fn table_form_pso() -> Vec<u8> {
        const ROOT: u32 = 0x1111_0001;
        let mut psin = [ROOT.to_le_bytes(), 4u32.to_le_bytes()].concat();
        psin.extend([0xAA; 8]);
        psin.extend([0x70; 8]);
        psin.extend(77u32.to_le_bytes());
        psin.resize(40, 0);
        let mut lsch = [ROOT.to_le_bytes(), 0x5151_5151u32.to_le_bytes()].concat();
        lsch.extend([0, 3, 0, 0, 0, 0, 0, 0]);
        lsch.extend([0xAA; 8]);
        lsch.extend([4, 0, 0, 0, 0, 0, 1, 0]);
        lsch.extend(0x2222_0000u32.to_le_bytes());
        lsch.extend([0, 0, 0, 0, 0x15, 0, 0, 0, 0, 0, 0, 0]);
        let mut lrsc = vec![0u8; 0x50];
        lrsc[0..4].copy_from_slice(b"0DRP");
        lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
        lrsc[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
        lrsc[0x18..0x38].fill(0xBB);
        lrsc[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
        lrsc[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes());
        lrsc[0x40..0x48].fill(0xBB);
        let mut out = Vec::new();
        frame(&mut out, b"PSIN", &psin);
        frame(&mut out, b"LSCH", &lsch);
        frame(&mut out, b"LENM", &[]);
        frame(&mut out, b"LSNS", &0u32.to_le_bytes());
        frame(&mut out, b"LGID", &[]);
        frame(&mut out, b"LRSC", &lrsc);
        let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
        chks.extend(CHECKSUM);
        chks.extend(b"zppp");
        frame(&mut out, b"CHKS", &chks);
        out
    }

    fn finished(state: &mut AppState) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(Instant::now() < deadline, "the save never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Open the body at `offset` of the scan `doc` and set its root's u32 to `value`.
    fn opened_and_edited(state: &mut AppState, doc: DocumentId, offset: u64, value: u64) {
        state.open_scanned_body(doc, row_at(&rows(state, doc), offset));
        settle(state);
        let body = (state.active_document).filter(|&d| d != doc);
        let body = body.unwrap_or_else(|| panic!("the body: {}", state.error_dialog_message));
        let Some(DocPayload::Meta(meta)) = state.document_mut(body).map(|d| &mut d.payload) else {
            panic!("a metadata document");
        };
        let parsed = meta.parsed.as_mut().expect("it parsed");
        parsed.instance.fields[0].1 = Value::UInt(value);
        meta.after_visual_edit_in_place();
    }

    fn root_value(instance: &or_io::meta::MetaInstance) -> &Value {
        &instance.fields[0].1
    }

    #[test]
    fn a_value_typed_into_the_form_of_a_scanned_gta_vi_metadata_body_is_kept() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1000, 0);
        bytes.extend(zlib(&table_form_pso()));
        let mut state = AppState::new();
        let scan = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(scan, row_at(&rows(&state, scan), 0x1000));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != scan)
            .expect("the body");

        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let frame = |state: &mut AppState, events| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1000.0),
                )),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| crate::ui::panels::meta_editor::draw(ui, state));
            let update = output.platform_output.accesskit_update;
            (update.into_iter().flat_map(|u| u.nodes))
                .filter(|(_, n)| n.value() == Some("77"))
                .find_map(|(_, n)| n.bounds())
                .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        };
        frame(&mut state, Vec::new());
        let at = frame(&mut state, Vec::new()).expect("the field holding 77");
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut state, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(&mut state, vec![press(false)]);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(
            &mut state,
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
        );
        frame(&mut state, vec![egui::Event::Text("78".into())]);
        frame(
            &mut state,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        frame(&mut state, Vec::new());

        let Some(DocPayload::Meta(meta)) = state.document(body).map(|d| &d.payload) else {
            panic!("a metadata document");
        };
        let parsed = meta.parsed.as_ref().expect("it parsed");
        assert_eq!(root_value(&parsed.instance), &Value::UInt(78));
        assert!(!said(&state, ToastType::Warning, &["shown read-only"]));
    }

    #[test]
    fn an_edited_gta_vi_metadata_body_saves_as_a_file_that_opens_with_the_edit() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1000, 0);
        bytes.extend(zlib(&table_form_pso()));
        bytes.resize(0x2000, 0);
        // Room after the block map for the slot a loose resource of one block writes there.
        let mut headerless = headerless_rbm(0x5000_00E0);
        headerless.resize(0x110, 0);
        bytes.extend(zlib(&headerless));
        let mut state = AppState::new();
        let archive = written(dir.path(), &bytes);
        let doc = scanned(&mut state, archive.clone());

        // An XML export holds no checksum, and a save over the archive is refused: neither warns of one.
        opened_and_edited(&mut state, doc, 0x1000, 78);
        assert!(state.save_active_document_as(dir.path().join("edited.xml")));
        finished(&mut state);
        assert!(!state.save_active_document_as(archive));
        assert!(!said(&state, ToastType::Warning, &["CHKS checksum"]));

        // A GTA VI `zppp` PSO body: its bytes with the edit and a checksum the save recomputed, so
        // no "checksum kept" warning, and the saved word differs from the one it was read with.
        state.toasts.clear();
        opened_and_edited(&mut state, doc, 0x1000, 78);
        let saved = dir.path().join("edited.pso");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        assert!(!said(&state, ToastType::Warning, &["CHKS checksum"]));
        let file = std::fs::read(&saved).expect("the file");
        assert_ne!(
            &file[file.len() - 8..file.len() - 4],
            &CHECKSUM,
            "a GTA VI zppp body's checksum is recomputed on an edit, not carried"
        );
        let reopened =
            or_io::meta::MetaDocument::parse(&Default::default(), file).expect("it reads");
        assert_eq!(root_value(&reopened.instance), &Value::UInt(78));

        // A headerless resource: a loose resource of its edited sections.
        state.toasts.clear();
        opened_and_edited(&mut state, doc, 0x2000, 79);
        let saved = dir.path().join("edited.rsc");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        assert!(!said(&state, ToastType::Warning, &["CHKS checksum"]));
        let file = std::fs::read(&saved).expect("the file");
        let (find, read) = read_rpf9_body(Cursor::new(file), 0, u64::MAX).expect("it reads");
        let sections = Rpf9Sections::of(&find, &read).expect("a resource");
        let (instance, _) =
            or_io::meta::binary::rbm::gta6::open(&sections.virtual_section, &sections.physical)
                .expect("it opens");
        assert_eq!(root_value(&instance), &Value::UInt(79));
    }

    /// `typed` typed at a click into the Text panel over the active document.
    fn typed_into_the_text_panel(state: &mut AppState, typed: &str) {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |state: &mut AppState, events: Vec<egui::Event>| {
            time += 1.0 / 30.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 300.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| crate::ui::panels::text_editor::draw(ui, state));
        };
        let at = egui::pos2(300.0, 60.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(state, Vec::new());
        frame(state, vec![egui::Event::PointerMoved(at), press(true)]);
        frame(state, vec![press(false)]);
        frame(state, Vec::new());
        frame(state, vec![egui::Event::Text(typed.into())]);
        frame(state, Vec::new());
    }

    #[test]
    fn xml_typed_into_the_text_panel_of_a_scanned_gta_vi_metadata_body_is_kept_and_saved() {
        use crate::state::TextDocument;

        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1000, 0);
        bytes.extend(zlib(&table_form_pso()));
        let mut state = AppState::new();
        let scan = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(scan, row_at(&rows(&state, scan), 0x1000));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != scan)
            .expect("the body");
        let text = |state: &AppState| match state.document(body).map(|d| &d.payload) {
            Some(DocPayload::Meta(meta)) => meta.text.text.to_string(),
            _ => panic!("a metadata document"),
        };

        let opened = text(&state);
        typed_into_the_text_panel(&mut state, "x");
        assert_ne!(text(&state), opened, "the Text panel takes typing");
        assert!(state.document(body).is_some_and(|d| d.dirty));

        // The root's value as XML typed in its place, saved as the body's own form.
        let xml = opened.replacen("value=\"77\"", "value=\"78\"", 1);
        assert_ne!(xml, opened, "the root's u32 is written as an attribute");
        let Some(DocPayload::Meta(meta)) = state.document_mut(body).map(|d| &mut d.payload) else {
            panic!("a metadata document");
        };
        meta.text = TextDocument::decode(xml.as_bytes());
        meta.mark_text_edited();
        // As the form panel does on every frame: the typed XML is the tree, and the body's form stays.
        meta.ensure_parsed_current();
        let saved = dir.path().join("typed.pso");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let file = std::fs::read(&saved).expect("the file");
        let reopened =
            or_io::meta::MetaDocument::parse(&Default::default(), file).expect("it reads");
        assert!(
            reopened.source_format.is_binary(),
            "saved as the body's own form"
        );
        assert_eq!(root_value(&reopened.instance), &Value::UInt(78));

        // A body read-only for any other reason takes no typing.
        if let Some(doc) = state.document_mut(body) {
            doc.read_only = Some("read-only for another reason");
        }
        let before = text(&state);
        typed_into_the_text_panel(&mut state, "y");
        assert_eq!(text(&state), before);
    }

    /// A GTA VI P-form body whose root holds an array of the u32s 7 and 9, a string kept in its 16
    /// bytes and a u32, placed as the measured rules place one.
    fn p_form_pso() -> Vec<u8> {
        use or_io::meta::binary::pso::schema::{string_subtype, ARRAY_INFO_NAME_HASH};
        // The root's type falls in the block table's first bucket, the string block's in the
        // second and the u32 values' in the seventh, which is the order the blocks are in.
        const ROOT: u32 = 0x0B0B_0B0B;
        const STRING_BLOCK: u32 = 1;
        const UINT: u8 = 0x06;
        const SIZE: u32 = 0x28;
        assert_eq!(
            (ROOT % 11, STRING_BLOCK % 11, u32::from(UINT) % 11),
            (0, 1, 6)
        );
        let mut root = vec![0u8; SIZE as usize];
        // The array's pointer names block 3 at offset 0, then its count twice.
        root[..4].copy_from_slice(&3u32.to_be_bytes());
        root[8..12].copy_from_slice(&[0, 2, 0, 2]);
        root[0x10..0x12].copy_from_slice(b"hi");
        root[0x1F] = 0x80 | 2;
        root[0x20..0x24].copy_from_slice(&5u32.to_be_bytes());
        let blocks = [
            (ROOT, root),
            (STRING_BLOCK, b"hi\0".to_vec()),
            (u32::from(UINT), [7u32, 9].map(u32::to_be_bytes).concat()),
        ];
        let mut psin = b"PSIN\0\0\0\0".to_vec();
        psin.extend([0x70; 8]);
        let mut pmap = 1i32.to_be_bytes().to_vec();
        pmap.extend(3i16.to_be_bytes());
        pmap.extend(0i16.to_be_bytes());
        for (hash, data) in &blocks {
            while !psin.len().is_multiple_of(16) {
                psin.push(0x70);
            }
            for word in [*hash, psin.len() as u32, 0, data.len() as u32] {
                pmap.extend(word.to_be_bytes());
            }
            psin.extend(data);
        }
        let len = psin.len() as u32;
        psin[4..8].copy_from_slice(&len.to_be_bytes());
        let entry =
            |out: &mut Vec<u8>, hash: u32, code: u8, subtype: u8, offset: u16, reference: u32| {
                out.extend(hash.to_be_bytes());
                out.extend([code, subtype]);
                out.extend(offset.to_be_bytes());
                out.extend(reference.to_be_bytes());
            };
        let mut items = vec![0, 0, 0, 4];
        items.extend(SIZE.to_be_bytes());
        items.extend([0; 4]);
        entry(&mut items, 0x0C0C_0010, 0x0D, 0, 0, 3);
        entry(
            &mut items,
            0x0C0C_0011,
            0x0B,
            string_subtype::CHAR_ARRAY,
            0x10,
            0,
        );
        entry(&mut items, 0x0C0C_0012, UINT, 0, 0x20, 0);
        entry(&mut items, ARRAY_INFO_NAME_HASH, UINT, 0, 0, 0);
        let mut psch = 1u32.to_be_bytes().to_vec();
        psch.extend(ROOT.to_be_bytes());
        psch.extend((8u32 + 4 + 8).to_be_bytes());
        psch.extend(items);
        let mut out = psin;
        frame(&mut out, b"PMAP", &pmap);
        frame(&mut out, b"PSCH", &psch);
        let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
        chks.extend(CHECKSUM);
        chks.extend(b"zppp");
        frame(&mut out, b"CHKS", &chks);
        out
    }

    #[test]
    fn xml_typed_into_the_text_panel_of_a_gta_vi_p_form_body_grows_its_array_and_saves() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1000, 0);
        bytes.extend(zlib(&p_form_pso()));
        let mut state = AppState::new();
        let scan = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(scan, row_at(&rows(&state, scan), 0x1000));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != scan)
            .expect("the body");
        let Some(DocPayload::Meta(meta)) = state.document_mut(body).map(|d| &mut d.payload) else {
            panic!("a metadata document");
        };
        // A third u32 typed after the array's last, which its allocation has no room for.
        let opened = meta.text.text.to_string();
        let xml = opened.replacen("value=\"9\"/>", "value=\"9\"/>\n<Item value=\"11\"/>", 1);
        assert_ne!(xml, opened, "the array's u32s are written as attributes");
        meta.text = crate::state::TextDocument::decode(xml.as_bytes());
        meta.mark_text_edited();
        meta.ensure_parsed_current();
        let saved = dir.path().join("typed.pso");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let file = std::fs::read(&saved).expect("the file");
        assert_ne!(
            &file[file.len() - 8..file.len() - 4],
            &CHECKSUM,
            "a GTA VI zppp body's checksum is recomputed on an edit, not carried"
        );
        let reopened =
            or_io::meta::MetaDocument::parse(&Default::default(), file).expect("it reads");
        assert_eq!(
            root_value(&reopened.instance),
            &Value::Array([7u64, 9, 11].map(Value::UInt).into())
        );
    }

    /// The table-form document whose category-1 run is enciphered, its root's text set to `text`.
    fn with_a_text(text: &str) -> (Vec<u8>, crate::state::MetaDocument, Arc<GtaViKeys>) {
        let keys = super::string_fixture::string_keys();
        let doc = super::string_fixture::document(&keys);
        let mut meta = crate::state::MetaDocument::from_bytes(doc.clone());
        let parsed = meta.parsed.as_mut().expect("it parsed");
        parsed.instance.fields[0].1 = Value::String(text.into());
        meta.after_visual_edit_in_place();
        (doc, meta, Arc::new(keys))
    }

    #[test]
    fn a_new_text_in_a_document_with_an_enciphered_run_saves_and_reads_back_named() {
        use super::string_fixture::{hash_of, HELD};
        let (_, meta, keys) = with_a_text("zq_new");
        let saved = super::meta_bytes(&meta, || Some(keys.clone())).expect("it saves");
        let strings = crate::state::DocumentStrings::of(&saved, || Some(keys.clone()));
        let hash = or_io::string_hash(b"zq_new");
        assert_eq!(strings.name(hash), Some("zq_new"));
        assert_eq!(
            strings.name(or_io::string_hash(HELD.as_bytes())),
            Some(HELD)
        );
        let reopened = crate::state::MetaDocument::from_bytes(saved);
        let parsed = reopened.parsed.expect("the saved document reads");
        assert_eq!(hash_of(root_value(&parsed.instance)), Some(hash));
    }

    /// A data XML no library types, with a comment, a standalone declaration and CRLF.
    const DATA_XML: &str = concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\r\n",
        "<!-- zq hand edit -->\r\n",
        "<OrZqChangeSetDef>\r\n",
        "  <id>zq_core</id>\r\n",
        "  <flags>ZqPersistent</flags>\r\n",
        "  <count value=\"12\"/>\r\n",
        "  <group></group>\r\n",
        "</OrZqChangeSetDef>\r\n",
    );

    /// `DATA_XML` opened as a GTA VI archive's data XML is, or as GTA V's, edited in its form and
    /// its text brought up to date, as a save into a tree row does.
    fn edited_in_the_form(
        gta6: bool,
        edit: impl FnOnce(&mut or_io::meta::MetaInstance),
    ) -> crate::state::MetaDocument {
        let bytes = DATA_XML.as_bytes().to_vec();
        let mut meta = if gta6 {
            let default = crate::state::meta_schema();
            let library = or_io::meta::names_library().unwrap_or(default);
            let (instance, untyped) =
                or_io::meta::instance::parser::parse_typed_or_tree(library, &bytes)
                    .expect("it reads");
            crate::state::MetaDocument::from_gta_vi_xml(bytes, instance, untyped)
        } else {
            crate::state::MetaDocument::from_bytes(bytes)
        };
        edit(&mut meta.parsed.as_mut().expect("it parsed").instance);
        meta.after_visual_edit_in_place();
        meta.ensure_text_current();
        meta
    }

    #[test]
    fn a_gta_vi_data_xml_edited_in_its_form_saves_with_only_the_values_bytes_changed() {
        let meta = edited_in_the_form(true, |i| i.set("count", Value::Int(34)));
        let saved = super::meta_bytes(&meta, || None).expect("it saves");
        assert_eq!(saved, DATA_XML.replace("\"12\"", "\"34\"").into_bytes());
        let meta = edited_in_the_form(true, |i| i.set("group", Value::String("zq".into())));
        let saved = super::meta_bytes(&meta, || None).expect("it saves");
        assert_eq!(
            saved,
            DATA_XML
                .replace("<group></group>", "<group>zq</group>")
                .into_bytes()
        );
    }

    #[test]
    fn a_structural_edit_or_a_gta_v_xml_is_written_again_from_its_fields_as_before() {
        let removed = edited_in_the_form(true, |i| {
            i.fields.pop();
        });
        let gta5 = edited_in_the_form(false, |i| i.set("count", Value::Int(34)));
        for meta in [removed, gta5] {
            let saved = super::meta_bytes(&meta, || None).expect("it saves");
            assert_eq!(Ok(saved.clone()), meta.to_bytes());
            let saved = String::from_utf8(saved).expect("text");
            assert!(!saved.contains("zq hand edit"), "written again: {saved}");
        }
    }

    #[test]
    fn without_keys_a_new_text_is_refused_by_name_and_an_unedited_save_needs_none() {
        let (doc, meta, _) = with_a_text("zq_new");
        let err = super::meta_bytes(&meta, || None).expect_err("no keys to encipher it");
        assert!(
            err.contains("zq_new") && err.contains("no key snapshot"),
            "{err}"
        );
        let unedited = crate::state::MetaDocument::from_bytes(doc);
        let saved = super::meta_bytes(&unedited, || None);
        assert!(saved.is_ok(), "{saved:?}");
    }
}
