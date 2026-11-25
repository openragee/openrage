//! Output: a program's unit written as one file, by the level asked.
//!
//! One file a program, named by the program's hash and never by a list, so that two runs with
//! different lists write the same names. The text depends on the unit, the names, the project
//! and the options alone: functions and members stand by offset, classes by the lowest offset of
//! their code, fields by offset, so two runs write the same bytes.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::class::{Class, ClassKind, Field, Method, Unit};
use super::ir::{Callee, Expr, Kind, TypeKey};
use super::print::{self, ListingWriter, Read};
use super::project::types;
use super::tree;
use super::{Context, File, Level, Stats};

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
    let natives = natives(unit);
    let read = Read {
        front: unit.front,
        natives: &natives,
        counted: "called",
    };
    let mut out = print::header(unit.program, &imports, &read, cx, stats);
    let mut free: Vec<&Method> = unit.functions.iter().collect();
    free.sort_by_key(|method| method.function.head.at);
    for method in free {
        out.push('\n');
        print::method(&mut out, method, None, 0, cx);
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
            if std::mem::replace(&mut written, true) {
                out.push('\n');
            }
            print::method(
                &mut out,
                method,
                Some((class.key, name.as_str())),
                depth + 1,
                cx,
            );
        }
        work.push(Work::Close(depth));
        for (n, nested) in ordered(&class.nested).into_iter().enumerate().rev() {
            work.push(Work::Open(nested, depth + 1, written || n > 0));
        }
    }
    out
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
    listed.extend(class.interfaces.iter().map(shown));
    indent(out, depth);
    let _ = write!(out, "{word} {name}");
    if !listed.is_empty() {
        let _ = write!(out, " : {}", listed.join(", "));
    }
    out.push_str(note);
    if let (true, Some(base)) = (value, &base) {
        let _ = write!(out, " // base {base}");
    }
    out.push('\n');
    indent(out, depth);
    out.push_str("{\n");
    if class.kind != ClassKind::Enum {
        return;
    }
    // The values an enum's constants take: no list names the constants themselves.
    let constants = cx
        .project
        .types
        .get(&class.key)
        .map(|facts| &facts.constants);
    for few in constants.into_iter().flat_map(|all| all.chunks(16)) {
        let few: Vec<String> = few.iter().map(i64::to_string).collect();
        indent(out, depth + 1);
        let _ = writeln!(out, "// values: {}", few.join(", "));
    }
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

/// The natives the functions of a unit call, by hash, each with whether the program itself names it.
fn natives(unit: &Unit) -> BTreeMap<u64, bool> {
    let mut bodies: Vec<&[tree::Stmt]> = Vec::new();
    bodies.extend(unit.functions.iter().map(|m| m.function.body.as_slice()));
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        bodies.extend(class.methods.iter().map(|m| m.function.body.as_slice()));
        open.extend(&class.nested);
    }
    let mut natives = BTreeMap::new();
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
                    // A compiled program states no hash of a native no list knows by its address: each is its address then.
                    let key = match hash {
                        0 => u64::from(*rva),
                        hash => *hash,
                    };
                    // A name that is only the hash spelled `_0x..`, or the address spelled `rva_..`, names nothing.
                    let spelled = name.starts_with("_0x") || name.starts_with("rva_");
                    natives.insert(key, !spelled);
                }
            });
        });
    }
    natives
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;
