//! A data XML body of a GTA VI archive, read through GTA VI's library where one is installed.

use std::path::Path;

use crate::state::{document_io, DocPayload, MetaDocument};

/// What text `bytes` of a GTA VI archive open as: XML through GTA VI's library where one is installed.
pub(crate) fn payload(path: &Path, bytes: Vec<u8>) -> DocPayload {
    crate::state::meta_schema();
    let Some(library) = or_io::meta::names_library().filter(|_| is_xml(&bytes)) else {
        return document_io::text_payload(path, bytes);
    };
    match or_io::meta::instance::parser::parse_typed_or_tree(library, &bytes) {
        Ok((instance, untyped)) => {
            DocPayload::Meta(MetaDocument::from_gta_vi_xml(bytes, instance, untyped))
        }
        Err(_) => document_io::text_payload(path, bytes),
    }
}

/// Whether `bytes` are XML: `<` and a name or `?` after an optional byte order mark and white space.
fn is_xml(bytes: &[u8]) -> bool {
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let start = text.iter().position(|b| !b.is_ascii_whitespace());
    match start.and_then(|at| text.get(at..at + 2)) {
        Some([b'<', next]) => *next == b'?' || *next == b'_' || next.is_ascii_alphabetic(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_xml;

    #[test]
    fn xml_is_told_from_other_text_by_its_first_tag() {
        assert!(is_xml(b"<?xml version=\"1.0\"?><a/>"));
        assert!(is_xml(b"\xEF\xBB\xBF \r\n\t<CZq>"));
        assert!(is_xml(b"<_zq/>"));
        assert!(!is_xml(b"<!-- a comment first -->"));
        assert!(!is_xml(b"zq = 1"));
        assert!(!is_xml(b"   "));
        assert!(!is_xml(b"<"));
    }

    /// Text edited to another root is read again as the document was opened, naming what is untyped.
    #[test]
    fn an_edited_root_is_read_again_and_names_the_structure_the_library_lacks() {
        use crate::state::document::TextDocument;
        let xml = b"<OrNoSuchRootZq><a>1</a></OrNoSuchRootZq>".to_vec();
        let library = crate::state::meta_schema();
        let (instance, untyped) =
            or_io::meta::instance::parser::parse_typed_or_tree(library, &xml).expect("it reads");
        assert_eq!(untyped.as_deref(), Some("OrNoSuchRootZq"));
        let mut doc = super::MetaDocument::from_gta_vi_xml(xml, instance, untyped);
        doc.text = TextDocument::decode(b"<OrOtherRootZq><b>2</b></OrOtherRootZq>");
        doc.mark_text_edited();
        doc.ensure_parsed_current();
        assert_eq!(doc.parse_error, None);
        assert_eq!(doc.gta6_untyped.as_deref(), Some("OrOtherRootZq"));
        let root = doc.parsed.as_ref().map(|p| p.instance.type_name.clone());
        assert_eq!(root.as_deref(), Some("OrOtherRootZq"));
    }

    /// A document no structure and no schemaless tree holds opens kept whole, named untyped, and
    /// saves as it came.
    #[test]
    fn a_document_kept_whole_opens_names_its_root_and_saves_as_it_came() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<or_zq_cycle_data version=\"2\">\n  <Item name=\"a\">text</Item>\n</or_zq_cycle_data>\n"
        )
        .as_bytes()
        .to_vec();
        let library = crate::state::meta_schema();
        let (instance, untyped) =
            or_io::meta::instance::parser::parse_typed_or_tree(library, &xml).expect("it opens");
        assert_eq!(untyped.as_deref(), Some("or_zq_cycle_data"));
        // The writer itself, not the text the document keeps, writes the element back whole.
        let written = or_io::meta::write_meta(&instance).expect("it writes");
        assert_eq!(written.as_bytes(), xml.as_slice());
        let doc = super::MetaDocument::from_gta_vi_xml(xml.clone(), instance, untyped);
        assert_eq!(doc.parse_error, None);
        assert_eq!(doc.gta6_untyped_elements, 1);
        assert_eq!(doc.to_bytes().expect("it saves"), xml);
    }
}
