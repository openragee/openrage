//! Sets of numbered facts, one a block, and the fixed point a pass takes of them along the
//! edges of a function's blocks.

use crate::gta6::script::cfg::{Cfg, ENTRY};

/// The words one table of sets may take; a function that would need more is left as it is.
// ponytail: a dense set a block; sparse sets would let in the few functions this turns away.
const WORDS: usize = 1 << 20;

/// The sweeps over the blocks a fixed point is given: one more than the deepest nest of loops needs.
const SWEEPS: usize = 64;

/// A set of facts, each known by its number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::gta6::script::passes) struct Set(Vec<u64>);

impl Set {
    /// The empty set over `facts` facts.
    pub fn new(facts: usize) -> Set {
        Set(vec![0; facts.div_ceil(64)])
    }

    /// Every one of `facts` facts.
    pub fn full(facts: usize) -> Set {
        Set(vec![u64::MAX; facts.div_ceil(64)])
    }

    pub fn has(&self, fact: usize) -> bool {
        (self.0.get(fact / 64)).is_some_and(|word| (*word >> (fact % 64)) & 1 == 1)
    }

    pub fn insert(&mut self, fact: usize) {
        if let Some(word) = self.0.get_mut(fact / 64) {
            *word |= 1u64 << (fact % 64);
        }
    }

    pub fn remove(&mut self, fact: usize) {
        if let Some(word) = self.0.get_mut(fact / 64) {
            *word &= !(1u64 << (fact % 64));
        }
    }

    /// Adds every fact of `other`.
    pub fn add(&mut self, other: &Set) {
        for (word, more) in self.0.iter_mut().zip(&other.0) {
            *word |= more;
        }
    }

    /// Keeps only the facts `other` holds too.
    pub fn keep(&mut self, other: &Set) {
        for (word, kept) in self.0.iter_mut().zip(&other.0) {
            *word &= kept;
        }
    }

    /// Removes every fact of `other`.
    pub fn without(&mut self, other: &Set) {
        for (word, gone) in self.0.iter_mut().zip(&other.0) {
            *word &= !gone;
        }
    }
}

/// Whether a set of `facts` facts for every block of `cfg` is within what a function is given.
pub(in crate::gta6::script::passes) fn fits(cfg: &Cfg, facts: usize) -> bool {
    cfg.blocks.len().saturating_mul(facts.div_ceil(64)) <= WORDS
}

/// The facts that hold on entry to each block, from those each block makes hold (`made`) and those it ends (`ended`).
// With `every`, a fact holds on entry only where it holds at the end of every block that leads
// in, and none holds where the function starts. Without, it holds where it holds at the end of
// any, and `start` holds where the function starts.
// ⚠ `None` where the sweeps ran out: sets taken before the fixed point claim what is not so.
pub(in crate::gta6::script::passes) fn entering(
    cfg: &Cfg,
    facts: usize,
    (made, ended): (&[Set], &[Set]),
    start: &Set,
    every: bool,
) -> Option<Vec<Set>> {
    let n = cfg.blocks.len();
    if made.len() != n || ended.len() != n {
        return None;
    }
    let order = cfg.reverse_post_order();
    let preds = cfg.predecessors();
    let top = match every {
        true => Set::full(facts),
        false => Set::new(facts),
    };
    let mut ins = vec![Set::new(facts); n];
    let mut outs = vec![top.clone(); n];
    for _ in 0..SWEEPS {
        let mut changed = false;
        for b in order.iter().copied() {
            let entry = match (b == ENTRY, every) {
                (true, true) => Set::new(facts),
                (first, _) => {
                    let mut entry = match first {
                        true => start.clone(),
                        false => top.clone(),
                    };
                    for p in &preds[b] {
                        match every {
                            true => entry.keep(&outs[*p]),
                            false => entry.add(&outs[*p]),
                        }
                    }
                    entry
                }
            };
            let mut exit = entry.clone();
            exit.without(&ended[b]);
            exit.add(&made[b]);
            if exit != outs[b] {
                outs[b] = exit;
                changed = true;
            }
            ins[b] = entry;
        }
        if !changed {
            return Some(ins);
        }
    }
    None
}
