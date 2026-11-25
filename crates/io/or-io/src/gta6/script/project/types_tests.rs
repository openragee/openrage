use std::ops::Range;

use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::ir::{Head, Returns, Stmt};
use crate::gta6::script::program::ImportRow;
use crate::gta6::script::project::Signature;

// Three program hashes, in rising order.
const A: u32 = 0x1000_0001;
const B: u32 = 0x2000_0002;
const C: u32 = 0x3000_0003;
/// The builtin a class with no base of its own has as its base.
const OBJECT: TypeKey = TypeKey(0x8000_0000);

/// The hash of a name: any 64 bits with an upper half.
fn name(n: u64) -> TypeKey {
    TypeKey(n << 40 | 0xABCD)
}

/// The record of a class of 16 bytes under the builtin object.
fn class(name: TypeKey) -> TypeRow {
    TypeRow {
        name: name.0,
        byte_size: 16,
        flags: 0x22,
        base: Some(OBJECT.0 as u32),
        ..TypeRow::default()
    }
}

fn program(hash: u32, imports: &[(u32, Range<u32>)], types: Vec<TypeRow>) -> Program {
    let imports = imports.iter().cloned();
    Program {
        program: hash,
        types,
        imports: imports
            .map(|(program, types)| ImportRow { program, types })
            .collect(),
        ..Program::default()
    }
}

/// The part of a project a compiled program's type table gives.
fn table(program: &Program) -> Project {
    let mut part = Project::default();
    of_program(&mut part, program);
    part
}

/// A type as code names it.
fn vm(name: TypeKey, id: u32, byte_size: u32, flags: u32) -> VmType {
    VmType {
        name: name.to_string(),
        byte_size,
        id,
        flags,
    }
}

/// The part of a project a listing gives whose one function makes an object of each type.
fn code(program: u32, imports: &[ImportInfo], types: &[VmType]) -> Project {
    let made = types
        .iter()
        .map(|ty| Stmt::Eval(Expr::NewObject(ty.clone())));
    let block = Block {
        at: None,
        stmts: made.collect(),
        term: Terminator::Return(None),
    };
    let head = Head {
        program,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 0,
        locals: 0,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let f = Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    };
    let info = ProgramInfo {
        program,
        imports: imports.to_vec(),
        ..ProgramInfo::default()
    };
    let mut part = Project::default();
    part.programs.insert(program, info);
    of_function(&mut part, &f);
    part
}

/// The parts as the index merges them: each part's program, then its types, and the whole settled.
fn merged(parts: Vec<Project>) -> Project {
    let mut project = Project::default();
    for mut part in parts {
        project.programs.append(&mut part.programs);
        absorb(&mut project, &mut part);
    }
    settle(&mut project);
    project
}

#[test]
fn an_id_of_an_importer_reaches_the_type_its_range_names() {
    let [derived, orphan, face, base] = [1, 2, 3, 4].map(name);
    // Ids 2 and 3 of program A are types 0 and 1 of program B; id 4 is type 0 of a program not read.
    let a = program(
        A,
        &[(B, 2..4), (C, 4..5)],
        vec![
            TypeRow {
                base: Some(3),
                interfaces: vec![2],
                ..class(derived)
            },
            TypeRow {
                base: Some(4),
                flags: NOT_ALLOCATED,
                ..class(orphan)
            },
        ],
    );
    let interface = TypeRow {
        base: None,
        flags: NOT_ALLOCATED,
        ..class(face)
    };
    let b = program(B, &[], vec![interface, class(base)]);
    let project = merged(vec![table(&a), table(&b)]);

    assert_eq!(project.type_key(A, 3), Some(base));
    let facts = &project.types[&derived];
    assert_eq!(
        (facts.home, facts.base, &facts.interfaces),
        (A, Some(base), &vec![face])
    );
    assert!(project.is_a(derived, base) && project.is_a(derived, face));
    assert_eq!(project.types[&base].base, Some(OBJECT));
    // A base in a program not read stays the home it was stated as, so the class does not read as an interface.
    assert_eq!(project.types[&orphan].base, Some(home(C, 0)));
    assert_eq!(project.types[&face].base, None);
}

#[test]
fn a_function_is_held_by_the_first_type_of_a_line_whose_table_has_it() {
    let [base, derived, ring] = [1, 2, 3].map(name);
    let above = TypeRow {
        slots: vec![Some(0x10), Some(0x20), None],
        ..class(base)
    };
    // The derived type keeps slot 0, overrides slot 1, fills slot 2 and maps a new slot onto the inherited 0x20.
    let below = TypeRow {
        base: Some(0),
        slots: vec![Some(0x10), Some(0x30), Some(0x40), Some(0x20)],
        ..class(derived)
    };
    // A type that is its own base: the walk up its line must end.
    let own = TypeRow {
        base: Some(2),
        slots: vec![Some(0x50)],
        ..class(ring)
    };
    let project = merged(vec![table(&program(A, &[], vec![above, below, own]))]);
    let held = |at| FunctionKey::new(A, at);

    assert_eq!(
        project.types[&base].slots,
        BTreeMap::from([(0, held(0x10)), (1, held(0x20))])
    );
    assert_eq!(
        project.types[&derived].slots,
        BTreeMap::from([(1, held(0x30)), (2, held(0x40))])
    );
    assert_eq!(project.slot(derived, 0), Some(held(0x10)));
    assert_eq!(project.slot(derived, 1), Some(held(0x30)));
    assert_eq!(project.slot(base, 2), None);
    assert!(project.types[&ring].slots.is_empty());
}

#[test]
fn two_programs_that_define_one_name_alike_are_one_type_with_no_slot() {
    let shared = name(7);
    let one = TypeRow {
        slots: vec![Some(0x10)],
        ..class(shared)
    };
    // The other home sets one more flag, of those no meaning is known for.
    let other = TypeRow {
        flags: 0x122,
        slots: vec![Some(0x90)],
        ..class(shared)
    };
    let parts = [(B, other), (A, one)].map(|(hash, row)| table(&program(hash, &[], vec![row])));
    let project = merged(parts.into());

    let facts = &project.types[&shared];
    assert_eq!(
        (facts.home, facts.byte_size, facts.flags, facts.base),
        (A, 16, 0x22, Some(OBJECT))
    );
    assert!(facts.slots.is_empty());
    assert_eq!(project.type_key(A, 0), Some(shared));
    assert_eq!(project.type_key(B, 0), Some(shared));
}

#[test]
fn two_programs_that_define_one_name_otherwise_leave_it_without_facts() {
    let shared = name(7);
    let others = [
        TypeRow {
            byte_size: 24,
            ..class(shared)
        },
        // One more flag, and this one says the type is a struct.
        TypeRow {
            flags: 0x26,
            ..class(shared)
        },
        TypeRow {
            base: Some(0x8000_0001),
            ..class(shared)
        },
        TypeRow {
            interfaces: vec![0x8000_0050],
            ..class(shared)
        },
    ];
    for other in others {
        // A third home that agrees with the first does not bring the name back.
        let homes = [(A, class(shared)), (B, other), (C, class(shared))];
        let parts = homes.map(|(hash, row)| table(&program(hash, &[], vec![row])));
        let project = merged(parts.into());
        assert!(!project.types.contains_key(&shared));
        assert_eq!(project.type_key(B, 0), Some(shared));
    }
}

#[test]
fn a_type_code_names_by_an_importers_id_is_kept_under_the_program_that_defines_it() {
    let (own, theirs) = (name(1), name(2));
    let imports = [ImportInfo {
        program: B,
        first: 4,
        end: 8,
    }];
    let a = code(
        A,
        &imports,
        &[vm(own, 0, 24, 0x22), vm(theirs, 5, 16, 0x07)],
    );
    let project = merged(vec![a, code(B, &[], &[])]);

    assert_eq!(project.type_key(A, 5), Some(theirs));
    assert_eq!(project.programs[&A].types, BTreeMap::from([(0, own)]));
    assert_eq!(project.programs[&B].types, BTreeMap::from([(1, theirs)]));
    let facts = &project.types[&theirs];
    assert_eq!(
        (facts.home, facts.byte_size, facts.flags, facts.base),
        (B, 16, 0x07, None)
    );
    assert_eq!(project.types[&own].home, A);
}

#[test]
fn a_program_states_its_own_types_whatever_an_importers_code_says() {
    let (theirs, other) = (name(2), name(3));
    let imports = [ImportInfo {
        program: B,
        first: 4,
        end: 8,
    }];
    // Program A's code gives type 1 of program B another name and another size than B's table does.
    let a = code(A, &imports, &[vm(other, 5, 24, 0x22)]);
    let b = table(&program(B, &[], vec![class(name(1)), class(theirs)]));
    // Whichever of the two is said first.
    for parts in [vec![a.clone(), b.clone()], vec![b, a]] {
        let project = merged(parts);
        assert_eq!(project.type_key(A, 5), Some(theirs));
        assert_eq!(project.types[&theirs].byte_size, 16);
        assert!(!project.types.contains_key(&other));
        assert!(project.programs[&A].types.is_empty());
    }
}

#[test]
fn two_listings_that_state_a_type_otherwise_leave_it_without_facts() {
    let theirs = name(2);
    let through = |first| {
        [ImportInfo {
            program: B,
            first,
            end: first + 4,
        }]
    };
    // Type 1 of program B is 16 bytes to program A's code and 24 to program C's, and B's own code is silent.
    let a = code(A, &through(4), &[vm(theirs, 5, 16, 0x22)]);
    let c = code(C, &through(2), &[vm(theirs, 3, 24, 0x22)]);
    let project = merged(vec![a, code(B, &[], &[]), c]);

    assert_eq!(project.type_key(A, 5), Some(theirs));
    assert_eq!(project.type_key(C, 3), Some(theirs));
    assert!(!project.types.contains_key(&theirs));
}

#[test]
fn an_id_two_importers_name_differently_names_no_type() {
    let (theirs, other) = (name(2), name(3));
    let through = |first| {
        [ImportInfo {
            program: B,
            first,
            end: first + 4,
        }]
    };
    let a = code(A, &through(4), &[vm(theirs, 5, 16, 0x22)]);
    let c = code(C, &through(2), &[vm(other, 3, 16, 0x22)]);
    let project = merged(vec![a, code(B, &[], &[]), c]);

    assert_eq!(project.type_key(A, 5), None);
    assert_eq!(project.type_key(C, 3), None);
    assert!(project.types.is_empty());
}

#[test]
fn a_builtin_type_is_what_every_listing_says_of_it() {
    let key = TypeKey(0x8000_000B);
    let int = VmType {
        name: "System.Int32".to_owned(),
        byte_size: 4,
        id: 0x8000_000B,
        flags: 0x2005,
    };
    // Program C's code names it with no flags, as a front end with no table of builtins does: that says nothing.
    let unknown = VmType {
        byte_size: 0,
        flags: 0,
        ..int.clone()
    };
    let project = merged(vec![
        code(A, &[], std::slice::from_ref(&int)),
        code(B, &[], std::slice::from_ref(&int)),
        code(C, &[], &[unknown]),
    ]);
    let facts = &project.types[&key];
    assert_eq!((facts.byte_size, facts.base), (4, None));
    assert!(facts.is_value());

    let wide = VmType {
        byte_size: 8,
        ..int.clone()
    };
    let project = merged(vec![code(A, &[], &[int]), code(B, &[], &[wide])]);
    assert!(!project.types.contains_key(&key));
}

#[test]
fn a_type_has_no_base_only_where_its_record_says_so_or_objects_are_made_of_it() {
    let [an_enum, face, an_abstract, unsaid, plain] = [1, 2, 3, 4, 5].map(name);
    let records = vec![
        TypeRow {
            base: Some(0x8000_0029),
            flags: 0x48,
            constants: vec![0, 1, 0xFFFF_FFFF],
            ..class(an_enum)
        },
        // An interface: its record states no base.
        TypeRow {
            base: None,
            flags: NOT_ALLOCATED,
            ..class(face)
        },
        TypeRow {
            flags: NOT_ALLOCATED,
            ..class(an_abstract)
        },
    ];
    // Types of program B that only its code names: one no object is made of, and a class.
    let said = [vm(unsaid, 0, 0, NOT_ALLOCATED), vm(plain, 1, 16, 0x22)];
    let project = merged(vec![table(&program(A, &[], records)), code(B, &[], &said)]);

    assert_eq!(project.types[&an_enum].constants, [0, 1, 0xFFFF_FFFF]);
    assert_eq!(project.types[&face].base, None);
    assert_eq!(project.types[&an_abstract].base, Some(OBJECT));
    assert_eq!(project.types[&plain].base, None);
    // An interface or an abstract class, and nothing says which: with no base it would read as an interface.
    assert!(!project.types.contains_key(&unsaid));
    assert_eq!(project.type_key(B, 0), Some(unsaid));
}

#[test]
fn an_instance_of_a_generic_type_prints_as_csharp_writes_it() {
    let [of, item, pair, nested, ring] = [1, 2, 3, 4, 5].map(name);
    let int = TypeKey(0x8000_000B);
    let instance = |args: [u32; 2], key| TypeRow {
        generic: Some((0, args.to_vec())),
        ..class(key)
    };
    let records = vec![
        // A generic type's own record names itself, with the arguments it was first made with.
        instance([1, 1], of),
        class(item),
        instance([int.0 as u32, 1], pair),
        instance([2, 1], nested),
        // A type that is its own argument: its name must end.
        instance([4, 4], ring),
    ];
    let project = merged(vec![table(&program(A, &[], records))]);
    let mut names = Names::default();
    names.types.insert(item.0, "Item".to_owned());

    // The generic type has no name: its hash, then the arguments.
    assert_eq!(
        display(&project, &names, pair),
        format!("{of}<{int}, Item>")
    );
    names.types.insert(of.0, "Dictionary_2".to_owned());
    assert_eq!(
        display(&project, &names, pair),
        "Dictionary<type_8000000B, Item>"
    );
    assert_eq!(
        display(&project, &names, nested),
        "Dictionary<Dictionary<type_8000000B, Item>, Item>"
    );
    assert_eq!(display(&project, &names, of), "Dictionary_2");
    // With no project, and for a type that is no instance, a name is what it always was.
    assert_eq!(display(Project::empty(), &names, pair), pair.to_string());
    assert_eq!(display(&project, &Names::default(), item), item.to_string());
    // A count at the end of the name that is not the count of the arguments is part of the name.
    names.types.insert(of.0, "Pair_3".to_owned());
    assert_eq!(
        display(&project, &names, pair),
        "Pair_3<type_8000000B, Item>"
    );
    let spelled = display(&project, &names, ring);
    assert!(
        spelled.matches('<').count() <= MAX_SPELLED as usize,
        "{spelled}"
    );
    assert!(spelled.ends_with('>'), "{spelled}");
}

/// A class that implements the type its program numbers 2 from slot `first`, holding there the functions at `fills` of its own program.
fn implementer(n: u64, first: u32, fills: &[u32]) -> TypeRow {
    let mut vtable = vec![SlotRow::Empty; first as usize];
    vtable.extend(fills.iter().map(|at| SlotRow::Function {
        import: 0,
        at: *at,
        unboxes: false,
    }));
    TypeRow {
        interfaces: vec![2],
        first_slots: vec![first],
        vtable,
        ..class(name(n))
    }
}

/// The project of an interface of program `A` and the two classes of `B` that implement it, each function of `B` returning the cells given.
fn implemented(classes: [TypeRow; 2], returns: &[(u32, u32)], stated: Option<u32>) -> Project {
    let interface = TypeRow {
        flags: 0x2_0000,
        base: None,
        ..class(name(1))
    };
    let mut home = table(&program(A, &[], vec![interface]));
    // What the interface's own method record says of its slot 0, where it has one.
    if let (Some(cells), Some(facts)) = (stated, home.types.values_mut().next()) {
        facts.returns.insert(0, cells);
    }
    // A program's own types come first, then those of each import.
    let part = table(&program(B, &[(A, 2..3)], classes.to_vec()));
    let mut project = merged(vec![home, part]);
    for (at, cells) in returns {
        let signature = Signature {
            returns: *cells,
            ..Signature::default()
        };
        (project.signatures).insert(FunctionKey::new(B, *at), signature);
    }
    settle_implemented(&mut project);
    project
}

#[test]
fn a_slot_of_an_interface_returns_what_every_type_that_fills_it_returns() {
    let both = [implementer(2, 3, &[0x10, 0x20]), implementer(3, 5, &[0x30])];
    let returns = [(0x10, 2), (0x20, 0), (0x30, 2)];
    let project = implemented(both.clone(), &returns, None);
    assert_eq!(project.slot_returns(name(1), 0), Some(2));
    // Only one class fills the second slot.
    assert_eq!(project.slot_returns(name(1), 1), Some(0));
    assert!(project.implemented.is_empty());

    // Two classes that return otherwise leave the slot unstated.
    let apart = [(0x10, 2), (0x20, 0), (0x30, 1)];
    let project = implemented(both.clone(), &apart, None);
    assert_eq!(project.slot_returns(name(1), 0), None);
    assert_eq!(project.slot_returns(name(1), 1), Some(0));

    // The interface's own record of a slot stands whatever fills it.
    let project = implemented(both, &returns, Some(7));
    assert_eq!(project.slot_returns(name(1), 0), Some(7));
}
