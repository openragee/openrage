//! The inflated programs of an effect container kept in one file, so a later run reads any range
//! of them without the archive: a count, the place of each program, then their bytes.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"ORRDNAC1";

fn malformed(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

/// Writes `programs`, in order, to `path`; a program that did not read is written as no bytes.
/// The file is written beside its place and moved there, so a reader never sees half of one.
pub fn write(path: &Path, programs: &[Option<Vec<u8>>]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    let mut out = io::BufWriter::new(File::create(&partial)?);
    out.write_all(MAGIC)?;
    out.write_all(&(programs.len() as u64).to_le_bytes())?;
    let mut at = 0u64;
    for program in programs {
        out.write_all(&at.to_le_bytes())?;
        at += program.as_ref().map_or(0, |bytes| bytes.len() as u64);
    }
    out.write_all(&at.to_le_bytes())?;
    for program in programs.iter().flatten() {
        out.write_all(program)?;
    }
    out.into_inner()?.sync_all()?;
    std::fs::rename(&partial, path)
}

/// A corpus file open for reading.
#[derive(Debug)]
pub struct Corpus {
    file: File,
    /// Where each program starts after the table, and where the last ends.
    places: Vec<u64>,
    /// Where the programs' bytes start.
    base: u64,
}

impl Corpus {
    pub fn open(path: &Path) -> io::Result<Corpus> {
        let mut file = File::open(path)?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head)?;
        if &head[..8] != MAGIC {
            return Err(malformed("not a corpus of native programs"));
        }
        let count = u64::from_le_bytes(head[8..].try_into().unwrap_or_default());
        let table = (count.checked_add(1))
            .and_then(|places| places.checked_mul(8))
            .filter(|table| *table <= file.metadata().map_or(0, |m| m.len()))
            .ok_or_else(|| malformed("the corpus states more programs than it holds"))?;
        let mut bytes = vec![0u8; table as usize];
        file.read_exact(&mut bytes)?;
        let places: Vec<u64> = (bytes.as_chunks::<8>().0.iter())
            .map(|place| u64::from_le_bytes(*place))
            .collect();
        if places.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(malformed("the corpus's programs are out of order"));
        }
        Ok(Corpus {
            file,
            places,
            base: 16 + table,
        })
    }

    /// How many programs the container held, read or not.
    pub fn len(&self) -> usize {
        self.places.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Program `index`'s inflated bytes; `None` for one that did not read or is past the end.
    pub fn program(&mut self, index: usize) -> io::Result<Option<Vec<u8>>> {
        let (Some(&start), Some(&end)) = (self.places.get(index), self.places.get(index + 1))
        else {
            return Ok(None);
        };
        if start == end {
            return Ok(None);
        }
        let mut bytes = vec![0u8; (end - start) as usize];
        self.file.seek(SeekFrom::Start(self.base + start))?;
        self.file.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_corpus_reads_back_program_by_program() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept").join("corpus.bin");
        let programs = vec![Some(vec![1, 2, 3]), None, Some(vec![9; 70_000]), None];
        write(&path, &programs).unwrap();
        let mut corpus = Corpus::open(&path).unwrap();
        assert_eq!(corpus.len(), 4);
        // Out of order, as a worker handed the middle of the range reads it.
        for index in [2, 0, 3, 1] {
            assert_eq!(corpus.program(index).unwrap(), programs[index]);
        }
        assert_eq!(corpus.program(4).unwrap(), None);
    }

    #[test]
    fn another_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.bin");
        std::fs::write(&path, b"ORRDNAC1\xff\xff\xff\xff\xff\xff\xff\xff").unwrap();
        assert!(Corpus::open(&path).is_err());
        std::fs::write(&path, b"something else, long enough").unwrap();
        assert!(Corpus::open(&path).is_err());
        assert!(Corpus::open(&dir.path().join("absent.bin")).is_err());
    }
}
