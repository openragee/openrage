use super::*;
use crate::meta::instance::HashedString;

fn hash(value: u32) -> Value {
    Value::Hash(HashedString::from_hash(value))
}

/// A texture entry of `word`, its names spelt as a body writes an unnamed member.
fn texture(word: u64, names: &[(u32, u64)]) -> Value {
    let mut entry = MetaInstance::new("hash_28689F25");
    entry.set("guid", Value::UInt(word));
    let pairs = names.iter().map(|(n, k)| (hash(*n), Value::UInt(*k)));
    entry.set(format!("hash_{NAMES:08X}"), Value::Map(pairs.collect()));
    Value::Struct(Box::new(entry))
}

fn map(entries: Vec<(u64, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Value::UInt(k), v))
            .collect(),
    )
}

/// An asset list of one albedo, normal and material, and two wrinkle entries.
fn list() -> MetaInstance {
    let mut tree = MetaInstance::new("MetaPedAssetList");
    tree.set("drawables", map(Vec::new()));
    tree.set(
        format!("hash_{ALBEDOS:08X}"),
        map(vec![(0xA1, texture(0xA1, &[]))]),
    );
    tree.set(
        format!("hash_{NORMALS:08X}"),
        map(vec![(
            0xB1,
            texture(0xB1, &[(MC, 0xC0), (name_hash32(b"nm"), 0xB1)]),
        )]),
    );
    tree.set(
        "materials",
        map(vec![(0xD1, texture(0xD1, &[(MA, 0xE0), (MB, 0xE1)]))]),
    );
    tree.set(
        format!("hash_{WRINKLES:08X}"),
        map(vec![
            (0xF1, texture(0xF1, &[(WRINKLE, 0xF0)])),
            (
                0xF2,
                texture(0xF2, &[(WRINKLE_MASK, 0xF2), (WRINKLE_INDEX, 0xF3)]),
            ),
        ]),
    );
    tree
}

fn state(name: u32, drawable: u64, albedo: u64, more: Vec<(u32, u64)>) -> AssetState {
    AssetState {
        name,
        drawable,
        albedo,
        normal: 0xB1,
        material: 0xD1,
        more,
    }
}

fn kit(id: u32, template_name: u32, states: Vec<AssetState>) -> AssetKit {
    AssetKit {
        id,
        template_name,
        category: 0,
        states,
    }
}

#[test]
fn an_asset_list_reads_each_texture_entry_by_its_key_with_its_names() {
    let read = read_tree(&list()).expect("an asset list");
    assert_eq!(
        read.albedos[&0xA1],
        TextureEntry {
            word: 0xA1,
            names: Vec::new()
        }
    );
    assert_eq!(read.materials[&0xD1].names, [(MA, 0xE0), (MB, 0xE1)]);
    assert_eq!(read.wrinkles.len(), 2);
    // The names are the spellings the executable asks by.
    assert_eq!(
        (WRINKLE_MASK, WRINKLE_INDEX, WRINKLE, MA, MC),
        (
            0xAEAD_D0AC,
            0xC849_B74A,
            0xC812_6724,
            0x179E_B44A,
            0x7AFA_FB01
        )
    );
    let mut no_map = list();
    no_map.set("materials", Value::UInt(3));
    let why = read_tree(&no_map).unwrap_err().to_string();
    assert_eq!(why, "the materials: is not a map the body states");
}

#[test]
fn a_drawable_joins_through_the_state_that_wears_it_to_each_name_s_texture_key() {
    let lists = [read_tree(&list()).unwrap()];
    let drawable = drawable_word("zq_hat_000.#dr");
    assert_eq!(drawable, name_hash64(b"zq_hat_000"));
    // Two kits wear it: the second in its `Base` state, which is the one taken. Its wrinkle
    // entry is stated by its template's state of that name.
    let other = state(0x51, drawable, 0xA1, vec![(WRINKLE_MASK_ASSET, 0xF2)]);
    let base = state(
        BASE,
        drawable,
        0xA1,
        vec![(WRINKLE_MASK_ASSET, 0xF2), (BLOOD_ASSET, 0x99)],
    );
    let template = state(BASE, 7, 0xA1, vec![(WRINKLE_ASSET, 0xF1)]);
    let catalogue = PedCatalogue {
        asset_lists: vec![vec![
            kit(0xC1, 0, vec![other]),
            kit(0xC2, 0xC3, vec![base]),
            kit(0xC3, 0, vec![template]),
        ]],
        set_lists: Vec::new(),
    };
    let joined = join(drawable, std::slice::from_ref(&catalogue), &lists).expect("a join");
    assert_eq!((joined.kit, joined.state, joined.sets), (0xC2, BASE, 1));
    assert_eq!((joined.albedo, joined.normal), (0xA1, 0xB1));
    assert!(joined.listed && joined.wrinkled);
    assert_eq!(
        joined.named,
        [
            (MC, 0xC0),
            (name_hash32(b"nm"), 0xB1),
            (MA, 0xE0),
            (MB, 0xE1),
            (WRINKLE, 0xF0),
            (WRINKLE_MASK, 0xF2),
            (WRINKLE_INDEX, 0xF3),
            (BLOOD_MASK, 0x99),
        ]
    );
    assert_eq!(joined.key(WRINKLE_INDEX), Some(0xF3));

    // A drawable no state wears; one whose state's albedo no list holds, and names no wrinkle
    // entry, though its wrinkle mask's entry still gives its names.
    assert_eq!(join(9, std::slice::from_ref(&catalogue), &lists), None);
    let mut unlisted = catalogue;
    unlisted.asset_lists[0].truncate(1);
    unlisted.asset_lists[0][0].states[0].albedo = 0xDEAD;
    let joined = join(drawable, &[unlisted], &lists).expect("a join");
    assert_eq!(joined.state, 0x51);
    assert!(!joined.listed && !joined.wrinkled);
    assert_eq!(joined.key(WRINKLE_MASK), Some(0xF2));
    assert_eq!(joined.key(WRINKLE), None);

    // The body of an asset list is told by its class and its albedos' map, either way round.
    let schema = [ASSET_LIST.to_be_bytes(), [0; 4], ALBEDOS.to_be_bytes()].concat();
    assert!(is_asset_list(&schema));
    assert!(!is_asset_list(&ASSET_LIST.to_le_bytes()));
}

#[test]
fn a_template_chain_that_loops_ends() {
    let drawable = 5;
    let looped = PedCatalogue {
        asset_lists: vec![vec![
            kit(0xC1, 0xC2, vec![state(BASE, drawable, 0xA1, Vec::new())]),
            kit(0xC2, 0xC1, vec![state(BASE, 6, 0xA1, Vec::new())]),
        ]],
        set_lists: Vec::new(),
    };
    let joined = join(drawable, &[looped], &[read_tree(&list()).unwrap()]).unwrap();
    assert_eq!(
        joined.key(WRINKLE),
        None,
        "no kit of the loop states a wrinkle entry"
    );
}
