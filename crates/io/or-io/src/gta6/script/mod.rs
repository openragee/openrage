//! Decompiling GTA VI's scripts to readable source, from the lifted listings a decompiler of
//! their bytecode writes.
//!
//! A function goes from [`listing`] through [`cfg::lower`], the four [`passes`] and [`print`]:
//! read, cut into blocks, values put back together, types inferred, blocks structured, names
//! given, text written. [`ir`] is what all of them speak, and a bytecode front end could emit it
//! in the listing's place.
//!
//! ⚠ **OpenRage ships no names.** A native prints under the name the listing gives it or the
//! user's list does ([`Names`]), and memory of no known layout under its offset: `p0->f_10`,
//! `Global_48`, `func_000100`. A name is never invented.

pub mod cfg;
pub mod compiled;
pub mod ir;
pub mod listing;
pub mod names;
pub mod native_addresses;
pub mod passes;
pub mod print;
pub mod tree;

pub use names::Names;

use ir::{Callee, Expr};
use listing::{Listing, ListingError};

/// Why a listing did not decompile.
#[derive(Debug, thiserror::Error)]
pub enum DecompileError {
    #[error(transparent)]
    Listing(#[from] ListingError),
    #[error("{function}: {source}")]
    Lower {
        function: String,
        source: cfg::LowerError,
    },
}

/// A listing's readable source, and what producing it found.
#[derive(Clone, Debug)]
pub struct Decompiled {
    pub text: String,
    pub stats: Stats,
}

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
    }
}

/// Reads a listing and writes every function of it as readable source.
pub fn decompile(text: &str, names: &Names) -> Result<Decompiled, DecompileError> {
    let listing = Listing::open(text)?;
    let mut stats = Stats::default();
    let mut writer = print::SourceWriter::new(names);
    for read in listing.functions() {
        writer.function(&function(read?, names, &mut stats)?);
    }
    Ok(Decompiled {
        text: writer.finish(&listing, &stats),
        stats,
    })
}

/// Takes one function the reader made through the passes, to the form the printer takes.
pub fn function(
    f: tree::Function,
    names: &Names,
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
    let mut blocks = lowered?;
    passes::values::run(&mut blocks, names, stats);
    passes::types::run(&mut blocks, names, stats);
    let mut done = passes::structure::run(blocks, names, stats);
    passes::names::run(&mut done, names, stats);

    let written = tree::counts(&done.body);
    stats.statements_out += written.statements;
    stats.gotos_out += written.gotos;
    stats.temporaries_out += tree::temporaries(&done.body).len();
    tree::each_expr(&done.body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Call {
                callee: Callee::Native { name, hash, .. },
                ..
            } = e
            {
                stats.native_calls += 1;
                // A name that is only the hash spelled `_0x..` names nothing.
                let named = !name.starts_with("_0x") || names.native(*hash).is_some();
                stats.natives_named += usize::from(named);
            }
        });
    });
    Ok(done)
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
