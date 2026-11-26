//! The lattice a cell's kind is inferred in, and what the runtime's names and a natives list say
//! of kinds.

use crate::gta5::script::natives::NativeName;
use crate::gta6::script::ir::{Const, Kind, Scalar};

/// What the evidence so far says a one-cell value is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Lat {
    /// Nothing says.
    #[default]
    None,
    /// A 32-bit integer of either sign: only zero tests, equality and bit operations read it.
    Int32,
    Is(Kind),
    /// A reference to one of several types that have nothing to do with each other.
    // ⚠ Not `Is(Pointer)`, which the next named type would replace: this one stays.
    Refs,
    /// The evidence disagrees, so the value stays untyped.
    Conflict,
}

impl Lat {
    /// What two pieces of evidence say together.
    pub(super) fn join(&self, other: &Lat) -> Lat {
        match (self, other) {
            (Lat::None, known) | (known, Lat::None) => known.clone(),
            (Lat::Conflict, _) | (_, Lat::Conflict) => Lat::Conflict,
            (Lat::Refs, Lat::Refs) => Lat::Refs,
            (Lat::Refs, Lat::Is(kind)) | (Lat::Is(kind), Lat::Refs) if reference(kind) => Lat::Refs,
            (Lat::Refs, _) | (_, Lat::Refs) => Lat::Conflict,
            (Lat::Int32, Lat::Int32) => Lat::Int32,
            (Lat::Int32, Lat::Is(kind)) | (Lat::Is(kind), Lat::Int32) => match int32_like(kind) {
                true => Lat::Is(kind.clone()),
                false => Lat::Conflict,
            },
            (Lat::Is(a), Lat::Is(b)) if a == b => Lat::Is(a.clone()),
            // Of two integers that hold the same values, the wider is the one to declare.
            (Lat::Is(a), Lat::Is(b)) if takes_in(b, a) => Lat::Is(b.clone()),
            (Lat::Is(a), Lat::Is(b)) if takes_in(a, b) => Lat::Is(a.clone()),
            (Lat::Is(a), Lat::Is(b)) if reference(a) && reference(b) => Lat::Is(match (a, b) {
                (Kind::Pointer, named) | (named, Kind::Pointer) => named.clone(),
                _ => Kind::Pointer,
            }),
            (Lat::Is(_), Lat::Is(_)) => Lat::Conflict,
        }
    }

    /// The kind to declare: an integer of either sign is an `int`, and what nothing settles is unknown.
    pub(super) fn kind(&self) -> Kind {
        match self {
            Lat::Int32 => Kind::I32,
            Lat::Is(kind) => kind.clone(),
            Lat::Refs => Kind::Pointer,
            Lat::None | Lat::Conflict => Kind::Unknown,
        }
    }
}

/// What a kind that a signature, a layout or another pass states says of one cell.
// A value of several cells, or of a type held in place, is not a kind this lattice joins.
pub(super) fn stated(kind: &Kind) -> Lat {
    match kind {
        Kind::Unknown | Kind::Vector | Kind::Handle | Kind::Aggregate(_) | Kind::Value { .. } => {
            Lat::None
        }
        kind => Lat::Is(kind.clone()),
    }
}

/// What a field of this kind says of the cell an access of `bytes` bytes loads from it or stores to it.
// ⚠ No sign for a 32-bit integer: a layout's sign may be a default, and the reads of the value decide.
pub(super) fn field(kind: &Kind, bytes: u32) -> Lat {
    match (kind, bytes) {
        (Kind::I32 | Kind::U32, 4) => Lat::Int32,
        (Kind::F32 | Kind::Hash, 4) | (Kind::F64, 8) => Lat::Is(kind.clone()),
        (kind, 8) if reference(kind) => Lat::Is(kind.clone()),
        _ => Lat::None,
    }
}

/// The bytes an integer constant fills, 1, 2 or 4; 0 for a float and for a cell 32 bits do not widen to.
pub(super) fn integer_bytes(c: &Const) -> u32 {
    let odd = c.kind == Kind::F32 && !plausible_float(c.bits as u32);
    let fits = normal(&Kind::I32, c.bits) || normal(&Kind::U32, c.bits);
    match c.kind.is_integer() || odd {
        true if c.bits <= 0xFF => 1,
        true if c.bits <= 0xFFFF => 2,
        true if fits => 4,
        _ => 0,
    }
}

/// Whether a 32-bit integer read of either sign may be a value of this kind.
fn int32_like(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Bool
            | Kind::I8
            | Kind::U8
            | Kind::I16
            | Kind::U16
            | Kind::I32
            | Kind::U32
            | Kind::Hash
    )
}

/// Whether a value of this kind is a reference.
pub(super) fn reference(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Pointer | Kind::Object(_) | Kind::String | Kind::Array(_)
    )
}

/// Whether a kind says what a reference points at.
pub(super) fn named(kind: &Kind) -> bool {
    reference(kind) && *kind != Kind::Pointer
}

/// Whether evidence of `narrow` and of `wide` together is a variable of kind `wide`.
fn takes_in(wide: &Kind, narrow: &Kind) -> bool {
    widens(narrow, wide) && !matches!(wide, Kind::I64 | Kind::U64)
}

/// Whether a value four bytes or fewer hold is what the evidence names.
pub(super) fn thin(lat: &Lat) -> bool {
    match lat {
        Lat::Int32 => true,
        Lat::Is(kind) => int32_like(kind) || *kind == Kind::F32,
        Lat::None | Lat::Refs | Lat::Conflict => false,
    }
}

/// Whether a kind has one way to widen into a cell that a narrower read undoes.
pub(super) fn narrow(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Bool
            | Kind::I8
            | Kind::U8
            | Kind::I16
            | Kind::U16
            | Kind::I32
            | Kind::U32
            | Kind::F32
    )
}

/// The bytes a number narrower than a cell fills; `None` for any other kind.
pub(super) fn number_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::I8 | Kind::U8 => Some(1),
        Kind::I16 | Kind::U16 => Some(2),
        Kind::I32 | Kind::U32 | Kind::F32 => Some(4),
        _ => None,
    }
}

/// Whether a cell widened as a value of `from` reads as the same value through `to`.
pub(super) fn widens(from: &Kind, to: &Kind) -> bool {
    match from {
        Kind::Bool => matches!(
            to,
            Kind::I8
                | Kind::U8
                | Kind::I16
                | Kind::U16
                | Kind::I32
                | Kind::U32
                | Kind::I64
                | Kind::U64
        ),
        Kind::U8 => matches!(
            to,
            Kind::I16 | Kind::U16 | Kind::I32 | Kind::U32 | Kind::I64 | Kind::U64
        ),
        Kind::I8 => matches!(to, Kind::I16 | Kind::I32 | Kind::I64),
        Kind::U16 => matches!(to, Kind::I32 | Kind::U32 | Kind::I64 | Kind::U64),
        Kind::I16 => matches!(to, Kind::I32 | Kind::I64),
        Kind::U32 => matches!(to, Kind::I64 | Kind::U64),
        Kind::I32 => matches!(to, Kind::I64),
        _ => false,
    }
}

/// Whether the cell `bits` is what a value of `kind` widens to.
pub(super) fn normal(kind: &Kind, bits: u64) -> bool {
    match kind {
        Kind::Bool => bits <= 1,
        Kind::I8 => bits == bits as i8 as i64 as u64,
        Kind::U8 => bits <= u64::from(u8::MAX),
        Kind::I16 => bits == bits as i16 as i64 as u64,
        Kind::U16 => bits <= u64::from(u16::MAX),
        Kind::I32 => bits == bits as i32 as i64 as u64,
        Kind::U32 | Kind::F32 => bits <= u64::from(u32::MAX),
        Kind::I64 | Kind::U64 | Kind::F64 => true,
        _ => false,
    }
}

/// Whether the cell `bits` is a value of what the evidence names, so that a literal of those bits says no more of a variable.
// ⚠ For 32 bits of any reading, a literal 32 bits hold under either sign: which sign the variable
// takes may still be open, and a float's bits are any 32.
pub(super) fn holds(lat: &Lat, bits: u64) -> bool {
    let thin = normal(&Kind::I32, bits) || normal(&Kind::U32, bits);
    match lat {
        Lat::Int32 | Lat::Is(Kind::I32 | Kind::U32 | Kind::Hash | Kind::F32) => thin,
        Lat::Is(kind) => normal(kind, bits),
        Lat::None | Lat::Refs | Lat::Conflict => false,
    }
}

/// Whether evidence leaves a variable given nothing but 0 and 1 free to be a truth: none, or an integer 32 bits hold.
pub(super) fn takes_truth(lat: &Lat) -> bool {
    match lat {
        Lat::None | Lat::Int32 => true,
        Lat::Is(kind) => int32_like(kind),
        Lat::Refs | Lat::Conflict => false,
    }
}

/// The cell `as_T` makes of the cell `bits`: its low bits, widened as the runtime widens a `T`.
pub(super) fn cast_bits(to: Scalar, bits: u64) -> u64 {
    match to {
        Scalar::I8 => bits as i8 as i64 as u64,
        Scalar::U8 => u64::from(bits as u8),
        Scalar::I16 => bits as i16 as i64 as u64,
        Scalar::U16 => u64::from(bits as u16),
        Scalar::I32 => bits as i32 as i64 as u64,
        // A float goes into a cell as its 32 bits with nothing above them.
        Scalar::U32 | Scalar::F32 => u64::from(bits as u32),
        Scalar::I64 | Scalar::U64 | Scalar::F64 => bits,
    }
}

/// What a cast says of the variable under it; `weak` when the operator above reads either sign alike.
pub(super) fn cast_evidence(to: Scalar, weak: bool) -> Lat {
    match to {
        Scalar::I32 | Scalar::U32 if weak => Lat::Int32,
        // Every cell reads as 64 bits: only an operator that orders or counts says it is a number.
        Scalar::I64 | Scalar::U64 if weak => Lat::None,
        _ => Lat::Is(to.kind()),
    }
}

/// Whether 32 bits are a float a source would hold: not a denormal, which is a small integer, nor a NaN.
pub(super) fn plausible_float(bits: u32) -> bool {
    let exponent = (bits >> 23) & 0xFF;
    let mantissa = bits & 0x007F_FFFF;
    exponent != 0 && (exponent != 0xFF || mantissa == 0)
}

/// Gives a constant the kind of what reads it, where its bits are a value of that kind; the bits stay.
pub(super) fn retype(c: &mut Const, want: &Kind) {
    let half = c.bits >> 32;
    // A 32-bit value, widened with its sign or without.
    let fits = half == 0 || half == 0xFFFF_FFFF;
    // A float literal no source wrote: an integer printed as the float of its bits.
    let odd = c.kind == Kind::F32 && !plausible_float(c.bits as u32);
    let integer = c.kind.is_integer() || odd;
    match want {
        Kind::F32 if c.kind.is_integer() && fits => c.kind = Kind::F32,
        Kind::F64 if matches!(c.kind, Kind::I64 | Kind::U64) => c.kind = Kind::F64,
        Kind::Bool if integer && c.bits <= 1 => c.kind = Kind::Bool,
        Kind::I32 | Kind::U32 | Kind::Hash if integer && fits => c.kind = want.clone(),
        Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 if integer && normal(want, c.bits) => {
            c.kind = want.clone();
        }
        Kind::I64 | Kind::U64 if integer => c.kind = want.clone(),
        Kind::Pointer | Kind::Object(_) | Kind::String | Kind::Array(_)
            if integer && c.bits == 0 =>
        {
            c.kind = Kind::Pointer;
        }
        // Nothing reads it as a float: the cell is the same number either way.
        _ if odd && c.bits != 0 && !want.is_float() => {
            c.kind = match c.bits < 0x0080_0000 {
                true => Kind::I32,
                false => Kind::U32,
            };
        }
        _ => {}
    }
}

/// `value << count` at the integer type `at`, as C# shifts.
pub(super) fn shifted(at: &Kind, value: u64, count: u64) -> Option<u64> {
    let count = count as u32;
    match at {
        Kind::I32 => Some((value as i32).wrapping_shl(count) as i64 as u64),
        Kind::U32 => Some(u64::from((value as u32).wrapping_shl(count))),
        Kind::I64 => Some((value as i64).wrapping_shl(count) as u64),
        Kind::U64 => Some(value.wrapping_shl(count)),
        _ => None,
    }
}

/// The one or two bits a mask of the type `at` clears, as a constant of that type, where it clears no more.
pub(super) fn cleared(at: &Kind, mask: u64) -> Option<u64> {
    let few = |clear: u64| (1..=2).contains(&clear.count_ones());
    match at {
        Kind::I32 | Kind::U32 => {
            let clear = !(mask as u32);
            let bits = match at {
                Kind::I32 => clear as i32 as i64 as u64,
                _ => u64::from(clear),
            };
            few(u64::from(clear)).then_some(bits)
        }
        Kind::I64 | Kind::U64 => few(!mask).then_some(!mask),
        _ => None,
    }
}

/// The C library's float functions, which the runtime's intrinsics keep the names of.
const FLOAT_FUNCTIONS: [&str; 11] = [
    "sqrtf", "sinf", "cosf", "tanf", "asinf", "acosf", "atanf", "atan2f", "powf", "floorf", "ceilf",
];

/// Intrinsics named `<stem>_<type>`, which read every argument as that type.
const TYPED_STEMS: [&str; 7] = [
    "abs",
    "min",
    "max",
    "clamp",
    "lerp",
    "sign",
    "round_half_up",
];

/// `<from>_to_<to>`: the two types of a conversion.
fn conversion(name: &str) -> Option<(Scalar, Scalar)> {
    let (from, to) = name.split_once("_to_")?;
    Some((Scalar::from_name(from)?, Scalar::from_name(to)?))
}

/// `<stem>_<type>` of [`TYPED_STEMS`].
fn typed(name: &str) -> Option<(&str, Scalar)> {
    let (stem, ty) = name.rsplit_once('_')?;
    let ty = Scalar::from_name(ty)?;
    TYPED_STEMS.contains(&stem).then_some((stem, ty))
}

/// The type the runtime reads argument `index` of an intrinsic as, where the intrinsic's name says.
pub(super) fn intrinsic_reads(name: &str, index: usize) -> Option<Scalar> {
    if let Some((from, _)) = conversion(name) {
        return (index == 0).then_some(from);
    }
    if FLOAT_FUNCTIONS.contains(&name) {
        return Some(Scalar::F32);
    }
    if let Some((_, ty)) = typed(name) {
        return Some(ty);
    }
    match (name, index) {
        // The second argument of every array intrinsic is the index, read as an `int`.
        (array, 1) if array.starts_with("array_") => Some(Scalar::I32),
        ("packed_vector_from_xyz", 0..=2) | ("packed_vector_splat", 0) => Some(Scalar::F32),
        _ => None,
    }
}

/// What the runtime takes argument `index` of an intrinsic for, where its name says and no narrower reading is stated.
// ⚠ A hint: the cell is still read whole. A scale is pushed before the two cells of the vector
// it scales, so it is the first argument and both cells after it are the vector's.
pub(super) fn intrinsic_takes(name: &str, index: usize) -> Lat {
    match (name, index) {
        ("packed_vector_scale", 0) => Lat::Is(Kind::F32),
        _ => Lat::None,
    }
}

/// What an intrinsic yields in one cell, by its name: what it is, and the kind its cell is widened as.
pub(super) fn intrinsic_yields(name: &str) -> (Lat, Option<Kind>) {
    let both = |kind: Kind| (Lat::Is(kind.clone()), Some(kind));
    if let Some((_, to)) = conversion(name) {
        return both(to.kind());
    }
    if FLOAT_FUNCTIONS.contains(&name) || name.starts_with("packed_vector_dot") {
        return both(Kind::F32);
    }
    if let Some((stem, ty)) = typed(name) {
        return both(match stem {
            // The sign of any number is an `int`.
            "sign" => Kind::I32,
            _ => ty.kind(),
        });
    }
    if name == "array_length" {
        return both(Kind::I32);
    }
    if name.starts_with("bytes_equal_") {
        return both(Kind::Bool);
    }
    match name.strip_prefix("array_get_").and_then(Scalar::from_name) {
        // Four bytes are widened as an `int` whatever the element is: a float array reads so too.
        Some(Scalar::I32) => (Lat::None, Some(Kind::I32)),
        Some(Scalar::U64) | None => (Lat::None, None),
        Some(ty) => both(ty.kind()),
    }
}

/// The cells a native's parameter or result of this type takes, by the lists' spelling of the type.
pub(super) fn native_cells(ty: &str) -> usize {
    match ty {
        "Entity" | "Ped" | "Vehicle" | "Object" | "Pickup" | "Vector3" => 2,
        _ => 1,
    }
}

/// What a one-cell type of a natives list is.
pub(super) fn native_lat(ty: &str) -> Lat {
    match ty {
        "int" => Lat::Is(Kind::I32),
        "float" => Lat::Is(Kind::F32),
        "BOOL" => Lat::Is(Kind::Bool),
        "Hash" => Lat::Is(Kind::Hash),
        "const char*" => Lat::Is(Kind::String),
        pointer if pointer.ends_with('*') => Lat::Is(Kind::Pointer),
        _ => Lat::None,
    }
}

/// What a two-cell type of a natives list is.
pub(super) fn native_wide(ty: &str) -> Kind {
    match ty {
        "Vector3" => Kind::Vector,
        "Entity" | "Ped" | "Vehicle" | "Object" | "Pickup" => Kind::Handle,
        _ => Kind::Unknown,
    }
}

/// What a list says each argument cell of a native is, where its parameters take the cells the call passes.
// ⚠ A list of another build fits by its count of cells alone: a signature that does not fit says nothing.
pub(super) fn native_args(native: &NativeName, passed: usize) -> Option<Vec<Lat>> {
    let mut cells = Vec::with_capacity(passed);
    for param in &native.params {
        match native_cells(&param.ty) {
            1 => cells.push(native_lat(&param.ty)),
            wide => cells.extend(std::iter::repeat_n(Lat::None, wide)),
        }
    }
    (!cells.is_empty() && cells.len() == passed).then_some(cells)
}
