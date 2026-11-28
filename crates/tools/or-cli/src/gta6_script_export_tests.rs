use or_io::gta6::script::Pass;
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
        let asks = Asks {
            options: Options::default(),
            threads: 2,
            said: &[],
            summary: false,
        };
        exported(&bodies, read, Names::default(), asks, out)
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
    for file in [PROJECT_FILE, SCRIPTS_FILE, INDEX_FILE] {
        let written = std::fs::read(first.path().join(file)).unwrap();
        let again = std::fs::read(second.path().join(file)).unwrap();
        assert_eq!(written, again, "{file}");
    }
    // A body that failed left its row, and no source.
    assert_eq!(std::fs::read_dir(first.path()).unwrap().count(), 3);
    let scripts = std::fs::read(first.path().join(SCRIPTS_FILE)).unwrap();
    let scripts: Value = serde_json::from_slice(&scripts).unwrap();
    assert_eq!(scripts["level"], "source");
    assert_eq!(scripts["off"], Value::Array(Vec::new()));
    // Only a pass that ran is said to have: a name no pass reads yet is in neither list.
    let ran: Vec<Value> = (Pass::ALL.into_iter())
        .filter(|pass| pass.rebuilds())
        .map(|pass| Value::from(pass.name()))
        .collect();
    assert_eq!(scripts["passes"], Value::Array(ran));
    assert_eq!(scripts["programs"].as_array().map(Vec::len), Some(2));
    let row = &scripts["programs"][0];
    assert_eq!(
        (&row["read"], &row["file"]),
        (&Value::from("a"), &Value::Null)
    );
    let status = row["status"].as_str().unwrap_or_default();
    assert!(status.starts_with("failed: "), "{status}");
    assert_eq!(row["classes"], Value::from(0));
    assert_eq!(row["not_lifted"], Value::Array(Vec::new()));

    // The index has a row a body, each saying why it failed, under its head, and the last line under them.
    let index = std::fs::read_to_string(first.path().join(INDEX_FILE)).unwrap();
    let rows: Vec<&str> = index
        .lines()
        .filter(|line| line.starts_with("| "))
        .collect();
    assert_eq!(rows.len(), 3, "{index}");
    assert!(
        rows[0].starts_with("| Source | Program | Name |"),
        "{index}"
    );
    assert!(rows[1].starts_with("| - | "), "{index}");
    assert!(rows[1].contains(" | - | failed: `"), "{index}");
    assert!(index.contains("no names list supplied"), "{index}");
    assert!(index.trim_end().ends_with(lines[2].as_str()), "{index}");
}

#[test]
fn a_summary_prints_the_failures_and_the_totals_as_a_table_in_place_of_a_line_a_program() {
    let zeros = [vec![0u8; 8], vec![0x11; 56]].concat();
    let bodies = [body("a", zeros)];
    let asks = Asks {
        options: Options::default(),
        threads: 1,
        said: &[],
        summary: true,
    };
    let (lines, failed) = exported(&bodies, "a folder", Names::default(), asks, None).unwrap();
    assert_eq!(failed, 1);
    assert!(lines[0].starts_with("a: failed: "), "{lines:?}");
    assert!(
        !lines.iter().any(|line| line.starts_with("total: ")),
        "{lines:?}"
    );
    let row = |what: &str| lines.iter().find(|line| line.starts_with(what)).cloned();
    // Every row is as wide as the widest, its count at the right.
    let programs = row("programs ").expect("a row of programs");
    assert!(programs.ends_with(" 1"), "{programs}");
    assert!(
        row("failed ").is_some_and(|line| line.ends_with(" 1")),
        "{lines:?}"
    );
    assert!(
        row("classes ").is_some_and(|line| line.ends_with(" 0")),
        "{lines:?}"
    );
    let widths: BTreeSet<usize> = lines[1..].iter().map(String::len).collect();
    assert_eq!(widths.len(), 1, "{lines:?}");

    let mut total = Stats {
        functions: 12,
        ..Stats::default()
    };
    total.constructs[0] = 3;
    let table = table(2, 0, 1, 4, &total);
    let functions = |line: &String| line.starts_with("functions ") && line.ends_with(" 12");
    assert!(table.iter().any(functions), "{table:?}");
    let read = format!("constructs read: {}", CONSTRUCTS[0]);
    assert!(table
        .iter()
        .any(|line| line.starts_with(&read) && line.ends_with(" 3")));
    // A construct none was read of has no row.
    let other = format!("constructs read: {}", CONSTRUCTS[1]);
    assert!(
        !table.iter().any(|line| line.starts_with(&other)),
        "{table:?}"
    );
}

#[test]
fn a_source_counts_its_classes_and_names_each_function_it_writes_with_no_body() {
    let text = "\
// project: `project.json`
// left out: 4 byte(s) from 000040
class Demo : Base
{
    class Inner
    {
    }
    void Run()
    {
        if (x)
        {
        }
    }
}

public static class Program_0A0B0C0D // 0x000000
{
}

struct Pair
{
}
class Wide : Base,
    DemoSecond,
    DemoThird
{
}
class Cut : Base,
    DemoSecond
enum Kind
{
}
var x = new class
{
}
    // func_000040: no body (a demo reason: of words)
// demo_run at 0x000080: the listing holds no body (another)
// func_0000C0: a note
// a note at 0x10: no body (it names no function)
";
    assert_eq!(classes_in(text), 4);
    assert_eq!(unlifted_in(text), ["func_000040", "demo_run"]);
    assert_eq!(classes_in("class Demo\n  {\n"), 0);

    let wrote = |incomplete: usize, unlifted: Vec<String>| Wrote {
        stats: Stats {
            incomplete,
            ..Stats::default()
        },
        classes: 0,
        unlifted,
    };
    assert_eq!(unlifted(&wrote(0, Vec::new())), "none");
    assert_eq!(unlifted(&wrote(1, Vec::new())), "1");
    let many: Vec<String> = (0..NAMED_UNLIFTED + 2)
        .map(|n| format!("demo_{n}"))
        .collect();
    let said = unlifted(&wrote(many.len(), many));
    let first = format!("{}: `demo_0`, `demo_1`, ", NAMED_UNLIFTED + 2);
    assert!(said.starts_with(&first), "{said}");
    assert!(
        said.ends_with(&format!("`demo_{}` and 2 more", NAMED_UNLIFTED - 1)),
        "{said}"
    );
}

#[test]
fn a_cell_of_the_index_is_a_code_span_on_one_line_whatever_text_it_holds() {
    assert_eq!(code("demo_program"), "`demo_program`");
    assert_eq!(code("a|b"), "`a\\|b`");
    assert_eq!(code("one\ntwo"), "`one two`");
    assert_eq!(code("a`b"), "``a`b``");
    assert_eq!(code("`a"), "`` `a ``");
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

#[test]
fn a_file_of_a_folder_that_starts_as_no_program_does_is_counted_and_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let zeros = [vec![0u8; 8], vec![0x11; 56]].concat();
    std::fs::write(dir.path().join("body"), &zeros).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "not a program\n").unwrap();
    std::fs::write(dir.path().join("other.bin"), [0xFFu8, 0xFE, 0x00, 0x01]).unwrap();

    let (read, found) =
        source::gathered(dir.path(), source::Keys::default()).expect("the folder reads");
    assert_eq!(read, "a folder of deciphered programs");
    assert_eq!((found.bodies.len(), found.passed_over), (1, 2));
    assert_eq!(found.bodies[0].label, "body");
}

#[test]
fn the_largest_items_start_first_and_each_result_stands_in_its_items_place() {
    use std::sync::Mutex;
    let sizes = [3usize, 9, 1, 9, 5];
    let started = Mutex::new(Vec::new());
    let each = |size: &usize| {
        started.lock().unwrap().push(*size);
        size * 10
    };
    // One thread takes the items in the order they are started.
    let done = largest_first(&sizes, |size| *size, 1, each);
    assert_eq!(done, [30, 90, 10, 90, 50]);
    assert_eq!(*started.lock().unwrap(), [9, 9, 5, 3, 1]);
    // On several threads each result still stands in its item's place.
    assert_eq!(
        largest_first(&sizes, |size| *size, 4, |size| size * 10),
        done
    );
    assert!(largest_first(&[] as &[usize], |size| *size, 4, |size| *size).is_empty());
}

#[test]
fn a_project_built_in_memory_says_how_far_each_pass_is_and_counts_what_gave_no_part() {
    use std::sync::Mutex;
    // Eight zero bytes start a program, and what follows them here is none.
    let zeros = [vec![0u8; 8], vec![0x11; 56]].concat();
    let bodies = [body("a", zeros.clone()), body("b", zeros)];
    let lines = Mutex::new(Vec::new());
    let said = |line: String| lines.lock().unwrap().push(line);
    let mut names = Names::default();
    // On one thread, so that the lines come in the order the programs are done.
    let (project, none) = in_memory(&bodies, &mut names, 1, &said);
    assert_eq!(none, 2);
    assert!(project.programs.is_empty());
    let lines = lines.into_inner().unwrap();
    assert_eq!(
        lines,
        [
            "first reading: 2 program(s) to read",
            "first reading: 1 of 2 program(s) done",
            "first reading: 2 of 2 program(s) done",
            "second reading with the project: 2 program(s) to read",
            "second reading with the project: 1 of 2 program(s) done",
            "second reading with the project: 2 of 2 program(s) done",
        ]
    );
}
