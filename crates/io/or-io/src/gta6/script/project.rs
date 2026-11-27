//! Facts gathered over every program of an install before any one of them is decompiled.
//!
//! [`index`] reads each program to a part, parts merge to one [`Project`], and each table is a
//! plain map in a fixed order, so that a project, the file it is kept in and all output made with
//! it are the same from run to run. [`Project::empty`] holds nothing, and a program decompiles
//! alone with it.
//!
//! ⚠ A project holds what programs state and what their code shows, never a name: names are the
//! user's, in [`super::Names`].

pub mod index;
pub mod layouts;
pub mod reach;
pub mod signatures;
pub mod types;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::cfg::LowerError;
use super::class::Role;
use super::ir::{FunctionKey, Kind, TypeKey};
use super::lift::LiftError;
use super::listing::ListingError;
use super::program::ProgramError;

/// The base types a lookup follows before it gives up: no hierarchy is that deep, and a ring of bases must end.
const MAX_BASES: usize = 64;

/// Why a project was not built, or not read back.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(transparent)]
    Listing(#[from] ListingError),
    #[error(transparent)]
    Program(#[from] ProgramError),
    #[error(transparent)]
    Lift(#[from] LiftError),
    #[error("{function}: {source}")]
    Lower {
        function: String,
        source: LowerError,
    },
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    /// A file that was read and refused for what it holds.
    #[error("{path}: {source}")]
    File {
        path: String,
        source: Box<ProjectError>,
    },
    #[error("the project file is refused: {0}")]
    Sidecar(String),
}

/// What one program states of itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramInfo {
    /// The program's 32-bit hash.
    pub program: u32,
    pub imports: Vec<ImportInfo>,
    /// The program's own types, by the id the program gives each.
    pub types: BTreeMap<u32, TypeKey>,
    /// What the global at each byte offset holds.
    pub globals: BTreeMap<u32, Kind>,
    /// What the static at each byte offset holds.
    pub statics: BTreeMap<u32, Kind>,
    /// The strings of the program's heap, by offset.
    pub strings: BTreeMap<u32, String>,
    /// What the classes pass holds of the program's code until every program is in; empty in a settled project.
    #[serde(skip)]
    pub carried: super::class::Carried,
    /// The program's globals as code reads and writes them, by rising offset: one cell an offset, of the width every access of it takes and the kind every access agrees on; two overlap where a value is accessed whole and by its parts.
    #[serde(default)]
    pub global_cells: Vec<FieldFacts>,
    /// The program's statics, as `global_cells` holds its globals.
    #[serde(default)]
    pub static_cells: Vec<FieldFacts>,
}

/// A program another imports, and the run of the importer's type ids that are its types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportInfo {
    pub program: u32,
    pub first: u32,
    pub end: u32,
}

/// What is known of one type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeFacts {
    pub byte_size: u32,
    pub flags: u32,
    /// The program that defines the type.
    pub home: u32,
    pub base: Option<TypeKey>,
    pub interfaces: Vec<TypeKey>,
    /// The function each virtual slot resolves to on this type, where the type itself fills the slot.
    pub slots: BTreeMap<u32, FunctionKey>,
    /// The generic type this is an instance of, and its arguments.
    pub generic: Option<(TypeKey, Vec<TypeKey>)>,
    /// The values the constants of an enum take.
    pub constants: Vec<i64>,
    /// The cells a call of a virtual slot returns: those the type's one method record for the slot states, else, for an interface, those every function a type fills the slot with returns.
    pub returns: BTreeMap<u32, u32>,
    /// The type parameters of a generic type itself, which is an instance of none; 0 for any other type.
    #[serde(default)]
    pub parameters: u32,
    /// The 32-bit hashes of the names of an enum's constants, one a constant; empty where the program keeps none.
    #[serde(default)]
    pub constant_names: Vec<u32>,
}

impl TypeFacts {
    /// Whether a value of the type is held in place, as an array element or a field is, and never made as an object.
    pub fn is_value(&self) -> bool {
        self.flags & 0x1C != 0
    }
}

/// What is known of one field of a type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldFacts {
    /// The byte offset from the object's start; an object's first field is at 8.
    pub offset: u32,
    pub bytes: u32,
    pub kind: Kind,
}

/// One parameter of a function: what it is and the cells it takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub kind: Kind,
    pub cells: u32,
}

/// What is known of one function across the calls of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// One entry a parameter, in order; their cells add up to the function's parameter cells.
    pub params: Vec<Param>,
    /// The cells the function returns.
    pub returns: u32,
    /// What the returned cells are.
    pub result: Kind,
}

/// The type a function is proved to belong to, and as what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub owner: TypeKey,
    pub role: Role,
    /// Whether the function takes no receiver.
    pub is_static: bool,
    /// The virtual slot of `owner` the function fills.
    pub slot: Option<u32>,
}

/// The facts of an install, each table in key order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub programs: BTreeMap<u32, ProgramInfo>,
    pub types: BTreeMap<TypeKey, TypeFacts>,
    /// A type's own fields, by rising offset; a base type's are under the base.
    pub fields: BTreeMap<TypeKey, Vec<FieldFacts>>,
    pub signatures: BTreeMap<FunctionKey, Signature>,
    /// The call graph: the functions each function calls, each once, in key order.
    pub calls: BTreeMap<FunctionKey, Vec<FunctionKey>>,
    pub members: BTreeMap<FunctionKey, Member>,
    /// The kind of the one cell a call through a virtual slot yields, by each function the slot holds, where every function the slot holds down its line proves that kind.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slot_results: BTreeMap<FunctionKey, Kind>,
    /// The parameters a call through a virtual slot passes, by each function the slot holds: each of the kind every function of its line that mentions it states, else `Unknown`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub slot_params: BTreeMap<FunctionKey, Vec<Param>>,
    /// The natives every read of whose result, over every program, is only ever whether it is zero, by `signatures::native_key`.
    // ⚠ How the code reads a result, not what the native returns: a reading of a test of one as
    // the truth it is holds only where only the truth is read.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub native_truths: BTreeSet<u64>,
    /// How each native's result is read so far, by key: `true` while every read is a test; empty in a settled project.
    #[serde(skip)]
    pub native_uses: BTreeMap<u64, bool>,
    /// The parameter cells of each function every call of which passes an `int` widened by its sign to its cell.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub widened: BTreeMap<FunctionKey, BTreeSet<u32>>,
    /// What each parameter cell of a function reaches as an address, by function and cell.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reach: BTreeMap<FunctionKey, BTreeMap<u32, reach::Reach>>,
    /// The functions the types that implement an interface fill each of its slots with; empty once a project is settled, when `TypeFacts::returns` holds what they say.
    #[serde(skip)]
    pub implemented: BTreeMap<TypeKey, BTreeMap<u32, BTreeSet<FunctionKey>>>,
    /// What the accesses of globals and statics show until every program is in; empty in a settled project, when `ProgramInfo::global_cells` holds what they say.
    #[serde(skip)]
    pub cells: index::Cells,
    /// Whether every access seen of the one byte at an offset of a type is of a truth, by the type the access went through and the offset; empty in a settled project, when a field's kind says it.
    #[serde(skip)]
    pub truths: BTreeMap<(TypeKey, u32), bool>,
    /// The rows and the truths the fields were last settled from: `layouts::refine` settles them again with more.
    #[serde(skip)]
    pub rows: layouts::Rows,
    /// The functions code no part read may call or refer to, and a program's `signatures::ANY` where it may call any of that program's; empty in a settled project.
    #[serde(skip)]
    pub unseen: BTreeSet<FunctionKey>,
    /// The slots code no part read calls through or takes a delegate of, and `signatures::ANY` where that code may call through any; empty in a settled project.
    #[serde(skip)]
    pub unseen_slots: BTreeSet<u32>,
    /// Whether code no part read may call a native that no table says; false in a settled project.
    #[serde(skip)]
    pub blind_natives: bool,
}

static EMPTY: LazyLock<Project> = LazyLock::new(Project::default);

impl Project {
    /// The project that holds nothing: what a program decompiled alone is given.
    pub fn empty() -> &'static Project {
        &EMPTY
    }

    /// The type a program means by one of its type ids: its own, an import's through the import's range, or a builtin.
    pub fn type_key(&self, program: u32, id: u32) -> Option<TypeKey> {
        if id & 0x8000_0000 != 0 {
            return Some(TypeKey(u64::from(id)));
        }
        let info = self.programs.get(&program)?;
        let imported = info.imports.iter().find(|i| (i.first..i.end).contains(&id));
        match imported {
            Some(import) => {
                let home = self.programs.get(&import.program)?;
                home.types.get(&(id - import.first)).copied()
            }
            None => info.types.get(&id).copied(),
        }
    }

    /// The field at byte `offset` of an object of type `ty`, its own or a base type's.
    pub fn field(&self, ty: TypeKey, offset: u32) -> Option<&FieldFacts> {
        let mut ty = ty;
        for _ in 0..MAX_BASES {
            let own = self.fields.get(&ty);
            let field = own.and_then(|fields| fields.iter().find(|f| f.offset == offset));
            if field.is_some() {
                return field;
            }
            ty = self.types.get(&ty)?.base?;
        }
        None
    }

    /// The function a virtual slot resolves to on type `ty`: the type's own, else the nearest base type's.
    pub fn slot(&self, ty: TypeKey, slot: u32) -> Option<FunctionKey> {
        let mut ty = ty;
        for _ in 0..MAX_BASES {
            let facts = self.types.get(&ty)?;
            if let Some(function) = facts.slots.get(&slot) {
                return Some(*function);
            }
            ty = facts.base?;
        }
        None
    }

    /// The cells a call of virtual slot `slot` of type `ty` returns: by the nearest method record of its line, else by the function the slot resolves to.
    pub fn slot_returns(&self, ty: TypeKey, slot: u32) -> Option<u32> {
        let mut at = ty;
        for _ in 0..MAX_BASES {
            let Some(facts) = self.types.get(&at) else {
                break;
            };
            if let Some(cells) = facts.returns.get(&slot) {
                return Some(*cells);
            }
            let Some(base) = facts.base else {
                break;
            };
            at = base;
        }
        let function = self.slot(ty, slot)?;
        self.signatures.get(&function).map(|held| held.returns)
    }

    /// The kind of the one cell a call of virtual slot `slot` on an object of type `ty` yields, where every function the call may reach proves it.
    pub fn slot_result(&self, ty: TypeKey, slot: u32) -> Option<&Kind> {
        self.slot_results.get(&self.slot(ty, slot)?)
    }

    /// The parameters a call of virtual slot `slot` on an object of type `ty` passes, as every function the call may reach that mentions each agrees.
    pub fn slot_parameters(&self, ty: TypeKey, slot: u32) -> Option<&[Param]> {
        let params = self.slot_params.get(&self.slot(ty, slot)?)?;
        Some(params.as_slice())
    }

    /// Whether `ty` is `ancestor`, derives from it or implements it.
    pub fn is_a(&self, ty: TypeKey, ancestor: TypeKey) -> bool {
        let mut ty = ty;
        for _ in 0..MAX_BASES {
            if ty == ancestor {
                return true;
            }
            let Some(facts) = self.types.get(&ty) else {
                return false;
            };
            if facts.interfaces.contains(&ancestor) {
                return true;
            }
            match facts.base {
                Some(base) => ty = base,
                None => return false,
            }
        }
        false
    }

    /// The global of `program` at byte `offset`, or its static there, as every access of it in the project agrees it is.
    pub fn cell(&self, program: u32, offset: u32, statics: bool) -> Option<&FieldFacts> {
        let info = self.programs.get(&program)?;
        let cells = match statics {
            true => &info.static_cells,
            false => &info.global_cells,
        };
        let at = cells.binary_search_by_key(&offset, |cell| cell.offset);
        at.ok().and_then(|at| cells.get(at))
    }
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
