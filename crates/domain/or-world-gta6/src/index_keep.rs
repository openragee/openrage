//! The index kept in the configuration folder: written whole, and read back while the archives
//! it was built from keep their sizes and modification times.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use bytemuck::Pod;
use or_io::gta6::hash::name_hash64;

use super::{Report, WorldIndex};

const MAGIC: &[u8; 8] = b"ORWIDX\0\0";
/// The version of the kept index: a change to it, or to what a build reads, takes another.
const VERSION: u64 = 8;

/// The size and modification time of the archive at `path`, in nanoseconds.
fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}

fn put_rows<T: Pod>(out: &mut impl Write, rows: &[T]) -> io::Result<()> {
    out.write_all(&(rows.len() as u64).to_le_bytes())?;
    out.write_all(bytemuck::cast_slice(rows))
}

fn word(from: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    from.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// The rows the next count of `from` states, read a block at a time so a count the bytes do not
/// bear out fails at their end and reserves nothing.
fn rows<T: Pod>(from: &mut impl Read) -> io::Result<Vec<T>> {
    let count = word(from)?;
    let mut out = Vec::new();
    let mut left = count;
    let mut block = [T::zeroed(); 1024];
    while left > 0 {
        let take = left.min(block.len() as u64) as usize;
        from.read_exact(bytemuck::cast_slice_mut(&mut block[..take]))?;
        out.extend_from_slice(&block[..take]);
        left -= take as u64;
    }
    Ok(out)
}

/// Where the index of the install at `root` is kept under the configuration folder `config`.
pub fn kept_at(config: &Path, root: &Path) -> PathBuf {
    let name = name_hash64(root.to_string_lossy().as_bytes());
    config.join("gta6_world").join(format!("{name:016x}.index"))
}

/// Where what a fresh game is taken to have is written for the index kept at `index`: beside it.
pub fn fresh_beside(index: &Path) -> PathBuf {
    index.with_file_name("fresh_game.json")
}

impl WorldIndex {
    /// What a kept index starts with: its version, then each archive's path, size and time, then the word of what a fresh game is taken to have.
    fn head(archives: &[PathBuf], assumed: u64) -> Vec<u8> {
        let mut head = MAGIC.to_vec();
        head.extend(VERSION.to_le_bytes());
        head.extend((archives.len() as u64).to_le_bytes());
        for path in archives {
            let (len, time) = stat(path).unwrap_or_default();
            let text = path.to_string_lossy();
            for value in [len, time, text.len() as u64] {
                head.extend(value.to_le_bytes());
            }
            head.extend(text.as_bytes());
        }
        head.extend(assumed.to_le_bytes());
        head
    }

    /// Keep the index at `file`, written whole beside it and then moved there.
    pub fn keep(&self, file: &Path) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let beside = file.with_extension("tmp");
        let mut out = BufWriter::new(File::create(&beside)?);
        let head = Self::head(&self.archives, self.assumed);
        out.write_all(&(head.len() as u64).to_le_bytes())?;
        out.write_all(&head)?;
        put_rows(&mut out, &self.maps)?;
        put_rows(&mut out, &self.parents)?;
        put_rows(&mut out, &self.archetypes)?;
        put_rows(&mut out, &self.models)?;
        put_rows(&mut out, &self.dictionaries)?;
        put_rows(&mut out, &self.interiors)?;
        put_rows(&mut out, &self.lights)?;
        put_rows(&mut out, &self.slod)?;
        put_rows(&mut out, &self.switches)?;
        put_rows(&mut out, &self.places)?;
        put_rows(&mut out, &self.outlines)?;
        put_rows(&mut out, std::slice::from_ref(&self.report))?;
        out.flush()?;
        drop(out);
        std::fs::rename(beside, file)
    }

    /// The index kept at `file`, where the archives it was built from are as they were.
    pub fn kept(file: &Path) -> Option<Self> {
        let mut from = BufReader::new(File::open(file).ok()?);
        let len = usize::try_from(word(&mut from).ok()?).ok()?;
        let mut head = vec![0; len.min(1 << 20)];
        from.read_exact(&mut head).ok()?;
        let archives = archives_of(&head)?;
        let (_, assumed) = super::states::assumed(&fresh_beside(file));
        if head != Self::head(&archives, assumed) {
            return None;
        }
        let read = |from: &mut BufReader<File>| -> io::Result<Self> {
            let mut index = WorldIndex {
                archives,
                maps: rows(from)?,
                parents: rows(from)?,
                archetypes: rows(from)?,
                models: rows(from)?,
                dictionaries: rows(from)?,
                interiors: rows(from)?,
                lights: rows(from)?,
                slod: rows(from)?,
                switches: rows(from)?,
                places: rows(from)?,
                outlines: rows(from)?,
                assumed,
                ..WorldIndex::default()
            };
            let report: Vec<Report> = rows(from)?;
            index.report = report.first().copied().unwrap_or_default();
            // A kept run past the names is a file cut short or of another build.
            let parents = index.parents.len() as u64;
            let runs = (index.maps.iter())
                .all(|m| u64::from(m.parents_from) + u64::from(m.parents_len) <= parents);
            let archives = index.archives.len() as u32;
            let placed = index.maps.iter().all(|m| m.archive < archives);
            match runs && placed {
                true => Ok(index),
                false => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a kept index out of step",
                )),
            }
        };
        read(&mut from)
            .inspect_err(|e| tracing::info!("{}: {e}", file.display()))
            .ok()
            .map(Self::finish)
    }
}

/// The next 64-bit word of `rest`, which moves past it.
fn next_word(rest: &mut &[u8]) -> Option<u64> {
    let (value, after) = rest.split_first_chunk::<8>()?;
    *rest = after;
    Some(u64::from_le_bytes(*value))
}

/// The archives a kept index's head names.
fn archives_of(head: &[u8]) -> Option<Vec<PathBuf>> {
    let mut rest = head.strip_prefix(MAGIC.as_slice())?;
    if next_word(&mut rest)? != VERSION {
        return None;
    }
    let count = next_word(&mut rest)?;
    let mut archives = Vec::new();
    for _ in 0..count {
        let (_len, _time) = (next_word(&mut rest)?, next_word(&mut rest)?);
        let text = usize::try_from(next_word(&mut rest)?).ok()?;
        let (path, after) = rest.split_at_checked(text)?;
        rest = after;
        archives.push(PathBuf::from(String::from_utf8_lossy(path).into_owned()));
    }
    Some(archives)
}
