//! The world family and scenario regions as typed roots: `.ymap` as
//! [`CMapData`], `.ytyp` as [`CMapTypes`] and a region `.ymt` as
//! [`CScenarioPointRegion`], each member found by the name hash the document's
//! own type table stores, so a read needs no schema.
//!
//! ⚠ A record keeps the value tree it was read from, and a save writes the
//! typed members back into that tree: what no member names (enums,
//! extensions, grass batches, entity overrides) reaches the file as it arrived,
//! and an untouched save is the tree's own save.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::decl::{File, HasOrigin, Origin};
use crate::decl::{Vec2, Vec3, Vec4};
use crate::error::Error;
use crate::hash::literal_string_hash;
use crate::meta::binary::synth::{name_hash, pso_layout_for_instance, rbm_layout_for_instance};
use crate::meta::binary::{
    type_tables, write_meta_binary, BinaryFormat, ContainerLayout, MetaLayout,
};
use crate::meta::instance::{write_meta, HashedString, MetaDocument, MetaInstance, SourceFormat};
use crate::meta::schema::{ArrayKind, FieldType, Literal, StringKind};
use crate::meta::{SchemaLibrary, Value};

/// Equal when the name's `atStringHash` is this hash, whatever text the reader resolved.
impl PartialEq<&str> for HashedString {
    fn eq(&self, name: &&str) -> bool {
        self.hash == crate::string_hash(name.as_bytes())
    }
}

fn is_named(spelt: &str, name: &str) -> bool {
    name_hash(spelt) == name_hash(name)
}

fn shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "nothing",
        Value::Bool(_) => "a bool",
        Value::Int(_) => "a signed integer",
        Value::UInt(_) => "an unsigned integer",
        Value::Float(_) => "a float",
        Value::String(_) => "a string",
        Value::Hash(_) => "a hash",
        Value::Vec2(_) => "a Vec2",
        Value::Vec3(_) => "a Vec3",
        Value::Vec4(_) => "a Vec4",
        Value::Mat34(_) => "a matrix",
        Value::Color(_) => "a colour",
        Value::Enum { .. } => "an enum",
        Value::Bitset { .. } => "a bitset",
        Value::Struct(_) => "a struct",
        Value::Pointer(_) => "a pointer",
        Value::Array(_) => "an array",
        Value::Map(_) => "a map",
        Value::ByteArray(_) => "a byte array",
        Value::Raw(_) => "raw text",
    }
}

fn expected<T>(what: &str, found: &Value) -> Result<T, String> {
    Err(format!("expected {what}, found {}", shape(found)))
}

/// A typed member's value in the tree: read from its slot, and written back
/// only when it changed, in the slot's own shape.
trait Member: Sized {
    fn read(value: &Value) -> Result<Self, String>;
    fn write(&self, slot: &mut Value) -> Result<(), String>;
}

impl Member for u32 {
    fn read(value: &Value) -> Result<Self, String> {
        let wide = match value {
            Value::UInt(v) => *v as i128,
            Value::Int(v) if *v < 0 => i128::from(*v as i32 as u32),
            Value::Int(v) => *v as i128,
            Value::Bitset { bits, .. } => *bits as i128,
            Value::Color(v) => return Ok(*v),
            other => return expected("an unsigned integer", other),
        };
        u32::try_from(wide).map_err(|_| format!("{wide} does not fit a u32"))
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if Self::read(slot).ok() == Some(*self) {
            return Ok(());
        }
        match slot {
            Value::UInt(v) => *v = u64::from(*self),
            Value::Int(v) => *v = i64::from(*self),
            Value::Bitset { bits, .. } => *bits = u128::from(*self),
            Value::Color(v) => *v = *self,
            other => *other = Value::UInt(u64::from(*self)),
        }
        Ok(())
    }
}

/// The engine's narrow unsigned members, read and written through `u32` so a slot keeps its shape.
macro_rules! narrow_member {
    ($($t:ty),*) => {$(
        impl Member for $t {
            fn read(value: &Value) -> Result<Self, String> {
                let wide = u32::read(value)?;
                <$t>::try_from(wide).map_err(|_| format!("{wide} does not fit a {}", stringify!($t)))
            }

            fn write(&self, slot: &mut Value) -> Result<(), String> {
                u32::from(*self).write(slot)
            }
        }

        impl Element for $t {
            fn read_element(value: &Value) -> Result<Self, String> {
                <Self as Member>::read(value)
            }

            fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String> {
                let mut value = was.or(sibling).cloned().unwrap_or(Value::UInt(0));
                self.write(&mut value)?;
                Ok(value)
            }
        }
    )*};
}

narrow_member!(u8, u16);

impl Element for u32 {
    fn read_element(value: &Value) -> Result<Self, String> {
        <Self as Member>::read(value)
    }

    fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String> {
        let mut value = was.or(sibling).cloned().unwrap_or(Value::UInt(0));
        self.write(&mut value)?;
        Ok(value)
    }
}

impl Member for i32 {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Int(v) => i32::try_from(*v).map_err(|_| format!("{v} does not fit an i32")),
            Value::UInt(v) => u32::try_from(*v)
                .map(|v| v as i32)
                .map_err(|_| format!("{v} does not fit an i32")),
            other => expected("a signed integer", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if Self::read(slot).ok() == Some(*self) {
            return Ok(());
        }
        match slot {
            Value::Int(v) => *v = i64::from(*self),
            Value::UInt(v) => *v = u64::from(*self as u32),
            other => *other = Value::Int(i64::from(*self)),
        }
        Ok(())
    }
}

/// The engine's narrow signed members, read and written through `i32` so a slot keeps its shape.
macro_rules! narrow_signed_member {
    ($($t:ty),*) => {$(
        impl Member for $t {
            fn read(value: &Value) -> Result<Self, String> {
                let wide = i32::read(value)?;
                <$t>::try_from(wide).map_err(|_| format!("{wide} does not fit an {}", stringify!($t)))
            }

            fn write(&self, slot: &mut Value) -> Result<(), String> {
                i32::from(*self).write(slot)
            }
        }
    )*};
}

narrow_signed_member!(i8, i16);

impl Member for f32 {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Float(v) => Ok(*v as f32),
            Value::Int(v) => Ok(*v as f32),
            Value::UInt(v) => Ok(*v as f32),
            other => expected("a float", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if Self::read(slot).is_ok_and(|v| v.to_bits() == self.to_bits()) {
            return Ok(());
        }
        *slot = Value::Float(f64::from(*self));
        Ok(())
    }
}

impl Member for bool {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Bool(v) => Ok(*v),
            Value::UInt(v) => Ok(*v != 0),
            Value::Int(v) => Ok(*v != 0),
            other => expected("a bool", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if Self::read(slot).ok() == Some(*self) {
            return Ok(());
        }
        match slot {
            Value::UInt(v) => *v = u64::from(*self),
            Value::Int(v) => *v = i64::from(*self),
            other => *other = Value::Bool(*self),
        }
        Ok(())
    }
}

fn same_bits(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
}

impl Member for Vec3 {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Vec3([x, y, z]) | Value::Vec4([x, y, z, _]) => Ok(Vec3 {
                x: *x,
                y: *y,
                z: *z,
            }),
            other => expected("a Vec3", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        let xyz = [self.x, self.y, self.z];
        match slot {
            Value::Vec3(v) => {
                if !same_bits(v, &xyz) {
                    *v = xyz;
                }
            }
            Value::Vec4(v) => {
                if !same_bits(&v[..3], &xyz) {
                    v[..3].copy_from_slice(&xyz);
                }
            }
            other => *other = Value::Vec3(xyz),
        }
        Ok(())
    }
}

impl Element for Vec3 {
    fn read_element(value: &Value) -> Result<Self, String> {
        <Self as Member>::read(value)
    }

    fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String> {
        let mut value = was.or(sibling).cloned().unwrap_or(Value::Vec3([0.0; 3]));
        self.write(&mut value)?;
        Ok(value)
    }
}

impl Member for Vec2 {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Vec2([x, y]) => Ok(Vec2 { x: *x, y: *y }),
            other => expected("a Vec2", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        let xy = [self.x, self.y];
        match slot {
            Value::Vec2(v) if same_bits(v, &xy) => {}
            other => *other = Value::Vec2(xy),
        }
        Ok(())
    }
}

impl Member for Vec4 {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Vec4([x, y, z, w]) => Ok(Vec4 {
                x: *x,
                y: *y,
                z: *z,
                w: *w,
            }),
            other => expected("a Vec4", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        let xyzw = [self.x, self.y, self.z, self.w];
        match slot {
            Value::Vec4(v) if same_bits(v, &xyzw) => {}
            other => *other = Value::Vec4(xyzw),
        }
        Ok(())
    }
}

impl Member for HashedString {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Hash(h) => Ok(h.clone()),
            Value::UInt(v) => u32::try_from(*v)
                .map(HashedString::from_hash)
                .map_err(|_| format!("{v} does not fit a hash")),
            Value::String(s) => Ok(HashedString::new(s.clone())),
            other => expected("a hash", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if Self::read(slot).is_ok_and(|h| h.hash == self.hash) {
            return Ok(());
        }
        let named = !self.raw.is_empty() && crate::string_hash(self.raw.as_bytes()) == self.hash;
        match slot {
            Value::UInt(v) => *v = u64::from(self.hash),
            Value::String(s) if named => s.clone_from(&self.raw),
            other => *other = Value::Hash(self.clone()),
        }
        Ok(())
    }
}

/// Text, as a document of plain strings states it: the weapon family's names, models and enumerators.
impl Member for String {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(s) => Ok(s.clone()),
            other => expected("a string", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        if !matches!(slot, Value::String(s) if s == self) {
            *slot = Value::String(self.clone());
        }
        Ok(())
    }
}

/// One element of a typed array: `was` is the element it replaces, `sibling` the array's first.
trait Element: Sized {
    fn read_element(value: &Value) -> Result<Self, String>;
    fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String>;
}

impl Element for HashedString {
    fn read_element(value: &Value) -> Result<Self, String> {
        <Self as Member>::read(value)
    }

    fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String> {
        let mut value = was.or(sibling).cloned().unwrap_or(Value::Null);
        self.write(&mut value)?;
        Ok(value)
    }
}

impl Element for String {
    fn read_element(value: &Value) -> Result<Self, String> {
        <Self as Member>::read(value)
    }

    fn write_element(&self, _: Option<&Value>, _: Option<&Value>) -> Result<Value, String> {
        Ok(Value::String(self.clone()))
    }
}

impl Element for f32 {
    fn read_element(value: &Value) -> Result<Self, String> {
        <Self as Member>::read(value)
    }

    fn write_element(&self, was: Option<&Value>, sibling: Option<&Value>) -> Result<Value, String> {
        let mut value = was.or(sibling).cloned().unwrap_or(Value::Float(0.0));
        self.write(&mut value)?;
        Ok(value)
    }
}

impl<E: Element> Member for Vec<E> {
    fn read(value: &Value) -> Result<Self, String> {
        match value {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, v)| E::read_element(v).map_err(|why| format!("element {i}: {why}")))
                .collect(),
            // ⚠ Read with no schema, `<list />` is the empty text of a leaf.
            Value::String(text) if text.trim().is_empty() => Ok(Vec::new()),
            // A run of bytes behind a bare pointer is read as one value, and a null pointer as none.
            Value::ByteArray(bytes) => (bytes.iter())
                .map(|byte| E::read_element(&Value::UInt(u64::from(*byte))))
                .collect(),
            Value::Null => Ok(Vec::new()),
            other => expected("an array", other),
        }
    }

    fn write(&self, slot: &mut Value) -> Result<(), String> {
        match slot {
            Value::ByteArray(was) => {
                let byte = |(i, e): (usize, &E)| {
                    let value = e.write_element(None, None)?;
                    let byte = u32::read(&value).ok().and_then(|b| u8::try_from(b).ok());
                    byte.ok_or_else(|| {
                        format!("element {i}: expected a byte, found {}", shape(&value))
                    })
                };
                let bytes = self
                    .iter()
                    .enumerate()
                    .map(byte)
                    .collect::<Result<Vec<u8>, _>>()?;
                if *was != bytes {
                    *was = bytes;
                }
                return Ok(());
            }
            // ⚠ A null pointer has no block for a writer to put a byte in.
            Value::Null if self.is_empty() => return Ok(()),
            _ => {}
        }
        let was: &[Value] = match slot {
            Value::Array(items) => items,
            _ => &[],
        };
        let items = self
            .iter()
            .enumerate()
            .map(|(i, e)| {
                e.write_element(was.get(i), was.first())
                    .map_err(|why| format!("element {i}: {why}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        *slot = Value::Array(items);
        Ok(())
    }
}

fn field<'a>(inst: &'a MetaInstance, name: &str) -> Option<&'a Value> {
    let key = literal_string_hash(name.as_bytes());
    inst.fields
        .iter()
        .find(|(n, _)| name_hash(n) == key)
        .map(|(_, v)| v)
}

/// ⚠ A member the document's own type table lacks reads as the engine's
/// `init` for it: shipped files carry older tables (`id2_17_strm_0.ymap` has
/// no `childLodDist`, eighteen `.ytyp` have no `assetName`).
fn member<T: Member>(inst: &MetaInstance, owner: &str, name: &str, init: T) -> Result<T, String> {
    match field(inst, name) {
        Some(value) => T::read(value).map_err(|why| format!("`{owner}::{name}`: {why}")),
        None => Ok(init),
    }
}

/// Writes a typed member into `inst`; `arrived` says `inst` is the tree the
/// record was read from, or one conformed to the class its document states,
/// whose type table a member it lacked is not in.
fn put<T: Member + PartialEq>(
    inst: &mut MetaInstance,
    name: &str,
    value: &T,
    init: T,
    arrived: bool,
) -> Result<(), String> {
    let key = literal_string_hash(name.as_bytes());
    match inst.fields.iter_mut().find(|(n, _)| name_hash(n) == key) {
        Some((_, slot)) => value.write(slot),
        None if arrived && *value == init => Ok(()),
        // ⚠ A text document has no type table that lacks a member: it states the members it has, and a member set on one is stated.
        None if arrived && weapon_types::is_text_type(&inst.type_name) => {
            let mut slot = Value::Null;
            value.write(&mut slot)?;
            inst.fields.push((name.to_string(), slot));
            Ok(())
        }
        None if arrived => Err(format!(
            "`{}::{name}` was set, and the document's own `{}` has no such member to hold it",
            inst.type_name, inst.type_name
        )),
        None => {
            let mut slot = Value::Null;
            value.write(&mut slot)?;
            inst.fields.push((name.to_string(), slot));
            Ok(())
        }
    }
}

/// The value a member holds in a record built from nothing: its schema `init`, or zero.
fn empty(library: &SchemaLibrary, ty: &FieldType, init: Option<&Literal>) -> Result<Value, String> {
    let number = |fallback: f64| match init {
        Some(Literal::Float(v)) => *v,
        Some(Literal::Int(v)) => *v as f64,
        _ => fallback,
    };
    Ok(match ty {
        FieldType::Bool => Value::Bool(matches!(init, Some(Literal::Bool(true)))),
        FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
            Value::UInt(number(0.0) as u64)
        }
        FieldType::S8 | FieldType::S16 | FieldType::S32 | FieldType::S64 => {
            Value::Int(number(0.0) as i64)
        }
        FieldType::Float | FieldType::Float16 | FieldType::Double => Value::Float(number(0.0)),
        FieldType::String(StringKind::Plain | StringKind::AtString) => Value::String(String::new()),
        FieldType::String(_) => Value::Hash(HashedString::default()),
        FieldType::Vector2 | FieldType::Vec2V => Value::Vec2([0.0; 2]),
        FieldType::Vector3 | FieldType::Vec3V => Value::Vec3([0.0; 3]),
        FieldType::Vector4 | FieldType::Vec4V => Value::Vec4([0.0; 4]),
        FieldType::Mat33V
        | FieldType::Mat34V
        | FieldType::Mat44V
        | FieldType::Matrix33
        | FieldType::Matrix34
        | FieldType::Matrix44 => Value::Mat34([[0.0; 4]; 3]),
        FieldType::Color32 => Value::Color(0),
        FieldType::Enum(name) => Value::Enum {
            type_name: name.clone(),
            variant: match init {
                Some(Literal::Enum(v) | Literal::Other(v) | Literal::Str(v)) => v.clone(),
                Some(Literal::Int(v)) => format!("0x{:08X}", *v as u32),
                _ => "0x00000000".into(),
            },
        },
        FieldType::Bitset { values, .. } => Value::Bitset {
            type_name: values.clone(),
            bits: 0,
        },
        FieldType::Struct(name) => {
            let mut inst = MetaInstance::new(name.clone());
            conform(library, &mut inst)?;
            Value::Struct(Box::new(inst))
        }
        FieldType::Pointer { .. } => Value::Pointer(None),
        FieldType::Array { elem, kind } => match kind {
            ArrayKind::AtFixedArray(n) | ArrayKind::Member(n) => Value::Array(
                (0..*n)
                    .map(|_| empty(library, elem, None))
                    .collect::<Result<_, _>>()?,
            ),
            _ => Value::Array(Vec::new()),
        },
        FieldType::Map { .. } => Value::Map(Vec::new()),
        FieldType::Pad(_) | FieldType::Unknown(_) => Value::Null,
    })
}

/// Why a `type_name` built here is not saved where nothing gives its class.
fn no_class_to_build(type_name: &str) -> String {
    if !crate::meta::has_default_library() {
        format!(
            "no schema snapshot is installed, so a `{type_name}` built or re-typed here cannot be saved; an array grows only by a copy of an element the file already holds"
        )
    } else {
        format!(
            "a `{type_name}` built or re-typed here is saved against the installed schema, which does not declare it"
        )
    }
}

/// Gives `inst` exactly the members the installed schema declares for its
/// type: the ones it holds are kept, the rest are built empty.
///
/// ⚠ A user's machine has no snapshot, so a record built here is refused
/// there: an array grows only by a copy of an element the file holds, which
/// keeps the tree it was read from and is never conformed. A record of a world
/// document is the exception, conformed to what [`world_types::Saving`] gives.
fn conform(library: &SchemaLibrary, inst: &mut MetaInstance) -> Result<(), String> {
    let resolved = library
        .resolve_any(&inst.type_name)
        .ok_or_else(|| no_class_to_build(&inst.type_name))?;
    let declared: Vec<_> = resolved
        .fields
        .iter()
        .filter(|f| !matches!(f.ty, FieldType::Pad(_) | FieldType::Unknown(_)))
        .collect();
    inst.fields.retain(|(n, _)| {
        let key = name_hash(n);
        declared.iter().any(|f| name_hash(&f.name) == key)
    });
    for field in declared {
        let key = name_hash(&field.name);
        if !inst.fields.iter().any(|(n, _)| name_hash(n) == key) {
            let value = empty(library, &field.ty, field.init.as_ref())?;
            // ⚠ Spelt as the writer looks it up, which is `hash_XXXXXXXX` for a name the hash index lacks; a text document spells every name it has.
            let spelt = match weapon_types::is_text_type(&inst.type_name) {
                true => field.name.clone(),
                false => crate::meta::binary::hash_index::default_hash_index().name_or_literal(key),
            };
            inst.fields.push((spelt, value));
        }
    }
    Ok(())
}

/// How an opened root was encoded, so a save writes it the same way.
#[derive(Clone)]
struct Arrival {
    layout: Option<MetaLayout>,
    format: SourceFormat,
}

thread_local! {
    /// The types of the world document this thread is saving, which a record built in it is conformed to.
    static SAVING: RefCell<Option<Rc<world_types::Saving>>> = const { RefCell::new(None) };
}

/// Runs `write` with every record built here conformed to `saving`.
fn while_saving<R>(saving: &Rc<world_types::Saving>, write: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<world_types::Saving>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SAVING.with(|saving| *saving.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SAVING.with(|held| held.replace(Some(Rc::clone(saving)))));
    write()
}

/// What a record holds besides its typed members: the tree it was read from,
/// whether its array held it behind a pointer and, on a root, the arrival.
#[derive(Clone, Default)]
struct Carry {
    instance: Option<Box<MetaInstance>>,
    /// On a creature root read from RBF, the tags it was read from, which a save writes the typed members back into.
    tags: Option<Box<MetaInstance>>,
    pointer: Option<bool>,
    /// The class of a record read where it is held by value: its holder fixes it, whatever a text read with no schema
    /// called the record (`Item`, or the member that holds it).
    by_value: Option<&'static str>,
    arrival: Option<Box<Arrival>>,
    origin: Origin,
}

impl Carry {
    fn of(inst: &MetaInstance) -> Self {
        Self {
            instance: Some(Box::new(inst.clone())),
            ..Self::default()
        }
    }

    /// Whether the tree the record carries is one a `type_name` arrived as.
    fn arrived_as(&self, type_name: &str) -> bool {
        (self.instance.as_ref())
            .is_some_and(|inst| is_named(self.by_value.unwrap_or(&inst.type_name), type_name))
    }

    /// The tree to write the typed members into, and whether it is the one the
    /// record arrived as. A record moved to another type, or built here, is
    /// conformed to the schema's `type_name`, so no member of a type it left survives.
    fn instance(&self, type_name: &str) -> Result<(MetaInstance, bool), String> {
        match &self.instance {
            Some(inst) if self.arrived_as(type_name) => {
                let mut inst = (**inst).clone();
                // ⚠ A copy from another file carries that file's tree, which this document may state no class for.
                if let Some(saving) = SAVING.with(|saving| saving.borrow().clone()) {
                    saving.conform_carried(&mut inst)?;
                }
                Ok((inst, true))
            }
            carried => {
                let mut inst = carried
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| MetaInstance::new(type_name));
                inst.type_name = type_name.to_string();
                match SAVING.with(|saving| saving.borrow().clone()) {
                    // ⚠ The class is the document's own, so a member it lacks cannot be held, as in a record that arrived.
                    Some(saving) => {
                        saving.conform(&mut inst)?;
                        Ok((inst, true))
                    }
                    None => {
                        conform(
                            &ped_types::library(crate::meta::default_library()),
                            &mut inst,
                        )?;
                        Ok((inst, false))
                    }
                }
            }
        }
    }

    fn carried(&self) -> impl Iterator<Item = (u32, &Value)> {
        self.instance.iter().flat_map(|inst| {
            // ⚠ By the class the record was read as: a text calls a room `Item`, a name no class's members are typed for.
            let class = self.by_value.unwrap_or(inst.type_name.as_str());
            let untyped = move |(key, _): &(u32, &Value)| !is_typed(class, *key);
            (inst.fields.iter())
                .map(|(n, v)| (name_hash(n), v))
                .filter(untyped)
        })
    }
}

/// Equal when the members no record types are; the typed ones are the record's
/// to compare, and a record built here carries none to compare.
impl PartialEq for Carry {
    fn eq(&self, other: &Self) -> bool {
        self.instance.is_none() || other.instance.is_none() || self.carried().eq(other.carried())
    }
}

impl fmt::Debug for Carry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} carried member(s)", self.carried().count())
    }
}

/// A member's engine `init`, or its type's default where the schema gives none.
macro_rules! meta_init {
    () => {
        Default::default()
    };
    ($init:expr) => {
        $init
    };
}

macro_rules! meta_record {
    (
        $(#[$doc:meta])*
        $name:ident = $type_name:literal {
            $($field:ident: $ty:ty = $member:literal $(=> $init:expr)?,)*
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            $(pub $field: $ty,)*
            carry: Carry,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    $($field: meta_init!($($init)?),)*
                    carry: Carry::default(),
                }
            }
        }

        impl $name {
            /// The engine names of the members this record types.
            pub const MEMBERS: &'static [&'static str] = &[$($member),*];

            fn read(inst: &MetaInstance) -> Result<Self, String> {
                Ok(Self {
                    $($field: member(inst, $type_name, $member, meta_init!($($init)?))?,)*
                    carry: Carry::of(inst),
                })
            }

            fn write_as(&self, type_name: &str) -> Result<(MetaInstance, bool), String> {
                let (mut inst, arrived) = self.carry.instance(type_name)?;
                $(put(&mut inst, $member, &self.$field, meta_init!($($init)?), arrived)?;)*
                Ok((inst, arrived))
            }

            fn write(&self) -> Result<MetaInstance, String> {
                self.write_as($type_name).map(|(inst, _)| inst)
            }
        }

        impl Member for $name {
            fn read(value: &Value) -> Result<Self, String> {
                match value {
                    Value::Struct(inst) => {
                        let mut record = Self::read(inst)?;
                        record.carry.by_value = Some($type_name);
                        Ok(record)
                    }
                    Value::Pointer(Some(inst)) => Self::read(inst),
                    // ⚠ Read with no schema, an element that holds nothing is the empty text of a leaf: a record that
                    // states no member.
                    Value::String(text) if text.trim().is_empty() => {
                        let mut record = Self::read(&MetaInstance::new($type_name))?;
                        record.carry.by_value = Some($type_name);
                        Ok(record)
                    }
                    other => expected("a struct", other),
                }
            }

            fn write(&self, slot: &mut Value) -> Result<(), String> {
                let inst = Box::new(self.write()?);
                match slot {
                    Value::Pointer(p) => *p = Some(inst),
                    other => *other = Value::Struct(inst),
                }
                Ok(())
            }
        }

        impl Element for $name {
            fn read_element(value: &Value) -> Result<Self, String> {
                let mut element = <Self as Member>::read(value)?;
                element.carry.pointer = Some(matches!(value, Value::Pointer(_)));
                Ok(element)
            }

            fn write_element(
                &self,
                was: Option<&Value>,
                sibling: Option<&Value>,
            ) -> Result<Value, String> {
                let inst = Box::new(self.write()?);
                let pointer = self
                    .carry
                    .pointer
                    .or_else(|| was.or(sibling).map(|v| matches!(v, Value::Pointer(_))))
                    .unwrap_or(false);
                Ok(if pointer {
                    Value::Pointer(Some(inst))
                } else {
                    Value::Struct(inst)
                })
            }
        }
    };
}

macro_rules! meta_derived {
    (
        $(#[$doc:meta])*
        $name:ident: $base:ty = $type_name:literal {
            $($field:ident: $ty:ty = $member:literal $(=> $init:expr)?,)*
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            pub base: $base,
            $(pub $field: $ty,)*
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    base: <$base>::default(),
                    $($field: meta_init!($($init)?),)*
                }
            }
        }

        impl $name {
            /// The engine names of the members this record types.
            pub const MEMBERS: &'static [&'static str] = &[$($member),*];

            fn read(inst: &MetaInstance) -> Result<Self, String> {
                Ok(Self {
                    base: <$base>::read(inst)?,
                    $($field: member(inst, $type_name, $member, meta_init!($($init)?))?,)*
                })
            }

            fn write(&self) -> Result<MetaInstance, String> {
                let (mut inst, arrived) = self.base.write_as($type_name)?;
                $(put(&mut inst, $member, &self.$field, meta_init!($($init)?), arrived)?;)*
                Ok(inst)
            }
        }
    };
}

/// A polymorphic array element: the derived records by name, and `Base` for the base itself.
macro_rules! meta_any {
    (
        $(#[$doc:meta])*
        $name:ident: $base:ident {
            $($derived:ident,)*
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $name {
            $($derived($derived),)*
            Base($base),
        }

        impl Default for $name {
            fn default() -> Self {
                Self::Base($base::default())
            }
        }

        impl Deref for $name {
            type Target = $base;

            fn deref(&self) -> &$base {
                match self {
                    $(Self::$derived(d) => &d.base,)*
                    Self::Base(b) => b,
                }
            }
        }

        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut $base {
                match self {
                    $(Self::$derived(d) => &mut d.base,)*
                    Self::Base(b) => b,
                }
            }
        }

        impl $name {
            /// The value tree this element saves as, behind a pointer when it arrived behind one.
            pub fn to_value(&self) -> Result<Value, Error> {
                self.write_element(None, None).map_err(|why| Error::Invalid {
                    what: concat!("a `", stringify!($base), "` element").to_string(),
                    why,
                })
            }

            /// The element a value tree reads as, which keeps that tree as what it saves.
            pub fn from_value(value: &Value) -> Result<Self, Error> {
                Self::read_element(value).map_err(|why| Error::Invalid {
                    what: concat!("a `", stringify!($base), "` element").to_string(),
                    why,
                })
            }

            fn read(inst: &MetaInstance) -> Result<Self, String> {
                $(if is_named(&inst.type_name, stringify!($derived)) {
                    return $derived::read(inst).map(Self::$derived);
                })*
                if is_named(&inst.type_name, stringify!($base)) {
                    return $base::read(inst).map(Self::Base);
                }
                Err(format!("a `{}` is not a `{}`", inst.type_name, stringify!($base)))
            }

            fn write(&self) -> Result<MetaInstance, String> {
                match self {
                    $(Self::$derived(d) => d.write(),)*
                    Self::Base(b) => b.write(),
                }
            }

            /// Built here or moved to another variant, so no arrival layout has its shape.
            fn reshaped(&self) -> bool {
                let (carry, type_name) = match self {
                    $(Self::$derived(d) => (&d.base.carry, stringify!($derived)),)*
                    Self::Base(b) => (&b.carry, stringify!($base)),
                };
                !carry
                    .instance
                    .as_ref()
                    .is_some_and(|inst| is_named(&inst.type_name, type_name))
            }
        }

        impl Element for $name {
            fn read_element(value: &Value) -> Result<Self, String> {
                let (inst, pointer) = match value {
                    Value::Struct(inst) => (inst, false),
                    Value::Pointer(Some(inst)) => (inst, true),
                    Value::Pointer(None) => return Err("a null pointer".into()),
                    other => return expected("a struct", other),
                };
                let mut element = Self::read(inst)?;
                element.carry.pointer = Some(pointer);
                Ok(element)
            }

            fn write_element(
                &self,
                was: Option<&Value>,
                sibling: Option<&Value>,
            ) -> Result<Value, String> {
                let inst = Box::new(self.write()?);
                let behind_pointer = |v: &Value| matches!(v, Value::Pointer(_));
                let pointer = self
                    .carry
                    .pointer
                    .or_else(|| was.or(sibling).map(behind_pointer))
                    .unwrap_or(true);
                Ok(if pointer {
                    Value::Pointer(Some(inst))
                } else {
                    Value::Struct(inst)
                })
            }
        }
    };
}

meta_record! {
    /// `CEntityDef`: one placement of an archetype in a map.
    CEntityDef = "CEntityDef" {
        archetype_name: HashedString = "archetypeName",
        flags: u32 = "flags",
        guid: u32 = "guid",
        position: Vec3 = "position",
        rotation: Vec4 = "rotation",
        scale_xy: f32 = "scaleXY",
        scale_z: f32 = "scaleZ",
        parent_index: i32 = "parentIndex",
        lod_dist: f32 = "lodDist",
        child_lod_dist: f32 = "childLodDist",
        num_children: u32 = "numChildren",
        ambient_occlusion_multiplier: i32 = "ambientOcclusionMultiplier" => 255,
        artificial_ambient_occlusion: i32 = "artificialAmbientOcclusion" => 255,
        tint_value: u32 = "tintValue",
    }
}

meta_derived! {
    /// `CMloInstanceDef`: a placed interior.
    CMloInstanceDef: CEntityDef = "CMloInstanceDef" {
        group_id: u32 = "groupId",
        floor_id: u32 = "floorId",
        default_entity_sets: Vec<HashedString> = "defaultEntitySets",
        num_exit_portals: u32 = "numExitPortals",
        mlo_instflags: u32 = "MLOInstflags",
    }
}

meta_any! {
    /// An element of an entities array: a `CEntityDef` or a record derived from it.
    CEntityDefAny: CEntityDef {
        CMloInstanceDef,
    }
}

impl CEntityDefAny {
    pub fn as_mlo_instance(&self) -> Option<&CMloInstanceDef> {
        match self {
            Self::CMloInstanceDef(d) => Some(d),
            Self::Base(_) => None,
        }
    }
}

meta_record! {
    /// `CBaseArchetypeDef`: what a map's entities are placements of.
    CBaseArchetypeDef = "CBaseArchetypeDef" {
        lod_dist: f32 = "lodDist",
        flags: u32 = "flags",
        special_attribute: u32 = "specialAttribute",
        bb_min: Vec3 = "bbMin",
        bb_max: Vec3 = "bbMax",
        bs_centre: Vec3 = "bsCentre",
        bs_radius: f32 = "bsRadius",
        hd_texture_dist: f32 = "hdTextureDist" => 5.0,
        name: HashedString = "name",
        texture_dictionary: HashedString = "textureDictionary",
        clip_dictionary: HashedString = "clipDictionary",
        drawable_dictionary: HashedString = "drawableDictionary",
        physics_dictionary: HashedString = "physicsDictionary",
        asset_name: HashedString = "assetName",
    }
}

meta_derived! {
    /// `CTimeArchetypeDef`: an archetype shown only in the hours its flags set.
    CTimeArchetypeDef: CBaseArchetypeDef = "CTimeArchetypeDef" {
        time_flags: u32 = "timeFlags",
    }
}

meta_derived! {
    /// `CMloArchetypeDef`: an interior, with its rooms, portals, entity sets and time cycle modifiers.
    CMloArchetypeDef: CBaseArchetypeDef = "CMloArchetypeDef" {
        mlo_flags: u32 = "mloFlags",
        entities: Vec<CEntityDefAny> = "entities",
        rooms: Vec<CMloRoomDef> = "rooms",
        portals: Vec<CMloPortalDef> = "portals",
        entity_sets: Vec<CMloEntitySet> = "entitySets",
        time_cycle_modifiers: Vec<CMloTimeCycleModifier> = "timeCycleModifiers",
    }
}

meta_any! {
    /// An element of an archetypes array: a `CBaseArchetypeDef` or a record derived from it.
    CBaseArchetypeDefAny: CBaseArchetypeDef {
        CTimeArchetypeDef,
        CMloArchetypeDef,
    }
}

meta_record! {
    /// `CMapData`: a `.ymap`, the entities it places and the extents it streams by.
    CMapData = "CMapData" {
        name: HashedString = "name",
        parent: HashedString = "parent",
        flags: u32 = "flags",
        content_flags: u32 = "contentFlags",
        streaming_extents_min: Vec3 = "streamingExtentsMin",
        streaming_extents_max: Vec3 = "streamingExtentsMax",
        entities_extents_min: Vec3 = "entitiesExtentsMin",
        entities_extents_max: Vec3 = "entitiesExtentsMax",
        entities: Vec<CEntityDefAny> = "entities",
        box_occluders: Vec<BoxOccluder> = "boxOccluders",
        occlude_models: Vec<OccludeModel> = "occludeModels",
        physics_dictionaries: Vec<HashedString> = "physicsDictionaries",
        time_cycle_modifiers: Vec<CTimeCycleModifier> = "timeCycleModifiers",
        car_generators: Vec<CCarGen> = "carGenerators",
        lod_lights_soa: CLODLight = "LODLightsSOA",
        distant_lod_lights_soa: CDistantLODLight = "DistantLODLightsSOA",
    }
}

meta_record! {
    /// `CMapTypes`: a `.ytyp`, the archetypes it declares.
    CMapTypes = "CMapTypes" {
        name: HashedString = "name",
        archetypes: Vec<CBaseArchetypeDefAny> = "archetypes",
        dependencies: Vec<HashedString> = "dependencies",
    }
}

meta_record! {
    /// `CScenarioPoint`: where a scenario runs; its type and model set index the region's look-ups.
    CScenarioPoint = "CScenarioPoint" {
        i_type: u8 = "iType",
        model_set_id: u8 = "ModelSetId",
        i_interior: u8 = "iInterior",
        i_required_imap_id: u8 = "iRequiredIMapId",
        i_probability: u8 = "iProbability",
        u_available_in_mp_sp: u8 = "uAvailableInMpSp",
        i_time_start_override: u8 = "iTimeStartOverride",
        i_time_end_override: u8 = "iTimeEndOverride",
        i_radius: u8 = "iRadius",
        i_time_till_ped_leaves: u8 = "iTimeTillPedLeaves",
        i_scenario_group: u16 = "iScenarioGroup",
        flags: u32 = "Flags",
        v_position_and_direction: Vec4 = "vPositionAndDirection",
    }
}

meta_record! {
    /// `CScenarioPointContainer`: a region's or a cluster's points.
    CScenarioPointContainer = "CScenarioPointContainer" {
        my_points: Vec<CScenarioPoint> = "MyPoints",
    }
}

meta_record! {
    /// `CScenarioChainingNode`: one stop on a scenario chain.
    CScenarioChainingNode = "CScenarioChainingNode" {
        position: Vec3 = "Position",
        attached_to_entity_type: HashedString = "AttachedToEntityType",
        scenario_type: HashedString = "ScenarioType",
        has_incoming_edges: bool = "HasIncomingEdges",
        has_outgoing_edges: bool = "HasOutgoingEdges",
    }
}

meta_record! {
    /// `CScenarioChainingEdge`: a step between two chaining nodes; its action and navigation are carried.
    CScenarioChainingEdge = "CScenarioChainingEdge" {
        node_index_from: u16 = "NodeIndexFrom",
        node_index_to: u16 = "NodeIndexTo",
    }
}

meta_record! {
    /// `CScenarioChain`: the edges one chain walks.
    CScenarioChain = "CScenarioChain" {
        max_users: u8 = "MaxUsers" => 1,
        edge_ids: Vec<u16> = "EdgeIds",
    }
}

meta_record! {
    /// `CScenarioChainingGraph`: a region's chains.
    CScenarioChainingGraph = "CScenarioChainingGraph" {
        nodes: Vec<CScenarioChainingNode> = "Nodes",
        edges: Vec<CScenarioChainingEdge> = "Edges",
        chains: Vec<CScenarioChain> = "Chains",
    }
}

meta_record! {
    /// `CScenarioPointCluster`: points that spawn together; its sphere is carried.
    CScenarioPointCluster = "CScenarioPointCluster" {
        points: CScenarioPointContainer = "Points",
        f_next_spawn_attempt_delay: f32 = "fNextSpawnAttemptDelay" => 30.0,
        b_all_point_required_for_spawn: bool = "bAllPointRequiredForSpawn",
    }
}

meta_record! {
    /// `CScenarioPointLookUps`: the names a region's points index.
    CScenarioPointLookUps = "CScenarioPointLookUps" {
        type_names: Vec<HashedString> = "TypeNames",
        ped_model_set_names: Vec<HashedString> = "PedModelSetNames",
        vehicle_model_set_names: Vec<HashedString> = "VehicleModelSetNames",
        group_names: Vec<HashedString> = "GroupNames",
        interior_names: Vec<HashedString> = "InteriorNames",
        required_imap_names: Vec<HashedString> = "RequiredIMapNames",
    }
}

meta_record! {
    /// `CScenarioPointRegion`: a region `.ymt`; its entity overrides and grid are carried.
    CScenarioPointRegion = "CScenarioPointRegion" {
        version_number: i32 = "VersionNumber",
        points: CScenarioPointContainer = "Points",
        chaining_graph: CScenarioChainingGraph = "ChainingGraph",
        clusters: Vec<CScenarioPointCluster> = "Clusters",
        look_ups: CScenarioPointLookUps = "LookUps",
    }
}

impl CScenarioPointRegion {
    /// The scenario type `point` names through this region's look-ups.
    pub fn scenario_type(&self, point: &CScenarioPoint) -> Option<&HashedString> {
        self.look_ups.type_names.get(usize::from(point.i_type))
    }

    /// The region's own points, then each cluster's, with where each sits.
    pub fn all_points(&self) -> impl Iterator<Item = (Option<usize>, &CScenarioPoint)> {
        let own = self.points.my_points.iter().map(|p| (None, p));
        let clustered =
            self.clusters.iter().enumerate().flat_map(|(c, cluster)| {
                cluster.points.my_points.iter().map(move |p| (Some(c), p))
            });
        own.chain(clustered)
    }
}

/// Whether a typed record names the member a `class` record stores under `key`.
///
/// ⚠ The records of an interior and of a map are judged by their class: their members bear names as plain as `flags`,
/// `hash` and `position`, and a name typed for every class would leave a member another class carries under it out of
/// what two records are compared by.
fn is_typed(class: &str, key: u32) -> bool {
    let names =
        |members: &[&str]| (members.iter()).any(|name| literal_string_hash(name.as_bytes()) == key);
    let mut by_class = interior::MEMBERS.iter().chain(map_records::MEMBERS);
    if by_class.any(|(of, members)| is_named(class, of) && names(members)) {
        return true;
    }
    [
        CEntityDef::MEMBERS,
        CMloInstanceDef::MEMBERS,
        CBaseArchetypeDef::MEMBERS,
        CTimeArchetypeDef::MEMBERS,
        CMapData::MEMBERS,
        CMapTypes::MEMBERS,
        CScenarioPoint::MEMBERS,
        CScenarioPointContainer::MEMBERS,
        CScenarioChainingNode::MEMBERS,
        CScenarioChainingEdge::MEMBERS,
        CScenarioChain::MEMBERS,
        CScenarioChainingGraph::MEMBERS,
        CScenarioPointCluster::MEMBERS,
        CScenarioPointLookUps::MEMBERS,
        CScenarioPointRegion::MEMBERS,
        CPedVariationInfo::MEMBERS,
        CPVComponentData::MEMBERS,
        CPVDrawblData::MEMBERS,
        CPVClothComponentData::MEMBERS,
        CPVTextureData::MEMBERS,
        CComponentInfo::MEMBERS,
        CPedPropInfo::MEMBERS,
        CPedPropMetaData::MEMBERS,
        CPedPropTexData::MEMBERS,
        CAnchorProps::MEMBERS,
        CPedSelectionSet::MEMBERS,
        CCreatureMetaData::MEMBERS,
        CShaderVariableComponent::MEMBERS,
        CPedPropExpressionData::MEMBERS,
        CPedCompExpressionData::MEMBERS,
        CAlternateVariations::MEMBERS,
        FirstPersonAlternateData::MEMBERS,
        FirstPersonAlternate::MEMBERS,
        SAlternateVariationPed::MEMBERS,
        SAlternateVariationSwitch::MEMBERS,
        SAlternateVariationComponent::MEMBERS,
    ]
    .iter()
    .any(|members| names(members))
        || weapon::is_member(key)
}

fn open_root<T>(
    ext: &'static str,
    type_name: &str,
    bytes: &[u8],
    read: fn(&MetaInstance) -> Result<T, String>,
) -> Result<(T, Arrival), Error> {
    let doc = MetaDocument::parse(crate::meta::default_library(), bytes.to_vec()).map_err(|e| {
        Error::Read {
            ext,
            why: e.to_string(),
        }
    })?;
    if !is_named(&doc.instance.type_name, type_name) {
        return Err(Error::WrongType {
            expected: type_name.to_string(),
            found: doc.instance.type_name,
        });
    }
    let root = read(&doc.instance).map_err(|why| Error::Read { ext, why })?;
    Ok((
        root,
        Arrival {
            layout: doc.layout,
            format: doc.source_format,
        },
    ))
}

/// The types a document that arrived with the type table `tables` is laid out by once it leaves the layout it arrived
/// with: the families declared in code, then each class `tables` states as it states it, then the installed snapshot.
///
/// ⚠ The installed schema is a developer's artifact, of one build of the game, and no user has one: it gives only a
/// class the document's own table neither states nor names, so an edit of the records a document holds saves the same
/// bytes with one installed and with none.
fn types_of(tables: &SchemaLibrary) -> SchemaLibrary {
    type_tables::layered(
        ped_types::family(),
        tables,
        crate::meta::snapshot::installed(),
    )
}

/// Encodes a root with the layout it arrived with, or a synthesised one when
/// it outgrew that layout, holds a `reshaped` record, or was built with none:
/// from `types` where a caller conformed its records to them, from
/// [`types_of`] its own type table where it only outgrew its layout.
///
/// ⚠ Synthesis is the fallback, never the default: it reproduces no shipped
/// document byte for byte, where the arrival layout reproduces them all. The
/// arrival layout places each array element as the type it arrived as, so a
/// record built here or moved to another variant does not fit it.
fn encode(
    ext: &'static str,
    arrival: Option<&Arrival>,
    root: &MetaInstance,
    reshaped: bool,
    types: Option<&SchemaLibrary>,
) -> Result<Vec<u8>, Error> {
    let save = |why: String| Error::Save { ext, why };
    let (format, layout) = match arrival {
        Some(arrival) => match arrival.format.to_binary() {
            Some(format) => (format, arrival.layout.as_ref()),
            None => {
                return write_meta(root)
                    .map(String::into_bytes)
                    .map_err(|e| save(e.to_string()))
            }
        },
        // ⚠ A world root built here does not reach this arm: `save_world_root` writes it as PSO, whose synthesis has no encoding
        // for a `CMapData`'s occluder vertices, which RBM has.
        None => (BinaryFormat::Rbm, None),
    };
    let arrival_layout = layout.filter(|_| !reshaped);
    match arrival_layout.map(|layout| write_meta_binary(format, Some(layout), root)) {
        Some(Err(e)) if e.outgrew_its_layout() => {
            tracing::info!(reason = %e, ".{ext} outgrew its own allocation; saving against a synthesised layout");
        }
        Some(written) => return written.map_err(|e| save(e.to_string())),
        None => {}
    }
    // A GTA VI P-form's own tables and the measured rules lay out any tree of its types.
    #[cfg(feature = "gta6")]
    if let Some(ContainerLayout::Pso(pso)) = layout.map(|l| &l.container) {
        if pso.is_gta6() && !pso.is_table_form() {
            return crate::meta::binary::pso::pform_layout::write(pso, root)
                .map_err(|e| save(e.to_string()));
        }
    }
    let own = layout.map(type_tables::of_layout);
    let family = ped_types::library(crate::meta::default_library());
    let library = match (types, &own) {
        // The records built here were conformed to these, so the layout is made of the same.
        (Some(types), _) => Cow::Borrowed(types),
        // ⚠ Only a root of the ped families reaches this arm: a record built in one was conformed to the families'
        // classes and the installed schema's, and laid out by another class the writer drops its members and says nothing.
        (None, Some(tables)) if reshaped => type_tables::extend(&family, tables),
        (None, Some(tables)) => Cow::Owned(types_of(tables)),
        (None, None) => Cow::Borrowed(&*family),
    };
    let library = &*library;
    let container = match layout.map(|l| &l.container) {
        Some(ContainerLayout::Rbm(donor)) => {
            let donor = ped_types::donor(Some(&donor.model));
            rbm_layout_for_instance(library, root, Some(&donor))
                .map(|l| ContainerLayout::Rbm(Box::new(l)))
        }
        None if format == BinaryFormat::Pso => {
            pso_layout_for_instance(library, root).map(ContainerLayout::Pso)
        }
        None => {
            let donor = ped_types::donor(None);
            rbm_layout_for_instance(library, root, Some(&donor))
                .map(|l| ContainerLayout::Rbm(Box::new(l)))
        }
        Some(ContainerLayout::Rbf(rbf)) => Ok(ContainerLayout::Rbf(rbf.clone())),
        // A table-form document's own tables lay out any tree of its types.
        Some(ContainerLayout::Pso(pso)) if pso.is_table_form() => {
            Ok(ContainerLayout::Pso(pso.clone()))
        }
        Some(ContainerLayout::Pso(pso)) if pso.is_gta6() => {
            Err(crate::meta::binary::pso::table_form::TableFormError::NoLayoutRules.into())
        }
        Some(ContainerLayout::Pso(_)) => {
            pso_layout_for_instance(library, root).map(ContainerLayout::Pso)
        }
    }
    .map_err(|e| save(e.to_string()))?;
    write_meta_binary(format, Some(&MetaLayout { format, container }), root)
        .map_err(|e| save(e.to_string()))
}

fn save_root(
    ext: &'static str,
    broken: Option<Broken>,
    root: Result<MetaInstance, String>,
    carry: &Carry,
    reshaped: bool,
) -> Result<Vec<u8>, Error> {
    save_root_as(ext, broken, root, carry, reshaped, None)
}

/// [`save_root`], written as `fallback` when the record arrived as nothing.
fn save_root_as(
    ext: &'static str,
    broken: Option<Broken>,
    root: Result<MetaInstance, String>,
    carry: &Carry,
    reshaped: bool,
    fallback: Option<&Arrival>,
) -> Result<Vec<u8>, Error> {
    if let Some(reason) = broken {
        return Err(Error::Refused {
            ext,
            reason: crate::error::Reason::new(reason),
        });
    }
    let root = root.map_err(|why| Error::Save { ext, why })?;
    encode(
        ext,
        carry.arrival.as_deref().or(fallback),
        &root,
        reshaped,
        None,
    )
}

/// [`save_root`] for a map, a type file and a region that arrived, each laid out by the classes its own type table
/// states: a record built here takes the members its class has there, with a snapshot installed and with none.
///
/// ⚠ A root built here is written as PSO: an RBM class carries a signature word that nothing derives ([`world_types`]).
fn save_world_root(
    ext: &'static str,
    broken: Option<Broken>,
    write: impl Fn() -> Result<MetaInstance, String>,
    carry: &Carry,
    mut reshaped: bool,
) -> Result<Vec<u8>, Error> {
    if let Some(reason) = broken {
        return Err(Error::Refused {
            ext,
            reason: crate::error::Reason::new(reason),
        });
    }
    let arrival = carry.arrival.as_deref();
    if let (Some(arrival), false) = (arrival, reshaped) {
        let root = write();
        // ⚠ A record copied from another file is `reshaped` to no caller: it carries a tree of its own class, which the
        // layout this file arrived with may not state, and that layout then has no place for it.
        let copied = match (&root, &arrival.layout) {
            (Ok(root), Some(layout)) => world_types::first_class_unstated(layout, root).is_some(),
            _ => false,
        };
        // Nothing was built or copied here, so nothing is conformed and the arrival's own layout stands.
        if !copied {
            let root = root.map_err(|why| Error::Save { ext, why })?;
            let layout = arrival.layout.as_ref();
            // ⚠ A copy of a class this file states may hold a member the class lacks here, which the layout has no place for.
            if let Some((class, member)) =
                layout.and_then(|layout| world_types::first_member_unstated(layout, &root))
            {
                let why = world_types::would_drop_stated(class, member);
                return Err(Error::Save { ext, why });
            }
            // ⚠ Or lack a member the class has here, which no writer has a value for: the save is then made again as
            // one that holds a record built here is, with the copy conformed to the class.
            let lacks_a_member = || {
                layout.is_some_and(|layout| {
                    world_types::first_member_missing(layout, &root).is_some()
                })
            };
            match encode(ext, Some(arrival), &root, false, None) {
                Err(Error::Save { .. }) if lacks_a_member() => {}
                saved => return saved,
            }
        }
        reshaped = true;
    }
    let saving = Rc::new(world_types::Saving::of(arrival));
    let root = while_saving(&saving, write)
        .and_then(|root| saving.refuse_what_has_no_place(&root).map(|()| root))
        .map_err(|why| Error::Save { ext, why })?;
    let built = Arrival {
        layout: None,
        format: SourceFormat::Pso,
    };
    encode(
        ext,
        Some(arrival.unwrap_or(&built)),
        &root,
        reshaped,
        Some(&saving.library),
    )
}

/// An index into an array the same file states, which the engine trusts and this one misses.
#[derive(Debug, thiserror::Error)]
enum Broken {
    #[error("entity {entity} names LOD parent {parent}, past the {len} entities in this map")]
    ParentOutOfRange {
        entity: usize,
        parent: usize,
        len: usize,
    },
    #[error("entity {entity} is its own LOD parent")]
    ParentIsSelf { entity: usize },
    #[error("archetype {archetype}: {what}")]
    Interior {
        archetype: usize,
        what: interior::Broken,
    },
    #[error("{0}")]
    MapRecords(map_records::Broken),
    #[error("{holder} point {point} names scenario type {index}, past the {len} type names in this region")]
    PointTypeOutOfRange {
        holder: String,
        point: usize,
        index: u8,
        len: usize,
    },
    #[error("availComp slot {slot} names component data {index}, past the {len} the file holds")]
    ComponentSlotOutOfRange { slot: usize, index: u8, len: usize },
    #[error("component data {component} states {stated} textures (numAvailTex, a byte that wraps at 256) and its drawables hold {held}")]
    TextureCountDiffers {
        component: usize,
        stated: u8,
        held: u8,
    },
    #[error("propInfo states {stated} props (numAvailProps) and holds {held}")]
    PropCountDiffers { stated: u8, held: u8 },
    #[error("compInfos[{info}] names drawable {drawable} of component slot {slot}, which the file does not hold")]
    ComponentInfoNamesNothing { info: usize, slot: u8, drawable: u8 },
    #[error("{list}[{entry}] holds parallel arrays (tracks, ids, components, and types where it has them) of different lengths")]
    ExpressionArraysDiffer { list: &'static str, entry: usize },
    #[error("a switch of {ped} states alternate 0 for component {component} drawable {index}: alternate 0 is the drawable itself, and the game refuses it")]
    ZeroAlternate {
        ped: String,
        component: u8,
        index: u8,
    },
    #[error("an asset of {ped} (component {component}, anchor {anchor}, drawable {index}) is the source of more than 159 switches, which is more than the game holds for one asset")]
    TooManySwitchesForOneAsset {
        ped: String,
        component: u8,
        anchor: u8,
        index: u8,
    },
}

fn broken_point_type(region: &CScenarioPointRegion) -> Option<Broken> {
    let len = region.look_ups.type_names.len();
    let own = std::iter::once((None, &region.points));
    let clustered = region
        .clusters
        .iter()
        .enumerate()
        .map(|(c, k)| (Some(c), &k.points));
    own.chain(clustered).find_map(|(cluster, points)| {
        let (point, p) = points
            .my_points
            .iter()
            .enumerate()
            .find(|(_, p)| usize::from(p.i_type) >= len)?;
        Some(Broken::PointTypeOutOfRange {
            holder: cluster.map_or("the region's".into(), |c| format!("cluster {c}'s")),
            point,
            index: p.i_type,
            len,
        })
    })
}

/// Built here or moved from another type, so no arrival layout has its shape.
fn built_here(carry: &Carry, type_name: &str) -> bool {
    !carry.arrived_as(type_name)
}

/// ⚠ Only a parent in this file: an entity with `flags` bit 3 set indexes
/// its parent map's array, which one file cannot see.
fn broken_hierarchy(entities: &[CEntityDefAny]) -> Option<Broken> {
    let len = entities.len();
    entities.iter().enumerate().find_map(|(entity, e)| {
        let parent = usize::try_from(e.parent_index).ok()?;
        if e.flags & (1 << 3) != 0 {
            None
        } else if parent >= len {
            Some(Broken::ParentOutOfRange {
                entity,
                parent,
                len,
            })
        } else {
            (parent == entity).then_some(Broken::ParentIsSelf { entity })
        }
    })
}

/// The first interior whose rooms, portals or entity sets index or pair what the interior does not hold.
fn broken_interior(archetypes: &[CBaseArchetypeDefAny]) -> Option<Broken> {
    archetypes.iter().enumerate().find_map(|(archetype, a)| {
        let CBaseArchetypeDefAny::CMloArchetypeDef(mlo) = a else {
            return None;
        };
        let what = interior::broken(mlo)?;
        Some(Broken::Interior { archetype, what })
    })
}

macro_rules! meta_origin {
    ($name:ident) => {
        impl HasOrigin for $name {
            fn origin(&self) -> &Origin {
                &self.carry.origin
            }

            fn origin_mut(&mut self) -> &mut Origin {
                &mut self.carry.origin
            }
        }
    };
}

meta_origin!(CMapData);
meta_origin!(CMapTypes);
meta_origin!(CScenarioPointRegion);

mod alternate;
pub mod alternate_types;
mod interior;
mod map_records;
mod ped;
mod ped_rbf;
pub mod ped_types;
pub mod weapon;
mod weapon_ammo;
mod weapon_archetypes;
mod weapon_blob;
mod weapon_components;
mod weapon_info;
pub mod weapon_types;
pub mod world_types;
pub use alternate::*;
pub use interior::{CMloEntitySet, CMloPortalDef, CMloRoomDef, CMloTimeCycleModifier};
pub use map_records::{
    save_far_lights, BoxOccluder, CCarGen, CDistantLODLight, CLODLight, CTimeCycleModifier,
    FloatXYZ, OccludeModel,
};
pub use ped::*;

impl CMapData {
    /// What the file it was read from states under `member`, a member no typed record names.
    pub fn carried(&self, member: &str) -> Option<&Value> {
        let key = name_hash(member);
        (self.carry.carried().find(|(held, _)| *held == key)).map(|(_, value)| value)
    }
}

impl File for CMapData {
    const EXTENSION: &'static str = "ymap";

    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let (mut root, arrival) = open_root(Self::EXTENSION, "CMapData", bytes, Self::read)?;
        root.carry.arrival = Some(Box::new(arrival));
        Ok(root)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let broken = broken_hierarchy(&self.entities)
            .or_else(|| map_records::broken(self).map(Broken::MapRecords));
        let reshaped = self.entities.iter().any(CEntityDefAny::reshaped)
            || map_records::holds_a_record_built_here(self);
        let write = || self.write();
        save_world_root(Self::EXTENSION, broken, write, &self.carry, reshaped)
    }
}

impl File for CMapTypes {
    const EXTENSION: &'static str = "ytyp";

    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let (mut root, arrival) = open_root(Self::EXTENSION, "CMapTypes", bytes, Self::read)?;
        root.carry.arrival = Some(Box::new(arrival));
        Ok(root)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let broken = broken_interior(&self.archetypes);
        let reshaped = self.archetypes.iter().any(|a| {
            a.reshaped()
                || matches!(a, CBaseArchetypeDefAny::CMloArchetypeDef(mlo)
                    if interior::holds_a_record_built_here(mlo))
        });
        let write = || self.write();
        save_world_root(Self::EXTENSION, broken, write, &self.carry, reshaped)
    }
}

impl File for CScenarioPointRegion {
    const EXTENSION: &'static str = "ymt";

    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let (mut root, arrival) =
            open_root(Self::EXTENSION, "CScenarioPointRegion", bytes, Self::read)?;
        root.carry.arrival = Some(Box::new(arrival));
        Ok(root)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let broken = broken_point_type(self);
        let graph = &self.chaining_graph;
        let reshaped = self
            .all_points()
            .any(|(_, p)| built_here(&p.carry, "CScenarioPoint"))
            || self
                .clusters
                .iter()
                .any(|c| built_here(&c.carry, "CScenarioPointCluster"))
            || graph
                .nodes
                .iter()
                .any(|n| built_here(&n.carry, "CScenarioChainingNode"))
            || graph
                .edges
                .iter()
                .any(|e| built_here(&e.carry, "CScenarioChainingEdge"))
            || graph
                .chains
                .iter()
                .any(|c| built_here(&c.carry, "CScenarioChain"));
        // ⚠ No class of a region is declared in code, so one built from nothing is saved against the installed schema.
        if self.carry.arrival.is_none() {
            return save_root(Self::EXTENSION, broken, self.write(), &self.carry, reshaped);
        }
        let write = || self.write();
        save_world_root(Self::EXTENSION, broken, write, &self.carry, reshaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_member_leaves_its_slot_as_it_was() {
        let mut slot = Value::Int(-1);
        u32::MAX.write(&mut slot).unwrap();
        assert_eq!(slot, Value::Int(-1));

        let mut slot = Value::Vec4([1.0, 2.0, 3.0, f32::from_bits(0x7fc0_0001)]);
        Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }
        .write(&mut slot)
        .unwrap();
        let Value::Vec4(v) = slot else { panic!() };
        assert_eq!(v[3].to_bits(), 0x7fc0_0001);

        let mut slot = Value::Hash(HashedString {
            raw: "resolved elsewhere".into(),
            hash: crate::string_hash(b"prop_bench_01a"),
        });
        let before = slot.clone();
        HashedString::new("prop_bench_01a")
            .write(&mut slot)
            .unwrap();
        assert_eq!(slot, before);
    }

    #[test]
    fn a_changed_member_keeps_its_slots_shape() {
        let mut slot = Value::Vec4([1.0, 2.0, 3.0, 9.0]);
        Vec3 {
            x: 1.0,
            y: 2.0,
            z: 4.0,
        }
        .write(&mut slot)
        .unwrap();
        assert_eq!(slot, Value::Vec4([1.0, 2.0, 4.0, 9.0]));

        let mut slot = Value::UInt(0);
        HashedString::new("hills").write(&mut slot).unwrap();
        assert_eq!(slot, Value::UInt(u64::from(crate::string_hash(b"hills"))));
    }

    #[test]
    fn a_name_equals_a_hash_by_its_string_hash() {
        let h = HashedString::from_hash(crate::string_hash(b"Prop_Bench_01A"));
        assert!(h == "prop_bench_01a");
        assert!(h != "prop_bench_01b");
    }

    #[test]
    fn an_element_comes_back_from_the_value_tree_it_saves_as() {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set("guid", Value::UInt(7));
        // A member no record types, which the element carries.
        inst.set("priorityLevel", Value::UInt(2));
        for value in [
            Value::Pointer(Some(Box::new(inst.clone()))),
            Value::Struct(Box::new(inst.clone())),
        ] {
            let mut element = CEntityDefAny::from_value(&value).unwrap();
            assert_eq!(element.guid, 7);
            assert_eq!(element.to_value().unwrap(), value);
            element.guid = 9;
            let changed = element.to_value().unwrap();
            assert_eq!(
                std::mem::discriminant(&changed),
                std::mem::discriminant(&value)
            );
            let (Value::Pointer(Some(saved)) | Value::Struct(saved)) = changed else {
                panic!("a struct, behind a pointer or not");
            };
            assert_eq!(saved.get("guid"), Some(&Value::UInt(9)));
            assert_eq!(saved.get("priorityLevel"), Some(&Value::UInt(2)));
        }
        assert!(CEntityDefAny::from_value(&Value::Pointer(None)).is_err());
        assert!(CEntityDefAny::from_value(&Value::UInt(1)).is_err());
        inst.type_name = "CMapData".into();
        let refused = CEntityDefAny::from_value(&Value::Struct(Box::new(inst))).unwrap_err();
        assert!(
            refused.to_string().contains("`CEntityDef` element"),
            "{refused}"
        );
    }

    #[test]
    fn a_member_spelt_as_its_number_reads_by_name() {
        let mut inst = MetaInstance::new(format!("0x{:08X}", literal_string_hash(b"CEntityDef")));
        inst.set(
            format!("0x{:08X}", literal_string_hash(b"guid")),
            Value::UInt(7),
        );
        assert_eq!(member::<u32>(&inst, "CEntityDef", "guid", 0), Ok(7));
        assert!(is_named(&inst.type_name, "CEntityDef"));
        assert_eq!(member::<u32>(&inst, "CEntityDef", "flags", 0), Ok(0));
    }

    #[test]
    fn a_member_its_type_table_lacks_reads_as_the_engines_init() {
        let entity = CEntityDef::read(&MetaInstance::new("CEntityDef")).unwrap();
        assert_eq!(entity.ambient_occlusion_multiplier, 255);
        assert_eq!(entity.artificial_ambient_occlusion, 255);
        assert_eq!(entity, CEntityDef::default());

        let timed = CTimeArchetypeDef::read(&MetaInstance::new("CTimeArchetypeDef")).unwrap();
        assert_eq!(timed.base.hd_texture_dist, 5.0);
        assert_eq!(timed, CTimeArchetypeDef::default());
    }
}
