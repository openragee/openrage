//! Writing the structured form as text: readable C#-like source, and the listing's own syntax
//! again, so that what was read can be read back.
//!
//! The readable form changes nothing of the tree. It leaves unwritten what the tree already
//! says: a cast of a value of that very kind, a 64-bit reading where every bit counts alike, a
//! zero test of a truth value. Memory prints as `p->f_10`, the field at byte 0x10 of what `p`
//! points at, or `v.f_10` for byte 0x10 of the variable `v` itself, and a variable is declared
//! where it is first assigned.
//!
//! ⚠ `(float)x` in the readable form is the listing's `as_f32(x)`: the low bits of `x` read as a
//! float, never a conversion. A conversion is a call, `i32_to_f32(x)`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write;

use super::ir::{
    BinOp, Callee, Const, Expr, Head, Kind, Returns, Scalar, Stmt, StoreKind, UnOp, Var, VmType,
    Width,
};
use super::listing::{Import, Listing, Target, Wrapper};
use super::names::{declared_natives, Names};
use super::passes::types::yielded_shape;
use super::tree;
use super::Stats;

/// The strength a unary operator and a cast bind at, above every binary operator.
const UNARY: u8 = 12;
/// The strength of a name, a call and an index.
const POSTFIX: u8 = 13;

/// Writes one function as readable source, naming what `names` has a name for.
pub fn function(out: &mut String, f: &tree::Function, names: &Names) {
    let mut writer = Writer::new(out, false, names, &f.head);
    writer.readable_function(&f.body);
}

/// Writes a listing's functions as readable source, under a comment saying what named things in it.
pub struct SourceWriter<'a> {
    names: &'a Names,
    out: String,
}

impl<'a> SourceWriter<'a> {
    pub fn new(names: &'a Names) -> Self {
        SourceWriter {
            names,
            out: String::new(),
        }
    }

    /// Writes one function under a blank line.
    pub fn function(&mut self, f: &tree::Function) {
        self.out.push('\n');
        function(&mut self.out, f, self.names);
    }

    /// The source under its header: the program, its imports, the lists that named things and what `stats` counted.
    pub fn finish(self, listing: &Listing<'_>, stats: &Stats) -> String {
        let mut text = header(listing, self.names, stats);
        text.push_str(&self.out);
        text
    }
}

/// The comment a decompiled listing opens with.
fn header(listing: &Listing<'_>, names: &Names, stats: &Stats) -> String {
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
            "// natives: {total} declared, no natives list supplied: {by_listing} named by the \
             listing itself, {by_hash} shown by hash"
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
    /// The `else` of an `if` whose first arm is written.
    Else(&'b [tree::Stmt], usize),
    Cases(&'b [tree::Case], usize),
    /// The `while (..);` that closes a `do`.
    Until(&'b Expr, usize),
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
    sites: HashSet<*const Stmt>,
}

impl<'a> Writer<'a> {
    fn new(out: &'a mut String, plain: bool, names: &'a Names, head: &'a Head) -> Self {
        Writer {
            out,
            plain,
            names,
            head,
            declared: None,
            sites: HashSet::new(),
        }
    }

    fn line(&mut self, indent: usize, text: &str) {
        push_indent(self.out, indent);
        self.out.push_str(text);
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
            let text = format!(
                "// {} at 0x{:06X}: the listing holds no body ({reason})",
                head.name, head.at
            );
            self.line(0, &text);
            return;
        }
        let ty = match head.returns {
            Returns::Void => "void",
            Returns::Cell => "var",
            Returns::Cells => "var[]",
        };
        let params: Vec<String> = (0..head.params)
            .map(Var::Param)
            .map(|p| format!("{} {}", self.declared_type(p), self.var(p)))
            .collect();
        self.line(0, &format!("{ty} {}({})", head.name, params.join(", ")));
        self.line(0, "{");

        let placement = placement(body);
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
            for few in names.chunks(8) {
                self.line(1, &format!("{ty} {};", few.join(", ")));
            }
        }
        self.body(body, 1);
        self.line(0, "}");
    }

    /// Writes `stmts` and everything nested in them.
    // An explicit stack: a body nests as deep as its listing does, and a chain of `else if` is
    // as deep as it is long.
    fn body(&mut self, stmts: &[tree::Stmt], indent: usize) {
        let mut work = vec![Work::Run(stmts, indent)];
        while let Some(item) = work.pop() {
            match item {
                Work::Line(indent, text) => self.line(indent, text),
                Work::Run(stmts, indent) => {
                    if let Some((first, rest)) = stmts.split_first() {
                        let pair = match self.plain {
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
                Work::Cases(cases, indent) => {
                    if let Some((case, rest)) = cases.split_first() {
                        work.push(Work::Cases(rest, indent));
                        for label in &case.labels {
                            match label {
                                Some(value) => self.line(indent + 1, &format!("case {value}:")),
                                None => self.line(indent + 1, "default:"),
                            }
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
                self.test("else if", cond, indent);
                self.open(then, indent, Some(Work::Else(els, indent)), work);
            }
            _ => {
                self.line(indent, "else");
                self.open(els, indent, None, work);
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
                self.test("if", cond, indent);
                self.open(then, indent, Some(Work::Else(els, indent)), work);
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
                let value = self.value(value);
                self.line(indent, &format!("switch ({value})"));
                self.line(indent, "{");
                work.push(Work::Line(indent, "}"));
                work.push(Work::Cases(cases, indent));
            }
            tree::Stmt::Break => self.line(indent, "break;"),
            tree::Stmt::Continue => self.line(indent, "continue;"),
            tree::Stmt::Return(None) => self.line(indent, "return;"),
            tree::Stmt::Return(Some(value)) => {
                let value = self.value(value);
                self.line(indent, &format!("return {value};"));
            }
            tree::Stmt::Goto(label) => self.line(indent, &format!("goto {label};")),
            tree::Stmt::Label(label) => self.line(indent, &format!("{label}: ;")),
            tree::Stmt::Unreachable if self.plain => self.line(
                indent,
                "throw new global::System.InvalidOperationException(\"Unexpected bytecode fallthrough.\");",
            ),
            tree::Stmt::Unreachable => self.line(indent, "unreachable;"),
        }
    }

    /// A simple statement without its semicolon, declaring its target where this is the place to.
    fn statement(&mut self, s: &Stmt) -> String {
        let text = self.simple(s);
        match s {
            Stmt::Assign { target, .. } if self.sites.contains(&std::ptr::from_ref(s)) => {
                format!("{} {text}", self.declared_type(*target))
            }
            _ => text,
        }
    }

    /// A simple statement without its semicolon.
    fn simple(&mut self, stmt: &Stmt) -> String {
        match stmt {
            Stmt::Assign { target, value } => match step_of(*target, value) {
                Some(step) if !self.plain => format!("{}{step}", self.var(*target)),
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
                let (place, _) = self.place(addr, *offset, narrow.is_some());
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
                    (None, ..) => self.value(value),
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
                let (place, _) = self.place(addr, *offset, true);
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
            Stmt::ZeroBytes { addr, bytes } => {
                format!("zero_bytes({}, {bytes})", self.expr(addr, 0))
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
                let (place, binds) = self.place(addr, *offset, true);
                format!("{}{step}", wrap(place, binds, POSTFIX))
            }
            Stmt::Eval(e) => {
                let assigned = match self.plain {
                    true => None,
                    false => self.assignment(e),
                };
                assigned.unwrap_or_else(|| self.expr(e, 0))
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
                self.place(addr, *offset, false).0
            }
            Expr::ZeroSlots(cells)
                if !self.plain && flat_cells(&Expr::Var(target), self.head) == Some(*cells) =>
            {
                "default".to_owned()
            }
            _ => self.value(value),
        }
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
        match self.names.listed_type(listed) {
            Some(name) => name.to_owned(),
            None => spelled_name(listed),
        }
    }

    /// The type a variable is declared with: its kind where one is known, `var` otherwise.
    fn declared_type(&self, var: Var) -> String {
        match self.head.vars.get(&var) {
            Some(info) if info.kind == Kind::Unknown && info.width == Width::Cells => {
                "var[]".to_owned()
            }
            Some(info) => self.kind_text(&info.kind),
            None => "var".to_owned(),
        }
    }

    /// The C#-like name of a kind, for a declaration.
    fn kind_text(&self, kind: &Kind) -> String {
        let name = match kind {
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
            Kind::Object(name) => return self.type_name(name),
            Kind::String => "string",
            Kind::Vector => "Vector3",
            Kind::Handle => "Handle",
            Kind::Aggregate(cells) => return format!("var[{cells}]"),
        };
        name.to_owned()
    }

    /// What `e` is, as far as its own shape and the kinds of its variables say.
    fn kind_of(&self, e: &Expr) -> Kind {
        match e {
            Expr::Const(c) => c.kind.clone(),
            Expr::Var(var) => (self.head.vars.get(var)).map_or(Kind::Unknown, |i| i.kind.clone()),
            Expr::Cast { to, .. } | Expr::Load { ty: to, .. } => to.kind(),
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => Kind::Bool,
            Expr::Binary { op, .. } if op.is_logical() => Kind::Bool,
            Expr::Binary { at, .. } => at.clone(),
            Expr::Vector3(_) => Kind::Vector,
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
            Expr::Const(c) => return constant(c),
            Expr::Var(var) => self.var(*var),
            Expr::GuidEmpty => "Guid.Empty".to_owned(),
            Expr::Cell { base, index } => format!("{}[{index}]", self.expr(base, POSTFIX)),
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
                let (place, binds) = self.place(addr, *offset, false);
                return match ty {
                    Scalar::U64 => (place, binds),
                    _ => (
                        format!("({}){}", ty.keyword(), wrap(place, binds, UNARY)),
                        UNARY,
                    ),
                };
            }
            // Whole cells are a field as wide as the cast says, as `(int)p->f_10` is four bytes.
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } if bytes.is_multiple_of(8) => {
                let (place, binds) = self.place(addr, *offset, false);
                let text = format!("(var[{}]){}", bytes / 8, wrap(place, binds, UNARY));
                return (text, UNARY);
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
            Expr::ZeroSlots(cells) => format!("default(var[{cells}])"),
            Expr::Vector3(parts) => format!("new Vector3({})", self.list(&parts[..])),
            Expr::NewObject(ty) => format!("new {}()", self.type_name(&ty.name)),
            Expr::NewArray { ty, len, .. } => {
                format!("new {}[{}]", self.type_name(&ty.name), self.value(len))
            }
            Expr::FunctionRef { program, at } => {
                let function = self.member(
                    *program,
                    format!("func_{at:06X}"),
                    format!("program_{program:08X}.func_{at:06X}"),
                );
                return (format!("&{function}"), UNARY);
            }
            Expr::BuiltinFunctionRef { ty, method } => format!("BuiltinFunction({ty}, {method})"),
            Expr::VirtualFunctionRef { receiver, ty, slot } => format!(
                "VirtualFunction({}, {}, {slot})",
                self.value(receiver),
                self.type_name(&ty.name)
            ),
            Expr::StringConstant { program, offset } => self.member(
                *program,
                format!("String_{offset:X}"),
                format!("String_{program:08X}_{offset:X}"),
            ),
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
            return constant(&reread(to, c));
        }
        let kind = self.kind_of(inner);
        // Zero or one is zero or one under any integer reading, and a hash is its 32 bits.
        let says_nothing = kind == to.kind()
            || (kind == Kind::Bool && !to.kind().is_float())
            || (kind == Kind::Hash && matches!(to, Scalar::I32 | Scalar::U32));
        // ⚠ Between a float and an integer a C# cast converts, and this reads the same bits: it
        // keeps the listing's own name.
        let whole = kind.is_integer() || matches!(kind, Kind::Bool | Kind::Hash);
        let reread = (to.kind().is_float() && whole) || (!to.kind().is_float() && kind.is_float());
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
            Expr::Call { callee, args, .. } if not && self.equality(callee, args).is_some() => {
                match self.equality(callee, args) {
                    Some((equal, l, r)) => {
                        let op = if equal { "!=" } else { "==" };
                        (format!("{} {op} {}", self.side(l), self.side(r)), 7)
                    }
                    None => self.spelled(e),
                }
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
                (self.expr(bits(l), p), self.expr(bits(r), p + 1))
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
        let (mut text, variable) = match base {
            Expr::AddrOf { var, cell: 0 } => (self.var(*var), true),
            Expr::AddrOf { var, cell } => (format!("{}[{cell}]", self.var(*var)), true),
            // The first bytes of what a pointer points at are the thing itself.
            pointer if !embedded && offset == 0 => {
                return (format!("*{}", self.expr(pointer, UNARY)), UNARY);
            }
            pointer => (self.expr(pointer, POSTFIX), false),
        };
        // ⚠ `->` goes through a pointer and `.` stays in place: `local_3->f_8` reads what the
        // pointer in `local_3` points at, `local_3.f_8` the frame cell after `local_3`.
        let mut through = !variable;
        for (n, field) in fields.iter().enumerate().rev() {
            let unsaid = *field == 0 && n == 0 && (embedded || (variable && !partial));
            if !unsaid {
                let via = if std::mem::take(&mut through) {
                    "->"
                } else {
                    "."
                };
                let _ = write!(text, "{via}f_{field:X}");
            }
        }
        (text, POSTFIX)
    }

    /// `value` as the low `to.bytes()` bytes a narrow store writes: under a cast unless its kind has that width.
    fn stored(&mut self, to: Scalar, value: &Expr) -> String {
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

    fn readable_call(&mut self, callee: &Callee, args: &[Expr]) -> (String, u8) {
        match callee {
            Callee::Intrinsic { name, .. } => {
                if let Some(spelled) = self.intrinsic(name, args) {
                    return spelled;
                }
            }
            // A receiver whose own type is the class says whose method it is.
            Callee::Virtual { class, method, .. } => match args.split_first() {
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
        if let Some((equal, l, r)) = self.equality(callee, args) {
            let op = if equal { "==" } else { "!=" };
            let text = format!("{} {op} {}", self.side(l), self.side(r));
            return (text, 7);
        }
        let name = callee_name(callee, self.names);
        (format!("{name}({})", self.list(args)), POSTFIX)
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
    fn test(&mut self, word: &str, cond: &Expr, indent: usize) {
        let one = self.cond(cond, 0);
        let op = match cond {
            Expr::Binary {
                op: op @ (BinOp::LogicalAnd | BinOp::LogicalOr),
                ..
            } if !self.plain && 4 * indent + word.len() + one.len() > 118 => *op,
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
                push_indent(&mut text, indent + 1);
                text.push_str(op.symbol());
                text.push(' ');
            }
            text.push_str(&self.cond(operand, p + 1));
        }
        self.line(indent, &format!("{word} ({text})"));
    }

    /// An intrinsic C# has an operator for, as that operator; `None` for one that stays a call.
    fn intrinsic(&mut self, name: &str, args: &[Expr]) -> Option<(String, u8)> {
        match args {
            [target, passed @ ..] if name == "invoke_delegate" => {
                let target = self.expr(target, POSTFIX);
                Some((format!("{target}({})", self.list(passed)), POSTFIX))
            }
            [array] if name == "array_length" => {
                Some((format!("{}.Length", self.expr(array, POSTFIX)), POSTFIX))
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
                Some(match ty {
                    Scalar::U64 => (element, POSTFIX),
                    _ => (format!("({}){element}", ty.keyword()), UNARY),
                })
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
                Kind::Aggregate(cells) => Some(cells),
                _ => None,
            },
            _ => Some(1),
        },
        Expr::Cells(_) | Expr::BuiltinGlobal(_) => None,
        Expr::GuidEmpty | Expr::Vector3(_) => Some(2),
        Expr::ZeroSlots(cells) => Some(*cells),
        Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => Some(bytes.div_ceil(8)),
        Expr::Call { returns, .. } => returns.filter(|cells| *cells > 0),
        _ => Some(1),
    }
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
struct Placer {
    /// The parent and the depth of each block; block 0 is the function's.
    blocks: Vec<(u32, u32)>,
    seen: BTreeMap<Var, Seen>,
    /// The positions of the labels, rising.
    labels: Vec<u32>,
    /// The position of what is being walked, counted in source order.
    at: u32,
}

impl Placer {
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
fn placement(body: &[tree::Stmt]) -> Placement {
    let mut placer = Placer {
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
        Kind::Unknown | Kind::Vector | Kind::Handle | Kind::Aggregate(_) => None,
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
