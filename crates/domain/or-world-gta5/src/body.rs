//! A map and an interior of GTA V as bodies of the contract: the nodes `or_depends` reads, a
//! parent named by its map's row where the reader names it by the map's name, and beside each
//! node what it places.

use std::any::Any;
use std::sync::Arc;

use or_depends::world_forest::read::{self, Placed, ReadMap, What};
use or_depends::world_forest::{self, NoModel};
use or_world::contract::{MapBody, MapReader, Worker, WorldError};
use or_world::model::{
    interior_key, signed_fade, AssetKey, Drawn, Draws, Index, Instance, Instances, InteriorPlaced,
    Light, MapId, Moment, Occluders,
};
use or_world::streaming::{Node, ParentRef};

use crate::index::{linked, map_of};
use crate::install::Gta5World;
use crate::models::BOX;

/// How many words of its own GTA V gives an instance: the name of its archetype, then its level in the low byte and above it why it stands as a box.
pub const WORDS: usize = 2;
/// The second word's bits above the level for an instance whose entity names no archetype.
pub const BOXED_UNNAMED: u32 = 1 << 8;
/// The second word's bits above the level for an instance whose archetype no type file declares.
pub const BOXED_UNDECLARED: u32 = 2 << 8;

/// A map or an interior, read.
pub struct Body {
    nodes: Vec<Node>,
    placed: Vec<Placed>,
    interiors: Vec<InteriorPlaced>,
    /// Entities that state no level, or one that is none.
    pub unleveled: u32,
    /// Entities that name a parent in a parent map the mount lacks.
    pub links_lost: u32,
    bytes: u64,
    occluders: Occluders,
}

impl Body {
    /// `read` as a body, each parent's key turned by `key_of` from the reader's to the forest's.
    fn of(mut read: ReadMap, key_of: impl Fn(u64) -> Option<u64>) -> Self {
        let bytes = read.bytes();
        for node in &mut read.nodes {
            let Some(parent) = node.parent else { continue };
            node.parent = key_of(parent.map).map(|map| ParentRef { map, ..parent });
            if node.parent.is_none() {
                // What stood under an instance stands alone with its parent.
                node.instance = None;
                read.links_lost += 1;
            }
        }
        let interiors = (read.interiors.iter())
            .filter_map(|at| {
                Some(InteriorPlaced {
                    node: *at,
                    interior: u64::from(read.placed.get(*at as usize)?.archetype),
                })
            })
            .collect();
        Self {
            nodes: read.nodes,
            placed: read.placed,
            interiors,
            unleveled: read.unleveled,
            links_lost: read.links_lost,
            bytes,
            occluders: Occluders::default(),
        }
    }

    /// What node `at` places.
    pub fn placed(&self, at: u32) -> Option<&Placed> {
        self.placed.get(at as usize)
    }
}

/// What draws an entity that is `what`, named `archetype`.
fn draws_of(what: &What, archetype: u32) -> Draws {
    match what {
        What::Model(_) => Draws::Model(AssetKey(u64::from(archetype))),
        What::Box(_) => Draws::StandIn,
        What::Interior | What::Nothing => Draws::Nothing,
    }
}

impl MapBody for Body {
    fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    fn draws(&self, _at: &Moment, out: &mut Vec<Draws>) {
        let placed = self.placed.iter();
        out.extend(placed.map(|placed| draws_of(&placed.what, placed.archetype)));
    }

    fn lights(&self) -> &[Light] {
        &[]
    }

    fn occluders(&self) -> &Occluders {
        &self.occluders
    }

    fn interiors(&self) -> &[InteriorPlaced] {
        &self.interiors
    }

    fn lay(&self, drawn: &Drawn<'_>, _at: &Moment, out: &mut Instances) {
        for draw in drawn.nodes {
            let at = draw.node.1 as usize;
            let (Some(placed), Some(node)) = (self.placed.get(at), self.nodes.get(at)) else {
                continue;
            };
            let (model, boxed) = match placed.what {
                What::Model(_) => (AssetKey(u64::from(placed.archetype)), 0),
                What::Box(NoModel::Unnamed) => (BOX, BOXED_UNNAMED),
                What::Box(NoModel::Undeclared) => (BOX, BOXED_UNDECLARED),
                What::Interior | What::Nothing => continue,
            };
            let m = &placed.to_world.matrix3;
            let t = placed.to_world.translation;
            // ⚠ A box has no archetype to state a span, so its node's radius is 0: its sphere is
            // the one about the cube of side 1 as the entity places it, half the cube's diagonal.
            let radius = match boxed {
                0 => node.radius.max(0.0),
                _ => {
                    let squared = m.x_axis.length_squared()
                        + m.y_axis.length_squared()
                        + m.z_axis.length_squared();
                    squared.sqrt() / 2.0
                }
            };
            out.push(Instance {
                node: draw.node,
                model,
                rows: [
                    [m.x_axis.x, m.y_axis.x, m.z_axis.x, t.x],
                    [m.x_axis.y, m.y_axis.y, m.z_axis.y, t.y],
                    [m.x_axis.z, m.y_axis.z, m.z_axis.z, t.z],
                ],
                sphere: [t.x, t.y, t.z, radius],
                fade: signed_fade(draw.fade, draw.yields),
                words: &[placed.archetype, node.level as u32 | boxed],
            });
        }
    }

    fn bytes(&self) -> u64 {
        self.bytes
    }
}

fn unread(what: &str, why: impl std::fmt::Display) -> WorldError {
    WorldError::Unread {
        what: what.to_owned(),
        why: why.to_string(),
    }
}

impl MapReader for Gta5World {
    fn read(
        &self,
        index: &Index,
        map: MapId,
        _worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        let (linked, of) = map_of(index, map)?;
        let read =
            read::map(&linked.world, &self.source, of).map_err(|why| unread(&of.logical, why))?;
        // The reader names a map by the hash of its name, and the forest by its row.
        let key_of = |key: u64| {
            let hash = u32::try_from(key).ok()?;
            Some(linked.row_of(hash)?.key())
        };
        Ok(Arc::new(Body::of(read, key_of)))
    }

    fn interior(
        &self,
        index: &Index,
        map: MapId,
        body: &dyn MapBody,
        placed: &InteriorPlaced,
        _worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        let (linked, of) = map_of(index, map)?;
        let what = format!("the interior entity {} of {} places", placed.node, of.name);
        let body: &dyn Any = body;
        let body = (body.downcast_ref::<Body>())
            .ok_or_else(|| unread(&what, "the body that places it is another title's"))?;
        let at = placed.node as usize;
        let (Some(instance), Some(node)) = (body.placed.get(at), body.nodes.get(at)) else {
            return Err(unread(&what, "the body holds no such entity"));
        };
        // ⚠ As far as the instance itself draws under a view that scales no distance: an entity
        // of the interior whose archetype no type file declares draws that far, however the
        // camera that brought the interior in was set.
        let view = self.policy.view(
            glam::Vec3::ZERO,
            glam::Vec3::ZERO,
            std::f32::consts::FRAC_PI_4,
        );
        let reach = node.scaled(node.lod_dist, &view) + view.fade[node.level as usize];
        let read = read::interior(
            &linked.world,
            &self.source,
            instance.archetype,
            (of.hash, placed.node),
            (&instance.to_world, reach),
        )
        .map_err(|why| unread(&what, why))?;
        let (theirs, ours) = (
            world_forest::interior_key(of.hash, placed.node),
            interior_key(map, placed.node),
        );
        let key_of = |key: u64| match key {
            key if key == theirs => Some(ours),
            key if key == u64::from(of.hash) => Some(map.key()),
            _ => None,
        };
        Ok(Arc::new(Body::of(read, key_of)))
    }
}

/// The index GTA V's link made of `index`, for a caller that names a map by its hash.
pub(crate) fn world_of(index: &Index) -> Result<&or_depends::WorldIndex, WorldError> {
    Ok(&linked(index)?.world)
}
