//! The decompiler's variables, expressions and simple statements: what a listing states, in a
//! form a bytecode front end can emit too, and what the passes rewrite.

use std::collections::BTreeMap;
use std::fmt;

/// A variable a function reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Var {
    /// `pN`.
    Param(u32),
    /// `local_N`.
    Local(u32),
    /// `v_OFFSET_n`: result `index` of the instruction at bytecode offset `at`.
    Value { at: u32, index: u32 },
    /// `stack_OFFSET_n`: slot `index` of the VM stack on entry to the block at `at`.
    Stack { at: u32, index: u32 },
    /// The cell at byte `offset` of a program's globals, or of its statics.
    Global {
        program: u32,
        offset: u32,
        statics: bool,
    },
}

impl Var {
    /// The variable a listing name spells: `p0`, `local_3`, `v_00001D_0` or `stack_00006C_0`.
    pub fn from_name(name: &str) -> Option<Var> {
        fn decimal(s: &str) -> Option<u32> {
            let digits = !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            digits.then(|| s.parse().ok()).flatten()
        }
        fn at_index(s: &str) -> Option<(u32, u32)> {
            let (at, index) = s.split_once('_')?;
            let hex = at.len() >= 6 && at.bytes().all(|b| b.is_ascii_hexdigit());
            let at = hex.then(|| u32::from_str_radix(at, 16).ok()).flatten()?;
            Some((at, decimal(index)?))
        }
        if let Some(n) = name.strip_prefix("local_") {
            return decimal(n).map(Var::Local);
        }
        if let Some(rest) = name.strip_prefix("v_") {
            return at_index(rest).map(|(at, index)| Var::Value { at, index });
        }
        if let Some(rest) = name.strip_prefix("stack_") {
            return at_index(rest).map(|(at, index)| Var::Stack { at, index });
        }
        name.strip_prefix('p').and_then(decimal).map(Var::Param)
    }

    /// Whether this is a `v_` or `stack_` variable, which the lifter made and the source never had.
    pub fn is_temporary(self) -> bool {
        matches!(self, Var::Value { .. } | Var::Stack { .. })
    }
}

/// The listing's own spelling.
impl fmt::Display for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Var::Param(n) => write!(f, "p{n}"),
            Var::Local(n) => write!(f, "local_{n}"),
            Var::Value { at, index } => write!(f, "v_{at:06X}_{index}"),
            Var::Stack { at, index } => write!(f, "stack_{at:06X}_{index}"),
            Var::Global {
                program,
                offset,
                statics: false,
            } => write!(f, "Vm.Globals(0x{program:08X}U)[0x{offset:X}]"),
            Var::Global {
                program,
                offset,
                statics: true,
            } => write!(f, "Vm.Globals(0x{program:08X}U, true)[0x{offset:X}]"),
        }
    }
}

/// A width and a reading of it: what `as_*` and `load_*` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Scalar {
    pub const ALL: [Scalar; 10] = [
        Scalar::I8,
        Scalar::U8,
        Scalar::I16,
        Scalar::U16,
        Scalar::I32,
        Scalar::U32,
        Scalar::I64,
        Scalar::U64,
        Scalar::F32,
        Scalar::F64,
    ];

    /// The listing's suffix, as in `as_i32` and `load_u64`.
    pub fn name(self) -> &'static str {
        match self {
            Scalar::I8 => "i8",
            Scalar::U8 => "u8",
            Scalar::I16 => "i16",
            Scalar::U16 => "u16",
            Scalar::I32 => "i32",
            Scalar::U32 => "u32",
            Scalar::I64 => "i64",
            Scalar::U64 => "u64",
            Scalar::F32 => "f32",
            Scalar::F64 => "f64",
        }
    }

    pub fn from_name(name: &str) -> Option<Scalar> {
        Scalar::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The C# keyword of the type.
    pub fn keyword(self) -> &'static str {
        match self {
            Scalar::I8 => "sbyte",
            Scalar::U8 => "byte",
            Scalar::I16 => "short",
            Scalar::U16 => "ushort",
            Scalar::I32 => "int",
            Scalar::U32 => "uint",
            Scalar::I64 => "long",
            Scalar::U64 => "ulong",
            Scalar::F32 => "float",
            Scalar::F64 => "double",
        }
    }

    pub fn bytes(self) -> u32 {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::I64 | Scalar::U64 | Scalar::F64 => 8,
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Scalar::I8 => Kind::I8,
            Scalar::U8 => Kind::U8,
            Scalar::I16 => Kind::I16,
            Scalar::U16 => Kind::U16,
            Scalar::I32 => Kind::I32,
            Scalar::U32 => Kind::U32,
            Scalar::I64 => Kind::I64,
            Scalar::U64 => Kind::U64,
            Scalar::F32 => Kind::F32,
            Scalar::F64 => Kind::F64,
        }
    }
}

/// What a `store_*` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoreKind {
    U8,
    U16,
    I32,
    U64,
    /// Eight bytes that are a reference.
    Ref,
}

impl StoreKind {
    pub const ALL: [StoreKind; 5] = [
        StoreKind::U8,
        StoreKind::U16,
        StoreKind::I32,
        StoreKind::U64,
        StoreKind::Ref,
    ];

    /// The listing's suffix, as in `store_ref`.
    pub fn name(self) -> &'static str {
        match self {
            StoreKind::U8 => "u8",
            StoreKind::U16 => "u16",
            StoreKind::I32 => "i32",
            StoreKind::U64 => "u64",
            StoreKind::Ref => "ref",
        }
    }

    pub fn from_name(name: &str) -> Option<StoreKind> {
        StoreKind::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn bytes(self) -> u32 {
        match self {
            StoreKind::U8 => 1,
            StoreKind::U16 => 2,
            StoreKind::I32 => 4,
            StoreKind::U64 | StoreKind::Ref => 8,
        }
    }
}

/// A type across programs: the 64-bit hash of a script type's name, or a builtin type's index.
// A builtin's index has its top bit set and no upper half, which no name hash is taken to have.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct TypeKey(pub u64);

impl TypeKey {
    /// The key of the type named `type_` and sixteen hex digits, or `type_` and a builtin's eight.
    pub fn of_name(name: &str) -> Option<TypeKey> {
        let digits = name.strip_prefix("type_")?;
        let hex = matches!(digits.len(), 8 | 16) && digits.bytes().all(|b| b.is_ascii_hexdigit());
        let key = hex
            .then(|| u64::from_str_radix(digits, 16).ok())
            .flatten()?;
        (digits.len() == 16 || key & 0x8000_0000 != 0).then_some(TypeKey(key))
    }

    /// The key of a type as a program states it: a builtin by its id, any other by its name.
    pub fn of(ty: &VmType) -> Option<TypeKey> {
        match ty.id & 0x8000_0000 != 0 {
            true => Some(TypeKey(u64::from(ty.id))),
            false => TypeKey::of_name(&ty.name),
        }
    }

    /// The index of a builtin type, `None` for a script type.
    pub fn builtin(self) -> Option<u32> {
        (self.0 >> 32 == 0 && self.0 & 0x8000_0000 != 0).then_some(self.0 as u32)
    }
}

/// The name a type no list names prints under.
impl fmt::Display for TypeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.builtin() {
            Some(index) => write!(f, "type_{index:08X}"),
            None => write!(f, "type_{:016X}", self.0),
        }
    }
}

/// A function across programs: its program's hash and its bytecode offset.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct FunctionKey(pub u64);

impl FunctionKey {
    pub fn new(program: u32, at: u32) -> FunctionKey {
        FunctionKey(u64::from(program) << 32 | u64::from(at))
    }

    pub fn program(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub fn at(self) -> u32 {
        self.0 as u32
    }
}

/// What a value is, as far as anything says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    /// A 64-bit cell nothing has typed.
    #[default]
    Unknown,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    /// A 32-bit name hash.
    Hash,
    Pointer,
    /// A reference to an object of the type named.
    Object(String),
    String,
    /// Three floats in two cells, as `vector3` packs them.
    Vector,
    /// A 16-byte handle in two cells.
    Handle,
    /// A value of this many cells whose layout nothing states.
    Aggregate(u32),
    /// A value of the type named, held in place in `cells` cells: a struct, an enum, a nullable.
    Value {
        ty: String,
        cells: u32,
    },
    /// A reference to an array of elements of this kind.
    Array(Box<Kind>),
}

impl Kind {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Kind::I8
                | Kind::U8
                | Kind::I16
                | Kind::U16
                | Kind::I32
                | Kind::U32
                | Kind::I64
                | Kind::U64
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Kind::F32 | Kind::F64)
    }

    /// The type C# widens an operand of this integer kind to before an operator works on it.
    fn promoted(&self) -> Kind {
        match self {
            Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 => Kind::I32,
            other => other.clone(),
        }
    }
}

/// A constant cell.
// The reader sets `kind` to the C# type of the literal, which says how the bits were widened and
// nothing of what the value means; the types pass replaces it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Const {
    pub bits: u64,
    pub kind: Kind,
}

impl Const {
    pub fn i32(value: i32) -> Const {
        Const {
            bits: value as i64 as u64,
            kind: Kind::I32,
        }
    }

    pub fn f32(value: f32) -> Const {
        Const {
            bits: u64::from(value.to_bits()),
            kind: Kind::F32,
        }
    }

    pub fn bool(value: bool) -> Const {
        Const {
            bits: u64::from(value),
            kind: Kind::Bool,
        }
    }

    /// The low 32 bits as a float.
    pub fn as_f32(&self) -> f32 {
        f32::from_bits(self.bits as u32)
    }

    pub fn as_f64(&self) -> f64 {
        f64::from_bits(self.bits)
    }
}

/// `new VmType("name", byteSize, idU, 0xFLAGSU)`: a type as the listing describes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VmType {
    /// `type_` and sixteen hex digits, or a runtime type's own name such as `System.Delegate`.
    pub name: String,
    pub byte_size: u32,
    pub id: u32,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// `!`.
    Not,
    /// `~`.
    BitNot,
    /// `-`.
    Neg,
}

impl UnOp {
    pub fn symbol(self) -> &'static str {
        match self {
            UnOp::Not => "!",
            UnOp::BitNot => "~",
            UnOp::Neg => "-",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Xor,
    Or,
    LogicalAnd,
    LogicalOr,
}

impl BinOp {
    pub const ALL: [BinOp; 18] = [
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
        BinOp::Add,
        BinOp::Sub,
        BinOp::Shl,
        BinOp::Shr,
        BinOp::Lt,
        BinOp::Le,
        BinOp::Gt,
        BinOp::Ge,
        BinOp::Eq,
        BinOp::Ne,
        BinOp::And,
        BinOp::Xor,
        BinOp::Or,
        BinOp::LogicalAnd,
        BinOp::LogicalOr,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::And => "&",
            BinOp::Xor => "^",
            BinOp::Or => "|",
            BinOp::LogicalAnd => "&&",
            BinOp::LogicalOr => "||",
        }
    }

    pub fn from_symbol(symbol: &str) -> Option<BinOp> {
        BinOp::ALL.into_iter().find(|op| op.symbol() == symbol)
    }

    /// C#'s binding strength, from 2 for `||` to 11 for `*`; a unary operator binds at 12.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Rem => 11,
            BinOp::Add | BinOp::Sub => 10,
            BinOp::Shl | BinOp::Shr => 9,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 8,
            BinOp::Eq | BinOp::Ne => 7,
            BinOp::And => 6,
            BinOp::Xor => 5,
            BinOp::Or => 4,
            BinOp::LogicalAnd => 3,
            BinOp::LogicalOr => 2,
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
        )
    }

    /// Whether the result is a truth value whatever the operands are.
    pub fn is_logical(self) -> bool {
        self.is_comparison() || matches!(self, BinOp::LogicalAnd | BinOp::LogicalOr)
    }
}

/// What a call reaches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    /// A function of the program being read, called by its own name.
    Local { at: u32, name: String },
    /// `Imports.<module>.<name>`: the function at `at` of another program.
    Import {
        module: String,
        name: String,
        program: u32,
        at: u32,
    },
    /// `Native.<namespace>.<name>`, or `Native.<name>` where the listing has no namespace for it.
    Native {
        namespace: Option<String>,
        name: String,
        hash: u64,
        rva: u32,
    },
    /// `Builtins.<path>`: method `method` of the runtime's type `ty`.
    Builtin { path: String, ty: u32, method: u32 },
    /// `Virtual.<class>.<method>`: slot `slot` of the type named `ty`.
    Virtual {
        class: String,
        method: String,
        ty: String,
        slot: u32,
    },
    /// `Operations.<wrapper>`, or `Vm.Intrinsic` and `Vm.TypedIntrinsic` written in place.
    Intrinsic {
        name: String,
        ty: Option<VmType>,
        wrapper: Option<String>,
    },
}

impl Callee {
    /// The path of the declaration the listing calls this through, `None` where it has none.
    pub fn path(&self) -> Option<String> {
        match self {
            Callee::Local { .. } => None,
            Callee::Import { module, name, .. } => Some(format!("Imports.{module}.{name}")),
            Callee::Native {
                namespace: Some(namespace),
                name,
                ..
            } => Some(format!("Native.{namespace}.{name}")),
            Callee::Native {
                namespace: None,
                name,
                ..
            } => Some(format!("Native.{name}")),
            Callee::Builtin { path, .. } => Some(format!("Builtins.{path}")),
            Callee::Virtual { class, method, .. } => Some(format!("Virtual.{class}.{method}")),
            Callee::Intrinsic { wrapper, .. } => {
                wrapper.as_ref().map(|w| format!("Operations.{w}"))
            }
        }
    }
}

/// The source construct an expression reads as; the value and the effects stay the expression's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExprSugar {
    /// `receiver.Method(args)`: a call whose first argument is its receiver.
    Method,
    /// `base.Name`: a load of a field of the type `owner`.
    Field { owner: TypeKey },
    /// `receiver.Name`: a call of a getter, whose one argument is its receiver.
    Property,
    /// `a + b`: a call of an operator a type declares.
    Operator,
    /// `(args) => body`: a reference to a function that is a lambda's body.
    Lambda,
    /// `a ?? b`: a select of `a` where `a` is not null.
    Coalesce,
    /// `a?.b`: a select of null where `a` is null.
    NullConditional,
    /// `a + b` over strings, or an interpolated string: a call that joins its arguments.
    Concat,
    /// `(T)x`: a call that boxes, unboxes or casts.
    Convert,
    /// `typeof(T)`: a call that yields a type's handle.
    TypeOf,
    /// `out x`: the address of a variable the callee fills.
    Out,
    /// `ref x`: the address of a variable the callee reads and may write.
    Ref,
    /// `(a, b)`: cells that are a tuple.
    Tuple,
    /// `x is T`: a comparison of what `x as T` makes with null, whole; `!(x is T)` where it asks for null.
    Is,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    /// `e` read as the source construct `kind`; [`Expr::desugar`] leaves `e`.
    // ⚠ Made on the structured form only: the passes over blocks run before any exists.
    Sugar {
        kind: ExprSugar,
        e: Box<Expr>,
    },
    Const(Const),
    Var(Var),
    /// `GuidEmpty`: two zero cells.
    GuidEmpty,
    /// `base[index]`: one cell of a value several cells wide.
    Cell {
        base: Box<Expr>,
        index: u32,
    },
    /// `new VmValues(a, b)`: cells gathered into one value.
    Cells(Vec<Expr>),
    /// `as_T(e)`: the low bits of `e` read as `to`.
    Cast {
        to: Scalar,
        e: Box<Expr>,
    },
    /// `Truth(e)`: whether any bit of `e` is set.
    Truth(Box<Expr>),
    Unary {
        op: UnOp,
        e: Box<Expr>,
    },
    /// `at` is the type the operator works at; `Unknown` compares whole cells.
    Binary {
        op: BinOp,
        at: Kind,
        l: Box<Expr>,
        r: Box<Expr>,
    },
    /// `cond ? then : els`.
    Select {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
    /// `returns` is the cells the call yields, `None` where the listing does not state it.
    Call {
        callee: Callee,
        args: Vec<Expr>,
        returns: Option<u32>,
    },
    /// `load_T(addr, offset)`.
    Load {
        ty: Scalar,
        addr: Box<Expr>,
        offset: u32,
    },
    /// `load_bytes(addr, offset, bytes)`: `bytes` bytes as whole cells.
    LoadBytes {
        addr: Box<Expr>,
        offset: u32,
        bytes: u32,
    },
    /// `extract_bytes(value, offset, bytes)`: `bytes` bytes of a value as whole cells.
    ExtractBytes {
        value: Box<Expr>,
        offset: u32,
        bytes: u32,
    },
    /// `byte_offset(addr, offset)`: an address moved by a count of bytes.
    ByteOffset {
        addr: Box<Expr>,
        offset: Box<Expr>,
    },
    /// `Vm.Address(..)` and `Vm.GlobalAddress(..)`: the address of a variable, or of cell `cell` of one.
    AddrOf {
        var: Var,
        cell: u32,
    },
    /// `zero_slots(n)`: `n` zero cells.
    ZeroSlots(u32),
    /// `vector3(x, y, z)`: x and y packed into one cell and z into the next.
    Vector3(Box<[Expr; 3]>),
    NewObject(VmType),
    /// `Operations.<wrapper>(len)`: a new array of `len` elements.
    NewArray {
        ty: VmType,
        len: Box<Expr>,
        wrapper: String,
    },
    /// `Vm.Function(program, offset)`: a reference to a script function.
    FunctionRef {
        program: u32,
        at: u32,
    },
    /// `Vm.BuiltinFunction(type, method)`.
    BuiltinFunctionRef {
        ty: u32,
        method: u32,
    },
    /// `Vm.VirtualFunction(receiver, type, slot)`.
    VirtualFunctionRef {
        receiver: Box<Expr>,
        ty: VmType,
        slot: u32,
    },
    /// `Vm.StringConstant(program, offset)`: a string of a program's heap.
    StringConstant {
        program: u32,
        offset: u32,
    },
    /// `Vm.BuiltinGlobal(n)`: cells the runtime owns.
    BuiltinGlobal(u32),
}

impl Expr {
    /// `l op r`, working at the type C# gives the two operands.
    pub fn binary(op: BinOp, l: Expr, r: Expr) -> Expr {
        let at = operator_kind(op, &l, &r);
        Expr::Binary {
            op,
            at,
            l: Box::new(l),
            r: Box::new(r),
        }
    }

    pub fn cast(to: Scalar, e: Expr) -> Expr {
        Expr::Cast { to, e: Box::new(e) }
    }

    /// `e` read as the source construct `kind`.
    pub fn sugar(kind: ExprSugar, e: Expr) -> Expr {
        Expr::Sugar {
            kind,
            e: Box::new(e),
        }
    }

    /// Takes every source reading off this expression and off those inside it.
    pub fn desugar(&mut self) {
        self.walk_mut(&mut |node| {
            if let Expr::Sugar { e: inner, .. } = node {
                let inner = std::mem::replace(&mut **inner, Expr::GuidEmpty);
                *node = inner;
            }
        });
    }

    /// The C# type the listing gives this expression, `Unknown` for an untyped cell.
    // Meaningful on what the reader made; a kind the types pass set on a constant shows through.
    pub fn listed_kind(&self) -> Kind {
        match self {
            Expr::Const(c) => c.kind.clone(),
            Expr::Cast { to, .. } => to.kind(),
            Expr::Truth(_) => Kind::Bool,
            Expr::Unary { op: UnOp::Not, .. } => Kind::Bool,
            Expr::Unary {
                op: UnOp::BitNot,
                e,
            } => match e.listed_kind() {
                k if k.is_integer() => k.promoted(),
                _ => Kind::Unknown,
            },
            Expr::Unary { op: UnOp::Neg, e } => match e.listed_kind() {
                Kind::U32 => Kind::I64,
                k if k.is_integer() => k.promoted(),
                k if k.is_float() => k,
                _ => Kind::Unknown,
            },
            Expr::Binary { op, .. } if op.is_logical() => Kind::Bool,
            Expr::Binary { at, .. } => at.clone(),
            _ => Kind::Unknown,
        }
    }

    /// Calls `f` on this expression and then on every expression inside it.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Const(_)
            | Expr::Var(_)
            | Expr::GuidEmpty
            | Expr::AddrOf { .. }
            | Expr::ZeroSlots(_)
            | Expr::NewObject(_)
            | Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::StringConstant { .. }
            | Expr::BuiltinGlobal(_) => {}
            Expr::Cell { base: e, .. }
            | Expr::Sugar { e, .. }
            | Expr::Cast { e, .. }
            | Expr::Truth(e)
            | Expr::Unary { e, .. }
            | Expr::Load { addr: e, .. }
            | Expr::LoadBytes { addr: e, .. }
            | Expr::ExtractBytes { value: e, .. }
            | Expr::NewArray { len: e, .. }
            | Expr::VirtualFunctionRef { receiver: e, .. } => e.walk(f),
            Expr::Binary { l, r, .. } => {
                l.walk(f);
                r.walk(f);
            }
            Expr::ByteOffset { addr, offset } => {
                addr.walk(f);
                offset.walk(f);
            }
            Expr::Select { cond, then, els } => {
                cond.walk(f);
                then.walk(f);
                els.walk(f);
            }
            Expr::Cells(items) | Expr::Call { args: items, .. } => {
                for e in items {
                    e.walk(f);
                }
            }
            Expr::Vector3(items) => {
                for e in items.iter() {
                    e.walk(f);
                }
            }
        }
    }

    /// Calls `f` on every expression inside this one and then on this one, so `f` may replace what it is given.
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut Expr)) {
        match self {
            Expr::Const(_)
            | Expr::Var(_)
            | Expr::GuidEmpty
            | Expr::AddrOf { .. }
            | Expr::ZeroSlots(_)
            | Expr::NewObject(_)
            | Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::StringConstant { .. }
            | Expr::BuiltinGlobal(_) => {}
            Expr::Cell { base: e, .. }
            | Expr::Sugar { e, .. }
            | Expr::Cast { e, .. }
            | Expr::Truth(e)
            | Expr::Unary { e, .. }
            | Expr::Load { addr: e, .. }
            | Expr::LoadBytes { addr: e, .. }
            | Expr::ExtractBytes { value: e, .. }
            | Expr::NewArray { len: e, .. }
            | Expr::VirtualFunctionRef { receiver: e, .. } => e.walk_mut(f),
            Expr::Binary { l, r, .. } => {
                l.walk_mut(f);
                r.walk_mut(f);
            }
            Expr::ByteOffset { addr, offset } => {
                addr.walk_mut(f);
                offset.walk_mut(f);
            }
            Expr::Select { cond, then, els } => {
                cond.walk_mut(f);
                then.walk_mut(f);
                els.walk_mut(f);
            }
            Expr::Cells(items) | Expr::Call { args: items, .. } => {
                for e in items {
                    e.walk_mut(f);
                }
            }
            Expr::Vector3(items) => {
                for e in items.iter_mut() {
                    e.walk_mut(f);
                }
            }
        }
        f(self);
    }
}

/// The type a C# operator works at over two operands, by the types the listing gives them.
// An `int` literal that is not negative takes the other operand's unsigned type, as C# has it;
// `uint` against a signed operand works as `long`.
pub fn operator_kind(op: BinOp, l: &Expr, r: &Expr) -> Kind {
    let (a, b) = (l.listed_kind(), r.listed_kind());
    match op {
        BinOp::LogicalAnd | BinOp::LogicalOr => return Kind::Bool,
        BinOp::Shl | BinOp::Shr => {
            return if a.is_integer() {
                a.promoted()
            } else {
                Kind::Unknown
            };
        }
        _ => {}
    }
    if a == Kind::Bool && b == Kind::Bool {
        return Kind::Bool;
    }
    let numeric = |k: &Kind| k.is_integer() || k.is_float();
    if !numeric(&a) || !numeric(&b) {
        return Kind::Unknown;
    }
    for wide in [Kind::F64, Kind::F32, Kind::U64, Kind::I64] {
        if a == wide || b == wide {
            return wide;
        }
    }
    let signed = |k: &Kind| matches!(k, Kind::I8 | Kind::I16 | Kind::I32);
    let literal =
        |e: &Expr| matches!(e, Expr::Const(c) if c.kind == Kind::I32 && (c.bits as i64) >= 0);
    match (a == Kind::U32, b == Kind::U32) {
        (true, _) if signed(&b) && !literal(r) => Kind::I64,
        (_, true) if signed(&a) && !literal(l) => Kind::I64,
        (false, false) => Kind::I32,
        _ => Kind::U32,
    }
}

/// A statement with no control flow in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `target = value;`.
    Assign { target: Var, value: Expr },
    /// `(a, b) = (x, y);`: every value is read before any target is written.
    AssignMany {
        targets: Vec<Var>,
        values: Vec<Expr>,
    },
    /// `store_T(addr, offset, value);`.
    Store {
        kind: StoreKind,
        addr: Expr,
        offset: u32,
        value: Expr,
    },
    /// `store_bytes(addr, offset, bytes, value);`: the first `bytes` bytes of `value`'s cells.
    StoreBytes {
        addr: Expr,
        offset: u32,
        bytes: u32,
        value: Expr,
    },
    /// `store_f32_slots(addr, offset, value);`: the low four bytes of each cell, one after another.
    StoreF32Slots {
        addr: Expr,
        offset: u32,
        value: Expr,
    },
    /// `zero_bytes(addr, bytes);`.
    ZeroBytes { addr: Expr, bytes: u32 },
    /// `increment_i32(addr, offset);`, or `decrement_i32` when `down`.
    StepI32 { addr: Expr, offset: u32, down: bool },
    /// A call whose result, if it has one, is dropped.
    Eval(Expr),
}

impl Stmt {
    /// The variables the statement assigns; a store through an address names none.
    pub fn targets(&self) -> &[Var] {
        match self {
            Stmt::Assign { target, .. } => std::slice::from_ref(target),
            Stmt::AssignMany { targets, .. } => targets,
            _ => &[],
        }
    }

    /// Calls `f` on each expression the statement evaluates, in the order it evaluates them.
    pub fn each_expr<'a>(&'a self, f: &mut impl FnMut(&'a Expr)) {
        match self {
            Stmt::Assign { value, .. } => f(value),
            Stmt::AssignMany { values, .. } => {
                for value in values {
                    f(value);
                }
            }
            Stmt::Store { addr, value, .. }
            | Stmt::StoreBytes { addr, value, .. }
            | Stmt::StoreF32Slots { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Stmt::ZeroBytes { addr, .. } | Stmt::StepI32 { addr, .. } => f(addr),
            Stmt::Eval(e) => f(e),
        }
    }

    pub fn each_expr_mut(&mut self, f: &mut impl FnMut(&mut Expr)) {
        match self {
            Stmt::Assign { value, .. } => f(value),
            Stmt::AssignMany { values, .. } => {
                for value in values {
                    f(value);
                }
            }
            Stmt::Store { addr, value, .. }
            | Stmt::StoreBytes { addr, value, .. }
            | Stmt::StoreF32Slots { addr, value, .. } => {
                f(addr);
                f(value);
            }
            Stmt::ZeroBytes { addr, .. } | Stmt::StepI32 { addr, .. } => f(addr),
            Stmt::Eval(e) => f(e),
        }
    }
}

/// What a function hands back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Returns {
    Void,
    /// One cell.
    Cell,
    /// Several cells; the `return` statements say how many.
    Cells,
}

/// Whether a variable holds one cell or several.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    Cell,
    Cells,
}

/// What is known of one variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarInfo {
    pub width: Width,
    /// `Unknown` until the types pass says otherwise.
    pub kind: Kind,
    /// The name to print in place of the slot name, once the names pass has one.
    pub name: Option<String>,
    /// Whether the form a front end made proves the local a variable of one cell that no other variable runs across.
    pub single: bool,
}

impl VarInfo {
    pub fn new(width: Width) -> VarInfo {
        VarInfo {
            width,
            kind: Kind::Unknown,
            name: None,
            single: false,
        }
    }
}

/// A function apart from its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// The hash of the program the function belongs to.
    pub program: u32,
    /// The function's bytecode offset.
    pub at: u32,
    /// `func_` and the offset, `main` or `initialize`.
    pub name: String,
    pub returns: Returns,
    pub params: u32,
    pub locals: u32,
    /// Every `v_` and `stack_` variable the function declares, and any other a pass has learned of.
    pub vars: BTreeMap<Var, VarInfo>,
    /// Why the listing holds no body for the function, where it holds none.
    pub incomplete: Option<String>,
    /// The value type and its cells that instructions name for the variable starting at each key: a whole value of the type stored there, made there or taken out of a box into it; `None` where they name two.
    // ⚠ Each is what one place of the code states: a frame cell may hold another value elsewhere,
    // so a pass reads it as evidence beside the variable's own cells, and a variable merged with
    // another keeps no promise of it. A listing states none.
    pub value_types: BTreeMap<Var, Option<(TypeKey, u32)>>,
}

#[cfg(test)]
#[path = "ir_tests.rs"]
mod tests;
