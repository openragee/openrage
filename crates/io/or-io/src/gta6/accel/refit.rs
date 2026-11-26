//! A posed model's structures rewritten where they are: a bottom level's triangles from the posed
//! positions and its boxes from the bottom up, a top level's instance nodes and its boxes from
//! the instances as they are now.
//!
//! The tree is kept: no node moves and no pointer, parent or instance range changes, so a
//! structure's header and parent table stay as they were and only its nodes are written. A box
//! then holds its children to the bit, as a built one does, but it is the tree of the positions
//! the structure was built from, and a walk takes more turns over it as the pose leaves them.

use std::collections::HashMap;
use std::ops::Range;

use super::build::{instance_node, AccelError, Arena, Instance, Structure, Triangle};
use super::layout::*;

type Bounds = ([f32; 3], [f32; 3]);
/// What rewrites a node that is no box and gives its bounds.
type Leaf<'a> = dyn FnMut(&mut [u32], u32) -> Result<Bounds, AccelError> + 'a;

/// The bounds of the node `node` of the structure at `base` after its subtree is rewritten:
/// `leaf` rewrites a node that is no box and gives its bounds, and each box states them.
fn rewrite(
    words: &mut [u32],
    base: u32,
    node: u32,
    leaf: &mut Leaf<'_>,
) -> Result<Bounds, AccelError> {
    if !is_box(node) {
        return leaf(words, node);
    }
    let at = ((base + offset(node)) / 4) as usize;
    let (mut low, mut high) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for i in 0..4 {
        let child = words[at + i];
        if child == INVALID {
            continue;
        }
        let (min, max) = rewrite(words, base, child, leaf)?;
        let bounds = at + ((BOX_BOUNDS + BOX_BOUNDS_STRIDE * i as u32) / 4) as usize;
        for k in 0..3 {
            words[bounds + k] = min[k].to_bits();
            words[bounds + 3 + k] = max[k].to_bits();
            low[k] = low[k].min(min[k]);
            high[k] = high[k].max(max[k]);
        }
    }
    Ok((low, high))
}

impl Arena {
    /// The bytes of `structure` a refit writes: its nodes, from the first to the parent table.
    pub fn node_bytes(&self, structure: &Structure) -> Range<u32> {
        let table = self.words[((structure.base + HEADER_PARENTS) / 4) as usize];
        structure.base + HEADER_BYTES..structure.base + table
    }

    /// Rewrites the bottom level `bottom` from `triangles`, the ones it was built from with
    /// their corners where the pose puts them, in any order: the structure with its new bounds.
    ///
    /// # Errors
    ///
    /// Where a triangle of the structure is not among `triangles`, or two that share a node no
    /// longer share their edge. The structure is then part rewritten, and is to be built anew.
    pub fn refit_bottom(
        &mut self,
        bottom: &Structure,
        triangles: &[Triangle],
    ) -> Result<Structure, AccelError> {
        let mut posed = HashMap::new();
        for (index, triangle) in triangles.iter().enumerate() {
            if !triangle.corners.iter().flatten().all(|x| x.is_finite()) {
                return Err(AccelError::NotFinite(index));
            }
            posed.insert((triangle.geometry, triangle.primitive), triangle.corners);
        }
        let base = bottom.base;
        let mut leaf = |words: &mut [u32], node: u32| -> Result<Bounds, AccelError> {
            let at = ((base + offset(node)) / 4) as usize;
            let tail = words[at + (TRIANGLE_WORD / 4) as usize];
            let pair = tail & TRIANGLE_PAIR != 0;
            // A corner two triangles share is written by both, and they must agree on it.
            let mut vertices = [None; 4];
            for k in 0..if pair { 2 } else { 1 } {
                let primitive = words[at + (TRIANGLE_PRIMITIVE / 4) as usize + k];
                let corners = posed
                    .get(&(tail >> 8, primitive))
                    .ok_or(AccelError::Unposed(tail >> 8, primitive))?;
                let id = words[at + (TRIANGLE_ID / 4) as usize] >> (8 * k);
                // The places in the node's triangle of the model's three corners.
                let (second, third) = (id & 3, (id >> 2) & 3);
                for (corner, place) in [3 - second - third, second, third].into_iter().enumerate() {
                    let vertex = CORNERS[k][place as usize] as usize;
                    let bits = corners[corner].map(f32::to_bits);
                    if vertices[vertex].is_some_and(|other| other != bits) {
                        return Err(AccelError::Torn(node));
                    }
                    vertices[vertex] = Some(bits);
                }
            }
            let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for (vertex, bits) in vertices.into_iter().enumerate() {
                let Some(bits) = bits else { continue };
                for k in 0..3 {
                    words[at + 3 * vertex + k] = bits[k];
                    min[k] = min[k].min(f32::from_bits(bits[k]));
                    max[k] = max[k].max(f32::from_bits(bits[k]));
                }
            }
            Ok((min, max))
        };
        let (min, max) = rewrite(&mut self.words, base, bottom.root, &mut leaf)?;
        Ok(Structure {
            min,
            max,
            ..*bottom
        })
    }

    /// Rewrites the top level `top` from `instances`, the list it was built from with each
    /// instance's transform, flags and bottom level as they are now: the structure with its new
    /// bounds.
    ///
    /// # Errors
    ///
    /// Where the list is shorter than the one the structure was built from, or a transform has
    /// no inverse. The structure is then part rewritten, and is to be built anew.
    pub fn refit_top(
        &mut self,
        top: &Structure,
        instances: &[Instance],
    ) -> Result<Structure, AccelError> {
        let base = top.base;
        let mut leaf = |words: &mut [u32], node: u32| -> Result<Bounds, AccelError> {
            let at = ((base + offset(node)) / 4) as usize;
            let index = (words[at + (INSTANCE_INDEX / 4) as usize] & 0xFFFF) as usize;
            let instance = instances.get(index).ok_or(AccelError::Unplaced(index))?;
            let (mut stated, min, max) = instance_node(instance, index)?;
            // The node stays where it is, under the parent it has.
            let parent = (INSTANCE_PARENT / 4) as usize;
            stated[parent] = words[at + parent];
            words[at..at + stated.len()].copy_from_slice(&stated);
            Ok((min, max))
        };
        let (min, max) = rewrite(&mut self.words, base, top.root, &mut leaf)?;
        Ok(Structure { min, max, ..*top })
    }
}

#[cfg(test)]
#[path = "refit_tests.rs"]
mod tests;
