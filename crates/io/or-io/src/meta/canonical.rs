//! The binary a metadata document converges to through XML.
//!
//! Exporting carries no types, so importing takes them from the struct
//! definitions and lays the result out from the value tree alone. The first
//! export and import may move bytes; every one after it must not. What is
//! asked is `import(export(C1)) == C1` for `C1 = import(export(O))`, and never
//! `C1 == O`.
//!
//! ⚠ **The one type an export states is a pointer's target**, because a
//! pointer is polymorphic and its declaration names only the base. Everything
//! else comes from the schema, the [`crate::meta::shipped`] definitions shipped
//! type tables add to it, and the type tables of a donor or an install
//! ([`type_tables`]), so a document whose root type none of them declares is
//! refused at import rather than guessed at.
//!
//! ⚠ RBF is the exception to schema typing and not an oversight: it is binary
//! XML, so [`crate::meta::binary::rbf::xml`] transcribes it and asks no schema.

use std::borrow::Cow;
use std::collections::BTreeSet;

use crate::meta::binary::layout::{ContainerLayout, MetaLayout};
use crate::meta::binary::rbf::xml as rbf_xml;
use crate::meta::binary::synth::{pso_layout_for_instance, rbm_layout_for_instance};
use crate::meta::binary::{parse_meta_binary_full, type_tables, write_meta_binary, BinaryFormat};
use crate::meta::diagnostics::Diagnostics;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::{parse_meta, write_meta, MetaInstance, Value};
use crate::meta::schema::SchemaLibrary;
use crate::meta::shipped;

/// The XML a document read with `layout` exports as.
pub fn export(instance: &MetaInstance, layout: &MetaLayout) -> Result<String> {
    match &layout.container {
        ContainerLayout::Rbf(rbf) => rbf_xml::to_xml(instance, rbf),
        _ => write_meta(instance),
    }
}

/// Encode `xml` as `format`, typed by `library` and laid out from its values.
///
/// `donor` types what `library` does not declare, through [`typed_by`], and
/// otherwise reaches an RBM class's `signature` word and nothing else — see
/// [`rbm_layout_for_instance`].
pub fn import(
    library: &SchemaLibrary,
    xml: &[u8],
    format: BinaryFormat,
    donor: Option<&MetaLayout>,
) -> Result<Vec<u8>> {
    let (instance, layout) = import_tree(library, xml, format, donor)?;
    write_meta_binary(format, Some(&layout), &instance)
}

/// The value tree and synthesised layout `xml` imports as.
pub fn import_tree(
    library: &SchemaLibrary,
    xml: &[u8],
    format: BinaryFormat,
    donor: Option<&MetaLayout>,
) -> Result<(MetaInstance, MetaLayout)> {
    if format == BinaryFormat::Rbf {
        let (instance, rbf) = rbf_xml::from_xml(xml)?;
        let layout = MetaLayout {
            format,
            container: ContainerLayout::Rbf(rbf),
        };
        return Ok((instance, layout));
    }
    let typed = typed_by(library, donor);
    if typed.structs.is_empty() {
        return Err(MetaError::Other(no_definitions(format)));
    }
    let (typed, instance) = parse(&typed, xml)?;
    refuse_undeclared(library, &instance)?;
    let layout = layout_for(&typed, &instance, format, donor)?;
    Ok((instance, layout))
}

/// `library` with what `donor`'s type table and the installed
/// [`type_tables`] state that it does not declare; the snapshot wins.
pub fn typed_by<'a>(
    library: &'a SchemaLibrary,
    donor: Option<&MetaLayout>,
) -> Cow<'a, SchemaLibrary> {
    let mut tables = donor.map(type_tables::of_layout).unwrap_or_default();
    // ⚠ Only behind a PSO, whose table states no signature: a world class the donor names without stating it (an empty `entities`)
    // is typed as `typed::world_types` declares it, which is what a typed save of that donor lays the class out as.
    if matches!(donor.map(|d| &d.container), Some(ContainerLayout::Pso(pso)) if !pso.is_gta6()) {
        type_tables::fill(&mut tables, crate::meta::typed::world_types::declared());
    }
    if let Some(install) = type_tables::installed() {
        type_tables::fill(&mut tables, install);
    }
    type_tables::extend(library, &tables)
}

/// Refuse a member of a type only a type table declares, which no table
/// states: laying it out would be a guess.
fn refuse_undeclared(library: &SchemaLibrary, instance: &MetaInstance) -> Result<()> {
    let declared = |type_name: &str| {
        library.resolve_any(type_name).is_some()
            || shipped::shipped_types().iter().any(|d| d.name == type_name)
    };
    fn first(inst: &MetaInstance, declared: &dyn Fn(&str) -> bool) -> Option<String> {
        if let Some(extra) = inst.extras.first().filter(|_| !declared(&inst.type_name)) {
            return Some(format!("{}.{}", inst.type_name, extra.tag));
        }
        inst.fields.iter().find_map(|(_, v)| in_value(v, declared))
    }
    fn in_value(value: &Value, declared: &dyn Fn(&str) -> bool) -> Option<String> {
        match value {
            Value::Struct(inner) | Value::Pointer(Some(inner)) => first(inner, declared),
            Value::Array(items) => items.iter().find_map(|v| in_value(v, declared)),
            Value::Map(pairs) => pairs
                .iter()
                .find_map(|(k, v)| in_value(k, declared).or_else(|| in_value(v, declared))),
            _ => None,
        }
    }
    match first(instance, &declared) {
        Some(member) => Err(MetaError::Other(format!(
            "'{member}' is a member no struct definition or type table declares, so it has \
             no layout to import into"
        ))),
        None => Ok(()),
    }
}

/// The value tree `xml` imports as, and the library that typed it: `library`
/// with the [`crate::meta::shipped`] definitions the document names.
///
/// ⚠ **Enhanced's members are asked for by the document, never assumed.**
/// A file carrying one is typed with all of them; a file carrying none is
/// typed exactly as the PSC corpus types it, so a Legacy document cannot
/// gain a member it does not hold.
pub fn parse<'a>(
    library: &'a SchemaLibrary,
    xml: &[u8],
) -> Result<(Cow<'a, SchemaLibrary>, MetaInstance)> {
    let shipped = shipped::library(library, false);
    let instance = parse_meta(&shipped, xml)?;
    if !shipped::carries_gen9_member(&shipped, &instance) {
        return Ok((shipped, instance));
    }
    let gen9 = shipped::library(library, true);
    let instance = parse_meta(&gen9, xml)?;
    Ok((gen9, instance))
}

/// A layout for `instance` derived from `library` alone.
pub fn layout_for(
    library: &SchemaLibrary,
    instance: &MetaInstance,
    format: BinaryFormat,
    donor: Option<&MetaLayout>,
) -> Result<MetaLayout> {
    let container = match format {
        BinaryFormat::Pso => ContainerLayout::Pso(pso_layout_for_instance(library, instance)?),
        BinaryFormat::Rbm => {
            let model = match donor.map(|d| &d.container) {
                Some(ContainerLayout::Rbm(r)) => Some(&r.model),
                _ => None,
            };
            ContainerLayout::Rbm(Box::new(rbm_layout_for_instance(library, instance, model)?))
        }
        BinaryFormat::Rbf => ContainerLayout::Rbf(Default::default()),
    };
    Ok(MetaLayout { format, container })
}

/// A document that reached its canonical form.
#[derive(Debug, Clone)]
pub struct Converged {
    /// `C1`, which is also `C2`.
    pub bytes: Vec<u8>,
    /// Whether the first cycle changed what the XML imports as, which is a
    /// value the export could not carry rather than a placement.
    ///
    /// ⚠ **Compared as imported values, not as text.** A member read as one
    /// run of bytes and written back as an array of `u8` exports differently
    /// and imports identically; comparing text called that a loss in 670
    /// occluder maps.
    pub values_moved: bool,
    /// `Type.member` for every member the file carries and the schema does
    /// not declare, which the import can only drop.
    pub undeclared: BTreeSet<String>,
    /// `Type.member` for every member the first cycle stored as a different
    /// kind of value, because the schema describes it differently from the
    /// file — a pointer to bytes laid out as an `atArray`, say.
    pub reshaped: BTreeSet<String>,
}

/// Whether two imported trees hold the same values: a hash by its number, a
/// float by its bits, and a run of bytes the same as an array of them.
fn same_instance(a: &MetaInstance, b: &MetaInstance) -> bool {
    a.type_name == b.type_name
        && a.extras == b.extras
        && a.fields.len() == b.fields.len()
        && a.fields
            .iter()
            .zip(&b.fields)
            .all(|((na, va), (nb, vb))| na == nb && same_value(va, vb))
}

fn same_value(a: &Value, b: &Value) -> bool {
    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|f| f.to_bits()).collect()
    }
    fn bytes_of(v: &Value) -> Option<Vec<u64>> {
        match v {
            Value::ByteArray(b) => Some(b.iter().map(|x| u64::from(*x)).collect()),
            Value::Array(items) => items
                .iter()
                .map(|i| match i {
                    Value::UInt(n) => Some(*n),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    }
    match (a, b) {
        (Value::Hash(x), Value::Hash(y)) => x.hash == y.hash,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Vec2(x), Value::Vec2(y)) => bits(x) == bits(y),
        (Value::Vec3(x), Value::Vec3(y)) => bits(x) == bits(y),
        (Value::Vec4(x), Value::Vec4(y)) => bits(x) == bits(y),
        (Value::Struct(x), Value::Struct(y)) => same_instance(x, y),
        (Value::Pointer(Some(x)), Value::Pointer(Some(y))) => same_instance(x, y),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q))
        }
        (Value::Map(x), Value::Map(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((ka, va), (kb, vb))| same_value(ka, kb) && same_value(va, vb))
        }
        (Value::ByteArray(_), Value::Array(_)) | (Value::Array(_), Value::ByteArray(_)) => {
            bytes_of(a).is_some() && bytes_of(a) == bytes_of(b)
        }
        _ => a == b,
    }
}

/// `Type.member` wherever `a` and `b` hold a different kind of value.
fn reshaped_members(a: &MetaInstance, b: &MetaInstance, out: &mut BTreeSet<String>) {
    for (name, va) in &a.fields {
        let Some(vb) = b.get(name) else {
            continue;
        };
        if std::mem::discriminant(va) != std::mem::discriminant(vb) {
            out.insert(format!("{}.{name}", a.type_name));
            continue;
        }
        match (va, vb) {
            (Value::Struct(x), Value::Struct(y))
            | (Value::Pointer(Some(x)), Value::Pointer(Some(y))) => reshaped_members(x, y, out),
            (Value::Array(x), Value::Array(y)) => {
                for (p, q) in x.iter().zip(y) {
                    if let (Value::Struct(s), Value::Struct(t))
                    | (Value::Pointer(Some(s)), Value::Pointer(Some(t))) = (p, q)
                    {
                        reshaped_members(s, t, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `Type.member` for every element an import set aside because the schema
/// does not declare it.
pub fn undeclared_members(instance: &MetaInstance) -> BTreeSet<String> {
    fn walk(inst: &MetaInstance, out: &mut BTreeSet<String>) {
        for extra in &inst.extras {
            out.insert(format!("{}.{}", inst.type_name, extra.tag));
        }
        for (_, value) in &inst.fields {
            walk_value(value, out);
        }
    }
    fn walk_value(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Struct(inner) | Value::Pointer(Some(inner)) => walk(inner, out),
            Value::Array(items) => items.iter().for_each(|v| walk_value(v, out)),
            Value::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                walk_value(k, out);
                walk_value(v, out);
            }),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(instance, &mut out);
    out
}

/// Where a document stopped on the way to its canonical form.
#[derive(Debug, thiserror::Error)]
pub enum NotCanonical {
    #[error("the original does not read: {0}")]
    Unreadable(MetaError),
    #[error("the first export and import is refused: {0}")]
    FirstCycle(MetaError),
    #[error("the first cycle's output does not read: {0}")]
    FirstUnreadable(MetaError),
    #[error("the second export and import is refused: {0}")]
    SecondCycle(MetaError),
    #[error("not a fixed point: C1 is {first} bytes, C2 is {second}, first difference at {at:#X}")]
    Moved {
        first: usize,
        second: usize,
        at: usize,
    },
}

impl NotCanonical {
    /// Whether the property was never reached, as against reached and failed.
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Unreadable(_) | Self::FirstCycle(_))
    }
}

/// Export and import `bytes` twice and require the second cycle to change
/// nothing.
///
/// ⚠ **Each cycle is typed by the type table of the bytes it exports**, behind
/// `library`, so with no snapshot a document converges on what it states.
pub fn converge(
    library: &SchemaLibrary,
    bytes: &[u8],
) -> std::result::Result<Converged, NotCanonical> {
    converge_typed(library, bytes, |typed, xml, format| {
        import(typed, xml, format, None)
    })
}

/// [`converge`], importing through `import` so a layer above can put its own
/// refusals in front of the encode.
pub fn converge_with(
    library: &SchemaLibrary,
    bytes: &[u8],
    import: impl Fn(&[u8], BinaryFormat) -> Result<Vec<u8>>,
) -> std::result::Result<Converged, NotCanonical> {
    converge_typed(library, bytes, |_, xml, format| import(xml, format))
}

fn converge_typed(
    library: &SchemaLibrary,
    bytes: &[u8],
    import: impl Fn(&SchemaLibrary, &[u8], BinaryFormat) -> Result<Vec<u8>>,
) -> std::result::Result<Converged, NotCanonical> {
    let read = |b: &[u8]| {
        let mut diags = Diagnostics::lenient();
        parse_meta_binary_full(library, b, &mut diags)
    };
    let cycle = |instance: &MetaInstance, layout: &MetaLayout| -> Result<(String, Vec<u8>)> {
        if let ContainerLayout::Pso(p) = &layout.container {
            #[cfg(not(feature = "gta6"))]
            use crate::meta::binary::pso::table_form::TableFormError;
            // ⚠ The table form's types are in no schema, so its XML comes back read without one
            // and is laid out by the document's own tables.
            if p.is_table_form() {
                let xml = export(instance, layout)?;
                let tree = crate::meta::instance::parse_schemaless(xml.as_bytes())?;
                let bytes =
                    crate::meta::binary::write_meta_binary(layout.format, Some(layout), &tree)?;
                return Ok((xml, bytes));
            }
            if p.is_gta6() {
                // A P-form's XML is typed by its own tables and laid out anew by the measured rules.
                #[cfg(feature = "gta6")]
                {
                    let xml = export(instance, layout)?;
                    let typed = typed_by(library, Some(layout));
                    let (_, tree) = parse(&typed, xml.as_bytes())?;
                    let bytes = crate::meta::binary::pso::pform_layout::write(p, &tree)?;
                    return Ok((xml, bytes));
                }
                #[cfg(not(feature = "gta6"))]
                return Err(TableFormError::NoLayoutRules.into());
            }
        }
        let xml = export(instance, layout)?;
        let bytes = import(
            &typed_by(library, Some(layout)),
            xml.as_bytes(),
            layout.format,
        )?;
        Ok((xml, bytes))
    };

    let (original, layout) = read(bytes).map_err(NotCanonical::Unreadable)?;
    let (xml_0, first) = cycle(&original, &layout).map_err(NotCanonical::FirstCycle)?;
    let (again, layout_1) = read(&first).map_err(NotCanonical::FirstUnreadable)?;
    let (xml_1, second) = cycle(&again, &layout_1).map_err(NotCanonical::SecondCycle)?;

    if first != second {
        let at = first
            .iter()
            .zip(&second)
            .position(|(a, b)| a != b)
            .unwrap_or(first.len().min(second.len()));
        return Err(NotCanonical::Moved {
            first: first.len(),
            second: second.len(),
            at,
        });
    }
    let mut reshaped = BTreeSet::new();
    reshaped_members(&original, &again, &mut reshaped);
    if layout.format == BinaryFormat::Rbf {
        return Ok(Converged {
            bytes: first,
            values_moved: xml_0 != xml_1,
            undeclared: BTreeSet::new(),
            reshaped,
        });
    }
    let (typed_0, typed_1) = (
        typed_by(library, Some(&layout)),
        typed_by(library, Some(&layout_1)),
    );
    let (values_moved, undeclared) = match (
        parse(&typed_0, xml_0.as_bytes()),
        parse(&typed_1, xml_1.as_bytes()),
    ) {
        (Ok((_, t0)), Ok((_, t1))) => {
            let moved = !same_instance(&t0, &t1);
            let undeclared = if moved {
                undeclared_members(&t0)
            } else {
                BTreeSet::new()
            };
            (moved, undeclared)
        }
        _ => (true, BTreeSet::new()),
    };
    Ok(Converged {
        bytes: first,
        values_moved,
        undeclared,
        reshaped,
    })
}

/// Why a canonical import with no donor cannot be asked for, or `None` when it
/// can.
///
/// ⚠ **The schema snapshot is gitignored**, so a clean checkout has struct
/// definitions only from a donor's type table or an installed install's.
pub fn unavailable(library: &SchemaLibrary, format: BinaryFormat) -> Option<String> {
    (format != BinaryFormat::Rbf
        && library.structs.is_empty()
        && type_tables::installed().is_none())
    .then(|| no_definitions(format))
}

fn no_definitions(format: BinaryFormat) -> String {
    format!(
        "importing XML as {} needs struct definitions: no schema snapshot is installed (it is \
         not part of the repository; see crates/io/or-io/src/meta/README.md), no install's \
         type tables were read, and no file was given to take them from",
        format.label()
    )
}
