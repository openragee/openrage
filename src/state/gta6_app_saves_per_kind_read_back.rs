//! A patched plan's edit read back: from a copy written under a temp dir, or from the archive read
//! with the plan's own bytes laid over it.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use or_io::container::rpf::{Gta6Mount, GtaViKeys, PatchPlan};

use super::super::super::{chains, stored_body, write_planned, Planned, Saved};

/// The body of the entry whose body starts at `at` in the archive `src`, and whether its row is a
/// headerless resource's.
fn read_back<R: Read + Seek>(
    src: &mut R,
    at: u64,
    keys: &GtaViKeys,
) -> Result<(Vec<u8>, bool), String> {
    let found = chains(&mut *src, &BTreeSet::from([at]), keys)?
        .remove(&at)
        .ok_or("the copy holds no entry there")?;
    let (last, above) = found.chain.split_last().ok_or("the row has no entry")?;
    let mut mount = Gta6Mount::open_root(&mut *src, keys).map_err(|e| e.to_string())?;
    for index in above {
        let next = {
            let e = (mount.archive.entries.iter().find(|e| e.index == *index))
                .ok_or("a nested archive is gone")?;
            mount
                .open_nested(&mut *src, e, keys)
                .map_err(|e| e.to_string())?
        };
        mount = next;
    }
    let e = (mount.archive.entries.iter().find(|e| e.index == *last)).ok_or("the entry is gone")?;
    let body = mount
        .read_entry_decrypted(&mut *src, e, keys)
        .map_err(|e| e.to_string())?;
    Ok((body, found.headerless))
}

/// `planned` written to a copy under a temp dir, refused unless it is a patched copy whose entry at
/// `at` reads back as `body`.
pub(super) fn copied(
    disk: &Path,
    keys: &GtaViKeys,
    planned: &Planned,
    at: u64,
    body: &[u8],
) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = dir.path().join("copy.rpf");
    let saved = write_planned(disk, &out, keys, planned)?;
    if saved != Saved::Patched {
        return Err(format!("the copy is written as {saved:?}"));
    }
    let mut copy = File::open(&out).map_err(|e| e.to_string())?;
    reads_as_edit(&mut copy, disk, at, keys, body)
}

/// The archive at `disk` read with every byte `planned` writes laid over it, as its copy would hold
/// it, refused unless its entry at `at` reads back as `body`; no copy is written.
pub(crate) fn overlaid(
    disk: &Path,
    keys: &GtaViKeys,
    planned: &Planned,
    at: u64,
    body: &[u8],
) -> Result<(), String> {
    let mut writes = Vec::new();
    for plan in &planned.patches {
        writes.extend(plan_writes(plan)?);
    }
    let file = File::open(disk).map_err(|e| e.to_string())?;
    let mut copy = Overlay {
        file,
        writes,
        pos: 0,
    };
    reads_as_edit(&mut copy, disk, at, keys, body)
}

/// Refused unless the entry at `at` of the archive `copy` reads as `body` and not as the one at
/// `disk` does, a headerless row's loose form read as its stored sections.
fn reads_as_edit<R: Read + Seek>(
    copy: &mut R,
    disk: &Path,
    at: u64,
    keys: &GtaViKeys,
    body: &[u8],
) -> Result<(), String> {
    let (read, headerless) = read_back(copy, at, keys)?;
    let mut source = File::open(disk).map_err(|e| e.to_string())?;
    let (source, _) = read_back(&mut source, at, keys)?;
    if read == source {
        return Err("the copy holds the entry as the source does: the edit is lost".into());
    }
    let want = if headerless {
        stored_body(body)
    } else {
        Some(body.to_vec())
    };
    match want.as_deref() == Some(read.as_slice()) {
        true => Ok(()),
        false => Err("the copy does not read the edit back".into()),
    }
}

/// The bytes `plan` lays at absolute offsets of a copy.
// ponytail: `PatchPlan` keeps its writes private to or-io, so they are read from its derived Debug
// form; an accessor there replaces this.
fn plan_writes(plan: &PatchPlan) -> Result<Vec<(u64, Vec<u8>)>, String> {
    writes_said(&format!("{plan:?}"))
}

/// The writes a plan's Debug form `said` states, its last field `writes: [(at, [bytes]), …]`.
fn writes_said(said: &str) -> Result<Vec<(u64, Vec<u8>)>, String> {
    let bad = || format!("the plan's writes do not read from {said:.80}");
    let (_, rest) = said.split_once("writes: [").ok_or_else(bad)?;
    let rest = rest.strip_suffix("] }").ok_or_else(bad)?;
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for tuple in rest.split("), (") {
        let tuple = tuple.trim_start_matches('(').trim_end_matches(')');
        let (at, bytes) = tuple.split_once(", [").ok_or_else(bad)?;
        let bytes = bytes.strip_suffix(']').ok_or_else(bad)?;
        let at = at.parse::<u64>().map_err(|_| bad())?;
        let bytes = (bytes.split(", ").filter(|b| !b.is_empty()))
            .map(|b| b.parse::<u8>().map_err(|_| bad()))
            .collect::<Result<Vec<u8>, String>>()?;
        out.push((at, bytes));
    }
    Ok(out)
}

/// A file read with `writes` laid over its bytes at their absolute offsets.
struct Overlay {
    file: File,
    writes: Vec<(u64, Vec<u8>)>,
    pos: u64,
}

impl Read for Overlay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(buf)?;
        let (start, end) = (self.pos, self.pos + n as u64);
        for (at, bytes) in &self.writes {
            let (from, to) = ((*at).max(start), (at + bytes.len() as u64).min(end));
            if from < to {
                buf[(from - start) as usize..(to - start) as usize]
                    .copy_from_slice(&bytes[(from - at) as usize..(to - at) as usize]);
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Overlay {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(at) => at,
            SeekFrom::End(_) => self.file.seek(to)?,
            SeekFrom::Current(by) => (self.pos.checked_add_signed(by))
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?,
        };
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::ChunkPatch;

    #[test]
    fn a_file_read_through_an_overlay_holds_each_write_at_its_offset_and_the_file_elsewhere() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("file");
        std::fs::write(&path, (0u8..100).collect::<Vec<u8>>()).expect("the file is written");
        let writes = vec![(10, vec![0xA0, 0xA1, 0xA2]), (98, vec![0xB0, 0xB1])];
        let file = File::open(&path).expect("the file opens");
        let mut overlay = Overlay {
            file,
            writes,
            pos: 0,
        };
        let mut whole = Vec::new();
        overlay.read_to_end(&mut whole).expect("it reads");
        let mut want: Vec<u8> = (0u8..100).collect();
        want[10..13].copy_from_slice(&[0xA0, 0xA1, 0xA2]);
        want[98..].copy_from_slice(&[0xB0, 0xB1]);
        assert_eq!(whole, want);
        // A read that starts inside a write takes only the write's bytes from there.
        let mut two = [0u8; 2];
        overlay.seek(SeekFrom::Start(11)).expect("it seeks");
        overlay.read_exact(&mut two).expect("it reads");
        assert_eq!(two, [0xA1, 0xA2]);
        assert_eq!(overlay.seek(SeekFrom::Current(-3)).expect("it seeks"), 10);
        assert_eq!(overlay.seek(SeekFrom::End(0)).expect("it seeks"), 100);
    }

    #[test]
    fn a_plans_writes_read_from_its_debug_form_are_the_writes_it_holds() {
        // The same fields as `PatchPlan`, so its derived Debug form is the one `writes_said` reads.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct PatchPlan {
            base: u64,
            entry: u32,
            ciphered: bool,
            chunks: Vec<ChunkPatch>,
            writes: Vec<(u64, Vec<u8>)>,
        }
        let chunk = ChunkPatch {
            inflated: 4,
            ceiling: 3,
            new_stored: 2,
            zlib: true,
            changed: true,
            optimal: false,
        };
        let writes = vec![
            (4096, vec![0, 255, 7]),
            (8192, Vec::new()),
            (12288, vec![1]),
        ];
        let plan = PatchPlan {
            base: 4096,
            entry: 3,
            ciphered: true,
            chunks: vec![chunk, chunk],
            writes: writes.clone(),
        };
        assert_eq!(writes_said(&format!("{plan:?}")), Ok(writes));
        let none = PatchPlan {
            writes: Vec::new(),
            ..plan
        };
        assert_eq!(writes_said(&format!("{none:?}")), Ok(Vec::new()));
        assert!(writes_said("PatchPlan { base: 0 }").is_err());
    }
}
