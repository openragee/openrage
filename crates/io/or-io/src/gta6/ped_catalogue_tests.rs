use super::*;
use crate::meta::instance::HashedString;

/// The name the value tree gives `hash`, as the reader asks for it.
fn n(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn structure(class: u32, members: Vec<(u32, Value)>) -> Value {
    let mut instance = MetaInstance::new(n(class));
    for (hash, value) in members {
        instance.set(n(hash), value);
    }
    Value::Struct(Box::new(instance))
}

fn hash(value: u32) -> Value {
    Value::Hash(HashedString::from_hash(value))
}

fn state(name: u32, drawable: u64, more: usize) -> Value {
    let assets = structure(
        0x147F_DAB5,
        vec![
            (lit(b"Drawable"), Value::UInt(drawable)),
            (lit(b"Albedo"), Value::UInt(drawable + 1)),
            (lit(b"Normal"), Value::UInt(drawable + 2)),
            (lit(b"Material"), Value::UInt(drawable + 3)),
            (
                MORE_ASSETS,
                Value::Map(vec![(hash(9), Value::Pointer(None)); more]),
            ),
        ],
    );
    structure(
        0x37E5_2277,
        vec![(STATE_NAME, hash(name)), (STATE_ASSETS, assets)],
    )
}

/// A catalogue of one asset kit of two states, an empty asset list, and one set kit that names
/// a state the asset kit has and one it has not.
fn catalogue() -> MetaInstance {
    let asset_kit = structure(
        0xF795_E8BA,
        vec![
            (KIT_ID, hash(0xA1)),
            (lit(b"TemplateName"), hash(0xA2)),
            (
                lit(b"States"),
                Value::Array(vec![state(0xB1, 0x1000, 0), state(0xB2, 0x2000, 3)]),
            ),
            (lit(b"Category"), hash(0xA3)),
        ],
    );
    let member = |kit, state| {
        structure(
            0x67E5_4CB2,
            vec![(KIT_ID, hash(kit)), (STATE_NAME, hash(state))],
        )
    };
    let set_kit = structure(
        0xA9D5_7751,
        vec![
            (KIT_ID, hash(0xC1)),
            // A body leaves a hash string empty, which reads as 0.
            (lit(b"TemplateName"), Value::Null),
            (
                SET_MEMBERS,
                Value::Array(vec![member(0xA1, 0xB2), member(0xA1, 0xEE)]),
            ),
            (lit(b"Scale"), Value::Float(1.5)),
        ],
    );
    let list = |class, kits: Vec<Value>| structure(class, vec![(lit(b"Kits"), Value::Array(kits))]);
    let Value::Struct(root) = structure(
        CATALOGUE,
        vec![
            (
                ASSET_LISTS,
                Value::Array(vec![
                    list(0x60BC_B6F3, vec![asset_kit]),
                    list(0x60BC_B6F3, vec![]),
                ]),
            ),
            (
                SET_LISTS,
                Value::Array(vec![list(0x4C70_A132, vec![set_kit])]),
            ),
        ],
    ) else {
        unreachable!()
    };
    *root
}

#[test]
fn a_catalogue_reads_its_kits_states_and_members_as_the_tree_states_them() {
    let read = read_tree(&catalogue()).expect("a catalogue");
    assert_eq!(read.asset_lists.len(), 2);
    assert!(read.asset_lists[1].is_empty());
    let kit = &read.asset_lists[0][0];
    assert_eq!(
        (kit.id, kit.template_name, kit.category),
        (0xA1, 0xA2, 0xA3)
    );
    assert_eq!(
        kit.states,
        [
            AssetState {
                name: 0xB1,
                drawable: 0x1000,
                albedo: 0x1001,
                normal: 0x1002,
                material: 0x1003,
                more_assets: 0,
            },
            AssetState {
                name: 0xB2,
                drawable: 0x2000,
                albedo: 0x2001,
                normal: 0x2002,
                material: 0x2003,
                more_assets: 3,
            },
        ]
    );
    let set = &read.set_lists[0][0];
    assert_eq!((set.id, set.template_name, set.scale), (0xC1, 0, 1.5));
    assert_eq!(
        set.members,
        [
            SetMember {
                kit: 0xA1,
                state: 0xB2
            },
            SetMember {
                kit: 0xA1,
                state: 0xEE
            }
        ]
    );
    // One member names a state the asset kit has; the other names none.
    assert_eq!(read.members_resolved(), 1);
}

#[test]
fn a_tree_that_is_no_catalogue_or_states_no_kits_is_refused_by_name() {
    let other = MetaInstance::new(n(0x1234_5678));
    let why = read_tree(&other).unwrap_err().to_string();
    assert!(why.contains("not of the catalogue's class"), "{why}");

    let mut no_kits = catalogue();
    no_kits.set(n(SET_LISTS), Value::UInt(7));
    let why = read_tree(&no_kits).unwrap_err().to_string();
    assert_eq!(why, "the set kit lists: is not an array the body states");

    let mut bad_state = catalogue();
    let Some(Value::Array(lists)) = bad_state.get_mut(&n(ASSET_LISTS)) else {
        unreachable!()
    };
    let Value::Struct(list) = &mut lists[0] else {
        unreachable!()
    };
    let Some(Value::Array(kits)) = list.get_mut(&n(lit(b"Kits"))) else {
        unreachable!()
    };
    let Value::Struct(kit) = &mut kits[0] else {
        unreachable!()
    };
    kit.set(
        n(lit(b"States")),
        Value::Array(vec![structure(0x37E5_2277, vec![])]),
    );
    let why = read_tree(&bad_state).unwrap_err().to_string();
    assert_eq!(why, "an asset kit's state: has no structure of assets");

    // Bytes whose root block names no class are no catalogue.
    assert!(!is_ped_catalogue(&[0u8; 0x60]));
    assert!(matches!(
        open(&[0u8; 0x60], &[]),
        Err(PedCatalogueError::NotACatalogue(0))
    ));
}
