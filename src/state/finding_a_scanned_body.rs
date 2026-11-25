//! A row is found in a GTA VI archive's scan list by its words, its kind or its offset; a nested
//! archive whose names the scan placed opens as a scan of its own; and a body opened from the list
//! says where it was found, and why it is shown as its bytes where no reader fits it.

use std::time::{Duration, Instant};

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content};

use super::nothing_draws_over_what_it_belongs_to::{painted, Painted, Panel};
use super::opening_an_rpf9_archive::{
    archive, files_and_centre, row_at, rows, rpf9_head, said, scanned, settle, written, zlib,
};
use super::rpf9_scan::{body_path, row_at_offset, Ended, ListFilter, Row, RowKind, View};
use super::{AppState, DocPayload, DocumentId, DocumentKind, ToastType};

const NESTED: u64 = 0x1000;
const LAST: u64 = 0x2000;

/// A top-level archive holding a nested archive whose names are placed after its one XML body,
/// then a second nested archive whose names are not; where the first one's names end.
pub(super) fn nested_archive() -> (Vec<u8>, u64) {
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(NESTED as usize, 0);
    bytes.extend(rpf9_head(2, 0x1B0));
    bytes.extend(zlib(b"<?xml version=\"1.0\"?>\n<CMapTypes/>\n"));
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    // The names size the head states.
    bytes.extend([0x33; 0x40]);
    let names_end = bytes.len() as u64;
    bytes.resize(LAST as usize, 0);
    bytes.extend(rpf9_head(1, 0x1A0));
    bytes.extend(zlib(b"a list of names\r\n"));
    (bytes, names_end)
}

fn until_scanned(state: &AppState, doc: DocumentId) {
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
}

fn offsets(rows: &[Row]) -> Vec<u64> {
    (rows.iter())
        .filter_map(|r| match r {
            Row::Body(f) => Some(f.offset),
            Row::Missing { .. } => None,
        })
        .collect()
}

#[test]
fn a_nested_archive_whose_names_were_placed_opens_as_a_scan_of_its_own() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, names_end) = nested_archive();
    let path = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let parent = scanned(&mut state, path.clone());
    let listed = rows(&state, parent);
    let names_at = names_end - 0x40;
    assert_eq!(
        offsets(&listed),
        [0, NESTED, NESTED + 0x1B0, names_at, LAST, LAST + 0x1A0],
        "{listed:?}"
    );

    state.open_scanned_body(parent, row_at(&listed, NESTED));
    settle(&mut state);
    let nested = state.active_document.expect("an open document");
    assert_ne!(nested, parent);
    assert_eq!(
        state.document(nested).map(|d| d.kind),
        Some(DocumentKind::Rpf9)
    );
    until_scanned(&state, nested);
    let Some(DocPayload::Rpf9(scan)) = state.document(nested).map(|d| &d.payload) else {
        unreachable!()
    };
    assert_eq!((scan.start, scan.path.as_path()), (NESTED, path.as_path()));
    let found = scan.found();
    assert_eq!(found.ended, Some(Ended::Whole));
    assert_eq!(found.reached, names_end);
    assert_eq!(offsets(&found.rows), [NESTED, NESTED + 0x1B0, names_at]);
    assert!(
        matches!(&found.rows[2], Row::Body(f) if f.kind == Rpf9BodyKind::Names { archive: NESTED })
    );
    drop(found);

    // Its own header does not open again; the second nested archive, whose names were not
    // placed, does not open from the list, and from its path says why its scan stopped.
    state.open_scanned_body(nested, 0);
    assert!(said(&state, ToastType::Info, &["own header"]));
    let before = state.documents.len();
    state.open_scanned_body(parent, row_at(&listed, LAST));
    assert!(said(&state, ToastType::Info, &["does not open on its own"]));
    assert_eq!(state.documents.len(), before);
    state.load_file(body_path(&path, LAST, View::Archive));
    settle(&mut state);
    let unplaced = state.active_document.expect("an open document");
    until_scanned(&state, unplaced);
    let Some(DocPayload::Rpf9(scan)) = state.document(unplaced).map(|d| &d.payload) else {
        unreachable!()
    };
    assert!(
        matches!(&scan.found().ended, Some(Ended::Failed(why)) if why.contains("not known")),
        "{:?}",
        scan.found().ended
    );
}

#[test]
fn the_list_filter_examines_a_bounded_number_of_rows_a_frame_and_only_new_ones_after() {
    let missing = |i: u64| Row::Missing {
        from: i * 16,
        to: i * 16 + 8,
        bytes: 8,
    };
    let mut rows: Vec<Row> = (0..120_000).map(missing).collect();
    let words = |r: &Row| match r {
        Row::Missing { from, .. } => format!("{from:#X}"),
        Row::Body(f) => format!("{:#X}", f.offset),
    };
    let mut list = ListFilter::default();
    assert_eq!(list.shown(&rows, words), None, "no filter shows every row");

    list.kinds.insert(RowKind::Missing);
    let mut frames = 0;
    while list.filtering(rows.len()) {
        list.shown(&rows, words);
        frames += 1;
    }
    assert!(frames > 1, "a list this long was filtered in one frame");
    assert_eq!(
        list.shown(&rows, words).map(<[usize]>::len),
        Some(rows.len())
    );

    rows.extend((120_000..120_010).map(missing));
    assert!(list.filtering(rows.len()), "the rows added are examined");
    list.shown(&rows, words);
    assert!(
        !list.filtering(rows.len()),
        "only the rows added were examined"
    );

    // An offset in the Find field keeps every row and sends the list to its own, once.
    let at = |offset: &str| {
        let row = rows.iter().position(|r| words(r) == offset);
        row.unwrap_or_else(|| panic!("no row at {offset}"))
    };
    list.text = "0X1D4C0".to_string();
    list.seek(&rows);
    assert_eq!((list.jumped, list.scroll), (Some(at("0x1D4C0")), true));
    assert_eq!(
        list.shown(&rows, words).map(<[usize]>::len),
        Some(rows.len())
    );
    list.scroll = false;
    list.seek(&rows);
    assert!(!list.scroll, "the same offset sent the list there twice");

    // Words keep the rows that hold them.
    list.text = "1d4c00".to_string();
    list.shown(&rows, words);
    while list.filtering(rows.len()) {
        list.shown(&rows, words);
    }
    assert_eq!(list.shown(&rows, words), Some(&[at("0x1D4C00")][..]));
    list.kinds = [RowKind::Text].into();
    while list.filtering(rows.len()) {
        list.shown(&rows, words);
    }
    assert_eq!(list.shown(&rows, words), Some(&[][..]));

    assert_eq!(row_at_offset(&rows, 0x105), Some(0x10));
    assert_eq!(row_at_offset(&rows, 0x100), Some(0x10));
    assert_eq!(row_at_offset(&rows[1..], 0x5), None);
}

fn draw_list(state: &mut AppState) -> Vec<Painted> {
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1100.0, 900.0),
    ));
    let mut output = None;
    for _ in 0..3 {
        output = Some(panel.frame(Vec::new(), |ui| files_and_centre(ui, state)));
    }
    painted(&output.expect("drawn"))
}

fn left_of(painted: &[Painted], text: impl Fn(&str) -> bool) -> f32 {
    (painted.iter())
        .find(|p| text(&p.text))
        .unwrap_or_else(|| panic!("nothing painted fits: {painted:?}"))
        .rect
        .left()
}

#[test]
fn the_list_s_archive_heading_lines_up_with_its_bodies_and_the_filter_hides_other_kinds() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, _) = nested_archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));

    let shown = draw_list(&mut state);
    let heading = left_of(&shown, |t| t.starts_with("archive header:"));
    let kind = left_of(&shown, |t| t.trim() == "XML");
    assert!((heading - kind).abs() < 0.5, "{heading} against {kind}");
    let header_at = left_of(&shown, |t| t == format!("{NESTED:#012X}"));
    let body_at = left_of(&shown, |t| t == format!("{:#012X}", NESTED + 0x1B0));
    assert!(
        (header_at - body_at).abs() < 0.5,
        "{header_at} against {body_at}"
    );

    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| &d.payload) else {
        unreachable!()
    };
    scan.list().kinds.insert(RowKind::Text);
    let shown = draw_list(&mut state);
    let texts: Vec<&str> = shown.iter().map(|p| p.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.trim() == "XML"), "{texts:?}");
    assert!(texts.iter().any(|t| t.trim() == "text"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t.starts_with("archive header:")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("2 rows of 6 match")),
        "{texts:?}"
    );
}

#[test]
fn a_nested_archive_s_scan_list_is_filtered_and_extracts_as_the_top_one_s_does() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, _) = nested_archive();
    let mut state = AppState::new();
    let parent = scanned(&mut state, written(dir.path(), &bytes));
    state.open_scanned_body(parent, row_at(&rows(&state, parent), NESTED));
    settle(&mut state);
    let nested = state.active_document.expect("an open document");
    assert_ne!(nested, parent);
    until_scanned(&state, nested);
    let Some(DocPayload::Rpf9(scan)) = state.document(nested).map(|d| &d.payload) else {
        unreachable!()
    };
    scan.list().kinds.insert(RowKind::Text);
    let shown = draw_list(&mut state);
    let texts: Vec<&str> = shown.iter().map(|p| p.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.trim() == "XML"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t.starts_with("archive header:")),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("1 row of 3 matches")),
        "{texts:?}"
    );

    let out = dir.path().join("out");
    let xml = row_at(&rows(&state, nested), NESTED + 0x1B0);
    assert!(state.extract_scanned_bodies(nested, &[xml], out.clone()));
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(Instant::now() < deadline, "the extraction never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let written = std::fs::read(out.join(format!("{:#X}.xml", NESTED + 0x1B0)));
    assert_eq!(
        written.expect("the XML"),
        b"<?xml version=\"1.0\"?>\n<CMapTypes/>\n"
    );
}

fn draw_inspector(state: &mut AppState) -> String {
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(900.0, 900.0),
    ));
    let mut output = None;
    for _ in 0..3 {
        output = Some(panel.frame(Vec::new(), |ui| {
            crate::ui::panels::inspector::draw(ui, state)
        }));
    }
    let texts: Vec<String> = (painted(&output.expect("drawn")).into_iter())
        .map(|p| p.text)
        .collect();
    texts.join("\n")
}

#[test]
fn a_body_opened_from_the_list_says_where_it_was_found_and_how_its_blocks_read() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let path = written(dir.path(), &bytes);
    let mut state = AppState::new();
    let doc = scanned(&mut state, path.clone());
    let listed = rows(&state, doc);
    let kind = |offset: u64| match &listed[row_at(&listed, offset)] {
        Row::Body(f) => f.kind,
        Row::Missing { .. } => unreachable!(),
    };
    assert!(matches!(kind(at[1]), Rpf9BodyKind::Zlib(r) if r.content == Rpf9Content::Xml));
    assert!(matches!(kind(at[4]), Rpf9BodyKind::Zlib(r) if r.resource.is_some()));

    state.open_scanned_body(doc, row_at(&listed, at[1]));
    settle(&mut state);
    let said = draw_inspector(&mut state);
    for words in [
        "Found by a scan of a GTA VI archive".to_string(),
        path.display().to_string(),
        format!("{:#X}", at[1]),
        "XML".to_string(),
    ] {
        assert!(
            said.contains(&words),
            "the inspector lacks {words:?}:\n{said}"
        );
    }
    assert!(
        !said.contains("Block map"),
        "an XML body has no block map:\n{said}"
    );

    state.open_scanned_body(doc, row_at(&listed, at[4]));
    settle(&mut state);
    let said = draw_inspector(&mut state);
    for words in [
        format!("{:#X}", at[4]),
        "Block map".to_string(),
        "counts 1 virtual and 0 physical blocks".to_string(),
        "every block its block map counts was found".to_string(),
    ] {
        assert!(
            said.contains(&words),
            "the inspector lacks {words:?}:\n{said}"
        );
    }

    // A resource in a file of its own says so, and names no archive or offset.
    let (_, stored) =
        or_io::container::rpf::extract_rpf9_body(std::io::Cursor::new(&bytes), at[4], u64::MAX)
            .expect("it extracts");
    let loose = dir.path().join("loose.rsc");
    std::fs::write(&loose, stored).expect("written");
    state.load_file(loose.clone());
    settle(&mut state);
    let said = draw_inspector(&mut state);
    assert!(
        said.contains("A GTA VI resource in a file of its own")
            && said.contains(&loose.display().to_string()),
        "{said}"
    );
    assert!(!said.contains("Found by a scan"), "{said}");

    // A document of any other origin says nothing of a scan.
    let other = dir.path().join("plain.txt");
    std::fs::write(&other, b"plain").expect("written");
    state.load_file(other);
    settle(&mut state);
    assert!(!draw_inspector(&mut state).contains("Found by a scan"));
}

#[test]
fn a_body_no_reader_fits_opens_read_only_as_its_bytes_and_its_inspector_says_why() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    state.preview_scanned_row(doc, row_at(&rows(&state, doc), at[9]));
    settle(&mut state);
    let body = state.active_document().expect("an open document");
    assert_ne!(body.id, doc, "no tab: {}", state.error_dialog_message);
    let (DocumentKind::Hex, DocPayload::Hex(hex)) = (body.kind, &body.payload) else {
        panic!("not shown as its bytes: {:?}", body.kind);
    };
    assert_eq!(hex.bytes, [0xA5; 300], "its content, inflated");
    assert!(body.read_only.is_some(), "not read-only");
    assert!(state.is_preview(body.id), "a row's click opened a kept tab");
    assert_eq!(state.scan_listing_the_active_tab(), Some(doc));
    assert_eq!(state.scan_row_on_screen(doc), Some(at[9]));
    assert!(!said(&state, ToastType::Info, &[]), "a toast was raised");
    let said = draw_inspector(&mut state);
    for words in ["Shown as bytes", "what it holds is not known"] {
        assert!(
            said.contains(words),
            "the inspector lacks {words:?}:\n{said}"
        );
    }
}

#[test]
fn the_row_of_a_body_no_reader_fits_says_under_the_pointer_that_its_bytes_open() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, _) = archive();
    let mut state = AppState::new();
    scanned(&mut state, written(dir.path(), &bytes));
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1100.0, 900.0),
    ));
    let mut frame = |state: &mut AppState, events: Vec<egui::Event>| {
        painted(&panel.frame(events, |ui| files_and_centre(ui, state)))
    };
    for _ in 0..2 {
        frame(&mut state, Vec::new());
    }
    let at = (frame(&mut state, Vec::new()).iter())
        .find(|p| p.text.trim() == "zlib, not known")
        .expect("the row is drawn")
        .rect
        .center();
    // Held still for long enough for a tooltip.
    let mut said = frame(&mut state, vec![egui::Event::PointerMoved(at)]);
    for _ in 0..45 {
        said = frame(&mut state, Vec::new());
    }
    let tip = (said.iter()).find(|p| p.text.contains("Click to open its bytes read-only."));
    let tip = tip.unwrap_or_else(|| panic!("the row says no such thing: {said:?}"));
    assert!(tip.text.contains("what it holds is not known"), "{tip:?}");
}

#[test]
fn a_row_with_no_end_a_read_knows_says_why_and_opens_nothing() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, names_end) = nested_archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let listed = rows(&state, doc);
    // Its own header, whose facts its tab gives, and the names of the archive nested in it.
    for (offset, words) in [(0, "the archive's tab"), (names_end - 0x40, "no key")] {
        state.toasts.clear();
        state.preview_scanned_row(doc, row_at(&listed, offset));
        settle(&mut state);
        assert!(
            said(&state, ToastType::Info, &[words]),
            "the row at {offset:#X} said nothing of {words}"
        );
        assert_eq!(state.documents.len(), 1, "the row at {offset:#X} opened");
    }
}

#[test]
fn the_same_offset_of_two_archives_opens_under_two_tab_titles() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let other = dir.path().join("other.rpf");
    std::fs::write(&other, &bytes).expect("written");
    let mut state = AppState::new();
    let mut titles = Vec::new();
    for path in [written(dir.path(), &bytes), other] {
        let doc = scanned(&mut state, path);
        state.open_scanned_body(doc, row_at(&rows(&state, doc), at[1]));
        settle(&mut state);
        let body = state.active_document().expect("the body opened");
        assert_ne!(body.id, doc, "the body opened no tab of its own");
        titles.push(state.tab_title(body));
    }
    let name = format!("{:#X}.", at[1]);
    assert!(titles.iter().all(|t| t.starts_with(&name)), "{titles:?}");
    assert!(titles[0].ends_with(" — data.rpf"), "{titles:?}");
    assert!(titles[1].ends_with(" — other.rpf"), "{titles:?}");
}

#[test]
fn back_and_forward_step_through_the_bodies_previewed_and_the_list_marks_the_one_on_screen() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    // The body on screen: its tab is listed by its archive's scan, on the row it opened from.
    let on_screen = |state: &AppState| {
        let body = state.active_document.filter(|&id| id != doc);
        assert_eq!(state.scan_listing_the_active_tab(), Some(doc));
        body.and_then(|_| state.scan_row_on_screen(doc))
    };
    for offset in [at[1], at[2]] {
        state.preview_scanned_row(doc, row_at(&rows(&state, doc), offset));
        settle(&mut state);
        assert_eq!(on_screen(&state), Some(offset));
    }

    assert!(state.navigate_back(), "nothing to go back to");
    settle(&mut state);
    assert_eq!(on_screen(&state), Some(at[1]));
    assert_eq!(state.documents.len(), 2, "the preview tab was not reused");

    assert!(state.navigate_forward(), "nothing to go forward to");
    settle(&mut state);
    assert_eq!(on_screen(&state), Some(at[2]));
    assert_eq!(state.documents.len(), 2, "the preview tab was not reused");
}

#[test]
fn the_next_row_previewed_takes_the_preview_tab_and_a_kept_body_stays() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let preview = |state: &mut AppState, offset: u64| {
        state.preview_scanned_row(doc, row_at(&rows(state, doc), offset));
        settle(state);
    };
    let open = |state: &AppState| -> Vec<String> {
        let bodies = state.documents.iter().filter(|d| d.id != doc);
        bodies.map(|d| d.file_name()).collect()
    };
    let named =
        |offsets: &[u64]| -> Vec<String> { offsets.iter().map(|at| format!("{at:#X}.")).collect() };
    let holds = |state: &AppState, offsets: &[u64]| {
        let (open, named) = (open(state), named(offsets));
        open.len() == named.len() && open.iter().zip(&named).all(|(o, n)| o.starts_with(n))
    };

    preview(&mut state, at[1]);
    let first = state.active_document.expect("the body opened");
    assert!(state.is_preview(first), "a row's click opened a kept tab");
    preview(&mut state, at[2]);
    assert!(holds(&state, &[at[2]]), "{:?}", open(&state));

    // A double click keeps it, and the next row opens beside it.
    state.open_scanned_body(doc, row_at(&rows(&state, doc), at[2]));
    settle(&mut state);
    preview(&mut state, at[5]);
    assert!(holds(&state, &[at[2], at[5]]), "{:?}", open(&state));

    // An open that no click in the list asked for keeps its tab, and what it opens.
    state.open_scanned_body(doc, row_at(&rows(&state, doc), at[1]));
    settle(&mut state);
    assert!(holds(&state, &[at[2], at[5], at[1]]), "{:?}", open(&state));
    let kept = state.active_document.expect("the body opened");
    assert!(!state.is_preview(kept));
}

#[test]
fn a_preview_replaced_before_it_has_loaded_leaves_no_tab() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let listed = rows(&state, doc);
    // Three clicks before a frame takes the first load: one loading, one waiting, one wanted.
    for offset in [at[1], at[2], at[5]] {
        state.preview_scanned_row(doc, row_at(&listed, offset));
    }
    settle(&mut state);
    let bodies = state.documents.iter().filter(|d| d.id != doc);
    let open: Vec<String> = bodies.map(|d| d.file_name()).collect();
    assert!(
        open.len() == 1 && open[0].starts_with(&format!("{:#X}.", at[5])),
        "{open:?}"
    );
    let shown = state.active_document.expect("the body opened");
    assert!(state.is_preview(shown), "the last row's tab is kept");
}

#[test]
fn the_extract_button_takes_the_rows_selected_or_else_every_row_shown_and_shift_takes_a_range() {
    use super::rpf9_scan::{rows_between, rows_to_extract};
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| d.payload.clone()) else {
        panic!("not a GTA VI archive document");
    };
    let listed = rows(&state, doc);
    let none = std::collections::BTreeSet::new();

    // One kind toggled on, then the button: every row of that kind, and no other, is written.
    scan.list().kinds.insert(RowKind::Text);
    let said: Vec<String> = draw_list(&mut state).into_iter().map(|p| p.text).collect();
    assert!(
        said.iter().any(|t| t == "Extract the 2 rows shown…"),
        "{said:?}"
    );
    let text: Vec<usize> = (0..listed.len())
        .filter(|&row| RowKind::of(&listed[row]) == RowKind::Text)
        .collect();
    let shown = scan
        .list()
        .shown(&listed, |_| String::new())
        .map(<[usize]>::to_vec);
    assert_eq!(shown.as_deref(), Some(&text[..]));
    let taken = rows_to_extract(&none, shown.as_deref(), listed.len());
    assert_eq!(taken, text);
    let out = dir.path().join("out");
    assert!(state.extract_scanned_bodies(doc, &taken, out.clone()));
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(Instant::now() < deadline, "the extraction never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut written: Vec<String> = std::fs::read_dir(&out)
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    written.sort();
    assert_eq!(
        written,
        [format!("{:#X}.xml", at[1]), format!("{:#X}.txt", at[2])]
    );

    // Rows selected are taken in place of the rows shown, and with no filter every row is.
    let picked = [text[1]].into();
    assert_eq!(
        rows_to_extract(&picked, shown.as_deref(), listed.len()),
        [text[1]]
    );
    assert_eq!(rows_to_extract(&none, None, 3), [0, 1, 2]);

    // A Shift+click takes the rows shown between the one clicked last and this one, either way round.
    assert_eq!(rows_between(1, 4, None), [1, 2, 3, 4]);
    assert_eq!(rows_between(4, 1, None), [1, 2, 3, 4]);
    assert_eq!(rows_between(6, 1, Some(&[0, 2, 5, 6, 9])), [2, 5, 6]);
}

#[test]
fn an_offset_typed_in_the_find_field_marks_its_row_and_hides_none() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let Some(DocPayload::Rpf9(scan)) = state.document(doc).map(|d| d.payload.clone()) else {
        panic!("not a GTA VI archive document");
    };
    scan.list().text = format!("{:#x}", at[4]);
    let said: Vec<String> = draw_list(&mut state).into_iter().map(|p| p.text).collect();
    let row = row_at(&rows(&state, doc), at[4]);
    assert_eq!(scan.list().jumped, Some(row));
    assert!(!said.iter().any(|t| t.contains("match")), "{said:?}");
}

#[test]
fn a_ctrl_click_then_a_shift_click_select_the_rows_between_them_and_open_none() {
    use super::the_files_view_in_the_centre::{labels, run};
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (bytes, at) = archive();
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        run(&ctx, &mut state, Vec::new(), egui::Modifiers::NONE);
    }
    let centre_of = |state: &mut AppState, offset: u64| {
        let named = format!("{offset:#012X}  ");
        let drawn = labels(&run(&ctx, state, Vec::new(), egui::Modifiers::NONE));
        let row = drawn.iter().find(|(text, _)| text.starts_with(&named));
        row.unwrap_or_else(|| panic!("no row at {offset:#X} among {drawn:?}"))
            .1
    };
    let click = |state: &mut AppState, at: egui::Pos2, modifiers: egui::Modifiers| {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let moved = egui::Event::PointerMoved(at);
        run(&ctx, state, vec![moved, press(true)], modifiers);
        run(&ctx, state, vec![press(false)], modifiers);
    };
    let (first, last) = (centre_of(&mut state, at[1]), centre_of(&mut state, at[3]));
    click(&mut state, first, egui::Modifiers::COMMAND);
    click(&mut state, last, egui::Modifiers::SHIFT);

    let selected: std::collections::BTreeSet<usize> = ctx
        .data(|d| d.get_temp(egui::Id::new(("scanned bodies selected", doc))))
        .unwrap_or_default();
    let listed = rows(&state, doc);
    let between: std::collections::BTreeSet<usize> =
        (row_at(&listed, at[1])..=row_at(&listed, at[3])).collect();
    assert_eq!(selected, between);
    settle(&mut state);
    assert_eq!(
        state.active_document,
        Some(doc),
        "a click that selects opened a body"
    );
}

#[test]
fn a_row_is_of_the_kind_its_family_in_or_io_names_for_every_family() {
    use or_io::container::rpf::{Rpf9Family as Family, Rpf9ScanFind, Rpf9Signature, Rpf9ZlibRun};
    use {RowKind as Kind, Rpf9Content as Content};
    // A virtual section's first 0x40 bytes whose slots have `classes`: `0` zero, `P` a pointer
    // into the section, `h` any other word.
    let head = |classes: &[u8; 8]| -> Vec<u8> {
        let word = |class: &u8| match class {
            b'0' => 0u64,
            b'P' => 0x5000_0000,
            _ => 1,
        };
        classes.iter().flat_map(|c| word(c).to_le_bytes()).collect()
    };
    let zlib = |content, head: Option<&[u8]>| {
        Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            content,
            chunks: 1,
            stored_pieces: 0,
            first_chunk_len: 0,
            first_chunk_inflated: 0,
            inflated: 0,
            signature: head.and_then(|head| Rpf9Signature::of(head, 0x100)),
            resource: None,
        })
    };
    let resource = |head: &[u8]| zlib(Content::ResourceShaped, Some(head));
    // A navigation mesh states its version in the upper half of its third slot.
    let mut mesh = head(b"0Ph0h0h0");
    mesh[0x14..0x18].copy_from_slice(&0x0001_0015u32.to_le_bytes());
    let awc = Rpf9BodyKind::Awc {
        flags: 0,
        streams: 1,
        chunks: 1,
    };
    let movie = Rpf9BodyKind::Bink2 {
        revision: b'n',
        length: 0,
    };
    let stored = |version| Rpf9BodyKind::StoredRsc8 { version, extent: 0 };
    let names = Rpf9BodyKind::Names { archive: 0x1000 };
    let effects = Rpf9BodyKind::Sgd5 { head_layouts: 1 };
    let cases = [
        (resource(&head(b"0PP0hhhh")), Family::Models, Kind::Model),
        (
            resource(&head(b"0P0hPhPh")),
            Family::Dictionaries,
            Kind::Dictionary,
        ),
        (
            resource(&head(b"0Ph0hhhh")),
            Family::Collision,
            Kind::Collision,
        ),
        (resource(&mesh), Family::Navigation, Kind::Navigation),
        (
            zlib(Content::ClipRoot, None),
            Family::Animations,
            Kind::Animation,
        ),
        (zlib(Content::Pso, None), Family::Metadata, Kind::Metadata),
        (zlib(Content::Xml, None), Family::Text, Kind::Text),
        (awc, Family::Audio, Kind::Audio),
        (movie, Family::Movies, Kind::Movie),
        (effects, Family::Shaders, Kind::Shaders),
        (names, Family::Archives, Kind::Archive),
        (stored(1), Family::Other, Kind::Resource),
        (zlib(Content::Other, None), Family::Unknown, Kind::Unknown),
    ];
    assert_eq!(cases.map(|case| case.1), Family::ALL);
    let animation = stored(or_io::gta6::anim::RSC8_VERSION);
    let stored_animation = (animation, Family::Animations, Kind::Animation);
    for (kind, family, row) in cases.into_iter().chain([stored_animation]) {
        let find = Rpf9ScanFind {
            offset: 0,
            kind,
            span: 0,
            stored_len: None,
            inflated_len: None,
            container: None,
        };
        assert_eq!(Family::of(&find), family, "{kind:?}");
        assert_eq!(Family::parse(family.word()), Some(family));
        assert_eq!(Kind::of(&Row::Body(Box::new(find))), row, "{kind:?}");
    }
    let missing = Row::Missing {
        from: 0,
        to: 1,
        bytes: 1,
    };
    assert_eq!(Kind::of(&missing), Kind::Missing);
}
