//! A GTA VI archive opens as what a scan without its table finds in it, a slice at a time off the
//! UI thread, and each body it lists opens read-only in the view the app has for its kind.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content, RPF9_MISSING_RUN};

use super::jobs::JobKind;
use super::rpf9_scan::{Ended, Row};
use super::{AppState, DocPayload, DocumentId, DocumentKind, ToastType};

pub(super) fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// An RPF9 head of layout 4 that holds every measured rule, and its table, `toc_end` bytes in
/// all; parts A to C take the first 0x190.
pub(super) fn rpf9_head(entries: u32, toc_end: u32) -> Vec<u8> {
    let mut bytes = vec![0x5Au8; toc_end as usize];
    bytes[..0x40].fill(0);
    bytes[..4].copy_from_slice(b"9FPR");
    bytes[0x04..0x08].copy_from_slice(&le32(toc_end));
    bytes[0x08..0x0C].copy_from_slice(&le32(toc_end - (entries * 9).next_multiple_of(16)));
    bytes[0x0C..0x10].copy_from_slice(&le32(0x40));
    bytes[0x1C..0x20].copy_from_slice(&le32(0x150));
    bytes[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x24..0x28].copy_from_slice(&le32(entries));
    bytes[0x30..0x34].copy_from_slice(&le32(0x10));
    bytes[0x2C] = 1;
    (bytes[0x19], bytes[0x1A], bytes[0x20]) = (0x7A, 4, 5);
    (bytes[0x2D], bytes[0x2E]) = (0x0F, 0x09);
    bytes
}

/// `data` as a zlib stream of one stored deflate block, under the `78 DA` head a scan looks for.
pub(super) fn zlib(data: &[u8]) -> Vec<u8> {
    let len = u16::try_from(data.len()).expect("one stored block");
    let mut out = vec![0x78, 0xDA, 0x01];
    out.extend(len.to_le_bytes());
    out.extend((!len).to_le_bytes());
    out.extend_from_slice(data);
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), &c| {
        let a = (a + u32::from(c)) % 65_521;
        (a, (b + a) % 65_521)
    });
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

const ROOT: u32 = 0x1111_0001;
const FIELD: u32 = 0x2222_0000;
/// A u32 member.
const U32: u8 = 0x15;

/// A 32-byte class record of one u32 member whose member table is at `members`.
fn class(members: u64, size: u32) -> [u8; 32] {
    let mut c = [0u8; 32];
    c[0..4].copy_from_slice(&le32(ROOT));
    c[4..8].copy_from_slice(&le32(0x5151_5151));
    c[9] = 3;
    c[16..24].copy_from_slice(&members.to_le_bytes());
    c[24..28].copy_from_slice(&le32(size));
    c[30..32].copy_from_slice(&1u16.to_le_bytes());
    c
}

fn member() -> [u8; 16] {
    let mut m = [0u8; 16];
    m[0..4].copy_from_slice(&le32(FIELD));
    m[8] = U32;
    m
}

/// A PSO document in GTA VI's table form whose root holds one u32.
fn table_form_pso() -> Vec<u8> {
    let mut psin = Vec::new();
    psin.extend(le32(ROOT));
    psin.extend(le32(4));
    psin.extend([0xAA; 8]);
    psin.resize(24, 0);
    psin.extend(le32(77));
    psin.resize(40, 0);
    let mut lsch = class(0xAAAA_AAAA_AAAA_AAAA, 4).to_vec();
    lsch.extend(member());
    let mut lrsc = vec![0u8; 0x50];
    lrsc[0..4].copy_from_slice(b"0DRP");
    lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
    lrsc[0x10..0x14].copy_from_slice(&le32(1));
    lrsc[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes());
    let mut out = Vec::new();
    frame(&mut out, b"PSIN", &psin);
    frame(&mut out, b"LSCH", &lsch);
    frame(&mut out, b"LENM", &[]);
    frame(&mut out, b"LSNS", &le32(0));
    frame(&mut out, b"LGID", &[]);
    frame(&mut out, b"LRSC", &lrsc);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend([0; 4]);
    chks.extend(b"zppp");
    frame(&mut out, b"CHKS", &chks);
    out
}

/// The virtual section of a headerless RBM resource whose root holds one u32 in the block at
/// `block`: 0x5 addresses the virtual section, 0x6 the physical.
pub(super) fn headerless_rbm(block: u64) -> Vec<u8> {
    const VIRTUAL: u64 = 0x5000_0000;
    let mut v = vec![0u8; 0x100];
    let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    // The block map: one virtual block and no physical one.
    put(0x08, &(VIRTUAL + 0xF0).to_le_bytes());
    put(0xF8, &[1, 0]);
    put(0x10, b"0DRP");
    put(0x14, &0x7Au16.to_le_bytes());
    put(0x20, &le32(1));
    put(0x28, &(VIRTUAL + 0x60).to_le_bytes());
    put(0x30, &(VIRTUAL + 0x80).to_le_bytes());
    put(0x38, &(VIRTUAL + 0xC0).to_le_bytes());
    put(0x48, &[1, 0, 0, 0, 1, 0]);
    put(0x60, &class(VIRTUAL + 0xA0, 4));
    put(0xA0, &member());
    put(0xC0, &le32(ROOT));
    put(0xC4, &le32(4));
    put(0xC8, &block.to_le_bytes());
    put(0xE0, &le32(77));
    v
}

/// A `KB2n` movie of one 64x48 frame and no sound track.
fn kb2n() -> Vec<u8> {
    let start = 44 + 4;
    let body = b"picture".to_vec();
    let total = (start + body.len()) as u32;
    let mut movie = b"KB2n".to_vec();
    for word in [total - 8, 1, 16, 1, 64, 48, 30, 1, 0, 0] {
        movie.extend(le32(word));
    }
    movie.extend(le32(start as u32));
    movie.extend(body);
    movie
}

/// A stored RSC8 resource with no physical section and a virtual section of `virtual_size` bytes.
fn rsc8(virtual_size: u32) -> Vec<u8> {
    let mut bytes = vec![0x31u8; 16 + virtual_size as usize];
    bytes[..4].copy_from_slice(b"RSC8");
    bytes[4..8].copy_from_slice(&le32(0x1F41));
    bytes[8..12].copy_from_slice(&le32(virtual_size | 4));
    bytes[12..16].copy_from_slice(&le32(1));
    bytes
}

/// An `ADAT` container of one stream with a data chunk and an `animation` chunk holding `rsc8`.
fn adat(rsc8: &[u8]) -> Vec<u8> {
    let chunks: [(u8, Vec<u8>); 2] = [(0x55, vec![0x77; 512]), (0x5C, rsc8.to_vec())];
    let tables = 16 + 4 + 8 * chunks.len();
    let mut bytes = vec![0u8; tables];
    bytes[..4].copy_from_slice(b"ADAT");
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&0xFF02u16.to_le_bytes());
    bytes[8..12].copy_from_slice(&le32(1));
    bytes[12..16].copy_from_slice(&le32(tables as u32));
    bytes[16..20].copy_from_slice(&le32((chunks.len() as u32) << 29 | 1));
    for (i, (kind, payload)) in chunks.iter().enumerate() {
        let at = bytes.len().next_multiple_of(16);
        bytes.resize(at, 0);
        bytes.extend_from_slice(payload);
        let raw = u64::from(*kind) << 56 | (payload.len() as u64) << 28 | at as u64;
        bytes[20 + 8 * i..28 + 8 * i].copy_from_slice(&raw.to_le_bytes());
    }
    bytes
}

pub(super) const CHANGE_SET: &[u8] =
    b"<?xml version=\"1.0\"?>\n<CDataFileMgr__ContentChangeSetDef>\
    <id>ccs_test</id><fileList><Item><filename>a.rpf</filename><fileType>RPF_FILE</fileType>\
    </Item></fileList></CDataFileMgr__ContentChangeSetDef>\n";

/// An archive of one body of every kind a scan finds, each at a 4 KiB page, and a run of zeros a
/// download has not delivered; the offset of each body, in order.
pub(super) fn archive() -> (Vec<u8>, Vec<u64>) {
    let bodies = [
        zlib(CHANGE_SET),
        zlib(b"a list of names\r\nand more names\r\n"),
        zlib(&table_form_pso()),
        zlib(&headerless_rbm(0x5000_00E0)),
        kb2n(),
        adat(&rsc8(0x80)),
        rsc8(0x2340),
        rpf9_head(2, 0x1B0),
        zlib(&[0xA5; 300]),
        // The headerless resource here has neither a zero header slot nor a page start before it.
        zlib(&headerless_rbm(0x5000_00E0)),
    ];
    let mut bytes = rpf9_head(1, 0x1A0);
    let mut offsets = vec![0];
    for (i, body) in bodies.iter().enumerate() {
        bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
        if i == bodies.len() - 1 {
            bytes.resize(bytes.len() + RPF9_MISSING_RUN as usize + 0x1000, 0);
            bytes.extend([1; 0x20]);
        }
        offsets.push(bytes.len() as u64);
        bytes.extend_from_slice(body);
    }
    (bytes, offsets)
}

pub(super) fn written(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("data.rpf");
    std::fs::write(&path, bytes).expect("written");
    path
}

/// Load `path` and wait for the load to end.
fn load(state: &mut AppState, path: PathBuf) {
    state.load_file(path);
    settle(state);
}

/// Wait for the loads in flight to end.
pub(super) fn settle(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
        state.poll_load_result();
        assert!(Instant::now() < deadline, "the load never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Open the archive and wait for its scan to end.
pub(super) fn scanned(state: &mut AppState, path: PathBuf) -> DocumentId {
    load(state, path);
    let doc = (state.active_document)
        .unwrap_or_else(|| panic!("the archive did not open: {}", state.error_dialog_message));
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| &d.payload) else {
        panic!(
            "not a GTA VI archive document: {}",
            state.error_dialog_message
        );
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while scan.found().ended.is_none() {
        assert!(Instant::now() < deadline, "the scan never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    doc
}

pub(super) fn rows(state: &AppState, doc: DocumentId) -> Vec<Row> {
    match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Rpf9(scan)) => scan.found().rows.clone(),
        _ => panic!("not a GTA VI archive document"),
    }
}

/// The row of the top-level body at `offset`.
pub(super) fn row_at(rows: &[Row], offset: u64) -> usize {
    rows.iter()
        .position(|r| matches!(r, Row::Body(f) if f.offset == offset && f.container.is_none()))
        .unwrap_or_else(|| panic!("no body at {offset:#X} in {rows:?}"))
}

pub(super) fn said(state: &AppState, kind: ToastType, words: &[&str]) -> bool {
    (state.toasts.iter())
        .filter(|t| t.toast_type == kind)
        .any(|t| words.iter().all(|w| t.message.contains(w)))
}

/// The text of every shape the frame painted.
fn painted(output: &egui::FullOutput) -> String {
    fn text(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, out)),
            egui::Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('\n');
            }
            _ => {}
        }
    }
    let mut out = String::new();
    output.shapes.iter().for_each(|c| text(&c.shape, &mut out));
    out
}

fn draw(state: &mut AppState, rect: egui::Rect) -> egui::FullOutput {
    use super::nothing_draws_over_what_it_belongs_to::Panel;
    let mut panel = Panel::new(rect);
    let mut output = None;
    for _ in 0..3 {
        output = Some(panel.frame(Vec::new(), |ui| files_and_centre(ui, state)));
    }
    output.expect("drawn")
}

fn draw_inspector(state: &mut AppState, rect: egui::Rect) -> egui::FullOutput {
    use super::nothing_draws_over_what_it_belongs_to::Panel;
    let mut panel = Panel::new(rect);
    let mut output = None;
    for _ in 0..3 {
        output = Some(panel.frame(Vec::new(), |ui| {
            crate::ui::panels::inspector::draw(ui, state)
        }));
    }
    output.expect("drawn")
}

#[test]
fn an_rpf9_archive_opens_as_the_bodies_a_scan_finds_with_its_header_facts() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, offsets) = archive();
    let path = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let doc = scanned(&mut state, path);
    assert_eq!(
        state.document(doc).map(|d| d.kind),
        Some(DocumentKind::Rpf9)
    );
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| &d.payload) else {
        unreachable!()
    };
    assert_eq!(scan.found().ended, Some(Ended::Whole));
    assert_eq!(scan.found().reached, bytes.len() as u64);

    let rows = rows(&state, doc);
    let found: Vec<u64> = (rows.iter())
        .filter_map(|r| match r {
            Row::Body(f) if f.container.is_none() => Some(f.offset),
            _ => None,
        })
        .collect();
    assert_eq!(found, offsets, "{rows:?}");
    let kind = |i: usize| match &rows[row_at(&rows, offsets[i])] {
        Row::Body(f) => f.kind,
        Row::Missing { .. } => unreachable!(),
    };
    assert!(matches!(kind(0), Rpf9BodyKind::Archive(_)));
    assert!(matches!(kind(1), Rpf9BodyKind::Zlib(r) if r.content == Rpf9Content::Xml));
    assert!(matches!(kind(5), Rpf9BodyKind::Bink2 { .. }));
    assert!(matches!(kind(6), Rpf9BodyKind::Awc { .. }));
    assert!(matches!(kind(8), Rpf9BodyKind::Archive(_)));
    assert!(
        rows.iter()
            .any(|r| matches!(r, Row::Body(f) if f.container == Some(offsets[6]))),
        "the audio container's resource is listed with it"
    );
    let missing: u64 = (rows.iter())
        .filter_map(|r| match r {
            Row::Missing { bytes, from, to } if *from < offsets[10] && *to == offsets[10] => {
                Some(*bytes)
            }
            _ => None,
        })
        .sum();
    assert!(missing >= RPF9_MISSING_RUN, "{rows:?}");

    let output = draw(
        &mut state,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0)),
    );
    let text = painted(&output);
    let header = or_io::container::rpf::Rpf9Header::parse(&bytes).expect("the head");
    for words in [
        "cannot be listed by name".to_string(),
        "a scan without the table".to_string(),
        format!("{} entry", header.entry_count),
        format!("{:#X}", header.toc_end),
        format!("{:#012X}", offsets[5]),
        "Bink 2 movie".to_string(),
        "audio container".to_string(),
        "missing between".to_string(),
        "read the whole file".to_string(),
    ] {
        assert!(text.contains(&words), "the view lacks {words:?}:\n{text}");
    }
}

#[test]
fn a_click_anywhere_on_a_body_s_row_opens_it_and_tab_reaches_the_row_by_its_words() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Panel};
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, offsets) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(900.0, 900.0),
    ));
    let mut frame = |state: &mut AppState, events: Vec<egui::Event>| {
        panel.frame(events, |ui| files_and_centre(ui, state))
    };
    let opened = |state: &AppState| {
        (state.active_document.filter(|&d| d != doc))
            .and_then(|d| state.document(d).map(|d| d.kind))
    };

    // The movie's row, at its kind's words and away from its offset.
    let mut output = frame(&mut state, Vec::new());
    for _ in 0..2 {
        output = frame(&mut state, Vec::new());
    }
    let at = (painted(&output).into_iter())
        .find(|p| p.text.trim() == "Bink 2 movie")
        .expect("the movie's row is drawn")
        .rect
        .center();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut state, vec![egui::Event::PointerMoved(at), press(true)]);
    frame(&mut state, vec![press(false)]);
    settle(&mut state);
    assert_eq!(opened(&state), Some(DocumentKind::Bik));

    // The row is one stop, named by the words it shows, and Enter opens it.
    state.set_active_document(doc);
    let named = format!("{:#012X}  Bink 2 movie", offsets[5]);
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let focused = |output: &egui::FullOutput| {
        let update = output.platform_output.accesskit_update.as_ref()?;
        let node = update.nodes.iter().find(|(id, _)| *id == update.focus)?;
        Some((node.1.role(), node.1.label()?.to_string()))
    };
    let mut reached = None;
    for _ in 0..40 {
        let output = frame(&mut state, vec![key(egui::Key::Tab)]);
        if let Some((role, name)) = focused(&output).filter(|(_, n)| n.starts_with(&named)) {
            reached = Some((role, name));
            break;
        }
    }
    let (role, name) = reached.unwrap_or_else(|| panic!("Tab never reached `{named}`"));
    assert_eq!(role, egui::accesskit::Role::Button, "{name}");
    frame(&mut state, vec![key(egui::Key::Enter)]);
    settle(&mut state);
    assert_eq!(opened(&state), Some(DocumentKind::Bik));
}

#[test]
fn a_scanned_gta_vi_audio_container_lists_its_opus_stream_and_exports_it() {
    use super::opening_a_gta_vi_awc::{gta_vi, opus_packets, opus_single};
    use or_media::Codec;

    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000, 0);
    let packets = opus_packets(0xB8, 20);
    bytes.extend(gta_vi(0xFF21, &[(0x0B01, opus_single(&packets, 1, -1))]));
    let path = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let doc = scanned(&mut state, path);
    state.open_scanned_body(doc, row_at(&rows(&state, doc), 0x1000));
    settle(&mut state);
    let audio = (state.active_document)
        .filter(|&d| d != doc)
        .expect("the audio container opened");

    let streams = state.awc_streams(audio).expect("its streams list");
    let listed: Vec<_> = (streams.iter())
        .map(|r| (r.id, r.codec, r.channels, r.sample_rate))
        .collect();
    assert_eq!(listed, [(0x0B01, Codec::Opus, 1, 48_000)]);
    let wav = state.awc_wav(audio, 0).expect("the stream decodes");
    assert_eq!(wav.len(), 44 + 2 * streams[0].samples as usize);
    let files = state.awc_ogg_opus(audio, 0).expect("the stream exports");
    let names: Vec<_> = files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["00000B01.opus"]);
}

#[test]
fn each_listed_body_opens_read_only_in_the_view_its_kind_has_or_says_why_not() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, offsets) = archive();
    let path = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let doc = scanned(&mut state, path);
    let rows = rows(&state, doc);

    let open = |state: &mut AppState, i: usize| -> Option<DocumentId> {
        state.set_active_document(doc);
        state.open_scanned_body(doc, row_at(&rows, offsets[i]));
        settle(state);
        state.active_document.filter(|&d| d != doc)
    };
    let kind_of = |state: &AppState, d: DocumentId| state.document(d).map(|d| d.kind);

    // The change set, as a loose registration document opens.
    let xml = open(&mut state, 1).expect("the XML opened");
    assert!(matches!(
        kind_of(&state, xml),
        Some(DocumentKind::Text | DocumentKind::Meta)
    ));
    let text = open(&mut state, 2).expect("the text opened");
    assert_eq!(kind_of(&state, text), Some(DocumentKind::Text));
    for (i, what) in [(3, "PSO"), (4, "headerless resource")] {
        let meta = open(&mut state, i).unwrap_or_else(|| panic!("the {what} opened"));
        let Some(DocPayload::Meta(m)) = state.document(meta).map(|d| &d.payload) else {
            panic!("the {what} is not a metadata document");
        };
        let parsed = m
            .parsed
            .as_ref()
            .unwrap_or_else(|| panic!("{:?}", m.parse_error));
        assert_eq!(parsed.instance.fields.len(), 1, "the {what}'s root");
    }
    let movie = open(&mut state, 5).expect("the movie opened");
    assert_eq!(kind_of(&state, movie), Some(DocumentKind::Bik));
    let audio = open(&mut state, 6).expect("the audio container opened");
    assert_eq!(kind_of(&state, audio), Some(DocumentKind::Awc));
    let refused = state
        .awc_play(audio, 0)
        .expect_err("a stream with no format plays");
    assert!(refused.contains("no format to play it by"), "{refused}");

    // Read-only: no save, and the text views take no edit.
    for opened in [xml, text, movie, audio] {
        let d = state.document(opened).expect("open");
        assert!(d.read_only.is_some(), "{:?} is not read-only", d.kind);
        state.set_active_document(opened);
        state.toasts.clear();
        assert!(!state.save_active_document());
        assert!(said(&state, ToastType::Warning, &["read-only"]));
    }

    // An archive's header has no end a read knows: its row says what it is and opens nothing.
    for (i, words) in [(0, "own header"), (8, "nested")] {
        state.toasts.clear();
        assert_eq!(open(&mut state, i), None, "body {i} opened");
        assert!(
            said(&state, ToastType::Info, &[words]),
            "body {i} said nothing of {words}"
        );
    }
    // A body no reader fits, a stored RSC8 of version 65 whose root GTA VI's animation reader
    // refuses by name and a run that starts no resource open as their bytes, under the reason.
    for (i, words) in [
        (9, "not known"),
        (7, "between 0x10 and 0x38"),
        (10, "no headerless resource starts here"),
    ] {
        state.toasts.clear();
        let opened = open(&mut state, i)
            .unwrap_or_else(|| panic!("body {i}: {}", state.error_dialog_message));
        let Some(DocPayload::Hex(hex)) = state.document(opened).map(|d| &d.payload) else {
            panic!("body {i} is not shown as its bytes");
        };
        assert!(hex.banner.contains(words), "body {i}: {}", hex.banner);
        assert!(
            !said(&state, ToastType::Info, &[]),
            "body {i} raised a toast"
        );
    }
}

#[test]
fn a_scan_runs_off_the_ui_thread_in_the_job_list_and_stops_when_asked() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("levels.rpf");
    let file = std::fs::File::create(&path).expect("created");
    file.set_len(1 << 30).expect("a long file of zeros");
    drop(file);
    let mut state = AppState::new();
    load(&mut state, path);
    let doc = state.active_document.expect("opened");
    assert!(
        state
            .running_jobs()
            .iter()
            .any(|j| j.kind == JobKind::ArchiveScan),
        "the scan is not in the job list"
    );
    state.cancel_job(JobKind::ArchiveScan);
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a GTA VI archive document");
    };
    assert!(scan.header.is_none(), "a head of zeros is not a header");
    let deadline = Instant::now() + Duration::from_secs(30);
    while scan.found().ended.is_none() {
        assert!(Instant::now() < deadline, "the scan never stopped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(scan.found().ended, Some(Ended::Stopped));
    assert!(
        scan.found().reached < 1 << 30,
        "the scan read the whole file"
    );
    assert!(state
        .running_jobs()
        .iter()
        .all(|j| j.kind != JobKind::ArchiveScan));
}

#[test]
fn an_entry_of_another_archive_named_as_a_scanned_body_is_not_read_only() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let archive = dir.path().join("mods.rpf");
    let mut head = vec![0u8; 0x40];
    head[..4].copy_from_slice(b"7FPR");
    std::fs::write(&archive, &head).expect("written");
    let path = archive.join("0x1A.xml");
    let mut state = AppState::new();
    let payload = super::document_io::text_payload(&path, b"<a/>".to_vec());
    let id = state.insert_document(path, payload, None, None);
    let read_only = state.document(id).and_then(|d| d.read_only);
    assert!(read_only.is_none(), "marked read-only: {read_only:?}");
}

/// A type-2 cloth: two vertices and one edge between them, its block map after its 0x7A0-byte root.
fn cloth_body() -> Vec<u8> {
    let mut v = vec![0u8; 0x800];
    let ptr = |v: &mut [u8], at: usize, to: usize| {
        v[at..at + 8].copy_from_slice(&(0x5000_0000 + to as u64).to_le_bytes())
    };
    ptr(&mut v, 0x08, 0x7A0);
    v[0x7A8] = 1;
    v[0x1A] = 2;
    v[0x1C..0x20].copy_from_slice(&le32(0x7D8F_3DB3));
    for (at, to, records) in [(0x28, 0x7B0, 2u16), (0x38, 0x7D0, 1)] {
        ptr(&mut v, at, to);
        v[at + 8..at + 10].copy_from_slice(&records.to_le_bytes());
        v[at + 10..at + 12].copy_from_slice(&records.to_le_bytes());
    }
    v[0x7BC..0x7C0].copy_from_slice(&1.0f32.to_le_bytes());
    v[0x7C0..0x7C4].copy_from_slice(&1.0f32.to_le_bytes());
    v[0x7CC..0x7D0].copy_from_slice(&1.0f32.to_le_bytes());
    v[0x7D0..0x7D4].copy_from_slice(&le32(1 << 16));
    v[0x7D4..0x7D8].copy_from_slice(&1.0f32.to_le_bytes());
    v
}

#[test]
fn a_body_opened_as_the_lines_its_reader_reads_is_told_from_one_of_text() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000, 0);
    bytes.extend(zlib(CHANGE_SET));
    bytes.resize(0x2010, 0);
    bytes.extend(zlib(&cloth_body()));
    let mut state = AppState::new();
    let scan = scanned(&mut state, written(dir.path(), &bytes));
    for (offset, facts) in [(0x1000, false), (0x2010, true)] {
        state.open_scanned_body(scan, row_at(&rows(&state, scan), offset));
        settle(&mut state);
        let doc = state.active_document().filter(|d| d.id != scan);
        let doc = doc.unwrap_or_else(|| panic!("{offset:#X}: {}", state.error_dialog_message));
        let path = doc.path.as_deref().expect("a path");
        assert_eq!(
            super::rpf9_scan::opened_as_facts(path),
            facts,
            "{offset:#X}"
        );
        // Only a body's own text takes edits in the Text panel, never the lines of its facts.
        assert_eq!(
            super::rpf9_scan::text_takes_no_edit(doc),
            facts,
            "{offset:#X}"
        );
    }
}

#[test]
fn an_archive_whose_head_is_zero_is_not_called_a_gta_vi_archive() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    scanned(&mut state, written(dir.path(), &[0; 0x1000]));
    let text = painted(&draw(
        &mut state,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0)),
    ));
    assert!(text.contains("not known"), "{text}");
    assert!(!text.contains("GTA VI"), "{text}");
}

#[test]
fn a_scan_keeps_at_most_its_cap_of_rows_and_says_where_it_stopped() {
    let max = super::rpf9_scan::MAX_ROWS;
    let mut bytes = rpf9_head(1, 0x1A0);
    for _ in 0..max + 16 {
        bytes.extend_from_slice(&rsc8(0));
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    assert_eq!(rows(&state, doc).len(), max);
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| &d.payload) else {
        unreachable!()
    };
    let ended = scan.found().ended.clone();
    assert!(
        matches!(&ended, Some(Ended::Failed(why)) if why.contains(&max.to_string())),
        "{ended:?}"
    );
}

#[test]
fn an_audio_container_of_a_hundred_thousand_chunks_lays_out_only_the_rows_in_view() {
    let _serial = crate::ui::theme::serial();
    const STREAMS: usize = 20_000;
    const PER_STREAM: usize = 5;
    let tables = 16 + 4 * STREAMS + 8 * STREAMS * PER_STREAM;
    let mut adat = vec![0u8; tables + 16];
    adat[..4].copy_from_slice(b"ADAT");
    adat[4..6].copy_from_slice(&1u16.to_le_bytes());
    adat[6..8].copy_from_slice(&0xFF02u16.to_le_bytes());
    adat[8..12].copy_from_slice(&le32(STREAMS as u32));
    adat[12..16].copy_from_slice(&le32(tables as u32));
    for s in 0..STREAMS {
        let head = le32((PER_STREAM as u32) << 29 | (s as u32 + 1));
        adat[16 + 4 * s..20 + 4 * s].copy_from_slice(&head);
    }
    // Empty data chunks at the tables' end.
    let chunk = (0x55u64 << 56 | tables as u64).to_le_bytes();
    let chunks = 16 + 4 * STREAMS;
    for c in 0..STREAMS * PER_STREAM {
        adat[chunks + 8 * c..chunks + 8 * c + 8].copy_from_slice(&chunk);
    }
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000, 0);
    bytes.extend_from_slice(&adat);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let rows = rows(&state, doc);
    state.open_scanned_body(doc, row_at(&rows, 0x1000));
    settle(&mut state);
    assert_ne!(state.active_document, Some(doc), "the container opened");
    let output = draw(
        &mut state,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 720.0)),
    );
    let nodes = (output.platform_output.accesskit_update.as_ref()).map_or(0, |u| u.nodes.len());
    let count = STREAMS * PER_STREAM;
    assert!(
        nodes < count / 100,
        "{nodes} widgets were laid out for {count} chunks"
    );
}

#[test]
fn a_hundred_thousand_bodies_lay_out_only_the_rows_in_view() {
    let _serial = crate::ui::theme::serial();
    const COUNT: usize = 100_000;
    let body = zlib(b"name\r\n");
    let mut bytes = rpf9_head(1, 0x1A0);
    for _ in 0..COUNT {
        bytes.extend_from_slice(&body);
        bytes.resize(bytes.len().next_multiple_of(16), 0);
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    assert_eq!(rows(&state, doc).len(), COUNT + 1);
    let output = draw(
        &mut state,
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 720.0)),
    );
    assert!(painted(&output).contains("0x0000"), "no row was painted");
    let nodes = (output.platform_output.accesskit_update.as_ref()).map_or(0, |u| u.nodes.len());
    assert!(
        nodes < COUNT / 100,
        "{nodes} widgets were laid out for {COUNT} bodies"
    );
}

/// An animation set named `dict_a` standing alone with no animation, as a headerless root and as a
/// stored RSC8 of the same bytes.
pub(super) fn animation_set() -> (Vec<u8>, Vec<u8>) {
    let mut root = vec![0u8; 0xC0];
    root[0x38..0x3E].copy_from_slice(b"dict_a");
    root[0x47] = 0x80 | 6;
    root[0x5F] = 1;
    root[0x68..0x6C].copy_from_slice(&or_io::gta6::hash::name_hash32(b"dict_a").to_le_bytes());
    // Its block map, at 0xB0: one virtual block.
    root[0x08..0x10].copy_from_slice(&0x5000_00B0u64.to_le_bytes());
    root[0xB8] = 1;
    let mut stored = b"RSC8".to_vec();
    for word in [0x1F41, root.len() as u32 | 4, 1] {
        stored.extend(le32(word));
    }
    stored.extend_from_slice(&root);
    (root, stored)
}

#[test]
fn a_gta_vi_animation_resource_opens_read_only_with_what_its_reader_reads() {
    let _serial = crate::ui::theme::serial();
    let (root, stored) = animation_set();
    let mut bytes = rpf9_head(1, 0x1A0);
    let mut offsets = Vec::new();
    for body in [zlib(&root), stored] {
        // A headerless resource follows a zero 16-byte slot at the start of a 4 KiB page.
        bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
        offsets.push(bytes.len() as u64);
        bytes.extend_from_slice(&body);
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let rows = rows(&state, doc);
    for offset in offsets {
        state.set_active_document(doc);
        state.open_scanned_body(doc, row_at(&rows, offset));
        settle(&mut state);
        let opened = (state.active_document)
            .filter(|&d| d != doc)
            .unwrap_or_else(|| panic!("the body at {offset:#X}: {}", state.error_dialog_message));
        let d = state.document(opened).expect("open");
        assert_eq!(d.kind, DocumentKind::Gta6Anim);
        assert!(d.read_only.is_some());
        let Some(DocPayload::Gta6Anim(resource)) = state.document(opened).map(|d| &d.payload)
        else {
            unreachable!()
        };
        assert_eq!(resource.name, "dict_a");
        state.toasts.clear();
        assert!(!state.save_active_document());
        assert!(said(&state, ToastType::Warning, &["read-only"]));
        let text = painted(&draw(
            &mut state,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0)),
        ));
        assert!(text.contains("dict_a"), "{text}");
    }
}

/// The virtual section of a GTA VI navigation mesh of one water triangle over a 128 m cell.
pub(super) fn navmesh_cell() -> Vec<u8> {
    let mut v = vec![0u8; 0x360];
    let put =
        |v: &mut Vec<u8>, at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    let ptr = |v: &mut Vec<u8>, at: usize, to: usize| {
        put(v, at, &(0x5000_0000 + to as u64).to_le_bytes())
    };
    // A split array at `at` of one chunk: header, chunk row, start, then the items.
    let list = |v: &mut Vec<u8>, at: usize, items: &[u8], stride: usize| {
        let count = (items.len() / stride) as u64;
        put(v, at + 8, &count.to_le_bytes());
        ptr(v, at + 0x10, at + 0x30);
        ptr(v, at + 0x18, at + 0x40);
        put(v, at + 0x20, &1u64.to_le_bytes());
        ptr(v, at + 0x30, at + 0x50);
        put(v, at + 0x38, &count.to_le_bytes());
        put(v, at + 0x50, items);
    };
    ptr(&mut v, 0x08, 0xF0);
    v[0xF8] = 1;
    put(&mut v, 0x10, &le32(9));
    put(&mut v, 0x14, &le32(0x0001_0015));
    for at in [0x20, 0x34, 0x48] {
        put(&mut v, at, &1.0f32.to_le_bytes());
    }
    let vertices: Vec<u8> = [0u16, 0, 0, 65535, 0, 0, 0, 65535, 0]
        .iter()
        .flat_map(|q| q.to_le_bytes())
        .collect();
    list(&mut v, 0x100, &vertices, 6);
    list(&mut v, 0x170, &[0, 0, 1, 0, 2, 0], 2);
    list(
        &mut v,
        0x1D0,
        &[[0xFF; 4], [0; 4], [0; 4]].repeat(3).concat(),
        12,
    );
    let mut polygon = [0u8; 44];
    polygon[4] = 3;
    polygon[8] = 0x80;
    list(&mut v, 0x250, &polygon, 44);
    for (slot, at) in [
        (0x70, 0x100),
        (0x80, 0x170),
        (0x88, 0x1D0),
        (0x98, 0x250),
        (0xA0, 0x2D0),
    ] {
        ptr(&mut v, slot, at);
    }
    put(&mut v, 0x90, &le32(3));
    for (at, x) in [
        (0x2D0, 0.0f32),
        (0x2E0, 128.0),
        (0x2E4, 128.0),
        (0x2E8, 4.0),
    ] {
        put(&mut v, at, &x.to_le_bytes());
    }
    ptr(&mut v, 0x2D0 + 0x2C, 0x330);
    ptr(&mut v, 0x330 + 0x08, 0x350);
    v[0x330 + 0x18] = 1;
    for (at, word) in [
        (0xC0, 3),
        (0xC4, 1),
        (0xC8, 256 + 512 * 256),
        (0xCC, 6 * 3 + 14 * 3 + 44),
    ] {
        put(&mut v, at, &le32(word));
    }
    v
}

#[test]
fn a_gta_vi_navigation_cell_opens_read_only_in_the_navigation_mesh_view() {
    let _serial = crate::ui::theme::serial();
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000 + 16, 0);
    bytes.extend(zlib(&navmesh_cell()));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let rows = rows(&state, doc);
    state.open_scanned_body(doc, row_at(&rows, 0x1010));
    settle(&mut state);
    let opened = (state.active_document)
        .filter(|&d| d != doc)
        .unwrap_or_else(|| panic!("the cell: {}", state.error_dialog_message));
    let d = state.document(opened).expect("open");
    assert_eq!(d.kind, DocumentKind::Ynv);
    assert!(d.read_only.is_some());
    let Some(DocPayload::Resource(or_io::Asset::NavMesh(mesh))) =
        &state.document(opened).map(|d| &d.payload)
    else {
        panic!("a navigation mesh");
    };
    let polygons = crate::render::navmesh::polygons(mesh);
    assert_eq!(polygons.len(), 1);
    assert_eq!(polygons[0][1], glam::Vec3::new(128.0, 0.0, 0.0));
    let flags = (mesh.polys_array.as_deref())
        .map(|l| l.iter().map(|p| p.as_integer1()).collect::<Vec<_>>());
    assert_eq!(flags, Some(vec![0x80]));
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 900.0));
    let text = painted(&draw_inspector(&mut state, rect));
    assert!(text.contains("GTA VI Navigation Cell"), "{text}");
    // The model the view draws is GTA V's, built from the cell; its header is not the cell's.
    for gta5 in ["Legacy", "Enhanced", "cannot read the fields"] {
        assert!(!text.contains(gta5), "{gta5}: {text}");
    }
    state.toasts.clear();
    assert!(!state.save_active_document());
    assert!(said(&state, ToastType::Warning, &["read-only"]));
    // Its one triangle leads nowhere already: a disable has nothing to cut, a delete is refused.
    let said = (state.disable_navmesh_polygon(opened, 0)).expect("a polygon of the cell");
    assert!(said.contains("left to cut"), "{said}");
    assert!(state.delete_navmesh_polygon(opened, 0).is_err());
}

/// The Files panel beside the centre, as Browse lays them out: the scan's list, then the archive's facts.
pub(super) fn files_and_centre(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::left("files")
        .resizable(false)
        .exact_size(560.0)
        .show_inside(ui, |ui| crate::ui::panels::hierarchy::draw(ui, state));
    crate::ui::panels::document_view::draw(ui, state, None);
}

/// A body opened from a GTA VI archive is of neither GTA V edition, so it offers no conversion.
#[test]
fn a_body_of_a_gta_vi_archive_is_of_no_gta_v_edition_and_does_not_convert() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1010, 0);
    bytes.extend(zlib(&navmesh_cell()));
    let mut state = AppState::new();
    let archive = scanned(&mut state, written(dir.path(), &bytes));
    state.open_scanned_body(archive, row_at(&rows(&state, archive), 0x1010));
    settle(&mut state);
    let doc = (state.active_document)
        .filter(|&d| d != archive)
        .expect("the body opens");
    let path = state.document(doc).and_then(|d| d.path.clone());
    assert!(super::rpf9_scan::from_gta_vi(path.as_deref()));
    let refused = state.document_conversion_to(doc).unwrap_err();
    assert!(refused.contains("neither GTA V edition"), "{refused}");
    assert!(!super::rpf9_scan::from_gta_vi(Some(Path::new("loose.ytd"))));
}
