//! What makes the readable form true C#: a literal by its kind, an operator typed as C# types
//! it, and the names a function, a method and a field are written under.
//!
//! ⚠ Each spelling here is of one node and changes nothing of what the node says. Where a rule
//! does not plainly hold, the text stays as the printer always wrote it.

use std::fmt::Write;

use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, ExprSugar, FunctionKey, Kind, Scalar, TypeKey, Var,
};
use crate::gta6::script::Pass;

use super::{
    bits, callee_name, flat_cells, is_zero, readable_const, reread, wrap, Writer, POSTFIX, UNARY,
};

/// The base types a name is looked for on: no hierarchy is that deep, and a ring of bases must end.
const MAX_BASES: usize = 64;

/// The C# keyword of a .NET primitive type, by the name a program states for it.
// A table of the language's own words: it holds no name of the game.
pub(super) fn keyword(name: &str) -> Option<&'static str> {
    Some(match name {
        "System.Boolean" => "bool",
        "System.Byte" => "byte",
        "System.SByte" => "sbyte",
        "System.Int16" => "short",
        "System.UInt16" => "ushort",
        "System.Int32" => "int",
        "System.UInt32" => "uint",
        "System.Int64" => "long",
        "System.UInt64" => "ulong",
        "System.Single" => "float",
        "System.Double" => "double",
        "System.Char" => "char",
        "System.String" => "string",
        "System.Object" => "object",
        _ => return None,
    })
}

/// The binary operator C# has a type declare under a method of this name.
pub(super) fn operator(name: &str) -> Option<BinOp> {
    Some(match name {
        "op_Equality" => BinOp::Eq,
        "op_Inequality" => BinOp::Ne,
        "op_Addition" => BinOp::Add,
        "op_Subtraction" => BinOp::Sub,
        "op_Multiply" => BinOp::Mul,
        "op_Division" => BinOp::Div,
        "op_Modulus" => BinOp::Rem,
        "op_LessThan" => BinOp::Lt,
        "op_LessThanOrEqual" => BinOp::Le,
        "op_GreaterThan" => BinOp::Gt,
        "op_GreaterThanOrEqual" => BinOp::Ge,
        _ => return None,
    })
}

/// Whether a 32-bit number is written in decimal: a small one, or a count of hundreds below 2^24.
// A hash is a count of hundreds below 2^24 one time in 25,600; such a one shows in decimal, bit for bit.
fn decimal(value: i32) -> bool {
    let size = value.unsigned_abs();
    size <= 0x1_0000 || (size < 0x100_0000 && value % 100 == 0)
}

/// A constant as the C# literal of its kind, which reads back to the same bits.
pub(super) fn literal(c: &Const) -> String {
    match c.kind {
        Kind::I32 => {
            let value = c.bits as i32;
            match decimal(value) {
                true => value.to_string(),
                false if value >= 0 => format!("0x{:08X}", value as u32),
                // ⚠ Eight hex digits with the top bit set are a `uint` to C#: the cast says they are an `int`.
                false => format!("(int)0x{:08X}", value as u32),
            }
        }
        Kind::U32 if (c.bits as u32) < 0x8000_0000 && decimal(c.bits as i32) => {
            (c.bits as u32).to_string()
        }
        Kind::Hash => format!("0x{:08X}", c.bits as u32),
        Kind::I64 | Kind::U64 | Kind::Unknown
            if c.bits < 0x100_0000 && c.bits.is_multiple_of(100) =>
        {
            c.bits.to_string()
        }
        Kind::F32 => {
            let value = c.as_f32();
            match (value.is_nan(), value.is_finite()) {
                // No literal spells a NaN: it keeps its bits.
                (true, _) => format!("float_bits(0x{:08X})", c.bits as u32),
                (false, true) => format!("{value:?}f"),
                (false, false) if value > 0.0 => "float.PositiveInfinity".to_owned(),
                (false, false) => "float.NegativeInfinity".to_owned(),
            }
        }
        Kind::F64 => {
            let value = c.as_f64();
            match (value.is_nan(), value.is_finite()) {
                (true, _) => format!("double_bits(0x{:016X}UL)", c.bits),
                (false, true) => format!("{value:?}"),
                (false, false) if value > 0.0 => "double.PositiveInfinity".to_owned(),
                (false, false) => "double.NegativeInfinity".to_owned(),
            }
        }
        _ => readable_const(c),
    }
}

/// An `int` constant as the eight hex digits of its bits with no cast, for a place that takes 32 bits; `None` where its literal has no cast.
// ⚠ Only for a constant of kind `I32`: one of a 64-bit kind is a whole cell, and the cast is what says its upper half.
pub(super) fn narrow_literal(c: &Const) -> Option<String> {
    let value = c.bits as i32;
    let cast = c.kind == Kind::I32 && value < 0 && !decimal(value);
    cast.then(|| format!("0x{:08X}", value as u32))
}

/// The float whose bits a 32-bit integer constant is, where that float is a round one: of few digits and of a size a script states.
// ⚠ Said beside the constant and never in its place: the bits alone prove no kind.
pub(super) fn round_float(c: &Const) -> Option<f32> {
    let low = c.bits as u32;
    let half = c.bits >> 32;
    let widened = half == 0 || (half == 0xFFFF_FFFF && low >> 31 == 1);
    let exponent = (low >> 23) & 0xFF;
    let round = (100..=150).contains(&exponent) && low & 0x7FF == 0;
    (c.kind.is_integer() || c.kind == Kind::Unknown)
        .then_some(f32::from_bits(low))
        .filter(|_| widened && round)
}

/// Whether a value of `kind` fills 32 bits and no more, so that the sign a constant is widened with says nothing to what holds it.
pub(super) fn narrow_kind(kind: &Kind) -> bool {
    matches!(kind, Kind::I32 | Kind::U32 | Kind::Hash | Kind::F32)
}

/// A C# string literal that keeps to its line: every character a line could end at is escaped.
// Never `\x`, whose length C# does not fix.
pub(super) fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\0' => out.push_str("\\0"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' '
                || ('\u{7F}'..='\u{9F}').contains(&c)
                || matches!(c, '\u{2028}' | '\u{2029}') =>
            {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The constant `e` prints as: the constant itself, or the one the readings around it make of it.
pub(super) fn folded(e: &Expr) -> Option<Const> {
    match e {
        Expr::Const(c) => Some(c.clone()),
        Expr::Cast { to, e: under } => {
            let mut inner = &**under;
            while let Expr::Cast { to: below, e } = inner {
                if below.bytes() < to.bytes() {
                    return None;
                }
                inner = &**e;
            }
            match inner {
                Expr::Const(c) => Some(reread(*to, c)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The constant `e` is where its 32 bits print as hex with the top bit set, which C# reads as a `uint`.
fn high(e: &Expr) -> Option<Const> {
    let c = folded(e)?;
    let low = c.bits as u32;
    let hex = match c.kind {
        Kind::U32 | Kind::Hash => true,
        Kind::I32 => !decimal(low as i32),
        _ => false,
    };
    let kind = match c.kind {
        Kind::Hash => Kind::Hash,
        _ => Kind::U32,
    };
    let bits = u64::from(low);
    (hex && low >> 31 == 1).then_some(Const { bits, kind })
}

impl Writer<'_> {
    /// A constant's text and the strength it binds at: a sign or a cast in front makes it a unary expression.
    pub(super) fn constant(&self, c: &Const) -> (String, u8) {
        if !self.sharp {
            return super::constant(c);
        }
        // ⚠ Only a constant the names pass left the kind `Hash`: any other number may have a text by accident.
        let named = match c.kind == Kind::Hash && self.options.on(Pass::HashText) {
            true => self.names.text(c.bits as u32),
            false => None,
        };
        let text = match named {
            Some(text) => format!("Hash({})", string(text)),
            None => literal(c),
        };
        // ⚠ Never a kind: the bits of a round float a constant of no proved kind holds are said beside it.
        let text =
            match round_float(c).filter(|_| text.ends_with(&format!("0x{:08X}", c.bits as u32))) {
                Some(float) => format!("{text} /* {float:?}f */"),
                None => text,
            };
        let binds = match text.starts_with('-') || text.starts_with('(') {
            true => UNARY,
            false => POSTFIX,
        };
        (text, binds)
    }

    /// `e` where it stands alone in a place that takes 32 bits: an `int` constant without its cast, and a float under a 32-bit reading as the float; `None` for anything else.
    // ⚠ Never for an operand: `x < 0x9A3B5C7D` compares with a `uint` in C#, and `x + 1.0f` adds floats.
    // `exact` says the place holds 32 bits and no more; an argument of a native is taken as 32
    // bits by the words of the header alone, so a float's reading stays written there.
    pub(super) fn narrow(&mut self, e: &Expr, exact: bool) -> Option<String> {
        if !self.sharp {
            return None;
        }
        let e = bits(e);
        // Either arm of a choice stands where the choice does.
        if let Expr::Select { cond, then, els } = e {
            let (a, b) = (self.narrow(then, exact), self.narrow(els, exact));
            if a.is_none() && b.is_none() {
                return None;
            }
            let cond = self.cond(cond, 2);
            let a = a.unwrap_or_else(|| self.expr(bits(then), 2));
            let b = b.unwrap_or_else(|| self.expr(bits(els), 2));
            return Some(format!("{cond} ? {a} : {b}"));
        }
        let (mut inner, mut read) = (e, false);
        while let Expr::Cast { to, e: under } = inner {
            if to.bytes() < 4 || to.kind().is_float() {
                break;
            }
            read |= to.bytes() == 4;
            inner = &**under;
        }
        if let (Expr::Const(c), true, true) = (inner, read, exact) {
            if c.kind == Kind::F32 && self.options.on(Pass::Kinds) {
                return Some(literal(c));
            }
        }
        if !self.options.on(Pass::Signs) {
            return None;
        }
        narrow_literal(&folded(e)?)
    }

    /// The name a function of a program is written under: the user's, else the one its front end gave it.
    pub(super) fn function_name(&self, program: u32, at: u32, listed: &str) -> String {
        let named = match self.sharp {
            true => self.names.function(FunctionKey::new(program, at)),
            false => None,
        };
        named.unwrap_or(listed).to_owned()
    }

    /// The user's name for virtual slot `slot` of `ty`, looked for on the type and then on its bases.
    pub(super) fn method_name(&self, ty: TypeKey, slot: u32) -> Option<&str> {
        let mut ty = ty;
        for _ in 0..MAX_BASES {
            if let Some(name) = self.names.method(ty, slot) {
                return Some(name);
            }
            ty = self.project.types.get(&ty)?.base?;
        }
        None
    }

    /// The name of the field at byte `offset` of an object of type `ty`: the user's for the type or a base of it, else `f_` and the offset.
    pub(super) fn field_name(&self, ty: Option<TypeKey>, offset: u32) -> String {
        let mut next = ty;
        for _ in 0..MAX_BASES {
            let Some(ty) = next else {
                break;
            };
            if let Some(name) = self.names.field(ty, offset) {
                return name.to_owned();
            }
            next = self.project.types.get(&ty).and_then(|facts| facts.base);
        }
        format!("f_{offset:X}")
    }

    /// The name a call is written under.
    pub(super) fn callee(&self, callee: &Callee) -> String {
        let listed = callee_name(callee, self.names);
        if !self.sharp {
            return listed;
        }
        match callee {
            Callee::Local { at, name } => self.function_name(self.head.program, *at, name),
            Callee::Import {
                name, program, at, ..
            } => {
                let module = listed.strip_suffix(name.as_str()).unwrap_or_default();
                format!("{module}{}", self.function_name(*program, *at, name))
            }
            // ⚠ The type the slot is of, never the listing's wrapper class, which no program holds.
            Callee::Virtual {
                ty, method, slot, ..
            } => {
                let named = TypeKey::of_name(ty).and_then(|key| self.method_name(key, *slot));
                format!(
                    "{}.{}",
                    self.type_name(ty),
                    named.unwrap_or(method.as_str())
                )
            }
            Callee::Builtin { path, .. } => match path.rsplit_once('.') {
                Some((owner, member)) => format!("{}.{member}", keyword(owner).unwrap_or(owner)),
                None => listed,
            },
            // `box<System.Int32>`: the type inside takes its keyword.
            Callee::Intrinsic { .. } => {
                let open = listed
                    .strip_suffix('>')
                    .and_then(|open| open.split_once('<'));
                let typed = open.map(|(head, ty)| format!("{head}<{}>", keyword(ty).unwrap_or(ty)));
                typed.unwrap_or(listed)
            }
            Callee::Native { .. } => listed,
        }
    }

    /// Whether `e` is proved to hold a reference or a pointer: a value of such a kind, or what an `as` yields.
    pub(super) fn reference(&self, e: &Expr) -> bool {
        let mut inner = e;
        while let Expr::Sugar { e: under, .. } = inner {
            inner = &**under;
        }
        let cast = matches!(
            inner,
            Expr::Call { callee: Callee::Intrinsic { name, .. }, .. }
                if name.starts_with("try_cast<") || name.starts_with("cast<")
        );
        cast || matches!(
            self.kind_of(e),
            Kind::Object(_) | Kind::String | Kind::Array(_) | Kind::Pointer
        )
    }

    /// `e` read as a `uint`: what stands beside a hex constant in a comparison of 32 bits that does not ask their sign.
    pub(super) fn as_uint(&mut self, e: &Expr) -> (String, u8) {
        let mut inner = e;
        while let Expr::Cast { to, e: under } = inner {
            if to.bytes() < 4 || to.kind().is_float() {
                break;
            }
            inner = &**under;
        }
        match inner {
            Expr::Load {
                ty: Scalar::I32 | Scalar::U32,
                addr,
                offset,
            } => {
                let (place, binds) = self.place_of(addr, *offset, false, Some(4));
                (format!("(uint){}", wrap(place, binds, UNARY)), UNARY)
            }
            _ => self.cast(Scalar::U32, e),
        }
    }

    /// The value whose field the bytes `extract_bytes(value, offset, bytes)` takes are, and the field's offset in it; `None` where they are no field of one value.
    // ⚠ Of a group of cells only where every cell is a variable's or a cell of one, so that
    // writing the one the bytes lie in leaves out no effect of the others. Never a read of
    // memory, whose field reads alike as the memory after it, nor the receiver or a reference,
    // whose field reads as a member of what it refers to.
    pub(super) fn taken_from<'e>(
        &self,
        value: &'e Expr,
        offset: u32,
        bytes: u32,
    ) -> Option<(&'e Expr, u32)> {
        if !self.sharp || bytes == 0 {
            return None;
        }
        let (mut value, mut offset) = (bits(value), offset);
        if let Expr::Cells(items) = value {
            match items.as_slice() {
                [one] => value = bits(one),
                _ => {
                    let cell = offset / 8;
                    let last = offset.checked_add(bytes - 1)? / 8;
                    let plain = items.iter().all(|item| self.one_cell(bits(item)));
                    if cell != last || !plain {
                        return None;
                    }
                    value = bits(items.get(cell as usize)?);
                    offset %= 8;
                }
            }
        }
        let own = match value {
            Expr::Var(var) => self.this != Some(*var) && !self.reference(value),
            Expr::Call { .. } => !self.reference(value),
            Expr::Sugar { kind, e } => {
                !matches!(kind, ExprSugar::Field { .. })
                    && matches!(&**e, Expr::Call { .. })
                    && !self.reference(value)
            }
            _ => false,
        };
        own.then_some((value, offset))
    }

    /// Whether `e` is one cell of a frame that evaluating it reads and does nothing more: a variable of one cell, or one cell of a variable.
    fn one_cell(&self, e: &Expr) -> bool {
        match e {
            Expr::Var(var) => flat_cells(&Expr::Var(*var), self.head) == Some(1),
            Expr::Cell { base, .. } => matches!(&**base, Expr::Var(_)),
            _ => false,
        }
    }

    /// The field `extract_bytes(value, offset, bytes)` takes, written as such without a word on its width: `x.f_4`, and a cell of a group taken whole as that cell.
    pub(super) fn extracted_place(
        &mut self,
        value: &Expr,
        offset: u32,
        bytes: u32,
    ) -> Option<String> {
        let (of, at) = self.taken_from(value, offset, bytes)?;
        if bytes == 8 && at == 0 && self.one_cell(of) {
            return Some(self.expr(of, POSTFIX));
        }
        // ⚠ A dot after a variable of one cell goes through it: its own bytes are its bits
        // shifted down.
        if let Expr::Var(var) = of {
            if self.single(*var) {
                return self.shifted(*var, at, bytes);
            }
        }
        let owner = match self.kind_of(of) {
            Kind::Value { ty, .. } => TypeKey::of_name(&ty),
            _ => None,
        };
        let base = self.expr(of, POSTFIX);
        Some(format!("{base}.{}", self.field_name(owner, at)))
    }

    /// `(x >> 32)`: the variable of one cell `var` shifted down to the `bytes` bytes at byte `at` of it, which a cast in front cuts to their width; `None` past the bytes its kind holds, or where C# shifts no value of its kind.
    // ⚠ Within those bytes the sign a shift brings in reaches none of the bits kept.
    fn shifted(&self, var: Var, at: u32, bytes: u32) -> Option<String> {
        let held: u64 = match self.kind_of(&Expr::Var(var)) {
            Kind::I8 | Kind::U8 => 1,
            Kind::I16 | Kind::U16 => 2,
            Kind::I32 | Kind::U32 => 4,
            Kind::I64 | Kind::U64 | Kind::Unknown => 8,
            _ => return None,
        };
        if u64::from(at) + u64::from(bytes) > held {
            return None;
        }
        let name = self.var(var);
        Some(match at {
            0 => name,
            at => format!("({name} >> {})", u64::from(at) * 8),
        })
    }

    /// `extract_bytes(value, offset, bytes)` as the field it takes, read as the unsigned number of its width, bare for one cell and under its cells for several; `None` for any other width.
    // The cell the bytes come to is zero above them, as the unsigned number is.
    pub(super) fn extracted(
        &mut self,
        value: &Expr,
        offset: u32,
        bytes: u32,
    ) -> Option<(String, u8)> {
        let unsigned = match bytes {
            1 => Some("byte"),
            2 => Some("ushort"),
            4 => Some("uint"),
            n if n.is_multiple_of(8) => None,
            _ => return None,
        };
        let place = self.extracted_place(value, offset, bytes)?;
        Some(match (unsigned, bytes) {
            (Some(ty), _) => (format!("({ty}){place}"), UNARY),
            (None, 8) => (place, POSTFIX),
            (None, n) => {
                let cells = self.kind_text(&Kind::Aggregate(n / 8));
                (format!("({cells}){place}"), UNARY)
            }
        })
    }

    /// The kind of what `extract_bytes(value, offset, bytes)` is written as where it is written as a field: the unsigned number of its width.
    pub(super) fn extracted_kind(&self, value: &Expr, offset: u32, bytes: u32) -> Kind {
        match (bytes, self.taken_from(value, offset, bytes)) {
            (1, Some(_)) => Kind::U8,
            (2, Some(_)) => Kind::U16,
            (4, Some(_)) => Kind::U32,
            _ => Kind::Unknown,
        }
    }

    /// `l op r` where C# would read the operands otherwise than the operator works on them; `None` where it reads them as they stand.
    pub(super) fn typed(
        &mut self,
        op: BinOp,
        at: &Kind,
        l: &Expr,
        r: &Expr,
    ) -> Option<(String, u8)> {
        let p = op.precedence();
        // A reference is compared with `null`, and one above zero as an address is one that is not null.
        let test = match (op, at) {
            (BinOp::Eq, _) => Some("=="),
            (BinOp::Ne, _) | (BinOp::Gt, Kind::U64) => Some("!="),
            _ => None,
        };
        if let Some(test) = test.filter(|_| is_zero(r) && self.reference(bits(l))) {
            return Some((format!("{} {test} null", self.expr(bits(l), 8)), 7));
        }
        // ⚠ `(int)x == 0x9A3B5C7D` widens both sides to `long` in C# and never holds: equality of 32
        // bits does not ask their sign, so the other side is read as the `uint` the constant is.
        // Never for an order, which does ask.
        if matches!(op, BinOp::Eq | BinOp::Ne) && matches!(at, Kind::I32 | Kind::U32) {
            let found = match (high(l), high(r)) {
                (None, Some(c)) => Some((l, c, false)),
                (Some(c), None) => Some((r, c, true)),
                _ => None,
            };
            if let Some((other, c, first)) = found {
                let constant = self.constant(&c).0;
                let (text, binds) = self.as_uint(other);
                let text = match first {
                    true => format!("{constant} {} {}", op.symbol(), wrap(text, binds, p + 1)),
                    false => format!("{} {} {constant}", wrap(text, binds, p), op.symbol()),
                };
                return Some((text, p));
            }
        }
        // ⚠ A bare number on the left of a shift is an `int` to C#, which then shifts 32 bits by
        // the low five of the count: a shift of 64 bits says so. One of 32 bits stays bare.
        let wide = matches!(at, Kind::I64 | Kind::U64 | Kind::Unknown);
        if let (true, true, Expr::Const(c)) = (matches!(op, BinOp::Shl | BinOp::Shr), wide, bits(l))
        {
            let (text, binds) = self.constant(c);
            let number = c.kind.is_integer() || c.kind == Kind::Unknown;
            let digits = text.trim_start_matches('-');
            if number && digits.bytes().all(|b| b.is_ascii_alphanumeric()) {
                let suffix = if *at == Kind::U64 { "UL" } else { "L" };
                let left = wrap(format!("{text}{suffix}"), binds, p);
                let text = format!("{left} {} {}", op.symbol(), self.expr(r, p + 1));
                return Some((text, p));
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "print_spell_tests.rs"]
mod tests;
