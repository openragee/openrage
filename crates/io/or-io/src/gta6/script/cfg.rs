//! The control-flow form the passes work on: basic blocks of simple statements, each ending in
//! one terminator, and the lowering of the structured form into it.

use std::collections::{HashMap, HashSet};

use super::ir::{Expr, Head, Stmt, TypeKey};
use super::tree::{self, Label};

/// A block's index in [`Cfg::blocks`].
pub type BlockId = usize;

/// The block a function starts in.
pub const ENTRY: BlockId = 0;

/// How control leaves a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    Jump(BlockId),
    /// To `then` when `cond` holds, else to `els`.
    Branch {
        cond: Expr,
        then: BlockId,
        els: BlockId,
    },
    /// To the block of the case `value` equals as an `int`, else to `default`.
    Switch {
        value: Expr,
        cases: Vec<(i64, BlockId)>,
        default: BlockId,
    },
    Return(Option<Expr>),
    /// The listing's `throw`: control that reaches the end of the block has fallen off the bytecode.
    Unreachable,
}

impl Terminator {
    /// The blocks control may go to, each once, in the order the terminator names them.
    pub fn successors(&self) -> Vec<BlockId> {
        let mut out = Vec::new();
        let mut add = |b: BlockId| {
            if !out.contains(&b) {
                out.push(b);
            }
        };
        match self {
            Terminator::Jump(b) => add(*b),
            Terminator::Branch { then, els, .. } => {
                add(*then);
                add(*els);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, b) in cases {
                    add(*b);
                }
                add(*default);
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
        out
    }

    /// Replaces every block the terminator names by what `f` maps it to.
    pub fn retarget(&mut self, f: impl Fn(BlockId) -> BlockId) {
        match self {
            Terminator::Jump(b) => *b = f(*b),
            Terminator::Branch { then, els, .. } => {
                *then = f(*then);
                *els = f(*els);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, b) in cases.iter_mut() {
                    *b = f(*b);
                }
                *default = f(*default);
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }

    /// The condition, switch value or returned value, where there is one.
    pub fn expr(&self) -> Option<&Expr> {
        match self {
            Terminator::Branch { cond: e, .. } | Terminator::Switch { value: e, .. } => Some(e),
            Terminator::Return(e) => e.as_ref(),
            Terminator::Jump(_) | Terminator::Unreachable => None,
        }
    }

    pub fn expr_mut(&mut self) -> Option<&mut Expr> {
        match self {
            Terminator::Branch { cond: e, .. } | Terminator::Switch { value: e, .. } => Some(e),
            Terminator::Return(e) => e.as_mut(),
            Terminator::Jump(_) | Terminator::Unreachable => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The bytecode offset the block starts at, where a listing label states it.
    pub at: Option<u32>,
    pub stmts: Vec<Stmt>,
    pub term: Terminator,
}

/// A function's blocks, in source order; block 0 is the entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cfg {
    pub blocks: Vec<Block>,
}

impl Cfg {
    pub fn successors(&self, b: BlockId) -> Vec<BlockId> {
        self.blocks[b].term.successors()
    }

    /// Which blocks control can reach from the entry.
    pub fn reachable(&self) -> Vec<bool> {
        let mut seen = vec![false; self.blocks.len()];
        let mut stack = Vec::new();
        if !self.blocks.is_empty() {
            stack.push(ENTRY);
        }
        while let Some(b) = stack.pop() {
            if !std::mem::replace(&mut seen[b], true) {
                stack.extend(self.successors(b));
            }
        }
        seen
    }

    /// The blocks that jump to each block, each once; only blocks the entry reaches are listed.
    pub fn predecessors(&self) -> Vec<Vec<BlockId>> {
        let reachable = self.reachable();
        let mut preds = vec![Vec::new(); self.blocks.len()];
        for (b, block) in self.blocks.iter().enumerate() {
            if reachable[b] {
                for s in block.term.successors() {
                    preds[s].push(b);
                }
            }
        }
        preds
    }

    /// The blocks the entry reaches, each after every block that leads to it but for a back edge.
    pub fn reverse_post_order(&self) -> Vec<BlockId> {
        let mut order = Vec::with_capacity(self.blocks.len());
        if self.blocks.is_empty() {
            return order;
        }
        let mut seen = vec![false; self.blocks.len()];
        seen[ENTRY] = true;
        // A block, its successors, and how many of them were taken.
        let mut stack = vec![(ENTRY, self.successors(ENTRY), 0usize)];
        while let Some(top) = stack.last_mut() {
            let (b, taken) = (top.0, top.2);
            top.2 += 1;
            match top.1.get(taken).copied() {
                Some(s) if seen[s] => {}
                Some(s) => {
                    seen[s] = true;
                    stack.push((s, self.successors(s), 0));
                }
                None => {
                    order.push(b);
                    stack.pop();
                }
            }
        }
        order.reverse();
        order
    }

    /// Each block's immediate dominator: the entry's is itself, an unreached block's is `None`.
    pub fn dominators(&self) -> Vec<Option<BlockId>> {
        let n = self.blocks.len();
        let mut idom: Vec<Option<BlockId>> = vec![None; n];
        if n == 0 {
            return idom;
        }
        let order = self.reverse_post_order();
        let mut rank = vec![usize::MAX; n];
        for (i, b) in order.iter().enumerate() {
            rank[*b] = i;
        }
        let preds = self.predecessors();
        idom[ENTRY] = Some(ENTRY);
        let mut changed = true;
        while changed {
            changed = false;
            for b in order.iter().copied().skip(1) {
                let mut new: Option<BlockId> = None;
                for p in preds[b].iter().copied() {
                    if idom[p].is_some() {
                        new = Some(match new {
                            Some(q) => intersect(&idom, &rank, p, q),
                            None => p,
                        });
                    }
                }
                if new.is_some() && idom[b] != new {
                    idom[b] = new;
                    changed = true;
                }
            }
        }
        idom
    }

    /// The simple statements of every block.
    pub fn statements(&self) -> usize {
        self.blocks.iter().map(|b| b.stmts.len()).sum()
    }
}

/// The nearest block that dominates both `a` and `b`, by the dominators found so far.
fn intersect(idom: &[Option<BlockId>], rank: &[usize], mut a: BlockId, mut b: BlockId) -> BlockId {
    while a != b {
        while rank[a] > rank[b] {
            match idom[a] {
                Some(up) => a = up,
                None => return b,
            }
        }
        while rank[b] > rank[a] {
            match idom[b] {
                Some(up) => b = up,
                None => return a,
            }
        }
    }
    a
}

/// Whether every path from the entry to `b` passes through `a`, by [`Cfg::dominators`]' answer.
pub fn dominates(idom: &[Option<BlockId>], a: BlockId, mut b: BlockId) -> bool {
    loop {
        if a == b {
            return true;
        }
        match idom.get(b).copied().flatten() {
            Some(up) if up != b => b = up,
            _ => return false,
        }
    }
}

/// A function in the control-flow form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub head: Head,
    pub cfg: Cfg,
}

/// Why a structured body does not lower.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LowerError {
    #[error("{0} is defined twice")]
    DuplicateLabel(Label),
    #[error("a goto names {0}, which the function does not define")]
    UnknownLabel(Label),
    #[error("a `break` outside a loop or a switch")]
    StrayBreak,
    #[error("a `continue` outside a loop")]
    StrayContinue,
}

/// A run of blocks whose faults go to a handler, as a compiled program's tables state it.
// ⚠ No terminator leads to a handler, so `successors` and all that is built on it see none: a
// function that has regions is carried beside them and given to no pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub kind: RegionKind,
    /// The blocks the region protects.
    pub blocks: Vec<BlockId>,
    /// The block control goes to when one of them faults.
    pub handler: BlockId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// Takes faults of the type named, any fault for `None`.
    Catch(Option<TypeKey>),
    /// Runs however the region is left.
    Finally,
}

/// The blocks in their order as labels and jumps, with no construct rebuilt: what a front end made, to print as it is.
pub fn flat(f: Function) -> tree::Function {
    let blocks = f.cfg.blocks;
    let labels: Vec<Label> = blocks
        .iter()
        .enumerate()
        .map(|(b, block)| match block.at {
            Some(at) => Label::At(at),
            None => Label::Block(b as u32),
        })
        .collect();
    // A block is labelled where something jumps to it other than the block before running into it.
    let mut named = vec![false; blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for to in block.term.successors() {
            let runs_in = matches!(block.term, Terminator::Jump(_)) && to == b + 1;
            if let (false, Some(flag)) = (runs_in, named.get_mut(to)) {
                *flag = true;
            }
        }
    }
    let go = |to: BlockId| {
        let label = labels.get(to).copied().unwrap_or(Label::Block(to as u32));
        vec![tree::Stmt::Goto(label)]
    };
    let mut body = Vec::new();
    for (b, block) in blocks.into_iter().enumerate() {
        if named[b] {
            body.push(tree::Stmt::Label(labels[b]));
        }
        body.extend(block.stmts.into_iter().map(tree::Stmt::Simple));
        match block.term {
            Terminator::Jump(to) if to == b + 1 => {}
            Terminator::Jump(to) => body.extend(go(to)),
            Terminator::Branch { cond, then, els } => body.push(tree::Stmt::If {
                cond,
                then: go(then),
                els: go(els),
            }),
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let mut all: Vec<tree::Case> = cases
                    .into_iter()
                    .map(|(at, to)| tree::Case {
                        labels: vec![Some(at)],
                        body: go(to),
                    })
                    .collect();
                all.push(tree::Case {
                    labels: vec![None],
                    body: go(default),
                });
                body.push(tree::Stmt::Switch { value, cases: all });
            }
            Terminator::Return(value) => body.push(tree::Stmt::Return(value)),
            Terminator::Unreachable => body.push(tree::Stmt::Unreachable),
        }
    }
    tree::Function { head: f.head, body }
}

/// [`lower_body`] over a function, keeping its head.
pub fn lower(f: tree::Function) -> Result<Function, LowerError> {
    Ok(Function {
        head: f.head,
        cfg: lower_body(f.body)?,
    })
}

/// Cuts a structured body into blocks, with every simple statement kept once.
// A block the entry does not reach is kept only while it holds statements or is jumped to by one
// that does. A block that holds nothing and only jumps on is kept only where a listing label
// names it, so `if (c) { goto a; } else { goto b; }` is one branch to `a` and `b`.
pub fn lower_body(body: Vec<tree::Stmt>) -> Result<Cfg, LowerError> {
    let mut lowering = Lowering {
        blocks: vec![Draft::default()],
        started: vec![ENTRY],
        labels: HashMap::new(),
        defined: HashSet::new(),
        cur: ENTRY,
    };
    lowering.seq(
        body,
        Ctx {
            brk: None,
            cont: None,
        },
    )?;
    lowering.finish()
}

#[derive(Default)]
struct Draft {
    at: Option<u32>,
    stmts: Vec<Stmt>,
    term: Option<Terminator>,
}

/// Where `break` and `continue` go from the statements being lowered.
#[derive(Clone, Copy)]
struct Ctx {
    brk: Option<BlockId>,
    cont: Option<BlockId>,
}

struct Lowering {
    blocks: Vec<Draft>,
    /// Blocks in the order statements first went into them, which is source order.
    started: Vec<BlockId>,
    labels: HashMap<Label, BlockId>,
    defined: HashSet<Label>,
    /// The block statements go into.
    cur: BlockId,
}

impl Lowering {
    fn new_block(&mut self) -> BlockId {
        self.blocks.push(Draft::default());
        self.blocks.len() - 1
    }

    /// Makes `b`, which nothing has gone into yet, the block statements go into.
    fn start(&mut self, b: BlockId) {
        self.started.push(b);
        self.cur = b;
    }

    fn seal(&mut self, term: Terminator) {
        self.blocks[self.cur].term = Some(term);
    }

    /// Ends the current block with a terminator nothing falls out of; what follows is dead until a label.
    fn leave(&mut self, term: Terminator) {
        self.seal(term);
        let dead = self.new_block();
        self.start(dead);
    }

    fn label(&mut self, label: Label) -> BlockId {
        if let Some(b) = self.labels.get(&label) {
            return *b;
        }
        let b = self.new_block();
        if let Label::At(at) = label {
            self.blocks[b].at = Some(at);
        }
        self.labels.insert(label, b);
        b
    }

    fn seq(&mut self, stmts: Vec<tree::Stmt>, ctx: Ctx) -> Result<(), LowerError> {
        for stmt in stmts {
            match stmt {
                tree::Stmt::Simple(s) => self.blocks[self.cur].stmts.push(s),
                // A source construct lowers as the statements it stands for.
                tree::Stmt::Sugar(sugar) => self.seq(sugar.desugar(), ctx)?,
                tree::Stmt::Label(label) => {
                    if !self.defined.insert(label) {
                        return Err(LowerError::DuplicateLabel(label));
                    }
                    let b = self.label(label);
                    self.seal(Terminator::Jump(b));
                    self.start(b);
                }
                tree::Stmt::Goto(label) => {
                    let b = self.label(label);
                    self.leave(Terminator::Jump(b));
                }
                tree::Stmt::Return(value) => self.leave(Terminator::Return(value)),
                tree::Stmt::Unreachable => self.leave(Terminator::Unreachable),
                tree::Stmt::Break => {
                    let b = ctx.brk.ok_or(LowerError::StrayBreak)?;
                    self.leave(Terminator::Jump(b));
                }
                tree::Stmt::Continue => {
                    let b = ctx.cont.ok_or(LowerError::StrayContinue)?;
                    self.leave(Terminator::Jump(b));
                }
                tree::Stmt::If { cond, then, els } => {
                    let join = self.new_block();
                    let then_b = if then.is_empty() {
                        join
                    } else {
                        self.new_block()
                    };
                    let els_b = if els.is_empty() {
                        join
                    } else {
                        self.new_block()
                    };
                    self.seal(Terminator::Branch {
                        cond,
                        then: then_b,
                        els: els_b,
                    });
                    if then_b != join {
                        self.start(then_b);
                        self.seq(then, ctx)?;
                        self.seal(Terminator::Jump(join));
                    }
                    if els_b != join {
                        self.start(els_b);
                        self.seq(els, ctx)?;
                        self.seal(Terminator::Jump(join));
                    }
                    self.start(join);
                }
                tree::Stmt::While { cond, body } => {
                    let head = self.new_block();
                    let exit = self.new_block();
                    self.seal(Terminator::Jump(head));
                    self.start(head);
                    // `while (Truth(true))` has no test: its body starts in the head itself.
                    if !always(&cond) {
                        let first = self.new_block();
                        self.seal(Terminator::Branch {
                            cond,
                            then: first,
                            els: exit,
                        });
                        self.start(first);
                    }
                    self.seq(
                        body,
                        Ctx {
                            brk: Some(exit),
                            cont: Some(head),
                        },
                    )?;
                    self.seal(Terminator::Jump(head));
                    self.start(exit);
                }
                tree::Stmt::DoWhile { body, cond } => {
                    let first = self.new_block();
                    let test = self.new_block();
                    let exit = self.new_block();
                    self.seal(Terminator::Jump(first));
                    self.start(first);
                    self.seq(
                        body,
                        Ctx {
                            brk: Some(exit),
                            cont: Some(test),
                        },
                    )?;
                    self.seal(Terminator::Jump(test));
                    self.start(test);
                    self.seal(Terminator::Branch {
                        cond,
                        then: first,
                        els: exit,
                    });
                    self.start(exit);
                }
                tree::Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    if let Some(init) = init {
                        self.blocks[self.cur].stmts.push(*init);
                    }
                    let head = self.new_block();
                    let next = self.new_block();
                    let exit = self.new_block();
                    self.seal(Terminator::Jump(head));
                    self.start(head);
                    if let Some(cond) = cond {
                        let first = self.new_block();
                        self.seal(Terminator::Branch {
                            cond,
                            then: first,
                            els: exit,
                        });
                        self.start(first);
                    }
                    self.seq(
                        body,
                        Ctx {
                            brk: Some(exit),
                            cont: Some(next),
                        },
                    )?;
                    self.seal(Terminator::Jump(next));
                    self.start(next);
                    if let Some(step) = step {
                        self.blocks[self.cur].stmts.push(*step);
                    }
                    self.seal(Terminator::Jump(head));
                    self.start(exit);
                }
                tree::Stmt::Switch { value, cases } => {
                    let exit = self.new_block();
                    let bodies: Vec<BlockId> = cases.iter().map(|_| self.new_block()).collect();
                    let mut table = Vec::new();
                    let mut default = exit;
                    for (case, b) in cases.iter().zip(bodies.iter().copied()) {
                        for label in &case.labels {
                            match label {
                                Some(v) => table.push((*v, b)),
                                None => default = b,
                            }
                        }
                    }
                    self.seal(Terminator::Switch {
                        value,
                        cases: table,
                        default,
                    });
                    let inner = Ctx {
                        brk: Some(exit),
                        cont: ctx.cont,
                    };
                    for (i, case) in cases.into_iter().enumerate() {
                        self.start(bodies[i]);
                        self.seq(case.body, inner)?;
                        let next = bodies.get(i + 1).copied().unwrap_or(exit);
                        self.seal(Terminator::Jump(next));
                    }
                    self.start(exit);
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Cfg, LowerError> {
        self.seal(Terminator::Unreachable);
        let undefined = self
            .labels
            .keys()
            .copied()
            .filter(|l| !self.defined.contains(l));
        if let Some(label) = undefined.min() {
            return Err(LowerError::UnknownLabel(label));
        }

        let n = self.blocks.len();
        let mut drafts = self.blocks;
        let mut terms: Vec<Terminator> = drafts
            .iter_mut()
            .map(|d| d.term.take().unwrap_or(Terminator::Unreachable))
            .collect();

        // A block with no statement, no label and only a jump out is one the cutting made: the arm
        // of an `if` that holds a `goto`, the join after arms that all leave. A jump to it goes
        // where it goes. A ring of such blocks is a loop that does nothing, and is left alone.
        let passes_on = |b: BlockId| match &terms[b] {
            Terminator::Jump(next) => {
                let made = b != ENTRY && drafts[b].at.is_none() && drafts[b].stmts.is_empty();
                (made && *next != b).then_some(*next)
            }
            _ => None,
        };
        let forward: Vec<BlockId> = (0..n)
            .map(|b| {
                let mut end = b;
                for _ in 0..n {
                    match passes_on(end) {
                        Some(next) => end = next,
                        None => return end,
                    }
                }
                b
            })
            .collect();
        for term in &mut terms {
            term.retarget(|t| forward[t]);
        }

        // Kept: what the entry reaches, what holds statements, and what either of those jumps to.
        let mut keep = vec![false; n];
        let mut stack: Vec<BlockId> = (0..n)
            .filter(|b| *b == ENTRY || !drafts[*b].stmts.is_empty())
            .collect();
        while let Some(b) = stack.pop() {
            if !std::mem::replace(&mut keep[b], true) {
                stack.extend(terms[b].successors());
            }
        }
        let mut renumbered = vec![usize::MAX; n];
        let mut kept = 0;
        for b in self.started.iter().copied() {
            if keep[b] {
                renumbered[b] = kept;
                kept += 1;
            }
        }
        let mut blocks = Vec::with_capacity(kept);
        for b in self.started.iter().copied() {
            if keep[b] {
                let mut term = std::mem::replace(&mut terms[b], Terminator::Unreachable);
                term.retarget(|t| renumbered[t]);
                blocks.push(Block {
                    at: drafts[b].at,
                    stmts: std::mem::take(&mut drafts[b].stmts),
                    term,
                });
            }
        }
        Ok(Cfg { blocks })
    }
}

/// Whether a loop condition is the listing's `Truth(true)`, or another constant that holds.
fn always(cond: &Expr) -> bool {
    match cond {
        Expr::Truth(e) => always(e),
        Expr::Const(c) => c.bits != 0,
        _ => false,
    }
}

#[cfg(test)]
#[path = "cfg_tests.rs"]
mod tests;
