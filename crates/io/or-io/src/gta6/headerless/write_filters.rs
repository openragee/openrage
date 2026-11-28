//! A dictionary's frame filter weights and animation record values written back in place, over the
//! entries the reader read; every other word of an entry is kept as it was read, but for a filter's
//! signature, which a changed weight writes anew.

use super::bytes::{sections, u32_at};
use super::dictionary::entry_of;
use super::{BoneRecord, FilterTrack, HeaderlessError, Result};

const TRACK_LEN: usize = 48;
const BONE_LEN: usize = 144;

fn refuse(why: &'static str) -> HeaderlessError {
    HeaderlessError::NotInPlace(why)
}

/// The weights of the frame filter of `code` written from `tracks`, which name the ones it holds.
pub fn write_filter_weights(
    virtual_section: &mut [u8],
    code: u32,
    tracks: &[FilterTrack],
) -> Result<()> {
    let first = sections(virtual_section, None, |v, _| {
        let at = entry_of(v, code)?.ok_or(refuse("no entry of the dictionary has that code"))?;
        let is_filter = v.u64("frame filter", at)? == 0
            && v.u32("frame filter", at + 0x08)? == 1
            && v.u32("frame filter", at + 0x10)? == 14
            && v.ptr("frame filter name", at + 0x18)?.is_some();
        if !is_filter {
            return Err(refuse("the entry of that code is no frame filter"));
        }
        let count = usize::from(v.u16("frame filter track count", at + 0x30)?);
        if count != tracks.len() {
            return Err(refuse(
                "the edit holds another number of tracks than the filter",
            ));
        }
        if count == 0 {
            return Ok(0);
        }
        let first = v.to("frame filter tracks", at + 0x28)?;
        let read = v.array("frame filter tracks", first, count, TRACK_LEN)?;
        for (held, edited) in read.as_chunks::<TRACK_LEN>().0.iter().zip(tracks) {
            let word = |o: usize| u32_at(held, o);
            if (word(0), word(0x28), word(0x2C)) != (edited.track, edited.group, edited.word_2c) {
                return Err(refuse(
                    "a track of the edit names another track, group or word at 0x2C than the filter's",
                ));
            }
        }
        Ok(first)
    })?;
    let mut changed = false;
    for (i, t) in tracks.iter().enumerate() {
        let at = first + TRACK_LEN * i + 0x04;
        changed |= virtual_section[at..at + 4] != t.weight.to_le_bytes();
        virtual_section[at..at + 4].copy_from_slice(&t.weight.to_le_bytes());
    }
    // A weight is an input of the filter's signature.
    if changed {
        super::write_filter_tracks::sign(virtual_section, code)?;
    }
    Ok(())
}

/// The values of the animation record of `code` written from `bones`, which name the ones it holds.
pub fn write_record_values(
    virtual_section: &mut [u8],
    code: u32,
    bones: &[BoneRecord],
) -> Result<()> {
    let first = sections(virtual_section, None, |v, _| {
        let at = entry_of(v, code)?.ok_or(refuse("no entry of the dictionary has that code"))?;
        let is_record = v.u64("animation record", at)? == 0
            && v.u64("animation record", at + 0x08)? == 0
            && v.u32("animation record", at + 0x10)? == 1
            && v.ptr("animation record bones", at + 0x20)?.is_some();
        if !is_record {
            return Err(refuse("the entry of that code is no animation record"));
        }
        let count = v.u32("animation record count", at + 0x18)? as usize;
        if count != bones.len() {
            return Err(refuse(
                "the edit holds another number of bones than the record",
            ));
        }
        let first = v.to("animation record bones", at + 0x20)?;
        let read = v.array("animation record bones", first, count, BONE_LEN)?;
        for (held, edited) in read.as_chunks::<BONE_LEN>().0.iter().zip(bones) {
            let word = |o: usize| u32_at(held, o);
            if (word(0x80), word(0x78), word(0x7C)) != (edited.bone, edited.kind, edited.flags) {
                return Err(refuse(
                    "a bone of the edit names another bone, kind or flags than the record's",
                ));
            }
        }
        Ok(first)
    })?;
    for (i, b) in bones.iter().enumerate() {
        for (f, value) in b.values.iter().enumerate() {
            let at = first + BONE_LEN * i + 0x0C + 4 * f;
            virtual_section[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(())
}
