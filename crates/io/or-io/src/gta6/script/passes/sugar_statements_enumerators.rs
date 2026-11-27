//! A struct enumerator read back as `foreach`: a value zeroed, made from the collection by one
//! script function, copied into a frame variable, advanced each round by a method of its value
//! type and read for the item at the top of the round.
//!
//! ⚠ Nothing names the members: what proves the pair is the project's word that the advance is a
//! virtual method of a value type, that it and the make touch only the enumerator's bytes and keep
//! no address, and that every such loop of the function over that type takes the same pair.

use std::collections::BTreeMap;

use super::{labelled, suspends, tested, under, used, var_of, Scope, Use};
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{self, Expr, FunctionKey, Head, TypeKey, Var, Width};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::reach::{Mode, UNBOUNDED};
use crate::gta6::script::project::signatures::key_of;
use crate::gta6::script::project::Project;
use crate::gta6::script::tree::{self, Stmt, Sugar};

/// What is done with one taking of a variable's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Taken {
    /// Read through at once by a load: nothing is written through it.
    Loaded,
    /// Handed to a script function, landing in its parameter cell `at`.
    Handed { to: FunctionKey, at: u32 },
    /// Kept, stored through, moved, or handed to what the project keeps no reach of.
    Loose,
}

/// Each taking of an address in `body`, by the variable and the cell of it taken.
pub(super) fn addresses(body: &[Stmt], head: &Head) -> BTreeMap<Var, Vec<(u32, Taken)>> {
    let mut found: BTreeMap<Var, Vec<(u32, Taken)>> = BTreeMap::new();
    tree::each_expr(body, &mut |top| {
        // `walk` meets a node before what it holds: a load or a call says what its operands are first.
        let mut known: BTreeMap<*const Expr, Taken> = BTreeMap::new();
        top.walk(&mut |e| match e {
            Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } => {
                known.insert(std::ptr::from_ref(under(addr, false)), Taken::Loaded);
            }
            Expr::Call { callee, args, .. } => {
                let Some(to) = key_of(head.program, callee) else {
                    return;
                };
                // The cell an argument lands in: the cells of the arguments before it.
                let mut at = Some(0u32);
                for arg in args {
                    if let Some(cell) = at {
                        let handed = Taken::Handed { to, at: cell };
                        known.insert(std::ptr::from_ref(under(arg, false)), handed);
                    }
                    at = at
                        .zip(cells_of(arg, head))
                        .and_then(|(a, b)| a.checked_add(b));
                }
            }
            Expr::AddrOf { var, cell } => {
                let taken = known.get(&std::ptr::from_ref(e)).copied();
                let taken = taken.unwrap_or(Taken::Loose);
                found.entry(*var).or_default().push((*cell, taken));
            }
            _ => {}
        });
    });
    found
}

/// How the item is read from the enumerator at the top of each round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Read {
    /// The bytes at an offset of it.
    Field { offset: u32, bytes: u32 },
    /// A script function handed its address alone.
    Getter(FunctionKey),
}

/// The variables and functions of one enumerator loop as it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shape {
    /// The value zeroed and made.
    made: Var,
    /// The cells it is zeroed as.
    cells: u32,
    make: FunctionKey,
    /// The variable the value is copied into and advanced in.
    held: Var,
    advance: FunctionKey,
    item: Var,
    read: Read,
}

impl Shape {
    /// What every such loop over one type must take alike.
    pub(super) fn pair(&self) -> (FunctionKey, FunctionKey, Read) {
        (self.make, self.advance, self.read)
    }

    pub(super) fn held(&self) -> Var {
        self.held
    }

    pub(super) fn advance(&self) -> FunctionKey {
        self.advance
    }
}

/// The variable `e` is the address of the first cell of, under the source readings.
fn address_of(e: &Expr) -> Option<Var> {
    match under(e, false) {
        Expr::AddrOf { var, cell: 0 } => Some(*var),
        _ => None,
    }
}

/// `e` under the source readings and every cast.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. } | Expr::Cast { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// The shape of `list[at - 3..=at]` where it is `v = default; make(&v, source, ..); e = v; while (advance(&e) != 0) { x = <read of e>; .. }`.
pub(super) fn shape(list: &[Stmt], at: usize, program: u32) -> Option<Shape> {
    let first = at.checked_sub(3)?;
    let [Stmt::Simple(zero), Stmt::Simple(make), Stmt::Simple(copy), Stmt::While { cond, body }] =
        list.get(first..=at)?
    else {
        return None;
    };
    let ir::Stmt::Assign {
        target: made,
        value,
    } = zero
    else {
        return None;
    };
    let Expr::ZeroSlots(cells) = under(value, false) else {
        return None;
    };
    let ir::Stmt::Eval(call) = make else {
        return None;
    };
    let Expr::Call { callee, args, .. } = under(call, false) else {
        return None;
    };
    // The value made, and at least the collection it is made from.
    let [into, _, ..] = args.as_slice() else {
        return None;
    };
    if address_of(into) != Some(*made) {
        return None;
    }
    let make = key_of(program, callee)?;
    let ir::Stmt::Assign {
        target: held,
        value,
    } = copy
    else {
        return None;
    };
    if var_of(value, false) != Some(*made) || *held == *made || !matches!(held, Var::Local(_)) {
        return None;
    }
    let Expr::Call { callee, args, .. } = tested(cond) else {
        return None;
    };
    let [on] = args.as_slice() else {
        return None;
    };
    if address_of(on) != Some(*held) {
        return None;
    }
    let advance = key_of(program, callee)?;
    let [Stmt::Simple(ir::Stmt::Assign {
        target: item,
        value,
    }), ..] = body.as_slice()
    else {
        return None;
    };
    let read = match bare(value) {
        Expr::Load { ty, addr, offset } if address_of(addr) == Some(*held) => Read::Field {
            offset: *offset,
            bytes: ty.bytes(),
        },
        Expr::LoadBytes {
            addr,
            offset,
            bytes,
        } if address_of(addr) == Some(*held) => Read::Field {
            offset: *offset,
            bytes: *bytes,
        },
        Expr::Call { callee, args, .. } => match args.as_slice() {
            [on] if address_of(on) == Some(*held) => Read::Getter(key_of(program, callee)?),
            _ => return None,
        },
        _ => return None,
    };
    Some(Shape {
        made: *made,
        cells: *cells,
        make,
        held: *held,
        advance,
        item: *item,
        read,
    })
}

/// The value type `function` is a method of that takes a receiver, where the project proves one.
pub(super) fn owner(project: &Project, function: FunctionKey, roles: &[Role]) -> Option<TypeKey> {
    let member = project.members.get(&function)?;
    let virtual_method = !member.is_static && member.slot.is_some() && roles.contains(&member.role);
    let value = project.types.get(&member.owner)?.is_value();
    (virtual_method && value).then_some(member.owner)
}

/// Whether the address in parameter cell `cell` of `function` reaches no more than `room` bytes and is kept by nothing past the call.
fn within(project: &Project, function: FunctionKey, cell: u32, room: u32) -> bool {
    let bounded = (project.reach(function, cell)).is_some_and(|b| b != UNBOUNDED && b <= room);
    bounded && project.address_dies(function, cell) == Some(true)
}

/// [`within`], with the first parameter used as `mode` says.
fn used_as(project: &Project, function: FunctionKey, room: u32, mode: Mode) -> bool {
    within(project, function, 0, room) && project.param_mode(function, 0) == Some(mode)
}

/// Whether nothing but its own loops reaches the frame cells `held` runs across: every address of a cell below it taken in the function is read through at once or handed to a function that reaches short of it, and no address of a later cell of it is taken by another name.
// ⚠ A callee given the address of a frame cell reaches the cells after it; a parameter's are
// taken to run on into the locals.
fn private(scope: &Scope<'_>, held: Var, cells: u32) -> bool {
    let Var::Local(first) = held else {
        return false;
    };
    let end = first.saturating_add(cells);
    let project = scope.cx.project;
    let params = scope.head.params;
    let short = |taken: Taken, room: u32| match taken {
        Taken::Loaded => true,
        Taken::Handed { to, at } => within(project, to, at, room),
        Taken::Loose => false,
    };
    scope.addresses.iter().all(|(var, takings)| {
        takings.iter().all(|(cell, taken)| match *var {
            // Its own are counted against its loops.
            var if var == held => true,
            Var::Local(m) => match m.saturating_add(*cell) {
                slot if slot >= end => true,
                slot if slot >= first => *taken == Taken::Loaded,
                slot => short(*taken, (first - slot).saturating_mul(8)),
            },
            Var::Param(k) => match k.saturating_add(*cell) {
                slot if slot < params => short(*taken, (params - slot).saturating_mul(8)),
                _ => *taken == Taken::Loaded,
            },
            _ => true,
        })
    })
}

/// Whether the loop at `list[at]`, of the shape `shape`, and the three statements before it are a `foreach` over a struct enumerator.
// The count of every use of the two variables is what proves that no other use was missed: the
// value is zeroed, handed to the make and copied once; the enumerator is set and has its address
// taken twice in each of its loops, and is named nowhere else.
pub(super) fn proved(list: &[Stmt], at: usize, shape: &Shape, scope: &Scope<'_>) -> bool {
    let project = scope.cx.project;
    let room = shape.cells.saturating_mul(8);
    let Some(ty) = owner(project, shape.advance, &[Role::Method]) else {
        return false;
    };
    // ⚠ A `foreach` disposes of an enumerator that is disposable, which this code never does; an
    // interface of the type, of the runtime's or one deriving from one, may make it so.
    if (project.types.get(&ty)).is_none_or(|facts| !facts.interfaces.is_empty()) {
        return false;
    }
    let agreed = scope.pairs.get(&ty).is_some_and(|pairs| pairs.len() == 1);
    let read = match shape.read {
        Read::Field { offset, bytes } => offset.checked_add(bytes).is_some_and(|end| end <= room),
        Read::Getter(getter) => {
            owner(project, getter, &[Role::Method, Role::Getter]) == Some(ty)
                && used_as(project, getter, room, Mode::In)
        }
    };
    let advances = used_as(project, shape.advance, room, Mode::Ref);
    let makes = used_as(project, shape.make, room, Mode::Out);
    if !agreed || !read || !advances || !makes {
        return false;
    }

    let (made, held) = (shape.made, shape.held);
    let wide = scope
        .head
        .vars
        .get(&held)
        .is_some_and(|info| info.width == Width::Cells);
    if !wide || scope.cells(held) != shape.cells {
        return false;
    }
    let loops = scope.enumerated.get(&held).copied().unwrap_or(0);
    let set = Use {
        assigned: loops,
        read: 0,
        addressed: true,
    };
    let own = scope.addresses.get(&held).map_or(&[][..], Vec::as_slice);
    let taken = own.len() == loops.saturating_mul(2) && own.iter().all(|(cell, _)| *cell == 0);
    let named = |var: &Var| *var != held && scope.covers(held, *var);
    let apart = !scope.uses.keys().any(named) && !scope.addresses.keys().any(named);
    let once = Use {
        assigned: 1,
        read: 1,
        addressed: true,
    };
    let handed = [(
        0,
        Taken::Handed {
            to: shape.make,
            at: 0,
        },
    )];
    let value = scope.uses.get(&made) == Some(&once)
        && scope.addresses.get(&made).map(Vec::as_slice) == Some(&handed[..]);
    let counted = scope.uses.get(&held) == Some(&set) && taken && apart && value;
    if loops == 0 || !counted || !private(scope, held, shape.cells) {
        return false;
    }

    let Some(Stmt::While { body, .. }) = list.get(at) else {
        return false;
    };
    let Some((_, rest)) = body.split_first() else {
        return false;
    };
    let item = shape.item;
    let uses = used(rest);
    // A `foreach` keeps its item as the round took it, and hides the enumerator from the body.
    let kept = !uses
        .iter()
        .any(|(var, of)| scope.covers(item, *var) && of.assigned > 0);
    let hidden = !uses
        .keys()
        .any(|var| scope.covers(held, *var) || *var == made);
    let item_apart = item != made && !scope.covers(held, item) && !scope.exposed(item);
    kept && hidden && item_apart && !labelled(rest) && !suspends(rest)
}

/// Turns the three statements before the loop at `list[at]` and the loop into one `foreach`, and returns where it stands; `None`, with nothing changed, where they are not three simple statements and a `while` whose body opens with one.
pub(super) fn fold(list: &mut Vec<Stmt>, at: usize) -> Option<usize> {
    let first = at.checked_sub(3)?;
    if at >= list.len() {
        return None;
    }
    let taken: Vec<Stmt> = list.drain(first..=at).collect();
    let made = match <[Stmt; 4]>::try_from(taken) {
        Ok(
            [Stmt::Simple(zero), Stmt::Simple(make), Stmt::Simple(copy), Stmt::While { cond, mut body }],
        ) if matches!(body.first(), Some(Stmt::Simple(_))) => match body.remove(0) {
            Stmt::Simple(item) => Ok(Sugar::Foreach {
                setup: vec![zero, make, copy],
                cond,
                item: Box::new(item),
                body,
                step: None,
                after: Vec::new(),
            }),
            other => {
                body.insert(0, other);
                let simple = [zero, make, copy].map(Stmt::Simple);
                let mut back = Vec::from(simple);
                back.push(Stmt::While { cond, body });
                Err(back)
            }
        },
        Ok(other) => Err(Vec::from(other)),
        Err(other) => Err(other),
    };
    match made {
        Ok(made) => {
            list.insert(first, Stmt::Sugar(Box::new(made)));
            Some(first)
        }
        // What was taken goes back where it stood.
        Err(back) => {
            for stmt in back.into_iter().rev() {
                list.insert(first, stmt);
            }
            None
        }
    }
}

/// The collection a `foreach` over an enumerator held in place goes through, and the statements of its setup still to be written: `setup` ends in a value zeroed, made from the collection and copied into the variable whose address `cond` advances.
pub fn enumerated<'a>(setup: &'a [ir::Stmt], cond: &Expr) -> Option<(&'a [ir::Stmt], &'a Expr)> {
    let (rest, [zero, made, copy]) = setup.split_last_chunk::<3>()?;
    let ir::Stmt::Assign { target: v, value } = zero else {
        return None;
    };
    let ir::Stmt::Eval(call) = made else {
        return None;
    };
    let Expr::Call { args, .. } = under(call, false) else {
        return None;
    };
    let [into, source, ..] = args.as_slice() else {
        return None;
    };
    let ir::Stmt::Assign {
        target: e,
        value: copied,
    } = copy
    else {
        return None;
    };
    let Expr::Call { args: on, .. } = tested(cond) else {
        return None;
    };
    let advanced = matches!(on.as_slice(), [on] if address_of(on) == Some(*e));
    let zeroed = matches!(under(value, false), Expr::ZeroSlots(_));
    let held = zeroed && address_of(into) == Some(*v) && var_of(copied, false) == Some(*v);
    (held && advanced).then_some((rest, source))
}

#[cfg(test)]
#[path = "sugar_statements_enumerators_tests.rs"]
mod tests;
