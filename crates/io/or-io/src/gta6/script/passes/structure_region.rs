//! Laid-out blocks to statements: one walk over the blocks in order, with a stack of the
//! constructs open around the block being written.
//!
//! A region is a run of consecutive blocks written as one list of statements; control that runs
//! off the end of the list goes to the region's `follow`. Every block is written once, in order,
//! and every jump out of it is stated: by falling to the block written next, by `break`,
//! `continue` or `return`, or by a `goto`. A construct only ever changes which of those states a
//! jump, so a shape nothing here knows still comes out as the jumps it is.
//!
//! The `gotos` rules choose three regions otherwise, each where the choice made without them
//! leaves a `goto`: an else arm from the jumps inside a then arm, a switch that leaves past its
//! region, a loop that takes in the tail one of its blocks leaves to. Being choices of region,
//! they change how the result reads and never what runs.

use std::collections::HashMap;

use crate::gta6::script::cfg::{Block, BlockId, Terminator};
use crate::gta6::script::ir::{self, Const, Expr, Var};
use crate::gta6::script::tree::{Case, Label, Stmt};

use super::cond;
use super::graph::{predecessors, SCAN};

/// Constructs open at once; past it a branch, a loop or a switch is written with jumps instead.
// The printer and every walk of the tree go as deep as the tree nests.
pub(super) const MAX_NEST: usize = 128;

/// The places past a then arm its blocks may jump to before no else arm is looked for among them.
const EXITS: usize = 8;

/// One body of a switch being written: the blocks `[start, end)` and the labels that lead to it.
struct Body {
    start: BlockId,
    end: BlockId,
    labels: Vec<Option<i64>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `while (cond)`, or `for` where the body ends in the step of a counter set just before.
    While,
    /// `for` whose step is a block of its own, which `continue` goes to.
    For,
    /// `do { } while (cond)`, the latch's test taken when the latch is reached.
    Do,
    /// `while (true)`.
    Forever,
}

struct Loop {
    mode: Mode,
    header: BlockId,
    latch: BlockId,
    /// The loop's test, for `While` and `For`.
    cond: Option<Expr>,
    /// The `continue` statements written for this loop.
    continues: u32,
    /// Whether nothing past the latch jumps to the header, so that no later `goto` can name it.
    closed: bool,
    /// Where the loop's test leaves to, where a jump must say so once the loop is written.
    exit: Option<BlockId>,
    /// The first block of the tail the loop took in after its last block, where it took one.
    tail: Option<BlockId>,
}

enum Kind {
    Root,
    /// The arm of an `if`; `else_end` is where an else arm runs to, if there is one.
    Then {
        cond: Expr,
        else_end: Option<BlockId>,
        follow: BlockId,
    },
    Else {
        cond: Expr,
        then: Vec<Stmt>,
        follow: BlockId,
    },
    Switch {
        value: Expr,
        bodies: Vec<Body>,
        current: usize,
        cases: Vec<Case>,
        /// Cases that leave at once, written after the bodies.
        extra: Vec<Case>,
        /// Where the switch leaves to past its region, which a jump states once it is written.
        past: Option<BlockId>,
    },
    Loop(Loop),
}

struct Scope {
    kind: Kind,
    /// The first block past the region.
    to: BlockId,
    /// Where control goes off the end of the region's statements.
    follow: Option<BlockId>,
    /// Where `break` goes, and `continue`.
    brk: Option<BlockId>,
    cont: Option<BlockId>,
    out: Vec<Stmt>,
}

/// What `open_loop` did at the block under the cursor.
enum Opened {
    No,
    /// A loop was opened and the block is its first: written without its label, which stands before the loop.
    Body,
    /// The block was taken into a loop's head, or was a whole loop: the cursor moved on.
    Moved,
}

enum Ends {
    Jump(BlockId),
    Return,
    Unreachable,
    Branch,
    Switch,
}

fn leads_to(term: &Terminator, x: BlockId) -> bool {
    match term {
        Terminator::Jump(b) => *b == x,
        Terminator::Branch { then, els, .. } => *then == x || *els == x,
        Terminator::Switch { cases, default, .. } => {
            *default == x || cases.iter().any(|case| case.1 == x)
        }
        Terminator::Return(_) | Terminator::Unreachable => false,
    }
}

/// The variable a statement steps: one it assigns from its own value.
fn stepped(stmt: &ir::Stmt) -> Option<Var> {
    match stmt {
        ir::Stmt::Assign { target, value } if cond::mentions(value, *target) => Some(*target),
        _ => None,
    }
}

fn assigned(stmt: &Stmt) -> Option<Var> {
    match stmt {
        Stmt::Simple(ir::Stmt::Assign { target, .. }) => Some(*target),
        _ => None,
    }
}

fn always() -> Expr {
    Expr::Const(Const::bool(true))
}

struct Machine {
    blocks: Vec<Block>,
    preds: Vec<Vec<BlockId>>,
    labels: Vec<Label>,
    /// The `goto`s written to each block.
    uses: Vec<u32>,
    /// Blocks that are a `return` and nothing else, of a value small enough to write at each jump to them.
    returns: Vec<bool>,
    stack: Vec<Scope>,
    /// The block being written.
    b: BlockId,
    /// Whether the `gotos` rules choose regions too.
    gotos: bool,
}

/// The statements of blocks in their laid-out order, and how many `goto`s name each label; `gotos` turns the `gotos` rules on.
// Every block's label is in the result; the caller drops those nothing names.
pub(super) fn structured(blocks: Vec<Block>, gotos: bool) -> (Vec<Stmt>, HashMap<Label, u32>) {
    let n = blocks.len();
    let labels: Vec<Label> = blocks
        .iter()
        .enumerate()
        .map(|(b, block)| block.at.map_or(Label::Block(b as u32), Label::At))
        .collect();
    let returns = blocks
        .iter()
        .map(|block| {
            let value = match &block.term {
                Terminator::Return(value) => value,
                _ => return false,
            };
            block.stmts.is_empty() && cond::cheap(value.as_ref())
        })
        .collect();
    let root = Scope {
        kind: Kind::Root,
        to: n,
        follow: None,
        brk: None,
        cont: None,
        out: Vec::new(),
    };
    let mut machine = Machine {
        preds: predecessors(&blocks),
        blocks,
        labels,
        uses: vec![0; n],
        returns,
        stack: vec![root],
        b: 0,
        gotos,
    };
    machine.run();
    let body = machine.stack.pop().map(|root| root.out).unwrap_or_default();
    let named = machine.labels.into_iter().zip(machine.uses).collect();
    (body, named)
}

impl Machine {
    /// The region's end, where it leaves to, and where `break` and `continue` go in it.
    fn ctx(&self) -> (BlockId, Option<BlockId>, Option<BlockId>, Option<BlockId>) {
        match self.stack.last() {
            Some(scope) => (scope.to, scope.follow, scope.brk, scope.cont),
            None => (self.blocks.len(), None, None, None),
        }
    }

    /// Where control goes from just before block `x` in the innermost region: `x`, or the region's `follow` at its end.
    fn fall_at(&self, x: BlockId) -> Option<BlockId> {
        let (to, follow, ..) = self.ctx();
        if x < to {
            Some(x)
        } else {
            follow
        }
    }

    fn push(&mut self, stmt: Stmt) {
        if let Some(scope) = self.stack.last_mut() {
            scope.out.push(stmt);
        }
    }

    fn extend(&mut self, stmts: impl IntoIterator<Item = Stmt>) {
        if let Some(scope) = self.stack.last_mut() {
            scope.out.extend(stmts);
        }
    }

    /// The statement that sends control to `target` from where running on leads to `fall`; none when that is `target`.
    // `break` before `continue` before `return`: a loop's own exits must read as the loop's.
    fn jump(
        &mut self,
        brk: Option<BlockId>,
        cont: Option<BlockId>,
        target: BlockId,
        fall: Option<BlockId>,
    ) -> Option<Stmt> {
        if fall == Some(target) {
            return None;
        }
        if brk == Some(target) {
            return Some(Stmt::Break);
        }
        if cont == Some(target) {
            // The innermost loop is the one `continue` goes back to.
            for scope in self.stack.iter_mut().rev() {
                if let Kind::Loop(lp) = &mut scope.kind {
                    lp.continues += 1;
                    break;
                }
            }
            return Some(Stmt::Continue);
        }
        if self.returns[target] {
            if let Terminator::Return(value) = &self.blocks[target].term {
                return Some(Stmt::Return(value.clone()));
            }
        }
        self.uses[target] += 1;
        Some(Stmt::Goto(self.labels[target]))
    }

    /// [`Machine::jump`] from the innermost region.
    fn leave(&mut self, target: BlockId, fall: Option<BlockId>) -> Option<Stmt> {
        let (_, _, brk, cont) = self.ctx();
        self.jump(brk, cont, target, fall)
    }

    fn run(&mut self) {
        let n = self.blocks.len();
        loop {
            while self.stack.len() > 1 && self.stack.last().map(|scope| scope.to) == Some(self.b) {
                self.close();
            }
            if self.b >= n {
                break;
            }
            match self.open_loop() {
                Opened::Moved => continue,
                Opened::Body => self.emit(true),
                Opened::No => self.emit(false),
            }
            self.b += 1;
        }
    }

    /// Whether a block of the loop `[h, latch]` jumps to what follows it, by a jump only `break` states.
    fn breaks(&self, h: BlockId, latch: BlockId, after: Option<BlockId>) -> bool {
        let Some(after) = after else {
            return false;
        };
        if self.returns[after] {
            return false;
        }
        latch - h > SCAN || (h..=latch).any(|p| leads_to(&self.blocks[p].term, after))
    }

    /// Where the loop `[h, latch]` ends once it takes in its tail, if it has one.
    // The tail is the blocks after the loop that one block of the loop leads to and nothing else,
    // up to the one place further on that the loop leaves to. `if (found) { ..; break; }` or
    // `if (done) { ..; return x; }` in a loop: the tail leaves, or runs on to where the loop's
    // other exits go, so that with the tail inside the loop those exits are `break`.
    fn loop_tail(&self, h: BlockId, latch: BlockId) -> Option<BlockId> {
        let (to, ..) = self.ctx();
        let first = latch + 1;
        if first >= to || latch - h > SCAN {
            return None;
        }
        // A loop whose head tests and leaves to what follows it reads as that loop already.
        if let Block {
            stmts,
            term: Terminator::Branch { then, els, .. },
            ..
        } = &self.blocks[h]
        {
            let own = (*then == h + 1 && *els == first) || (*els == h + 1 && *then == first);
            if stmts.is_empty() && h < latch && own {
                return None;
            }
        }
        // A second way into the tail would be a `goto`, and a tail that is only a `return` is
        // written at the jump to it.
        let entered = matches!(self.preds[first].as_slice(), [p] if (h..=latch).contains(p));
        if !entered || self.returns[first] {
            return None;
        }
        let mut end: Option<BlockId> = None;
        for p in h..=latch {
            for x in self.blocks[p].term.successors() {
                if x > first && x <= to && !self.returns[x] {
                    if end.is_some_and(|e| e != x) {
                        return None;
                    }
                    end = Some(x);
                }
            }
        }
        let end = end?;
        // A `break` must be what states a jump to `end`.
        if self.fall_at(end) != Some(end) || end - first > SCAN {
            return None;
        }
        let tail = first..end;
        for x in tail.clone() {
            if x > first && self.preds[x].iter().any(|q| !tail.contains(q)) {
                return None;
            }
            let strays = |y: &BlockId| !tail.contains(y) && *y != end && !self.returns[*y];
            if self.blocks[x].term.successors().iter().any(strays) {
                return None;
            }
        }
        match &self.blocks[end - 1].term {
            Terminator::Return(_) | Terminator::Unreachable | Terminator::Jump(_) => Some(end),
            Terminator::Branch { .. } | Terminator::Switch { .. } => None,
        }
    }

    /// Whether the innermost construct is a loop that took in a tail starting at `x`.
    fn tail_at(&self, x: BlockId) -> bool {
        match self.stack.last().map(|scope| &scope.kind) {
            Some(Kind::Loop(lp)) => lp.tail == Some(x),
            _ => false,
        }
    }

    /// Opens the loop that starts at the block under the cursor, if one does.
    // The loop is the blocks from here to the last one in the region that jumps back here.
    fn open_loop(&mut self) -> Opened {
        let h = self.b;
        let (to, follow, ..) = self.ctx();
        let mut back = 0u32;
        let mut last: Option<BlockId> = None;
        for p in self.preds[h].iter().copied() {
            if (h..to).contains(&p) {
                back += 1;
                last = Some(last.map_or(p, |l| l.max(p)));
            }
        }
        let Some(mut latch) = last else {
            return Opened::No;
        };
        if self.stack.len() >= MAX_NEST {
            return Opened::No;
        }
        let mut tail_from = None;
        if self.gotos {
            if let Some(end) = self.loop_tail(h, latch) {
                tail_from = Some(latch + 1);
                latch = end - 1;
            }
        }
        let after = if latch + 1 < to {
            Some(latch + 1)
        } else {
            follow
        };
        let closed = self.preds[h].iter().all(|p| *p <= latch);
        let label = Stmt::Label(self.labels[h]);

        // A header that only tests, with the body next: `while (cond)`.
        let tested = match &self.blocks[h] {
            Block {
                stmts,
                term: Terminator::Branch { then, els, .. },
                ..
            } if stmts.is_empty() && h < latch && then != els => {
                if *then == h + 1 {
                    Some((true, *els))
                } else if *els == h + 1 {
                    Some((false, *then))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((stays, out)) = tested {
            // A test that leaves elsewhere than to what follows the loop is still the loop's
            // own where no `break` needs that place: the jump is then stated after the loop.
            let tail = Some(out) != after;
            let own = !(tail && ((h..=latch).contains(&out) || self.breaks(h, latch, after)));
            let taken = match own {
                true => std::mem::replace(&mut self.blocks[h].term, Terminator::Unreachable),
                false => Terminator::Unreachable,
            };
            if let Terminator::Branch { cond, .. } = taken {
                let cond = if stays { cond } else { cond::negate(cond) };
                // The latch is the step of a counter set just before the loop: `continue` goes to it.
                let step = match (
                    self.blocks[latch].stmts.as_slice(),
                    &self.blocks[latch].term,
                ) {
                    ([step], Terminator::Jump(back_to)) if *back_to == h => stepped(step),
                    _ => None,
                };
                let init = self
                    .stack
                    .last()
                    .and_then(|scope| scope.out.last())
                    .and_then(assigned);
                // Only where nothing else jumps back: a jump to the test would skip the step.
                let counted =
                    back == 1 && step == init && step.is_some_and(|var| cond::mentions(&cond, var));
                let brk = if tail { None } else { after };
                let lp = Loop {
                    mode: if counted { Mode::For } else { Mode::While },
                    header: h,
                    latch,
                    cond: Some(cond),
                    continues: 0,
                    closed,
                    exit: tail.then_some(out),
                    tail: tail_from,
                };
                let (to, cont) = match counted {
                    true => (latch, latch),
                    false => (latch + 1, h),
                };
                self.push(label);
                self.stack.push(Scope {
                    kind: Kind::Loop(lp),
                    to,
                    follow: Some(cont),
                    brk,
                    cont: Some(cont),
                    out: Vec::new(),
                });
                self.b = h + 1;
                return Opened::Moved;
            }
        }

        // One jump back, from a test: `do { } while (cond)`.
        let exit = match &self.blocks[latch].term {
            Terminator::Branch { then, els, .. } if back == 1 && then != els && *then == h => {
                Some(*els)
            }
            Terminator::Branch { then, els, .. } if back == 1 && then != els && *els == h => {
                Some(*then)
            }
            _ => None,
        };
        if let Some(exit) = exit {
            if latch == h {
                // The whole loop is this block.
                let stmts = std::mem::take(&mut self.blocks[h].stmts);
                let taken = std::mem::replace(&mut self.blocks[h].term, Terminator::Unreachable);
                if let Terminator::Branch { cond, then, .. } = taken {
                    let cond = if then == h { cond } else { cond::negate(cond) };
                    let fall = self.fall_at(h + 1);
                    let tail = self.leave(exit, fall);
                    self.push(label);
                    self.push(Stmt::DoWhile {
                        body: stmts.into_iter().map(Stmt::Simple).collect(),
                        cond,
                    });
                    self.extend(tail);
                }
                self.b = h + 1;
                return Opened::Moved;
            }
            // A `break` lands on what follows the loop, so the loop has one only when its test leaves there too.
            let plain = Some(exit) == after;
            if plain || !((h..=latch).contains(&exit) || self.breaks(h, latch, after)) {
                let lp = Loop {
                    mode: Mode::Do,
                    header: h,
                    latch,
                    cond: None,
                    continues: 0,
                    closed,
                    exit: Some(exit),
                    tail: tail_from,
                };
                // `continue` in a `do` goes to the test, which is the latch only while the latch holds nothing else.
                let cont = self.blocks[latch].stmts.is_empty().then_some(latch);
                self.push(label);
                self.stack.push(Scope {
                    kind: Kind::Loop(lp),
                    to: latch,
                    follow: Some(latch),
                    brk: if plain { after } else { None },
                    cont,
                    out: Vec::new(),
                });
                return Opened::Body;
            }
        }

        let lp = Loop {
            mode: Mode::Forever,
            header: h,
            latch,
            cond: None,
            continues: 0,
            closed,
            exit: None,
            tail: tail_from,
        };
        self.push(label);
        self.stack.push(Scope {
            kind: Kind::Loop(lp),
            to: latch + 1,
            follow: Some(h),
            brk: after,
            cont: Some(h),
            out: Vec::new(),
        });
        Opened::Body
    }

    /// Writes the block under the cursor: its label, its statements, and what its terminator comes to here.
    fn emit(&mut self, skip_label: bool) {
        let b = self.b;
        if !skip_label {
            self.push(Stmt::Label(self.labels[b]));
        }
        let stmts = std::mem::take(&mut self.blocks[b].stmts);
        self.extend(stmts.into_iter().map(Stmt::Simple));
        let fall = self.fall_at(b + 1);
        let ends = match &self.blocks[b].term {
            Terminator::Jump(target) => Ends::Jump(*target),
            Terminator::Return(_) => Ends::Return,
            Terminator::Unreachable => Ends::Unreachable,
            Terminator::Branch { .. } => Ends::Branch,
            Terminator::Switch { .. } => Ends::Switch,
        };
        match ends {
            Ends::Jump(target) => {
                let stmt = self.leave(target, fall);
                self.extend(stmt);
            }
            Ends::Return => {
                // A `return` written at the jumps to it too stays in its block to be read again.
                let value = match &mut self.blocks[b].term {
                    Terminator::Return(value) if self.returns[b] => value.clone(),
                    Terminator::Return(value) => value.take(),
                    _ => None,
                };
                self.push(Stmt::Return(value));
            }
            Ends::Unreachable => self.push(Stmt::Unreachable),
            Ends::Branch => self.branch(b, fall),
            Ends::Switch => self.switch(b, fall),
        }
    }

    /// Where the region `[near, end)` leaves to when it runs off its last block, if that block says.
    fn tail_exit(&self, near: BlockId, end: BlockId) -> Option<BlockId> {
        let x = end - 1;
        match &self.blocks[x].term {
            Terminator::Jump(e) if !(near..=x).contains(e) => Some(*e),
            // The region ends in a loop: the nearest place past the region that the loop leaves to.
            Terminator::Jump(e) if x - *e <= SCAN => (*e..=x)
                .flat_map(|p| self.blocks[p].term.successors())
                .filter(|s| *s >= end)
                .min(),
            Terminator::Branch { then, els, .. } if then != els => {
                let inside = |t: BlockId| (near..=x).contains(&t);
                match (inside(*then), inside(*els)) {
                    (true, false) => Some(*els),
                    (false, true) => Some(*then),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Where an else arm ends and what follows the `if`, when a then arm that ends before `far` leaves to `e`.
    fn else_for(&self, far: BlockId, e: BlockId) -> Option<(BlockId, BlockId)> {
        let (to, follow, brk, cont) = self.ctx();
        if brk == Some(e) {
            return None;
        }
        let between = e > far && e <= to;
        let inside = e < to && cont != Some(e);
        if between && (inside || (e == to && follow == Some(e))) {
            // The then arm leaves over the blocks up to `e`: they are the else arm.
            Some((e, e))
        } else if follow == Some(e) && far < to && !between {
            // It leaves to where the region does: the rest of the region is the else arm.
            Some((to, e))
        } else {
            None
        }
    }

    /// The else arm and the follow that the one place past the then arm `[near, far)` its blocks jump to makes, where one place makes them.
    // The arm's last block says nothing here (it returns, say), but a block inside it leaves
    // past `far`: with the blocks in between as the else arm, that jump runs off the then arm.
    // ⚠ Only where one place does: with two, either choice leaves the other a `goto` into an arm.
    fn inner_exit(&self, near: BlockId, far: BlockId) -> Option<(BlockId, BlockId)> {
        if far - near > SCAN {
            return None;
        }
        let (_, _, brk, cont) = self.ctx();
        let mut seen: Vec<BlockId> = Vec::new();
        let mut arms = None;
        for p in near..far {
            for x in self.blocks[p].term.successors() {
                // A `break`, a `continue` or a `return` states the jump already.
                let stated = self.returns[x] || brk == Some(x) || cont == Some(x);
                if x <= far || stated || seen.contains(&x) {
                    continue;
                }
                if seen.len() == EXITS {
                    return None;
                }
                seen.push(x);
                if let Some(made) = self.else_for(far, x) {
                    if arms.is_some() {
                        return None;
                    }
                    arms = Some(made);
                }
            }
        }
        arms
    }

    fn branch(&mut self, b: BlockId, fall: Option<BlockId>) {
        let taken = std::mem::replace(&mut self.blocks[b].term, Terminator::Unreachable);
        let Terminator::Branch { cond, then, els } = taken else {
            return;
        };
        let (to, follow, brk, cont) = self.ctx();
        let near = b + 1;
        // The block the test skips to, and whether the blocks in between run when it fails.
        let skips = if then == els {
            None
        } else if els == near {
            Some((then, true))
        } else if then == near {
            Some((els, false))
        } else {
            None
        };
        if let Some((far, on_failure)) = skips {
            let open = near < to && brk != Some(far) && self.stack.len() < MAX_NEST;
            let within = far > near && far <= to;
            // `if (c) continue;` reads better than nesting what follows it, but for the tail a
            // loop took in: that is the arm of the test that leaves to it.
            let nests = cont != Some(far) || self.tail_at(near);
            if open && !within && follow == Some(far) && nests {
                // The rest of the region runs or is skipped: the test leaves to where the region leaves.
                let cond = if on_failure { cond::negate(cond) } else { cond };
                self.stack.push(Scope {
                    kind: Kind::Then {
                        cond,
                        else_end: None,
                        follow: far,
                    },
                    to,
                    follow: Some(far),
                    brk,
                    cont,
                    out: Vec::new(),
                });
                return;
            }
            if open && within {
                let mut else_end = None;
                let mut after = far;
                // An else is what nothing of the then arm reaches.
                let apart = !self.preds[far].iter().any(|p| (near..far).contains(p));
                let mut arms = self
                    .tail_exit(near, far)
                    .filter(|_| apart)
                    .and_then(|e| self.else_for(far, e));
                if arms.is_none() && apart && self.gotos {
                    arms = self.inner_exit(near, far);
                }
                if let Some((end, join)) = arms {
                    else_end = Some(end);
                    after = join;
                }
                let cond = if on_failure { cond::negate(cond) } else { cond };
                self.stack.push(Scope {
                    kind: Kind::Then {
                        cond,
                        else_end,
                        follow: after,
                    },
                    to: far,
                    follow: Some(after),
                    brk,
                    cont,
                    out: Vec::new(),
                });
                return;
            }
        }

        // No region is the test's own: it only chooses between two jumps.
        if then == els {
            let on = self.leave(then, fall);
            self.push(Stmt::If {
                cond,
                then: Vec::new(),
                els: Vec::new(),
            });
            self.extend(on);
        } else if fall == Some(els) {
            let go = self.leave(then, None);
            self.push(Stmt::If {
                cond,
                then: go.into_iter().collect(),
                els: Vec::new(),
            });
        } else if fall == Some(then) {
            let go = self.leave(els, None);
            self.push(Stmt::If {
                cond: cond::negate(cond),
                then: go.into_iter().collect(),
                els: Vec::new(),
            });
        } else {
            let go = self.leave(then, None);
            let on = self.leave(els, fall);
            self.push(Stmt::If {
                cond,
                then: go.into_iter().collect(),
                els: Vec::new(),
            });
            self.extend(on);
        }
    }

    /// The case that leaves to `target` among `cases`, made if there is none yet.
    fn leaving<'a>(
        &mut self,
        cases: &'a mut Vec<(BlockId, Case)>,
        brk: Option<BlockId>,
        cont: Option<BlockId>,
        target: BlockId,
    ) -> &'a mut Case {
        let at = match cases.iter().position(|(to, _)| *to == target) {
            Some(at) => at,
            None => {
                let go = self.jump(brk, cont, target, None);
                let case = Case {
                    labels: Vec::new(),
                    body: go.into_iter().collect(),
                };
                cases.push((target, case));
                cases.len() - 1
            }
        };
        &mut cases[at].1
    }

    fn switch(&mut self, b: BlockId, fall: Option<BlockId>) {
        let taken = std::mem::replace(&mut self.blocks[b].term, Terminator::Unreachable);
        let Terminator::Switch {
            value,
            mut cases,
            default,
        } = taken
        else {
            return;
        };
        let (to, follow, _, cont) = self.ctx();
        cases.sort_unstable();
        let mut inside: Vec<BlockId> = cases
            .iter()
            .map(|case| case.1)
            .chain([default])
            .filter(|x| *x > b && *x < to)
            .collect();
        inside.sort_unstable();
        inside.dedup();

        if inside.first() == Some(&(b + 1)) && self.stack.len() < MAX_NEST {
            // The switch ends where a body leaves to past the body after it, at the nearest such place.
            let mut exit: Option<BlockId> = None;
            for start in inside.iter().copied().skip(1) {
                if let Terminator::Jump(x) = &self.blocks[start - 1].term {
                    if *x > start && *x <= to && exit.is_none_or(|e| *x < e) {
                        exit = Some(*x);
                    }
                }
            }
            let last = inside[inside.len() - 1];
            let said = exit.is_some();
            if exit.is_none() && inside.len() >= 2 && default == last {
                // No body says, and the default is past every case: it is what follows the switch.
                exit = Some(last);
            }
            let mut exit = exit.unwrap_or_else(|| {
                // The last body runs to the first block something before it also leads to.
                let limit = to.min(last + 1 + SCAN);
                let mut x = last + 1;
                while x < limit && !self.preds[x].iter().any(|p| *p < last) {
                    x += 1;
                }
                x
            });
            let mut after = if exit < to { Some(exit) } else { follow };
            // No body says where the switch ends inside the region, and the bodies that say
            // leave past it to one place: the switch runs to the region's end and leaves there,
            // so that those jumps are its `break`.
            let mut past: Option<BlockId> = None;
            if !said && self.gotos && to - b <= SCAN {
                let mut one = true;
                for start in inside.iter().copied().skip(1) {
                    if let Terminator::Jump(x) = &self.blocks[start - 1].term {
                        // A `return` or a `continue` states the jump already.
                        if *x > to && !self.returns[*x] && cont != Some(*x) {
                            one &= past.is_none_or(|place| place == *x);
                            past = Some(*x);
                        }
                    }
                }
                // ⚠ Not where anything of the switch leaves to where it would end otherwise:
                // that jump would become a `goto`. A default taken for what follows becomes a
                // body, so the switch's own jump to it is none.
                let defaulted = exit == last && default == last;
                let stays = after.filter(|nat| Some(*nat) != past).is_some_and(|nat| {
                    let own = default == nat || cases.iter().any(|case| case.1 == nat);
                    (own && !defaulted)
                        || (b + 1..exit).any(|x| leads_to(&self.blocks[x].term, nat))
                });
                if !one || stays {
                    past = None;
                }
            }
            if let Some(place) = past {
                exit = to;
                after = Some(place);
            }
            let starts: Vec<BlockId> = inside.into_iter().filter(|x| *x < exit).collect();
            let mut bodies: Vec<Body> = starts
                .iter()
                .enumerate()
                .map(|(i, start)| Body {
                    start: *start,
                    end: starts.get(i + 1).copied().unwrap_or(exit),
                    labels: Vec::new(),
                })
                .collect();
            // A value that no case names leaves the switch: a default that goes where that leads needs no case.
            let default_body = starts.binary_search(&default).ok();
            let default_out = default_body.is_none() && after == Some(default);
            let mut extra: Vec<(BlockId, Case)> = Vec::new();
            for (label, target) in cases {
                match starts.binary_search(&target) {
                    Ok(body) => bodies[body].labels.push(Some(label)),
                    Err(_) if default_out && after == Some(target) => {}
                    Err(_) => self
                        .leaving(&mut extra, after, cont, target)
                        .labels
                        .push(Some(label)),
                }
            }
            match default_body {
                Some(body) => bodies[body].labels.push(None),
                None if default_out => {}
                None => self
                    .leaving(&mut extra, after, cont, default)
                    .labels
                    .push(None),
            }
            let first_end = bodies[0].end;
            let second = bodies.get(1).map(|body| body.start);
            self.stack.push(Scope {
                kind: Kind::Switch {
                    value,
                    bodies,
                    current: 0,
                    cases: Vec::new(),
                    extra: extra.into_iter().map(|(_, case)| case).collect(),
                    past,
                },
                to: first_end,
                // A body that runs off its end runs into the next; the last has nowhere to run to.
                follow: second,
                brk: after,
                cont,
                out: Vec::new(),
            });
            return;
        }

        // No body follows the switch: each case is a jump, and `break` is the statement after it.
        let default_out = fall == Some(default);
        let mut flat: Vec<(BlockId, Case)> = Vec::new();
        for (label, target) in cases {
            if default_out && target == default {
                continue;
            }
            self.leaving(&mut flat, fall, cont, target)
                .labels
                .push(Some(label));
        }
        if !default_out {
            self.leaving(&mut flat, fall, cont, default)
                .labels
                .push(None);
        }
        self.push(Stmt::Switch {
            value,
            cases: flat.into_iter().map(|(_, case)| case).collect(),
        });
    }

    /// Closes the innermost construct, which ends at the block under the cursor.
    fn close(&mut self) {
        let Some(mut scope) = self.stack.pop() else {
            return;
        };
        let out = std::mem::take(&mut scope.out);
        match scope.kind {
            Kind::Root => {
                scope.kind = Kind::Root;
                scope.out = out;
                self.stack.push(scope);
            }
            Kind::Then {
                cond,
                else_end: Some(end),
                follow,
            } => {
                scope.kind = Kind::Else {
                    cond,
                    then: out,
                    follow,
                };
                scope.to = end;
                self.stack.push(scope);
            }
            Kind::Then {
                cond,
                else_end: None,
                follow,
            } => self.close_if(cond, out, Vec::new(), follow),
            Kind::Else { cond, then, follow } => self.close_if(cond, then, out, follow),
            Kind::Switch {
                value,
                mut bodies,
                current,
                mut cases,
                extra,
                past,
            } => {
                let labels = bodies
                    .get_mut(current)
                    .map(|body| std::mem::take(&mut body.labels))
                    .unwrap_or_default();
                cases.push(Case { labels, body: out });
                match bodies.get(current + 1).map(|body| body.end) {
                    Some(end) => {
                        scope.to = end;
                        scope.follow = bodies.get(current + 2).map(|body| body.start);
                        scope.kind = Kind::Switch {
                            value,
                            bodies,
                            current: current + 1,
                            cases,
                            extra,
                            past,
                        };
                        self.stack.push(scope);
                    }
                    None => {
                        cases.extend(extra);
                        self.push(Stmt::Switch { value, cases });
                        // A `break` of the switch lands here, short of where it leaves to.
                        if let Some(place) = past {
                            let fall = self.fall_at(self.b);
                            let tail = self.leave(place, fall);
                            self.extend(tail);
                        }
                    }
                }
            }
            Kind::Loop(lp) => self.close_loop(lp, out),
        }
    }

    fn close_if(&mut self, cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>, follow: BlockId) {
        let fall = self.fall_at(self.b);
        let tail = self.leave(follow, fall);
        self.push(Stmt::If { cond, then, els });
        self.extend(tail);
    }

    /// Whether the statements before a loop end in the setting of `var` and the loop's own label, with nothing naming the label.
    fn counter_before(&self, lp: &Loop, var: Var) -> bool {
        let Some(scope) = self.stack.last() else {
            return false;
        };
        let label = self.labels[lp.header];
        let ends = match scope.out.as_slice() {
            [.., init, Stmt::Label(l)] => *l == label && assigned(init) == Some(var),
            _ => false,
        };
        ends && lp.closed && self.uses[lp.header] == 0
    }

    /// The counter's setting, taken from before the loop along with the loop's label.
    fn take_init(&mut self) -> Option<Box<ir::Stmt>> {
        let scope = self.stack.last_mut()?;
        scope.out.pop();
        match scope.out.pop() {
            Some(Stmt::Simple(init)) => Some(Box::new(init)),
            Some(other) => {
                scope.out.push(other);
                None
            }
            None => None,
        }
    }

    fn close_loop(&mut self, mut lp: Loop, mut body: Vec<Stmt>) {
        match lp.mode {
            Mode::Forever => self.push(Stmt::While {
                cond: always(),
                body,
            }),
            Mode::While => {
                let cond = lp.cond.take().unwrap_or_else(always);
                // The body ends in the step of a counter set just before the loop, and no
                // `continue` would skip the step: `for`.
                let step = match body.last() {
                    Some(Stmt::Simple(last)) if lp.continues == 0 => stepped(last),
                    _ => None,
                };
                let counted = step
                    .is_some_and(|var| cond::mentions(&cond, var) && self.counter_before(&lp, var));
                match (counted, body.pop()) {
                    (true, Some(Stmt::Simple(step))) => {
                        let init = self.take_init();
                        self.push(Stmt::For {
                            init,
                            cond: Some(cond),
                            step: Some(Box::new(step)),
                            body,
                        });
                    }
                    (_, last) => {
                        body.extend(last);
                        self.push(Stmt::While { cond, body });
                    }
                }
            }
            Mode::For => {
                let latch = lp.latch;
                // A `goto` to the step lands at the end of the body, which runs on into the step.
                body.push(Stmt::Label(self.labels[latch]));
                let step = self.blocks[latch].stmts.pop().map(Box::new);
                let var = step.as_deref().and_then(stepped);
                let init = match var.is_some_and(|var| self.counter_before(&lp, var)) {
                    true => self.take_init(),
                    false => None,
                };
                self.push(Stmt::For {
                    init,
                    cond: lp.cond.take(),
                    step,
                    body,
                });
                self.b = latch + 1;
            }
            Mode::Do => {
                let latch = lp.latch;
                body.push(Stmt::Label(self.labels[latch]));
                let stmts = std::mem::take(&mut self.blocks[latch].stmts);
                body.extend(stmts.into_iter().map(Stmt::Simple));
                let taken =
                    std::mem::replace(&mut self.blocks[latch].term, Terminator::Unreachable);
                let cond = match taken {
                    Terminator::Branch { cond, then, .. } if then == lp.header => cond,
                    Terminator::Branch { cond, .. } => cond::negate(cond),
                    _ => always(),
                };
                self.push(Stmt::DoWhile { body, cond });
                self.b = latch + 1;
            }
        }
        if let Some(exit) = lp.exit {
            let fall = self.fall_at(self.b);
            let tail = self.leave(exit, fall);
            self.extend(tail);
        }
    }
}
