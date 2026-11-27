//! An edited scanned GTA VI height map written by its own writer: the stored body read again on the
//! worker for the bytes the document does not hold, the document's grid put in its place.

use std::sync::Arc;

use or_io::gta6::other_bodies::{Gta6HeightGrid, Gta6HeightMap, OtherBody};

use super::document::{DocPayload, Document};
use super::gta6_resource_save::EditedSave;

#[path = "height_map_cell.rs"]
mod cell;

/// A file's bytes, made on the worker that writes them.
pub type Write = dyn Fn() -> Result<Vec<u8>, String> + Send + Sync;

/// The height map row's `edited_sections` where `document` is an edited height map a GTA VI scan
/// found; `None` for any other.
pub(super) fn edited(document: &Document) -> Option<Result<EditedSave, String>> {
    let DocPayload::HeightMap(grid) = &document.payload else {
        return None;
    };
    if !document.dirty {
        return None;
    }
    let (archive, offset) = super::rpf9_scan::scanned_body(document.path.as_deref()?)?;
    let (archive, grid) = (archive.to_path_buf(), grid.clone());
    let write: Arc<Write> = Arc::new(move || {
        let stored = super::rpf9_scan::extracted_body(&archive, offset)?;
        if OtherBody::of(&stored, stored.len() as u64) == Some(OtherBody::HeightGrid) {
            let mut body = Gta6HeightGrid::open(&stored).map_err(|e| e.to_string())?;
            body.set_height_map(&grid).map_err(|e| e.to_string())?;
            return body.save().map_err(|e| e.to_string());
        }
        let mut map = Gta6HeightMap::open(&stored).map_err(|e| e.to_string())?;
        map.grid = grid.clone();
        map.save().map_err(|e| e.to_string())
    });
    Some(Ok(EditedSave::Written(write)))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use or_io::gta6::other_bodies::Gta6HeightMap;

    use crate::state::opening_an_rpf9_archive::{
        row_at, rows, rpf9_head, scanned, settle, written, zlib,
    };
    use crate::state::{AppState, DocPayload, DocumentKind};

    const CARRIED: [u8; 7] = [0x13, 0x9E, 0x04, 0xB7, 0x60, 0x2D, 0xF1];

    /// A 3 x 2 GTA VI height map whose every cell differs, with seven carried bytes.
    fn pamh() -> Vec<u8> {
        let mut out = b"PAMH\x01".to_vec();
        out.extend_from_slice(&CARRIED);
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        for v in [512.0f32, 1_024.0, -40.0, 1_024.0, 1_536.0, 210.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(2u32 * 8 + 2 * 6).to_le_bytes());
        for row in 0..2u32 {
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&3u16.to_le_bytes());
            out.extend_from_slice(&(row * 3).to_le_bytes());
        }
        out.extend_from_slice(&[90, 81, 72, 63, 54, 45]);
        out.extend_from_slice(&[11, 12, 13, 14, 15, 16]);
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

    #[test]
    fn an_edited_scanned_height_map_saves_through_its_writer_with_its_carried_bytes() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1010, 0);
        bytes.extend(zlib(&pamh()));
        let mut state = AppState::new();
        let scan = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(scan, row_at(&rows(&state, scan), 0x1010));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != scan)
            .unwrap_or_else(|| panic!("the body: {}", state.error_dialog_message));
        assert_eq!(
            state.document(body).map(|d| d.kind),
            Some(DocumentKind::HeightMap)
        );

        // Unedited, Save As writes the stored body as it is.
        let copy = dir.path().join("copy.hmap");
        assert!(state.save_active_document_as(copy.clone()));
        finished(&mut state);
        assert_eq!(std::fs::read(&copy).expect("the copy"), pamh());

        let doc = state.document_mut(body).expect("open");
        let DocPayload::HeightMap(grid) = &mut doc.payload else {
            panic!("a height map");
        };
        grid.max[4] = 200;
        grid.min[1] = 3;
        doc.mark_edited();
        let saved = dir.path().join("saved.hmap");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let read = Gta6HeightMap::open(&std::fs::read(&saved).expect("the file")).expect("opens");
        let mut expected = Gta6HeightMap::open(&pamh()).expect("opens");
        expected.grid.max[4] = 200;
        expected.grid.min[1] = 3;
        assert_eq!(read, expected);
        assert_eq!(read.carried, CARRIED);
    }

    /// A 3 x 2 grid of one array: row 0 holds columns 1 and 2, row 1 is empty; byte 0x28 is 4.
    fn mhiu() -> Vec<u8> {
        let mut out = b"MHIU\x01\x00".to_vec();
        for v in [3u16, 2, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [512.0f32, 1_024.0, -40.0, 1_024.0, 1_536.0, 210.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(2u32 * 8 + 2).to_le_bytes());
        out.extend_from_slice(&[4, 0xA1, 0xB2, 0xC3]);
        for (start, count, offset) in [(1u16, 2u16, -1i32), (0, 0, 0)] {
            out.extend_from_slice(&start.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
        }
        out.extend_from_slice(&[61, 62]);
        out
    }

    #[test]
    fn an_edited_scanned_height_grid_saves_through_its_own_writer() {
        use or_io::gta6::other_bodies::Gta6HeightGrid;

        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1010, 0);
        bytes.extend(zlib(&mhiu()));
        let mut state = AppState::new();
        let scan = scanned(&mut state, written(dir.path(), &bytes));
        state.open_scanned_body(scan, row_at(&rows(&state, scan), 0x1010));
        settle(&mut state);
        let body = (state.active_document)
            .filter(|&d| d != scan)
            .unwrap_or_else(|| panic!("the body: {}", state.error_dialog_message));
        assert_eq!(
            state.document(body).map(|d| d.kind),
            Some(DocumentKind::HeightMap)
        );

        let copy = dir.path().join("copy.hmap");
        assert!(state.save_active_document_as(copy.clone()));
        finished(&mut state);
        assert_eq!(std::fs::read(&copy).expect("the copy"), mhiu());

        let doc = state.document_mut(body).expect("open");
        let DocPayload::HeightMap(grid) = &mut doc.payload else {
            panic!("a height map");
        };
        // Row 1's column 2, outside every span: both halves, as the grid holds one array.
        (grid.max[5], grid.min[5]) = (99, 99);
        let edited = grid.clone();
        doc.mark_edited();
        let saved = dir.path().join("saved.hmap");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let said: Vec<_> = state.toasts.iter().map(|t| t.message.clone()).collect();
        let out = std::fs::read(&saved).unwrap_or_else(|e| panic!("{e}: {said:?}"));
        let read = Gta6HeightGrid::open(&out).expect("opens as a height grid");
        assert_eq!(read.height_map().expect("a height map"), edited);
        assert_eq!(read.carried, u32::from_le_bytes([4, 0xA1, 0xB2, 0xC3]));
    }
}
