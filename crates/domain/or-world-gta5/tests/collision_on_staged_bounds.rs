//! Every staged bound, a `.ybn`'s and a fragment's, becomes a collider, and two queries that
//! share no arithmetic agree on it: a ray dropped on it and a small sphere dropped the same way.

use glam::{Quat, Vec3};
use or_io::gta5::{File, FragDrawable, GtaFragType, PhBound, PhBoundAny};
use or_test_support::{shared_corpus_root, Corpus, TARGET_GEN8};
use or_world::collision::{BoundCollider, OrientedBox};
use or_world_gta5::collision::OfBound;

fn of_drawable(drawable: &FragDrawable<or_io::decl::Gen8>, out: &mut Vec<PhBoundAny>) {
    for bound in std::iter::once(&drawable.bound)
        .chain(&drawable.extra_bounds)
        .flatten()
    {
        out.push((**bound).clone());
    }
}

/// The bounds a fragment holds: its drawables', and its physics tree's.
fn of_fragment(frag: &GtaFragType, out: &mut Vec<PhBoundAny>) {
    for drawable in frag.common_drawable.iter().chain(&frag.extra_drawables) {
        of_drawable(drawable, out);
    }
    let Some(group) = frag.physics_lod_group.as_ref() else {
        return;
    };
    for lod in group.lods() {
        for child in &lod.children {
            for drawable in [&child.undamaged_entity, &child.damaged_entity]
                .into_iter()
                .flatten()
            {
                of_drawable(drawable, out);
            }
        }
        if let Some(bound) = lod.composite_bounds.as_ref() {
            out.push((**bound).clone());
        }
        for archetype in [&lod.phys_damp_undamaged, &lod.phys_damp_damaged]
            .into_iter()
            .flatten()
        {
            if let Some(bound) = archetype.bound.as_ref() {
                out.push((**bound).clone());
            }
        }
    }
}

fn staged(what: &str) -> Option<Vec<(String, PhBoundAny)>> {
    let files = |ext: &str| {
        Corpus::with_root(shared_corpus_root(env!("CARGO_MANIFEST_DIR")), ext)
            .target(TARGET_GEN8)
            .find()
    };
    let name = |path: &std::path::Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for path in files("ybn") {
        let bound = PhBoundAny::open(&path).unwrap_or_else(|e| panic!("{}: {e}", name(&path)));
        out.push((name(&path), bound));
    }
    for path in files("yft") {
        let frag = GtaFragType::open(&path).unwrap_or_else(|e| panic!("{}: {e}", name(&path)));
        let mut found = Vec::new();
        of_fragment(&frag, &mut found);
        out.extend(found.into_iter().map(|bound| (name(&path), bound)));
    }
    if out.is_empty() {
        or_test_support::skip_corpus(what, "no .ybn or .yft corpus");
        return None;
    }
    Some(out)
}

#[test]
fn every_staged_bound_becomes_a_collider_whose_ray_and_swept_sphere_agree() {
    let Some(bounds) =
        staged("every_staged_bound_becomes_a_collider_whose_ray_and_swept_sphere_agree")
    else {
        return;
    };
    const RADIUS: f32 = 0.05;
    let (mut shapes, mut shapeless, mut dropped, mut landed) = (0usize, 0usize, 0usize, 0usize);
    for (file, bound) in &bounds {
        let collider = BoundCollider::new(bound).unwrap_or_else(|e| panic!("{file}: {e}"));
        shapes += collider.shape_count();
        shapeless += collider.shapeless();
        let all = collider.local_box();
        if all.is_empty() {
            continue;
        }
        let size = all.max - all.min;
        let reach = size.z + 2.0;

        // A box about the whole bound holds every shape, and one beside it none.
        let whole = OrientedBox {
            centre: (all.min + all.max) * 0.5,
            rotation: Quat::IDENTITY,
            half_extents: size * 0.5 + Vec3::splat(1e-3) + size * 1e-4,
        };
        let held = collider.overlap_box(&whole);
        assert_eq!(
            (held.inside.len(), held.crossing.len()),
            (collider.shape_count(), 0),
            "{file}: the box of the bound holds all of it"
        );
        let beside = OrientedBox {
            centre: whole.centre + Vec3::X * (size.x + 10.0),
            ..whole
        };
        let none = collider.overlap_box(&beside);
        assert!(none.inside.is_empty() && none.crossing.is_empty(), "{file}");

        for i in 0..16 {
            let across = Vec3::new((i % 4) as f32 + 0.5, (i / 4) as f32 + 0.5, 0.0) / 4.0;
            let from = Vec3::new(
                all.min.x + size.x * across.x,
                all.min.y + size.y * across.y,
                all.max.z + 1.0,
            );
            dropped += 1;
            let ray = collider.ray_cast(from, Vec3::NEG_Z, reach);
            let ball = collider.sweep_sphere(from, Vec3::NEG_Z, reach, RADIUS);
            let Some(ray) = ray else { continue };
            landed += 1;
            assert!(
                ray.position.cmpge(all.min - 1e-3).all()
                    && ray.position.cmple(all.max + 1e-3).all(),
                "{file}: met at {} outside the bound's box",
                ray.position
            );
            // ⚠ A sphere about a point touches whatever the point meets, and no later.
            let ball = ball.unwrap_or_else(|| {
                panic!(
                    "{file}: a ray from {from} lands at {} and a sphere passes",
                    ray.distance
                )
            });
            assert!(
                ball.distance <= ray.distance + 1e-3,
                "{file}: from {from} the sphere touches at {} and its middle at {}",
                ball.distance,
                ray.distance
            );
            // And no sooner than its radius over the cosine of the slope it lands on allows.
            if ray.normal.z > 0.5 && ray.part == ball.part {
                assert!(
                    ball.distance >= ray.distance - RADIUS / ray.normal.z - 1e-3,
                    "{file}: from {from} the sphere touches at {} and its middle at {}",
                    ball.distance,
                    ray.distance
                );
            }
        }
    }
    println!(
        "{} bounds came to {shapes} shapes with {shapeless} given none; {landed} of {dropped} drops landed",
        bounds.len()
    );
    assert!(
        shapes > 1000 && landed > 100,
        "too little was reached to say anything"
    );
}

/// The bounds under `bound` that are not composites, through every composite.
fn leaves<'a>(bound: &'a PhBoundAny, out: &mut Vec<&'a PhBoundAny>) {
    match bound {
        PhBoundAny::PhBoundComposite(composite) => {
            for child in composite.bounds.iter().flatten() {
                leaves(child, out);
            }
        }
        leaf => out.push(leaf),
    }
}

/// ⚠ A capsule, a disc and a cylinder state a box and no layout of their own says how the
/// shape lies in it: which axis it runs along, and which of the header's numbers is its radius.
/// The shape made of each fills the box its header states, or it is read wrongly.
#[test]
fn every_staged_primitive_bound_fills_the_box_it_states() {
    let Some(bounds) = staged("every_staged_primitive_bound_fills_the_box_it_states") else {
        return;
    };
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for (file, bound) in &bounds {
        let mut found = Vec::new();
        leaves(bound, &mut found);
        for leaf in found {
            let (kind, header): (&str, &PhBound) = match leaf {
                PhBoundAny::PhBoundSphere(b) => ("sphere", &b.base),
                PhBoundAny::PhBoundCapsule(b) => ("capsule", &b.base),
                PhBoundAny::PhBoundBox(b) => ("box", &b.base),
                PhBoundAny::PhBoundCylinder(b) => ("cylinder", &b.base),
                PhBoundAny::PhBoundDisc(b) => ("disc", &b.base),
                _ => continue,
            };
            let shape = BoundCollider::new(leaf).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(shape.shape_count(), 1, "{file}: a {kind} is one shape");
            let stated_max = &header.bounding_box_max_xyz_margin_w;
            let (low, high) = (
                Vec3::from(header.bounding_box_min()),
                Vec3::new(stated_max.x, stated_max.y, stated_max.z),
            );
            let made = shape.local_box();
            let slack = 1e-3 * (high - low).max_element().max(1.0);
            assert!(
                (made.min - low).abs().max_element() <= slack
                    && (made.max - high).abs().max_element() <= slack,
                "{file}: a {kind} stated from {low} to {high} was made from {} to {}",
                made.min,
                made.max
            );
            *kinds.entry(kind).or_default() += 1;
        }
    }
    println!("the shapes that fill the box their header states: {kinds:?}");
    // The three whose reading is a choice, each seen.
    for kind in ["capsule", "disc", "cylinder"] {
        assert!(
            kinds.contains_key(kind),
            "no {kind} was reached, so its reading was not measured"
        );
    }
}

/// ⚠ A geometry's vertices are sixteen bits an axis about a centre the bound states, and its
/// header states the box of the whole: shapes made of vertices read in another space, or to
/// another scale, leave that box. How far inside it they lie is printed, not held: every
/// fragment's geometry states a box some way past its vertices.
#[test]
fn every_staged_geometry_lies_in_the_box_it_states() {
    let Some(bounds) = staged("every_staged_geometry_lies_in_the_box_it_states") else {
        return;
    };
    let (mut geometries, mut flush, mut inset) = (0usize, 0usize, 0usize);
    for (file, bound) in &bounds {
        let mut found = Vec::new();
        leaves(bound, &mut found);
        for leaf in found {
            let header: &PhBound = match leaf {
                PhBoundAny::PhBoundGeometry(g) => &g.base,
                PhBoundAny::PhBoundBVH(b) => &b.base.base,
                _ => continue,
            };
            let shapes = BoundCollider::new(leaf).unwrap_or_else(|e| panic!("{file}: {e}"));
            if shapes.shape_count() == 0 {
                continue;
            }
            let stated_max = &header.bounding_box_max_xyz_margin_w;
            let (low, high) = (
                Vec3::from(header.bounding_box_min()),
                Vec3::new(stated_max.x, stated_max.y, stated_max.z),
            );
            let made = shapes.local_box();
            let slack = 1e-3 * (high - low).max_element().max(1.0);
            assert!(
                made.min.cmpge(low - slack).all() && made.max.cmple(high + slack).all(),
                "{file}: a geometry stated from {low} to {high} reaches from {} to {}",
                made.min,
                made.max
            );
            geometries += 1;
            // The least it is inside its box, over the six sides, to five millimetres.
            let inside = (made.min - low).min(high - made.max).min_element();
            flush += usize::from(inside <= 5e-3);
            inset += usize::from((inside - 0.04).abs() <= 5e-3);
        }
    }
    println!(
        "{geometries} geometries lie in the box they state: {flush} reach a side of it, and \
         {inset} stop 40 mm short of every side"
    );
    assert!(
        geometries > 0,
        "no geometry was reached, so this measured nothing"
    );
}
