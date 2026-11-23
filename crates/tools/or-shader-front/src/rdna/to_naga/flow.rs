//! A program's control flow as nested ifs and loops, for any graph whose loops are each entered
//! at one block. A branch past the construct that follows notes where it goes, and the code
//! between is kept from running; a branch out of several loops counts the loops left to leave.

use super::End;

/// One statement of the structured form; a block is named by its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Flow {
    /// A block's own statements.
    Code(u32),
    /// The two arms of the branch block `cond` ends on.
    If {
        cond: u32,
        then: Vec<Flow>,
        otherwise: Vec<Flow>,
    },
    Loop(Vec<Flow>),
    Break,
    Continue,
    Return,
    /// Control is on its way to this block: the code before it is passed over.
    Leave(u32),
    /// Control that was on its way to this block has arrived.
    Arrive(u32),
    /// Runs only while control is on its way nowhere.
    Guard(Vec<Flow>),
    /// This many more loops are to be left, past the one the `Break` after this leaves.
    Unwind(u32),
    /// After a loop: leaves the loop around it too, while any is left to leave.
    Unwound,
    /// The loop to run again is not the innermost: the `Break` after this leaves towards it.
    Again,
    /// After a loop: runs the loop around it again where a branch inside asked to.
    Resume,
    /// Statements moved to a function of their own, by its number, so that no function nests
    /// deeper than whoever reads the module is sure to follow.
    Call(usize, Vec<Flow>),
}

/// Why a graph has no structured form here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unstructured {
    /// A loop entered at more than one block.
    Irreducible,
    /// A nest deeper than the walk follows, or a block the walk cannot place.
    Shape,
}

/// The deepest nest the walk follows: it recurses a frame or two a level.
const MAX_WALK: usize = 96;

/// What encloses the point being emitted, innermost last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    If,
    Loop(u32),
    /// The code before merge block `0`, which falling off its end runs.
    Follow(u32),
}

struct Walk<'a> {
    ends: &'a [End],
    /// Each block's place in reverse post-order.
    rpo: Vec<usize>,
    header: Vec<bool>,
    /// Two or more forward predecessors.
    merge: Vec<bool>,
    /// The merge blocks each block immediately dominates, in reverse post-order.
    merges: Vec<Vec<u32>>,
    /// The merge blocks some branch passes over, and those some branch leaves for.
    guarded: Vec<bool>,
    arrives: Vec<bool>,
    /// The loops a branch leaves on its way out of another, and those one continues past.
    unwound: Vec<bool>,
    resumed: Vec<bool>,
    frames: Vec<Frame>,
    depth: usize,
}

fn successors(end: End) -> Vec<u32> {
    match end {
        End::Return => Vec::new(),
        End::Jump(to) => vec![to],
        End::Branch { then, otherwise } => vec![then, otherwise],
    }
}

fn intersect(idom: &[usize], mut a: usize, mut b: usize) -> usize {
    while a != b {
        while a > b {
            a = idom[a];
        }
        while b > a {
            b = idom[b];
        }
    }
    a
}

/// Whether place `a` dominates place `b`.
fn dominates(idom: &[usize], a: usize, mut b: usize) -> bool {
    while b > a {
        b = idom[b];
    }
    b == a
}

/// The structured form of the graph `ends` states, entered at block 0.
pub(super) fn structure(ends: &[End]) -> Result<Vec<Flow>, Unstructured> {
    let mut walk = Walk::new(ends)?;
    let mut out = Vec::new();
    walk.tree(0, &mut out)?;
    Ok(out)
}

impl<'a> Walk<'a> {
    fn new(ends: &'a [End]) -> Result<Self, Unstructured> {
        let count = ends.len();
        if count == 0 {
            return Err(Unstructured::Shape);
        }
        let next: Vec<Vec<u32>> = ends.iter().map(|&end| successors(end)).collect();
        let mut post = Vec::with_capacity(count);
        let mut seen = vec![false; count];
        seen[0] = true;
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        while let Some(top) = stack.last_mut() {
            let (block, nth) = *top;
            match next[block].get(nth) {
                Some(&to) => {
                    top.1 += 1;
                    let target = to as usize;
                    if target >= count {
                        return Err(Unstructured::Shape);
                    }
                    if !seen[target] {
                        seen[target] = true;
                        stack.push((target, 0));
                    }
                }
                None => {
                    post.push(block);
                    stack.pop();
                }
            }
        }
        let order: Vec<usize> = post.into_iter().rev().collect();
        let mut rpo = vec![usize::MAX; count];
        for (place, &block) in order.iter().enumerate() {
            rpo[block] = place;
        }
        let mut preds = vec![Vec::new(); count];
        for &block in &order {
            for &to in &next[block] {
                preds[to as usize].push(rpo[block]);
            }
        }
        // Cooper, Harvey and Kennedy's iteration, over reverse post-order places.
        let mut idom = vec![usize::MAX; order.len()];
        idom[0] = 0;
        let mut changed = true;
        while changed {
            changed = false;
            for (place, &block) in order.iter().enumerate().skip(1) {
                let mut new = usize::MAX;
                for &pred in &preds[block] {
                    if idom[pred] != usize::MAX {
                        new = match new {
                            usize::MAX => pred,
                            _ => intersect(&idom, pred, new),
                        };
                    }
                }
                if idom[place] != new {
                    idom[place] = new;
                    changed = true;
                }
            }
        }
        let mut header = vec![false; count];
        let mut forward = vec![0u32; count];
        for &block in &order {
            for &to in &next[block] {
                let (from, target) = (rpo[block], rpo[to as usize]);
                if target <= from {
                    if !dominates(&idom, target, from) {
                        return Err(Unstructured::Irreducible);
                    }
                    header[to as usize] = true;
                } else {
                    forward[to as usize] += 1;
                }
            }
        }
        let merge: Vec<bool> = forward.iter().map(|&n| n >= 2).collect();
        let mut merges = vec![Vec::new(); count];
        for (place, &block) in order.iter().enumerate().skip(1) {
            if merge[block] {
                merges[order[idom[place]]].push(block as u32);
            }
        }
        Ok(Self {
            ends,
            rpo,
            header,
            merge,
            merges,
            guarded: vec![false; count],
            arrives: vec![false; count],
            unwound: vec![false; count],
            resumed: vec![false; count],
            frames: Vec::new(),
            depth: 0,
        })
    }

    /// Emits merge block `merge` where the code before it ends: behind what says control
    /// arrived, and kept from running where control passes over it. `None` once it is emitted
    /// whole, else the block for the caller to go on with.
    fn merged(&mut self, merge: u32, out: &mut Vec<Flow>) -> Result<Option<u32>, Unstructured> {
        if self.arrives[merge as usize] {
            out.push(Flow::Arrive(merge));
        }
        if !self.guarded[merge as usize] {
            return Ok(Some(merge));
        }
        let mut inner = Vec::new();
        self.tree(merge, &mut inner)?;
        out.push(Flow::Guard(inner));
        Ok(None)
    }

    /// Emits `block` and what it dominates; a loop header as a loop.
    fn tree(&mut self, mut block: u32, out: &mut Vec<Flow>) -> Result<(), Unstructured> {
        self.depth += 1;
        if self.depth > MAX_WALK {
            return Err(Unstructured::Shape);
        }
        loop {
            if self.header[block as usize] {
                self.frames.push(Frame::Loop(block));
                let mut body = Vec::new();
                if let Some(last) = self.within(block, &mut body)? {
                    if let Some(last) = self.merged(last, &mut body)? {
                        self.tree(last, &mut body)?;
                    }
                }
                self.frames.pop();
                out.push(Flow::Loop(body));
                if self.unwound[block as usize] {
                    out.push(Flow::Unwound);
                }
                if self.resumed[block as usize] {
                    out.push(Flow::Resume);
                }
                break;
            }
            // The last merge block follows everything else here: emitted in this loop, not nested.
            let Some(last) = self.within(block, out)? else {
                break;
            };
            match self.merged(last, out)? {
                Some(last) => block = last,
                None => break,
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// Emits `block`, then each merge block it immediately dominates in reverse post-order, each
    /// after the code that reaches it; the last is returned for the caller to emit next.
    fn within(&mut self, block: u32, out: &mut Vec<Flow>) -> Result<Option<u32>, Unstructured> {
        let merges = std::mem::take(&mut self.merges[block as usize]);
        for &merge in merges.iter().rev() {
            self.frames.push(Frame::Follow(merge));
        }
        self.leaf(block, out)?;
        for (n, &merge) in merges.iter().enumerate() {
            self.frames.pop();
            if n + 1 == merges.len() {
                return Ok(Some(merge));
            }
            if let Some(merge) = self.merged(merge, out)? {
                self.tree(merge, out)?;
            }
        }
        Ok(None)
    }

    /// Emits `block`'s code and how it hands control on.
    fn leaf(&mut self, block: u32, out: &mut Vec<Flow>) -> Result<(), Unstructured> {
        out.push(Flow::Code(block));
        match self.ends[block as usize] {
            End::Return => out.push(Flow::Return),
            End::Jump(to) => self.branch(block, to, out)?,
            End::Branch { then, otherwise } if then == otherwise => {
                self.branch(block, then, out)?;
            }
            End::Branch { then, otherwise } => {
                self.frames.push(Frame::If);
                let (mut taken, mut passed) = (Vec::new(), Vec::new());
                self.branch(block, then, &mut taken)?;
                self.branch(block, otherwise, &mut passed)?;
                self.frames.pop();
                out.push(Flow::If {
                    cond: block,
                    then: taken,
                    otherwise: passed,
                });
            }
        }
        Ok(())
    }

    /// Emits the branch from `from` to `to`: the loop it runs again, the merge block it falls
    /// or leaves for, or `to` itself where `from` is its only forward predecessor.
    fn branch(&mut self, from: u32, to: u32, out: &mut Vec<Flow>) -> Result<(), Unstructured> {
        let target = to as usize;
        if self.rpo[target] <= self.rpo[from as usize] {
            // Back to a loop's first block: of the innermost loop at once, of one around it
            // by leaving every loop between.
            let mut between = Vec::new();
            for frame in self.frames.iter().rev() {
                match *frame {
                    Frame::Loop(header) if header == to => {
                        let Some((&outermost, inner)) = between.split_last() else {
                            out.push(Flow::Continue);
                            return Ok(());
                        };
                        out.push(Flow::Again);
                        self.leave(inner, out);
                        self.resumed[outermost as usize] = true;
                        return Ok(());
                    }
                    Frame::Loop(header) => between.push(header),
                    _ => {}
                }
            }
            return Err(Unstructured::Shape);
        }
        if !self.merge[target] {
            return self.tree(to, out);
        }
        let (mut passed, mut loops) = (Vec::new(), Vec::new());
        for frame in self.frames.iter().rev() {
            match *frame {
                Frame::If => {}
                Frame::Loop(header) => loops.push(header),
                Frame::Follow(merge) if merge == to => {
                    if !passed.is_empty() {
                        out.push(Flow::Leave(to));
                        self.arrives[target] = true;
                        for over in passed {
                            self.guarded[over as usize] = true;
                        }
                    }
                    if let Some((_, inner)) = loops.split_last() {
                        self.leave(inner, out);
                    }
                    return Ok(());
                }
                Frame::Follow(merge) => passed.push(merge),
            }
        }
        Err(Unstructured::Shape)
    }

    /// Leaves the innermost loop, and the loops of `inner` on the way out of it: each says
    /// after itself that the loop around it is left too.
    fn leave(&mut self, inner: &[u32], out: &mut Vec<Flow>) {
        if !inner.is_empty() {
            out.push(Flow::Unwind(inner.len() as u32));
        }
        for &header in inner {
            self.unwound[header as usize] = true;
        }
        out.push(Flow::Break);
    }
}

/// How deep the ifs and loops of `nodes` nest within one function.
pub(super) fn nesting(nodes: &[Flow]) -> usize {
    (nodes.iter())
        .map(|node| match node {
            Flow::If {
                then, otherwise, ..
            } => 1 + nesting(then).max(nesting(otherwise)),
            Flow::Loop(body) | Flow::Guard(body) => 1 + nesting(body),
            // What follows a loop to leave the next is an if of its own.
            Flow::Unwound | Flow::Resume | Flow::Arrive(_) | Flow::Call(..) => 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// `nodes` with every nest deeper than `limit` moved to a function of its own, numbered from
/// `next` on; a function's own nest counts from nothing.
pub(super) fn outlined(
    nodes: Vec<Flow>,
    limit: usize,
    depth: usize,
    next: &mut usize,
) -> Vec<Flow> {
    let inside = |nodes: Vec<Flow>, next: &mut usize| match depth + 1 >= limit {
        // Nothing nests here: the statements stay where they are.
        _ if nesting(&nodes) == 0 => nodes,
        true => {
            let number = *next;
            *next += 1;
            vec![Flow::Call(number, outlined(nodes, limit, 0, next))]
        }
        false => outlined(nodes, limit, depth + 1, next),
    };
    (nodes.into_iter())
        .map(|node| match node {
            Flow::If {
                cond,
                then,
                otherwise,
            } => Flow::If {
                cond,
                then: inside(then, next),
                otherwise: inside(otherwise, next),
            },
            Flow::Loop(body) => Flow::Loop(inside(body, next)),
            Flow::Guard(body) => Flow::Guard(inside(body, next)),
            other => other,
        })
        .collect()
}

/// The function each block's statements are lowered in, by the block's number: 0 where no
/// `Call` holds it.
pub(super) fn homes(nodes: &[Flow], home: usize, into: &mut [usize]) {
    for node in nodes {
        match node {
            Flow::Code(block) => {
                if let Some(slot) = into.get_mut(*block as usize) {
                    *slot = home;
                }
            }
            Flow::If {
                then, otherwise, ..
            } => {
                homes(then, home, into);
                homes(otherwise, home, into);
            }
            Flow::Loop(body) | Flow::Guard(body) => homes(body, home, into),
            Flow::Call(number, body) => homes(body, *number, into),
            _ => {}
        }
    }
}
