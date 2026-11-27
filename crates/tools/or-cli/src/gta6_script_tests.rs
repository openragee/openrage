use std::io::Read;

use or_io::container::rpf::RPF9_MAGIC;
use or_io::gta5::script::natives::NativeDb;
use or_io::gta6::script::program::{FieldRow, MethodRow, TypeRow};

use super::*;

/// A program hash that is no game's.
const PROGRAM: u32 = 0x0A0B_0C0D;

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

/// An instruction written by hand: the word of the opcode named, then each operand byte under the key of its place.
fn insn(name: &str, operands: &[u8]) -> Vec<u8> {
    let opcode = (0..512u16).find(|n| opcode::name(*n) == Some(name));
    let opcode = opcode.expect("an opcode of that name");
    let mut bytes = opcode.wrapping_mul(0x843D).to_le_bytes().to_vec();
    let keyed = operands.iter().zip(0u8..);
    bytes.extend(keyed.map(|(byte, n)| byte.wrapping_mul(0x65) ^ 0x85 ^ 0x11u8.wrapping_mul(n)));
    bytes
}

/// A program of two functions built here: `main` calls one native twice, the other returns its parameter.
fn program() -> Program {
    let native = insn("call_native", &[0x04, 0x03, 0x02, 0x01, 0, 0]);
    let first = [
        insn("enter", &[0x00]),
        native.clone(),
        native,
        insn("return", &[]),
    ];
    let second = [
        insn("enter", &[0x10]),
        insn("push_param_0", &[]),
        insn("return_1", &[]),
    ];
    let (first, second) = (first.concat(), second.concat());
    let split = first.len() as u32;
    let row = |at: u32, end: u32, cells: u32| FunctionRow {
        at,
        end,
        params: cells,
        locals: 4,
        returns: cells,
    };
    Program {
        program: PROGRAM,
        functions: vec![row(0, split, 0), row(split, split + second.len() as u32, 1)],
        code: [first, second].concat(),
        main: Some(0),
        ..Program::default()
    }
}

/// What a decompile of all of a program is asked with no project, no list and nothing more to say.
fn ask<'a>(names: &'a Names, options: Options, only: &'a [String]) -> Decompile<'a> {
    Decompile {
        names,
        project: None,
        project_said: "none",
        options,
        only,
        with_classes: false,
        said: &[],
    }
}

/// A reader of a file's first 64 bytes that fails when more is asked of it.
struct Head64<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Read for Head64<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= 64 {
            return Err(std::io::Error::other("read past the first 64 bytes"));
        }
        let n = buf.len().min(64 - self.at).min(self.bytes.len() - self.at);
        buf[..n].copy_from_slice(&self.bytes[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[test]
fn a_file_is_told_by_its_first_bytes_and_nothing_past_them_is_read() {
    let mut archive = RPF9_MAGIC.to_le_bytes().to_vec();
    archive.resize(4096, 0xAB);
    let head = Head64 {
        bytes: &archive,
        at: 0,
    };
    // An archive is tens of gigabytes: reading on past its head fails this reader.
    assert_eq!(source::head_of(head).ok(), Some(Head::Archive));

    let told = |bytes: &[u8]| source::head_of(bytes).ok();
    let zeros = [vec![0u8; 8], vec![0x44; 56]].concat();
    assert_eq!(told(&zeros), Some(Head::Program));
    let listing = b"// import 1: program_0x0A0B0C0D, types [2, 100)\r\n\tusing Demo;\n";
    assert_eq!(told(listing), Some(Head::Text));
    // GTA V's resource is GTA V's command's, whatever follows its magic.
    assert_eq!(told(b"RSC7\x0A\0\0\0 and the rest"), Some(Head::Resource));
    assert_eq!(
        told(b"RSC7 with nothing but text after it"),
        Some(Head::Resource)
    );
    // Seven zero bytes are not eight, and no text holds one.
    assert_eq!(told(&[0, 0, 0, 0, 0, 0, 0, 1, 2, 3]), Some(Head::Other));
    assert_eq!(told(&[]), Some(Head::Other));
}

#[test]
fn the_arguments_are_this_commands_only_for_a_file_that_starts_as_a_gta_vi_one_does() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    let program = write("program", &[vec![0u8; 8], vec![0x44; 56]].concat());
    let script = write("script.ysc", b"RSC7\x0A\0\0\0\0\0\0\0\0\0\0\0");
    let text = write("listing.ysc.cs", b"// import 1: program_0x0A0B0C0D\n");
    let missing = dir.path().join("missing");

    for verb in [Verb::Info, Verb::Disasm, Verb::Decompile, Verb::Check] {
        assert!(ours(verb, &program));
        assert!(ours(verb, &text));
        // A GTA V script, and a file that does not open, stay GTA V's command's.
        assert!(!ours(verb, &script));
        assert!(!ours(verb, &missing));
    }
    // A folder is a set of programs to `check` alone.
    assert!(ours(Verb::Check, dir.path()));
    assert!(!ours(Verb::Info, dir.path()));
    // `list` and `export` are no verbs of GTA V's command.
    assert!(ours(Verb::List, &script) && ours(Verb::Export, &missing));
}

#[test]
fn a_source_is_refused_by_what_its_first_bytes_say() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        vec![path.display().to_string()]
    };
    let refusal = |paths: &[String], entry: Option<&str>| {
        let refused = source::open(paths, entry, Keys::default());
        refused.err().unwrap_or_default()
    };

    let text = write("listing.ysc.cs", b"// import 1: program_0x0A0B0C0D\n");
    assert!(refusal(&text, None).ends_with("`script lifted` reads it"));

    // An archive with no entry named is refused before any key is looked for.
    let mut archive = RPF9_MAGIC.to_le_bytes().to_vec();
    archive.resize(64, 0);
    let archive = write("demo.rpf", &archive);
    assert!(refusal(&archive, None).contains("with --entry"));

    // Eight zero bytes only send a file to the program reader, whose refusal stands.
    let zeros = [vec![0u8; 8], vec![0x11; 56]].concat();
    let no_program = write("no_program", &zeros);
    assert!(refusal(&no_program, Some("0")).contains("is no archive"));
    let src =
        source::open(&no_program, None, Keys::default()).expect("it starts as a program does");
    assert_eq!(src.bytes, zeros);
    assert!(Program::read(&src.bytes).is_err());
}

#[test]
fn a_pass_is_left_off_by_name_and_only_a_pass_that_ran_is_said_to_have() {
    let on = options(&asked(&words(&["x"])));
    let off = options(&asked(&words(&[
        "x",
        "--no",
        "foreach",
        "--no",
        "structure",
    ])));
    assert!(on.on(Pass::Foreach) && on.on(Pass::Structure));
    assert!(!off.on(Pass::Foreach) && !off.on(Pass::Structure));
    // No pass goes but those named.
    assert!(off.on(Pass::Using));
    assert_eq!(left_off(off), ["structure", "foreach"]);

    // A name no pass reads is on and ran nothing: it is in no list of what ran.
    let all = Pass::ALL.into_iter();
    let built: Vec<&str> = all.filter(|pass| pass.rebuilds()).map(Pass::name).collect();
    assert_eq!(ran(on), built);
    assert!(!ran(off).contains(&"structure") && ran(off).contains(&"values"));

    let listed = passes();
    assert_eq!(listed.lines().count(), Pass::ALL.len());
    for (pass, line) in Pass::ALL.into_iter().zip(listed.lines()) {
        assert!(line.starts_with(pass.name()), "{line}");
        assert!(line.contains(pass.level().name()), "{line}");
        assert_eq!(
            line.ends_with("rebuilds nothing yet"),
            !pass.rebuilds(),
            "{line}"
        );
    }

    let structured = options(&asked(&words(&["x", "--level", "structured"])));
    assert_eq!(structured.level, Level::Structured);
    assert!(structured.on(Pass::Structure) && !structured.on(Pass::Foreach));
}

#[test]
fn a_source_has_one_header_and_this_commands_lines_end_it() {
    let (program, names) = (program(), Names::default());
    let (text, stats) =
        written(&program, &ask(&names, Options::default(), &[])).expect("the program decompiles");
    assert_eq!(stats.functions, 2);
    // The header is the comment lines the text opens with, and the printer wrote the first.
    let header: Vec<&str> = text.lines().take_while(|l| l.starts_with("//")).collect();
    assert!(header.len() > 1, "{text}");
    assert_eq!(header.last(), Some(&"// project: none"), "{text}");
    assert!(!header[0].starts_with("// project"), "{text}");
    for said_twice in ["// project: ", "// level: ", "// names: "] {
        let lines = header.iter().filter(|l| l.starts_with(said_twice));
        assert!(lines.count() <= 1, "{said_twice} in {text}");
    }

    // At the lowest level the printer writes a listing and no header: the first lines are this command's.
    let (text, _) = written(&program, &ask(&names, Options::at(Level::Lifted), &[]))
        .expect("the program lifts");
    let first = "// Readable source of a compiled GTA VI script, program 0x0A0B0C0D: ";
    assert!(text.starts_with(first), "{text}");
    assert!(
        text.contains("// level: lifted, at which no pass runs\n// project: none\n"),
        "{text}"
    );

    // What a caller has to say follows, each thing on the one line of its comment.
    let said = ["a list said this".to_owned(), "two\nlines".to_owned()];
    let more = Decompile {
        said: &said,
        ..ask(&names, Options::default(), &[])
    };
    let own = header_of(&program, &more);
    assert_eq!(own, "// project: none\n// a list said this\n// two lines\n");
    // A listing's reader takes a line that starts `// import ` for an import.
    assert!(
        (own.lines().chain(intro(&program, &names).lines()))
            .all(|line| !line.starts_with("// import ")),
        "{own}"
    );
}

/// The lines this command adds for all of `program`.
fn header_of(program: &Program, ask: &Decompile<'_>) -> String {
    header(program, ask, &[])
}

#[test]
fn a_name_no_pass_and_no_level_has_is_refused_with_the_names_there_are() {
    let wrong = asked(&words(&["x", "--no", "nonsense"]))
        .wrong
        .unwrap_or_default();
    assert!(wrong.starts_with("unknown pass `nonsense`"), "{wrong}");
    assert!(
        Pass::ALL.iter().all(|pass| wrong.contains(pass.name())),
        "{wrong}"
    );
    let wrong = asked(&words(&["x", "--level", "nowhere"]))
        .wrong
        .unwrap_or_default();
    let levels = Level::ALL.map(Level::name).join(", ");
    assert!(
        wrong.ends_with(&format!("it is one of: {levels}")),
        "{wrong}"
    );
    assert!(asked(&words(&["x", "--no", "foreach"])).wrong.is_none());

    // The first thing wrong is the one reported, and a value is never taken for a path.
    let both = asked(&words(&["x", "--bogus", "--threads", "0", "--entry"]));
    assert_eq!(both.wrong.as_deref(), Some("unknown option: --bogus"));
    assert_eq!(both.paths, ["x"]);
    let wrong = |args: &[&str]| asked(&words(args)).wrong.unwrap_or_default();
    assert_eq!(
        wrong(&["x", "--threads", "0"]),
        "--threads takes a number above 0, not `0`"
    );
    assert_eq!(
        wrong(&["x", "--limit", "0"]),
        "--limit takes a number above 0, not `0`"
    );
    assert_eq!(
        wrong(&["x", "--offset", "some"]),
        "--offset takes a number, not `some`"
    );
    // GTA V's option is known, and answered with what a GTA VI program holds instead.
    assert_eq!(wrong(&["x", "--statics"]), STATICS);
}

#[test]
fn the_page_the_exe_and_the_two_switches_are_read_and_an_option_a_verb_ignores_is_named() {
    let paged = asked(&words(&[
        "x",
        "--functions",
        "--offset",
        "0",
        "--limit",
        "3",
        "--exe",
        "game.exe",
        "--keys",
        "keys.dat",
        "--with-classes",
        "--alone",
    ]));
    assert!(paged.wrong.is_none(), "{:?}", paged.wrong);
    assert_eq!((paged.page.offset, paged.page.limit), (0, Some(3)));
    assert!(paged.paged && paged.with_classes && paged.alone);
    let keys = paged.keys();
    assert_eq!(keys.exe, Some(Path::new("game.exe")));
    assert_eq!(keys.snapshot, Some(Path::new("keys.dat")));

    assert_eq!(unused(Verb::Info, &paged), ["--with-classes", "--alone"]);
    assert_eq!(
        unused(Verb::Check, &paged),
        [
            "--natives, --strings or --functions",
            "--with-classes",
            "--offset or --limit",
            "--alone"
        ]
    );
    let plain = asked(&words(&["x", "--json", "--function", "f", "--offset", "1"]));
    assert_eq!(
        unused(Verb::Decompile, &plain),
        ["--json", "--offset or --limit"]
    );
    assert_eq!(
        unused(Verb::Info, &plain),
        ["--function", "--offset or --limit"]
    );
    assert!(unused(Verb::Decompile, &asked(&words(&["x", "--alone"]))).is_empty());
}

#[test]
fn a_function_is_named_by_its_offset_or_its_name_and_a_name_two_carry_names_neither() {
    let program = program();
    let mut names = Names::default();
    let [first, second] = [program.functions[0], program.functions[1]];
    let by = |name: String, names: &Names| selected(&program, names, &[name]);

    assert_eq!(by(format!("0x{:X}", second.at), &names), Ok(vec![second]));
    assert_eq!(
        by(format!("func_{:06X}", second.at), &names),
        Ok(vec![second])
    );
    assert_eq!(by("main".to_owned(), &names), Ok(vec![first]));
    // The offset as a listing prints it, with nothing before its digits.
    assert_eq!(by(format!("{:06X}", second.at), &names), Ok(vec![second]));
    let none = by("func_000001".to_owned(), &names)
        .err()
        .unwrap_or_default();
    assert!(none.starts_with("no function `func_000001`"), "{none}");

    let key = |row: FunctionRow| FunctionKey::new(PROGRAM, row.at);
    names.functions.insert(key(second), "Run".to_owned());
    assert_eq!(by("Run".to_owned(), &names), Ok(vec![second]));
    names.functions.insert(key(first), "Run".to_owned());
    let twice = by("Run".to_owned(), &names).err().unwrap_or_default();
    assert!(twice.contains("names 2 functions"), "{twice}");
    assert!(
        twice.contains(&format!("0x000000, 0x{:06X}", second.at)),
        "{twice}"
    );
}

#[test]
fn a_listing_is_cut_to_the_functions_asked_for() {
    let (program, names) = (program(), Names::default());
    let second = format!("func_{:06X}", program.functions[1].at);

    let whole = listing(&program, &names, &[]).expect("the program decodes");
    let first_line = "; program 0x0A0B0C0D: 28 byte(s) of code; 7 instruction(s) in 2 function(s) \
                      listed\n";
    assert!(whole.starts_with(first_line), "{whole}");
    assert!(whole.contains("\nfunc_000000: "), "{whole}");
    assert!(
        whole.contains(&format!("\n{second}: 1 parameter cell(s)")),
        "{whole}"
    );

    let one =
        listing(&program, &names, std::slice::from_ref(&second)).expect("the program decodes");
    assert!(
        one.contains("; 3 instruction(s) in 1 function(s) listed\n"),
        "{one}"
    );
    assert!(one.contains(&format!("\n{second}: ")), "{one}");
    assert!(
        one.contains("return_1") && !one.contains("call_native"),
        "{one}"
    );
    assert!(!one.contains("func_000000"), "{one}");
}

#[test]
fn a_native_is_counted_by_its_handlers_address_and_an_instruction_past_its_function_is_refused() {
    let mut program = program();
    let walk = walked(&program).expect("every byte is an instruction");
    assert_eq!(walk.instructions, 7);
    let called: Vec<_> = walk.natives.into_iter().collect();
    assert_eq!(called, [(0x0102_0304, (0, 0, 2))]);
    let mut names = Names::default();
    let printed = natives(&program, &names, false).expect("the program decodes");
    assert!(
        printed.starts_with("rva_01020304  0 -> 0  2 site(s)\n"),
        "{printed}"
    );
    assert!(printed.contains("\n0 of them have the hash "), "{printed}");

    // A names list gives the handler's hash, and a natives list the name of that hash.
    names.native_hashes.insert(0x0102_0304, 0xA1);
    let printed = natives(&program, &names, false).expect("the program decodes");
    let hashed = "rva_01020304  0 -> 0  2 site(s)  0x00000000000000A1\n";
    assert!(printed.starts_with(hashed), "{printed}");
    let list = NativeDb::parse("demo", b"0x00000000000000A1 DEMO::GET_DEMO_THING\n");
    names.natives = list.ok();
    let printed = natives(&program, &names, false).expect("the program decodes");
    let named = "rva_01020304  0 -> 0  2 site(s)  0x00000000000000A1  DEMO::GET_DEMO_THING\n";
    assert!(printed.starts_with(named), "{printed}");
    assert!(
        printed.contains("\n1 of them have the hash a names list gives their handler, and 1 of "),
        "{printed}"
    );
    let rows = natives(&program, &names, true).expect("the program decodes");
    let rows: Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(rows[0]["rva"], "0x01020304");
    assert_eq!(rows[0]["hash"], "0x00000000000000A1");
    assert_eq!(rows[0]["name"], "DEMO::GET_DEMO_THING");
    let rows = natives(&program, &Names::default(), true).expect("the program decodes");
    let rows: Value = serde_json::from_str(&rows).unwrap();
    assert!(rows[0]["hash"].is_null() && rows[0]["name"].is_null());

    // An end inside the last instruction is refused, not read past.
    program.functions[1].end -= 1;
    let refused = walked(&program).err();
    assert!(matches!(
        refused,
        Some(decode::DisasmError::Malformed { .. })
    ));
}

#[test]
fn what_is_printed_of_a_program_names_what_a_list_names_and_says_when_none_did() {
    let program = program();
    let src = source::Source {
        bytes: vec![0; 64],
        read_from: "demo".to_owned(),
    };
    let mut names = Names::default();
    let printed = info(&src, &program, &names, false);
    let head = "\
program           0x0A0B0C0D
read from         demo
size              64 byte(s)
code              28 byte(s) in 2 function(s)
";
    assert!(printed.starts_with(head), "{printed}");
    let counted = "natives           1 handler(s) called at 2 site(s)\ninstructions      7\n";
    assert!(printed.contains(counted), "{printed}");
    assert!(
        printed.contains("\nNames: no names list supplied"),
        "{printed}"
    );

    let read = names.read_list("names.tsv", "0x0A0B0C0D\tdemo_program\n");
    assert!(read.is_ok(), "{read:?}");
    let printed = info(&src, &program, &names, false);
    assert!(
        printed.starts_with("program           0x0A0B0C0D (demo_program)\n"),
        "{printed}"
    );
    assert!(printed.contains("\nNames: from `names.tsv`"), "{printed}");
    assert!(!printed.contains("Supply lists with --names"), "{printed}");

    let split = program.functions[1].at;
    let listed = format!(
        "0x000000  {split} byte(s)  0 -> 0  main\n0x{split:06X}  7 byte(s)  1 -> 1  \
         func_{split:06X}\n"
    );
    let whole = Page::default();
    assert_eq!(functions(&program, &names, false, whole), (listed, None));
    let (rows, said) = functions(&program, &names, true, whole);
    assert!(said.is_none());
    let parsed: Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(parsed.as_array().map(Vec::len), Some(2));
    assert_eq!(parsed[0]["name"], "main");
    assert_eq!(parsed[1]["at"], format!("0x{split:06X}"));
    // A function is one line, so a list cut short loses whole functions.
    assert_eq!(rows.lines().count(), 4, "{rows}");

    let page = |offset: usize, limit: Option<usize>| {
        functions(&program, &names, true, Page { offset, limit })
    };
    let (rows, said) = page(0, Some(1));
    let parsed: Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(parsed.as_array().map(Vec::len), Some(1));
    assert_eq!(parsed[0]["name"], "main");
    let said = said.unwrap_or_default();
    assert!(said.contains("functions 0 to 0 of 2"), "{said}");
    assert!(said.ends_with("--offset 1 prints the next"), "{said}");
    let (rows, said) = page(1, Some(5));
    let parsed: Value = serde_json::from_str(&rows).unwrap();
    assert_eq!(parsed[0]["at"], format!("0x{split:06X}"));
    let said = said.unwrap_or_default();
    assert!(
        said.ends_with("functions 1 to 1 of 2, numbered from 0: the last of them"),
        "{said}"
    );
    let (rows, said) = page(7, None);
    assert_eq!(rows, "[]\n");
    assert!(said
        .unwrap_or_default()
        .contains("--offset 7 is past the last"));
    let (text, said) = functions(
        &program,
        &names,
        false,
        Page {
            offset: 1,
            limit: None,
        },
    );
    assert_eq!(
        text,
        format!("0x{split:06X}  7 byte(s)  1 -> 1  func_{split:06X}\n")
    );
    assert!(said.is_some());
    let parsed: Value = serde_json::from_str(&info(&src, &program, &names, true)).unwrap();
    assert_eq!(parsed["name"], "demo_program");
    assert_eq!(parsed["instructions"], 7);
}

#[test]
fn a_record_is_named_only_where_a_list_names_the_hash_it_holds() {
    let owner = TypeKey(0x0123_4567_89AB_CDEF);
    let field = |offset: u32, name: u64| FieldRow {
        offset,
        name,
        ..FieldRow::default()
    };
    let method = |name: u32, slot: Option<u32>, owner: u32| MethodRow {
        name,
        slot,
        owner,
        ..MethodRow::default()
    };
    let ty = TypeRow {
        name: owner.0,
        fields: vec![field(0x10, 0xF1), field(0x18, 0xF2)],
        ..TypeRow::default()
    };
    // The third record fills no slot, and the fourth belongs to a type the program does not hold.
    let methods = vec![
        method(0xA1, Some(3), 0),
        method(0xA2, Some(4), 0),
        method(0xA1, None, 0),
        method(0xA1, Some(9), 7),
    ];
    let program = Program {
        types: vec![ty],
        methods,
        ..Program::default()
    };
    let mut names = Names::default();
    names.field_names.insert(0xF1, "_tint".to_owned());
    names.method_names.insert(0xA1, "Start".to_owned());

    bind(&mut names, &program);
    assert_eq!(names.field(owner, 0x10), Some("_tint"));
    assert_eq!(names.field(owner, 0x18), None);
    assert_eq!(names.method(owner, 3), Some("Start"));
    assert_eq!(names.method(owner, 4), None);
    assert_eq!((names.methods.len(), names.fields.len()), (1, 1));
}

#[test]
fn one_function_is_decompiled_alone_under_first_lines_that_name_it() {
    let (program, names) = (program(), Names::default());
    let second = format!("func_{:06X}", program.functions[1].at);
    let only = [second.clone()];
    let (text, stats) = written(&program, &ask(&names, Options::default(), &only))
        .expect("the function decompiles");
    assert_eq!(stats.functions, 1);
    assert!(
        text.contains(&second) && !text.contains("func_000000"),
        "{text}"
    );
    let header: Vec<&str> = text.lines().take_while(|l| l.starts_with("//")).collect();
    let named = format!("// only: {second}");
    assert!(header.contains(&named.as_str()), "{text}");

    let none = [String::from("func_000001")];
    let refused = written(&program, &ask(&names, Options::default(), &none));
    assert!(refused.is_err_and(|e| e.starts_with("no function `func_000001`")));
}

#[test]
fn a_type_that_holds_no_code_of_the_functions_asked_for_is_left_out_and_counted() {
    let kept = "\
// a header
// of two lines

void func_000010(Cell p0)
{
}

class type_A : type_B
{
    void* f_8;

    virtual void Method_4() // func_000020
    {
    }
}
";
    let data = "
enum type_C : type_D // base type_E
{
    // values: 0, 1
}

class type_F
    : type_G, type_H
{
    Cell f_10;
}
";
    let bodiless = "
struct type_I
{
    // func_000030: no body (not lifted: a reason)
}
";
    let text = format!("{kept}{data}{bodiless}");
    let marks = ["func_000030".to_owned()];
    // A type with a method's body stays, and so does one that only names a function asked for.
    assert_eq!(
        without_data_types(&text, &marks),
        (format!("{kept}{bodiless}"), 2)
    );
    assert_eq!(without_data_types(&text, &[]), (kept.to_owned(), 3));

    // What does not close as the printer closes a type is no type here: not a line of it goes.
    let open = "class type_J\n{\n    Cell f_0;\nvoid stray()\n{\n}\n";
    assert_eq!(without_data_types(open, &[]), (open.to_owned(), 0));
    let one_line = "enum type_K { }\n\nclass type_L;\n\nvoid main()\n{\n}\n";
    assert_eq!(without_data_types(one_line, &[]), (one_line.to_owned(), 0));
    assert_eq!(without_data_types("", &[]), (String::new(), 0));
}

#[test]
fn a_program_with_no_project_named_reads_the_one_beside_it_and_says_what_gives_one() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program");
    std::fs::write(&program, [vec![0u8; 8], vec![0x44; 56]].concat()).unwrap();
    let path = program.display().to_string();
    let held_for = |more: &[&str]| {
        let args = [&[path.as_str()][..], more].concat();
        held(&asked(&words(&args)))
    };

    let alone = held_for(&[]).expect("no project is no failure");
    assert!(alone.project.is_none() && alone.unread.is_none());
    assert!(
        alone
            .said
            .starts_with("none: the program was decompiled alone"),
        "{}",
        alone.said
    );
    assert!(
        alone.said.contains("`script export <folder> -o <out>`"),
        "{}",
        alone.said
    );
    assert!(
        alone
            .said
            .ends_with("which --project <out>/project.json reads"),
        "{}",
        alone.said
    );

    // A file of that name that is no project is said and not read, and nothing fails.
    let beside = dir.path().join(PROJECT_FILE);
    std::fs::write(&beside, "not a project").unwrap();
    let unread = held_for(&[]).expect("a file nobody named is no failure");
    assert!(unread.project.is_none());
    let why = unread.unread.unwrap_or_default();
    assert!(
        why.starts_with("`project.json` beside the program is no project"),
        "{why}"
    );
    assert!(unread.said.contains(&why), "{}", unread.said);
    // Named, the same file is a failure.
    let named = beside.display().to_string();
    assert!(held_for(&["--project", named.as_str()]).is_err());

    let empty = index::save(&index::merge(Vec::new())).expect("an empty project saves");
    std::fs::write(&beside, empty).unwrap();
    let found = held_for(&[]).expect("the project reads");
    assert!(found.project.is_some() && found.unread.is_none());
    assert_eq!(
        found.said,
        "`project.json`, found beside the program; --alone decompiles without it"
    );
    let left = held_for(&["--alone"]).expect("alone is no failure");
    assert!(left.project.is_none());
    assert!(
        left.said.starts_with("none, as --alone asks: "),
        "{}",
        left.said
    );
    // An entry of an archive lies beside no export.
    assert!(held_for(&["--entry", "0"]).is_ok_and(|held| held.project.is_none()));
}

#[test]
fn stderr_is_told_how_many_functions_have_no_body_and_what_gives_them_one() {
    let stats = |incomplete: usize| Stats {
        functions: 5,
        incomplete,
        ..Stats::default()
    };
    assert_eq!(bodies_note(&stats(0), false), None);
    let alone = bodies_note(&stats(2), false).unwrap_or_default();
    assert!(
        alone.starts_with("note: 2 of 5 function(s) have no body; "),
        "{alone}"
    );
    assert!(
        alone.contains("`script export <folder> -o <out>`"),
        "{alone}"
    );
    let with_project = bodies_note(&stats(2), true).unwrap_or_default();
    assert!(with_project.starts_with("note: 2 of 5 function(s) have no body; "));
    assert!(!with_project.contains("--project"), "{with_project}");
}

#[test]
fn a_folder_stands_for_the_files_under_it_that_start_as_a_program_does() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("archive");
    std::fs::create_dir(&nested).unwrap();
    // Eight zero bytes and nothing a program holds after them: found, read and refused.
    std::fs::write(nested.join("body"), [vec![0u8; 8], vec![0x11; 56]].concat()).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "not a program\n").unwrap();

    let folder = dir.path().display().to_string();
    let (printed, clean) = check(&[folder]).expect("the folder reads");
    assert!(!clean);
    assert!(printed.ends_with("0 of 1 script(s) decode.\n"), "{printed}");
    assert!(
        printed.contains("body: ") && !printed.contains("notes.txt"),
        "{printed}"
    );

    // A file named on its own is told what it is, and is no program either.
    let notes = dir.path().join("notes.txt").display().to_string();
    let (printed, clean) = check(&[notes]).expect("the file reads");
    assert!(!clean && printed.contains(source::TEXT), "{printed}");

    let empty = tempfile::tempdir().unwrap();
    assert!(check(&[empty.path().display().to_string()]).is_err());
}

#[test]
fn each_file_of_a_check_goes_to_the_reader_its_first_bytes_ask_for_whatever_the_order() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    };
    let program = write("program", &[vec![0u8; 8], vec![0x11; 56]].concat());
    let script = write("script.ysc", b"RSC7 and nothing a script holds");
    let other = write("other.bin", &[0xFF, 0xFE, 0x00, 0x01]);

    let (first, clean) = check(&[script.clone(), program.clone()]).expect("the files read");
    assert!(!clean);
    let (second, _) = check(&[program.clone(), script.clone()]).expect("the files read");
    let sorted = |text: &str| {
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines.sort();
        lines
    };
    assert_eq!(sorted(&first), sorted(&second));
    assert!(first.ends_with("0 of 2 script(s) decode.\n"), "{first}");
    // GTA V's script is refused by GTA V's reader, in its words, and not for its eight first bytes.
    let of_script = first.lines().find(|line| line.starts_with(&script));
    let of_script = of_script.unwrap_or_default();
    assert!(
        !of_script.is_empty() && !of_script.contains("eight zero bytes"),
        "{first}"
    );
    let (third, _) = check(std::slice::from_ref(&other)).expect("the file reads");
    assert!(
        third.contains("neither a GTA V script, which starts `RSC7`"),
        "{third}"
    );

    // One GTA VI file anywhere among them makes a check this command's; any other verb is told by the first.
    assert!(claimed(Verb::Check, &[script.clone(), program.clone()]));
    assert!(!claimed(Verb::Check, &[script.clone(), other]));
    assert!(!claimed(Verb::Info, &[script, program.clone()]));
    assert!(claimed(Verb::Info, &[program]));
}

#[test]
fn project_auto_is_read_as_a_build_in_memory_and_the_last_project_named_is_taken() {
    let auto = asked(&words(&["x", "--project", "auto", "--threads", "3"]));
    assert!(auto.wrong.is_none(), "{:?}", auto.wrong);
    assert!(auto.auto && auto.project.is_none());
    // --threads says something to a decompile that builds a project, and to no other.
    assert!(unused(Verb::Decompile, &auto).is_empty());
    assert!(unused(Verb::Disasm, &auto).is_empty());
    assert_eq!(unused(Verb::Info, &auto), ["--project", "--threads"]);
    let file = asked(&words(&["x", "--project", "./auto", "--threads", "3"]));
    assert!(!file.auto && file.project.as_deref() == Some(Path::new("./auto")));
    assert_eq!(unused(Verb::Decompile, &file), ["--threads"]);
    let last = asked(&words(&["x", "--project", "auto", "--project", "p.json"]));
    assert!(!last.auto && last.project.is_some());
    let last = asked(&words(&["x", "--project", "p.json", "--project", "auto"]));
    assert!(last.auto && last.project.is_none());

    // The programs are those of the folder a file lies in, or of the archive an entry is read from.
    let root = |args: &[&str]| auto_root(&asked(&words(args)));
    assert_eq!(root(&["dir/program"]), Ok(PathBuf::from("dir")));
    assert_eq!(root(&["program"]), Ok(PathBuf::from(".")));
    let entry = root(&["game.rpf", "inner", "--entry", "0"]);
    assert_eq!(entry, Ok(PathBuf::from("game.rpf")));
    assert!(root(&["a", "b"]).is_err());
}

#[test]
fn a_project_built_in_memory_reads_the_folder_beside_the_program_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    // Eight zero bytes and nothing a program holds after them: read, and no part of the project.
    let body = [vec![0u8; 8], vec![0x11; 56]].concat();
    std::fs::write(dir.path().join("first"), &body).unwrap();
    std::fs::write(dir.path().join("second"), &body).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "not a program\n").unwrap();
    assert_eq!(auto_bytes(dir.path()), Ok(2 * body.len() as u64));
    let program = dir.path().join("first").display().to_string();
    let ask = asked(&words(&[
        program.as_str(),
        "--project",
        "auto",
        "--threads",
        "1",
    ]));
    let mut names = Names::default();
    let held = auto_project(&ask, &mut names).expect("the folder builds");
    assert!(held.project.is_some() && held.unread.is_none());
    assert!(
        held.said.starts_with("built in memory by --project auto"),
        "{}",
        held.said
    );
    assert!(
        held.said.contains("from the 2 program(s) of `"),
        "{}",
        held.said
    );
    assert!(
        held.said.contains("2 of which gave it no part"),
        "{}",
        held.said
    );
    // A folder of no program builds nothing, and the reason stands.
    let empty = tempfile::tempdir().unwrap();
    let lone = empty.path().join("lone");
    std::fs::write(&lone, "text\n").unwrap();
    let lone = lone.display().to_string();
    let ask = asked(&words(&[lone.as_str(), "--project", "auto"]));
    let none = auto_project(&ask, &mut names).err().unwrap_or_default();
    assert!(
        none.ends_with("holds no GTA VI program to build a project of"),
        "{none}"
    );
}
