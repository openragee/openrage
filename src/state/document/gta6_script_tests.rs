use crate::ui::panels::text_editor::script_view::ScriptAsk;

use super::*;

/// A source written by hand in the shapes a decompile writes; nothing of a game.
const SOURCE: &str = "\
// program 0x0A0B0C0D

class type_00000000AAAA0001 : type_00000000AAAA0002
{
    var f_10;
    void Run(var p0)
    {
        if (p0 == 0)
        {
            func_000040(1);
        }
        else
        {
            Add(new Item(2)
            {
                f_8 = 3,
            });
        }
    }
}

void main()
{
    func_000040(func_000010());
    other.func_000040(4);
}

var func_000040(var p0)
{
    return p0;
}

// func_000080 at 0x000080: the listing holds no body (not lifted)
";

fn document(source: &str) -> Gta6ScriptDocument {
    let program = Program {
        program: 0x0A0B_0C0D,
        main: Some(0x10),
        ..Program::default()
    };
    Gta6ScriptDocument::of(program, source, Level::Source)
}

#[test]
fn the_outline_lists_the_heads_of_a_source_and_nothing_a_statement_opens() {
    let doc = document(SOURCE);
    type Row<'a> = (&'a str, u32, u32, u8, bool, Option<u32>);
    let rows: Vec<Row<'_>> = (doc.outline.iter())
        .map(|r| (r.name.as_str(), r.line, r.lines, r.depth, r.class, r.at))
        .collect();
    assert_eq!(
        rows,
        [
            ("type_00000000AAAA0001", 2, 18, 0, true, None),
            ("Run", 5, 14, 1, false, None),
            ("main", 21, 5, 0, false, Some(0x10)),
            ("func_000040", 27, 4, 0, false, Some(0x40)),
            ("func_000080", 32, 1, 0, false, Some(0x80)),
        ]
    );
    assert!(document("").outline.is_empty());
    // A listing's own syntax: an attribute over the head, and the offset after it as a comment.
    let listed = "public static class Program_0A0B0C0D\n{\n    [VmFunction(0x000040)]\n    \
                  public static VmValue func_000040(VmValue arg0) // 0x000040\n    {\n    }\n}\n";
    let names: Vec<String> = (document(listed).outline.iter())
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names, ["Program_0A0B0C0D", "func_000040"]);
}

#[test]
fn the_outline_lists_the_classes_with_what_they_hold_before_the_free_functions() {
    // As the source level writes a program: free functions first, then each class.
    let source = "\
void main()
{
}

class type_00000000AAAA0001
{
    void Run()
    {
    }
}

// func_000040: no body (a demo reason: of words)

class type_00000000AAAA0002
{
}
";
    let doc = document(source);
    type Row<'a> = (&'a str, u32, u8, bool, Option<u32>);
    let rows: Vec<Row<'_>> = (doc.outline.iter())
        .map(|r| (r.name.as_str(), r.line, r.depth, r.class, r.at))
        .collect();
    assert_eq!(
        rows,
        [
            ("type_00000000AAAA0001", 4, 0, true, None),
            ("Run", 6, 1, false, None),
            ("type_00000000AAAA0002", 13, 0, true, None),
            ("main", 0, 0, false, Some(0x10)),
            ("func_000040", 11, 0, false, Some(0x40)),
        ]
    );
    // A name is still followed to its line, wherever its row stands.
    assert_eq!(doc.definition("func_000040").map(|row| row.line), Some(11));
    assert_eq!(doc.definition("main").map(|row| row.line), Some(0));
}

#[test]
fn a_class_head_over_several_lines_is_one_row_and_a_head_takes_the_offset_its_comment_notes() {
    let source = "\
class type_00000000AAAA0001 : type_00000000AAAA0002, type_00000000AAAA0003,
    type_00000000AAAA0004,
    type_00000000AAAA0005
{
    void Run(var p0) // func_000020
    {
    }
    void Stop() // a note
    {
    }
}
class type_00000000AAAA0006 // a closure
{
}
";
    let doc = document(source);
    type Row<'a> = (&'a str, u32, u32, u8, bool, Option<u32>);
    let rows: Vec<Row<'_>> = (doc.outline.iter())
        .map(|r| (r.name.as_str(), r.line, r.lines, r.depth, r.class, r.at))
        .collect();
    assert_eq!(
        rows,
        [
            ("type_00000000AAAA0001", 0, 11, 0, true, None),
            ("Run", 4, 3, 1, false, Some(0x20)),
            ("Stop", 7, 3, 1, false, None),
            ("type_00000000AAAA0006", 11, 3, 0, true, None),
        ]
    );
    // The function the slot names is the one whose head notes it.
    assert_eq!(doc.definition("func_000020").map(|row| row.line), Some(4));
    assert_eq!(noted_offset("0x000040"), Some(0x40));
    for none in ["a closure", "0x", "func_", "base type_00000000AAAA0002"] {
        assert_eq!(noted_offset(none), None, "{none}");
    }
}

#[test]
fn a_line_is_a_head_only_where_its_whole_shape_is_one() {
    assert_eq!(function_head("void Add(var p0)"), Some("Add"));
    assert_eq!(function_head("Demo(var p0)"), Some("Demo"));
    assert_eq!(function_head("var[] func_000010()"), Some("func_000010"));
    // A call cut in two over an initialiser's brace closes fewer brackets than it opens.
    assert_eq!(function_head("Add(new Item(2)"), None);
    for statement in [
        "if (p0 == 0)",
        "while (Run(p0))",
        "foreach (var x in Items(p0))",
        "var x = Make(1)",
        "return Make(1)",
        "new Item(2)",
        "p0.Run(1)",
        "else",
    ] {
        assert_eq!(function_head(statement), None, "{statement}");
    }
    assert_eq!(class_head("class A : B"), Some("A"));
    assert_eq!(class_head("public static class A"), Some("A"));
    assert_eq!(class_head("enum Kind"), Some("Kind"));
    assert_eq!(class_head("var x = new class"), None);
    assert_eq!(class_head("class"), None);
    assert_eq!(
        bodiless("// main at 0x000010: no body"),
        Some(("main", 0x10))
    );
    assert_eq!(bodiless("// a note at 0x10"), None);
    // The source level names a function with no body by the slot that states its offset.
    assert_eq!(
        bodiless("// func_000040: no body (a demo reason at 0x10: of words)"),
        Some(("func_000040", 0x40))
    );
    assert_eq!(bodiless("// func_000040: a note"), None);
    assert_eq!(bodiless("// project: no body (a note)"), None);
}

#[test]
fn a_name_is_followed_to_its_one_head_by_name_or_by_the_offset_it_states() {
    let doc = document(SOURCE);
    let line = |name: &str| doc.definition(name).map(|row| row.line);
    assert_eq!(line("func_000040"), Some(27));
    assert_eq!(line("Run"), Some(5));
    assert_eq!(line("type_00000000AAAA0001"), Some(2));
    // The head printed as `main` is the function its callers spell by its offset.
    assert_eq!(line("func_000010"), Some(21));
    assert_eq!(line("func_000080"), Some(32));
    for nothing in ["p0", "func_000099", "func_", "func_00zz", "other", ""] {
        assert_eq!(line(nothing), None, "{nothing:?}");
    }
    // Two heads of one name: neither is followed.
    let twice = "class A\n{\n    void Run()\n    {\n    }\n}\n\nclass B\n{\n    void Run()\n    {\n    }\n}\n";
    let doc = document(twice);
    assert_eq!(doc.outline.len(), 4);
    assert_eq!(doc.definition("Run"), None);
    assert_eq!(doc.definition("B").map(|row| row.line), Some(7));
}

#[test]
fn references_are_every_spelling_of_a_function_and_never_another_programs() {
    let doc = document(SOURCE);
    assert_eq!(
        doc.references("main", None),
        r"[^.\w](?:main|func_000010)\b"
    );
    assert_eq!(
        doc.references("func_000010", None),
        r"[^.\w](?:func_000010|main)\b"
    );
    assert_eq!(doc.references("p0", None), r"[^.\w](?:p0)\b");
    assert_eq!(
        doc.references("func_000040", Some("other")),
        r"\bother\.func_000040\b"
    );
    assert_eq!(doc.references("Run", Some("")), r"\.Run\b");
    let count = |pattern: String| {
        let found = regex::Regex::new(&pattern).expect("a pattern");
        found.find_iter(SOURCE).count()
    };
    // Two calls and the head; the call through `other.` is another program's function.
    assert_eq!(count(doc.references("func_000040", None)), 3);
    assert_eq!(count(doc.references("func_000040", Some("other"))), 1);
    // The head `main` and the one call that spells its offset.
    assert_eq!(count(doc.references("main", None)), 2);
}

#[test]
fn bytes_that_are_no_program_are_no_script() {
    // Eight zero bytes are where a program starts and are not a program.
    assert!(sniffed(&[0u8; 64]).is_none());
    assert!(sniffed(b"RSC7\0\0\0\0\0\0\0\0").is_none());
    assert!(sniffed(&[]).is_none());
    assert!(open(&[0u8; 64]).is_err());
    assert!(open(&[]).is_err());
}

#[test]
fn a_list_refused_part_way_names_nothing_and_the_lists_after_it_are_read() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let write = |name: &str, text: &str| {
        std::fs::write(folder.path().join(name), text).expect("a list is written");
    };
    write("a_broken.txt", "0x0A0B0C0D demo_program\nnot a row\n");
    write("b_names.txt", "0x0E0F1011 other_program\n");
    write("natives_none.txt", "");
    let read = lists::supplied(Some(folder.path()));
    assert_eq!(read.names.lists, ["b_names.txt"]);
    assert_eq!(read.names.program(0x0E0F_1011), Some("other_program"));
    assert_eq!(read.names.program(0x0A0B_0C0D), None);
    assert!(read.names.natives.is_none() && read.natives.is_empty());
    // Each file no reader took is said, under its name and with a reason.
    assert_eq!(read.refused.len(), 2, "{:?}", read.refused);
    assert!(read.refused[0].starts_with("a_broken.txt: ") && read.refused[0].len() > 14);
    assert!(read.refused[1].starts_with("natives_none.txt: "));
    assert!(lists::supplied(None).names.lists.is_empty());
    let missing = folder.path().join("missing");
    assert!(lists::supplied(Some(&missing)).names.lists.is_empty());
}

#[test]
fn a_file_of_the_names_folder_is_a_names_list_where_it_reads_as_one_else_a_natives_list() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let write = |name: &str, text: &str| {
        std::fs::write(folder.path().join(name), text).expect("a list is written");
    };
    // A table of handlers' addresses is a names list, whatever its file is called.
    write(
        "natives_demo.tsv",
        "hash\trva\n0x00000000000A0B0C\t0x1000\n",
    );
    // JSON is a natives list, whatever its file is called.
    write("demo_list.json", "{\"0x00000000000A0B0C\": \"DEMO_TICK\"}");
    // Neither is a list: a project is read as a project, and a dot file is the file manager's.
    write(lists::PROJECT_FILE, "{}");
    write(".demo_hidden", "not a row at all\n");
    let read = lists::supplied(Some(folder.path()));
    assert_eq!(read.names.lists, ["natives_demo.tsv"]);
    assert_eq!(read.names.native_at(0x1000), Some(0x000A_0B0C));
    assert_eq!(read.natives, ["demo_list.json"]);
    let used = read.names.natives.as_ref().map(|list| list.label());
    assert_eq!(used, Some("demo_list.json"));
    assert!(read.refused.is_empty(), "{:?}", read.refused);

    // A natives list written as text reads as a names list too: its file's name says which it is.
    write("natives_text.txt", "0x00000000000A0B0C DEMO_TICK\n");
    let read = lists::supplied(Some(folder.path()));
    assert_eq!(read.names.lists, ["natives_demo.tsv"]);
    assert_eq!(read.natives, ["demo_list.json", "natives_text.txt"]);

    // A program states an address: with no hash of any handler, no natives list names anything.
    std::fs::remove_file(folder.path().join("natives_demo.tsv")).expect("the table goes");
    let read = lists::supplied(Some(folder.path()));
    assert_eq!(read.natives.len(), 2);
    assert!(read.names.natives.is_none());
}

#[test]
fn a_labels_file_of_the_names_folder_is_read_only_when_asked_and_labels_what_no_list_names() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let write = |name: &str, text: &str| {
        std::fs::write(folder.path().join(name), text).expect("a list is written");
    };
    write(
        "handlers.tsv",
        "hash\trva\n0x00000000000A0B0C\t0x1000\n0x00000000000A0B0D\t0x1008\n",
    );
    write(
        "natives_demo.json",
        "{\"0x00000000000A0B0C\": \"DEMO_TICK\"}",
    );
    // By its name alone it would be read as a natives list and name both natives.
    write(
        "natives-demo-labels.txt",
        "0x00000000000A0B0C _GUESS_DEMO_TICK\n0x00000000000A0B0D _GUESS_DEMO_TOCK\n",
    );
    let mut read = lists::supplied(Some(folder.path()));
    assert_eq!(read.labels, ["natives-demo-labels.txt"]);
    assert_eq!(read.natives, ["natives_demo.json"]);
    assert!(read.names.native(0x000A_0B0D).is_none());
    assert!(read.labelled.is_empty());

    lists::labelled(&mut read, Some(folder.path()));
    assert_eq!(read.labelled, [("natives-demo-labels.txt".to_owned(), 1)]);
    let name = |hash: u64| read.names.native(hash).map(ToString::to_string);
    let label = |hash: u64| read.names.native_label(hash).map(ToString::to_string);
    // A native a natives list names keeps that name; a label is a guess, never a name.
    assert_eq!(name(0x000A_0B0C).as_deref(), Some("DEMO_TICK"));
    assert_eq!(label(0x000A_0B0C), None);
    assert_eq!(name(0x000A_0B0D), None);
    assert_eq!(label(0x000A_0B0D).as_deref(), Some("_GUESS_DEMO_TOCK"));
    let used = read.names.natives.as_ref().map(|list| list.label());
    assert_eq!(used, Some("natives_demo.json"));

    // A labels file that reads as no list of labels is refused with its reason.
    write("broken-labels.txt", "no hash here\n");
    let mut read = lists::supplied(Some(folder.path()));
    lists::labelled(&mut read, Some(folder.path()));
    assert_eq!(read.labelled.len(), 1, "{:?}", read.labelled);
    assert!(
        read.refused
            .iter()
            .any(|why| why.starts_with("broken-labels.txt: ")),
        "{:?}",
        read.refused
    );
}

#[test]
fn a_record_of_the_program_is_named_by_the_hash_it_holds_of_its_name() {
    use or_io::gta6::script::ir::TypeKey;
    use or_io::gta6::script::program::{FieldRow, MethodRow, TypeRow};
    let mut names = or_io::gta6::script::Names::default();
    names.field_names.insert(0x1234, "demo_field".to_owned());
    names.method_names.insert(0x77, "DemoRun".to_owned());
    let field = FieldRow {
        offset: 0x10,
        name: 0x1234,
        ..FieldRow::default()
    };
    let unnamed = FieldRow {
        offset: 0x18,
        name: 0x5678,
        ..FieldRow::default()
    };
    let program = Program {
        types: vec![TypeRow {
            name: 0xAAAA_0001,
            fields: vec![field, unnamed],
            ..TypeRow::default()
        }],
        methods: vec![MethodRow {
            name: 0x77,
            owner: 0,
            slot: Some(3),
            ..MethodRow::default()
        }],
        ..Program::default()
    };
    lists::bind(&mut names, &program);
    let owner = TypeKey(0xAAAA_0001);
    assert_eq!(names.field(owner, 0x10), Some("demo_field"));
    assert_eq!(names.field(owner, 0x18), None);
    assert_eq!(names.method(owner, 3), Some("DemoRun"));
}

#[test]
fn a_project_is_read_from_the_first_folder_that_holds_one_and_kept_for_the_next_script() {
    use or_io::gta6::script::project::index;
    let beside = tempfile::tempdir().expect("a scratch folder");
    let names = tempfile::tempdir().expect("a scratch folder");
    let both = [Some(beside.path()), Some(names.path())];
    let found = lists::project(&both);
    assert!(found.held.is_none() && found.refused.is_none());
    assert!(std::ptr::eq(
        found.project(),
        or_io::gta6::script::Project::empty()
    ));

    let written = index::save(&or_io::gta6::script::Project::default()).expect("a project");
    std::fs::write(names.path().join(lists::PROJECT_FILE), written).expect("it is written");
    let found = lists::project(&[None, Some(names.path())]);
    assert!(found.refused.is_none());
    let (stamp, first) = found.held.expect("the project of the names folder");
    assert_eq!(stamp.file, names.path().join("project.json"));
    // Read once: the next script is handed the same project.
    let (_, again) = lists::project(&both).held.expect("the project kept");
    assert!(Arc::ptr_eq(&first, &again));

    // A file that is no project is said, and the next folder's project is the one used.
    std::fs::write(beside.path().join(lists::PROJECT_FILE), "not a project").expect("written");
    let found = lists::project(&both);
    let refused = found.refused.clone().expect("the file beside is refused");
    assert!(
        refused.starts_with(&beside.path().display().to_string()),
        "{refused}"
    );
    let (stamp, third) = found.held.expect("the project of the names folder");
    assert_eq!(stamp.file, names.path().join("project.json"));
    // The refused file is remembered, so the one kept stays kept.
    let found = lists::project(&both);
    assert_eq!(found.refused.as_deref(), Some(refused.as_str()));
    let (_, fourth) = found.held.expect("the project kept");
    assert!(Arc::ptr_eq(&third, &fourth));
}

#[test]
fn a_large_program_opens_with_its_main_alone_and_lists_the_rest_as_lines_the_outline_reads() {
    let row = |at: u32| FunctionRow {
        at,
        end: at + 4,
        ..FunctionRow::default()
    };
    let mut program = Program {
        main: Some(0x10),
        functions: vec![row(0x10), row(0x40), row(0x80)],
        ..Program::default()
    };
    let whole = Ask {
        level: Level::Source,
        picked: None,
    };
    assert_eq!(first_ask(&program, Level::Source), whole);
    program.code = vec![0; WHOLE + 1];
    let first = first_ask(&program, Level::Structured);
    assert_eq!(first.level, Level::Structured);
    assert_eq!(first.picked, Some(BTreeSet::from([0x10])));

    // The functions left out, a line each, under one line that is no function.
    let text = stubs(&program, &BTreeSet::from([0x40]));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert_eq!(lines[2], "// main at 0x000010: not decompiled yet");
    assert_eq!(lines[3], "// func_000080 at 0x000080: not decompiled yet");
    assert!(stubs(&program, &BTreeSet::from([0x10, 0x40, 0x80])).is_empty());
    let doc = Gta6ScriptDocument::of(program, &text, Level::Source);
    let rows: Vec<(&str, Option<u32>, bool)> = (doc.outline.iter())
        .map(|r| (r.name.as_str(), r.at, r.class))
        .collect();
    assert_eq!(
        rows,
        [
            ("main", Some(0x10), false),
            ("func_000080", Some(0x80), false)
        ]
    );
}

#[test]
fn a_source_holds_the_functions_asked_and_says_what_stood_beside_the_program() {
    let none: Arc<[u8]> = Arc::from(Vec::new());
    let ask = Ask {
        level: Level::Structured,
        picked: Some(BTreeSet::from([0x40])),
    };
    let held = (Arc::new(Program::default()), None);
    let script = written(held, &none, &ask, None, None).expect("an empty program decompiles");
    // No function starts at the offset asked: the source holds none, and is not the whole program's.
    assert_eq!(script.picked, Some(BTreeSet::new()));
    assert_eq!(script.level, Level::Structured);
    assert!(script.read.project.is_none() && script.read.project_refused.is_none());
    let same = Ask {
        level: Level::Structured,
        picked: Some(BTreeSet::new()),
    };
    assert_eq!(script.ask(), same);
    let whole = Ask {
        level: Level::Structured,
        picked: None,
    };
    let again = (script.again(&whole, &Progress::default())).expect("it decompiles again");
    assert!(again.picked.is_none());
}

#[test]
fn the_folder_of_the_file_a_load_reads_is_where_a_project_is_looked_for() {
    // On a thread of its own, as a load is: the file is this thread's alone.
    let asked = std::thread::spawn(|| {
        let before = beside();
        reading(Path::new("demo_folder/demo_file"));
        (before, beside())
    });
    let (before, after) = asked.join().expect("the thread ends");
    assert_eq!(before, None);
    assert_eq!(after, Some(PathBuf::from("demo_folder")));
}

fn a_script_in(state: &mut AppState, script: Gta6ScriptDocument) -> DocumentId {
    let id = state.new_empty_document(crate::state::DocumentKind::Text);
    let d = state.document_mut(id).expect("the blank document");
    d.payload = DocPayload::Gta6Script(Arc::new(script));
    d.kind = crate::state::DocumentKind::Gta6Script;
    id
}

#[test]
fn a_script_opened_while_another_load_is_in_flight_waits_for_the_load_worker() {
    use crate::state::{DocumentKind, LoadResult};
    let mut state = AppState::default();
    let (tx, rx) = std::sync::mpsc::channel::<LoadResult>();
    state.load_receiver = Some(("demo_first".into(), rx));
    // Bytes that start as a program does: read here, they would be a document by now.
    state.open_bytes("demo_second".into(), DocumentKind::Hex, vec![0u8; 64]);
    state.open_bytes("demo_second".into(), DocumentKind::Hex, vec![0u8; 64]);
    assert!(state.documents.is_empty());
    // Bytes no script starts as are read at once, as they were.
    state.open_bytes("demo_third".into(), DocumentKind::Hex, vec![1u8; 64]);
    assert_eq!(state.documents.len(), 1);
    assert!(may_be_script(DocumentKind::Gta6Script, &[]));
    assert!(!may_be_script(DocumentKind::Text, &[0u8; 64]));

    // The worker is free once the first load ends: the script that waited is the load in flight.
    tx.send(LoadResult::Error {
        path: "demo_first".into(),
        message: "refused".into(),
    })
    .expect("the first load's result");
    state.poll_load_result();
    let in_flight = state.load_receiver.as_ref().map(|(path, _)| path.clone());
    assert_eq!(in_flight, Some(PathBuf::from("demo_second")));
    // It waited once, however often it was asked.
    assert!(state.next_waiting_gta6_script().is_none());
}

#[test]
fn a_script_decompiled_again_lands_as_asked_and_one_dropped_changes_nothing() {
    let mut state = AppState::default();
    let id = a_script_in(&mut state, document(SOURCE));
    let picked_of = |state: &AppState| match state.document(id).map(|d| &d.payload) {
        Some(DocPayload::Gta6Script(script)) => script.picked.clone(),
        _ => None,
    };
    let ask = Ask {
        level: Level::Source,
        picked: Some(BTreeSet::new()),
    };
    // Dropped: what the thread makes lands nowhere.
    state.redo_gta6_script(id, ask.clone(), None);
    assert!(state.gta6_script_job(id).is_some());
    state.drop_gta6_script_job(id);
    state.poll_gta6_scripts();
    assert_eq!(picked_of(&state), None);

    // Waited for: the source is the one asked for.
    state.redo_gta6_script(id, ask.clone(), None);
    let started = std::time::Instant::now();
    while state.gta6_script_job(id).is_some() {
        assert!(
            started.elapsed().as_secs() < 60,
            "the decompile never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        state.poll_gta6_scripts();
    }
    assert_eq!(picked_of(&state), Some(BTreeSet::new()));

    // A script that is closed, or is no script any more, frees its job at the next poll.
    state.redo_gta6_script(id, ask, None);
    let d = state.document_mut(id).expect("the document");
    d.payload = DocPayload::Hex(HexDocument {
        bytes: Vec::new(),
        banner: String::new(),
    });
    state.poll_gta6_scripts();
    assert!(state.gta6_script_job(id).is_none());
}

#[test]
fn a_function_picked_in_a_large_script_is_asked_with_the_picks_before_it() {
    let mut state = AppState::default();
    let row = |at: u32| FunctionRow {
        at,
        end: at + 4,
        ..FunctionRow::default()
    };
    let program = || Program {
        functions: vec![row(0x40), row(0x80)],
        ..Program::default()
    };
    let large = Gta6ScriptDocument {
        picked: Some(BTreeSet::new()),
        ..Gta6ScriptDocument::of(program(), "", Level::Source)
    };
    let id = a_script_in(&mut state, large);
    let asked = |state: &AppState, id: DocumentId| {
        let job = state.gta6_script_job(id)?;
        Some((job.ask.picked.clone(), job.focus))
    };
    // No function starts there: nothing is asked.
    state.pick_gta6_function(id, 0x44);
    assert_eq!(asked(&state, id), None);
    state.pick_gta6_function(id, 0x40);
    let one = Some(BTreeSet::from([0x40]));
    assert_eq!(asked(&state, id), Some((one, Some(0x40))));
    let first = Arc::clone(
        &state
            .gta6_script_job(id)
            .expect("the first pick's job")
            .progress,
    );
    // Still in flight: the next pick keeps the one before it, and the job it replaces is stopped.
    state.pick_gta6_function(id, 0x80);
    let two = Some(BTreeSet::from([0x40, 0x80]));
    assert_eq!(asked(&state, id), Some((two, Some(0x80))));
    assert!(first.stopped());
    state.drop_gta6_script_job(id);

    // A source that holds every function has nothing to pick.
    let whole = Gta6ScriptDocument::of(program(), "", Level::Source);
    let whole = a_script_in(&mut state, whole);
    state.pick_gta6_function(whole, 0x40);
    assert_eq!(asked(&state, whole), None);
}

/// A row of a names list whose hash is the 32-bit hash of its name in lower case.
fn proved_row(name: &str) -> String {
    let hash = or_io::hash::literal_string_hash(name.to_ascii_lowercase().as_bytes());
    format!("0x{hash:08X}\t{name}\n")
}

#[test]
fn the_names_folder_reads_a_proof_before_a_label_and_never_a_file_of_candidates() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let write = |name: &str, text: &str| {
        std::fs::write(folder.path().join(name), text).expect("a list is written");
    };
    let hash = or_io::hash::literal_string_hash(b"demoalpha");
    // First by its name, the label would leave the proof of the same hash refused.
    write(
        "a_label.tsv",
        &format!("program_hash\tname\n0x{hash:08X}\tdemo_label\n"),
    );
    write("b_proved.tsv", &proved_row("demoalpha"));
    write("demo-candidates.tsv", &proved_row("demobeta"));
    let read = lists::supplied(Some(folder.path()));
    assert_eq!(read.names.lists, ["b_proved.tsv"]);
    assert_eq!(read.names.program(u64::from(hash)), Some("demoalpha"));
    assert_eq!(read.named, [("b_proved.tsv".to_owned(), 1)]);
    assert_eq!(read.candidates, ["demo-candidates.tsv"]);
    assert_eq!(read.refused.len(), 1, "{:?}", read.refused);
    assert!(read.refused[0].starts_with("a_label.tsv: "));
    let beta = or_io::hash::literal_string_hash(b"demobeta");
    assert_eq!(read.names.program(u64::from(beta)), None);
}

#[test]
fn a_project_is_built_only_for_a_script_of_an_archive_and_a_pick_waits_for_it() {
    let mut state = AppState::default();
    let row = |at: u32| FunctionRow {
        at,
        end: at + 4,
        ..FunctionRow::default()
    };
    let program = Program {
        functions: vec![row(0x40)],
        ..Program::default()
    };
    let large = Gta6ScriptDocument {
        picked: Some(BTreeSet::new()),
        ..Gta6ScriptDocument::of(program, "", Level::Source)
    };
    // A program of a file of its own has no archive to build from.
    let alone = a_script_in(&mut state, large.clone());
    state.build_gta6_project(alone);
    assert!(state.gta6_script_job(alone).is_none());

    // The file is no archive, so the build fails on its thread; here only its job is looked at.
    let folder = tempfile::tempdir().expect("a scratch folder");
    let archive = folder.path().join("demo_archive.rpf");
    std::fs::write(&archive, [0u8; 16]).expect("written");
    let of_archive = Gta6ScriptDocument {
        archive: Some(archive.clone()),
        ..large
    };
    let id = a_script_in(&mut state, of_archive);
    state.build_gta6_project(id);
    let job = state.gta6_script_job(id).expect("the build's job");
    // A build begun by hand is the archive's one build of the session too.
    let offered = state.panels.peek::<super::jobs::Offered>();
    assert!(offered.is_some_and(|offered| offered.0.contains(&archive)));
    let building = Arc::clone(job.building.as_ref().expect("a job that builds"));
    assert!(!building.built());
    // A pick while the project is built is no new job.
    state.pick_gta6_function(id, 0x40);
    let job = state.gta6_script_job(id).expect("the build's job, still");
    assert!(job.building.is_some() && job.focus.is_none());
    let progress = Arc::clone(&job.progress);
    state.drop_gta6_script_job(id);
    assert!(progress.stopped());
}

#[test]
fn a_script_of_an_archive_decompiled_alone_has_its_project_built_once_as_its_job() {
    let mut state = AppState::default();
    let folder = tempfile::tempdir().expect("a scratch folder");
    let archive_at = |name: &str| {
        let archive = folder.path().join(name);
        std::fs::write(&archive, [0u8; 16]).expect("written");
        archive
    };
    let of = |archive: &PathBuf| Gta6ScriptDocument {
        archive: Some(archive.clone()),
        ..document("")
    };
    let archive = archive_at("demo_archive.rpf");
    // Decompiled with a project already: nothing to build.
    let mut with = of(&archive);
    with.read.project = Some(folder.path().join("project.json"));
    let read_with = a_script_in(&mut state, with);
    state.panels.get::<ScriptAsk>().project_wanted = true;
    state.build_wanted_gta6_project();
    assert!(state.gta6_script_job(read_with).is_none());

    // The file is no archive, so the build fails on its thread; here only its job is looked at.
    let id = a_script_in(&mut state, of(&archive));
    // Nothing is built unasked while the session's question was not answered to build.
    state.panels.get::<ScriptAsk>().project_wanted = false;
    state.build_wanted_gta6_project();
    assert!(state.gta6_script_job(id).is_none());
    state.panels.get::<ScriptAsk>().project_wanted = true;
    state.build_wanted_gta6_project();
    let job = state
        .gta6_script_job(id)
        .expect("the build begun for the script");
    assert!(job.building.is_some() && job.focus.is_none());
    // Another script of the archive waits for the build in flight, and is not built for again.
    let second = a_script_in(&mut state, of(&archive));
    state.build_wanted_gta6_project();
    assert!(state.gta6_script_job(second).is_none());
    state.drop_gta6_script_job(id);
    state.build_wanted_gta6_project();
    assert!(state.gta6_script_job(id).is_none() && state.gta6_script_job(second).is_none());

    // The poll that lands decompiles is what begins a build.
    let other = archive_at("demo_other.rpf");
    let third = a_script_in(&mut state, of(&other));
    state.poll_gta6_scripts();
    let offered = state.panels.peek::<super::jobs::Offered>();
    assert!(offered.is_some_and(|offered| offered.0.contains(&other)));
    state.drop_gta6_script_job(third);
}
