use or_io::gta5::shaders::sgd5::Sgd5Constant;
use or_io::hash::string_hash_str;

use super::*;

/// A package of `len` bytes whose list words state `count` records at `at`, with `list` written there.
fn package(len: usize, at: u64, count: u32, list: &[[u32; 5]]) -> Vec<u8> {
    let mut bytes = vec![0xA5u8; len];
    bytes[LIST_AT..LIST_AT + 8].copy_from_slice(&at.to_le_bytes());
    bytes[LIST_COUNT..LIST_COUNT + 4].copy_from_slice(&count.to_le_bytes());
    let start = PACKAGE_BASE.wrapping_add(at as usize);
    for (i, word) in list.iter().flatten().enumerate() {
        let at = start + 4 * i;
        if let Some(slot) = bytes.get_mut(at..at + 4) {
            slot.copy_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}

fn record(class: Class, register: u32, root: u8, offset: u32) -> Record {
    Record {
        class,
        register,
        root,
        offset,
        dwords: 4,
        stride: 0,
    }
}

fn resource(code: u8, slot: u16, name_hash: u32) -> Sgd5Resource {
    Sgd5Resource {
        code,
        flags: 0,
        slot,
        members: 0,
        buffer_size: 64,
        name_hash,
        semantic_hash: name_hash,
    }
}

fn constant(name_hash: u32, semantic_hash: u32) -> Sgd5Constant {
    Sgd5Constant {
        code: 0x0C,
        count: 0,
        offset: 0,
        default_offset: 0,
        default: None,
        name_hash,
        semantic_hash,
        cbuffer: None,
    }
}

#[test]
fn a_package_lists_each_binding_by_class_register_table_and_place() {
    let list = [
        // An image, eight dwords at dword 6 of the table in user register 7.
        [0x300, 6, 41, 4, 0x0708],
        // A constant buffer at dword 12 of the table in user register 11.
        [0x302, 12, 3, 0, 0x1_0B04],
        [0x303, 2, 9, 12, 0x0704],
        [0x401, 0, 5, 0, 0x6_0004],
        [0x100, 0, 2, 1, 0x3_0D04],
        [2, 0, 502, 0, 0x4_0E04],
    ];
    let read = records(&package(0x200, 0x100, 6, &list)).unwrap();
    assert_eq!(
        read[0],
        Record {
            class: Class::Srv,
            register: 41,
            root: 7,
            offset: 24,
            dwords: 8,
            stride: 4,
        }
    );
    assert_eq!(
        (
            read[1].class,
            read[1].register,
            read[1].root,
            read[1].offset
        ),
        (Class::Cbv, 3, 11, 48)
    );
    let classes: Vec<Class> = read.iter().map(|r| r.class).collect();
    assert_eq!(
        classes,
        [
            Class::Srv,
            Class::Cbv,
            Class::Uav,
            Class::Sampler,
            Class::Address,
            Class::RootConstants
        ]
    );
    assert_eq!(
        (read[4].root, read[5].root, read[5].register),
        (13, 14, 502)
    );
}

#[test]
fn a_package_with_no_list_binds_nothing_and_a_bad_one_is_refused() {
    assert_eq!(records(&package(0x200, NO_LIST, 3, &[])), Ok(Vec::new()));
    assert_eq!(records(&[0u8; 0xF8]), Err(RecordError::Short));
    // Two records stated where one fits.
    let list = [[0x300, 0, 1, 4, 0x0808]];
    assert_eq!(
        records(&package(0x110 + 20, 0x100, 2, &list)),
        Err(RecordError::Outside {
            at: 0x100,
            count: 2
        })
    );
    let list = [[0x302, 0, 1, 0, 0x0804], [0x777, 0, 1, 0, 0x0804]];
    assert_eq!(
        records(&package(0x200, 0x100, 2, &list)),
        Err(RecordError::Type {
            index: 1,
            word: 0x777
        })
    );
}

#[test]
fn a_table_place_joins_the_one_record_there_and_the_table_s_record_of_its_class_and_register() {
    let records = [
        record(Class::Srv, 41, 7, 24),
        record(Class::Cbv, 3, 7, 56),
        // The same register as the constant buffer, of another class.
        record(Class::Uav, 3, 7, 88),
        // A register past 255, which the table states less 256 under a code with bit 7 set.
        record(Class::Srv, 275, 11, 0),
        record(Class::Srv, 60, 11, 32),
        record(Class::Address, 4, 13, 0),
        // A sampler states no place: it is never the record at one.
        record(Class::Sampler, 5, 7, 24),
    ];
    let table = Sgd5Table {
        resources: vec![
            resource(0x02, 41, 0xA000_0001),
            resource(0x30, 3, 0xA000_0002),
            resource(0x14, 3, 0xA000_0003),
            resource(0x81, 19, 0xA000_0004),
            resource(0x07, 4, 0xA000_0005),
        ],
        ..Sgd5Table::default()
    };
    let name = |place: Place| match join(place, &records, &table) {
        Join::Named { named, .. } => Some(named.name_hash),
        _ => None,
    };
    let at = |root: u8, offset: u32| Place::Table { root, offset };
    assert_eq!(name(at(7, 24)), Some(0xA000_0001));
    assert_eq!(name(at(7, 56)), Some(0xA000_0002));
    assert_eq!(name(at(7, 88)), Some(0xA000_0003));
    assert_eq!(name(at(11, 0)), Some(0xA000_0004));
    assert_eq!(name(Place::Address { root: 13 }), Some(0xA000_0005));

    // The same offset in another table, and an offset between two records.
    assert_eq!(
        join(at(11, 24), &records, &table),
        Join::NoRecord {
            root: 11,
            offset: 24
        }
    );
    assert_eq!(
        join(at(7, 40), &records, &table),
        Join::NoRecord {
            root: 7,
            offset: 40
        }
    );
    // A record whose register the table does not state.
    assert_eq!(
        join(at(11, 32), &records, &table),
        Join::NoName { record: records[4] }
    );
    assert_eq!(join(Place::Sampler, &records, &table), Join::Sampler);
    assert_eq!(join(Place::UserData, &records, &table), Join::UserData);
    // A table's own dwords are the game's heap, whatever records the table holds.
    let words = Resource {
        kind: ResourceKind::TableWords,
        at: Some(rdna::Loc {
            root: 7,
            path: Vec::new(),
            offset: 0,
            index: None,
        }),
        words: None,
        group: 0,
        binding: 0,
        count: None,
        written: false,
    };
    assert_eq!(place(&words), Place::Words { root: 7 });
    assert_eq!(join(place(&words), &records, &table), Join::Words);
    assert!(!Join::Words.joins());
    assert_eq!(join(Place::Picked, &records, &table), Join::Picked);
    assert!(Join::Sampler.joins() && !Join::UserData.joins() && !Join::Picked.joins());
}

#[test]
fn two_records_at_a_place_or_two_names_for_a_register_join_nothing() {
    let twice = [record(Class::Srv, 41, 7, 24), record(Class::Uav, 9, 7, 24)];
    let table = Sgd5Table {
        resources: vec![resource(0x02, 41, 1), resource(0x05, 41, 2)],
        ..Sgd5Table::default()
    };
    let place = Place::Table {
        root: 7,
        offset: 24,
    };
    assert_eq!(
        join(place, &twice, &table),
        Join::NoRecord {
            root: 7,
            offset: 24
        }
    );
    assert_eq!(
        join(place, &twice[..1], &table),
        Join::NoName { record: twice[0] }
    );
}

#[test]
fn a_parameter_is_named_by_its_engine_name_else_by_the_hash_the_effect_states_for_a_drawable() {
    let engine = string_hash_str("gViewProj");
    assert_eq!(stated(engine, None), "gViewProj");
    let effect = Sgd5Table {
        resources: vec![Sgd5Resource {
            semantic_hash: 0xB000_0002,
            ..resource(0x02, 7, 0xA000_0002)
        }],
        constants: vec![
            constant(0xA000_0001, 0xB000_0001),
            // An effect record that states no hash for a drawable keeps its own.
            constant(0xA000_0003, 0),
        ],
        ..Sgd5Table::default()
    };
    assert_eq!(stated(0xA000_0001, Some(&effect)), "0xB0000001");
    assert_eq!(stated(0xA000_0002, Some(&effect)), "0xB0000002");
    assert_eq!(stated(0xA000_0003, Some(&effect)), "0xA0000003");
    // One the effect does not hold, and one with no effect to ask.
    assert_eq!(stated(0xA000_0009, Some(&effect)), "0xA0000009");
    assert_eq!(stated(0xA000_0001, None), "0xA0000001");
}

#[test]
fn each_refusal_has_its_census_word() {
    let cases = [
        (Unbound::NoVertexInput, "no-vertex-input"),
        (Unbound::UserData, "user-data"),
        (Unbound::Words, "descriptor-words"),
        (Unbound::EngineBuffer("0x00000001".into()), "engine-buffer"),
        (Unbound::Picked, "picked"),
        (
            Unbound::NoRecord {
                root: 8,
                offset: 32,
            },
            "no-record",
        ),
        (Unbound::NoName { register: 4 }, "no-name"),
        (Unbound::Halves, "halves"),
        (Unbound::Alone, "alone"),
        (Unbound::Stage(ShaderKind::Mesh), "stage"),
        (Unbound::Records(RecordError::Short), "records"),
        (Unbound::Invalid("anything".into()), "invalid"),
    ];
    for (refusal, word) in cases {
        assert_eq!(refusal.name(), word, "{refusal}");
    }
}

#[test]
fn bytes_that_are_no_container_are_refused_before_anything_is_bound() {
    let bytes: Vec<u8> = (0..96u8).map(|i| i.wrapping_mul(29) ^ 0x3C).collect();
    let refusal = bind(&bytes, &Sgd5Table::default(), None).unwrap_err();
    assert_eq!(refusal, Unbound::Container(ContainerError::NotDxbc));
}

#[test]
fn a_mesh_program_s_user_registers_are_numbered_eight_below_the_registers_its_code_names() {
    let records = [
        record(Class::Cbv, 3, 9, 160),
        record(Class::Address, 5, 11, 0),
    ];
    let table = Sgd5Table {
        resources: vec![
            resource(0x30, 3, 0xA000_0001),
            resource(0x07, 5, 0xA000_0002),
        ],
        ..Sgd5Table::default()
    };
    let name = |place: Place, kind: ShaderKind| match join(numbered(place, kind), &records, &table)
    {
        Join::Named { named, .. } => Some(named.name_hash),
        _ => None,
    };
    let table_at = |root: u8| Place::Table { root, offset: 160 };
    // The code of a mesh program reaches user register 9's table through s17, and 11's address through s19.
    assert_eq!(name(table_at(17), ShaderKind::Mesh), Some(0xA000_0001));
    assert_eq!(
        name(Place::Address { root: 19 }, ShaderKind::Mesh),
        Some(0xA000_0002)
    );
    assert_eq!(name(table_at(9), ShaderKind::Mesh), None);
    // A pixel and a vertex program name the register itself.
    assert_eq!(name(table_at(9), ShaderKind::Pixel), Some(0xA000_0001));
    assert_eq!(name(table_at(17), ShaderKind::Vertex), None);
    // A register below the shift is no user register, and what has no register is left alone.
    assert_eq!(
        numbered(Place::Words { root: 3 }, ShaderKind::Mesh),
        Place::Words { root: 251 }
    );
    assert_eq!(numbered(Place::Picked, ShaderKind::Mesh), Place::Picked);
}

#[test]
fn a_program_that_loads_texels_is_lowered_again_with_each_such_texture_stated_as_floats() {
    use naga::{Expression as E, Literal, Span};
    let at = rdna::Loc {
        root: 2,
        path: Vec::new(),
        offset: 64,
        index: None,
    };
    // A module that loads a texel of the one texture it binds.
    let mut module = naga::Module::default();
    let image = module.types.insert(
        naga::Type {
            name: None,
            inner: naga::TypeInner::Image {
                dim: naga::ImageDimension::D1,
                arrayed: false,
                class: naga::ImageClass::Sampled {
                    kind: naga::ScalarKind::Float,
                    multi: false,
                },
            },
        },
        Span::UNDEFINED,
    );
    let global = module.global_variables.append(
        naga::GlobalVariable {
            name: None,
            space: naga::AddressSpace::Handle,
            binding: Some(naga::ResourceBinding {
                group: 0,
                binding: 7,
            }),
            ty: image,
            init: None,
            memory_decorations: Default::default(),
        },
        Span::UNDEFINED,
    );
    let mut function = naga::Function::default();
    let mut put = |e| function.expressions.append(e, Span::UNDEFINED);
    let (image, coordinate) = (
        put(E::GlobalVariable(global)),
        put(E::Literal(Literal::I32(0))),
    );
    put(E::ImageLoad {
        image,
        coordinate,
        array_index: None,
        sample: None,
        level: Some(coordinate),
    });
    module.functions.append(function, Span::UNDEFINED);
    let lowered = |resources| Lowered {
        module: module.clone(),
        resources,
        approximations: Default::default(),
        half_slots: Default::default(),
        subgroups: false,
        intersections: Vec::new(),
        region: None,
    };
    let texture = Resource {
        kind: ResourceKind::Texture {
            dim: 0,
            compared: false,
        },
        at: Some(at.clone()),
        words: None,
        group: 0,
        binding: 7,
        count: None,
        written: false,
    };
    let asked = std::cell::RefCell::new(Vec::new());
    let translate = |resources: Vec<Resource>| {
        let asked = &asked;
        move |options: &Options| {
            asked.borrow_mut().push(options.image_kinds.clone());
            Ok::<_, ()>(lowered(resources.clone()))
        }
    };
    lower_by(&BTreeSet::new(), translate(vec![texture])).unwrap();
    let stated = [(at, naga::ScalarKind::Float)].into();
    assert_eq!(*asked.borrow(), [Default::default(), stated]);
    // A program that loads none is lowered once.
    asked.borrow_mut().clear();
    lower_by(&BTreeSet::new(), translate(Vec::new())).unwrap();
    assert_eq!(asked.borrow().len(), 1);
}

#[test]
fn a_native_program_that_binds_is_still_not_handed_to_a_pack_alone() {
    // The one instruction that ends a program: it binds nothing, so nothing else stops it.
    let native = Native {
        program: Program {
            kind: ShaderKind::Pixel,
            slot: 5,
            words: vec![0xBF81_0000],
            wave32: false,
            inputs: Vec::new(),
            vertex_inputs: Vec::new(),
            position_read: 0,
            front_face: false,
            launch: None,
        },
        records: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    let table = Sgd5Table::default();
    let lowered = lower(&native, &BTreeSet::new()).expect("the program lowers");
    assert!(bind_lowered(&native, lowered, &table, None).is_ok());
    assert_eq!(
        bind_alone(&native, &table, None).unwrap_err(),
        Unbound::Alone
    );
}
