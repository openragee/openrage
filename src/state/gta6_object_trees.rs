//! A script archive's table of object trees, opened as the lines of what its reader reads.

use std::collections::BTreeSet;

use or_io::gta6::headerless::read_object_trees;

/// One line for the table, then one for each tree: its root's key and type, and what it points at.
pub(super) fn lines(bytes: &[u8]) -> Result<Vec<String>, String> {
    let trees = read_object_trees(bytes).map_err(|e| {
        tr!(
            "GTA VI's object-tree reader does not read this body: {e}",
            e = e
        )
    })?;
    let types: BTreeSet<u32> = trees.items.iter().map(|i| i.type_tag).collect();
    let mut lines = vec![tr!(
        "A GTA VI table of {items} objects in {trees} trees, of {types} types. Each tree's root is keyed by the 32-bit name hash of a model's name, and an object points at later objects of its tree by their offsets. What the objects' fields hold and what the type tags name are not known.",
        items = trees.items.len(),
        trees = trees.roots.len(),
        types = types.len(),
    )];
    for tree in trees.trees() {
        let [root, ..] = tree else { continue };
        let children: Vec<String> = (root.children.iter())
            .filter_map(|c| trees.items.get(*c))
            .map(|c| format!("{:#010X} ({} B)", c.type_tag, c.size))
            .collect();
        lines.push(tr_n!(
            "{key}: a root of type {tag}, alone in its tree; it points at [{children}]",
            "{key}: a root of type {tag} and {count} objects in its tree; it points at [{children}]",
            tree.len(),
            key = super::gta6_names::said32(root.key),
            tag = format!("{:#010X}", root.type_tag),
            children = children.join(", "),
        ));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_the_reader_refuses_says_why_and_a_table_lists_one_line_a_tree() {
        let why = lines(&[0; 0x40]).unwrap_err();
        assert!(why.contains("object-tree"), "{why}");
        // A root keyed 0x0BADF00D of type 0xA1 over a child of type 0xB2, then a root of type 0xC3
        // keyed by the hash of a name the public names hold.
        let mut body = vec![0u8; 0xA0];
        body[0x14] = 3;
        for (i, at) in [0x40u64, 0x60, 0x80].iter().enumerate() {
            body[0x28 + 8 * i..0x30 + 8 * i].copy_from_slice(&at.to_le_bytes());
        }
        body[0x40..0x44].copy_from_slice(&0x0BAD_F00Du32.to_le_bytes());
        body[0x48] = 0xA1;
        let to_child = (u64::from(or_io::gta6::headerless::OBJECT_POINTER_TAG) << 32) | 0x68;
        body[0x50..0x58].copy_from_slice(&to_child.to_le_bytes());
        body[0x68] = 0xB2;
        let named = or_io::hash::literal_string_hash(b"CMapData");
        body[0x80..0x84].copy_from_slice(&named.to_le_bytes());
        body[0x88] = 0xC3;
        let read = lines(&body).expect("the trees");
        assert_eq!(read.len(), 3);
        assert!(read[1].starts_with("hash_0BADF00D"), "{}", read[1]);
        assert!(read[1].contains("0x000000B2 (32 B)"), "{}", read[1]);
        assert!(read[2].starts_with("CMapData: "), "{}", read[2]);
    }
}
