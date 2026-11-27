//! A world index on disk: a magic word, a version, the rules it was read under and its length,
//! then the archives' rows as a zlib stream whose checksum says the bytes are whole.

use std::io::{self, Write};
use std::path::Path;

use flate2::write::ZlibEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use glam::Vec3;
use or_world::ArchetypeKind;

use super::{
    ArchetypeRow, Archive, ArchiveRows, BoundRow, Counted, GroupGate, KeptState, ManifestRow,
    MapFacts, MapRow, Report, Span, Stamp, TypeFileRow, WorldIndex, WorldSource,
};
use crate::tables::NamedBounds;

const MAGIC: [u8; 4] = *b"ORWI";
/// The version of the bytes this file writes: a change to them takes another.
pub const VERSION: u32 = 3;
/// The version of what a build reads of each file: a change to what a row holds of the same file takes another.
pub const RULES: u32 = 3;
/// The most a kept index inflates to, in bytes.
const MOST_BYTES: usize = 1 << 28;

/// Why a kept index was not used.
#[derive(Debug, thiserror::Error)]
pub enum KeptError {
    #[error("the kept world index could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("the file is not a kept world index")]
    NotAnIndex,
    #[error("the kept world index is of version {found}, and this build reads version {VERSION}")]
    Version { found: u32 },
    #[error("the kept world index was read under rules {found}, and this build reads under rules {RULES}")]
    Rules { found: u32 },
    #[error("the kept world index is cut short or altered")]
    Damaged,
    #[error("the kept world index states {0}")]
    Malformed(&'static str),
}

/// What a kept index holds: the install it is of, the key of its mount and each archive's rows.
#[derive(Debug, PartialEq)]
pub struct Kept {
    pub(super) identity: String,
    pub(super) key: u64,
    pub(super) archives: Vec<ArchiveRows>,
}

/// The key of a mount: what its identity and each of its root archives' place, size and
/// modification time hash to, in mount order.
///
/// ⚠ FNV-1a, written out, because the number is compared across runs and builds and the standard
/// library's hasher promises neither.
pub(super) fn key_of<'a>(identity: &str, archives: impl Iterator<Item = &'a Archive>) -> u64 {
    let mut key: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            key = (key ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A terminator, so two fields do not hash as one field with their bytes joined.
        key = (key ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(&RULES.to_le_bytes());
    feed(identity.as_bytes());
    for archive in archives {
        feed(archive.device.as_bytes());
        feed(archive.container.as_bytes());
        let (present, stamp) = match archive.stamp {
            Some(stamp) => (1u8, stamp),
            None => (
                0,
                Stamp {
                    size: 0,
                    modified: 0,
                },
            ),
        };
        feed(&[present]);
        feed(&stamp.size.to_le_bytes());
        feed(&stamp.modified.to_le_bytes());
    }
    key
}

impl Kept {
    /// The key of the mount the index was read from.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// What [`WorldIndex::seal`] wrote; an error for another version, other rules and bytes cut short or altered.
    pub fn unseal(sealed: &[u8]) -> Result<Self, KeptError> {
        let mut head = Reader(sealed);
        if head.take(4).map_err(|_| KeptError::NotAnIndex)? != MAGIC {
            return Err(KeptError::NotAnIndex);
        }
        let version = head.u32()?;
        if version != VERSION {
            return Err(KeptError::Version { found: version });
        }
        let rules = head.u32()?;
        if rules != RULES {
            return Err(KeptError::Rules { found: rules });
        }
        let len = usize::try_from(head.u64()?).unwrap_or(usize::MAX);
        if len > MOST_BYTES {
            return Err(KeptError::Malformed("more bytes than an index holds"));
        }
        // ⚠ A stream cut short inflates to fewer bytes without an error: only its end, which holds
        // the checksum, and the stated length say the bytes are whole.
        let mut plain = Vec::with_capacity(len.saturating_add(1));
        let mut stream = Decompress::new(true);
        let ended = stream.decompress_vec(head.0, &mut plain, FlushDecompress::Finish);
        let whole = plain.len() == len && stream.total_in() == head.0.len() as u64;
        if !matches!(ended, Ok(Status::StreamEnd)) || !whole {
            return Err(KeptError::Damaged);
        }

        let mut body = Reader(&plain);
        let key = body.u64()?;
        let identity = body.text()?;
        let mut archives = Vec::new();
        for _ in 0..body.u32()? {
            archives.push(body.archive()?);
        }
        if !body.0.is_empty() {
            return Err(KeptError::Malformed("bytes after its last archive"));
        }
        if key != key_of(&identity, archives.iter().map(|rows| &rows.archive)) {
            return Err(KeptError::Malformed("a key that is not its archives'"));
        }
        Ok(Self {
            identity,
            key,
            archives,
        })
    }

    /// The index of the archives `parts` hold, in their order, with no archive read; `patches` names the folder that patches an archive, by the archive's place.
    // ⚠ Each part's rows stand as kept, whatever its archive's stamp: the caller says they stand.
    pub fn joined(parts: Vec<Self>, patches: &[(usize, String)]) -> WorldIndex {
        let identity = parts
            .first()
            .map_or_else(String::new, |part| part.identity.clone());
        let archives = parts.into_iter().flat_map(|part| part.archives).collect();
        WorldIndex::of_rows(identity, archives, patches)
    }

    /// The index kept at `file`; `None` where there is no such file.
    pub fn open(file: &Path) -> Result<Option<Self>, KeptError> {
        match std::fs::read(file) {
            Ok(sealed) => Self::unseal(&sealed).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(KeptError::Io(e)),
        }
    }
}

impl WorldIndex {
    /// The key of the mount this index was read from.
    pub fn key(&self) -> u64 {
        key_of(&self.identity, self.archives())
    }

    /// The index as the bytes [`Kept::unseal`] reads.
    pub fn seal(&self) -> io::Result<Vec<u8>> {
        let mut plain = Vec::new();
        put_u64(&mut plain, self.key());
        put_text(&mut plain, &self.identity);
        put_len(&mut plain, self.archives.len());
        for rows in &self.archives {
            put_archive(&mut plain, rows);
        }
        let mut out = MAGIC.to_vec();
        put_u32(&mut out, VERSION);
        put_u32(&mut out, RULES);
        put_u64(&mut out, plain.len() as u64);
        let mut stream = ZlibEncoder::new(out, Compression::fast());
        stream.write_all(&plain)?;
        stream.finish()
    }

    /// Write the index to `file`, in place of any kept there before.
    pub fn keep(&self, file: &Path) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        // Written beside the file and moved over it, so a write that stops leaves the old index.
        let mut beside = file.as_os_str().to_owned();
        beside.push(".writing");
        let beside = Path::new(&beside);
        let kept =
            std::fs::write(beside, self.seal()?).and_then(|()| std::fs::rename(beside, file));
        if kept.is_err() {
            // What was written beside a file that could not be replaced is no index of anything.
            let _ = std::fs::remove_file(beside);
        }
        kept
    }
}

/// The index of `source`, from `file` where the mount stands as it did, and `file` replaced where it does not.
pub fn refresh(source: &dyn WorldSource, file: &Path) -> (WorldIndex, Report) {
    let (kept, refused) = match Kept::open(file) {
        Ok(kept) => (kept, None),
        Err(e) => {
            tracing::warn!(file = %file.display(), error = %e, "the kept world index is refused and read again");
            (None, Some(e))
        }
    };
    let (index, mut report) = WorldIndex::build(source, kept);
    if let Some(refused) = refused {
        report.kept = KeptState::Refused(refused);
    }
    if !matches!(report.kept, KeptState::Current) {
        report.not_kept = index.keep(file).err();
    }
    (index, report)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend(value.to_le_bytes());
}

fn put_len(out: &mut Vec<u8>, len: usize) {
    // A list an index holds is counted in a u32: a retail install's longest is its archetypes.
    put_u32(out, len as u32);
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    put_len(out, text.len());
    out.extend(text.as_bytes());
}

fn put_span(out: &mut Vec<u8>, span: Option<Span>) {
    out.push(u8::from(span.is_some()));
    if let Some((min, max)) = span {
        for corner in [min, max] {
            for value in corner.to_array() {
                out.extend(value.to_le_bytes());
            }
        }
    }
}

/// Each name with how often it comes.
fn put_counted(out: &mut Vec<u8>, counted: &[(u32, u32)]) {
    put_len(out, counted.len());
    for (name, count) in counted {
        put_u32(out, *name);
        put_u32(out, *count);
    }
}

fn put_named(out: &mut Vec<u8>, named: &[NamedBounds]) {
    put_len(out, named.len());
    for (name, bounds) in named {
        put_u32(out, *name);
        put_len(out, bounds.len());
        for bound in bounds {
            put_u32(out, *bound);
        }
    }
}

fn put_archive(out: &mut Vec<u8>, rows: &ArchiveRows) {
    put_text(out, &rows.archive.device);
    put_text(out, &rows.archive.container);
    out.push(u8::from(rows.archive.stamp.is_some()));
    if let Some(stamp) = rows.archive.stamp {
        put_u64(out, stamp.size);
        put_u64(out, stamp.modified);
    }

    put_len(out, rows.maps.len());
    for map in &rows.maps {
        put_text(out, &map.logical);
        out.push(u8::from(map.facts.is_some()));
        if let Some(facts) = &map.facts {
            put_u32(out, facts.parent);
            put_u32(out, facts.flags);
            put_u32(out, facts.content_flags);
            put_span(out, facts.entities);
            put_span(out, facts.streaming);
            put_len(out, facts.interiors.len());
            for interior in &facts.interiors {
                put_u32(out, *interior);
            }
            put_counted(out, &facts.placed);
        }
    }

    put_len(out, rows.type_files.len());
    for file in &rows.type_files {
        put_text(out, &file.logical);
        out.push(u8::from(file.archetypes.is_some()));
        if let Some(archetypes) = &file.archetypes {
            put_len(out, archetypes.len());
            for archetype in archetypes {
                put_u32(out, archetype.name);
                out.push(match archetype.kind {
                    ArchetypeKind::Base => 0,
                    ArchetypeKind::Time => 1,
                    ArchetypeKind::Mlo => 2,
                });
                put_span(out, archetype.span);
                for value in archetype.sphere.0.to_array() {
                    out.extend(value.to_le_bytes());
                }
                out.extend(archetype.sphere.1.to_le_bytes());
                out.extend(archetype.lod_dist.to_le_bytes());
                put_u32(out, archetype.asset_name);
                put_u32(out, archetype.drawable_dictionary);
                put_u32(out, archetype.texture_dictionary);
            }
        }
        put_counted(out, &file.inside);
    }

    put_len(out, rows.manifests.len());
    for manifest in &rows.manifests {
        put_text(out, &manifest.logical);
        out.push(u8::from(manifest.read));
        put_named(out, &manifest.groups);
        put_named(out, &manifest.interiors);
        put_len(out, manifest.gates.len());
        for gate in &manifest.gates {
            put_u32(out, gate.name);
            put_u32(out, gate.flags);
            put_u32(out, gate.hours);
            put_len(out, gate.weather_types.len());
            for weather in &gate.weather_types {
                put_u32(out, *weather);
            }
        }
    }

    put_len(out, rows.bounds.len());
    for bound in &rows.bounds {
        put_text(out, &bound.logical);
        put_span(out, bound.span);
    }

    put_len(out, rows.files.len());
    for file in &rows.files {
        put_text(out, file);
    }
}

/// Bytes still to read, each read moving past what it took.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], KeptError> {
        if len > self.0.len() {
            return Err(KeptError::Damaged);
        }
        let (taken, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], KeptError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, KeptError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, KeptError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, KeptError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    /// Whether what follows is there: a byte that is 0 or 1.
    fn flag(&mut self) -> Result<bool, KeptError> {
        match self.array::<1>()?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(KeptError::Malformed("a flag that is neither 0 nor 1")),
        }
    }

    fn text(&mut self) -> Result<String, KeptError> {
        let len = self.u32()? as usize;
        let text = std::str::from_utf8(self.take(len)?);
        text.map(str::to_string)
            .map_err(|_| KeptError::Malformed("a name that is not text"))
    }

    fn span(&mut self) -> Result<Option<Span>, KeptError> {
        if !self.flag()? {
            return Ok(None);
        }
        let mut corner =
            || -> Result<Vec3, KeptError> { Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?)) };
        Ok(Some((corner()?, corner()?)))
    }

    /// A list whose length is stated, read an item at a time so a length no file could hold runs
    /// out of bytes before it is allocated for.
    fn list<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, KeptError>,
    ) -> Result<Vec<T>, KeptError> {
        let mut out = Vec::new();
        for _ in 0..self.u32()? {
            out.push(item(self)?);
        }
        Ok(out)
    }

    fn counted(&mut self) -> Result<Counted, KeptError> {
        self.list(|r| Ok((r.u32()?, r.u32()?)))
    }

    fn named(&mut self) -> Result<Vec<NamedBounds>, KeptError> {
        self.list(|r| Ok((r.u32()?, r.list(Self::u32)?)))
    }

    fn archive(&mut self) -> Result<ArchiveRows, KeptError> {
        let device = self.text()?;
        let container = self.text()?;
        let stamp = match self.flag()? {
            true => Some(Stamp {
                size: self.u64()?,
                modified: self.u64()?,
            }),
            false => None,
        };
        let maps = self.list(|r| {
            let logical = r.text()?;
            let facts = match r.flag()? {
                true => Some(MapFacts {
                    parent: r.u32()?,
                    flags: r.u32()?,
                    content_flags: r.u32()?,
                    entities: r.span()?,
                    streaming: r.span()?,
                    interiors: r.list(Self::u32)?,
                    placed: r.counted()?,
                }),
                false => None,
            };
            Ok(MapRow { logical, facts })
        })?;
        let type_files = self.list(|r| {
            let logical = r.text()?;
            let archetypes = match r.flag()? {
                true => Some(r.list(|r| {
                    Ok(ArchetypeRow {
                        name: r.u32()?,
                        kind: match r.array::<1>()?[0] {
                            0 => ArchetypeKind::Base,
                            1 => ArchetypeKind::Time,
                            2 => ArchetypeKind::Mlo,
                            _ => return Err(KeptError::Malformed("an archetype of no kind")),
                        },
                        span: r.span()?,
                        sphere: (Vec3::new(r.f32()?, r.f32()?, r.f32()?), r.f32()?),
                        lod_dist: r.f32()?,
                        asset_name: r.u32()?,
                        drawable_dictionary: r.u32()?,
                        texture_dictionary: r.u32()?,
                    })
                })?),
                false => None,
            };
            Ok(TypeFileRow {
                logical,
                archetypes,
                inside: r.counted()?,
            })
        })?;
        let manifests = self.list(|r| {
            Ok(ManifestRow {
                logical: r.text()?,
                read: r.flag()?,
                groups: r.named()?,
                interiors: r.named()?,
                gates: r.list(|r| {
                    Ok(GroupGate {
                        name: r.u32()?,
                        flags: r.u32()?,
                        hours: r.u32()?,
                        weather_types: r.list(Self::u32)?,
                    })
                })?,
            })
        })?;
        let bounds = self.list(|r| {
            Ok(BoundRow {
                logical: r.text()?,
                span: r.span()?,
            })
        })?;
        let files = self.list(Self::text)?;
        Ok(ArchiveRows {
            archive: Archive {
                device,
                container,
                stamp,
            },
            maps,
            type_files,
            manifests,
            bounds,
            files,
        })
    }
}
