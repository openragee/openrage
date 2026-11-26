//! A bare block, one that is no arm of an `if`, a `switch` or a loop, joined to the block that
//! holds it. It ran its statements in their order and so does its parent; what it ended was only
//! the scope of the expressions it emitted, which the passes after this one read across.

use naga::{Block, Statement as S};

/// Joins every bare block under `block` to its parent, in place and in order; how many it joined.
pub(super) fn splice(block: &mut Block) -> usize {
    let mut joined = 0;
    let mut out = Block::with_capacity(block.len());
    for (mut statement, span) in std::mem::take(block).span_into_iter() {
        match &mut statement {
            S::Block(inner) => {
                joined += 1 + splice(inner);
                out.append(inner);
                continue;
            }
            S::If { accept, reject, .. } => joined += splice(accept) + splice(reject),
            S::Switch { cases, .. } => {
                joined += (cases.iter_mut())
                    .map(|case| splice(&mut case.body))
                    .sum::<usize>();
            }
            S::Loop {
                body, continuing, ..
            } => joined += splice(body) + splice(continuing),
            _ => {}
        }
        out.push(statement, span);
    }
    *block = out;
    joined
}
