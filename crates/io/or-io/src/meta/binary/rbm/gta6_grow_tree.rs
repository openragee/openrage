//! A GTA VI headerless RBM body written from a tree whose `AtArray64` or counted array members
//! hold more or fewer elements than the body stores: each such array first takes or loses the
//! elements that match no
//! stored one, by [`super::gta6_grow`]'s rule, an added element zero and an added pointer naming a
//! zeroed record of its class; then the tree is written into the grown blocks, a member an added
//! element leaves unset taking what its zero bytes read as.
//!
//! ⚠ An array grows only where the block that holds it is a record of the structure that declares
//! it, and its elements are structures or values whose size their type gives; a region that
//! changes size anywhere else is refused by the in-place writer as before.

use crate::container::rpf::{Rpf9Form, Rpf9Sections};
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::layout::{ContainerLayout, RbmLayout};
use crate::meta::binary::pso::table_form;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::Value;
use crate::meta::instance::MetaInstance;

use super::gta6::{open, write_sections};
use super::gta6_grow::{append_to_block, insert_element, remove_element, Gta6GrowError, COUNTED};
use super::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};

/// The name of the member that describes an array's element.
const ARRAY_INFO: u32 = 0x100;

/// An element added to an array or taken out of it.
enum Change {
    Insert(Added),
    Remove,
}

/// An added element's bytes.
enum Added {
    Zero(usize),
    /// A pointer to a zeroed record of `size` bytes appended to block `block`.
    Record {
        block: u32,
        size: usize,
    },
}

/// The `AtArray64` at `member` of block `block` changed at its element `index`.
struct Op {
    block: u32,
    member: usize,
    index: usize,
    change: Change,
}

fn other(why: impl std::fmt::Display) -> MetaError {
    MetaError::Other(format!("GTA VI RBM: {why}"))
}

fn grow(e: Gta6GrowError) -> MetaError {
    other(e)
}

/// A headerless resource's sections with `instance` written in, its arrays resized to it first.
pub fn write_grown(layout: &RbmLayout, instance: &MetaInstance) -> Result<(Vec<u8>, Vec<u8>)> {
    let _names = crate::meta::binary::hash_index::title_names();
    let model = &layout.model;
    let stored = table_form::decode(model)?;
    let root = model.root_block_id;
    let class = class_of(model, block_type(model, root)?)?;
    let mut ops = Vec::new();
    walk(model, &mut ops, class, root, 0, &stored, instance)?;
    if ops.is_empty() {
        return write_sections(layout, instance);
    }
    let split = layout.virtual_page_size.min(layout.decompressed.len());
    let mut sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: layout.decompressed[..split].to_vec(),
        physical: layout.decompressed[split..].to_vec(),
    };
    // Children come before what holds them, so a holder an array moves is moved last.
    for op in ops {
        apply(&mut sections, op)?;
    }
    let (decoded, grown) = open(&sections.virtual_section, &sections.physical)?;
    let ContainerLayout::Rbm(grown) = grown.container else {
        return Err(other(
            "the grown body no longer reads as a headerless resource",
        ));
    };
    let mut completed = instance.clone();
    complete(&mut completed, &decoded);
    let (v, p) = write_sections(&grown, &completed)?;
    open(&v, &p)?;
    Ok((v, p))
}

fn class_of(model: &RbmModel, ty: u32) -> Result<&RbmClass> {
    (model.classes.get(&ty)).ok_or_else(|| other(format!("no structure {ty:#010X} is declared")))
}

fn data(model: &RbmModel, block: u32) -> Result<&[u8]> {
    (i32::try_from(block).ok())
        .and_then(|id| model.block(id))
        .map(|b| b.data.as_slice())
        .ok_or_else(|| other(format!("no data block {block} is stored")))
}

fn block_type(model: &RbmModel, block: u32) -> Result<u32> {
    (i32::try_from(block).ok())
        .and_then(|id| model.block(id))
        .map(|b| b.data_type)
        .ok_or_else(|| other(format!("no data block {block} is stored")))
}

fn word(model: &RbmModel, block: u32, at: usize) -> Result<u64> {
    (data(model, block)?.get(at..at.saturating_add(8)))
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| other(format!("block {block} holds no word at {at:#X}")))
}

/// The block and offset a pointer inside data names.
fn target(raw: u64) -> Result<(u32, usize)> {
    if raw & 0xFFF == 0 || (raw >> 12) & 0xFF != 0 {
        return Err(other(format!("{raw:#X} is no pointer to a block")));
    }
    let at = usize::try_from(raw >> 20).map_err(|_| other(format!("{raw:#X} points too far")))?;
    Ok(((raw & 0xFFF) as u32, at))
}

/// The structure of `class` at `base` of block `block`, read as `stored` and edited to `edited`.
fn walk(
    model: &RbmModel,
    ops: &mut Vec<Op>,
    class: &RbmClass,
    block: u32,
    base: usize,
    stored: &MetaInstance,
    edited: &MetaInstance,
) -> Result<()> {
    let index = default_hash_index();
    for entry in &class.entries {
        if entry.name_hash == ARRAY_INFO {
            continue;
        }
        let name = index.name_or_literal(entry.name_hash);
        let (Some(s), Some(e)) = (stored.get(&name), edited.get(&name)) else {
            continue;
        };
        if s == e {
            continue;
        }
        let at = base.saturating_add(entry.offset as usize);
        match (entry.data_type, s, e) {
            (PsoType::Struct, Value::Struct(s), Value::Struct(e)) => {
                let inner = class_of(model, entry.referent_hash())?;
                walk(model, ops, inner, block, at, s, e)?;
            }
            (PsoType::Pointer64, Value::Pointer(Some(s)), Value::Pointer(Some(e))) => {
                let (to, offset) = target(word(model, block, at)?)?;
                let inner = class_of(model, block_type(model, to)?)?;
                walk(model, ops, inner, to, offset, s, e)?;
            }
            (PsoType::AtArray64 | PsoType::Unknown(COUNTED), Value::Array(s), Value::Array(e)) => {
                let (holder, member) = (Holder { block, at }, (class, entry));
                array(model, ops, holder, member, s, e)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Where an array's head lies.
#[derive(Clone, Copy)]
struct Holder {
    block: u32,
    at: usize,
}

/// The array of `entry` of `owner`, read as `stored` and edited to `edited`: what its elements
/// held in both changed, then its own count.
fn array(
    model: &RbmModel,
    ops: &mut Vec<Op>,
    holder: Holder,
    (owner, entry): (&RbmClass, &RbmClassEntry),
    stored: &[Value],
    edited: &[Value],
) -> Result<()> {
    let element = (owner.entry(entry.contents_index()))
        .ok_or_else(|| other("an array's element is not declared"))?;
    // A counted array's pointer follows its count and capacity.
    let pointer_at = match entry.data_type == PsoType::Unknown(COUNTED) {
        true => 8,
        false => 0,
    };
    let raw = word(model, holder.block, holder.at + pointer_at)?;
    let (ns, ne) = (stored.len(), edited.len());
    // Each stored element still held, with the edited one it becomes; an element that matches
    // none is added or taken out, only one of the two in an array.
    let (mut kept, mut inserted, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < ns || j < ne {
        let same = i < ns && j < ne && stored[i] == edited[j];
        if !same && ne - j > ns - i {
            inserted.push(j);
            j += 1;
        } else if !same && ns - i > ne - j {
            removed.push(i);
            i += 1;
        } else {
            kept.push((i, j));
            (i, j) = (i + 1, j + 1);
        }
    }
    for (i, j) in kept {
        let (s, e) = (&stored[i], &edited[j]);
        if s == e {
            continue;
        }
        let (elements, first) = target(raw)?;
        match (element.data_type, s, e) {
            (PsoType::Struct, Value::Struct(s), Value::Struct(e)) => {
                let class = class_of(model, element.referent_hash())?;
                let at = first.saturating_add(i.saturating_mul(class.data_size as usize));
                walk(model, ops, class, elements, at, s, e)?;
            }
            (PsoType::Pointer64, Value::Pointer(Some(s)), Value::Pointer(Some(e))) => {
                let at = first.saturating_add(8 * i);
                let (to, offset) = target(word(model, elements, at)?)?;
                let class = class_of(model, block_type(model, to)?)?;
                walk(model, ops, class, to, offset, s, e)?;
            }
            _ => {}
        }
    }
    // Each added where the array then holds the elements before it; each taken out last first.
    let op = |index, change| Op {
        block: holder.block,
        member: holder.at,
        index,
        change,
    };
    for j in inserted {
        let bytes = added(model, element, raw, ns, &edited[j])?;
        ops.push(op(j, Change::Insert(bytes)));
    }
    ops.extend(removed.into_iter().rev().map(|i| op(i, Change::Remove)));
    Ok(())
}

/// The bytes element `e`, added to an array of `count` stored elements that `raw` points at,
/// starts as.
fn added(
    model: &RbmModel,
    element: &RbmClassEntry,
    raw: u64,
    count: usize,
    e: &Value,
) -> Result<Added> {
    let code = element.data_type.to_byte();
    match element.data_type {
        PsoType::Struct => {
            let class = class_of(model, element.referent_hash())?;
            Ok(Added::Zero(class.data_size as usize))
        }
        PsoType::Pointer64 => {
            let Value::Pointer(Some(inst)) = e else {
                return Err(other(
                    "an added element of an array of pointers names no structure, so no record is laid out for it: give it one",
                ));
            };
            // A stored element's record of the class the added one names gives the block it joins.
            let (elements, first) = target(raw)?;
            let index = default_hash_index();
            for i in 0..count {
                let Ok((block, _)) = target(word(model, elements, first.saturating_add(8 * i))?)
                else {
                    continue;
                };
                let ty = block_type(model, block)?;
                if index.name_or_literal(ty) == inst.type_name {
                    let size = class_of(model, ty)?.data_size as usize;
                    return Ok(Added::Record { block, size });
                }
            }
            Err(other(format!(
                "no stored element of the array is a {}, so no block holds a record of its class to add one to",
                inst.type_name
            )))
        }
        // A zeroed value is its type's null form: zero, a null pointer, an empty array.
        ty => table_form::fixed_width(ty).map(Added::Zero).ok_or_else(|| {
            other(format!(
                "an array of type {code:#04X} elements is not grown: its element's size does not follow from its type"
            ))
        }),
    }
}

fn apply(s: &mut Rpf9Sections, op: Op) -> Result<()> {
    let bytes = match op.change {
        Change::Remove => return remove_element(s, op.block, op.member, op.index).map_err(grow),
        Change::Insert(Added::Zero(len)) => vec![0; len],
        Change::Insert(Added::Record { block, size }) => {
            let at = append_to_block(s, block, 16, &vec![0; size]).map_err(grow)?;
            (u64::from(block) | (at as u64) << 20)
                .to_le_bytes()
                .to_vec()
        }
    };
    insert_element(s, op.block, op.member, op.index, &bytes).map_err(grow)
}

/// `edited` with each member it lacks taken from `decoded`, the same tree read from the grown body.
fn complete(edited: &mut MetaInstance, decoded: &MetaInstance) {
    for (name, value) in &decoded.fields {
        match edited.fields.iter().position(|(n, _)| n == name) {
            Some(k) => complete_value(&mut edited.fields[k].1, value),
            None => edited.fields.push((name.clone(), value.clone())),
        }
    }
}

fn complete_value(edited: &mut Value, decoded: &Value) {
    match (edited, decoded) {
        (
            Value::Struct(e) | Value::Pointer(Some(e)),
            Value::Struct(d) | Value::Pointer(Some(d)),
        ) => complete(e, d),
        (Value::Array(e), Value::Array(d)) if e.len() == d.len() => {
            for (e, d) in e.iter_mut().zip(d) {
                complete_value(e, d);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::gta6_grow::tests::{body, member, pointer};
    use super::*;
    use crate::meta::binary::layout::MetaLayout;

    const ROOT: u32 = 0x4646_0001;
    const ELEMENT: u32 = 0x4646_0002;
    const FIRST: u32 = 0x4646_0010;
    const SECOND: u32 = 0x4646_0011;
    /// The type of a block of pointers.
    const POINTERS: u32 = 7;

    fn name(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    fn element(i: i32, f: f32) -> Vec<u8> {
        [i.to_le_bytes(), f.to_le_bytes()].concat()
    }

    fn pair(i: i64, f: f64) -> MetaInstance {
        let mut e = MetaInstance::new(name(ELEMENT));
        e.fields.push((name(0x4646_0020), Value::Int(i)));
        e.fields.push((name(0x4646_0021), Value::Float(f)));
        e
    }

    fn pair_class() -> (u32, u32, Vec<[u8; 16]>) {
        let members = vec![
            member(0x4646_0020, 0, 0x14, 0, 0),
            member(0x4646_0021, 4, 0x21, 0, 0),
        ];
        (ELEMENT, 8, members)
    }

    /// A root of two arrays of 8-byte structures in one block: `FIRST` holds two at 0, `SECOND`
    /// one at 16, the block's last region.
    fn two_arrays() -> Rpf9Sections {
        let root = (
            ROOT,
            0x30,
            vec![
                member(FIRST, 0x00, 0x52, 1, 0),
                member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
                member(SECOND, 0x10, 0x52, 3, 0),
                member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
                member(0x4646_0012, 0x20, 0x15, 0, 0),
            ],
        );
        let mut data = vec![0u8; 0x30];
        data[0x00..0x08].copy_from_slice(&pointer(2, 0).to_le_bytes());
        data[0x08..0x0C].copy_from_slice(&[2, 0, 2, 0]);
        data[0x10..0x18].copy_from_slice(&pointer(2, 16).to_le_bytes());
        data[0x18..0x1C].copy_from_slice(&[1, 0, 1, 0]);
        data[0x20..0x24].copy_from_slice(&0x77u32.to_le_bytes());
        let elements = [element(10, 0.25), element(-20, 0.5), element(30, 0.75)].concat();
        body(
            &[root, pair_class()],
            &[(ROOT, data), (ELEMENT, elements)],
            1,
        )
    }

    /// A root whose one array points at two records of a block of their own.
    fn pointed() -> Rpf9Sections {
        let root = (
            ROOT,
            0x10,
            vec![
                member(FIRST, 0x00, 0x52, 1, 0),
                member(ARRAY_INFO, 0, 0x07, 0, 0),
            ],
        );
        let mut data = vec![0u8; 0x10];
        data[0x00..0x08].copy_from_slice(&pointer(2, 0).to_le_bytes());
        data[0x08..0x0C].copy_from_slice(&[2, 0, 2, 0]);
        let pointers = [pointer(3, 0), pointer(3, 8)]
            .map(u64::to_le_bytes)
            .concat();
        let records = [element(10, 0.25), element(-20, 0.5)].concat();
        body(
            &[root, pair_class()],
            &[(ROOT, data), (POINTERS, pointers), (ELEMENT, records)],
            1,
        )
    }

    fn opened(s: &Rpf9Sections) -> (MetaInstance, RbmLayout) {
        let (instance, MetaLayout { container, .. }) =
            open(&s.virtual_section, &s.physical).expect("it opens");
        let ContainerLayout::Rbm(layout) = container else {
            panic!("a headerless RBM layout");
        };
        (instance, *layout)
    }

    fn items(tree: &mut MetaInstance, array: u32) -> &mut Vec<Value> {
        match tree.get_mut(&name(array)) {
            Some(Value::Array(items)) => items,
            _ => panic!("an array"),
        }
    }

    fn structs(pairs: &[(i64, f64)]) -> Vec<Value> {
        (pairs.iter())
            .map(|&(i, f)| Value::Struct(Box::new(pair(i, f))))
            .collect()
    }

    #[test]
    fn an_unchanged_tree_writes_the_stored_bytes_and_a_grown_one_reads_back_at_its_fixed_point() {
        let s = two_arrays();
        let (mut tree, layout) = opened(&s);
        let written = write_grown(&layout, &tree).expect("unchanged");
        assert_eq!(written, (s.virtual_section.clone(), s.physical.clone()));

        // An element given values at the front of SECOND, one with none at its end, and FIRST's
        // first taken out.
        let second = items(&mut tree, SECOND);
        second.insert(0, Value::Struct(Box::new(pair(5, 2.0))));
        second.push(Value::Struct(Box::new(MetaInstance::new(name(ELEMENT)))));
        items(&mut tree, FIRST).remove(0);
        let (v, p) = write_grown(&layout, &tree).expect("grown");

        let (mut again, layout) = opened(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v.clone(),
            physical: p.clone(),
        });
        assert_eq!(*items(&mut again, FIRST), structs(&[(-20, 0.5)]));
        assert_eq!(
            *items(&mut again, SECOND),
            structs(&[(5, 2.0), (30, 0.75), (0, 0.0)])
        );
        assert_eq!(write_grown(&layout, &again).expect("again"), (v, p));
    }

    #[test]
    fn an_array_of_eight_byte_values_takes_an_element_and_reads_back_at_its_fixed_point() {
        const U64: u8 = 0x17;
        let root = (
            ROOT,
            0x10,
            vec![
                member(FIRST, 0x00, 0x52, 1, 0),
                member(ARRAY_INFO, 0, U64, 0, 0),
            ],
        );
        let mut data = vec![0u8; 0x10];
        data[0x00..0x08].copy_from_slice(&pointer(2, 0).to_le_bytes());
        data[0x08..0x0C].copy_from_slice(&[2, 0, 2, 0]);
        let values = [0x1111_2222_3333_4444u64, 5].map(u64::to_le_bytes).concat();
        let s = body(&[root], &[(ROOT, data), (u32::from(U64), values)], 1);
        let (mut tree, layout) = opened(&s);
        items(&mut tree, FIRST).push(Value::UInt(0xFEED_0000_0000_0009));
        let (v, p) = write_grown(&layout, &tree).expect("grown");
        let (mut again, layout) = opened(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v.clone(),
            physical: p.clone(),
        });
        let want: Vec<Value> = [0x1111_2222_3333_4444u64, 5, 0xFEED_0000_0000_0009]
            .map(Value::UInt)
            .into();
        assert_eq!(*items(&mut again, FIRST), want);
        assert_eq!(write_grown(&layout, &again).expect("again"), (v, p));
    }

    #[test]
    fn an_added_pointer_names_a_zeroed_record_beside_its_class_and_a_null_one_is_refused() {
        let s = pointed();
        let (mut tree, layout) = opened(&s);
        let first = items(&mut tree, FIRST);
        first.insert(1, Value::Pointer(Some(Box::new(pair(7, 1.5)))));
        let (v, p) = write_grown(&layout, &tree).expect("grown");
        let (mut again, _) = opened(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: p,
        });
        let pointers = |pairs: &[(i64, f64)]| -> Vec<Value> {
            (pairs.iter())
                .map(|&(i, f)| Value::Pointer(Some(Box::new(pair(i, f)))))
                .collect()
        };
        assert_eq!(
            *items(&mut again, FIRST),
            pointers(&[(10, 0.25), (7, 1.5), (-20, 0.5)])
        );

        let (mut tree, layout) = opened(&s);
        items(&mut tree, FIRST).push(Value::Pointer(None));
        let err = write_grown(&layout, &tree)
            .expect_err("refused")
            .to_string();
        assert!(err.contains("names no structure"), "{err}");
        // Taking one out shrinks the array and leaves the other's record as it was.
        let (mut tree, layout) = opened(&s);
        items(&mut tree, FIRST).remove(0);
        let (v, p) = write_grown(&layout, &tree).expect("shrunk");
        let (mut again, _) = opened(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: p,
        });
        assert_eq!(*items(&mut again, FIRST), pointers(&[(-20, 0.5)]));
    }
}
