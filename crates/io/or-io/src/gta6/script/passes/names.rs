//! Names: a role name for a variable that one fact names.
//!
//! A variable is named from the native it holds the only result of, `playerPed` from
//! `PLAYER_PED_ID`, or from the parameter of a native it is passed as on every use. Natives,
//! types and programs take their names where the printer writes them, from [`Names`].
//!
//! ⚠ A name is a label and changes no code, but a wrong one misleads: a variable assigned twice,
//! one whose address is taken, or one two parameters name differently keeps its slot name.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta5::script::natives::{NativeName, NativeParam};
use crate::gta6::script::ir::{Callee, Expr, Head, Kind, Stmt, Var, VarInfo, Width};
use crate::gta6::script::names::Names;
use crate::gta6::script::print;
use crate::gta6::script::tree;
use crate::gta6::script::Stats;

/// Names each variable of `f` that one fact names: the named native it holds the result of, or the named parameter it is always passed as.
pub fn run(f: &mut tree::Function, names: &Names, _stats: &mut Stats) {
    if f.head.incomplete.is_some() {
        return;
    }
    let Gathered { facts, mut taken } = gather(&f.body, &f.head, names);
    taken.insert(f.head.name.clone());
    taken.extend(f.head.vars.values().filter_map(|info| info.name.clone()));
    let exposed: Vec<u32> = facts
        .iter()
        .filter_map(|(var, facts)| match var {
            Var::Local(n) if facts.addressed => Some(*n),
            _ => None,
        })
        .collect();

    // In the order of the variables, so that which of two takes the bare name never changes.
    for (var, facts) in &facts {
        let named = (f.head.vars.get(var)).is_some_and(|info| info.name.is_some());
        let Some(role) = role(*var, facts, &exposed, &f.head).filter(|_| !named) else {
            continue;
        };
        let name = unique(&role, &mut taken);
        let info = f.head.vars.entry(*var);
        info.or_insert_with(|| VarInfo::new(Width::Cell)).name = Some(name);
    }
}

/// What a function's body says of one variable.
#[derive(Default)]
struct Facts {
    /// The assignments that name the variable as a target.
    assigned: u32,
    /// The role of the native an assignment holds the result of.
    result_of: Option<String>,
    /// Every read, the taking of its address included.
    read: u32,
    addressed: bool,
    /// The reads that pass it as the whole of a named parameter of a named native.
    passed: u32,
    /// The name of those parameters.
    parameter: Option<String>,
    /// Two of those parameters have different names.
    disagree: bool,
}

#[derive(Default)]
struct Gathered {
    facts: BTreeMap<Var, Facts>,
    /// The names the printer writes in front of a call's first dot, which a variable must not share.
    taken: BTreeSet<String>,
}

// An explicit stack: a body nests as deep as its listing does.
fn gather(body: &[tree::Stmt], head: &Head, names: &Names) -> Gathered {
    let mut gathered = Gathered::default();
    let mut work: Vec<&[tree::Stmt]> = vec![body];
    while let Some(stmts) = work.pop() {
        for stmt in stmts {
            match stmt {
                tree::Stmt::Simple(s) => gathered.simple(s, head, names),
                tree::Stmt::If { cond, then, els } => {
                    gathered.expr(cond, head, names);
                    work.push(then);
                    work.push(els);
                }
                tree::Stmt::While { cond, body } | tree::Stmt::DoWhile { body, cond } => {
                    gathered.expr(cond, head, names);
                    work.push(body);
                }
                tree::Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    for s in [init, step].into_iter().flatten() {
                        gathered.simple(s, head, names);
                    }
                    if let Some(cond) = cond {
                        gathered.expr(cond, head, names);
                    }
                    work.push(body);
                }
                tree::Stmt::Switch { value, cases } => {
                    gathered.expr(value, head, names);
                    work.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                tree::Stmt::Return(Some(value)) => gathered.expr(value, head, names),
                tree::Stmt::Return(None)
                | tree::Stmt::Break
                | tree::Stmt::Continue
                | tree::Stmt::Goto(_)
                | tree::Stmt::Label(_)
                | tree::Stmt::Unreachable => {}
            }
        }
    }
    gathered
}

impl Gathered {
    fn simple(&mut self, s: &Stmt, head: &Head, names: &Names) {
        s.each_expr(&mut |e| self.expr(e, head, names));
        for target in s.targets() {
            self.facts.entry(*target).or_default().assigned += 1;
        }
        let held = match s {
            Stmt::Assign { target, value } => result_role(value, names).map(|role| (*target, role)),
            _ => None,
        };
        if let Some((target, role)) = held {
            self.facts.entry(target).or_default().result_of = Some(role);
        }
    }

    fn expr(&mut self, top: &Expr, head: &Head, names: &Names) {
        top.walk(&mut |e| match e {
            Expr::Var(var) => self.facts.entry(*var).or_default().read += 1,
            Expr::AddrOf { var, .. } => {
                let facts = self.facts.entry(*var).or_default();
                facts.read += 1;
                facts.addressed = true;
            }
            Expr::Call { callee, args, .. } => {
                let printed = print::callee_name(callee, names);
                let front = printed.split('.').next().unwrap_or_default();
                self.taken.insert(front.to_owned());
                let listed = match callee {
                    Callee::Native { hash, .. } => names.native(*hash),
                    _ => None,
                };
                if let Some(native) = listed {
                    self.passed(native, args, head);
                }
            }
            _ => {}
        });
    }

    /// Counts each argument that is a variable passed as the whole of a named parameter of `native`.
    fn passed(&mut self, native: &NativeName, args: &[Expr], head: &Head) {
        for (arg, param) in args.iter().zip(whole_params(native, args, head)) {
            let role = param.and_then(|param| parameter_role(&param.name));
            let (Expr::Var(var), Some(role)) = (uncast(arg), role) else {
                continue;
            };
            let facts = self.facts.entry(*var).or_default();
            facts.passed += 1;
            facts.disagree |= (facts.parameter.as_ref()).is_some_and(|held| *held != role);
            facts.parameter = Some(role);
        }
    }
}

/// `e` without the casts around it: a reading of a value's low bits is still that value's role.
fn uncast(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// The role a call of a named native gives the variable that holds its result.
fn result_role(value: &Expr, names: &Names) -> Option<String> {
    let Expr::Call {
        callee: Callee::Native { name, hash, .. },
        ..
    } = uncast(value)
    else {
        return None;
    };
    let name = names.native(*hash).map_or(name, |listed| &listed.name);
    role_of_native(name)
}

/// The role one fact gives `var`: the result of a native before the parameter of one.
fn role(var: Var, facts: &Facts, exposed: &[u32], head: &Head) -> Option<String> {
    if facts.addressed || matches!(var, Var::Global { .. }) {
        return None;
    }
    // ⚠ A parameter also holds what its caller passed: no assignment is its only value.
    let holds_one =
        facts.assigned == 1 && !matches!(var, Var::Param(_)) && !covered(var, exposed, head);
    if let Some(role) = facts.result_of.as_ref().filter(|_| holds_one) {
        return Some(role.clone());
    }
    let always = facts.read > 0 && facts.read == facts.passed && !facts.disagree;
    facts.parameter.clone().filter(|_| always)
}

/// Whether a write through the address of an earlier local reaches `var`.
// ⚠ The address of `local_n` is also that of the locals after it; how many, only the kind of
// `local_n` says. A local of no known kind is taken to be the one cell the listing shows.
fn covered(var: Var, exposed: &[u32], head: &Head) -> bool {
    let Var::Local(n) = var else {
        return false;
    };
    exposed.iter().any(|m| {
        let cells = match head.vars.get(&Var::Local(*m)).map(|info| &info.kind) {
            Some(Kind::Vector | Kind::Handle) => 2,
            Some(Kind::Aggregate(cells)) => *cells,
            _ => 1,
        };
        *m < n && n - *m < cells
    })
}

/// `role`, or `role_2` and on while a name of the function has it; the name is taken from then on.
fn unique(role: &str, taken: &mut BTreeSet<String>) -> String {
    let mut name = role.to_owned();
    let mut n = 1;
    while taken.contains(&name) {
        n += 1;
        name = format!("{role}_{n}");
    }
    taken.insert(name.clone());
    name
}

/// For each argument, the parameter of `native` it is the whole of.
// ⚠ A list's parameters are another game's or a guess. They name an argument only where their
// cells, counted by `param_cells`, are exactly the call's: a parameter then lines up with one
// argument of its width, or with a run of one-cell arguments, none of which is the whole of it.
fn whole_params<'n>(
    native: &'n NativeName,
    args: &[Expr],
    head: &Head,
) -> Vec<Option<&'n NativeParam>> {
    let nothing = vec![None; args.len()];
    let mut whole = nothing.clone();
    let mut next = 0;
    for param in &native.params {
        let cells = param_cells(&param.ty);
        let width = args.get(next).and_then(|arg| print::cells_of(arg, head));
        let spread = |run: &[Expr]| run.iter().all(|arg| print::cells_of(arg, head) == Some(1));
        if width == Some(cells) {
            whole[next] = Some(param);
            next += 1;
        } else if args.get(next..next + cells as usize).is_some_and(spread) {
            next += cells as usize;
        } else {
            return nothing;
        }
    }
    match next == args.len() {
        true => whole,
        false => nothing,
    }
}

/// The cells a parameter of the type a list names takes in a GTA VI script: two for a world handle and for a vector.
fn param_cells(ty: &str) -> u32 {
    match ty {
        "Entity" | "Ped" | "Vehicle" | "Object" | "Pickup" | "Vector3" => 2,
        _ => 1,
    }
}

/// `playerPed` for `PLAYER_PED_ID`: a native's name in camel case, without a leading `GET_` or `CREATE_` and a trailing `_ID`.
fn role_of_native(name: &str) -> Option<String> {
    let name = name.trim_start_matches('_');
    let name = ["GET_", "CREATE_"]
        .iter()
        .find_map(|verb| name.strip_prefix(*verb))
        .unwrap_or(name);
    let name = name.strip_suffix("_ID").unwrap_or(name);
    let mut role = String::new();
    for word in name.split('_').filter(|word| !word.is_empty()) {
        if !word.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let (first, rest) = word.split_at(1);
        match role.is_empty() {
            true => role.push_str(&first.to_ascii_lowercase()),
            false => role.push_str(&first.to_ascii_uppercase()),
        }
        role.push_str(&rest.to_ascii_lowercase());
    }
    spelled(&role)
}

/// A list's name for a parameter as a variable's, where it is a name and no placeholder.
fn parameter_role(name: &str) -> Option<String> {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
    let placeholder = ["", "p", "unk", "unknown", "param", "arg"]
        .iter()
        .any(|held| stem.eq_ignore_ascii_case(held));
    spelled(name).filter(|_| !placeholder)
}

/// `name` as the printer may write it for a variable: `None` for what is no identifier or reads as a name the printer makes.
fn spelled(name: &str) -> Option<String> {
    const MADE: [&str; 9] = [
        "Global_", "Static_", "String_", "func_", "label_", "block_", "type_", "module_",
        "program_",
    ];
    let starts = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    let identifier = starts && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let made = Var::from_name(name).is_some() || MADE.iter().any(|made| name.starts_with(*made));
    if !identifier || made {
        return None;
    }
    // C# spells a keyword used as a name with `@` in front.
    Some(match KEYWORDS.contains(&name) {
        true => format!("@{name}"),
        false => name.to_owned(),
    })
}

/// The words C# reserves, and `var`.
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
];

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
