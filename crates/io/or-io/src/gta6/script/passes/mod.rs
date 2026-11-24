//! The passes a function goes through between the reader and the printer, in the order
//! `values`, `types`, `structure`, `names`. None may change what the code does: a pass that is
//! not sure of a rewrite leaves the code as it found it.

pub mod names;
pub mod structure;
pub mod types;
pub mod values;
