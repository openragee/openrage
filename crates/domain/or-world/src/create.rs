//! A map, an entity, a type file and a base archetype made from nothing: no donor file and no schema snapshot.
//!
//! The records are the typed world family's (`or_io::meta::typed`), built with the members that family types and saved by its
//! own save, which lays a root built here out from the classes `or-io` declares (`or_io::meta::typed::world_types`). What a
//! person does not choose is derived by the code that derives it for a file that arrived: a guid by [`free_guid`], the content
//! flags by [`crate::write::apply_flags`], the extents by [`crate::write::apply_extents`]; an entity has no parent and no child.
//!
//! ⚠ **The file is a PSO, and RBM is refused.** An RBM class carries a `signature` word that nothing in this tree derives
//! ([`CreateError::NoSignature`]), and the game's own world files are RBM, so whether the game loads a world file written as PSO
//! is not known here: nothing in the tree runs the game.
//!
//! ⚠ **Only what the family types is stated.** An entity states no `lodLevel`, `priorityLevel` or extension and an archetype no
//! `assetType`, so each is read at its class's zero. An interior, a timed archetype and an interior instance are not made here.

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
};
use or_io::meta::{BinaryFormat, Diagnostics};

use crate::archetypes::ArchetypeIndex;
use crate::generate::{Authoring, DrawableFacts};
use crate::transform::Placement;
use crate::write::{self, WriteError};

/// Why a world file was not made from nothing.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("the {0} of an entity is not a finite number, and a file stores none that is not")]
    NotANumber(&'static str),
    #[error("a level-of-detail distance of {0} is below zero")]
    NegativeLodDist(f32),
    #[error(
        "a world file made from nothing is not written as RBM: an RBM class carries a signature \
         word that nothing derives, and there is no file to take it from; write it as PSO, or copy \
         a file that holds the class"
    )]
    NoSignature,
    #[error("a world file is written as PSO or RBM, and {0} is neither")]
    NotAWorldContainer(&'static str),
    #[error("the file arrived as {0} and saves as it arrived, so it is not written as {1}")]
    SavesAs(&'static str, &'static str),
    #[error(transparent)]
    Save(#[from] or_io::Error),
    #[error(transparent)]
    Derive(#[from] WriteError),
    #[error("the file made does not read back: {0}")]
    ReadBack(or_io::meta::MetaError),
}

/// What a person chooses of a new entity: what it places, where it stands and how far it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The archetype, by its name or by the hash of one.
    pub archetype: HashedString,
    pub position: Vec3,
    /// The rotation that places it, which the file stores conjugated ([`Placement::Entity`]).
    pub rotation: Quat,
    /// The scale of X and Y together.
    pub scale_xy: f32,
    pub scale_z: f32,
    /// The distance it is drawn to, in world units; zero leaves it to the archetype.
    pub lod_dist: f32,
    pub flags: u32,
}

impl Placed {
    /// `archetype` at `position`, unturned and unscaled, drawn to `lod_dist`, with no flag set.
    pub fn at(archetype: HashedString, position: Vec3, lod_dist: f32) -> Self {
        Self {
            archetype,
            position,
            rotation: Quat::IDENTITY,
            scale_xy: 1.0,
            scale_z: 1.0,
            lod_dist,
            flags: 0,
        }
    }

    /// Refuses a number a file cannot store: one that is not finite, or a distance below zero.
    pub fn check(&self) -> Result<(), CreateError> {
        for (what, finite) in [
            ("position", self.position.is_finite()),
            ("rotation", self.rotation.is_finite()),
            (
                "scale",
                self.scale_xy.is_finite() && self.scale_z.is_finite(),
            ),
            ("level-of-detail distance", self.lod_dist.is_finite()),
        ] {
            if !finite {
                return Err(CreateError::NotANumber(what));
            }
        }
        if self.lod_dist < 0.0 {
            return Err(CreateError::NegativeLodDist(self.lod_dist));
        }
        Ok(())
    }
}

/// A guid no entity of `map` states: one more than the largest, or the smallest above zero that is free when the largest is the last number.
pub fn free_guid(map: &CMapData) -> u32 {
    match map.entities.iter().map(|e| e.guid).max() {
        None => 1,
        Some(largest) if largest < u32::MAX => largest + 1,
        Some(_) => {
            let stated: std::collections::BTreeSet<u32> =
                map.entities.iter().map(|e| e.guid).collect();
            (1..=u32::MAX).find(|g| !stated.contains(g)).unwrap_or(0)
        }
    }
}

/// The record of an entity that is no copy: `placed`, with `guid`, no parent and no child.
pub fn entity(placed: &Placed, guid: u32) -> Result<CEntityDefAny, CreateError> {
    placed.check()?;
    let stored = Placement::Entity.store(placed.rotation);
    let mut entity = CEntityDef::default();
    entity.archetype_name = placed.archetype.clone();
    entity.flags = placed.flags;
    entity.guid = guid;
    entity.position.x = placed.position.x;
    entity.position.y = placed.position.y;
    entity.position.z = placed.position.z;
    entity.rotation.x = stored[0];
    entity.rotation.y = stored[1];
    entity.rotation.z = stored[2];
    entity.rotation.w = stored[3];
    entity.scale_xy = placed.scale_xy;
    entity.scale_z = placed.scale_z;
    entity.parent_index = -1;
    entity.lod_dist = placed.lod_dist;
    Ok(CEntityDefAny::Base(entity))
}

/// Add an entity that is no copy to `map`, with a guid of its own; its index.
pub fn place(map: &mut CMapData, placed: &Placed) -> Result<usize, CreateError> {
    let built = entity(placed, free_guid(map))?;
    map.entities.push(built);
    Ok(map.entities.len() - 1)
}

/// A name as a file stores it, or no name for empty text.
///
/// ⚠ `0xHHHHHHHH` and `hash_HHHHHHHH` are taken as the number and not hashed as text: most names of an install survive only as
/// their hash.
fn named(text: &str) -> HashedString {
    match text.is_empty() {
        true => HashedString::from_hash(0),
        false => HashedString::from_xml_text(text),
    }
}

/// An empty map called `name`.
pub fn map(name: &str) -> CMapData {
    let mut map = CMapData::default();
    map.name = named(name);
    map
}

/// An empty type file called `name`.
pub fn types(name: &str) -> CMapTypes {
    let mut types = CMapTypes::default();
    types.name = named(name);
    types
}

/// A base archetype for `facts`, with what `authoring` states of it: the typed members [`crate::generate::archetype`] sets.
pub fn archetype(facts: &DrawableFacts, authoring: &Authoring) -> CBaseArchetypeDefAny {
    let vec3 = |v: Vec3| or_io::decl::Vec3 {
        x: v.x,
        y: v.y,
        z: v.z,
    };
    let optional = |text: &Option<String>| named(text.as_deref().unwrap_or(""));
    let mut archetype = CBaseArchetypeDef::default();
    archetype.lod_dist = authoring.lod_dist;
    archetype.flags = authoring.flags;
    archetype.special_attribute = authoring.special_attribute;
    archetype.bb_min = vec3(facts.bb_min);
    archetype.bb_max = vec3(facts.bb_max);
    archetype.bs_centre = vec3(facts.bs_centre);
    archetype.bs_radius = facts.bs_radius;
    archetype.hd_texture_dist = authoring.hd_texture_dist;
    archetype.name = named(&facts.name);
    archetype.texture_dictionary = named(
        authoring
            .texture_dictionary
            .as_deref()
            .unwrap_or(&facts.name),
    );
    archetype.clip_dictionary = optional(&authoring.clip_dictionary);
    archetype.drawable_dictionary = optional(&facts.dictionary);
    archetype.physics_dictionary = optional(&authoring.physics_dictionary);
    archetype.asset_name = named(&facts.name);
    CBaseArchetypeDefAny::Base(archetype)
}

/// Refuses a container a world file made from nothing is not written in.
pub fn writable(format: BinaryFormat) -> Result<(), CreateError> {
    match format {
        BinaryFormat::Pso => Ok(()),
        BinaryFormat::Rbm => Err(CreateError::NoSignature),
        BinaryFormat::Rbf => Err(CreateError::NotAWorldContainer(format.label())),
    }
}

/// `saved`, refused when it is not a `format`: a root that arrived saves in the container it arrived in, whatever is asked for.
fn held_to(saved: Vec<u8>, format: BinaryFormat) -> Result<Vec<u8>, CreateError> {
    match or_io::meta::detect_binary_format(&saved) {
        Some(written) if written == format => Ok(saved),
        Some(written) => Err(CreateError::SavesAs(written.label(), format.label())),
        None => Err(CreateError::SavesAs("text", format.label())),
    }
}

/// The file `map` saves as, with the content flags and the extents that what it places implies; `index` gives each archetype's box.
pub fn map_file(
    map: &CMapData,
    index: &ArchetypeIndex,
    format: BinaryFormat,
) -> Result<Vec<u8>, CreateError> {
    writable(format)?;
    let saved = held_to(map.to_bytes()?, format)?;
    let mut diagnostics = Diagnostics::lenient();
    let (mut root, layout) =
        parse_meta_binary_full(or_io::meta::default_library(), &saved, &mut diagnostics)
            .map_err(CreateError::ReadBack)?;
    write::apply_flags(&mut root)?;
    // ⚠ Refused when `index` lacks an entity's archetype: extents that leave it out are too small, and the game does not stream a
    // map by extents its entities stand outside.
    write::apply_extents(&mut root, index)?;
    Ok(write::encode(&root, &layout)?)
}

/// The file `types` saves as.
pub fn types_file(types: &CMapTypes, format: BinaryFormat) -> Result<Vec<u8>, CreateError> {
    writable(format)?;
    held_to(types.to_bytes()?, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_flags::content;
    use crate::generate::AssetKind;
    use crate::map_data::MapData;
    use crate::map_types::MapTypes;
    use crate::validate;
    use or_io::meta::binary::hash_index::with_public_names_only;
    use or_io::meta::{canonical, detect_binary_format, MetaInstance, SchemaLibrary};

    const BENCH: &str = "prop_bench_01a";

    fn tree(bytes: &[u8]) -> MetaInstance {
        let mut diagnostics = Diagnostics::lenient();
        parse_meta_binary_full(or_io::meta::default_library(), bytes, &mut diagnostics)
            .expect("the file reads")
            .0
    }

    /// A type file made from nothing that declares the bench with the box `bb_min` to `bb_max`.
    fn types_declaring(bb_min: Vec3, bb_max: Vec3) -> Vec<u8> {
        let mut types = types("hills_types");
        types.archetypes.push(archetype(
            &DrawableFacts {
                name: BENCH.into(),
                kind: AssetKind::Drawable,
                dictionary: None,
                bb_min,
                bb_max,
                bs_centre: Vec3::new(0.0, 0.0, 1.5),
                bs_radius: 2.5,
            },
            &Authoring::default(),
        ));
        types_file(&types, BinaryFormat::Pso).expect("a type file made from nothing saves")
    }

    /// A type file made from nothing that declares the bench, two units wide, four deep and three high.
    fn bench_types() -> Vec<u8> {
        types_declaring(Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 2.0, 3.0))
    }

    fn index_of(types: &[u8]) -> ArchetypeIndex {
        let tree = tree(types);
        let mut index = ArchetypeIndex::new();
        index.add(MapTypes::new(&tree).expect("a CMapTypes"));
        index
    }

    fn bench_at(position: Vec3) -> Placed {
        Placed {
            flags: 32,
            ..Placed::at(HashedString::new(BENCH), position, 60.0)
        }
    }

    fn vec3(v: or_io::decl::Vec3) -> Vec3 {
        Vec3::new(v.x, v.y, v.z)
    }

    #[test]
    fn a_map_made_from_nothing_reads_back_with_its_name_and_its_entity_and_breaks_no_check() {
        let at = Vec3::new(2000.0, -3000.0, 50.0);
        let mut made = map("hills");
        assert_eq!(place(&mut made, &bench_at(at)).expect("placed"), 0);
        let saved = map_file(&made, &index_of(&bench_types()), BinaryFormat::Pso)
            .expect("a map made from nothing saves");
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));

        let back = CMapData::from_bytes(&saved).expect("it opens as its typed root");
        assert!(back.name == "hills");
        assert_eq!(back.parent.hash, 0);
        assert_eq!(back.entities.len(), 1);
        let entity = &back.entities[0];
        assert!(entity.archetype_name == BENCH);
        assert_eq!(vec3(entity.position), at);
        assert_eq!(
            Placement::Entity.rotation_of([
                entity.rotation.x,
                entity.rotation.y,
                entity.rotation.z,
                entity.rotation.w
            ]),
            Quat::IDENTITY
        );
        assert_eq!(
            (entity.scale_xy, entity.scale_z, entity.lod_dist),
            (1.0, 1.0, 60.0)
        );
        assert_eq!((entity.flags, entity.guid), (32, 1));
        assert_eq!(
            (entity.parent_index, entity.num_children),
            (-1, 0),
            "an entity made from nothing has no parent and no child"
        );

        let root = tree(&saved);
        let view = MapData::new(&root).expect("a CMapData");
        assert_eq!(validate::check(&view), Vec::new());
        write::refuse_if_broken(&root).expect("nothing the game would misread");
        assert_eq!(back.content_flags, content::HD);
        assert_eq!(back.flags, 0);
    }

    #[test]
    fn the_extents_of_a_map_made_from_nothing_hold_its_entity_and_nothing_else() {
        let at = Vec3::new(2000.0, -3000.0, 50.0);
        let turned = Placed {
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ..bench_at(at)
        };
        let mut made = map("hills");
        place(&mut made, &turned).expect("placed");
        let saved =
            map_file(&made, &index_of(&bench_types()), BinaryFormat::Pso).expect("it saves");
        let back = CMapData::from_bytes(&saved).expect("opens");

        // The box is two wide and four deep, and a quarter turn swaps the two.
        let (min, max) = (
            vec3(back.entities_extents_min),
            vec3(back.entities_extents_max),
        );
        assert!(
            min.abs_diff_eq(at + Vec3::new(-2.0, -1.0, 0.0), 1e-3),
            "{min}"
        );
        assert!(
            max.abs_diff_eq(at + Vec3::new(2.0, 1.0, 3.0), 1e-3),
            "{max}"
        );
        let (wide_min, wide_max) = (
            vec3(back.streaming_extents_min),
            vec3(back.streaming_extents_max),
        );
        let reach = Vec3::splat(turned.lod_dist);
        assert!(wide_min.abs_diff_eq(min - reach, 1e-3), "{wide_min}");
        assert!(wide_max.abs_diff_eq(max + reach, 1e-3), "{wide_max}");
    }

    #[test]
    fn a_turned_entity_stands_where_the_turn_puts_it_and_not_where_its_inverse_would() {
        // ⚠ A box that reaches four along +X only: a quarter turn about Z lays it along +Y, and the inverse turn along -Y, which
        // a box the same on both sides of its origin cannot tell apart.
        let types = types_declaring(Vec3::ZERO, Vec3::new(4.0, 1.0, 3.0));
        let at = Vec3::new(2000.0, -3000.0, 50.0);
        let turn = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut made = map("hills");
        place(
            &mut made,
            &Placed {
                rotation: turn,
                ..bench_at(at)
            },
        )
        .expect("placed");
        let saved = map_file(&made, &index_of(&types), BinaryFormat::Pso).expect("it saves");
        let back = CMapData::from_bytes(&saved).expect("opens");

        let stored = back.entities[0].rotation;
        let placing = Placement::Entity.rotation_of([stored.x, stored.y, stored.z, stored.w]);
        assert!(placing.abs_diff_eq(turn, 1e-6), "{placing}");
        let (min, max) = (
            vec3(back.entities_extents_min),
            vec3(back.entities_extents_max),
        );
        assert!(
            min.abs_diff_eq(at + Vec3::new(-1.0, 0.0, 0.0), 1e-3),
            "{min}"
        );
        assert!(
            max.abs_diff_eq(at + Vec3::new(0.0, 4.0, 3.0), 1e-3),
            "{max}"
        );
    }

    #[test]
    fn a_name_spelt_as_its_hash_is_that_hash_and_not_the_hash_of_its_spelling() {
        let hills = or_io::string_hash(b"hills");
        let bench = or_io::string_hash(BENCH.as_bytes());
        for spelt in [format!("hash_{hills:08X}"), format!("0x{hills:08X}")] {
            let saved =
                map_file(&map(&spelt), &ArchetypeIndex::new(), BinaryFormat::Pso).expect("saves");
            let back = CMapData::from_bytes(&saved).expect("opens");
            assert_eq!(back.name.hash, hills, "{spelt}");
            assert_eq!(types(&spelt).name.hash, hills, "{spelt}");
        }
        let declared = archetype(
            &DrawableFacts {
                name: format!("hash_{bench:08X}"),
                kind: AssetKind::Drawable,
                dictionary: None,
                bb_min: Vec3::ZERO,
                bb_max: Vec3::ONE,
                bs_centre: Vec3::splat(0.5),
                bs_radius: 1.0,
            },
            &Authoring::default(),
        );
        assert_eq!(
            (
                declared.name.hash,
                declared.asset_name.hash,
                declared.texture_dictionary.hash
            ),
            (bench, bench, bench)
        );
    }

    #[test]
    fn an_archetype_states_what_its_author_and_its_drawable_state() {
        let mut made = types("hills_types");
        made.archetypes.push(archetype(
            &DrawableFacts {
                name: BENCH.into(),
                kind: AssetKind::Drawable,
                dictionary: Some("hills_props".into()),
                bb_min: Vec3::new(-1.0, -2.0, 0.0),
                bb_max: Vec3::new(1.0, 2.0, 3.0),
                bs_centre: Vec3::new(0.0, 0.0, 1.5),
                bs_radius: 2.5,
            },
            &Authoring {
                lod_dist: 80.0,
                hd_texture_dist: 20.0,
                flags: 0x2000_0000,
                special_attribute: 3,
                texture_dictionary: Some("hills_txd".into()),
                physics_dictionary: Some("hills_phys".into()),
                clip_dictionary: Some("hills_clips".into()),
            },
        ));
        let saved = types_file(&made, BinaryFormat::Pso).expect("saves");
        let back = CMapTypes::from_bytes(&saved).expect("opens");
        let declared = &back.archetypes[0];
        assert_eq!((declared.lod_dist, declared.hd_texture_dist), (80.0, 20.0));
        assert_eq!(
            (declared.flags, declared.special_attribute),
            (0x2000_0000, 3)
        );
        assert!(declared.texture_dictionary == "hills_txd");
        assert!(declared.physics_dictionary == "hills_phys");
        assert!(declared.clip_dictionary == "hills_clips");
        assert!(declared.drawable_dictionary == "hills_props");
        assert!(declared.name == BENCH && declared.asset_name == BENCH);
    }

    /// `pso`, a file made from nothing, written again as RBM with the classes its own table states.
    fn as_rbm(pso: &[u8]) -> Vec<u8> {
        let document = or_io::meta::MetaDocument::parse(&SchemaLibrary::default(), pso.to_vec())
            .expect("reads");
        let library = or_io::meta::typed::world_types::declared();
        let layout = canonical::layout_for(library, &document.instance, BinaryFormat::Rbm, None)
            .expect("lays out");
        or_io::meta::write_meta_binary(BinaryFormat::Rbm, Some(&layout), &document.instance)
            .expect("encodes")
    }

    #[test]
    fn a_file_that_arrived_in_another_container_is_not_handed_back_as_the_one_asked_for() {
        let pso =
            map_file(&map("hills"), &ArchetypeIndex::new(), BinaryFormat::Pso).expect("saves");
        let arrived = CMapData::from_bytes(&as_rbm(&pso)).expect("opens");
        let refused = map_file(&arrived, &ArchetypeIndex::new(), BinaryFormat::Pso)
            .expect_err("it saves as RBM, as it arrived");
        assert!(
            matches!(refused, CreateError::SavesAs("RBM", "PSO")),
            "{refused}"
        );
        let pso = types_file(&types("hills_types"), BinaryFormat::Pso).expect("saves");
        let arrived = CMapTypes::from_bytes(&as_rbm(&pso)).expect("opens");
        assert!(matches!(
            types_file(&arrived, BinaryFormat::Pso),
            Err(CreateError::SavesAs("RBM", "PSO"))
        ));
        // One that arrived as what is asked for is written.
        let reopened = CMapData::from_bytes(
            &map_file(&map("hills"), &ArchetypeIndex::new(), BinaryFormat::Pso).expect("saves"),
        )
        .expect("opens");
        map_file(&reopened, &ArchetypeIndex::new(), BinaryFormat::Pso).expect("saves again");
    }

    #[test]
    fn a_map_whose_largest_guid_is_the_last_number_gives_one_no_entity_states() {
        let mut made = map("hills");
        for guid in [1, 2, u32::MAX] {
            place(&mut made, &bench_at(Vec3::ZERO)).expect("placed");
            made.entities.last_mut().expect("just placed").guid = guid;
        }
        assert_eq!(free_guid(&made), 3);
    }

    #[test]
    fn an_empty_map_made_from_nothing_states_no_extent_and_no_content() {
        let saved = map_file(&map("hills"), &ArchetypeIndex::new(), BinaryFormat::Pso)
            .expect("an empty map saves");
        let back = CMapData::from_bytes(&saved).expect("opens");
        assert!(back.name == "hills");
        assert!(back.entities.is_empty());
        assert_eq!((back.flags, back.content_flags), (0, 0));
        for extent in [
            back.entities_extents_min,
            back.entities_extents_max,
            back.streaming_extents_min,
            back.streaming_extents_max,
        ] {
            assert_eq!(vec3(extent), Vec3::ZERO);
        }
        assert_eq!(
            validate::check(&MapData::new(&tree(&saved)).expect("a CMapData")),
            Vec::new()
        );
    }

    #[test]
    fn a_map_and_a_type_file_made_from_nothing_are_the_fixed_point_of_their_own_xml() {
        let mut made = map("hills");
        place(&mut made, &bench_at(Vec3::new(10.0, 20.0, 30.0))).expect("placed");
        let types = bench_types();
        let files = [
            map_file(&made, &index_of(&types), BinaryFormat::Pso).expect("saves"),
            map_file(&map("empty"), &ArchetypeIndex::new(), BinaryFormat::Pso).expect("saves"),
            types,
        ];
        for saved in files {
            let converge = || canonical::converge(&SchemaLibrary::default(), &saved);
            for converged in [converge(), with_public_names_only(converge)] {
                let converged = converged.expect("its XML compiles back");
                assert!(!converged.values_moved);
                assert_eq!(
                    converged.bytes, saved,
                    "the file its XML compiles to is the file itself"
                );
            }
        }
    }

    #[test]
    fn a_type_file_made_from_nothing_reads_back_and_the_archetype_index_finds_its_archetype() {
        let saved = bench_types();
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));
        let back = CMapTypes::from_bytes(&saved).expect("it opens as its typed root");
        assert!(back.name == "hills_types");
        assert_eq!(back.archetypes.len(), 1);
        let declared = &back.archetypes[0];
        assert!(matches!(declared, CBaseArchetypeDefAny::Base(_)));
        assert!(declared.name == BENCH && declared.asset_name == BENCH);
        assert!(
            declared.texture_dictionary == BENCH,
            "no dictionary was named, so it is its own"
        );
        assert_eq!(declared.physics_dictionary.hash, 0);
        assert_eq!(
            (declared.lod_dist, declared.hd_texture_dist),
            (
                Authoring::default().lod_dist,
                Authoring::default().hd_texture_dist
            )
        );

        let index = index_of(&saved);
        let name = or_io::string_hash(BENCH.as_bytes());
        let bounds = index
            .bounds_of(name)
            .expect("the index finds the archetype");
        assert_eq!(bounds.min, Vec3::new(-1.0, -2.0, 0.0));
        assert_eq!(bounds.max, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            (bounds.centre, bounds.radius),
            (Vec3::new(0.0, 0.0, 1.5), 2.5)
        );
        assert_eq!(index.lod_dist_of(name), Some(Authoring::default().lod_dist));
        assert_eq!(
            index.owner_of(name),
            Some(or_io::string_hash(b"hills_types")),
            "by the type file's own name"
        );

        let empty = types_file(&types("nothing_yet"), BinaryFormat::Pso).expect("saves");
        let back = CMapTypes::from_bytes(&empty).expect("an empty type file opens");
        assert!(back.name == "nothing_yet" && back.archetypes.is_empty());
    }

    #[test]
    fn a_number_a_file_cannot_store_is_refused_before_anything_is_placed() {
        let good = bench_at(Vec3::new(1.0, 2.0, 3.0));
        for (what, placed) in [
            (
                "position",
                Placed {
                    position: Vec3::new(1.0, f32::NAN, 3.0),
                    ..good.clone()
                },
            ),
            (
                "position",
                Placed {
                    position: Vec3::new(f32::INFINITY, 2.0, 3.0),
                    ..good.clone()
                },
            ),
            (
                "position",
                Placed {
                    position: Vec3::new(1.0, 2.0, f32::MAX * 2.0),
                    ..good.clone()
                },
            ),
            (
                "rotation",
                Placed {
                    rotation: Quat::from_xyzw(0.0, f32::NAN, 0.0, 1.0),
                    ..good.clone()
                },
            ),
            (
                "scale",
                Placed {
                    scale_z: f32::NEG_INFINITY,
                    ..good.clone()
                },
            ),
            (
                "level-of-detail distance",
                Placed {
                    lod_dist: f32::NAN,
                    ..good.clone()
                },
            ),
        ] {
            let mut made = map("hills");
            let refused = place(&mut made, &placed).expect_err(what);
            assert!(
                matches!(refused, CreateError::NotANumber(named) if named == what),
                "{refused}"
            );
            assert!(made.entities.is_empty(), "nothing was placed");
        }
        let below = Placed {
            lod_dist: -1.0,
            ..good.clone()
        };
        assert!(matches!(
            place(&mut map("hills"), &below),
            Err(CreateError::NegativeLodDist(_))
        ));
        place(&mut map("hills"), &good).expect("a finite placement is placed");
    }

    #[test]
    fn rbm_is_refused_with_the_word_nothing_derives_named() {
        let refused = map_file(&map("hills"), &ArchetypeIndex::new(), BinaryFormat::Rbm)
            .expect_err("no signature to write");
        assert!(matches!(refused, CreateError::NoSignature), "{refused}");
        assert!(refused.to_string().contains("signature"), "{refused}");
        assert!(matches!(
            types_file(&types("hills_types"), BinaryFormat::Rbm),
            Err(CreateError::NoSignature)
        ));
        assert!(matches!(
            types_file(&types("hills_types"), BinaryFormat::Rbf),
            Err(CreateError::NotAWorldContainer("RBF"))
        ));
    }

    #[test]
    fn an_entity_whose_archetype_nothing_declares_is_refused_rather_than_left_outside_the_extents()
    {
        let mut made = map("hills");
        place(&mut made, &bench_at(Vec3::new(1.0, 2.0, 3.0))).expect("placed");
        let refused = map_file(&made, &ArchetypeIndex::new(), BinaryFormat::Pso)
            .expect_err("the box of the bench is not known");
        assert!(
            matches!(
                refused,
                CreateError::Derive(WriteError::IncompleteExtents(1))
            ),
            "{refused}"
        );
    }

    #[test]
    fn a_second_entity_takes_a_guid_no_entity_of_the_map_states() {
        let mut made = map("hills");
        place(&mut made, &bench_at(Vec3::ZERO)).expect("placed");
        made.entities[0].guid = 41;
        assert_eq!(place(&mut made, &bench_at(Vec3::X)).expect("placed"), 1);
        assert_eq!(made.entities[1].guid, 42);
    }
}
