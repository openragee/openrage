use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::Kind;
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
