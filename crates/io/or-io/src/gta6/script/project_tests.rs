use super::*;

#[test]
fn a_project_is_read_back_as_it_was_saved() {
    let (ty, base) = (TypeKey(0x0123_4567_89AB_CDEF), TypeKey(0x8000_000B));
    let function = FunctionKey::new(7, 0x40);
    let mut project = Project::default();
    let info = ProgramInfo {
        program: 7,
        imports: vec![ImportInfo {
            program: 9,
            first: 2,
            end: 5,
        }],
        types: BTreeMap::from([(0, ty)]),
        globals: BTreeMap::from([(8, Kind::Array(Box::new(Kind::F32)))]),
        strings: BTreeMap::from([(0, "a string".to_owned())]),
        ..ProgramInfo::default()
    };
    project.programs.insert(7, info);
    let facts = TypeFacts {
        byte_size: 24,
        base: Some(base),
        slots: BTreeMap::from([(3, function)]),
        generic: Some((base, vec![ty])),
        ..TypeFacts::default()
    };
    project.types.insert(ty, facts);
    let field = FieldFacts {
        offset: 8,
        bytes: 4,
        kind: Kind::Value {
            ty: base.to_string(),
            cells: 1,
        },
    };
    project.fields.insert(ty, vec![field]);
    let signature = Signature {
        params: vec![Param {
            kind: Kind::Object(ty.to_string()),
            cells: 1,
        }],
        returns: 1,
        result: Kind::Bool,
    };
    project.signatures.insert(function, signature);
    project
        .calls
        .insert(function, vec![FunctionKey::new(9, 0x10)]);
    let member = Member {
        owner: ty,
        role: Role::Constructor,
        is_static: false,
        slot: Some(3),
    };
    project.members.insert(function, member);

    let saved = index::save(&project).expect("the project saves");
    let read = index::load(&saved).expect("the file reads back");
    assert_eq!(read, project);
    // The same project is the same bytes.
    assert_eq!(index::save(&read).expect("the project saves"), saved);

    assert!(matches!(
        index::load(b"not a project"),
        Err(ProjectError::Sidecar(_))
    ));
    let other = String::from_utf8_lossy(&saved).replacen("\"version\":1", "\"version\":2", 1);
    let refused = index::load(other.as_bytes());
    assert!(
        matches!(&refused, Err(ProjectError::Sidecar(why)) if why.contains("version 2")),
        "{refused:?}"
    );
}

#[test]
fn a_type_id_is_resolved_through_the_import_ranges() {
    let (own, theirs) = (TypeKey(0x11 << 40), TypeKey(0x22 << 40));
    let mut project = Project::default();
    let importer = ProgramInfo {
        program: 1,
        imports: vec![ImportInfo {
            program: 2,
            first: 10,
            end: 12,
        }],
        types: BTreeMap::from([(0, own)]),
        ..ProgramInfo::default()
    };
    let imported = ProgramInfo {
        program: 2,
        types: BTreeMap::from([(1, theirs)]),
        ..ProgramInfo::default()
    };
    project.programs.insert(1, importer);
    project.programs.insert(2, imported);

    assert_eq!(project.type_key(1, 0), Some(own));
    assert_eq!(project.type_key(1, 11), Some(theirs));
    // An id of the range that the imported program does not define, and an id no range holds.
    assert_eq!(project.type_key(1, 10), None);
    assert_eq!(project.type_key(1, 12), None);
    assert_eq!(project.type_key(1, 0x8000_000B), Some(TypeKey(0x8000_000B)));
    assert_eq!(project.type_key(3, 0), None);
    assert_eq!(TypeKey(0x8000_000B).builtin(), Some(0x8000_000B));
    assert_eq!(own.builtin(), None);
}

#[test]
fn a_field_and_a_slot_are_found_on_a_base_type_and_a_ring_of_bases_ends() {
    let (derived, base, other) = (TypeKey(1 << 40), TypeKey(2 << 40), TypeKey(3 << 40));
    let function = FunctionKey::new(1, 0x40);
    let mut project = Project::default();
    let above = TypeFacts {
        base: Some(base),
        ..TypeFacts::default()
    };
    // A base that names its own descendant as its base: a lookup must still end.
    let below = TypeFacts {
        base: Some(derived),
        slots: BTreeMap::from([(4, function)]),
        ..TypeFacts::default()
    };
    project.types.insert(derived, above);
    project.types.insert(base, below);
    let field = FieldFacts {
        offset: 8,
        bytes: 4,
        kind: Kind::I32,
    };
    project.fields.insert(base, vec![field]);

    assert_eq!(project.field(derived, 8).map(|f| f.bytes), Some(4));
    assert_eq!(project.field(derived, 16), None);
    assert_eq!(project.slot(derived, 4), Some(function));
    assert_eq!(project.slot(derived, 5), None);
    assert!(project.is_a(derived, base));
    assert!(!project.is_a(derived, other));
    assert_eq!((function.program(), function.at()), (1, 0x40));
    assert!(Project::empty().programs.is_empty());
}

#[test]
fn the_cells_a_slot_returns_are_a_method_records_before_a_functions() {
    let (derived, base) = (TypeKey(1 << 40), TypeKey(2 << 40));
    let function = FunctionKey::new(1, 0x40);
    let mut project = Project::default();
    let above = TypeFacts {
        base: Some(base),
        ..TypeFacts::default()
    };
    let below = TypeFacts {
        slots: BTreeMap::from([(4, function), (5, function)]),
        returns: BTreeMap::from([(4, 2)]),
        ..TypeFacts::default()
    };
    project.types.insert(derived, above);
    project.types.insert(base, below);
    let one = Signature {
        returns: 1,
        ..Signature::default()
    };
    project.signatures.insert(function, one);

    // A record of the type's line states slot 4; slot 5 has only the function that fills it.
    assert_eq!(project.slot_returns(derived, 4), Some(2));
    assert_eq!(project.slot_returns(derived, 5), Some(1));
    assert_eq!(project.slot_returns(derived, 6), None);
    assert_eq!(project.slot_returns(TypeKey(3 << 40), 4), None);
}
