//! A compiled GTA VI script: the container a program is kept in, read to its tables.
//!
//! ⚠ The reader refuses. A count, an offset or a length the bytes do not bear out is an error,
//! and no table is sized from a number the file states before that number is held against the
//! file's own length.

#[path = "program_functions.rs"]
mod functions;
#[path = "program_read.rs"]
mod read;

use std::collections::BTreeMap;
use std::ops::Range;

use super::project::{ImportInfo, ProgramInfo};

/// The bytes of a string object before its text: ten words of 32 bits.
const STRING_HEAD: usize = 0x28;

/// Why bytes were refused as a compiled program.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    #[error("the program is {len} byte(s) long and {what} needs byte {needed}")]
    Truncated {
        what: &'static str,
        needed: usize,
        len: usize,
    },
    #[error("byte {at}: {what}")]
    Malformed { at: usize, what: String },
    #[error("code offset {at:06X}: {what}")]
    Code { at: u32, what: String },
}

/// A compiled program's tables, each as the container states it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The program's 32-bit hash.
    pub program: u32,
    /// The bytecode of every function as the file keeps it, operands enciphered; a function's offset is a byte of this.
    pub code: Vec<u8>,
    /// By rising offset; found in the code, since no table of the container lists them.
    pub functions: Vec<FunctionRow>,
    /// The program's own types, by the id the program gives each.
    pub types: Vec<TypeRow>,
    /// The programs imported; an instruction's import index `n` above 0 is `imports[n - 1]`, see [`Program::imported`].
    pub imports: Vec<ImportRow>,
    /// The native table, one row a native the code calls; the code names a native by address and never by a row.
    pub natives: Vec<NativeRow>,
    pub globals: Area,
    pub statics: Area,
    /// The pooled data that offsets of the code count into: switch tables and the string objects, see [`Program::string`].
    pub strings: Vec<u8>,
    /// The bytes of the constants array initialisers copy from, see [`Program::constant`].
    pub constants: Vec<u8>,
    /// Always empty: the container has no exception table.
    pub handlers: Vec<HandlerRow>,
    /// The functions the engine calls by the hash of a name.
    pub entries: Vec<EntryRow>,
    /// The offset of the function the header names as `main`.
    pub main: Option<u32>,
    /// The offset of the function the header names as `initialize`.
    pub initialize: Option<u32>,
    /// The method records, by rising owner; a type's own are `TypeRow::methods`.
    pub methods: Vec<MethodRow>,
    /// Each constant's bytes as a run of `constants`, by the index an array initialiser names it with.
    pub constant_rows: Vec<Range<u32>>,
    /// The map of the global cells that hold a reference, see [`Program::global_is_reference`]; empty where none is kept.
    pub global_references: Vec<u8>,
}

/// One function of the code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FunctionRow {
    /// The offset of its first instruction.
    pub at: u32,
    /// The offset one past its last.
    pub end: u32,
    /// The cells of its parameters.
    pub params: u32,
    /// The cells of its locals, the four every frame has among them.
    pub locals: u32,
    /// The cells it returns: those of its first return instruction, 0 where it has none.
    pub returns: u32,
}

/// One type of the type table; every id in it is one of this program's type ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeRow {
    /// The 64-bit hash of the type's name.
    pub name: u64,
    pub byte_size: u32,
    pub flags: u32,
    pub base: Option<u32>,
    /// What the type implements, or extends where it is an interface itself: its base list past its base type.
    pub interfaces: Vec<u32>,
    /// The offset of the function of this program that fills each virtual slot, `None` for any other slot; `vtable` says what that one holds.
    pub slots: Vec<Option<u32>>,
    pub fields: Vec<FieldRow>,
    /// The generic type this is an instance of, and its arguments.
    pub generic: Option<(u32, Vec<u32>)>,
    /// The 32-bit hash of the type's name.
    pub name32: u32,
    /// The virtual slot at which the methods of each of `interfaces` begin, one an interface.
    pub first_slots: Vec<u32>,
    /// Every virtual slot as the container states it.
    pub vtable: Vec<SlotRow>,
    /// The type's own method records, as a run of `Program::methods`.
    pub methods: Range<u32>,
    /// The values of an enum's members, each `byte_size` bytes wide and zero-extended.
    pub constants: Vec<u64>,
    /// The 32-bit hashes of an enum's member names, one a member; empty where the container keeps none.
    pub constant_names: Vec<u32>,
    /// The type parameters of a generic type itself, which is an instance of none; 0 for any other type.
    pub parameters: u32,
}

/// What fills one virtual slot of a type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlotRow {
    /// Nothing: the slot of an interface, or one the type leaves open.
    #[default]
    Empty,
    /// The function at code offset `at` of import `import`, 0 being the program itself.
    Function {
        import: u32,
        at: u32,
        /// Whether the receiver is a value the slot takes out of its box.
        unboxes: bool,
    },
    /// Method `method` of builtin type `ty`, which the VM implements.
    Builtin { ty: u32, method: u32, unboxes: bool },
}

/// One field a type's member table states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FieldRow {
    pub offset: u32,
    pub bytes: u32,
    /// The id of the field's type, where the table states one.
    pub ty: Option<u32>,
    /// The 64-bit hash of the field's name.
    pub name: u64,
    /// The 32-bit hash of the field's name.
    pub name32: u32,
}

/// One method record: what a type states of a method beyond its code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MethodRow {
    /// The 32-bit hash of the method's name, its case kept.
    pub name: u32,
    /// The offset of its function, `None` for a method with no code in this program.
    pub at: Option<u32>,
    pub flags: u32,
    /// The virtual slot it fills.
    pub slot: Option<u32>,
    /// The cells it returns.
    pub returns: u32,
    /// The cells of its arguments, the receiver among them.
    pub params: u32,
    /// The id of the type it returns, `None` where it returns nothing.
    pub result: Option<u32>,
    /// The id of the type it belongs to, one of the program's own.
    pub owner: u32,
}

/// One function the engine calls by name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryRow {
    /// The 32-bit hash of the name.
    pub name: u32,
    /// The offset of the function.
    pub at: u32,
}

/// What the container itself marks a function as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marked {
    /// The function the header names as `main`.
    Main,
    /// The function the header names as `initialize`.
    Initialize,
    /// A function the engine calls by the hash of a name; `name` is 0 where the entry states none.
    Entry { name: u32 },
}

/// One imported program, and the run of this program's type ids that are its types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRow {
    pub program: u32,
    pub types: Range<u32>,
}

/// One row of the native table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeRow {
    /// The word the table keeps for the native: enciphered for each program, and not the native's hash.
    pub hash: u64,
    /// Always 0: the table does not state them, a call instruction does.
    pub params: u32,
    /// Always 0, as `params`.
    pub returns: u32,
}

/// The globals of a program, or its statics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Area {
    /// The cells of the program's own block, a last part of a cell counted as one.
    pub cells: u32,
    /// The bytes the area starts as; always empty, since the container holds none.
    pub initial: Vec<u8>,
}

/// One row of the exception tables: the instructions protected and where a fault in them goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandlerRow {
    /// The offset of the first protected instruction.
    pub from: u32,
    /// The offset one past the last.
    pub to: u32,
    /// The offset of the handler's first instruction.
    pub handler: u32,
    /// The id of the type caught; `None` catches any fault.
    pub catches: Option<u32>,
    /// Whether the handler runs however the protected instructions are left.
    pub finally: bool,
}

impl Program {
    /// Reads a compiled program, refusing bytes that are not one.
    pub fn read(bytes: &[u8]) -> Result<Program, ProgramError> {
        read::read(bytes)
    }

    /// The function whose first instruction is at `at`.
    pub fn function(&self, at: u32) -> Option<&FunctionRow> {
        let found = self.functions.binary_search_by_key(&at, |row| row.at);
        found.ok().and_then(|n| self.functions.get(n))
    }

    /// The text of the string object at byte `offset` of the pooled data, `None` where what is there is not one.
    pub fn string(&self, offset: u32) -> Option<&str> {
        let (end, text) = self.string_bytes(offset)?.split_last()?;
        let ended = *end == 0 && !text.contains(&0);
        std::str::from_utf8(text).ok().filter(|_| ended)
    }

    /// The bytes the string object at `offset` states as its text, the zero that ends the text among them.
    // ⚠ Only the object of the empty string was measured, a head and one zero byte. Every word of
    // the head is held to what that object has, so an object laid out another way is no string here.
    fn string_bytes(&self, offset: u32) -> Option<&[u8]> {
        let object = self.strings.get(offset as usize..)?;
        let (head, text) = object.split_first_chunk::<STRING_HEAD>()?;
        let word = |n: usize| {
            let at = n * 4;
            u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]])
        };
        let words: [u32; STRING_HEAD / 4] = std::array::from_fn(word);
        // Three zeros, the bytes of the text stated twice, a zero, a 9 and three zeros.
        let [0, 0, 0, bytes, again, 0, 9, 0, 0, 0] = words else {
            return None;
        };
        text.get(..bytes as usize).filter(|_| bytes == again)
    }

    /// The string objects the code pushes, each text by the object's offset in the pooled data.
    fn pushed_strings(&self) -> BTreeMap<u32, String> {
        // ⚠ What is read is counted against the pooled data. Objects do not overlap, so their
        // bytes fit in it; a file that names one long object from many offsets is cut short.
        let mut left = self.strings.len();
        let mut strings = BTreeMap::new();
        // The code was walked whole when the program was read, so it walks again.
        for offset in functions::strings(&self.code).unwrap_or_default() {
            let Some(bytes) = self.string_bytes(offset) else {
                continue;
            };
            let Some(rest) = left.checked_sub(bytes.len()) else {
                break;
            };
            left = rest;
            if let Some(text) = self.string(offset) {
                strings.insert(offset, text.to_owned());
            }
        }
        strings
    }

    /// What the container marks the function at `at` as, `None` for a function it marks as nothing.
    pub fn mark(&self, at: u32) -> Option<Marked> {
        let entry = || self.entries.iter().find(|entry| entry.at == at);
        match Some(at) {
            named if named == self.main => Some(Marked::Main),
            named if named == self.initialize => Some(Marked::Initialize),
            _ => entry().map(|entry| Marked::Entry { name: entry.name }),
        }
    }

    /// The bytes of the constant an array initialiser names by `index`.
    pub fn constant(&self, index: u32) -> Option<&[u8]> {
        let row = self.constant_rows.get(index as usize)?;
        self.constants.get(row.start as usize..row.end as usize)
    }

    /// The program an instruction's import index names; 0 is this program.
    pub fn imported(&self, index: u32) -> Option<u32> {
        match index.checked_sub(1) {
            None => Some(self.program),
            Some(n) => self.imports.get(n as usize).map(|import| import.program),
        }
    }

    /// Whether cell `cell` of the global block, this program's own cells first, holds a reference.
    pub fn global_is_reference(&self, cell: u32) -> bool {
        // A bit a cell, the lowest bit of a byte first.
        let byte = self.global_references.get((cell / 8) as usize);
        byte.is_some_and(|byte| (byte >> (cell % 8)) & 1 != 0)
    }

    /// Each function of another program this program's code calls or refers to, by that program's hash and the function's offset.
    pub fn called_elsewhere(&self) -> Vec<(u32, u32)> {
        // The code was walked whole when the program was read, so it walks again.
        let found = functions::foreign(&self.code).unwrap_or_default();
        let named = found.into_iter();
        named
            .filter_map(|(import, at)| Some((self.imported(import)?, at)))
            .collect()
    }

    /// Starts a function at `at`, as a call from another program proves one does where nothing of this program shows it.
    pub fn add_function(&mut self, at: u32) -> Result<(), ProgramError> {
        functions::split(&self.code, &mut self.functions, at)
    }

    /// What a project keeps of the program before any function is read: its hash, its imports and the strings its code pushes.
    pub fn info(&self) -> ProgramInfo {
        let imports = self.imports.iter().map(|import| ImportInfo {
            program: import.program,
            first: import.types.start,
            end: import.types.end,
        });
        ProgramInfo {
            program: self.program,
            imports: imports.collect(),
            strings: self.pushed_strings(),
            ..ProgramInfo::default()
        }
    }
}

#[cfg(test)]
#[path = "program_tests.rs"]
mod tests;
