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
    let names = supplied(Some(folder.path().to_path_buf()));
    assert_eq!(names.lists, ["b_names.txt"]);
    assert_eq!(names.program(0x0E0F_1011), Some("other_program"));
    assert_eq!(names.program(0x0A0B_0C0D), None);
    assert!(names.natives.is_none());
    assert!(supplied(None).lists.is_empty());
    assert!(supplied(Some(folder.path().join("missing")))
        .lists
        .is_empty());
}
