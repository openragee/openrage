//! What an RPF9 entry is, as its row says: a 24-byte row by its class byte, a 20-byte row by its
//! resource type code. A byte or code no table names prints as its hex.

use std::borrow::Cow;

use super::gta6_archive::Gta6Archive;

/// The kind of a 24-byte row whose class byte says only that it holds a file.
pub const PLAIN_FILE: &str = "file";

/// The kinds a 24-byte row's class byte names.
pub const CLASSES: [(u8, &str); 10] = [
    (0x00, "archive"),
    (0x0B, "resource"),
    (0x40, "MoVE network"),
    (0x42, "GFX"),
    (0x5D, "PSO"),
    (0x5E, "PSO"),
    // Its body is an offset table of object trees, `OtherBody::ObjectTrees`.
    (0x60, "object trees"),
    (0x68, "span grid"),
    (0x69, "height map"),
    // Its rows hold XML, PSO and movies alike.
    (0xFF, PLAIN_FILE),
];

/// The kinds a 20-byte row's type code names, one per code of the headerless version table.
pub const TYPES: [(u32, &str); 36] = [
    (0x02, "drawable"),
    (0x03, "fragment"),
    (0x04, "drawable dictionary"),
    (0x05, "texture dictionary"),
    (0x06, "bound"),
    (0x0B, "headerless metadata"),
    (0x0F, "acceleration structure"),
    (0x10, "acceleration dictionary"),
    (0x11, "acceleration structure"),
    (0x12, "resource 0x12"),
    (0x43, "clip dictionary"),
    (0x45, "resource 0x45"),
    (0x46, "animation"),
    (0x48, "particles"),
    (0x49, "map data"),
    (0x4A, "map types"),
    (0x4D, "resource 0x4d"),
    (0x4E, "animation"),
    (0x4F, "navigation mesh"),
    (0x56, "animation dictionary"),
    (0x57, "headerless metadata"),
    (0x58, "cloth"),
    (0x59, "script program"),
    (0x5F, "keyed-blob descriptor"),
    (0x63, "dictionary"),
    (0x64, "navigation octree"),
    (0x65, "neighbour table"),
    (0x66, "animation"),
    (0x67, "morph target"),
    (0x6C, "headerless metadata"),
    (0x6D, "animation"),
    (0x6E, "asset index"),
    (0x6F, "text table"),
    (0x71, "headerless metadata"),
    (0x72, "keyed blob"),
    (0x73, "navigation metadata"),
];

/// The kind label of a row: its class byte where it is a 24-byte row, else its type code.
pub fn kind_label(class: Option<u8>, type_code: Option<u32>) -> Cow<'static, str> {
    match (class, type_code) {
        (Some(class), _) => CLASSES.iter().find(|(byte, _)| *byte == class).map_or_else(
            || format!("class {class:#04x}").into(),
            |(_, l)| (*l).into(),
        ),
        (None, Some(code)) => TYPES
            .iter()
            .find(|(c, _)| *c == code)
            .map_or_else(|| format!("type {code:#04x}").into(), |(_, l)| (*l).into()),
        (None, None) => "no row".into(),
    }
}

/// Each entry's kind label, in the archive's own order.
pub fn entry_kinds(archive: &Gta6Archive) -> Vec<Cow<'static, str>> {
    let extras = archive.entry_extras();
    (archive.entries.iter().enumerate())
        .map(|(at, entry)| {
            let code = extras
                .get(at)
                .and_then(|x| x.nested)
                .map(|n| n.resource_type);
            kind_label(entry.class, code)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::gta6_form_h::VERSIONS;
    use super::*;

    #[test]
    fn every_type_code_of_the_version_table_has_a_label_and_no_other_code_does() {
        for (code, _) in VERSIONS {
            let label = kind_label(None, Some(code));
            assert!(!label.starts_with("type "), "{code:#x} is {label}");
        }
        assert_eq!(TYPES.map(|(c, _)| c), VERSIONS.map(|(c, _)| c));
        assert_eq!(kind_label(None, Some(0x02)), "drawable");
        // Type 4's rows hold drawables, at a drawable's version; type 5's open as textures.
        assert_eq!(kind_label(None, Some(0x04)), "drawable dictionary");
        assert_eq!(kind_label(None, Some(0x05)), "texture dictionary");
        assert_eq!(kind_label(None, Some(0x58)), "cloth");
    }

    #[test]
    fn a_class_byte_names_its_kind_and_one_no_table_names_prints_as_its_hex() {
        assert_eq!(kind_label(Some(0x00), None), "archive");
        assert_eq!(kind_label(Some(0x40), None), "MoVE network");
        assert_eq!(kind_label(Some(0x42), None), "GFX");
        assert_eq!(kind_label(Some(0x5D), None), "PSO");
        assert_eq!(kind_label(Some(0x5E), None), "PSO");
        assert_eq!(kind_label(Some(0x60), None), "object trees");
        assert_eq!(kind_label(Some(0x68), None), "span grid");
        assert_eq!(kind_label(Some(0x69), None), "height map");
        assert_eq!(kind_label(Some(0xFF), None), "file");
        // A 24-byte row's class wins over any code; unknown bytes and codes print as hex.
        assert_eq!(kind_label(Some(0x0B), Some(0x02)), "resource");
        assert_eq!(kind_label(Some(0x0E), None), "class 0x0e");
        assert_eq!(kind_label(None, Some(0x7F)), "type 0x7f");
        assert_eq!(kind_label(None, Some(0x1_0000)), "type 0x10000");
        assert_eq!(kind_label(None, None), "no row");
    }
}
