//! The statements a pass read as one statement of the source, each written as that statement
//! where its pass is on and its parts have the shape the form needs, and a line each otherwise.
//!
//! ⚠ The printer reads the parts and adds nothing. A form is refused at the first part that is
//! not what it takes, and what is refused is written as the statements the tree holds.

use std::fmt::Write;

use crate::gta6::script::ir::{BinOp, Callee, Expr, ExprSugar, Kind, Scalar, Stmt, TypeKey, Var};
use crate::gta6::script::tree::{self, Group};
use crate::gta6::script::Pass;

use super::{
    assigned_receiver, bits, falls, is_zero, receiver, takes, unread, wrap, Labels, Work, Writer,
    POSTFIX,
};

/// The most elements a collection is written in one statement with.
const ELEMENTS: u64 = 4096;

/// `e` under the source readings around it.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// Whether evaluating `e` calls or makes anything, which one statement would do once and two do twice.
fn calls(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(
            node,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    found
}

/// The array an indexed `foreach` goes through: the one its item is the element of, at the index its one setup statement starts at zero and its step steps.
fn indexed<'e>(setup: &[Stmt], item: &'e Stmt, step: &Stmt) -> Option<&'e Expr> {
    let [Stmt::Assign {
        target: index,
        value: start,
    }] = setup
    else {
        return None;
    };
    let (
        Stmt::Assign { value, .. },
        Stmt::Assign {
            target: stepped, ..
        },
    ) = (item, step)
    else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Intrinsic { name, .. },
        args,
        ..
    } = bare(bits(value))
    else {
        return None;
    };
    let [array, at] = args.as_slice() else {
        return None;
    };
    let read = name.starts_with("array_get_") && *unread(at) == Expr::Var(*index);
    (read && index == stepped && is_zero(start)).then_some(array)
}

/// Whether `call` hands `var` to what it calls as an `out`.
fn fills(call: &Stmt, var: Var) -> bool {
    let mut found = false;
    call.each_expr(&mut |top| {
        top.walk(&mut |node| {
            found |= matches!(
                node,
                Expr::Sugar { kind: ExprSugar::Out, e: held } if **held == Expr::AddrOf { var, cell: 0 }
            );
        });
    });
    found
}

impl Writer<'_> {
    /// Writes a group as the one statement it reads as, then whatever of it that statement does not hold.
    pub(super) fn group(&mut self, kind: Group, parts: &[Stmt], indent: usize) {
        let read = match kind {
            Group::Construct => self.construction(parts),
            Group::Collection => self.collection(parts),
            Group::Delegate => self.delegate(parts),
            Group::Compound => self.compound(parts),
            Group::Field { owner } => self.field_store(owner, parts),
            Group::Out => self.filled(parts),
            Group::Deconstruct => self.deconstruction(parts),
            // No pass states which pieces a join gathers: the statements stay as they are.
            Group::Concat => None,
        };
        match read {
            Some((text, held)) => {
                self.line(indent, &format!("{text};"));
                self.simples(parts.get(held..).unwrap_or_default(), indent);
            }
            None => self.simples(parts, indent),
        }
    }

    /// The target of the assignment `made`, with its type where this is the place it is declared.
    fn target(&self, made: &Stmt, target: Var) -> String {
        match self.sites.contains(&std::ptr::from_ref(made)) {
            true => format!("{} {}", self.declared_type(target), self.var(target)),
            false => self.var(target),
        }
    }

    /// `x = new T(a) { F = b }`: an object made, the constructor run on it, and the fields stored straight after.
    fn construction(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        let (made, rest) = parts.split_first()?;
        let Stmt::Assign { target, value } = made else {
            return None;
        };
        let Expr::NewObject(ty) = bare(value) else {
            return None;
        };
        // The constructor is the call that stands second, with the object first among its arguments.
        let passed = match rest.first() {
            Some(Stmt::Eval(call)) => {
                let Expr::Call { args, .. } = bare(call) else {
                    return None;
                };
                match args.split_first() {
                    Some((object, passed)) if *bare(bits(object)) == Expr::Var(*target) => {
                        Some(passed)
                    }
                    _ => return None,
                }
            }
            _ => None,
        };
        if passed.is_some() && !self.options.on(Pass::Constructors) {
            return None;
        }
        let stores = rest
            .get(usize::from(passed.is_some())..)
            .unwrap_or_default();
        let mut fields = Vec::new();
        if self.options.on(Pass::Initialisers) {
            for store in stores {
                match self.initialised(store, *target, TypeKey::of(ty)) {
                    Some(field) => fields.push(field),
                    // One store that is no plain field of the object leaves them all statements.
                    None => {
                        fields.clear();
                        break;
                    }
                }
            }
        }
        if passed.is_none() && fields.is_empty() {
            return None;
        }
        let name = self.target(made, *target);
        let mut text = format!("{name} = new {}", self.type_name(&ty.name));
        if let Some(passed) = passed {
            let _ = write!(text, "({})", self.list(passed));
        }
        if !fields.is_empty() {
            let _ = write!(text, " {{ {} }}", fields.join(", "));
        }
        Some((text, 1 + usize::from(passed.is_some()) + fields.len()))
    }

    /// `F = value` for a store to a field of the object `object` holds, as an initialiser writes it.
    fn initialised(&mut self, store: &Stmt, object: Var, owner: Option<TypeKey>) -> Option<String> {
        let (Stmt::Store { addr, offset, .. } | Stmt::StoreBytes { addr, offset, .. }) = store
        else {
            return None;
        };
        if *bare(bits(addr)) != Expr::Var(object) || *offset == 0 {
            return None;
        }
        // The statement is written `x->f_18 = value`: what follows the place is what the field takes.
        let text = self.simple(store);
        let place = format!("{}->f_{offset:X} = ", self.var(object));
        let value = text.strip_prefix(&place)?;
        Some(format!("{} = {value}", self.field_name(owner, *offset)))
    }

    /// `x = new T[] { a, b }`: an array made and every element of it set, in order.
    fn collection(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::Initialisers) {
            return None;
        }
        let (made, sets) = parts.split_first()?;
        let Stmt::Assign { target, value } = made else {
            return None;
        };
        let Expr::NewArray { ty, len, .. } = bare(value) else {
            return None;
        };
        let Expr::Const(len) = bits(len) else {
            return None;
        };
        // ⚠ Every element and no other: one left out is zero, which fewer items would not say.
        if sets.is_empty() || len.bits != sets.len() as u64 || len.bits > ELEMENTS {
            return None;
        }
        let mut items = Vec::with_capacity(sets.len());
        for (index, set) in sets.iter().enumerate() {
            let Stmt::Eval(call) = set else {
                return None;
            };
            // The statement is written `x[3] = value`.
            let text = self.assignment(bare(call))?;
            let place = format!("{}[{index}] = ", self.var(*target));
            items.push(text.strip_prefix(&place)?.to_owned());
        }
        let name = self.target(made, *target);
        let ty = self.type_name(&ty.name);
        let text = format!("{name} = new {ty}[] {{ {} }}", items.join(", "));
        Some((text, parts.len()))
    }

    /// `d = receiver.Name`: a delegate made and bound, to a function of this program by its name.
    fn delegate(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::Delegates) {
            return None;
        }
        let [made @ Stmt::Assign { target, value }, Stmt::Eval(call)] = parts else {
            return None;
        };
        let Expr::NewObject(ty) = bare(value) else {
            return None;
        };
        let Expr::Call { args, .. } = bare(call) else {
            return None;
        };
        let [object, bound, function] = args.as_slice() else {
            return None;
        };
        if *bare(bits(object)) != Expr::Var(*target) {
            return None;
        }
        let name = self.target(made, *target);
        let text = match bare(function) {
            // A function of this program reads as a method does: on what it is bound to, or
            // alone where it is bound to nothing.
            Expr::FunctionRef { program, at } if *program == self.head.program => {
                let function = self.function_ref(*program, *at);
                match is_zero(bound) {
                    true => format!("{name} = {function}"),
                    false => format!("{name} = {}.{function}", self.expr(bits(bound), POSTFIX)),
                }
            }
            _ => format!(
                "{name} = new {}({}, {})",
                self.type_name(&ty.name),
                self.value(bound),
                self.value(function)
            ),
        };
        Some((text, 2))
    }

    /// `place op= value`, and `place++` for one up or down: one statement that works on the place it writes.
    fn compound(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::Compound) {
            return None;
        }
        let [part] = parts else {
            return None;
        };
        let (Stmt::Assign { value, .. } | Stmt::Store { value, .. }) = part else {
            return None;
        };
        let Expr::Binary { op, at, l, r } = value else {
            return None;
        };
        if op.is_logical() {
            return None;
        }
        // What the operator reads on its left, under its readings and the field it is read as.
        let (mut read, mut owner) = (&**l, None);
        loop {
            match read {
                Expr::Cast { e, .. } => read = &**e,
                Expr::Sugar { kind, e } => {
                    if let ExprSugar::Field { owner: of } = kind {
                        owner = Some(*of);
                    }
                    read = &**e;
                }
                _ => break,
            }
        }
        let mut typed = true;
        let (place, binds) = match (part, read) {
            (Stmt::Assign { target, .. }, Expr::Var(var)) if var == target => {
                (self.var(*target), POSTFIX)
            }
            // ⚠ The one statement evaluates the address once: it must be the address the value reads, and call nothing.
            (
                Stmt::Store {
                    kind, addr, offset, ..
                },
                Expr::Load {
                    addr: from,
                    offset: read_at,
                    ..
                },
            ) if read_at == offset && **from == *addr && !calls(addr) => match owner {
                Some(owner) => {
                    let base = self.expr(addr, POSTFIX);
                    let name = self.field_name(Some(owner), *offset);
                    (format!("{base}.{name}"), POSTFIX)
                }
                // ⚠ A place no type declares says no width: only a whole cell reads as one.
                None if kind.bytes() == 8 => {
                    typed = false;
                    self.place(addr, *offset, false)
                }
                None => return None,
            },
            _ => return None,
        };
        let whole = matches!(at, Kind::I64 | Kind::U64 | Kind::Unknown);
        let one = matches!(&**r, Expr::Const(c) if c.bits == 1 && !c.kind.is_float());
        // ⚠ `p->f_10++` is the step of an `int` there, never one of a whole cell.
        let text = match (op, one && typed) {
            (BinOp::Add, true) => format!("{}++", wrap(place, binds, POSTFIX)),
            (BinOp::Sub, true) => format!("{}--", wrap(place, binds, POSTFIX)),
            _ => {
                // The sign of a cell does not count to an operator that works on whole cells.
                let right = match whole {
                    true => self.expr(bits(r), 0),
                    false => self.expr(r, 0),
                };
                format!("{place} {}= {right}", op.symbol())
            }
        };
        Some((text, 1))
    }

    /// `base.Name = value`: one store to a field its type declares.
    fn field_store(&mut self, owner: TypeKey, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::Fields) {
            return None;
        }
        let [store] = parts else {
            return None;
        };
        let (Stmt::Store { addr, offset, .. } | Stmt::StoreBytes { addr, offset, .. }) = store
        else {
            return None;
        };
        let Expr::Var(base) = addr else {
            return None;
        };
        if *offset == 0 {
            return None;
        }
        // The statement is written `x->f_18 = value`.
        let text = self.simple(store);
        let base = self.var(*base);
        let value = text.strip_prefix(&format!("{base}->f_{offset:X} = "))?;
        let name = self.field_name(Some(owner), *offset);
        Some((format!("{base}.{name} = {value}"), 1))
    }

    /// `F(out T x)`: the call that fills the variables the statements before it only made zero.
    fn filled(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::RefOut) {
            return None;
        }
        let (call, zeroed) = parts.split_last()?;
        if zeroed.is_empty() {
            return None;
        }
        let mut declared = Vec::new();
        for zero in zeroed {
            let Stmt::Assign { target, value } = zero else {
                return None;
            };
            let empty = is_zero(value) || matches!(bits(value), Expr::ZeroSlots(_));
            if !empty || !fills(call, *target) {
                return None;
            }
            if self.sites.contains(&std::ptr::from_ref(zero)) {
                declared.push(*target);
            }
        }
        self.outs = declared;
        let text = self.statement(call);
        self.outs.clear();
        Some((text, parts.len()))
    }

    /// `(a, b) = F()`: a value of several cells and the variables that take its cells, in order.
    fn deconstruction(&mut self, parts: &[Stmt]) -> Option<(String, usize)> {
        if !self.options.on(Pass::Tuples) {
            return None;
        }
        let (
            Stmt::Assign {
                target: whole,
                value,
            },
            taken,
        ) = parts.split_first()?
        else {
            return None;
        };
        if taken.len() < 2 {
            return None;
        }
        let mut names = Vec::with_capacity(taken.len());
        for (index, take) in taken.iter().enumerate() {
            let Stmt::Assign {
                target,
                value: cell,
            } = take
            else {
                return None;
            };
            let own = matches!(
                bits(cell),
                Expr::Cell { base, index: of } if **base == Expr::Var(*whole) && *of as usize == index
            );
            if !own {
                return None;
            }
            names.push(self.target(take, *target));
        }
        let text = format!("({}) = {}", names.join(", "), self.value(value));
        Some((text, parts.len()))
    }

    /// Writes a `foreach` where its pass is on and its item is declared by it, else the loop it stands for, from the same parts.
    pub(super) fn foreach<'b>(
        &mut self,
        sugar: &'b tree::Sugar,
        indent: usize,
        work: &mut Vec<Work<'b>>,
    ) {
        let tree::Sugar::Foreach {
            setup,
            cond,
            item,
            body,
            step,
            after,
        } = sugar
        else {
            return;
        };
        work.push(Work::Run(after, indent));
        let item: &'b Stmt = item;
        let none: &[Stmt] = &[];
        let asked = receiver(cond);
        let (shown, source) = match (step.as_deref(), setup.split_last()) {
            // An index stepped over an array: the array is what is gone through, and nothing else is shown.
            (Some(step), _) => (none, indexed(setup, item, step)),
            // The collection is what the enumerator is taken from, else what each round asks.
            (None, Some((taken, rest))) if takes(taken, asked) => (rest, assigned_receiver(taken)),
            (None, _) => (setup.as_slice(), asked),
        };
        let declares = self.sites.contains(&std::ptr::from_ref(item));
        let read = (self.options.on(Pass::Foreach) && declares, item, source);
        if let (true, Stmt::Assign { target, .. }, Some(source)) = read {
            self.simples(shown, indent);
            let head = format!(
                "foreach ({} {} in {})",
                self.declared_type(*target),
                self.var(*target),
                self.value(source)
            );
            self.line(indent, &head);
            self.open(body, indent, None, work);
            return;
        }
        self.simples(setup, indent);
        match step.as_deref() {
            Some(step) => {
                let (cond, step) = (self.cond(cond, 0), self.simple(step));
                self.line(indent, &format!("for (; {cond}; {step})"));
            }
            None => self.test("while", cond, indent),
        }
        self.line(indent, "{");
        work.push(Work::Line(indent, "}"));
        work.push(Work::Run(body, indent + 1));
        work.push(Work::Simples(std::slice::from_ref(item), indent + 1));
    }

    /// Writes a chain of tests as the `switch` it reads as; `false`, with nothing written, where its arms are not the cases of one value.
    pub(super) fn switch_on<'b>(
        &mut self,
        arms: &'b [tree::Arm],
        default: &'b [tree::Stmt],
        indent: usize,
        work: &mut Vec<Work<'b>>,
    ) -> bool {
        let Some((subject, cases)) = self.arms(arms) else {
            return false;
        };
        self.line(indent, &format!("switch ({subject})"));
        self.line(indent, "{");
        // Pushed last to first. Each arm is left by a `break`: in the tests it stands for, no arm runs into the next.
        work.push(Work::Line(indent, "}"));
        if !default.is_empty() {
            if falls(default) {
                work.push(Work::Line(indent + 2, "break;"));
            }
            work.push(Work::Run(default, indent + 2));
            work.push(Work::Line(indent + 1, "default:"));
        }
        for (arm, labels) in arms.iter().zip(cases).rev() {
            if falls(&arm.body) {
                work.push(Work::Line(indent + 2, "break;"));
            }
            work.push(Work::Run(&arm.body, indent + 2));
            for label in labels.into_iter().rev() {
                work.push(Work::Text(indent + 1, format!("case {label}:")));
            }
        }
        true
    }

    /// The value every arm tests and the labels of each arm, where each test is that one value against constants.
    fn arms(&mut self, arms: &[tree::Arm]) -> Option<(String, Vec<Vec<String>>)> {
        let mut subject: Option<(&Expr, &Kind)> = None;
        let mut values: Vec<Vec<i64>> = Vec::with_capacity(arms.len());
        for arm in arms {
            let mut of = Vec::new();
            // The tests of the arm in the order they are made, however their chain nests.
            let mut open = vec![&arm.test];
            while let Some(test) = open.pop() {
                let (at, l, r) = match bare(test) {
                    Expr::Binary {
                        op: BinOp::LogicalOr,
                        l,
                        r,
                        ..
                    } => {
                        open.push(&**r);
                        open.push(&**l);
                        continue;
                    }
                    Expr::Binary {
                        op: BinOp::Eq,
                        at,
                        l,
                        r,
                    } => (at, &**l, &**r),
                    _ => return None,
                };
                let wide = match at {
                    Kind::I32 | Kind::U32 => false,
                    Kind::I64 | Kind::U64 => true,
                    _ => return None,
                };
                if subject.is_some_and(|(held, kind)| kind != at || held != l) {
                    return None;
                }
                subject = Some((l, at));
                // A reading of 32 bits keeps what a comparison of 32 bits sees.
                let mut constant = r;
                loop {
                    match constant {
                        Expr::Sugar { e, .. }
                        | Expr::Cast {
                            to: Scalar::I64 | Scalar::U64,
                            e,
                        } => constant = &**e,
                        Expr::Cast {
                            to: Scalar::I32 | Scalar::U32,
                            e,
                        } if !wide => constant = &**e,
                        _ => break,
                    }
                }
                let Expr::Const(c) = constant else {
                    return None;
                };
                if c.kind.is_float() {
                    return None;
                }
                of.push(match (wide, at) {
                    (true, _) => c.bits as i64,
                    (false, Kind::U32) => i64::from(c.bits as u32),
                    (false, _) => i64::from(c.bits as u32 as i32),
                });
            }
            values.push(of);
        }
        let (value, at) = subject?;
        // A `switch` reads its value once and the tests read it once each.
        if calls(value) {
            return None;
        }
        let narrow = matches!(at, Kind::I32 | Kind::U32);
        let large = (values.iter().flatten()).any(|label| label.unsigned_abs() > 0x1_0000);
        let texts = self.kind_of(bits(value)) == Kind::Hash && self.options.on(Pass::HashText);
        let labels = match (self.sharp && narrow && large, texts) {
            (false, _) => Labels::Decimal,
            (true, false) => Labels::Hex,
            (true, true) => Labels::Hashes,
        };
        let subject = match labels {
            Labels::Decimal => self.value(value),
            Labels::Hex | Labels::Hashes => self.as_uint(value).0,
        };
        let written = |of: &Vec<i64>| -> Vec<String> {
            of.iter().map(|label| self.label(*label, labels)).collect()
        };
        Some((subject, values.iter().map(written).collect()))
    }
}

#[cfg(test)]
#[path = "print_sugar_tests.rs"]
mod tests;
