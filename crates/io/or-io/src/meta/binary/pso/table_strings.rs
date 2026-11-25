//! A GTA VI table-form PSO saved with the texts its tree adds: a new text goes to the enciphered
//! `LSNS` run its member's kind names (`0x102` for a `StringHash`, `0x101 + byte 9` for an
//! `NsStringHash`, where that run is registered), enciphered again with the key snapshot, and the
//! `LSNS` frame takes the new length. Only a run the document holds takes a text.
//!
//! ⚠ A structural result: an archive holding such a document is refused by the game, which checks
//! a signature over the archive's table at mount that only its maker can make.

use crate::container::rpf::GtaViKeys;
use crate::meta::binary::layout::{ContainerLayout, MetaLayout};
use crate::meta::binary::rbm::schema::RbmModel;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::MetaInstance;

use super::sections::RawPsoSection;
use super::string_blob;
use super::table_layout::{free_namespace, texts_to_add, Held};

const LSNS: [u8; 4] = *b"LSNS";

fn strings_bad(why: impl Into<String>) -> MetaError {
    MetaError::Other(format!("GTA VI PSO: {}", why.into()))
}

/// The table-form PSO `layout` was read with, holding `instance` and the texts it adds to `LSNS`.
pub fn write_adding_strings(
    layout: &MetaLayout,
    instance: &MetaInstance,
    keys: Option<&GtaViKeys>,
) -> Result<Vec<u8>> {
    match &layout.container {
        ContainerLayout::Pso(pso) if pso.is_table_form() => {
            super::table_layout::write_with_keys(pso, instance, keys)
        }
        _ => Err(strings_bad(
            "only a table-form document takes texts into LSNS",
        )),
    }
}

/// Adds to `sections`' `LSNS` the texts `instance` holds and its registered runs do not, and to
/// `held` what a layout may then hold; `true` when `LSNS` grew. A text to add with no `keys` is
/// refused by name.
pub(super) fn add_texts(
    sections: &mut [RawPsoSection],
    model: &RbmModel,
    instance: &MetaInstance,
    held: &mut Held,
    keys: Option<&GtaViKeys>,
) -> Result<bool> {
    let Some(lsns) = (sections.iter_mut()).find(|s| s.ident.ident().to_be_bytes() == LSNS) else {
        return Ok(false);
    };
    // An `LSNS` that does not parse frees no namespace and takes no text: the save is today's.
    let Some(ids) = (lsns.bytes.get(8..))
        .and_then(|body| string_blob::parse(body).ok())
        .map(|runs| runs.iter().map(|r| r.id).collect::<Vec<u32>>())
    else {
        return Ok(false);
    };
    // A namespace with no run of any kind holds no text, so its 64-bit hashes are free to add.
    let free = (0..=u8::MAX)
        .filter(|&ns| !ids.iter().any(|id| id & 0xFF == (u32::from(ns) + 1) & 0xFF))
        .map(free_namespace);
    held.extend(free);
    let registered: Vec<u32> = (ids.into_iter())
        .filter(|&id| string_blob::is_enciphered(id))
        .collect();
    if registered.is_empty() {
        return Ok(false);
    }
    let (added, laid) = texts_to_add(model, instance, held, &registered)?;
    let Some((run, text)) = added.first() else {
        return Ok(false);
    };
    let keys = keys.ok_or_else(|| {
        strings_bad(format!(
            "the tree adds the text \"{text}\" to the enciphered LSNS run {run:#X}, and no key snapshot was given to encipher it"
        ))
    })?;
    let mut body = lsns.bytes[8..].to_vec();
    for (run, text) in &added {
        body = string_blob::append_string(&body, *run, text, keys)?;
    }
    held.extend(laid);
    if body == lsns.bytes[8..] {
        return Ok(false);
    }
    let len = u32::try_from(body.len() + 8)
        .map_err(|_| strings_bad("LSNS outgrows the 4 GiB its frame states"))?;
    let grew = body.len() + 8 - lsns.bytes.len();
    lsns.bytes = [&LSNS[..], &len.to_be_bytes()[..], &body[..]].concat();
    // `CHKS` states the document's length at its +8, which the stored document grows by too.
    let chks = (sections.iter_mut()).find(|s| s.ident.ident().to_be_bytes() == *b"CHKS");
    if let Some(total) = chks.and_then(|s| s.bytes.get_mut(8..12)) {
        let was = u32::from_be_bytes([total[0], total[1], total[2], total[3]]);
        let now = u32::try_from(grew)
            .ok()
            .and_then(|grew| was.checked_add(grew))
            .ok_or_else(|| strings_bad("the document outgrows the 4 GiB its CHKS states"))?;
        total.copy_from_slice(&now.to_be_bytes());
    }
    Ok(true)
}

#[cfg(test)]
#[path = "table_strings_tests.rs"]
mod tests;
