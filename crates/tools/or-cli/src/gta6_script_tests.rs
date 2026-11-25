use std::io::Read;

use or_io::container::rpf::RPF9_MAGIC;
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
    let native = insn("call_native", &[0x50, 0xE4, 0xC0, 0x03, 0, 0]);
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
    assert_eq!(told(b"RSC7\x0A\0\0\0 and the rest"), Some(Head::Other));
    assert_eq!(
        told(b"RSC7 with nothing but text after it"),
        Some(Head::Other)
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
        source::open(paths, entry, None).err().unwrap_or_default()
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
    let src = source::open(&no_program, None, None).expect("it starts as a program does");
    assert_eq!(src.bytes, zeros);
    assert!(Program::read(&src.bytes).is_err());
}

#[test]
fn a_pass_is_left_off_by_name_and_the_first_lines_say_so() {
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

    let (program, names) = (program(), Names::default());
    let said = header(&program, &names, None, off, &[]);
    assert!(
        said.contains("// level: source; passes left off: structure, foreach\n"),
        "{said}"
    );
    let said = header(&program, &names, None, on, &[]);
    assert!(
        said.contains("// level: source; passes left off: none\n"),
        "{said}"
    );
    assert!(said.contains("// project: none: "), "{said}");
    // A listing's reader takes a line that starts so for an import.
    assert!(
        said.lines().all(|line| !line.starts_with("// import ")),
        "{said}"
    );

    let structured = options(&asked(&words(&["x", "--level", "structured"])));
    assert_eq!(structured.level, Level::Structured);
    assert!(structured.on(Pass::Structure) && !structured.on(Pass::Foreach));
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
    let both = asked(&words(&["x", "--statics", "--threads", "0", "--entry"]));
    assert_eq!(both.wrong.as_deref(), Some("unknown option: --statics"));
    assert_eq!(both.paths, ["x"]);
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
    assert_eq!(called, [(0x03C0_E450, (0, 0, 2))]);
    let printed = natives(&program, false).expect("the program decodes");
    assert!(
        printed.starts_with("rva_03C0E450  0 -> 0  2 site(s)\n"),
        "{printed}"
    );

    // An end inside the last instruction is refused, not read past.
    program.functions[1].end -= 1;
    let refused = walked(&program).err();
    assert!(matches!(
        refused,
        Some(disasm::DisasmError::Malformed { .. })
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
    assert_eq!(functions(&program, &names, false), listed);
    let parsed: Value = serde_json::from_str(&functions(&program, &names, true)).unwrap();
    assert_eq!(parsed.as_array().map(Vec::len), Some(2));
    assert_eq!(parsed[0]["name"], "main");
    assert_eq!(parsed[1]["at"], format!("0x{split:06X}"));
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
    names.field_names.insert(0xF1, "_style".to_owned());
    names.method_names.insert(0xA1, "Start".to_owned());

    bind(&mut names, &program);
    assert_eq!(names.field(owner, 0x10), Some("_style"));
    assert_eq!(names.field(owner, 0x18), None);
    assert_eq!(names.method(owner, 3), Some("Start"));
    assert_eq!(names.method(owner, 4), None);
    assert_eq!((names.methods.len(), names.fields.len()), (1, 1));
}

#[test]
fn one_function_is_decompiled_alone_under_first_lines_that_name_it() {
    let (program, names) = (program(), Names::default());
    let second = format!("func_{:06X}", program.functions[1].at);
    let rows =
        selected(&program, &names, std::slice::from_ref(&second)).expect("the function is there");
    let cx = Context::new(&names, Project::empty(), Options::default());
    let output = lift::functions(&program, &rows, &cx);
    assert_eq!(output.stats.functions, 1);
    let text: String = output.files.iter().map(|file| file.text.as_str()).collect();
    assert!(
        text.contains(&second) && !text.contains("func_000000"),
        "{text}"
    );
    let said = header(&program, &names, None, cx.options, &rows);
    assert!(said.ends_with(&format!("// only: {second}\n")), "{said}");
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
fn a_file_of_the_names_folder_that_is_no_names_list_is_passed_over_whole() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    write("a.tsv", "0x0A0B0C0D\tdemo_program\n");
    write("b.json", "{\"DEMO\": {}}\n");
    // A list refused at its second line names nothing with its first.
    write("c.tsv", "0x00000001\tearly\nno hash here\n");

    let names = names_in(dir.path());
    assert_eq!(names.lists, ["a.tsv"]);
    assert_eq!(names.program(0x0A0B_0C0D), Some("demo_program"));
    assert_eq!(names.program(1), None);
    assert!(names_in(&dir.path().join("missing")).lists.is_empty());
}
