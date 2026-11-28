use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, Kind, TypeKey, Var, VmType};
use crate::gta6::script::opcode::Op;
use crate::gta6::script::project::{
    index, FieldFacts, ImportInfo, Member, Param, ProgramInfo, Signature, TypeFacts,
};

/// The program asked of, one that imports it, and one more; no game's hashes.
const P: u32 = 0x0000_0A11;
const Q: u32 = 0x0000_0B22;
const R: u32 = 0x0000_0C33;

/// Types of P, and one of Q.
const T: TypeKey = TypeKey(0x0A11_0000_0000_0001);
const U: TypeKey = TypeKey(0x0A11_0000_0000_0002);
const M: TypeKey = TypeKey(0x0A11_0000_0000_0003);
const N: TypeKey = TypeKey(0x0A11_0000_0000_0004);
const B: TypeKey = TypeKey(0x0A11_0000_0000_0005);
const X: TypeKey = TypeKey(0x0A11_0000_0000_0006);
const V: TypeKey = TypeKey(0x0B22_0000_0000_0001);
/// A type no row states.
const UNSTATED: TypeKey = TypeKey(0x0A11_0000_0000_0099);
/// Builtin classes no fact places, and a builtin interface.
const CLASS: TypeKey = TypeKey(0x8000_0007);
const OTHER: TypeKey = TypeKey(0x8000_000B);
const LISTED: TypeKey = TypeKey(0x8000_0009);

fn at(program: u32, offset: u32) -> FunctionKey {
    FunctionKey::new(program, offset)
}

/// A type row of `home` whose slots hold the functions of `home` at these offsets.
fn row(
    key: TypeKey,
    home: u32,
    base: Option<TypeKey>,
    interfaces: &[TypeKey],
    vtable: &[Option<u32>],
) -> Row {
    let vtable = vtable
        .iter()
        .map(|held| held.map(|offset| at(home, offset)));
    Row {
        key,
        home,
        base,
        interfaces: interfaces.to_vec(),
        generic: None,
        unresolved: false,
        vtable: vtable.collect(),
    }
}

/// A row of P with no slot.
fn bare(key: TypeKey) -> Row {
    row(key, P, None, &[], &[])
}

/// A project whose parts read each of `programs` from a compiled program.
fn stated(programs: &[u32]) -> Project {
    let mut project = Project::default();
    project.naming.stated.extend(programs.iter().copied());
    project
}

fn object(key: TypeKey) -> Kind {
    Kind::Object(key.to_string())
}

fn field(kind: Kind) -> FieldFacts {
    FieldFacts {
        offset: 8,
        bytes: 8,
        kind,
    }
}

fn reached(project: &Project, program: u32, offsets: &[u32]) {
    for offset in offsets {
        let key = at(program, *offset);
        assert!(project.reached_from_outside(key), "{offset:#x} is reached");
    }
}

fn unreached(project: &Project, program: u32, offsets: &[u32]) {
    for offset in offsets {
        let key = at(program, *offset);
        assert!(
            !project.reached_from_outside(key),
            "{offset:#x} is unreached"
        );
    }
}

#[test]
fn a_function_is_reached_from_outside_only_where_something_outside_names_it() {
    let mut project = stated(&[P, Q]);
    // Q's code calls P's function at 0x40, and the engine runs P's at 0x10.
    project.naming.functions.extend([at(P, 0x40), at(P, 0x10)]);
    settle(&mut project);
    reached(&project, P, &[0x40, 0x10]);
    unreached(&project, P, &[0x50]);
    unreached(&project, Q, &[0x40]);
    // A program no part states proves nothing, nor does a project no merge settled.
    reached(&project, R, &[0x40]);
    assert!(Project::empty().reached_from_outside(at(P, 0x50)));
    assert!(Project::empty().type_named_outside(T));
    assert_eq!(project.naming, Naming::default());
}

#[test]
fn parts_merge_to_what_every_part_names_in_any_order() {
    let parts = || {
        let own = stated(&[P]);
        let mut theirs = stated(&[Q]);
        theirs.naming.functions.insert(at(P, 0x40));
        theirs.naming.rows.push(row(V, Q, None, &[], &[Some(0x80)]));
        vec![own, theirs]
    };
    let merged = index::merge(parts());
    reached(&merged, P, &[0x40]);
    unreached(&merged, P, &[0x50]);
    unreached(&merged, Q, &[0x80]);
    assert!(!merged.type_named_outside(V));
    let saved = index::save(&merged).expect("the project saves");
    let mut turned = parts();
    turned.reverse();
    let again = index::save(&index::merge(turned)).expect("the project saves");
    assert!(again == saved);
}

#[test]
fn code_no_part_read_proves_nothing_of_what_it_may_name() {
    let mut project = stated(&[P, Q, R]);
    project.naming.rows = vec![row(N, R, None, &[], &[Some(0x60)]), bare(T)];
    // A function of Q that did not decode, where Q imports P.
    project.naming.unread(Q, [P]);
    settle(&mut project);
    reached(&project, P, &[0x50]);
    reached(&project, Q, &[0x50]);
    assert!(project.type_named_outside(T));
    // R is read whole, but such code may call through any slot.
    unreached(&project, R, &[0x50]);
    reached(&project, R, &[0x60]);

    // A body of the set that did not read as a program leaves every program open.
    let mut project = stated(&[P]);
    project.naming.unread_file = true;
    settle(&mut project);
    reached(&project, P, &[0x50]);

    // A listing states no value's type: no type of what it imports is unnamed, though its functions may be unreached.
    let mut project = stated(&[P]);
    project.naming.rows = vec![bare(T)];
    listed(&mut project, &[P]);
    settle(&mut project);
    assert!(project.type_named_outside(T));
    unreached(&project, P, &[0x50]);

    // A settled project told that a program of its install was not read proves nothing.
    let mut project = stated(&[P]);
    project.naming.rows = vec![bare(T)];
    settle(&mut project);
    unreached(&project, P, &[0x50]);
    assert!(!project.type_named_outside(T));
    index::left_unread(&mut project);
    reached(&project, P, &[0x50]);
    assert!(project.type_named_outside(T));
}

#[test]
fn a_call_through_a_class_reaches_what_the_slot_holds_in_each_type_below_it() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![
        row(T, P, None, &[], &[None, Some(0x100), Some(0x110)]),
        row(
            U,
            P,
            Some(T),
            &[],
            &[None, Some(0x200), Some(0x110), Some(0x300)],
        ),
        // A type of a line no row states may be below any type.
        row(N, P, Some(UNSTATED), &[], &[None, Some(0x400)]),
        row(M, P, None, &[], &[None, Some(0x500)]),
    ];
    // Q calls slot 1 through T; P itself calls slot 3 through U, which is no way in.
    project.naming.slots.insert((Some(Q), T, 1));
    project.naming.slots.insert((Some(P), U, 3));
    settle(&mut project);
    reached(&project, P, &[0x100, 0x200, 0x400]);
    unreached(&project, P, &[0x110, 0x300, 0x500]);

    // A class the runtime holds: an object may be of it where its line runs up to it, or to a
    // builtin no fact places, which may also implement whatever the class is.
    let through = |ty: TypeKey, slot: u32| {
        let mut project = stated(&[P, Q]);
        project.naming.rows = vec![
            row(T, P, Some(OBJECT), &[], &[None, Some(0x100), Some(0x110)]),
            row(
                M,
                P,
                Some(VALUE_TYPE),
                &[],
                &[None, Some(0x500), Some(0x510)],
            ),
            row(N, P, Some(CLASS), &[], &[None, Some(0x600), Some(0x610)]),
        ];
        project.naming.slots.insert((Some(Q), ty, slot));
        settle(&mut project);
        project
    };
    let rooted = through(OBJECT, 2);
    reached(&rooted, P, &[0x110, 0x510, 0x610]);
    unreached(&rooted, P, &[0x100, 0x500]);
    let other = through(OTHER, 1);
    reached(&other, P, &[0x600, 0x610]);
    unreached(&other, P, &[0x100, 0x110, 0x500, 0x510]);
}

#[test]
fn a_call_through_an_interface_reaches_every_slot_of_each_type_below_it() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![
        row(T, P, None, &[LISTED], &[Some(0x100), Some(0x110)]),
        row(U, P, Some(T), &[], &[Some(0x200), Some(0x110)]),
        row(M, P, None, &[], &[Some(0x500)]),
        row(B, P, Some(OBJECT), &[], &[Some(0x700)]),
        // A builtin above N may implement it without N listing it.
        row(N, P, Some(CLASS), &[], &[Some(0x600)]),
    ];
    project.naming.slots.insert((Some(Q), LISTED, 0));
    settle(&mut project);
    reached(&project, P, &[0x100, 0x110, 0x200, 0x600]);
    unreached(&project, P, &[0x500, 0x700]);
}

#[test]
fn a_function_a_type_of_another_program_holds_or_derives_is_reached() {
    let mut project = stated(&[P, Q]);
    // V derives from T, holds T's function in slot 1 and fills slot 2 itself.
    let mut theirs = row(V, Q, Some(T), &[], &[]);
    theirs.vtable = vec![None, Some(at(P, 0x100)), Some(at(Q, 0x80))];
    project.naming.rows = vec![
        row(T, P, None, &[], &[None, Some(0x100), Some(0x120)]),
        row(M, P, None, &[], &[Some(0x130)]),
        theirs,
    ];
    settle(&mut project);
    reached(&project, P, &[0x100, 0x120]);
    unreached(&project, P, &[0x130]);
    unreached(&project, Q, &[0x80]);
}

#[test]
fn a_type_is_named_outside_where_code_or_a_table_of_another_program_names_it() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![
        bare(T),
        bare(M),
        bare(N),
        bare(U),
        row(U, Q, None, &[], &[]),
    ];
    project
        .naming
        .types
        .extend([(Some(Q), T), (Some(P), M), (None, N)]);
    settle(&mut project);
    assert!(project.type_named_outside(T));
    assert!(!project.type_named_outside(M));
    // Code no part read names what it names wherever it is.
    assert!(project.type_named_outside(N));
    // Two programs define U: neither is its one home.
    assert!(project.type_named_outside(U));
    assert!(project.type_named_outside(CLASS));
    assert!(project.type_named_outside(UNSTATED));
}

#[test]
fn a_type_is_named_outside_where_what_another_program_sees_states_it() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![bare(T), bare(M), bare(N), bare(U)];
    project.naming.functions.insert(at(P, 0x40));
    let one = |kind: Kind| Param { kind, cells: 1 };
    // P's function called from outside takes an M; one only P calls gives an N.
    let takes = Signature {
        params: vec![one(object(M))],
        returns: 0,
        result: Kind::Unknown,
    };
    project.signatures.insert(at(P, 0x40), takes);
    let gives = Signature {
        params: Vec::new(),
        returns: 1,
        result: object(N),
    };
    project.signatures.insert(at(P, 0x50), gives);
    // A function of Q gives arrays of T, and is a member of U.
    let arrays = Signature {
        params: Vec::new(),
        returns: 1,
        result: Kind::Array(Box::new(object(T))),
    };
    project.signatures.insert(at(Q, 0x10), arrays);
    let member = Member {
        owner: U,
        role: Role::Method,
        is_static: true,
        slot: None,
    };
    project.members.insert(at(Q, 0x10), member);
    settle(&mut project);
    for named in [M, T, U] {
        assert!(project.type_named_outside(named), "{named}");
    }
    assert!(!project.type_named_outside(N));
}

#[test]
fn a_type_named_outside_names_what_it_holds_and_derives_from() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![
        bare(T),
        bare(M),
        bare(N),
        bare(B),
        bare(X),
        row(V, Q, None, &[], &[]),
    ];
    project.naming.types.insert((Some(Q), T));
    let facts = TypeFacts {
        home: P,
        base: Some(B),
        ..TypeFacts::default()
    };
    project.types.insert(T, facts);
    project.fields.insert(T, vec![field(object(M))]);
    let value = Kind::Value {
        ty: N.to_string(),
        cells: 2,
    };
    project.fields.insert(M, vec![field(value)]);
    settle(&mut project);
    for named in [T, B, M, N] {
        assert!(project.type_named_outside(named), "{named}");
    }
    assert!(!project.type_named_outside(X));
    assert!(!project.type_named_outside(V));

    // Fields settled again are read again: one of Q's own type that holds an X names X outside P.
    project.fields.insert(V, vec![field(object(X))]);
    close(&mut project);
    assert!(project.type_named_outside(X));
    assert!(!project.type_named_outside(V));
}

#[test]
fn a_global_another_program_reads_names_its_kind_outside_its_program() {
    let cells = |shared: bool| {
        let mut project = stated(&[P, Q]);
        project.naming.rows = vec![bare(M), bare(N)];
        let mut info = ProgramInfo {
            program: P,
            ..ProgramInfo::default()
        };
        info.globals.insert(0x8, object(M));
        info.static_cells.push(field(object(N)));
        project.programs.insert(P, info);
        if shared {
            project.naming.shared_cells.insert(P);
        }
        settle(&mut project);
        project
    };
    let own = cells(false);
    assert!(!own.type_named_outside(M) && !own.type_named_outside(N));
    let shared = cells(true);
    assert!(shared.type_named_outside(M) && shared.type_named_outside(N));
}

#[test]
fn what_code_names_of_other_programs_is_noted() {
    let vm = |key: TypeKey| VmType {
        name: key.to_string(),
        byte_size: 16,
        id: 1,
        flags: 0,
    };
    let call = |callee: Callee| Op::Call {
        callee,
        args: 0,
        returns: 0,
    };
    let imported = Callee::Import {
        module: format!("program_{P:08X}"),
        name: "func_000040".to_owned(),
        program: P,
        at: 0x40,
    };
    let local = Callee::Local {
        at: 0x50,
        name: "func_000050".to_owned(),
    };
    let ops = vec![
        call(imported),
        call(local),
        Op::Push {
            e: Expr::FunctionRef {
                program: P,
                at: 0x60,
            },
            cells: 1,
        },
        Op::VirtualFunctionRef { ty: vm(T), slot: 2 },
        Op::Set(Var::Global {
            program: P,
            offset: 0,
            statics: false,
        }),
        Op::Push {
            e: Expr::NewObject(vm(M)),
            cells: 1,
        },
        Op::Set(Var::Global {
            program: Q,
            offset: 8,
            statics: true,
        }),
    ];
    let walk = |namer: Namer| {
        let mut naming = Naming::default();
        let mut seen = Seen {
            naming: &mut naming,
            own: Q,
            namer,
        };
        for op in &ops {
            seen.op(op);
        }
        naming
    };
    let read = walk(Some(Q));
    assert_eq!(read.functions, BTreeSet::from([at(P, 0x40), at(P, 0x60)]));
    assert_eq!(read.slots, BTreeSet::from([(Some(Q), T, 2)]));
    assert_eq!(read.types, BTreeSet::from([(Some(Q), T), (Some(Q), M)]));
    assert_eq!(read.shared_cells, BTreeSet::from([P]));
    assert!(!read.any_slot);
    // Code that did not lift names its own program's functions too.
    let unread = walk(None);
    assert!(unread.functions.contains(&at(Q, 0x50)));

    // A slot of a type no key names may be any slot.
    let mut naming = Naming::default();
    let mut seen = Seen {
        naming: &mut naming,
        own: Q,
        namer: Some(Q),
    };
    seen.op(&call(Callee::Virtual {
        class: "an_unkeyed_class".to_owned(),
        method: "Method_1".to_owned(),
        ty: "an_unkeyed_class".to_owned(),
        slot: 1,
    }));
    assert!(naming.any_slot);
}

#[test]
fn a_reference_whose_top_byte_holds_flags_names_the_function_at_its_offset() {
    let mut naming = Naming::default();
    let mut seen = Seen {
        naming: &mut naming,
        own: Q,
        namer: Some(Q),
    };
    seen.op(&Op::Push {
        e: Expr::FunctionRef {
            program: P,
            at: 0x4000_0040,
        },
        cells: 1,
    });
    assert_eq!(naming.functions, BTreeSet::from([at(P, 0x40)]));
    let mut project = stated(&[P, Q]);
    project.naming = naming;
    project.naming.stated.extend([P, Q]);
    settle(&mut project);
    reached(&project, P, &[0x40]);
}

#[test]
fn a_settled_table_reads_back_and_a_view_keeps_what_its_programs_are_asked() {
    let mut project = stated(&[P, Q]);
    project.naming.rows = vec![bare(T), row(V, Q, None, &[], &[])];
    project.naming.functions.insert(at(P, 0x40));
    let import = ImportInfo {
        program: P,
        first: 1,
        end: 2,
    };
    for (program, imports) in [(P, Vec::new()), (Q, vec![import])] {
        let info = ProgramInfo {
            program,
            imports,
            ..ProgramInfo::default()
        };
        project.programs.insert(program, info);
    }
    settle(&mut project);
    let saved = index::save(&project).expect("the project saves");
    let read = index::load(&saved).expect("the file reads back");
    assert_eq!(read, project);
    // A file kept before the table answers that anything may be reached and named.
    let before = index::save(&Project::default()).expect("the project saves");
    let before = index::load(&before).expect("the file reads back");
    assert!(before.reached_from_outside(at(P, 0x50)));
    assert!(before.type_named_outside(T));

    let seen = index::view(&project, P);
    unreached(&seen, P, &[0x50]);
    reached(&seen, P, &[0x40]);
    assert!(!seen.type_named_outside(T));
    // What a view leaves out it proves nothing of.
    reached(&seen, Q, &[0x50]);
    assert!(seen.type_named_outside(V));
    let both = index::view(&project, Q);
    unreached(&both, Q, &[0x50]);
    unreached(&both, P, &[0x50]);
    assert!(!both.type_named_outside(V));
}

#[test]
fn a_file_read_back_is_looked_up_whatever_order_it_holds() {
    let project = Project {
        outside: Outside {
            closed: vec![Q, P],
            reached: BTreeMap::from([(P, vec![0x60, 0x50, 0x40])]),
            unnamed: vec![(M, P), (T, P), (M, Q)],
            shared_cells: Vec::new(),
        },
        ..Project::default()
    };
    let saved = index::save(&project).expect("the project saves");
    let read = index::load(&saved).expect("the file reads back");
    reached(&read, P, &[0x40, 0x50, 0x60]);
    unreached(&read, P, &[0x70]);
    unreached(&read, Q, &[0x70]);
    assert!(!read.type_named_outside(T));
    // A type stated of two homes has neither.
    assert!(read.type_named_outside(M));
}
