//! The collision of a building over bounds sources built by hand: what is proposed for its box is taken out of the three versions of
//! its area alike and the inverse puts every file back byte for byte; a ray that met its wall then passes and the ground under it is
//! still found; a version that lacks a polygon, and a bound the source cannot build again, refuse by name with nothing written; a
//! move leaves the corners a neighbour shares where they were; and a material is set in every version.
//!
//! ⚠ The bounds are made here and hold no game content, so the clean checkout runs this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3};
use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::{Aligned, File};
use or_io::desc::Image;
use or_io::gta5::bounds::build::{build, COMPOSITE_ITEM_THRESHOLD};
use or_io::gta5::bounds::{PhBoundAny, PhBoundBVH, PhBoundComposite};
use or_maps::collision::sample::area_bound;
use or_maps::collision::{
    apply, apply_all, bounds_sources, carried, object_box, propose, Error, Operation, Operations,
    Proposal, FORMAT, SAME_CORNER,
};
use or_maps::import::{add_files, Incoming};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_source::asset::Source;
use or_source::bounds::edit::{leaves, Polygon, PolygonKind};
use or_source::bounds::{self, MODEL};
use or_source::textures::Extraction;
use or_world::collision::{
    AssociationOptions, BoundCollider, CollisionSet, GroundProbe, OrientedBox,
};
use or_world_gta5::catalogue::area_of;
use or_world_gta5::collision::OfBound;

const PLAIN: &str = "street_1.bounds.src";
const HI: &str = "hi@street_1.bounds.src";
const MA: &str = "ma@street_1.bounds.src";
/// The soup every version holds its polygons in: the one child of its composite.
const SOUP: &str = "bound_child0";

/// The building's box in its own space, which is the world's: it stands on the origin.
const BB_MIN: Vec3 = Vec3::ZERO;
const BB_MAX: Vec3 = Vec3::new(10.0, 8.0, 4.0);
/// Where the composite puts its child, so a soup's own space is not the world's.
const CHILD_AT: Vec3 = Vec3::new(2.5, -1.5, 0.25);
/// Where the composite of the `hi@` version puts its child: the versions hold one street in the file's space and not in a soup's.
const HI_CHILD_AT: Vec3 = Vec3::new(-4.0, 7.5, -1.0);
const FLOOR_REACH: f32 = 30.0;
const CELL: f32 = 5.0;

const CONCRETE: u64 = 0x11;
const BRICK: u64 = 0x0000_0001_0000_0022;
const GLASS: u64 = 0x0000_0002_0000_0033;

/// The triangles of an area in the world, each with the slot of its material.
#[derive(Default, Clone)]
struct Area {
    triangles: Vec<([Vec3; 3], u8)>,
}

impl Area {
    fn quad(&mut self, [a, b, c, d]: [Vec3; 4], material: u8) {
        self.triangles.push(([a, b, c], material));
        self.triangles.push(([a, c, d], material));
    }

    fn wall(&mut self, from: [f32; 2], to: [f32; 2], top: f32, material: u8) {
        let at = |p: [f32; 2], z: f32| Vec3::new(p[0], p[1], z);
        self.quad(
            [at(from, 0.0), at(to, 0.0), at(to, top), at(from, top)],
            material,
        );
    }

    fn floor(&mut self) {
        let cells = (2.0 * FLOOR_REACH / CELL) as i32;
        for i in 0..cells {
            for j in 0..cells {
                let (x, y) = (
                    -FLOOR_REACH + i as f32 * CELL,
                    -FLOOR_REACH + j as f32 * CELL,
                );
                self.quad(
                    [
                        Vec3::new(x, y, 0.0),
                        Vec3::new(x + CELL, y, 0.0),
                        Vec3::new(x + CELL, y + CELL, 0.0),
                        Vec3::new(x, y + CELL, 0.0),
                    ],
                    0,
                );
            }
        }
    }

    /// The four walls and the roof of the building: ten triangles.
    fn building(&mut self) {
        let (lo, hi) = (BB_MIN, BB_MAX);
        for (from, to) in [
            ([lo.x, lo.y], [hi.x, lo.y]),
            ([hi.x, lo.y], [hi.x, hi.y]),
            ([hi.x, hi.y], [lo.x, hi.y]),
            ([lo.x, hi.y], [lo.x, lo.y]),
        ] {
            self.wall(from, to, hi.z, 1);
        }
        self.quad(
            [
                Vec3::new(lo.x, lo.y, hi.z),
                Vec3::new(hi.x, lo.y, hi.z),
                Vec3::new(hi.x, hi.y, hi.z),
                Vec3::new(lo.x, hi.y, hi.z),
            ],
            1,
        );
    }

    /// The neighbour's wall, which goes on from the building's corner and shares its two corners there.
    fn neighbour(&mut self) {
        self.wall(
            [BB_MAX.x, BB_MAX.y],
            [BB_MAX.x, BB_MAX.y + 8.0],
            BB_MAX.z,
            1,
        );
    }

    /// A mast far from the building that stands taller than it, so a bound that holds it stores its heights in another quantum.
    fn mast(&mut self) {
        self.wall([-25.0, -25.0], [-24.0, -25.0], 9.0, 1);
    }

    /// The street as each version holds it: a floor, the building and its neighbour.
    fn street() -> Self {
        let mut area = Self::default();
        area.floor();
        area.building();
        area.neighbour();
        area
    }
}

/// The file of a composite whose one child is a tree bound over `area`, placed at [`CHILD_AT`]; `spoil` changes the composite first.
fn file_of(area: &Area, materials: &[u64], spoil: impl FnOnce(&mut PhBoundComposite)) -> Vec<u8> {
    file_at(area, materials, CHILD_AT, spoil)
}

/// [`file_of`], with the child placed at `child_at`.
fn file_at(
    area: &Area,
    materials: &[u64],
    child_at: Vec3,
    spoil: impl FnOnce(&mut PhBoundComposite),
) -> Vec<u8> {
    let triangles: Vec<_> = (area.triangles.iter())
        .map(|(corners, slot)| (corners.map(|c| c.to_array()), *slot))
        .collect();
    let PhBoundAny::PhBoundComposite(mut composite) =
        area_bound(&triangles, materials, child_at.to_array()).expect("a mesh a bound holds")
    else {
        panic!("a composite");
    };
    spoil(&mut composite);
    PhBoundAny::PhBoundComposite(composite)
        .to_bytes()
        .expect("the bound saves")
}

/// A change made to a sample composite before it is saved.
type Spoil = fn(&mut PhBoundComposite);

/// The tree bound a sample composite holds.
fn child_of(composite: &mut PhBoundComposite) -> &mut PhBoundBVH {
    match composite.bounds[0].as_deref_mut() {
        Some(PhBoundAny::PhBoundBVH(child)) => child,
        _ => panic!("the one child is a tree bound"),
    }
}

fn extract(folder: &Path, name: &str, file: &[u8]) {
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    bounds::extract(file, name, folder, &how).unwrap_or_else(|why| panic!("{name}: {why:?}"));
}

/// A pack that holds the three versions of the street: the plain one, `hi@` with a mast besides, its polygons in another order and
/// its soup placed elsewhere by its composite, and `ma@` with a third material in its list.
fn pack(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    let street = Area::street();
    extract(
        &folder,
        "street_1",
        &file_of(&street, &[CONCRETE, BRICK], |_| {}),
    );
    let mut high = Area::default();
    high.mast();
    high.neighbour();
    high.building();
    high.floor();
    extract(
        &folder,
        "hi@street_1",
        &file_at(&high, &[CONCRETE, BRICK], HI_CHILD_AT, |_| {}),
    );
    extract(
        &folder,
        "ma@street_1",
        &file_of(&street, &[CONCRETE, BRICK, GLASS], |_| {}),
    );
    folder
}

/// Every file under `folder`, by its path.
fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(folder, &mut out);
    out
}

/// The paths whose bytes differ between two listings, or that only one holds.
fn differing(a: &BTreeMap<PathBuf, Vec<u8>>, b: &BTreeMap<PathBuf, Vec<u8>>) -> Vec<String> {
    let mut paths: Vec<&PathBuf> = a.keys().chain(b.keys()).collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter(|path| a.get(*path) != b.get(*path))
        .map(|path| path.display().to_string())
        .collect()
}

/// The bound a source compiles to, which passes the check of what the game would misread, edited or not.
fn compiled(folder: &Path, source: &str) -> PhBoundAny {
    let made = bounds::compile(&Source::open(&folder.join(source)).unwrap())
        .unwrap_or_else(|why| panic!("{source}: {why:?}"));
    let opened = Rsc7::open(&made.bytes).unwrap();
    let aligned = Aligned::from_bytes(&opened.data);
    let image = Image::new(aligned.bytes(), opened.virtual_size as usize);
    or_io::gta5::bounds::validate(image, 0)
        .unwrap_or_else(|why| panic!("{source} compiles to a bound that does not validate: {why}"));
    PhBoundAny::from_bytes(&made.bytes).unwrap()
}

/// The polygons of the one soup of a version, in the world.
fn polygons_of(folder: &Path, source: &str) -> Vec<Polygon> {
    let held = leaves(&compiled(folder, source)).unwrap();
    let [leaf] = held.as_slice() else {
        panic!("{source} holds one leaf");
    };
    assert_eq!(leaf.address, SOUP);
    let [[_, _, _, child_at]] = leaf.placed_by.as_slice() else {
        panic!("{source} places its soup by one composite");
    };
    let mut soup = leaf.soup.clone().expect("a soup");
    for polygon in &mut soup.polygons {
        for point in &mut polygon.points {
            *point = (Vec3::from_array(*point) + Vec3::from_array(*child_at)).to_array();
        }
    }
    soup.polygons
}

/// Whether a polygon is one of the building's: within its box and not lying on the floor.
fn of_the_building(polygon: &Polygon) -> bool {
    let slack = Vec3::splat(0.02);
    let within = polygon.points.iter().all(|p| {
        let p = Vec3::from_array(*p);
        p.cmpge(BB_MIN - slack).all() && p.cmple(BB_MAX + slack).all()
    });
    within && polygon.points.iter().any(|p| p[2] > 1.0)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    Vec3::from_array(a).distance(Vec3::from_array(b)) <= SAME_CORNER
}

/// Whether `held` is the triangle on `corners`, whichever comes first.
fn is_triangle(held: &Polygon, corners: &[Vec3; 3]) -> bool {
    held.kind == PolygonKind::Triangle
        && corners
            .iter()
            .all(|c| held.points.iter().any(|p| close(*p, c.to_array())))
}

fn object() -> OrientedBox {
    OrientedBox::placed(BB_MIN, BB_MAX, Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)
}

fn proposal(folder: &Path) -> Proposal {
    propose(folder, &object(), &AssociationOptions::default()).unwrap()
}

/// The operation a proposal makes for `source`, asked of every version of its area and not of `source` alone.
fn of_every_version(folder: &Path, source: &str) -> Operation {
    let mut operation = (proposal(folder).removal().operations.into_iter())
        .find(|operation| operation.source() == source)
        .expect("polygons are proposed in the source");
    let Operation::RemovePolygons { alone, .. } = &mut operation else {
        panic!("a removal");
    };
    assert!(
        *alone,
        "a proposal changes each version by what is proposed in it"
    );
    *alone = false;
    operation
}

/// The one bound of `source` as a set a query is asked of.
fn world(folder: &Path, source: &str) -> CollisionSet<&'static str> {
    let collider = BoundCollider::new(&compiled(folder, source)).unwrap();
    let mut set = CollisionSet::new();
    set.add("street", Arc::new(collider), Affine3A::IDENTITY)
        .unwrap();
    set
}

const VERSIONS: [&str; 3] = [PLAIN, HI, MA];

#[test]
fn the_three_versions_are_told_by_the_name_of_their_file_and_found_in_the_pack() {
    assert_eq!(area_of("street_1"), "street_1");
    assert_eq!(area_of("hi@street_1"), "street_1");
    assert_eq!(area_of("ma@street_1"), "street_1");
    assert_eq!(area_of("lo@street_1"), "lo@street_1", "no third prefix");
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let mut wanted = VERSIONS.map(str::to_string).to_vec();
    wanted.sort();
    assert_eq!(bounds_sources(&folder).unwrap(), wanted);
}

#[test]
fn the_proposal_names_the_buildings_polygons_in_each_version_and_comes_to_an_operation_for_each() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let proposal = proposal(&folder);
    assert!(proposal.association.not_judged.is_none());
    assert_eq!(proposal.association.bounds.len(), 3);
    for bound in &proposal.association.bounds {
        let held = polygons_of(&folder, &bound.key);
        let [leaf] = bound.leaves.as_slice() else {
            panic!("{} is one soup", bound.key);
        };
        let name = proposal
            .leaf(&bound.key, leaf.leaf)
            .expect("a leaf it names");
        assert_eq!((name.address.as_str(), name.soup), (SOUP, true));
        let proposed: Vec<usize> = leaf
            .proposed
            .iter()
            .map(|p| p.polygon.expect("a polygon of a soup") as usize)
            .collect();
        let own: Vec<usize> = (0..held.len())
            .filter(|i| of_the_building(&held[*i]))
            .collect();
        assert_eq!(own.len(), 10, "four walls and a roof");
        assert_eq!(proposed, own, "{}", bound.key);
    }
    // The tree of a bound orders its polygons, so the two versions that hold the same street hold it in another order than `hi@`.
    let places = |source: &str| -> Vec<u32> {
        let bound = (proposal.association.bounds.iter())
            .find(|bound| bound.key == source)
            .unwrap();
        (bound.leaves[0].proposed.iter())
            .filter_map(|p| p.polygon)
            .collect()
    };
    assert_ne!(places(PLAIN), places(HI));

    let planned = proposal.removal();
    assert_eq!(planned.notes, Vec::<String>::new());
    let mut sources = VERSIONS;
    sources.sort_unstable();
    let wanted: Vec<Operation> = sources
        .iter()
        .map(|source| Operation::RemovePolygons {
            source: source.to_string(),
            soup: SOUP.to_string(),
            polygons: places(source),
            expects: proposal.digests.get(*source).cloned(),
            alone: true,
        })
        .collect();
    assert_eq!(
        planned.operations, wanted,
        "an operation for each version, on what is proposed in it"
    );
    assert_eq!(proposal.digests.len(), 3);
    assert_eq!(
        proposal.digests[PLAIN].len(),
        64,
        "a digest of 32 bytes in hex"
    );
}

#[test]
fn taking_the_proposal_out_changes_the_three_versions_alike_and_the_inverse_restores_every_byte() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    let street = Area::street();
    let building = {
        let mut area = Area::default();
        area.building();
        area.triangles
    };
    let held_before: Vec<usize> = VERSIONS
        .iter()
        .map(|source| polygons_of(&folder, source).len())
        .collect();

    // One operation, on the version with no prefix, asked of every version of the area.
    let operations = vec![of_every_version(&folder, PLAIN)];
    let operation = &operations[0];
    // A dry run says what would change and writes nothing.
    let dry = apply_all(&folder, &operations, true).unwrap();
    assert_eq!(dry[0].changed.len(), 3);
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    let applied = apply(&folder, operation).unwrap();
    let mut changed = VERSIONS.map(str::to_string).to_vec();
    changed.sort();
    assert_eq!(applied.changed, changed);
    assert_eq!(applied.report.len(), 3, "{:?}", applied.report);
    let after = files(&folder);
    // The triangles of a bound are in its glb, and its document states none of them.
    let mut glbs: Vec<String> = VERSIONS
        .iter()
        .map(|source| folder.join(source).join(MODEL).display().to_string())
        .collect();
    glbs.sort();
    assert_eq!(differing(&before, &after), glbs);
    for (source, was) in VERSIONS.iter().zip(&held_before) {
        let held = polygons_of(&folder, source);
        assert_eq!(held.len(), was - building.len(), "{source}");
        for (corners, _) in &building {
            assert!(
                !held.iter().any(|p| is_triangle(p, corners)),
                "{source} still holds a triangle of the building"
            );
        }
        // What is not the building's stays: the floor and the neighbour's wall, triangle for triangle.
        for (corners, _) in
            (street.triangles.iter()).filter(|(c, _)| !building.iter().any(|(b, _)| b == c))
        {
            assert!(
                held.iter().any(|p| is_triangle(p, corners)),
                "{source} lost a triangle that is not the building's"
            );
        }
    }

    // The places were chosen in the source as it was: the same operation is refused now that the source is another, and one that
    // states no digest is the caller's to answer for.
    let said = apply(&folder, operation)
        .expect_err("a stale place")
        .to_string();
    assert!(
        said.contains(&format!(
            "`{PLAIN}` is not the source the polygons were chosen in"
        )),
        "{said}"
    );
    assert_eq!(differing(&after, &files(&folder)), Vec::<String>::new());

    // The inverse, as a list of operations a file holds, puts every file of every version back.
    let list = Operations::new(vec![applied.inverse.clone()]);
    let text = list.to_json().unwrap();
    assert!(text.contains(&format!("\"format\": \"{FORMAT}\"")));
    let read = Operations::from_json(text.as_bytes(), "the inverse").unwrap();
    assert_eq!(read, list);
    let back = apply_all(&folder, &read.operations, false).unwrap();
    assert_eq!(back[0].changed, changed);
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
    // And the inverse of the inverse takes the building out again.
    apply(&folder, &back[0].inverse).unwrap();
    assert_eq!(differing(&after, &files(&folder)), Vec::<String>::new());
}

#[test]
fn versions_that_hold_other_polygons_are_each_changed_by_what_is_proposed_in_them() {
    // As the game ships them: the version with no prefix holds the street, `hi@` holds the building with each wall cut the other
    // way and no floor, and `ma@` holds the floor and the neighbour and nothing of the building.
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    extract(
        &folder,
        "street_1",
        &file_of(&Area::street(), &[CONCRETE, BRICK], |_| {}),
    );
    let mut cut_the_other_way = Area::default();
    cut_the_other_way.building();
    for pair in cut_the_other_way.triangles.chunks_mut(2) {
        // A rectangle `a b c d` as `a b d` and `b c d`, where it was `a b c` and `a c d`.
        let ([a, b, c], [_, _, d]) = (pair[0].0, pair[1].0);
        (pair[0].0, pair[1].0) = ([a, b, d], [b, c, d]);
    }
    cut_the_other_way.mast();
    extract(
        &folder,
        "hi@street_1",
        &file_at(&cut_the_other_way, &[CONCRETE, BRICK], HI_CHILD_AT, |_| {}),
    );
    let mut ground = Area::default();
    ground.floor();
    ground.neighbour();
    extract(
        &folder,
        "ma@street_1",
        &file_of(&ground, &[CONCRETE, BRICK], |_| {}),
    );
    let before = files(&folder);
    let held: Vec<usize> = VERSIONS
        .iter()
        .map(|source| polygons_of(&folder, source).len())
        .collect();

    // Asked of every version, the building's polygons of one are not in the next, which is named.
    let said = apply(&folder, &of_every_version(&folder, PLAIN))
        .expect_err("the versions are not copies")
        .to_string();
    assert!(
        said.contains(&format!("the version `{HI}` holds no polygon like polygon")),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    // The proposal judges the box against each version: ten polygons in two of them and none in the third.
    let planned = proposal(&folder).removal();
    assert_eq!(
        planned
            .operations
            .iter()
            .map(Operation::source)
            .collect::<Vec<_>>(),
        [HI, PLAIN]
    );
    let applied = apply_all(&folder, &planned.operations, false).unwrap();
    let after = files(&folder);
    let mut glbs = [HI, PLAIN].map(|source| folder.join(source).join(MODEL).display().to_string());
    glbs.sort();
    assert_eq!(differing(&before, &after), glbs, "`ma@` is not written");
    for (source, (was, lost)) in VERSIONS.iter().zip(held.iter().zip([10, 10, 0])) {
        let now = polygons_of(&folder, source);
        assert_eq!(now.len(), was - lost, "{source}");
        assert!(!now.iter().any(of_the_building), "{source}");
    }
    // The inverses, last first, put every file back.
    for done in applied.iter().rev() {
        apply(&folder, &done.inverse).unwrap();
    }
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_ray_that_met_the_wall_passes_once_it_is_taken_out_and_the_ground_under_it_is_still_found() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let from = Vec3::new(-5.0, 4.0, 2.0);
    let inside = Vec3::new(5.0, 4.0, 1.0);
    let probe = GroundProbe {
        max_rise: 0.5,
        max_drop: 10.0,
        max_slope: 45f32.to_radians(),
    };
    for source in VERSIONS {
        let set = world(&folder, source);
        let (_, wall) = set.ray_cast(from, Vec3::X, 100.0).expect("the west wall");
        assert!((wall.distance - 5.0).abs() < 0.01, "{}", wall.distance);
        let (_, ground) = set.ground(inside, &probe).expect("the floor");
        assert!(ground.position.z.abs() < 0.01);
    }

    let operations = proposal(&folder).removal().operations;
    apply_all(&folder, &operations, false).unwrap();
    for source in VERSIONS {
        let set = world(&folder, source);
        assert!(
            set.ray_cast(from, Vec3::X, 100.0).is_none(),
            "{source}: the ray passes where both walls stood"
        );
        let (_, ground) = set
            .ground(inside, &probe)
            .expect("the floor is still there");
        assert!(ground.position.z.abs() < 0.01, "{source}");
        // From above where the roof was, the ground is the floor too.
        let over = set
            .ground(inside + Vec3::Z * 9.0, &probe)
            .expect("nothing is between");
        assert!(over.1.position.z.abs() < 0.01, "{source}");
    }
}

#[test]
fn a_version_that_lacks_a_polygon_refuses_the_operation_by_name_and_nothing_is_written() {
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    let street = Area::street();
    extract(
        &folder,
        "street_1",
        &file_of(&street, &[CONCRETE, BRICK], |_| {}),
    );
    extract(
        &folder,
        "hi@street_1",
        &file_of(&street, &[CONCRETE, BRICK], |_| {}),
    );
    // The third version holds the street without the last triangle of the building's roof.
    let roof_end = {
        let mut area = Area::default();
        area.floor();
        area.building();
        area.triangles.len() - 1
    };
    let mut short = street.clone();
    short.triangles.remove(roof_end);
    extract(
        &folder,
        "ma@street_1",
        &file_of(&short, &[CONCRETE, BRICK], |_| {}),
    );
    let before = files(&folder);

    let refused = apply(&folder, &of_every_version(&folder, PLAIN)).expect_err("a version differs");
    let said = refused.to_string();
    assert!(
        matches!(&refused, Error::Refused { name: "remove_polygons", at, .. } if at == PLAIN),
        "{said}"
    );
    assert!(
        said.contains(&format!("the version `{MA}` holds no polygon like polygon"))
            && said.contains("a triangle at"),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_bound_the_source_cannot_build_again_refuses_the_operation_by_name_and_nothing_is_written() {
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    let street = Area::street();
    extract(
        &folder,
        "hi@street_1",
        &file_of(&street, &[CONCRETE, BRICK], |_| {}),
    );
    // A hull follows the vertices one for one, and the source does not make one again.
    let with_a_hull = file_of(&street, &[CONCRETE, BRICK], |composite| {
        let child = child_of(composite);
        child.base.compressed_shrunk_vertices = child.base.compressed_vertices.clone();
    });
    extract(&folder, "street_1", &with_a_hull);
    let before = files(&folder);

    // The proposal's list takes the building out of `hi@` first, which can be done, and is refused at the version with the hull:
    // what was done before it is not written.
    let operations = proposal(&folder).removal().operations;
    assert_eq!(
        operations.iter().map(Operation::source).collect::<Vec<_>>(),
        [HI, PLAIN]
    );
    let refused = apply_all(&folder, &operations, false).expect_err("a hull is not built again");
    let said = refused.to_string();
    assert!(
        matches!(
            &refused,
            Error::InList {
                number: 2,
                of: 2,
                ..
            }
        ),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "the bounds source `{PLAIN}` cannot be built again with the change to `{SOUP}`"
        )) && said.contains("a shrunk hull"),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
    // Asked of every version, the one operation is refused by the same name.
    let said = apply(&folder, &of_every_version(&folder, HI))
        .expect_err("a hull is not built again")
        .to_string();
    assert!(
        said.contains(&format!(
            "the bounds source `{PLAIN}` cannot be built again"
        )),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    // A bound that is one shape has no polygon to take, and a source that is no bounds source is not edited.
    let stray = Operation::RemovePolygons {
        source: PLAIN.to_string(),
        soup: "bound".to_string(),
        polygons: vec![0],
        expects: None,
        alone: false,
    };
    let said = apply(&folder, &stray)
        .expect_err("no soup there")
        .to_string();
    assert!(said.contains("holds no bound at `bound`"), "{said}");
    let outside = Operation::RemovePolygons {
        source: "../elsewhere.bounds.src".to_string(),
        soup: SOUP.to_string(),
        polygons: vec![0],
        expects: None,
        alone: false,
    };
    let said = apply(&folder, &outside)
        .expect_err("outside the pack")
        .to_string();
    assert!(said.contains("leaves the folder of the pack"), "{said}");
    let past = Operation::RemovePolygons {
        source: PLAIN.to_string(),
        soup: SOUP.to_string(),
        polygons: vec![1_000_000],
        expects: None,
        alone: false,
    };
    let said = apply(&folder, &past)
        .expect_err("no such polygon")
        .to_string();
    assert!(said.contains("polygon 1000000 is past the"), "{said}");
    // What is put back has to compile: a document that is none is refused, with the source it was for.
    let Operation::RestoreSources { mut sources } = apply_all(
        &folder,
        &[Operation::RemovePolygons {
            source: "street_1.bounds.src".to_string(),
            soup: SOUP.to_string(),
            polygons: Vec::new(),
            expects: None,
            alone: false,
        }],
        true,
    )
    .map(|mut applied| applied.remove(0).inverse)
    .unwrap_or_else(|_| Operation::RestoreSources {
        sources: Vec::new(),
    }) else {
        panic!("an inverse restores sources");
    };
    sources.retain(|restored| restored.source == PLAIN);
    assert_eq!(
        sources.len(),
        1,
        "taking nothing out still says what puts the source back"
    );
    sources[0].document = "not a document".to_string();
    // The run wrote nothing, so the source is not the one its inverse expects: a list a person writes may state no digest.
    sources[0].expects = None;
    let said = apply(&folder, &Operation::RestoreSources { sources })
        .expect_err("no document")
        .to_string();
    assert!(
        said.contains(&format!(
            "what is to be put back in `{PLAIN}` does not compile"
        )),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn moving_the_buildings_polygons_leaves_the_corners_a_neighbour_shares_where_they_were() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    let neighbour = {
        let mut area = Area::default();
        area.neighbour();
        area.triangles
    };
    let building = {
        let mut area = Area::default();
        area.building();
        area.triangles
    };
    // The corner the two share is a corner of each before the move.
    let shared = Vec3::new(BB_MAX.x, BB_MAX.y, BB_MAX.z);
    assert!(neighbour.iter().any(|(c, _)| c.contains(&shared)));
    assert!(building.iter().any(|(c, _)| c.contains(&shared)));
    let of_the_neighbour = |held: &[Polygon]| -> Vec<Vec<[u32; 3]>> {
        let mut found: Vec<Vec<[u32; 3]>> = neighbour
            .iter()
            .map(|(corners, _)| {
                let polygon = held
                    .iter()
                    .find(|p| is_triangle(p, corners))
                    .expect("the neighbour's wall is held");
                polygon.points.iter().map(|p| p.map(f32::to_bits)).collect()
            })
            .collect();
        found.sort();
        found
    };
    let stood: Vec<_> = VERSIONS
        .iter()
        .map(|source| of_the_neighbour(&polygons_of(&folder, source)))
        .collect();

    // The building goes six metres west and one south, turned thirty degrees about its own origin.
    let (rotation, translation) = carried(
        (Vec3::ZERO, Quat::IDENTITY),
        (
            Vec3::new(-6.0, -1.0, 0.0),
            Quat::from_rotation_z(30f32.to_radians()),
        ),
    );
    let by = Affine3A::from_rotation_translation(
        Quat::from_array(rotation),
        Vec3::from_array(translation),
    );
    let planned = proposal(&folder).moving(rotation, translation);
    let applied = apply_all(&folder, &planned.operations, false).unwrap();
    assert_eq!(applied.len(), 3, "each version by what is proposed in it");
    for done in &applied {
        // The operation says what it moved.
        let [line] = done.report.as_slice() else {
            panic!("one version each: {:?}", done.report);
        };
        assert!(line.contains("10 polygon(s) moved in"), "{line}");
        assert_eq!(done.changed.len(), 1);
    }

    for (source, stood) in VERSIONS.iter().zip(&stood) {
        let held = polygons_of(&folder, source);
        assert_eq!(
            &of_the_neighbour(&held),
            stood,
            "{source}: the neighbour's corners are where they were, to the bit"
        );
        for (corners, _) in &building {
            let moved = corners.map(|c| by.transform_point3(c));
            assert!(
                held.iter().any(|p| is_triangle(p, &moved)),
                "{source}: a triangle of the building is where the move puts it"
            );
            assert!(
                !held.iter().any(|p| is_triangle(p, corners)),
                "{source}: and no longer where it was"
            );
        }
    }
    for done in applied.iter().rev() {
        apply(&folder, &done.inverse).unwrap();
    }
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    // The same move asked of every version through one of them moves the same polygons of each.
    let mut through_one = (planned.operations.iter())
        .find(|operation| operation.source() == PLAIN)
        .expect("a move of the version with no prefix")
        .clone();
    let Operation::MovePolygons {
        polygons, alone, ..
    } = &mut through_one
    else {
        panic!("a move");
    };
    *alone = false;
    let polygons = polygons.clone();
    let alike = apply(&folder, &through_one).unwrap();
    assert_eq!(alike.changed.len(), 3);
    for (source, stood) in VERSIONS.iter().zip(&stood) {
        let held = polygons_of(&folder, source);
        assert_eq!(&of_the_neighbour(&held), stood, "{source}");
        for (corners, _) in &building {
            let moved = corners.map(|c| by.transform_point3(c));
            assert!(held.iter().any(|p| is_triangle(p, &moved)), "{source}");
        }
    }
    apply(&folder, &alike.inverse).unwrap();
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    // A turn of no length and a shift that is no number move nothing, and say so.
    for (rotation, translation, why) in [
        ([0.0; 4], [0.0; 3], "a rotation of no length"),
        (
            [0.0, 0.0, 0.0, 1.0],
            [f32::NAN, 0.0, 0.0],
            "not a finite number",
        ),
    ] {
        let broken = Operation::MovePolygons {
            source: PLAIN.to_string(),
            soup: SOUP.to_string(),
            polygons: polygons.clone(),
            rotation,
            translation,
            expects: None,
            alone: false,
        };
        // A number that is none is not JSON, so this one is handed over as it is.
        let said = apply(&folder, &broken).expect_err(why).to_string();
        assert!(said.contains(why), "{said}");
    }
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn two_polygons_on_the_same_corners_are_each_taken_out_of_every_version() {
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    // A sheet inside the building that collides from both sides: one triangle wound each way on the same corners.
    let sheet = [
        Vec3::new(4.0, 4.0, 0.5),
        Vec3::new(6.0, 4.0, 0.5),
        Vec3::new(5.0, 4.0, 3.0),
    ];
    let mut street = Area::street();
    street.triangles.push((sheet, 1));
    street.triangles.push(([sheet[0], sheet[2], sheet[1]], 1));
    extract(
        &folder,
        "street_1",
        &file_of(&street, &[CONCRETE, BRICK], |_| {}),
    );
    extract(
        &folder,
        "hi@street_1",
        &file_at(&street, &[CONCRETE, BRICK], HI_CHILD_AT, |_| {}),
    );
    let sheets = |source: &str| {
        (polygons_of(&folder, source).iter())
            .filter(|p| is_triangle(p, &sheet))
            .count()
    };
    assert_eq!([sheets(PLAIN), sheets(HI)], [2, 2]);
    let held: Vec<usize> = [PLAIN, HI]
        .map(|source| polygons_of(&folder, source).len())
        .to_vec();

    // Asked of every version through one: each of the two on the same corners is found once in the other version.
    apply(&folder, &of_every_version(&folder, PLAIN)).unwrap();
    assert_eq!([sheets(PLAIN), sheets(HI)], [0, 0]);
    for (source, was) in [PLAIN, HI].iter().zip(held) {
        assert_eq!(polygons_of(&folder, source).len(), was - 12, "{source}");
    }
}

#[test]
fn a_move_is_refused_where_a_composite_places_its_child_by_a_matrix_that_is_no_rigid_one() {
    // An axis leant over at its own length, and an axis twice as long as the others.
    let leant = |composite: &mut PhBoundComposite| {
        let axis = &mut composite.current_matrices[0].col1;
        (axis.x, axis.y) = (0.6, 0.8);
    };
    let stretched = |composite: &mut PhBoundComposite| {
        composite.current_matrices[0].col2.z = 2.0;
    };
    let spoils: [(&str, Spoil); 2] = [("leant", leant), ("stretched", stretched)];
    for (what, spoil) in spoils {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("maps");
        std::fs::create_dir_all(&folder).unwrap();
        extract(
            &folder,
            "street_1",
            &file_of(&Area::street(), &[CONCRETE, BRICK], spoil),
        );
        let before = files(&folder);
        let shift = Operation::MovePolygons {
            source: PLAIN.to_string(),
            soup: SOUP.to_string(),
            polygons: vec![0, 1],
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [1.0, 0.0, 0.0],
            expects: None,
            alone: false,
        };
        let said = apply(&folder, &shift).expect_err(what).to_string();
        assert!(
            said.contains(&format!(
                "`{SOUP}` is placed by a matrix that is not a rotation"
            )),
            "{what}: {said}"
        );
        assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
        // Taking polygons out asks nothing of the matrix.
        let taken = Operation::RemovePolygons {
            source: PLAIN.to_string(),
            soup: SOUP.to_string(),
            polygons: vec![0, 1],
            expects: None,
            alone: false,
        };
        assert_eq!(apply(&folder, &taken).unwrap().changed, [PLAIN], "{what}");
    }
}

#[test]
fn a_material_is_set_on_the_same_polygons_of_every_version_and_listed_where_it_was_not() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    let building = {
        let mut area = Area::default();
        area.building();
        area.triangles
    };
    let Operation::RemovePolygons {
        source,
        soup,
        polygons: chosen,
        expects,
        ..
    } = proposal(&folder).removal().operations.remove(0)
    else {
        panic!("a removal");
    };
    let glass = Operation::SetMaterial {
        source,
        soup,
        polygons: chosen,
        material: format!("{GLASS:#x}"),
        expects,
        alone: false,
    };
    let applied = apply(&folder, &glass).unwrap();
    assert_eq!(applied.changed.len(), 3);
    for source in VERSIONS {
        let held = leaves(&compiled(&folder, source)).unwrap().remove(0);
        let soup = held.soup.expect("a soup");
        // `ma@` listed the material already, and the other two gained it at the end of their lists.
        assert_eq!(soup.materials, [CONCRETE, BRICK, GLASS], "{source}");
        let in_the_world = polygons_of(&folder, source);
        for (polygon, placed) in soup.polygons.iter().zip(&in_the_world) {
            let material = soup.materials[usize::from(polygon.material.expect("a material"))];
            let of_the_building = building
                .iter()
                .any(|(corners, _)| is_triangle(placed, corners));
            if of_the_building {
                assert_eq!(material, GLASS, "{source}");
            } else {
                assert_ne!(material, GLASS, "{source}");
            }
        }
    }
    apply(&folder, &applied.inverse).unwrap();
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());

    let said = apply(
        &folder,
        &Operation::SetMaterial {
            source: PLAIN.to_string(),
            soup: SOUP.to_string(),
            polygons: vec![0],
            material: "glass".to_string(),
            expects: None,
            alone: false,
        },
    )
    .expect_err("no material")
    .to_string();
    assert!(said.contains("`glass` is not a material"), "{said}");
}

#[test]
fn a_list_of_another_format_or_schema_is_refused_before_an_operation_is_read() {
    let other = br#"{"format": "openrage-map-operations", "schema": 1, "operations": []}"#;
    assert!(matches!(
        Operations::from_json(other, "a list"),
        Err(Error::Format(_, format)) if format == "openrage-map-operations"
    ));
    let newer = format!(r#"{{"format": "{FORMAT}", "schema": 2, "operations": []}}"#);
    assert!(matches!(
        Operations::from_json(newer.as_bytes(), "a list"),
        Err(Error::Schema(_, 2))
    ));
    let odd = format!(
        r#"{{"format": "{FORMAT}", "schema": 1, "operations": [{{"op": "restore_sources", "sources": [{{"source": "a", "document": "{{}}", "model": "0g"}}]}}]}}"#
    );
    assert!(matches!(
        Operations::from_json(odd.as_bytes(), "a list"),
        Err(Error::Json(..))
    ));
}

#[test]
fn the_box_of_an_object_is_its_archetypes_box_under_its_placement() {
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    let at = [40.0, -12.0, 3.0];
    let yaw = 0.6f32;
    let map = map_file(
        "street",
        &[
            Entity::at("street_house", [0.0; 3], 1),
            Entity {
                yaw,
                scale: 2.0,
                ..Entity::at("street_house", at, 2)
            },
            Entity::at("prop_of_the_game", [5.0; 3], 3),
        ],
        ([-100.0; 3], [100.0; 3]),
    );
    let types = types_file(
        "street_types",
        &[Archetype::new(
            "street_house",
            BB_MIN.to_array(),
            BB_MAX.to_array(),
        )],
    );
    let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
        name,
        bytes,
        from_game: false,
        virtual_path: None,
    };
    let added = add_files(
        &folder,
        &[
            incoming("street.ymap", map.leak()),
            incoming("street_types.ytyp", types.leak()),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    const MAP: &str = "street.metadata.src";

    let unturned = object_box(&folder, MAP, 0, None).unwrap();
    assert_eq!(unturned, object());

    let placed = object_box(&folder, MAP, 1, None).unwrap();
    let entity = or_maps::edit::pose_of(&or_maps::edit::read_entity(&folder.join(MAP), 1).unwrap());
    assert!((entity.rotation.to_scaled_axis() - Vec3::Z * yaw).length() < 1e-5);
    let middle = (BB_MIN + BB_MAX) * 0.5 * 2.0;
    assert!((placed.centre - (Vec3::from_array(at) + entity.rotation * middle)).length() < 1e-4);
    assert_eq!(placed.half_extents, (BB_MAX - BB_MIN) * 0.5 * 2.0);
    assert_eq!(placed.rotation, entity.rotation);

    // An archetype no type file of the pack declares has no box until one is stated.
    let said = object_box(&folder, MAP, 2, None)
        .expect_err("a prop of the game")
        .to_string();
    // A map file keeps the hash of a name and not its text.
    let spelled = format!("hash_{:08X}", or_io::string_hash(b"prop_of_the_game"));
    assert!(
        said.contains(&format!("no type file of the pack declares `{spelled}`")),
        "{said}"
    );
    let stated = object_box(&folder, MAP, 2, Some((Vec3::splat(-1.0), Vec3::splat(1.0)))).unwrap();
    assert_eq!(
        (stated.centre, stated.half_extents),
        (Vec3::splat(5.0), Vec3::ONE)
    );
    let said = object_box(&folder, MAP, 9, None)
        .expect_err("no entity")
        .to_string();
    assert!(said.contains("entity 9 is past the 3"), "{said}");

    // A map file is no bounds source, and its collision is not edited in it.
    let stray = Operation::RemovePolygons {
        source: MAP.to_string(),
        soup: SOUP.to_string(),
        polygons: vec![0],
        expects: None,
        alone: false,
    };
    let said = apply(&folder, &stray)
        .expect_err("a map source")
        .to_string();
    assert!(
        said.contains("it is a metadata source, and collision is edited in a bounds source"),
        "{said}"
    );
}

/// The file of a composite of two tree bounds, the first over `first` at [`CHILD_AT`] and the second over `second` at [`HI_CHILD_AT`].
fn file_of_two(first: &Area, second: &Area) -> Vec<u8> {
    let composite_of = |area: &Area, child_at: Vec3| {
        let triangles: Vec<_> = (area.triangles.iter())
            .map(|(corners, slot)| (corners.map(|c| c.to_array()), *slot))
            .collect();
        match area_bound(&triangles, &[CONCRETE, BRICK], child_at.to_array()) {
            Ok(PhBoundAny::PhBoundComposite(composite)) => composite,
            _ => panic!("a composite"),
        }
    };
    let mut whole = composite_of(first, CHILD_AT);
    let mut other = composite_of(second, HI_CHILD_AT);
    whole.bounds.append(&mut other.bounds);
    whole.current_matrices.append(&mut other.current_matrices);
    whole
        .local_box_min_maxs
        .append(&mut other.local_box_min_maxs);
    whole
        .type_and_include_flags
        .append(&mut other.type_and_include_flags);
    let held: Vec<([f32; 3], [f32; 3])> = (whole.local_box_min_maxs.iter())
        .map(|b| ([b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z]))
        .collect();
    whole.bvh_structure = Some(Box::new(
        build(&held, COMPOSITE_ITEM_THRESHOLD)
            .expect("a tree over two children")
            .bvh,
    ));
    PhBoundAny::PhBoundComposite(whole)
        .to_bytes()
        .expect("the bound saves")
}

/// The polygons of every soup of a source, in the world, by the address of the soup.
fn soups_of(folder: &Path, source: &str) -> BTreeMap<String, Vec<Polygon>> {
    let mut found = BTreeMap::new();
    for leaf in leaves(&compiled(folder, source)).unwrap() {
        let [[_, _, _, child_at]] = leaf.placed_by.as_slice() else {
            panic!("{source} places each soup by one composite");
        };
        let mut soup = leaf.soup.expect("a soup");
        for point in soup.polygons.iter_mut().flat_map(|p| &mut p.points) {
            *point = (Vec3::from_array(*point) + Vec3::from_array(*child_at)).to_array();
        }
        found.insert(leaf.address, soup.polygons);
    }
    found
}

#[test]
fn a_building_held_in_two_soups_of_one_source_is_taken_out_of_both_and_put_back() {
    // The floor, the neighbour and two walls in one soup; the other two walls, the roof and a mast in the other.
    let building = {
        let mut area = Area::default();
        area.building();
        area.triangles
    };
    let mut low = Area::default();
    low.floor();
    low.neighbour();
    low.triangles.extend_from_slice(&building[..4]);
    let mut high = Area::default();
    high.mast();
    high.triangles.extend_from_slice(&building[4..]);
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    extract(&folder, "street_1", &file_of_two(&low, &high));
    let before = files(&folder);
    let held = soups_of(&folder, PLAIN);
    assert_eq!(
        held.keys().collect::<Vec<_>>(),
        ["bound_child0", "bound_child1"]
    );

    let planned = proposal(&folder).removal();
    assert_eq!(
        (planned.operations.iter())
            .map(|operation| match operation {
                Operation::RemovePolygons { soup, polygons, .. } =>
                    (operation.source(), soup.as_str(), polygons.len()),
                other => panic!("{other:?}"),
            })
            .collect::<Vec<_>>(),
        [(PLAIN, "bound_child0", 4), (PLAIN, "bound_child1", 6)],
        "an operation for each soup, both of the one source and both stating its digest"
    );
    // The second operation names its polygons by the places they had before the first rebuilt the source.
    let applied = apply_all(&folder, &planned.operations, false).unwrap();
    let now = soups_of(&folder, PLAIN);
    for (address, lost) in [("bound_child0", 4), ("bound_child1", 6)] {
        assert_eq!(now[address].len(), held[address].len() - lost, "{address}");
        assert!(!now[address].iter().any(of_the_building), "{address}");
    }
    for done in applied.iter().rev() {
        apply(&folder, &done.inverse).unwrap();
    }
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_list_that_names_one_soup_twice_by_the_places_of_one_proposal_is_refused_at_the_second() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    let Some(Operation::RemovePolygons {
        source,
        soup,
        polygons,
        expects,
        alone,
    }) = (proposal(&folder).removal().operations.into_iter())
        .find(|operation| operation.source() == PLAIN)
    else {
        panic!("a removal is proposed in the version with no prefix");
    };
    assert!(expects.is_some() && alone);
    // A person splits the operation in two: the second half names places of the soup as it was before the first half rebuilt it.
    let halves: Vec<Operation> = [&polygons[..5], &polygons[5..]]
        .into_iter()
        .map(|half| Operation::RemovePolygons {
            source: source.clone(),
            soup: soup.clone(),
            polygons: half.to_vec(),
            expects: expects.clone(),
            alone: true,
        })
        .collect();
    let refused = apply_all(&folder, &halves, false).expect_err("places of another compile");
    let said = refused.to_string();
    assert!(
        matches!(
            &refused,
            Error::InList {
                number: 2,
                of: 2,
                ..
            }
        ),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "an earlier operation of the list changed `{SOUP}` of `{PLAIN}`"
        )),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
    // Each half alone is what the proposal chose, and is done.
    apply(&folder, &halves[1]).unwrap();
    let held = polygons_of(&folder, PLAIN);
    assert_eq!(held.iter().filter(|p| of_the_building(p)).count(), 5);
}

#[test]
fn a_soup_that_is_the_buildings_alone_is_not_emptied_and_the_refusal_names_it() {
    // One version holds the building and nothing else: taking the proposal out would leave it a bound over no polygon.
    let work = tempfile::tempdir().unwrap();
    let folder = work.path().join("maps");
    std::fs::create_dir_all(&folder).unwrap();
    extract(
        &folder,
        "hi@street_1",
        &file_of(&Area::street(), &[CONCRETE, BRICK], |_| {}),
    );
    let mut only = Area::default();
    only.building();
    extract(
        &folder,
        "street_1",
        &file_of(&only, &[CONCRETE, BRICK], |_| {}),
    );
    let before = files(&folder);
    let operations = proposal(&folder).removal().operations;
    assert_eq!(
        operations.iter().map(Operation::source).collect::<Vec<_>>(),
        [HI, PLAIN]
    );
    // The version that can be built again comes first in the list, and is not written.
    let said = apply_all(&folder, &operations, false)
        .expect_err("a bound over nothing")
        .to_string();
    assert!(
        said.starts_with("operation 2 of 2, remove_polygons of `street_1.bounds.src`")
            && said.contains(&format!(
                "the bounds source `{PLAIN}` cannot be built again with the change to `{SOUP}`"
            ))
            && said.contains("holds no polygon"),
        "{said}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

#[test]
fn a_source_that_cannot_be_written_leaves_the_versions_written_before_it_as_they_were() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    // The sources are written in the order of their names, and the last of the three cannot take the part file of its glb, which
    // is written after the part file of its document.
    let mut sources = VERSIONS;
    sources.sort_unstable();
    assert_eq!(sources[2], PLAIN);
    let blocked = folder.join(PLAIN).join(format!("{MODEL}.part"));
    std::fs::create_dir(&blocked).unwrap();
    let before = files(&folder);

    let refused = apply(&folder, &of_every_version(&folder, HI)).expect_err("a folder in the way");
    assert!(matches!(&refused, Error::Write(_)), "{refused}");
    assert!(
        refused.to_string().contains("cannot write") && !refused.to_string().contains("put back"),
        "{refused}"
    );
    assert_eq!(
        differing(&before, &files(&folder)),
        Vec::<String>::new(),
        "the two versions written first are back to the byte, and no part file is left"
    );
    // With the way clear the same operation is done.
    std::fs::remove_dir(&blocked).unwrap();
    assert_eq!(
        apply(&folder, &of_every_version(&folder, HI))
            .unwrap()
            .changed
            .len(),
        3
    );
}

#[test]
fn what_stood_with_an_object_is_carried_about_where_the_object_stands() {
    // An object off the origin, already turned, turned a quarter more and moved: a point two metres along its own x goes with it.
    let from = (Vec3::new(40.0, -12.0, 3.0), Quat::from_rotation_z(0.6));
    let to = (
        Vec3::new(31.0, 5.0, 3.5),
        Quat::from_rotation_z(0.6 + std::f32::consts::FRAC_PI_2),
    );
    let (rotation, translation) = carried(from, to);
    let by = Affine3A::from_rotation_translation(
        Quat::from_array(rotation),
        Vec3::from_array(translation),
    );
    let own = Vec3::new(2.0, 0.0, 1.0);
    let stood = from.0 + from.1 * own;
    let stands = to.0 + to.1 * own;
    assert!((by.transform_point3(stood) - stands).length() < 1e-4);
    assert!((by.transform_point3(from.0) - to.0).length() < 1e-4);
}

#[test]
fn a_source_named_otherwise_than_the_pack_holds_it_is_not_edited() {
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    // ⚠ On a disk that does not tell `S` from `s` the folder opens under either name: one source would be held twice and written twice.
    let shouted = PLAIN.to_uppercase().replace(".BOUNDS.SRC", ".bounds.src");
    assert_ne!(shouted, PLAIN);
    let operations: Vec<Operation> = [PLAIN, shouted.as_str()]
        .into_iter()
        .zip([0u32, 1])
        .map(|(source, polygon)| Operation::RemovePolygons {
            source: source.to_string(),
            soup: SOUP.to_string(),
            polygons: vec![polygon],
            expects: None,
            alone: true,
        })
        .collect();
    let refused = apply_all(&folder, &operations, false).expect_err("one source by two names");
    assert!(
        matches!(&refused, Error::InList { number: 2, .. }),
        "{refused}"
    );
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}

/// ⚠ An inverse carries whole files, so applied over a later edit of the source it would take that edit out too.
#[test]
fn the_inverse_of_an_edit_is_refused_over_a_later_edit_of_the_same_source_and_taken_back_in_order()
{
    let work = tempfile::tempdir().unwrap();
    let folder = pack(work.path());
    let before = files(&folder);
    // A material no soup of the pack lists.
    const METAL: u64 = 0x0000_0003_0000_0044;
    let third = |folder: &Path| {
        let held = leaves(&compiled(folder, PLAIN)).unwrap();
        let soup = held[0].soup.clone().expect("a soup");
        let names = |slot: u8| soup.materials.get(usize::from(slot)) == Some(&METAL);
        let naming = soup
            .polygons
            .iter()
            .filter(|p| p.material.is_some_and(names));
        naming.count()
    };

    // The building is taken out, and then six polygons of what stays are given a material the soup did not list.
    let first = apply(&folder, &of_every_version(&folder, PLAIN)).unwrap();
    let later = Operation::SetMaterial {
        source: PLAIN.to_string(),
        soup: SOUP.to_string(),
        polygons: vec![0, 1, 2, 3, 4, 5],
        material: format!("{METAL:#018x}"),
        expects: None,
        alone: true,
    };
    let second = apply(&folder, &later).unwrap();
    assert_eq!(third(&folder), 6);
    let with_both = files(&folder);

    // The inverse of the first is refused while the second stands, by name, and writes nothing.
    let said = apply(&folder, &first.inverse)
        .expect_err("a later edit stands")
        .to_string();
    assert!(
        said.contains(&format!("`{PLAIN}` changed since the edit this takes back")),
        "{said}"
    );
    assert_eq!(differing(&with_both, &files(&folder)), Vec::<String>::new());
    assert_eq!(third(&folder), 6, "the later edit was discarded");

    // In order, the last first, both are taken back to the byte: as two calls, and as one list.
    let in_order = [second.inverse.clone(), first.inverse.clone()];
    let dry = apply_all(&folder, &in_order, true);
    assert!(dry.is_ok(), "{:?}", dry.err().map(|e| e.to_string()));
    apply(&folder, &second.inverse).unwrap();
    apply(&folder, &first.inverse).unwrap();
    assert_eq!(differing(&before, &files(&folder)), Vec::<String>::new());
}
