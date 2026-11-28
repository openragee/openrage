//! The one kept file of a world's index: a head that names the title, the world, the version of
//! the title's rules and a stamp for each input, then a section of rows for each archive and one
//! section derived across them. A link follows every load, so no id is ever stored.
//!
//! ⚠ An archive whose stamp stands as kept is not read again, and its section is the kept one
//! byte for byte; the section derived across archives is made again when any input changed. An
//! input with no stamp is read at every open: nothing says it stands. Stamps are taken before the
//! rows are read, so an archive whose rows a title could read only in part keeps no stamp, and a
//! derived section made in part is made again at the next open ([`Section::in_part`]).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::contract::{WorldError, WorldIndexer, WorldInstall, WorldSpec};
use crate::model::Index;

const MAGIC: [u8; 4] = *b"ORWK";
/// The version of the bytes this file writes: a change to them takes another.
pub const FORMAT: u32 = 1;
/// The most bytes a kept file is read to.
const MOST_BYTES: u64 = 1 << 30;

/// A 64-bit FNV-1a hash, written out.
// The number is compared across runs and builds, which the standard library's hasher promises
// neither of.
#[derive(Debug, Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn value(self) -> u64 {
        self.0
    }

    /// The hash of `bytes` alone.
    pub fn of(bytes: &[u8]) -> u64 {
        let mut hash = Self::default();
        hash.feed(bytes);
        hash.value()
    }
}

/// What tells a changed file from the one read before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Bytes.
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: u64,
}

impl Stamp {
    /// The stamp of the file at `path`; `None` where it cannot be asked.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(Self {
            size: meta.len(),
            modified: u64::try_from(modified.as_nanos()).ok()?,
        })
    }
}

/// One thing an index is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// What names it within the world, such as its place under the install.
    pub name: String,
    /// `None` where nothing on disk could be asked: it is read at every open.
    pub stamp: Option<Stamp>,
}

/// What an index is read from: the archives, each with rows of its own, and what else it reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inputs {
    pub archives: Vec<Input>,
    pub others: Vec<Input>,
}

/// Why a section's bytes did not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SectionError {
    #[error("it ends before what it states")]
    CutShort,
    #[error("a text of it is not UTF-8")]
    NotText,
    #[error("bytes follow its last row")]
    BytesLeft,
}

/// The rows of one section, as the bytes a title writes and reads back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    bytes: Vec<u8>,
    /// Not written: it says of rows just read that they are to be read again.
    part: bool,
}

impl Section {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Say these rows are what could be read of something that did not read whole: they are kept, and the next open reads them again, an archive's by its stamp not being kept and the derived section whatever stands.
    pub fn in_part(&mut self) {
        self.part = true;
    }

    pub fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn put_u32(&mut self, value: u32) {
        self.bytes.extend(value.to_le_bytes());
    }

    pub fn put_u64(&mut self, value: u64) {
        self.bytes.extend(value.to_le_bytes());
    }

    pub fn put_f32(&mut self, value: f32) {
        self.bytes.extend(value.to_le_bytes());
    }

    /// A count, then that many bytes.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        self.put_u64(bytes.len() as u64);
        self.bytes.extend(bytes);
    }

    pub fn put_text(&mut self, text: &str) {
        self.put_bytes(text.as_bytes());
    }

    /// What reads the section from its first byte.
    pub fn reader(&self) -> SectionReader<'_> {
        SectionReader(&self.bytes)
    }
}

/// The bytes of a section still to read, each read moving past what it took.
#[derive(Debug, Clone, Copy)]
pub struct SectionReader<'a>(&'a [u8]);

impl<'a> SectionReader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], SectionError> {
        if len > self.0.len() {
            return Err(SectionError::CutShort);
        }
        let (taken, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SectionError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, SectionError> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u32(&mut self) -> Result<u32, SectionError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64, SectionError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32, SectionError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn bytes(&mut self) -> Result<&'a [u8], SectionError> {
        let len = usize::try_from(self.u64()?).map_err(|_| SectionError::CutShort)?;
        self.take(len)
    }

    pub fn text(&mut self) -> Result<&'a str, SectionError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| SectionError::NotText)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// An error where bytes are left: a section read whole ends here.
    pub fn end(&self) -> Result<(), SectionError> {
        match self.0.is_empty() {
            true => Ok(()),
            false => Err(SectionError::BytesLeft),
        }
    }
}

/// Why a kept file was not used.
#[derive(Debug, thiserror::Error)]
pub enum KeptError {
    #[error("the kept index could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("the file is not a kept index")]
    NotAKeptIndex,
    #[error("the kept index is of format {found}, and this build reads format {FORMAT}")]
    Format { found: u32 },
    #[error(
        "the kept index was read under rules {found}, and the title reads under rules {reads}"
    )]
    Rules { found: u32, reads: u32 },
    #[error("the kept index is cut short or altered")]
    Damaged,
}

/// What a kept file starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The title that wrote it, by its id.
    pub title: String,
    /// The world it is of, as [`WorldSpec::identity`] states it.
    pub world: String,
    /// The version of the title's rules its rows were read under.
    pub rules: u32,
    pub inputs: Inputs,
}

/// A kept file, read: its head, a section for each archive of the head, and the one derived across them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub head: Head,
    pub archives: Vec<Section>,
    pub whole: Section,
}

fn put_inputs(out: &mut Section, inputs: &[Input]) {
    out.put_u32(inputs.len() as u32);
    for input in inputs {
        out.put_text(&input.name);
        let stamp = input.stamp.unwrap_or(Stamp {
            size: 0,
            modified: 0,
        });
        out.put_u8(u8::from(input.stamp.is_some()));
        out.put_u64(stamp.size);
        out.put_u64(stamp.modified);
    }
}

fn inputs(from: &mut SectionReader<'_>) -> Result<Vec<Input>, SectionError> {
    let mut out = Vec::new();
    for _ in 0..from.u32()? {
        let name = from.text()?.to_owned();
        let present = from.u8()? != 0;
        let stamp = Stamp {
            size: from.u64()?,
            modified: from.u64()?,
        };
        out.push(Input {
            name,
            stamp: present.then_some(stamp),
        });
    }
    Ok(out)
}

impl Kept {
    /// The file as the bytes [`Self::unseal`] reads: the magic word, the format, the body and the body's hash.
    pub fn seal(&self) -> Vec<u8> {
        let mut body = Section::new();
        body.put_text(&self.head.title);
        body.put_text(&self.head.world);
        body.put_u32(self.head.rules);
        put_inputs(&mut body, &self.head.inputs.archives);
        put_inputs(&mut body, &self.head.inputs.others);
        for section in self.archives.iter().chain([&self.whole]) {
            body.put_bytes(section.bytes());
        }
        let mut out = MAGIC.to_vec();
        out.extend(FORMAT.to_le_bytes());
        out.extend(Fnv::of(body.bytes()).to_le_bytes());
        out.extend(body.bytes());
        out
    }

    /// What [`Self::seal`] wrote, read under the rules `rules`; an error for another format, other rules and bytes cut short or altered.
    pub fn unseal(sealed: &[u8], rules: u32) -> Result<Self, KeptError> {
        let mut front = SectionReader(sealed);
        if front.take(4).map_err(|_| KeptError::NotAKeptIndex)? != MAGIC {
            return Err(KeptError::NotAKeptIndex);
        }
        let found = front.u32().map_err(|_| KeptError::Damaged)?;
        if found != FORMAT {
            return Err(KeptError::Format { found });
        }
        let hash = front.u64().map_err(|_| KeptError::Damaged)?;
        if hash != Fnv::of(front.0) {
            return Err(KeptError::Damaged);
        }
        let mut body = front;
        let read =
            |body: &mut SectionReader<'_>| -> Result<(Head, Vec<Section>, Section), SectionError> {
                let title = body.text()?.to_owned();
                let world = body.text()?.to_owned();
                let rules = body.u32()?;
                let archives = inputs(body)?;
                let others = inputs(body)?;
                let mut sections = Vec::with_capacity(archives.len());
                for _ in 0..=archives.len() {
                    sections.push(Section {
                        bytes: body.bytes()?.to_vec(),
                        part: false,
                    });
                }
                body.end()?;
                let whole = sections.pop().unwrap_or_default();
                let head = Head {
                    title,
                    world,
                    rules,
                    inputs: Inputs { archives, others },
                };
                Ok((head, sections, whole))
            };
        let (head, archives, whole) = read(&mut body).map_err(|_| KeptError::Damaged)?;
        if head.rules != rules {
            return Err(KeptError::Rules {
                found: head.rules,
                reads: rules,
            });
        }
        Ok(Self {
            head,
            archives,
            whole,
        })
    }

    /// The file kept at `file`, read under the rules `rules`; `None` where there is no such file.
    pub fn open(file: &Path, rules: u32) -> Result<Option<Self>, KeptError> {
        match std::fs::metadata(file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(KeptError::Io(e)),
            Ok(meta) if meta.len() > MOST_BYTES => return Err(KeptError::Damaged),
            Ok(_) => {}
        }
        match std::fs::read(file) {
            Ok(sealed) => Self::unseal(&sealed, rules).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeptError::Io(e)),
        }
    }

    /// Write the file to `file`, in place of any kept there before.
    pub fn keep(&self, file: &Path) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // Written beside the file and moved over it, so a write that stops leaves the old file.
        // ⚠ Under a name of this writer's own: two writers of one name write through each other,
        // and the one that moves the file second moves a file that is gone.
        let thread = format!("{:?}", std::thread::current().id());
        let thread: String = thread.chars().filter(char::is_ascii_digit).collect();
        let mut beside = file.as_os_str().to_owned();
        beside.push(format!(".writing-{}-{thread}", std::process::id()));
        let beside = Path::new(&beside);
        let kept = std::fs::write(beside, self.seal()).and_then(|()| std::fs::rename(beside, file));
        if kept.is_err() {
            // What was written beside a file that could not be replaced is a kept file of nothing.
            let _ = std::fs::remove_file(beside);
        }
        kept
    }
}

/// Whether a kept file served an open.
#[derive(Debug)]
pub enum Verdict {
    /// There was none.
    Absent,
    /// It was of this world as it stands: no archive was read and nothing derived again.
    Current,
    /// It was of this world with other inputs: each changed archive was read again.
    Changed,
    /// It was of another title or another world, so nothing of it was taken.
    Foreign,
    /// It could not be used, and why; every archive was read.
    Refused(KeptError),
}

/// What an open took from a kept file and what it read.
#[derive(Debug)]
pub struct Report {
    pub verdict: Verdict,
    /// The archives whose rows were read, by their place among the inputs.
    pub read: Vec<usize>,
    /// Whether the section derived across archives was made again.
    pub derived: bool,
    /// Why the file was not written back, where a write was tried and failed.
    pub not_kept: Option<io::Error>,
}

/// Where the index of `world` of the title `title` is kept under the configuration folder `config`.
pub fn file_for(config: &Path, title: &str, world: &WorldSpec) -> PathBuf {
    let name = Fnv::of(world.identity().as_bytes());
    (config.join("worlds").join(title)).join(format!("{name:016x}.index"))
}

/// The index of `install`, which is `world` opened by the title `title`: from `file` where its inputs stand as kept, and `file` replaced where they do not.
pub fn refresh(
    title: &str,
    world: &WorldSpec,
    install: &dyn WorldInstall,
    file: &Path,
) -> Result<(Index, Report), WorldError> {
    refresh_with(title, world, install.inputs(), install.indexer(), file)
}

/// [`refresh`] for a world that is no [`WorldInstall`] yet: its inputs as they stand now, and what reads their rows.
pub fn refresh_with(
    title: &str,
    world: &WorldSpec,
    mut now: Inputs,
    indexer: &dyn WorldIndexer,
    file: &Path,
) -> Result<(Index, Report), WorldError> {
    let (rules, identity) = (indexer.rules(), world.identity());
    let (kept, mut verdict) = match Kept::open(file, rules) {
        Ok(None) => (None, Verdict::Absent),
        Ok(Some(kept)) if kept.head.title != title || kept.head.world != identity => {
            (None, Verdict::Foreign)
        }
        Ok(Some(kept)) => (Some(kept), Verdict::Current),
        Err(e) => {
            tracing::warn!(file = %file.display(), error = %e, "the kept index is refused and read again");
            (None, Verdict::Refused(e))
        }
    };
    // ⚠ A queue: a world can list one archive twice, and each listing takes a section of its own.
    let mut carried: HashMap<String, VecDeque<(Option<Stamp>, Section)>> = HashMap::new();
    let mut whole = None;
    if let Some(kept) = kept {
        // What else the index reads stands only where every such input stands, stamped.
        let others_stand = kept.head.inputs.others == now.others
            && now.others.iter().all(|input| input.stamp.is_some());
        // ⚠ What is derived across archives may rest on their order: the same archives listed
        // otherwise, or fewer of them, derive it again though none is read.
        let listed = |inputs: &Inputs| -> Vec<String> {
            inputs
                .archives
                .iter()
                .map(|input| input.name.clone())
                .collect()
        };
        let same_archives = listed(&kept.head.inputs) == listed(&now);
        whole = (others_stand && same_archives).then_some(kept.whole);
        for (input, section) in kept.head.inputs.archives.into_iter().zip(kept.archives) {
            let at = carried.entry(input.name).or_default();
            at.push_back((input.stamp, section));
        }
    }
    let (mut read, mut in_part) = (Vec::new(), Vec::new());
    let mut archives = Vec::with_capacity(now.archives.len());
    for (at, input) in now.archives.iter().enumerate() {
        let same = (carried.get_mut(&input.name).and_then(VecDeque::pop_front))
            .filter(|(stamp, _)| input.stamp.is_some() && *stamp == input.stamp);
        archives.push(match same {
            Some((_, section)) => section,
            None => {
                let mut section = Section::new();
                indexer.rows(at, &mut section)?;
                read.push(at);
                if section.part {
                    in_part.push(at);
                }
                section
            }
        });
    }
    let (whole, derived) = match whole.filter(|_| read.is_empty()) {
        Some(whole) => (whole, false),
        None => {
            let mut whole = Section::new();
            indexer.whole(&archives, &mut whole)?;
            (whole, true)
        }
    };
    if matches!(verdict, Verdict::Current) && derived {
        verdict = Verdict::Changed;
    }
    let index = indexer.link(&archives, &whole)?;
    let mut report = Report {
        verdict,
        read,
        derived,
        not_kept: None,
    };
    // ⚠ The stamps were taken before the rows were read: one of an archive read in part would
    // say rows stand that were never read.
    for at in in_part {
        now.archives[at].stamp = None;
    }
    // ⚠ Nothing of the head says of the derived section alone that it does not stand: one made in
    // part is kept beside an input of no name and no stamp, which no open finds as it was.
    if whole.part {
        now.others.push(Input {
            name: String::new(),
            stamp: None,
        });
    }
    if !matches!(report.verdict, Verdict::Current) {
        let kept = Kept {
            head: Head {
                title: title.to_owned(),
                world: identity,
                rules,
                inputs: now,
            },
            archives,
            whole,
        };
        report.not_kept = kept.keep(file).err();
    }
    Ok((index, report))
}

#[cfg(test)]
#[path = "kept_tests.rs"]
mod tests;
