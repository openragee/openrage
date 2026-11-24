//! A GTA VI metadata document's hash values no table names, named by the vocabulary harvested
//! from the archives and marked as from the text, which may be a collision; the tree and what it
//! saves are never changed.

use or_io::meta::binary::hash_index::default_hash_index;
use or_io::meta::binary::{ContainerLayout, MetaLayout};
use or_io::meta::instance::HashedString;

/// Whether `layout` is GTA VI's table form, P-form or headerless resource.
pub fn meta_names_from_the_text(layout: Option<&MetaLayout>) -> bool {
    match layout.map(|l| &l.container) {
        Some(ContainerLayout::Pso(p)) => p.is_table_form() || p.is_gta6(),
        Some(ContainerLayout::Rbm(r)) => r.seal.is_none(),
        _ => false,
    }
}

/// The name the vocabulary holds alone for `hash`, marked as from the text.
pub fn hash_text_name(hash: u32) -> Option<String> {
    let name = crate::state::gta6_names::name32(hash)?;
    Some(tr!("{name} (from the text)", name = name))
}

/// The marked vocabulary name of a hash value no table names.
pub fn meta_text_name(value: &HashedString) -> Option<String> {
    match value.raw.is_empty() && value.hash != 0 {
        true => hash_text_name(value.hash),
        false => None,
    }
}

/// `hash`'s name in the tables, else its marked vocabulary name, else `hash_XXXXXXXX`.
pub fn hash_name_or_text(hash: u32) -> String {
    let index = default_hash_index();
    match index.lookup(hash) {
        Some(name) => name.to_string(),
        None => hash_text_name(hash).unwrap_or_else(|| index.name_or_literal(hash)),
    }
}

/// `xml` as the Text panel shows it: a GTA VI form's named hash values each take a comment, the
/// document's own string first.
pub(super) fn shown_xml(
    xml: String,
    layout: Option<&MetaLayout>,
    strings: &super::gta6_meta_strings::DocumentStrings,
) -> String {
    let vocabulary = meta_names_from_the_text(layout);
    match vocabulary || strings.has_names() {
        true => with_text_names(&xml, |hash| {
            (strings.name(hash).map(str::to_owned))
                .or_else(|| vocabulary.then(|| hash_text_name(hash)).flatten())
        }),
        false => xml,
    }
}

/// `xml` with `<!-- name -->` after each element text `hash_XXXXXXXX` that `name` names; a
/// reader skips a comment, so the text parses to the same tree.
fn with_text_names(xml: &str, name: impl Fn(u32) -> Option<String>) -> String {
    const LITERAL: &str = ">hash_";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(LITERAL) {
        let (head, tail) = rest.split_at(at + LITERAL.len());
        out.push_str(head);
        rest = tail;
        let digits = (rest.get(..8)).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        let closed = digits.filter(|_| rest.as_bytes().get(8) == Some(&b'<'));
        let Some(hash) = closed.and_then(|d| u32::from_str_radix(d, 16).ok()) else {
            continue;
        };
        out.push_str(&rest[..8]);
        rest = &rest[8..];
        // A comment holds no `--`.
        if let Some(said) = name(hash).filter(|said| !said.contains("--")) {
            out.push_str("<!-- ");
            out.push_str(&said);
            out.push_str(" -->");
        }
    }
    out.push_str(rest);
    out
}

/// The inverse of [`with_text_names`]: the `<!-- name -->` comments it inserts after an element's
/// `hash_XXXXXXXX` text removed, so the bytes are the deterministic document the CLI writes.
pub(super) fn without_text_names(xml: &str) -> String {
    const LITERAL: &str = ">hash_";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(LITERAL) {
        let (head, tail) = rest.split_at(at + LITERAL.len());
        out.push_str(head);
        rest = tail;
        let digits = (rest.get(..8)).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()));
        if digits.is_none() || rest.as_bytes().get(8) != Some(&b'<') {
            continue;
        }
        out.push_str(&rest[..8]);
        rest = &rest[8..];
        if let Some(stripped) = rest.strip_prefix("<!-- ") {
            if let Some(end) = stripped.find(" -->") {
                rest = &stripped[end + " -->".len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(hash: u32) -> Option<String> {
        match hash {
            0x0000_00AB => Some("zq_crate (from the text)".to_string()),
            0x0000_00CD => Some("zq--odd".to_string()),
            _ => None,
        }
    }

    #[test]
    fn a_named_hash_value_takes_a_comment_and_the_rest_of_the_xml_is_kept() {
        let rest = concat!(
            "\n  <b>hash_000000EF</b>\n  <c>hash_000000ab_x</c>",
            "\n  <d>hash_000000CD</d>\n</Root>\n"
        );
        let xml = format!("<Root>\n  <hash_000000AB>hash_000000AB</hash_000000AB>{rest}");
        let shown = with_text_names(&xml, named);
        let comment = "<!-- zq_crate (from the text) -->";
        let wanted =
            format!("<Root>\n  <hash_000000AB>hash_000000AB{comment}</hash_000000AB>{rest}");
        assert_eq!(shown, wanted);
        assert_eq!(with_text_names(&xml, |_| None), xml);
    }

    #[test]
    fn the_shown_xml_still_parses_to_the_same_tree() {
        let xml = "<Root>\n  <a>hash_000000AB</a>\n</Root>\n";
        let shown = with_text_names(xml, named);
        assert_ne!(shown, xml);
        let read = |text: &str| {
            or_io::meta::instance::parse_schemaless(text.as_bytes()).expect("the XML parses")
        };
        assert_eq!(read(shown.as_str()), read(xml));
    }

    #[test]
    fn a_document_read_with_no_layout_is_no_gta6_form_and_its_xml_is_unchanged() {
        assert!(!meta_names_from_the_text(None));
        let xml = "<a>hash_000000AB</a>".to_string();
        let none = super::super::gta6_meta_strings::DocumentStrings::default();
        assert_eq!(shown_xml(xml.clone(), None, &none), xml);
    }

    #[test]
    fn the_saved_xml_is_the_comment_free_document_whatever_the_vocabulary_shows() {
        let xml = concat!(
            "<Root>\n  <a>hash_000000AB</a>\n  <b>hash_000000EF</b>",
            "\n  <c>hash_000000ab_x</c>\n</Root>\n"
        );
        // Whether or not the vocabulary names a hash, the saved bytes are the same document.
        let shown = with_text_names(xml, named);
        assert_ne!(shown, xml, "the Text panel shows the comment");
        assert_eq!(without_text_names(&shown), xml, "the save drops it");
        assert_eq!(
            without_text_names(xml),
            xml,
            "a document with no comment is untouched"
        );
    }

    #[test]
    fn a_hash_value_a_table_names_takes_no_name_from_the_text() {
        let named = HashedString::new("zq_named");
        assert_eq!(meta_text_name(&named), None);
        assert_eq!(meta_text_name(&HashedString::from_hash(0)), None);
    }

    #[test]
    fn a_hash_the_tables_name_keeps_that_name_and_one_nothing_names_shows_as_a_literal() {
        let hash = or_io::string_hash(b"zq_meta_names_nothing_names_this");
        assert_eq!(hash_name_or_text(hash), format!("hash_{hash:08X}"));
        let index = default_hash_index();
        let held =
            (index.names()).find(|n| index.lookup(or_io::string_hash(n.as_bytes())) == Some(*n));
        if let Some(name) = held {
            assert_eq!(hash_name_or_text(or_io::string_hash(name.as_bytes())), name);
        }
    }
}
