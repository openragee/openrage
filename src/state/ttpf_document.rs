//! GTA VI's textures in the app: a `.ttpf` opened as its directory, a texture read from the file
//! only when it is shown and replaced from an image, the pack saved through a file beside it; and a scanned
//! texture dictionary, read-only. Each shows as Legacy textures so the `.ytd` preview and export
//! take them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use or_io::authoring::TextureBuildOptions;
use or_io::codec::texture::{GrcTextureFormat, RgbaImage};
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::shared::PgDictionary;
use or_io::gta6::bcpack::calibrate::load_cache;
use or_io::gta6::bcpack::{Decode, Reading, Standing};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{Dictionary, DictionaryTexture, Entry, Resource};
use or_io::gta6::texture_names::name_keys;
use or_io::gta6::TexturePack;

use super::document::{DocPayload, Document};
use super::edit_history::Change;
use super::texture_edit::TextureEdit;
use super::textures::{self, Texture, TextureSource};

/// Names whose 64-bit hash may key a pack's texture: each texture dictionary's the app has read,
/// each one the user typed that a pack's key matched, and each the naming rules gave for a key.
// ponytail: one list for the whole app, which no view empties.
static NAMES: Mutex<BTreeMap<u64, String>> = Mutex::new(BTreeMap::new());

fn names() -> std::sync::MutexGuard<'static, BTreeMap<u64, String>> {
    NAMES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The name the app has learned for a texture key, else the one the kept vocabulary holds for it.
// ⚠ What a harvest learns lasts a session, and the vocabulary it keeps is what names a key after it.
pub(crate) fn known_name(key: u64) -> Option<String> {
    let learned = names().get(&key).cloned();
    named_by(learned, &super::gta6_names::vocabulary(), key)
}

pub(super) fn named_by(
    learned: Option<String>,
    vocabulary: &or_io::gta6::names::Vocabulary,
    key: u64,
) -> Option<String> {
    learned.or_else(|| vocabulary.name64(key).map(str::to_string))
}

/// How many texture names the app has learned.
pub(crate) fn names_known() -> usize {
    names().len()
}

/// How each texture read was rebuilt from its bodies through the BCPack coder, by its pack's
/// file and its index: a worker's copy of a document reads for the one that is shown.
// ponytail: one map for the whole app, which a calibration that ends empties.
static DECODES: Mutex<BTreeMap<(PathBuf, usize), Decode>> = Mutex::new(BTreeMap::new());

fn decodes() -> std::sync::MutexGuard<'static, BTreeMap<(PathBuf, usize), Decode>> {
    DECODES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forget how every texture was rebuilt, as a calibration that ended changes it.
pub(super) fn forget_decodes() {
    decodes().clear();
}

/// Where the BCPack coder's tables fitted on this machine are kept.
pub(crate) fn calibration_dir() -> Option<PathBuf> {
    super::prefs::config_root().map(|dir| dir.join("bcpack"))
}

/// The calibration kept for `pack`, if one read it, set on it.
fn calibrate(pack: &mut TexturePack) {
    let tables = calibration_dir().and_then(|dir| load_cache(&dir, pack.member_key()));
    pack.set_calibration(tables.map(Arc::new));
}

/// The pack `payload` is, read from now on through the calibration kept for it, with what it has read forgotten.
pub(super) fn recalibrate(payload: &mut DocPayload) {
    if let Some(ttpf) = edited(payload) {
        calibrate(&mut ttpf.pack);
    }
}

pub struct TtpfDocument {
    pub pack: TexturePack,
    /// The texture each key on disk keys.
    keys: HashMap<u64, usize>,
    read: Mutex<Read>,
}

/// What reading has found, so a frame that asks again reads nothing from disk.
#[derive(Default)]
struct Read {
    refused: HashMap<usize, String>,
    // ponytail: one texture slice held; an LRU if a gallery's tiles and the preview thrash it.
    last: Option<((usize, u32), Arc<GrcTexture>)>,
    /// The slice of a cube, an array or a volume that is shown, where it is not the first.
    shown: HashMap<usize, u32>,
}

impl TtpfDocument {
    /// Reads the pack's header, table and directory, and no body.
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let mut pack = TexturePack::open(path).map_err(|e| e.to_string())?;
        // A body that does not decode exactly is drawn through a calibration that read the pack.
        calibrate(&mut pack);
        let keys = (pack.entries.iter().enumerate())
            .filter(|(_, e)| e.on_disk && e.key != 0)
            .map(|(index, e)| (e.key, index))
            .collect();
        Ok(Self {
            pack,
            keys,
            read: Mutex::default(),
        })
    }

    /// Texture `index`'s name where a known one hashes to its key, else `hash_` and the key, or `#` and the index without one.
    pub fn name(&self, index: usize) -> String {
        let Some(entry) = (self.pack.entries.get(index)).filter(|e| e.on_disk && e.key != 0) else {
            return format!("#{index}");
        };
        known_name(entry.key).unwrap_or_else(|| format!("hash_{:016X}", entry.key))
    }

    /// What texture `index` is used by, where no name is known for it and a model of an archive
    /// harvested names its key: what its tile and its row say beside the hash.
    pub fn use_label(&self, index: usize) -> Option<String> {
        let entry = (self.pack.entries.get(index)).filter(|e| e.on_disk && e.key != 0)?;
        if known_name(entry.key).is_some() {
            return None;
        }
        super::gta6_names::texture_use_label(entry.key)
    }

    /// The texture [`Self::name`] names, or whose key a name of the user's hashes to.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        if let Some(index) = name.strip_prefix('#') {
            return index.parse().ok();
        }
        let key = match name.strip_prefix("hash_") {
            Some(hex) => u64::from_str_radix(hex, 16).ok()?,
            None => name_hash64(name.as_bytes()),
        };
        self.keys.get(&key).copied()
    }

    /// The texture `name` keys, which is then shown by that name; `None` when it keys none here.
    pub fn learn(&self, name: &str) -> Option<usize> {
        let key = name_hash64(name.as_bytes());
        let index = self.keys.get(&key).copied()?;
        names().insert(key, name.to_string());
        Some(index)
    }

    /// Why texture `index` is not shown, from what the table says or a read that failed.
    pub fn refusal(&self, index: usize) -> Option<String> {
        let known = self.lock().refused.get(&index).cloned();
        known.or_else(|| self.pack.check(index).err().map(|e| e.to_string()))
    }

    /// The slices texture `index` is shown one at a time in: a cube's six faces, an array's or a volume's slices; 1 for any other.
    pub fn slices(&self, index: usize) -> u32 {
        self.pack.entries.get(index).map_or(1, |e| e.slices.into())
    }

    /// Shows slice `slice` of texture `index` wherever it is asked for from now on.
    pub fn show_slice(&self, index: usize, slice: u32) {
        self.lock().shown.insert(index, slice);
    }

    /// The slice of texture `index` shown, which a replacement takes: 0 until another is shown.
    pub fn shown_slice(&self, index: usize) -> u32 {
        let shown = self.lock().shown.get(&index).copied().unwrap_or(0);
        shown.min(self.slices(index) - 1)
    }

    /// Texture `index`'s slice shown, read from the file the first time it is asked for.
    pub fn texture(&self, index: usize) -> Result<Arc<GrcTexture>, String> {
        let mut read = self.lock();
        let shown = read.shown.get(&index).copied().unwrap_or(0);
        let at = (index, shown.min(self.slices(index) - 1));
        if let Some((_, texture)) = read.last.as_ref().filter(|(shown, _)| *shown == at) {
            return Ok(texture.clone());
        }
        if let Some(why) = read.refused.get(&index) {
            return Err(why.clone());
        }
        let noted = self.pack.texture_noted(index).map(|(texture, how)| {
            decodes().insert((self.pack.path().to_path_buf(), index), how);
            self.keep_against(&how);
            texture
        });
        match noted
            .map_err(|e| e.to_string())
            .and_then(|t| grc(self.name(index), t.face(at.1).unwrap_or(t)))
        {
            Ok(texture) => {
                let texture = Arc::new(texture);
                read.last = Some((at, texture.clone()));
                Ok(texture)
            }
            Err(why) => {
                read.refused.insert(index, why.clone());
                Err(why)
            }
        }
    }

    /// What the textures of the pack read so far count together.
    // ponytail: summed at each call, which each label of a frame makes; a sum kept beside the
    // reads if a pack with thousands of textures read shows in a frame's time.
    fn read_so_far(&self) -> Decode {
        let path = self.pack.path().to_path_buf();
        let of_the_pack = (path.clone(), 0)..=(path, usize::MAX);
        let mut all = Decode::default();
        for (_, how) in decodes().range(of_the_pack) {
            all.add(*how);
        }
        all
    }

    /// How the tables file the pack reads through stands for it: as it is kept, and against the
    /// file for this pack once a texture of it read so far has counted a block against it.
    fn standing(&self) -> Standing {
        self.pack.standing_after(&self.read_so_far())
    }

    /// Keeps, where `how`, a texture's read, counted a block against the tables supplied, what
    /// the pack's textures read so far count against them: a later read of the pack, here or
    /// from the command line, is then not called exact either.
    fn keep_against(&self, how: &Decode) {
        if how.supplied == 0 || how.disagree() == 0 {
            return;
        }
        let kept = calibration_dir().map(|dir| self.pack.keep_read(&dir, &self.read_so_far()));
        if let Some(Err(e)) = kept {
            tracing::warn!("what a read counted against the supplied tables is not kept: {e}");
        }
    }

    /// How texture `index` was rebuilt and what its read may be called, once it has been read.
    fn reading(&self, index: usize) -> Option<(Decode, Reading)> {
        let read = decodes()
            .get(&(self.pack.path().to_path_buf(), index))
            .copied();
        read.map(|how| (how, how.reading(self.standing())))
    }

    /// Whether the pack reads its coded bodies through tables the user supplied: a file gives
    /// every mode's or none.
    fn supplied(&self) -> bool {
        self.pack.reads_supplied(1)
    }

    /// What texture `index`'s read through the BCPack coder may be called: exact, an estimate
    /// through tables fitted on this machine, what speaks against the tables the user supplied,
    /// or that a coded body of it was not read. `None` for a texture with no such body, and for
    /// one not read yet of which nothing can be said.
    ///
    /// ⚠ A read is called exact only once it has been made: that a tables file is kept says
    /// nothing of a texture.
    pub fn decode_label(&self, index: usize) -> Option<String> {
        use or_io::gta6::ttpf::{KIND_STORED, KIND_ZLIB};
        let rows = self.pack.rows_of(index)?;
        let mut coded = (rows.map(|row| self.pack.rows[row].kind))
            .filter(|&kind| kind != KIND_STORED && kind != KIND_ZLIB);
        let kind = coded.next()?;
        let not_checked = || tr!("supplied tables, not checked").to_string();
        let no_rule = || tr!("supplied tables, no rule checks this format").to_string();
        let not_the_coders = || tr!("not this coder's tables").to_string();
        let standing = self.standing();
        Some(match self.reading(index) {
            Some((_, Reading::Exact)) => tr!("exact").to_string(),
            // Nothing through the coder was rebuilt, or a body under the ones that were was not.
            Some((how, Reading::CodedUnread)) => match how.supplied {
                0 => tr!("plain levels read, coded levels not read").to_string(),
                _ => tr!("a coded body not read, the levels above it read").to_string(),
            },
            Some((_, Reading::BuiltIn)) => tr!("built-in BC1 tables, not confirmed").to_string(),
            Some((_, Reading::Unseconded { blocks })) => tr!(
                "{count} coded blocks read a value the built-in BC1 tables give otherwise",
                count = blocks
            ),
            Some((how, Reading::Calibrated)) => tr!(
                "calibrated, {percent} % of its coded blocks through a slot not known",
                percent = format!("{:.1}", how.unknown_percent())
            ),
            Some((_, Reading::Unchecked)) => not_checked(),
            Some((_, Reading::Unruled)) => no_rule(),
            Some((_, Reading::Disagrees { disagree, blocks })) => tr!(
                "{count} of {blocks} coded blocks disagree with the supplied tables",
                count = disagree,
                blocks = blocks
            ),
            Some((_, Reading::NotTheCoders { .. })) => not_the_coders(),
            // Not read yet: only how the tables stand for its format is known.
            None if self.supplied() => match kind {
                _ if standing.refuted().is_some() => not_the_coders(),
                3..=5 => no_rule(),
                6 | 7 if standing.held(kind).is_none() => not_checked(),
                _ => return None,
            },
            // A BC3, BC4 or BC5 body reads only through a calibration.
            None if self.pack.is_calibrated() && (3..=5).contains(&kind) => {
                tr!("calibrated").to_string()
            }
            None => return None,
        })
    }

    /// [`Self::decode_label`] where the texture is an estimate through fitted tables.
    pub fn estimate_label(&self, index: usize) -> Option<String> {
        match self.reading(index) {
            Some((_, Reading::Calibrated)) => self.decode_label(index),
            Some(_) => None,
            None if self.supplied() => None,
            None => self.decode_label(index),
        }
    }

    /// [`Self::decode_label`] where the texture has been read and its read is neither called
    /// exact nor an estimate: something speaks against the read through the tables the user
    /// supplied or nothing confirms it, it went through BC1's built-in tables alone, or a coded
    /// body of it was not read. The texture view and an export say it under "Not called exact".
    pub fn doubt_label(&self, index: usize) -> Option<String> {
        match self.reading(index)?.1 {
            Reading::Exact | Reading::Calibrated => None,
            _ => self.decode_label(index),
        }
    }

    /// What speaks against the tables file the pack reads through: a check that failed it, or
    /// the textures of this pack read so far, here or by a listing or an export kept with the
    /// pack; `None` where nothing does.
    fn tables_warning(&self) -> Option<String> {
        let standing = self.standing();
        if let Some(failed) = standing.failed() {
            return Some(tr!(
                "The tables you supplied are not this coder's: a check found {count} of {blocks} coded blocks that read a value no table gives or hold what no encoder writes. No texture read through them is the pack's",
                count = failed.disagree,
                blocks = failed.blocks
            ));
        }
        standing.failed_in_pack().map(|against| tr!(
            "The tables you supplied disagree with this pack: {count} of the coded blocks read so far read a value no table gives or hold what no encoder writes",
            count = against.disagree
        ))
    }

    /// How the tables file the pack reads through stands, and what the textures read so far say
    /// against it; `None` where the pack reads through none.
    pub fn tables_note(&self) -> Option<String> {
        if !self.supplied() {
            return None;
        }
        let standing = self.standing();
        let held = |kind: u8| standing.held(kind).unwrap_or(0);
        Some(match self.tables_warning() {
            Some(warning) => warning,
            None if held(6) + held(7) == 0 => tr!(
                "The tables you supplied are not checked against your packs: `openrage-cli ttpf tables --check` does that for the packs of a folder"
            )
            .to_string(),
            None => tr!(
                "A check of the tables you supplied held {bc6h} BC6H blocks and {bc7} BC7 blocks to the rules of their formats, and none broke one. No rule holds a BC1, BC3, BC4 or BC5 block",
                bc6h = held(6),
                bc7 = held(7)
            ),
        })
    }

    /// What the word texture `index`'s read is called by rests on, a sentence a line; none for
    /// a texture with no body through the coder, one not read yet and an estimate.
    ///
    /// ⚠ The counts it gives are of the texture's own format: what a check found of another
    /// format says nothing of this texture.
    pub fn decode_basis(&self, index: usize) -> Vec<String> {
        let (Some(_), Some((how, reading))) = (self.decode_label(index), self.reading(index))
        else {
            return Vec::new();
        };
        let standing = self.standing();
        let checked = [6, 7].into_iter().find(|&kind| how.kinds >> kind & 1 != 0);
        let own = match reading {
            Reading::BuiltIn => Some(tr!(
                "BC1's built-in tables were fitted by their author and nothing confirms them value for value, so a read through them alone is not called exact. Through a tables file you supply, BC1 reads through that file, and a block is counted where the two tables give another value"
            ).to_string()),
            Reading::Unseconded { .. } => Some(tr!(
                "The blocks counted read a preset slot or an index symbol for which BC1's built-in tables give another value, or none. They were rebuilt through the tables you supplied, and no second table stands behind them"
            ).to_string()),
            // A texture with no body through the coder has no label, so none is read here.
            Reading::Exact if how.supplied == 0 => None,
            Reading::Exact => Some(match checked {
                Some(kind) => tr!(
                    "Exact rests on the rules of this block format, which a wrong table breaks: none of this texture's {count} coded blocks breaks one, and neither did any of the {blocks} blocks of the format that a check read through the tables you supplied. The rules see an entry of a preset through the blocks that read it: whether it is in its place, a BC7 end's p-bit and the top bit of an anchor's index. They do not see each end value, nor an entry that few blocks read. No texture is stored both coded and plain, so no read is compared with the pack's own blocks",
                    count = how.supplied,
                    blocks = standing.held(kind).unwrap_or(0)
                ),
                None => tr!(
                    "Exact rests on two tables: the tables you supplied and BC1's built-in ones give the same value for every preset slot and index symbol this texture reads. No texture is stored both coded and plain, so no read is compared with the pack's own blocks"
                ).to_string(),
            }),
            Reading::Unchecked => Some(tr!(
                "Read through the tables you supplied. None of this texture's coded blocks breaks a rule of its format, but one texture reads few of a preset's entries: the read is called exact only once `openrage-cli ttpf tables --check` has held the tables to the blocks of this format in a folder's packs"
            ).to_string()),
            Reading::Unruled => Some(tr!(
                "Read through the tables you supplied. No rule of this block format can show a wrong table and no second table is known, so nothing can confirm the read and it is not called exact"
            ).to_string()),
            Reading::Disagrees { .. } => Some(tr!(
                "A block that disagrees was not rebuilt as the pack stores it: the tables you supplied are wrong for it"
            ).to_string()),
            Reading::CodedUnread => Some(tr!(
                "The levels read are not all of this texture: a body of it stored through the coder was not read, so nothing is said of the levels that body holds, and the texture is not called exact"
            ).to_string()),
            Reading::NotTheCoders { .. } | Reading::Calibrated => None,
        };
        own.into_iter().chain(self.tables_warning()).collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Read> {
        self.read
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The pack as a save writes it: every row it leaves alone as the file stores it.
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        self.pack.save_to(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

/// The same pack with nothing read yet, so an edit of one copy shows nothing stale in the other.
impl Clone for TtpfDocument {
    fn clone(&self) -> Self {
        Self {
            pack: self.pack.clone(),
            keys: self.keys.clone(),
            read: Mutex::default(),
        }
    }
}

/// The pack's document, which an edit changes, with what it has read forgotten.
fn edited(payload: &mut DocPayload) -> Option<&mut TtpfDocument> {
    let DocPayload::Ttpf(ttpf) = payload else {
        return None;
    };
    let ttpf = Arc::make_mut(ttpf);
    ttpf.read = Mutex::default();
    Some(ttpf)
}

/// The label and step that undo an edit, and what it changed.
type Replaced = ((String, Change), TextureEdit);

/// Texture `name` of the pack `payload` is, replaced from `image`: the step that undoes it and what changed; `None` for any other document.
pub(super) fn replace(
    payload: &mut DocPayload,
    name: &str,
    image: &RgbaImage,
    options: &TextureBuildOptions,
) -> Option<Result<Replaced, String>> {
    // A cube's, an array's or a volume's shown slice is replaced alone; an edit forgets it.
    let shown = match &*payload {
        DocPayload::Ttpf(ttpf) => ttpf.index_of(name).map(|index| ttpf.shown_slice(index)),
        _ => None,
    };
    let ttpf = edited(payload)?;
    let Some(index) = ttpf.index_of(name) else {
        return Some(Err(tr!("{name} is not in this pack", name = name)));
    };
    if let Some(refused) = bcpack_refusal(&ttpf.pack, index) {
        return Some(Err(refused));
    }
    let before = ttpf.pack.replaced_mut().clone();
    let slice = shown.unwrap_or(0);
    let mips = match ttpf.pack.replace_slice(index, slice, image, options) {
        Ok(mips) => mips,
        Err(e) => return Some(Err(e.to_string())),
    };
    let format = (ttpf.pack.entries[index].format()).map_or(GrcTextureFormat::None, |f| f.format());
    let change = Change::part(format!("texture {name}"), before, |payload| {
        Some(edited(payload)?.pack.replaced_mut())
    });
    let edit = TextureEdit::Imported {
        name: name.to_string(),
        format,
        mips,
        replaced: true,
    };
    Some(Ok(((tr!("Replace {name}", name = name), change), edit)))
}

/// Why texture `index` is not replaced, where it is stored through the BCPack coder OpenRage does
/// not encode.
pub(super) fn bcpack_refusal(
    pack: &or_io::gta6::ttpf::TexturePack,
    index: usize,
) -> Option<String> {
    use or_io::gta6::ttpf_author::{bcpack_kind, AuthorError};
    bcpack_kind(pack, index).map(|kind| AuthorError::BcPack { index, kind }.to_string())
}

/// A writer that fails once `cancel` is set, so a save stops between two writes.
struct Stoppable<'a, W> {
    out: W,
    cancel: &'a or_job::Cancel,
}

impl<W: std::io::Write> std::io::Write for Stoppable<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancel.is_set() {
            return Err(std::io::Error::other("stopped"));
        }
        self.out.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// The save of `ttpf` to `path`: its rows streamed, the ones it leaves alone from the file it was
/// opened from, into a file beside `path` that is renamed over it once whole.
pub(super) fn saved(
    ttpf: &TtpfDocument,
    path: &Path,
    cancel: &or_job::Cancel,
    doc_id: super::DocumentId,
    at: u64,
) -> super::SaveResult {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = std::fs::File::create(&temp)
        .map_err(|e| e.to_string())
        .and_then(|file| {
            let file = std::io::BufWriter::new(file);
            let mut out = Stoppable { out: file, cancel };
            ttpf.pack.save_to(&mut out).map_err(|e| e.to_string())?;
            std::io::Write::flush(&mut out).map_err(|e| e.to_string())
        });
    let written = written.and_then(|()| {
        if cancel.is_set() {
            return Err(String::new());
        }
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    });
    match written {
        Ok(()) => super::SaveResult::Success {
            doc_id,
            path: path.to_path_buf(),
            at,
        },
        Err(_) if cancel.is_set() => {
            let _ = std::fs::remove_file(&temp);
            super::SaveResult::Stopped
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            super::SaveResult::Error {
                message: tr!("Write error: {e}", e = e),
            }
        }
    }
}

impl super::AppState {
    /// Read a pack again from the file it was saved to, whose rows have moved; another tab on that
    /// file with unsaved edits asks first.
    pub(super) fn reread_saved_pack(&mut self, doc_id: super::DocumentId, path: &Path) {
        let pack =
            (self.document(doc_id)).is_some_and(|d| matches!(d.payload, DocPayload::Ttpf(_)));
        if pack {
            if let Err(why) = self.reload_open_file_where(path, |id| id == doc_id) {
                self.set_error(tr!("Could not read the saved pack again"), why);
            }
        }
    }
}

/// The names a scanned body states a drawable by: its own, its fragment's, its dictionary's drawables'.
pub(super) fn stated_names(body: &Resource) -> Vec<&str> {
    match body {
        Resource::Drawable(d) => vec![d.name.as_str()],
        Resource::DrawableWithBound(r) => vec![r.name.as_str(), r.drawable.name.as_str()],
        Resource::Dictionary(dict) => (dict.entries.iter())
            .filter_map(|(_, e)| match e {
                Ok(Entry::Drawable(d)) => Some(d.name.as_str()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The names drawables state and the keys the texture naming rules may name from them as `payload` opens.
// A drawable's names meet its own keys and every open pack's; a pack's keys meet the names of
// every drawable open. None when either is empty.
pub(super) fn texture_naming_work(
    documents: &[Document],
    payload: &DocPayload,
) -> Option<(Vec<String>, HashSet<u64>)> {
    let packs = || {
        documents.iter().filter_map(|d| match &d.payload {
            DocPayload::Ttpf(pack) => Some(pack.keys.keys().copied()),
            _ => None,
        })
    };
    let (stems, keys): (Vec<&str>, HashSet<u64>) = match payload {
        DocPayload::Gta6Drawable(body) => {
            let drawables = super::gta6_resource::dictionary_drawables(body)
                .map(|(_, d)| d)
                .chain(super::gta6_resource::drawable_of(body, None));
            let bound = drawables
                .flat_map(|d| &d.shaders)
                .flat_map(|s| &s.texture_keys);
            let keys = bound.flatten().copied().chain(packs().flatten()).collect();
            (stated_names(body), keys)
        }
        DocPayload::Ttpf(pack) => {
            let stems = (documents.iter())
                .filter_map(|d| match &d.payload {
                    DocPayload::Gta6Drawable(body) => Some(stated_names(body)),
                    _ => None,
                })
                .flatten()
                .collect();
            (stems, pack.keys.keys().copied().collect())
        }
        _ => return None,
    };
    let stems: Vec<String> = stems.into_iter().map(str::to_string).collect();
    (!stems.is_empty() && !keys.is_empty()).then_some((stems, keys))
}

/// Keep the name the texture naming rules give a key of `keys` from a drawable's `name`; how many it named.
pub(super) fn learn_rule_names(name: &str, keys: &HashSet<u64>) -> usize {
    let found = name_keys([name], keys);
    learn_named(found.into_iter().map(|(key, (name, _))| (key, name)))
}

/// Keep each name of `found` for its key where no name is known for it; how many it named.
pub(super) fn learn_named(found: impl IntoIterator<Item = (u64, String)>) -> usize {
    let mut known = names();
    let mut named = 0;
    for (key, name) in found {
        if let std::collections::btree_map::Entry::Vacant(at) = known.entry(key) {
            at.insert(name);
            named += 1;
        }
    }
    named
}

/// Keep the name of each texture of `dict`, which a pack's or a drawable's 64-bit key may hash.
pub(super) fn learn_names(dict: &Dictionary) {
    let mut known = names();
    for (_, entry) in &dict.entries {
        if let Ok(Entry::Texture(t)) = entry {
            known.insert(name_hash64(t.name.as_bytes()), t.name.clone());
        }
    }
}

/// Why each texture of a scanned dictionary does not read, by the dictionary's path.
// ponytail: one list for the whole app, which no close empties.
static REFUSED: Mutex<BTreeMap<PathBuf, Vec<(String, String)>>> = Mutex::new(BTreeMap::new());

fn refused_lists() -> std::sync::MutexGuard<'static, BTreeMap<PathBuf, Vec<(String, String)>>> {
    REFUSED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why the texture `name` of the scanned dictionary `doc` is does not read; `None` for one that reads, or another document.
pub(crate) fn refused(doc: &Document, name: &str) -> Option<String> {
    let lists = refused_lists();
    let list = lists.get(doc.path.as_deref()?)?;
    (list.iter())
        .find(|(refused, _)| refused.eq_ignore_ascii_case(name))
        .map(|(_, why)| why.clone())
}

/// The engine's image type of a volume texture.
pub(super) const VOLUME_IMAGE: u8 = 3;

/// A scanned dictionary of textures, read from `path`, as a Legacy texture dictionary: each
/// texture that reads, and each that does not as its name with no pixels, its reason kept for
/// [`refused`]. `None` where an entry is no texture or none reads, which opens as the facts its
/// reader reads.
pub(super) fn scanned_textures(path: &Path, dict: Dictionary) -> Option<DocPayload> {
    learn_names(&dict);
    refused_lists().remove(path);
    let mut dictionary = PgDictionary::<GrcTexture>::default();
    let mut refused = Vec::new();
    for (code, entry) in dict.entries {
        let (placeholder, why) = match entry {
            Ok(Entry::Texture(t)) => {
                let read = t.texture.map_err(|e| e.to_string());
                match read.and_then(|texture| grc(t.name.clone(), texture)) {
                    Ok(mut texture) => {
                        // The engine's image types: a cube's slices are faces, six per cube, and
                        // a volume's z-slabs each hold part of every level.
                        texture.image_type = match t.dimension {
                            2 => 1,
                            3 => VOLUME_IMAGE,
                            _ => 0,
                        };
                        dictionary.insert(texture);
                        continue;
                    }
                    Err(why) => {
                        let (width, height) = (t.width, t.height);
                        let mip_count = t.mip_levels;
                        let name = t.name;
                        let texture = GrcTexture {
                            name,
                            width,
                            height,
                            depth: 1,
                            mip_count,
                            ..Default::default()
                        };
                        (texture, why)
                    }
                }
            }
            Ok(_) => return None,
            Err(e) => {
                let name = format!("hash_{code:08X}");
                (
                    GrcTexture {
                        name,
                        ..Default::default()
                    },
                    e.to_string(),
                )
            }
        };
        refused.push((placeholder.name.clone(), why));
        dictionary.insert(placeholder);
    }
    if refused.len() == dictionary.codes.len() {
        return None;
    }
    if !refused.is_empty() {
        refused_lists().insert(path.to_path_buf(), refused);
    }
    Some(DocPayload::Resource(or_io::Asset::TextureDict(dictionary)))
}

/// The slices a scanned dictionary's texture holds one after another: an array's, a cube's six faces, a volume's; 1 for any other document's.
pub(crate) fn scanned_slices(doc: &Document, texture: &GrcTexture) -> u32 {
    use or_io::codec::texture::TextureSource;
    let (width, height) = texture.stored_size();
    let one = (texture.pixel_format()).total_size(width, height, texture.stored_levels());
    match (
        doc.read_only,
        texture.backing_store.len() as u64 / u64::from(one.max(1)),
    ) {
        (Some(_), slices) if slices > 1 => u32::try_from(slices).unwrap_or(u32::MAX),
        _ => 1,
    }
}

/// The slice of each scanned dictionary's texture the preview shows, by the dictionary's path and the texture's name.
// ponytail: one map for the whole app, which no close empties; an entry is a path and a name.
static SHOWN: Mutex<BTreeMap<(PathBuf, String), u32>> = Mutex::new(BTreeMap::new());

fn shown() -> std::sync::MutexGuard<'static, BTreeMap<(PathBuf, String), u32>> {
    SHOWN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Have slice `slice` of the texture `name` of the scanned dictionary `doc` be the one a replacement takes.
pub(crate) fn show_scanned_slice(doc: &Document, name: &str, slice: u32) {
    if let Some(path) = doc.path.clone() {
        shown().insert((path, name.to_ascii_lowercase()), slice);
    }
}

/// The slice of that texture the preview shows: 0 until it shows another.
pub(crate) fn shown_scanned_slice(doc: &Document, name: &str) -> u32 {
    let Some(path) = doc.path.clone() else {
        return 0;
    };
    let key = (path, name.to_ascii_lowercase());
    shown().get(&key).copied().unwrap_or(0)
}

/// Slice `slice` of `slices` of such a texture, as a texture of one slice.
pub(crate) fn scanned_slice(mut texture: GrcTexture, slice: u32, slices: u32) -> GrcTexture {
    let each = texture.backing_store.len() / slices.max(1) as usize;
    let at = each * slice.min(slices.saturating_sub(1)) as usize;
    texture.backing_store = texture.backing_store[at..at + each].to_vec();
    texture
}

/// The texture of a scanned dictionary named `name`, whatever its case, where its pixels read.
fn texture_entry<'a>(resource: &'a Resource, name: &str) -> Option<&'a DictionaryTexture> {
    let Resource::Dictionary(dict) = resource else {
        return None;
    };
    dict.entries.iter().find_map(|(_, entry)| match entry {
        Ok(Entry::Texture(t)) if t.name.eq_ignore_ascii_case(name) && t.texture.is_ok() => Some(t),
        _ => None,
    })
}

/// What every open pack's edits come to, which moves when a texture of one is replaced, and when
/// the packs on disk are indexed: a key none of them holds is then no longer asked for.
pub(crate) fn packs_epoch(state: &super::AppState) -> u64 {
    let packs = state
        .documents
        .iter()
        .filter(|d| matches!(d.payload, DocPayload::Ttpf(_)));
    packs.fold(state.texture_packs_indexed(), |sum, d| {
        sum.wrapping_add(d.id.wrapping_mul(31).wrapping_add(d.payload_epoch()))
    })
}

/// Whether a pack or a scanned dictionary `payload` is holds a texture `name` names, asked for by
/// a GTA VI document `asker`; `false` for any other document or asker.
pub(super) fn holds(asker: Option<&Document>, payload: &DocPayload, name: &str) -> bool {
    // ⚠ A GTA V model's texture is never a GTA VI one, whatever its name hashes to.
    let gta6 = is_gta6(asker);
    match payload {
        DocPayload::Ttpf(pack) if gta6 => pack.index_of(name).is_some(),
        DocPayload::Gta6Drawable(body) if gta6 => texture_entry(body, name).is_some(),
        _ => false,
    }
}

/// Whether `doc` is a GTA VI document: a pack, a scanned body or a GTA VI animation.
pub(super) fn is_gta6(doc: Option<&Document>) -> bool {
    doc.is_some_and(|doc| {
        doc.read_only == Some(super::rpf9_scan::READ_ONLY_BODY)
            || matches!(
                doc.payload,
                DocPayload::Ttpf(_) | DocPayload::Gta6Drawable(_) | DocPayload::Gta6Anim(_)
            )
    })
}

/// That texture, a scanned dictionary's read from it each time it is asked for.
pub(super) fn fetch<'a>(payload: &'a DocPayload, name: &str) -> Option<Texture<'a>> {
    match payload {
        DocPayload::Ttpf(_) => textures::rows::at::ttpf(payload, TextureSource::Document, name),
        DocPayload::Gta6Drawable(body) => {
            let t = texture_entry(body, name)?;
            let texture = grc(t.name.clone(), t.texture.as_ref().ok()?.clone()).ok()?;
            Some(Texture::Owned(std::borrow::Cow::Owned(texture)))
        }
        _ => None,
    }
}

pub(super) fn grc(name: String, texture: or_io::gta6::Texture) -> Result<GrcTexture, String> {
    let format = texture.format.format();
    let (Ok(width), Ok(height), Ok(mip_count)) = (
        u16::try_from(texture.width),
        u16::try_from(texture.height),
        u8::try_from(texture.levels),
    ) else {
        return Err(format!(
            "{}x{} with {} mips does not fit a texture",
            texture.width, texture.height, texture.levels
        ));
    };
    Ok(GrcTexture {
        name,
        width,
        height,
        depth: 1,
        mip_stride: u16::try_from(format.row_pitch(texture.width)).unwrap_or(0),
        format: format.to_raw_format(),
        mip_count,
        info_bits: u8::from(texture.format.is_srgb()),
        backing_store: texture.pixels,
        resource_type_and_conversion_flags: or_io::gta5::texture_dict::gen8::PROCESSED,
        ..Default::default()
    })
}

#[path = "gta6_bcpack_calibration.rs"]
pub(crate) mod calibration;

#[path = "ttpf_reshape.rs"]
mod relaid;
pub(super) use relaid::{import, reshape};

#[cfg(test)]
mod tests {
    use or_io::gta6::bcpack::calibrate::{keep_check, keep_supplied, load_cache};
    use or_io::gta6::bcpack::made_up::{bc7_body, bc7_entries_swapped, tables_file};
    use or_io::gta6::bcpack::Checked;

    use super::*;
    use crate::state::opening_a_ttpf::{pack_of, BODY};

    /// A pack written under `folder` as `made_up.ttpf`, opened.
    fn opened(folder: &Path, bytes: &[u8]) -> TtpfDocument {
        std::fs::create_dir_all(folder).expect("a folder");
        let path = folder.join("made_up.ttpf");
        std::fs::write(&path, bytes).expect("written");
        TtpfDocument::open(&path).expect("the pack opens")
    }

    #[test]
    fn a_texture_that_counts_blocks_against_the_supplied_tables_fails_them_for_its_pack() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        crate::state::prefs::keep_prefs_in(dir.path());
        let kept = calibration_dir().expect("a configuration folder");
        // The tables supplied are a made-up file with the first two entries of BC7's list
        // changed over, which a whole check of other packs has held.
        keep_supplied(&kept, &bc7_entries_swapped(&tables_file())).expect("kept");
        let tables = load_cache(&kept, 0).expect("the file reads");
        let held = Checked {
            blocks: 512,
            disagree: 0,
        };
        keep_check(&kept, &tables, &[(7, held)]).expect("kept");

        // Two textures of one body each through the coder's BC7 mode: texture 0 reads the two
        // entries, texture 1 neither.
        let blocks = BODY / 16;
        // A row starts at a 256-byte boundary, and a coded stream may end in zeros.
        let body = |from_presets: usize| {
            let mut stream = bc7_body(blocks, from_presets);
            stream.resize(stream.len().next_multiple_of(0x100), 0);
            stream
        };
        let bytes = pack_of(&[(0x238, 7, body(2)), (0x238, 7, body(0))]);
        let pack = opened(&dir.path().join("here"), &bytes);
        assert!(pack.supplied());
        let not_the_coders = Some(tr!("not this coder's tables").to_string());

        // Read alone, texture 1 is exact: nothing of it speaks against the file, and a check
        // held BC7.
        pack.texture(1).expect("reads");
        assert_eq!(pack.decode_label(1), Some(tr!("exact").to_string()));
        assert_eq!(pack.doubt_label(1), None);

        // Texture 0 counts its two blocks against the file. Texture 1 went through the same
        // file, and is not called exact from then on.
        pack.texture(0).expect("reads");
        let two = tr!(
            "{count} of {blocks} coded blocks disagree with the supplied tables",
            count = 2,
            blocks = blocks
        );
        assert_eq!(pack.decode_label(0), Some(two));
        assert_eq!(pack.decode_label(1), not_the_coders);
        assert_eq!(pack.doubt_label(1), not_the_coders);
        let warning = tr!(
            "The tables you supplied disagree with this pack: {count} of the coded blocks read so far read a value no table gives or hold what no encoder writes",
            count = 2
        );
        assert_eq!(pack.tables_note(), Some(warning.clone()));
        assert_eq!(pack.decode_basis(1), std::slice::from_ref(&warning));

        // That is kept with the pack, whose key is its file's name and length: the same pack
        // opened from another folder says it of a texture before any is read.
        let again = opened(&dir.path().join("there"), &bytes);
        assert_eq!(again.decode_label(1), not_the_coders);
        assert_eq!(again.tables_note(), Some(warning));
        // Another pack reads as the check left the file.
        let other = pack_of(&[(0x238, 7, body(0)), (0x238, 0, vec![0; BODY])]);
        let other = opened(&dir.path().join("other"), &other);
        other.texture(0).expect("reads");
        assert_eq!(other.decode_label(0), Some(tr!("exact").to_string()));
    }

    #[test]
    fn a_texture_a_coded_body_of_which_was_not_read_is_not_called_exact() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let bytes = pack_of(&[(0x238, 5, vec![0x31; 0x200])]);
        let mut pack = opened(dir.path(), &bytes);
        pack.pack.set_calibration(None);
        let path = pack.pack.path().to_path_buf();
        // What a read that stopped at a coded body counts: the levels above it, and that body.
        let partly = Decode {
            unread: 1,
            ..Decode::default()
        };
        decodes().insert((path.clone(), 0), partly);
        let plain = Some(tr!("plain levels read, coded levels not read").to_string());
        assert_eq!(pack.decode_label(0), plain);
        assert_eq!(pack.doubt_label(0), plain);
        assert_eq!(pack.estimate_label(0), None);
        assert_eq!(pack.decode_basis(0).len(), 1);

        // A read that rebuilt coded levels and then met a body that did not read.
        let above = Decode {
            supplied: 64,
            kinds: 1 << 1,
            ..partly
        };
        decodes().insert((path.clone(), 0), above);
        let above = tr!("a coded body not read, the levels above it read").to_string();
        assert_eq!(pack.decode_label(0), Some(above));

        // A read through BC1's built-in tables alone is not called exact either, and the
        // texture view and an export say so as the list and the inspector do.
        let built_in = Decode {
            unseconded: 64,
            ..Decode::default()
        };
        decodes().insert((path, 0), built_in);
        let rests = Some(tr!("built-in BC1 tables, not confirmed").to_string());
        assert_eq!(pack.decode_label(0), rests);
        assert_eq!(pack.doubt_label(0), rests);
    }
}
