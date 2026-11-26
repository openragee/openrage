use super::skeleton::{skeleton_into, BONES};
use super::*;
use crate::gta6::hash::name_hash32;

const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Child 0's own transform: a quarter turn about x, and a step off its bone.
const TURN_X: [f32; 4] = [HALF, 0.0, 0.0, HALF];
const STEP: [f32; 3] = [0.1, 0.2, 0.3];
/// The bone the group is on, `SKEL_Head`: a quarter turn about z at (0.5, 0.25, 1) at bind.
const BONE: u16 = 2;
/// Child 0 placed: the head's turn about z after its own about x, as a composite's rows hold it,
/// the rotation's columns first.
const PLACED: [[f32; 4]; 4] = [
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [1.0, 0.0, 0.0, 0.0],
    [0.3, 0.35, 1.3, 0.0],
];
const MASSES: [f32; 2] = [5.0, 0.5];

/// Where a built fragment's records lie.
struct Parts {
    root: usize,
    drawable: usize,
    groups: usize,
    children: usize,
    matrices: usize,
}

/// A sphere bound of radius 0.25 about `centre`.
fn sphere_into(img: &mut Image, centre: [f32; 3]) -> usize {
    let at = img.alloc(0x70);
    img.bytes[at + 0x10] = 0;
    img.put(at + 0x14, &0.25f32.to_le_bytes());
    img.vec3(at + 0x20, centre.map(|c| c + 0.25));
    img.put(at + 0x2C, &0.25f32.to_le_bytes());
    img.vec3(at + 0x30, centre.map(|c| c - 0.25));
    img.vec3(at + 0x40, centre);
    at
}

/// A fragment of one group on [`BONE`] with a child the composite lists and one of the second set,
/// a matrix set, a bone-bound list and a vehicle glass block.
fn fragment() -> (Image, Vec<u8>, Parts) {
    let (mut img, mut phys) = (Image::default(), Vec::new());
    let root = img.alloc(0x100);
    let drawable = drawable_into(&mut img, &mut phys, [1.0, 1.0, 0.0]).root;
    img.u64(drawable + 0xC8, 0x4000_0000_3F80_0000);
    let (skeleton, _) = skeleton_into(&mut img, &BONES);
    img.ptr(drawable + 0x18, skeleton);
    let name = img.alloc(16);
    img.put(name, b"prop_test\0");
    img.ptr(root + 0xA8, name);

    let composite = bound_into(&mut img).root;
    let pointer_of = |img: &Image, at: usize| u64_at(&img.bytes, at) as usize & 0x0FFF_FFFF;
    let matrices = pointer_of(&img, composite + 0x78);
    for (r, row) in PLACED.iter().enumerate() {
        for (c, x) in row.iter().enumerate() {
            img.put(matrices + 16 * r + 4 * c, &x.to_le_bytes());
        }
    }
    let archetype = img.alloc(0xD0);
    img.u32(archetype + 0x10, 2);
    img.ptr(archetype + 0x18, name);
    img.ptr(archetype + 0x20, composite);
    img.u64(archetype + 0x28, 0x3F);
    for (o, w) in [
        (0x30, u32::MAX),
        (0x34, u32::MAX),
        (0x38, 0x1_0000),
        (0x3C, 0xFFFF),
    ] {
        img.u32(archetype + o, w);
    }
    let mass: f32 = MASSES[0];
    for (k, x) in [2.0f32, 4.0, 8.0, mass].iter().enumerate() {
        img.put(archetype + 0x40 + 4 * k, &x.to_le_bytes());
        img.put(archetype + 0x50 + 4 * k, &(1.0 / x).to_le_bytes());
    }

    let groups = img.alloc(0x70);
    img.u32(
        groups + 0x08,
        name_hash32(BONES[usize::from(BONE)].name.as_bytes()),
    );
    img.u16(groups + 0x10, BONE);
    for (o, x) in [
        (0x14, 0xFFFF),
        (0x16, 0xFFFF),
        (0x18, 0),
        (0x1A, 0),
        (0x1C, 1),
        (0x1E, 2),
        (0x20, 2),
    ] {
        img.u16(groups + o, x);
    }
    img.u32(groups + 0x24, u32::MAX);

    let children = img.alloc(0x50 * 2);
    let own = sphere_into(&mut img, [0.0, 0.0, 0.5]);
    for (i, (q, t)) in [(TURN_X, STEP), ([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.5])]
        .iter()
        .enumerate()
    {
        let c = children + 0x50 * i;
        for (k, x) in q.iter().enumerate() {
            img.put(c + 4 * k, &x.to_le_bytes());
        }
        img.vec3(c + 0x10, *t);
        img.put(c + 0x2C, &MASSES[i].to_le_bytes());
    }
    let listed = pointer_of(&img, composite + 0x70);
    let first = u64_at(&img.bytes, listed);
    img.u64(children + 0x30, first);
    img.u64(children + 0x38, 0x13);
    img.u16(children + 0x4A, 0);
    img.ptr(children + 0x50 + 0x30, own);
    img.u16(children + 0x50 + 0x4A, 0xFFFF);

    let set = img.alloc(0x20 + 0x30 * BONES.len());
    img.u16(set + 0x10, BONES.len() as u16);
    img.u16(set + 0x12, BONES.len() as u16);
    let entries = img.alloc(0x40);
    img.put(entries + 12, &1.0f32.to_le_bytes());
    let bound = sphere_into(&mut img, [0.0; 3]);
    img.ptr(entries + 0x20, bound);
    img.u32(entries + 0x2C, 0x10);
    img.u32(entries + 0x30, 0x03E0_0000);
    img.u32(entries + 0x38, name_hash32(BONES[0].name.as_bytes()));
    let list = img.alloc(0x18);
    img.ptr(list, entries);
    img.u32(list + 0x0C, 0x10);
    img.u32(list + 0x10, 1);
    let glass = img.alloc(0x10);
    img.put(glass, b"HWGV\x70\x00");

    img.ptr(root + 0x10, archetype);
    img.ptr(root + 0x18, drawable);
    img.ptr(root + 0x20, groups);
    img.ptr(root + 0x28, children);
    for (k, x) in [2u16, 1, 1, 2].iter().enumerate() {
        img.u16(root + 0x30 + 2 * k, *x);
    }
    img.ptr(root + 0x38, list);
    img.ptr(root + 0x90, set);
    img.ptr(root + 0xF0, glass);
    let parts = Parts {
        root,
        drawable,
        groups,
        children,
        matrices,
    };
    (img, phys, parts)
}

fn fragment_of(img: &Image, phys: &[u8]) -> Result<DrawableWithBound> {
    read_drawable_with_bound(&img.bytes, Some(phys))
}

#[test]
fn a_fragment_reads_its_archetype_group_and_children_each_placed_at_its_bone() {
    let (img, phys, _) = fragment();
    let read = fragment_of(&img, &phys).expect("the fragment reads");
    assert_eq!(read.name, "prop_test");
    assert!(read.drawable.bound.is_none());
    let f = &read.fragment;
    assert_eq!(f.sets, 2);
    assert_eq!(f.matrices, Some(BONES.len() as u16));
    assert_eq!(
        f.archetype.map(|a| (a.mass, a.angular_inertia)),
        Some((5.0, [2.0, 4.0, 8.0]))
    );
    assert_eq!(
        f.groups,
        [FragmentGroup {
            bone: BONE,
            bone_hash: name_hash32(b"SKEL_Head"),
            parent: None,
            child_groups: 0..0,
            children: 0..1,
            second_set: 1..2,
        }]
    );
    let [listed, own] = f.children.as_slice() else {
        panic!("two children, not {:?}", f.children);
    };
    assert_eq!(
        (listed.composite, listed.flags, listed.mass),
        (Some(0), [0x13, 0], 5.0)
    );
    assert!(listed.bound.is_none(), "the composite holds its bound");
    for (row, want) in listed.matrix.iter().zip(PLACED) {
        for (x, w) in row.iter().zip(want) {
            assert!(
                (x - w).abs() < 1e-5,
                "{:?} is not {PLACED:?}",
                listed.matrix
            );
        }
    }
    assert_eq!(own.composite, None);
    assert!(matches!(
        own.bound,
        Some(Ok(ChildBound::Sphere { radius: 0.25, .. }))
    ));
    // A turn about z leaves the step along z where it is: the head's place plus 0.5 m up.
    let at = own.matrix[3];
    assert!(
        (at[0] - 0.5).abs() < 1e-5 && (at[1] - 0.25).abs() < 1e-5 && (at[2] - 1.5).abs() < 1e-5,
        "{at:?}"
    );
    assert_eq!(f.bone_bounds.len(), 1);
    assert_eq!(f.bone_bounds[0].bone_hash, name_hash32(b"SKEL_ROOT"));
    assert_eq!(f.records.iter().map(|r| r.0).collect::<Vec<_>>(), [0xF0]);
}

#[test]
fn a_composite_matrix_that_is_not_its_child_at_its_bone_is_refused() {
    let refused = |img: &Image, phys: &[u8]| {
        matches!(fragment_of(img, phys), Err(HeaderlessError::Fragment(_)))
    };
    // Moved by a quarter metre.
    let (mut img, phys, parts) = fragment();
    img.put(parts.matrices + 0x30, &0.55f32.to_le_bytes());
    assert!(refused(&img, &phys));
    // The rotation's rows where its columns are.
    let (mut img, phys, parts) = fragment();
    for (r, row) in [[0.0f32, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .iter()
        .enumerate()
    {
        img.vec3(parts.matrices + 16 * r, *row);
    }
    assert!(refused(&img, &phys));
    // A group on another bone than the hash it states.
    let (mut img, phys, parts) = fragment();
    img.u16(parts.groups + 0x10, 1);
    assert!(refused(&img, &phys));
    // A listed child heavier than the archetype.
    let (mut img, phys, parts) = fragment();
    img.put(parts.children + 0x2C, &6.0f32.to_le_bytes());
    assert!(refused(&img, &phys));
    // A second-set child that names a place among the composite's children.
    let (mut img, phys, parts) = fragment();
    img.u16(parts.children + 0x50 + 0x4A, 1);
    assert!(refused(&img, &phys));
    // A vehicle glass block of another stride.
    let (mut img, phys, parts) = fragment();
    let glass = u64_at(&img.bytes, parts.root + 0xF0) as usize & 0x0FFF_FFFF;
    img.u16(glass + 4, 0x60);
    assert!(refused(&img, &phys));
}

#[test]
fn a_fragment_with_no_archetype_has_no_group_and_one_with_groups_alone_is_refused() {
    let (mut img, phys, parts) = fragment();
    for slot in [0x10, 0x20, 0x28, 0x38] {
        img.u64(parts.root + slot, 0);
    }
    for (k, x) in [1u16, 0, 0, 0].iter().enumerate() {
        img.u16(parts.root + 0x30 + 2 * k, *x);
    }
    let read = fragment_of(&img, &phys).expect("it reads");
    assert!(read.bound.is_none() && read.fragment.archetype.is_none());
    assert!(read.fragment.groups.is_empty() && read.fragment.children.is_empty());
    img.ptr(parts.root + 0x20, parts.groups);
    assert!(matches!(
        fragment_of(&img, &phys),
        Err(HeaderlessError::Fragment(_))
    ));
}

#[test]
fn a_fragment_s_drawable_of_no_level_reads_under_the_empty_box() {
    let (mut img, phys, parts) = fragment();
    img.u64(parts.drawable + 0x50, 0);
    let refused = fragment_of(&img, &phys).expect_err("a box that is not the empty one");
    assert!(
        matches!(refused, HeaderlessError::NotADrawable { .. }),
        "{refused}"
    );
    for k in 0..4 {
        img.u32(parts.drawable + 0x30 + 4 * k, 0x7F7F_FFFF);
        img.u32(parts.drawable + 0x40 + 4 * k, 0xFF7F_FFFF);
    }
    let read = fragment_of(&img, &phys).expect("the empty drawable reads");
    assert!(read.drawable.levels.iter().all(Option::is_none));
    assert_eq!(read.fragment.children.len(), 2);
}

#[test]
fn a_fragment_s_masses_and_inertia_write_in_place_and_a_moved_child_is_refused() {
    let (img, phys, _) = fragment();
    let mut bytes = img.bytes.clone();
    let unchanged = fragment_of(&img, &phys)
        .expect("the fragment reads")
        .fragment;
    write_fragment(&mut bytes, &phys, &unchanged).expect("nothing edited writes");
    assert_eq!(bytes, img.bytes);

    let mut edited = fragment_of(&img, &phys)
        .expect("the fragment reads")
        .fragment;
    let mass = 7.25f32;
    if let Some(a) = edited.archetype.as_mut() {
        a.mass = mass;
        a.angular_inertia[1] = 3.5;
    }
    edited.children[0].mass = mass;
    edited.children[1].mass = 0.75;
    edited.children[1].angular_inertia = [0.125, 0.25, 0.375];
    write_fragment(&mut bytes, &phys, &edited).expect("the masses write in place");
    assert_eq!(bytes.len(), img.bytes.len());
    let read = read_drawable_with_bound(&bytes, Some(phys.as_slice()))
        .expect("it reads back")
        .fragment;
    assert_eq!(
        read.archetype.map(|a| (a.mass, a.angular_inertia)),
        Some((mass, [2.0, 3.5, 8.0]))
    );
    let masses: Vec<f32> = read.children.iter().map(|c| c.mass).collect();
    assert_eq!(masses, [mass, 0.75]);
    assert_eq!(read.children[1].angular_inertia, [0.125, 0.25, 0.375]);

    // The composite's children no longer summing to the archetype's mass is refused as the reader refuses it.
    edited.children[0].mass = 1.0;
    assert!(write_fragment(&mut bytes.clone(), &phys, &edited).is_err());
    edited.children[0].mass = mass;
    edited.children[1].translation[0] += 1.0;
    assert!(matches!(
        write_fragment(&mut bytes, &phys, &edited),
        Err(HeaderlessError::NotInPlace(_))
    ));
}

#[path = "write_fragment_child.rs"]
mod write_fragment_child;

#[test]
fn a_fragment_states_the_trace_words_of_the_drawable_its_root_points_at() {
    let (mut img, _, parts) = fragment();
    img.u32(parts.drawable + 0xB8, 0x0000_0600);
    let words = trace_words(Rpf9Role::DrawableWithBound, &img.bytes);
    let [Ok(words)] = words.as_slice() else {
        panic!("one drawable's words, not {words:?}");
    };
    assert_eq!(words.level_word, 0x0000_0600);
    let models = words.levels[0].as_ref().expect("level 0");
    assert_eq!(
        (models.len(), models[0].geometries[0].triangle_count),
        (1, 1)
    );
}
