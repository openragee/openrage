//! The container read: the words of the header, each table it points at and the pooled data,
//! with every rule the programs of the game keep held against the bytes.
//!
//! The file is one page mapped at [`BASE`]: a pointer is a 64-bit word of `BASE` plus a file
//! offset, and 0 is no pointer. A table of records is a count and a pointer in the header; what
//! the records point at in turn (a vtable, a base list, an enum's values) is in the pooled data.

use std::ops::Range;

use super::{
    functions, Area, EntryRow, FieldRow, ImportRow, MethodRow, NativeRow, Program, ProgramError,
    SlotRow, TypeRow,
};

const BASE: u64 = 0x5000_0000;
const HEADER: usize = 0x180;
/// The format and the build whose tables and instruction numbers are read.
const FORMAT: u32 = 0x6F;
const BUILD: u64 = 0x5936_CAB2_EADB_CE67;
/// The word that stands for no offset and for no type.
const NONE: u32 = u32::MAX;
/// The bytes of one record of a table.
const TYPE: usize = 136;
const METHOD: usize = 32;
const FIELD: usize = 24;
const IMPORT: usize = 24;
const ENTRY: usize = 12;
const REFLECTION: usize = 20;
/// The header words that hold nothing.
const RESERVED: &[usize] = &[
    0x34, 0x44, 0x54, 0x64, 0x74, 0x84, 0x94, 0x9C, 0xA4, 0xAC, 0xB4, 0xEC, 0xF4, 0x104, 0x108,
    0x10C, 0x134, 0x138, 0x13C, 0x14C, 0x150, 0x154, 0x158, 0x15C, 0x164, 0x174, 0x178, 0x17C,
];
/// The header word that counts the third group of reflection records.
const THIRD_GROUP: usize = 0xD4;

// ⚠ The three readers index: each is called inside bytes whose length was checked first, the
// header, a record of a table or a run that `span` or `pooled` gave.
fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from(u16_at(bytes, at)) | (u32::from(u16_at(bytes, at + 2)) << 16)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32_at(bytes, at)) | (u64::from(u32_at(bytes, at + 4)) << 32)
}

/// An offset or a type id, `None` for the word that stands for none.
fn some(word: u32) -> Option<u32> {
    (word != NONE).then_some(word)
}

fn refuse<T>(at: usize, what: impl Into<String>) -> Result<T, ProgramError> {
    let what = what.into();
    Err(ProgramError::Malformed { at, what })
}

/// The bytes of the file, and the reads every table goes through.
#[derive(Clone, Copy)]
struct File<'a> {
    bytes: &'a [u8],
}

impl<'a> File<'a> {
    /// `count` records of `record` bytes at file offset `at`, refused where the file ends before them.
    fn span(
        self,
        at: usize,
        count: usize,
        record: usize,
        what: &'static str,
    ) -> Result<&'a [u8], ProgramError> {
        let end = count.checked_mul(record).and_then(|n| n.checked_add(at));
        let found = end.and_then(|end| self.bytes.get(at..end));
        found.ok_or(ProgramError::Truncated {
            what,
            needed: end.map_or(usize::MAX, |end| end.saturating_sub(1)),
            len: self.bytes.len(),
        })
    }

    /// The file offset the pointer at `at` holds, `None` for a null one.
    fn pointer(self, at: usize, what: &str) -> Result<Option<usize>, ProgramError> {
        let word = u64_at(self.bytes, at);
        let offset = word.checked_sub(BASE).and_then(|o| usize::try_from(o).ok());
        match offset {
            _ if word == 0 => Ok(None),
            Some(offset) if offset <= self.bytes.len() => Ok(Some(offset)),
            _ => refuse(
                at,
                format!("the pointer to {what} is {word:#X}, outside the file"),
            ),
        }
    }

    /// The file offset the pointer at `at` holds; a null one is refused.
    fn required(self, at: usize, what: &str) -> Result<usize, ProgramError> {
        match self.pointer(at, what)? {
            Some(offset) => Ok(offset),
            None => refuse(at, format!("the pointer to {what} is null")),
        }
    }

    /// `count` records at the pointer at `at`, which is null exactly when there are none.
    fn table(
        self,
        at: usize,
        count: usize,
        record: usize,
        what: &'static str,
    ) -> Result<Table<'a>, ProgramError> {
        match self.pointer(at, what)? {
            Some(start) if count > 0 => {
                let bytes = self.span(start, count, record, what)?;
                Ok(Table {
                    at: start,
                    record,
                    bytes,
                })
            }
            None if count == 0 => Ok(Table {
                at: 0,
                record,
                bytes: &[],
            }),
            _ => refuse(
                at,
                format!(
                    "{what}: {count} stated, and the pointer is null exactly when there are none"
                ),
            ),
        }
    }

    /// `count` records at the pointer at `at`, where a null pointer is a table of none.
    fn absent_or(
        self,
        at: usize,
        count: usize,
        record: usize,
        what: &'static str,
    ) -> Result<Table<'a>, ProgramError> {
        match self.pointer(at, what)? {
            Some(start) => {
                let bytes = self.span(start, count, record, what)?;
                Ok(Table {
                    at: start,
                    record,
                    bytes,
                })
            }
            None if count == 0 => Ok(Table {
                at: 0,
                record,
                bytes: &[],
            }),
            None => refuse(
                at,
                format!("the pointer to {what} is null and {count} are stated"),
            ),
        }
    }

    /// A record array of the header: its count at `at`, the room for as many after it, then its pointer.
    fn array(
        self,
        at: usize,
        record: usize,
        what: &'static str,
    ) -> Result<Table<'a>, ProgramError> {
        let (count, room) = (u32_at(self.bytes, at), u32_at(self.bytes, at + 4));
        if count != room {
            return refuse(at, format!("{what}: {count} stated and room for {room}"));
        }
        self.table(at + 8, count as usize, record, what)
    }
}

/// A run of records of one size inside the file.
#[derive(Clone, Copy)]
struct Table<'a> {
    /// The file offset of the first record.
    at: usize,
    record: usize,
    bytes: &'a [u8],
}

impl<'a> Table<'a> {
    fn len(&self) -> usize {
        self.bytes.len() / self.record
    }

    /// Each record with its file offset.
    fn rows(&self) -> impl Iterator<Item = (usize, &'a [u8])> + 'a {
        let (at, record) = (self.at, self.record);
        let records = self.bytes.chunks_exact(record).enumerate();
        records.map(move |(n, bytes)| (at + n * record, bytes))
    }

    /// The index of the record at file offset `at`; the end of the table is one, where a run of none may start.
    fn index_of(&self, at: usize) -> Option<usize> {
        let offset = at.checked_sub(self.at)?;
        let found = offset <= self.bytes.len() && offset.is_multiple_of(self.record);
        found.then_some(offset / self.record)
    }
}

/// What a type record is read against.
struct Tables<'a> {
    file: File<'a>,
    /// The file offset of the pooled data.
    blob_at: usize,
    blob: &'a [u8],
    code_len: u32,
    /// The records of the import table, the program's own among them.
    imports: usize,
    methods: Table<'a>,
    fields: Table<'a>,
    events: Table<'a>,
    properties: Table<'a>,
}

/// What reading the types adds up.
struct Tally {
    /// The bytes of pooled data the types may still state for their vtables, base lists, arguments and enum values.
    // ⚠ This is what bounds the rows a type is given: without it every type could state the whole pooled data as its own.
    pooled: usize,
    /// The records of the field table the types may still state.
    fields: usize,
    /// The offsets the tables name as functions of this program.
    roots: Vec<u32>,
}

impl<'a> Tables<'a> {
    /// `count` records of `record` bytes at `offset` of the pooled data, taken off what the types may still state.
    fn pooled(
        &self,
        offset: usize,
        count: usize,
        record: usize,
        at: usize,
        what: &str,
        tally: &mut Tally,
    ) -> Result<&'a [u8], ProgramError> {
        let end = count
            .checked_mul(record)
            .and_then(|n| n.checked_add(offset));
        let Some(bytes) = end.and_then(|end| self.blob.get(offset..end)) else {
            return refuse(
                at,
                format!("{what}: {count} at {offset:#X} of the pooled data run past its end"),
            );
        };
        match tally.pooled.checked_sub(bytes.len()) {
            Some(left) => tally.pooled = left,
            None => {
                let what = "the tables of the types state more bytes than the pooled data holds";
                return refuse(at, what);
            }
        }
        Ok(bytes)
    }

    /// The offset in the pooled data that the pointer at `at` names; a null one and one to anywhere else are refused.
    fn pooled_at(&self, at: usize, what: &str) -> Result<usize, ProgramError> {
        let offset = self.file.required(at, what)?.checked_sub(self.blob_at);
        match offset {
            Some(offset) if offset <= self.blob.len() => Ok(offset),
            _ => refuse(
                at,
                format!("the pointer to {what} is outside the pooled data"),
            ),
        }
    }

    /// The records of `table` a type states as its own, by the pointer at `at` and the count of them.
    fn run(
        &self,
        table: &Table<'a>,
        at: usize,
        count: usize,
        what: &str,
    ) -> Result<Range<usize>, ProgramError> {
        let Some(start) = self.file.pointer(at, what)? else {
            return match count {
                0 => Ok(0..0),
                _ => refuse(at, format!("a type has {what} and no pointer to them")),
            };
        };
        match table.index_of(start) {
            Some(first) if count <= table.len() - first => Ok(first..first + count),
            _ => refuse(at, format!("a type's {what} are not a run of their table")),
        }
    }

    /// One type: the record `r` at file offset `at`, with what it points at.
    fn type_row(&self, r: &[u8], at: usize, tally: &mut Tally) -> Result<TypeRow, ProgramError> {
        if u64_at(r, 0) != 0 {
            return refuse(at, "the first word of a type is not zero");
        }
        let unused = [u32_at(r, 0x24), u32_at(r, 0x84)];
        if unused != [0, 0] || u16_at(r, 0x78) != 0 || u16_at(r, 0x82) != 0 {
            return refuse(at, "a word of a type that holds nothing is not zero");
        }
        if u64_at(r, 0x50) != BASE {
            return refuse(
                at + 0x50,
                "a type does not point back at the start of the file",
            );
        }
        let mut row = TypeRow {
            name: u64_at(r, 0x10),
            name32: u32_at(r, 0x18),
            byte_size: u32::from(u16_at(r, 0x70)),
            flags: u32_at(r, 0x6C),
            base: some(u32_at(r, 0x20)),
            ..TypeRow::default()
        };
        self.members(r, at, &mut row, tally)?;

        let fields = self.run(
            &self.fields,
            at + 0x38,
            usize::from(u16_at(r, 0x76)),
            "fields",
        )?;
        match tally.fields.checked_sub(fields.len()) {
            Some(left) => tally.fields = left,
            None => {
                let what = "the types state more fields than the field table holds";
                return refuse(at + 0x38, what);
            }
        }
        let records = &self.fields.bytes[fields.start * FIELD..fields.end * FIELD];
        let fields = records.as_chunks::<FIELD>().0.iter().map(|f| FieldRow {
            name: u64_at(f, 0),
            name32: u32_at(f, 8),
            offset: u32_at(f, 12),
            ty: some(u32_at(f, 16)),
            bytes: u32::from(u16_at(f, 20)),
        });
        row.fields = fields.collect();
        // Its events and its properties are held to their tables and not kept.
        let events = usize::from(u16_at(r, 0x7A));
        let _ = self.run(&self.events, at + 0x40, events, "events")?;
        let properties = usize::from(u16_at(r, 0x7C));
        let _ = self.run(&self.properties, at + 0x48, properties, "properties")?;

        let (definition, args) = (some(u32_at(r, 0x34)), usize::from(u16_at(r, 0x74)));
        let offset = u32_at(r, 0x68) as usize;
        // The caller held the record's index to its place in the table.
        let own = u32_at(r, 0x1C);
        row.generic = match (definition, args) {
            (None, 0) if offset == 0 => None,
            // ⚠ A generic type names itself: what it counts are its parameters, and the pooled
            // data holds none of them. Its offset is 0, where a zero byte and the order of the
            // imports are, so reading ids there gives nonsense and states bytes no type has.
            (Some(definition), 1..) if definition == own => {
                if offset != 0 {
                    let what = "a generic type states where arguments are, and it has parameters";
                    return refuse(at + 0x68, what);
                }
                row.parameters = args as u32;
                None
            }
            (Some(_), 1..) if offset == 0 => {
                let what = "a type's arguments are said to be at the first byte of the pooled data";
                return refuse(at + 0x68, what);
            }
            (Some(definition), 1..) => {
                let what = "a type's arguments";
                let ids = self.pooled(offset, args, 4, at + 0x68, what, tally)?;
                let ids = ids.as_chunks::<4>().0.iter().map(|id| u32_at(id, 0));
                Some((definition, ids.collect()))
            }
            (None, 0) => {
                let what = "a type states where its arguments are and has none";
                return refuse(at + 0x68, what);
            }
            _ => {
                let what = "a type is an instance of a generic type exactly when it has arguments";
                return refuse(at + 0x34, what);
            }
        };

        let count = usize::from(u16_at(r, 0x80));
        let list = self.pooled_at(at + 0x60, "a type's base list")?;
        let what = "a type's base list and its end";
        let list = self.pooled(list, count * 8 + 4, 1, at + 0x60, what, tally)?;
        let (list, end) = list.split_at(count * 8);
        if u32_at(end, 0) != NONE {
            return refuse(at + 0x60, "a type's base list has no end after its entries");
        }
        let mut bases = list
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| (u32_at(b, 0), u32_at(b, 4)));
        // The list runs from the root of the type's ancestors to its base type; what follows is implemented.
        if row
            .base
            .is_some_and(|base| !bases.any(|(id, _)| id == base))
        {
            return refuse(at + 0x20, "a type's base type is not in its base list");
        }
        (row.interfaces, row.first_slots) = bases.unzip();

        let count = usize::from(u16_at(r, 0x7E));
        let table = self.pooled_at(at + 0x58, "a type's vtable")?;
        let slots = self.pooled(table, count, 8, at + 0x58, "a type's vtable", tally)?;
        for (n, slot) in slots.as_chunks::<8>().0.iter().enumerate() {
            let read = self.slot(u32_at(slot, 0), u32_at(slot, 4));
            let slot = match read {
                Ok(slot) => slot,
                Err(what) => return refuse(self.blob_at + table + n * 8, what),
            };
            let own = match slot {
                SlotRow::Function { import: 0, at, .. } => Some(at),
                _ => None,
            };
            tally.roots.extend(own);
            row.slots.push(own);
            row.vtable.push(slot);
        }
        Ok(row)
    }

    /// A type's members: the values of an enum in the pooled data, or a run of the method table.
    fn members(
        &self,
        r: &[u8],
        at: usize,
        row: &mut TypeRow,
        tally: &mut Tally,
    ) -> Result<(), ProgramError> {
        let count = u32_at(r, 0x30) as usize;
        let pointer = self.file.pointer(at + 0x28, "a type's members")?;
        let pooled = pointer.and_then(|p| p.checked_sub(self.blob_at));
        match pooled.filter(|offset| count > 0 && *offset < self.blob.len()) {
            Some(offset) => {
                let what = "an enum's descriptor";
                let descriptor = self.pooled(offset, 8, 1, at + 0x28, what, tally)?;
                let (values, names) = (u32_at(descriptor, 0), u32_at(descriptor, 4));
                let width = row.byte_size as usize;
                if !matches!(width, 1 | 2 | 4 | 8) {
                    return refuse(
                        at + 0x70,
                        format!("an enum's values are {width} byte(s) wide, not 1, 2, 4 or 8"),
                    );
                }
                let what = "an enum's values";
                let values = self.pooled(values as usize, count, width, at + 0x28, what, tally)?;
                let number =
                    |value: &[u8]| value.iter().rev().fold(0, |n, b| (n << 8) | u64::from(*b));
                row.constants = values.chunks_exact(width).map(number).collect();
                if names != NONE {
                    let what = "the names of an enum's members";
                    let names = self.pooled(names as usize, count, 4, at + 0x28, what, tally)?;
                    let names = names.as_chunks::<4>().0.iter().map(|name| u32_at(name, 0));
                    row.constant_names = names.collect();
                }
            }
            None => {
                let methods = self.run(&self.methods, at + 0x28, count, "methods");
                // A pointer to anything else is no fault of a type that states no member.
                match methods {
                    Ok(methods) => row.methods = methods.start as u32..methods.end as u32,
                    Err(_) if count == 0 => {}
                    Err(refused) => return Err(refused),
                }
            }
        }
        Ok(())
    }

    /// One slot of a vtable: a target, and the word that says whose it is.
    fn slot(&self, target: u32, owner: u32) -> Result<SlotRow, &'static str> {
        if owner == NONE {
            return match target {
                NONE => Ok(SlotRow::Empty),
                _ => Err("an empty vtable slot has a target"),
            };
        }
        // The low byte is an import, 0 the program itself and 0xFF the VM; bit 8 says the receiver is taken out of a box.
        let (import, unboxes) = (owner & 0xFF, (owner & 0x100) != 0);
        match import {
            _ if owner >> 9 != 0 => Err("a vtable slot's owner has bits above bit 8"),
            0xFF if target >> 16 == 0 => Ok(SlotRow::Builtin {
                ty: target >> 8,
                method: target & 0xFF,
                unboxes,
            }),
            0xFF => Err("a vtable slot of the VM has a target above 16 bits"),
            0 if target >= self.code_len => Err("a vtable slot is outside the code"),
            _ if import as usize >= self.imports => Err("a vtable slot's owner is not an import"),
            _ => Ok(SlotRow::Function {
                import,
                at: target,
                unboxes,
            }),
        }
    }
}

/// The imports after import 0, which is the program itself, and the table of the records with it.
fn imports(file: File<'_>, type_count: u32) -> Result<(Vec<ImportRow>, Table<'_>), ProgramError> {
    let records = file.array(0x140, IMPORT, "the imports")?;
    if records.len() == 0 {
        return refuse(0x140, "there is no import 0, the program itself");
    }
    let mut imports = Vec::with_capacity(records.len() - 1);
    let (mut next, mut last) = (type_count, None);
    for (n, (at, r)) in records.rows().enumerate() {
        if u32_at(r, 4) != 0 {
            return refuse(at + 4, "the second word of an import is not zero");
        }
        if n == 0 {
            if r.iter().any(|byte| *byte != 0) {
                return refuse(at, "import 0, the program itself, is not all zero");
            }
            continue;
        }
        let (program, first, end) = (u32_at(r, 8), u32_at(r, 16), u32_at(r, 20));
        if first != next || end < first {
            return refuse(
                at + 16,
                format!("the type ids of import {n} are {first} to {end}, not a run from {next}"),
            );
        }
        if last.is_some_and(|last| program <= last) {
            return refuse(
                at + 8,
                format!("import {n} is not after the one before it by program hash"),
            );
        }
        (next, last) = (end, Some(program));
        imports.push(ImportRow {
            program,
            types: first..end,
        });
    }
    Ok((imports, records))
}

/// The order a listing numbers the imports in: a permutation at byte 1 of the pooled data, held to be one and not kept.
fn import_order(
    order: u32,
    imports: usize,
    blob: &[u8],
    blob_at: usize,
) -> Result<(), ProgramError> {
    if order != 1 {
        let what = "the order of the imports is neither at byte 1 of the pooled data nor absent";
        return refuse(0x130, what);
    }
    let Some(listed) = blob.get(1..1 + imports * 2) else {
        return refuse(0x130, "the order of the imports runs past the pooled data");
    };
    let mut seen = vec![false; imports];
    for (n, import) in listed.as_chunks::<2>().0.iter().enumerate() {
        let import = usize::from(u16_at(import, 0));
        if import >= imports || std::mem::replace(&mut seen[import], true) {
            let what = "the order of the imports names one twice, or one that is not there";
            return refuse(blob_at + 1 + n * 2, what);
        }
    }
    Ok(())
}

/// The constants of the table the pointer at 0x98 names: the bytes of all of them, and the run of each among those.
// ⚠ The table has no count. It ends at a pair of zeros, or at the table that follows it in the
// file: `sections` is where each table starts.
fn constants(
    file: File<'_>,
    blob: &[u8],
    sections: &[usize],
) -> Result<(Vec<u8>, Vec<Range<u32>>), ProgramError> {
    let Some(table) = file.pointer(0x98, "the constants")? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let after = sections.iter().copied().filter(|start| *start > table);
    let end = after.min().unwrap_or(file.bytes.len());
    let pairs = file.bytes.get(table..end).unwrap_or(&[]);
    let mut rows = Vec::new();
    for (n, pair) in pairs.as_chunks::<8>().0.iter().enumerate() {
        let (offset, len) = (u32_at(pair, 0), u32_at(pair, 4));
        if (offset, len) == (0, 0) {
            break;
        }
        if u64::from(offset) + u64::from(len) > blob.len() as u64 {
            return refuse(
                table + n * 8,
                format!("constant {n}: {len} byte(s) at {offset:#X} of the pooled data run past its end"),
            );
        }
        rows.push(offset..offset + len);
    }
    // One copy of the pooled data from the first constant to the last, whatever the pairs share.
    let start = rows.iter().map(|row| row.start).min().unwrap_or(0);
    let end = rows.iter().map(|row| row.end).max().unwrap_or(0);
    let bytes = blob.get(start as usize..end as usize).unwrap_or(&[]);
    for row in &mut rows {
        *row = row.start - start..row.end - start;
    }
    Ok((bytes.to_vec(), rows))
}

/// Reads a compiled program to its tables.
pub(super) fn read(bytes: &[u8]) -> Result<Program, ProgramError> {
    let file = File { bytes };
    let h = file.span(0, HEADER, 1, "the header")?;
    let word = |at: usize| u32_at(h, at);
    if u64_at(h, 0) != 0 {
        return refuse(0, "the first eight bytes are not zero");
    }
    if word(0x14) != 1 {
        return refuse(0x14, format!("the word is {}, not 1", word(0x14)));
    }
    if word(0x18) != FORMAT {
        let format = word(0x18);
        return refuse(
            0x18,
            format!("format {format:#X} is not read, format {FORMAT:#X} is"),
        );
    }
    if u64_at(h, 0x1C) != BUILD {
        let build = u64_at(h, 0x1C);
        return refuse(
            0x1C,
            format!("build {build:#X} is not read, build {BUILD:#X} is"),
        );
    }
    if let Some(at) = RESERVED.iter().find(|at| word(**at) != 0) {
        return refuse(*at, "a header word that holds nothing is not zero");
    }
    let page_at = file.required(0x08, "the page map")?;
    let page = file.span(page_at, 16, 1, "the page map")?;
    if u64_at(page, 0) != 0 || !matches!(u32_at(page, 8), 1 | 2) || u32_at(page, 12) != 0 {
        return refuse(
            page_at,
            "the page map is not a zero, a 1 or a 2, and a zero",
        );
    }

    let code_len = word(0xF8);
    let code_at = file.required(0xE8, "the code")?;
    let code = file.span(code_at, code_len as usize, 1, "the code")?;
    let blob_at = file.required(0xF0, "the pooled data")?;
    let blob = file.span(blob_at, word(0xFC) as usize, 1, "the pooled data")?;
    if blob.first() != Some(&0) {
        return refuse(blob_at, "the pooled data does not start with a zero byte");
    }

    let type_count = word(0x28);
    let (imports, import_table) = imports(file, type_count)?;
    let order = word(0x130);
    let mut global_references = Vec::new();
    if order != NONE {
        import_order(order, imports.len(), blob, blob_at)?;
        // ⚠ Read only beside an order of imports: a program others import has none, and its word
        // at 0x124 is no offset of its pooled data.
        if word(0x120) == 0x10 {
            // A bit a cell of the whole global block, kept as 64-bit words.
            let (map, len) = (
                word(0x124) as usize,
                (word(0x118) as usize).div_ceil(512) * 8,
            );
            let Some(map) = map.checked_add(len).and_then(|end| blob.get(map..end)) else {
                let what = "the map of the globals that hold a reference runs past the pooled data";
                return refuse(0x124, what);
            };
            global_references = map.to_vec();
        }
    }

    // A program that calls no native has no table: both pointers are null, and the count is zero.
    let native_count = word(0xB8) as usize;
    let native_table = file.absent_or(0xA8, native_count, 8, "the natives")?;
    let natives = native_table.bytes.as_chunks::<8>().0.iter();
    let natives = natives.map(|stored| NativeRow {
        hash: u64_at(stored, 0),
        ..NativeRow::default()
    });
    // The second half of the table, a 32-bit word a native, is held to be there and not kept.
    let native_keys = file.absent_or(0xB0, native_count, 4, "the native keys")?;

    let records = file.table(0x100, word(0xC8) as usize, ENTRY, "the entries")?;
    let entries_at = records.at;
    let mut entries = Vec::with_capacity(records.len());
    for (at, r) in records.rows() {
        let entry = EntryRow {
            name: u32_at(r, 0),
            at: u32_at(r, 4),
        };
        if entry.at >= code_len {
            return refuse(at + 4, "an entry is outside the code");
        }
        if u32_at(r, 8) > 1 {
            return refuse(at + 8, "the third word of an entry is neither 0 nor 1");
        }
        entries.push(entry);
    }
    let named = |at: usize, what: &str| match word(at) {
        NONE => Ok(None),
        offset if offset < code_len => Ok(Some(offset)),
        _ => refuse(at, format!("{what} is outside the code")),
    };
    let main = named(0xBC, "`main`")?;
    let initialize = named(0xC0, "`initialize`")?;

    // Six groups of records under one pointer, counted by six words; the last also states its room.
    // ⚠ Measured on few programs: the second, fourth and fifth group on 6, 13 and 6, the sixth on
    // one. A refusal here on another build says "not measured" before it says "malformed".
    if word(THIRD_GROUP) != 0 {
        let what = "a third group of reflection records, which no program was seen to hold";
        return refuse(THIRD_GROUP, what);
    }
    if word(0xE0) != word(0xE4) {
        let (count, room) = (word(0xE0), word(0xE4));
        return refuse(
            0xE0,
            format!("the reflection records: {count} stated and room for {room}"),
        );
    }
    let groups = [0xCC, 0xD0, THIRD_GROUP, 0xD8, 0xDC, 0xE0].map(word);
    let reflected = groups
        .iter()
        .fold(0usize, |sum, n| sum.saturating_add(*n as usize));
    let reflection = file.table(0xA0, reflected, REFLECTION, "the reflection records")?;
    let attributes = file.array(0x38, 16, "the attributes")?;
    let arguments = file.array(0x48, 16, "the attribute arguments")?;
    let methods = file.array(0x58, METHOD, "the methods")?;
    let fields = file.array(0x68, FIELD, "the fields")?;
    let events = file.array(0x78, 16, "the events")?;
    let properties = file.array(0x88, 32, "the properties")?;
    let type_bits = file.table(0x170, word(0x168) as usize, 1, "the type bits")?;
    let types = file.array(0x28, TYPE, "the types")?;

    let mut method_rows = Vec::with_capacity(methods.len());
    for (at, r) in methods.rows() {
        let row = MethodRow {
            name: u32_at(r, 0),
            at: some(u32_at(r, 4)),
            flags: u32_at(r, 8),
            slot: Some(u32::from(u16_at(r, 0x12))).filter(|slot| *slot != 0xFFFF),
            returns: u32::from(r[0x14]),
            params: u32::from(r[0x15]),
            result: some(u32_at(r, 0x18)),
            owner: u32_at(r, 0x1C),
        };
        if row.owner >= type_count {
            return refuse(
                at + 0x1C,
                "a method's owner is not one of the program's own types",
            );
        }
        if row.at.is_some_and(|function| function >= code_len) {
            return refuse(at + 4, "a method is outside the code");
        }
        method_rows.push(row);
    }

    let sections = [
        code_at,
        blob_at,
        page_at,
        native_table.at,
        native_keys.at,
        entries_at,
        import_table.at,
        reflection.at,
        attributes.at,
        arguments.at,
        methods.at,
        fields.at,
        events.at,
        properties.at,
        type_bits.at,
        types.at,
    ];
    let (constants, constant_rows) = constants(file, blob, &sections)?;

    let tables = Tables {
        file,
        blob_at,
        blob,
        code_len,
        imports: import_table.len(),
        methods,
        fields,
        events,
        properties,
    };
    let roots = entries.iter().map(|entry| entry.at);
    let roots = roots.chain(method_rows.iter().filter_map(|method| method.at));
    let mut tally = Tally {
        pooled: blob.len(),
        fields: fields.len(),
        roots: roots.chain(main).chain(initialize).collect(),
    };
    let mut type_rows = Vec::with_capacity(types.len());
    let mut last = None;
    for (n, (at, r)) in types.rows().enumerate() {
        if u32_at(r, 0x1C) as usize != n {
            let what = "a type's index is not its place in the type table";
            return refuse(at + 0x1C, what);
        }
        let row = tables.type_row(r, at, &mut tally)?;
        if last.is_some_and(|last| row.name <= last) {
            let what = "the types are not in the order of the hashes of their names";
            return refuse(at + 0x10, what);
        }
        last = Some(row.name);
        type_rows.push(row);
    }

    Ok(Program {
        program: word(0x10),
        code: code.to_vec(),
        functions: functions::discover(code, &tally.roots)?,
        types: type_rows,
        imports,
        natives: natives.collect(),
        // The sizes of the program's own blocks, in bytes: the globals at 0x11C, the statics at 0x128.
        globals: Area {
            cells: word(0x11C).div_ceil(8),
            initial: Vec::new(),
        },
        statics: Area {
            cells: word(0x128).div_ceil(8),
            initial: Vec::new(),
        },
        strings: blob.to_vec(),
        constants,
        handlers: Vec::new(),
        entries,
        main,
        initialize,
        methods: method_rows,
        constant_rows,
        global_references,
    })
}
