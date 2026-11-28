//! Batch-generated archetypes going into a `.ytyp` and coming back out.
//!
//! The donor is built from the installed schema's own type table rather than
//! staged, so this measures the wiring and says nothing about a retail file;
//! `what_an_archetype_takes_from_its_drawable` is the install-backed half.

mod support;

use glam::Vec3;
use or_io::meta::binary::layout::{ContainerLayout, MetaLayout};
use or_io::meta::binary::synth::rbm_layout_for_instance;
use or_io::meta::binary::{parse_meta_binary_full, write_meta_binary};
use or_io::meta::instance::HashedString;
use or_io::meta::{BinaryFormat, Diagnostics, MetaInstance, Value};
use or_world_gta5::generate::{self, Agreement, AssetKind, Authoring, Closeness, DrawableFacts};
use or_world_gta5::map_types::{Archetype, MapTypes};

const ENUM: &str = "::rage::fwArchetypeDef::eAssetType";

fn facts(i: usize) -> DrawableFacts {
    let f = i as f32;
    DrawableFacts {
        name: format!("generated_prop_{i}"),
        kind: if i.is_multiple_of(2) {
            AssetKind::Drawable
        } else {
            AssetKind::Fragment
        },
        dictionary: None,
        bb_min: Vec3::new(-1.0 - f, -2.0, -0.5),
        bb_max: Vec3::new(1.0 + f, 2.0, 3.25),
        bs_centre: Vec3::new(0.0, 0.0, 1.375),
        bs_radius: 2.5 + f,
    }
}

fn types_holding(archetypes: Vec<MetaInstance>) -> MetaInstance {
    let mut root = MetaInstance::new("CMapTypes");
    root.set("extensions", Value::Array(Vec::new()));
    root.set(
        "archetypes",
        Value::Array(
            archetypes
                .into_iter()
                .map(|a| Value::Pointer(Some(Box::new(a))))
                .collect(),
        ),
    );
    root.set("name", Value::Hash(HashedString::new("donor_types")));
    root.set("dependencies", Value::Array(Vec::new()));
    root.set("compositeEntityTypes", Value::Array(Vec::new()));
    root
}

/// A donor as the reader returns one: bytes, the tree and its own layout.
fn donor() -> Option<(Vec<u8>, MetaInstance, MetaLayout)> {
    let lib = support::schema();
    if lib.struct_("CMapTypes").is_none() || lib.struct_("CBaseArchetypeDef").is_none() {
        or_test_support::skip_corpus(
            "generating_a_type_file",
            "the installed schema library has no CMapTypes; see crates/io/or-io/src/meta/README.md",
        );
        return None;
    }
    let seed = generate::archetype(&facts(100), &Authoring::default(), ENUM);
    let tree = types_holding(vec![seed]);
    let layout = MetaLayout {
        format: BinaryFormat::Rbm,
        container: ContainerLayout::Rbm(Box::new(
            rbm_layout_for_instance(lib, &tree, None).expect("CMapTypes lays out"),
        )),
    };
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &tree).expect("encodes");
    let mut diags = Diagnostics::lenient();
    let (tree, layout) = parse_meta_binary_full(lib, &bytes, &mut diags).expect("re-reads");
    Some((bytes, tree, layout))
}

#[test]
fn generated_archetypes_encode_into_a_donor_and_read_back_exactly() {
    let Some((donor_bytes, donor, layout)) = donor() else {
        return;
    };

    // Untouched, the donor goes back out as the bytes it came in as: the
    // fallback stayed a fallback.
    let unchanged = or_world_gta5::write::encode(&donor, &layout).expect("re-encodes");
    assert_eq!(unchanged, donor_bytes, "an untouched type file changed");

    let spelling = generate::asset_type_enum_of(&donor).expect("the donor spells its enum");
    let wanted: Vec<DrawableFacts> = (0..3).map(facts).collect();
    let archetypes = wanted
        .iter()
        .map(|f| generate::archetype(f, &Authoring::default(), &spelling))
        .collect();
    let name = or_io::string_hash(b"generated_types");
    let tree = generate::type_file(&donor, name, archetypes).expect("the donor has the class");

    let refused = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &tree)
        .expect_err("the donor's allocation holds one archetype, not three");
    assert!(refused.outgrew_its_layout(), "{refused}");

    let bytes = or_world_gta5::write::encode(&tree, &layout).expect("the fallback encodes");
    let mut diags = Diagnostics::lenient();
    let (back, _) =
        parse_meta_binary_full(support::schema(), &bytes, &mut diags).expect("reads back");
    let types = MapTypes::new(&back).expect("a CMapTypes");
    assert_eq!(types.name(), name);
    let read: Vec<Archetype<'_>> = types.archetypes().collect();
    assert_eq!(read.len(), wanted.len());
    for (arch, facts) in read.iter().zip(&wanted) {
        let a = Agreement::of(arch, facts).expect("bounds read");
        assert_eq!(
            (a.bb_min, a.bb_max, a.bs_centre, a.bs_radius),
            (
                Closeness::Exact,
                Closeness::Exact,
                Closeness::Exact,
                Closeness::Exact
            ),
            "{}",
            facts.name
        );
        assert!(a.name_is_asset);
        assert_eq!(AssetKind::of(arch), Some(facts.kind), "{}", facts.name);
        assert_eq!(arch.name(), or_io::string_hash(facts.name.as_bytes()));
    }
}

/// ⚠ A donor with no archetype class is refused rather than written with a
/// zero signature.
#[test]
fn a_donor_without_the_archetype_class_is_refused() {
    let empty = types_holding(Vec::new());
    let err = generate::type_file(&empty, 1, vec![MetaInstance::new("CBaseArchetypeDef")])
        .expect_err("nothing to take the signature from");
    assert!(matches!(err, generate::GenerateError::NoArchetypeClass));
    assert!(matches!(
        generate::type_file(&MetaInstance::new("CMapData"), 1, Vec::new()),
        Err(generate::GenerateError::NotATypeFile)
    ));
}

#[test]
fn a_generated_type_file_carries_none_of_its_donors_content() {
    let seed = generate::archetype(&facts(7), &Authoring::default(), ENUM);
    let mut donor = types_holding(vec![seed]);
    donor.set(
        "dependencies",
        Value::Array(vec![Value::Hash(HashedString::new("somewhere_else"))]),
    );
    let fresh = generate::archetype(&facts(1), &Authoring::default(), ENUM);
    let out = generate::type_file(&donor, 9, vec![fresh]).unwrap();
    let types = MapTypes::new(&out).unwrap();
    assert_eq!(types.dependency_count(), 0);
    assert_eq!(types.archetype_count(), 1);
    assert_eq!(
        types.archetypes().next().unwrap().name(),
        or_io::string_hash(b"generated_prop_1")
    );
}
