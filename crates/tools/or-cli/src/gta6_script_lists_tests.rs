use super::*;

/// A folder of lists written by hand, under names and hashes that are no game's.
fn written(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

const HANDLERS: &str = "hash\trva\n0x00000000000000A1\t0x01020304\n";
const NATIVES: &str = "0x00000000000000A1 DEMO::GET_DEMO_THING\n";

#[test]
fn a_file_of_the_folder_is_read_as_the_list_it_reads_as_and_one_that_is_none_is_said() {
    let dir = written(&[
        ("a.tsv", "0x0A0B0C0D\tdemo_program\n"),
        ("b.json", "{\"DEMO\": {}}\n"),
        // A list refused at its second line names nothing with its first.
        ("c.tsv", "0x00000001\tearly\nno hash here\n"),
        // A table of handlers is one whatever it is called: here it is called as a natives list is.
        ("natives.tsv", HANDLERS),
        ("natives-demo.txt", NATIVES),
        // A natives list is one whatever it is called, where it reads as no names list.
        (
            "z.json",
            "{\"DEMO\": {\"0x00000000000000B2\": \"USE_DEMO_THING\"}}\n",
        ),
        (".hidden", "not a list\tat all\n"),
        (
            "project.json",
            "{\"DEMO\": {\"0x00000000000000C3\": \"NEVER_READ\"}}\n",
        ),
    ]);
    std::fs::create_dir(dir.path().join("natives-folder")).unwrap();

    let found = folder(dir.path());
    assert_eq!(found.names.lists, ["a.tsv", "natives.tsv"]);
    assert_eq!(found.names.program(0x0A0B_0C0D), Some("demo_program"));
    assert_eq!(found.names.program(1), None);
    assert_eq!(found.names.native_at(0x0102_0304), Some(0xA1));
    let labels: Vec<&str> = found.natives.iter().map(NativeDb::label).collect();
    assert_eq!(labels, ["natives-demo.txt", "z.json"]);
    assert!(found.natives.iter().all(|list| list.name(0xC3).is_none()));

    // Each file that is neither is said once, with what each reader refused it for.
    assert_eq!(found.refused.len(), 2, "{:?}", found.refused);
    assert!(found.refused[0].starts_with("`b.json` of the names folder is no list"));
    assert!(found.refused[1].starts_with("`c.tsv` of the names folder is no list"));
    assert!(found.refused[1].contains("line 2"), "{:?}", found.refused);

    let missing = folder(&dir.path().join("missing"));
    assert!(missing.names.lists.is_empty() && missing.natives.is_empty());
    assert!(missing.refused.is_empty());
}

#[test]
fn the_folder_names_a_native_where_it_holds_the_handlers_and_a_natives_list() {
    let config = tempfile::tempdir().unwrap();
    let dir = names_dir(config.path());
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("natives-demo.txt"), NATIVES).unwrap();

    // With no hash of any handler the natives list names nothing, and that is said of it.
    let idle = supplied(&[], &[], Some(config.path())).expect("the folder reads");
    assert!(idle.names.natives.is_none());
    assert_eq!(idle.idle, ["natives-demo.txt"]);
    let said = idle.said();
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("natives lists: `natives-demo.txt` read, and none names a native"),
        "{said:?}"
    );

    // A natives list is named beside the folder so that no list of this machine's is looked for.
    std::fs::write(dir.join("natives.tsv"), HANDLERS).unwrap();
    let named = config.path().join("named.txt");
    std::fs::write(&named, NATIVES).unwrap();
    let lists = supplied(&[], std::slice::from_ref(&named), Some(config.path()));
    let lists = lists.expect("the lists read");
    assert!(lists.idle.is_empty() && lists.said().is_empty());
    let native = lists.names.native(0xA1).map(ToString::to_string);
    assert_eq!(native.as_deref(), Some("DEMO::GET_DEMO_THING"));
    assert_eq!(
        lists.names.natives.as_ref().map(NativeDb::label),
        Some("named.txt")
    );

    // A list that holds the hash of no handler names nothing either, and is said to for that.
    let other = config.path().join("other.txt");
    std::fs::write(&other, "0x00000000000000B2 DEMO::USE_DEMO_THING\n").unwrap();
    let lists = supplied(&[], std::slice::from_ref(&other), Some(config.path()));
    let lists = lists.expect("the lists read");
    assert!(lists.names.natives.is_none());
    assert_eq!(lists.idle, ["other.txt"]);
    assert!(
        lists.said()[0].ends_with(NO_HASH_HELD),
        "{:?}",
        lists.said()
    );
}

#[test]
fn a_list_that_is_named_must_read_whatever_it_could_then_name() {
    let config = tempfile::tempdir().unwrap();
    let missing = config.path().join("missing.json");
    // No names list gives a handler's hash here: the natives list is read all the same.
    let unread = supplied(&[], std::slice::from_ref(&missing), Some(config.path()));
    assert!(unread.is_err_and(|e| e.starts_with("read ")));

    let program = config.path().join("program");
    std::fs::write(&program, [0u8; 64]).unwrap();
    let no_list = supplied(&[], std::slice::from_ref(&program), Some(config.path()));
    assert!(no_list.is_err());

    let no_names = supplied(std::slice::from_ref(&missing), &[], Some(config.path()));
    assert!(no_names.is_err_and(|e| e.starts_with("read ")));
}
