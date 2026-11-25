use or_io::meta::binary::pso::string_names::registered_strings;

use super::gta6_string_fixture::{document, hash_of, string_keys, HELD};
use super::*;

/// The fixture document read, with its root's text set to `text`.
fn with_a_text(text: &str) -> (Vec<u8>, MetaInstance, or_io::meta::binary::MetaLayout) {
    let doc = document(&string_keys());
    let (mut instance, layout, _) = read_instance(&doc, ErrorMode::Lenient).expect("it reads");
    instance.fields[0].1 = Value::String(text.into());
    (doc, instance, layout.expect("a binary layout"))
}

#[test]
fn from_xml_like_a_donor_with_an_enciphered_run_saves_a_new_text_that_reads_back_named() {
    let (_, instance, layout) = with_a_text("zq_new");
    let saved = write_like(&layout, &instance, || Ok(Some(string_keys()))).expect("it saves");
    let strings: Vec<(u32, String)> = (registered_strings(&saved, Some(&string_keys())))
        .expect("its strings read")
        .into_iter()
        .map(|s| (s.category, s.text))
        .collect();
    assert_eq!(strings, [(1, HELD.to_string()), (1, "zq_new".into())]);
    let (again, _, _) = read_instance(&saved, ErrorMode::Lenient).expect("it reopens");
    assert_eq!(
        hash_of(&again.fields[0].1),
        Some(or_io::string_hash(b"zq_new"))
    );
}

#[test]
fn from_xml_like_with_no_keys_refuses_a_new_text_by_name() {
    let (doc, instance, layout) = with_a_text("zq_new");
    let err = write_like(&layout, &instance, || Ok(None)).expect_err("no keys");
    assert!(
        err.contains("zq_new")
            && err.contains("no key snapshot")
            && err.contains(GTA6_SNAPSHOT_ENV),
        "{err}"
    );
    let err = write_like(&layout, &instance, || Err("unreadable".into())).expect_err("no keys");
    assert!(
        err.contains("zq_new") && err.ends_with("; unreadable"),
        "{err}"
    );
    // The unedited tree needs no keys.
    let (instance, _, _) = read_instance(&doc, ErrorMode::Lenient).expect("it reads");
    assert!(write_like(&layout, &instance, || Ok(None)).is_ok());
}
