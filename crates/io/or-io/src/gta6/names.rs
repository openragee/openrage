//! A vocabulary of names harvested from GTA VI bodies. An archive's table is encrypted, and its
//! bodies spell their own and each other's names (`name.#dr`, `name.#ft`, ...) beside plain ones;
//! each is hashed both ways, so a 32-bit or 64-bit word read elsewhere is shown by its name.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::hash::{name_hash32, name_hash64};
use crate::container::rpf::{
    Rpf9BodyKind, Rpf9Content, Rpf9Role, Rpf9Scan, Rpf9ScanCarry, Rpf9ScanFind, Rpf9ZlibRun,
};

/// The bytes of a body a harvest reads by default, from its start: what each worker of a harvest
/// holds of one body at most.
pub const HARVEST_PREFIX: usize = 16 << 20;
/// The bytes of an archive one scan of a harvest covers, and the step its progress moves by.
const HARVEST_RANGE: u64 = 64 << 20;
/// The shortest range a harvest scans alone, where the archive is too short to give each worker more.
const SMALL_RANGE: u64 = 4 << 20;
/// The longest name kept, without its `.#xx` ending.
pub const MAX_NAME: usize = 96;
/// The shortest plain name kept: a shorter one is as likely chance as a name.
const MIN_PLAIN: usize = 3;
/// The bytes a body is judged text by: one with no NUL among them is.
const TEXT_PROBE: usize = 4096;
/// The longest line of a kept file read as one: a kind or an offset, a space and a name.
const MAX_LINE: u64 = 160;
const VOCABULARY_HEAD: &str = "openrage gta6 names 1";
const LABELS_HEAD: &str = "openrage gta6 labels 1";
const TEXTURES_HEAD: &str = "openrage gta6 textures 1";

/// A name a body spells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Harvested<'a> {
    /// The name without its `.#xx` ending, as written.
    pub name: &'a str,
    /// The two characters after `.#`, lower-cased; `None` for a plain name.
    pub kind: Option<[u8; 2]>,
}

fn printable(b: u8) -> bool {
    (0x20..0x7F).contains(&b)
}

fn name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'@')
}

fn kind_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// A name's stem and the two characters after `.#`, lower-cased, where it ends in them.
pub fn split_kind(name: &str) -> (&str, Option<[u8; 2]>) {
    match name.as_bytes() {
        [stem @ .., b'.', b'#', a, b] if kind_byte(*a) && kind_byte(*b) => (
            &name[..stem.len()],
            Some([a.to_ascii_lowercase(), b.to_ascii_lowercase()]),
        ),
        _ => (name, None),
    }
}

/// Each name the first `prefix` bytes of `body` spell, handed to `each` in file order.
// A `name.#xx` is taken wherever it is printed. A plain name is taken from text, and from a
// binary body only out of a NUL-ended string that is all names and path separators: chance
// bytes spell short printable runs, and a vocabulary of them names nothing.
pub fn harvest<'a>(body: &'a [u8], prefix: usize, each: &mut impl FnMut(Harvested<'a>)) {
    let body = &body[..body.len().min(prefix)];
    let text = !body[..body.len().min(TEXT_PROBE)].contains(&0);
    let mut at = 0;
    while at < body.len() {
        if !printable(body[at]) {
            at += 1;
            continue;
        }
        let end = (body[at..].iter())
            .position(|b| !printable(*b))
            .map_or(body.len(), |n| at + n);
        let run = &body[at..end];
        let path_shaped = || (run.iter()).all(|b| name_byte(*b) || b"/\\:.#".contains(b));
        let plain = text || (body.get(end) == Some(&0) && path_shaped());
        tokens(run, plain, each);
        at = end;
    }
}

/// The names of one printable run; the plain ones only where `plain`.
fn tokens<'a>(run: &'a [u8], plain: bool, each: &mut impl FnMut(Harvested<'a>)) {
    let mut at = 0;
    while at < run.len() {
        if !name_byte(run[at]) {
            at += 1;
            continue;
        }
        let end = (run[at..].iter())
            .position(|b| !name_byte(*b))
            .map_or(run.len(), |n| at + n);
        let token = &run[at..end];
        let kind = match &run[end..] {
            [b'.', b'#', a, b, rest @ ..]
                if kind_byte(*a)
                    && kind_byte(*b)
                    && rest.first().is_none_or(|c| !name_byte(*c)) =>
            {
                Some([a.to_ascii_lowercase(), b.to_ascii_lowercase()])
            }
            _ => None,
        };
        let kept = match kind {
            Some(_) => true,
            None => plain && token.len() >= MIN_PLAIN && token.iter().any(u8::is_ascii_alphabetic),
        };
        if let (true, true, Ok(name)) = (kept, token.len() <= MAX_NAME, std::str::from_utf8(token))
        {
            each(Harvested { name, kind });
        }
        at = end + if kind.is_some() { 4 } else { 0 };
    }
}

/// Whether a vocabulary holds `name`: 1 to [`MAX_NAME`] printable bytes with no space.
fn holdable(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.bytes().all(|b| b.is_ascii_graphic())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Name {
    /// Lower-cased, as both hashes fold it.
    text: Box<str>,
    kind: Option<[u8; 2]>,
}

/// How many names a vocabulary holds, and of what kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub names: usize,
    /// The names a body spelled with a `.#xx` ending, by that ending.
    pub kinds: BTreeMap<[u8; 2], usize>,
    pub plain: usize,
    /// The distinct 32-bit hashes of the names.
    pub hashes32: usize,
    /// The 32-bit hashes two names or more share, which name nothing alone.
    pub shared32: usize,
}

/// Names by their 64-bit and 32-bit hashes.
// ponytail: every name is held in memory, a few hundred megabytes for some millions of names; an
// index on disk if a full install's plain names outgrow that.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vocabulary {
    names: HashMap<u64, Name>,
    /// The first name of each 32-bit hash, by its 64-bit word.
    by32: HashMap<u32, u64>,
    shared32: HashSet<u32>,
}

impl Vocabulary {
    /// Holds `name`, lower-cased, where it is 1 to [`MAX_NAME`] printable bytes with no space;
    /// whether it is new. A name held as plain takes the `kind` a later body spells it with.
    pub fn add(&mut self, name: &str, kind: Option<[u8; 2]>) -> bool {
        if !holdable(name) {
            return false;
        }
        match self.names.entry(name_hash64(name.as_bytes())) {
            Entry::Occupied(mut held) => {
                let held = held.get_mut();
                held.kind = held.kind.or(kind);
                false
            }
            Entry::Vacant(slot) => {
                let text: Box<str> = name.to_ascii_lowercase().into();
                let (word, hash) = (*slot.key(), name_hash32(text.as_bytes()));
                slot.insert(Name { text, kind });
                match self.by32.entry(hash) {
                    Entry::Vacant(first) => {
                        first.insert(word);
                    }
                    Entry::Occupied(_) => {
                        self.shared32.insert(hash);
                    }
                }
                true
            }
        }
    }

    /// Holds each name the first `prefix` bytes of `body` spell; how many were new.
    pub fn harvest(&mut self, body: &[u8], prefix: usize) -> usize {
        let mut new = 0;
        harvest(body, prefix, &mut |found| {
            new += usize::from(self.add(found.name, found.kind));
        });
        new
    }

    /// Holds each line of `list`, a file of names the user supplies, as a plain name; how many
    /// were new. A line that starts with `//` is a comment.
    pub fn add_lines(&mut self, list: &str) -> usize {
        let names = list.lines().map(str::trim);
        let names = names.filter(|line| !line.starts_with("//"));
        names.filter(|name| self.add(name, None)).count()
    }

    /// Holds every name of `other`, as another archive's harvest.
    pub fn merge(&mut self, other: &Self) {
        for name in other.names.values() {
            self.add(&name.text, name.kind);
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The name whose 64-bit hash is `word`.
    pub fn name64(&self, word: u64) -> Option<&str> {
        self.names.get(&word).map(|n| &*n.text)
    }

    /// The name whose 32-bit hash is `hash`; `None` where two held names share it.
    pub fn name32(&self, hash: u32) -> Option<&str> {
        if self.shared32.contains(&hash) {
            return None;
        }
        self.name64(*self.by32.get(&hash)?)
    }

    /// The name a 32-bit `hash` and the 64-bit `word` stated beside it both name; a zero `word`
    /// states none, and the 32-bit hash then names alone.
    pub fn confirmed(&self, hash: u32, word: u64) -> Option<&str> {
        match word {
            0 => self.name32(hash),
            _ => (self.name64(word)).filter(|name| name_hash32(name.as_bytes()) == hash),
        }
    }

    /// Every name held, in no order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.values().map(|n| &*n.text)
    }

    /// The names a body spelled with a `.#xx` ending, without it.
    pub fn resource_names(&self) -> impl Iterator<Item = &str> {
        (self.names.values())
            .filter(|n| n.kind.is_some())
            .map(|n| &*n.text)
    }

    /// The names no body spelled with an ending.
    pub fn plain_names(&self) -> impl Iterator<Item = &str> {
        (self.names.values())
            .filter(|n| n.kind.is_none())
            .map(|n| &*n.text)
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            names: self.names.len(),
            hashes32: self.by32.len(),
            shared32: self.shared32.len(),
            ..Counts::default()
        };
        for name in self.names.values() {
            match name.kind {
                Some(kind) => *counts.kinds.entry(kind).or_default() += 1,
                None => counts.plain += 1,
            }
        }
        counts
    }

    /// Writes the names as text, a line each: the kind or `--`, a space and the name.
    pub fn write(&self, mut out: impl Write) -> io::Result<()> {
        writeln!(out, "{VOCABULARY_HEAD}")?;
        for name in self.names.values() {
            let kind = name.kind.unwrap_or(*b"--");
            let kind = [char::from(kind[0]), char::from(kind[1])];
            writeln!(out, "{}{} {}", kind[0], kind[1], name.text)?;
        }
        Ok(())
    }

    /// Reads what [`Self::write`] wrote; a line that is no name is passed over, and every hash
    /// is computed again from its name.
    pub fn read(mut from: impl BufRead) -> io::Result<Self> {
        let mut line = Vec::new();
        if !next_line(&mut from, &mut line)? || line != VOCABULARY_HEAD.as_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a kept GTA VI vocabulary",
            ));
        }
        let mut held = Self::default();
        while next_line(&mut from, &mut line)? {
            let Some((kind, name)) =
                (std::str::from_utf8(&line).ok()).and_then(|l| l.split_once(' '))
            else {
                continue;
            };
            let kind = match kind.as_bytes() {
                b"--" => None,
                [a, b] if kind_byte(*a) && kind_byte(*b) => Some([*a, *b]),
                _ => continue,
            };
            held.add(name, kind);
        }
        Ok(held)
    }

    /// The vocabulary kept at `file`; an empty one where none is kept or it does not read.
    pub fn load(file: &Path) -> Self {
        let read = |file: &Path| Self::read(BufReader::new(fs::File::open(file)?));
        read(file).unwrap_or_default()
    }

    /// Keeps the vocabulary at `file`, whose folder is made.
    pub fn save(&self, file: &Path) -> io::Result<()> {
        keep(file, |out| self.write(out))
    }
}

/// The next line of `from` without its line end, cut at [`MAX_LINE`] bytes; `false` at the end.
fn next_line(from: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<bool> {
    line.clear();
    let read = Read::take(&mut *from, MAX_LINE).read_until(b'\n', line)?;
    while line.last().is_some_and(|b| matches!(b, b'\n' | b'\r')) {
        line.pop();
    }
    Ok(read > 0)
}

/// Writes `file` whole beside itself, then moves it into place.
fn keep(
    file: &Path,
    write: impl FnOnce(&mut BufWriter<fs::File>) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let beside = file.with_extension("tmp");
    let mut out = BufWriter::new(fs::File::create(&beside)?);
    write(&mut out)?;
    out.flush()?;
    drop(out);
    fs::rename(beside, file)
}

/// The names the bodies of one archive state of themselves, by the offset each starts at.
pub type Labels = BTreeMap<u64, String>;

/// The folder the names are kept in, under the configuration directory `root`.
fn folder(root: &Path) -> PathBuf {
    root.join("gta6_names")
}

/// Where the vocabulary of every archive harvested is kept, under the configuration directory `root`.
pub fn vocabulary_file(root: &Path) -> PathBuf {
    folder(root).join("vocabulary.txt")
}

/// Where what is kept of `archive` alone lies under the configuration directory `root`, by its
/// `extension`.
fn archive_file(root: &Path, archive: &Path, extension: &str) -> PathBuf {
    let full = fs::canonicalize(archive).unwrap_or_else(|_| archive.to_path_buf());
    let word = name_hash64(full.to_string_lossy().as_bytes());
    folder(root).join(format!("{word:016x}.{extension}"))
}

/// Where the labels of `archive` are kept, under the configuration directory `root`.
pub fn labels_file(root: &Path, archive: &Path) -> PathBuf {
    archive_file(root, archive, "labels")
}

/// Writes the labels of an archive of `archive_len` bytes as text, a line each: the offset in
/// hexadecimal, a space and the name.
pub fn write_labels(labels: &Labels, archive_len: u64, mut out: impl Write) -> io::Result<()> {
    writeln!(out, "{LABELS_HEAD} {archive_len:X}")?;
    for (offset, name) in labels {
        writeln!(out, "{offset:X} {name}")?;
    }
    Ok(())
}

/// Reads what [`write_labels`] wrote of an archive that is still `archive_len` bytes; none of an
/// archive whose length changed.
pub fn read_labels(mut from: impl BufRead, archive_len: u64) -> io::Result<Labels> {
    let mut line = Vec::new();
    let mut labels = Labels::new();
    let head = format!("{LABELS_HEAD} {archive_len:X}");
    if !next_line(&mut from, &mut line)? || line != head.as_bytes() {
        return Ok(labels);
    }
    while next_line(&mut from, &mut line)? {
        let Some((offset, name)) =
            (std::str::from_utf8(&line).ok()).and_then(|l| l.split_once(' '))
        else {
            continue;
        };
        if let (Ok(offset), true) = (u64::from_str_radix(offset, 16), stated(name)) {
            labels.insert(offset, name.to_string());
        }
    }
    Ok(labels)
}

/// Keeps the labels of `archive` under the configuration directory `root`; the file written.
pub fn save_labels(root: &Path, archive: &Path, labels: &Labels) -> io::Result<PathBuf> {
    let len = fs::metadata(archive)?.len();
    let file = labels_file(root, archive);
    keep(&file, |out| write_labels(labels, len, out))?;
    Ok(file)
}

/// The labels kept of `archive` as it is now; none where none are kept or they do not read.
pub fn load_labels(root: &Path, archive: &Path) -> Labels {
    let read = || {
        let len = fs::metadata(archive)?.len();
        let file = fs::File::open(labels_file(root, archive))?;
        read_labels(BufReader::new(file), len)
    };
    read().unwrap_or_default()
}

/// A texture a model's material names by its 64-bit key, as a pack lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureUse {
    /// The first model that names it, by the name the model states less its ending, lower-cased.
    pub model: String,
    /// The hash of the shader parameter that binds it in that model.
    pub parameter: u32,
    /// How many models name it.
    pub models: u32,
}

/// The textures the models of archives name, by key.
pub type TextureUses = BTreeMap<u64, TextureUse>;

/// The texture keys the materials of each model in the body `find` name, `bytes` being what a
/// read of it returned: the model's name less its ending, lower-cased, and each key once with the
/// parameter that binds it first. None of a body that is no drawable, fragment or dictionary, or
/// whose shaders do not read from the bytes held.
pub fn body_textures(find: &Rpf9ScanFind, bytes: &[u8]) -> Vec<(String, Vec<(u64, u32)>)> {
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return Vec::new();
    };
    let Some(role) = signature.role() else {
        return Vec::new();
    };
    let virtual_section = resource.sections(bytes).0;
    let models = super::headerless::model_shaders(role, virtual_section).unwrap_or_default();
    let named = models.into_iter().filter(|(name, _)| stated(name));
    let textures = named.map(|(name, shaders)| {
        let mut seen = HashSet::new();
        let mut keys = Vec::new();
        for shader in &shaders {
            // A shader's texture parameters come first, one a slot.
            for (key, parameter) in shader.texture_keys.iter().zip(&shader.parameters) {
                if let Some(key) = (*key).filter(|key| seen.insert(*key)) {
                    keys.push((key, parameter.hash));
                }
            }
        }
        (split_kind(&name).0.to_ascii_lowercase(), keys)
    });
    textures.filter(|(_, keys)| !keys.is_empty()).collect()
}

/// Holds in `uses` that `model` names each of `keys`: the first model to name a key is the one
/// kept for it.
pub fn add_texture_uses(uses: &mut TextureUses, model: &str, keys: &[(u64, u32)]) {
    for (key, parameter) in keys {
        match uses.entry(*key) {
            std::collections::btree_map::Entry::Occupied(mut held) => held.get_mut().models += 1,
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(TextureUse {
                    model: model.to_string(),
                    parameter: *parameter,
                    models: 1,
                });
            }
        }
    }
}

/// Writes `uses` as text, a line each: the key and the parameter in hexadecimal, the count of
/// models and the first model's name.
pub fn write_texture_uses(uses: &TextureUses, mut out: impl Write) -> io::Result<()> {
    writeln!(out, "{TEXTURES_HEAD}")?;
    for (key, used) in uses {
        let (parameter, models) = (used.parameter, used.models);
        writeln!(out, "{key:016X} {parameter:08X} {models} {}", used.model)?;
    }
    Ok(())
}

/// Reads what [`write_texture_uses`] wrote into `uses`, a key another file's models name too
/// keeping its first model; a line that is no use is passed over.
pub fn read_texture_uses(mut from: impl BufRead, uses: &mut TextureUses) -> io::Result<()> {
    let mut line = Vec::new();
    if !next_line(&mut from, &mut line)? || line != TEXTURES_HEAD.as_bytes() {
        return Ok(());
    }
    while next_line(&mut from, &mut line)? {
        let mut words = std::str::from_utf8(&line)
            .unwrap_or_default()
            .splitn(4, ' ');
        let (Some(key), Some(parameter), Some(models), Some(model)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        let (Ok(key), Ok(parameter), Ok(models), true) = (
            u64::from_str_radix(key, 16),
            u32::from_str_radix(parameter, 16),
            models.parse::<u32>(),
            stated(model),
        ) else {
            continue;
        };
        match uses.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut held) => {
                let held = held.get_mut();
                held.models = held.models.saturating_add(models);
            }
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(TextureUse {
                    model: model.to_string(),
                    parameter,
                    models,
                });
            }
        }
    }
    Ok(())
}

/// Keeps the textures the models of `archive` name under the configuration directory `root`, in
/// place of those kept of it before; the file written.
pub fn save_texture_uses(root: &Path, archive: &Path, uses: &TextureUses) -> io::Result<PathBuf> {
    let file = archive_file(root, archive, "textures");
    keep(&file, |out| write_texture_uses(uses, out))?;
    Ok(file)
}

/// The textures the models of every archive harvested name, as kept under the configuration
/// directory `root`; none where none are kept.
pub fn load_texture_uses(root: &Path) -> TextureUses {
    let mut files: Vec<PathBuf> = (fs::read_dir(folder(root)).into_iter().flatten())
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "textures"))
        .collect();
    // In one order, so the model kept for a key two archives name is the same each time.
    files.sort();
    let mut uses = TextureUses::new();
    for file in files {
        if let Ok(file) = fs::File::open(file) {
            // A file cut short gives the lines before the cut.
            let _ = read_texture_uses(BufReader::new(file), &mut uses);
        }
    }
    uses
}

/// Whether `name` reads as a name a body states of itself: a name the vocabulary would hold, with
/// or without a `.#xx` ending.
fn stated(name: &str) -> bool {
    let stem = split_kind(name).0;
    !stem.is_empty() && stem.len() <= MAX_NAME && name.bytes().all(|b| b.is_ascii_graphic())
}

/// Whether a harvest reads the body `find`: a resource, a stored RSC8, text or metadata. The
/// other zlib runs are vertex and index data, and audio, movies and the nested archives' own
/// names spell none.
pub fn harvests(find: &Rpf9ScanFind) -> bool {
    match find.kind {
        Rpf9BodyKind::Zlib(run) => run.resource.is_some() || run.content != Rpf9Content::Other,
        Rpf9BodyKind::StoredRsc8 { .. } => true,
        _ => false,
    }
}

/// Holds in `vocabulary` each name the first `prefix` bytes of the body `find` spell, `bytes`
/// being what a read of it returned; the name the body states of itself, where it is a drawable
/// or a fragment whose root states one.
pub fn harvest_body(
    vocabulary: &mut Vocabulary,
    find: &Rpf9ScanFind,
    bytes: &[u8],
    prefix: usize,
) -> Option<String> {
    body_names(find, bytes, prefix, &mut |name, kind| {
        vocabulary.add(name, kind);
    })
}

/// As [`harvest_body`], each name handed to `each` with the ending it is spelled with.
fn body_names(
    find: &Rpf9ScanFind,
    bytes: &[u8],
    prefix: usize,
    each: &mut impl FnMut(&str, Option<[u8; 2]>),
) -> Option<String> {
    if !harvests(find) {
        return None;
    }
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature,
        ..
    }) = find.kind
    else {
        harvest(bytes, prefix, &mut |found| each(found.name, found.kind));
        return None;
    };
    // The physical section is vertex and index data.
    let virtual_section = resource.sections(bytes).0;
    harvest(virtual_section, prefix, &mut |found| {
        each(found.name, found.kind)
    });
    let role = signature.and_then(|s| s.role())?;
    if !matches!(role, Rpf9Role::Drawable | Rpf9Role::DrawableWithBound) {
        return None;
    }
    let name = super::headerless::stated_model_name(virtual_section).ok()?;
    if !stated(&name) {
        return None;
    }
    // Its own name is held wherever the root points, past the prefix too.
    let (stem, kind) = split_kind(&name);
    each(stem, kind);
    Some(name)
}

/// The word a map data body states of its own name; `None` for any other body.
#[cfg(feature = "meta")]
fn map_word(find: &Rpf9ScanFind, bytes: &[u8]) -> Option<u64> {
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(bytes);
    super::map::stated_name_word(virtual_section, physical)
}

/// What a harvest of one archive gave.
#[derive(Debug, Default)]
pub struct ArchiveHarvest {
    pub vocabulary: Vocabulary,
    /// The name each drawable and fragment states of itself, by its offset.
    pub labels: Labels,
    /// The textures the materials of its models name.
    pub textures: TextureUses,
    /// The bodies read for names.
    pub bodies: usize,
    /// The 64-bit word each map data body states of its own name.
    pub map_words: Vec<u64>,
    /// Whether it was stopped before the archive's end; what it read until then is given.
    pub stopped: bool,
}

/// A name the scan of one range read, as written.
#[derive(Debug)]
struct Seen {
    text: Box<str>,
    /// Where the last body that spells it starts.
    last: u64,
    /// The ending a body spelled it with, and where the last body that spelled one starts.
    kind: Option<([u8; 2], u64)>,
}

/// What the scan of one range of an archive gave.
#[derive(Debug, Default)]
struct Ranged {
    /// The range's end: a body that starts at or past it is a later range's.
    limit: u64,
    /// Where each find starts and where the scan stood after it, in the scan's order.
    finds: Vec<(u64, u64)>,
    /// Where each body read for names starts.
    bodies: Vec<u64>,
    labels: Vec<(u64, String)>,
    names: HashMap<u64, Seen>,
    /// Each model that names textures: where its body starts, its name and the keys.
    textures: Vec<ModelTextures>,
    /// Each map data body: where it starts and the word it states of its name.
    maps: Vec<(u64, u64)>,
    /// Where the scan ended: past the limit where its last body is.
    end: u64,
}

/// A model that names textures: where its body starts, its name, and each key with its use.
type ModelTextures = (u64, String, Vec<(u64, u32)>);

/// The names of the bodies of `file` that start in `from..to`, each read once, and what the scan
/// leaves the scan of the bytes after.
fn scan_range(
    file: &fs::File,
    (from, to): (u64, u64),
    carry: Option<Rpf9ScanCarry>,
    keep: usize,
    stop: &(dyn Fn() -> bool + Sync),
) -> io::Result<(Ranged, Rpf9ScanCarry)> {
    let mut ranged = Ranged {
        limit: to,
        end: from,
        ..Ranged::default()
    };
    if from >= to {
        return Ok((ranged, carry.unwrap_or_default()));
    }
    let scan = Rpf9Scan::new(file, from..to).map_err(io::Error::other)?;
    let mut scan = match carry {
        Some(carry) => scan.carrying(carry),
        None => scan,
    }
    .keeping(keep);
    while let Some(find) = scan.next() {
        // A scan that fails says so once and ends: the rest of its range is not read.
        let (Ok(find), false) = (find, stop()) else {
            break;
        };
        let at = find.offset;
        ranged.finds.push((at, scan.position()));
        let bytes = scan.kept(&find);
        if bytes.is_empty() || !harvests(&find) {
            continue;
        }
        ranged.bodies.push(at);
        let names = &mut ranged.names;
        let label = body_names(&find, bytes, keep, &mut |name, kind| {
            if !holdable(name) {
                return;
            }
            let seen = (names.entry(name_hash64(name.as_bytes()))).or_insert_with(|| Seen {
                text: name.into(),
                last: at,
                kind: None,
            });
            seen.last = at;
            if let Some(kind) = kind {
                seen.kind = Some((seen.kind.map_or(kind, |(first, _)| first), at));
            }
        });
        if let Some(name) = label {
            ranged.labels.push((at, name));
        }
        #[cfg(feature = "meta")]
        ranged
            .maps
            .extend(map_word(&find, bytes).map(|word| (at, word)));
        let models = body_textures(&find, bytes).into_iter();
        (ranged.textures).extend(models.map(|(model, keys)| (at, model, keys)));
    }
    ranged.end = scan.position();
    Ok((ranged, scan.carry()))
}

/// The ranges of a harvest joined in the archive's order, each from where the bodies before end.
// A scan that starts inside a body, as a range's does where the range before ends in one, reads
// that body's later streams as runs of their own, and its last such run may take the body after
// for one more of its chunks. So what a range found before the end of the bodies read is dropped,
// and where one of its finds reaches past that end the bytes from there are scanned again.
#[derive(Debug, Default)]
struct Joined {
    harvest: ArchiveHarvest,
    /// Where the last body joined ends.
    read_to: u64,
    /// The range to join next, and the ranges scanned before their turn.
    next: u64,
    waiting: BTreeMap<u64, Ranged>,
}

impl Joined {
    /// Holds what `ranged` read of the bodies that start at or past the end of those joined.
    fn keep(&mut self, ranged: &Ranged) {
        let from = self.read_to;
        self.harvest.bodies += ranged.bodies.iter().filter(|at| **at >= from).count();
        let labels = ranged.labels.iter().filter(|(at, _)| *at >= from);
        self.harvest.labels.extend(labels.cloned());
        for (_, model, keys) in ranged.textures.iter().filter(|(at, ..)| *at >= from) {
            add_texture_uses(&mut self.harvest.textures, model, keys);
        }
        let maps = ranged.maps.iter().filter(|(at, _)| *at >= from);
        self.harvest.map_words.extend(maps.map(|(_, word)| *word));
        for seen in ranged.names.values().filter(|seen| seen.last >= from) {
            let kind = seen.kind.filter(|(_, at)| *at >= from);
            (self.harvest.vocabulary).add(&seen.text, kind.map(|(kind, _)| kind));
        }
    }

    /// Joins `ranged`, the range after the last one joined; `again` scans the bytes it is given
    /// from a body's start.
    fn join(
        &mut self,
        ranged: &Ranged,
        again: &mut dyn FnMut(u64, u64) -> io::Result<Ranged>,
    ) -> io::Result<()> {
        loop {
            let from = self.read_to;
            if from >= ranged.limit {
                return Ok(());
            }
            let over = (ranged.finds.iter()).find(|(at, end)| *at < from && from < *end);
            let Some((_, end)) = over else {
                // The scan stood at `from` between two finds, as a scan from a body's start does.
                self.keep(ranged);
                self.read_to = from.max(ranged.end);
                return Ok(());
            };
            let to = (*end).min(ranged.limit);
            let truly = again(from, to)?;
            self.keep(&truly);
            self.read_to = to.max(truly.end);
        }
    }

    /// Joins each range scanned whose turn has come.
    fn join_waiting(
        &mut self,
        again: &mut dyn FnMut(u64, u64) -> io::Result<Ranged>,
    ) -> io::Result<()> {
        while let Some(ranged) = self.waiting.remove(&self.next) {
            self.next += 1;
            self.join(&ranged, again)?;
        }
        Ok(())
    }
}

/// Harvests the first `keep` inflated bytes of every body of the archive at `path` a harvest
/// reads, on up to `workers` threads; `stop` is asked between bodies, and `reached` is told the
/// archive's bytes done and its length as each range of it ends.
// Each body is inflated once, by the scan that finds it, and no worker holds more of one than
// `keep` bytes beside the scan's own window.
pub fn harvest_archive(
    path: &Path,
    keep: usize,
    workers: usize,
    stop: &(dyn Fn() -> bool + Sync),
    reached: &(dyn Fn(u64, u64) + Sync),
) -> io::Result<ArchiveHarvest> {
    // An archive shorter than a range a worker is cut finer, so every worker reads some of it:
    // a scan runs over the bytes no body claims a slot at a time, and two ranges left a small
    // archive of them to two cores.
    let each = fs::metadata(path)?.len() / workers.max(1) as u64;
    let range = (each.next_multiple_of(SMALL_RANGE)).clamp(SMALL_RANGE, HARVEST_RANGE);
    harvest_ranges(path, keep, range, workers, stop, reached)
}

/// As [`harvest_archive`], a scan to each `range` bytes of the archive.
// One worker scans the ranges in order, each from where the last ended. Several start each range
// at its boundary and the ranges are joined in order ([`Joined`]), so every body is read once
// whatever the workers; a nested archive's names that a range boundary parts from its header
// are then found as no body, and a harvest reads none of them.
// ponytail: the bytes scanned again where a find reaches past a body's end are read by the worker
// that joins, with the join held; hand them to the workers if a profile shows the join waiting.
fn harvest_ranges(
    path: &Path,
    keep: usize,
    range: u64,
    workers: usize,
    stop: &(dyn Fn() -> bool + Sync),
    reached: &(dyn Fn(u64, u64) + Sync),
) -> io::Result<ArchiveHarvest> {
    let len = fs::metadata(path)?.len();
    let ranges = len.div_ceil(range.max(1));
    let workers = (workers.max(1) as u64).min(ranges.max(1));
    let (next, done) = (AtomicU64::new(0), AtomicU64::new(0));
    let joined = Mutex::new(Joined::default());
    let work = || -> io::Result<()> {
        let file = fs::File::open(path)?;
        let mut again = |from: u64, to: u64| -> io::Result<Ranged> {
            Ok(scan_range(&file, (from, to), None, keep, stop)?.0)
        };
        let mut carried = None;
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= ranges || stop() {
                return Ok(());
            }
            let start = index.saturating_mul(range);
            let end = start.saturating_add(range).min(len);
            // One worker takes the ranges in order, so each scan goes on from the last.
            let (from, carry) = match carried.filter(|_| workers == 1) {
                Some((position, carry)) => (start.max(position), Some(carry)),
                None => (start, None),
            };
            let (ranged, carry) = scan_range(&file, (from, end), carry, keep, stop)?;
            carried = Some((ranged.end, carry));
            {
                let mut joined = joined.lock().unwrap_or_else(|p| p.into_inner());
                joined.waiting.insert(index, ranged);
                joined.join_waiting(&mut again)?;
            }
            let step = end - start;
            reached(done.fetch_add(step, Ordering::Relaxed) + step, len);
        }
    };
    let gave: Vec<io::Result<()>> = match workers {
        // No thread is started for one, as where none can be.
        1 => vec![work()],
        _ => std::thread::scope(|scope| {
            let started: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
            let ended = started.into_iter().map(|worker| {
                let ended = worker.join();
                ended.unwrap_or_else(|_| Err(io::Error::other("a harvest worker panicked")))
            });
            ended.collect()
        }),
    };
    gave.into_iter().collect::<io::Result<()>>()?;
    let mut joined = joined.into_inner().unwrap_or_else(|p| p.into_inner());
    // A stop leaves ranges scanned past one that was not: what they read is given too.
    if !joined.waiting.is_empty() {
        let file = fs::File::open(path)?;
        let mut again = |from: u64, to: u64| -> io::Result<Ranged> {
            Ok(scan_range(&file, (from, to), None, keep, stop)?.0)
        };
        for ranged in std::mem::take(&mut joined.waiting).into_values() {
            joined.join(&ranged, &mut again)?;
        }
    }
    joined.harvest.stopped = stop();
    Ok(joined.harvest)
}

#[cfg(feature = "meta")]
impl super::map::Entity {
    /// The archetype's name, where `vocabulary` holds one both of its hashes name.
    pub fn archetype_in<'a>(&self, vocabulary: &'a Vocabulary) -> Option<&'a str> {
        vocabulary.confirmed(self.archetype_name, self.archetype_word)
    }
}

#[cfg(feature = "meta")]
impl super::map::Archetype {
    /// The archetype's name, where `vocabulary` holds one both of its hashes name.
    pub fn name_in<'a>(&self, vocabulary: &'a Vocabulary) -> Option<&'a str> {
        vocabulary.confirmed(self.name, self.name_word)
    }
}

#[cfg(feature = "meta")]
impl super::map::MapData {
    /// The map's name, where `vocabulary` holds one both of its hashes name.
    pub fn name_in<'a>(&self, vocabulary: &'a Vocabulary) -> Option<&'a str> {
        vocabulary.confirmed(self.name, self.name_word)
    }
}

#[cfg(feature = "meta")]
impl super::map::links::InteriorEntity {
    /// The archetype's name, where `vocabulary` holds one both of its hashes name.
    pub fn archetype_in<'a>(&self, vocabulary: &'a Vocabulary) -> Option<&'a str> {
        vocabulary.confirmed(self.archetype_name, self.archetype_word)
    }
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
