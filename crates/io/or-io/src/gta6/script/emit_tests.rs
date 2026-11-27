use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    BinOp, Const, ExprSugar, Head, Returns, Scalar, Stmt, StoreKind, Var, VarInfo, VmType, Width,
};
use crate::gta6::script::listing::Import;
use crate::gta6::script::project::TypeFacts;
use crate::gta6::script::{Names, Options, Project};

const PROGRAM: u32 = 0x1234_ABCD;

fn function(at: u32, params: u32, returns: Returns, body: Vec<tree::Stmt>) -> tree::Function {
    let head = Head {
        program: PROGRAM,
        at,
        name: format!("func_{at:06X}"),
        returns,
        params,
        locals: 0,
        vars: BTreeMap::new(),
        incomplete: None,
        value_types: BTreeMap::new(),
    };
    tree::Function { head, body }
}

fn member(role: Role, is_static: bool, function: tree::Function) -> Method {
    Method {
        function,
        role,
        is_static,
        is_async: false,
        slot: None,
        overrides: false,
    }
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn load(ty: Scalar, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(p(0)),
        offset,
    }
}

fn store(kind: StoreKind, offset: u32, value: Expr) -> tree::Stmt {
    tree::Stmt::Simple(Stmt::Store {
        kind,
        addr: p(0),
        offset,
        value,
    })
}

fn eval(e: Expr) -> tree::Stmt {
    tree::Stmt::Simple(Stmt::Eval(e))
}

fn field(offset: u32, bytes: u32, kind: Kind, is_static: bool) -> Field {
    Field {
        offset,
        bytes,
        kind,
        is_static,
    }
}

fn class(key: u64, kind: ClassKind) -> Class {
    Class {
        key: TypeKey(key),
        kind,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: Vec::new(),
        nested: Vec::new(),
    }
}

/// A program of two free functions, a class with every kind of member and a class nested in it, a struct and an enum.
// ⚠ Out of order on purpose: the file's order is by offset and by key, whatever order the unit holds.
fn program() -> Unit {
    let shared = Callee::Import {
        module: "shared".to_owned(),
        name: "func_00A1B2".to_owned(),
        program: 0x0A0B_0C0D,
        at: 0xA1B2,
    };
    let call = Expr::Call {
        callee: shared,
        args: vec![p(0), Expr::Const(Const::f32(15.0))],
        returns: Some(0),
    };
    let free = function(0, 1, Returns::Void, vec![eval(call)]);
    let mut unread = function(0x20, 0, Returns::Void, Vec::new());
    unread.head.incomplete = Some("no code".to_owned());

    let stored = store(StoreKind::I32, 0x8, p(1));
    let mut made = function(0x40, 2, Returns::Void, vec![stored]);
    let mut count = VarInfo::new(Width::Cell);
    count.kind = Kind::I32;
    made.head.vars.insert(Var::Param(1), count);
    let read = Expr::cast(Scalar::I32, p(1));
    let sum = Expr::binary(
        BinOp::Add,
        Expr::cast(Scalar::I32, load(Scalar::I32, 0x8)),
        read,
    );
    let update = function(0x60, 2, Returns::Cell, vec![tree::Stmt::Return(Some(sum))]);
    let got = tree::Stmt::Return(Some(load(Scalar::I32, 0x8)));
    let get = function(0x80, 1, Returns::Cell, vec![got]);
    let kept = store(StoreKind::U64, 0x18, p(1));
    let set = function(0xA0, 2, Returns::Void, vec![kept]);
    let fixed = function(0xC0, 1, Returns::Void, vec![tree::Stmt::Return(None)]);
    let back = Expr::Call {
        callee: Callee::Local {
            at: 0,
            name: "func_000000".to_owned(),
        },
        args: vec![p(0)],
        returns: Some(0),
    };
    let lambda = function(0xE0, 1, Returns::Void, vec![eval(back)]);

    let mut closure = class(0xBB, ClassKind::Closure);
    closure.fields = vec![field(0x8, 8, Kind::Unknown, false)];
    closure.methods = vec![member(Role::Lambda, false, lambda)];
    let mut overriding = member(Role::Method, false, update);
    overriding.slot = Some(3);
    overriding.overrides = true;
    let mut whole = class(0x0123_4567_89AB_CDEF, ClassKind::Class);
    whole.base = Some(TypeKey(0xAA));
    whole.fields = vec![
        field(0x10, 16, Kind::Unknown, false),
        field(0x8, 4, Kind::I32, false),
    ];
    whole.methods = vec![
        member(Role::Method, true, fixed),
        member(Role::Setter, false, set),
        overriding,
        member(Role::Getter, false, get),
        member(Role::Constructor, false, made),
    ];
    whole.nested = vec![closure];
    let mut value = class(1, ClassKind::Struct);
    value.base = Some(TypeKey(0xCC));
    value.interfaces = vec![TypeKey(0xDD)];
    value.fields = vec![
        field(0x20, 8, Kind::Unknown, true),
        field(0, 4, Kind::Unknown, false),
    ];
    let listed = class(2, ClassKind::Enum);
    Unit {
        program: PROGRAM,
        imports: vec![Import {
            index: 1,
            program: 0x0A0B_0C0D,
            types: 2..100,
        }],
        classes: vec![listed, value, whole],
        functions: vec![Method::free(unread), Method::free(free)],
        front: None,
    }
}

/// A project that knows the values of the enum of `program`.
fn project() -> Project {
    let mut project = Project::default();
    let facts = TypeFacts {
        constants: vec![0, 1, 2],
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(2), facts);
    project
}

fn stats() -> Stats {
    Stats {
        functions: 9,
        incomplete: 1,
        ..Stats::default()
    }
}

/// The one file `unit` is written as by the options given.
fn file(unit: &Unit, names: &Names, project: &Project, options: Options) -> File {
    let cx = Context::new(names, project, options);
    let mut files = super::unit(unit, &cx, &stats());
    assert_eq!(files.len(), 1, "one file a program");
    files.remove(0)
}

const BODY: &str = "\
void func_000000(Cell p0)
{
    shared.func_00A1B2(p0, 15.0f);
}

// func_000020: no body (no code)

class type_0123456789ABCDEF : type_00000000000000AA
{
    int f_8;
    Cells2 f_10;

    type_0123456789ABCDEF(int p1) // func_000040
    {
        this.f_8 = p1;
    }

    override Cell Method_3(Cell p1) // func_000060
    {
        return (int)this.f_8 + (int)p1;
    }

    Cell func_000080 => (int)this.f_8;

    Cell func_0000A0
    {
        set
        {
            this.f_18 = value;
        }
    }

    static void func_0000C0(Cell p0)
    {
        return;
    }

    class type_00000000000000BB // a closure
    {
        Cell f_8;

        void func_0000E0()
        {
            func_000000(this);
        }
    }
}

struct type_0000000000000001 : type_00000000000000DD // base type_00000000000000CC
{
    Bytes4 f_0;
    static Cell f_20;
}

enum type_0000000000000002
{
    // values: 0, 1, 2
}
";

#[test]
fn a_program_is_one_file_of_its_functions_and_then_its_classes() {
    let (names, project) = (Names::default(), project());
    let written = file(&program(), &names, &project, Options::default());
    assert_eq!(written.name, "program_1234ABCD.cs");
    let Some((header, body)) = written.text.split_once("\n\n") else {
        panic!("no blank line after the header:\n{}", written.text);
    };
    let top = "\
// OpenRage: GTA VI script, program 0x1234ABCD
// level: source; passes off: none
// imports: 0x0A0B0C0D
// names: no names list supplied
// 9 function(s), 1 with no body; 0 goto(s) left
// words: ";
    assert!(header.starts_with(top), "{header}");
    // Six facts, each a comment that goes on under itself where it passes the width.
    let facts = header.lines().filter(|line| !line.starts_with("//   "));
    assert_eq!(facts.count(), 6, "{header}");
    assert!(header.lines().all(|line| line.chars().count() <= 120));
    assert_eq!(body, BODY);

    // ⚠ C# takes a function at the top of a file only before the first type.
    let first_class = body.find("\nclass ").unwrap_or(0);
    let last_function = body.find("// func_000020").unwrap_or(usize::MAX);
    assert!(last_function < first_class, "{body}");
    // No tab, no space at the end of a line, and one line end at the end of the file.
    assert!(!written.text.contains('\t') && !written.text.contains('\r'));
    assert!(written.text.lines().all(|line| !line.ends_with(' ')));
    assert!(written.text.ends_with("}\n") && !written.text.ends_with("\n\n"));
}

#[test]
fn the_bytes_are_the_same_whatever_order_the_unit_holds_its_parts_in() {
    let (names, project) = (Names::default(), project());
    let unit = program();
    let first = file(&unit, &names, &project, Options::default());
    assert_eq!(first, file(&unit, &names, &project, Options::default()));

    // Functions, classes, members and fields turned round: the offsets and the keys order the file.
    let mut turned = program();
    turned.functions.reverse();
    turned.classes.reverse();
    for class in &mut turned.classes {
        class.methods.reverse();
        class.fields.reverse();
    }
    assert_eq!(first, file(&turned, &names, &project, Options::default()));
}

#[test]
fn a_file_is_named_by_the_programs_hash_whatever_the_lists_name() {
    let mut names = Names::default();
    names
        .programs
        .insert(u64::from(PROGRAM), "minigame".to_owned());
    names.programs.insert(0x0A0B_0C0D, "shared_code".to_owned());
    names.types.insert(0xAA, "CBase".to_owned());
    let owner = TypeKey(0x0123_4567_89AB_CDEF);
    names.fields.insert((owner, 0x8), "count".to_owned());
    names.methods.insert((owner, 3), "Update".to_owned());
    let project = project();
    let written = file(&program(), &names, &project, Options::default());
    assert_eq!(written.name, "program_1234ABCD.cs");
    let text = &written.text;
    let top = "\
// OpenRage: GTA VI script, program 0x1234ABCD (minigame)
// level: source; passes off: none
// imports: 0x0A0B0C0D (shared_code)
";
    assert!(text.starts_with(top), "{text}");
    for line in [
        "    shared_code.func_00A1B2(p0, 15.0f);",
        "class type_0123456789ABCDEF : CBase",
        // A name the user gave leaves the slot unsaid: a comment keeps it.
        "    int count; // f_8",
        "    override Cell Update(Cell p1) // func_000060",
    ] {
        assert!(
            text.lines().any(|l| l == line),
            "no line `{line}` in:\n{text}"
        );
    }
}

#[test]
fn the_natives_a_unit_calls_are_counted_in_its_header() {
    let native = |name: &str, hash: u64| {
        let callee = Callee::Native {
            namespace: None,
            name: name.to_owned(),
            hash,
            rva: 0,
        };
        eval(Expr::Call {
            callee,
            args: Vec::new(),
            returns: Some(0),
        })
    };
    let body = vec![
        native("DO_THING", 0xA1),
        native("_0x00000000000000BB", 0xBB),
        native("DO_THING", 0xA1),
    ];
    let mut inner = class(0xBB, ClassKind::Class);
    let nested = function(0x40, 0, Returns::Void, vec![native("OTHER", 0xA2)]);
    inner.methods = vec![member(Role::Method, true, nested)];
    let mut outer = class(0xAA, ClassKind::Class);
    outer.nested = vec![inner];
    let unit = Unit {
        program: PROGRAM,
        imports: Vec::new(),
        classes: vec![outer],
        functions: vec![Method::free(function(0, 0, Returns::Void, body))],
        front: None,
    };
    let (names, project) = (Names::default(), Project::default());
    let text = file(&unit, &names, &project, Options::default()).text;
    let said = "\n// natives: 3 called: 2 named by the program itself, 1 shown by hash; \
                no natives list names a native\n";
    assert!(text.contains(said), "{text}");
    // A unit with no import and no native says nothing of either.
    let empty = Unit::flat(PROGRAM, Vec::new(), Vec::new());
    let text = file(&empty, &names, &project, Options::default()).text;
    assert!(
        !text.contains("imports") && !text.contains("natives"),
        "{text}"
    );
    let facts = text.lines().filter(|line| !line.starts_with("//   "));
    assert_eq!(facts.count(), 5, "{text}");

    // ⚠ A compiled program states no hash of a native no list reaches: such a one is counted
    // by the address of its handler, and never as named by the program.
    let by_address = |rva: u32| {
        let callee = Callee::Native {
            namespace: None,
            name: format!("rva_{rva:08X}"),
            hash: 0,
            rva,
        };
        eval(Expr::Call {
            callee,
            args: Vec::new(),
            returns: Some(0),
        })
    };
    let body = vec![
        by_address(0x1000),
        native("_0x00000000000000BB", 0xBB),
        by_address(0x2000),
        by_address(0x1000),
    ];
    let mut compiled = Unit::flat(PROGRAM, Vec::new(), Vec::new());
    compiled.functions = vec![Method::free(function(0, 0, Returns::Void, body))];
    compiled.front = Some(print::PROGRAM);
    let text = file(&compiled, &names, &project, Options::default()).text;
    let said = "\n// natives: 3 called: 1 shown by hash, 2 shown by the address of their handler; \
                no natives list names a native\n";
    assert!(text.contains(said), "{text}");
}

#[test]
fn a_long_list_of_bases_and_a_long_list_of_values_go_on_under_their_line() {
    let mut project = Project::default();
    let facts = TypeFacts {
        constants: (0..80).map(|n| 1_000_000_000 + n).collect(),
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(2), facts);
    let mut wide = class(0xA0, ClassKind::Class);
    wide.base = Some(TypeKey(0xB0));
    wide.interfaces = (0xC0..0xC8).map(TypeKey).collect();
    wide.nested = vec![class(2, ClassKind::Enum)];
    let unit = Unit {
        program: PROGRAM,
        imports: Vec::new(),
        classes: vec![wide],
        functions: Vec::new(),
        front: None,
    };
    let names = Names::default();
    let text = file(&unit, &names, &project, Options::default()).text;
    assert!(
        text.lines().all(|line| line.chars().count() <= 120),
        "{text}"
    );
    // The head breaks after a comma and goes on one level in; the brace stays under the word.
    let head = "\nclass type_00000000000000A0 : type_00000000000000B0, type_00000000000000C0, \
                type_00000000000000C1,\n    type_00000000000000C2, type_00000000000000C3, ";
    assert!(text.contains(head), "{text}");
    assert!(text.contains(",\n    type_00000000000000C7\n{\n"), "{text}");
    // The values are one comment, broken between two of them.
    assert!(
        text.contains("\n        // values: 1000000000, 1000000001, "),
        "{text}"
    );
    assert!(text.contains(",\n        //   10000000"), "{text}");
    assert!(text.contains(", 1000000079\n    }\n"), "{text}");
}

#[test]
fn the_level_asked_is_the_form_written() {
    let (names, project) = (Names::default(), project());
    // `Lifted`: every function in a listing's own syntax, by offset, whatever class holds it.
    let lifted = file(&program(), &names, &project, Options::at(Level::Lifted)).text;
    assert!(lifted.contains("[VmProgram(0x1234ABCDU)]"), "{lifted}");
    let at = |name: &str| lifted.find(name).unwrap_or(usize::MAX);
    let heads = [
        "func_000000(",
        "func_000040(",
        "func_0000C0(",
        "func_0000E0(",
    ];
    assert!(
        heads.windows(2).all(|pair| at(pair[0]) < at(pair[1])),
        "{lifted}"
    );
    assert!(!lifted.contains("class type_"), "{lifted}");

    // The four first passes write the form of old under the same header.
    let old = file(&program(), &names, &project, Options::baseline()).text;
    assert!(old.contains("\nvoid func_000000(var p0)\n"), "{old}");
    assert!(old.contains("// level: structured; passes off: "), "{old}");
}

#[test]
fn a_class_is_written_as_the_kind_c_sharp_can_hold_it_as() {
    let (names, project) = (Names::default(), project());
    let write = |class: Class| {
        let unit = Unit {
            program: PROGRAM,
            imports: Vec::new(),
            classes: vec![class],
            functions: Vec::new(),
            front: None,
        };
        file(&unit, &names, &project, Options::default()).text
    };
    // ⚠ An enum with members is no enum of C#: it is written as the value type it is.
    let mut listed = class(2, ClassKind::Enum);
    listed.fields = vec![field(0, 4, Kind::I32, false)];
    let text = write(listed);
    let expected = "\
struct type_0000000000000002 // an enum
{
    // values: 0, 1, 2
    int f_0;
}
";
    assert!(text.ends_with(expected), "{text}");

    let mut machine = class(3, ClassKind::StateMachine);
    machine.interfaces = vec![TypeKey(0xDD), TypeKey(0xEE)];
    machine.fields = vec![field(0x8, 12, Kind::Unknown, false)];
    let text = write(machine);
    let expected = "\
class type_0000000000000003 : type_00000000000000DD, type_00000000000000EE // a state machine
{
    Bytes12 f_8;
}
";
    assert!(text.ends_with(expected), "{text}");

    let text = write(class(4, ClassKind::Interface));
    assert!(
        text.ends_with("\ninterface type_0000000000000004\n{\n}\n"),
        "{text}"
    );
}

/// A unit of one function that makes a closure object, keeps `p0` in it and binds the lambda of its closure class to it.
fn binding() -> Unit {
    let ty = |name: &str, id: u32| VmType {
        name: name.to_owned(),
        byte_size: 16,
        id,
        flags: 0,
    };
    let local = |n: u32| Expr::Var(Var::Local(n));
    let reference = Expr::FunctionRef {
        program: PROGRAM,
        at: 0xE0,
    };
    let ctor = Expr::Call {
        callee: Callee::Builtin {
            path: "System.Delegate.Ctor".to_owned(),
            ty: 9,
            method: 0,
        },
        args: vec![
            local(1),
            local(0),
            Expr::sugar(ExprSugar::Lambda, reference),
        ],
        returns: Some(0),
    };
    let parts = vec![
        Stmt::Assign {
            target: Var::Local(1),
            value: Expr::NewObject(ty("System.Delegate", 9)),
        },
        Stmt::Eval(ctor),
    ];
    let body = vec![
        tree::Stmt::Simple(Stmt::Assign {
            target: Var::Local(0),
            value: Expr::NewObject(ty("type_00000000000000BB", 5)),
        }),
        tree::Stmt::Simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: local(0),
            offset: 0x8,
            value: p(0),
        }),
        tree::Stmt::Sugar(Box::new(tree::Sugar::Group {
            kind: tree::Group::Delegate,
            parts,
        })),
        tree::Stmt::Return(Some(local(1))),
    ];
    let called = Expr::Call {
        callee: Callee::Import {
            module: "shared".to_owned(),
            name: "func_00A1B2".to_owned(),
            program: 0x0A0B_0C0D,
            at: 0xA1B2,
        },
        args: vec![load(Scalar::U64, 0x8), p(1)],
        returns: Some(0),
    };
    let lambda = function(0xE0, 2, Returns::Void, vec![eval(called)]);
    let mut closure = class(0xBB, ClassKind::Closure);
    closure.fields = vec![field(0x8, 8, Kind::Unknown, false)];
    closure.methods = vec![member(Role::Lambda, false, lambda)];
    Unit {
        program: PROGRAM,
        imports: Vec::new(),
        classes: vec![closure],
        functions: vec![Method::free(function(0, 1, Returns::Cell, body))],
        front: None,
    }
}

#[test]
fn a_lambda_bound_once_is_written_where_it_is_bound_and_left_out_of_its_closure_class() {
    let (names, project) = (Names::default(), Project::default());
    let text = file(&binding(), &names, &project, Options::default()).text;
    let written = "\
Cell func_000000(Cell p0)
{
    Cell local_0 = new type_00000000000000BB();
    local_0->f_8 = p0;
    Cell local_1 = (Cell p1) => shared.func_00A1B2(local_0->f_8, p1);
    return local_1;
}

class type_00000000000000BB // a closure
{
    Cell f_8;
}
";
    assert!(text.ends_with(written), "{text}");
    assert!(!text.contains("func_0000E0"), "{text}");

    // With `lambdas` off the delegate names the function, and the function stands in its class.
    let off = Options::default().without(Pass::Lambdas);
    let text = file(&binding(), &names, &project, off).text;
    assert!(
        text.contains("\n    Cell local_1 = local_0.func_0000E0;\n"),
        "{text}"
    );
    let member = "\n    void func_0000E0(Cell p1)\n    {\n        \
                  shared.func_00A1B2(this.f_8, p1);\n    }\n}\n";
    assert!(text.ends_with(member), "{text}");

    // ⚠ A body the unit names twice is written once, in its class: neither place is the one.
    let mut twice = binding();
    let again = Expr::FunctionRef {
        program: PROGRAM,
        at: 0xE0,
    };
    let names_it = function(
        0x100,
        0,
        Returns::Cell,
        vec![tree::Stmt::Return(Some(again))],
    );
    twice.functions.push(Method::free(names_it));
    let text = file(&twice, &names, &project, Options::default()).text;
    assert!(
        text.contains("\n    Cell local_1 = local_0.func_0000E0;\n"),
        "{text}"
    );
    assert!(text.contains("\n    void func_0000E0(Cell p1)\n"), "{text}");
}

#[test]
fn an_enum_is_headed_by_the_integer_its_values_are_and_its_base_and_interfaces_are_said_beside() {
    let mut project = Project::default();
    // A value type that is no struct, of four bytes, one of whose values a signed int cannot hold.
    let facts = TypeFacts {
        byte_size: 4,
        flags: 0x08,
        constants: vec![0, 1, 0x8000_0000],
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(2), facts);
    let mut listed = class(2, ClassKind::Enum);
    listed.base = Some(TypeKey(0xCC));
    listed.interfaces = vec![TypeKey(0xDD)];
    let unit = Unit {
        program: PROGRAM,
        imports: Vec::new(),
        classes: vec![listed],
        functions: Vec::new(),
        front: None,
    };
    let names = Names::default();
    let text = file(&unit, &names, &project, Options::default()).text;
    let expected = "\
enum type_0000000000000002 : uint // base type_00000000000000CC; implements type_00000000000000DD
{
    // values: 0, 1, 2147483648
}
";
    assert!(text.ends_with(expected), "{text}");

    // Of no proved values, the head names no integer, and its interfaces still stand aside.
    let unknown = Project::default();
    let text = file(&unit, &names, &unknown, Options::default()).text;
    let head = "\nenum type_0000000000000002 // base type_00000000000000CC; implements \
                type_00000000000000DD\n{\n}\n";
    assert!(text.ends_with(head), "{text}");

    // With `kinds` off the head is what it was.
    let off = Options::default().without(Pass::Kinds);
    let text = file(&unit, &names, &project, off).text;
    let old = "\nenum type_0000000000000002 : type_00000000000000DD // base \
               type_00000000000000CC\n";
    assert!(text.contains(old), "{text}");
}
