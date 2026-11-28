//! Classes: which function belongs to which type and as what, and the calls, constructions and
//! accessors that read as members once that is known.
//!
//! Three parts. `of_program`, `of_function`, `absorb` and `settle` gather membership into the
//! project during the first pass over an install. `run` rewrites one function's body on the
//! structured form. `unit` groups a program's finished functions under their types for the printer.
//!
//! ⚠ A function is a member by one of four proofs and by nothing else: one type's virtual table
//! is the first to hold it; every mention of it in the install runs it on an object of one type
//! just made, before anything else uses the object; every mention binds it as the delegate of
//! a closure object of one compiler-made type; or every mention hands it, first, the receiver a
//! member was given or an object of a script type its settled signature states every such call
//! hands, and of the receivers' type and its bases those of the function's program that are made
//! from each class whose slot it calls on that parameter and hold every byte it reaches through it
//! are one, or the highest of them holds a byte reached that no type above it holds. Every mention
//! handing it, first, a reference field of a member's receiver that the project types is that
//! proof again, with the fields' type for the receivers'.
//!
//! ⚠ The fourth proof cannot tell a method from a static function that is only ever handed
//! objects of one type and works on that type's own fields: the two are one function in the code.
//!
//! ⚠ Every way into a function counts against these proofs: a call or reference of a function
//! that does not lift, read off its instructions; a function the engine calls by name or by the
//! header's word; a reference whatever flags it carries.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg;
use crate::gta6::script::class::{Carried, Class, ClassKind, Field, Method, Role, Unit};
use crate::gta6::script::ir::{
    self, Callee, Expr, ExprSugar, FunctionKey, Head, Kind, Returns, Scalar, TypeKey, Var, VmType,
    Width,
};
use crate::gta6::script::listing::Import;
use crate::gta6::script::opcode;
use crate::gta6::script::program::{FunctionRow, Program};
use crate::gta6::script::project::{FieldFacts, Member, Project, TypeFacts};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};
use crate::hash::literal_string_hash;

use super::sugar_expressions::Reached;

#[path = "classes_properties.rs"]
mod properties;

#[path = "classes_typed.rs"]
mod typed;

#[path = "classes_frame.rs"]
mod frame;

#[path = "classes_fields.rs"]
mod fields;

use properties::{accessed, standing, Stands};
pub use properties::{accessor, Accessor};

/// The names this file's rewrites are switched by; `Pass::Classes` switches the grouping `unit` does.
pub const PASSES: &[Pass] = &[
    Pass::Receivers,
    Pass::Constructors,
    Pass::Properties,
    Pass::Operators,
];

/// The bit of a type's flags that says the compiler made the type: a lambda's captures, a state machine.
const GENERATED: u32 = 0x10_0000;

/// The bit that says no object of the type is ever made: an interface, or an abstract class.
const ABSTRACT: u32 = 0x2_0000;

/// The bit of a value type's flags that says it is a struct; a value type without it is an enum.
const STRUCT: u32 = 0x04;

/// The bit a builtin type's id carries above its index in the runtime's table.
const BUILTIN: u32 = 0x8000_0000;

/// The bytes of an object's header: its fields start after them.
const HEADER: u32 = 8;

/// The bytes of a reference, and so of a field that holds one.
const REFERENCE: u32 = 8;

/// The hash a method record holds of the name every constructor has.
const CONSTRUCTOR: u32 = literal_string_hash(b".ctor");

/// The hash a method record holds of the name a type's static constructor has.
const STATIC_CONSTRUCTOR: u32 = literal_string_hash(b".cctor");

/// The links a walk up a chain of bases or of enclosing classes follows before it gives up: a ring must end.
const MAX_CHAIN: usize = 64;

/// The bits of a function reference that are the function's offset; the top byte holds the reference's flags.
const OFFSET: u32 = 0x00FF_FFFF;

/// The owner of a note that names no type: what the gathering holds of a function that is no membership.
// ⚠ Notes live in `Project::members` between `of_function` and `settle` only, and `settle` leaves
// none; `member` reads past one, so a part nothing settled proves nothing.
const NO_OWNER: TypeKey = TypeKey(0);

/// What the gathering says of a function, before `settle` knows every mention of it.
fn provisional(owner: TypeKey, role: Role) -> Member {
    Member {
        owner,
        role,
        is_static: false,
        slot: None,
    }
}

/// The note that no mention of a function may make it a member: it is called like any function, two types claim it, or it writes its first parameter.
fn disputed() -> Member {
    provisional(NO_OWNER, Role::Method)
}

/// The note that a function returns a value, which a constructor never does.
fn returning() -> Member {
    provisional(NO_OWNER, Role::Getter)
}

/// What two things said of one function say together.
// A least upper bound, so the order the parts of a project arrive in changes nothing.
fn join(a: &Member, b: &Member) -> Member {
    if a == b {
        return a.clone();
    }
    // A value returned rules out a constructor and nothing else.
    let other = match (*a == returning(), *b == returning()) {
        (true, _) => b,
        (_, true) => a,
        _ => return disputed(),
    };
    match other.owner != NO_OWNER && other.role != Role::Constructor {
        true => other.clone(),
        false => disputed(),
    }
}

fn note(project: &mut Project, function: FunctionKey, said: Member) {
    let joined = match project.members.get(&function) {
        Some(held) => join(held, &said),
        None => said,
    };
    project.members.insert(function, joined);
}

/// What the project proves of a function; a note the gathering left proves nothing.
fn member(project: &Project, function: FunctionKey) -> Option<&Member> {
    let found = project.members.get(&function);
    found.filter(|member| member.owner != NO_OWNER)
}

/// The function of a script a call reaches by its own offset, `None` for any other callee.
fn callee_key(program: u32, callee: &Callee) -> Option<FunctionKey> {
    match callee {
        Callee::Local { at, .. } => Some(FunctionKey::new(program, *at)),
        Callee::Import { program, at, .. } => Some(FunctionKey::new(*program, *at)),
        _ => None,
    }
}

/// The script function `e` itself calls or takes a reference to.
fn referred(program: u32, e: &Expr) -> Option<FunctionKey> {
    match e {
        Expr::Call { callee, .. } => callee_key(program, callee),
        Expr::FunctionRef { program, at } => Some(FunctionKey::new(*program, *at & OFFSET)),
        _ => None,
    }
}

/// Whether `e` reads `var` or takes its address.
fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |e| {
        found |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// `e` under the casts that keep every bit of a cell.
fn whole(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::U64 | Scalar::I64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// Marks every variable `top` takes the address of as one that may hold anything.
fn addressed(top: &Expr, assigned: &mut BTreeMap<Var, Option<&VmType>>) {
    top.walk(&mut |e| {
        if let Expr::AddrOf { var, .. } = e {
            assigned.insert(*var, None);
        }
    });
}

/// Every variable the function assigns or takes the address of, with the type of the object it holds where every assignment makes one of that type.
fn assigned(f: &cfg::Function) -> BTreeMap<Var, Option<&VmType>> {
    let mut assigned: BTreeMap<Var, Option<&VmType>> = BTreeMap::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            let fresh = match stmt {
                ir::Stmt::Assign {
                    value: Expr::NewObject(ty),
                    ..
                } => Some(ty),
                _ => None,
            };
            for target in stmt.targets() {
                let held = assigned.entry(*target).or_insert(fresh);
                if *held != fresh {
                    *held = None;
                }
            }
            stmt.each_expr(&mut |top| addressed(top, &mut assigned));
        }
        if let Some(top) = block.term.expr() {
            addressed(top, &mut assigned);
        }
    }
    assigned
}

/// Whether a statement assigns `var`, reads it or takes its address.
fn uses(stmt: &ir::Stmt, var: Var) -> bool {
    let mut found = stmt.targets().contains(&var);
    stmt.each_expr(&mut |top| found |= mentions(top, var));
    found
}

/// What one mention of a script's function says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Said {
    /// A note, which `settle` reads with every other note of the function.
    Note(Member),
    /// The function is handed, first, the receiver the function that mentions it was given.
    Receiver,
    /// The function is handed, first, a value of one cell the function that mentions it holds of its own: what that is, its signature says of every such call.
    Typed,
    /// The function is handed, first, the reference the field at this byte offset of the receiver the function that mentions it was given holds.
    Field(u32),
}

/// Whether `object` is an object of the builtin type `ty` that the function itself makes: in place, or into a variable that holds nothing else.
fn made_of(object: &Expr, ty: u32, assigned: &BTreeMap<Var, Option<&VmType>>) -> bool {
    let made = match whole(object) {
        Expr::NewObject(made) => Some(made),
        Expr::Var(var) => assigned.get(var).copied().flatten(),
        _ => None,
    };
    made.is_some_and(|made| made.id & BUILTIN != 0 && made.id & !BUILTIN == ty)
}

/// Whether `addr` is the variable `var` itself, whole.
fn through(addr: &Expr, var: Var) -> bool {
    matches!(whole(addr), Expr::Var(held) if *held == var)
}

/// One past the last byte `top` loads through `receiver` itself.
fn loaded(top: &Expr, receiver: Var) -> u32 {
    let mut end = 0u32;
    top.walk(&mut |e| {
        let here = match e {
            Expr::Load { ty, addr, offset } if through(addr, receiver) => {
                offset.saturating_add(ty.bytes())
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if through(addr, receiver) => offset.saturating_add(*bytes),
            _ => 0,
        };
        end = end.max(here);
    });
    end
}

/// One past the last byte a statement writes through `receiver` itself.
fn stored(stmt: &ir::Stmt, receiver: Var) -> u32 {
    match stmt {
        ir::Stmt::Store {
            kind, addr, offset, ..
        } if through(addr, receiver) => offset.saturating_add(kind.bytes()),
        ir::Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } if through(addr, receiver) => offset.saturating_add(*bytes),
        // A cell stored as a float, and a step, fill four bytes; one cell at least is stored.
        ir::Stmt::StoreF32Slots { addr, offset, .. } | ir::Stmt::StepI32 { addr, offset, .. }
            if through(addr, receiver) =>
        {
            offset.saturating_add(4)
        }
        ir::Stmt::ZeroBytes { addr, bytes } if through(addr, receiver) => *bytes,
        _ => 0,
    }
}

/// Whether a call's first argument is the receiver, whole.
fn hands(args: &[Expr], receiver: Option<Var>) -> bool {
    match (args.first(), receiver) {
        (Some(first), Some(receiver)) => through(first, receiver),
        _ => false,
    }
}

/// The function a statement proves a member or hands the receiver, with what it says and the expression that says it.
// `fresh` is the object the block last made and the type made, while nothing has used it since;
// `receiver` is the function's first parameter where the function leaves it as it was given.
fn site<'a>(
    program: u32,
    assigned: &BTreeMap<Var, Option<&VmType>>,
    fresh: Option<(Var, &VmType)>,
    receiver: Option<Var>,
    stmt: &'a ir::Stmt,
) -> Option<(FunctionKey, Said, &'a Expr)> {
    let ir::Stmt::Eval(call @ Expr::Call { callee, args, .. }) = stmt else {
        return None;
    };
    match (callee, args.as_slice()) {
        // A delegate made and bound: the function bound takes the object bound as its first parameter.
        // ⚠ No name is asked, since a compiled program spells no builtin: an object of a builtin
        // type the function makes, handed with a target and a function to a builtin of that same
        // type, is what only the runtime's delegate is made by.
        (
            Callee::Builtin { ty: of, .. },
            [object, target, bound @ Expr::FunctionRef { program: home, at }],
        ) if made_of(object, *of, assigned) => {
            let function = FunctionKey::new(*home, *at & OFFSET);
            let Expr::Var(target) = whole(target) else {
                return None;
            };
            // Bound to the receiver: the function is run on the object this one was.
            if receiver == Some(*target) {
                return Some((function, Said::Receiver, bound));
            }
            // Bound to a closure object this function makes: the function is the closure's own.
            let made = assigned.get(target).copied().flatten();
            let closure = made.filter(|ty| ty.flags & GENERATED != 0)?;
            let lambda = provisional(TypeKey::of(closure)?, Role::Lambda);
            Some((function, Said::Note(lambda), bound))
        }
        // The first thing run on an object just made, or on one made in the call itself: its constructor.
        (Callee::Local { .. } | Callee::Import { .. }, [first, passed @ ..]) => {
            let ty = match (first, fresh) {
                (Expr::NewObject(ty), _) => ty,
                (Expr::Var(object), Some((made, ty)))
                    if *object == made && !passed.iter().any(|arg| mentions(arg, made)) =>
                {
                    ty
                }
                _ => return None,
            };
            let constructor = provisional(TypeKey::of(ty)?, Role::Constructor);
            Some((callee_key(program, callee)?, Said::Note(constructor), call))
        }
        _ => None,
    }
}

/// Notes what `top` says of every script function it calls or refers to, but for the mention at `proof`: handed the receiver first, handed a value of the caller's own first, handed a field of the receiver first, or mentioned like any function.
fn others(
    top: &Expr,
    head: &Head,
    receiver: Option<Var>,
    proof: Option<&Expr>,
    found: &mut Vec<(FunctionKey, Said)>,
) {
    top.walk(&mut |e| {
        if proof.is_some_and(|proof| std::ptr::eq(proof, e)) {
            return;
        }
        let Some(function) = referred(head.program, e) else {
            return;
        };
        let said = match e {
            Expr::Call { args, .. } if hands(args, receiver) => Said::Receiver,
            Expr::Call { args, .. } if typed::handed_value(args, head) => Said::Typed,
            Expr::Call { args, .. } => match fields::handed(args, receiver) {
                Some(offset) => Said::Field(offset),
                None => Said::Note(disputed()),
            },
            _ => Said::Note(disputed()),
        };
        found.push((function, said));
    });
}

/// Adds what `from` holds to `into`.
fn carry(into: &mut Carried, from: Carried) {
    for (function, by) in from.handed {
        into.handed.entry(function).or_default().extend(by);
    }
    // Two readings of one function keep the lesser reach: what both show.
    for (function, end) in from.reach {
        let held = into.reach.entry(function).or_insert(end);
        *held = (*held).min(end);
    }
    for (function, named) in from.unread {
        into.unread.entry(function).or_default().extend(named);
    }
    into.blind.extend(from.blind);
    for (function, by) in from.typed {
        into.typed.entry(function).or_default().extend(by);
    }
    for (function, types) in from.needs {
        into.needs.entry(function).or_default().extend(types);
    }
    into.self_typed.extend(from.self_typed);
    for (function, by) in from.fields {
        into.fields.entry(function).or_default().extend(by);
    }
}

/// The script functions the instructions of `row` call or take a reference to; `None` where a word of it is no instruction, or names an import the program does not hold.
// ⚠ Read off the bytes, since a function that does not lift shows nothing else: what it hands
// the functions it calls is never seen.
fn named_in(program: &Program, row: &FunctionRow) -> Option<BTreeSet<FunctionKey>> {
    let code = program.code.as_slice();
    let end = (row.end as usize).min(code.len());
    let mut named = BTreeSet::new();
    let mut at = row.at as usize;
    while at < end {
        let word = code.get(at..at + 2)?;
        let op = opcode::of_word(u16::from_le_bytes([word[0], word[1]]));
        // Every instruction is two bytes at least, so each step moves on.
        let spec = opcode::spec(op)?;
        let len = spec.bytes() as usize;
        let operands = code.get(at + 2..at + len)?;
        let byte = |n: usize| {
            let stored = operands.get(n)?;
            Some(u32::from(opcode::plain(*stored, n)))
        };
        // The lowest byte first.
        let number = |from: usize, width: usize| {
            let mut bytes = (from..from + width).rev();
            bytes.try_fold(0u32, |held, n| Some((held << 8) | byte(n)?))
        };
        // The operand of that name, at the place and width the table's row gives it.
        let field = |name: &str| {
            let mut from = 0usize;
            for opcode::Field(named, ty) in spec.fields {
                let width = ty.bytes() as usize;
                if *named == name {
                    return number(from, width);
                }
                from += width;
            }
            None
        };
        // A call and a reference pushed name a script function by an import and an offset.
        let target = match spec.act {
            opcode::Act::Call | opcode::Act::PushFunction => {
                Some((field("import")?, field("offset")?))
            }
            _ => None,
        };
        if let Some((import, to)) = target {
            named.insert(FunctionKey::new(program.imported(import)?, to));
        }
        at += len;
    }
    Some(named)
}

/// Adds what a compiled program's container and bytes state: a record under the name of a constructor makes its function the constructor of the type the record belongs to; what the engine calls, and what each function's instructions name, until it is read, are mentions no code shows.
pub fn of_program(project: &mut Project, program: &Program) {
    records(project, program);
    // The engine runs these by a name's hash or by the header's word, and hands them what it will.
    let engine = program.entries.iter().map(|entry| entry.at);
    let rows = &program.functions;
    for at in engine.chain(program.main).chain(program.initialize) {
        // One that takes nothing is handed nothing.
        let row = rows.binary_search_by_key(&at, |row| row.at).ok();
        let takes = row
            .and_then(|n| rows.get(n))
            .is_none_or(|row| row.params > 0);
        if takes {
            note(project, FunctionKey::new(program.program, at), disputed());
        }
    }
    let mut carried = Carried::default();
    for row in &program.functions {
        match named_in(program, row) {
            Some(named) if named.is_empty() => {}
            Some(named) => {
                let own = FunctionKey::new(program.program, row.at);
                carried.unread.insert(own, named);
            }
            None => {
                carried.blind.insert(program.program);
            }
        }
    }
    if carried == Carried::default() {
        return;
    }
    let row = project.programs.entry(program.program).or_default();
    row.program = program.program;
    carry(&mut row.carried, carried);
}

/// Adds what a compiled program's method records state of constructors.
fn records(project: &mut Project, program: &Program) {
    for record in &program.methods {
        // A name's hash has an upper half: a row without one is no type the registry keeps.
        let owner = program.types.get(record.owner as usize);
        let owner = owner.filter(|row| row.name >> 32 != 0);
        let (Some(at), Some(owner), None) = (record.at, owner, record.slot) else {
            continue;
        };
        let owner = TypeKey(owner.name);
        let said = match (record.name, record.params, record.returns) {
            (CONSTRUCTOR, 1.., 0) => provisional(owner, Role::Constructor),
            (STATIC_CONSTRUCTOR, 0, 0) => Member {
                is_static: true,
                ..provisional(owner, Role::StaticConstructor)
            },
            _ => continue,
        };
        note(project, FunctionKey::new(program.program, at), said);
    }
}

/// Adds what one function's code proves of membership: a constructor run on an object just made, a lambda bound to its closure object, a function handed the receiver, and how far the function reaches through its own.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let program = f.head.program;
    let own = FunctionKey::new(program, f.head.at);
    // Its code is read now, and says more than its instructions did.
    if let Some(row) = project.programs.get_mut(&program) {
        row.carried.unread.remove(&own);
    }
    let assigned = assigned(f);
    // ⚠ The first parameter is the receiver only while the function leaves it as it was given,
    // and only where it is one cell: a value of several is no reference.
    let first = f.head.vars.get(&Var::Param(0));
    let wide = first.is_some_and(|info| info.width == Width::Cells);
    let written = assigned.contains_key(&Var::Param(0)) || wide;
    let receiver = (f.head.params > 0 && !written).then_some(Var::Param(0));
    let mut found: Vec<(FunctionKey, Said)> = Vec::new();
    let mut end = 0u32;
    let mut needs: BTreeSet<TypeKey> = BTreeSet::new();
    for block in &f.cfg.blocks {
        let mut fresh: Option<(Var, &VmType)> = None;
        for stmt in &block.stmts {
            let proved = site(program, &assigned, fresh, receiver, stmt);
            let proof = proved.as_ref().map(|(_, _, proof)| *proof);
            found.extend(proved.map(|(function, said, _)| (function, said)));
            stmt.each_expr(&mut |top| others(top, &f.head, receiver, proof, &mut found));
            if let Some(receiver) = receiver {
                end = end.max(stored(stmt, receiver));
                stmt.each_expr(&mut |top| {
                    end = end.max(loaded(top, receiver));
                    typed::needed(top, receiver, &mut needs);
                });
            }
            fresh = match stmt {
                ir::Stmt::Assign {
                    target,
                    value: Expr::NewObject(ty),
                } => Some((*target, ty)),
                _ => fresh.filter(|(object, _)| !uses(stmt, *object)),
            };
        }
        if let Some(top) = block.term.expr() {
            others(top, &f.head, receiver, None, &mut found);
            if let Some(receiver) = receiver {
                end = end.max(loaded(top, receiver));
                typed::needed(top, receiver, &mut needs);
            }
        }
    }
    // What the function's own code says of it: `this` is never written, and a constructor returns nothing.
    if written {
        found.push((own, Said::Note(disputed())));
    } else if f.head.returns != Returns::Void {
        found.push((own, Said::Note(returning())));
    }
    let mut carried = Carried::default();
    for (function, said) in found {
        match said {
            Said::Note(said) => note(project, function, said),
            // A function that hands itself its own receiver says nothing of whose that is.
            Said::Receiver if function == own => {}
            Said::Receiver => {
                carried.handed.entry(function).or_default().insert(own);
            }
            Said::Typed => {
                carried.typed.entry(function).or_default().insert(own);
            }
            Said::Field(offset) => {
                let by = carried.fields.entry(function).or_default();
                by.insert((own, offset));
            }
        }
    }
    if end > 0 {
        carried.reach.insert(own, end);
    }
    if !needs.is_empty() {
        carried.needs.insert(own, needs);
    }
    if typed::self_typed(&f.head) {
        carried.self_typed.insert(own);
    }
    if carried == Carried::default() {
        return;
    }
    let row = project.programs.entry(program).or_default();
    row.program = program;
    carry(&mut row.carried, carried);
}

/// Moves the members of `from` into `into`; a function both hold keeps what the two say together.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (function, said) in std::mem::take(&mut from.members) {
        note(into, function, said);
    }
    for (program, info) in &mut from.programs {
        let carried = std::mem::take(&mut info.carried);
        if carried == Carried::default() {
            continue;
        }
        let row = into.programs.entry(*program).or_default();
        row.program = *program;
        carry(&mut row.carried, carried);
    }
}

/// Whether a type takes a function of its table from an ancestor that holds it too.
fn inherits(project: &Project, ty: TypeKey, holders: &BTreeMap<TypeKey, u32>) -> bool {
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        match project.types.get(&ty).and_then(|facts| facts.base) {
            Some(base) if holders.contains_key(&base) => return true,
            Some(base) => ty = base,
            None => return false,
        }
    }
    false
}

/// Whether every ancestor of a type is known, so that none can hold a function unseen.
fn rooted(project: &Project, ty: TypeKey) -> bool {
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        match project.types.get(&ty) {
            Some(facts) => match facts.base {
                Some(base) => ty = base,
                None => return true,
            },
            // A builtin type holds no function of a script.
            None => return ty.builtin().is_some(),
        }
    }
    false
}

/// Whether `ty` is `ancestor` or is made from it through its bases; an interface it implements does not count.
fn derives(project: &Project, ty: TypeKey, ancestor: TypeKey) -> bool {
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        if ty == ancestor {
            return true;
        }
        match project.types.get(&ty).and_then(|facts| facts.base) {
            Some(base) => ty = base,
            None => return false,
        }
    }
    false
}

/// The nearest type both `a` and `b` are or are made from, where a program states it.
fn common(project: &Project, a: TypeKey, b: TypeKey) -> Option<TypeKey> {
    if a == b {
        return Some(a);
    }
    let mut above = Vec::new();
    let mut ty = a;
    for _ in 0..MAX_CHAIN {
        above.push(ty);
        match project.types.get(&ty).and_then(|facts| facts.base) {
            Some(base) => ty = base,
            None => break,
        }
    }
    let mut ty = b;
    for _ in 0..MAX_CHAIN {
        if above.contains(&ty) {
            // A builtin type holds no function of a script.
            let stated = ty.builtin().is_none() && project.types.contains_key(&ty);
            return stated.then_some(ty);
        }
        ty = project.types.get(&ty)?.base?;
    }
    None
}

/// What is left to prove of a function some mention hands a receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Claim {
    /// The type the function's other mentions name: of the objects it is first run on, or of the closure objects it is bound to.
    seed: Option<TypeKey>,
    role: Role,
}

/// What is known of the receivers a function is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrives {
    /// No receiver of a known type yet.
    Nothing,
    /// The nearest type every receiver so far is of.
    One(TypeKey),
    /// Receivers no one script type is under.
    Unrelated,
}

impl Arrives {
    /// What two findings say together.
    // A least upper bound, so the order the receivers are found in changes nothing.
    fn with(self, other: Arrives, project: &Project) -> Arrives {
        match (self, other) {
            (Arrives::Nothing, held) | (held, Arrives::Nothing) => held,
            (Arrives::One(a), Arrives::One(b)) => {
                common(project, a, b).map_or(Arrives::Unrelated, Arrives::One)
            }
            _ => Arrives::Unrelated,
        }
    }
}

/// Settles the constructors some mention hands a receiver: one stands where only constructors of its own type, or of types made from it, hand it theirs.
// ⚠ A constructor runs its base's on its own receiver. A constructor handed a receiver by
// anything else is left to `received`, as a function like any other.
fn chained(
    project: &Project,
    carried: &Carried,
    members: &mut BTreeMap<FunctionKey, Member>,
    open: &mut BTreeMap<FunctionKey, Claim>,
) {
    let mut standing: BTreeMap<FunctionKey, TypeKey> = BTreeMap::new();
    for (function, claim) in open.iter() {
        if let (Role::Constructor, Some(ty)) = (claim.role, claim.seed) {
            standing.insert(*function, ty);
        }
    }
    let mut settled = false;
    // Each round drops what the last round's drops no longer hold up, one link of a chain of bases a round.
    for _ in 0..MAX_CHAIN {
        let constructs = |by: &FunctionKey, ty: TypeKey| {
            let owner = match members.get(by) {
                Some(member) => (member.role == Role::Constructor).then_some(member.owner),
                None => standing.get(by).copied(),
            };
            owner.is_some_and(|owner| derives(project, owner, ty))
        };
        let mut fallen = Vec::new();
        for (function, ty) in &standing {
            let mut by = carried.handed.get(function).into_iter().flatten();
            if !by.all(|by| constructs(by, *ty)) {
                fallen.push(*function);
            }
        }
        if fallen.is_empty() {
            settled = true;
            break;
        }
        for function in &fallen {
            standing.remove(function);
        }
    }
    if !settled {
        standing.clear();
    }
    for (function, claim) in open.iter_mut() {
        if claim.role == Role::Constructor && !standing.contains_key(function) {
            claim.role = Role::Method;
        }
    }
    for (function, ty) in standing {
        open.remove(&function);
        members.insert(function, provisional(ty, Role::Constructor));
    }
}

/// The one type that declares a function handed receivers of the type `arrived`: of `arrived` and its bases, those of the program that holds the function, made from each class of `needs`, and holding every byte the function reaches through its receiver, where one alone does or, as `several` allows, the highest of them holds a byte reached that no type above it holds; `None` otherwise.
// ⚠ The receivers say the type that declares the function is `arrived` or a base of it, and a
// type of another program holds none of this program's functions. A generic type's functions
// may be made in any program that uses it, and a base no program states may be anything: either
// in the line leaves the function free. A value type's method takes a pointer to the value,
// which no call is written with as a receiver, so none is taken. A builtin type holds no function
// of a script, and nor does a type above it. A class whose slot the function calls on its
// receiver is one the receiver is made from: a type above it is no `this` that call is made on.
// ⚠ Of several types that may declare it, the highest is the one that declares the field of the
// last byte reached, which every type under it holds by inheritance: where a type above it holds
// that byte too, a builtin above it states no size, or no byte past the header is reached, which
// declares it is not said.
fn declarer(
    project: &Project,
    arrived: TypeKey,
    function: FunctionKey,
    reach: Option<u32>,
    needs: &[TypeKey],
    several: bool,
) -> Option<TypeKey> {
    let end = reach.unwrap_or(0);
    let highest = |found: &[TypeKey], declares: bool| highest(found, declares && several);
    // The types that may declare it, from `arrived` up, and whether the highest of them holds a
    // byte reached that the type above it does not.
    let mut found: Vec<TypeKey> = Vec::new();
    let mut declares = false;
    let mut above = false;
    let mut ty = arrived;
    for _ in 0..MAX_CHAIN {
        if let Some(index) = ty.builtin() {
            // A builtin's objects hold the bytes the runtime's table states, after the header.
            if above {
                let bytes = opcode::builtin_type(index & 0x7FFF_FFFF).map(|(bytes, _)| bytes);
                declares = bytes.is_some_and(|bytes| end > HEADER.saturating_add(bytes));
            }
            return highest(&found, declares);
        }
        let facts = project.types.get(&ty)?;
        if facts.is_value() {
            return None;
        }
        // Sizes are not taken to fall up the line: a base too small holds nothing, and the walk goes on.
        let fits = end <= HEADER.saturating_add(facts.byte_size);
        let holds = fits && needs.iter().all(|need| derives(project, ty, *need));
        let own = facts.home == function.program();
        let generic = facts.generic.is_some() || facts.parameters > 0;
        if holds && (generic || (own && facts.base.is_none())) {
            return None;
        }
        if above {
            declares = !fits;
        }
        above = holds && own;
        if above {
            found.push(ty);
            declares = end > HEADER;
        }
        match facts.base {
            Some(base) => ty = base,
            None => return highest(&found, declares),
        }
    }
    None
}

/// The type of `found`, lowest first, that declares a function: the one alone, or the highest where `declares` says it holds a byte reached that no type above it holds.
fn highest(found: &[TypeKey], declares: bool) -> Option<TypeKey> {
    match found {
        [one] => Some(*one),
        [.., top] if declares => Some(*top),
        _ => None,
    }
}

/// Settles the functions every mention of which hands them a receiver or a value of the caller's own: each is a method of the one type `declarer` names for the nearest type all those receivers and objects are of.
fn received(
    project: &Project,
    carried: &Carried,
    members: &mut BTreeMap<FunctionKey, Member>,
    mut open: BTreeMap<FunctionKey, Claim>,
) {
    // The functions each function hands its receiver to.
    let mut hands: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
    for (function, by) in &carried.handed {
        for by in by {
            hands.entry(*by).or_default().push(*function);
        }
    }
    // ⚠ The first parameter of a function that is no member, and that is not itself only ever
    // handed a receiver or objects its signature states, is anything: what it hands that
    // parameter to is refused, and so on down.
    let mut refused: Vec<FunctionKey> = Vec::new();
    for function in open.keys() {
        let mut by = carried.handed.get(function).into_iter().flatten();
        let receivers = by.all(|by| match members.get(by) {
            Some(member) => !member.is_static,
            None => open.contains_key(by),
        });
        // A value a caller holds of its own is what the signature says of every such call, or anything.
        let valued = carried.typed.contains_key(function);
        let objects = !valued || typed::handed_object(project, carried, *function).is_some();
        if !receivers || !objects {
            refused.push(*function);
        }
    }
    while let Some(function) = refused.pop() {
        if open.remove(&function).is_none() {
            continue;
        }
        let to = hands.get(&function).into_iter().flatten();
        refused.extend(to.filter(|to| open.contains_key(*to)));
    }

    let mut arrives: BTreeMap<FunctionKey, Arrives> = BTreeMap::new();
    let mut work: Vec<FunctionKey> = Vec::new();
    for (function, claim) in &open {
        let mut held = claim.seed.map_or(Arrives::Nothing, Arrives::One);
        for by in carried.handed.get(function).into_iter().flatten() {
            if let Some(member) = members.get(by) {
                held = held.with(Arrives::One(member.owner), project);
            }
        }
        let valued = carried.typed.contains_key(function);
        if let Some(ty) = valued.then(|| typed::handed_object(project, carried, *function)) {
            held = held.with(ty.map_or(Arrives::Unrelated, Arrives::One), project);
        }
        if held != Arrives::Nothing {
            work.push(*function);
        }
        arrives.insert(*function, held);
    }
    // Each step moves one function's type up its chain of bases or off it; a ring of bases must not keep the steps coming.
    let mut steps = arrives.len().saturating_mul(MAX_CHAIN + 2);
    while let Some(function) = work.pop() {
        let Some(from) = arrives.get(&function).copied() else {
            continue;
        };
        for to in hands.get(&function).into_iter().flatten() {
            let Some(held) = arrives.get_mut(to) else {
                continue;
            };
            let joined = held.with(from, project);
            if joined == *held {
                continue;
            }
            if steps == 0 {
                return;
            }
            steps -= 1;
            *held = joined;
            work.push(*to);
        }
    }

    for (function, claim) in &open {
        let Some(Arrives::One(arrived)) = arrives.get(function).copied() else {
            continue;
        };
        let proved = match claim.role {
            // A lambda stays its closure's own: every receiver it is handed is that closure.
            Role::Lambda => (claim.seed == Some(arrived)).then_some((arrived, Role::Lambda)),
            _ => {
                let reach = carried.reach.get(function).copied();
                let handed = hands.get(function).map(Vec::as_slice);
                let needs = typed::needs(project, carried, members, handed, *function);
                // ⚠ A function that hands its receiver on may hand it to a method of a type
                // under the highest that may declare it, which a `this` of that type cannot call.
                let several = handed.is_none_or(<[FunctionKey]>::is_empty);
                let owner = declarer(project, arrived, *function, reach, &needs, several);
                owner.map(|owner| (owner, Role::Method))
            }
        };
        if let Some((owner, role)) = proved {
            members.insert(*function, provisional(owner, role));
        }
    }
}

/// Notes as mentioned like any function what a function no reading lifted names, and every function of a program whose instructions could not all be read, or of a program it imports.
fn unseen(project: &Project, carried: &Carried, gathered: &mut BTreeMap<FunctionKey, Member>) {
    for named in carried.unread.values().flatten() {
        gathered.insert(*named, disputed());
    }
    let mut blind = carried.blind.clone();
    for program in &carried.blind {
        let info = project.programs.get(program);
        let imports = info.map(|info| info.imports.as_slice()).unwrap_or_default();
        blind.extend(imports.iter().map(|import| import.program));
    }
    let known = gathered.keys().chain(carried.handed.keys());
    let known = known
        .chain(carried.typed.keys())
        .chain(carried.fields.keys());
    let known = known.filter(|function| blind.contains(&function.program()));
    let known: Vec<FunctionKey> = known.copied().collect();
    for function in known {
        gathered.insert(function, disputed());
    }
}

/// Once every part is in: adds the functions the types' virtual slots name, drops a function two types claim, and settles the functions handed a receiver.
pub fn settle(project: &mut Project) {
    let mut gathered = std::mem::take(&mut project.members);
    let mut carried = Carried::default();
    for info in project.programs.values_mut() {
        carry(&mut carried, std::mem::take(&mut info.carried));
    }
    unseen(project, &carried, &mut gathered);
    // ⚠ A function handed a value its caller holds of its own, or a field of its receiver, is
    // called like any function: it is no constructor and no lambda, whatever another mention says.
    for function in carried.typed.keys().chain(carried.fields.keys()) {
        if gathered
            .get(function)
            .is_some_and(|said| said.owner != NO_OWNER)
        {
            gathered.insert(*function, disputed());
        }
    }
    // The types whose table holds each function, each with the lowest slot it holds it in.
    let mut holders: BTreeMap<FunctionKey, BTreeMap<TypeKey, u32>> = BTreeMap::new();
    for (ty, facts) in &project.types {
        for (slot, function) in &facts.slots {
            let types = holders.entry(*function).or_default();
            types.entry(*ty).or_insert(*slot);
        }
    }
    let mut members = BTreeMap::new();
    for (function, types) in &holders {
        // The one type that holds the function and inherits it from no ancestor declares it.
        let mut first = types
            .iter()
            .filter(|(ty, _)| !inherits(project, **ty, types));
        if let (Some((owner, slot)), None) = (first.next(), first.next()) {
            if rooted(project, *owner) {
                let method = Member {
                    slot: Some(*slot),
                    ..provisional(*owner, Role::Method)
                };
                members.insert(*function, method);
            }
        }
    }
    // What is left to prove of each function handed a receiver; a table's function is spoken for
    // by its table, and one some mention hands a field is left to `fields::settle` alone.
    let mut open: BTreeMap<FunctionKey, Claim> = BTreeMap::new();
    for function in carried.handed.keys().chain(carried.typed.keys()) {
        if holders.contains_key(function) || carried.fields.contains_key(function) {
            continue;
        }
        let method = Claim {
            seed: None,
            role: Role::Method,
        };
        let claim = match gathered.get(function) {
            None => method,
            Some(said) if *said == returning() => method,
            // A function that takes no receiver is handed none.
            Some(said) if said.owner != NO_OWNER && !said.is_static => Claim {
                seed: Some(said.owner),
                role: said.role,
            },
            Some(_) => continue,
        };
        open.insert(*function, claim);
    }
    // The functions only ever handed a field of a receiver, and mentioned in no other way.
    let only_fields: BTreeSet<FunctionKey> = carried
        .fields
        .keys()
        .filter(|function| {
            let other = holders.contains_key(*function)
                || carried.handed.contains_key(*function)
                || carried.typed.contains_key(*function);
            let said = gathered.get(*function);
            !other && said.is_none_or(|said| *said == returning())
        })
        .copied()
        .collect();
    for (function, said) in gathered {
        // A function of a virtual table is no constructor and no lambda, whichever type holds it.
        let handed = carried.handed.contains_key(&function);
        if said.owner != NO_OWNER && !handed && !holders.contains_key(&function) {
            members.insert(function, said);
        }
    }
    chained(project, &carried, &mut members, &mut open);
    received(project, &carried, &mut members, open);
    fields::settle(project, &carried, &mut members, only_fields);
    project.members = members;
}

/// Whether `call` runs a constructor of the type `made` makes an object of, on that object and with nothing else of it.
// ⚠ `x = new T(a)` assigns after the constructor has run, and the two statements before: the
// constructor must have no way to `x` but the object it is handed, so `x` is no global and no
// variable a pointer reaches: none `reached` says may, or a local `frame` proves none does.
fn constructs(
    made: &ir::Stmt,
    call: &ir::Stmt,
    head: &Head,
    project: &Project,
    reached: &Reached,
    frame: &frame::Frame,
) -> bool {
    let target = constructed(made, call, head, project);
    target.is_some_and(|target| !reached.has(target) || frame.private(target, head))
}

/// The variable `made` assigns an object to where `call` runs a constructor of the object's type on it, and is handed nothing else of the variable.
fn constructed(made: &ir::Stmt, call: &ir::Stmt, head: &Head, project: &Project) -> Option<Var> {
    let program = head.program;
    let (
        ir::Stmt::Assign {
            target,
            value: Expr::NewObject(ty),
        },
        ir::Stmt::Eval(Expr::Call { callee, args, .. }),
    ) = (made, call)
    else {
        return None;
    };
    let Some((Expr::Var(receiver), passed)) = args.split_first() else {
        return None;
    };
    let found = callee_key(program, callee).and_then(|key| member(project, key));
    let owned =
        found.is_some_and(|m| m.role == Role::Constructor && Some(m.owner) == TypeKey::of(ty));
    let alone = receiver == target && !passed.iter().any(|arg| mentions(arg, *target));
    (owned && alone).then_some(*target)
}

/// Groups each object made and the constructor the next statement runs on it into the one `new` they were; `early` holds the places in the list, as it was, of the objects made that `frame::early` proves of the outermost list.
fn construct(
    list: &mut Vec<Stmt>,
    head: &Head,
    project: &Project,
    reached: &Reached,
    frame: &frame::Frame,
    early: &BTreeSet<usize>,
) {
    let mut out: Vec<Stmt> = Vec::with_capacity(list.len());
    for (at, stmt) in list.drain(..).enumerate() {
        // The statement before is the one at `at - 1` wherever it is still a simple one.
        let proved = at.checked_sub(1).is_some_and(|made| early.contains(&made));
        match (out.pop(), stmt) {
            (Some(Stmt::Simple(made)), Stmt::Simple(call))
                if (proved && constructed(&made, &call, head, project).is_some())
                    || constructs(&made, &call, head, project, reached, frame) =>
            {
                let parts = vec![made, call];
                let kind = Group::Construct;
                out.push(Stmt::Sugar(Box::new(Sugar::Group { kind, parts })));
            }
            (before, stmt) => {
                out.extend(before);
                out.push(stmt);
            }
        }
    }
    *list = out;
}

/// The type the receiver of a call is declared to hold: a variable's, that of the field it is loaded from, or what the script's function it is the result of returns.
fn declared_type<'a>(receiver: &'a Expr, head: &'a Head, project: &'a Project) -> Option<&'a str> {
    let mut e = whole(receiver);
    // A call already read as a method call is the call still.
    if let Expr::Sugar {
        kind: ExprSugar::Method,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    let kind = match e {
        Expr::Var(var) => head.vars.get(var).map(|info| &info.kind),
        Expr::Sugar {
            kind: ExprSugar::Field { owner },
            e: load,
        } => match &**load {
            Expr::Load { ty, offset, .. } if ty.bytes() == REFERENCE => {
                let field = project.field(*owner, *offset);
                let reference = field.filter(|field| field.bytes == REFERENCE);
                reference.map(|field| &field.kind)
            }
            _ => None,
        },
        Expr::Call { callee, .. } => {
            let key = callee_key(head.program, callee)?;
            project.signatures.get(&key).map(|held| &held.result)
        }
        _ => None,
    };
    match kind? {
        Kind::Object(name) => Some(name.as_str()),
        _ => None,
    }
}

/// Whether a names list names virtual slot `slot` of the type `ty`, on the type or on a base of it.
// ⚠ The printer looks a slot's name up the same way: a call this takes is written under that name.
fn named(cx: &Context<'_>, ty: TypeKey, slot: u32) -> bool {
    slot_name(cx, ty, slot).is_some()
}

/// The name a names list gives virtual slot `slot` of the type `ty`, on the type or on a base of it.
fn slot_name<'a>(cx: &Context<'a>, ty: TypeKey, slot: u32) -> Option<&'a str> {
    let names = cx.names;
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        if let Some(name) = names.method(ty, slot) {
            return Some(name);
        }
        ty = cx.project.types.get(&ty)?.base?;
    }
    None
}

/// Whether a call's first argument is the object its callee is a method of, and the callee's own name says which method.
fn on_receiver(callee: &Callee, args: &[Expr], head: &Head, cx: &Context<'_>) -> bool {
    let project = cx.project;
    let Some(receiver) = args.first() else {
        return false;
    };
    match callee {
        // ⚠ A class's slot is one method down its whole chain, so its number names it on any
        // receiver. An interface numbers its own slots from 0: without the interface the number
        // names nothing, and such a call stays as it is unless the receiver is declared to hold
        // the type named, or a names list gives the slot a name to be written under.
        Callee::Virtual {
            class, ty, slot, ..
        } => {
            let key = TypeKey::of_name(ty);
            let declared = key.and_then(|key| project.types.get(&key));
            let held = declared_type(receiver, head, project);
            declared.is_some_and(|facts| facts.base.is_some())
                || held.is_some_and(|name| name == ty.as_str() || name == class.as_str())
                || key.is_some_and(|key| named(cx, key, *slot))
        }
        // ⚠ A struct's method takes the address of its value, which reads as no receiver.
        Callee::Local { .. } | Callee::Import { .. } => {
            let found = callee_key(head.program, callee).and_then(|key| member(project, key));
            let owner = found.and_then(|m| project.types.get(&m.owner));
            found.is_some_and(|m| m.role == Role::Method && !m.is_static)
                && owner.is_some_and(|facts| !facts.is_value())
        }
        _ => false,
    }
}

/// Whether a builtin's name says it is the `==` or the `!=` its type declares.
fn is_operator(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("op_Equality" | "op_Inequality")
    )
}

/// Reads one call as the method call, the use of a property or the operator it is proved to be.
fn read(e: &mut Expr, stands: Stands, head: &Head, cx: &Context<'_>) {
    let Expr::Call {
        callee,
        args,
        returns,
    } = &*e
    else {
        return;
    };
    let options = &cx.options;
    let property = || {
        let read = options.on(Pass::Properties);
        let runtime = accessed(callee, args, *returns, stands);
        read && runtime
    };
    let kind = match callee {
        Callee::Builtin { path, .. } => {
            let operator = options.on(Pass::Operators) && args.len() == 2 && is_operator(path);
            match (operator, property()) {
                (true, _) => Some(ExprSugar::Operator),
                (false, true) => Some(ExprSugar::Property),
                (false, false) => None,
            }
        }
        _ if property() => Some(ExprSugar::Property),
        _ => {
            let method = options.on(Pass::Receivers) && on_receiver(callee, args, head, cx);
            method.then_some(ExprSugar::Method)
        }
    };
    if let Some(kind) = kind {
        *e = Expr::sugar(kind, std::mem::replace(e, Expr::GuidEmpty));
    }
}

/// Reads a call on a receiver as `receiver.Method(..)`, an object made and constructed as `new T(..)` and a type's own `==` as the operator.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let options = &cx.options;
    let calls =
        options.on(Pass::Receivers) || options.on(Pass::Operators) || options.on(Pass::Properties);
    let constructors = options.on(Pass::Constructors) && !cx.project.members.is_empty();
    if !calls && !constructors {
        return;
    }
    let tree::Function { head, body } = f;
    let head: &Head = head;
    let (reached, frame) = match constructors {
        true => (Reached::of(body), frame::Frame::of(body, head, cx.project)),
        false => (Reached::default(), frame::Frame::default()),
    };
    let mut outermost = match constructors {
        true => Some(frame::early(body, head, cx.project, &reached, &frame)),
        false => None,
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        if constructors {
            // The outermost list is the first taken off.
            let proved = outermost.take().unwrap_or_default();
            construct(list, head, cx.project, &reached, &frame, &proved);
        }
        for stmt in list {
            if calls {
                let root = standing(stmt);
                tree::own_exprs_mut(stmt, &mut |top| {
                    let whole: *const Expr = &*top;
                    top.walk_mut(&mut |e| {
                        // The walk reaches the whole expression last, at the place it was handed.
                        let stands = match std::ptr::eq(&*e, whole) {
                            true => root,
                            false => Stands::Inside,
                        };
                        read(e, stands, head, cx);
                    });
                });
            }
            work.extend(tree::nested_mut(stmt));
        }
    }
}

/// What a type is in the source, by the flags its program states.
fn kind(facts: Option<&TypeFacts>) -> ClassKind {
    let Some(facts) = facts else {
        return ClassKind::Class;
    };
    let has = |bit: u32| facts.flags & bit != 0;
    match facts.is_value() {
        true if has(GENERATED) => ClassKind::StateMachine,
        true if has(STRUCT) => ClassKind::Struct,
        true => ClassKind::Enum,
        false if facts.base.is_none() && has(ABSTRACT) => ClassKind::Interface,
        false => ClassKind::Class,
    }
}

/// A type as the project knows it, with no method yet.
fn class_of(project: &Project, key: TypeKey) -> Class {
    let facts = project.types.get(&key);
    let fields = project.fields.get(&key).map(Vec::as_slice);
    let field = |held: &FieldFacts| Field {
        offset: held.offset,
        bytes: held.bytes,
        kind: held.kind.clone(),
        is_static: false,
    };
    Class {
        key,
        kind: kind(facts),
        base: facts.and_then(|facts| facts.base),
        interfaces: facts
            .map(|facts| facts.interfaces.clone())
            .unwrap_or_default(),
        fields: fields.unwrap_or_default().iter().map(field).collect(),
        methods: Vec::new(),
        nested: Vec::new(),
    }
}

/// Moves each compiler-made class into the class whose methods alone make its objects.
fn nest(
    classes: &mut BTreeMap<TypeKey, Class>,
    makers: &BTreeMap<TypeKey, BTreeSet<Option<TypeKey>>>,
) {
    let mut around: BTreeMap<TypeKey, TypeKey> = BTreeMap::new();
    for (made, by) in makers {
        let mut who = by.iter();
        if let (Some(Some(by)), None) = (who.next(), who.next()) {
            if by != made && classes.contains_key(made) && classes.contains_key(by) {
                around.insert(*made, *by);
            }
        }
    }
    // The classes around a class; `None` where they close a ring, which nests nowhere.
    let depth = |key: TypeKey| {
        let mut key = key;
        for steps in 0..MAX_CHAIN {
            match around.get(&key) {
                Some(outer) => key = *outer,
                None => return Some(steps),
            }
        }
        None
    };
    let mut inner: Vec<(usize, TypeKey, TypeKey)> = around
        .iter()
        .filter_map(|(made, by)| Some((depth(*made)?, *made, *by)))
        .collect();
    // Deepest first, so that a class is whole when it goes into the one around it.
    inner.sort_by_key(|&(depth, made, _)| (Reverse(depth), made));
    for (_, made, by) in inner {
        let Some(class) = classes.remove(&made) else {
            continue;
        };
        match classes.get_mut(&by) {
            Some(outer) => outer.nested.push(class),
            None => {
                classes.insert(made, class);
            }
        }
    }
}

/// The fields the functions of a unit read and write as a type's that the project's layouts hold no row of, by type and offset, with the widths seen.
#[derive(Default)]
struct Unrowed(BTreeMap<(TypeKey, u32), BTreeSet<u32>>);

impl Unrowed {
    /// Notes each access in `body` read as a field the project has no row of.
    fn note(&mut self, body: &[Stmt], project: &Project) {
        let mut seen = |owner: TypeKey, offset: u32, bytes: u32| {
            if project.field(owner, offset).is_none() {
                self.0.entry((owner, offset)).or_default().insert(bytes);
            }
        };
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| {
                let Expr::Sugar {
                    kind: ExprSugar::Field { owner },
                    e,
                } = e
                else {
                    return;
                };
                match &**e {
                    Expr::Load { ty, offset, .. } => seen(*owner, *offset, ty.bytes()),
                    Expr::LoadBytes { offset, bytes, .. } => seen(*owner, *offset, *bytes),
                    _ => {}
                }
            });
        });
        tree::each_sugar(body, &mut |sugar| {
            let Sugar::Group {
                kind: Group::Field { owner },
                parts,
            } = sugar
            else {
                return;
            };
            for part in parts {
                match part {
                    ir::Stmt::Store { kind, offset, .. } => seen(*owner, *offset, kind.bytes()),
                    ir::Stmt::StoreBytes { offset, bytes, .. } => seen(*owner, *offset, *bytes),
                    _ => {}
                }
            }
        });
    }

    /// Adds to each class of `program` the fields its functions use and no row states: of no proved kind, as wide as every access of it is.
    // ⚠ A field accessed at two widths is left out: which bytes it holds is not said.
    fn add(self, classes: &mut BTreeMap<TypeKey, Class>, project: &Project, program: u32) {
        for ((owner, offset), widths) in self.0 {
            let [bytes] = widths.into_iter().collect::<Vec<u32>>()[..] else {
                continue;
            };
            let home = project.types.get(&owner).map(|facts| facts.home);
            if !classes.contains_key(&owner) && home != Some(program) {
                continue;
            }
            let class = classes
                .entry(owner)
                .or_insert_with(|| class_of(project, owner));
            let field = Field {
                offset,
                bytes,
                kind: Kind::Unknown,
                is_static: false,
            };
            let at = class.fields.partition_point(|held| held.offset < offset);
            class.fields.insert(at, field);
        }
    }
}

/// Whether a body holds a wait: only the body of an `async` method does, since a wait is made of a proved state machine alone.
fn waits(body: &[Stmt]) -> bool {
    let mut found = false;
    tree::each_sugar(body, &mut |sugar| {
        found |= matches!(sugar, Sugar::Await { .. })
    });
    found
}

/// A program's finished functions under the types the project proves them members of; every other function stays free.
pub fn unit(
    program: u32,
    imports: Vec<Import>,
    functions: Vec<tree::Function>,
    cx: &Context<'_>,
) -> Unit {
    if !cx.options.on(Pass::Classes) {
        let mut flat = Unit::flat(program, imports, functions);
        for method in &mut flat.functions {
            method.is_async = waits(&method.function.body);
        }
        return flat;
    }
    let project = cx.project;
    let mut classes: BTreeMap<TypeKey, Class> = BTreeMap::new();
    // ponytail: every type of the install is looked at for each program; index the types by home if units take long.
    for (key, facts) in &project.types {
        let fields = project.fields.get(key).is_some_and(|held| !held.is_empty());
        if facts.home == program && (fields || !facts.constants.is_empty()) {
            classes.insert(*key, class_of(project, *key));
        }
    }

    // Which classes' methods make the objects of each compiler-made type; `None` is a free function.
    let mut makers: BTreeMap<TypeKey, BTreeSet<Option<TypeKey>>> = BTreeMap::new();
    let mut unrowed = Unrowed::default();
    let mut free = Vec::new();
    for f in functions {
        unrowed.note(&f.body, project);
        let found = member(project, FunctionKey::new(f.head.program, f.head.at));
        let owner = found.map(|m| m.owner);
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| {
                let made = match e {
                    Expr::NewObject(ty) if ty.flags & GENERATED != 0 => TypeKey::of(ty),
                    _ => None,
                };
                if let Some(made) = made {
                    makers.entry(made).or_default().insert(owner);
                }
            });
        });
        let is_async = waits(&f.body);
        let Some(found) = found else {
            let mut method = Method::free(f);
            method.is_async = is_async;
            // The one function a program names as what readies its statics.
            if method.function.head.name == "initialize" {
                method.role = Role::StaticConstructor;
            }
            free.push(method);
            continue;
        };
        let base = project.types.get(&found.owner).and_then(|t| t.base);
        let filled = |(slot, base): (u32, TypeKey)| project.slot(base, slot).is_some();
        let method = Method {
            function: f,
            role: found.role,
            is_static: found.is_static,
            is_async,
            slot: found.slot,
            overrides: found.slot.zip(base).is_some_and(filled),
        };
        let class = classes
            .entry(found.owner)
            .or_insert_with(|| class_of(project, found.owner));
        class.methods.push(method);
    }

    unrowed.add(&mut classes, project, program);
    for class in classes.values_mut() {
        class.methods.sort_by_key(|method| method.function.head.at);
        if class.methods.iter().any(|m| m.role == Role::Lambda) {
            class.kind = ClassKind::Closure;
        }
    }
    nest(&mut classes, &makers);
    Unit {
        program,
        imports,
        classes: classes.into_values().collect(),
        functions: free,
        front: None,
    }
}

/// What the class model of one program holds and what `run` read in its functions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Classes, those nested in another included.
    pub classes: usize,
    /// Functions under a class.
    pub methods: usize,
    /// Functions no type claims.
    pub free: usize,
    pub constructors: usize,
    pub lambdas: usize,
    /// Getters and setters.
    pub accessors: usize,
    pub overrides: usize,
    /// Calls read as `receiver.Method(..)`.
    pub method_calls: usize,
    /// Objects made and constructed that read as one `new`.
    pub constructions: usize,
    /// Calls read as an operator.
    pub operators: usize,
    /// Calls read as the use of a property.
    pub properties: usize,
}

/// Counts a unit's classes and members, and the readings `run` left in its functions.
pub fn counts(unit: &Unit) -> Counts {
    let mut counts = Counts {
        free: unit.functions.len(),
        ..Counts::default()
    };
    let mut methods: Vec<&Method> = unit.functions.iter().collect();
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        counts.classes += 1;
        counts.methods += class.methods.len();
        methods.extend(&class.methods);
        open.extend(&class.nested);
    }
    for method in methods {
        match method.role {
            Role::Constructor => counts.constructors += 1,
            Role::Lambda => counts.lambdas += 1,
            Role::Getter | Role::Setter => counts.accessors += 1,
            _ => {}
        }
        counts.overrides += usize::from(method.overrides);
        tree::each_expr(&method.function.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Sugar {
                    kind: ExprSugar::Method,
                    ..
                } => counts.method_calls += 1,
                Expr::Sugar {
                    kind: ExprSugar::Operator,
                    ..
                } => counts.operators += 1,
                Expr::Sugar {
                    kind: ExprSugar::Property,
                    ..
                } => counts.properties += 1,
                _ => {}
            });
        });
        counts.constructions += constructions(&method.function.body);
    }
    counts
}

/// The groups of a body that hold a call of a script's own function: the constructions `run` grouped.
fn constructions(body: &[Stmt]) -> usize {
    let mut found = 0;
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::If { then, els, .. } => work.extend([then.as_slice(), els.as_slice()]),
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push(body);
                }
                Stmt::Switch { cases, .. } => work.extend(cases.iter().map(|c| c.body.as_slice())),
                Stmt::Sugar(sugar) => {
                    let scripted = |part: &ir::Stmt| {
                        let ir::Stmt::Eval(Expr::Call { callee, .. }) = part else {
                            return false;
                        };
                        matches!(callee, Callee::Local { .. } | Callee::Import { .. })
                    };
                    found += usize::from(matches!(
                        &**sugar,
                        Sugar::Group { kind: Group::Construct, parts } if parts.iter().any(scripted)
                    ));
                    work.extend(sugar.parts().bodies);
                }
                _ => {}
            }
        }
    }
    found
}

#[cfg(test)]
#[path = "classes_tests.rs"]
mod tests;
