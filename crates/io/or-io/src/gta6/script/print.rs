//! Writing the structured form as text: readable C#-like source, and the listing's own syntax
//! again, so that what was read can be read back.
//!
//! The readable form changes nothing of the tree. It leaves unwritten what the tree already
//! says: a cast of a value of that very kind, a 64-bit reading where every bit counts alike, a
//! zero test of a truth value. Memory prints as `p->f_10`, the field at byte 0x10 of what `p`
//! points at, or `v.f_10` for byte 0x10 of the variable `v` itself, and a variable is declared
//! where it is first assigned.
//!
//! The readable form is written two ways. With the four first passes alone it is what it always
//! was, byte for byte. With any other pass on it is true C#: a C# parser reads every line, and
//! no operator, cast or literal says to C# anything but what the tree means.
//!
//! ⚠ `(float)x` in the form of old is the listing's `as_f32(x)`: the low bits of `x` read as a
//! float, never a conversion. A conversion is a call, `i32_to_f32(x)`. True C# writes
//! `as_f32(x)` for the first, so that a cast there always converts.

#[path = "print_sugar.rs"]
mod constructs;
#[path = "print_lay.rs"]
mod lay;
#[path = "print_member.rs"]
mod members;
#[path = "print_old.rs"]
mod old;
#[path = "print_spell.rs"]
mod spell;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write;

use super::class::{Method, Role};
use super::decode::TYPE_TOKEN;
use super::ir::{
    BinOp, Callee, Const, Expr, ExprSugar, FunctionKey, Head, Kind, Returns, Scalar, Stmt,
    StoreKind, TypeKey, UnOp, Var, VmType, Width,
};
use super::listing::{Import, Listing, Target, Wrapper};
use super::names::{declared_natives, Names};
use super::passes::closures;
use super::passes::types::yielded_shape;
use super::project::signatures::native_key;
use super::project::TypeFacts;
use super::tree;
use super::{Context, Level, Options, Pass, Project, Stats};

/// The strength a unary operator and a cast bind at, above every binary operator.
const UNARY: u8 = 12;
/// The strength of a name, a call and an index.
const POSTFIX: u8 = 13;

/// What every header of true C# says of the words no C# has.
const WORDS: [&str; 7] = [
    "Cell is one 64-bit cell of no proved kind, Cells2 two of them",
    "Handle is a value of two cells the code only takes from the engine, compares and hands back to it",
    "f_10 is the field at byte 0x10",
    "as_f32(x) is the bits of x read as a float",
    "p->f_10++ steps the int at f_10",
    "a local declared without a value starts as zero",
    "0x3F800000 /* 1.0f */ is a number of no proved kind whose bits are those of the float beside it",
];

/// What a header says of a constant written without the cast C# would ask for, where `signs` is on.
const NARROW: &str = "0x89ABCDEF alone, as an argument of a native or where an int is taken, is \
                      the int of those bits, which C# writes unchecked((int)0x89ABCDEF)";

/// What a header says of the two things `typeof` stands in, where `operators` is on.
const TOKEN: &str = "typeof(T) is the Type and typeof(T).TypeHandle the token of the type alone";

/// What a header says of a `return` in an `async` method, where `await` is on.
const HANDED: &str = "return; in an async method that is not void hands back the task its \
                      state machine holds, which the code returns by a field of the machine; \
                      async Task<T> heads one whose machine gives its task a value of T, and \
                      async void one of which the machine says no more";

/// What a header says of the variable a `foreach` over an array may be given, where `foreach` is on.
const ITEM: &str = "item, item_2 and on is the element a foreach reads where the code reads it \
                    from the array in place and keeps it in no variable of its own";

/// What a header says of the names of variables, which no list gives.
const DERIVED: &str = "a variable named otherwise than by its slot (p1, local_2, v_.., stack_..) \
                       has a name derived from the native, the type, the field or the getter its \
                       value comes from or goes to, or is i, j or k where it only counts a loop \
                       and indexes with the count, and no list names a variable";

/// The comment that says what the words of a file mean, by the passes that write them.
fn words(options: &Options) -> String {
    let mut said: Vec<&str> = WORDS.to_vec();
    if options.on(Pass::Signs) {
        said.push(NARROW);
    }
    if options.on(Pass::Operators) {
        said.push(TOKEN);
    }
    if options.on(Pass::Await) {
        said.push(HANDED);
    }
    if options.on(Pass::Foreach) {
        said.push(ITEM);
    }
    if options.any(&[Pass::Names, Pass::RoleNames, Pass::NativeParams]) {
        said.push(DERIVED);
    }
    lay::comment(&format!("words: {}.", said.join("; ")), 0)
}

/// The runtime's method that makes a `Type` of a type's token, by the path a user's list of the runtime's methods gives it: what C# writes `typeof` for.
const TYPE_OF: &str = "System.Type.GetTypeFromHandle";

/// The runtime's comparison of two objects, which compares two boxed values by value.
const OBJECT_EQUALS: &str = "System.Object.Equals";

/// The runtime's conversion of an object to an `int`, which throws where the value is past an `int`'s range.
const CONVERT_TO_INT32: &str = "System.Convert.ToInt32";

/// The flags of a value type that say it is a struct, and that the compiler made it: neither is an enum.
const STRUCT: u32 = 0x04;
const GENERATED: u32 = 0x10_0000;

/// What a header says of text read from a lifted listing.
pub(super) const LISTING: &str =
    "a lifted listing, the text another decompiler wrote of this script";

/// What a header says of text read from a compiled program.
pub(super) const PROGRAM: &str = "the compiled script's own bytes, of which nothing was run";

/// Writes one function as readable source, naming what `names` has a name for.
pub fn function(out: &mut String, f: &tree::Function, names: &Names) {
    let mut writer = Writer::new(out, false, names, &f.head);
    writer.readable_function(&f.body);
}

/// Writes one function as source, by the passes `cx` has on.
pub fn function_in(out: &mut String, f: &tree::Function, cx: &Context<'_>) {
    let mut writer = Writer::in_context(out, cx, &f.head);
    writer.readable_function(&f.body);
}

/// Writes one function `depth` levels in, as the member its role makes it of the class whose key and name are given; free where none is.
pub fn method(
    out: &mut String,
    method: &Method,
    class: Option<(TypeKey, &str)>,
    depth: usize,
    cx: &Context<'_>,
) {
    method_in(out, method, class, depth, cx, &Lambdas::new());
}

/// The lambdas of a program whose bodies are written where they are bound, each by its offset.
pub(super) type Lambdas<'a> = BTreeMap<u32, &'a tree::Function>;

/// Writes one function as [`method`] does, the body of each lambda of `lambdas` it binds written where it binds it.
pub(super) fn method_in(
    out: &mut String,
    method: &Method,
    class: Option<(TypeKey, &str)>,
    depth: usize,
    cx: &Context<'_>,
    lambdas: &Lambdas<'_>,
) {
    let mut writer = Writer::in_context(out, cx, &method.function.head);
    writer.depth = depth;
    // ⚠ A lambda's own body binds none in place: written in place itself, it is written by a
    // writer that holds no lambda, and what it binds must be written in one place either way.
    if method.role != Role::Lambda {
        writer.sited = bound_in_place(&method.function, lambdas, cx);
        writer.lambdas = Some(lambdas);
    }
    writer.as_member(method, class);
}

/// `e` under its source readings and its casts to a whole cell, which change no bit of what it holds.
fn underneath(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. }
    | Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// What a delegate is bound to and the offset of the function bound, where `parts` are a delegate made and bound to a function of `program` that is read as a lambda's body.
fn bound_lambda(parts: &[Stmt], program: u32) -> Option<(&Expr, u32)> {
    let [Stmt::Assign { target, value }, Stmt::Eval(call)] = parts else {
        return None;
    };
    if !matches!(underneath(value), Expr::NewObject(_)) {
        return None;
    }
    let Expr::Call { args, .. } = underneath(call) else {
        return None;
    };
    let [object, held, Expr::Sugar {
        kind: ExprSugar::Lambda,
        e: function,
    }] = args.as_slice()
    else {
        return None;
    };
    let Expr::FunctionRef { program: of, at } = &**function else {
        return None;
    };
    let made = *underneath(object) == Expr::Var(*target);
    (made && *of == program).then_some((held, *at))
}

/// The name a variable of a function is written under where no class gives it a word of its own.
fn label(head: &Head, var: Var) -> String {
    match head.vars.get(&var).and_then(|info| info.name.as_deref()) {
        Some(name) => name.to_owned(),
        None => var.to_string(),
    }
}

/// Whether `body`, written as a lambda in `binder` where it is bound to `held`, reads the object it was bound to and no other.
// ⚠ In place the body names the binder's variable where it named its own first parameter. That
// variable must hold one object for as long as the function runs, and nothing of the body may
// go by its name.
fn reads_as_bound(binder: &tree::Function, held: &Expr, body: &tree::Function) -> bool {
    let mut mentioned: BTreeSet<Var> = (1..body.head.params).map(Var::Param).collect();
    tree::each_simple(&body.body, &mut |s| {
        mentioned.extend(s.targets().iter().copied())
    });
    tree::each_expr(&body.body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                mentioned.insert(*var);
            }
        });
    });
    let reads = mentioned.remove(&Var::Param(0));
    let object = match underneath(held) {
        Expr::Var(object @ (Var::Local(_) | Var::Value { .. } | Var::Stack { .. })) => Some(object),
        _ => None,
    };
    // A body that never reads its object says nothing of what it was bound to: only its own
    // variables must not go by the name of the variable it is given to.
    if !reads {
        let text = object.map(|object| label(&binder.head, *object));
        return mentioned
            .iter()
            .all(|var| Some(label(&body.head, *var)) != text);
    }
    let Some(object) = object else {
        return false;
    };
    let (mut assigned, mut addressed) = (0usize, false);
    tree::each_simple(&binder.body, &mut |s| {
        assigned += s
            .targets()
            .iter()
            .filter(|target| *target == object)
            .count();
    });
    tree::each_expr(&binder.body, &mut |top| {
        top.walk(&mut |e| {
            addressed |= matches!(e, Expr::AddrOf { var, .. } if var == object);
        });
    });
    let text = label(&binder.head, *object);
    let own = |var: &Var| label(&body.head, *var) != text;
    assigned == 1 && !addressed && mentioned.iter().all(own)
}

/// The lambdas of `bodies` that `binder` binds and whose bodies are written where it binds them, by offset.
// ⚠ Only a statement every setting of the passes writes is read: a construct may leave some of
// what it holds unwritten, and a body counted here is written nowhere else.
pub(super) fn bound_in_place(
    binder: &tree::Function,
    bodies: &Lambdas<'_>,
    cx: &Context<'_>,
) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let on =
        true_csharp(&cx.options) && cx.options.on(Pass::Delegates) && cx.options.on(Pass::Lambdas);
    if !on || bodies.is_empty() || binder.head.incomplete.is_some() {
        return found;
    }
    let steady = closures::inlinable(binder);
    if steady.is_empty() {
        return found;
    }
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&[tree::Stmt]> = vec![binder.body.as_slice()];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                tree::Stmt::If { then, els, .. } => {
                    work.extend([then.as_slice(), els.as_slice()]);
                }
                tree::Stmt::While { body, .. }
                | tree::Stmt::DoWhile { body, .. }
                | tree::Stmt::For { body, .. } => work.push(body.as_slice()),
                tree::Stmt::Switch { cases, .. } => {
                    work.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                tree::Stmt::Sugar(sugar) => {
                    let tree::Sugar::Group {
                        kind: tree::Group::Delegate,
                        parts,
                    } = &**sugar
                    else {
                        continue;
                    };
                    let Some((held, at)) = bound_lambda(parts, binder.head.program) else {
                        continue;
                    };
                    let Some(body) = bodies.get(&at).copied() else {
                        continue;
                    };
                    if steady.contains(&at) && reads_as_bound(binder, held, body) {
                        found.insert(at);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

/// Whether the passes on ask for text a C# parser reads: any pass beyond the four `decompile` always ran.
// ⚠ With those four alone the text is what it always was: `var[2]`, a group in braces,
// `(float)x` for bits read as a float.
pub(super) fn true_csharp(options: &Options) -> bool {
    let mut on = options.passes.iter();
    on.any(|pass| !Pass::BASELINE.contains(&pass))
}

/// A line that stands `indent` levels in, broken where it passes the width; the indentation of its first line is the caller's.
pub(super) fn laid(text: &str, indent: usize) -> String {
    lay::lay(text, indent)
}

/// One comment `indent` levels in, as lines of words no wider than the width, each ended.
pub(super) fn comment(text: &str, indent: usize) -> String {
    lay::comment(text, indent)
}

/// Writes a listing's functions as readable source, under a comment saying what named things in it.
pub struct SourceWriter<'a> {
    cx: Context<'a>,
    out: String,
}

impl<'a> SourceWriter<'a> {
    /// A writer of the form the four first passes always had.
    pub fn new(names: &'a Names) -> Self {
        SourceWriter::in_context(&Context::baseline(names))
    }

    /// A writer of the form the passes `cx` has on ask for.
    pub fn in_context(cx: &Context<'a>) -> Self {
        SourceWriter {
            cx: *cx,
            out: String::new(),
        }
    }

    /// Writes one function under a blank line.
    pub fn function(&mut self, f: &tree::Function) {
        self.out.push('\n');
        function_in(&mut self.out, f, &self.cx);
    }

    /// The source under its header: the program, its imports, the lists that named things and what `stats` counted.
    pub fn finish(self, listing: &Listing<'_>, stats: &Stats) -> String {
        let mut text = match true_csharp(&self.cx.options) {
            true => {
                let imports: Vec<u32> = listing.imports.iter().map(|i| i.program).collect();
                let natives = declared_natives(listing.declarations());
                let read = Read {
                    front: Some(LISTING),
                    natives: &natives,
                    by_address: 0,
                    counted: "declared",
                };
                header(listing.program, &imports, &read, &self.cx, stats)
            }
            false => listing_header(listing, self.cx.names, stats),
        };
        text.push_str(&self.out);
        text
    }
}

/// What a header says its text was read from.
pub(super) struct Read<'r> {
    /// The front end, in words; `None` where what is written does not say.
    pub front: Option<&'static str>,
    /// The natives by hash, each with whether the program itself names it.
    pub natives: &'r BTreeMap<u64, bool>,
    /// The natives known only by the address of their handler: a compiled program states no hash of one no list reaches.
    pub by_address: usize,
    /// What the natives counted are to the program: `declared` by a listing, `called` by its functions.
    pub counted: &'static str,
}

/// The kinds of thing the user's lists gave a name to, in the words a header lists them by.
fn named_kinds(names: &Names) -> Vec<&'static str> {
    let held = [
        (!names.programs.is_empty(), "programs"),
        (!names.types.is_empty(), "types"),
        (
            !names.fields.is_empty() || !names.field_names.is_empty(),
            "fields",
        ),
        (
            !names.methods.is_empty() || !names.method_names.is_empty(),
            "methods",
        ),
        (!names.functions.is_empty(), "functions"),
        (
            !names.texts.is_empty() || !names.texts64.is_empty(),
            "the texts of hashes",
        ),
        (
            !names.hash_slots.is_empty(),
            "the arguments that are hashes",
        ),
        (
            !names.native_hashes.is_empty(),
            "the hashes of native handlers",
        ),
        (!names.builtins.is_empty(), "the runtime's methods"),
        (!names.words.is_empty(), "candidate words"),
    ];
    (held.into_iter())
        .filter(|(any, _)| *any)
        .map(|(_, kind)| kind)
        .collect()
}

/// A label a user supplied, kept to the one line of the comment it stands in.
fn one_line(label: &str) -> String {
    label.replace(['\n', '\r'], " ")
}

/// The comment a file of true C# opens with: one fact a line, broken where it passes the width, and no line with nothing to say.
// ⚠ No date, no path and no version of the tool: the same input writes the same bytes.
pub(super) fn header(
    program: u32,
    imports: &[u32],
    read: &Read<'_>,
    cx: &Context<'_>,
    stats: &Stats,
) -> String {
    let names = cx.names;
    let named = |hash: u32| match names.program(u64::from(hash)) {
        Some(name) => format!("0x{hash:08X} ({name})"),
        None => format!("0x{hash:08X}"),
    };
    let mut out = String::new();
    let mut say = |text: String| out.push_str(&lay::comment(&text, 0));
    say(format!(
        "OpenRage: GTA VI script, program {}",
        named(program)
    ));
    if let Some(front) = read.front {
        say(format!("read from: {front}"));
    }
    let asked = Options::at(cx.options.level).passes;
    let off: Vec<&str> = asked
        .iter()
        .filter(|pass| !cx.options.on(*pass))
        .map(Pass::name)
        .collect();
    let off = match off.is_empty() {
        true => "none".to_owned(),
        false => off.join(", "),
    };
    let level = cx.options.level.name();
    say(format!("level: {level}; passes off: {off}"));
    if !imports.is_empty() {
        let imports: Vec<String> = imports.iter().map(|hash| named(*hash)).collect();
        say(format!("imports: {}", imports.join(", ")));
    }

    let total = read.natives.len() + read.by_address;
    let listed = |hash: &u64| names.native(*hash).is_some();
    let by_list = read.natives.keys().filter(|hash| listed(hash)).count();
    let own = read.natives.iter();
    let own = own.filter(|(hash, own)| **own && !listed(hash)).count();
    let by_hash = read.natives.len() - by_list - own;
    // ⚠ A compiled program names no native and states the hash of none: what it calls it calls
    // by the address of a handler, so those two counts are said of it only where they are not zero.
    let compiled = read.front == Some(PROGRAM);
    let mut shown: Vec<String> = Vec::new();
    if let Some(db) = &names.natives {
        shown.push(format!("{by_list} named by `{}`", one_line(db.label())));
    }
    if own > 0 || !compiled {
        let verb = if shown.is_empty() { "named " } else { "" };
        shown.push(format!("{own} {verb}by the program itself"));
    }
    if by_hash > 0 || !compiled {
        shown.push(format!("{by_hash} shown by hash"));
    }
    if read.by_address > 0 {
        let by_address = read.by_address;
        shown.push(format!(
            "{by_address} shown by the address of their handler"
        ));
    }
    let unlisted = match &names.natives {
        Some(_) => "",
        None => "; no natives list names a native",
    };
    if total > 0 {
        let counted = read.counted;
        say(format!(
            "natives: {total} {counted}: {}{unlisted}",
            shown.join(", ")
        ));
    }
    match names.lists.is_empty() {
        true => say("names: no names list supplied".to_owned()),
        false => {
            // A row that is the hash of its name proves itself; any other is the user's word.
            let labelled = |(n, list): (usize, &String)| match names.rows.get(n) {
                // A list of words states no hash: a word names only what its own hashes are found to be.
                _ if names.candidates.get(n).is_some_and(|words| *words > 0) => format!(
                    "`{}` ({} words with no hash, each a name only of what its own hashes are \
                     found to be)",
                    one_line(list),
                    names.candidates.get(n).copied().unwrap_or(0)
                ),
                Some((proved, trusted)) => format!(
                    "`{}` ({proved} proved by hash, {trusted} on trust)",
                    one_line(list)
                ),
                None => format!("`{}`", one_line(list)),
            };
            let lists: Vec<String> = names.lists.iter().enumerate().map(labelled).collect();
            let kinds = named_kinds(names);
            let kinds = match kinds.split_last() {
                None => "names".to_owned(),
                Some((one, [])) => (*one).to_owned(),
                Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            };
            say(format!("names: {kinds} from {}", lists.join(", ")));
        }
    }
    say(format!(
        "{} function(s), {} with no body; {} goto(s) left",
        stats.functions, stats.incomplete, stats.gotos_out
    ));
    out.push_str(&words(&cx.options));
    out
}

/// The comment a decompiled listing opens with in the form of old.
fn listing_header(listing: &Listing<'_>, names: &Names, stats: &Stats) -> String {
    let program = |hash: u32| match names.program(u64::from(hash)) {
        Some(name) => format!("0x{hash:08X} ({name})"),
        None => format!("0x{hash:08X}"),
    };
    let mut out = String::new();
    // ⚠ What was read is another decompiler's text. No compiled script was.
    out.push_str(
        "// Readable source of a lifted listing: read from the text another decompiler wrote of \
         this GTA VI script, not from the compiled script.\n\
         // A name is the listing's or a supplied list's, else an offset or a hash: `f_10` is \
         the field at byte 0x10, `as_f32(x)` the bits of `x` read as a float.\n",
    );
    let _ = writeln!(out, "// program {}", program(listing.program));
    if !listing.imports.is_empty() {
        let imports: Vec<String> = listing.imports.iter().map(|i| program(i.program)).collect();
        let _ = writeln!(out, "// imports: {}", imports.join(", "));
    }

    let natives = declared_natives(listing.declarations());
    let listed = |hash: &u64| names.native(*hash).is_some();
    let by_list = natives.keys().filter(|hash| listed(hash)).count();
    let by_listing = natives
        .iter()
        .filter(|(hash, own)| **own && !listed(hash))
        .count();
    let by_hash = natives.len() - by_list - by_listing;
    let total = natives.len();
    let _ = match &names.natives {
        _ if total == 0 => writeln!(out, "// natives: none declared"),
        Some(db) => writeln!(
            out,
            "// natives: {total} declared: {by_list} named by `{}`, {by_listing} by the listing \
             itself, {by_hash} shown by hash",
            db.label()
        ),
        None => writeln!(
            out,
            "// natives: {total} declared, no natives list names a native: {by_listing} named by \
             the listing itself, {by_hash} shown by hash"
        ),
    };
    let _ = match names.lists.is_empty() {
        true => writeln!(
            out,
            "// names: no names list supplied: a type or a program the listing does not name is \
             shown by its hash"
        ),
        false => {
            let lists: Vec<String> = names.lists.iter().map(|l| format!("`{l}`")).collect();
            writeln!(
                out,
                "// names: types and programs from {}",
                lists.join(", ")
            )
        }
    };
    let _ = writeln!(
        out,
        "// {} function(s), {} of them with no body in the listing; {} goto(s) left, of {} read",
        stats.functions, stats.incomplete, stats.gotos_out, stats.gotos_in
    );
    out
}

/// Writes functions back in the listing's own syntax, with the declarations they call through.
pub struct ListingWriter {
    out: String,
    declared: BTreeMap<String, Wrapper>,
    names: Names,
}

impl ListingWriter {
    /// Starts a listing of the program `program` with its head written.
    pub fn new(program: u32, imports: &[Import]) -> Self {
        let mut out = String::new();
        for import in imports {
            let _ = writeln!(
                out,
                "// import {}: program_0x{:08X}, types [{}, {})",
                import.index, import.program, import.types.start, import.types.end
            );
        }
        let _ = writeln!(out, "[VmProgram(0x{program:08X}U)]");
        let _ = writeln!(out, "public static class Program_{program:08X}");
        out.push_str("{\n");
        ListingWriter {
            out,
            declared: BTreeMap::new(),
            names: Names::default(),
        }
    }

    pub fn function(&mut self, f: &tree::Function) {
        let mut writer = Writer::new(&mut self.out, true, &self.names, &f.head);
        writer.declared = Some(&mut self.declared);
        writer.plain_function(&f.body);
    }

    /// The listing, closed with the declaration classes of everything the functions called.
    pub fn finish(mut self) -> String {
        declarations(&mut self.out, &self.declared);
        self.out.push_str("}\n");
        self.out
    }
}

/// What is still to write of a body: the stack that stands in for the nesting of its blocks.
enum Work<'b> {
    Run(&'b [tree::Stmt], usize),
    Line(usize, &'static str),
    /// A line made for one statement: the head of a `catch`, a case's label.
    Text(usize, String),
    /// The `else` of an `if` whose first arm is written.
    Else(&'b [tree::Stmt], usize),
    Cases(&'b [tree::Case], usize, Labels),
    /// The `while (..);` that closes a `do`.
    Until(&'b Expr, usize),
    /// Simple statements a construct holds, a line each.
    Simples(&'b [Stmt], usize),
}

/// How the labels of one switch are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Labels {
    Decimal,
    /// As the 32 bits they are: `0x8A0B0C0D`.
    Hex,
    /// The same, and a hash the user's vocabulary has the text of as that text.
    Hashes,
}

struct Writer<'a> {
    out: &'a mut String,
    /// The listing's own syntax when set, the readable form otherwise.
    plain: bool,
    names: &'a Names,
    head: &'a Head,
    /// The wrappers the plain form called through, by path.
    declared: Option<&'a mut BTreeMap<String, Wrapper>>,
    /// The assignments the readable form declares their target at, by where the tree holds them.
    // ⚠ Asked only whether a statement is in it: nothing is ever written in its order.
    sites: HashSet<*const Stmt>,
    /// What the programs of the install state: the text of a string, the type a field is declared with.
    project: &'a Project,
    /// The passes whose constructs are written as source, and the level.
    options: Options,
    /// Whether the readable form is true C#.
    sharp: bool,
    /// Whether source constructs are written as the form of old always wrote them, whatever pass is on.
    old: bool,
    /// The levels the function stands in: one for each class it is in.
    depth: usize,
    /// The receiver of a method that is not static, written `this`.
    this: Option<Var>,
    /// The one parameter of a setter, written `value`.
    setter: Option<Var>,
    /// The variables an `out` of the statement being written declares.
    outs: Vec<Var>,
    /// The lambdas of the program whose bodies are written where they are bound.
    lambdas: Option<&'a Lambdas<'a>>,
    /// Those of them the function being written binds.
    sited: BTreeSet<u32>,
    /// What the receiver is written as in the body of a lambda written where it is bound: the variable that holds the object there.
    bound: Option<String>,
    /// The task an `async` method's hidden last `return` hands back: a `return` of it elsewhere is written bare.
    back: Option<Expr>,
    /// What an `async` method hands back, where its machine proves it: the type its head is written with.
    task: Option<String>,
}

impl<'a> Writer<'a> {
    /// A writer of the listing's own syntax, or of the readable form as the four first passes always had it.
    // No pass is asked for its name: a source construct is written as that form always wrote it.
    fn new(out: &'a mut String, plain: bool, names: &'a Names, head: &'a Head) -> Self {
        Writer {
            out,
            plain,
            names,
            head,
            declared: None,
            sites: HashSet::new(),
            project: Project::empty(),
            options: Options::baseline(),
            sharp: false,
            old: true,
            depth: 0,
            this: None,
            setter: None,
            outs: Vec::new(),
            lambdas: None,
            sited: BTreeSet::new(),
            bound: None,
            back: None,
            task: None,
        }
    }

    /// A writer of the readable form the passes `cx` has on ask for.
    fn in_context(out: &'a mut String, cx: &Context<'a>, head: &'a Head) -> Self {
        Writer {
            project: cx.project,
            options: cx.options,
            sharp: true_csharp(&cx.options),
            old: false,
            ..Writer::new(out, false, cx.names, head)
        }
    }

    /// Writes one line `indent` levels into the function, broken where true C# passes the width.
    fn line(&mut self, indent: usize, text: &str) {
        let indent = self.depth + indent;
        push_indent(self.out, indent);
        match self.sharp && 4 * indent + text.len() > lay::WIDTH {
            true => self.out.push_str(&lay::lay(text, indent)),
            false => self.out.push_str(text),
        }
        self.out.push('\n');
    }

    fn plain_function(&mut self, body: &[tree::Stmt]) {
        let head = self.head;
        let ty = match head.returns {
            Returns::Void => "void",
            Returns::Cell => "VmValue",
            Returns::Cells => "VmValues",
        };
        let args: Vec<String> = (0..head.params).map(|n| format!("arg{n}")).collect();
        let typed: Vec<String> = args.iter().map(|arg| format!("VmValue {arg}")).collect();
        self.line(1, &format!("[VmFunction(0x{:06X})]", head.at));
        self.line(
            1,
            &format!(
                "public static {ty} {}({}) // 0x{:06X}",
                head.name,
                typed.join(", "),
                head.at
            ),
        );
        self.line(1, "{");
        if let Some(reason) = &head.incomplete {
            let message = quoted(&format!("{}: {reason}", head.name));
            self.line(2, &format!("// INCOMPLETE: {reason}"));
            self.line(
                2,
                &format!("throw new global::System.NotSupportedException({message});"),
            );
        } else {
            self.line(
                2,
                &format!(
                    "VmValue[] parameters = new VmValue[] {{ {} }};",
                    args.join(", ")
                ),
            );
            self.line(
                2,
                &format!("VmValue[] locals = new VmValue[{}];", head.locals),
            );
            for n in 0..head.params {
                self.line(2, &format!("ref VmValue p{n} = ref parameters[{n}];"));
            }
            for n in 0..head.locals {
                self.line(2, &format!("ref VmValue local_{n} = ref locals[{n}];"));
            }
            for (var, info) in head.vars.iter().filter(|(var, _)| var.is_temporary()) {
                let text = match info.width {
                    Width::Cell => format!("VmValue {var} = default;"),
                    Width::Cells => format!("VmValues {var} = null!;"),
                };
                self.line(2, &text);
            }
            self.body(body, 2);
        }
        self.line(1, "}");
        self.out.push('\n');
    }

    fn readable_function(&mut self, body: &[tree::Stmt]) {
        let head = self.head;
        if let Some(reason) = &head.incomplete {
            return self.unread_function(reason);
        }
        let name = self.function_name(head.program, head.at, &head.name);
        self.function_as("", &name, body);
    }

    /// The comment that stands in place of a function its front end gives no body for.
    fn unread_function(&mut self, reason: &str) {
        let head = self.head;
        let text = match self.sharp {
            true => format!("// func_{:06X}: no body ({reason})", head.at),
            false => format!(
                "// {} at 0x{:06X}: the listing holds no body ({reason})",
                head.name, head.at
            ),
        };
        self.line(0, &text);
    }

    /// Writes a function under `modifiers`, each word of them followed by a space, and the name `name`.
    fn function_as(&mut self, modifiers: &str, name: &str, body: &[tree::Stmt]) {
        let text = format!(
            "{modifiers}{} {name}({})",
            self.returned(),
            self.params(body)
        );
        self.headed(text, name);
        self.line(0, "{");
        self.block(body, 1);
        self.line(0, "}");
    }

    /// Writes a head; one of true C# whose name is not `func_` and its offset says the offset after it.
    fn headed(&mut self, text: String, name: &str) {
        let slot = format!("func_{:06X}", self.head.at);
        match self.sharp && name != slot {
            true => self.line(0, &format!("{text} // {slot}")),
            false => self.line(0, &text),
        }
    }

    /// The type a function hands back, where a type must stand.
    fn returned(&self) -> String {
        let head = self.head;
        if let Some(task) = &self.task {
            return task.clone();
        }
        if !self.sharp {
            let ty = match head.returns {
                Returns::Void => "void",
                Returns::Cell => "var",
                Returns::Cells => "var[]",
            };
            return ty.to_owned();
        }
        // What the calls of the function agree it yields, where the project gathered that.
        let key = FunctionKey::new(head.program, head.at);
        let proved = self.project.signatures.get(&key);
        match (head.returns, proved) {
            (Returns::Void, _) => "void".to_owned(),
            (Returns::Cell, Some(sig)) if sig.returns == 1 && sig.result != Kind::Unknown => {
                self.kind_text(&sig.result)
            }
            (Returns::Cells, Some(sig)) if sig.returns >= 2 => match &sig.result {
                Kind::Unknown => format!("Cells{}", sig.returns),
                kind => self.kind_text(kind),
            },
            (Returns::Cell, _) => "Cell".to_owned(),
            (Returns::Cells, _) => "Cells".to_owned(),
        }
    }

    /// The parameters of a head: in true C# a value of several cells once, and the receiver of a method not at all.
    fn params(&self, body: &[tree::Stmt]) -> String {
        let head = self.head;
        let mut written = Vec::new();
        let mut named: Option<BTreeSet<u32>> = None;
        let mut n = 0;
        while n < head.params {
            let var = Var::Param(n);
            if self.this != Some(var) {
                written.push(format!("{} {}", self.declared_type(var), self.var(var)));
            }
            n += 1;
            let cells = flat_cells(&Expr::Var(var), head).filter(|cells| self.sharp && *cells >= 2);
            let Some(cells) = cells else {
                continue;
            };
            // ⚠ The slots a value of several cells takes are its own and go unsaid; one the body
            // names, or a pass knows something of, is a parameter and is written.
            let named = named.get_or_insert_with(|| named_params(body));
            let end = (n - 1).saturating_add(cells).min(head.params);
            while n < end && !head.vars.contains_key(&Var::Param(n)) && !named.contains(&n) {
                n += 1;
            }
        }
        written.join(", ")
    }

    /// Writes a body `indent` levels in, under the variables it declares at its top.
    fn block(&mut self, body: &[tree::Stmt], indent: usize) {
        let placement = placement(body, self.old, self.sharp.then_some(self.head));
        self.sites = placement.sites;
        let mut hoisted: Vec<(String, Vec<String>)> = Vec::new();
        for var in &placement.hoisted {
            let (ty, name) = (self.declared_type(*var), self.var(*var));
            match hoisted.iter_mut().find(|(held, _)| *held == ty) {
                Some((_, names)) => names.push(name),
                None => hoisted.push((ty, vec![name])),
            }
        }
        for (ty, names) in &hoisted {
            // The form of old holds eight names a line; true C# fills each line to the width.
            let most = if self.sharp { names.len() } else { 8 };
            for few in names.chunks(most.max(1)) {
                self.line(indent, &format!("{ty} {};", few.join(", ")));
            }
        }
        self.body(body, indent);
    }

    /// Writes `stmts` and everything nested in them.
    // An explicit stack: a body nests as deep as its listing does, and a chain of `else if` is
    // as deep as it is long.
    fn body(&mut self, stmts: &[tree::Stmt], indent: usize) {
        let mut work = vec![Work::Run(stmts, indent)];
        while let Some(item) = work.pop() {
            match item {
                Work::Line(indent, text) => self.line(indent, text),
                Work::Text(indent, text) => self.line(indent, &text),
                Work::Simples(parts, indent) => self.simples(parts, indent),
                Work::Run(stmts, indent) => {
                    if let Some((first, rest)) = stmts.split_first() {
                        // ⚠ Only the form of old pairs the two itself: in true C# a pass says
                        // they are one statement, and without it they print as the two they are.
                        let pair = match self.plain || self.sharp {
                            true => None,
                            false => self.constructed(first, rest.first()),
                        };
                        match pair {
                            Some(text) => {
                                self.line(indent, &text);
                                work.push(Work::Run(&rest[1..], indent));
                            }
                            None => {
                                work.push(Work::Run(rest, indent));
                                self.stmt(first, indent, &mut work);
                            }
                        }
                    }
                }
                Work::Else(els, indent) => self.otherwise(els, indent, &mut work),
                Work::Cases(cases, indent, labels) => {
                    if let Some((case, rest)) = cases.split_first() {
                        work.push(Work::Cases(rest, indent, labels));
                        for label in &case.labels {
                            match label {
                                Some(value) => {
                                    let text = format!("case {}:", self.label(*value, labels));
                                    self.line(indent + 1, &text);
                                }
                                None => self.line(indent + 1, "default:"),
                            }
                        }
                        if let Some(closer) = self.closer(case, rest, labels) {
                            work.push(Work::Text(indent + 2, closer));
                        }
                        work.push(Work::Run(&case.body, indent + 2));
                    }
                }
                Work::Until(cond, indent) => {
                    let cond = self.cond(cond, 0);
                    self.line(indent, &format!("while ({cond});"));
                }
            }
        }
    }

    /// Opens a block under the line just written; `after` is what follows its closing brace.
    fn open<'b>(
        &mut self,
        block: &'b [tree::Stmt],
        indent: usize,
        after: Option<Work<'b>>,
        work: &mut Vec<Work<'b>>,
    ) {
        self.line(indent, "{");
        work.extend(after);
        work.push(Work::Line(indent, "}"));
        work.push(Work::Run(block, indent + 1));
    }

    fn otherwise<'b>(&mut self, els: &'b [tree::Stmt], indent: usize, work: &mut Vec<Work<'b>>) {
        match els {
            [] => {}
            // ⚠ The readable form only: the listing's own syntax has no `else if`.
            [tree::Stmt::If { cond, then, els }] if !self.plain => {
                self.branch("else if", cond, then, els, indent, work);
            }
            _ => {
                self.line(indent, "else");
                self.open(els, indent, None, work);
            }
        }
    }

    /// Whether true C# writes nothing for `arm`: it is empty, or holds only what the compiler added and the source level leaves out.
    fn unwritten(&self, arm: &[tree::Stmt]) -> bool {
        let hides = self.options.level == Level::Source;
        let hidden = |stmt: &tree::Stmt| match stmt {
            tree::Stmt::Sugar(sugar) => hides && matches!(**sugar, tree::Sugar::Hidden(_)),
            _ => false,
        };
        self.sharp && arm.iter().all(hidden)
    }

    /// Writes an `if` under `word` with its arms: an arm nothing is written for is no `else`, and where it is the first the other stands under the test turned round.
    // The test is made once either way, and the same arm runs.
    fn branch<'b>(
        &mut self,
        word: &str,
        cond: &'b Expr,
        then: &'b [tree::Stmt],
        els: &'b [tree::Stmt],
        indent: usize,
        work: &mut Vec<Work<'b>>,
    ) {
        match (self.unwritten(then), self.unwritten(els)) {
            (true, false) => {
                let (turned, _) = self.truth(cond, true, true);
                self.line(indent, &format!("{word} ({turned})"));
                self.open(els, indent, None, work);
            }
            (_, true) => {
                self.test(word, cond, indent);
                self.open(then, indent, None, work);
            }
            (false, false) => {
                self.test(word, cond, indent);
                self.open(then, indent, Some(Work::Else(els, indent)), work);
            }
        }
    }

    fn stmt<'b>(&mut self, stmt: &'b tree::Stmt, indent: usize, work: &mut Vec<Work<'b>>) {
        match stmt {
            tree::Stmt::Simple(s) => {
                let text = self.statement(s);
                self.line(indent, &format!("{text};"));
            }
            tree::Stmt::If { cond, then, els } => {
                self.branch("if", cond, then, els, indent, work);
            }
            tree::Stmt::While { cond, body } => {
                self.test("while", cond, indent);
                self.open(body, indent, None, work);
            }
            tree::Stmt::DoWhile { body, cond } => {
                self.line(indent, "do");
                self.open(body, indent, Some(Work::Until(cond, indent)), work);
            }
            tree::Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let init = init
                    .as_deref()
                    .map(|s| self.statement(s))
                    .unwrap_or_default();
                let cond = cond.as_ref().map(|e| self.cond(e, 0)).unwrap_or_default();
                let step = step.as_deref().map(|s| self.simple(s)).unwrap_or_default();
                self.line(indent, &format!("for ({init}; {cond}; {step})"));
                self.open(body, indent, None, work);
            }
            tree::Stmt::Switch { value, cases } => {
                let labels = self.labels(value, cases);
                let value = match labels {
                    Labels::Decimal if self.plain => self.value(value),
                    Labels::Decimal => self.switched(value),
                    Labels::Hex | Labels::Hashes => self.as_uint(bits(value)).0,
                };
                self.line(indent, &format!("switch ({value})"));
                self.line(indent, "{");
                work.push(Work::Line(indent, "}"));
                work.push(Work::Cases(cases, indent, labels));
            }
            tree::Stmt::Break => self.line(indent, "break;"),
            tree::Stmt::Continue => self.line(indent, "continue;"),
            tree::Stmt::Return(None) => self.line(indent, "return;"),
            // ⚠ An `async` method hands its task back wherever it leaves: with the last such
            // `return` out of sight, one that showed the task would read as the method's result.
            tree::Stmt::Return(Some(value)) if self.back.as_ref() == Some(value) => {
                self.line(indent, "return;");
            }
            tree::Stmt::Return(Some(value)) => {
                let narrow = match self.yields_narrow() {
                    true => self.narrow(value, true),
                    false => None,
                };
                let value = narrow.unwrap_or_else(|| self.value(value));
                self.line(indent, &format!("return {value};"));
            }
            tree::Stmt::Goto(label) => self.line(indent, &format!("goto {label};")),
            tree::Stmt::Label(label) => self.line(indent, &format!("{label}: ;")),
            tree::Stmt::Unreachable if self.plain => self.line(
                indent,
                "throw new global::System.InvalidOperationException(\"Unexpected bytecode fallthrough.\");",
            ),
            tree::Stmt::Unreachable if self.sharp => self.line(
                indent,
                "throw new System.Diagnostics.UnreachableException();",
            ),
            tree::Stmt::Unreachable => self.line(indent, "unreachable;"),
            tree::Stmt::Sugar(sugar) => self.sugar(sugar, indent, work),
        }
    }

    /// The value a switch goes by, under the `int` its labels are compared with it as unless C# reads it as one already.
    // ⚠ Bare, a whole cell would be compared whole: another test than the one the code makes.
    fn switched(&mut self, value: &Expr) -> String {
        let value = bits(value);
        let widens = matches!(
            self.kind_of(value),
            Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16
        );
        match widens {
            true => self.expr(value, 0),
            false => self.cast(Scalar::I32, value).0,
        }
    }

    /// How the labels of a switch are written: as the 32 bits they are where one of them is no small number.
    // ⚠ Only where the value switched on is 32 bits wide: a whole cell compared with a label
    // widened by its sign is another test than its low half compared with the label's.
    fn labels(&self, value: &Expr, cases: &[tree::Case]) -> Labels {
        let kind = self.kind_of(bits(value));
        let narrow = matches!(kind, Kind::I32 | Kind::U32 | Kind::Hash);
        let labels = || cases.iter().flat_map(|case| &case.labels).flatten();
        let large = labels().any(|label| label.unsigned_abs() > 0x1_0000);
        let fit = labels().all(|label| (-0x8000_0000..0x1_0000_0000i64).contains(label));
        let texts = kind == Kind::Hash && self.options.on(Pass::HashText);
        match (self.sharp && narrow && large && fit, texts) {
            (false, _) => Labels::Decimal,
            (true, false) => Labels::Hex,
            (true, true) => Labels::Hashes,
        }
    }

    /// One label of a switch.
    fn label(&self, value: i64, labels: Labels) -> String {
        let text = match labels {
            Labels::Hashes => self.names.text(value as u32),
            Labels::Decimal | Labels::Hex => None,
        };
        match (labels, text) {
            (Labels::Decimal, _) => value.to_string(),
            (_, Some(text)) => format!("Hash({})", spell::string(text)),
            (_, None) => format!("0x{:08X}", value as u32),
        }
    }

    /// The line that says where a case of true C# goes on to, which may not run into the next unsaid.
    // ⚠ A wrong `true` from `falls` costs a line nothing reaches; a wrong `false` would leave a
    // case C# refuses.
    fn closer(&self, case: &tree::Case, rest: &[tree::Case], labels: Labels) -> Option<String> {
        if !self.sharp || !falls(&case.body) {
            return None;
        }
        match rest.first() {
            None => Some("break;".to_owned()),
            // An empty body is the next case's: its labels stand above that one's.
            Some(_) if case.body.is_empty() => None,
            Some(next) => match next.labels.first()? {
                Some(value) => Some(format!("goto case {};", self.label(*value, labels))),
                None => Some("goto default;".to_owned()),
            },
        }
    }

    /// A source construct: its keyword and its blocks where its pass is on and its parts say what to write, else the statements it stands for.
    // The listing's own syntax has none, so the plain form always writes the statements.
    fn sugar<'b>(&mut self, sugar: &'b tree::Sugar, indent: usize, work: &mut Vec<Work<'b>>) {
        if self.plain {
            return self.lowered(sugar, indent);
        }
        if self.old {
            return self.old_sugar(sugar, indent, work);
        }
        let options = self.options;
        let on = |pass: Pass| options.on(pass);
        match sugar {
            // What the compiler added and no source wrote shows only below the source level.
            tree::Sugar::Hidden(_) if self.options.level == Level::Source => {}
            tree::Sugar::Hidden(stmts) => work.push(Work::Run(stmts, indent)),
            tree::Sugar::Group { kind, parts } => self.group(*kind, parts, indent),
            tree::Sugar::Await {
                before,
                ready,
                after,
                ..
            } if on(Pass::Await) => {
                // What is awaited is what the awaiter is taken from, else the awaiter the test asks.
                let awaiter = receiver(ready);
                let (shown, awaited) = match before.split_last() {
                    Some((taken, rest)) if takes(taken, awaiter) => {
                        (rest, assigned_receiver(taken))
                    }
                    _ => (before.as_slice(), awaiter),
                };
                self.simples(shown, indent);
                let awaited = self.value(awaited.unwrap_or(ready));
                match (after.as_slice(), awaiter) {
                    // The one statement that asks the awaiter for its result takes what the `await` yields.
                    ([Stmt::Assign { target, value }], Some(awaiter))
                        if asked_of(value, awaiter) =>
                    {
                        let text = format!("{} = await {awaited};", self.var(*target));
                        self.line(indent, &text);
                    }
                    _ => {
                        self.line(indent, &format!("await {awaited};"));
                        self.simples(after, indent);
                    }
                }
            }
            tree::Sugar::Yield { value, .. } if on(Pass::Iterators) => match value.as_slice() {
                // The one statement that keeps what is yielded says what that is.
                [Stmt::Assign { value: kept, .. } | Stmt::Store { value: kept, .. }] => {
                    let yielded = self.value(kept);
                    self.line(indent, &format!("yield return {yielded};"));
                }
                _ => {
                    self.simples(value, indent);
                    self.line(indent, "yield return;");
                }
            },
            tree::Sugar::Foreach { .. } => self.foreach(sugar, indent, work),
            tree::Sugar::Guarded {
                kind: tree::Guard::Using,
                ..
            } if on(Pass::Using) => self.blocked(sugar, indent, work),
            tree::Sugar::Guarded {
                kind: tree::Guard::Lock,
                ..
            } if on(Pass::Lock) => self.blocked(sugar, indent, work),
            tree::Sugar::Try { .. } | tree::Sugar::Throw(_) if on(Pass::Try) => {
                self.blocked(sugar, indent, work);
            }
            tree::Sugar::SwitchOn { arms, default } => {
                if !(on(Pass::SwitchOn) && self.switch_on(arms, default, indent, work)) {
                    self.lowered(sugar, indent);
                }
            }
            tree::Sugar::Event { seed, round, again } => {
                if !(on(Pass::Events) && self.event(seed, round, again, indent)) {
                    self.lowered(sugar, indent);
                }
            }
            tree::Sugar::Cached {
                test, made, kept, ..
            } => {
                if !(on(Pass::Lambdas) && self.cached(test, made, kept, indent)) {
                    self.lowered(sugar, indent);
                }
            }
            _ => self.lowered(sugar, indent),
        }
    }

    /// Writes a `using`, a `lock`, a `try` or a `throw` by its keyword; nothing for any other construct.
    fn blocked<'b>(&mut self, sugar: &'b tree::Sugar, indent: usize, work: &mut Vec<Work<'b>>) {
        match sugar {
            tree::Sugar::Guarded {
                kind, enter, body, ..
            } => {
                let (taken, shown) = match enter.split_last() {
                    Some((taken, rest)) => (Some(taken), rest),
                    None => (None, enter.as_slice()),
                };
                self.simples(shown, indent);
                let held = match (kind, taken) {
                    (tree::Guard::Lock, Some(Stmt::Eval(Expr::Call { args, .. })))
                        if !args.is_empty() =>
                    {
                        self.value(&args[0])
                    }
                    (_, Some(taken)) => self.simple(taken),
                    (_, None) => String::new(),
                };
                let word = match kind {
                    tree::Guard::Using => "using",
                    tree::Guard::Lock => "lock",
                };
                self.line(indent, &format!("{word} ({held})"));
                self.open(body, indent, None, work);
            }
            tree::Sugar::Try {
                body,
                catches,
                finally,
            } => {
                self.line(indent, "try");
                // Pushed last to first: the handlers follow the body and `finally` follows them.
                if !finally.is_empty() {
                    work.push(Work::Line(indent, "}"));
                    work.push(Work::Run(finally, indent + 1));
                    work.push(Work::Line(indent, "{"));
                    work.push(Work::Line(indent, "finally"));
                }
                for catch in catches.iter().rev() {
                    work.push(Work::Line(indent, "}"));
                    work.push(Work::Run(&catch.body, indent + 1));
                    work.push(Work::Line(indent, "{"));
                    work.push(Work::Text(indent, self.catch_head(catch)));
                }
                self.open(body, indent, None, work);
            }
            tree::Sugar::Throw(raise) => match &**raise {
                Stmt::Eval(Expr::Call { args, .. }) if args.len() == 1 => {
                    let thrown = self.value(&args[0]);
                    self.line(indent, &format!("throw {thrown};"));
                }
                other => {
                    let text = self.simple(other);
                    self.line(indent, &format!("{text};"));
                    self.line(indent, "throw;");
                }
            },
            _ => {}
        }
    }

    /// Writes the statements a source construct stands for.
    // ⚠ They are a copy, so no assignment among them is one `sites` holds: what a construct
    // written this way assigns must be declared at the top of the function, as `placement` has it.
    fn lowered(&mut self, sugar: &tree::Sugar, indent: usize) {
        let stmts = sugar.clone().desugar();
        self.body(&stmts, indent);
    }

    /// Writes simple statements a source construct holds, a line each.
    fn simples(&mut self, parts: &[Stmt], indent: usize) {
        for part in parts {
            let text = self.statement(part);
            self.line(indent, &format!("{text};"));
        }
    }

    fn catch_head(&self, catch: &tree::Catch) -> String {
        match (catch.ty, catch.var) {
            (Some(ty), Some(var)) => format!("catch ({} {})", self.key_name(ty), self.var(var)),
            (Some(ty), None) => format!("catch ({})", self.key_name(ty)),
            (None, _) => "catch".to_owned(),
        }
    }

    /// The name a type prints under by its key: the user's, else `type_` and the hash.
    fn key_name(&self, key: TypeKey) -> String {
        match self.names.type_name(key.0) {
            Some(name) => name.to_owned(),
            None => key.to_string(),
        }
    }

    /// `e` as the source construct `kind` reads it, where `e` has the shape that construct takes; else `e` as it is.
    fn sugared(&mut self, kind: ExprSugar, e: &Expr) -> (String, u8) {
        if self.old {
            return self.old_sugared(kind, e);
        }
        let pass = match kind {
            ExprSugar::Method => Some(Pass::Receivers),
            ExprSugar::Field { .. } => Some(Pass::Fields),
            ExprSugar::Property => Some(Pass::Properties),
            ExprSugar::Operator => Some(Pass::Operators),
            ExprSugar::Lambda => Some(Pass::Lambdas),
            ExprSugar::Coalesce | ExprSugar::NullConditional => Some(Pass::NullOps),
            ExprSugar::Concat => Some(Pass::Concat),
            ExprSugar::Out | ExprSugar::Ref => Some(Pass::RefOut),
            ExprSugar::Tuple => Some(Pass::Tuples),
            // No name switches these two: they are written wherever the tree holds one.
            ExprSugar::Convert | ExprSugar::TypeOf => None,
        };
        if pass.is_some_and(|pass| !self.options.on(pass)) {
            return self.spelled(e);
        }
        match (kind, e) {
            (ExprSugar::Method, Expr::Call { callee, args, .. }) => {
                if let Some((receiver, _)) = args.split_first() {
                    let receiver = self.expr(receiver, POSTFIX);
                    let passed = self.arguments(callee, args, 1);
                    let text = format!("{receiver}.{}({passed})", self.called(callee));
                    return (text, POSTFIX);
                }
            }
            (ExprSugar::Property, Expr::Call { callee, args, .. }) => {
                if let [receiver] = args.as_slice() {
                    let receiver = self.expr(receiver, POSTFIX);
                    return (format!("{receiver}.{}", self.called(callee)), POSTFIX);
                }
            }
            (ExprSugar::Field { owner }, Expr::Load { ty, addr, offset }) => {
                let base = self.expr(addr, POSTFIX);
                let place = format!("{base}.{}", self.field_name(Some(owner), *offset));
                // ⚠ The field's own type is said where it is declared: a reading of its bytes
                // as anything else is said here, as it is of any place.
                let declared = self.project.field(owner, *offset);
                let own = declared.is_some_and(|field| field.kind == ty.kind());
                let own = own || self.truth_field(owner, e);
                return match own {
                    true => (place, POSTFIX),
                    false => self.read_as(*ty, place, POSTFIX),
                };
            }
            (ExprSugar::Field { owner }, Expr::LoadBytes { addr, offset, .. }) => {
                let base = self.expr(addr, POSTFIX);
                let name = self.field_name(Some(owner), *offset);
                return (format!("{base}.{name}"), POSTFIX);
            }
            (
                ExprSugar::Operator,
                Expr::Call {
                    callee: Callee::Builtin { path, .. },
                    args,
                    ..
                },
            ) => {
                let op = path.rsplit('.').next().and_then(spell::operator);
                if let (Some(op), [l, r]) = (op, args.as_slice()) {
                    let p = op.precedence();
                    let text = match matches!(op, BinOp::Eq | BinOp::Ne) {
                        // Zero cells are as many as the other side holds.
                        true => format!("{} {} {}", self.side(l), op.symbol(), self.side(r)),
                        false => {
                            format!(
                                "{} {} {}",
                                self.expr(l, p),
                                op.symbol(),
                                self.expr(r, p + 1)
                            )
                        }
                    };
                    return (text, p);
                }
            }
            (ExprSugar::Lambda, Expr::FunctionRef { program, at }) => {
                return (self.function_ref(*program, *at), POSTFIX);
            }
            (ExprSugar::Coalesce, Expr::Select { then, els, .. }) => {
                let text = format!(
                    "{} ?? {}",
                    self.expr(bits(then), 2),
                    self.expr(bits(els), 2)
                );
                return (text, 1);
            }
            (ExprSugar::NullConditional, Expr::Select { then, .. }) => {
                let (mut field, mut owner) = (bits(then), None);
                if let Expr::Sugar {
                    kind: ExprSugar::Field { owner: of },
                    e: inner,
                } = field
                {
                    (field, owner) = (&**inner, Some(*of));
                }
                // ⚠ Only a field read whole: a narrower reading of nothing would not be null.
                if let Expr::Load {
                    ty: Scalar::U64,
                    addr,
                    offset,
                } = field
                {
                    let tested = self.expr(bits(addr), POSTFIX);
                    return (
                        format!("{tested}?.{}", self.field_name(owner, *offset)),
                        POSTFIX,
                    );
                }
            }
            (ExprSugar::Concat, Expr::Call { args, .. }) if !args.is_empty() => {
                let p = BinOp::Add.precedence();
                let parts: Vec<String> = args.iter().map(|arg| self.expr(arg, p + 1)).collect();
                return (parts.join(" + "), p);
            }
            (
                ExprSugar::Convert,
                Expr::Call {
                    callee: Callee::Intrinsic { name, ty, .. },
                    args,
                    ..
                },
            ) => {
                // The type converted to: the intrinsic's own, or the one its name holds in `<>`.
                let generic = name.split_once('<');
                let to = match ty {
                    Some(ty) => Some(ty.name.as_str()),
                    None => generic.and_then(|(_, rest)| rest.strip_suffix('>')),
                };
                if let (Some(to), [value]) = (to, args.as_slice()) {
                    let text = format!("({}){}", self.type_name(to), self.expr(value, UNARY));
                    return (text, UNARY);
                }
            }
            (ExprSugar::Out | ExprSugar::Ref, Expr::AddrOf { var, cell: 0 }) => {
                let word = match kind {
                    ExprSugar::Out => "out",
                    _ => "ref",
                };
                // An `out` whose variable the statement before only made zero declares it.
                let ty = match self.outs.contains(var) {
                    false => String::new(),
                    true => format!("{} ", self.declared_type(*var)),
                };
                return (format!("{word} {ty}{}", self.var(*var)), UNARY);
            }
            (ExprSugar::Tuple, Expr::Cells(items)) => {
                return (format!("({})", self.list(items)), POSTFIX);
            }
            _ => {}
        }
        self.spelled(e)
    }

    /// A method's own name: the last part of the name its call goes by.
    fn called(&self, callee: &Callee) -> String {
        let name = self.callee(callee);
        name.rsplit('.').next().unwrap_or(&name).to_owned()
    }

    /// The name a reference to the function at `at` of `program` is written under.
    fn function_ref(&self, program: u32, at: u32) -> String {
        let own = self.function_name(program, at, &format!("func_{at:06X}"));
        let unnamed = format!("program_{program:08X}.{own}");
        self.member(program, own, unnamed)
    }

    /// A place read as `ty`: bare for a whole cell, under a cast for an integer, and in true C# as its bits for a float, which a cast would convert.
    fn read_as(&self, ty: Scalar, place: String, binds: u8) -> (String, u8) {
        match ty {
            Scalar::U64 => (place, binds),
            Scalar::F32 | Scalar::F64 if self.sharp => {
                (format!("as_{}({place})", ty.name()), POSTFIX)
            }
            _ => (
                format!("({}){}", ty.keyword(), wrap(place, binds, UNARY)),
                UNARY,
            ),
        }
    }

    /// The two values `e` compares by their type's own `==` or `!=`, and whether it is `==`.
    // The form of old finds the call itself; in true C# the operators pass says which calls are one.
    fn equated<'e>(&self, e: &'e Expr) -> Option<(bool, &'e Expr, &'e Expr)> {
        match e {
            Expr::Call { callee, args, .. } if !self.sharp => self.equality(callee, args),
            Expr::Sugar {
                kind: ExprSugar::Operator,
                e: inner,
            } if !self.old && self.options.on(Pass::Operators) => {
                let Expr::Call {
                    callee: Callee::Builtin { path, .. },
                    args,
                    ..
                } = &**inner
                else {
                    return None;
                };
                match (path.rsplit('.').next(), args.as_slice()) {
                    (Some("op_Equality"), [l, r]) => Some((true, l, r)),
                    (Some("op_Inequality"), [l, r]) => Some((false, l, r)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A simple statement without its semicolon, declaring its target where this is the place to.
    fn statement(&mut self, s: &Stmt) -> String {
        let text = self.simple(s);
        let declared = match s {
            Stmt::Assign { target, .. } => Some(*target),
            Stmt::ZeroBytes { .. } => whole_clear(s, self.head),
            _ => None,
        };
        match declared {
            Some(var) if self.sites.contains(&std::ptr::from_ref(s)) => {
                format!("{} {text}", self.declared_type(var))
            }
            _ => text,
        }
    }

    /// Whether a variable is declared a 32-bit integer, which `++` and `--` step as the code does.
    fn is_int(&self, var: Var) -> bool {
        let kind = self.head.vars.get(&var).map(|info| &info.kind);
        matches!(kind, Some(Kind::I32 | Kind::U32))
    }

    /// `value` as a store of a whole cell takes it: widened where its kind is narrower, since a store reads as wide as its value.
    fn widened(&mut self, value: &Expr) -> String {
        let inner = bits(value);
        let kind = self.kind_of(inner);
        let narrow = kind_bytes(&kind).is_some_and(|bytes| bytes < 8);
        match (narrow, kind.is_float()) {
            (false, _) => self.expr(inner, 0),
            // ⚠ A cast would convert a float: the store writes its bits.
            (true, true) => format!("as_u64({})", self.expr(inner, 0)),
            (true, false) => format!("(long){}", self.expr(inner, UNARY)),
        }
    }

    /// An operand of an operator that works on whole cells, without the 64-bit reading that says nothing.
    // ⚠ A float keeps a reading of its bits, which C# would otherwise compare or add as a float;
    // `marked` puts `(long)` on a narrower number, which C# would otherwise work on at its width.
    fn whole_operand(&mut self, e: &Expr, marked: bool, min: u8) -> String {
        let inner = bits(e);
        if self.kind_of(inner).is_float() {
            let (text, binds) = self.cast(Scalar::I64, inner);
            return wrap(text, binds, min);
        }
        match marked && !matches!(inner, Expr::Const(_)) {
            true => wrap(format!("(long){}", self.expr(inner, UNARY)), UNARY, min),
            false => self.expr(inner, min),
        }
    }

    /// Whether C# takes `e` for a number narrower than a cell: a constant, or a value of such a kind.
    fn thin(&self, e: &Expr) -> bool {
        let narrow = kind_bytes(&self.kind_of(e)).is_some_and(|bytes| bytes < 8);
        narrow || matches!(e, Expr::Const(_))
    }

    /// An expression evaluated for its effect alone: as it stands where C# takes it for a statement, else given to a discard.
    fn evaluated(&mut self, e: &Expr) -> String {
        let (text, binds) = self.spelled(e);
        let mut inner = e;
        while let Expr::Sugar { e: under, .. } = inner {
            inner = &**under;
        }
        let call = matches!(inner, Expr::Call { .. }) && binds == POSTFIX && text.ends_with(')');
        // ⚠ `typeof(T)` ends like a call and is none: C# takes it for no statement.
        let call = call && !text.starts_with("typeof(");
        match call || matches!(inner, Expr::NewObject(_)) {
            true => text,
            false => format!("_ = {text}"),
        }
    }

    /// A simple statement without its semicolon.
    fn simple(&mut self, stmt: &Stmt) -> String {
        match stmt {
            Stmt::Assign { target, value } => match step_of(*target, value) {
                // ⚠ A step is of 32 bits: on a variable declared otherwise it stays the sum it is.
                Some(step) if !self.plain && self.is_int(*target) => {
                    format!("{}{step}", self.var(*target))
                }
                _ => format!("{} = {}", self.var(*target), self.taken(*target, value)),
            },
            Stmt::AssignMany { targets, values } => {
                let targets: Vec<String> = targets.iter().map(|v| self.var(*v)).collect();
                format!("({}) = ({})", targets.join(", "), self.list(values))
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } if self.plain => format!(
                "store_{}({}, 0x{offset:X}, {})",
                kind.name(),
                self.expr(addr, 0),
                self.expr(value, 0)
            ),
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                let narrow = match kind {
                    StoreKind::U8 => Some(Scalar::U8),
                    StoreKind::U16 => Some(Scalar::U16),
                    StoreKind::I32 => Some(Scalar::I32),
                    StoreKind::U64 | StoreKind::Ref => None,
                };
                let wide = Some(kind.bytes());
                let (place, _) = self.place_of(addr, *offset, narrow.is_some(), wide);
                let value = match (narrow, kind, bits(value)) {
                    (Some(to), ..) => self.stored(to, value),
                    // Eight bytes of a number: a bare one would read as the four an `int` takes.
                    (None, StoreKind::U64, Expr::Const(c)) if c.kind.is_integer() => {
                        let text = self.value(value);
                        // An `(int)` in front already says how the eight bytes come about.
                        match text.starts_with('(') {
                            true => text,
                            false => format!("{text}L"),
                        }
                    }
                    (None, ..) => self.widened(value),
                };
                format!("{place} = {value}")
            }
            // Whole cells of a value as wide as the store are the field assigned.
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } if !self.plain
                && bytes.is_multiple_of(8)
                && cells_of(value, self.head) == Some(bytes / 8) =>
            {
                let (place, _) = self.place_of(addr, *offset, true, Some(*bytes));
                format!("{place} = {}", self.value(value))
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } => format!(
                "store_bytes({}, 0x{offset:X}, {bytes}, {})",
                self.expr(addr, 0),
                self.expr(value, 0)
            ),
            Stmt::StoreF32Slots {
                addr,
                offset,
                value,
            } => format!(
                "store_f32_slots({}, 0x{offset:X}, {})",
                self.expr(addr, 0),
                self.expr(value, 0)
            ),
            // ⚠ In true C# only: the readable form is read back, where a clear is no assignment.
            Stmt::ZeroBytes { addr, bytes } => {
                match whole_clear(stmt, self.head).filter(|_| self.sharp) {
                    Some(var) => format!("{} = default", self.var(var)),
                    None => format!("zero_bytes({}, {bytes})", self.expr(addr, 0)),
                }
            }
            Stmt::StepI32 { addr, offset, down } if self.plain => {
                let helper = if *down {
                    "decrement_i32"
                } else {
                    "increment_i32"
                };
                format!("{helper}({}, 0x{offset:X})", self.expr(addr, 0))
            }
            Stmt::StepI32 { addr, offset, down } => {
                let step = if *down { "--" } else { "++" };
                let (place, binds) = self.place_of(addr, *offset, true, Some(4));
                format!("{}{step}", wrap(place, binds, POSTFIX))
            }
            Stmt::Eval(e) => {
                let assigned = match self.plain {
                    true => None,
                    false => self.assignment(e),
                };
                match assigned {
                    Some(text) => text,
                    None if self.sharp => self.evaluated(e),
                    None => self.expr(e, 0),
                }
            }
        }
    }

    /// `value` as `target` takes it: a load of as many cells as the variable holds needs no word on its width.
    fn taken(&mut self, target: Var, value: &Expr) -> String {
        match value {
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if !self.plain
                && bytes.is_multiple_of(8)
                && flat_cells(&Expr::Var(target), self.head) == Some(bytes / 8) =>
            {
                self.place_of(addr, *offset, false, Some(*bytes)).0
            }
            Expr::ZeroSlots(cells)
                if !self.plain && flat_cells(&Expr::Var(target), self.head) == Some(*cells) =>
            {
                "default".to_owned()
            }
            _ => {
                // A variable of 32 bits holds a constant's bits whatever sign widened them.
                let kind = self.head.vars.get(&target).map(|info| &info.kind);
                let narrow = match kind.is_some_and(spell::narrow_kind) {
                    true => self.narrow(value, true),
                    false => None,
                };
                narrow.unwrap_or_else(|| self.value(value))
            }
        }
    }

    /// For each argument of a call, whether what takes it takes 32 bits: `Some(true)` where the project's signature says so, `Some(false)` for a native, whose handler reads each cell as its own type.
    fn slots(&self, callee: &Callee, args: &[Expr]) -> Vec<Option<bool>> {
        let key = match callee {
            Callee::Native { .. } => return vec![Some(false); args.len()],
            Callee::Local { at, .. } => FunctionKey::new(self.head.program, *at),
            Callee::Import { program, at, .. } => FunctionKey::new(*program, *at),
            _ => return Vec::new(),
        };
        let Some(signature) = self.project.signatures.get(&key) else {
            return Vec::new();
        };
        // ⚠ An argument answers a parameter only while every one before it filled its own cells.
        let mut slots = Vec::with_capacity(args.len());
        for (arg, param) in args.iter().zip(&signature.params) {
            if cells_of(arg, self.head) != Some(param.cells) {
                break;
            }
            slots.push(spell::narrow_kind(&param.kind).then_some(true));
        }
        slots
    }

    /// The arguments of a call from the one at `from` on, each as what takes it is given it: a constant alone where 32 bits are taken stands without its cast.
    fn arguments(&mut self, callee: &Callee, args: &[Expr], from: usize) -> String {
        let passed = args.get(from..).unwrap_or_default();
        if !self.sharp {
            return self.list(passed);
        }
        let slots = self.slots(callee, args);
        let mut items = Vec::with_capacity(passed.len());
        for (n, arg) in args.iter().enumerate().skip(from) {
            let narrow = match slots.get(n) {
                Some(Some(exact)) => self.narrow(arg, *exact),
                _ => None,
            };
            items.push(narrow.unwrap_or_else(|| self.value(arg)));
        }
        items.join(", ")
    }

    /// Whether the function is proved to hand back one value of 32 bits.
    fn yields_narrow(&self) -> bool {
        let key = FunctionKey::new(self.head.program, self.head.at);
        let proved = self.project.signatures.get(&key);
        self.head.returns == Returns::Cell
            && proved.is_some_and(|sig| sig.returns == 1 && spell::narrow_kind(&sig.result))
    }

    /// One side of a type's own equality: zero cells are as many as the other side holds.
    fn side(&mut self, e: &Expr) -> String {
        match e {
            Expr::ZeroSlots(_) => "default".to_owned(),
            e => self.expr(e, 8),
        }
    }

    /// `e` where only its bits count: a value assigned, passed or returned.
    fn value(&mut self, e: &Expr) -> String {
        match self.plain {
            true => self.expr(e, 0),
            false => self.expr(bits(e), 0),
        }
    }

    fn list(&mut self, items: &[Expr]) -> String {
        let items: Vec<String> = items.iter().map(|e| self.value(e)).collect();
        items.join(", ")
    }

    fn var(&self, var: Var) -> String {
        if self.plain {
            return var.to_string();
        }
        // The two parameters a class gives a word of its own.
        if self.this == Some(var) {
            return self.bound.clone().unwrap_or_else(|| "this".to_owned());
        }
        if self.setter == Some(var) {
            return "value".to_owned();
        }
        if let Some(name) = self.head.vars.get(&var).and_then(|i| i.name.as_deref()) {
            return name.to_owned();
        }
        match var {
            Var::Global {
                program,
                offset,
                statics,
            } => {
                let space = if statics { "Static" } else { "Global" };
                self.member(
                    program,
                    format!("{space}_{offset:X}"),
                    format!("{space}_{program:08X}_{offset:X}"),
                )
            }
            other => other.to_string(),
        }
    }

    /// What belongs to `program`: `own` in that program itself, under the program's name where the user named it, else `unnamed`.
    fn member(&self, program: u32, own: String, unnamed: String) -> String {
        match self.names.program(u64::from(program)) {
            _ if program == self.head.program => own,
            Some(name) => format!("{name}.{own}"),
            None => unnamed,
        }
    }

    /// The name a type prints under: the user's for a `type_` hash they named, else the listing's.
    fn type_name(&self, listed: &str) -> String {
        type_name(listed, self.names, self.sharp)
    }

    /// The type a variable is declared with where a type must stand: its kind where one is known.
    // ⚠ `var[]` and `var` are no types of C#: true C# says `Cells` and `Cell`, which the header explains.
    fn declared_type(&self, var: Var) -> String {
        let (cell, cells) = match self.sharp {
            true => ("Cell", "Cells"),
            false => ("var", "var[]"),
        };
        match self.head.vars.get(&var) {
            Some(info) if info.kind == Kind::Unknown && info.width == Width::Cells => {
                cells.to_owned()
            }
            Some(info) => self.kind_text(&info.kind),
            None => cell.to_owned(),
        }
    }

    /// The C#-like name of a kind, for a declaration.
    fn kind_text(&self, kind: &Kind) -> String {
        kind_text(kind, self.names, self.sharp)
    }

    /// What `e` is, as far as its own shape and the kinds of its variables say.
    fn kind_of(&self, e: &Expr) -> Kind {
        match e {
            Expr::Const(c) => c.kind.clone(),
            // A type's own `==` yields a truth, whatever its call is listed to.
            Expr::Sugar { .. } if self.equated(e).is_some() => Kind::Bool,
            Expr::Sugar {
                kind: ExprSugar::Field { owner },
                e: inner,
            } if self.truth_field(*owner, inner) => Kind::Bool,
            Expr::Sugar { e: inner, .. } => self.kind_of(inner),
            // A global that the project states, or every access of which agrees, holds a reference.
            Expr::Var(Var::Global {
                program,
                offset,
                statics,
            }) if self.sharp => {
                let info = self.project.programs.get(program);
                let stated = info.and_then(|info| match statics {
                    true => info.statics.get(offset),
                    false => info.globals.get(offset),
                });
                let cell = self.project.cell(*program, *offset, *statics);
                let seen = cell.filter(|cell| cell.bytes == 8).map(|cell| &cell.kind);
                match stated.or(seen) {
                    Some(kind) if reference(kind) => kind.clone(),
                    _ => Kind::Unknown,
                }
            }
            Expr::Var(var) => (self.head.vars.get(var)).map_or(Kind::Unknown, |i| i.kind.clone()),
            Expr::Cast { to, .. } | Expr::Load { ty: to, .. } => to.kind(),
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => Kind::Bool,
            Expr::Binary { op, .. } if op.is_logical() => Kind::Bool,
            Expr::Binary { at, .. } => at.clone(),
            Expr::Vector3(_) => Kind::Vector,
            Expr::Call { callee, args, .. } if self.boxed_equality(callee, args).is_some() => {
                Kind::Bool
            }
            // A native every program only ever tests the result of for zero yields a truth to C#.
            Expr::Call { callee, .. } if self.tested_native(callee) => Kind::Bool,
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } if self.whole_equality(name, args).is_some() => Kind::Bool,
            Expr::Call {
                callee,
                args,
                returns,
            } => {
                let element = match callee {
                    Callee::Intrinsic { name, .. } if args.len() == 2 => {
                        (name.strip_prefix("array_get_")).and_then(Scalar::from_name)
                    }
                    _ => None,
                };
                match element {
                    Some(ty) => ty.kind(),
                    None => yielded_shape(self.names, callee, *returns),
                }
            }
            _ => Kind::Unknown,
        }
    }

    fn is_truth(&self, e: &Expr) -> bool {
        self.kind_of(e) == Kind::Bool
    }

    /// Whether `load` reads the one byte of a field of `owner` the project proves a `bool`, as the byte it is.
    fn truth_field(&self, owner: TypeKey, load: &Expr) -> bool {
        let Expr::Load {
            ty: Scalar::U8,
            offset,
            ..
        } = load
        else {
            return false;
        };
        let field = self.project.field(owner, *offset);
        field.is_some_and(|field| field.kind == Kind::Bool && field.bytes == 1)
    }

    /// `e` as the condition of an `if` or a loop, in parentheses where it binds less tightly than `min`.
    fn cond(&mut self, e: &Expr, min: u8) -> String {
        if self.plain {
            return self.expr(e, min);
        }
        let (text, binds) = self.truth(e, false, true);
        wrap(text, binds, min)
    }

    /// The text of `e`, in parentheses where it binds less tightly than `min`.
    fn expr(&mut self, e: &Expr, min: u8) -> String {
        if self.plain {
            return self.plain_expr(e, min);
        }
        let (text, binds) = self.spelled(e);
        wrap(text, binds, min)
    }

    /// The readable text of `e` and the strength it binds at.
    fn spelled(&mut self, e: &Expr) -> (String, u8) {
        let text = match e {
            Expr::Const(c) => return self.constant(c),
            Expr::Sugar { kind, e: inner } => return self.sugared(*kind, inner),
            Expr::Var(var) => self.var(*var),
            Expr::GuidEmpty => "Guid.Empty".to_owned(),
            Expr::Cell { base, index } => format!("{}[{index}]", self.expr(base, POSTFIX)),
            // ⚠ A group in braces is no expression of C#: true C# writes a tuple, and one
            // cell gathered alone is that cell.
            Expr::Cells(items) if self.sharp => match items.as_slice() {
                [one] => return self.spelled(bits(one)),
                _ => format!("({})", self.list(items)),
            },
            Expr::Cells(items) => format!("{{ {} }}", self.list(items)),
            Expr::Cast { to, e } => return self.cast(*to, e),
            Expr::Truth(inner) => return self.truth(inner, false, false),
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => return self.truth(inner, true, false),
            Expr::Unary { op, e } => {
                return (format!("{}{}", op.symbol(), self.expr(e, POSTFIX)), UNARY)
            }
            Expr::Binary { op, .. } if op.is_logical() => return self.truth(e, false, false),
            Expr::Binary { op, at, l, r } => return self.binary(*op, at, l, r),
            Expr::Select { cond, then, els } => {
                let text = format!(
                    "{} ? {} : {}",
                    self.cond(cond, 2),
                    self.expr(bits(then), 2),
                    self.expr(bits(els), 2)
                );
                return (text, 1);
            }
            Expr::Call { callee, args, .. } => return self.readable_call(callee, args),
            Expr::Load { ty, addr, offset } => {
                let (place, binds) = self.place_of(addr, *offset, false, Some(ty.bytes()));
                return self.read_as(*ty, place, binds);
            }
            // Whole cells are a field as wide as the cast says, as `(int)p->f_10` is four bytes.
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if bytes.is_multiple_of(8) => {
                let (place, binds) = self.place_of(addr, *offset, false, Some(*bytes));
                let cells = self.kind_text(&Kind::Aggregate(bytes / 8));
                let text = format!("({cells}){}", wrap(place, binds, UNARY));
                return (text, UNARY);
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => format!("load_bytes({}, 0x{offset:X}, {bytes})", self.expr(addr, 0)),
            // Bytes taken out of a variable are its field, read as the unsigned number they make
            // up: the cell they come to is zero above them.
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } if self.sharp
                && matches!(bits(value), Expr::Var(_))
                && matches!(bytes, 1 | 2 | 4 | 8) =>
            {
                let owner = match self.kind_of(bits(value)) {
                    Kind::Value { ty, .. } => TypeKey::of_name(&ty),
                    _ => None,
                };
                let place = format!(
                    "{}.{}",
                    self.expr(bits(value), POSTFIX),
                    self.field_name(owner, *offset)
                );
                let unsigned = match bytes {
                    1 => Some("byte"),
                    2 => Some("ushort"),
                    4 => Some("uint"),
                    _ => None,
                };
                return match unsigned {
                    Some(ty) => (format!("({ty}){place}"), UNARY),
                    None => (place, POSTFIX),
                };
            }
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } => format!("extract_bytes({}, {offset}, {bytes})", self.expr(value, 0)),
            Expr::ByteOffset { addr, offset } => match &**offset {
                Expr::Const(c) if c.bits == 0 => return self.spelled(addr),
                Expr::Const(c) if c.bits <= u64::from(u32::MAX) => {
                    let (place, _) = self.place(addr, c.bits as u32, false);
                    return (format!("&{place}"), UNARY);
                }
                _ => format!(
                    "byte_offset({}, {})",
                    self.expr(addr, 0),
                    self.expr(offset, 0)
                ),
            },
            Expr::AddrOf { var, cell: 0 } => return (format!("&{}", self.var(*var)), UNARY),
            Expr::AddrOf { var, cell } => {
                return (format!("&{}[{cell}]", self.var(*var)), UNARY);
            }
            // Zero cells, and how many: a bare `default` stands only where what takes it says.
            Expr::ZeroSlots(cells) => {
                format!("default({})", self.kind_text(&Kind::Aggregate(*cells)))
            }
            Expr::Vector3(parts) => format!("new Vector3({})", self.list(&parts[..])),
            Expr::NewObject(ty) => format!("new {}()", self.type_name(&ty.name)),
            Expr::NewArray { ty, len, .. } => {
                format!("new {}[{}]", self.type_name(&ty.name), self.value(len))
            }
            Expr::FunctionRef { program, at } => {
                return (format!("&{}", self.function_ref(*program, *at)), UNARY);
            }
            Expr::BuiltinFunctionRef { ty, method } => format!("BuiltinFunction({ty}, {method})"),
            Expr::VirtualFunctionRef { receiver, ty, slot } => format!(
                "VirtualFunction({}, {}, {slot})",
                self.value(receiver),
                self.type_name(&ty.name)
            ),
            Expr::StringConstant { program, offset } => {
                // The text itself where the project read the program's strings, in true C# alone.
                let read = self.project.programs.get(program).filter(|_| self.sharp);
                match read.and_then(|info| info.strings.get(offset)) {
                    Some(text) => spell::string(text),
                    None => self.member(
                        *program,
                        format!("String_{offset:X}"),
                        format!("String_{program:08X}_{offset:X}"),
                    ),
                }
            }
            Expr::BuiltinGlobal(n) => format!("BuiltinGlobal({n})"),
        };
        (text, POSTFIX)
    }

    /// `as_T(e)`: `e` itself where it already is a `T`, the constant it reads for a constant, else a C# cast.
    fn cast(&mut self, to: Scalar, e: &Expr) -> (String, u8) {
        // A reading of no fewer bits under this one changes nothing this one sees.
        let mut inner = e;
        while let Expr::Cast { to: under, e } = inner {
            if under.bytes() < to.bytes() {
                break;
            }
            inner = &**e;
        }
        if let Expr::Const(c) = inner {
            return self.constant(&reread(to, c));
        }
        let kind = self.kind_of(inner);
        // Zero or one is zero or one under any integer reading, and a hash is its 32 bits.
        let says_nothing = kind == to.kind()
            || (kind == Kind::Bool && !to.kind().is_float())
            || (kind == Kind::Hash && matches!(to, Scalar::I32 | Scalar::U32));
        // ⚠ Between a float and an integer a C# cast converts, and this reads the same bits: it
        // keeps the listing's own name. In true C# so does a float read of a cell of no known
        // kind, so that a cast to a float there is always a conversion.
        let whole = kind.is_integer() || matches!(kind, Kind::Bool | Kind::Hash);
        let reread = match self.sharp {
            true => to.kind().is_float() || kind.is_float(),
            false => (to.kind().is_float() && whole) || (!to.kind().is_float() && kind.is_float()),
        };
        match (says_nothing, reread) {
            (true, _) => self.spelled(inner),
            (false, true) => (
                format!("as_{}({})", to.name(), self.expr(inner, 0)),
                POSTFIX,
            ),
            (false, false) => (
                format!("({}){}", to.keyword(), self.expr(inner, UNARY)),
                UNARY,
            ),
        }
    }

    /// The truth of `e`, of its negation when `not`; `cond` says the text stands where only a truth can.
    // ⚠ Outside a condition a cell of no known kind is spelled `x != 0`: the value is the truth
    // of the cell and not the cell.
    fn truth(&mut self, e: &Expr, not: bool, cond: bool) -> (String, u8) {
        let (mut e, mut not) = (e, not);
        loop {
            match e {
                Expr::Truth(inner) => e = &**inner,
                Expr::Unary {
                    op: UnOp::Not,
                    e: inner,
                } => {
                    e = &**inner;
                    not = !not;
                }
                Expr::Cast { .. } if self.is_truth(unread(e)) => e = unread(e),
                Expr::Binary {
                    op: op @ (BinOp::Eq | BinOp::Ne),
                    l,
                    r,
                    ..
                } if is_zero(r) && self.is_truth(unread(l)) => {
                    not ^= *op == BinOp::Eq;
                    e = unread(l);
                }
                _ => break,
            }
        }
        match e {
            Expr::Const(c) => (((c.bits != 0) != not).to_string(), POSTFIX),
            Expr::Binary { op, at, l, r } if op.is_comparison() => {
                // ⚠ Not for floats: `!(a < b)` holds for a NaN and `a >= b` does not.
                let flips = matches!(op, BinOp::Eq | BinOp::Ne) || at.is_integer();
                match (not, flips) {
                    (false, _) => self.binary(*op, at, l, r),
                    (true, true) => self.binary(flipped(*op), at, l, r),
                    (true, false) => (format!("!({})", self.binary(*op, at, l, r).0), UNARY),
                }
            }
            Expr::Binary { op, at, l, r } if op.is_logical() => match not {
                false => self.binary(*op, at, l, r),
                true => (format!("!({})", self.binary(*op, at, l, r).0), UNARY),
            },
            // The negation of a type's own equality is its inequality.
            _ if not && self.equated(e).is_some() => match self.equated(e) {
                Some((equal, l, r)) => {
                    let op = if equal { "!=" } else { "==" };
                    (format!("{} {op} {}", self.side(l), self.side(r)), 7)
                }
                None => self.spelled(e),
            },
            // ⚠ A reference is no truth to C#: one that is set is one that is not `null`.
            _ if self.sharp && self.reference(bits(e)) => {
                let test = if not { "==" } else { "!=" };
                (format!("{} {test} null", self.expr(bits(e), 8)), 7)
            }
            _ => match (not, cond || self.is_truth(e)) {
                (false, true) => self.spelled(e),
                (true, true) => (format!("!{}", self.expr(e, POSTFIX)), UNARY),
                (false, false) => (format!("{} != 0", self.expr(e, 8)), 7),
                (true, false) => (format!("{} == 0", self.expr(e, 8)), 7),
            },
        }
    }

    fn binary(&mut self, op: BinOp, at: &Kind, l: &Expr, r: &Expr) -> (String, u8) {
        let typed = match self.sharp {
            true => self.typed(op, at, l, r),
            false => None,
        };
        if let Some(typed) = typed {
            return typed;
        }
        let p = op.precedence();
        let whole = matches!(at, Kind::I64 | Kind::U64 | Kind::Unknown);
        let (left, right) = match op {
            BinOp::LogicalAnd | BinOp::LogicalOr => {
                // `a || (b || c)` tests `a`, `b` and `c` in that order with or without the
                // parentheses, so a chain is written without them.
                let chained = matches!(r, Expr::Binary { op: under, .. } if *under == op);
                let right = if chained { p } else { p + 1 };
                (self.cond(l, p), self.cond(r, right))
            }
            // The sign of a cell does not count to these, so a 64-bit reading of an operand says nothing.
            BinOp::Eq
            | BinOp::Ne
            | BinOp::And
            | BinOp::Or
            | BinOp::Xor
            | BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
                if whole =>
            {
                // A sum, a difference or a product of two narrower numbers is one of 64 bits here.
                let sums = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul);
                let marked = sums && self.thin(bits(l)) && self.thin(bits(r));
                let left = self.whole_operand(l, marked, p);
                let unmarked = !left.starts_with("(long)");
                (left, self.whole_operand(r, marked && unmarked, p + 1))
            }
            _ => (self.expr(l, p), self.expr(r, p + 1)),
        };
        // A sign straight after `+` or `-` would run into it.
        let right = match matches!(op, BinOp::Add | BinOp::Sub) && right.starts_with('-') {
            true => format!("({right})"),
            false => right,
        };
        (format!("{left} {} {right}", op.symbol()), p)
    }

    /// The memory at byte `offset` from `addr`, as a field of what `addr` points at, and the strength the text binds at.
    // ⚠ `partial` is a write of fewer bytes than a cell. On a variable it is spelled `.f_0`,
    // because it leaves the cell's other bytes and an assignment would not.
    fn place(&mut self, addr: &Expr, offset: u32, partial: bool) -> (String, u8) {
        self.place_of(addr, offset, partial, None)
    }

    /// [`Writer::place`] for an access of `bytes` bytes, where the access states them.
    // ⚠ An element of an array is written `a[i]` only where the access is as wide as the array's
    // kind says an element is: fewer bytes are a part of the element, spelled `(&a[i])->f_0`,
    // as an assignment to `a[i]` would replace the whole of it.
    fn place_of(
        &mut self,
        addr: &Expr,
        offset: u32,
        partial: bool,
        bytes: Option<u32>,
    ) -> (String, u8) {
        // The access's own offset first, then those of the structs it is in, innermost first.
        let mut fields = vec![u64::from(offset)];
        let mut base = addr;
        while let Expr::ByteOffset { addr, offset } = base {
            match &**offset {
                Expr::Const(c) if c.bits <= u64::from(u32::MAX) => fields.push(c.bits),
                _ => break,
            }
            base = &**addr;
        }
        let embedded = fields.len() > 1;
        // The receiver is no pointer to C#: what it holds is written with a dot, the bytes at 0 too.
        let sharp = self.sharp && self.options.on(Pass::Fields);
        // ⚠ Not in a lambda's body written where it is bound: its object is a variable there.
        let dotted = sharp && self.bound.is_none();
        let own = dotted && matches!(bits(base), Expr::Var(var) if self.this == Some(*var));
        // ⚠ A byte of the receiver's own cell would then read as a member of the object: that
        // one is spelled through the cell's address.
        let mut boxed =
            dotted && matches!(base, Expr::AddrOf { var, cell: 0 } if self.this == Some(*var));
        let (mut text, variable) = match base {
            Expr::AddrOf { var, cell: 0 } => (self.var(*var), true),
            Expr::AddrOf { var, cell } => (format!("{}[{cell}]", self.var(*var)), true),
            // `*&a[i]` is `a[i]`: what stands at the address of an element is the element.
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } if sharp
                && !embedded
                && offset == 0
                && name == "array_element_address"
                && args.len() == 2 =>
            {
                let element = self.element(&args[0], &args[1]);
                let wide = match self.kind_of(&args[0]) {
                    Kind::Array(of) => element_bytes(&of),
                    _ => None,
                };
                return match bytes.is_some() && wide == bytes {
                    true => (element, POSTFIX),
                    false => (format!("(&{element})->f_0"), POSTFIX),
                };
            }
            pointer if own => (self.expr(bits(pointer), POSTFIX), false),
            // The first bytes of what a pointer points at are the thing itself.
            pointer if !embedded && offset == 0 => {
                return (format!("*{}", self.expr(pointer, UNARY)), UNARY);
            }
            pointer => (self.expr(pointer, POSTFIX), false),
        };
        // ⚠ `->` goes through a pointer and `.` stays in place: `local_3->f_8` reads what the
        // pointer in `local_3` points at, `local_3.f_8` the frame cell after `local_3`.
        let mut through = !variable;
        // ⚠ Fewer bytes than a variable of several cells holds are a part of it, as a narrow
        // store is of a cell: the variable's name alone is every cell of it.
        let part = self.sharp
            && match (base, bytes) {
                (Expr::AddrOf { var, cell: 0 }, Some(bytes)) => {
                    match flat_cells(&Expr::Var(*var), self.head) {
                        Some(cells) => cells >= 2 && u64::from(bytes) < u64::from(cells) * 8,
                        None => true,
                    }
                }
                _ => false,
            };
        for (n, field) in fields.iter().enumerate().rev() {
            let unsaid = *field == 0 && n == 0 && (embedded || (variable && !partial && !part));
            if !unsaid {
                if std::mem::take(&mut boxed) {
                    text = format!("(&{text})");
                    through = true;
                }
                let via = match (std::mem::take(&mut through), own) {
                    (true, false) => "->",
                    _ => ".",
                };
                let _ = write!(text, "{via}f_{field:X}");
            }
        }
        (text, POSTFIX)
    }

    /// `value` as the low `to.bytes()` bytes a narrow store writes: under a cast unless its kind has that width.
    fn stored(&mut self, to: Scalar, value: &Expr) -> String {
        // Four bytes are written whatever sign widened the constant they are of.
        if let Some(text) = self.narrow(value, true).filter(|_| to.bytes() == 4) {
            return text;
        }
        if kind_bytes(&self.kind_of(value)) == Some(to.bytes()) {
            return self.expr(value, 0);
        }
        let (text, binds) = self.cast(to, value);
        // A constant prints bare under a cast, and a bare number reads as an `int`: one a byte
        // or two wide says so.
        match (unread(value), to.bytes() < 4) {
            (Expr::Const(_), true) => format!("({}){}", to.keyword(), wrap(text, binds, UNARY)),
            _ => text,
        }
    }

    fn element(&mut self, array: &Expr, index: &Expr) -> String {
        format!("{}[{}]", self.expr(array, POSTFIX), self.value(index))
    }

    /// Whether `callee` is the function the project proves the constructor of `ty`.
    fn constructor_of(&self, callee: &Callee, ty: &VmType) -> bool {
        let key = match callee {
            Callee::Local { at, .. } => FunctionKey::new(self.head.program, *at),
            Callee::Import { program, at, .. } => FunctionKey::new(*program, *at),
            _ => return false,
        };
        let owner = TypeKey::of(ty);
        (self.project.members.get(&key))
            .is_some_and(|member| member.role == Role::Constructor && Some(member.owner) == owner)
    }

    fn readable_call(&mut self, callee: &Callee, args: &[Expr]) -> (String, u8) {
        // The token of a type the program names is `typeof` of that type.
        // ⚠ Where the call that makes a `Type` of a token is written `typeof`, as C# has it, the
        // token alone says it is one: the two never read alike.
        let typed = self.sharp && self.options.on(Pass::Operators);
        if let Some(ty) = type_token(callee, args) {
            let token = format!("typeof({})", self.type_name(&ty.name));
            return match typed {
                true => (format!("{token}.TypeHandle"), POSTFIX),
                false => (token, POSTFIX),
            };
        }
        if let (true, Callee::Builtin { path, .. }, [token]) = (typed, callee, args) {
            let mut token = bits(token);
            while let Expr::Sugar { e: inner, .. } = token {
                token = bits(inner);
            }
            let named = match token {
                Expr::Call { callee, args, .. } if path == TYPE_OF => type_token(callee, args),
                _ => None,
            };
            if let Some(ty) = named {
                return (format!("typeof({})", self.type_name(&ty.name)), POSTFIX);
            }
        }
        // A constructor run on an object made in the call itself is that object made with the
        // arguments: its type declares it as a constructor, under no name a call could use.
        if let (true, Some((Expr::NewObject(ty), _))) = (
            self.sharp && self.options.on(Pass::Constructors),
            args.split_first(),
        ) {
            if self.constructor_of(callee, ty) {
                let passed = self.arguments(callee, args, 1);
                let made = format!("new {}({passed})", self.type_name(&ty.name));
                return (made, POSTFIX);
            }
        }
        match callee {
            Callee::Intrinsic { name, .. } => {
                if let Some(spelled) = self.intrinsic(name, args) {
                    return spelled;
                }
            }
            // A receiver whose own type is the class says whose method it is.
            // ⚠ The form of old alone: in true C# the receivers pass says which calls are a
            // method's, and the operators pass which are a type's own `==`.
            Callee::Virtual { class, method, .. } if !self.sharp => match args.split_first() {
                Some((receiver, passed))
                    if self.kind_of(receiver) == Kind::Object(class.clone()) =>
                {
                    let receiver = self.expr(receiver, POSTFIX);
                    return (
                        format!("{receiver}.{method}({})", self.list(passed)),
                        POSTFIX,
                    );
                }
                _ => {}
            },
            _ => {}
        }
        if let Some((l, r)) = self.boxed_equality(callee, args) {
            let text = format!("{} == {}", self.expr(l, 8), self.expr(r, 8));
            return (text, 7);
        }
        if let Some((to, value)) = self.boxed_conversion(callee, args) {
            return (format!("({to}){}", self.expr(value, UNARY)), UNARY);
        }
        let equality = match self.sharp {
            true => None,
            false => self.equality(callee, args),
        };
        if let Some((equal, l, r)) = equality {
            let op = if equal { "==" } else { "!=" };
            let text = format!("{} {op} {}", self.side(l), self.side(r));
            return (text, 7);
        }
        let name = self.callee(callee);
        (
            format!("{name}({})", self.arguments(callee, args, 0)),
            POSTFIX,
        )
    }

    /// Whether `callee` is a native whose result every program only ever tests for zero.
    fn tested_native(&self, callee: &Callee) -> bool {
        let tested =
            || native_key(callee).is_some_and(|key| self.project.native_truths.contains(&key));
        self.sharp && self.options.on(Pass::Interproc) && tested()
    }

    /// The two variables a comparison of bytes compares whole: `bytes_equal_16(&a, &b)` of two variables of 16 bytes, which C# compares with `==`.
    fn whole_equality(&self, name: &str, args: &[Expr]) -> Option<(Var, Var)> {
        let bytes: u32 = name.strip_prefix("bytes_equal_")?.parse().ok()?;
        let [Expr::AddrOf { var: l, cell: 0 }, Expr::AddrOf { var: r, cell: 0 }] = args else {
            return None;
        };
        // Three floats fill twelve bytes of their two cells.
        let whole = |var: &Var| {
            let vector = (self.head.vars.get(var)).is_some_and(|info| info.kind == Kind::Vector);
            let cells = flat_cells(&Expr::Var(*var), self.head).filter(|cells| *cells >= 2);
            let held = cells.map(|cells| if vector { 12 } else { cells * 8 });
            held == Some(bytes)
        };
        let on = self.sharp && self.options.on(Pass::Operators);
        (on && whole(l) && whole(r)).then_some((*l, *r))
    }

    /// The value `e` boxes and the type it boxes it as, where `e` is a `box<T>` of one value.
    fn boxed<'e>(&self, e: &'e Expr) -> Option<(&'e str, &'e Expr)> {
        let Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        } = bits(e)
        else {
            return None;
        };
        let ty = name.strip_prefix("box<")?.strip_suffix('>')?;
        match args.as_slice() {
            [value] => Some((ty, value)),
            _ => None,
        }
    }

    /// Whether a value of the type named `ty` is equal to another exactly where its bits are: a number of no fraction, a `bool`, a `char`, or an enum of a script.
    // ⚠ Never a float: `Equals` holds of two NaNs, which `==` does not.
    fn equal_by_bits(&self, ty: &str) -> bool {
        let keyword = self.type_name(ty);
        let plain = [
            "bool", "byte", "sbyte", "short", "ushort", "int", "uint", "long", "ulong", "char",
        ];
        plain.contains(&keyword.as_str()) || self.enumeration(ty).is_some()
    }

    /// The facts of the enum of a script the type named `ty` is.
    fn enumeration(&self, ty: &str) -> Option<&TypeFacts> {
        let facts = self.project.types.get(&TypeKey::of_name(ty)?)?;
        (facts.is_value() && facts.flags & (STRUCT | GENERATED) == 0).then_some(facts)
    }

    /// The two values `object.Equals(box<T>(a), box<T>(b))` holds equal: one type's two values, which C# compares with `==`.
    fn boxed_equality<'e>(
        &self,
        callee: &Callee,
        args: &'e [Expr],
    ) -> Option<(&'e Expr, &'e Expr)> {
        if !(self.sharp && self.options.on(Pass::Boxing)) {
            return None;
        }
        let (Callee::Builtin { path, .. }, [l, r]) = (callee, args) else {
            return None;
        };
        let ((a, l), (b, r)) = (self.boxed(l)?, self.boxed(r)?);
        let same = path == OBJECT_EQUALS && a == b && self.equal_by_bits(a);
        same.then_some((l, r))
    }

    /// `Convert.ToInt32(box<T>(x))` as the cast it is: the keyword of the type and the value, where no value of `T` is past an `int`'s range.
    // ⚠ A value past it throws where a cast would not: only a type of fewer than four bytes, or
    // an enum of four whose constants hold one below zero, which only a signed one can.
    fn boxed_conversion<'e>(
        &self,
        callee: &Callee,
        args: &'e [Expr],
    ) -> Option<(&'static str, &'e Expr)> {
        if !(self.sharp && self.options.on(Pass::Boxing)) {
            return None;
        }
        let (Callee::Builtin { path, .. }, [boxed]) = (callee, args) else {
            return None;
        };
        if path != CONVERT_TO_INT32 {
            return None;
        }
        let (ty, value) = self.boxed(boxed)?;
        let keyword = self.type_name(ty);
        let small = ["byte", "sbyte", "short", "ushort", "int", "char"];
        let fits = small.contains(&keyword.as_str())
            || self.enumeration(ty).is_some_and(|facts| {
                facts.byte_size <= 2
                    || (facts.byte_size == 4 && facts.constants.iter().any(|c| *c < 0))
            });
        fits.then_some(("int", value))
    }

    /// The two values a type's own `==` or `!=` compares, and whether it is `==`.
    // Only for values of several cells: two cells compared whole are written with the same
    // signs, and the operator of a type must not read as that.
    fn equality<'e>(
        &self,
        callee: &Callee,
        args: &'e [Expr],
    ) -> Option<(bool, &'e Expr, &'e Expr)> {
        let (Callee::Builtin { path, .. }, [l, r]) = (callee, args) else {
            return None;
        };
        let equal = match path.rsplit('.').next() {
            Some("op_Equality") => true,
            Some("op_Inequality") => false,
            _ => return None,
        };
        let wide = |e: &Expr| cells_of(e, self.head).is_some_and(|cells| cells >= 2);
        (wide(l) && wide(r)).then_some((equal, l, r))
    }

    /// `x = new T(a, b)` for an object made by one statement and constructed by the next.
    // The two stand side by side, so nothing runs between the making and the constructing.
    fn constructed(&mut self, made: &tree::Stmt, next: Option<&tree::Stmt>) -> Option<String> {
        let tree::Stmt::Simple(
            made @ Stmt::Assign {
                target,
                value: Expr::NewObject(ty),
            },
        ) = made
        else {
            return None;
        };
        let Some(tree::Stmt::Simple(Stmt::Eval(Expr::Call {
            callee: Callee::Builtin { path, .. },
            args,
            ..
        }))) = next
        else {
            return None;
        };
        let (receiver, passed) = args.split_first()?;
        let own = path.strip_suffix(".Ctor") == Some(ty.name.as_str());
        let mut again = false;
        for arg in passed {
            arg.walk(&mut |e| {
                again |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if v == target);
            });
        }
        if !own || *receiver != Expr::Var(*target) || again {
            return None;
        }
        let name = match self.sites.contains(&std::ptr::from_ref(made)) {
            true => format!("{} {}", self.declared_type(*target), self.var(*target)),
            false => self.var(*target),
        };
        let ty = self.type_name(&ty.name);
        Some(format!("{name} = new {ty}({});", self.list(passed)))
    }

    /// A condition as the lines of an `if` or a loop: one line, or one operand a line where one line would be long.
    // True C# leaves the breaking to `line`, which breaks every statement by one rule.
    fn test(&mut self, word: &str, cond: &Expr, indent: usize) {
        let one = self.cond(cond, 0);
        let long = 4 * (self.depth + indent) + word.len() + one.len() > 118;
        let op = match cond {
            Expr::Binary {
                op: op @ (BinOp::LogicalAnd | BinOp::LogicalOr),
                ..
            } if !self.plain && !self.sharp && long => *op,
            _ => {
                self.line(indent, &format!("{word} ({one})"));
                return;
            }
        };
        // The operands of the chain, in the order they are tested, however the chain nests.
        let mut operands = Vec::new();
        let mut open = vec![cond];
        while let Some(e) = open.pop() {
            match e {
                Expr::Binary {
                    op: under, l, r, ..
                } if *under == op => {
                    open.push(r);
                    open.push(l);
                }
                operand => operands.push(operand),
            }
        }
        let p = op.precedence();
        let mut text = String::new();
        for (n, operand) in operands.into_iter().enumerate() {
            if n > 0 {
                text.push('\n');
                push_indent(&mut text, self.depth + indent + 1);
                text.push_str(op.symbol());
                text.push(' ');
            }
            text.push_str(&self.cond(operand, p + 1));
        }
        self.line(indent, &format!("{word} ({text})"));
    }

    /// An intrinsic C# has an operator for, as that operator; `None` for one that stays a call.
    fn intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<(String, u8)> {
        if let Some((l, r)) = self.whole_equality(name, args) {
            let text = format!("{} == {}", self.var(l), self.var(r));
            return Some((text, 7));
        }
        match args {
            [target, passed @ ..] if name == "invoke_delegate" => {
                let target = self.expr(target, POSTFIX);
                Some((format!("{target}({})", self.list(passed)), POSTFIX))
            }
            [array] if name == "array_length" => {
                Some((format!("{}.Length", self.expr(array, POSTFIX)), POSTFIX))
            }
            // A cast that must hold; ⚠ never the `as` below, which yields null where this faults.
            [value] if name.starts_with("cast<") => {
                let ty = name.strip_prefix("cast<")?.strip_suffix('>')?;
                let text = format!("({}){}", self.type_name(ty), self.expr(value, UNARY));
                Some((text, UNARY))
            }
            [value] => {
                let ty = name.strip_prefix("try_cast<")?.strip_suffix('>')?;
                let text = format!("{} as {}", self.expr(value, 9), self.type_name(ty));
                // Weaker than C# has it, so that a comparison puts it in parentheses.
                Some((text, 6))
            }
            [array, index] if name == "array_element_address" => {
                Some((format!("&{}", self.element(array, index)), UNARY))
            }
            [array, index] if name == "array_get_value" => {
                Some((self.element(array, index), POSTFIX))
            }
            [array, index] => {
                let ty = Scalar::from_name(name.strip_prefix("array_get_")?)?;
                let element = self.element(array, index);
                Some(self.read_as(ty, element, POSTFIX))
            }
            _ => None,
        }
    }

    /// An intrinsic call that is an assignment in C#, as that assignment.
    fn assignment(&mut self, e: &Expr) -> Option<String> {
        let Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        } = e
        else {
            return None;
        };
        let [array, index, cells @ ..] = args.as_slice() else {
            return None;
        };
        let value = match (name.strip_prefix("array_set_")?, cells) {
            ("ref" | "u64" | "slots", [value]) => self.value(value),
            ("u32", [value]) => self.stored(Scalar::U32, value),
            ("u16", [value]) => self.stored(Scalar::U16, value),
            ("u8", [value]) => self.stored(Scalar::U8, value),
            ("slots", [_, _, ..]) if self.sharp => format!("({})", self.list(cells)),
            ("slots", [_, _, ..]) => format!("{{ {} }}", self.list(cells)),
            _ => return None,
        };
        Some(format!("{} = {value}", self.element(array, index)))
    }

    /// The text of `e` in the listing's own syntax, in parentheses where it binds less tightly than `min`.
    fn plain_expr(&mut self, e: &Expr, min: u8) -> String {
        let text = match e {
            Expr::Const(c) => {
                let text = plain_const(c);
                // A sign in front of an operand would run into the operator before it.
                return match min > 0 && text.starts_with('-') {
                    true => format!("({text})"),
                    false => text,
                };
            }
            // The listing's own syntax has no source construct: the expression is written as it is.
            Expr::Sugar { e: inner, .. } => return self.plain_expr(inner, min),
            Expr::Var(var) => self.var(*var),
            Expr::GuidEmpty => "GuidEmpty".to_owned(),
            Expr::Cell { base, index } => format!("{}[{index}]", self.expr(base, POSTFIX)),
            Expr::Cells(items) => format!("new VmValues({})", self.list(items)),
            Expr::Cast { to, e } => format!("as_{}({})", to.name(), self.expr(e, 0)),
            Expr::Truth(e) => format!("Truth({})", self.expr(e, 0)),
            Expr::Unary { op, e } => format!("{}{}", op.symbol(), self.expr(e, POSTFIX)),
            Expr::Binary { op, l, r, .. } => {
                let p = op.precedence();
                format!(
                    "{} {} {}",
                    self.expr(l, p),
                    op.symbol(),
                    self.expr(r, p + 1)
                )
            }
            Expr::Select { cond, then, els } => format!(
                "{} ? (VmValue)({}) : (VmValue)({})",
                self.expr(cond, 2),
                self.expr(then, 0),
                self.expr(els, 0)
            ),
            Expr::Call {
                callee,
                args,
                returns,
            } => self.call(callee, args, *returns),
            Expr::Load { ty, addr, offset } => {
                format!("load_{}({}, 0x{offset:X})", ty.name(), self.expr(addr, 0))
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => format!("load_bytes({}, 0x{offset:X}, {bytes})", self.expr(addr, 0)),
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } => format!("extract_bytes({}, {offset}, {bytes})", self.expr(value, 0)),
            Expr::ByteOffset { addr, offset } => format!(
                "byte_offset({}, {})",
                self.expr(addr, 0),
                self.expr(offset, 0)
            ),
            Expr::AddrOf { var, cell } => match *var {
                Var::Local(n) => format!("Vm.Address(locals, {})", n + cell),
                Var::Param(n) => format!("Vm.Address(parameters, {})", n + cell),
                Var::Global {
                    program,
                    offset,
                    statics,
                } => {
                    let flag = if statics { ", true" } else { "" };
                    format!("Vm.GlobalAddress(0x{program:08X}U, 0x{offset:X}{flag})")
                }
                temporary => format!("Vm.Address({temporary}, {cell})"),
            },
            Expr::ZeroSlots(cells) => format!("zero_slots({cells})"),
            Expr::Vector3(parts) => format!("vector3({})", self.list(&parts[..])),
            Expr::NewObject(ty) => format!("Vm.NewObject({})", vm_type(ty)),
            Expr::NewArray { ty, len, wrapper } => {
                let path = format!("Operations.{wrapper}");
                if let Some(declared) = self.declared.as_deref_mut() {
                    declared.entry(path.clone()).or_insert_with(|| Wrapper {
                        target: Target::NewArray {
                            ty: ty.clone(),
                            wrapper: wrapper.clone(),
                        },
                        returns: 1,
                        params: 1,
                    });
                }
                format!("{path}({})", self.expr(len, 0))
            }
            Expr::FunctionRef { program, at } => {
                format!("Vm.Function(0x{program:08X}U, 0x{at:06X})")
            }
            Expr::BuiltinFunctionRef { ty, method } => {
                format!("Vm.BuiltinFunction({ty}, {method})")
            }
            Expr::VirtualFunctionRef { receiver, ty, slot } => format!(
                "Vm.VirtualFunction({}, {}, {slot})",
                self.expr(receiver, 0),
                vm_type(ty)
            ),
            Expr::StringConstant { program, offset } => {
                format!("Vm.StringConstant(0x{program:08X}U, 0x{offset:X}U)")
            }
            Expr::BuiltinGlobal(n) => format!("Vm.BuiltinGlobal({n})"),
        };
        let binds = match e {
            Expr::Select { .. } => 1,
            Expr::Binary { op, .. } => op.precedence(),
            Expr::Unary { .. } => UNARY,
            _ => POSTFIX,
        };
        wrap(text, binds, min)
    }

    /// A call in the listing's own syntax, through the declaration it names.
    fn call(&mut self, callee: &Callee, args: &[Expr], returns: Option<u32>) -> String {
        let passed = self.list(args);
        // An intrinsic no declaration wraps is written as the runtime's own call, as a listing has it.
        if let Callee::Intrinsic {
            name,
            ty,
            wrapper: None,
        } = callee
        {
            let mut text = match ty {
                Some(ty) => format!("Vm.TypedIntrinsic({}, {}", quoted(name), vm_type(ty)),
                None => format!("Vm.Intrinsic({}, {}", quoted(name), returns.unwrap_or(0)),
            };
            if !args.is_empty() {
                text.push_str(", ");
                text.push_str(&passed);
            }
            text.push(')');
            if ty.is_none() && returns == Some(1) {
                text.push_str("[0]");
            }
            return text;
        }
        let path = match callee.path() {
            Some(path) => path,
            None => return format!("{}({passed})", callee_name(callee, self.names)),
        };
        if let Some(declared) = self.declared.as_deref_mut() {
            declared.entry(path.clone()).or_insert_with(|| Wrapper {
                target: Target::Call(callee.clone()),
                returns: returns.unwrap_or(0),
                params: args.len() as u32,
            });
        }
        format!("{path}({passed})")
    }
}

/// The type a call is the token of: the intrinsic that pushes one, of a type the program names.
fn type_token<'e>(callee: &'e Callee, args: &[Expr]) -> Option<&'e VmType> {
    match (callee, args) {
        (Callee::Intrinsic { name, ty, .. }, []) if name == TYPE_TOKEN => ty.as_ref(),
        _ => None,
    }
}

/// The first argument of the call `e` is under any reading or test of it: the receiver of a method's call.
fn receiver(e: &Expr) -> Option<&Expr> {
    let mut e = e;
    loop {
        match e {
            Expr::Cast { e: inner, .. }
            | Expr::Sugar { e: inner, .. }
            | Expr::Truth(inner)
            | Expr::Unary { e: inner, .. } => e = &**inner,
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } if is_zero(r) => e = &**l,
            Expr::Call { args, .. } => return args.first(),
            _ => return None,
        }
    }
}

/// The receiver of the call a statement assigns the result of.
fn assigned_receiver(stmt: &Stmt) -> Option<&Expr> {
    match stmt {
        Stmt::Assign { value, .. } => receiver(value),
        _ => None,
    }
}

/// Whether `taken` gives the variable `asked` is, and no other, what a call on a receiver yields: an enumerator or an awaiter taken from what is gone through.
fn takes(taken: &Stmt, asked: Option<&Expr>) -> bool {
    let own = matches!(
        (taken, asked),
        (Stmt::Assign { target, .. }, Some(Expr::Var(var))) if target == var
    );
    own && assigned_receiver(taken).is_some()
}

/// Whether `e` is a call of one argument, and that argument `of`: a method that takes nothing, asked of it.
fn asked_of(e: &Expr, of: &Expr) -> bool {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. }
    | Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    matches!(e, Expr::Call { args, .. } if args.len() == 1 && args[0] == *of)
}

/// Whether control may run off the end of `body`; where nothing says, it may.
// An explicit stack: a chain of `else if` is as deep as it is long.
fn falls(body: &[tree::Stmt]) -> bool {
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        match list.last() {
            Some(
                tree::Stmt::Return(_)
                | tree::Stmt::Break
                | tree::Stmt::Continue
                | tree::Stmt::Goto(_)
                | tree::Stmt::Unreachable,
            ) => {}
            Some(tree::Stmt::Sugar(sugar)) if matches!(**sugar, tree::Sugar::Throw(_)) => {}
            // Both arms leave, so nothing comes out of the `if`.
            Some(tree::Stmt::If { then, els, .. }) if !then.is_empty() && !els.is_empty() => {
                open.push(then);
                open.push(els);
            }
            _ => return true,
        }
    }
    false
}

/// The parameters a body reads, writes or takes the address of.
fn named_params(body: &[tree::Stmt]) -> BTreeSet<u32> {
    let mut named = BTreeSet::new();
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            if let Var::Param(n) = target {
                named.insert(*n);
            }
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(Var::Param(n))
            | Expr::AddrOf {
                var: Var::Param(n), ..
            } = e
            {
                named.insert(*n);
            }
        });
    });
    named
}

/// The name a type prints under: the user's for a `type_` hash they named, else the listing's; in true C# a .NET primitive by its keyword.
fn type_name(listed: &str, names: &Names, sharp: bool) -> String {
    let name = match names.listed_type(listed) {
        Some(name) => name.to_owned(),
        None => spelled_name(listed),
    };
    match sharp.then(|| spell::keyword(&name)).flatten() {
        Some(keyword) => keyword.to_owned(),
        None => name,
    }
}

/// The C#-like name of a kind where a type must stand; `sharp` asks for the words of true C#.
pub(super) fn kind_text(kind: &Kind, names: &Names, sharp: bool) -> String {
    let name = match kind {
        Kind::Unknown if sharp => "Cell",
        Kind::Unknown => "var",
        Kind::Bool => "bool",
        Kind::I8 => "sbyte",
        Kind::U8 => "byte",
        Kind::I16 => "short",
        Kind::U16 => "ushort",
        Kind::I32 => "int",
        Kind::U32 => "uint",
        Kind::I64 => "long",
        Kind::U64 => "ulong",
        Kind::F32 => "float",
        Kind::F64 => "double",
        Kind::Hash => "Hash",
        Kind::Pointer => "void*",
        Kind::Object(name) => return type_name(name, names, sharp),
        Kind::String => "string",
        Kind::Vector => "Vector3",
        Kind::Handle => "Handle",
        Kind::Aggregate(cells) if sharp => return format!("Cells{cells}"),
        Kind::Aggregate(cells) => return format!("var[{cells}]"),
        Kind::Value { ty, .. } => return type_name(ty, names, sharp),
        Kind::Array(of) => return format!("{}[]", kind_text(of, names, sharp)),
    };
    name.to_owned()
}

/// The name a call prints under in the readable form.
pub(super) fn callee_name(callee: &Callee, names: &Names) -> String {
    match callee {
        Callee::Local { name, .. } => name.clone(),
        Callee::Intrinsic { name, ty, .. } => {
            // `box<type_..>`: the type inside takes the name the user gave it.
            let generic = name
                .split_once('<')
                .and_then(|(head, rest)| Some((head, rest.strip_suffix('>')?)));
            // A typed intrinsic works on values of its type, which its name alone does not say.
            let typed = ty.as_ref().map(|ty| (name.as_str(), ty.name.as_str()));
            match generic.or(typed) {
                Some((head, ty)) => {
                    let ty = names.listed_type(ty).unwrap_or(ty);
                    spelled_name(&format!("{head}<{ty}>"))
                }
                None => spelled_name(name),
            }
        }
        Callee::Import {
            module,
            name,
            program,
            ..
        } => {
            let module = names
                .listed_module(module)
                .or_else(|| names.program(u64::from(*program)))
                .unwrap_or(module);
            format!("{module}.{name}")
        }
        Callee::Native {
            namespace,
            name,
            hash,
            ..
        } => {
            let (namespace, name) = match names.native(*hash) {
                Some(listed) => (&listed.namespace, &listed.name),
                None => (namespace, name),
            };
            match namespace {
                Some(namespace) => format!("{namespace}.{name}"),
                None => name.clone(),
            }
        }
        Callee::Builtin { path, .. } => path.clone(),
        Callee::Virtual { class, method, .. } => {
            format!("{}.{method}", names.listed_type(class).unwrap_or(class))
        }
    }
}

/// The cells `e` holds, `None` where nothing says.
pub(super) fn cells_of(e: &Expr, head: &Head) -> Option<u32> {
    match e {
        Expr::Cells(items) => items.iter().map(|item| flat_cells(item, head)).sum(),
        other => flat_cells(other, head),
    }
}

fn flat_cells(e: &Expr, head: &Head) -> Option<u32> {
    match e {
        Expr::Var(var) => match head.vars.get(var) {
            Some(info) if info.width == Width::Cells => match info.kind {
                Kind::Vector | Kind::Handle => Some(2),
                Kind::Aggregate(cells) | Kind::Value { cells, .. } => Some(cells),
                _ => None,
            },
            _ => Some(1),
        },
        Expr::Sugar { e: inner, .. } => flat_cells(inner, head),
        Expr::Cells(_) | Expr::BuiltinGlobal(_) => None,
        Expr::GuidEmpty | Expr::Vector3(_) => Some(2),
        Expr::ZeroSlots(cells) => Some(*cells),
        Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => Some(bytes.div_ceil(8)),
        Expr::Call { returns, .. } => returns.filter(|cells| *cells > 0),
        _ => Some(1),
    }
}

/// Whether a value of `kind` is a reference: to an object, a string or an array.
fn reference(kind: &Kind) -> bool {
    matches!(kind, Kind::Object(_) | Kind::String | Kind::Array(_))
}

/// The variable a clear empties whole: `zero_bytes(&x, n)` where `n` is every byte of `x`.
fn whole_clear(s: &Stmt, head: &Head) -> Option<Var> {
    let Stmt::ZeroBytes {
        addr: Expr::AddrOf { var, cell: 0 },
        bytes,
    } = s
    else {
        return None;
    };
    let cells = flat_cells(&Expr::Var(*var), head)?;
    (u64::from(*bytes) == u64::from(cells) * 8).then_some(*var)
}

/// Where the readable form declares the variables of a body.
struct Placement {
    /// The assignments that declare their target: its first mention, in the block that holds every other.
    sites: HashSet<*const Stmt>,
    /// The variables declared at the top of the function, where a cell starts as zero.
    hoisted: BTreeSet<Var>,
}

/// What the walk has seen of one variable.
struct Seen {
    /// The assignment that mentions it first, where it is an assignment that does.
    site: Option<*const Stmt>,
    first_block: u32,
    first: u32,
    last: u32,
    /// The innermost block that holds every mention.
    within: u32,
    addressed: bool,
}

/// One step of the walk over a body.
enum Visit<'b> {
    Block(&'b [tree::Stmt], u32),
    /// What a `do`'s condition reads, after its body.
    Reads(&'b Expr, u32),
    /// A `for`'s step, after its body.
    Step(&'b Stmt, u32),
}

#[derive(Default)]
struct Placer<'h> {
    /// The function's head, where a clear of a whole variable is written as its assignment.
    head: Option<&'h Head>,
    /// The parent and the depth of each block; block 0 is the function's.
    blocks: Vec<(u32, u32)>,
    seen: BTreeMap<Var, Seen>,
    /// The positions of the labels, rising.
    labels: Vec<u32>,
    /// The position of what is being walked, counted in source order.
    at: u32,
}

impl Placer<'_> {
    fn child(&mut self, parent: u32) -> u32 {
        let depth = self.blocks[parent as usize].1 + 1;
        self.blocks.push((parent, depth));
        (self.blocks.len() - 1) as u32
    }

    /// The innermost block that holds both `a` and `b`.
    fn common(&self, mut a: u32, mut b: u32) -> u32 {
        while a != b {
            let (above_a, depth_a) = self.blocks[a as usize];
            let (above_b, depth_b) = self.blocks[b as usize];
            match depth_a >= depth_b {
                true => a = above_a,
                false => b = above_b,
            }
        }
        a
    }

    fn mention(&mut self, var: Var, block: u32, site: Option<*const Stmt>, addressed: bool) {
        if !matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. }) {
            return;
        }
        let at = self.at;
        match self.seen.get(&var).map(|seen| seen.within) {
            Some(within) => {
                let within = self.common(within, block);
                if let Some(seen) = self.seen.get_mut(&var) {
                    seen.within = within;
                    seen.last = at;
                    seen.addressed |= addressed;
                }
            }
            None => {
                let seen = Seen {
                    site,
                    first_block: block,
                    first: at,
                    last: at,
                    within: block,
                    addressed,
                };
                self.seen.insert(var, seen);
            }
        }
    }

    fn reads(&mut self, e: &Expr, block: u32) {
        e.walk(&mut |e| match e {
            Expr::Var(var) => self.mention(*var, block, None, false),
            Expr::AddrOf { var, .. } => self.mention(*var, block, None, true),
            _ => {}
        });
    }

    /// A simple statement: what it reads, then what it assigns; `declares` where an assignment standing here may declare its target.
    fn simple(&mut self, s: &Stmt, block: u32, declares: bool) {
        // A clear of a whole variable is written as its assignment, which may declare it.
        if let Some(var) = self.head.and_then(|head| whole_clear(s, head)) {
            let site = declares.then(|| std::ptr::from_ref(s));
            self.mention(var, block, site, false);
            return;
        }
        s.each_expr(&mut |e| self.reads(e, block));
        let site = match s {
            Stmt::Assign { .. } if declares => Some(std::ptr::from_ref(s)),
            _ => None,
        };
        for target in s.targets() {
            self.mention(*target, block, site, false);
        }
    }
}

/// Finds where each variable of `body` is declared.
// A variable is declared at its first assignment when that is its first mention and the block it
// stands in holds every other; else at the top, where a read before any assignment finds zero.
// ⚠ `old` is the form that writes a construct's statements from a copy: there every variable a
// construct mentions is declared at the top.
fn placement(body: &[tree::Stmt], old: bool, head: Option<&Head>) -> Placement {
    let mut placer = Placer {
        head,
        blocks: vec![(0, 0)],
        ..Placer::default()
    };
    let mut work = vec![Visit::Block(body, 0)];
    while let Some(visit) = work.pop() {
        placer.at += 1;
        let (stmts, block) = match visit {
            Visit::Block(stmts, block) => (stmts, block),
            Visit::Reads(e, block) => {
                placer.reads(e, block);
                continue;
            }
            Visit::Step(s, block) => {
                placer.simple(s, block, false);
                continue;
            }
        };
        let Some((first, rest)) = stmts.split_first() else {
            continue;
        };
        work.push(Visit::Block(rest, block));
        match first {
            tree::Stmt::Simple(s) => placer.simple(s, block, true),
            tree::Stmt::If { cond, then, els } => {
                placer.reads(cond, block);
                // An `else` that is one `if` prints as `else if`, with no block of its own.
                let other = match els.as_slice() {
                    [tree::Stmt::If { .. }] => block,
                    _ => placer.child(block),
                };
                work.push(Visit::Block(els, other));
                work.push(Visit::Block(then, placer.child(block)));
            }
            tree::Stmt::While { cond, body } => {
                placer.reads(cond, block);
                work.push(Visit::Block(body, placer.child(block)));
            }
            tree::Stmt::DoWhile { body, cond } => {
                work.push(Visit::Reads(cond, block));
                work.push(Visit::Block(body, placer.child(block)));
            }
            tree::Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                // What the head declares is the loop's own: the head is a block around the body.
                let head = placer.child(block);
                if let Some(init) = init.as_deref() {
                    placer.simple(init, head, true);
                }
                if let Some(cond) = cond {
                    placer.reads(cond, head);
                }
                if let Some(step) = step.as_deref() {
                    work.push(Visit::Step(step, head));
                }
                work.push(Visit::Block(body, placer.child(head)));
            }
            tree::Stmt::Switch { value, cases } => {
                placer.reads(value, block);
                for case in cases.iter().rev() {
                    work.push(Visit::Block(&case.body, placer.child(block)));
                }
            }
            tree::Stmt::Return(Some(value)) => placer.reads(value, block),
            tree::Stmt::Label(_) => placer.labels.push(placer.at),
            tree::Stmt::Sugar(sugar) if old => placer.construct(sugar, block),
            tree::Stmt::Sugar(sugar) => match &**sugar {
                // A group is simple statements side by side in this block, written from
                // where the tree holds them in either form: each may declare its target.
                tree::Sugar::Group { parts, .. } => {
                    for s in parts {
                        placer.simple(s, block, true);
                    }
                }
                // A `foreach` declares its item, as a `for` does what its head assigns; the
                // loop it stands for is written from the same parts, so the item's assignment
                // may declare in that form too.
                tree::Sugar::Foreach {
                    setup,
                    cond,
                    item,
                    body,
                    step,
                    after,
                } => {
                    for s in setup {
                        placer.simple(s, block, false);
                    }
                    let head = placer.child(block);
                    placer.reads(cond, head);
                    placer.simple(item, head, true);
                    // Pushed last to first: the body, the step, then what follows the loop.
                    work.push(Visit::Block(after, block));
                    if let Some(step) = step.as_deref() {
                        work.push(Visit::Step(step, head));
                    }
                    work.push(Visit::Block(body, placer.child(head)));
                }
                // What any other construct holds is declared at the top: its statements may
                // be written from a copy, and it may hold a label a jump goes round to.
                _ => placer.construct(sugar, block),
            },
            tree::Stmt::Return(None)
            | tree::Stmt::Break
            | tree::Stmt::Continue
            | tree::Stmt::Goto(_)
            | tree::Stmt::Unreachable => {}
        }
    }

    let mut placement = Placement {
        sites: HashSet::new(),
        hoisted: BTreeSet::new(),
    };
    for (var, seen) in placer.seen {
        // ⚠ A jump to a label after the assignment may have gone round it.
        let next = placer.labels.partition_point(|label| *label <= seen.first);
        let jumped = (placer.labels.get(next)).is_some_and(|label| *label <= seen.last);
        // The address of a local can be read through before the local is assigned.
        let exposed = seen.addressed && matches!(var, Var::Local(_));
        match seen.site {
            Some(site) if seen.within == seen.first_block && !jumped && !exposed => {
                placement.sites.insert(site);
            }
            _ => {
                placement.hoisted.insert(var);
            }
        }
    }
    placement
}

impl Placer<'_> {
    /// What a construct holds, none of it a place to declare: every variable it mentions is declared at the top.
    fn construct(&mut self, sugar: &tree::Sugar, block: u32) {
        let parts = sugar.parts();
        for s in parts.simples {
            self.simple(s, block, false);
        }
        for e in parts.exprs {
            self.reads(e, block);
        }
        for body in parts.bodies {
            tree::each_simple(body, &mut |s| self.simple(s, block, false));
            tree::each_expr(body, &mut |e| self.reads(e, block));
        }
        self.labels.push(self.at);
    }
}

/// `text` in parentheses where it binds less tightly than `min`.
fn wrap(text: String, binds: u8, min: u8) -> String {
    match binds < min {
        true => format!("({text})"),
        false => text,
    }
}

/// `e` without the 64-bit readings around it, which change no bit of a cell.
fn bits(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// `e` without the integer readings around it.
fn unread(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { to, e: inner } = e {
        if to.kind().is_float() {
            break;
        }
        e = &**inner;
    }
    e
}

/// Whether `e` is a constant with no bit set, under any reading of it.
fn is_zero(e: &Expr) -> bool {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } | Expr::Truth(inner) = e {
        e = &**inner;
    }
    matches!(e, Expr::Const(c) if c.bits == 0)
}

/// The comparison that holds exactly when `op` does not, for operands that always compare.
fn flipped(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Ge,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        BinOp::Ge => BinOp::Lt,
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        other => other,
    }
}

/// `++` or `--` where `value` is `target` one up or down in 32 bits, which is how a listing's step reads.
fn step_of(target: Var, value: &Expr) -> Option<&'static str> {
    let Expr::Binary {
        op,
        at: Kind::I32,
        l,
        r,
    } = value
    else {
        return None;
    };
    let mut stepped = &**l;
    while let Expr::Cast {
        to: Scalar::I32,
        e: inner,
    } = stepped
    {
        stepped = &**inner;
    }
    let one = matches!(&**r, Expr::Const(c) if c.bits == 1);
    let itself = matches!(stepped, Expr::Var(var) if *var == target);
    match (op, one && itself) {
        (BinOp::Add, true) => Some("++"),
        (BinOp::Sub, true) => Some("--"),
        _ => None,
    }
}

/// The constant `as_T(c)` is: the low bits of `c` read as `to`.
fn reread(to: Scalar, c: &Const) -> Const {
    let bits = match to {
        Scalar::I8 => c.bits as i8 as i64 as u64,
        Scalar::U8 => u64::from(c.bits as u8),
        Scalar::I16 => c.bits as i16 as i64 as u64,
        Scalar::U16 => u64::from(c.bits as u16),
        Scalar::I32 => c.bits as i32 as i64 as u64,
        Scalar::U32 | Scalar::F32 => u64::from(c.bits as u32),
        Scalar::I64 | Scalar::U64 | Scalar::F64 => c.bits,
    };
    // A kind that says what the constant means, a hash or a truth, outlives a change of width.
    let numeric = c.kind.is_integer() || c.kind.is_float() || c.kind == Kind::Unknown;
    let kind = match numeric {
        true => to.kind(),
        false => c.kind.clone(),
    };
    Const { bits, kind }
}

/// A constant's readable text and the strength it binds at: a sign in front makes it a unary expression.
fn constant(c: &Const) -> (String, u8) {
    let text = readable_const(c);
    let binds = match text.starts_with('-') || text.starts_with('(') {
        true => UNARY,
        false => POSTFIX,
    };
    (text, binds)
}

/// The bytes a value of `kind` fills, `None` where the kind does not say.
fn kind_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Bool | Kind::I8 | Kind::U8 => Some(1),
        Kind::I16 | Kind::U16 => Some(2),
        Kind::I32 | Kind::U32 | Kind::F32 | Kind::Hash => Some(4),
        Kind::I64 | Kind::U64 | Kind::F64 | Kind::Pointer | Kind::Object(_) | Kind::String => {
            Some(8)
        }
        Kind::Array(_) => Some(8),
        Kind::Unknown | Kind::Vector | Kind::Handle | Kind::Aggregate(_) | Kind::Value { .. } => {
            None
        }
    }
}

/// The bytes an element of an array of `kind` takes in the array, where the kind states them.
fn element_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Vector | Kind::Handle => Some(16),
        Kind::Aggregate(cells) | Kind::Value { cells, .. } => cells.checked_mul(8),
        scalar => kind_bytes(scalar),
    }
}

/// A name the listing states in a string: as it stands where it is one, quoted where it is not.
fn spelled_name(text: &str) -> String {
    let spelled = |c: char| c.is_ascii_alphanumeric() || "_.<>,`[]".contains(c);
    match !text.is_empty() && text.chars().all(spelled) {
        true => text.to_owned(),
        false => quoted(text),
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("    ");
    }
}

/// The declaration classes of the wrappers `declared` holds, nested as their paths say.
fn declarations(out: &mut String, declared: &BTreeMap<String, Wrapper>) {
    let mut open: Vec<&str> = Vec::new();
    for (path, wrapper) in declared {
        let mut classes: Vec<&str> = path.split('.').collect();
        let Some(member) = classes.pop() else {
            continue;
        };
        let shared = open
            .iter()
            .zip(classes.iter())
            .take_while(|(a, b)| a == b)
            .count();
        while open.len() > shared {
            open.pop();
            push_indent(out, open.len() + 1);
            out.push_str("}\n");
        }
        for class in classes.iter().skip(shared).copied() {
            push_indent(out, open.len() + 1);
            let _ = writeln!(out, "public static class {class}");
            push_indent(out, open.len() + 1);
            out.push_str("{\n");
            open.push(class);
        }
        push_indent(out, open.len() + 1);
        out.push_str(&member_text(member, wrapper));
        out.push('\n');
    }
    while open.pop().is_some() {
        push_indent(out, open.len() + 1);
        out.push_str("}\n");
    }
}

/// One wrapper as the listing declares it.
fn member_text(member: &str, wrapper: &Wrapper) -> String {
    let returns = wrapper.returns;
    let params: Vec<String> = (0..wrapper.params)
        .map(|n| format!("VmValue p{n}"))
        .collect();
    let params = params.join(", ");
    let mut passed = String::new();
    for n in 0..wrapper.params {
        let _ = write!(passed, ", p{n}");
    }
    let (ty, cell) = match returns {
        0 => ("void", ""),
        1 => ("VmValue", "[0]"),
        _ => ("VmValues", ""),
    };
    let call = match &wrapper.target {
        Target::NewArray { ty, .. } => {
            let ty = vm_type(ty);
            return format!(
                "public static VmValue {member}({params}) => Vm.NewArray({ty}{passed});"
            );
        }
        Target::Call(Callee::Import { program, at, .. }) => {
            format!("Vm.Script(0x{program:08X}U, 0x{at:06X}, {returns}{passed})")
        }
        Target::Call(Callee::Native { hash, rva, .. }) => {
            format!("Vm.Native(0x{hash:016X}UL, 0x{rva:08X}U, {returns}{passed})")
        }
        Target::Call(Callee::Builtin { ty, method, .. }) => {
            format!("Vm.Builtin({ty}, {method}, {returns}{passed})")
        }
        Target::Call(Callee::Virtual { ty, slot, .. }) => {
            format!("Vm.Virtual({}, {slot}, {returns}{passed})", quoted(ty))
        }
        Target::Call(Callee::Intrinsic { name, .. } | Callee::Local { name, .. }) => {
            format!("Vm.Intrinsic({}, {returns}{passed})", quoted(name))
        }
    };
    format!("public static {ty} {member}({params}) => {call}{cell};")
}

/// `new VmType(..)` as a listing writes it.
fn vm_type(ty: &VmType) -> String {
    format!(
        "new VmType({}, {}, {}U, 0x{:08X}U)",
        quoted(&ty.name),
        ty.byte_size,
        ty.id,
        ty.flags
    )
}

/// A C# string literal.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A constant as a C# literal of the type its kind names, so that it reads back as it was.
fn plain_const(c: &Const) -> String {
    match c.kind {
        Kind::Bool => (c.bits != 0).to_string(),
        Kind::I32 => (c.bits as i64).to_string(),
        Kind::U32 => format!("{}U", c.bits),
        Kind::I64 => format!("{}L", c.bits as i64),
        Kind::F32 => {
            let value = c.as_f32();
            let text = format!("{value:?}");
            let read_back = text.parse::<f32>().ok().map(f32::to_bits);
            // A float no literal spells, such as a NaN, keeps its bits.
            match value.is_finite() && read_back == Some(c.bits as u32) {
                true => format!("{text}f"),
                false => format!("float_bits(0x{:08X})", c.bits as u32),
            }
        }
        Kind::F64 => {
            let value = c.as_f64();
            let text = format!("{value:?}");
            let read_back = text.parse::<f64>().ok().map(f64::to_bits);
            // ⚠ No literal spells a double that is not finite: this reads back as a cast of bits.
            match value.is_finite() && read_back == Some(c.bits) {
                true => format!("{text}d"),
                false => format!("as_f64(0x{:016X}UL)", c.bits),
            }
        }
        _ => format!("0x{:X}UL", c.bits),
    }
}

/// A constant by what its kind says it is; a number of no known kind is hex once it is large.
fn readable_const(c: &Const) -> String {
    // ⚠ Sixteen digits that are a negative `int` widened with its sign read as the `int` they
    // are: `(int)0x8E7B3F2C` is the cell `0xFFFFFFFF8E7B3F2C`, bit for bit.
    let widened = |bits: u64| bits >> 32 == 0xFFFF_FFFF && (bits as i32) < -0x1_0000;
    let decimal = |value: i64| match (-0x1_0000..=0x1_0000).contains(&value) {
        true => value.to_string(),
        false if widened(value as u64) => format!("(int)0x{:08X}", value as u32),
        false => format!("0x{:X}", value as u64),
    };
    match c.kind {
        Kind::Bool => (c.bits != 0).to_string(),
        Kind::I8 => (c.bits as i8).to_string(),
        Kind::U8 => (c.bits as u8).to_string(),
        Kind::I16 => (c.bits as i16).to_string(),
        Kind::U16 => (c.bits as u16).to_string(),
        Kind::I32 if (c.bits as i32).unsigned_abs() > 0x1_0000 => {
            format!("0x{:08X}", c.bits as u32)
        }
        Kind::I32 => (c.bits as i32).to_string(),
        Kind::U32 | Kind::Hash if c.bits as u32 > 0x1_0000 => format!("0x{:08X}", c.bits as u32),
        Kind::U32 | Kind::Hash => (c.bits as u32).to_string(),
        Kind::I64 => decimal(c.bits as i64),
        Kind::U64 if widened(c.bits) => format!("(int)0x{:08X}", c.bits as u32),
        Kind::U64 if c.bits > 0x1_0000 => format!("0x{:X}", c.bits),
        Kind::U64 => c.bits.to_string(),
        Kind::F32 if c.as_f32().is_finite() => format!("{:?}f", c.as_f32()),
        Kind::F32 => format!("float_bits(0x{:08X})", c.bits as u32),
        Kind::F64 => format!("{:?}", c.as_f64()),
        // Only zero is ever given a reference's kind.
        Kind::Pointer if c.bits == 0 => "null".to_owned(),
        _ => decimal(c.bits as i64),
    }
}

#[cfg(test)]
#[path = "print_tests.rs"]
mod tests;
