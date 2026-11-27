//! A height map's one edit: a cell's height set, as the inspector's field sets it, as the byte the
//! file stores.

use crate::state::{AppState, Change, DocPayload, DocumentId};

fn cell_at(p: &mut DocPayload, at: usize) -> Option<&mut u8> {
    match p {
        DocPayload::HeightMap(map) => map.max.get_mut(at),
        _ => None,
    }
}

fn floor_at(p: &mut DocPayload, at: usize) -> Option<&mut u8> {
    match p {
        DocPayload::HeightMap(map) => map.min.get_mut(at),
        _ => None,
    }
}

impl AppState {
    /// Sets the cell at `col`, `row` of the height map `doc` to `height`, recorded as one step.
    pub(crate) fn set_height_map_cell(
        &mut self,
        doc: DocumentId,
        col: u16,
        row: u16,
        height: u8,
    ) -> Result<(), String> {
        let d = self
            .document_mut(doc)
            .ok_or_else(|| tr!("its document is closed"))?;
        let DocPayload::HeightMap(map) = &mut d.payload else {
            return Err(tr!("not a height map").to_string());
        };
        let at = usize::from(row) * usize::from(map.num_cols) + usize::from(col);
        let before = (map.max.get(at).copied(), map.min.get(at).copied());
        map.set_cell(col, row, height).map_err(|e| e.to_string())?;
        // The set refuses a cell either half lacks, so both were read.
        let (Some(max), Some(min)) = before else {
            return Ok(());
        };
        if (map.max[at], map.min[at]) == (max, min) {
            return Ok(());
        }
        let change = Change::part(format!("height map cell {at}"), max, move |p| {
            cell_at(p, at)
        })
        .and_part(min, move |p| floor_at(p, at));
        let label = tr!("Set cell {col}, {row}", col = col, row = row);
        self.record(doc, label, change);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{AppState, DocPayload, Document};

    const DOC: crate::state::DocumentId = 9_711;

    fn opened() -> AppState {
        let map = or_io::height_map::HeightMap {
            version: 1,
            flags: or_io::height_map::HeightMap::RLE_DATA,
            num_cols: 3,
            num_rows: 2,
            bounds_min: [0.0, 0.0, -40.0],
            bounds_max: [300.0, 200.0, 210.0],
            max: vec![90, 81, 72, 63, 54, 45],
            min: vec![11, 81, 13, 14, 15, 16],
            water_mask: Vec::new(),
        };
        let mut state = AppState::new();
        let doc = Document::new(DOC, None, DocPayload::HeightMap(map));
        state.documents.push(doc);
        state
    }

    fn halves(state: &AppState) -> (Vec<u8>, Vec<u8>) {
        match state.document(DOC).map(|d| &d.payload) {
            Some(DocPayload::HeightMap(map)) => (map.max.clone(), map.min.clone()),
            _ => panic!("a height map"),
        }
    }

    #[test]
    fn a_cell_set_from_the_field_marks_the_map_edited_and_undoes_to_both_halves() {
        let mut state = opened();
        state.set_height_map_cell(DOC, 1, 0, 200).expect("sets");
        let (max, min) = halves(&state);
        // The cell's halves were equal, so both move, as a one-array grid needs.
        assert_eq!((max[1], min[1]), (200, 200));
        assert!(state.document(DOC).is_some_and(|d| d.dirty));
        state.set_height_map_cell(DOC, 0, 1, 10).expect("sets");
        assert_eq!(
            halves(&state).1[3],
            10,
            "a minimum is never left above its maximum"
        );
        let base = state.document(DOC).map(|d| d.history.base()).expect("open");
        state.go_to_step(DOC, base);
        assert_eq!(
            halves(&state),
            (vec![90, 81, 72, 63, 54, 45], vec![11, 81, 13, 14, 15, 16])
        );
        let error = state.set_height_map_cell(DOC, 3, 0, 1).unwrap_err();
        assert!(error.contains("outside the 3x2 grid"), "{error}");
    }

    /// A 2 x 2 GTA VI height map, the first of its seven carried bytes set.
    fn pamh() -> Vec<u8> {
        let mut out = b"PAMH\x01\x13\x00\x00\x00\x00\x00\x00".to_vec();
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        for v in [0.0f32, 0.0, -10.0, 512.0, 512.0, 90.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(2u32 * 8 + 2 * 4).to_le_bytes());
        for row in 0..2u32 {
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&2u16.to_le_bytes());
            out.extend_from_slice(&(row * 2).to_le_bytes());
        }
        out.extend_from_slice(&[40, 50, 60, 70]);
        out.extend_from_slice(&[4, 5, 6, 7]);
        out
    }

    #[test]
    fn a_cell_set_on_a_scanned_height_map_saves_through_its_writer_at_its_length() {
        use crate::state::opening_an_rpf9_archive::{
            row_at, rows, rpf9_head, scanned, settle, written, zlib,
        };
        use std::time::{Duration, Instant};

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
        // Column 1 of row 0, which holds 50 over 5.
        state.set_height_map_cell(body, 1, 0, 51).expect("sets");
        let saved = dir.path().join("saved.hmap");
        assert!(state.save_active_document_as(saved.clone()));
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(Instant::now() < deadline, "the save never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        let out = std::fs::read(&saved).expect("the file");
        let mut expected = pamh();
        expected[44 + 16 + 1] = 51;
        assert_eq!(out, expected, "only the cell's byte changes");
    }
}
