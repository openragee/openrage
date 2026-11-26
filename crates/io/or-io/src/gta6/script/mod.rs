//! Decompiling GTA VI's scripts to readable source, from a compiled program or from the lifted
//! listing a decompiler of its bytecode writes.
//!
//! A function comes from a front end, [`listing`] or [`program`] through [`decode`] and [`lift`],
//! goes through the [`passes`] in the order [`blocks_in`] states, and is written by [`print`]
//! and [`emit`]. [`ir`] is what all of them speak. [`project`] holds what was gathered over
//! every program of an install beforehand, [`class`] the types a program's functions are put
//! under, and [`options`] the level of the output and the passes that run.
//!
//! ⚠ **OpenRage ships no names.** A native prints under the name the listing gives it or the
//! user's list does ([`Names`]), and memory of no known layout under its offset: `p0->f_10`,
//! `Global_48`, `func_000100`. A name is never invented.

pub mod cfg;
pub mod class;
pub mod compiled;
pub mod compiled_write;
pub mod decode;
pub mod emit;
pub mod ir;
pub mod lift;
pub mod listing;
pub mod names;
pub mod native_addresses;
pub mod opcode;
pub mod options;
pub mod passes;
pub mod print;
pub mod program;
pub mod project;
pub mod reparse;
pub mod tree;

pub use names::Names;
pub use options::{Level, Options, Pass};
pub use project::Project;

use ir::{Callee, Expr, ExprSugar};
use listing::{Listing, ListingError};

/// The rounds `forward` and `coalesce` are given with `values` between them: each feeds the others.
const ROUNDS: usize = 4;

/// The blocks of a function the passes are given: one of more is written as its blocks.
// ⚠ Some pass takes time that grows faster than the blocks do, and 100,000 tests in a row are
// under a megabyte of code. No function of the scripts read holds a tenth of this.
const BLOCKS: usize = 1 << 16;

/// Why a program did not decompile.
#[derive(Debug, thiserror::Error)]
pub enum DecompileError {
    #[error(transparent)]
    Listing(#[from] ListingError),
    #[error("{function}: {source}")]
    Lower {
        function: String,
        source: cfg::LowerError,
    },
    #[error(transparent)]
    Program(#[from] program::ProgramError),
    #[error(transparent)]
    Lift(#[from] lift::LiftError),
}

/// What every pass and the printer are handed beside a function.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub names: &'a Names,
    pub project: &'a Project,
    pub options: Options,
}

impl<'a> Context<'a> {
    pub fn new(names: &'a Names, project: &'a Project, options: Options) -> Self {
        Context {
            names,
            project,
            options,
        }
    }

    /// `names` with an empty project and the four first passes: what [`decompile`] runs.
    pub fn baseline(names: &'a Names) -> Self {
        Context::new(names, Project::empty(), Options::baseline())
    }
}

/// A listing's readable source, and what producing it found.
#[derive(Clone, Debug)]
pub struct Decompiled {
    pub text: String,
    pub stats: Stats,
}

/// One file of output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub text: String,
}

/// What decompiling one program wrote, and what producing it found.
#[derive(Clone, Debug)]
pub struct Output {
    /// In the order of their names.
    pub files: Vec<File>,
    pub stats: Stats,
}

/// What [`Stats::constructs`] counts, in its order: the statement constructs, the groups, then the readings of an expression.
pub const CONSTRUCTS: [&str; 31] = [
    "hidden",
    "await",
    "yield",
    "foreach",
    "guarded",
    "try",
    "throw",
    "switch-on",
    "construct",
    "collection",
    "delegate",
    "compound",
    "field-store",
    "out",
    "deconstruct",
    "concat-group",
    "method",
    "field",
    "property",
    "operator",
    "lambda",
    "coalesce",
    "null-conditional",
    "concat",
    "convert",
    "typeof",
    "out-argument",
    "ref",
    "tuple",
    "event",
    "kept-delegate",
];

/// What decompiling found; each `_in` is counted on what the reader made, each `_out` on what the printer was given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub functions: usize,
    /// Functions the listing holds no body for.
    pub incomplete: usize,
    /// Simple statements.
    pub statements_in: usize,
    pub statements_out: usize,
    pub gotos_in: usize,
    pub gotos_out: usize,
    /// `v_` and `stack_` variables: declared going in, still used coming out.
    pub temporaries_in: usize,
    pub temporaries_out: usize,
    pub native_calls: usize,
    /// Native calls that print under a name, the listing's or the user's.
    pub natives_named: usize,
    /// The source constructs the printer was given, each kind at its place in [`CONSTRUCTS`].
    // ⚠ A rewrite that leaves no construct (a store forwarded, a copy merged, a kind found) is
    // not here: its measure is the other counts with its pass switched off.
    pub constructs: [usize; CONSTRUCTS.len()],
}

impl Stats {
    pub fn add(&mut self, other: &Stats) {
        self.functions += other.functions;
        self.incomplete += other.incomplete;
        self.statements_in += other.statements_in;
        self.statements_out += other.statements_out;
        self.gotos_in += other.gotos_in;
        self.gotos_out += other.gotos_out;
        self.temporaries_in += other.temporaries_in;
        self.temporaries_out += other.temporaries_out;
        self.native_calls += other.native_calls;
        self.natives_named += other.natives_named;
        for (sum, more) in self.constructs.iter_mut().zip(other.constructs) {
            *sum += more;
        }
    }
}

/// Reads a listing and writes every function of it as readable source, by the four first passes.
pub fn decompile(text: &str, names: &Names) -> Result<Decompiled, DecompileError> {
    let done = decompile_listing(text, &Context::baseline(names))?;
    Ok(Decompiled {
        text: done.files.into_iter().map(|file| file.text).collect(),
        stats: done.stats,
    })
}

/// Reads a listing and writes every function of it, by the level and the passes `cx` asks.
// Below `Source` a function is written as soon as it is done, so one is held at a time.
pub fn decompile_listing(text: &str, cx: &Context<'_>) -> Result<Output, DecompileError> {
    let listing = Listing::open(text)?;
    let mut stats = Stats::default();
    let text = match cx.options.level {
        Level::Lifted => {
            let mut writer = print::ListingWriter::new(listing.program, &listing.imports);
            for read in listing.functions() {
                let read = read?;
                stats.functions += 1;
                stats.incomplete += usize::from(read.head.incomplete.is_some());
                writer.function(&read);
            }
            writer.finish()
        }
        Level::Structured => {
            let mut writer = print::SourceWriter::in_context(cx);
            for read in listing.functions() {
                writer.function(&function_in(read?, cx, &mut stats)?);
            }
            writer.finish(&listing, &stats)
        }
        Level::Source => {
            let mut done = Vec::new();
            for read in listing.functions() {
                done.push(function_in(read?, cx, &mut stats)?);
            }
            let imports = listing.imports.clone();
            let done = passes::async_await::joined(done, cx);
            let mut unit = passes::classes::unit(listing.program, imports, done, cx);
            unit.front = Some(print::LISTING);
            let files = emit::unit(&unit, cx, &stats);
            return Ok(Output { files, stats });
        }
    };
    let name = format!("program_{:08X}.cs", listing.program);
    Ok(Output {
        files: vec![File { name, text }],
        stats,
    })
}

/// Reads a compiled program and writes every function of it, by the level and the passes `cx` asks.
pub fn decompile_program(bytes: &[u8], cx: &Context<'_>) -> Result<Output, DecompileError> {
    lift::decompile(bytes, cx)
}

/// The output of a program whose functions are done: its classes gathered, then written.
pub fn assemble(
    program: u32,
    imports: Vec<listing::Import>,
    functions: Vec<tree::Function>,
    cx: &Context<'_>,
    stats: Stats,
) -> Output {
    let functions = passes::async_await::joined(functions, cx);
    let unit = passes::classes::unit(program, imports, functions, cx);
    Output {
        files: emit::unit(&unit, cx, &stats),
        stats,
    }
}

/// Takes one function the reader made through the four first passes, to the form the printer takes.
pub fn function(
    f: tree::Function,
    names: &Names,
    stats: &mut Stats,
) -> Result<tree::Function, DecompileError> {
    function_in(f, &Context::baseline(names), stats)
}

/// Takes one function the reader made through the passes `cx` asks, to the form the printer takes.
pub fn function_in(
    f: tree::Function,
    cx: &Context<'_>,
    stats: &mut Stats,
) -> Result<tree::Function, DecompileError> {
    stats.functions += 1;
    if f.head.incomplete.is_some() {
        stats.incomplete += 1;
        return Ok(f);
    }
    let read = tree::counts(&f.body);
    stats.statements_in += read.statements;
    stats.gotos_in += read.gotos;
    stats.temporaries_in += f.head.vars.keys().filter(|v| v.is_temporary()).count();

    let name = f.head.name.clone();
    let lowered = cfg::lower(f).map_err(|source| DecompileError::Lower {
        function: name,
        source,
    });
    Ok(blocks_in(lowered?, cx, stats))
}

/// Takes one function's blocks through the passes `cx` asks, in the one order they run in.
// ⚠ A pass runs only where its name is on, so with the four first names alone this is what
// `function` always did, and switching one name off shows what that pass changes.
pub fn blocks_in(f: cfg::Function, cx: &Context<'_>, stats: &mut Stats) -> tree::Function {
    let options = &cx.options;
    if f.cfg.blocks.len() > BLOCKS {
        let (function, blocks) = (&f.head.name, f.cfg.blocks.len());
        tracing::warn!(%function, blocks, "more blocks than the passes are given: no pass runs");
        let done = cfg::flat(f);
        counted(&done, cx, stats);
        return done;
    }
    let mut blocks = f;
    // ⚠ Before any pass: the widths are proved of the form a front end made, and of no other.
    if options.on(Pass::Escape) {
        passes::values::widths(&mut blocks);
    }
    if options.on(Pass::Values) {
        passes::values::run_in(&mut blocks, cx, stats);
    }
    for _ in 0..ROUNDS {
        let mut changed = false;
        if options.any(passes::forward::PASSES) {
            changed |= passes::forward::run(&mut blocks, cx, stats);
        }
        if options.any(passes::coalesce::PASSES) {
            changed |= passes::coalesce::run(&mut blocks, cx, stats);
        }
        if !changed {
            break;
        }
        if options.on(Pass::Values) {
            passes::values::run_in(&mut blocks, cx, stats);
        }
    }
    if options.any(passes::interproc::PASSES) {
        passes::interproc::run(&mut blocks, cx, stats);
    }
    if options.any(passes::aggregates::PASSES) {
        passes::aggregates::run(&mut blocks, cx, stats);
    }
    if options.any(passes::aggregates::PASSES) {
        passes::coalesce::settle(&mut blocks, cx, stats);
    }
    if options.on(Pass::Types) {
        passes::types::run_in(&mut blocks, cx, stats);
    }
    let machine = match options.any(passes::async_await::PASSES) {
        true => passes::async_await::untangle(&mut blocks, cx, stats),
        false => None,
    };
    let mut done = match options.on(Pass::Structure) {
        true => passes::structure::run_in(blocks, cx, stats),
        false => cfg::flat(blocks),
    };
    if let Some(machine) = machine {
        passes::async_await::run(&mut done, machine, cx, stats);
    }
    if options.any(passes::fields::PASSES) {
        passes::fields::run(&mut done, cx, stats);
    }
    if options.any(passes::classes::PASSES) {
        passes::classes::run(&mut done, cx, stats);
    }
    if options.any(passes::closures::PASSES) {
        passes::closures::run(&mut done, cx, stats);
    }
    if options.any(passes::sugar_statements::PASSES) {
        passes::sugar_statements::run(&mut done, cx, stats);
    }
    if options.any(passes::sugar_expressions::PASSES) {
        passes::sugar_expressions::run(&mut done, cx, stats);
    }
    if options.on(Pass::Names) {
        passes::names::run_in(&mut done, cx, stats);
    }
    counted(&done, cx, stats);
    done
}

/// Counts what the printer is given of one function.
fn counted(done: &tree::Function, cx: &Context<'_>, stats: &mut Stats) {
    let names = cx.names;
    // What a state machine added is written as nothing at `Source`, and as its statements below.
    let written = match cx.options.level {
        Level::Source => tree::shown(&done.body),
        _ => tree::counts(&done.body),
    };
    stats.statements_out += written.statements;
    stats.gotos_out += written.gotos;
    stats.temporaries_out += tree::temporaries(&done.body).len();
    tree::each_sugar(&done.body, &mut |sugar| {
        stats.constructs[construct(sugar)] += 1
    });
    tree::each_expr(&done.body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::Call {
                callee: Callee::Native { name, hash, .. },
                ..
            } => {
                stats.native_calls += 1;
                // A name that is only the hash spelled `_0x..`, or the handler's address spelled `rva_..`, names nothing.
                let spelled = name.starts_with("_0x") || name.starts_with("rva_");
                let named = !spelled || names.native(*hash).is_some();
                stats.natives_named += usize::from(named);
            }
            Expr::Sugar { kind, .. } => stats.constructs[reading(*kind)] += 1,
            _ => {}
        });
    });
}

/// The place of a statement construct in [`CONSTRUCTS`].
fn construct(sugar: &tree::Sugar) -> usize {
    use tree::{Group, Sugar};
    match sugar {
        Sugar::Hidden(_) => 0,
        Sugar::Await { .. } => 1,
        Sugar::Yield { .. } => 2,
        Sugar::Foreach { .. } => 3,
        Sugar::Guarded { .. } => 4,
        Sugar::Try { .. } => 5,
        Sugar::Throw(_) => 6,
        Sugar::SwitchOn { .. } => 7,
        Sugar::Event { .. } => 29,
        Sugar::Cached { .. } => 30,
        Sugar::Group { kind, .. } => match kind {
            Group::Construct => 8,
            Group::Collection => 9,
            Group::Delegate => 10,
            Group::Compound => 11,
            Group::Field { .. } => 12,
            Group::Out => 13,
            Group::Deconstruct => 14,
            Group::Concat => 15,
        },
    }
}

/// The place of an expression's reading in [`CONSTRUCTS`].
fn reading(kind: ExprSugar) -> usize {
    match kind {
        ExprSugar::Method => 16,
        ExprSugar::Field { .. } => 17,
        ExprSugar::Property => 18,
        ExprSugar::Operator => 19,
        ExprSugar::Lambda => 20,
        ExprSugar::Coalesce => 21,
        ExprSugar::NullConditional => 22,
        ExprSugar::Concat => 23,
        ExprSugar::Convert => 24,
        ExprSugar::TypeOf => 25,
        ExprSugar::Out => 26,
        ExprSugar::Ref => 27,
        ExprSugar::Tuple => 28,
    }
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod exec;

#[cfg(test)]
#[path = "sample_tests.rs"]
mod sample;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
