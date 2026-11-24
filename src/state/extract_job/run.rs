//! The extraction worker and its cancellable RPF9 output.

use std::io::{self, Write};
use std::path::PathBuf;

use or_io::container::rpf::BodyForm;
use or_job::{Cancel, Progress, Report, Step};

use super::{kept_copy, ExtractItem, ExtractSource, OnClash};

struct Rpf9Output<'a, W> {
    writer: &'a mut W,
    cancel: &'a Cancel,
}

impl<W: Write> Rpf9Output<'_, W> {
    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancel.is_set() {
            // Interrupted is retried by write_all, so cancellation must use a terminal error.
            return Err(io::Error::other("stopped"));
        }
        Ok(())
    }
}

impl<W: Write> Write for Rpf9Output<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check_cancelled()?;
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_cancelled()?;
        self.writer.flush()
    }
}

/// Write every item under `dest`, counting what fails and stopping when asked.
///
/// ⚠ Runs on a worker and touches no app state, which is why it is a free
/// function taking everything it needs.
pub fn run_extraction(
    items: Vec<ExtractItem>,
    dest: PathBuf,
    form: BodyForm,
    fixes: bool,
    on_clash: OnClash,
    cancel: Cancel,
    progress: Progress,
) -> Report {
    // ⚠ A save of several files, such as a cell and the neighbours whose links it renumbered,
    // writes none of them unless every one is made: a part written names records the rest lack.
    let (items, fixes) = match on_clash {
        OnClash::Save if items.len() > 1 => {
            let made: Result<Vec<ExtractItem>, String> = (items.into_iter())
                .map(|i| match i.export_bytes(form, fixes) {
                    Ok(bytes) => Ok(ExtractItem::bytes(i.rel_path, bytes)),
                    Err(e) => Err(format!("{}: {e}", i.rel_path)),
                })
                .collect();
            match made {
                Ok(made) => (made, false),
                Err(e) => return or_job::run([e], &cancel, &progress, Step::Failed),
            }
        }
        _ => (items, fixes),
    };
    let mut rpf9_cancelled = false;
    let mut report = or_job::run(items, &cancel, &progress, |item| {
        progress.now_on(item.rel_path.clone());
        let mut target = dest.join(&item.rel_path);
        if std::fs::symlink_metadata(&target).is_ok() {
            match on_clash {
                OnClash::Replace | OnClash::Save => {}
                OnClash::Skip => return Step::Skipped,
                OnClash::KeepBoth => target = kept_copy(&target),
            }
        }
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Step::Failed(format!("{}: {e}", parent.display()));
            }
        }
        // A scanned body its list gave an end is copied as a range, never held whole.
        if let (ExtractSource::Scanned { archive, offset }, false) =
            (&item.source, on_clash == OnClash::Save)
        {
            let copy = crate::state::gta6_first_entry::copied;
            if let Some(step) = copy(archive, *offset, &target, &item.rel_path, &cancel) {
                return step;
            }
        }
        if let (ExtractSource::Rpf9Table(source), false) = (&item.source, on_clash == OnClash::Save)
        {
            let write = || -> Result<u64, String> {
                let mut temp = tempfile::NamedTempFile::new_in(&dest).map_err(|e| e.to_string())?;
                let mut output = Rpf9Output {
                    writer: temp.as_file_mut(),
                    cancel: &cancel,
                };
                let count = source.extract(&mut output)?;
                output.check_cancelled().map_err(|e| e.to_string())?;
                if matches!(on_clash, OnClash::KeepBoth | OnClash::Skip) {
                    temp.persist_noclobber(&target).map_err(|e| e.to_string())?;
                } else {
                    temp.persist(&target).map_err(|e| e.to_string())?;
                }
                Ok(count)
            };
            return match write() {
                Ok(count) => Step::Done(count),
                Err(_) if cancel.is_set() => {
                    rpf9_cancelled = true;
                    Step::Skipped
                }
                Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
            };
        }
        let write = |bytes: &[u8]| match on_clash {
            OnClash::Save => crate::state::document_io::atomic_write_with_backup(&target, bytes),
            _ => std::fs::write(&target, bytes),
        };
        match item.export_bytes(form, fixes) {
            Ok(bytes) => match write(&bytes) {
                Ok(()) => Step::Done(bytes.len() as u64),
                Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
            },
            Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
        }
    });
    // A stop inside the final entry leaves no following iteration for or_job to observe it.
    report.cancelled |= rpf9_cancelled;
    report
}

#[cfg(test)]
mod tests {
    #[test]
    fn rpf9_output_stops_after_cancellation_during_a_write() {
        struct StopAfterWrite {
            bytes: Vec<u8>,
            cancel: super::Cancel,
        }
        impl std::io::Write for StopAfterWrite {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                self.cancel.set();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        use std::io::Write;
        let cancel = super::Cancel::new();
        let mut writer = StopAfterWrite {
            bytes: Vec::new(),
            cancel: cancel.clone(),
        };
        let mut output = super::Rpf9Output {
            writer: &mut writer,
            cancel: &cancel,
        };
        assert!(!cancel.is_set());
        output.write_all(b"first block").unwrap();
        assert!(cancel.is_set());
        let error = output.write_all(b"must not be written").unwrap_err();
        assert_ne!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(
            output.check_cancelled().is_err(),
            "the last block must not be published either"
        );
        assert_eq!(writer.bytes, b"first block");
    }
}
