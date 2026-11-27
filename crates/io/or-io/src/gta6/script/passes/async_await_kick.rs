//! The kick-off of an `async` method: the function that makes the state machine, starts it and
//! hands back the builder's task.
//!
//! Two forms. One runs the step function's code in line on a machine it holds in a local: `found`
//! proves its start, and `untangle` then reads the rest as it reads a step function, through the
//! variable that holds the machine's address. The other calls its step function: `joined` puts
//! that function's body where the call stood. Either way the method reads as one body, with what
//! makes the machine out of sight.
//!
//! ⚠ A statement that copies a parameter into the machine stays in sight: the body reads the
//! machine's field, and nothing here proves the field never changes after it.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg::{self, Block, Terminator};
use crate::gta6::script::ir::{Callee, Expr, Head, Returns, Stmt, StoreKind, Var, VarInfo};
use crate::gta6::script::tree::{self, Sugar};
use crate::gta6::script::{Context, Pass};

use super::{
    constant, entry_read, field_read, framed, is_step, place, rooted, state_read, unwide, Base,
    Held, Reach, BUILDER, FIELDS, RUNNING, TASK,
};

/// The assignments of an entry looked at for the one that keeps the machine's address.
const CANDIDATES: usize = 16;
/// The statements a kick-off that calls its step function may hold; a longer body is none.
const STUB: usize = 64;

/// A kick-off that runs its step function's code in line, as `found` proved its start.
pub(super) struct Kick {
    pub reach: Reach,
    /// The local the machine is in.
    storage: Var,
    /// One flag for each statement of the entry before the step's own reads: whether the compiler alone wrote it.
    pub hide: Vec<bool>,
    /// What every way out returns: the builder's task, or nothing.
    pub leave: Option<Expr>,
}

impl Kick {
    /// What `run` needs of the kick-off once the blocks are a body again.
    pub fn settle(self) -> Settle {
        Settle {
            pointer: self.reach.pointer,
            storage: self.storage,
            leave: self.leave,
        }
    }
}

/// Where a kick-off holds its machine, and what it returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Settle {
    pointer: Var,
    storage: Var,
    leave: Option<Expr>,
}

/// Whether `value` is made of cleared cells alone.
fn blank(value: &Expr, zeros: &BTreeSet<Var>) -> bool {
    let mut blank = true;
    value.walk(&mut |node| {
        blank &= match node {
            Expr::Var(var) => zeros.contains(var),
            Expr::Const(c) => c.bits == 0,
            Expr::Cell { .. }
            | Expr::Cells(_)
            | Expr::Cast { .. }
            | Expr::Sugar { .. }
            | Expr::ZeroSlots(_)
            | Expr::GuidEmpty => true,
            _ => false,
        };
    });
    blank
}

/// Whether `value` is one or more parameters as they were given, under whatever reading.
fn given(value: &Expr) -> bool {
    let (mut only, mut any) = (true, false);
    value.walk(&mut |node| match node {
        Expr::Var(Var::Param(_)) => any = true,
        Expr::Cell { .. } | Expr::Cells(_) | Expr::Cast { .. } | Expr::Sugar { .. } => {}
        _ => only = false,
    });
    only && any
}

/// Reads the statements that make a kick-off's machine; for each, whether the compiler alone wrote it. `None` where one does anything else.
// What the compiler writes: the builder cleared, the running state stored once, and addresses
// of the machine kept in variables. What the source wrote, in its parameter list: each
// parameter copied to a field of its own.
fn prelude<'a>(stmts: impl Iterator<Item = &'a Stmt>, reach: Reach) -> Option<Vec<bool>> {
    let mut held = Held::of(reach);
    let mut zeros: BTreeSet<Var> = BTreeSet::new();
    let mut fields: BTreeSet<u32> = BTreeSet::new();
    let mut running = 0u32;
    let mut hide = Vec::new();
    for stmt in stmts {
        let hidden = match stmt {
            Stmt::Assign { target, value } if framed(*target) && Some(*target) != reach.storage => {
                match rooted(value, &held) {
                    Some(root) => {
                        zeros.remove(target);
                        held.vars.insert(*target, root);
                    }
                    // A variable given cleared cells holds what it held when the function started.
                    None if blank(value, &zeros) => {
                        held.vars.remove(target);
                        zeros.insert(*target);
                    }
                    None => return None,
                }
                true
            }
            Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, cell: 0 },
                ..
            } if zeros.contains(var) => true,
            Stmt::ZeroBytes { addr, bytes } => {
                let builder = place(addr, 0, &held) == Some((Base::Machine, BUILDER));
                if !builder || *bytes != FIELDS - BUILDER {
                    return None;
                }
                true
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } => match place(addr, *offset, &held)? {
                (Base::Machine, BUILDER) if *bytes == FIELDS - BUILDER && blank(value, &zeros) => {
                    true
                }
                (Base::Machine, at) if at >= FIELDS && given(value) && fields.insert(at) => false,
                _ => return None,
            },
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => match place(addr, *offset, &held)? {
                (Base::Machine, 0) => {
                    let runs = constant(value).is_some_and(|c| c.bits as i32 == RUNNING);
                    if *kind != StoreKind::I32 || !runs {
                        return None;
                    }
                    running += 1;
                    true
                }
                (Base::Machine, BUILDER | TASK) => {
                    if kind.bytes() != 8 || !blank(value, &zeros) {
                        return None;
                    }
                    true
                }
                (Base::Machine, at) if at >= FIELDS && given(value) && fields.insert(at) => false,
                _ => return None,
            },
            _ => return None,
        };
        hide.push(hidden);
    }
    (running == 1).then_some(hide)
}

/// Whether a statement reads `var`.
fn reads(stmt: &Stmt, var: Var) -> bool {
    let mut read = false;
    stmt.each_expr(&mut |top| top.walk(&mut |e| read |= matches!(e, Expr::Var(v) if *v == var)));
    read
}

/// Whether a statement writes over the cell of the local `var` through the address of a local before it.
// ⚠ A value of several cells is one local by the name of its first cell, and a clear or a store
// through that name runs on into the cells after it.
fn writes_over(stmt: &Stmt, var: Var) -> bool {
    let Var::Local(at) = var else {
        return false;
    };
    let (addr, from, bytes) = match stmt {
        Stmt::ZeroBytes { addr, bytes } => (addr, 0, *bytes),
        Stmt::Store {
            kind, addr, offset, ..
        } => (addr, *offset, kind.bytes()),
        Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } => (addr, *offset, *bytes),
        _ => return false,
    };
    let Expr::AddrOf {
        var: Var::Local(first),
        cell: 0,
    } = addr
    else {
        return false;
    };
    let end = u64::from(from) + u64::from(bytes);
    let cells = u64::from(*first) + u64::from(from / 8)..u64::from(*first) + end.div_ceil(8);
    cells.contains(&u64::from(at))
}

/// Whether statements and tests use a kick-off's two variables as a kick-off does: the pointer is given a value once, is written over by nothing and its own address is never taken, and the local the machine is in is reached by its address alone.
fn kept<'a>(
    stmts: impl Iterator<Item = &'a Stmt>,
    tests: impl Iterator<Item = &'a Expr>,
    reach: Reach,
) -> bool {
    let (mut given, mut set, mut named) = (0usize, false, false);
    let mut look = |top: &'a Expr| {
        top.walk(&mut |e| match e {
            Expr::AddrOf { var, .. } if *var == reach.pointer => named = true,
            Expr::Var(var) if Some(*var) == reach.storage => named = true,
            _ => {}
        });
    };
    for stmt in stmts {
        for target in stmt.targets() {
            given += usize::from(*target == reach.pointer);
            set |= Some(*target) == reach.storage;
        }
        set |= writes_over(stmt, reach.pointer);
        stmt.each_expr(&mut look);
    }
    for test in tests {
        look(test);
    }
    given == 1 && !set && !named
}

/// Whether statements or tests name one of `vars`: assign it, read it or take its address.
fn names<'a>(
    stmts: impl Iterator<Item = &'a Stmt>,
    tests: impl Iterator<Item = &'a Expr>,
    vars: &BTreeSet<Var>,
) -> bool {
    let (mut set, mut named) = (false, false);
    let mut look = |top: &'a Expr| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                named |= vars.contains(var);
            }
        });
    };
    for stmt in stmts {
        set |= stmt.targets().iter().any(|var| vars.contains(var));
        stmt.each_expr(&mut look);
    }
    for test in tests {
        look(test);
    }
    set || named
}

/// The kick-off a function is, where its entry makes a machine in a local and goes on as a step function does on it.
pub(super) fn found(f: &cfg::Function) -> Option<Kick> {
    let entry = f.cfg.blocks.first()?;
    let keeps = |(at, stmt): (usize, &Stmt)| match stmt {
        Stmt::Assign {
            target: target @ Var::Local(_),
            value:
                Expr::AddrOf {
                    var: storage @ Var::Local(_),
                    cell: 0,
                },
        } if target != storage => Some((at, *target, *storage)),
        _ => None,
    };
    let candidates = entry.stmts.iter().enumerate().filter_map(keeps);
    for (at, pointer, storage) in candidates.take(CANDIDATES) {
        let unread = Reach {
            pointer,
            storage: Some(storage),
            state: None,
        };
        let reach = Reach {
            state: state_read(&entry.stmts, unread).map(|(_, var)| var),
            ..unread
        };
        if is_step(f, reach) {
            return kicked(f, reach, storage, at);
        }
    }
    None
}

/// Proves the start and the ways out of a kick-off whose entry keeps the machine's address at statement `at`.
fn kicked(f: &cfg::Function, reach: Reach, storage: Var, at: usize) -> Option<Kick> {
    let blocks = &f.cfg.blocks;
    let entry = blocks.first()?;
    let none = Held::of(reach);
    // Every way out returns the same thing: the builder's task, or nothing from a method that hands nothing back.
    let mut returned = blocks.iter().filter_map(|block| match &block.term {
        Terminator::Return(value) => Some(value),
        _ => None,
    });
    let leave = returned.next()?.clone();
    if returned.any(|other| *other != leave) {
        return None;
    }
    match (&leave, f.head.returns) {
        (None, Returns::Void) => {}
        (Some(task), Returns::Cell) if field_read(task, &none) == Some(TASK) => {}
        _ => return None,
    }
    // The step's own entry is the reads of the machine the entry ends in; what makes the machine is before them.
    let own = entry.stmts.iter().rev();
    let cut = entry.stmts.len() - own.take_while(|stmt| entry_read(stmt, &none)).count();
    if at >= cut
        || entry.stmts[..at]
            .iter()
            .any(|stmt| reads(stmt, reach.pointer))
    {
        return None;
    }
    let stmts = blocks.iter().flat_map(|block| &block.stmts);
    let tests = blocks.iter().filter_map(|block| block.term.expr());
    if !kept(stmts, tests, reach) {
        return None;
    }
    let hide = prelude(entry.stmts[..cut].iter(), reach)?;
    // ⚠ What the hidden statements give a variable is out of sight with them: but for the
    // machine's address, which `settle` writes out, nothing after them may name such a variable.
    let mut made: BTreeSet<Var> = BTreeSet::new();
    for stmt in &entry.stmts[..cut] {
        made.extend(stmt.targets().iter().filter(|var| **var != reach.pointer));
    }
    let rest = blocks.iter().skip(1).flat_map(|block| &block.stmts);
    let later = entry.stmts[cut..].iter().chain(rest);
    let tests = blocks.iter().filter_map(|block| block.term.expr());
    if names(later, tests, &made) {
        return None;
    }
    Some(Kick {
        reach,
        storage,
        hide,
        leave,
    })
}

/// Makes each jump to a `return` that stands alone the `return` itself.
// A kick-off's suspend paths and its end all go to the one block that returns the task, where
// a step function's each return for themselves: after this the two read alike.
pub(super) fn returned(blocks: &mut [Block], leave: &Terminator) {
    let alone: Vec<bool> = blocks
        .iter()
        .map(|block| block.stmts.is_empty() && block.term == *leave)
        .collect();
    for block in blocks.iter_mut() {
        let leaves = matches!(block.term, Terminator::Jump(to) if alone.get(to) == Some(&true));
        if leaves {
            block.term = leave.clone();
        }
    }
}

/// Writes the machine's address where the variable that holds it is read.
fn pointed(top: &mut Expr, pointer: Var, storage: Var) {
    top.walk_mut(&mut |e| {
        if matches!(e, Expr::Var(var) if *var == pointer) {
            *e = Expr::AddrOf {
                var: storage,
                cell: 0,
            };
        }
    });
}

/// Writes a kick-off's body by the local its machine is in, and takes the `return` of the task it ends in out of sight.
// The variable that holds the machine's address is given it once, by a statement now hidden:
// read through it, the fields of the machine would be fields of nothing in sight.
// ⚠ Only the `return` the body ends in is hidden: one elsewhere leaves the method early, and
// with nothing written there the statements after it would read as if they ran.
pub(super) fn settle(body: &mut Vec<tree::Stmt>, kick: &Settle) {
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut *body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| pointed(top, kick.pointer, kick.storage));
            work.extend(tree::nested_mut(stmt));
        }
    }
    let Some(task) = &kick.leave else {
        return;
    };
    let mut task = task.clone();
    pointed(&mut task, kick.pointer, kick.storage);
    if matches!(body.last(), Some(tree::Stmt::Return(Some(value))) if *value == task) {
        let last = body.pop();
        body.push(tree::Stmt::Sugar(Box::new(Sugar::Hidden(
            last.into_iter().collect(),
        ))));
    }
}

/// A kick-off that calls its step function, as `stub` read it.
struct Stub {
    /// The offset of the step function it calls.
    step: u32,
    storage: Var,
    /// One flag for each statement before the call: whether the compiler alone wrote it.
    hide: Vec<bool>,
    leave: Option<Expr>,
}

/// The kick-off a finished function is: statements that make a machine in a local, one call that is handed its address, and the builder's task returned.
fn stub(f: &tree::Function) -> Option<Stub> {
    if f.head.incomplete.is_some() {
        return None;
    }
    let (leave, rest) = match f.body.split_last() {
        Some((tree::Stmt::Return(value), rest)) => (value.clone(), rest),
        _ => (None, f.body.as_slice()),
    };
    if rest.len() > STUB {
        return None;
    }
    let mut simples: Vec<&Stmt> = Vec::with_capacity(rest.len());
    for stmt in rest {
        match stmt {
            tree::Stmt::Simple(simple) => simples.push(simple),
            _ => return None,
        }
    }
    let (call, before) = simples.split_last()?;
    let Stmt::Eval(called) = call else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Local { at: step, .. },
        args,
        ..
    } = unwide(called)
    else {
        return None;
    };
    let [handed] = args.as_slice() else {
        return None;
    };
    let Expr::Var(pointer @ Var::Local(_)) = unwide(handed) else {
        return None;
    };
    let keeps = |stmt: &&Stmt| match stmt {
        Stmt::Assign {
            target,
            value:
                Expr::AddrOf {
                    var: storage @ Var::Local(_),
                    cell: 0,
                },
        } if target == pointer && storage != pointer => Some(*storage),
        _ => None,
    };
    let made = before.iter().position(|stmt| keeps(stmt).is_some())?;
    let storage = keeps(&before[made])?;
    let reach = Reach {
        pointer: *pointer,
        storage: Some(storage),
        state: None,
    };
    let none = Held::of(reach);
    match (&leave, f.head.returns) {
        (None, Returns::Void) => {}
        (Some(task), Returns::Cell) if field_read(task, &none) == Some(TASK) => {}
        _ => return None,
    }
    let early = before[..made].iter().any(|stmt| reads(stmt, *pointer));
    if early || !kept(before.iter().copied(), std::iter::empty(), reach) {
        return None;
    }
    let hide = prelude(before.iter().copied(), reach)?;
    Some(Stub {
        step: *step,
        storage,
        hide,
        leave,
    })
}

/// Whether a body is a step function's as `untangle` proved it on its one parameter: an await of it sets the state to running through that parameter.
fn steps(body: &[tree::Stmt]) -> bool {
    let none = Held::of(Reach::STEP);
    let runs = |stmt: &tree::Stmt| match stmt {
        tree::Stmt::Simple(Stmt::Store {
            kind: StoreKind::I32,
            addr,
            offset,
            value,
        }) => {
            place(addr, *offset, &none) == Some((Base::Machine, 0))
                && constant(value).is_some_and(|c| c.bits as i32 == RUNNING)
        }
        _ => false,
    };
    let mut found = false;
    tree::each_sugar(body, &mut |sugar| {
        if let Sugar::Await { resume, .. } = sugar {
            found |= resume.iter().any(&runs);
        }
    });
    found
}

/// Calls `f` on every variable a body assigns, reads or takes the address of.
fn each_var(body: &[tree::Stmt], f: &mut impl FnMut(Var)) {
    tree::each_simple(body, &mut |stmt| {
        stmt.targets().iter().for_each(|var| f(*var))
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                f(*var);
            }
        });
    });
}

/// A step function's body on its way into the frame of the kick-off that calls it.
struct Move<'a> {
    /// The cells the step function's locals go up by: past every local of the kick-off.
    by: u32,
    /// The local of the kick-off the machine is in, whose address the step function's parameter is.
    storage: Var,
    /// The variables the kick-off names.
    mine: &'a BTreeSet<Var>,
    /// Cleared where a variable cannot be moved.
    fits: bool,
    /// Whether the body goes back out of the kick-off's frame into the step function's own.
    back: bool,
}

impl Move<'_> {
    /// Where a variable of the step function stands in the kick-off's frame, or back in its own.
    fn var(&mut self, var: &mut Var) {
        match *var {
            Var::Local(n) => {
                let moved = match self.back {
                    false => n.checked_add(self.by),
                    true => n.checked_sub(self.by),
                };
                match moved {
                    Some(moved) => *var = Var::Local(moved),
                    None => self.fits = false,
                }
            }
            // The one parameter is the machine's address, written as that where it is read.
            Var::Param(_) => self.fits = false,
            Var::Value { .. } | Var::Stack { .. } => self.fits &= !self.mine.contains(var),
            Var::Global { .. } => {}
        }
    }

    fn expr(&mut self, top: &mut Expr) {
        let (storage, back) = (self.storage, self.back);
        top.walk_mut(&mut |e| match e {
            Expr::Var(Var::Param(0)) if !back => {
                *e = Expr::AddrOf {
                    var: storage,
                    cell: 0,
                };
            }
            Expr::AddrOf { var, cell: 0 } if back && *var == storage => {
                *e = Expr::Var(Var::Param(0));
            }
            Expr::Var(var) | Expr::AddrOf { var, .. } => self.var(var),
            _ => {}
        });
    }

    fn assigned(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Assign { target, .. } => self.var(target),
            Stmt::AssignMany { targets, .. } => {
                for target in targets {
                    self.var(target);
                }
            }
            _ => {}
        }
    }

    /// Moves every variable of `body`, and makes each `return` hand back what the kick-off does.
    fn body(&mut self, body: &mut Vec<tree::Stmt>, leave: &Option<Expr>) {
        // An explicit stack: a body nests as deep as its listing does.
        let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
        while let Some(list) = work.pop() {
            for stmt in list {
                // Going back, what the kick-off returns is its own expression, and is not moved.
                if let (true, tree::Stmt::Return(value)) = (self.back, &mut *stmt) {
                    match *value == *leave {
                        true => *value = None,
                        false => self.fits = false,
                    }
                }
                tree::own_exprs_mut(stmt, &mut |top| self.expr(top));
                match stmt {
                    tree::Stmt::Simple(simple) => self.assigned(simple),
                    tree::Stmt::For { init, step, .. } => {
                        for held in init.iter_mut().chain(step.iter_mut()) {
                            self.assigned(held);
                        }
                    }
                    // A step function returns nothing; the kick-off's task is the kick-off's own expression, and is not moved.
                    tree::Stmt::Return(value) if !self.back => match value {
                        Some(_) => self.fits = false,
                        None => *value = leave.clone(),
                    },
                    tree::Stmt::Sugar(sugar) => {
                        // ⚠ A construct this file does not know may hold a variable outside its parts, which would stay unmoved.
                        self.fits &= matches!(
                            **sugar,
                            Sugar::Hidden(_)
                                | Sugar::Group { .. }
                                | Sugar::Await { .. }
                                | Sugar::Yield { .. }
                                | Sugar::Foreach { .. }
                                | Sugar::Guarded { .. }
                                | Sugar::Try { .. }
                                | Sugar::Throw(_)
                                | Sugar::SwitchOn { .. }
                                | Sugar::Event { .. }
                                | Sugar::Cached { .. }
                        );
                        if let Sugar::Try { catches, .. } = &mut **sugar {
                            for caught in catches.iter_mut() {
                                if let Some(var) = &mut caught.var {
                                    self.var(var);
                                }
                            }
                        }
                        for held in sugar.parts_mut().simples {
                            self.assigned(held);
                        }
                    }
                    _ => {}
                }
                work.extend(tree::nested_mut(stmt));
            }
        }
    }
}

/// The kick-off `kick` with the body of the step function it calls in the place of the call.
// ⚠ This is the call written out: the step function's locals go after the kick-off's, where
// they start as zero as a call's do, its parameter is the address the call was handed, and each
// of its `return` statements returns what the kick-off returns after the call.
fn inlined(kick: &tree::Function, stub: &Stub, step: &tree::Function) -> Option<Join> {
    let head = &step.head;
    let fits = head.params == 1
        && head.returns == Returns::Void
        && head.incomplete.is_none()
        && steps(&step.body);
    if !fits {
        return None;
    }
    let mut mine: BTreeSet<Var> = kick.head.vars.keys().copied().collect();
    each_var(&kick.body, &mut |var| {
        mine.insert(var);
    });
    let mut by = kick.head.locals;
    for var in &mine {
        if let Var::Local(n) = var {
            by = by.max(n.checked_add(1)?);
        }
    }

    let mut moved = Move {
        by,
        storage: stub.storage,
        mine: &mine,
        fits: true,
        back: false,
    };
    let mut body = step.body.clone();
    moved.body(&mut body, &stub.leave);
    let mut vars: BTreeMap<Var, VarInfo> = BTreeMap::new();
    for (var, info) in &head.vars {
        let mut var = *var;
        if matches!(var, Var::Param(_)) {
            continue;
        }
        moved.var(&mut var);
        // A name was chosen among the step function's own: here it may be another variable's.
        let info = VarInfo {
            name: None,
            ..info.clone()
        };
        vars.insert(var, info);
    }
    if !moved.fits {
        return None;
    }

    let mut head = kick.head.clone();
    head.locals = by.checked_add(step.head.locals)?;
    head.vars.extend(vars);
    let mut out: Vec<tree::Stmt> = Vec::new();
    let mut run: Vec<tree::Stmt> = Vec::new();
    let hidden = |run: &mut Vec<tree::Stmt>, out: &mut Vec<tree::Stmt>| {
        if !run.is_empty() {
            let run = Sugar::Hidden(std::mem::take(run));
            out.push(tree::Stmt::Sugar(Box::new(run)));
        }
    };
    for (stmt, hide) in kick.body.iter().zip(&stub.hide) {
        match *hide {
            true => run.push(stmt.clone()),
            false => {
                hidden(&mut run, &mut out);
                out.push(stmt.clone());
            }
        }
    }
    hidden(&mut run, &mut out);
    let front = out.len();
    out.extend(body);
    if stub.leave.is_some() {
        let back = tree::Stmt::Return(stub.leave.clone());
        out.push(tree::Stmt::Sugar(Box::new(Sugar::Hidden(vec![back]))));
    }
    let call = stub.hide.len();
    Some(Join {
        kick: 0,
        step: 0,
        whole: tree::Function { head, body: out },
        front,
        by,
        storage: stub.storage,
        leave: stub.leave.clone(),
        call: kick.body.get(call)?.clone(),
        tail: kick.body.get(call + 1..)?.to_vec(),
        heads: [kick.head.clone(), step.head.clone()],
    })
}

/// A kick-off read as one method with the body of the step function it calls, and what gives the two back.
pub struct Join {
    /// The places of the kick-off and of the step function among the functions joined.
    pub kick: usize,
    pub step: usize,
    /// The method the two are read as.
    pub whole: tree::Function,
    /// The statements of `whole` that are the kick-off's, before the step function's body.
    front: usize,
    by: u32,
    storage: Var,
    leave: Option<Expr>,
    /// The kick-off's call of the step function, and what follows it.
    call: tree::Stmt,
    tail: Vec<tree::Stmt>,
    heads: [Head; 2],
}

impl Join {
    /// The way back: the kick-off and the step function as `whole` holds them, the call written where the step function's body stands; `None` where the body does not go back into a frame of its own.
    // ⚠ Read from `whole` itself, so a join that lost or moved a statement or a variable gives
    // back other functions than the two it was made of.
    pub fn parts(&self) -> Option<(tree::Function, tree::Function)> {
        let ends = usize::from(self.leave.is_some());
        let split = self.whole.body.len().checked_sub(ends)?;
        let (front, body) = (
            self.whole.body.get(..self.front)?,
            self.whole.body.get(self.front..split)?,
        );
        let mut kick: Vec<tree::Stmt> = Vec::with_capacity(front.len() + 1 + self.tail.len());
        for stmt in front {
            match stmt {
                tree::Stmt::Sugar(sugar) => match &**sugar {
                    Sugar::Hidden(run) => kick.extend(run.iter().cloned()),
                    _ => kick.push(stmt.clone()),
                },
                _ => kick.push(stmt.clone()),
            }
        }
        kick.push(self.call.clone());
        kick.extend(self.tail.iter().cloned());
        let mut mine: BTreeSet<Var> = self.heads[0].vars.keys().copied().collect();
        each_var(&kick, &mut |var| {
            mine.insert(var);
        });
        let mut moved = Move {
            by: self.by,
            storage: self.storage,
            mine: &mine,
            fits: true,
            back: true,
        };
        let mut step = body.to_vec();
        moved.body(&mut step, &self.leave);
        if !moved.fits {
            return None;
        }
        let [kick_head, step_head] = self.heads.clone();
        let kick = tree::Function {
            head: kick_head,
            body: kick,
        };
        Some((
            kick,
            tree::Function {
                head: step_head,
                body: step,
            },
        ))
    }
}

/// The kick-offs of a program's finished functions that [`joined`] reads as one method with the step function each calls.
pub fn joins(functions: &[tree::Function], cx: &Context<'_>) -> Vec<Join> {
    if !cx.options.on(Pass::Await) {
        return Vec::new();
    }
    let by_place: BTreeMap<(u32, u32), usize> = functions
        .iter()
        .enumerate()
        .map(|(index, f)| ((f.head.program, f.head.at), index))
        .collect();
    let mut read: Vec<(usize, usize, Stub)> = Vec::new();
    let mut callers: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, f) in functions.iter().enumerate() {
        let Some(stub) = stub(f) else {
            continue;
        };
        let step = by_place.get(&(f.head.program, stub.step)).copied();
        if let Some(step) = step.filter(|step| *step != index) {
            *callers.entry(step).or_default() += 1;
            read.push((index, step, stub));
        }
    }
    // Every kick-off is read against the functions as they came: one joined is no step function to another.
    let mut done = Vec::new();
    for (index, step, stub) in &read {
        if callers.get(step) != Some(&1) {
            continue;
        }
        if let Some(join) = inlined(&functions[*index], stub, &functions[*step]) {
            done.push(Join {
                kick: *index,
                step: *step,
                ..join
            });
        }
    }
    done
}

/// A program's finished functions, each kick-off that calls a step function of the program read as the one `async` method the two were compiled from.
// ⚠ The step function stays where it was: the runtime calls it again at every resumed await,
// and nothing here says no other function does.
// A step function two kick-offs call is joined to neither: a method has one kick-off, and each
// body is then copied once at the most.
pub fn joined(functions: Vec<tree::Function>, cx: &Context<'_>) -> Vec<tree::Function> {
    let mut functions = functions;
    for join in joins(&functions, cx) {
        functions[join.kick] = join.whole;
    }
    functions
}
