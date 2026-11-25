//! Which pieces of the bounds under a placed object are the object's own: a proposal, as data a person edits before anything is taken.
//!
//! [`associate`] reads the pieces of every bound of a [`CollisionSet`] that the object's box reaches, in the world with Z up and the
//! box grown by [`BOX_SLACK`] each way, and sorts them by this rule:
//!
//! 1. An object whose box stands lower than [`JUDGED_FROM`] is not judged: nothing is read and [`Association::not_judged`] says why.
//! 2. A piece wholly in the box is *held*. A piece partly in it is *cut*, and a cut triangle has a *share*: the area of its part in
//!    the box over its area.
//! 3. A piece *stands* when it is no triangle, or a triangle whose plane is more than [`GROUND_SLOPE`] from level; which way a
//!    triangle is wound is not asked. The *foot* is the lowest point a standing piece reaches inside the box.
//! 4. A triangle that does not stand is *ground* when the lowest point of its part in the box is no more than [`GROUND_STEP`] above
//!    the foot, or when nothing stands in the box: of what the box holds, nothing is under it but more ground. So the floor the walls
//!    stand on is ground, under the object and around it, and a roof is not, nor a floor of the object's own that its walls reach
//!    under by more than the step.
//! 5. A triangle that does not stand is ground too when its surface *goes on past the box*: it shares an edge, itself or through
//!    other such triangles the box reaches, with one the box cuts and holds less than [`MOST`] of, two corners being one within
//!    [`SHARED_CORNER`]. So a slope is ground on the high side of the object as on the low side, whatever the walls reach down to,
//!    and so is a deck that crosses the box; a roof and a floor that end inside the box are not.
//! 6. Ground is left, unless [`AssociationOptions::take_ground`] asks for it. Of the rest a held piece is proposed, a cut triangle is
//!    proposed when its share is at least [`MOST`] and left otherwise, and a cut piece that is no triangle is left.
//!
//! ⚠ It is a judgement, so it proposes and nothing here applies it. A flat roof that goes on at one height over the house next door
//! is left as ground; the foot is one height for the whole box, so the floor over a cellar the box holds is proposed, and so is
//! ground above the foot that shares no edge with the ground around the box; and a wall, a rail or a shape of something else that
//! the box holds whole is proposed, because it stands in the box. None of the six thresholds is measured on a game file.

use std::collections::HashMap;

use glam::Vec3;
use serde::Serialize;

use super::bound::obb_of;
use super::shape::{Obb, Shape};
use super::{CollisionSet, OrientedBox, Part};

/// How tall an object's box stands before its collision is looked for, in metres: lower than a door, an object is a post, a bin or a kerb stone, which collides through the bound of its own model and whose box holds little but ground.
pub const JUDGED_FROM: f32 = 2.0;
/// How far the object's box is grown each way, in metres: a bound stores a vertex in sixteen bits an axis, centimetres for a bound a kilometre wide, so a wall that stands on a face of its archetype's box is stored a little outside it.
pub const BOX_SLACK: f32 = 0.05;
/// The steepest plane that is ground, as its angle from level in radians: what a person walks up, past which a surface is a bank or a pitched wall.
pub const GROUND_SLOPE: f32 = std::f32::consts::FRAC_PI_4;
/// How far above the foot of the standing pieces a level surface is still ground, in metres: the rise of a kerb, so a pavement is the ground of the wall that stands in the gutter beside it, and the floor of a storey is not.
pub const GROUND_STEP: f32 = 0.25;
/// The share of a cut triangle's area that is most of it: a rectangle of two triangles cut across its middle has one of them three quarters in the box, which is the neighbour's wall as much as the other one is, so most is more than that.
pub const MOST: f32 = 0.9;
/// How far apart two triangles may hold one corner and still share it, in metres: two bounds store one place each in a quantum of its own, which is centimetres for a bound a kilometre wide.
pub const SHARED_CORNER: f32 = 0.01;

/// What is asked beyond the rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssociationOptions {
    /// Propose the ground too, for a person who digs where the object stood.
    pub take_ground: bool,
}

/// Why an object's collision was not looked for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum NotJudged {
    /// Its box stands `height` metres tall, under the `limit` of [`JUDGED_FROM`].
    TooLow { height: f32, limit: f32 },
    /// Its box has a size or a place that is no finite number.
    NoBox,
}

/// Why a piece the box reaches is not proposed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Reason {
    /// It is ground, by the rule of this module.
    Ground,
    /// A triangle the box cuts, with `share` of its area in the box, which is less than [`MOST`].
    MostlyOutside { share: f32 },
    /// A sphere, a capsule, a box or a cylinder the box cuts, whose share is not measured.
    CutShape,
}

/// A piece proposed as the object's own.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Proposed {
    /// The polygon of a geometry, by its place in `polygons`; none for a bound that is one shape.
    pub polygon: Option<u32>,
    /// The share of its area in the box, for a triangle the box cuts; none for a piece wholly in the box.
    pub share: Option<f32>,
}

/// A piece the box reaches that stays.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Left {
    /// The polygon of a geometry, by its place in `polygons`; none for a bound that is one shape.
    pub polygon: Option<u32>,
    /// Whether the box cuts it; false for a piece wholly in the box.
    pub cut: bool,
    #[serde(flatten)]
    pub reason: Reason,
}

/// The pieces of one leaf bound the box reaches, each list in the order the bound holds them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeafAssociation {
    /// The bound, counted depth first among those that are not composites, as [`Part::leaf`] counts.
    pub leaf: u32,
    pub proposed: Vec<Proposed>,
    pub left: Vec<Left>,
}

/// The pieces of one placed bound the box reaches, by leaf.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundAssociation<K> {
    pub key: K,
    pub leaves: Vec<LeafAssociation>,
}

/// What of the bounds under an object is proposed as the object's own, and what is left with its reason.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Association<K> {
    /// How tall the object's box stands, in metres.
    pub height: f32,
    /// Why nothing was looked for, when nothing was; `bounds` is then empty.
    pub not_judged: Option<NotJudged>,
    /// The bounds the box reaches a piece of, in key order.
    pub bounds: Vec<BoundAssociation<K>>,
}

impl<K> Association<K> {
    /// How many pieces are proposed, over every bound.
    pub fn proposed(&self) -> usize {
        self.leaves().map(|leaf| leaf.proposed.len()).sum()
    }

    /// How many pieces the box reaches and leaves, over every bound.
    pub fn left(&self) -> usize {
        self.leaves().map(|leaf| leaf.left.len()).sum()
    }

    fn leaves(&self) -> impl Iterator<Item = &LeafAssociation> {
        self.bounds.iter().flat_map(|bound| &bound.leaves)
    }
}

/// One piece the box reaches, as the rule sees it.
struct Piece {
    part: Part,
    /// None for a piece wholly in the box; for a cut one its share, where it is a triangle.
    cut: Option<Option<f32>>,
    stands: bool,
    /// The lowest point of its part in the box.
    low: f32,
    /// The corners of a triangle that does not stand, in the world.
    level: Option<[Vec3; 3]>,
    /// Whether its surface goes on past the box.
    goes_on: bool,
}

/// The part of a triangle in a box, as the corners of a convex polygon in the box's own frame.
fn clipped(corners: [Vec3; 3], q: &Obb) -> Vec<Vec3> {
    let mut polygon: Vec<Vec3> = corners.iter().map(|p| q.local(*p)).collect();
    for axis in 0..3 {
        for sign in [1.0f32, -1.0] {
            // How far a point is inside the face, negative past it.
            let depth = |p: Vec3| q.half[axis] - sign * p[axis];
            let mut kept = Vec::with_capacity(polygon.len() + 1);
            for (i, from) in polygon.iter().enumerate() {
                let to = polygon[(i + 1) % polygon.len()];
                let (a, b) = (depth(*from), depth(to));
                if a >= 0.0 {
                    kept.push(*from);
                }
                if (a >= 0.0) != (b >= 0.0) {
                    kept.push(from.lerp(to, a / (a - b)));
                }
            }
            polygon = kept;
            if polygon.is_empty() {
                return polygon;
            }
        }
    }
    polygon
}

/// The area of a convex polygon.
fn area(polygon: &[Vec3]) -> f32 {
    let Some((first, rest)) = polygon.split_first() else {
        return 0.0;
    };
    rest.windows(2)
        .map(|pair| (pair[0] - *first).cross(pair[1] - *first))
        .sum::<Vec3>()
        .length()
        * 0.5
}

fn piece(part: Part, shape: &Shape, q: &Obb) -> Option<Piece> {
    let held = shape.within(q);
    if !held && !shape.touches(q) {
        return None;
    }
    let lowest = shape.interval(Vec3::Z).0;
    let box_floor = Shape::Box(*q).interval(Vec3::Z).0;
    let Shape::Triangle { a, b, c } = *shape else {
        return Some(Piece {
            part,
            cut: (!held).then_some(None),
            stands: true,
            low: lowest.max(box_floor),
            level: None,
            goes_on: false,
        });
    };
    let normal = (b - a).cross(c - a);
    // A triangle of no area has no plane to be level.
    let stands = normal
        .try_normalize()
        .is_none_or(|unit| unit.z.abs() < GROUND_SLOPE.cos());
    let level = (!stands).then_some([a, b, c]);
    if held {
        return Some(Piece {
            part,
            cut: None,
            stands,
            low: lowest,
            level,
            goes_on: false,
        });
    }
    let inside = clipped([a, b, c], q);
    let whole = normal.length() * 0.5;
    let share = if whole > 0.0 {
        (area(&inside) / whole).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let low = inside
        .iter()
        .map(|p| (q.centre + q.axes[0] * p.x + q.axes[1] * p.y + q.axes[2] * p.z).z)
        .fold(f32::INFINITY, f32::min);
    Some(Piece {
        part,
        cut: Some(Some(share)),
        stands,
        // A triangle that only touches a face has no part in the box to be low.
        low: if low.is_finite() {
            low
        } else {
            lowest.max(box_floor)
        },
        level,
        goes_on: false,
    })
}

/// The cell of a grid of [`SHARED_CORNER`] a place is in.
fn cell(p: Vec3) -> [i64; 3] {
    (p / SHARED_CORNER).floor().to_array().map(|c| c as i64)
}

/// Marks every triangle that does not stand and whose surface goes on past the box, over all the bounds at once.
fn mark_what_goes_on<K>(reached: &mut [(K, Vec<Piece>)]) {
    let level: Vec<(usize, usize, [Vec3; 3])> = (reached.iter().enumerate())
        .flat_map(|(b, (_, pieces))| {
            (pieces.iter().enumerate()).filter_map(move |(i, piece)| Some((b, i, piece.level?)))
        })
        .collect();
    // Each corner is the first one stated within the tolerance of it.
    let mut stated: HashMap<[i64; 3], Vec<(Vec3, usize)>> = HashMap::new();
    let mut corners = 0;
    let mut corner = |p: Vec3| -> usize {
        let [x, y, z] = cell(p);
        let near =
            (-1..=1).flat_map(|i| (-1..=1).flat_map(move |j| (-1..=1).map(move |k| [i, j, k])));
        for [i, j, k] in near {
            let held = stated.get(&[x + i, y + j, z + k]).into_iter().flatten();
            if let Some((_, id)) = held
                .into_iter()
                .find(|(q, _)| p.distance(*q) <= SHARED_CORNER)
            {
                return *id;
            }
        }
        corners += 1;
        stated.entry([x, y, z]).or_default().push((p, corners - 1));
        corners - 1
    };
    // The triangles joined by the edges they share: each names the first of its surface found so far.
    let mut joined: Vec<usize> = (0..level.len()).collect();
    fn first(joined: &mut [usize], mut t: usize) -> usize {
        while joined[t] != t {
            joined[t] = joined[joined[t]];
            t = joined[t];
        }
        t
    }
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    for (t, (_, _, [a, b, c])) in level.iter().enumerate() {
        let [a, b, c] = [corner(*a), corner(*b), corner(*c)];
        for (from, to) in [(a, b), (b, c), (c, a)] {
            if from == to {
                continue;
            }
            let other = *edges.entry((from.min(to), from.max(to))).or_insert(t);
            let (one, two) = (first(&mut joined, t), first(&mut joined, other));
            joined[one] = two;
        }
    }
    let mut goes_on = vec![false; level.len()];
    for (t, (b, i, _)) in level.iter().enumerate() {
        if matches!(reached[*b].1[*i].cut, Some(Some(share)) if share < MOST) {
            goes_on[first(&mut joined, t)] = true;
        }
    }
    for (t, (b, i, _)) in level.iter().enumerate() {
        reached[*b].1[*i].goes_on = goes_on[first(&mut joined, t)];
    }
}

/// The pieces of the set's bounds proposed as the own collision of an object whose box is `object`, and those left, by the rule of this module.
pub fn associate<K: Ord + Clone>(
    set: &CollisionSet<K>,
    object: &OrientedBox,
    options: &AssociationOptions,
) -> Association<K> {
    let exact = obb_of(object);
    let (bottom, top) = Shape::Box(exact).interval(Vec3::Z);
    let height = top - bottom;
    let not_judged = if !(Shape::Box(exact).sound() && height.is_finite()) {
        Some(NotJudged::NoBox)
    } else if height < JUDGED_FROM {
        Some(NotJudged::TooLow {
            height,
            limit: JUDGED_FROM,
        })
    } else {
        None
    };
    if not_judged.is_some() {
        return Association {
            height,
            not_judged,
            bounds: Vec::new(),
        };
    }
    let q = Obb {
        half: exact.half + Vec3::splat(BOX_SLACK),
        ..exact
    };
    let mut reached: Vec<(K, Vec<Piece>)> = set
        .shapes_near(&q)
        .into_iter()
        .map(|(key, shapes)| {
            let pieces = shapes
                .iter()
                .filter_map(|(part, shape)| piece(*part, shape, &q))
                .collect();
            (key.clone(), pieces)
        })
        .collect();
    mark_what_goes_on(&mut reached);
    let foot = reached
        .iter()
        .flat_map(|(_, pieces)| pieces)
        .filter(|piece| piece.stands)
        .map(|piece| piece.low)
        .min_by(f32::total_cmp);
    let is_ground = |piece: &Piece| {
        !piece.stands && (piece.goes_on || foot.is_none_or(|foot| piece.low <= foot + GROUND_STEP))
    };

    let mut bounds = Vec::new();
    for (key, pieces) in reached {
        let mut leaves: Vec<LeafAssociation> = Vec::new();
        for piece in &pieces {
            if leaves
                .last()
                .is_none_or(|leaf| leaf.leaf != piece.part.leaf)
            {
                leaves.push(LeafAssociation {
                    leaf: piece.part.leaf,
                    proposed: Vec::new(),
                    left: Vec::new(),
                });
            }
            let Some(leaf) = leaves.last_mut() else {
                continue;
            };
            let polygon = piece.part.polygon;
            let reason = if is_ground(piece) && !options.take_ground {
                Some(Reason::Ground)
            } else {
                match piece.cut {
                    None => None,
                    Some(Some(share)) if share >= MOST => None,
                    Some(Some(share)) => Some(Reason::MostlyOutside { share }),
                    Some(None) => Some(Reason::CutShape),
                }
            };
            match reason {
                Some(reason) => leaf.left.push(Left {
                    polygon,
                    cut: piece.cut.is_some(),
                    reason,
                }),
                None => leaf.proposed.push(Proposed {
                    polygon,
                    share: piece.cut.flatten(),
                }),
            }
        }
        if !leaves.is_empty() {
            bounds.push(BoundAssociation { key, leaves });
        }
    }
    Association {
        height,
        not_judged,
        bounds,
    }
}
