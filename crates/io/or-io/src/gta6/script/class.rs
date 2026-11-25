//! The class model: a program's functions under the types they are proved to belong to, which
//! is what the output is written from.
//!
//! ⚠ A function belongs to a type only by proof: a virtual slot the type fills, a constructor
//! run on an object just made, a closure a delegate binds. Every other function stays free, and
//! prints as a function of its program.

use serde::{Deserialize, Serialize};

use super::ir::{Kind, TypeKey};
use super::listing::Import;
use super::tree;

/// One program as the printer takes it: its types with their members, and the functions no type claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// The program's 32-bit hash.
    pub program: u32,
    pub imports: Vec<Import>,
    /// In the order of their keys.
    pub classes: Vec<Class>,
    /// The free functions, in the order of their offsets.
    pub functions: Vec<Method>,
    /// What the unit was read from, in the words its file's header says it in.
    pub front: Option<&'static str>,
}

impl Unit {
    /// A program none of whose functions is claimed by a type.
    pub fn flat(program: u32, imports: Vec<Import>, functions: Vec<tree::Function>) -> Unit {
        Unit {
            program,
            imports,
            classes: Vec::new(),
            functions: functions.into_iter().map(Method::free).collect(),
            front: None,
        }
    }
}

/// What a type is in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassKind {
    Class,
    Struct,
    Interface,
    Enum,
    Delegate,
    /// The class a compiler makes of the variables a lambda captures.
    Closure,
    /// The class a compiler makes of an `async` method or an iterator.
    StateMachine,
}

/// One type with what is proved to be its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub key: TypeKey,
    pub kind: ClassKind,
    pub base: Option<TypeKey>,
    pub interfaces: Vec<TypeKey>,
    /// By rising offset.
    pub fields: Vec<Field>,
    /// In the order of their offsets.
    pub methods: Vec<Method>,
    /// The closure and state-machine classes of its methods.
    pub nested: Vec<Class>,
}

/// One field of a class; its name is the user's or `f_` and its offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The byte offset in an object, or in the program's statics for a static field.
    pub offset: u32,
    pub bytes: u32,
    pub kind: Kind,
    pub is_static: bool,
}

/// What a function is to the type it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Method,
    Constructor,
    StaticConstructor,
    Finalizer,
    Getter,
    Setter,
    Operator,
    /// The body of a lambda, a member of its closure class.
    Lambda,
    LocalFunction,
    /// The step function of a state machine, which holds the body of the `async` method or iterator.
    Step,
}

/// One function with what is proved of its place in a class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    pub function: tree::Function,
    pub role: Role,
    /// Whether it takes no receiver; the receiver of one that does is its first parameter.
    pub is_static: bool,
    pub is_async: bool,
    /// The virtual slot it fills.
    pub slot: Option<u32>,
    /// Whether a base type fills the same slot.
    pub overrides: bool,
}

impl Method {
    /// A function no type claims.
    pub fn free(function: tree::Function) -> Method {
        Method {
            function,
            role: Role::Method,
            is_static: true,
            is_async: false,
            slot: None,
            overrides: false,
        }
    }
}
