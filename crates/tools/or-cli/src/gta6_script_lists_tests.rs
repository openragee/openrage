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
    assert!(lists.idle.is_empty());
    // The one line said is of the names list the folder gave: the handlers' table, one handler.
    assert_eq!(
        lists.said(),
        [
            "lists read, in order, each with how many hashes, cells, indices, functions or words \
          it named that no list before it did: `natives.tsv` 1"
        ]
    );
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
    // The natives lists are said after the names lists.
    let said = lists.said();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[1].ends_with(NO_HASH_HELD), "{said:?}");
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

/// A row of a names list whose hash is the 32-bit hash of its name in lower case.
fn proved_row(name: &str) -> String {
    let hash = or_io::hash::literal_string_hash(name.to_ascii_lowercase().as_bytes());
    format!("0x{hash:08X}\t{name}\n")
}

#[test]
fn a_list_that_proves_every_row_is_read_before_a_label_the_folder_gives_on_trust() {
    let hash = or_io::hash::literal_string_hash(b"demoalpha");
    // Read in the order of the names alone, the label first would leave the proof refused. Its
    // row of column names keeps it from reading as a natives list.
    let label = format!("program_hash\tname\n0x{hash:08X}\tdemo_label\n");
    let dir = written(&[
        ("a_label.tsv", label.as_str()),
        ("b_proved.tsv", proved_row("demoalpha").as_str()),
    ]);
    let found = folder(dir.path());
    assert_eq!(found.names.lists, ["b_proved.tsv"]);
    assert_eq!(found.names.program(u64::from(hash)), Some("demoalpha"));
    assert_eq!(found.named, [("b_proved.tsv".to_owned(), 1)]);
    assert_eq!(found.refused.len(), 1, "{:?}", found.refused);
    assert!(found.refused[0].starts_with("`a_label.tsv` of the names folder is no list"));

    // Named on the command line, the lists are read in the order given, whatever they prove.
    let paths = [
        dir.path().join("a_label.tsv"),
        dir.path().join("b_proved.tsv"),
    ];
    assert!(supplied(&paths, &[], None).is_err());
    let paths = [dir.path().join("b_proved.tsv")];
    let named = supplied(&paths, &[], None).expect("the list reads");
    assert_eq!(named.named, [("b_proved.tsv".to_owned(), 1)]);
    assert!(
        named.said()[0].ends_with("`b_proved.tsv` 1"),
        "{:?}",
        named.said()
    );
    assert!(proves_alone(
        "b_proved.tsv",
        proved_row("demoalpha").as_bytes()
    ));
    assert!(!proves_alone("a_label.tsv", label.as_bytes()));
    assert!(!proves_alone("words.txt", b"demoalpha\n"));
    assert!(!proves_alone("bytes.bin", &[0xFF, 0xFE]));
}

#[test]
fn a_file_whose_name_says_it_holds_candidates_is_never_read_from_the_folder() {
    let dir = written(&[
        ("demo-candidates.tsv", proved_row("demobeta").as_str()),
        ("Demo_Candidate.txt", proved_row("demobeta").as_str()),
        ("demo-proved.tsv", proved_row("demoalpha").as_str()),
    ]);
    let found = folder(dir.path());
    assert_eq!(found.names.lists, ["demo-proved.tsv"]);
    assert_eq!(
        found.candidates,
        ["Demo_Candidate.txt", "demo-candidates.tsv"]
    );
    assert!(found.refused.is_empty(), "{:?}", found.refused);
    let beta = or_io::hash::literal_string_hash(b"demobeta");
    assert_eq!(found.names.program(u64::from(beta)), None);

    // Each passed over is said in the header, and named with --names one is read.
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir(names_dir(config.path())).unwrap();
    for name in [
        "demo-candidates.tsv",
        "Demo_Candidate.txt",
        "demo-proved.tsv",
    ] {
        std::fs::copy(dir.path().join(name), names_dir(config.path()).join(name)).unwrap();
    }
    let lists = supplied(&[], &[], Some(config.path())).expect("the folder reads");
    let said = lists.said();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(
        said[1].starts_with("candidates: `Demo_Candidate.txt`, `demo-candidates.tsv` of the"),
        "{said:?}"
    );
    let named = [names_dir(config.path()).join("demo-candidates.tsv")];
    let lists = supplied(&named, &[], Some(config.path())).expect("the list reads");
    assert_eq!(lists.names.program(u64::from(beta)), Some("demobeta"));
    assert!(lists.candidates.is_empty());
    assert!(guessed("X-CANDIDATES.tsv") && !guessed("proved.tsv"));
}

#[test]
fn a_proved_generic_type_name_of_the_folder_is_never_displaced_by_a_printed_label_of_it() {
    let generic = "DemoMap`2<DemoKey, DemoValue>";
    let hash = or_io::gta6::hash::literal_hash64(generic.as_bytes());
    let proved = format!("type_hash64\tname\n0x{hash:016X}\t{generic}\n");
    let label = format!("type_hash64\tname\n0x{hash:016X}\tDemoMap_2__DemoKey__DemoValue\n");
    // The label's file sorts first, as a printed spelling's beside its proof does.
    let dir = written(&[
        ("r_demo-printed.tsv", label.as_str()),
        ("r_demo-proved.tsv", proved.as_str()),
    ]);
    let found = folder(dir.path());
    assert_eq!(found.names.lists, ["r_demo-proved.tsv"]);
    let written_as = found.names.type_name(hash).unwrap_or_default();
    assert!(written_as.starts_with("DemoMap<"), "{written_as}");
    assert!(
        found.refused[0].starts_with("`r_demo-printed.tsv`"),
        "{:?}",
        found.refused
    );
}
