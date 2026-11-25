//! The passes a function goes through between a front end and the printer. `values`, `types`,
//! `structure` and `names` are the first four and always were; each other one runs only where a
//! name of its `PASSES` is on in the [`Options`](super::Options), and `script::blocks_in` states
//! the order.
//!
//! ⚠ None may change what the code does: a pass that is not sure of a rewrite leaves the code as
//! it found it. A reconstruction of a source construct holds the statements it was made from,
//! so that `tree::desugar` gives back the form it replaced.

pub mod aggregates;
pub mod async_await;
pub mod classes;
pub mod closures;
pub mod coalesce;
pub mod fields;
pub mod forward;
pub mod interproc;
pub mod names;
pub mod structure;
pub mod sugar_expressions;
pub mod sugar_statements;
pub mod types;
pub mod values;
