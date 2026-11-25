//! One export over many files: the rows selected in Files, each written into one folder, as one job.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_interchange::gltf::{self, Container};
use or_interchange::{ExtractOptions, LodSelection};
use or_job::Step;

use super::document::{DocumentId, RpfNodeId};
use super::extract_job::extension_of;
use super::jobs::JobKind;
use super::{AppState, DocPayload, GateOutcome};

/// What a batch writes of each file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Batch {
    /// Each model as a `.glb` named for it, as Export to glTF… writes one with nothing changed in its question.
    Gltf,
    /// The first level of each texture of each dictionary as a PNG, in a folder named for the dictionary.
    Pictures,
}

impl Batch {
    /// Whether a file called `name` is one this batch takes.
    pub fn takes(self, name: &str) -> bool {
        let ext = extension_of(name);
        match self {
            Batch::Gltf => matches!(ext.as_str(), "ydr" | "ydd" | "yft"),
            Batch::Pictures => ext == "ytd",
        }
    }

    /// Write what this makes of the file `name` into `folder`: the bytes written, or why nothing was.
    fn write(self, name: &str, bytes: &[u8], folder: &Path) -> Result<u64, String> {
        let stem = Path::new(name)
            .file_stem()
            .map_or_else(|| "file".to_string(), |s| s.to_string_lossy().into_owned());
        match self {
            Batch::Gltf => {
                let options = ExtractOptions {
                    lods: LodSelection::Highest,
                    textures: true,
                };
                let exported = or_cli::export_scene(name, bytes, &stem, None, &options)?;
                let files = gltf::write(&exported.scene, &stem, Container::Glb);
                let dest = folder.join(format!("{stem}.glb"));
                std::fs::write(&dest, &files.primary)
                    .map_err(|e| format!("{}: {e}", dest.display()))?;
                Ok(files.primary.len() as u64)
            }
            Batch::Pictures => or_cli::export_pngs(name, bytes, &folder.join(&stem)),
        }
    }
}

impl AppState {
    /// The files among `rows` of `doc` that a batch of `kind` takes, each with its name, a folder or an archive walked down the rows it lists.
    pub fn batch_inputs(
        &self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        kind: Batch,
    ) -> Vec<(RpfNodeId, String)> {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut left: Vec<RpfNodeId> = rows.iter().rev().copied().collect();
        while let Some(row) = left.pop() {
            let Some(node) = rpf.session.node(row).filter(|_| seen.insert(row)) else {
                continue;
            };
            // ⚠ Only the rows already listed: an archive nobody has expanded holds none yet, so it adds nothing.
            let inside = rpf.session.children_of(row);
            if node.is_dir() || !inside.is_empty() {
                left.extend(inside.into_iter().rev());
            } else if kind.takes(&node.name) {
                out.push((row, node.name.clone()));
            }
        }
        out
    }

    /// Write what `kind` makes of each of `rows` into `folder`, as one job that Background jobs lists and cancels.
    ///
    /// ponytail: reads every file before the job starts, as a conversion of rows does; a selection is not an install, whose batches are `openrage-cli bulk`'s.
    pub fn batch_export(
        &mut self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        kind: Batch,
        folder: PathBuf,
    ) {
        let inputs = self.batch_inputs(doc, rows, kind);
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let read: Vec<(String, Arc<Vec<u8>>)> = (inputs.into_iter())
            .filter_map(|(row, name)| Some((name, rpf.session.bytes_for(row)?)))
            .collect();
        if read.is_empty() {
            return self.show_warning_toast(tr!("Nothing selected is a file this exports"));
        }
        if let Some(what) = self.extract_jobs.running_noun() {
            return self.show_error_toast(tr!(
                "{what} is running; export once it is done",
                what = what
            ));
        }
        match self.gate_permits(Some(&folder)) {
            GateOutcome::Proceed => {}
            GateOutcome::Refused => return,
            GateOutcome::Confirm(_) => {
                return self.show_warning_toast(tr!(
                    "A batch does not ask to write into the game as shipped: choose a folder \
                     outside the install or in its mods overlay"
                ))
            }
        }
        let label = tr!("{count} file(s)", count = read.len());
        let dest = folder.clone();
        self.extract_jobs.start(
            JobKind::Export,
            label,
            folder,
            Vec::new(),
            move |cancel, progress| {
                or_job::run(read, &cancel, &progress, |(name, bytes)| {
                    progress.now_on(name.clone());
                    match kind.write(&name, &bytes, &dest) {
                        Ok(written) => Step::Done(written),
                        Err(why) => Step::Failed(why),
                    }
                })
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_takes_the_files_of_its_kind_and_no_other() {
        assert!(Batch::Gltf.takes("adder.yft") && Batch::Gltf.takes("Prop.YDR"));
        assert!(!Batch::Gltf.takes("adder.ytd") && !Batch::Gltf.takes("notes.txt"));
        assert!(Batch::Pictures.takes("adder.ytd"));
        assert!(!Batch::Pictures.takes("adder.yft"));
    }

    #[test]
    fn a_model_and_a_dictionary_of_the_corpus_are_written_as_a_glb_and_as_pictures() {
        const WHAT: &str =
            "a_model_and_a_dictionary_of_the_corpus_are_written_as_a_glb_and_as_pictures";
        // ⚠ The first dictionary staged is 10 MB, and writing each of its textures as a picture took
        // this test past its 30 s cap; one file of each kind is all it asks.
        const MAX_BYTES: u64 = 1 << 20;
        let first = |ext: &str| {
            let found = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), ext).find();
            (found.into_iter())
                .find(|path| std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_BYTES))
        };
        let (Some(model), Some(dictionary)) = (first("ydr"), first("ytd")) else {
            or_test_support::skip_corpus(WHAT, "no .ydr or .ytd of 1 MiB or less staged");
            return;
        };
        let dir = tempfile::tempdir().expect("a scratch folder");
        for (kind, path) in [(Batch::Gltf, &model), (Batch::Pictures, &dictionary)] {
            let name = path.file_name().expect("a name").to_string_lossy();
            let bytes = std::fs::read(path).expect("a corpus file");
            let written = kind.write(&name, &bytes, dir.path());
            assert!(matches!(written, Ok(n) if n > 0), "{name}: {written:?}");
        }
        let stem = |path: &Path| path.file_stem().expect("a stem").to_owned();
        let glb = std::fs::read(dir.path().join(stem(&model)).with_extension("glb"));
        assert!(
            glb.as_ref().is_ok_and(|bytes| bytes.starts_with(b"glTF")),
            "no binary glTF named for the model"
        );
        let pictures = std::fs::read_dir(dir.path().join(stem(&dictionary))).expect("its folder");
        let png = (pictures.filter_map(Result::ok))
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .any(|bytes| bytes.starts_with(b"\x89PNG"));
        assert!(png, "no PNG in the folder named for the dictionary");
    }

    #[test]
    fn a_folder_selected_gives_the_files_listed_under_it_and_each_once() {
        use crate::state::navigation::{resolve_path, Arrival, Resolved};

        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::create_dir(dir.path().join("props")).expect("a folder");
        for name in [
            "props/lamp.ydr",
            "props/lamp.ytd",
            "props/notes.txt",
            "car.yft",
        ] {
            std::fs::write(dir.path().join(name), [0; 8]).expect("a file");
        }
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.open_folder(dir.path().to_path_buf());
        state.walk_to(
            dir.path(),
            vec!["props/lamp.ydr".to_string()],
            Arrival::Reveal,
        );
        let row = |state: &AppState, path: &str| {
            let doc = state.active_document?;
            let Some(DocPayload::Rpf(rpf)) = state.document(doc).map(|d| &d.payload) else {
                return None;
            };
            match resolve_path(&rpf.session, path) {
                Resolved::Found(id) => Some((doc, id)),
                _ => None,
            }
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while row(&state, "props/lamp.ydr").is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the folder never listed its rows"
            );
            state.poll_load_result();
            state.poll_expansions();
            state.tick_navigation();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (doc, props) = row(&state, "props").expect("the folder's row");
        let (_, lamp) = row(&state, "props/lamp.ydr").expect("the model's row");
        let names = |kind| -> Vec<String> {
            let inputs = state.batch_inputs(doc, &[props, lamp], kind);
            inputs.into_iter().map(|(_, name)| name).collect()
        };
        assert_eq!(
            names(Batch::Gltf),
            ["lamp.ydr"],
            "the folder and its file, once"
        );
        assert_eq!(names(Batch::Pictures), ["lamp.ytd"]);
    }
    #[test]
    fn a_file_that_is_no_model_is_a_failed_step_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let refused = Batch::Gltf.write("broken.ydr", b"not a drawable", dir.path());
        assert!(refused.is_err(), "{refused:?}");
        let refused = Batch::Pictures.write("broken.ytd", b"not a dictionary", dir.path());
        assert!(refused.is_err(), "{refused:?}");
        let left: Vec<_> = (std::fs::read_dir(dir.path()).expect("the folder"))
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .collect();
        assert!(left.is_empty(), "a refused file left {left:?}");
    }
}
