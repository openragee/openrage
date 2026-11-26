//! Interproc: what the project knows of a callee, written into the calls of it.
//!
//! Works on the blocks, after the rounds of `values` and before `types`: a call of a function
//! of the same program states the cells it yields, a reading of a result as the kind its callee
//! proves it is goes, a variable that holds one such result takes its kind, and a constant passed
//! for a parameter the callee's signature types is written as that kind.
//!
//! ⚠ Every rewrite rests on a signature of the project. A result's kind there says that each
//! cell the callee can return is widened as that kind, which is all that makes a cast of it say
//! nothing: with the name off, or with a project that holds no signature, nothing is touched.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{
    BinOp, Expr, FunctionKey, Kind, Scalar, Stmt, UnOp, Var, VarInfo, Width,
};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::signatures::{float_bits, key_of, param_at, passed_on, settled};
use crate::gta6::script::project::{Param, Project};
use crate::gta6::script::{Context, Pass, Stats};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Interproc];

/// The choices one inside another whose constants a parameter's kind is carried into.
const MAX_CHOICES: u32 = 8;

/// What one run did, each a count of places.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// Calls given the cells their callee returns.
    pub sized: usize,
    /// Casts taken off a result that already is what they read it as.
    pub casts: usize,
    /// Tests of a truth against zero written as the truth.
    pub tests: usize,
    /// Variables and parameters given a kind.
    pub kinds: usize,
    /// Constants passed as arguments written as the kind their parameter is.
    pub consts: usize,
}

/// Fills each call's returned cells and gives each variable the kind a signature of the project proves for it.
pub fn run(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) {
    if !cx.options.on(Pass::Interproc) {
        return;
    }
    let done = apply(f, cx.project);
    tracing::trace!(
        function = %f.head.name,
        sized = done.sized,
        casts = done.casts,
        tests = done.tests,
        kinds = done.kinds,
        consts = done.consts,
        "interproc"
    );
}

/// [`run`] whatever the options say, with the count of what it did.
pub(crate) fn apply(f: &mut cfg::Function, project: &Project) -> Applied {
    let mut done = Applied::default();
    if project.signatures.is_empty() {
        return done;
    }
    done.sized = sized(f, project);
    done.consts = passed_constants(f, project);
    let program = f.head.program;
    let kinds = Kinds {
        project,
        program,
        proved: proved(f, project),
    };
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            match stmt {
                // ⚠ What a variable is assigned is what types it: the top of the value stays as
                // it reads, unless the variable's kind is proved here.
                Stmt::Assign { target, value } => {
                    rewrite(value, kinds.proved.contains_key(target), &kinds, &mut done);
                }
                Stmt::AssignMany { values, .. } => {
                    for value in values {
                        rewrite(value, false, &kinds, &mut done);
                    }
                }
                other => other.each_expr_mut(&mut |top| rewrite(top, true, &kinds, &mut done)),
            }
        }
        if let Some(e) = block.term.expr_mut() {
            rewrite(e, true, &kinds, &mut done);
        }
    }

    for (var, kind) in &kinds.proved {
        done.kinds += usize::from(declare(f, *var, kind));
    }
    // A parameter this code only passes on is typed by no reading of its own: its kind is the callees'.
    let own = project
        .signatures
        .get(&FunctionKey::new(program, f.head.at));
    let own = own
        .filter(|held| settled(held) && held.params.iter().any(|param| one_kind(param).is_some()));
    if let Some(own) = own {
        for cell in passed_on(f) {
            let param = param_at(own, cell).and_then(|n| own.params.get(n));
            if let Some(kind) = param.and_then(one_kind) {
                done.kinds += usize::from(declare(f, Var::Param(cell), kind));
            }
        }
    }
    done
}

/// States the cells each call of a function of a program yields, where the call does not and the project does; returns how many it stated.
// A count is no rewrite: it is what the callee's own head says, and what runs is the same.
pub(crate) fn sized(f: &mut cfg::Function, project: &Project) -> usize {
    let program = f.head.program;
    let mut stated = 0;
    let mut state = |top: &mut Expr| {
        top.walk_mut(&mut |e| {
            let Expr::Call {
                callee, returns, ..
            } = e
            else {
                return;
            };
            let held = key_of(program, callee).and_then(|key| project.signatures.get(&key));
            if let (None, Some(held)) = (*returns, held.filter(|held| settled(held))) {
                *returns = Some(held.returns);
                stated += 1;
            }
        });
    };
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut state);
        }
        if let Some(e) = block.term.expr_mut() {
            state(e);
        }
    }
    stated
}

/// Writes each constant passed for a parameter the callee's signature types as a `float` or a `bool` as that kind, where its bits are a value of it; returns how many it wrote so.
// ⚠ The bits stay and so does what the callee is handed: a constant's kind is only how it is
// written. An argument is matched to the parameter that starts at its own cell, so a value of
// several cells before it shifts nothing.
pub(crate) fn passed_constants(f: &mut cfg::Function, project: &Project) -> usize {
    let program = f.head.program;
    let cfg::Function { head, cfg: code } = f;
    let head = &*head;
    let mut written = 0;
    let mut write = |top: &mut Expr| {
        top.walk_mut(&mut |e| {
            let Expr::Call { callee, args, .. } = e else {
                return;
            };
            let held = key_of(program, callee).and_then(|key| project.signatures.get(&key));
            let Some(held) = held.filter(|held| settled(held)) else {
                return;
            };
            // The cell of the callee's parameters each argument starts at, while the arguments before it say their cells.
            let mut at = Some(0u32);
            for arg in args.iter_mut() {
                let Some(cell) = at else {
                    break;
                };
                at = cells_of(arg, head).and_then(|cells| cell.checked_add(cells));
                let param = param_at(held, cell).and_then(|n| held.params.get(n));
                if let Some(kind) = param.and_then(one_kind) {
                    written += written_as(arg, kind, 0);
                }
            }
        });
    };
    for block in &mut code.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut write);
        }
        if let Some(e) = block.term.expr_mut() {
            write(e);
        }
    }
    written
}

/// Gives the constants an argument comes to the kind `kind`, where each is a value of it; returns how many took it.
// A `float` is zero or the bits of a float a source would hold, a `bool` zero or one: any other
// integer stays as it is written, whatever the parameter is.
fn written_as(e: &mut Expr, kind: &Kind, depth: u32) -> usize {
    match e {
        Expr::Const(c) => {
            let fits = match kind {
                Kind::F32 => c.bits == 0 || float_bits(c.bits, false),
                Kind::Bool => c.bits <= 1,
                _ => false,
            };
            let takes = fits && c.kind.is_integer();
            if takes {
                c.kind = kind.clone();
            }
            usize::from(takes)
        }
        Expr::Select { then, els, .. } if depth < MAX_CHOICES => {
            written_as(then, kind, depth + 1) + written_as(els, kind, depth + 1)
        }
        _ => 0,
    }
}

/// The kind of a parameter of one cell, where a signature states one.
fn one_kind(param: &Param) -> Option<&Kind> {
    (param.cells == 1 && param.kind != Kind::Unknown).then_some(&param.kind)
}

/// Gives a variable of one cell nothing has typed the kind `kind`; returns whether it did.
fn declare(f: &mut cfg::Function, var: Var, kind: &Kind) -> bool {
    let info = f.head.vars.entry(var);
    let info = info.or_insert_with(|| VarInfo::new(Width::Cell));
    let untyped = info.width == Width::Cell && info.kind == Kind::Unknown;
    if untyped {
        info.kind = kind.clone();
    }
    untyped
}

/// The kinds one run reads off the project: of a call's result, and of a variable that holds one.
struct Kinds<'a> {
    project: &'a Project,
    program: u32,
    proved: BTreeMap<Var, Kind>,
}

impl Kinds<'_> {
    /// The kind `e` is widened as, where `e` is a result a signature types or a variable that holds one.
    fn of(&self, e: &Expr) -> Option<&Kind> {
        match e {
            Expr::Var(var) => self.proved.get(var),
            other => result_of(self.project, self.program, other),
        }
    }
}

/// The kind of the one cell a call of a function of a program yields, where a settled signature proves one.
fn result_of<'p>(project: &'p Project, program: u32, e: &Expr) -> Option<&'p Kind> {
    let Expr::Call { callee, .. } = e else {
        return None;
    };
    let held = project.signatures.get(&key_of(program, callee)?)?;
    let proved = settled(held) && held.returns == 1 && held.result != Kind::Unknown;
    proved.then_some(&held.result)
}

/// The variables that hold one result and nothing else: assigned once, by a call whose result has a kind, and written by nothing but that.
// ⚠ A local at or above one whose address is taken may be written through that address, and a
// parameter is the caller's: neither is proved.
fn proved(f: &cfg::Function, project: &Project) -> BTreeMap<Var, Kind> {
    let program = f.head.program;
    let mut held: BTreeMap<Var, Kind> = BTreeMap::new();
    let mut sets: HashMap<Var, u32> = HashMap::new();
    let mut addressed: HashSet<Var> = HashSet::new();
    let mut lowest_local: Option<u32> = None;
    let mut note = |top: &Expr| {
        top.walk(&mut |e| {
            if let Expr::AddrOf { var, .. } = e {
                addressed.insert(*var);
                if let Var::Local(n) = var {
                    lowest_local = Some(lowest_local.map_or(*n, |low| low.min(*n)));
                }
            }
        });
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            for target in stmt.targets() {
                *sets.entry(*target).or_insert(0) += 1;
            }
            if let Stmt::Assign { target, value } = stmt {
                let kind = match value {
                    Expr::Cast { to, e } => {
                        result_of(project, program, e).filter(|kind| silent(*to, kind))
                    }
                    other => result_of(project, program, other),
                };
                if let Some(kind) = kind {
                    held.insert(*target, kind.clone());
                }
            }
            stmt.each_expr(&mut note);
        }
        if let Some(e) = block.term.expr() {
            note(e);
        }
    }
    held.retain(|var, _| {
        let once = sets.get(var) == Some(&1);
        let sealed = match *var {
            Var::Local(n) => lowest_local.is_none_or(|low| n < low),
            Var::Value { .. } | Var::Stack { .. } => !addressed.contains(var),
            Var::Param(_) | Var::Global { .. } => false,
        };
        let info = f.head.vars.get(var);
        once && sealed && info.is_none_or(|info| info.width == Width::Cell)
    });
    held
}

/// Whether reading a cell that is widened as `kind` through `as_<to>` gives the same cell.
// Zero or one is itself under every integer reading; any other kind only under its own.
fn silent(to: Scalar, kind: &Kind) -> bool {
    to.kind() == *kind || (*kind == Kind::Bool && to.kind().is_integer())
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Const(c) if c.bits == 0 && !c.kind.is_float())
}

/// Takes an expression out of its place, leaving a placeholder the caller overwrites.
fn take(e: &mut Expr) -> Expr {
    std::mem::replace(e, Expr::GuidEmpty)
}

/// Which side of `l == r` or `l != r` is a truth held against zero: `true` for the left.
fn zero_test(kinds: &Kinds<'_>, l: &Expr, r: &Expr) -> Option<bool> {
    let truth = |e: &Expr| kinds.of(e) == Some(&Kind::Bool);
    if is_zero(r) && truth(l) {
        Some(true)
    } else if is_zero(l) && truth(r) {
        Some(false)
    } else {
        None
    }
}

/// Rewrites what stands under `top`, and `top` itself when `whole`: a cast that says nothing goes, and a truth held against zero is the truth.
// The cell each expression comes to stays the same: a truth is zero or one, so `t != 0` is `t`
// and `t == 0` is `!t`, cell for cell.
fn rewrite(top: &mut Expr, whole: bool, kinds: &Kinds<'_>, done: &mut Applied) {
    let root: *const Expr = &*top;
    top.walk_mut(&mut |e| {
        if !whole && std::ptr::eq(&*e, root) {
            return;
        }
        let says_nothing = matches!(
            &*e,
            Expr::Cast { to, e: inner } if kinds.of(inner).is_some_and(|kind| silent(*to, kind))
        );
        let tested = match &*e {
            Expr::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                at,
                l,
                r,
            } if !at.is_float() => zero_test(kinds, l, r).map(|left| (left, *op == BinOp::Eq)),
            _ => None,
        };
        if says_nothing {
            if let Expr::Cast { e: inner, .. } = e {
                let read = take(inner);
                *e = read;
                done.casts += 1;
            }
        } else if let Some((left, negated)) = tested {
            let truth = match e {
                Expr::Binary { l, .. } if left => take(l),
                Expr::Binary { r, .. } => take(r),
                _ => return,
            };
            *e = match negated {
                true => Expr::Unary {
                    op: UnOp::Not,
                    e: Box::new(truth),
                },
                false => truth,
            };
            done.tests += 1;
        }
    });
}

#[cfg(test)]
#[path = "interproc_tests.rs"]
mod tests;
