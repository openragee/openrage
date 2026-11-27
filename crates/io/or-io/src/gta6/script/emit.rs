//! Output: a program's unit written as one file, by the level asked.
//!
//! One file a program, named by the program's hash and never by a list, so that two runs with
//! different lists write the same names. The text depends on the unit, the names, the project
//! and the options alone: functions and members stand by offset, classes by the lowest offset of
//! their code, fields by offset, so two runs write the same bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use super::class::{Class, ClassKind, Field, Method, Role, Unit};
use super::ir::{Callee, Expr, Kind, TypeKey, Var};
use super::passes::closures;
use super::print::{self, Lambdas, ListingWriter, Read};
use super::project::types;
use super::tree;
use super::{Context, File, Level, Pass, Stats};

/// What is still to write of the classes and of those nested in them.
enum Work<'a> {
    /// A class `depth` levels in, and whether a blank line stands before it.
    Open(&'a Class, usize, bool),
    Close(usize),
}

/// Writes one program: a lifted listing at `Lifted`, else its free functions and then its classes as source.
pub fn unit(unit: &Unit, cx: &Context<'_>, stats: &Stats) -> Vec<File> {
    let text = match cx.options.level {
        Level::Lifted => lifted(unit),
        Level::Structured | Level::Source => source(unit, cx, stats),
    };
    let name = format!("program_{:08X}.cs", unit.program);
    vec![File { name, text }]
}

/// Every function of the unit in the order of their offsets, in a listing's own syntax.
fn lifted(unit: &Unit) -> String {
    let mut all: Vec<&tree::Function> = unit.functions.iter().map(|m| &m.function).collect();
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        all.extend(class.methods.iter().map(|m| &m.function));
        open.extend(&class.nested);
    }
    all.sort_by_key(|f| f.head.at);
    let mut writer = ListingWriter::new(unit.program, &unit.imports);
    for f in all {
        writer.function(f);
    }
    writer.finish()
}

/// The header, the functions no type owns, then the classes, with one blank line between two of them.
// ⚠ The free functions come first: C# takes a function at the top of a file only before the first type.
fn source(unit: &Unit, cx: &Context<'_>, stats: &Stats) -> String {
    let imports: Vec<u32> = unit.imports.iter().map(|i| i.program).collect();
    let (natives, addresses) = natives(unit);
    let read = Read {
        front: unit.front,
        natives: &natives,
        by_address: addresses.len(),
        counted: "called",
    };
    let mut out = print::header(unit.program, &imports, &read, cx, stats);
    out.push_str(&globals(unit, cx));
    let lambdas = in_place(unit, cx);
    let hidden = print::Hidden::of(unit, &lambdas, cx);
    let mut free: Vec<&Method> = unit.functions.iter().collect();
    free.sort_by_key(|method| method.function.head.at);
    for method in free {
        out.push('\n');
        print::method_hiding(&mut out, method, None, 0, cx, &lambdas, &hidden);
    }

    // An explicit stack: classes nest as deep as the closures they were made for.
    let top = ordered(&unit.classes).into_iter().rev();
    let mut work: Vec<Work<'_>> = top.map(|class| Work::Open(class, 0, true)).collect();
    while let Some(item) = work.pop() {
        let (class, depth, gap) = match item {
            Work::Open(class, depth, gap) => (class, depth, gap),
            Work::Close(depth) => {
                indent(&mut out, depth);
                out.push_str("}\n");
                continue;
            }
        };
        // A closure class every member of which is written in place is no class to write.
        if hidden.hides(class.key) {
            continue;
        }
        if gap {
            out.push('\n');
        }
        let name = types::display(cx.project, cx.names, class.key);
        opened(&mut out, class, &name, depth, cx);
        let mut fields: Vec<&Field> = class.fields.iter().collect();
        fields.sort_by_key(|field| (field.is_static, field.offset));
        for field in &fields {
            field_line(&mut out, class, field, depth + 1, cx);
        }
        let mut methods: Vec<&Method> = class.methods.iter().collect();
        methods.sort_by_key(|method| method.function.head.at);
        // One blank line before a member that is no field, unless it is the first thing in the class.
        let mut written = !fields.is_empty();
        for method in methods {
            // A lambda's body written where it is bound is no member to write again.
            let at = method.function.head.at;
            if method.role == Role::Lambda && lambdas.contains_key(&at) {
                continue;
            }
            if std::mem::replace(&mut written, true) {
                out.push('\n');
            }
            print::method_hiding(
                &mut out,
                method,
                Some((class.key, name.as_str())),
                depth + 1,
                cx,
                &lambdas,
                &hidden,
            );
        }
        work.push(Work::Close(depth));
        for (n, nested) in ordered(&class.nested).into_iter().enumerate().rev() {
            work.push(Work::Open(nested, depth + 1, written || n > 0));
        }
    }
    out
}

/// What the comment over the globals and statics a file declares says of them.
const GLOBALS: &str = "the globals and statics of this program that its code names, each of \
                       the kind the program states of it or every access of it in the project \
                       agrees on, and Cell where neither says one";

/// The globals and statics of the program its own code names, under a blank line and a comment, each declared with the kind the project holds of it; nothing where none is named or `kinds` is off.
// ⚠ Before the first function: C# takes a statement at the top of a file only there.
fn globals(unit: &Unit, cx: &Context<'_>) -> String {
    if !(cx.options.on(Pass::Kinds) && print::true_csharp(&cx.options)) {
        return String::new();
    }
    let own = |var: &Var| match *var {
        Var::Global {
            program,
            offset,
            statics,
        } if program == unit.program => Some((statics, offset)),
        _ => None,
    };
    let mut named: BTreeSet<(bool, u32)> = BTreeSet::new();
    // ⚠ What a callee does through an address no access of the project shows: a global whose
    // address the code hands on is declared only by what the program states of it.
    let mut addressed: BTreeSet<(bool, u32)> = BTreeSet::new();
    for body in bodies(unit) {
        tree::each_simple(body, &mut |s| {
            named.extend(s.targets().iter().filter_map(own))
        });
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Var(var) => named.extend(own(var)),
                Expr::AddrOf { var, .. } => {
                    named.extend(own(var));
                    addressed.extend(own(var));
                }
                _ => {}
            });
        });
    }
    if named.is_empty() {
        return String::new();
    }
    // One line a type, in the order its first variable stands.
    let mut lines: Vec<(String, Vec<String>)> = Vec::new();
    for (statics, offset) in named {
        let seen = !addressed.contains(&(statics, offset));
        let ty = global_type(unit.program, offset, statics, seen, cx);
        let space = if statics { "Static" } else { "Global" };
        let name = format!("{space}_{offset:X}");
        match lines.iter_mut().find(|(held, _)| *held == ty) {
            Some((_, names)) => names.push(name),
            None => lines.push((ty, vec![name])),
        }
    }
    let mut out = String::from("\n");
    out.push_str(&print::comment(GLOBALS, 0));
    for (ty, names) in lines {
        out.push_str(&print::laid(&format!("{ty} {};", names.join(", ")), 0));
        out.push('\n');
    }
    out
}

/// The type a global or a static of `program` is declared with: the kind the program states of it, else, where `seen`, the one every access agrees on where that kind is as wide as the accesses, else `Cell`.
fn global_type(program: u32, offset: u32, statics: bool, seen: bool, cx: &Context<'_>) -> String {
    let info = cx.project.programs.get(&program);
    let stated = info.and_then(|info| match statics {
        true => info.statics.get(&offset),
        false => info.globals.get(&offset),
    });
    if let Some(kind) = stated {
        return print::kind_text(kind, cx.names, true);
    }
    let cell = cx.project.cell(program, offset, statics).filter(|_| seen);
    let Some(cell) = cell else {
        return "Cell".to_owned();
    };
    let kind = &cell.kind;
    let fits = match cell.bytes {
        1 => matches!(kind, Kind::Bool | Kind::I8 | Kind::U8),
        2 => matches!(kind, Kind::I16 | Kind::U16),
        4 => matches!(kind, Kind::I32 | Kind::U32 | Kind::F32 | Kind::Hash),
        8 => matches!(
            kind,
            Kind::Unknown
                | Kind::I64
                | Kind::U64
                | Kind::F64
                | Kind::Pointer
                | Kind::Object(_)
                | Kind::String
                | Kind::Array(_)
        ),
        _ => false,
    };
    match (fits, kind) {
        (true, kind) => print::kind_text(kind, cx.names, true),
        (false, Kind::Unknown) if cell.bytes > 8 && cell.bytes.is_multiple_of(8) => {
            format!("Cells{}", cell.bytes / 8)
        }
        (false, _) => "Cell".to_owned(),
    }
}

/// Every body of a unit: its functions' and those of every class, nested ones too.
fn bodies(unit: &Unit) -> Vec<&[tree::Stmt]> {
    let mut bodies: Vec<&[tree::Stmt]> = Vec::new();
    bodies.extend(unit.functions.iter().map(|m| m.function.body.as_slice()));
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        bodies.extend(class.methods.iter().map(|m| m.function.body.as_slice()));
        open.extend(&class.nested);
    }
    bodies
}

/// The lambdas of a unit whose bodies are written where they are bound and left out of their closure class, by offset.
// ⚠ Each is a body the unit names once, where a function that is no lambda binds it: read
// with another program that names it too, that program would name a function written nowhere.
fn in_place<'u>(unit: &'u Unit, cx: &Context<'_>) -> Lambdas<'u> {
    let mut bodies = Lambdas::new();
    if !(cx.options.on(Pass::Lambdas) && cx.options.on(Pass::Delegates)) {
        return bodies;
    }
    let mut all: Vec<&'u Method> = unit.functions.iter().collect();
    let mut made: Vec<&'u Method> = Vec::new();
    let mut open: Vec<&'u Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        all.extend(&class.methods);
        if class.kind == ClassKind::Closure {
            made.extend(class.methods.iter().filter(|m| m.role == Role::Lambda));
        }
        open.extend(&class.nested);
    }
    if made.is_empty() {
        return bodies;
    }
    // How often the unit names each of its functions: a call, or a reference bound or kept.
    let mut named: BTreeMap<u32, u32> = BTreeMap::new();
    for method in &all {
        tree::each_expr(&method.function.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::FunctionRef { program, at } if *program == unit.program => {
                    *named.entry(*at).or_default() += 1;
                }
                Expr::Call {
                    callee: Callee::Local { at, .. },
                    ..
                } => *named.entry(*at).or_default() += 1,
                _ => {}
            });
        });
    }
    let mut candidates = Lambdas::new();
    for method in made {
        let body = &method.function;
        let once = named.get(&body.head.at) == Some(&1);
        if once && !method.is_async && closures::inline(body).is_some() {
            candidates.insert(body.head.at, body);
        }
    }
    for binder in all.iter().filter(|method| method.role != Role::Lambda) {
        for at in print::bound_in_place(&binder.function, &candidates, cx) {
            if let Some(body) = candidates.get(&at) {
                bodies.insert(at, *body);
            }
        }
    }
    bodies
}

/// Classes in the order they are written: by the lowest offset of their code, then by key; one with no code after those that have some.
fn ordered(classes: &[Class]) -> Vec<&Class> {
    let mut all: Vec<(Option<u32>, &Class)> = classes.iter().map(|c| (lowest(c), c)).collect();
    all.sort_by_key(|(at, class)| (at.is_none(), *at, class.key));
    all.into_iter().map(|(_, class)| class).collect()
}

/// The lowest offset a method of `class`, or of a class nested in it, starts at.
fn lowest(class: &Class) -> Option<u32> {
    let mut found: Option<u32> = None;
    let mut open = vec![class];
    while let Some(class) = open.pop() {
        let own = class.methods.iter().map(|m| m.function.head.at).min();
        found = match (found, own) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        open.extend(&class.nested);
    }
    found
}

/// The head of a class and the brace that opens it.
fn opened(out: &mut String, class: &Class, name: &str, depth: usize, cx: &Context<'_>) {
    let shown = |key: &TypeKey| types::display(cx.project, cx.names, *key);
    // ⚠ An enum of C# holds constants alone, and a value type names no base: an enum with
    // members is written as the value type it is, and the base of one as a comment.
    let bare = class.fields.is_empty() && class.methods.is_empty() && class.nested.is_empty();
    let (word, note) = match class.kind {
        ClassKind::Class => ("class", ""),
        ClassKind::Struct => ("struct", ""),
        ClassKind::Interface => ("interface", ""),
        ClassKind::Enum if bare => ("enum", ""),
        ClassKind::Enum => ("struct", " // an enum"),
        ClassKind::Delegate => ("class", " // a delegate type"),
        ClassKind::Closure => ("class", " // a closure"),
        ClassKind::StateMachine => ("class", " // a state machine"),
    };
    let value = matches!(class.kind, ClassKind::Struct | ClassKind::Enum);
    let base = class.base.as_ref().map(shown);
    let mut listed: Vec<String> = Vec::new();
    if let (false, Some(base)) = (value, &base) {
        listed.push(base.clone());
    }
    // ⚠ What follows the name of a C# enum is the integer its values are: its base and its
    // interfaces are a comment's.
    let as_enum = word == "enum" && cx.options.on(Pass::Kinds);
    let interfaces: Vec<String> = class.interfaces.iter().map(shown).collect();
    let mut head = format!("{word} {name}");
    match as_enum {
        true => {
            if let Some(of) = types::enumeration(cx.project, class.key) {
                let underlying = print::kind_text(&of.underlying, cx.names, true);
                let _ = write!(head, " : {underlying}");
            }
            let mut said: Vec<String> = base.iter().map(|base| format!("base {base}")).collect();
            if !interfaces.is_empty() {
                said.push(format!("implements {}", interfaces.join(", ")));
            }
            if !said.is_empty() {
                let _ = write!(head, " // {}", said.join("; "));
            }
        }
        false => {
            listed.extend(interfaces);
            if !listed.is_empty() {
                let _ = write!(head, " : {}", listed.join(", "));
            }
            head.push_str(note);
            if let (true, Some(base)) = (value, &base) {
                let _ = write!(head, " // base {base}");
            }
        }
    }
    // A list of bases that passes the width goes on under the head, one level in.
    indent(out, depth);
    out.push_str(&print::laid(&head, depth));
    out.push('\n');
    indent(out, depth);
    out.push_str("{\n");
    if class.kind != ClassKind::Enum {
        return;
    }
    // The values an enum's constants take: a member each where a list names it, the rest said.
    let constants = cx
        .project
        .types
        .get(&class.key)
        .map(|facts| facts.constants.as_slice())
        .unwrap_or_default();
    let members = match as_enum {
        true => enum_members(class.key, cx),
        false => Vec::new(),
    };
    let mut left: Vec<String> = Vec::new();
    let mut written: BTreeSet<i64> = BTreeSet::new();
    for value in constants {
        let line = members.iter().find(|(held, _)| held == value);
        match line.filter(|_| written.insert(*value)) {
            Some((_, line)) => {
                indent(out, depth + 1);
                out.push_str(line);
                out.push('\n');
            }
            None => left.push(value.to_string()),
        }
    }
    if !left.is_empty() {
        let said = match members.is_empty() {
            true => format!("values: {}", left.join(", ")),
            false => format!("values no list names a member of: {}", left.join(", ")),
        };
        out.push_str(&print::comment(&said, depth + 1));
    }
}

/// The members of the enum `key` a list names, each as the line that declares it, by the constant whose value it is: one name a member and one member a value.
fn enum_members(key: TypeKey, cx: &Context<'_>) -> Vec<(i64, String)> {
    let members = types::declared_members(cx.project, cx.names, key);
    let line = |(value, low, member): (i64, u64, &str)| {
        let written = match low <= 0x1_0000 {
            true => low.to_string(),
            false => format!("0x{low:X}"),
        };
        (value, format!("{member} = {written},"))
    };
    members.into_iter().map(line).collect()
}

/// One field: `[static] <type> <name>;`, with the offset that a user's name leaves unsaid.
fn field_line(out: &mut String, class: &Class, field: &Field, depth: usize, cx: &Context<'_>) {
    let ty = match &field.kind {
        // A field of no proved kind is as wide as its bytes say.
        Kind::Unknown if field.bytes == 8 => "Cell".to_owned(),
        Kind::Unknown if field.bytes > 8 && field.bytes.is_multiple_of(8) => {
            format!("Cells{}", field.bytes / 8)
        }
        Kind::Unknown => format!("Bytes{}", field.bytes),
        kind => print::kind_text(kind, cx.names, true),
    };
    let fixed = if field.is_static { "static " } else { "" };
    indent(out, depth);
    let _ = match cx.names.field(class.key, field.offset) {
        Some(name) => writeln!(out, "{fixed}{ty} {name}; // f_{:X}", field.offset),
        None => writeln!(out, "{fixed}{ty} f_{:X};", field.offset),
    };
}

/// The natives the functions of a unit call: by hash, each with whether the program itself names it, and by the address of its handler each one no hash is stated of.
fn natives(unit: &Unit) -> (BTreeMap<u64, bool>, BTreeSet<u32>) {
    let mut bodies: Vec<&[tree::Stmt]> = Vec::new();
    bodies.extend(unit.functions.iter().map(|m| m.function.body.as_slice()));
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        bodies.extend(class.methods.iter().map(|m| m.function.body.as_slice()));
        open.extend(&class.nested);
    }
    let mut natives = BTreeMap::new();
    let mut addresses = BTreeSet::new();
    for body in bodies {
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::Call {
                    callee:
                        Callee::Native {
                            name, hash, rva, ..
                        },
                    ..
                } = e
                {
                    // A name that is only the hash spelled `_0x..`, or the address spelled `rva_..`, names nothing.
                    let spelled = name.starts_with("_0x") || name.starts_with("rva_");
                    // A compiled program states no hash of a native no list knows by its address: each is its address then.
                    match (*hash, *rva) {
                        (0, rva) if rva != 0 => {
                            addresses.insert(rva);
                        }
                        (hash, _) => {
                            natives.insert(hash, !spelled);
                        }
                    }
                }
            });
        });
    }
    (natives, addresses)
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;
