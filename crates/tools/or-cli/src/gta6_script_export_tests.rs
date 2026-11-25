use serde_json::Value;

use super::*;

fn body(label: &str, bytes: Vec<u8>) -> Body {
    Body {
        archive: None,
        entry: None,
        label: label.to_owned(),
        bytes,
    }
}

#[test]
fn a_body_that_is_no_program_is_a_line_and_a_row_and_never_the_end() {
    // Eight zero bytes start a program, and what follows them here is none.
    let zeros = [vec![0u8; 8], vec![0x11; 56]].concat();
    let bodies = [body("a", zeros), body("b", Vec::new())];
    let run = |out: Option<&Path>| {
        let read = "a folder of deciphered programs";
        exported(&bodies, read, Names::default(), Options::default(), 2, out)
    };

    let (lines, failed) = run(None).expect("the export goes on");
    assert_eq!((lines.len(), failed), (3, 2), "{lines:?}");
    assert!(lines[0].starts_with("a: failed: "), "{lines:?}");
    assert!(lines[1].starts_with("b: failed: "), "{lines:?}");
    let total = "total: 2 program(s), 2 failed, 0 that gave the project no part: 0 function(s), ";
    assert!(lines[2].starts_with(total), "{lines:?}");

    // Two runs over one input write the same bytes.
    let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    run(Some(first.path())).expect("the export goes on");
    run(Some(second.path())).expect("the export goes on");
    for file in [PROJECT_FILE, SCRIPTS_FILE] {
        let written = std::fs::read(first.path().join(file)).unwrap();
        let again = std::fs::read(second.path().join(file)).unwrap();
        assert_eq!(written, again, "{file}");
    }
    // A body that failed left its row, and no source.
    assert_eq!(std::fs::read_dir(first.path()).unwrap().count(), 2);
    let scripts = std::fs::read(first.path().join(SCRIPTS_FILE)).unwrap();
    let scripts: Value = serde_json::from_slice(&scripts).unwrap();
    assert_eq!(scripts["level"], "source");
    assert_eq!(scripts["off"], Value::Array(Vec::new()));
    assert_eq!(scripts["programs"].as_array().map(Vec::len), Some(2));
    let row = &scripts["programs"][0];
    assert_eq!(
        (&row["read"], &row["file"]),
        (&Value::from("a"), &Value::Null)
    );
    let status = row["status"].as_str().unwrap_or_default();
    assert!(status.starts_with("failed: "), "{status}");
}

#[test]
fn a_programs_name_never_leaves_the_folder_its_source_is_written_to() {
    let mut names = Names::default();
    names
        .programs
        .insert(0x0A0B_0C0D, "..\\up/and:stream".to_owned());
    names
        .programs
        .insert(0x0A0B_0C0E, "demo_program".to_owned());
    assert_eq!(stem_of(&names, 0x0A0B_0C0D), ".._up_and_stream");
    assert_eq!(stem_of(&names, 0x0A0B_0C0E), "demo_program");
    // A program no list names is written under its hash.
    assert_eq!(stem_of(&names, 0x0000_0001), "0x00000001");
}
