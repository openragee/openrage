//! What `rpf scan-open` prints of a script archive's bodies that are no document: the object
//! trees' roots and types, and the opaque bodies after a first entry, stated and never read.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Scan};
use or_io::gta6::headerless::read_object_trees;
use or_io::gta6::script_bodies::{opaque_bodies, NotOpaque, Placement};

pub(crate) fn print_object_trees(bytes: &[u8]) -> Result<(), String> {
    let trees = read_object_trees(bytes)
        .map_err(|e| format!("the object-tree reader does not read this body: {e}"))?;
    let types: BTreeSet<u32> = trees.items.iter().map(|i| i.type_tag).collect();
    println!(
        "  object trees  {} items in {} trees, {} item types; an item points at later items of \
         its tree by their offsets",
        trees.items.len(),
        trees.roots.len(),
        types.len()
    );
    println!("  root key      the 32-bit name hash of a model's name");
    println!("  not read      what the items' fields hold, and what the type tags name");
    for tree in trees.trees() {
        let [root, ..] = tree else { continue };
        let children: Vec<String> = (root.children.iter())
            .filter_map(|c| trees.items.get(*c))
            .map(|c| format!("{:#010X} ({} bytes)", c.type_tag, c.size))
            .collect();
        println!(
            "    hash_{:08X}  type {:#010X}, {} items; the root points at [{}]",
            root.key,
            root.type_tag,
            tree.len(),
            children.join(", ")
        );
    }
    Ok(())
}

/// The bodies from the first entry at `offset` of `file` up to the next row a scan lists.
pub(crate) fn print_span(file: &fs::File, offset: u64, limit: u64) -> Result<(), String> {
    let mut names_len = None;
    let mut end = None;
    for find in Rpf9Scan::new(file, 0..u64::MAX).map_err(|e| e.to_string())? {
        let find = find.map_err(|e| e.to_string())?;
        if find.offset > offset {
            end = Some(find.offset);
            break;
        }
        if let Rpf9BodyKind::Archive(header) = find.kind {
            names_len = Some(u64::from(header.names_size));
        }
    }
    let end = match end {
        Some(end) => end,
        None => file.metadata().map_err(|e| e.to_string())?.len(),
    };
    let len = end.saturating_sub(offset);
    if len > limit {
        return Err(format!(
            "the {len} bytes from {offset:#X} to the next row pass the limit of {limit}; \
             --limit raises it"
        ));
    }
    let mut span = vec![0; len as usize];
    let mut reader = file;
    reader
        .seek(SeekFrom::Start(offset))
        .and_then(|_| reader.read_exact(&mut span))
        .map_err(|e| format!("read at {offset:#X}: {e}"))?;
    let found = opaque_bodies(&span, offset, names_len).map_err(|e| refusal(&e, offset, end))?;
    println!(
        "  opaque        {} bodies from {offset:#X} to {end:#X}, the next row: not read",
        found.bodies.len()
    );
    for body in &found.bodies {
        let placement = match body.placement {
            Placement::Page => "page + 0",
            Placement::AfterZeroSlot => "page + 0x10",
            Placement::Unpaged => "unpaged",
        };
        let pieces: Vec<String> = (body.pieces.iter())
            .map(|p| match p.zlib_head {
                Some(head) => format!("{:#X}, {} bytes, a zlib head at {head:#X}", p.offset, p.len),
                None => format!("{:#X}, {} bytes", p.offset, p.len),
            })
            .collect();
        // A body behind a zero slot is the coder's; any other is only known to hold no structure.
        let named = match body.coder {
            Some(entry) => format!(
                "an entry of the script coder nothing decodes, tag {:#06X}",
                entry.tag
            ),
            None => "ciphertext-like".to_string(),
        };
        println!(
            "    {:#012X}  {placement}; {named}; pieces: {}",
            body.offset,
            pieces.join("; ")
        );
    }
    if let Some(at) = found.names {
        println!("  names         at {at:#X}, as long as the header before states: encrypted");
    }
    Ok(())
}

/// `e`, a refusal of the bytes from the first entry at `offset` to the next row at `end`, saying
/// so where it names a later body of them.
fn refusal(e: &NotOpaque, offset: u64, end: u64) -> String {
    match e.at == offset {
        true => e.to_string(),
        false => format!(
            "{e}; {:#X} is a later body of the bytes from the first entry at {offset:#X} to the \
             next row at {end:#X}, which are not read",
            e.at
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_of_a_later_body_names_the_first_entry_it_was_asked_of() {
        let at = |at| NotOpaque {
            rule: "O2",
            at,
            why: "zlib shrinks its first piece below 0.98 of its length",
        };
        let own = at(0x1000);
        assert_eq!(refusal(&own, 0x1000, 0x9000), own.to_string());
        let later = refusal(&at(0x5000), 0x1000, 0x9000);
        assert!(later.starts_with(&at(0x5000).to_string()), "{later}");
        for offset in ["0x5000", "0x1000", "0x9000"] {
            assert!(later.contains(offset), "{offset}: {later}");
        }
    }
}
