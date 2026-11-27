use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{Const, Head, Kind, Returns, VmType};
use crate::gta6::script::project::{FieldFacts, Member, Signature, TypeFacts};

/// A listing of one function that does nothing, of a program whose hash is no game's.
fn listing(program: u32) -> String {
    format!(
        "// import 1: program_0x0A0B0C0D, types [2, 100)
[VmProgram(0x{program:08X}U)]
public static class Program_{program:08X}
{{
    [VmFunction(0x000400)]
    public static void main() // 0x000400
    {{
        VmValue[] parameters = new VmValue[] {{  }};
        VmValue[] locals = new VmValue[0];
        return;
        throw new global::System.InvalidOperationException(\"Unexpected bytecode fallthrough.\");
    }}
}}
"
    )
}

/// What a part states of `program`: these strings of its heap.
fn row(program: u32, strings: &[(u32, &str)]) -> ProgramInfo {
    let strings = strings.iter().map(|(at, text)| (*at, (*text).to_owned()));
    ProgramInfo {
        program,
        strings: strings.collect(),
        ..ProgramInfo::default()
    }
}

fn part(rows: Vec<ProgramInfo>) -> Project {
    Project {
        programs: rows.into_iter().map(|row| (row.program, row)).collect(),
        ..Project::default()
    }
}

#[test]
fn a_listing_gives_the_part_of_its_program() {
    let part = of_listing(&listing(0x1122_3344)).expect("the listing reads");
    let info = part.programs.get(&0x1122_3344);
    let info = info.expect("the part holds its program");
    assert_eq!(info.program, 0x1122_3344);
    let import = ImportInfo {
        program: 0x0A0B_0C0D,
        first: 2,
        end: 100,
    };
    assert_eq!(info.imports, [import]);

    let unread = of_listing("not a listing\n");
    assert!(
        matches!(&unread, Err(ProjectError::Listing(_))),
        "{unread:?}"
    );
    let refused = of_program(&[]);
    assert!(
        matches!(
            &refused,
            Err(ProjectError::Program(_) | ProjectError::Lift(_))
        ),
        "{refused:?}"
    );
}

#[test]
fn parts_merge_to_the_same_project_in_any_order() {
    let imported = ImportInfo {
        program: 3,
        first: 1,
        end: 2,
    };
    // Programs 7 and 8 each state one string of program 9 their own way, and 7 a global of it,
    // in a row made as `or_default` makes one; two parts are of program 9 itself.
    let parts = || {
        let mut of_seven = part(vec![row(7, &[]), row(9, &[(0, "as 7 has it")])]);
        let stated = of_seven.programs.get_mut(&9).expect("the row was made");
        stated.program = 0;
        stated.globals.insert(8, Kind::I32);
        let mut of_nine = part(vec![row(9, &[(4, "one")])]);
        let own = of_nine.programs.get_mut(&9).expect("the row was made");
        own.imports.push(imported);
        vec![
            of_seven,
            part(vec![row(8, &[]), row(9, &[(0, "as 8 has it")])]),
            of_nine,
            part(vec![row(9, &[(4, "two")])]),
        ]
    };
    let merged = merge(parts());
    let saved = save(&merged).expect("the project saves");
    // Every turn puts another part first, and one of them the second part of program 9 before the first.
    for turn in 1..4 {
        let mut turned = parts();
        turned.rotate_left(turn);
        let again = save(&merge(turned)).expect("the project saves");
        assert!(again == saved, "turned by {turn}, the project differs");
    }

    let programs: Vec<u32> = merged.programs.keys().copied().collect();
    assert_eq!(programs, [7, 8, 9]);
    // What only one part states is kept, whichever part made the row first.
    let nine = &merged.programs[&9];
    assert_eq!(nine.program, 9);
    assert_eq!(nine.imports, [imported]);
    assert_eq!(nine.strings.len(), 2);
    assert_eq!(nine.globals.get(&8), Some(&Kind::I32));
}

#[test]
fn files_are_read_largest_first_and_given_back_in_their_own_order() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    // Each listing is longer than the one before, so reading starts at the last of them.
    let mut paths = Vec::new();
    for (n, program) in [0x30_u32, 0x10, 0x20].into_iter().enumerate() {
        let path = folder.path().join(format!("{program:08X}.ysc.cs"));
        let text = "// a line of the head\n".repeat(n) + &listing(program);
        std::fs::write(&path, text).expect("the listing is written");
        paths.push(path);
    }
    for workers in [1, 4] {
        let read = read_on(workers, &paths);
        let own = |part: &Result<Project, ProjectError>| {
            let part = part.as_ref().ok()?;
            part.programs.keys().next().copied()
        };
        let programs: Vec<Option<u32>> = read.iter().map(own).collect();
        assert_eq!(
            programs,
            [Some(0x30), Some(0x10), Some(0x20)],
            "on {workers} thread(s)"
        );
    }

    let built = build(&paths).expect("the listings build");
    for program in [0x10, 0x20, 0x30] {
        assert!(built.programs.contains_key(&program), "{program:#x}");
    }
    let saved = save(&built).expect("the project saves");
    paths.reverse();
    let again = build(&paths).expect("the listings build");
    assert!(save(&again).expect("the project saves") == saved);

    // A file that opens with zero bytes is a compiled program though its bytes are text, and
    // the refusal of a build names it.
    let program = folder.path().join("a_program");
    std::fs::write(&program, [0_u8; 16]).expect("the file is written");
    let refused = read_on(1, std::slice::from_ref(&program));
    assert!(
        matches!(
            refused.as_slice(),
            [Err(ProjectError::Program(_) | ProjectError::Lift(_))]
        ),
        "{refused:?}"
    );
    paths.push(program);
    let refused = build(&paths);
    assert!(
        matches!(&refused, Err(ProjectError::File { path, .. }) if path.ends_with("a_program")),
        "{refused:?}"
    );
    let missing = build(&[folder.path().join("no_such_file")]);
    assert!(
        matches!(&missing, Err(ProjectError::Io { path, .. }) if path.ends_with("no_such_file")),
        "{missing:?}"
    );
}

#[test]
fn a_file_of_another_version_is_refused_for_its_version() {
    // Its tables are not this version's, and that is not what is said of it.
    let refused = load(br#"{"version":2,"project":7}"#);
    assert!(
        matches!(&refused, Err(ProjectError::Sidecar(why)) if why.contains("version 2")),
        "{refused:?}"
    );
    // A file of this version whose tables do not read is refused for them.
    let unread = load(br#"{"version":1,"project":7}"#);
    assert!(
        matches!(&unread, Err(ProjectError::Sidecar(why)) if !why.contains("version")),
        "{unread:?}"
    );
}

#[test]
fn a_view_holds_what_a_program_reaches_and_nothing_else() {
    let (own, theirs, base, lone) = (
        TypeKey(1 << 40),
        TypeKey(2 << 40),
        TypeKey(3 << 40),
        TypeKey(4 << 40),
    );
    let builtin = TypeKey(0x8000_000B);
    let function = |program: u32| FunctionKey::new(program, 0x10 * program);
    // Program 1 imports 2, whose type derives from a type of 3 that fills its slot with a
    // function of 3; nothing leads to 4, though the builtin type's facts were read from it.
    let defining = |program: u32, key: TypeKey| ProgramInfo {
        program,
        types: BTreeMap::from([(0, key)]),
        ..ProgramInfo::default()
    };
    let of = |home: u32, base: Option<TypeKey>| TypeFacts {
        home,
        base,
        ..TypeFacts::default()
    };
    let mut project = Project::default();
    for (program, key) in [(1, own), (2, theirs), (3, base), (4, lone)] {
        project.programs.insert(program, defining(program, key));
        let signature = Signature::default();
        project.signatures.insert(function(program), signature);
    }
    let importer = project.programs.get_mut(&1).expect("the row was made");
    importer.imports.push(ImportInfo {
        program: 2,
        first: 1,
        end: 2,
    });
    project.types.insert(own, of(1, Some(builtin)));
    project.types.insert(theirs, of(2, Some(base)));
    let mut slotted = of(3, None);
    slotted.slots.insert(0, function(3));
    project.types.insert(base, slotted);
    project.types.insert(lone, of(4, None));
    project.types.insert(builtin, of(4, None));
    let field = FieldFacts {
        offset: 8,
        bytes: 4,
        kind: Kind::I32,
    };
    project.fields.insert(base, vec![field.clone()]);
    project.fields.insert(lone, vec![field]);
    project.calls.insert(function(1), vec![function(2)]);
    project.calls.insert(function(4), vec![function(1)]);
    for program in [3, 4] {
        let member = Member {
            owner: base,
            role: Role::Method,
            is_static: false,
            slot: Some(0),
        };
        project.members.insert(function(program), member);
    }

    let seen = view(&project, 1);
    let programs: Vec<u32> = seen.programs.keys().copied().collect();
    assert_eq!(programs, [1, 2, 3]);
    let types: Vec<TypeKey> = seen.types.keys().copied().collect();
    assert_eq!(types, [builtin, own, theirs, base]);
    let fields: Vec<TypeKey> = seen.fields.keys().copied().collect();
    assert_eq!(fields, [base]);
    let signatures: Vec<FunctionKey> = seen.signatures.keys().copied().collect();
    assert_eq!(signatures, [function(1), function(2), function(3)]);
    let calls: Vec<FunctionKey> = seen.calls.keys().copied().collect();
    assert_eq!(calls, [function(1)]);
    let members: Vec<FunctionKey> = seen.members.keys().copied().collect();
    assert_eq!(members, [function(3)]);
    // What is asked from the program's own ids is answered as the whole project answers it.
    assert_eq!(seen.type_key(1, 1), Some(theirs));
    assert_eq!(seen.slot(theirs, 0), project.slot(theirs, 0));
    assert_eq!(seen.slot(theirs, 0), Some(function(3)));
    assert_eq!(seen.field(theirs, 8), project.field(theirs, 8));

    // A program nothing leads from is seen alone, and one the project does not hold as nothing.
    let alone = view(&project, 4);
    let programs: Vec<u32> = alone.programs.keys().copied().collect();
    assert_eq!(programs, [4]);
    assert!(alone.types.contains_key(&lone) && !alone.types.contains_key(&own));
    assert!(view(&project, 99).programs.is_empty());
}

/// A listing of one function of `program` that has one local and runs `body`.
fn listing_of(program: u32, body: &str) -> String {
    format!(
        "// import 1: program_0x0A0B0C0D, types [2, 100)
[VmProgram(0x{program:08X}U)]
public static class Program_{program:08X}
{{
    [VmFunction(0x000400)]
    public static void main() // 0x000400
    {{
        VmValue[] parameters = new VmValue[] {{  }};
        VmValue[] locals = new VmValue[1];
        ref VmValue local_0 = ref locals[0];
{body}
        return;
        throw new global::System.InvalidOperationException(\"Unexpected bytecode fallthrough.\");
    }}
}}
"
    )
}

/// A function of `program` whose one block runs `stmts`.
fn function(program: u32, stmts: Vec<Stmt>) -> Function {
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    let head = Head {
        value_types: Default::default(),
        program,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 0,
        locals: 1,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

/// The global of program 7 at byte `offset`.
fn held(offset: u32) -> Var {
    Var::Global {
        program: 7,
        offset,
        statics: false,
    }
}

/// `value` taken into a temporary, as a front end takes what it reads.
fn taken(index: u32, value: Expr) -> Stmt {
    let target = Var::Value { at: 0x40, index };
    Stmt::Assign { target, value }
}

fn cell(offset: u32, bytes: u32, kind: Kind) -> FieldFacts {
    FieldFacts {
        offset,
        bytes,
        kind,
    }
}

#[test]
fn a_global_is_the_cell_every_access_of_it_takes_at_one_width() {
    let read = |to, var| Expr::cast(to, Expr::Var(var));
    let address = |offset, cell| Expr::AddrOf {
        var: held(offset),
        cell,
    };
    let moved = Expr::ByteOffset {
        addr: Box::new(address(0x30, 0)),
        offset: Box::new(Expr::Const(Const::i32(8))),
    };
    let fixed = Var::Global {
        program: 7,
        offset: 0x10,
        statics: true,
    };
    let stmts = vec![
        // A byte read without a sign, and stored as a byte.
        taken(0, read(Scalar::U8, held(0x10))),
        Stmt::Assign {
            target: held(0x10),
            value: Expr::cast(Scalar::U8, Expr::Const(Const::i32(1))),
        },
        // Four bytes by one access and the whole cell by another.
        taken(1, read(Scalar::I32, held(0x20))),
        taken(2, Expr::Var(held(0x20))),
        // Sixteen bytes from the second cell of the value at 0x30, read and then cleared.
        taken(
            3,
            Expr::LoadBytes {
                addr: Box::new(address(0x30, 1)),
                offset: 0,
                bytes: 16,
            },
        ),
        Stmt::ZeroBytes {
            addr: moved,
            bytes: 16,
        },
        // A static at the offset of a global is another cell.
        taken(4, read(Scalar::I32, fixed)),
        // What is stored under no cast is stored whole.
        Stmt::Assign {
            target: held(0x48),
            value: Expr::Var(Var::Local(0)),
        },
    ];
    // Program 8's code reads and writes the cells of program 7.
    let mut part = part(vec![row(7, &[]), row(8, &[])]);
    accessed(&mut part, &function(8, stmts));
    let project = merge(vec![part]);

    let seven = &project.programs[&7];
    let cells = [
        cell(0x10, 1, Kind::U8),
        cell(0x38, 16, Kind::Aggregate(2)),
        cell(0x48, 8, Kind::Unknown),
    ];
    assert_eq!(seven.global_cells, cells);
    assert_eq!(seven.static_cells, [cell(0x10, 4, Kind::Unknown)]);
    assert_eq!(project.cell(7, 0x38, false), Some(&cells[1]));
    assert_eq!(project.cell(7, 0x20, false), None, "read at two widths");
    assert_eq!(project.cell(7, 0x10, true).map(|cell| cell.bytes), Some(4));
    assert_eq!(project.cell(9, 0x10, false), None);
    // A width proves nothing of the values a cell holds, and a settled project holds no access.
    assert!(seven.globals.is_empty() && seven.statics.is_empty());
    assert_eq!(project.cells, Cells::default());
    assert!(project.programs[&8].global_cells.is_empty());
}

#[test]
fn a_global_called_through_is_an_object_of_the_most_derived_type_it_is_called_as() {
    let (base, derived, other) = (TypeKey(1 << 40), TypeKey(2 << 40), TypeKey(3 << 40));
    let through = |ty: TypeKey, receiver: Expr| {
        let callee = Callee::Virtual {
            class: String::new(),
            method: String::new(),
            ty: ty.to_string(),
            slot: 0,
        };
        Stmt::Eval(Expr::Call {
            callee,
            args: vec![receiver],
            returns: Some(0),
        })
    };
    let on = |ty, offset| through(ty, Expr::Var(held(offset)));
    let made = VmType {
        name: other.to_string(),
        byte_size: 16,
        id: 0,
        flags: 0x22,
    };
    let length = Callee::Intrinsic {
        name: "array_length".to_owned(),
        ty: None,
        wrapper: None,
    };
    let local = Callee::Local {
        at: 0x80,
        name: "func_000080".to_owned(),
    };
    let one = function(
        8,
        vec![
            on(base, 0x10),
            on(base, 0x20),
            on(derived, 0x30),
            Stmt::Assign {
                target: held(0x40),
                value: Expr::NewObject(made),
            },
            // A function that is handed a cell says nothing of it.
            Stmt::Eval(Expr::Call {
                callee: local,
                args: vec![Expr::Var(held(0x60))],
                returns: Some(0),
            }),
        ],
    );
    let two = function(
        9,
        vec![
            on(derived, 0x10),
            on(other, 0x20),
            Stmt::Eval(Expr::Call {
                callee: length,
                args: vec![Expr::cast(Scalar::U64, Expr::Var(held(0x50)))],
                returns: Some(1),
            }),
        ],
    );
    let parts = || {
        let (mut first, mut second) = (Project::default(), Project::default());
        called(&mut first, &one);
        called(&mut second, &two);
        [first, second]
    };
    let settled = |mut parts: [Project; 2]| {
        let mut project = part(vec![row(7, &[]), row(8, &[]), row(9, &[])]);
        let of = |base: Option<TypeKey>| TypeFacts {
            home: 7,
            base,
            ..TypeFacts::default()
        };
        project.types.insert(base, of(None));
        project.types.insert(derived, of(Some(base)));
        project.types.insert(other, of(None));
        // What the container's map says of one cell, and what a part states of another.
        let stated = project.programs.get_mut(&7).expect("the row was made");
        stated.globals.insert(0x10, Kind::Pointer);
        stated.globals.insert(0x30, Kind::I32);
        for part in &mut parts {
            absorb_cells(&mut project, part);
        }
        settle_cells(&mut project);
        project
    };
    let project = settled(parts());

    let seven = &project.programs[&7];
    let object = Kind::Object(derived.to_string());
    let cells = [
        cell(0x10, 8, object.clone()),
        // Two types of no one line.
        cell(0x20, 8, Kind::Pointer),
        cell(0x40, 8, Kind::Pointer),
        cell(0x50, 8, Kind::Pointer),
    ];
    assert_eq!(seven.global_cells, cells);
    // What every value of a cell is proved to be is stated where the passes read it, and never
    // over what was stated of the cell otherwise: four bytes are no receiver.
    let stated = BTreeMap::from([
        (0x10, object),
        (0x20, Kind::Pointer),
        (0x30, Kind::I32),
        (0x40, Kind::Pointer),
        (0x50, Kind::Pointer),
    ]);
    assert_eq!(seven.globals, stated);
    assert!(seven.static_cells.is_empty() && project.cells == Cells::default());

    // The parts in the other order settle to the same project.
    let mut turned = parts();
    turned.reverse();
    assert_eq!(settled(turned), project);
}

#[test]
fn a_listing_shows_the_width_of_each_access_of_a_global_as_it_wrote_it() {
    let body = "        local_0 = as_u8(Vm.Globals(0x0A0B0C0DU)[0x18]);
        Vm.Globals(0x0A0B0C0DU)[0x18] = as_u8(local_0);";
    let part = of_listing(&listing_of(0x1122_3344, body)).expect("the listing reads");
    let shown = part.cells.shown.get(&(0x0A0B_0C0D, false, 0x18));
    let shown = shown.expect("the accesses of the cell were read");
    assert_eq!(shown.widths, BTreeSet::from([1]));
    assert_eq!(shown.kinds, [Kind::U8]);
    assert!(shown.called.is_empty());
}

#[test]
fn a_file_written_before_cells_were_kept_still_reads() {
    let old = br#"{"version":1,"project":{"programs":{"7":{"program":7,"imports":[],"types":{},"globals":{"16":"Pointer"},"statics":{},"strings":{}}},"types":{"1099511627776":{"byte_size":4,"flags":72,"home":7,"base":null,"interfaces":[],"slots":{},"generic":null,"constants":[1,2],"returns":{}}},"fields":{},"signatures":{},"calls":{},"members":{}}}"#;
    let project = load(old).expect("a file of this version reads");
    let info = &project.programs[&7];
    assert_eq!(info.globals.get(&16), Some(&Kind::Pointer));
    assert!(info.global_cells.is_empty() && info.static_cells.is_empty());
    let facts = &project.types[&TypeKey(1 << 40)];
    assert_eq!((facts.parameters, facts.constants.len()), (0, 2));
    assert!(facts.constant_names.is_empty());
}
