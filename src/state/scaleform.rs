//! A `.gfx` Scaleform movie as a document: its tags listed, exported to a
//! standard SWF and replaced by one imported back.

use or_io::gfx::GfxMovie;

use super::{AppState, Change, DocPayload, DocumentId};

impl AppState {
    pub fn gfx_movie(&self, doc: DocumentId) -> Result<&GfxMovie, String> {
        match self.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Gfx(movie)) => Ok(movie),
            _ => Err("not a .gfx document".to_string()),
        }
    }

    /// The movie as an SWF file's bytes.
    pub fn gfx_swf(&self, doc: DocumentId) -> Result<Vec<u8>, String> {
        self.gfx_movie(doc)?.save_swf().map_err(|e| e.to_string())
    }

    /// Replace the movie with the one an SWF file's bytes hold; a refused SWF changes nothing.
    pub fn gfx_import_swf(&mut self, doc: DocumentId, swf: &[u8]) -> Result<(), String> {
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY).to_string());
        }
        let imported = GfxMovie::open(swf).map_err(|e| e.to_string())?;
        let Some(document) = self.document_mut(doc) else {
            return Err("not a .gfx document".to_string());
        };
        let DocPayload::Gfx(movie) = &mut document.payload else {
            return Err("not a .gfx document".to_string());
        };
        let before = std::mem::replace(movie, imported);
        let change = Change::part("movie", before, |p| match p {
            DocPayload::Gfx(movie) => Some(movie),
            _ => None,
        });
        self.record(doc, "Replace the movie from an SWF", change);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::state::{DocumentKind, LoadingState};

    /// One frame: a background colour, a shape and the end, built by hand.
    fn movie(colour: u8) -> Vec<u8> {
        let mut body = vec![0x00, 0x00, 0x1E, 0x01, 0x00];
        body.extend_from_slice(&((9u16 << 6) | 3).to_le_bytes());
        body.extend_from_slice(&[colour, 0, 0]);
        body.extend_from_slice(&((2u16 << 6) | 4).to_le_bytes());
        body.extend_from_slice(&[3, 0, 0xAA, 0xBB]);
        body.extend_from_slice(&[0x40, 0x00, 0x00, 0x00]);
        let mut out = b"GFX\x08".to_vec();
        out.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn open(state: &mut AppState, path: PathBuf) -> DocumentId {
        state.load_file(path);
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.load_receiver.is_some() && Instant::now() < deadline {
            state.poll_load_result();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(state.loading, LoadingState::Idle),
            "the load did not finish: {}",
            state.error_dialog_message
        );
        let doc = state.active_document.expect("the .gfx opened");
        assert_eq!(state.document(doc).map(|d| d.kind), Some(DocumentKind::Gfx));
        doc
    }

    #[test]
    fn a_table_movie_refuses_replacement_and_still_exports() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::new();
        let path = "scaleform.rpf/.rpf9-table/0/hash_movie.gfx".into();
        state.documents.push(crate::state::Document::new(
            1,
            Some(path),
            DocPayload::Gfx(GfxMovie::open(&movie(0xFF)).unwrap()),
        ));
        let before = state.gfx_swf(1).unwrap();
        let mut replacement = movie(0x10);
        replacement[..3].copy_from_slice(b"FWS");
        assert!(state.gfx_import_swf(1, &replacement).is_err());
        assert_eq!(state.gfx_swf(1).unwrap(), before);
        let document = state.document(1).unwrap();
        assert!(!document.dirty);
        assert!(!document.history.can_undo());
    }

    #[test]
    fn a_gfx_opens_with_its_tags_and_exports_to_swf() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("hud.gfx");
        std::fs::write(&path, movie(0xFF)).expect("the movie is written");
        let mut state = AppState::new();
        let doc = open(&mut state, path);

        let codes: Vec<u16> = state
            .gfx_movie(doc)
            .unwrap()
            .tags
            .iter()
            .map(|t| t.code)
            .collect();
        assert_eq!(codes, [9, 2, 1, 0]);
        let swf = state.gfx_swf(doc).expect("the export");
        assert_eq!(&swf[..3], b"FWS");
        assert_eq!(swf[3..], movie(0xFF)[3..]);
    }

    #[test]
    fn an_imported_swf_replaces_the_movie_and_saves_as_gfx() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("hud.gfx");
        std::fs::write(&path, movie(0xFF)).expect("the movie is written");
        let mut state = AppState::new();
        let doc = open(&mut state, path);

        let mut zws = movie(0x10);
        zws[..3].copy_from_slice(b"ZWS");
        let why = state.gfx_import_swf(doc, &zws).expect_err("LZMA");
        assert!(why.contains("ZWS"), "{why}");
        assert!(!state.document(doc).expect("doc").dirty);

        let mut swf = movie(0x10);
        swf[..3].copy_from_slice(b"FWS");
        state.gfx_import_swf(doc, &swf).expect("the import");
        assert!(state.document(doc).expect("doc").dirty);

        let saved = dir.path().join("saved.gfx");
        assert!(state.save_active_document_as(saved.clone()));
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.save_receiver.is_some() && Instant::now() < deadline {
            state.poll_save_result();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read(&saved).expect("the save wrote"), movie(0x10));
    }

    #[test]
    fn an_imported_swf_undoes_to_the_movie_the_file_opened_with() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("hud.gfx");
        std::fs::write(&path, movie(0xFF)).expect("the movie is written");
        let mut state = AppState::new();
        let doc = open(&mut state, path);
        let written = |state: &AppState| state.gfx_movie(doc).unwrap().save().unwrap();
        let mut swf = movie(0x10);
        swf[..3].copy_from_slice(b"FWS");
        state.gfx_import_swf(doc, &swf).expect("the import");
        assert_eq!(written(&state), movie(0x10));

        assert_eq!(
            state.undo().as_deref(),
            Some("Replace the movie from an SWF")
        );
        assert_eq!(
            written(&state),
            movie(0xFF),
            "undone, it writes other bytes"
        );
        assert!(!state.document(doc).expect("doc").dirty);
        state.redo();
        assert_eq!(written(&state), movie(0x10), "redone, the import is lost");
    }
}
