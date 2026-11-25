//! A model's palette given the bones a geometry's new blend indices name: a list of bone hashes
//! takes each hash it lacks in its order, as a new list at the end of the virtual section, and
//! every blend index of the model's geometries follows its bone; a bone mask takes each bone's bit
//! in place.

use super::bytes::{sections, u32_at, u64_at};
use super::dictionary::entry_of;
use super::drawable::{drawable_at, Model, Palette};
use super::write_geometry::{write_geometry, GeometryAt, GeometryEdit, GeometryWriteError};
use super::write_material::based;
use super::write_physics::put;
use super::HeaderlessError;
use crate::container::rpf::{grow_section, Rpf9Role, Rpf9Section, Rpf9Sections};

const VIRTUAL_TAG: u64 = 0x5000_0000;
/// The offset a pointer word holds below its section nibble.
const TARGET: u64 = 0x0FFF_FFFF;

type Written<T> = std::result::Result<T, GeometryWriteError>;

use GeometryWriteError::Refused;

/// The bones a model's palette takes: hashes for a list of bone hashes, own-skeleton bone numbers for a mask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaletteAdd {
    pub hashes: Vec<u32>,
    pub bones: Vec<u16>,
}

impl PaletteAdd {
    /// Whether it adds no bone.
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty() && self.bones.is_empty()
    }
}

/// Model `model` of level `level` as read, where it lies, and where each of its geometries lies.
fn located(
    s: &Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    level: usize,
    model: usize,
) -> Result<(Model, usize, Vec<usize>), HeaderlessError> {
    sections(&s.virtual_section, Some(&s.physical), |v, p| {
        let (root, bound_at_c8) = match (role, code) {
            (Rpf9Role::Drawable, None) => (0, true),
            (Rpf9Role::DrawableWithBound, None) => (v.to("drawable", 0x18)?, false),
            (Rpf9Role::Dictionary, Some(code)) => {
                let entry = entry_of(v, code)?.ok_or(HeaderlessError::NotInPlace(
                    "no entry of the dictionary has that code",
                ))?;
                (entry, true)
            }
            _ => {
                return Err(HeaderlessError::NotInPlace(
                    "the resource holds no drawable there",
                ))
            }
        };
        let mut drawable = drawable_at(v, p, root, bound_at_c8)?;
        let past = |what, index: usize, count: usize| HeaderlessError::IndexPastCount {
            what,
            index: index as u64,
            count: count as u64,
        };
        let mut models = (drawable.levels.get_mut(level))
            .and_then(Option::take)
            .ok_or(past("level of detail", level, 4))?;
        if model >= models.len() {
            return Err(past("model", model, models.len()));
        }
        let read = models.swap_remove(model);
        let level_at = v.to("level", root + 0x50 + 8 * level)?;
        let at = v.to("model", v.to("models", level_at)? + 8 * model)?;
        let geometries = match read.geometries.len() {
            0 => Vec::new(),
            n => {
                let list = v.to("geometries", at + 0x08)?;
                (0..n)
                    .map(|j| v.to("geometry", list + 8 * j))
                    .collect::<Result<Vec<usize>, HeaderlessError>>()?
            }
        };
        Ok((read, at, geometries))
    })
}

/// The palette count the reader takes: the model's byte, or geometry 0's u16 at +0x2C where it
/// passes 0xFF and keeps the byte as its low 8 bits; the u16 at +0x2E is another word.
fn count_of(v: &[u8], model: usize, geometries: &[usize]) -> Written<usize> {
    let byte = *(v.get(model + 0x30)).ok_or(Refused("its model lies past the virtual section"))?;
    let whole = (geometries.first())
        .and_then(|g| v.get(g + 0x2C..g + 0x2E))
        .map(|b| u16::from_le_bytes([b[0], b[1]]));
    Ok(whole
        .filter(|&n| n > 0xFF && n & 0xFF == u16::from(byte))
        .map_or(usize::from(byte), |n| n as usize))
}

/// Where the list the model's +0x28 points at lies; `None` for a null pointer.
fn list_of(v: &[u8], model: usize) -> Written<Option<usize>> {
    let word = (v.get(model + 0x28..model + 0x30))
        .map(|b| u64_at(b, 0))
        .ok_or(Refused("its model lies past the virtual section"))?;
    match word {
        0 => Ok(None),
        w if w >> 28 == VIRTUAL_TAG >> 28 => Ok(Some((w & TARGET) as usize)),
        _ => Err(Refused("its model's palette pointer is not a virtual one")),
    }
}

/// The mask of the model at `model` with the bit of each bone of `bones` set.
fn set_bits(v: &mut [u8], model: usize, geometries: &[usize], bones: &[u16]) -> Written<()> {
    let count = count_of(v, model, geometries)?;
    let list = (list_of(v, model)?.filter(|_| count > 0)).ok_or(Refused(
        "its model holds no bone mask, so a bone none of its geometries uses is not added",
    ))?;
    for &bone in bones {
        let b = usize::from(bone);
        if b >= count {
            return Err(Refused("a bone at or past its mask's count is not added"));
        }
        let at = list + 4 * (b / 32);
        let word = (v.get(at..at + 4))
            .map(|w| u32_at(w, 0))
            .ok_or(Refused("its bone mask lies past the virtual section"))?;
        put(v, at, &(word | 1 << (b % 32)).to_le_bytes())?;
    }
    Ok(())
}

/// `held` with each hash of `add` it lacks, in order where `held` ascends and after it where it
/// does not; and the index each held entry moves to.
fn inserted(held: &[u32], add: &[u32]) -> Written<(Vec<u32>, Vec<u16>)> {
    let ascending = held.windows(2).all(|w| w[0] < w[1]);
    let mut list = held.to_vec();
    for &hash in add {
        if hash == 0 {
            return Err(Refused("a bone hash of 0 names no bone"));
        }
        if list.contains(&hash) {
            continue;
        }
        match ascending {
            true => list.insert(list.partition_point(|h| *h < hash), hash),
            false => list.push(hash),
        }
    }
    let moved = (held.iter())
        .map(|h| (list.iter().position(|x| x == h)).and_then(|at| u16::try_from(at).ok()))
        .collect::<Option<Vec<u16>>>()
        .ok_or(Refused(
            "the palette would pass the 65,536 entries a blend index names",
        ))?;
    Ok((list, moved))
}

/// Each blend index of `read`'s geometries moved as `moved` says, each written at its own count.
fn renumber(
    s: &mut Rpf9Sections,
    (role, code, level, model): (Rpf9Role, Option<u32>, usize, usize),
    read: &Model,
    moved: &[u16],
) -> Written<()> {
    if (moved.iter().enumerate()).all(|(k, to)| usize::from(*to) == k) {
        return Ok(());
    }
    for (j, g) in read.geometries.iter().enumerate() {
        let g = g.as_ref().map_err(|_| {
            Refused(
                "a geometry of its model does not read, so its blend indices are not renumbered",
            )
        })?;
        let unread = (g.channels.iter())
            .filter(|c| matches!(c.index, 20 | 21))
            .any(|c| !g.vertices.joints.iter().any(|(k, _)| *k == c.index));
        if unread {
            return Err(Refused(
                "a geometry of its model holds blend indices in a form not read, so they are not \
                 renumbered",
            ));
        }
        if g.vertices.joints.is_empty() {
            continue;
        }
        let positions = g.positions.as_ref().map_err(|_| {
            Refused("a geometry of its model has positions that do not read, so its blend indices are not renumbered")
        })?;
        let to = |k: u16| moved.get(usize::from(k)).copied().unwrap_or(k);
        let joints: Vec<(u8, Vec<[u16; 4]>)> = (g.vertices.joints.iter())
            .map(|(c, lanes)| (*c, lanes.iter().map(|l| l.map(to)).collect()))
            .collect();
        let edit = GeometryEdit {
            positions: positions.clone(),
            triangles: g.triangles.clone(),
            joints,
            ..GeometryEdit::default()
        };
        let at = GeometryAt {
            level,
            model,
            geometry: j,
        };
        write_geometry(s, role, code, at, &edit)?;
    }
    Ok(())
}

/// Model `model` of level `level` of the drawable of `role` (or dictionary entry `code`) given the bones of `add`.
// A refusal leaves the sections as they were; the palette is read back before it is kept.
pub fn write_palette(
    sections: &mut Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    level: usize,
    model: usize,
    add: &PaletteAdd,
) -> Written<()> {
    let (read, at, geometries) = located(sections, role, code, level, model)?;
    let palette = (read.palette.as_ref())
        .map_err(|_| Refused("its model's palette is in neither measured form"))?;
    let mut out = sections.clone();
    let wanted = match palette {
        Palette::Own => {
            if !add.hashes.is_empty() {
                return Err(Refused(
                    "its model's palette is a bone mask, which takes no bone hash",
                ));
            }
            set_bits(&mut out.virtual_section, at, &geometries, &add.bones)?;
            Palette::Own
        }
        Palette::Hashes(held) => {
            if !add.bones.is_empty() {
                return Err(Refused(
                    "its model's palette lists bone hashes, which take no bone by its number",
                ));
            }
            let (list, moved) = inserted(held, &add.hashes)?;
            if list.len() != held.len() {
                let bytes: Vec<u8> = list.iter().flat_map(|h| h.to_le_bytes()).collect();
                let end = out.virtual_section.len();
                let pointers = based(&out);
                let by = bytes.len().next_multiple_of(16) as isize;
                let start = grow_section(&mut out, &pointers, Rpf9Section::Virtual, end, by)?;
                // The block map's slots may have grown before the model, so it is found again.
                let (_, at, geometries) = located(&out, role, code, level, model)?;
                let v = &mut out.virtual_section;
                put(v, start, &bytes)?;
                put(v, at + 0x28, &(VIRTUAL_TAG + start as u64).to_le_bytes())?;
                // The model's byte keeps the count's low 8 bits; a geometry's u16 at +0x2C states it whole.
                put(v, at + 0x30, &[list.len() as u8])?;
                let new = u16::try_from(list.len())
                    .map_err(|_| Refused("a palette passes 65,535 entries"))?;
                let old = held.len() as u16;
                for g in &geometries {
                    if v.get(g + 0x2C..g + 0x2E) == Some(&old.to_le_bytes()[..]) {
                        put(v, g + 0x2C, &new.to_le_bytes())?;
                    }
                }
                renumber(&mut out, (role, code, level, model), &read, &moved)?;
            }
            Palette::Hashes(list)
        }
    };
    let (back, ..) = located(&out, role, code, level, model)?;
    if back.palette.ok() != Some(wanted) {
        return Err(Refused("its model's palette does not read back as written"));
    }
    *sections = out;
    Ok(())
}
