//! The names a decompile may use beyond the program's own: a natives list, and lists of programs,
//! types, methods, fields, builtins and the text of hash constants, all supplied by the user.
//! OpenRage ships none, so every lookup may come back empty.
//!
//! A names list is text, one `0xHASH name` per line. A tab-separated table whose leading columns
//! are hashes and whose next column is the name reads the same way, under one row of column names.
//! A row whose hashes are the hashes of its name proves itself, and names what that hash function
//! names ([`Proof`]); a row none of whose hashes is one is the user's own label for a program or
//! a type.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::Hash;

use super::ir::{Callee, FunctionKey, TypeKey};
use super::listing::{Declarations, Target};
use crate::gta5::script::natives::{NativeDb, NativeName};
use crate::gta6::hash::literal_hash64 as murmur_low;
use crate::hash::literal_string_hash;

/// The hash function by which a row's hash is the hash of its name, which says what the row names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Proof {
    /// Jenkins' one-at-a-time hash of the name in lower case: a program, and the text of a 32-bit hash constant.
    Folded32,
    /// The same hash of the name as it is spelled: a method.
    Kept32,
    /// The low half of MurmurHash3 x64-128 of the name as it is spelled: a type, and a field.
    Kept64,
    /// The same hash of the name in lower case: a module, and the text of a 64-bit hash constant.
    Folded64,
}

/// What the passes and the printer consult for a name.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// The user's natives list, keyed by the 64-bit hash a native is called by.
    pub natives: Option<NativeDb>,
    /// Type names by the 64-bit hash a listing spells as `type_` and sixteen hex digits, and builtin type names by `0x80000000` and the index.
    pub types: HashMap<u64, String>,
    /// Program names by 32-bit program hash, and module names by 64-bit module hash.
    pub programs: HashMap<u64, String>,
    /// The label of each names list read, in the order read.
    pub lists: Vec<String>,
    /// Field names by the type that declares the field and the field's byte offset.
    pub fields: HashMap<(TypeKey, u32), String>,
    /// Method names by the type that declares the virtual slot and the slot.
    pub methods: HashMap<(TypeKey, u32), String>,
    /// Function names by program hash and offset.
    pub functions: HashMap<FunctionKey, String>,
    /// The text a 32-bit hash constant is the hash of, from the user's vocabulary.
    pub texts: HashMap<u32, String>,
    /// The text a 64-bit hash constant is the hash of.
    pub texts64: HashMap<u64, String>,
    /// Method names by the hash of the name that a program's method record holds.
    pub method_names: HashMap<u32, String>,
    /// Field names by the hash of the name that a program's field record holds.
    pub field_names: HashMap<u64, String>,
    /// The argument cells of natives that hold name hashes: a native's hash and the index of the cell among its arguments.
    pub hash_slots: BTreeSet<(u64, u32)>,
    /// The 64-bit hash of the native whose handler is at an address of the executable: a compiled program calls by the address alone.
    pub native_hashes: HashMap<u32, u64>,
    /// The runtime's own methods as dotted paths, by builtin type index and method index.
    pub builtins: HashMap<(u32, u32), String>,
    /// For each list of `lists`, the rows that proved their own hashes and the rows taken on trust.
    pub rows: Vec<(usize, usize)>,
    /// The hashes two rows proved different names for: such a hash names nothing.
    pub ambiguous: HashSet<(Proof, u64)>,
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

/// What the rows of a list are, which its row of column names says.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Table {
    /// Hashes, then the name they are the hashes of or the user's label for them.
    Names,
    /// A column named `text`: hashes, then the text they are the hashes of, which names no program, type or member.
    // ⚠ A vocabulary is large, and one name in thousands of a large one has the 32-bit hash of
    // some program or method by accident: under this column a row is the text of a constant only.
    Texts,
    /// `native_hash64`, `cell`: a native's hash, the index of one of its argument cells, and `Hash`.
    HashSlots,
    /// `owner_type_hash64`, `offset`: a type's hash, a byte offset as `0x` hex, and the name of the field there.
    Fields,
    /// `owner_type_hash64`, `slot`: a type's hash, a virtual slot, and the name of the method there; what a listing, which states no method record, is named by.
    Slots,
    /// `type_index`, `method_index`: a builtin type's index, a method's index, and the method's dotted path.
    Builtins,
    /// `index`, `name`: a builtin type's index and its name.
    BuiltinTypes,
    /// `hash`, `rva`: a native's hash and the address of its handler in the executable the scripts were built for.
    NativeAddresses,
}

impl Table {
    /// The table a first row of column names opens; `None` where the row is the first of the names.
    fn of(fields: &[&str]) -> Result<Option<Table>, String> {
        let column = |n: usize| fields.get(n).copied().unwrap_or("");
        Ok(Some(match (column(0), column(1)) {
            ("native_hash64", "cell") => Table::HashSlots,
            ("native_hash64", _) => {
                return Err(
                    "a table of native names is a natives list, not a names list".to_owned(),
                )
            }
            ("owner_type_hash64", "offset") => Table::Fields,
            ("owner_type_hash64", "slot") => Table::Slots,
            ("type_index", "method_index") => Table::Builtins,
            ("index", "name") => Table::BuiltinTypes,
            ("hash", "rva") => Table::NativeAddresses,
            (first, _) if hash_of(first).is_some() => return Ok(None),
            _ if fields.contains(&"text") => Table::Texts,
            _ => Table::Names,
        }))
    }
}

/// How one row was taken, with the count of what it newly named.
enum Taken {
    Nothing,
    Proved(usize),
    Trusted(usize),
}

impl Names {
    pub fn native(&self, hash: u64) -> Option<&NativeName> {
        self.natives.as_ref()?.name(hash)
    }

    pub fn type_name(&self, hash: u64) -> Option<&str> {
        self.types.get(&hash).map(String::as_str)
    }

    /// The name of a type the listing calls `type_` and sixteen hex digits, or `type_` and a builtin's eight.
    pub fn listed_type(&self, listed: &str) -> Option<&str> {
        self.type_name(TypeKey::of_name(listed)?.0)
    }

    pub fn program(&self, hash: u64) -> Option<&str> {
        self.programs.get(&hash).map(String::as_str)
    }

    /// The name of a module the listing calls `module_0x` and sixteen hex digits.
    pub fn listed_module(&self, listed: &str) -> Option<&str> {
        self.program(hashed(listed.strip_prefix("module_0x")?)?)
    }

    /// The name of the field at byte `offset` that the type `owner` declares.
    pub fn field(&self, owner: TypeKey, offset: u32) -> Option<&str> {
        self.fields.get(&(owner, offset)).map(String::as_str)
    }

    /// The name of the method in virtual slot `slot` that the type `owner` declares.
    pub fn method(&self, owner: TypeKey, slot: u32) -> Option<&str> {
        self.methods.get(&(owner, slot)).map(String::as_str)
    }

    pub fn function(&self, function: FunctionKey) -> Option<&str> {
        self.functions.get(&function).map(String::as_str)
    }

    /// The one text `hash` is the hash of in the user's vocabulary; `None` for a number too small, or too near -1, to be a name's hash.
    // ⚠ A text is half of the rule. One 32-bit value in 3,800 is the hash of some name of a list
    // of a million, so a constant shows as its text only where the names pass left it the kind
    // `Hash`: in a place the vocabulary is known to cover.
    pub fn text(&self, hash: u32) -> Option<&str> {
        let large = (0x1_0000..0xFFFF_0000).contains(&hash);
        self.texts.get(&hash).filter(|_| large).map(String::as_str)
    }

    /// The one text the 64-bit constant `hash` is the hash of; `None` for a value that fits 32 bits.
    pub fn text64(&self, hash: u64) -> Option<&str> {
        let wide = !matches!(hash >> 32, 0 | 0xFFFF_FFFF);
        self.texts64.get(&hash).filter(|_| wide).map(String::as_str)
    }

    /// The hash of the native whose handler is at `rva`, where a supplied list states it.
    pub fn native_at(&self, rva: u32) -> Option<u64> {
        self.native_hashes.get(&rva).copied()
    }

    /// The dotted path of method `method` of the runtime's builtin type `ty`.
    pub fn builtin(&self, ty: u32, method: u32) -> Option<&str> {
        self.builtins.get(&(ty, method)).map(String::as_str)
    }

    /// Names virtual slot `slot` of `owner` as the method record stating it does, by the record's hash of the name; `false` where no list names that hash.
    // A name that is no identifier, `.ctor` among them, is no name a call is written with.
    pub fn bind_method(&mut self, name_hash: u32, owner: TypeKey, slot: u32) -> bool {
        let name = self.method_names.get(&name_hash).filter(|n| identifier(n));
        let Some(name) = name.cloned() else {
            return false;
        };
        self.methods.entry((owner, slot)).or_insert(name);
        true
    }

    /// Names the field at byte `offset` of `owner` as the field record stating it does, by the record's hash of the name; `false` where no list names that hash.
    pub fn bind_field(&mut self, name_hash: u64, owner: TypeKey, offset: u32) -> bool {
        let name = self.field_names.get(&name_hash).filter(|n| identifier(n));
        let Some(name) = name.cloned() else {
            return false;
        };
        self.fields.entry((owner, offset)).or_insert(name);
        true
    }

    /// Each list read as a header names it: its label, with the rows that proved their own hashes and the rows taken on trust.
    pub fn sources(&self) -> Vec<String> {
        let rows = self.rows.iter().map(Some).chain(std::iter::repeat(None));
        let listed = self.lists.iter().zip(rows);
        listed
            .map(|(label, rows)| match rows {
                Some((proved, trusted)) => {
                    format!("`{label}` ({proved} row(s) proved by their hash, {trusted} on trust)")
                }
                None => format!("`{label}`"),
            })
            .collect()
    }

    /// Reads a names list; returns how many hashes, cells and indices it newly named.
    // ⚠ A proved row names by the function its hash is of the name by, and a short hash and a long
    // one are never the same function. A row taken on trust names a program by a hash of eight
    // digits or fewer, and both a type and a module by a longer one.
    // ⚠ On an error the lines before it stay read; a caller that goes on must start again.
    pub fn read_list(&mut self, label: &str, text: &str) -> Result<usize, NamesError> {
        let refused = |n: usize, what: String| NamesError::Line {
            label: label.to_owned(),
            line: n + 1,
            what,
        };
        let (mut proved, mut trusted, mut named) = (0usize, 0usize, 0usize);
        let mut table = Table::Names;
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
            if std::mem::replace(&mut first_row, false) {
                if let Some(columns) = Table::of(&fields).map_err(|what| refused(n, what))? {
                    table = columns;
                    continue;
                }
            }
            match self.row(table, &fields).map_err(|what| refused(n, what))? {
                Taken::Nothing => {}
                Taken::Proved(new) => {
                    proved += 1;
                    named += new;
                }
                Taken::Trusted(new) => {
                    trusted += 1;
                    named += new;
                }
            }
        }
        if proved + trusted == 0 {
            return Err(NamesError::Empty {
                label: label.to_owned(),
            });
        }
        self.lists.push(label.to_owned());
        self.rows.push((proved, trusted));
        Ok(named)
    }

    /// Takes one row of a table.
    fn row(&mut self, table: Table, fields: &[&str]) -> Result<Taken, String> {
        let field = |n: usize| fields.get(n).copied().unwrap_or("");
        let hash = |n: usize| hash_of(field(n)).map(|(hash, _)| hash);
        let index = |n: usize| decimal(field(n));
        let shape = || Err("the row does not start as its column names say".to_owned());
        let again = |name: &str, held: String| format!("`{name}` here was `{held}` before");
        let new = match table {
            Table::Names => return self.names_row(fields, false),
            Table::Texts => return self.names_row(fields, true),
            Table::HashSlots => {
                let (Some(native), Some(cell)) = (hash(0), index(1)) else {
                    return shape();
                };
                if field(2) != "Hash" {
                    return Err(format!("`{}` is no kind of cell a list states", field(2)));
                }
                self.hash_slots.insert((native, cell))
            }
            Table::Fields => {
                let offset = hash(1).and_then(|offset| u32::try_from(offset).ok());
                let (Some(owner), Some(offset), name) = (hash(0), offset, field(2)) else {
                    return shape();
                };
                if name.is_empty() {
                    return Ok(Taken::Nothing);
                }
                if !identifier(name) {
                    return Err(format!("`{name}` is not a name"));
                }
                let key = (TypeKey(owner), offset);
                put(&mut self.fields, key, name).map_err(|held| again(name, held))?
            }
            Table::Slots => {
                let (Some(owner), Some(slot), name) = (hash(0), index(1), field(2)) else {
                    return shape();
                };
                if name.is_empty() {
                    return Ok(Taken::Nothing);
                }
                if !identifier(name) {
                    return Err(format!("`{name}` is not a name"));
                }
                let key = (TypeKey(owner), slot);
                put(&mut self.methods, key, name).map_err(|held| again(name, held))?
            }
            Table::Builtins => {
                let (Some(ty), Some(method), path) = (index(0), index(1), field(2)) else {
                    return shape();
                };
                if path.is_empty() {
                    return Ok(Taken::Nothing);
                }
                if !path.chars().all(spelled) {
                    return Err(format!("`{path}` is not a name"));
                }
                put(&mut self.builtins, (ty, method), path).map_err(|held| again(path, held))?
            }
            Table::NativeAddresses => {
                let rva = hash(1).and_then(|rva| u32::try_from(rva).ok());
                let (Some(native), Some(rva)) = (hash(0), rva) else {
                    return shape();
                };
                match self.native_hashes.insert(rva, native) {
                    Some(held) if held != native => {
                        return Err(format!(
                            "the handler at 0x{rva:08X} is of 0x{native:016X} here and of 0x{held:016X} before"
                        ));
                    }
                    held => held.is_none(),
                }
            }
            Table::BuiltinTypes => {
                // A builtin type's key is its index under the top bit of 32.
                let builtin = index(0).filter(|n| *n < 0x8000_0000);
                let (Some(builtin), name) = (builtin, field(1)) else {
                    return shape();
                };
                if name.is_empty() {
                    return Ok(Taken::Nothing);
                }
                if !name.chars().all(spelled) {
                    return Err(format!("`{name}` is not a name"));
                }
                let key = u64::from(0x8000_0000 | builtin);
                put(&mut self.types, key, name).map_err(|held| again(name, held))?
            }
        };
        Ok(Taken::Trusted(usize::from(new)))
    }

    /// Takes one row of hashes and a name: proved where every hash is a hash of the name, on trust where none is and the row is no text's.
    fn names_row(&mut self, fields: &[&str], text: bool) -> Result<Taken, String> {
        let hashes: Vec<(u64, bool)> = fields.iter().map_while(|f| hash_of(f)).collect();
        if hashes.is_empty() {
            return Err("the line starts with no `0x` hash".to_owned());
        }
        // A table keeps a row for a hash nothing names.
        let name = fields.get(hashes.len()).copied().unwrap_or("");
        if name.is_empty() {
            return Ok(Taken::Nothing);
        }
        let by: Vec<Vec<Proof>> = (hashes.iter())
            .map(|(hash, wide)| proofs(*hash, *wide, name, text))
            .collect();
        let passing = by.iter().filter(|proofs| !proofs.is_empty()).count();
        let mut named = 0;
        if passing == 0 {
            if text {
                return Err(format!("`{name}` is not the text the row's hash is of"));
            }
            if !name.chars().all(spelled) {
                return Err(format!("`{name}` is not a name"));
            }
            for (hash, wide) in hashes {
                let again = |held: String| twice(hash, name, &held);
                let program = put(&mut self.programs, hash, name).map_err(again)?;
                let ty = wide && put(&mut self.types, hash, name).map_err(again)?;
                named += usize::from(program || ty);
            }
            return Ok(Taken::Trusted(named));
        }
        // One of its columns is wrong, and which one no reader can say.
        if passing < hashes.len() {
            return Err(format!(
                "`{name}` has the hash of some of the row's hash columns and not of the others"
            ));
        }
        for ((hash, _), proofs) in hashes.iter().zip(by) {
            let mut new = false;
            for proof in proofs {
                new |= self.prove(proof, *hash, name, text)?;
            }
            named += usize::from(new);
        }
        Ok(Taken::Proved(named))
    }

    /// Takes `name` as what `proof` proves `hash` the hash of, as a text alone where `text`; `false` where that names nothing new.
    // ⚠ Two different names proved for one hash name nothing: the hash is of both, and no list
    // says which the program meant. A list of a million names holds about a hundred such pairs,
    // so this is no error. A label another row gave the hash on trust is one.
    fn prove(&mut self, proof: Proof, hash: u64, name: &str, text: bool) -> Result<bool, String> {
        if self.ambiguous.contains(&(proof, hash)) {
            return Ok(false);
        }
        let low = hash as u32;
        let folded = matches!(proof, Proof::Folded32 | Proof::Folded64);
        let took = match proof {
            Proof::Folded32 => take(&mut self.texts, low, name, folded),
            Proof::Kept32 => take(&mut self.method_names, low, name, folded),
            Proof::Kept64 => take(&mut self.field_names, hash, name, folded),
            Proof::Folded64 => take(&mut self.texts64, hash, name, folded),
        };
        // The names a program, a module and a type print under in code, which a label may hold too.
        let shared = match proof {
            _ if text => None,
            Proof::Folded32 | Proof::Folded64 => Some(&mut self.programs),
            Proof::Kept64 => Some(&mut self.types),
            Proof::Kept32 => None,
        };
        match took {
            Some(false) => Ok(false),
            Some(true) => {
                if let Some(shared) = shared.filter(|_| name.chars().all(spelled)) {
                    put(shared, hash, name).map_err(|held| twice(hash, name, &held))?;
                }
                Ok(true)
            }
            None => {
                if let Some(shared) = shared {
                    shared.remove(&hash);
                }
                self.ambiguous.insert((proof, hash));
                Ok(false)
            }
        }
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

/// Names `key` in `map`; `false` when it already had this name, and the name it had when it had another.
// A program's hash folds case, so two lists may spell one name in two cases: the first stays.
fn put<K: Eq + Hash>(map: &mut HashMap<K, String>, key: K, name: &str) -> Result<bool, String> {
    match map.get(&key) {
        Some(held) if held.eq_ignore_ascii_case(name) => Ok(false),
        Some(held) => Err(held.clone()),
        None => {
            map.insert(key, name.to_owned());
            Ok(true)
        }
    }
}

/// Enters `name` under `key` in a map only proved rows fill: whether it is new there, and `None` where another name was, which is taken out.
fn take<K: Eq + Hash>(
    map: &mut HashMap<K, String>,
    key: K,
    name: &str,
    folded: bool,
) -> Option<bool> {
    match map.get(&key) {
        Some(held) if held == name || (folded && held.eq_ignore_ascii_case(name)) => Some(false),
        Some(_) => {
            map.remove(&key);
            None
        }
        None => {
            map.insert(key, name.to_owned());
            Some(true)
        }
    }
}

/// What a hash that two rows label differently is refused with.
fn twice(hash: u64, name: &str, held: &str) -> String {
    format!("0x{hash:X} is named `{name}` here and `{held}` before")
}

/// The functions by which `hash` is the hash of `name`: of the two 32-bit ones for a short hash, of the two 64-bit ones for a long one; of the two that fold case alone where `text`.
fn proofs(hash: u64, wide: bool, name: &str, text: bool) -> Vec<Proof> {
    let lower = name.to_ascii_lowercase();
    let (kept, folded) = (name.as_bytes(), lower.as_bytes());
    let of = |proof: Proof| match proof {
        Proof::Folded32 => u64::from(literal_string_hash(folded)),
        Proof::Kept32 => u64::from(literal_string_hash(kept)),
        Proof::Kept64 => murmur_low(kept),
        Proof::Folded64 => murmur_low(folded),
    };
    let tried = match wide {
        true => [Proof::Folded64, Proof::Kept64],
        false => [Proof::Folded32, Proof::Kept32],
    };
    // The function that folds case is the first of each pair.
    let tried = &tried[..if text { 1 } else { 2 }];
    let passed = tried.iter().copied().filter(|proof| of(*proof) == hash);
    passed.collect()
}

/// Whether `c` may stand in a name written in code on trust: a dotted or scoped path's characters.
fn spelled(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':')
}

/// Whether `name` is one identifier, which a member is written as.
fn identifier(name: &str) -> bool {
    let starts = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    starts && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
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

/// The index decimal digits spell.
fn decimal(digits: &str) -> Option<u32> {
    let all = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    all.then(|| digits.parse().ok()).flatten()
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
