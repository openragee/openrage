//! The names a decompile may use beyond the listing's own: a natives list, type names and program
//! names, all supplied by the user. OpenRage ships none, so every lookup may come back empty.
//!
//! A names list is text, one `0xHASH name` per line. A tab-separated table whose leading columns
//! are hashes and whose next column is the name reads the same way, under one row of column names.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::ir::Callee;
use super::listing::{Declarations, Target};
use crate::gta5::script::natives::{NativeDb, NativeName};

/// What the passes and the printer consult for a name.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// The user's natives list, keyed by the 64-bit hash a native is called by.
    pub natives: Option<NativeDb>,
    /// Type names by the 64-bit hash a listing spells as `type_` and sixteen hex digits.
    pub types: HashMap<u64, String>,
    /// Program names by 32-bit program hash, and module names by 64-bit module hash.
    pub programs: HashMap<u64, String>,
    /// The label of each names list read into `types` and `programs`, in the order read.
    pub lists: Vec<String>,
}

/// Why a names list was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NamesError {
    #[error("names list `{label}`, line {line}: {what}")]
    Line {
        label: String,
        line: usize,
        what: String,
    },
    #[error("names list `{label}` names nothing: it holds no `0xHASH name` line")]
    Empty { label: String },
}

impl Names {
    pub fn native(&self, hash: u64) -> Option<&NativeName> {
        self.natives.as_ref()?.name(hash)
    }

    pub fn type_name(&self, hash: u64) -> Option<&str> {
        self.types.get(&hash).map(String::as_str)
    }

    /// The name of a type the listing calls `type_` and sixteen hex digits.
    pub fn listed_type(&self, listed: &str) -> Option<&str> {
        self.type_name(hashed(listed.strip_prefix("type_")?)?)
    }

    pub fn program(&self, hash: u64) -> Option<&str> {
        self.programs.get(&hash).map(String::as_str)
    }

    /// The name of a module the listing calls `module_0x` and sixteen hex digits.
    pub fn listed_module(&self, listed: &str) -> Option<&str> {
        self.program(hashed(listed.strip_prefix("module_0x")?)?)
    }

    /// Reads a list of `0xHASH name` lines into the type and program names; returns how many hashes it newly named.
    // ⚠ A hash of eight digits or fewer is a program's. A longer one is a type's or a module's:
    // the two are one hash function apart only in case folding, so one line names both.
    // ⚠ On an error the lines before it stay read; a caller that goes on must start again.
    pub fn read_list(&mut self, label: &str, text: &str) -> Result<usize, NamesError> {
        let refused = |n: usize, what: String| NamesError::Line {
            label: label.to_owned(),
            line: n + 1,
            what,
        };
        let (mut rows, mut named) = (0usize, 0usize);
        let mut first_row = true;
        for (n, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            let fields: Vec<&str> = match raw.contains('\t') {
                true => raw.split('\t').map(str::trim).collect(),
                false => raw.split_whitespace().collect(),
            };
            let hashes: Vec<(u64, bool)> = fields.iter().map_while(|f| hash_of(f)).collect();
            let column_names = std::mem::replace(&mut first_row, false) && hashes.is_empty();
            if column_names {
                continue;
            }
            if hashes.is_empty() {
                return Err(refused(n, "the line starts with no `0x` hash".to_owned()));
            }
            // A table keeps a row for a hash nothing names.
            let name = fields.get(hashes.len()).copied().unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let spelled = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':');
            if !name.chars().all(spelled) {
                return Err(refused(n, format!("`{name}` is not a name")));
            }
            rows += 1;
            for (hash, wide) in hashes {
                let program = put(&mut self.programs, hash, name).map_err(|e| refused(n, e))?;
                let ty = wide && put(&mut self.types, hash, name).map_err(|e| refused(n, e))?;
                named += usize::from(program || ty);
            }
        }
        if rows == 0 {
            return Err(NamesError::Empty {
                label: label.to_owned(),
            });
        }
        self.lists.push(label.to_owned());
        Ok(named)
    }

    /// The list naming the most of `hashes` and how many it names, `None` when none names any; a tie goes to the list given first.
    pub fn best_natives<'a>(
        lists: &'a [NativeDb],
        hashes: &BTreeSet<u64>,
    ) -> Option<(&'a NativeDb, usize)> {
        let mut best: Option<(&NativeDb, usize)> = None;
        for db in lists {
            let named = hashes.iter().filter(|h| db.name(**h).is_some()).count();
            if named > best.map_or(0, |(_, most)| most) {
                best = Some((db, named));
            }
        }
        best
    }
}

/// The natives a listing declares by hash, each with whether the listing itself names it.
pub fn declared_natives(declarations: &Declarations) -> BTreeMap<u64, bool> {
    let mut natives = BTreeMap::new();
    for (_, wrapper) in declarations.iter() {
        if let Target::Call(Callee::Native { name, hash, .. }) = &wrapper.target {
            // A name that is only the hash spelled `_0x..` names nothing.
            natives.insert(*hash, !name.starts_with("_0x"));
        }
    }
    natives
}

/// Names `hash` in `map`; `false` when it already had this name, an error when it had another.
// A program's hash folds case, so two lists may spell one name in two cases: the first stays.
fn put(map: &mut HashMap<u64, String>, hash: u64, name: &str) -> Result<bool, String> {
    match map.get(&hash) {
        Some(held) if held.eq_ignore_ascii_case(name) => Ok(false),
        Some(held) => Err(format!(
            "0x{hash:X} is named `{name}` here and `{held}` before"
        )),
        None => {
            map.insert(hash, name.to_owned());
            Ok(true)
        }
    }
}

/// The hash a `0x` field spells, and whether it has more digits than a 32-bit hash.
fn hash_of(field: &str) -> Option<(u64, bool)> {
    let digits = field
        .strip_prefix("0x")
        .or_else(|| field.strip_prefix("0X"))?;
    let hex = (1..=16).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit());
    let hash = hex
        .then(|| u64::from_str_radix(digits, 16).ok())
        .flatten()?;
    Some((hash, digits.len() > 8))
}

/// The hash sixteen hex digits spell.
fn hashed(digits: &str) -> Option<u64> {
    let hex = digits.len() == 16 && digits.bytes().all(|b| b.is_ascii_hexdigit());
    hex.then(|| u64::from_str_radix(digits, 16).ok()).flatten()
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
