use super::super::scenes::Sequence;
use super::super::{Arena, Triangle};
use super::*;

const INFINITE: f32 = f32::INFINITY;

/// A box node at byte 0 whose child `i` is the pointer `8 * (i + 1)` over `boxes[i]`.
fn box_words(boxes: &[Option<([f32; 3], [f32; 3])>; 4]) -> Vec<u32> {
    let mut words = vec![0u32; 32];
    for (i, bounds) in boxes.iter().enumerate() {
        words[i] = INVALID;
        if let Some((min, max)) = bounds {
            words[i] = 8 * (i as u32 + 1);
            for k in 0..3 {
                words[4 + 6 * i + k] = min[k].to_bits();
                words[4 + 6 * i + 3 + k] = max[k].to_bits();
            }
        }
    }
    words
}

fn slab(from: f32, to: f32) -> Option<([f32; 3], [f32; 3])> {
    Some(([from, -1.0, -1.0], [to, 1.0, 1.0]))
}

fn along_x(words: &[u32], from: f32, t_max: f32) -> [u32; 4] {
    let direction = [1.0, 0.0, 0.0];
    let inverse = direction.map(|x: f32| 1.0 / x);
    intersect_ray(
        words,
        0,
        TYPE_BOX32,
        t_max,
        [from, 0.0, 0.0],
        direction,
        inverse,
    )
}

#[test]
fn the_rules_constants_are_the_powers_of_two_they_are_named_for() {
    let bits = [MIN, DET_MAX, BOX_GROW, TRI_GROW].map(f32::to_bits);
    assert_eq!(bits, [1 << 23, 253 << 23, 107 << 23, 106 << 23]);
}

#[test]
fn a_box_gives_the_children_it_meets_nearest_first_then_none() {
    let words = box_words(&[slab(5.0, 6.0), slab(1.0, 2.0), None, slab(3.0, 4.0)]);
    assert_eq!(along_x(&words, 0.0, 10.0), [16, 32, 8, INVALID]);
    // Behind the origin is not met; an extent that ends before a box leaves it out.
    assert_eq!(along_x(&words, 2.5, 10.0), [32, 8, INVALID, INVALID]);
    assert_eq!(along_x(&words, 0.0, 2.9), [16, INVALID, INVALID, INVALID]);
    assert_eq!(along_x(&words, 0.0, 3.5), [16, 32, INVALID, INVALID]);
    // Inside a box its near side is behind, and it is met at once.
    assert_eq!(along_x(&words, 5.5, 10.0), [8, INVALID, INVALID, INVALID]);
    // As near as each other: by child number.
    let words = box_words(&[
        slab(3.0, 6.0),
        slab(1.0, 2.0),
        slab(3.0, 6.0),
        slab(3.0, 6.0),
    ]);
    assert_eq!(along_x(&words, 0.0, 10.0), [16, 8, 24, 32]);
    // Bounds that are not numbers, or inside out, are no box.
    let none = Some(([f32::NAN; 3], [f32::NAN; 3]));
    let words = box_words(&[none, slab(4.0, 3.0), slab(1.0, 2.0), None]);
    assert_eq!(along_x(&words, 0.0, 10.0), [24, INVALID, INVALID, INVALID]);
}

#[test]
fn a_ray_in_the_plane_two_boxes_share_meets_both() {
    let boxes = [
        Some(([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])),
        Some(([1.0, 0.0, 0.0], [2.0, 1.0, 1.0])),
        None,
        None,
    ];
    let words = box_words(&boxes);
    let down = |x: f32, zero: f32| {
        let direction = [zero, zero, -1.0];
        let inverse = direction.map(|d: f32| 1.0 / d);
        intersect_ray(
            &words,
            0,
            TYPE_BOX32,
            INFINITE,
            [x, 0.5, 5.0],
            direction,
            inverse,
        )
    };
    for zero in [0.0, -0.0, 1e-40] {
        assert_eq!(down(1.0, zero), [8, 16, INVALID, INVALID]);
        assert_eq!(down(1.5, zero), [16, INVALID, INVALID, INVALID]);
        assert_eq!(down(0.0, zero), [8, INVALID, INVALID, INVALID]);
        assert_eq!(down(2.5, zero), [INVALID; 4]);
    }
    // A ray along the very skin of a widened box is inside it. The box is widened by 2^-20 of
    // its greatest distance, 2^20 here, so by one: its side at nought in x reaches the origin's.
    let far = 1_048_576.0;
    let skin = Some(([-1.0, far - 1.0, -1.0], [0.0, far + 1.0, 1.0]));
    let words = box_words(&[skin, None, None, None]);
    let down = |x: f32| {
        let (direction, inverse) = ([0.0, 0.0, -1.0], [INFINITE, INFINITE, -1.0]);
        intersect_ray(
            &words,
            0,
            TYPE_BOX32,
            INFINITE,
            [x, far, 5.0],
            direction,
            inverse,
        )
    };
    assert_eq!(down(1.0), [8, INVALID, INVALID, INVALID]);
    assert_eq!(down(1.5), [INVALID; 4]);
}

fn lone(corners: [[f32; 3]; 3]) -> (Arena, u32, u32) {
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners,
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    (arena, bottom.base, bottom.root)
}

fn shoot(arena: &Arena, base: u32, node: u32, origin: [f32; 3], direction: [f32; 3]) -> [f32; 4] {
    let inverse = direction.map(|x| 1.0 / x);
    intersect_ray(
        arena.words(),
        base,
        node,
        INFINITE,
        origin,
        direction,
        inverse,
    )
    .map(f32::from_bits)
}

/// A result's distance, as the programs make it: a reciprocal, then a product.
fn distance(result: [f32; 4]) -> f32 {
    (1.0 / result[1]) * result[0]
}

#[test]
fn a_triangle_gives_its_distance_its_side_and_two_weights() {
    let (arena, base, root) = lone([[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    // From the side its corners run counter-clockwise on, the denominator is positive.
    let front = shoot(&arena, base, root, [0.25, 0.5, 1.0], [0.0, 0.0, -2.0]);
    assert_eq!(front, [0.5, 1.0, 0.25, 0.5]);
    let back = shoot(&arena, base, root, [0.25, 0.5, -1.0], [0.0, 0.0, 2.0]);
    assert_eq!(back, [-0.5, -1.0, -0.25, -0.5]);
    // An edge and a corner are the triangle's; past them is a miss, and so is a ray in its plane.
    assert_eq!(
        distance(shoot(&arena, base, root, [0.0, 0.5, 1.0], [0.0, 0.0, -2.0])),
        0.5
    );
    assert_eq!(
        distance(shoot(&arena, base, root, [0.0, 0.0, 1.0], [0.0, 0.0, -2.0])),
        0.5
    );
    let miss = MISS.map(f32::from_bits);
    assert_eq!(
        shoot(&arena, base, root, [0.75, 0.5, 1.0], [0.0, 0.0, -2.0]),
        miss
    );
    assert_eq!(
        shoot(&arena, base, root, [-1.0, 0.25, 0.0], [1.0, 0.0, 0.0]),
        miss
    );
    assert_eq!(shoot(&arena, base, root, [0.25, 0.5, 1.0], [0.0; 3]), miss);
    // A hit at the origin is one, at no distance, and one behind it is one at a distance below
    // nought: the walk is what holds a distance to an extent.
    assert_eq!(
        distance(shoot(
            &arena,
            base,
            root,
            [0.25, 0.5, 0.0],
            [0.0, 0.0, -2.0]
        )),
        0.0
    );
    assert_eq!(
        distance(shoot(
            &arena,
            base,
            root,
            [0.25, 0.5, -1.0],
            [0.0, 0.0, -2.0]
        )),
        -0.5
    );
    assert_eq!(distance(miss), INFINITE);
}

#[test]
fn a_pair_gives_each_triangle_its_own_side_and_its_own_corners_weights() {
    // The second shares the first's first edge, so the builder turns both to lie in one node.
    let p = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.5],
        [0.0, 2.0, 0.25],
        [2.0, -2.0, 1.0f32],
    ];
    let triangles = [[p[0], p[1], p[2]], [p[1], p[0], p[3]]].map(|corners| Triangle {
        corners,
        geometry: 3,
        primitive: 7,
        opaque: false,
    });
    let mut arena = Arena::new();
    let bottom = arena.bottom_level(&triangles).expect("a bottom level");
    assert_eq!(bottom.root & 7, 0);
    let tail = word(
        arena.words(),
        bottom.base + offset(bottom.root),
        TRIANGLE_WORD,
    );
    assert_eq!(tail, (3 << 8) | TRIANGLE_PAIR);
    for (k, triangle) in triangles.iter().enumerate() {
        let [a, b, c] = triangle.corners;
        let (wb, wc) = (0.25f32, 0.5f32);
        let on: [f32; 3] = std::array::from_fn(|i| a[i] + wb * (b[i] - a[i]) + wc * (c[i] - a[i]));
        // Towards the triangle against its normal: the front.
        let (ab, ac): ([f32; 3], [f32; 3]) = (
            std::array::from_fn(|i| b[i] - a[i]),
            std::array::from_fn(|i| c[i] - a[i]),
        );
        let normal = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let origin: [f32; 3] = std::array::from_fn(|i| on[i] + normal[i]);
        let direction = normal.map(|x| -2.0 * x);
        let result = shoot(
            &arena,
            bottom.base,
            bottom.root | k as u32,
            origin,
            direction,
        );
        let [_, denominator, first, second] = result;
        assert!(denominator > 0.0, "triangle {k} keeps its winding");
        let near = |x: f32, y: f32| (x - y).abs() < 1e-5;
        let weights = (first / denominator, second / denominator);
        assert!(near(distance(result), 0.5) && near(weights.0, wb) && near(weights.1, wc));
    }
}

/// A float from `10^low` to `10^high`.
fn power(sequence: &mut Sequence, low: f32, high: f32) -> f32 {
    // No function of a platform's library, whose last place differs from one to the next: the
    // counts below are of this sequence.
    let bits = (low + sequence.next() * (high - low)) * std::f32::consts::LOG2_10;
    let whole = bits.floor();
    (2f64.powi(whole as i32) * f64::from(1.0 + (bits - whole))) as f32
}

type Aimed = ([[f32; 3]; 3], [f32; 3], [f32; 3]);

/// A triangle of about `size` across, and the origin and direction of a ray aimed at a point of
/// it from about `away` times that far, the direction the way there times `shrink`. Where `thin`
/// is given the third corner lies that share of the size off the line of the other two.
fn aimed(sequence: &mut Sequence, size: f32, away: f32, shrink: f32, thin: Option<f32>) -> Aimed {
    let mut corners: [[f32; 3]; 3] =
        std::array::from_fn(|_| std::array::from_fn(|_| sequence.within(size)));
    if let Some(thin) = thin {
        let along = sequence.next();
        let [a, b, _] = corners;
        corners[2] =
            std::array::from_fn(|i| a[i] + along * (b[i] - a[i]) + sequence.within(thin * size));
    }
    let (wb, wc) = (sequence.next() * 0.5, sequence.next() * 0.5);
    let [a, b, c] = corners;
    let on: [f32; 3] = std::array::from_fn(|i| a[i] + wb * (b[i] - a[i]) + wc * (c[i] - a[i]));
    let origin: [f32; 3] = std::array::from_fn(|i| on[i] + sequence.within(away * size));
    let direction = std::array::from_fn(|i| (on[i] - origin[i]) * shrink);
    (corners, origin, direction)
}

#[test]
fn no_result_gives_a_quotient_that_is_not_a_number() {
    let mut sequence = Sequence(99);
    let (mut hits, rays) = (0, 20_000);
    for _ in 0..rays {
        // A triangle anywhere from far below a unit to near the greatest float, and a direction
        // anywhere from a unit to far too short for its distance to be stated.
        let size = power(&mut sequence, -30.0, 30.0);
        let (away, shrink) = (
            power(&mut sequence, -3.0, 3.0),
            power(&mut sequence, -44.0, 0.0),
        );
        let (corners, origin, mut direction) = aimed(&mut sequence, size, away, shrink, None);
        if sequence.next() < 0.2 {
            direction[(sequence.next() * 3.0) as usize % 3] = 0.0;
        }
        let (arena, base, root) = lone(corners);
        let result = shoot(&arena, base, root, origin, direction);
        if result.map(f32::to_bits) == MISS {
            continue;
        }
        hits += 1;
        let [_, denominator, first, second] = result;
        assert!(denominator.abs() >= MIN && denominator.abs() <= DET_MAX);
        let reciprocal = 1.0 / denominator;
        // A distance may be an infinity, which no extent admits; a weight is a share.
        assert!(!distance(result).is_nan(), "a ray along {direction:?}");
        assert!((reciprocal * first).abs() <= 1.001 && (reciprocal * second).abs() <= 1.001);
    }
    println!("{hits} results that are no miss checked, of {rays} rays");
    assert_eq!(hits, 5_961, "of {rays} rays");
}

#[test]
fn a_box_of_a_triangle_is_met_no_later_than_the_triangle_is_hit() {
    let mut sequence = Sequence(41);
    let (mut hits, rays) = (0, 20_000);
    for _ in 0..rays {
        // Sizes and directions whose working values stay clear of the least floats; every
        // other triangle thin, where a distance is a quotient of two small numbers.
        let size = power(&mut sequence, -15.0, 15.0);
        let (away, shrink) = (
            power(&mut sequence, -3.0, 3.0),
            power(&mut sequence, -6.0, 0.0),
        );
        let thin = (sequence.next() < 0.5).then(|| power(&mut sequence, -7.0, -3.0));
        let (corners, origin, direction) = aimed(&mut sequence, size, away, shrink, thin);
        let (arena, base, root) = lone(corners);
        let t = distance(shoot(&arena, base, root, origin, direction));
        if !t.is_finite() || t < 0.0 {
            continue;
        }
        hits += 1;
        let [a, b, c] = corners;
        let hull = |pick: fn(f32, f32) -> f32| -> [f32; 3] {
            std::array::from_fn(|i| pick(pick(a[i], b[i]), c[i]))
        };
        let holding = box_words(&[Some((hull(f32::min), hull(f32::max))), None, None, None]);
        let inverse = direction.map(|x| 1.0 / x);
        let met = intersect_ray(
            &holding,
            0,
            TYPE_BOX32,
            t.next_up(),
            origin,
            direction,
            inverse,
        );
        assert_eq!(
            met[0], 8,
            "the box of a triangle hit at {t} along {direction:?}"
        );
    }
    println!("{hits} hits checked, of {rays} rays");
    assert_eq!(hits, 17_795, "of {rays} rays");
}

#[test]
fn a_thin_triangle_that_points_at_a_ray_from_its_side_is_no_hit() {
    // Three corners all but in line with a point of the ray, a hundred units to its side and
    // farther: sheared, their edges' areas against the ray are nought or of one sign by chance.
    let mut sequence = Sequence(7);
    let (mut kept, mut tested, needles) = (0, 0, 40_000);
    for _ in 0..needles {
        let direction: [f32; 3] = std::array::from_fn(|_| sequence.within(2.0));
        let origin: [f32; 3] = std::array::from_fn(|_| sequence.within(50.0));
        let along = sequence.next();
        let through: [f32; 3] = std::array::from_fn(|k| origin[k] + along * direction[k]);
        let side: [f32; 3] = std::array::from_fn(|_| sequence.within(2.0));
        let width = sequence.next() * 1e-4;
        let corners: [[f32; 3]; 3] = std::array::from_fn(|n| {
            let out = 100.0 * (1.0 + 0.35 * n as f32);
            std::array::from_fn(|k| through[k] + out * side[k] + sequence.within(width))
        });
        // Only needles the ray's whole line stays well clear of.
        let reach = side.iter().map(|x| x * x).sum::<f32>().sqrt();
        let lean = (0..3).map(|k| side[k] * direction[k]).sum::<f32>().abs();
        let length = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
        if reach < 0.1 || lean > 0.5 * reach * length {
            continue;
        }
        tested += 1;
        let (arena, base, root) = lone(corners);
        let result = shoot(&arena, base, root, origin, direction);
        kept += usize::from(result.map(f32::to_bits) != MISS);
    }
    println!("{tested} needles tested, of {needles} made");
    assert_eq!(tested, 20_366, "needles that stay clear of the ray");
    assert_eq!(kept, 0, "needles kept of the {tested} tested");
}
