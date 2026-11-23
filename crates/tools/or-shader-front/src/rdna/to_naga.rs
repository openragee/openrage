//! A pixel or vertex program as a naga module: one lane, every register a 32-bit local, the graph
//! structured into ifs and loops, tables followed to the resources they name, and everything this
//! front end does not model refused by name.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::num::NonZeroU32;

use naga::{
    BinaryOperator as Bo, Block, DerivativeAxis, DerivativeControl, Expression as E, Handle,
    ImageClass, ImageDimension, Literal, MathFunction as Mf, RelationalFunction, SampleLevel,
    Scalar, ScalarKind, Span, Statement, TypeInner, UnaryOperator as Uo, VectorSize,
};

use super::cfg::{Cfg, Terminator};
use super::decode::{Inst, Operand};
use super::sym::{self, Index, IndexSource, Loc, State, Sym, EXEC, VCC};
use super::Program;
use crate::dxil::ShaderKind;

mod buffers;
mod flow;
mod geometry;
mod halves;
mod wave;

use flow::Flow;
pub use geometry::{emitted, Emitted, Pair};
use halves::Rounding;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotTranslated {
    #[error("{name} at word {pc} is not translated")]
    Opcode { pc: u32, name: String },
    #[error("{name} at word {pc} is not translated with {what}")]
    Form {
        pc: u32,
        name: String,
        what: &'static str,
    },
    #[error("{register} is read at word {pc} before the program writes it: a system value or a user datum this front end does not model")]
    LiveIn { pc: u32, register: String },
    #[error("the operand {operand} at word {pc} is not translated")]
    Operand { pc: u32, operand: String },
    #[error("the instruction at word {pc} reads {what}")]
    Descriptor { pc: u32, what: &'static str },
    #[error("the control flow is not structured: {0}")]
    Structure(String),
    #[error("the jump through a register at word {pc} is not followed")]
    Indirect { pc: u32 },
    #[error("the {0} stage is not translated")]
    Stage(String),
    #[error("a vertex program that exports no position")]
    NoPosition,
    #[error("{0}")]
    Pipeline(&'static str),
}

/// Why a geometry program is not translated, and why the vertex half of its pair is not.
const GEOMETRY: &str = "a geometry program: it reads its primitive's vertices from memory the \
    vertex half of its pair wrote, and leaves the vertices it emits there for code the \
    pipeline holds to export, in a stage wgpu does not have";
/// Why a pixel program that waits its turn at a pixel is not translated.
const ORDERED: &str = "a pixel program shaded in primitive order: it holds every later \
    primitive's pixel at its place until it has read and written its targets, which no stage \
    of a device does";
/// Why a program that intersects rays itself is not translated.
const RAY_TRACING: &str = "a program that traces rays: it walks an acceleration structure's \
    memory by address and tests each node with image_bvh_intersect_ray, where a device gives a \
    structure to a ray query and to nothing else";
const VERTEX_HALF: &str = "the vertex half of a geometry pair: it leaves its vertex in memory \
    its group shares and returns to the pipeline's own code, with no position exported";

impl NotTranslated {
    /// The short stable word a census counts a refusal by: the opcode or the construct, without its place.
    pub fn name(&self) -> String {
        match self {
            Self::Opcode { name, .. } => name.clone(),
            Self::Form { name, what, .. } => format!("{name}: {what}"),
            Self::LiveIn { register, .. } => {
                let class: String = register
                    .chars()
                    .take_while(|c| !c.is_ascii_digit())
                    .collect();
                format!("live-in {class}")
            }
            Self::Operand { operand, .. } => format!("operand {operand}"),
            Self::Descriptor { what, .. } => (*what).to_owned(),
            Self::Structure(_) => "control flow".to_owned(),
            Self::Indirect { .. } => "indirect jump".to_owned(),
            Self::Stage(stage) => format!("{stage} stage"),
            Self::NoPosition => "no position".to_owned(),
            Self::Pipeline(why) => (why.split(':').next().unwrap_or(why)).to_owned(),
        }
    }
}

/// What a resource of a translated module is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// The 32 dwords the pipeline loads into `s0`..`s31`, as eight rows of four.
    UserData,
    /// `rows` rows of four dwords.
    ConstantBuffer {
        rows: u32,
    },
    /// `dim` is the instruction's dimension field: 0 one-dimensional, 1 two-dimensional, 2
    /// three-dimensional, 3 a cube, 5 a two-dimensional array.
    Texture {
        dim: u8,
        compared: bool,
    },
    Sampler {
        comparison: bool,
    },
    /// Dwords read by address. With no `stride` the first dword is the element size an indexed read
    /// multiplies by and the elements start at the fifth; with one, the code stated it and they start at the first.
    Buffer {
        stride: Option<u32>,
    },
    /// The dwords of a table itself, as rows of four: what a program reads of a descriptor as a
    /// number, such as an image's size or a buffer's length.
    TableWords,
    /// A texture the program stores texels to or updates them in one step, declared in
    /// `format`: an update states its own, a store takes [`Options::storage_formats`]'s or
    /// 16-bit floats. `dim` is as a [`ResourceKind::Texture`]'s.
    StorageTexture {
        dim: u8,
        format: naga::StorageFormat,
    },
}

/// One binding of a translated module and the table word its descriptor starts at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub kind: ResourceKind,
    /// `None` for the user data, which no table holds, and for a descriptor the code wrote itself. A
    /// buffer with a stride is the memory the user registers `root` and `root + 1` point at.
    pub at: Option<Loc>,
    /// The four words of a sampler the code builds from constants.
    pub words: Option<[u32; 4]>,
    pub group: u32,
    pub binding: u32,
    /// `Some(n)`: a binding array of the first `n` entries of the table `at` names, which the
    /// program indexes by a place it computes (32 bytes an entry, 16 for a sampler). The
    /// buffers of a table, and its constant buffers, are one buffer of dwords instead, since
    /// no array of them binds on Direct3D: its first `n` dwords are the dword each entry's
    /// buffer starts at, and the buffers follow.
    pub count: Option<u32>,
    /// Whether the program stores to it: a buffer bound to be written as well as read.
    pub written: bool,
}

/// A program translated: the module, what it binds, and each way it departs from the hardware, counted.
#[derive(Debug, Clone)]
pub struct Lowered {
    pub module: naga::Module,
    pub resources: Vec<Resource>,
    pub approximations: BTreeMap<&'static str, u32>,
    /// The parameter slots a pixel module reads as 16-bit halves: the stage before it has to export those as halves.
    pub half_slots: BTreeSet<u8>,
    /// Whether the module holds subgroup operations, which a device has to allow: a wave of the
    /// program is the subgroup the device runs an invocation in.
    pub subgroups: bool,
}

/// What a lowering is told of the program's neighbours.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The parameter slots the pixel stage reads as 16-bit halves ([`Lowered::half_slots`]).
    pub half_slots: BTreeSet<u8>,
    /// The format of each texture the program stores to, by the place of its descriptor
    /// ([`Resource::at`]): a module declares a stored texture's format, which only the texture bound states.
    pub storage_formats: BTreeMap<Loc, naga::StorageFormat>,
    /// Whether a wave is the subgroup its device runs the invocation in, which the device has
    /// to allow ([`Lowered::subgroups`]) and which is the program's wave only on a device as
    /// wide, filled in thread order. Otherwise a pixel is a wave of one lane: a fold over the
    /// wave is the pixel's own value. And a compute group that is one wave keeps it in memory
    /// the group shares, the same on every device; a group of several waves has only the
    /// subgroup.
    pub wave: bool,
    /// The other half of a geometry pair: the geometry program a vertex half runs before, or the
    /// vertex half a geometry program runs after. With it either half lowers to the pair's one module.
    pub pair: Option<std::sync::Arc<Pair>>,
}

/// The first export target of each kind: the colour targets start at 0.
const MRT_COUNT: u8 = 8;
const MRTZ: u8 = 8;
const NULL_TARGET: u8 = 9;
const POS0: u8 = 12;
const PARAM0: u8 = 32;
/// The user registers the pipeline loads, and the rows of four they make.
const USER_REGISTERS: u8 = 32;
/// The rows a constant buffer read at a computed offset is declared with: 64 KiB, the most a uniform block is sure to hold.
const DYNAMIC_ROWS: u32 = 4096;
/// The entries a table bound whole is declared with.
// ponytail: the furthest constant slot seen was under 200; a table longer than this needs its own count from the binder.
const TABLE_SLOTS: u32 = 256;
/// The dwords of a table entry: an image's or a buffer's descriptor, and a sampler's.
const SLOT_DWORDS: u32 = 8;
const SAMPLER_DWORDS: u32 = 4;
/// The fetch routine leaves component `c` of the input in register `r` here.
const FETCH_BASE: u32 = 4;
/// The location of a parameter's high halves, past every parameter's own.
// ponytail: a device that links fewer than 64 locations refuses the pipeline; pack the halves densely if one matters.
const HIGH_HALVES: u32 = 32;
/// The export target of a mesh program's primitive: three vertex numbers and a cull bit.
const PRIM: u8 = 20;
/// The threads of a mesh program's group, each of which may place one vertex and one primitive.
// ponytail: every mesh program of the build was reported to state 128 threads; a program's own count belongs in `Program`.
const MESH_THREADS: u32 = 128;
/// The lanes of the wave a mesh program was compiled for.
const WAVE: u32 = 64;
/// The registers an array kept in registers is taken to fill at most.
const REGISTER_ARRAY: u8 = 64;
/// The first vector register that holds a pixel program's system values, after the barycentric pair.
const SYSTEM_BASE: u8 = 2;
/// The vector register a vertex program finds its instance's number in.
const INSTANCE: u8 = 3;
/// The export target of the first four clip distances.
const POS1: u8 = 13;
const TAU: f32 = std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Reg {
    V(u8),
    S(u8),
    Scc,
    /// The condition block `0` ends on.
    Cond(u32),
    /// The block a dispatch loop runs next.
    Next,
    /// One lane of a vector register the program keeps a scalar register in.
    Spill(u8, u8),
    /// The place in its table, in dwords, of the table word a scalar register holds.
    Place(u8),
    /// What the structured form keeps of a branch on its way: one of the four below.
    Way(u8),
}

/// The merge block control is on its way to, plus one; nothing while it is on its way nowhere.
const LEAVING: Reg = Reg::Way(0);
/// The loops still to leave.
const LOOPS_LEFT: Reg = Reg::Way(1);
/// Whether the loop reached by leaving is to run again.
const AGAIN: Reg = Reg::Way(2);
/// How a function statements were moved to ended: by falling off its end, or one of the three below.
const ENDED: Reg = Reg::Way(3);
const ENDED_BREAK: u32 = 1;
const ENDED_CONTINUE: u32 = 2;
const ENDED_RETURN: u32 = 3;

impl Reg {
    fn name(self) -> String {
        match self {
            Self::V(n) => format!("v{n}"),
            Self::S(n) => match n {
                106 => "vcc_lo".to_owned(),
                107 => "vcc_hi".to_owned(),
                108..=123 => format!("ttmp{}", n - 108),
                124 => "m0".to_owned(),
                126 => "exec_lo".to_owned(),
                127 => "exec_hi".to_owned(),
                _ => format!("s{n}"),
            },
            Self::Scc => "scc".to_owned(),
            Self::Cond(block) => format!("cond{block}"),
            Self::Next => "next".to_owned(),
            Self::Spill(n, lane) => format!("v{n}_lane{lane}"),
            Self::Place(n) => format!("s{n}_place"),
            Self::Way(0) => "leaving".to_owned(),
            Self::Way(1) => "loops_left".to_owned(),
            Self::Way(2) => "again".to_owned(),
            Self::Way(_) => "ended".to_owned(),
        }
    }
}

/// A function's expressions and statements as they are built, with the `Emit` of those not yet covered.
#[derive(Default)]
struct Builder {
    fun: naga::Function,
    blocks: Vec<Block>,
    pending: Option<(Handle<E>, Handle<E>)>,
    /// How many operators deep each expression is written where it is used, by its index.
    depths: Vec<u8>,
}

/// The most statements one block of a long run holds: the run is written as blocks of this
/// many, one after another. naga's Metal writer forgets a block's named values one by one when
/// the block ends, at a cost that grows with the square of how many it named.
const RUN: usize = 64;
/// The most constructs of the structured form one block of a long list holds.
const LIST: usize = 16;

/// The deepest tree of operators a statement is written with: a deeper one is given a name, so
/// it is written once on a line of its own. naga's writers recurse a frame for every operand.
const MAX_DEPTH: u8 = 12;

impl Builder {
    fn new() -> Self {
        Self {
            blocks: vec![Block::new()],
            ..Self::default()
        }
    }

    fn emit(&mut self, expression: E) -> Handle<E> {
        // naga refuses an `Emit` of an expression in scope from the start.
        let covered = !expression.needs_pre_emit();
        if !covered {
            self.flush();
        }
        let depths = &self.depths;
        let of = |operand: &Handle<E>| depths.get(operand.index()).copied().unwrap_or(0);
        // A load, a literal or an access is written as it stands or once: nothing is under it.
        let mut depth = match &expression {
            E::Binary { left, right, .. } => 1 + of(left).max(of(right)),
            E::Unary { expr, .. } | E::As { expr, .. } => 1 + of(expr),
            E::Select {
                condition,
                accept,
                reject,
            } => 1 + of(condition).max(of(accept)).max(of(reject)),
            E::Math {
                arg, arg1, arg2, ..
            } => 1 + (arg1.iter().chain(arg2)).fold(of(arg), |deep, operand| deep.max(of(operand))),
            _ => 0,
        };
        let handle = self.fun.expressions.append(expression, Span::UNDEFINED);
        if depth > MAX_DEPTH {
            (self.fun.named_expressions).insert(handle, format!("t{}", handle.index()));
            depth = 0;
        }
        self.depths.resize(handle.index() + 1, 0);
        self.depths[handle.index()] = depth;
        if covered {
            self.pending = Some((self.pending.map_or(handle, |(first, _)| first), handle));
        }
        handle
    }

    /// An expression a statement gives its value, which no `Emit` covers.
    fn result(&mut self, expression: E) -> Handle<E> {
        self.flush();
        self.fun.expressions.append(expression, Span::UNDEFINED)
    }

    fn flush(&mut self) {
        if let Some((first, last)) = self.pending.take() {
            if let Some(block) = self.blocks.last_mut() {
                block.push(
                    Statement::Emit(naga::Range::new_from_bounds(first, last)),
                    Span::UNDEFINED,
                );
            }
        }
    }

    fn push(&mut self, statement: Statement) {
        self.flush();
        if let Some(block) = self.blocks.last_mut() {
            block.push(statement, Span::UNDEFINED);
        }
    }

    fn open(&mut self) {
        self.flush();
        self.blocks.push(Block::new());
    }

    fn close(&mut self) -> Block {
        self.flush();
        self.blocks.pop().unwrap_or_default()
    }

    /// How many statements the open block holds.
    fn written(&self) -> usize {
        self.blocks.last().map_or(0, Block::len)
    }

    /// Closes the open block as a block of its own in the one around it.
    fn close_run(&mut self) {
        let run = self.close();
        if !run.is_empty() {
            self.push(Statement::Block(run));
        }
    }

    fn finish(mut self) -> naga::Function {
        self.flush();
        self.fun.body = self.blocks.drain(..).next().unwrap_or_default();
        self.fun
    }

    fn u32(&mut self, word: u32) -> Handle<E> {
        self.emit(E::Literal(Literal::U32(word)))
    }

    fn f32(&mut self, value: f32) -> Handle<E> {
        self.emit(E::Literal(Literal::F32(value)))
    }

    fn cast(&mut self, expr: Handle<E>, kind: ScalarKind) -> Handle<E> {
        self.emit(E::As {
            expr,
            kind,
            convert: None,
        })
    }

    /// A value converted, not reread: an integer to the nearest float, a float truncated and clamped.
    fn convert(&mut self, expr: Handle<E>, kind: ScalarKind) -> Handle<E> {
        self.emit(E::As {
            expr,
            kind,
            convert: Some(4),
        })
    }

    fn float(&mut self, bits: Handle<E>) -> Handle<E> {
        self.cast(bits, ScalarKind::Float)
    }

    fn bits(&mut self, value: Handle<E>) -> Handle<E> {
        self.cast(value, ScalarKind::Uint)
    }

    fn sint(&mut self, bits: Handle<E>) -> Handle<E> {
        self.cast(bits, ScalarKind::Sint)
    }

    fn bin(&mut self, op: Bo, left: Handle<E>, right: Handle<E>) -> Handle<E> {
        self.emit(E::Binary { op, left, right })
    }

    fn un(&mut self, op: Uo, expr: Handle<E>) -> Handle<E> {
        self.emit(E::Unary { op, expr })
    }

    fn math(&mut self, fun: Mf, args: &[Handle<E>]) -> Handle<E> {
        self.emit(E::Math {
            fun,
            arg: args[0],
            arg1: args.get(1).copied(),
            arg2: args.get(2).copied(),
            arg3: None,
        })
    }

    fn select(&mut self, condition: Handle<E>, accept: Handle<E>, reject: Handle<E>) -> Handle<E> {
        self.emit(E::Select {
            condition,
            accept,
            reject,
        })
    }

    fn index(&mut self, base: Handle<E>, index: u32) -> Handle<E> {
        self.emit(E::AccessIndex { base, index })
    }

    fn load(&mut self, pointer: Handle<E>) -> Handle<E> {
        self.emit(E::Load { pointer })
    }

    fn ne0(&mut self, value: Handle<E>) -> Handle<E> {
        let zero = self.u32(0);
        self.bin(Bo::NotEqual, value, zero)
    }

    /// A boolean as the dword 1 or 0.
    fn flag(&mut self, condition: Handle<E>) -> Handle<E> {
        let (one, zero) = (self.u32(1), self.u32(0));
        self.select(condition, one, zero)
    }

    fn not(&mut self, value: Handle<E>) -> Handle<E> {
        self.un(Uo::BitwiseNot, value)
    }

    fn lnot(&mut self, condition: Handle<E>) -> Handle<E> {
        self.un(Uo::LogicalNot, condition)
    }

    /// `left` shifted by the low five bits of `by`; `left` is read signed for an arithmetic shift.
    fn shift(&mut self, op: Bo, signed: bool, left: Handle<E>, by: Handle<E>) -> Handle<E> {
        let mask = self.u32(31);
        let by = self.bin(Bo::And, by, mask);
        if !signed {
            return self.bin(op, left, by);
        }
        let left = self.sint(left);
        let value = self.bin(op, left, by);
        self.bits(value)
    }

    /// The low `width` bits set, every bit for a width of 32 or more.
    fn low_bits(&mut self, width: Handle<E>) -> Handle<E> {
        let (one, limit, all) = (self.u32(1), self.u32(32), self.u32(u32::MAX));
        let shifted = self.shift(Bo::ShiftLeft, false, one, width);
        let mask = self.bin(Bo::Subtract, shifted, one);
        let whole = self.bin(Bo::GreaterEqual, width, limit);
        self.select(whole, all, mask)
    }

    /// `op` over the float readings of two patterns, as a pattern.
    fn fbin(&mut self, op: Bo, a: Handle<E>, b: Handle<E>) -> Handle<E> {
        let (a, b) = (self.float(a), self.float(b));
        let value = self.bin(op, a, b);
        self.bits(value)
    }

    /// `fun` over the float readings of `args`, as a pattern.
    fn fmath(&mut self, fun: Mf, args: &[Handle<E>]) -> Handle<E> {
        let args: Vec<_> = args.iter().map(|&arg| self.float(arg)).collect();
        let value = self.math(fun, &args);
        self.bits(value)
    }

    /// `a * b` over floats, with the legacy rule that a zero operand gives zero whatever the other is.
    fn mul_legacy(&mut self, a: Handle<E>, b: Handle<E>) -> Handle<E> {
        let zero = self.f32(0.0);
        let (za, zb) = (self.bin(Bo::Equal, a, zero), self.bin(Bo::Equal, b, zero));
        let either = self.bin(Bo::LogicalOr, za, zb);
        let product = self.bin(Bo::Multiply, a, b);
        self.select(either, zero, product)
    }

    /// `a * b + c` over float readings, the product rounded before the sum.
    fn mad(&mut self, legacy: bool, a: Handle<E>, b: Handle<E>, c: Handle<E>) -> Handle<E> {
        let (a, b, c) = (self.float(a), self.float(b), self.float(c));
        let product = match legacy {
            true => self.mul_legacy(a, b),
            false => self.bin(Bo::Multiply, a, b),
        };
        let value = self.bin(Bo::Add, product, c);
        self.bits(value)
    }

    /// The high dword of the 64-bit product of two dwords, by 16-bit limbs.
    fn mul_hi(&mut self, signed: bool, a: Handle<E>, b: Handle<E>) -> Handle<E> {
        let (sixteen, low) = (self.u32(16), self.u32(0xFFFF));
        let (al, ah) = (
            self.bin(Bo::And, a, low),
            self.bin(Bo::ShiftRight, a, sixteen),
        );
        let (bl, bh) = (
            self.bin(Bo::And, b, low),
            self.bin(Bo::ShiftRight, b, sixteen),
        );
        let (ll, lh) = (
            self.bin(Bo::Multiply, al, bl),
            self.bin(Bo::Multiply, al, bh),
        );
        let (hl, hh) = (
            self.bin(Bo::Multiply, ah, bl),
            self.bin(Bo::Multiply, ah, bh),
        );
        let carried = self.bin(Bo::ShiftRight, ll, sixteen);
        let (lh_low, hl_low) = (self.bin(Bo::And, lh, low), self.bin(Bo::And, hl, low));
        let cross = self.bin(Bo::Add, carried, lh_low);
        let cross = self.bin(Bo::Add, cross, hl_low);
        let carry = self.bin(Bo::ShiftRight, cross, sixteen);
        let (lh_high, hl_high) = (
            self.bin(Bo::ShiftRight, lh, sixteen),
            self.bin(Bo::ShiftRight, hl, sixteen),
        );
        let mut high = self.bin(Bo::Add, hh, lh_high);
        high = self.bin(Bo::Add, high, hl_high);
        high = self.bin(Bo::Add, high, carry);
        if signed {
            // Read unsigned, a negative operand counts the other one 2^32 times too many.
            let zero = self.u32(0);
            for (x, other) in [(a, b), (b, a)] {
                let top = self.u32(31);
                let sign = self.bin(Bo::ShiftRight, x, top);
                let negative = self.ne0(sign);
                let excess = self.select(negative, other, zero);
                high = self.bin(Bo::Subtract, high, excess);
            }
        }
        high
    }

    /// One of the ten bitwise functions the scalar group names, `None` for another word.
    fn logic(&mut self, kind: &str, a: Handle<E>, b: Handle<E>) -> Option<Handle<E>> {
        let (op, invert_a, invert_b, invert) = match kind {
            "and" => (Bo::And, false, false, false),
            "or" => (Bo::InclusiveOr, false, false, false),
            "xor" => (Bo::ExclusiveOr, false, false, false),
            "andn1" => (Bo::And, true, false, false),
            "andn2" => (Bo::And, false, true, false),
            "orn1" => (Bo::InclusiveOr, true, false, false),
            "orn2" => (Bo::InclusiveOr, false, true, false),
            "nand" => (Bo::And, false, false, true),
            "nor" => (Bo::InclusiveOr, false, false, true),
            "xnor" => (Bo::ExclusiveOr, false, false, true),
            _ => return None,
        };
        let a = if invert_a { self.not(a) } else { a };
        let b = if invert_b { self.not(b) } else { b };
        let value = self.bin(op, a, b);
        Some(if invert { self.not(value) } else { value })
    }

    /// An integer comparison by its word in a mnemonic.
    fn icmp(&mut self, rel: &str, signed: bool, a: Handle<E>, b: Handle<E>) -> Option<Handle<E>> {
        let op = match rel {
            "eq" => Bo::Equal,
            "lg" | "ne" => Bo::NotEqual,
            "gt" => Bo::Greater,
            "ge" => Bo::GreaterEqual,
            "lt" => Bo::Less,
            "le" => Bo::LessEqual,
            "f" => return Some(self.emit(E::Literal(Literal::Bool(false)))),
            "tru" | "t" => return Some(self.emit(E::Literal(Literal::Bool(true)))),
            _ => return None,
        };
        let (a, b) = match signed {
            true => (self.sint(a), self.sint(b)),
            false => (a, b),
        };
        Some(self.bin(op, a, b))
    }

    fn is_nan(&mut self, argument: Handle<E>) -> Handle<E> {
        self.emit(E::Relational {
            fun: RelationalFunction::IsNan,
            argument,
        })
    }

    /// A float comparison by its word in a mnemonic, over two patterns; the `n` forms are true of a NaN.
    fn fcmp(&mut self, rel: &str, a: Handle<E>, b: Handle<E>) -> Option<Handle<E>> {
        let (a, b) = (self.float(a), self.float(b));
        let (base, negate) = match rel.strip_prefix('n') {
            Some(base @ ("ge" | "lg" | "gt" | "le" | "eq" | "lt")) => (base, true),
            _ => (rel, false),
        };
        let value = match base {
            "lt" => self.bin(Bo::Less, a, b),
            "eq" => self.bin(Bo::Equal, a, b),
            "le" => self.bin(Bo::LessEqual, a, b),
            "gt" => self.bin(Bo::Greater, a, b),
            "ge" => self.bin(Bo::GreaterEqual, a, b),
            "lg" => {
                let (less, greater) = (self.bin(Bo::Less, a, b), self.bin(Bo::Greater, a, b));
                self.bin(Bo::LogicalOr, less, greater)
            }
            "o" | "u" => {
                let (na, nb) = (self.is_nan(a), self.is_nan(b));
                let unordered = self.bin(Bo::LogicalOr, na, nb);
                match base {
                    "u" => unordered,
                    _ => self.lnot(unordered),
                }
            }
            "f" => self.emit(E::Literal(Literal::Bool(false))),
            "tru" | "t" => self.emit(E::Literal(Literal::Bool(true))),
            _ => return None,
        };
        Some(if negate { self.lnot(value) } else { value })
    }

    /// Whether the float pattern `x` is of a class `mask` selects: bit 0 a signalling NaN, 1 a quiet
    /// one, 2 and 9 the infinities, 3 and 8 the normals, 4 and 7 the denormals, 5 and 6 the zeros.
    fn class(&mut self, x: Handle<E>, mask: Handle<E>) -> Handle<E> {
        let (s31, s23, s22) = (self.u32(31), self.u32(23), self.u32(22));
        let (one, nil, byte) = (self.u32(1), self.u32(0), self.u32(0xFF));
        let sign = self.bin(Bo::ShiftRight, x, s31);
        let negative = self.ne0(sign);
        let shifted = self.bin(Bo::ShiftRight, x, s23);
        let exponent = self.bin(Bo::And, shifted, byte);
        let low = self.u32(0x7F_FFFF);
        let mantissa = self.bin(Bo::And, x, low);
        // The class by sign, the negative one first.
        let mut classes = [nil; 4];
        for (class, pair) in classes.iter_mut().zip([(2, 9), (3, 8), (4, 7), (5, 6)]) {
            let (n, p) = (self.u32(pair.0), self.u32(pair.1));
            *class = self.select(negative, n, p);
        }
        let [infinity, normal, denormal, zero] = classes;
        let quiet = self.bin(Bo::ShiftRight, mantissa, s22);
        let nan = self.bin(Bo::And, quiet, one);
        let no_mantissa = self.bin(Bo::Equal, mantissa, nil);
        let top = self.bin(Bo::Equal, exponent, byte);
        let bottom = self.bin(Bo::Equal, exponent, nil);
        let special = self.select(no_mantissa, infinity, nan);
        let small = self.select(no_mantissa, zero, denormal);
        let finite = self.select(bottom, small, normal);
        let class = self.select(top, special, finite);
        let bit = self.bin(Bo::ShiftRight, mask, class);
        let bit = self.bin(Bo::And, bit, one);
        self.ne0(bit)
    }

    /// One of the four values the hardware gives for a direction's face of a cube: the face's
    /// number, its two coordinates before the division, and twice the major axis.
    fn cube(&mut self, name: &str, x: Handle<E>, y: Handle<E>, z: Handle<E>) -> Handle<E> {
        let (x, y, z) = (self.float(x), self.float(y), self.float(z));
        let (ax, ay, az) = (
            self.math(Mf::Abs, &[x]),
            self.math(Mf::Abs, &[y]),
            self.math(Mf::Abs, &[z]),
        );
        let (over_x, over_y) = (
            self.bin(Bo::GreaterEqual, az, ax),
            self.bin(Bo::GreaterEqual, az, ay),
        );
        let z_major = self.bin(Bo::LogicalAnd, over_x, over_y);
        let y_major = self.bin(Bo::GreaterEqual, ay, ax);
        let zero = self.f32(0.0);
        let (xn, yn, zn) = (
            self.bin(Bo::Less, x, zero),
            self.bin(Bo::Less, y, zero),
            self.bin(Bo::Less, z, zero),
        );
        let (on_z, on_y, on_x) = match name {
            "v_cubeid_f32" => {
                let faces = [5.0, 4.0, 3.0, 2.0, 1.0, 0.0].map(|face| self.f32(face));
                (
                    self.select(zn, faces[0], faces[1]),
                    self.select(yn, faces[2], faces[3]),
                    self.select(xn, faces[4], faces[5]),
                )
            }
            "v_cubesc_f32" => {
                let (nx, nz) = (self.un(Uo::Negate, x), self.un(Uo::Negate, z));
                (self.select(zn, nx, x), x, self.select(xn, z, nz))
            }
            "v_cubetc_f32" => {
                let (ny, nz) = (self.un(Uo::Negate, y), self.un(Uo::Negate, z));
                (ny, self.select(yn, nz, z), ny)
            }
            _ => {
                let two = self.f32(2.0);
                (
                    self.bin(Bo::Multiply, z, two),
                    self.bin(Bo::Multiply, y, two),
                    self.bin(Bo::Multiply, x, two),
                )
            }
        };
        let rest = self.select(y_major, on_y, on_x);
        let value = self.select(z_major, on_z, rest);
        self.bits(value)
    }

    /// `v_perm_b32`: each byte of the result is the byte of the two sources its selector byte
    /// numbers (the second source's four first), a source byte's sign spread for 8 to 11, zero
    /// for 12 and every bit for more.
    fn perm(&mut self, high: Handle<E>, low: Handle<E>, selector: Handle<E>) -> Handle<E> {
        let (byte, three, one, zero) = (self.u32(0xFF), self.u32(3), self.u32(1), self.u32(0));
        let (four, eight, twelve, thirteen) =
            (self.u32(4), self.u32(8), self.u32(12), self.u32(13));
        let mut out = zero;
        for i in 0..4u32 {
            let at = self.u32(8 * i);
            let moved = self.bin(Bo::ShiftRight, selector, at);
            let sel = self.bin(Bo::And, moved, byte);
            let within = self.bin(Bo::And, sel, three);
            let by = self.bin(Bo::Multiply, within, eight);
            let (from_low, from_high) = (
                self.bin(Bo::ShiftRight, low, by),
                self.bin(Bo::ShiftRight, high, by),
            );
            let in_low = self.bin(Bo::Less, sel, four);
            let chosen = self.select(in_low, from_low, from_high);
            let data = self.bin(Bo::And, chosen, byte);
            // Selectors 8 to 11 spread the top bit of bytes 1, 3, 5 and 7.
            let two = self.u32(2);
            let (low_pair, upper) = (
                self.bin(Bo::Less, within, two),
                self.bin(Bo::And, within, one),
            );
            let word = self.select(low_pair, low, high);
            let (bit15, bit31) = (self.u32(15), self.u32(31));
            let odd = self.bin(Bo::NotEqual, upper, zero);
            let place = self.select(odd, bit31, bit15);
            let top = self.bin(Bo::ShiftRight, word, place);
            let sign = self.bin(Bo::And, top, one);
            let spread = self.bin(Bo::Multiply, sign, byte);
            let (is_sign, is_zero, is_ones) = (
                self.bin(Bo::GreaterEqual, sel, eight),
                self.bin(Bo::Equal, sel, twelve),
                self.bin(Bo::GreaterEqual, sel, thirteen),
            );
            let mut value = self.select(is_sign, spread, data);
            value = self.select(is_zero, zero, value);
            value = self.select(is_ones, byte, value);
            let placed = self.bin(Bo::ShiftLeft, value, at);
            out = self.bin(Bo::InclusiveOr, out, placed);
        }
        out
    }

    /// A sum or a difference of two dwords and a carry or borrow in, with the one out.
    fn carried(
        &mut self,
        name: &str,
        a: Handle<E>,
        b: Handle<E>,
        carry: Option<Handle<E>>,
    ) -> Option<(Handle<E>, Handle<E>)> {
        let (x, y, subtract) = match name {
            "v_add_co_u32" | "v_addc_co_u32" => (a, b, false),
            "v_sub_co_u32" | "v_subb_co_u32" => (a, b, true),
            "v_subrev_co_u32" | "v_subbrev_co_u32" => (b, a, true),
            _ => return None,
        };
        let carry = match carry {
            Some(set) => self.flag(set),
            None => self.u32(0),
        };
        Some(match subtract {
            false => {
                let partial = self.bin(Bo::Add, x, y);
                let sum = self.bin(Bo::Add, partial, carry);
                let (first, second) = (
                    self.bin(Bo::Less, partial, x),
                    self.bin(Bo::Less, sum, partial),
                );
                (sum, self.bin(Bo::LogicalOr, first, second))
            }
            true => {
                let partial = self.bin(Bo::Subtract, x, y);
                let difference = self.bin(Bo::Subtract, partial, carry);
                let (first, second) = (
                    self.bin(Bo::Greater, y, x),
                    self.bin(Bo::Greater, carry, partial),
                );
                (difference, self.bin(Bo::LogicalOr, first, second))
            }
        })
    }

    /// The pattern an arithmetic instruction writes from its sources' patterns, `None` for one not lowered.
    fn alu(&mut self, name: &str, s: &[Handle<E>]) -> Option<Handle<E>> {
        Some(match (name, s) {
            ("v_mov_b32", &[a]) => a,
            ("v_add_f32", &[a, b]) => self.fbin(Bo::Add, a, b),
            ("v_sub_f32", &[a, b]) => self.fbin(Bo::Subtract, a, b),
            ("v_subrev_f32", &[a, b]) => self.fbin(Bo::Subtract, b, a),
            ("v_mul_f32", &[a, b]) => self.fbin(Bo::Multiply, a, b),
            ("v_mul_legacy_f32", &[a, b]) => {
                let (a, b) = (self.float(a), self.float(b));
                let value = self.mul_legacy(a, b);
                self.bits(value)
            }
            // ponytail: naga's min and max leave a NaN operand's result open; the hardware returns the other operand.
            ("v_min_f32", &[a, b]) => self.fmath(Mf::Min, &[a, b]),
            ("v_max_f32", &[a, b]) => self.fmath(Mf::Max, &[a, b]),
            ("v_min3_f32", &[a, b, c]) => {
                let first = self.fmath(Mf::Min, &[a, b]);
                self.fmath(Mf::Min, &[first, c])
            }
            ("v_max3_f32", &[a, b, c]) => {
                let first = self.fmath(Mf::Max, &[a, b]);
                self.fmath(Mf::Max, &[first, c])
            }
            ("v_med3_f32", &[a, b, c]) => {
                let (low, high) = (self.fmath(Mf::Min, &[a, b]), self.fmath(Mf::Max, &[a, b]));
                let capped = self.fmath(Mf::Min, &[high, c]);
                self.fmath(Mf::Max, &[low, capped])
            }
            ("v_and_b32", &[a, b]) => self.bin(Bo::And, a, b),
            ("v_or_b32", &[a, b]) => self.bin(Bo::InclusiveOr, a, b),
            ("v_xor_b32", &[a, b]) => self.bin(Bo::ExclusiveOr, a, b),
            ("v_xnor_b32", &[a, b]) => {
                let value = self.bin(Bo::ExclusiveOr, a, b);
                self.not(value)
            }
            ("v_add_u32", &[a, b]) => self.bin(Bo::Add, a, b),
            ("v_sub_u32", &[a, b]) => self.bin(Bo::Subtract, a, b),
            ("v_subrev_u32", &[a, b]) => self.bin(Bo::Subtract, b, a),
            ("v_mul_lo_u32" | "v_mul_lo_i32", &[a, b]) => self.bin(Bo::Multiply, a, b),
            // A 16-bit sum, difference or product is the low half of the 32-bit one.
            ("v_add_u16" | "v_sub_u16" | "v_subrev_u16" | "v_mul_lo_u16", &[a, b]) => {
                let whole = match name {
                    "v_add_u16" => self.bin(Bo::Add, a, b),
                    "v_sub_u16" => self.bin(Bo::Subtract, a, b),
                    "v_subrev_u16" => self.bin(Bo::Subtract, b, a),
                    _ => self.bin(Bo::Multiply, a, b),
                };
                let low = self.u32(0xFFFF);
                self.bin(Bo::And, whole, low)
            }
            ("v_mad_u32_u24", &[a, b, c]) => {
                let low = self.u32(0xFF_FFFF);
                let (a, b) = (self.bin(Bo::And, a, low), self.bin(Bo::And, b, low));
                let product = self.bin(Bo::Multiply, a, b);
                self.bin(Bo::Add, product, c)
            }
            ("v_mad_i32_i24", &[a, b, c]) => {
                let eight = self.u32(8);
                let mut wide = [a, b];
                for x in &mut wide {
                    let up = self.bin(Bo::ShiftLeft, *x, eight);
                    *x = self.shift(Bo::ShiftRight, true, up, eight);
                }
                let product = self.bin(Bo::Multiply, wide[0], wide[1]);
                self.bin(Bo::Add, product, c)
            }
            ("v_mul_hi_u32", &[a, b]) => self.mul_hi(false, a, b),
            ("v_mul_hi_i32", &[a, b]) => self.mul_hi(true, a, b),
            ("v_mul_u32_u24", &[a, b]) => {
                let low = self.u32(0xFF_FFFF);
                let (a, b) = (self.bin(Bo::And, a, low), self.bin(Bo::And, b, low));
                self.bin(Bo::Multiply, a, b)
            }
            ("v_mul_i32_i24", &[a, b]) => {
                let eight = self.u32(8);
                let mut wide = [a, b];
                for x in &mut wide {
                    let up = self.bin(Bo::ShiftLeft, *x, eight);
                    *x = self.shift(Bo::ShiftRight, true, up, eight);
                }
                self.bin(Bo::Multiply, wide[0], wide[1])
            }
            ("v_min_u32", &[a, b]) => self.math(Mf::Min, &[a, b]),
            ("v_max_u32", &[a, b]) => self.math(Mf::Max, &[a, b]),
            ("v_min_i32" | "v_max_i32", &[a, b]) => {
                let (a, b) = (self.sint(a), self.sint(b));
                let fun = if name == "v_min_i32" {
                    Mf::Min
                } else {
                    Mf::Max
                };
                let value = self.math(fun, &[a, b]);
                self.bits(value)
            }
            ("v_min3_u32", &[a, b, c]) => {
                let first = self.math(Mf::Min, &[a, b]);
                self.math(Mf::Min, &[first, c])
            }
            ("v_max3_u32", &[a, b, c]) => {
                let first = self.math(Mf::Max, &[a, b]);
                self.math(Mf::Max, &[first, c])
            }
            ("v_med3_u32", &[a, b, c]) => {
                let (low, high) = (self.math(Mf::Min, &[a, b]), self.math(Mf::Max, &[a, b]));
                let capped = self.math(Mf::Min, &[high, c]);
                self.math(Mf::Max, &[low, capped])
            }
            ("v_min3_i32" | "v_max3_i32" | "v_med3_i32", &[a, b, c]) => {
                let (a, b, c) = (self.sint(a), self.sint(b), self.sint(c));
                let value = match name {
                    "v_min3_i32" => {
                        let first = self.math(Mf::Min, &[a, b]);
                        self.math(Mf::Min, &[first, c])
                    }
                    "v_max3_i32" => {
                        let first = self.math(Mf::Max, &[a, b]);
                        self.math(Mf::Max, &[first, c])
                    }
                    _ => {
                        let (low, high) =
                            (self.math(Mf::Min, &[a, b]), self.math(Mf::Max, &[a, b]));
                        let capped = self.math(Mf::Min, &[high, c]);
                        self.math(Mf::Max, &[low, capped])
                    }
                };
                self.bits(value)
            }
            ("v_lshlrev_b32", &[a, b]) => self.shift(Bo::ShiftLeft, false, b, a),
            ("v_lshrrev_b32", &[a, b]) => self.shift(Bo::ShiftRight, false, b, a),
            ("v_ashrrev_i32", &[a, b]) => self.shift(Bo::ShiftRight, true, b, a),
            ("v_lshrrev_b16" | "v_lshlrev_b16", &[a, b]) => {
                // A 16-bit value in the low half of its register, moved by the low four bits of a count.
                let (low, most) = (self.u32(0xFFFF), self.u32(15));
                let (value, by) = (self.bin(Bo::And, b, low), self.bin(Bo::And, a, most));
                match name {
                    "v_lshrrev_b16" => self.bin(Bo::ShiftRight, value, by),
                    _ => {
                        let moved = self.bin(Bo::ShiftLeft, value, by);
                        self.bin(Bo::And, moved, low)
                    }
                }
            }
            ("v_not_b32", &[a]) => self.not(a),
            ("v_bfrev_b32", &[a]) => self.math(Mf::ReverseBits, &[a]),
            ("v_ffbl_b32", &[a]) => self.math(Mf::FirstTrailingBit, &[a]),
            ("v_bfe_u32", &[a, offset, width]) => {
                let low = self.u32(31);
                let width = self.bin(Bo::And, width, low);
                let mask = self.low_bits(width);
                let shifted = self.shift(Bo::ShiftRight, false, a, offset);
                self.bin(Bo::And, shifted, mask)
            }
            ("v_bfe_i32", &[a, offset, width]) => {
                // The field moved to the top of the dword and back down with its sign; no bits, no value.
                let (low, word, zero) = (self.u32(31), self.u32(32), self.u32(0));
                let width = self.bin(Bo::And, width, low);
                let spare = self.bin(Bo::Subtract, word, width);
                let field = self.shift(Bo::ShiftRight, false, a, offset);
                let top = self.shift(Bo::ShiftLeft, false, field, spare);
                let signed = self.shift(Bo::ShiftRight, true, top, spare);
                let none = self.bin(Bo::Equal, width, zero);
                self.select(none, zero, signed)
            }
            ("v_bfi_b32", &[mask, a, b]) => {
                let (kept, inverse) = (self.bin(Bo::And, mask, a), self.not(mask));
                let rest = self.bin(Bo::And, inverse, b);
                self.bin(Bo::InclusiveOr, kept, rest)
            }
            ("v_lshl_add_u32", &[a, b, c]) => {
                let shifted = self.shift(Bo::ShiftLeft, false, a, b);
                self.bin(Bo::Add, shifted, c)
            }
            ("v_lshl_or_b32", &[a, b, c]) => {
                let shifted = self.shift(Bo::ShiftLeft, false, a, b);
                self.bin(Bo::InclusiveOr, shifted, c)
            }
            ("v_add3_u32", &[a, b, c]) => {
                let sum = self.bin(Bo::Add, a, b);
                self.bin(Bo::Add, sum, c)
            }
            ("v_and_or_b32", &[a, b, c]) => {
                let both = self.bin(Bo::And, a, b);
                self.bin(Bo::InclusiveOr, both, c)
            }
            ("v_or3_b32", &[a, b, c]) => {
                let either = self.bin(Bo::InclusiveOr, a, b);
                self.bin(Bo::InclusiveOr, either, c)
            }
            ("v_xor3_b32", &[a, b, c]) => {
                let either = self.bin(Bo::ExclusiveOr, a, b);
                self.bin(Bo::ExclusiveOr, either, c)
            }
            ("v_xad_u32", &[a, b, c]) => {
                let either = self.bin(Bo::ExclusiveOr, a, b);
                self.bin(Bo::Add, either, c)
            }
            ("v_fract_f32", &[a]) => {
                // The hardware returns the largest float below one where the subtraction rounds to one.
                let (x, below_one) = (self.float(a), self.f32(f32::from_bits(0x3F7F_FFFF)));
                let fraction = self.math(Mf::Fract, &[x]);
                let value = self.math(Mf::Min, &[fraction, below_one]);
                self.bits(value)
            }
            ("v_trunc_f32", &[a]) => self.fmath(Mf::Trunc, &[a]),
            ("v_ceil_f32", &[a]) => self.fmath(Mf::Ceil, &[a]),
            ("v_floor_f32", &[a]) => self.fmath(Mf::Floor, &[a]),
            ("v_rndne_f32", &[a]) => self.fmath(Mf::Round, &[a]),
            ("v_sqrt_f32", &[a]) => self.fmath(Mf::Sqrt, &[a]),
            ("v_rsq_f32", &[a]) => self.fmath(Mf::InverseSqrt, &[a]),
            ("v_exp_f32", &[a]) => self.fmath(Mf::Exp2, &[a]),
            ("v_log_f32", &[a]) => self.fmath(Mf::Log2, &[a]),
            ("v_sin_f32" | "v_cos_f32", &[a]) => {
                // The operand is in turns: reduced to half a turn either way, then scaled.
                let x = self.float(a);
                let whole = self.math(Mf::Round, &[x]);
                let part = self.bin(Bo::Subtract, x, whole);
                let tau = self.f32(TAU);
                let angle = self.bin(Bo::Multiply, part, tau);
                let fun = if name == "v_sin_f32" {
                    Mf::Sin
                } else {
                    Mf::Cos
                };
                let value = self.math(fun, &[angle]);
                self.bits(value)
            }
            ("v_rcp_f32" | "v_rcp_iflag_f32", &[a]) => {
                let one = self.u32(1f32.to_bits());
                self.fbin(Bo::Divide, one, a)
            }
            ("v_cvt_f32_i32", &[a]) => {
                let a = self.sint(a);
                let value = self.convert(a, ScalarKind::Float);
                self.bits(value)
            }
            ("v_cvt_f32_u32", &[a]) => {
                let value = self.convert(a, ScalarKind::Float);
                self.bits(value)
            }
            (
                "v_cvt_f32_ubyte0" | "v_cvt_f32_ubyte1" | "v_cvt_f32_ubyte2" | "v_cvt_f32_ubyte3",
                &[a],
            ) => {
                let byte = u32::from(name.as_bytes()[name.len() - 1] - b'0');
                let (by, low) = (self.u32(8 * byte), self.u32(0xFF));
                let shifted = self.bin(Bo::ShiftRight, a, by);
                let byte = self.bin(Bo::And, shifted, low);
                let value = self.convert(byte, ScalarKind::Float);
                self.bits(value)
            }
            // naga's conversion truncates, clamps to the integer's range and reads a NaN as zero, as the hardware does.
            ("v_cvt_u32_f32", &[a]) => {
                let x = self.float(a);
                self.convert(x, ScalarKind::Uint)
            }
            ("v_cvt_i32_f32" | "v_cvt_flr_i32_f32" | "v_cvt_rpi_i32_f32", &[a]) => {
                let mut x = self.float(a);
                if name == "v_cvt_rpi_i32_f32" {
                    let half = self.f32(0.5);
                    x = self.bin(Bo::Add, x, half);
                }
                if name != "v_cvt_i32_f32" {
                    x = self.math(Mf::Floor, &[x]);
                }
                let value = self.convert(x, ScalarKind::Sint);
                self.bits(value)
            }
            ("v_mad_f32", &[a, b, c]) => self.mad(false, a, b, c),
            ("v_mad_legacy_f32", &[a, b, c]) => self.mad(true, a, b, c),
            ("v_fma_f32", &[a, b, c]) => self.fmath(Mf::Fma, &[a, b, c]),
            ("v_add_lshl_u32", &[a, b, c]) => {
                let sum = self.bin(Bo::Add, a, b);
                self.shift(Bo::ShiftLeft, false, sum, c)
            }
            ("v_bfm_b32", &[a, b]) => {
                let low = self.u32(31);
                let width = self.bin(Bo::And, a, low);
                let mask = self.low_bits(width);
                self.shift(Bo::ShiftLeft, false, mask, b)
            }
            ("v_alignbit_b32", &[a, b, c]) => {
                // The 64 bits of the first source over the second, moved right, low dword kept.
                let (low, word, zero) = (self.u32(31), self.u32(32), self.u32(0));
                let by = self.bin(Bo::And, c, low);
                let spare = self.bin(Bo::Subtract, word, by);
                let (kept, carried) = (
                    self.bin(Bo::ShiftRight, b, by),
                    self.shift(Bo::ShiftLeft, false, a, spare),
                );
                let joined = self.bin(Bo::InclusiveOr, kept, carried);
                let none = self.bin(Bo::Equal, by, zero);
                self.select(none, b, joined)
            }
            ("v_bcnt_u32_b32", &[a, b]) => {
                let count = self.math(Mf::CountOneBits, &[a]);
                self.bin(Bo::Add, count, b)
            }
            ("v_ffbh_u32", &[a]) => {
                // The first set bit counted from the top, and every bit set where there is none.
                let (zero, none) = (self.u32(0), self.u32(u32::MAX));
                let leading = self.math(Mf::CountLeadingZeros, &[a]);
                let empty = self.bin(Bo::Equal, a, zero);
                self.select(empty, none, leading)
            }
            ("v_ffbh_i32", &[a]) => {
                // The first bit unlike the sign counted from the top, and every bit set where none is.
                let (most, none, zero) = (self.u32(31), self.u32(u32::MAX), self.u32(0));
                let sign = self.shift(Bo::ShiftRight, true, a, most);
                let unlike = self.bin(Bo::ExclusiveOr, a, sign);
                let leading = self.math(Mf::CountLeadingZeros, &[unlike]);
                let empty = self.bin(Bo::Equal, unlike, zero);
                self.select(empty, none, leading)
            }
            ("v_alignbyte_b32", &[a, b, c]) => {
                // [`v_alignbit_b32`] by whole bytes, three at most.
                let (three, eight, word, zero) =
                    (self.u32(3), self.u32(8), self.u32(32), self.u32(0));
                let bytes = self.bin(Bo::And, c, three);
                let by = self.bin(Bo::Multiply, bytes, eight);
                let spare = self.bin(Bo::Subtract, word, by);
                let (kept, carried) = (
                    self.bin(Bo::ShiftRight, b, by),
                    self.shift(Bo::ShiftLeft, false, a, spare),
                );
                let joined = self.bin(Bo::InclusiveOr, kept, carried);
                let none = self.bin(Bo::Equal, by, zero);
                self.select(none, b, joined)
            }
            ("v_cvt_f32_f16", &[a]) => self.float_of_half(a),
            ("v_cvt_pk_u8_f32", &[a, b, c]) => {
                // The float as a byte, into the byte of the third source the second one numbers.
                let (x, least, most) = (self.float(a), self.f32(0.0), self.f32(255.0));
                let clamped = self.math(Mf::Clamp, &[x, least, most]);
                let byte = self.convert(clamped, ScalarKind::Uint);
                let (three, eight, ones) = (self.u32(3), self.u32(8), self.u32(0xFF));
                let which = self.bin(Bo::And, b, three);
                let by = self.bin(Bo::Multiply, which, eight);
                let (mask, placed) = (
                    self.bin(Bo::ShiftLeft, ones, by),
                    self.bin(Bo::ShiftLeft, byte, by),
                );
                let hole = self.not(mask);
                let kept = self.bin(Bo::And, c, hole);
                self.bin(Bo::InclusiveOr, kept, placed)
            }
            ("v_cubeid_f32" | "v_cubesc_f32" | "v_cubetc_f32" | "v_cubema_f32", &[x, y, z]) => {
                self.cube(name, x, y, z)
            }
            ("v_perm_b32", &[a, b, c]) => self.perm(a, b, c),
            _ => return None,
        })
    }
}

fn refused(pc: u32, inst: &Inst) -> NotTranslated {
    NotTranslated::Opcode {
        pc,
        name: inst.op_word(),
    }
}

fn form(pc: u32, inst: &Inst, what: &'static str) -> NotTranslated {
    NotTranslated::Form {
        pc,
        name: inst.op_word(),
        what,
    }
}

/// Whether a 16-bit mnemonic is a function the hardware rounds to the nearest whatever its
/// float mode says, where a sum or a product follows the mode.
fn half_function(name: &str) -> bool {
    let kind = name.trim_start_matches("v_pk_").trim_start_matches("v_");
    let kind = kind.split('_').next().unwrap_or(kind);
    matches!(kind, "rcp" | "rsq" | "sqrt" | "exp" | "log" | "sin" | "cos")
}

/// Whether an instruction writes the float mode register.
fn writes_float_mode(inst: &Inst) -> bool {
    matches!(inst, Inst::Sopk { imm, imm32: Some(_), .. } if *imm & 0x3F == 1)
        && inst.name() == "s_setreg_imm32_b32"
}

/// Whether a mnemonic computes over 16-bit floats as its 32-bit form does over whole ones.
fn is_half(name: &str) -> bool {
    name.ends_with("_f16") && !name.starts_with("v_cvt_")
}

/// Refuses a part selection that negates or takes the magnitude of a 16-bit float: the modifier
/// is written for a whole register.
fn half_parts(pc: u32, inst: &Inst, name: &str, word: u32) -> Done {
    let modified = (word >> 20) & 3 != 0 || (word >> 28) & 3 != 0;
    match modified && name.ends_with("_f16") {
        true => Err(form(pc, inst, "a float modifier on a 16-bit part")),
        false => Ok(()),
    }
}

/// The pattern, in the low half of a dword, of the 16-bit float an inline float constant is.
fn half_word(word: u32) -> u32 {
    u32::from(half_bits(f32::from_bits(word)))
}

/// A float as the nearest 16-bit float's pattern, a tie to the even one.
pub(super) fn half_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 128 {
        // An infinity, or a NaN kept quiet.
        return sign | 0x7C00 | if mantissa == 0 { 0 } else { 0x200 };
    }
    if exponent > 15 {
        return sign | 0x7C00;
    }
    if exponent < -25 {
        return sign;
    }
    // A normal half keeps ten bits of the mantissa; a denormal one fewer for each step below.
    let (kept, shift) = match exponent >= -14 {
        true => ((((exponent + 15) as u32) << 10) | (mantissa >> 13), 13),
        false => {
            let shift = (-1 - exponent) as u32;
            ((mantissa | 0x80_0000) >> shift, shift)
        }
    };
    let whole = match exponent >= -14 {
        true => mantissa,
        false => mantissa | 0x80_0000,
    };
    let (dropped, tie) = (whole & ((1 << shift) - 1), 1 << (shift - 1));
    // A carry out of the mantissa rolls into the exponent, up to infinity.
    let up = dropped > tie || (dropped == tie && kept & 1 != 0);
    sign | (kept + u32::from(up)) as u16
}

/// What a fold over a wave's lanes computes, by the mnemonic of its steps: the operation, how
/// its operands are read, and whether they are 16-bit floats.
fn fold_of(name: &str) -> Option<(naga::SubgroupOperation, ScalarKind, bool)> {
    use naga::SubgroupOperation as Op;
    Some(match name {
        "v_or_b32" => (Op::Or, ScalarKind::Uint, false),
        "v_xor_b32" => (Op::Xor, ScalarKind::Uint, false),
        "v_and_b32" => (Op::And, ScalarKind::Uint, false),
        "v_add_u32" => (Op::Add, ScalarKind::Uint, false),
        "v_min_u32" => (Op::Min, ScalarKind::Uint, false),
        "v_max_u32" => (Op::Max, ScalarKind::Uint, false),
        "v_min_i32" => (Op::Min, ScalarKind::Sint, false),
        "v_max_i32" => (Op::Max, ScalarKind::Sint, false),
        "v_add_f32" => (Op::Add, ScalarKind::Float, false),
        "v_min_f32" => (Op::Min, ScalarKind::Float, false),
        "v_max_f32" => (Op::Max, ScalarKind::Float, false),
        "v_add_f16" => (Op::Add, ScalarKind::Float, true),
        _ => return None,
    })
}

/// The lane selectors of the step a fold over the wave opens with: each lane reads its pair's other.
const PAIR_SWAP: u32 = 0o6745_2301;
/// The control of the step a running sum opens with: each lane reads the one before it in its row.
const ROW_SHR_1: u32 = 0x111;

/// What a register holds of a fold over the wave, within a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waved {
    /// The fold's result, the same in every lane a fold of the whole wave reaches.
    Folded,
    /// Another lane's partial result of the fold in this register, which only a step of the fold reads.
    Foreign(Reg),
}

fn insert(module: &mut naga::Module, inner: TypeInner) -> Handle<naga::Type> {
    module
        .types
        .insert(naga::Type { name: None, inner }, Span::UNDEFINED)
}

fn vector(module: &mut naga::Module, size: VectorSize, scalar: Scalar) -> Handle<naga::Type> {
    insert(module, TypeInner::Vector { size, scalar })
}

fn size_of(count: usize) -> VectorSize {
    match count {
        2 => VectorSize::Bi,
        3 => VectorSize::Tri,
        _ => VectorSize::Quad,
    }
}

/// What a mesh program run as a vertex program keeps between the two runs of its body: the thread
/// it stands for, its group, and what that thread exported.
#[derive(Debug, Clone, Copy)]
struct Mesh {
    thread: Handle<naga::GlobalVariable>,
    group: Handle<naga::GlobalVariable>,
    prim: Handle<naga::GlobalVariable>,
    prim_set: Handle<naga::GlobalVariable>,
    pos_set: Handle<naga::GlobalVariable>,
}

/// The lanes of a quad a permutation reads, one selector a lane.
type Quad = [u8; 4];

/// A register that holds another's value as a neighbouring lane computed it.
#[derive(Debug, Clone, Copy)]
struct Permuted {
    source: u8,
    /// How many times the source had been written in this block when it was read.
    version: u32,
    quad: Quad,
}

/// What lowering one block left: its statements, what it reads before writing, and what it writes.
struct Lowering {
    block: Block,
    used: BTreeMap<Reg, u32>,
    defined: BTreeSet<Reg>,
    maybe: BTreeSet<Reg>,
}

struct Cx<'a> {
    program: &'a Program,
    module: naga::Module,
    b: Builder,
    fragment: bool,
    /// A program of the vertex stage itself, which no mesh, geometry or compute program is.
    vertex: bool,
    /// Whether any instruction changes the lane mask: then every vector write is kept to an active lane.
    masked: bool,
    locals: HashMap<Reg, Handle<E>>,
    /// Whether the program's statements are spread over several functions: then a register is
    /// a private variable of the module, which every function of it reads and writes.
    spread: bool,
    registers: HashMap<Reg, Handle<naga::GlobalVariable>>,
    /// The block being lowered: its index, where each scalar register's value came from, and its sets.
    index: u32,
    state: State,
    used: BTreeMap<Reg, u32>,
    defined: BTreeSet<Reg>,
    /// The registers the block writes on some of its runs only: an array's, by a computed index.
    maybe: BTreeSet<Reg>,
    versions: HashMap<u8, u32>,
    permuted: HashMap<u8, Permuted>,
    /// The registers that held a neighbouring lane's value where a block ended, with the block.
    rebuilt: Vec<(u32, u8)>,
    /// Whether a wave is the subgroup the device runs the invocation in: a pixel or a compute
    /// program, whose folds, votes and lane reads are subgroup operations. Elsewhere one
    /// invocation is the whole wave.
    subgroups: bool,
    /// Whether the module holds a subgroup operation.
    wave_ops: bool,
    /// The registers that hold a fold over the wave in this block, and the carry a 64-bit sum's
    /// low half hands its high half.
    waved: HashMap<Reg, Waved>,
    carry: Option<Handle<E>>,
    /// Whether 16-bit results round toward zero, where the program writes its float mode.
    half_mode: Option<Handle<naga::GlobalVariable>>,
    /// This lane's number in its subgroup, where the program reads it.
    lane: Option<Handle<naga::GlobalVariable>>,
    /// The wave of a compute group kept in the memory the group shares.
    shared_wave: wave::Shared,
    mesh: Option<Mesh>,
    half_slots: &'a BTreeSet<u8>,
    /// A pixel input by its parameter slot: the private variable, and whether anything interpolates it.
    attributes: BTreeMap<u8, (Handle<naga::GlobalVariable>, bool)>,
    /// A pixel input read as 16-bit halves, by its parameter slot and whether the halves are the high ones.
    halves: BTreeMap<(u8, bool), Handle<naga::GlobalVariable>>,
    /// The pixel's position and whether its triangle faces front, where the program reads them.
    frag_coord: Option<Handle<naga::GlobalVariable>>,
    front_face: Option<Handle<naga::GlobalVariable>>,
    /// The pixel's weights of its triangle's vertices, where the program computes with the
    /// pair it interpolates by.
    barycentric: Option<Handle<naga::GlobalVariable>>,
    /// A vertex input by its register, with the scalar the signature gives it.
    fetched: BTreeMap<u32, (Handle<naga::GlobalVariable>, Scalar)>,
    vertex_index: Option<Handle<naga::GlobalVariable>>,
    instance_index: Option<Handle<naga::GlobalVariable>>,
    /// An export target's private `vec4`, and the depth.
    exports: BTreeMap<u8, Handle<naga::GlobalVariable>>,
    depth: Option<Handle<naga::GlobalVariable>>,
    user_data: Option<Handle<naga::GlobalVariable>>,
    /// A constant buffer by its descriptor's place, with the rows it is read to.
    buffers: BTreeMap<Loc, (Handle<naga::GlobalVariable>, u32)>,
    textures: BTreeMap<(Loc, u8, bool), Handle<naga::GlobalVariable>>,
    /// A sampler by its descriptor's place or its constant words, and whether it compares.
    samplers: BTreeMap<SamplerKey, Handle<naga::GlobalVariable>>,
    /// A buffer read by address, by its descriptor's place and the stride the code states.
    storage: BTreeMap<(Loc, Option<u32>), Handle<naga::GlobalVariable>>,
    /// The constant buffers of a table bound whole, by the table.
    blocks: BTreeMap<Loc, Handle<naga::GlobalVariable>>,
    /// A table's own dwords, by the table.
    tables: BTreeMap<Loc, Handle<naga::GlobalVariable>>,
    /// A texture the program stores to, by its descriptor's place and dimension: its global, its
    /// format, and whether it is updated in one step.
    targets: BTreeMap<(Loc, u8), (Handle<naga::GlobalVariable>, naga::StorageFormat, bool)>,
    /// The single images this lowering stored to, those an earlier one found, and the formats
    /// the binder states.
    images_stored: StoredImages,
    /// The images stored to whole before an update in one step stated their format: the next
    /// lowering binds them for updates from their first use.
    restated: StoredImages,
    told: &'a StoredImages,
    formats: &'a BTreeMap<Loc, naga::StorageFormat>,
    /// Whether every table load also reads its words as numbers into its registers: the
    /// lowering of a program that computes with a register only some paths gave a table's word.
    eager: bool,
    /// The joined halves of a geometry pair, where the program is one.
    pair: Option<geometry::Pairing>,
    /// The buffers the program updates in one step, and whether it so updates its shared memory.
    atomics: BTreeSet<Handle<naga::GlobalVariable>>,
    shared_atomic: bool,
    /// The words after the program's last instruction as a private array, once the code reads
    /// them; `data_start` is the number of the first, and `reads_code` whether the program
    /// takes its own address at all.
    code: Option<Handle<naga::GlobalVariable>>,
    data_start: u32,
    reads_code: bool,
    /// The lanes of vector registers the program writes scalar registers into.
    spills: BTreeSet<Reg>,
    /// A compute program's launch, the private vectors of its thread's and its group's numbers,
    /// the memory its group shares and the buffers it stores to.
    launch: Option<super::Launch>,
    ids: Option<[Handle<naga::GlobalVariable>; 2]>,
    shared: Option<Handle<naga::GlobalVariable>>,
    stored: BTreeSet<Handle<naga::GlobalVariable>>,
    resources: Vec<Resource>,
    notes: BTreeMap<&'static str, u32>,
}

/// Where an image instruction's next address operand is: the register, and with 16-bit
/// addresses whether its high half is next.
struct Cursor {
    next: usize,
    high: bool,
    packed: bool,
}

/// A sampler's descriptor place or its constant words, and whether it compares.
type SamplerKey = (Option<Loc>, Option<[u32; 4]>, bool);

/// The images a program stores to, by their descriptors' places, each with the format an
/// update in one step states of it.
type StoredImages = BTreeMap<Loc, Option<naga::StorageFormat>>;

/// What one lowering came to: the module, or the images it loaded before it found that the
/// program stores to them, which the next lowering reads through their stored bindings.
enum Pass {
    Done(Box<Lowered>),
    Again(StoredImages),
}

/// The atomic update a shared-memory mnemonic makes, and whether it gives back what the dword held.
fn shared_update(name: &str) -> Option<(&'static str, bool)> {
    let rest = name.strip_prefix("ds_")?;
    let fun = match rest.replace("_rtn", "").as_str() {
        "add_u32" => "add",
        "sub_u32" => "sub",
        "and_b32" => "and",
        "or_b32" => "or",
        "xor_b32" => "xor",
        "min_u32" => "umin",
        "max_u32" => "umax",
        "min_i32" => "smin",
        "max_i32" => "smax",
        "wrxchg_b32" => "swap",
        "cmpst_b32" => "cmpswap",
        _ => return None,
    };
    Some((fun, rest.contains("_rtn_")))
}

/// The whole table a computed place is in: what a binding array is keyed and listed by.
fn whole(at: &Loc) -> Loc {
    Loc {
        root: at.root,
        path: at.path.clone(),
        offset: 0,
        index: Some(Index {
            source: IndexSource::Runtime { site: 0 },
            shift: 0,
        }),
    }
}
/// A pixel lane's neighbour as this lane's value moved by its derivatives: the neighbour in its
/// row or column to a float's rounding, the diagonal one less closely.
const REBUILT: &str = "a neighbouring lane's value rebuilt from this lane's derivatives";
/// What the blocks of one function are lowered in: its builder, and where its registers are.
type Part = (Builder, HashMap<Reg, Handle<E>>);

/// Where statements are written: how many loops of their function enclose them, and whether
/// the function is one statements were moved to.
#[derive(Debug, Clone, Copy, Default)]
struct Place {
    loops: usize,
    moved: bool,
}

type Done = Result<(), NotTranslated>;
/// One of the samplers a read may go through, and the condition a run takes it on.
type Chosen = (Handle<E>, Option<Handle<E>>);
type Value = Result<Handle<E>, NotTranslated>;

impl<'a> Cx<'a> {
    fn note(&mut self, what: &'static str) {
        *self.notes.entry(what).or_default() += 1;
    }

    fn ty(&mut self, inner: TypeInner) -> Handle<naga::Type> {
        insert(&mut self.module, inner)
    }

    fn private(&mut self, name: String, ty: Handle<naga::Type>) -> Handle<naga::GlobalVariable> {
        let global = naga::GlobalVariable {
            name: Some(name),
            space: naga::AddressSpace::Private,
            binding: None,
            ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        };
        self.module.global_variables.append(global, Span::UNDEFINED)
    }

    /// A bound global at the next binding of group 0, listed among the resources.
    fn bound(
        &mut self,
        name: String,
        inner: TypeInner,
        space: naga::AddressSpace,
        kind: ResourceKind,
        at: Option<Loc>,
    ) -> Handle<naga::GlobalVariable> {
        let ty = self.ty(inner);
        let binding = self.resources.len() as u32;
        self.resources.push(Resource {
            kind,
            at,
            words: None,
            group: 0,
            binding,
            count: None,
            written: false,
        });
        let global = naga::GlobalVariable {
            name: Some(name),
            space,
            binding: Some(naga::ResourceBinding { group: 0, binding }),
            ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        };
        self.module.global_variables.append(global, Span::UNDEFINED)
    }

    /// A bound table: every entry of it as one binding array of `inner`.
    fn bound_table(
        &mut self,
        name: String,
        inner: TypeInner,
        space: naga::AddressSpace,
        kind: ResourceKind,
        at: Loc,
    ) -> Handle<naga::GlobalVariable> {
        let base = self.ty(inner);
        let array = TypeInner::BindingArray {
            base,
            size: naga::ArraySize::Constant(
                NonZeroU32::new(TABLE_SLOTS).unwrap_or(NonZeroU32::MIN),
            ),
        };
        let global = self.bound(name, array, space, kind, Some(at));
        if let Some(resource) = self.resources.last_mut() {
            resource.count = Some(TABLE_SLOTS);
        }
        global
    }

    /// The entry of its table a descriptor is, where a run computed its place: the register holds
    /// the place in dwords, and an entry is `dwords` long.
    fn entry(&mut self, pc: u32, first: u8, at: &Loc, dwords: u32) -> Option<Handle<E>> {
        if !sym::computed(at) {
            return None;
        }
        let place = self.get(pc, Reg::Place(first));
        let by = self.b.u32(dwords);
        Some(self.b.bin(Bo::Divide, place, by))
    }

    fn rows(&mut self, rows: u32) -> TypeInner {
        let base = vector(&mut self.module, VectorSize::Quad, Scalar::U32);
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(NonZeroU32::new(rows).unwrap_or(NonZeroU32::MIN)),
            stride: 16,
        }
    }

    fn global(&mut self, global: Handle<naga::GlobalVariable>) -> Handle<E> {
        self.b.emit(E::GlobalVariable(global))
    }

    /// The pointer to a register's local, made on its first use.
    fn local(&mut self, reg: Reg) -> Handle<E> {
        if let Some(&pointer) = self.locals.get(&reg) {
            return pointer;
        }
        let scalar = match reg {
            Reg::Cond(_) => Scalar::BOOL,
            _ => Scalar::U32,
        };
        let ty = self.ty(TypeInner::Scalar(scalar));
        let pointer = match (self.spread, self.registers.get(&reg).copied()) {
            (true, Some(global)) => self.b.emit(E::GlobalVariable(global)),
            (true, None) => {
                let global = self.private(reg.name(), ty);
                self.registers.insert(reg, global);
                self.b.emit(E::GlobalVariable(global))
            }
            (false, _) => {
                let variable = naga::LocalVariable {
                    name: Some(reg.name()),
                    ty,
                    init: None,
                };
                let variable = (self.b.fun.local_variables).append(variable, Span::UNDEFINED);
                self.b.emit(E::LocalVariable(variable))
            }
        };
        self.locals.insert(reg, pointer);
        pointer
    }

    /// A register's value, with no word on whether the program wrote it.
    fn raw(&mut self, reg: Reg) -> Handle<E> {
        let pointer = self.local(reg);
        self.b.load(pointer)
    }

    fn get(&mut self, pc: u32, reg: Reg) -> Handle<E> {
        if !self.defined.contains(&reg) {
            self.used.entry(reg).or_insert(pc);
        }
        self.shared_wave.read(reg);
        // A neighbour's value read for itself: the difference of two is a derivative, which
        // reads neither register.
        if matches!(reg, Reg::V(n) if self.fragment && self.permuted.contains_key(&n)) {
            self.note(REBUILT);
        }
        self.raw(reg)
    }

    fn put(&mut self, reg: Reg, value: Handle<E>) {
        let pointer = self.local(reg);
        self.b.push(Statement::Store { pointer, value });
        self.defined.insert(reg);
        self.shared_wave.wrote(reg);
        self.waved.remove(&reg);
    }

    fn vreg(&mut self, pc: u32, n: u8) -> Value {
        if let Some(Waved::Foreign(_)) = self.waved.get(&Reg::V(n)) {
            return Err(NotTranslated::Descriptor {
                pc,
                what: "another lane's value read outside a fold over the wave",
            });
        }
        Ok(self.get(pc, Reg::V(n)))
    }

    /// The fold `name`'s steps make of `bits` over the lanes of the wave that run: the whole
    /// fold, or the running one up to this lane. Where an invocation is the whole wave it is
    /// this lane's own value.
    fn collective(
        &mut self,
        name: &str,
        how: naga::CollectiveOperation,
        bits: Handle<E>,
    ) -> Option<Handle<E>> {
        let (op, kind, half) = fold_of(name)?;
        if how != naga::CollectiveOperation::Reduce && op != naga::SubgroupOperation::Add {
            return None;
        }
        if !self.subgroups {
            self.note("a fold over the wave's lanes read as this lane's");
            let zero = self.b.u32(0);
            return Some(match how {
                naga::CollectiveOperation::ExclusiveScan => zero,
                _ => bits,
            });
        }
        if self.shared_wave.threads.is_some() {
            return self.shared_fold(name, how, bits);
        }
        let argument = match (kind, half) {
            (ScalarKind::Float, true) => self.half(bits, false),
            (ScalarKind::Float, false) => self.b.float(bits),
            (ScalarKind::Sint, _) => self.b.sint(bits),
            _ => bits,
        };
        let ty = self.ty(TypeInner::Scalar(Scalar { kind, width: 4 }));
        let result = self.b.result(E::SubgroupOperationResult { ty });
        self.b.push(Statement::SubgroupCollectiveOperation {
            op,
            collective_op: how,
            argument,
            result,
        });
        self.wave_ops = true;
        Some(match (kind, half) {
            (ScalarKind::Uint, _) => result,
            (_, true) => {
                let wide = self.b.bits(result);
                self.narrow(wide)
            }
            _ => self.b.bits(result),
        })
    }

    /// A lane mask as the wave holds it: each lane's own bit of `low`, in the two dwords of a
    /// mask of 64 lanes.
    fn ballot(&mut self, low: Handle<E>) -> [Handle<E>; 2] {
        let one = self.b.u32(1);
        let bit = self.b.bin(Bo::And, low, one);
        if self.shared_wave.threads.is_some() {
            if let Some(words) = self.shared_ballot(bit) {
                return words;
            }
        }
        let predicate = self.b.ne0(bit);
        let result = self.b.result(E::SubgroupBallotResult);
        self.b.push(Statement::SubgroupBallot {
            result,
            predicate: Some(predicate),
        });
        self.wave_ops = true;
        [self.b.index(result, 0), self.b.index(result, 1)]
    }

    /// `bits` as the lane numbered `index` of this lane's subgroup holds it.
    fn gather(&mut self, bits: Handle<E>, index: Handle<E>) -> Handle<E> {
        if self.shared_wave.threads.is_some() {
            return self.shared_gather(bits, index);
        }
        let ty = self.ty(TypeInner::Scalar(Scalar::U32));
        let result = self.b.result(E::SubgroupOperationResult { ty });
        self.b.push(Statement::SubgroupGather {
            mode: naga::GatherMode::Shuffle(index),
            argument: bits,
            result,
        });
        self.wave_ops = true;
        result
    }

    /// This lane's number in its subgroup.
    fn lane_number(&mut self) -> Handle<E> {
        if self.shared_wave.threads.is_some() {
            return self.wave_lane();
        }
        let global = match self.lane {
            Some(global) => global,
            None => {
                let ty = self.ty(TypeInner::Scalar(Scalar::U32));
                let global = self.private("lane".to_owned(), ty);
                self.lane = Some(global);
                global
            }
        };
        let pointer = self.global(global);
        self.b.load(pointer)
    }

    /// The register a fold's step accumulates in, where one operand is another lane's value of the other.
    fn fold_pair(&self, a: Operand, b: Operand) -> Option<u8> {
        let foreign = |operand: Operand| {
            let reg = match operand {
                Operand::Vgpr(n) => Reg::V(n),
                other => Reg::S(sym::register(other)?),
            };
            match self.waved.get(&reg) {
                Some(Waved::Foreign(of)) => Some(*of),
                _ => None,
            }
        };
        [(a, b), (b, a)]
            .into_iter()
            .find_map(|(other, own)| match (foreign(other), own) {
                (Some(Reg::V(of)), Operand::Vgpr(n)) if of == n => Some(n),
                _ => None,
            })
    }

    /// A step of a fold that joins the accumulator `acc` with another lane's value of it: the
    /// fold itself where no earlier step made it, and nothing more after that.
    fn fold_join(&mut self, pc: u32, inst: &Inst, name: &str, dst: u8, acc: u8) -> Done {
        use naga::CollectiveOperation::Reduce;
        let held = self.vreg(pc, acc)?;
        let folded = self.waved.get(&Reg::V(acc)) == Some(&Waved::Folded);
        let none = || form(pc, inst, "a fold over the wave with no collective");
        let value = match (name, folded) {
            (_, true) => held,
            // The low dword of a 64-bit sum: its 16-bit halves summed apart, so no carry is lost.
            ("v_add_co_u32", false) => {
                let (low, sixteen) = (self.b.u32(0xFFFF), self.b.u32(16));
                let (a, b) = (
                    self.b.bin(Bo::And, held, low),
                    self.b.bin(Bo::ShiftRight, held, sixteen),
                );
                let a = self.collective("v_add_u32", Reduce, a).ok_or_else(none)?;
                let b = self.collective("v_add_u32", Reduce, b).ok_or_else(none)?;
                let raised = self.b.bin(Bo::ShiftLeft, b, sixteen);
                let sum = self.b.bin(Bo::Add, a, raised);
                let over = self.b.bin(Bo::ShiftRight, a, sixteen);
                let upper = self.b.bin(Bo::Add, b, over);
                self.carry = Some(self.b.bin(Bo::ShiftRight, upper, sixteen));
                sum
            }
            ("v_addc_co_u32", false) => {
                let carry = self.carry.take().ok_or_else(none)?;
                let sum = (self.collective("v_add_u32", Reduce, held)).ok_or_else(none)?;
                self.b.bin(Bo::Add, sum, carry)
            }
            _ => self.collective(name, Reduce, held).ok_or_else(none)?,
        };
        self.vput(dst, value);
        self.waved.insert(Reg::V(dst), Waved::Folded);
        Ok(())
    }

    /// A step of a fold over the wave: the instruction's first source is its second as another
    /// lane holds it, by a pattern that is no quad's. The first step is the whole fold.
    #[allow(clippy::too_many_arguments)]
    fn fold_step(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        (src0, src1): (Operand, u8),
        word: u32,
    ) -> Done {
        if word as u8 != src1 {
            return Err(form(pc, inst, "a fold over the wave of two registers"));
        }
        let held = self.vreg(pc, src1)?;
        let value = match self.waved.get(&Reg::V(src1)) {
            Some(Waved::Folded) => held,
            _ => {
                let how = match src0 {
                    Operand::Dpp if (word >> 8) & 0x1FF == ROW_SHR_1 => {
                        naga::CollectiveOperation::InclusiveScan
                    }
                    Operand::Dpp8 | Operand::Dpp8Fi if word >> 8 == PAIR_SWAP => {
                        naga::CollectiveOperation::Reduce
                    }
                    _ => {
                        return Err(form(
                            pc,
                            inst,
                            "a lane permutation that opens no fold over the wave",
                        ))
                    }
                };
                (self.collective(name, how, held))
                    .ok_or_else(|| form(pc, inst, "a fold over the wave with no collective"))?
            }
        };
        self.vput(dst, value);
        self.waved.insert(Reg::V(dst), Waved::Folded);
        Ok(())
    }

    /// The pixel's position, as the stage gives it.
    fn position(&mut self) -> Handle<E> {
        let global = match self.frag_coord {
            Some(global) => global,
            None => {
                let ty = vector(&mut self.module, VectorSize::Quad, Scalar::F32);
                let global = self.private("frag_coord".to_owned(), ty);
                self.frag_coord = Some(global);
                global
            }
        };
        let pointer = self.global(global);
        self.b.load(pointer)
    }

    fn sreg(&mut self, pc: u32, r: u8) -> Value {
        match self.state.get(usize::from(r)).cloned() {
            Some(Sym::Word(at)) => self.table_word(pc, r, &at),
            // Only some paths gave the register a table's word: a number where every table
            // load read its words as numbers, which the next lowering does.
            Some(Sym::Mixed) if !self.eager => Err(NotTranslated::Descriptor {
                pc,
                what: MIXED_NUMBER,
            }),
            _ => Ok(self.get(pc, Reg::S(r))),
        }
    }

    /// A table's word read as a number: the dword of the table's own memory, where a descriptor
    /// states an image's size or a buffer's length.
    fn table_word(&mut self, pc: u32, r: u8, at: &Loc) -> Value {
        if let Some(stated) = self.pair_word(at) {
            return Ok(stated);
        }
        let key = Loc {
            root: at.root,
            path: at.path.clone(),
            offset: 0,
            index: None,
        };
        let global = match self.tables.get(&key) {
            Some(&global) => global,
            None => {
                let inner = self.rows(TABLE_SLOTS * SLOT_DWORDS / 4);
                let global = self.bound(
                    format!("table{}", self.tables.len()),
                    inner,
                    naga::AddressSpace::Uniform,
                    ResourceKind::TableWords,
                    Some(key.clone()),
                );
                self.tables.insert(key, global);
                global
            }
        };
        let pointer = self.global(global);
        let cell = match at.index {
            None => {
                let word = at.offset / 4;
                if word >= TABLE_SLOTS * SLOT_DWORDS {
                    return Err(NotTranslated::Descriptor {
                        pc,
                        what: "a table's word past the entries a table is bound with",
                    });
                }
                let row = self.b.index(pointer, word / 4);
                self.b.index(row, word % 4)
            }
            // An indexed load left the word's place in its table beside the register.
            Some(_) => {
                let place = self.get(pc, Reg::Place(r));
                let (two, three) = (self.b.u32(2), self.b.u32(3));
                let (row, column) = (
                    self.b.bin(Bo::ShiftRight, place, two),
                    self.b.bin(Bo::And, place, three),
                );
                let row = self.b.emit(E::Access {
                    base: pointer,
                    index: row,
                });
                self.b.emit(E::Access {
                    base: row,
                    index: column,
                })
            }
        };
        Ok(self.b.load(cell))
    }

    /// Whether this lane runs: bit 0 of the lane mask.
    fn active(&mut self) -> Handle<E> {
        let exec = self.raw(Reg::S(EXEC));
        let one = self.b.u32(1);
        let bit = self.b.bin(Bo::And, exec, one);
        self.b.ne0(bit)
    }

    /// Writes a vector register, in an active lane only where the program masks lanes.
    fn vput(&mut self, n: u8, value: Handle<E>) {
        let value = match self.masked {
            true => {
                let (active, old) = (self.active(), self.raw(Reg::V(n)));
                self.b.select(active, value, old)
            }
            false => value,
        };
        self.put(Reg::V(n), value);
        *self.versions.entry(n).or_default() += 1;
        self.permuted.remove(&n);
    }

    /// A scalar source's dword.
    fn sv(&mut self, pc: u32, operand: Operand) -> Value {
        if let Some(r) = sym::register(operand) {
            return self.sreg(pc, r);
        }
        Ok(match operand {
            Operand::Null => self.b.u32(0),
            Operand::Int(n) => self.b.u32(n as u32),
            Operand::Float(word) | Operand::Literal(word) => self.b.u32(word),
            Operand::Scc => self.get(pc, Reg::Scc),
            Operand::Vccz | Operand::Execz => {
                let base = if operand == Operand::Vccz { VCC } else { EXEC };
                let any = self.mask_any(pc, base)?;
                let none = self.b.lnot(any);
                self.b.flag(none)
            }
            Operand::Vgpr(n) => self.vreg(pc, n)?,
            _ => {
                return Err(NotTranslated::Operand {
                    pc,
                    operand: operand.to_string(),
                })
            }
        })
    }

    /// A 64-bit scalar source as its two dwords: a register pair, or a constant extended.
    fn sv64(&mut self, pc: u32, operand: Operand) -> Result<[Handle<E>; 2], NotTranslated> {
        if let Some(r) = sym::register(operand) {
            return Ok([self.sreg(pc, r)?, self.sreg(pc, r + 1)?]);
        }
        Ok(match operand {
            Operand::Null => [self.b.u32(0), self.b.u32(0)],
            Operand::Int(n) => [self.b.u32(n as u32), self.b.u32((n >> 31) as u32)],
            Operand::Literal(word) => [self.b.u32(word), self.b.u32(0)],
            _ => {
                return Err(NotTranslated::Operand {
                    pc,
                    operand: operand.to_string(),
                })
            }
        })
    }

    fn sput(&mut self, pc: u32, dst: Operand, value: Handle<E>) -> Done {
        match (sym::register(dst), dst) {
            (Some(r), _) => self.put(Reg::S(r), value),
            (None, Operand::Null) => {}
            (None, _) => {
                return Err(NotTranslated::Operand {
                    pc,
                    operand: dst.to_string(),
                })
            }
        }
        Ok(())
    }

    fn sput64(&mut self, pc: u32, dst: Operand, value: [Handle<E>; 2]) -> Done {
        match (sym::register(dst), dst) {
            (Some(r), _) => {
                self.put(Reg::S(r), value[0]);
                self.put(Reg::S(r + 1), value[1]);
            }
            (None, Operand::Null) => {}
            (None, _) => {
                return Err(NotTranslated::Operand {
                    pc,
                    operand: dst.to_string(),
                })
            }
        }
        Ok(())
    }

    fn scc(&mut self, condition: Handle<E>) {
        let flag = self.b.flag(condition);
        self.put(Reg::Scc, flag);
    }

    fn scc_set(&mut self, pc: u32) -> Handle<E> {
        let scc = self.get(pc, Reg::Scc);
        self.b.ne0(scc)
    }

    /// Whether a lane mask has any lane set: both halves of a 64-lane mask, the low one of a 32-lane.
    // ponytail: this lane stands for the wave, so a vote over the wave is this lane's own bit.
    fn mask_any(&mut self, pc: u32, base: u8) -> Value {
        let mut bits = self.sreg(pc, base)?;
        if !self.program.wave32 {
            let high = self.sreg(pc, base + 1)?;
            bits = self.b.bin(Bo::InclusiveOr, bits, high);
        }
        Ok(self.b.ne0(bits))
    }

    /// This lane's bit of the mask in register `base`.
    fn mask_bit(&mut self, pc: u32, base: u8) -> Value {
        let mask = self.sreg(pc, base)?;
        let one = self.b.u32(1);
        let bit = self.b.bin(Bo::And, mask, one);
        Ok(self.b.ne0(bit))
    }

    /// Writes a comparison's result as a lane mask: this lane's bit where it is active, nothing of the other lanes.
    fn put_mask(&mut self, base: u8, condition: Handle<E>) {
        let active = self.active();
        let bit = self.b.bin(Bo::LogicalAnd, condition, active);
        let low = self.b.flag(bit);
        self.put(Reg::S(base), low);
        if !self.program.wave32 {
            let zero = self.b.u32(0);
            self.put(Reg::S(base + 1), zero);
        }
        self.shared_wave.masks.insert(base);
    }

    /// Clears this lane's bit of the lane mask where `condition` fails.
    fn narrow_exec(&mut self, condition: Handle<E>) {
        let exec = self.raw(Reg::S(EXEC));
        let (all, others) = (self.b.u32(u32::MAX), self.b.u32(!1));
        let keep = self.b.select(condition, all, others);
        let value = self.b.bin(Bo::And, exec, keep);
        self.put(Reg::S(EXEC), value);
    }

    /// Whether a mask a scalar instruction wrote holds a lane, as the wave sees it. A pixel
    /// program takes the lanes it kills out of its live mask and leaves if no lane of the wave
    /// is left; a lane killed whose neighbours live runs on as their helper, so here the wave
    /// always holds one, and the lane is discarded where the program ends.
    fn lanes_left(&mut self, name: &str, [dst, a, b]: [Operand; 3], value: Handle<E>) -> Handle<E> {
        let kill = self.fragment
            && matches!(name, "s_andn2_b64" | "s_andn2_b32")
            && sym::register(dst).is_some_and(|r| r != EXEC && r != VCC)
            && sym::register(dst) == sym::register(a)
            && sym::register(b) == Some(EXEC);
        match kill {
            true => self.b.emit(E::Literal(Literal::Bool(true))),
            false => self.b.ne0(value),
        }
    }

    /// A lane mask in whole-quad mode, as this lane holds it: its own bit, set too where a
    /// lane of its quad is in the mask. A pixel program asks for it to keep the lanes it killed
    /// running as helpers, and here such a lane runs on whether or not its quad holds a live
    /// one: where it holds none, nothing reads what the lane computes. Elsewhere the mask is
    /// the wave's at entry, with this lane in it.
    fn whole_quad(&mut self, low: Handle<E>) -> Handle<E> {
        if !self.fragment {
            return low;
        }
        let one = self.b.u32(1);
        self.b.bin(Bo::InclusiveOr, low, one)
    }

    fn sopp(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let condition = match name {
            "s_nop" | "s_inst_prefetch" | "s_waitcnt" | "s_waitcnt_depctr" | "s_endpgm"
            | "s_code_end" | "s_branch" => return Ok(()),
            // The counts a mesh program announces and the wait for its other threads: each thread
            // here runs alone and says by its own exports what it placed.
            "s_sendmsg" | "s_barrier" if self.mesh.is_some() || self.pair.is_some() => {
                return Ok(())
            }
            // A compute group's threads wait for one another before any reads what another wrote.
            "s_barrier" if self.launch.is_some() => {
                (self.b).push(Statement::ControlBarrier(naga::Barrier::WORK_GROUP));
                return Ok(());
            }
            "s_cbranch_scc0" | "s_cbranch_scc1" => {
                let set = self.scc_set(pc);
                let set = self.voted(set);
                match name {
                    "s_cbranch_scc0" => self.b.lnot(set),
                    _ => set,
                }
            }
            "s_cbranch_vccz" | "s_cbranch_vccnz" | "s_cbranch_execz" | "s_cbranch_execnz" => {
                let base = if name.contains("vcc") { VCC } else { EXEC };
                let any = self.mask_any(pc, base)?;
                let any = self.voted(any);
                match name.ends_with("nz") {
                    true => any,
                    false => self.b.lnot(any),
                }
            }
            _ => return Err(refused(pc, inst)),
        };
        self.put(Reg::Cond(self.index), condition);
        Ok(())
    }

    /// The place a scalar source's table word is at, where a run computed it: what a move of
    /// the word into a vector register carries.
    fn computed_place(&mut self, pc: u32, operand: Operand) -> Option<Handle<E>> {
        let r = sym::register(operand)?;
        match self.state.get(usize::from(r)) {
            Some(Sym::Word(at)) if at.index.is_some() => Some(self.get(pc, Reg::Place(r))),
            _ => None,
        }
    }

    /// Gives scalar register `r` the table word at `place`, which a vector register carried.
    fn put_place(&mut self, pc: u32, r: u8, place: Handle<E>, at: &Loc) {
        self.put(Reg::Place(r), place);
        self.defined.insert(Reg::S(r));
        if self.eager {
            if let Ok(value) = self.table_word(pc, r, at) {
                self.put(Reg::S(r), value);
            }
        }
    }

    /// Whether a scalar source holds a table's word, which a move carries without reading.
    fn carries_table(&self, operand: Operand, words: u8) -> bool {
        sym::register(operand).is_some_and(|r| {
            (0..words).any(|k| {
                matches!(
                    self.state.get(usize::from(r) + usize::from(k)),
                    Some(Sym::Word(_) | Sym::Mixed)
                )
            })
        })
    }

    fn define_scalars(&mut self, dst: Operand, words: u8) {
        if let Some(r) = sym::register(dst) {
            for k in 0..words {
                self.defined.insert(Reg::S(r + k));
            }
        }
    }

    fn sop1(&mut self, pc: u32, inst: &Inst, name: &str, dst: Operand, src: Operand) -> Done {
        match name {
            "s_mov_b32" | "s_mov_b64" => {
                let words = if name == "s_mov_b64" { 2 } else { 1 };
                if self.carries_table(src, words) {
                    // A table's word moves as the place it holds, which is a number only where a run computed it.
                    match (sym::register(src), sym::register(dst)) {
                        (Some(from), Some(to)) => {
                            let values: Vec<_> = (0..words)
                                .map(|k| {
                                    let held = self.get(pc, Reg::S(from + k));
                                    (held, self.get(pc, Reg::Place(from + k)))
                                })
                                .collect();
                            for (k, (value, place)) in (0..words).zip(values) {
                                self.put(Reg::S(to + k), value);
                                self.put(Reg::Place(to + k), place);
                            }
                        }
                        _ => self.define_scalars(dst, words),
                    }
                } else if words == 2 {
                    let value = self.sv64(pc, src)?;
                    self.sput64(pc, dst, value)?;
                } else {
                    let value = self.sv(pc, src)?;
                    self.sput(pc, dst, value)?;
                }
            }
            "s_not_b32" | "s_wqm_b32" => {
                let mut value = self.sv(pc, src)?;
                value = match name {
                    "s_not_b32" => self.b.not(value),
                    _ => self.whole_quad(value),
                };
                self.sput(pc, dst, value)?;
                let any = self.b.ne0(value);
                self.scc(any);
            }
            "s_not_b64" | "s_wqm_b64" => {
                let mut value = self.sv64(pc, src)?;
                value = match name {
                    "s_not_b64" => [self.b.not(value[0]), self.b.not(value[1])],
                    _ => [self.whole_quad(value[0]), value[1]],
                };
                self.sput64(pc, dst, value)?;
                let either = self.b.bin(Bo::InclusiveOr, value[0], value[1]);
                let any = self.b.ne0(either);
                self.scc(any);
            }
            // A wave of 32 lanes keeps a lane mask in one dword.
            "s_bcnt1_i32_b32" | "s_ff1_i32_b32" if self.program.wave32 && self.holds_mask(src) => {
                let (low, zero) = (self.sv(pc, src)?, self.b.u32(0));
                self.mask_read(pc, name, dst, [low, zero])?;
            }
            "s_brev_b32" | "s_bcnt1_i32_b32" | "s_ff1_i32_b32" => {
                // Half of a wave of 64 lanes' mask holds this lane's bit alone here, or nothing.
                if name != "s_brev_b32" && self.holds_mask(src) {
                    self.note("a count over half a wave's mask read as this lane's");
                }
                let value = self.sv(pc, src)?;
                let fun = match name {
                    "s_brev_b32" => Mf::ReverseBits,
                    "s_bcnt1_i32_b32" => Mf::CountOneBits,
                    _ => Mf::FirstTrailingBit,
                };
                let value = self.b.math(fun, &[value]);
                self.sput(pc, dst, value)?;
                if name == "s_bcnt1_i32_b32" {
                    let any = self.b.ne0(value);
                    self.scc(any);
                }
            }
            "s_bcnt1_i32_b64" | "s_ff1_i32_b64" => {
                let own = self.sv64(pc, src)?;
                self.mask_read(pc, name, dst, own)?;
            }
            "s_flbit_i32_b32" => {
                // The first set bit counted from the top, and every bit set where there is none.
                let value = self.sv(pc, src)?;
                let (zero, none) = (self.b.u32(0), self.b.u32(u32::MAX));
                let leading = self.b.math(Mf::CountLeadingZeros, &[value]);
                let empty = self.b.bin(Bo::Equal, value, zero);
                let value = self.b.select(empty, none, leading);
                self.sput(pc, dst, value)?;
            }
            "s_abs_i32" => {
                let value = self.sv(pc, src)?;
                let signed = self.b.sint(value);
                let magnitude = self.b.math(Mf::Abs, &[signed]);
                let value = self.b.bits(magnitude);
                self.sput(pc, dst, value)?;
                let any = self.b.ne0(value);
                self.scc(any);
            }
            "s_sext_i32_i8" | "s_sext_i32_i16" => {
                let value = self.sv(pc, src)?;
                let by = self.b.u32(if name == "s_sext_i32_i8" { 24 } else { 16 });
                let up = self.b.bin(Bo::ShiftLeft, value, by);
                let value = self.b.shift(Bo::ShiftRight, true, up, by);
                self.sput(pc, dst, value)?;
            }
            "s_bitset0_b32" | "s_bitset1_b32" => {
                let (old, bit) = (self.sv(pc, dst)?, self.sv(pc, src)?);
                let one = self.b.u32(1);
                let mask = self.b.shift(Bo::ShiftLeft, false, one, bit);
                let value = match name {
                    "s_bitset1_b32" => self.b.bin(Bo::InclusiveOr, old, mask),
                    _ => {
                        let rest = self.b.not(mask);
                        self.b.bin(Bo::And, old, rest)
                    }
                };
                self.sput(pc, dst, value)?;
            }
            "s_getpc_b64" => {
                // The code's first word is at address zero here, so the next instruction's
                // address is four times its word's number.
                let value = [self.b.u32(4 * (pc + 1)), self.b.u32(0)];
                self.sput64(pc, dst, value)?;
            }
            // A far branch, which the graph has followed to where it lands.
            "s_setpc_b64" => {}
            "s_swappc_b64" if !self.fragment && self.fetched.is_empty() => self.fetch(pc, inst)?,
            _ if name.contains("_saveexec_") || name.contains("_wrexec_") => {
                self.save_exec(pc, inst, name, dst, src)?
            }
            _ => return Err(refused(pc, inst)),
        }
        Ok(())
    }

    /// `s_<f>_saveexec` keeps the lane mask in its destination and sets it to `f` of the source and
    /// itself; `s_<f>_wrexec` sets it and copies the new one.
    /// A count of a lane mask's lanes, or the first of them. A lane mask holds this lane's
    /// bit alone; the wave's is every lane's, gathered.
    fn mask_read(&mut self, pc: u32, name: &str, dst: Operand, own: [Handle<E>; 2]) -> Done {
        let count = name.starts_with("s_bcnt1");
        let [low, high] = match self.subgroups {
            true => self.ballot(own[0]),
            false => {
                self.note("a count over the wave's lanes read as this lane's");
                own
            }
        };
        let value = match count {
            true => {
                let (a, b) = (
                    self.b.math(Mf::CountOneBits, &[low]),
                    self.b.math(Mf::CountOneBits, &[high]),
                );
                self.b.bin(Bo::Add, a, b)
            }
            false => {
                let (first, second) = (
                    self.b.math(Mf::FirstTrailingBit, &[low]),
                    self.b.math(Mf::FirstTrailingBit, &[high]),
                );
                let (zero, word, none) = (self.b.u32(0), self.b.u32(32), self.b.u32(u32::MAX));
                let past = self.b.bin(Bo::Add, second, word);
                let (no_low, no_high) = (
                    self.b.bin(Bo::Equal, low, zero),
                    self.b.bin(Bo::Equal, high, zero),
                );
                let upper = self.b.select(no_high, none, past);
                self.b.select(no_low, upper, first)
            }
        };
        self.sput(pc, dst, value)?;
        if count {
            let any = self.b.ne0(value);
            self.scc(any);
        }
        Ok(())
    }

    fn save_exec(&mut self, pc: u32, inst: &Inst, name: &str, dst: Operand, src: Operand) -> Done {
        let wide = name.ends_with("_b64");
        let save = name.contains("_saveexec_");
        let kind = (name.strip_prefix("s_"))
            .and_then(|rest| rest.split('_').next())
            .unwrap_or_default();
        let source = match wide {
            true => self.sv64(pc, src)?.to_vec(),
            false => vec![self.sv(pc, src)?],
        };
        let mut old = Vec::with_capacity(2);
        let mut new = Vec::with_capacity(2);
        for (half, &s) in source.iter().enumerate() {
            let exec = self.raw(Reg::S(EXEC + half as u8));
            let value = (self.b.logic(kind, s, exec)).ok_or_else(|| refused(pc, inst))?;
            old.push(exec);
            new.push(value);
        }
        let kept = if save { &old } else { &new };
        match wide {
            true => self.sput64(pc, dst, [kept[0], kept[1]])?,
            false => self.sput(pc, dst, kept[0])?,
        }
        for (half, &value) in new.iter().enumerate() {
            self.put(Reg::S(EXEC + half as u8), value);
        }
        let mut bits = new[0];
        if let Some(&high) = new.get(1) {
            bits = self.b.bin(Bo::InclusiveOr, bits, high);
        }
        let any = self.b.ne0(bits);
        self.scc(any);
        Ok(())
    }

    /// The call a vertex program opens with runs a fetch routine the program does not hold: each
    /// element of the input signature arrives in the four registers from `4 + 4 * register`.
    fn fetch(&mut self, pc: u32, inst: &Inst) -> Done {
        if self.pair.is_some() {
            return self.pair_fetch(pc, inst);
        }
        let program = self.program;
        for input in &program.vertex_inputs {
            let first = FETCH_BASE + 4 * input.register;
            if first + 3 > u32::from(u8::MAX) {
                return Err(form(
                    pc,
                    inst,
                    "an input register past the vector registers",
                ));
            }
            let scalar = match input.component_type {
                1 => Scalar::U32,
                2 => Scalar::I32,
                _ => Scalar::F32,
            };
            let ty = vector(&mut self.module, VectorSize::Quad, scalar);
            let global = match self.fetched.get(&input.register) {
                Some(&(global, _)) => global,
                None => {
                    let global = self.private(format!("in{}", input.register), ty);
                    self.fetched.insert(input.register, (global, scalar));
                    global
                }
            };
            let pointer = self.global(global);
            let value = self.b.load(pointer);
            for c in 0..4u32 {
                // A component the element does not hold reads as the format's default: 0, 0, 0, 1.
                let part = match (input.mask >> c & 1 != 0, c, scalar.kind) {
                    (true, _, _) => {
                        let part = self.b.index(value, c);
                        self.b.bits(part)
                    }
                    (false, 3, ScalarKind::Float) => self.b.u32(1f32.to_bits()),
                    (false, 3, _) => self.b.u32(1),
                    (false, _, _) => self.b.u32(0),
                };
                self.vput((first + c) as u8, part);
            }
        }
        // The call leaves its return address in the destination pair, which nothing reads back.
        if let Inst::Sop1 { dst, .. } = inst {
            self.define_scalars(*dst, 2);
        }
        if program.vertex_inputs.is_empty() {
            self.note("a fetch call with no input signature");
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn sop2(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: Operand,
        src0: Operand,
        src1: Operand,
    ) -> Done {
        if matches!(name, "s_lshl_b64" | "s_lshr_b64" | "s_ashr_i64") {
            let (pair, by) = (self.sv64(pc, src0)?, self.sv(pc, src1)?);
            let value = self.shifted64(name, by, pair);
            self.sput64(pc, dst, value)?;
            let either = self.b.bin(Bo::InclusiveOr, value[0], value[1]);
            let any = self.b.ne0(either);
            self.scc(any);
            return Ok(());
        }
        let bare = name.strip_prefix("s_").unwrap_or(name);
        if let Some(kind) = bare.strip_suffix("_b64") {
            if matches!(
                kind,
                "and" | "or" | "xor" | "andn2" | "orn2" | "nand" | "nor" | "xnor"
            ) {
                let (a, b) = (self.sv64(pc, src0)?, self.sv64(pc, src1)?);
                let low = self.b.logic(kind, a[0], b[0]);
                let high = self.b.logic(kind, a[1], b[1]);
                let (Some(low), Some(high)) = (low, high) else {
                    return Err(refused(pc, inst));
                };
                let value = [low, high];
                self.sput64(pc, dst, value)?;
                let either = self.b.bin(Bo::InclusiveOr, value[0], value[1]);
                let any = self.lanes_left(name, [dst, src0, src1], either);
                self.scc(any);
                return Ok(());
            }
            if kind == "cselect" {
                let (a, b) = (self.sv64(pc, src0)?, self.sv64(pc, src1)?);
                let set = self.scc_set(pc);
                let value = [
                    self.b.select(set, a[0], b[0]),
                    self.b.select(set, a[1], b[1]),
                ];
                return self.sput64(pc, dst, value);
            }
            if kind == "bfm" {
                return self.field_mask64(pc, dst, src0, src1);
            }
            return Err(refused(pc, inst));
        }
        let (a, b) = (self.sv(pc, src0)?, self.sv(pc, src1)?);
        if let Some(value) = (bare.strip_suffix("_b32")).and_then(|kind| self.b.logic(kind, a, b)) {
            self.sput(pc, dst, value)?;
            let any = self.lanes_left(name, [dst, src0, src1], value);
            self.scc(any);
            return Ok(());
        }
        let top = |b: &mut Builder, bits: Handle<E>| {
            let by = b.u32(31);
            let sign = b.bin(Bo::ShiftRight, bits, by);
            b.ne0(sign)
        };
        let (value, scc) = match name {
            "s_add_u32" => {
                let sum = self.b.bin(Bo::Add, a, b);
                (sum, Some(self.b.bin(Bo::Less, sum, a)))
            }
            "s_sub_u32" => {
                let difference = self.b.bin(Bo::Subtract, a, b);
                (difference, Some(self.b.bin(Bo::Greater, b, a)))
            }
            "s_add_i32" => {
                let sum = self.b.bin(Bo::Add, a, b);
                let (x, y) = (
                    self.b.bin(Bo::ExclusiveOr, a, sum),
                    self.b.bin(Bo::ExclusiveOr, b, sum),
                );
                let both = self.b.bin(Bo::And, x, y);
                (sum, Some(top(&mut self.b, both)))
            }
            "s_sub_i32" => {
                let difference = self.b.bin(Bo::Subtract, a, b);
                let (x, y) = (
                    self.b.bin(Bo::ExclusiveOr, a, b),
                    self.b.bin(Bo::ExclusiveOr, a, difference),
                );
                let both = self.b.bin(Bo::And, x, y);
                (difference, Some(top(&mut self.b, both)))
            }
            "s_addc_u32" => {
                let carry = self.get(pc, Reg::Scc);
                let partial = self.b.bin(Bo::Add, a, b);
                let sum = self.b.bin(Bo::Add, partial, carry);
                let (first, second) = (
                    self.b.bin(Bo::Less, partial, a),
                    self.b.bin(Bo::Less, sum, partial),
                );
                (sum, Some(self.b.bin(Bo::LogicalOr, first, second)))
            }
            "s_subb_u32" => {
                let borrow = self.get(pc, Reg::Scc);
                let partial = self.b.bin(Bo::Subtract, a, b);
                let difference = self.b.bin(Bo::Subtract, partial, borrow);
                let (first, second) = (
                    self.b.bin(Bo::Greater, b, a),
                    self.b.bin(Bo::Greater, borrow, partial),
                );
                (difference, Some(self.b.bin(Bo::LogicalOr, first, second)))
            }
            "s_min_i32" | "s_max_i32" | "s_min_u32" | "s_max_u32" => {
                let rel = if name.contains("min") { "lt" } else { "gt" };
                let chosen = (self.b.icmp(rel, name.ends_with("i32"), a, b))
                    .ok_or_else(|| refused(pc, inst))?;
                (self.b.select(chosen, a, b), Some(chosen))
            }
            "s_cselect_b32" => {
                let set = self.scc_set(pc);
                (self.b.select(set, a, b), None)
            }
            "s_lshl_b32" | "s_lshr_b32" | "s_ashr_i32" => {
                let value = match name {
                    "s_lshl_b32" => self.b.shift(Bo::ShiftLeft, false, a, b),
                    "s_lshr_b32" => self.b.shift(Bo::ShiftRight, false, a, b),
                    _ => self.b.shift(Bo::ShiftRight, true, a, b),
                };
                (value, Some(self.b.ne0(value)))
            }
            "s_bfm_b32" => {
                let low = self.b.u32(31);
                let width = self.b.bin(Bo::And, a, low);
                let mask = self.b.low_bits(width);
                (self.b.shift(Bo::ShiftLeft, false, mask, b), None)
            }
            "s_mul_i32" => (self.b.bin(Bo::Multiply, a, b), None),
            "s_mul_hi_u32" => (self.b.mul_hi(false, a, b), None),
            "s_mul_hi_i32" => (self.b.mul_hi(true, a, b), None),
            "s_bfe_u32" | "s_bfe_i32" => {
                // The second source holds the offset in its low five bits and the width in bits 22:16.
                let (sixteen, low7) = (self.b.u32(16), self.b.u32(0x7F));
                let width = self.b.bin(Bo::ShiftRight, b, sixteen);
                let width = self.b.bin(Bo::And, width, low7);
                let mask = self.b.low_bits(width);
                let shifted = self.b.shift(Bo::ShiftRight, false, a, b);
                let mut value = self.b.bin(Bo::And, shifted, mask);
                if name == "s_bfe_i32" {
                    // Sign-extended from the field's top bit; a width of 32 or more leaves it as it is.
                    let (limit, zero) = (self.b.u32(32), self.b.u32(0));
                    let spare = self.b.bin(Bo::Subtract, limit, width);
                    let narrow = self.b.bin(Bo::Less, width, limit);
                    let spare = self.b.select(narrow, spare, zero);
                    let up = self.b.shift(Bo::ShiftLeft, false, value, spare);
                    value = self.b.shift(Bo::ShiftRight, true, up, spare);
                }
                (value, Some(self.b.ne0(value)))
            }
            "s_absdiff_i32" => {
                let difference = self.b.bin(Bo::Subtract, a, b);
                let signed = self.b.sint(difference);
                let magnitude = self.b.math(Mf::Abs, &[signed]);
                let value = self.b.bits(magnitude);
                (value, Some(self.b.ne0(value)))
            }
            "s_lshl1_add_u32" | "s_lshl2_add_u32" | "s_lshl3_add_u32" | "s_lshl4_add_u32" => {
                let by = self.b.u32(u32::from(name.as_bytes()[6] - b'0'));
                let shifted = self.b.bin(Bo::ShiftLeft, a, by);
                let sum = self.b.bin(Bo::Add, shifted, b);
                (sum, Some(self.b.bin(Bo::Less, sum, b)))
            }
            _ => return Err(refused(pc, inst)),
        };
        self.sput(pc, dst, value)?;
        if let Some(scc) = scc {
            self.scc(scc);
        }
        Ok(())
    }

    fn sopk(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: Operand,
        imm: i16,
        imm32: Option<u32>,
    ) -> Done {
        let signed = self.b.u32(i32::from(imm) as u32);
        match name {
            // A wait for the stores or loads still under way, which here are done as they are met.
            _ if name.starts_with("s_waitcnt_") => {}
            "s_movk_i32" => self.sput(pc, dst, signed)?,
            "s_cmovk_i32" => {
                let (set, old) = (self.scc_set(pc), self.sv(pc, dst)?);
                let value = self.b.select(set, signed, old);
                self.sput(pc, dst, value)?;
            }
            "s_addk_i32" => {
                let a = self.sv(pc, dst)?;
                let sum = self.b.bin(Bo::Add, a, signed);
                let (x, y) = (
                    self.b.bin(Bo::ExclusiveOr, a, sum),
                    self.b.bin(Bo::ExclusiveOr, signed, sum),
                );
                let both = self.b.bin(Bo::And, x, y);
                let by = self.b.u32(31);
                let sign = self.b.bin(Bo::ShiftRight, both, by);
                let overflow = self.b.ne0(sign);
                self.sput(pc, dst, sum)?;
                self.scc(overflow);
            }
            "s_mulk_i32" => {
                let a = self.sv(pc, dst)?;
                let value = self.b.bin(Bo::Multiply, a, signed);
                self.sput(pc, dst, value)?;
            }
            "s_setreg_b32" | "s_setreg_imm32_b32" => {
                // The immediate names a register, a first bit and a width. Only the float mode is
                // written here, and only its single-precision rounding changes what this module computes.
                let word = imm as u16;
                let (id, first, width) = (
                    word & 0x3F,
                    u32::from(word >> 6 & 0x1F),
                    u32::from(word >> 11) + 1,
                );
                // Register 25 turns on the order pixels at one place are shaded in.
                if id == 25 {
                    return Err(NotTranslated::Pipeline(ORDERED));
                }
                let rounding = first < 2;
                let nearest = match imm32 {
                    Some(value) => {
                        value & (((1u64 << width.min(2 - first.min(2))) - 1) as u32) == 0
                    }
                    None => false,
                };
                // Registers 20 and 21 are the scratch memory's base, which only scratch instructions read.
                let scratch = matches!(id, 20 | 21);
                if !scratch && (id != 1 || (rounding && !nearest) || width + first > 32) {
                    return Err(form(pc, inst, "a mode other than round-to-nearest"));
                }
                // Bits 2 and 3 round 16-bit results: to the nearest, or toward zero, which
                // each narrowing after this reads as the program runs.
                let halves = match (id, imm32, self.half_mode) {
                    (1, Some(value), Some(global)) if first <= 2 && first + width >= 4 => {
                        let toward_zero = match (value >> (2 - first)) & 3 {
                            0 => false,
                            3 => true,
                            _ => {
                                return Err(form(
                                    pc,
                                    inst,
                                    "16-bit results rounded toward an infinity",
                                ))
                            }
                        };
                        let value = self.b.emit(E::Literal(Literal::Bool(toward_zero)));
                        let pointer = self.global(global);
                        self.b.push(Statement::Store { pointer, value });
                        true
                    }
                    _ => first >= 4 || first + width <= 2,
                };
                // The four rounding bits are all this module computes by.
                if !scratch && !(halves && id == 1 && first + width <= 4) {
                    self.note("a float mode write that is not modelled");
                }
            }
            _ => {
                let Some(rest) = name.strip_prefix("s_cmpk_") else {
                    return Err(refused(pc, inst));
                };
                let (rel, kind) = rest.split_once('_').unwrap_or((rest, ""));
                let a = self.sv(pc, dst)?;
                let b = match kind {
                    "i32" => signed,
                    _ => self.b.u32(u32::from(imm as u16)),
                };
                let condition =
                    (self.b.icmp(rel, kind == "i32", a, b)).ok_or_else(|| refused(pc, inst))?;
                self.scc(condition);
            }
        }
        Ok(())
    }

    fn sopc(&mut self, pc: u32, inst: &Inst, name: &str, src0: Operand, src1: Operand) -> Done {
        let condition = match name {
            "s_cmp_eq_u64" | "s_cmp_lg_u64" => {
                let (a, b) = (self.sv64(pc, src0)?, self.sv64(pc, src1)?);
                let (low, high) = (
                    self.b.bin(Bo::Equal, a[0], b[0]),
                    self.b.bin(Bo::Equal, a[1], b[1]),
                );
                let equal = self.b.bin(Bo::LogicalAnd, low, high);
                match name {
                    "s_cmp_eq_u64" => equal,
                    _ => self.b.lnot(equal),
                }
            }
            "s_bitcmp0_b32" | "s_bitcmp1_b32" => {
                let (a, b) = (self.sv(pc, src0)?, self.sv(pc, src1)?);
                let shifted = self.b.shift(Bo::ShiftRight, false, a, b);
                let one = self.b.u32(1);
                let bit = self.b.bin(Bo::And, shifted, one);
                let set = self.b.ne0(bit);
                match name {
                    "s_bitcmp1_b32" => set,
                    _ => self.b.lnot(set),
                }
            }
            _ => {
                let Some(rest) = name.strip_prefix("s_cmp_") else {
                    return Err(refused(pc, inst));
                };
                let (rel, kind) = rest.split_once('_').unwrap_or((rest, ""));
                let (a, b) = (self.sv(pc, src0)?, self.sv(pc, src1)?);
                (self.b.icmp(rel, kind == "i32", a, b)).ok_or_else(|| refused(pc, inst))?
            }
        };
        self.scc(condition);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn smem(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        base: u8,
        data: Operand,
        offset: u32,
        soffset: Operand,
    ) -> Done {
        let (Some(words), Some(to)) = (sym::load_words(name), sym::register(data)) else {
            return Err(refused(pc, inst));
        };
        if name.starts_with("s_load_dword") {
            if matches!(self.state.get(usize::from(base)), Some(Sym::Code)) {
                // A pointer the code made from its own address: the words are the program's.
                let mut bytes = self.get(pc, Reg::S(base));
                let fixed = self.b.u32(offset & !3);
                bytes = self.b.bin(Bo::Add, bytes, fixed);
                if soffset != Operand::Null {
                    let by = self.sv(pc, soffset)?;
                    bytes = self.b.bin(Bo::Add, bytes, by);
                }
                let mut values = Vec::with_capacity(usize::from(words));
                for k in 0..words {
                    let step = self.b.u32(4 * u32::from(k));
                    let at = self.b.bin(Bo::Add, bytes, step);
                    values.push(self.code_word(at));
                }
                for (k, value) in (0..words).zip(values) {
                    self.put(Reg::S(to + k), value);
                }
                return Ok(());
            }
            // A table read computes nothing here: `sym` carries which words the registers hold.
            let Some(at) = sym::loaded_at(&self.state, base, offset, soffset, pc) else {
                return Err(NotTranslated::Descriptor {
                    pc,
                    what: "a table through a pointer or at an offset the program computed",
                });
            };
            // Beside each register goes its word's place in the table, in dwords, which a
            // binding array is indexed by and a word read as a number is found by.
            let mut first = self.b.u32((offset & !3) / 4);
            if at.index.is_some() {
                let by = self.sv(pc, soffset)?;
                let two = self.b.u32(2);
                let moved = self.b.bin(Bo::ShiftRight, by, two);
                first = self.b.bin(Bo::Add, first, moved);
            }
            for k in 0..words {
                let step = self.b.u32(u32::from(k));
                let place = self.b.bin(Bo::Add, first, step);
                self.put(Reg::Place(to + k), place);
                self.defined.insert(Reg::S(to + k));
            }
            if self.eager {
                for k in 0..words {
                    // A word past the entries a table is bound with stays unread.
                    if let Ok(value) = self.table_word(pc, to + k, &at.plus(4 * u32::from(k))) {
                        self.put(Reg::S(to + k), value);
                    }
                }
            }
            return Ok(());
        }
        let dynamic = match soffset {
            Operand::Null => None,
            _ => {
                let by = self.sv(pc, soffset)?;
                let mask = self.b.u32(!3);
                Some(self.b.bin(Bo::And, by, mask))
            }
        };
        let Some(at) = sym::descriptor(&self.state, base, 4) else {
            // Memory the code describes itself is read as the dwords it is.
            return self.memory_words(pc, base, offset & !3, dynamic, (words, to));
        };
        if let Some(slot) = self.entry(pc, base, &at, SLOT_DWORDS) {
            return self.table_rows(&at, slot, offset & !3, dynamic, (words, to));
        }
        let last = (offset & !3) / 4 + u32::from(words) - 1;
        let rows = match dynamic {
            Some(_) => DYNAMIC_ROWS,
            None => last / 4 + 1,
        };
        let global = match self.buffers.get_mut(&at) {
            Some((global, held)) => {
                *held = (*held).max(rows);
                *global
            }
            None => {
                let inner = self.rows(1);
                let global = self.bound(
                    format!("cb{}", self.buffers.len()),
                    inner,
                    naga::AddressSpace::Uniform,
                    ResourceKind::ConstantBuffer { rows: 1 },
                    Some(at.clone()),
                );
                self.buffers.insert(at, (global, rows));
                global
            }
        };
        for k in 0..u32::from(words) {
            let word = (offset & !3) / 4 + k;
            let pointer = self.global(global);
            let cell = match dynamic {
                None => {
                    let row = self.b.index(pointer, word / 4);
                    self.b.index(row, word % 4)
                }
                Some(by) => {
                    let (two, three, fixed) = (self.b.u32(2), self.b.u32(3), self.b.u32(word));
                    let moved = self.b.bin(Bo::ShiftRight, by, two);
                    let index = self.b.bin(Bo::Add, moved, fixed);
                    let (row, column) = (
                        self.b.bin(Bo::ShiftRight, index, two),
                        self.b.bin(Bo::And, index, three),
                    );
                    let row = self.b.emit(E::Access {
                        base: pointer,
                        index: row,
                    });
                    self.b.emit(E::Access {
                        base: row,
                        index: column,
                    })
                }
            };
            let value = self.b.load(cell);
            self.put(Reg::S(to + k as u8), value);
        }
        Ok(())
    }

    /// A constant buffer read through a descriptor at a place a run computed: one block of the
    /// table's binding array, as long as a block read at a computed offset.
    fn table_rows(
        &mut self,
        at: &Loc,
        slot: Handle<E>,
        offset: u32,
        dynamic: Option<Handle<E>>,
        (words, to): (u8, u8),
    ) -> Done {
        let key = whole(at);
        let global = match self.blocks.get(&key) {
            Some(&global) => global,
            // The table's blocks lie one after another in one buffer of dwords, as its buffers
            // do: an array of blocks indexed by a computed place is nothing Direct3D's writer takes.
            None => {
                let base = self.ty(TypeInner::Scalar(Scalar::U32));
                let inner = TypeInner::Array {
                    base,
                    size: naga::ArraySize::Dynamic,
                    stride: 4,
                };
                let space = naga::AddressSpace::Storage {
                    access: naga::StorageAccess::LOAD,
                };
                let global = self.bound(
                    format!("cbs{}", self.blocks.len()),
                    inner,
                    space,
                    ResourceKind::ConstantBuffer { rows: DYNAMIC_ROWS },
                    Some(key.clone()),
                );
                if let Some(resource) = self.resources.last_mut() {
                    resource.count = Some(TABLE_SLOTS);
                }
                self.blocks.insert(key, global);
                global
            }
        };
        for k in 0..u32::from(words) {
            let mut index = self.b.u32(offset / 4 + k);
            if let Some(by) = dynamic {
                let two = self.b.u32(2);
                let moved = self.b.bin(Bo::ShiftRight, by, two);
                index = self.b.bin(Bo::Add, index, moved);
            }
            let value = self.buffer_word(global, Some(slot), index);
            self.put(Reg::S(to + k as u8), value);
        }
        Ok(())
    }

    /// The dword of the program's own code at a byte address counted from its first word: one of
    /// the words after its last instruction, kept as a private array.
    fn code_word(&mut self, bytes: Handle<E>) -> Handle<E> {
        let global = match self.code {
            Some(global) => global,
            None => {
                let program = self.program;
                let words = (program.words.get(self.data_start as usize..)).unwrap_or_default();
                // An array holds one element at least.
                let count = (words.len() as u32).max(1);
                let base = self.ty(TypeInner::Scalar(Scalar::U32));
                let ty = self.ty(TypeInner::Array {
                    base,
                    size: naga::ArraySize::Constant(
                        NonZeroU32::new(count).unwrap_or(NonZeroU32::MIN),
                    ),
                    stride: 4,
                });
                let mut components = Vec::with_capacity(count as usize);
                for &word in words.iter().chain(words.is_empty().then_some(&0)) {
                    components.push(
                        (self.module.global_expressions)
                            .append(E::Literal(Literal::U32(word)), Span::UNDEFINED),
                    );
                }
                let init = (self.module.global_expressions)
                    .append(E::Compose { ty, components }, Span::UNDEFINED);
                let variable = naga::GlobalVariable {
                    name: Some("code_data".to_owned()),
                    space: naga::AddressSpace::Private,
                    binding: None,
                    ty,
                    init: Some(init),
                    memory_decorations: naga::MemoryDecorations::empty(),
                };
                let global = (self.module.global_variables).append(variable, Span::UNDEFINED);
                self.code = Some(global);
                global
            }
        };
        let (two, first) = (self.b.u32(2), self.b.u32(self.data_start));
        let word = self.b.bin(Bo::ShiftRight, bytes, two);
        let index = self.b.bin(Bo::Subtract, word, first);
        let pointer = self.global(global);
        let cell = self.b.emit(E::Access {
            base: pointer,
            index,
        });
        self.b.load(cell)
    }

    /// `global_load_dword*` in a program that takes its own address: words of the program itself.
    // ponytail: an address from anywhere but the code reads past the array, which no program of
    // the build was seen to do; a pointer in user data needs its own buffer.
    fn flat(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Flat {
            addr,
            saddr,
            dst,
            offset,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        // A byte or a 16-bit word is cut from the dword it lies in.
        let part = match name {
            "global_load_ubyte" => Some((8, false)),
            "global_load_sbyte" => Some((8, true)),
            "global_load_ushort" => Some((16, false)),
            "global_load_sshort" => Some((16, true)),
            _ => None,
        };
        let count = match name {
            "global_load_dword" => 1u8,
            "global_load_dwordx2" => 2,
            "global_load_dwordx3" => 3,
            "global_load_dwordx4" => 4,
            _ if part.is_some() => 1,
            _ => return Err(refused(pc, inst)),
        };
        if !self.reads_code {
            return Err(form(pc, inst, "an address that is not the code's own"));
        }
        let mut bytes = self.vreg(pc, *addr)?;
        if *saddr != Operand::Null {
            let by = self.sv(pc, *saddr)?;
            bytes = self.b.bin(Bo::Add, bytes, by);
        }
        let fixed = self.b.u32(i32::from(*offset) as u32);
        bytes = self.b.bin(Bo::Add, bytes, fixed);
        if let Some((bits, signed)) = part {
            let word = self.code_word(bytes);
            let (three, eight) = (self.b.u32(3), self.b.u32(8));
            let within = self.b.bin(Bo::And, bytes, three);
            let by = self.b.bin(Bo::Multiply, within, eight);
            let moved = self.b.bin(Bo::ShiftRight, word, by);
            let value = self.field(moved, 0, bits, signed);
            self.vput(*dst, value);
            return Ok(());
        }
        let mut values = Vec::with_capacity(usize::from(count));
        for k in 0..count {
            let step = self.b.u32(4 * u32::from(k));
            let at = self.b.bin(Bo::Add, bytes, step);
            values.push(self.code_word(at));
        }
        for (k, value) in (0..count).zip(values) {
            self.vput(dst.wrapping_add(k), value);
        }
        Ok(())
    }

    /// `bits` with the float modifiers a three-source encoding gives a source.
    fn modified(&mut self, bits: Handle<E>, abs: bool, neg: bool) -> Handle<E> {
        if !abs && !neg {
            return bits;
        }
        let mut x = self.b.float(bits);
        if abs {
            x = self.b.math(Mf::Abs, &[x]);
        }
        if neg {
            x = self.b.un(Uo::Negate, x);
        }
        self.b.bits(x)
    }

    /// A vector comparison's verdict by its mnemonic, `None` for one not lowered.
    fn compare(&mut self, name: &str, a: Handle<E>, b: Handle<E>) -> Option<Handle<E>> {
        let rest = (name.strip_prefix("v_cmpx_")).or_else(|| name.strip_prefix("v_cmp_"))?;
        let (rel, kind) = rest.rsplit_once('_')?;
        match (rel, kind) {
            ("class", "f32") => Some(self.b.class(a, b)),
            (_, "f32") => self.b.fcmp(rel, a, b),
            (_, "i32" | "u32") => self.b.icmp(rel, kind == "i32", a, b),
            // The low halves of the two patterns, read signed or not.
            (_, "i16" | "u16") => {
                let (sixteen, signed) = (self.b.u32(16), kind == "i16");
                let mut halves = [a, b];
                for half in &mut halves {
                    let up = self.b.bin(Bo::ShiftLeft, *half, sixteen);
                    *half = self.b.shift(Bo::ShiftRight, signed, up, sixteen);
                }
                self.b.icmp(rel, signed, halves[0], halves[1])
            }
            _ => None,
        }
    }

    /// A comparison's verdict where its operands may be 16-bit floats in the low halves of their
    /// patterns, which compare as the 32-bit floats they widen to.
    fn compared(&mut self, pc: u32, inst: &Inst, name: &str, a: Handle<E>, b: Handle<E>) -> Value {
        let condition = match name.ends_with("_f16") && !name.contains("_class_") {
            true => {
                let (a, b) = (self.widen(a), self.widen(b));
                self.compare(&name.replace("_f16", "_f32"), a, b)
            }
            false => self.compare(name, a, b),
        };
        condition.ok_or_else(|| refused(pc, inst))
    }

    /// A 64-bit value in a register pair moved by the low six bits of a count.
    fn wide_shift(&mut self, pc: u32, name: &str, dst: u8, srcs: [Operand; 3]) -> Done {
        let by = self.sv(pc, srcs[0])?;
        let pair = self.vv64(pc, srcs[1])?;
        let value = self.shifted64(name, by, pair);
        self.vput(dst, value[0]);
        self.vput(dst.wrapping_add(1), value[1]);
        Ok(())
    }

    /// The two dwords of a 64-bit value moved by the low six bits of a count: left where the
    /// mnemonic says so, else right, with the sign kept for an arithmetic one.
    fn shifted64(
        &mut self,
        name: &str,
        by: Handle<E>,
        [low, high]: [Handle<E>; 2],
    ) -> [Handle<E>; 2] {
        let (most, within_word) = (self.b.u32(63), self.b.u32(31));
        let (word, zero) = (self.b.u32(32), self.b.u32(0));
        let by = self.b.bin(Bo::And, by, most);
        let within = self.b.bin(Bo::And, by, within_word);
        let spare = self.b.bin(Bo::Subtract, word, within);
        let far = self.b.bin(Bo::GreaterEqual, by, word);
        let none = self.b.bin(Bo::Equal, within, zero);
        match name.contains("lshl") {
            true => {
                // The bits the low dword hands up: none where the count is a whole word's.
                let handed = self.b.shift(Bo::ShiftRight, false, low, spare);
                let handed = self.b.select(none, zero, handed);
                let near_low = self.b.bin(Bo::ShiftLeft, low, within);
                let raised = self.b.bin(Bo::ShiftLeft, high, within);
                let near_high = self.b.bin(Bo::InclusiveOr, raised, handed);
                [
                    self.b.select(far, zero, near_low),
                    self.b.select(far, near_low, near_high),
                ]
            }
            false => {
                let signed = name.contains("ashr");
                let fill = match signed {
                    true => self.b.shift(Bo::ShiftRight, true, high, within_word),
                    false => zero,
                };
                let handed = self.b.shift(Bo::ShiftLeft, false, high, spare);
                let handed = self.b.select(none, zero, handed);
                let lowered = self.b.bin(Bo::ShiftRight, low, within);
                let near_low = self.b.bin(Bo::InclusiveOr, lowered, handed);
                let near_high = self.b.shift(Bo::ShiftRight, signed, high, within);
                [
                    self.b.select(far, near_high, near_low),
                    self.b.select(far, fill, near_high),
                ]
            }
        }
    }

    /// Writes a comparison where its encoding says: the lane mask for a `cmpx`, else `dst`.
    fn put_compare(&mut self, name: &str, dst: u8, condition: Handle<E>) {
        match name.starts_with("v_cmpx") {
            true => self.narrow_exec(condition),
            false => self.put_mask(dst, condition),
        }
    }

    /// The derivative a subtraction of two neighbouring lanes' values is, `None` where neither operand is one.
    fn derivative(
        &mut self,
        pc: u32,
        _inst: &Inst,
        left: Operand,
        right: Operand,
    ) -> Result<Option<Handle<E>>, NotTranslated> {
        let tag = |cx: &Self, operand: Operand| match operand {
            Operand::Vgpr(n) => cx.permuted.get(&n).copied(),
            _ => None,
        };
        // Any other pair subtracts the values the registers hold, rebuilt from this lane's derivatives.
        let (Some(a), Some(b)) = (tag(self, left), tag(self, right)) else {
            return Ok(None);
        };
        let current = self.versions.get(&a.source).copied().unwrap_or(0);
        if a.source != b.source || a.version != b.version || a.version != current {
            return Ok(None);
        }
        let (axis, ctrl, negate) = match (a.quad, b.quad) {
            ([1, 1, 1, 1], [0, 0, 0, 0]) => (DerivativeAxis::X, DerivativeControl::Coarse, false),
            ([0, 0, 0, 0], [1, 1, 1, 1]) => (DerivativeAxis::X, DerivativeControl::Coarse, true),
            ([2, 2, 2, 2], [0, 0, 0, 0]) => (DerivativeAxis::Y, DerivativeControl::Coarse, false),
            ([0, 0, 0, 0], [2, 2, 2, 2]) => (DerivativeAxis::Y, DerivativeControl::Coarse, true),
            ([1, 1, 3, 3], [0, 0, 2, 2]) => (DerivativeAxis::X, DerivativeControl::Fine, false),
            ([0, 0, 2, 2], [1, 1, 3, 3]) => (DerivativeAxis::X, DerivativeControl::Fine, true),
            ([2, 3, 2, 3], [0, 1, 0, 1]) => (DerivativeAxis::Y, DerivativeControl::Fine, false),
            ([0, 1, 0, 1], [2, 3, 2, 3]) => (DerivativeAxis::Y, DerivativeControl::Fine, true),
            _ => return Ok(None),
        };
        // Outside a pixel program the two registers hold the lanes' own values, which subtract as they are.
        if !self.fragment {
            return Ok(None);
        }
        let source = self.get(pc, Reg::V(a.source));
        let expr = self.b.float(source);
        let mut value = self.b.emit(E::Derivative { axis, ctrl, expr });
        if negate {
            value = self.b.un(Uo::Negate, value);
        }
        Ok(Some(self.b.bits(value)))
    }

    /// `source` as the lane of its quad that `quad` gives this lane computed it: this lane's value
    /// plus its derivative toward that lane.
    // ponytail: exact for the neighbour in this lane's row or column; the diagonal one takes this
    // lane's two derivatives, which misses how the row's derivative changes down the quad.
    fn neighbour(&mut self, pc: u32, inst: &Inst, source: u8, quad: Quad) -> Value {
        if !self.subgroups && !self.fragment {
            return Err(form(
                pc,
                inst,
                "another thread's value in a program run a thread at a time",
            ));
        }
        let bits = self.vreg(pc, source)?;
        if !self.fragment {
            // A compute wave's lanes are threads that all run: the lane's own value is read.
            let lane = self.lane_number();
            let (three, rest) = (self.b.u32(3), self.b.u32(!3));
            let within = self.b.bin(Bo::And, lane, three);
            let mut from = self.b.u32(u32::from(quad[0]));
            for (number, &read) in quad.iter().enumerate().skip(1) {
                let (number, read) = (self.b.u32(number as u32), self.b.u32(u32::from(read)));
                let this = self.b.bin(Bo::Equal, within, number);
                from = self.b.select(this, read, from);
            }
            let base = self.b.bin(Bo::And, lane, rest);
            let index = self.b.bin(Bo::InclusiveOr, base, from);
            return Ok(self.gather(bits, index));
        }
        let expr = self.b.float(bits);
        // This lane's number in its quad: its column's parity, then its row's.
        let position = self.position();
        let one = self.b.u32(1);
        let mut parity = [one; 2];
        for (axis, bit) in parity.iter_mut().enumerate() {
            let along = self.b.index(position, axis as u32);
            let whole = self.b.convert(along, ScalarKind::Uint);
            *bit = self.b.bin(Bo::And, whole, one);
        }
        let row = self.b.bin(Bo::ShiftLeft, parity[1], one);
        let lane = self.b.bin(Bo::InclusiveOr, parity[0], row);
        let mut from = self.b.u32(u32::from(quad[0]));
        for (number, &read) in quad.iter().enumerate().skip(1) {
            let (number, read) = (self.b.u32(number as u32), self.b.u32(u32::from(read)));
            let this = self.b.bin(Bo::Equal, lane, number);
            from = self.b.select(this, read, from);
        }
        let there = [
            self.b.bin(Bo::And, from, one),
            self.b.bin(Bo::ShiftRight, from, one),
        ];
        let mut value = expr;
        for (k, axis) in [DerivativeAxis::X, DerivativeAxis::Y]
            .into_iter()
            .enumerate()
        {
            let ctrl = DerivativeControl::Fine;
            let slope = self.b.emit(E::Derivative { axis, ctrl, expr });
            let (to, here) = (self.b.sint(there[k]), self.b.sint(parity[k]));
            let steps = self.b.bin(Bo::Subtract, to, here);
            let steps = self.b.convert(steps, ScalarKind::Float);
            let change = self.b.bin(Bo::Multiply, steps, slope);
            value = self.b.bin(Bo::Add, value, change);
        }
        Ok(self.b.bits(value))
    }

    /// Writes `dst` with a neighbouring lane's value of `source`, and marks it so for the
    /// subtraction that makes an exact derivative of two of them.
    fn permute(&mut self, pc: u32, inst: &Inst, dst: u8, source: u8, quad: Quad) -> Done {
        let version = self.versions.get(&source).copied().unwrap_or(0);
        let value = self.neighbour(pc, inst, source, quad)?;
        self.vput(dst, value);
        self.permuted.insert(
            dst,
            Permuted {
                source,
                version,
                quad,
            },
        );
        Ok(())
    }

    /// Writes `dst` as another lane's value of `source`, which only a step of a fold over the wave reads.
    fn foreign(&mut self, pc: u32, dst: u8, source: u8) -> Done {
        let value = self.vreg(pc, source)?;
        self.vput(dst, value);
        self.waved
            .insert(Reg::V(dst), Waved::Foreign(Reg::V(source)));
        Ok(())
    }

    /// `ds_swizzle_b32` by bits: each lane reads the lane of its 32 whose number is its own
    /// masked, then set, then flipped by the pattern's three five-bit fields.
    fn swizzle_bits(&mut self, pc: u32, inst: &Inst, dst: u8, source: u8, pattern: u32) -> Done {
        if self.mesh.is_some() {
            return self.mesh_swizzle(pc, inst, dst, source);
        }
        if !self.subgroups {
            return Err(form(
                pc,
                inst,
                "another thread's value in a program run a thread at a time",
            ));
        }
        let bits = self.vreg(pc, source)?;
        let lane = self.lane_number();
        let (and, or, xor) = (
            self.b.u32(pattern & 0x1F),
            self.b.u32((pattern >> 5) & 0x1F),
            self.b.u32((pattern >> 10) & 0x1F),
        );
        let (low, rest) = (self.b.u32(31), self.b.u32(!31));
        let within = self.b.bin(Bo::And, lane, low);
        let kept = self.b.bin(Bo::And, within, and);
        let set = self.b.bin(Bo::InclusiveOr, kept, or);
        let flipped = self.b.bin(Bo::ExclusiveOr, set, xor);
        let base = self.b.bin(Bo::And, lane, rest);
        let index = self.b.bin(Bo::InclusiveOr, base, flipped);
        let value = self.gather(bits, index);
        self.vput(dst, value);
        Ok(())
    }

    fn vop1(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        src: Operand,
        extra: Option<u32>,
    ) -> Done {
        match (name, src, extra) {
            ("v_nop", _, _) => {}
            // The two registers change places.
            ("v_swap_b32", Operand::Vgpr(other), None) => {
                let (a, b) = (self.vreg(pc, dst)?, self.vreg(pc, other)?);
                self.vput(dst, b);
                self.vput(other, a);
            }
            // A table's word at a place a run computed moves as that place.
            ("v_mov_b32", _, None) if self.computed_place(pc, src).is_some() => {
                if let Some(place) = self.computed_place(pc, src) {
                    self.vput(dst, place);
                }
            }
            // An array kept in registers, read and written at the index the `m0` register holds.
            // ponytail: an array is taken as no longer than [`REGISTER_ARRAY`] registers.
            ("v_movrels_b32", Operand::Vgpr(first), None) => {
                let index = self.get(pc, Reg::S(sym::M0));
                let mut value = self.raw(Reg::V(first));
                for k in 1..REGISTER_ARRAY.min(u8::MAX - first) {
                    let (number, held) = (self.b.u32(u32::from(k)), self.raw(Reg::V(first + k)));
                    let this = self.b.bin(Bo::Equal, index, number);
                    value = self.b.select(this, held, value);
                }
                self.vput(dst, value);
            }
            ("v_movreld_b32", _, None) => {
                let (value, index) = (self.sv(pc, src)?, self.get(pc, Reg::S(sym::M0)));
                let active = self.masked.then(|| self.active());
                let mut cases = Vec::new();
                for k in 0..REGISTER_ARRAY.min(u8::MAX - dst) {
                    let (register, pointer) = (Reg::V(dst + k), self.local(Reg::V(dst + k)));
                    let mut body = Block::new();
                    body.push(Statement::Store { pointer, value }, Span::UNDEFINED);
                    cases.push(naga::SwitchCase {
                        value: naga::SwitchValue::U32(u32::from(k)),
                        body,
                        fall_through: false,
                    });
                    self.maybe.insert(register);
                    *self.versions.entry(dst + k).or_default() += 1;
                    self.permuted.remove(&(dst + k));
                    self.waved.remove(&register);
                }
                cases.push(naga::SwitchCase {
                    value: naga::SwitchValue::Default,
                    body: Block::new(),
                    fall_through: false,
                });
                let store = Statement::Switch {
                    selector: index,
                    cases,
                };
                match active {
                    Some(condition) => {
                        let mut accept = Block::new();
                        accept.push(store, Span::UNDEFINED);
                        self.b.push(Statement::If {
                            condition,
                            accept,
                            reject: Block::new(),
                        });
                    }
                    None => self.b.push(store),
                }
            }
            ("v_readfirstlane_b32", _, _) => {
                // ponytail: this lane stands for the first active one, which is exact where the value is the same in every lane.
                let value = self.sv(pc, src)?;
                let first = match self.shared_wave.threads {
                    Some(_) => self.shared_first(value),
                    None => None,
                };
                let value = match first {
                    Some(first) => first,
                    None => {
                        self.note("the first lane read as this lane");
                        value
                    }
                };
                let moved = match src {
                    Operand::Vgpr(n) => self.state.moved(n).cloned(),
                    _ => None,
                };
                match moved {
                    Some(Sym::Word(at)) if at.index.is_some() => {
                        self.put_place(pc, dst, value, &at)
                    }
                    _ => self.put(Reg::S(dst), value),
                }
            }
            // Another lane's value by a pattern that is no quad's: the first half of a fold's
            // step, whose second half joins it with this lane's.
            ("v_mov_b32", Operand::Dpp, Some(word)) if (word >> 8) & 0x1FF > 0xFF => {
                self.foreign(pc, dst, word as u8)?;
            }
            ("v_mov_b32", Operand::Dpp8 | Operand::Dpp8Fi, Some(word)) => {
                self.foreign(pc, dst, word as u8)?;
            }
            ("v_mov_b32", Operand::Dpp, Some(word)) => {
                // The control word: the source register, a nine-bit control, then the bank and row masks.
                let (source, control, masks) = (word as u8, word >> 8 & 0x1FF, word >> 24);
                if control > 0xFF || masks != 0xFF {
                    return Err(form(
                        pc,
                        inst,
                        "a lane permutation that is not a whole quad's",
                    ));
                }
                let quad = [0, 2, 4, 6].map(|low| (control >> low & 3) as u8);
                self.permute(pc, inst, dst, source, quad)?;
            }
            (_, Operand::Sdwa, Some(word)) => {
                half_parts(pc, inst, name, word)?;
                let (a, _) = self.sdwa_sources(pc, inst, word, None)?;
                let value = self.unary(pc, inst, name, a)?;
                self.sdwa_put(pc, inst, word, dst, value)?;
            }
            _ => {
                let a = match src {
                    Operand::Float(word) if is_half(name) => self.b.u32(half_word(word)),
                    _ => self.sv(pc, src)?,
                };
                let value = self.unary(pc, inst, name, a)?;
                match is_half(name) || name == "v_cvt_f16_f32" {
                    true => self.vput_half(dst, value),
                    false => self.vput(dst, value),
                }
            }
        }
        Ok(())
    }

    /// What a one-source instruction writes from its source's pattern; a 16-bit float is in the
    /// low half of its register.
    fn unary(&mut self, pc: u32, inst: &Inst, name: &str, a: Handle<E>) -> Value {
        Ok(match name {
            "v_cvt_f16_f32" => self.narrow(a),
            "v_cvt_f32_f16" => self.widen(a),
            _ if is_half(name) => {
                let wide = self.widen(a);
                (self.half_alu(name, &[wide])).ok_or_else(|| refused(pc, inst))?
            }
            _ => self.b.alu(name, &[a]).ok_or_else(|| refused(pc, inst))?,
        })
    }

    /// A float pattern as a 16-bit float in the low half of a dword, the high half zero:
    /// the nearest, or the one toward zero where the program has set that rounding.
    fn narrow(&mut self, bits: Handle<E>) -> Handle<E> {
        let rounding = self.half_rounding();
        self.b.half_of(bits, rounding)
    }

    /// How an instruction's 16-bit result is rounded where the program stands: to the nearest,
    /// unless the program writes its float mode, which then says as it runs.
    fn half_rounding(&mut self) -> Rounding {
        match self.half_mode {
            Some(global) => {
                let pointer = self.global(global);
                Rounding::By(self.b.load(pointer))
            }
            None => Rounding::Nearest,
        }
    }

    /// The 16-bit float in the low half of a dword as a 32-bit float's pattern, which holds it exactly.
    fn widen(&mut self, bits: Handle<E>) -> Handle<E> {
        self.b.float_of_half(bits)
    }

    /// Writes the low half of a vector register and keeps its high half, as an instruction
    /// whose result is one 16-bit float does.
    fn vput_half(&mut self, n: u8, half: Handle<E>) {
        let (old, high) = (self.raw(Reg::V(n)), self.b.u32(0xFFFF_0000));
        let kept = self.b.bin(Bo::And, old, high);
        let value = self.b.bin(Bo::InclusiveOr, kept, half);
        self.vput(n, value);
    }

    /// A 16-bit instruction's result narrowed: a function's is rounded to the nearest whatever
    /// the float mode says, a sum's or a product's as the mode says.
    fn narrow_as(&mut self, name: &str, bits: Handle<E>) -> Handle<E> {
        match half_function(name) {
            true => self.b.half_of(bits, Rounding::Nearest),
            false => self.narrow(bits),
        }
    }

    /// Counts a 16-bit result that is not the hardware's on every input: a fused multiply-add
    /// is rounded to 32 bits before 16, and a function is as exact as the device's 32-bit one.
    fn note_half(&mut self, name: &str) {
        let kind = name.trim_start_matches("v_pk_").trim_start_matches("v_");
        let kind = kind.split('_').next().unwrap_or(kind);
        match kind {
            "fma" | "fmac" | "mac" => {
                self.note("a 16-bit fused multiply-add rounded at 32 bits before 16");
            }
            _ if half_function(name) => {
                self.note("a 16-bit float function as exact as the device's 32-bit one");
            }
            _ => {}
        }
    }

    /// A 16-bit float source widened: an inline float constant is that value as a half.
    fn widened(&mut self, pc: u32, operand: Operand) -> Value {
        let bits = match operand {
            Operand::Float(word) => self.b.u32(half_word(word)),
            _ => self.sv(pc, operand)?,
        };
        Ok(self.widen(bits))
    }

    /// A 16-bit float instruction through its 32-bit form, over sources already widened: the
    /// result is rounded once more to a half, which changes nothing for a sum, a product, a
    /// quotient or a root, since 24 bits are two more than twice a half's eleven.
    fn half_alu(&mut self, name: &str, widened: &[Handle<E>]) -> Option<Handle<E>> {
        let wide = format!("{}_f32", name.strip_suffix("_f16")?);
        let value = self.b.alu(&wide, widened)?;
        self.note_half(name);
        Some(self.narrow_as(name, value))
    }

    /// A two-source instruction over 16-bit floats, in the low halves of its registers or in the
    /// parts a trailing word selects.
    #[allow(clippy::too_many_arguments)]
    fn vop2_half(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        src0: Operand,
        src1: u8,
        extra: Option<u32>,
    ) -> Done {
        let parts = match (src0, extra) {
            (Operand::Sdwa, Some(word)) => Some(word),
            _ => None,
        };
        let (a, b) = match parts {
            Some(word) => {
                half_parts(pc, inst, name, word)?;
                let (a, b) = self.sdwa_sources(pc, inst, word, Some(src1))?;
                let b = b.ok_or_else(|| refused(pc, inst))?;
                (self.widen(a), self.widen(b))
            }
            None => {
                let a = self.widened(pc, src0)?;
                let b = self.vreg(pc, src1)?;
                (a, self.widen(b))
            }
        };
        let value = match name {
            // The product is added to what the destination holds.
            "v_mac_f16" | "v_fmac_f16" => {
                let held = self.vreg(pc, dst)?;
                let c = self.widen(held);
                let sum = match name {
                    "v_mac_f16" => self.b.mad(false, a, b, c),
                    _ => self.b.fmath(Mf::Fma, &[a, b, c]),
                };
                self.note_half(name);
                self.narrow(sum)
            }
            _ => (self.half_alu(name, &[a, b])).ok_or_else(|| refused(pc, inst))?,
        };
        match parts {
            Some(word) => self.sdwa_put(pc, inst, word, dst, value),
            None => {
                self.vput_half(dst, value);
                Ok(())
            }
        }
    }

    /// A three-source encoding over 16-bit floats: arithmetic, a comparison, or a conversion to
    /// or from a 32-bit float.
    #[allow(clippy::too_many_arguments)]
    fn vop3_half(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        srcs: [Operand; 3],
        (abs, neg, opsel): (u8, u8, u8),
        (clamp, omod): (bool, u8),
        count: usize,
    ) -> Done {
        let compare = name.starts_with("v_cmp");
        let scaled = omod != 0 && name != "v_cvt_f32_f16";
        if (!compare && opsel != 0) || scaled || name.contains("_class_") {
            return Err(form(
                pc,
                inst,
                "a half selection, a scale or a class on a 16-bit float",
            ));
        }
        let mut values = Vec::with_capacity(count);
        for (i, src) in srcs.iter().take(count).enumerate() {
            let bits = match name {
                "v_cvt_f16_f32" => self.sv(pc, *src)?,
                _ => self.widened(pc, *src)?,
            };
            values.push(self.modified(bits, (abs >> i) & 1 != 0, (neg >> i) & 1 != 0));
        }
        let wide = name.replace("_f16", "_f32");
        if compare {
            let condition = match values.as_slice() {
                &[a, b] => self.compare(&wide, a, b),
                _ => None,
            };
            let condition = condition.ok_or_else(|| refused(pc, inst))?;
            self.put_compare(name, dst & 0x7F, condition);
            return Ok(());
        }
        let mut value = match (name, values.first()) {
            // A conversion's source is already a 32-bit pattern, whichever way it goes.
            ("v_cvt_f16_f32" | "v_cvt_f32_f16", Some(&only)) => only,
            _ => (self.b.alu(&wide, &values)).ok_or_else(|| refused(pc, inst))?,
        };
        // A scale by two, four or a half is exact in 32 bits, and comes before the clamp.
        value = self.output_modified(value, clamp, omod);
        if name == "v_cvt_f32_f16" {
            self.vput(dst, value);
            return Ok(());
        }
        if name != "v_cvt_f16_f32" {
            self.note_half(name);
        }
        let half = self.narrow_as(name, value);
        self.vput_half(dst, half);
        Ok(())
    }

    /// Two float patterns as the two 16-bit halves of a dword, the first in the low half.
    fn pack_halves(&mut self, low: Handle<E>, high: Handle<E>, rounding: Rounding) -> Handle<E> {
        let (low, high) = (
            self.b.half_of(low, rounding),
            self.b.half_of(high, rounding),
        );
        let sixteen = self.b.u32(16);
        let raised = self.b.bin(Bo::ShiftLeft, high, sixteen);
        self.b.bin(Bo::InclusiveOr, low, raised)
    }

    /// One 16-bit half of a dword as a float.
    fn half(&mut self, bits: Handle<E>, high: bool) -> Handle<E> {
        let bits = match high {
            true => {
                let sixteen = self.b.u32(16);
                self.b.bin(Bo::ShiftRight, bits, sixteen)
            }
            false => bits,
        };
        let wide = self.b.float_of_half(bits);
        self.b.float(wide)
    }

    #[allow(clippy::too_many_arguments)]
    fn vop2(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        src0: Operand,
        src1: u8,
        extra: Option<u32>,
    ) -> Done {
        if matches!(name, "v_sub_f32" | "v_subrev_f32") {
            let (left, right) = match name {
                "v_sub_f32" => (src0, Operand::Vgpr(src1)),
                _ => (Operand::Vgpr(src1), src0),
            };
            if let Some(value) = self.derivative(pc, inst, left, right)? {
                self.vput(dst, value);
                return Ok(());
            }
        }
        // Another lane's value folded into the instruction, by a pattern that is no quad's: a
        // step of a fold over the wave.
        let quad =
            matches!((src0, extra), (Operand::Dpp, Some(word)) if (word >> 8) & 0x1FF <= 0xFF);
        if let (Operand::Dpp | Operand::Dpp8 | Operand::Dpp8Fi, false, Some(word)) =
            (src0, quad, extra)
        {
            return self.fold_step(pc, inst, name, dst, (src0, src1), word);
        }
        if let Some(acc) = self.fold_pair(src0, Operand::Vgpr(src1)) {
            return self.fold_join(pc, inst, name, dst, acc);
        }
        if is_half(name) {
            return self.vop2_half(pc, inst, name, dst, src0, src1, extra);
        }
        if let (Operand::Sdwa, Some(word)) = (src0, extra) {
            let (a, b) = self.sdwa_sources(pc, inst, word, Some(src1))?;
            let value =
                (b.and_then(|b| self.b.alu(name, &[a, b]))).ok_or_else(|| refused(pc, inst))?;
            return self.sdwa_put(pc, inst, word, dst, value);
        }
        let a = match (src0, extra) {
            // A lane permutation folded into the instruction: its first source is a neighbour's value.
            (Operand::Dpp, Some(word)) => {
                let (source, control) = (word as u8, (word >> 8) & 0x1FF);
                if control > 0xFF || word >> 20 != 0xFF0 {
                    return Err(form(
                        pc,
                        inst,
                        "a lane permutation that is not a whole quad's",
                    ));
                }
                let quad = [0, 2, 4, 6].map(|low| ((control >> low) & 3) as u8);
                if self.fragment {
                    self.note(REBUILT);
                }
                self.neighbour(pc, inst, source, quad)?
            }
            _ => self.sv(pc, src0)?,
        };
        let b = self.vreg(pc, src1)?;
        if name.contains("_co_") {
            // The two-source encoding carries through the comparison mask.
            let carry = match name.contains("addc") || name.contains("subb") {
                true => Some(self.mask_bit(pc, VCC)?),
                false => None,
            };
            let (value, out) =
                (self.b.carried(name, a, b, carry)).ok_or_else(|| refused(pc, inst))?;
            self.vput(dst, value);
            self.put_mask(VCC, out);
            return Ok(());
        }
        let constant = match (src0.takes_word(), extra) {
            (false, Some(word)) => Some(self.b.u32(word)),
            _ => None,
        };
        let value = match (name, constant) {
            ("v_cndmask_b32", _) => {
                let chosen = self.mask_bit(pc, VCC)?;
                self.b.select(chosen, b, a)
            }
            ("v_mac_f32" | "v_mac_legacy_f32", _) => {
                let c = self.vreg(pc, dst)?;
                self.b.mad(name == "v_mac_legacy_f32", a, b, c)
            }
            ("v_fmac_f32", _) => {
                let c = self.vreg(pc, dst)?;
                self.b.fmath(Mf::Fma, &[a, b, c])
            }
            ("v_madmk_f32", Some(k)) => self.b.mad(false, a, k, b),
            ("v_madak_f32", Some(k)) => self.b.mad(false, a, b, k),
            ("v_fmamk_f32", Some(k)) => self.b.fmath(Mf::Fma, &[a, k, b]),
            ("v_fmaak_f32", Some(k)) => self.b.fmath(Mf::Fma, &[a, b, k]),
            ("v_cvt_pkrtz_f16_f32", _) => self.pack_halves(a, b, Rounding::TowardZero),
            _ => (self.b.alu(name, &[a, b])).ok_or_else(|| refused(pc, inst))?,
        };
        self.vput(dst, value);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn vop3(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        dst: u8,
        srcs: [Operand; 3],
        (abs, neg, opsel): (u8, u8, u8),
        (clamp, omod): (bool, u8),
        count: usize,
    ) -> Done {
        // A select takes the float modifiers too: it is how a negated or absolute value is chosen.
        let float_in = name.ends_with("_f32") || name == "v_cndmask_b32";
        let float_out =
            (name.ends_with("_f32") && !name.starts_with("v_cvt_") && !name.starts_with("v_cmp"))
                || name.starts_with("v_cvt_f32_");
        let compare = name.starts_with("v_cmp");
        if matches!(name, "v_readlane_b32" | "v_writelane_b32") {
            return self.lane(pc, inst, name, dst, srcs);
        }
        // The other row's value of a register: a step of a fold over the wave reads it.
        if name == "v_permlanex16_b32" {
            let Operand::Vgpr(source) = srcs[0] else {
                return Err(form(pc, inst, "a source that is no vector register"));
            };
            return self.foreign(pc, dst, source);
        }
        if count >= 2 {
            if let Some(acc) = self.fold_pair(srcs[0], srcs[1]) {
                return self.fold_join(pc, inst, name, dst, acc);
            }
        }
        if name.contains("_co_") {
            // The carry leaves in the mask register the modifier bits name, and comes in the third source.
            let (a, b) = (self.sv(pc, srcs[0])?, self.sv(pc, srcs[1])?);
            let carry = match (count, sym::register(srcs[2])) {
                (3, Some(mask)) => Some(self.mask_bit(pc, mask)?),
                (3, None) => return Err(form(pc, inst, "a carry that is no mask register")),
                _ => None,
            };
            let (value, out) =
                (self.b.carried(name, a, b, carry)).ok_or_else(|| refused(pc, inst))?;
            self.vput(dst, value);
            self.put_mask(((opsel << 3) | abs) & 0x7F, out);
            return Ok(());
        }
        if compare && (name.ends_with("_u64") || name.ends_with("_i64")) {
            let condition = self.wide_compare(pc, inst, name, srcs[0], srcs[1])?;
            self.put_compare(name, dst & 0x7F, condition);
            return Ok(());
        }
        if matches!(name, "v_mbcnt_lo_u32_b32" | "v_mbcnt_hi_u32_b32") {
            return self.mbcnt(pc, inst, name, dst, srcs);
        }
        if name == "v_pack_b32_f16" {
            if clamp || omod != 0 {
                return Err(form(pc, inst, "an output modifier on packed halves"));
            }
            return self.pack_b32(pc, dst, srcs, (abs, neg, opsel));
        }
        if matches!(name, "v_lshlrev_b64" | "v_lshrrev_b64" | "v_ashrrev_i64") {
            return self.wide_shift(pc, name, dst, srcs);
        }
        if name.ends_with("_f16") || name == "v_cvt_f16_f32" {
            let modifiers = ((abs, neg, opsel), (clamp, omod));
            return self.vop3_half(pc, inst, name, dst, srcs, modifiers.0, modifiers.1, count);
        }
        if !compare && opsel != 0 {
            return Err(form(pc, inst, "a half selection"));
        }
        if (omod != 0 || clamp) && !float_out {
            return Err(form(pc, inst, "an output modifier on an integer result"));
        }
        if (abs != 0 || neg != 0) && !float_in {
            return Err(form(pc, inst, "a float modifier on an integer operand"));
        }
        if matches!(name, "v_sub_f32" | "v_subrev_f32") && abs == 0 && neg == 0 {
            let (left, right) = match name {
                "v_sub_f32" => (srcs[0], srcs[1]),
                _ => (srcs[1], srcs[0]),
            };
            if let Some(value) = self.derivative(pc, inst, left, right)? {
                self.vput(dst, value);
                return Ok(());
            }
        }
        let mut values = Vec::with_capacity(count);
        for (i, src) in srcs.iter().take(count).enumerate() {
            let bits = self.sv(pc, *src)?;
            values.push(self.modified(bits, abs >> i & 1 != 0, neg >> i & 1 != 0));
        }
        if compare {
            let condition = match values.as_slice() {
                &[a, b] => self.compare(name, a, b),
                _ => None,
            };
            let condition = condition.ok_or_else(|| refused(pc, inst))?;
            self.put_compare(name, dst & 0x7F, condition);
            return Ok(());
        }
        let value = match (name, values.as_slice()) {
            ("v_cndmask_b32", &[a, b, _]) => {
                let mask = sym::register(srcs[2]).ok_or_else(|| refused(pc, inst))?;
                let chosen = self.mask_bit(pc, mask)?;
                self.b.select(chosen, b, a)
            }
            ("v_mac_f32" | "v_mac_legacy_f32", &[a, b]) => {
                let c = self.vreg(pc, dst)?;
                self.b.mad(name == "v_mac_legacy_f32", a, b, c)
            }
            ("v_fmac_f32", &[a, b]) => {
                let c = self.vreg(pc, dst)?;
                self.b.fmath(Mf::Fma, &[a, b, c])
            }
            ("v_cvt_pkrtz_f16_f32", &[a, b]) => self.pack_halves(a, b, Rounding::TowardZero),
            (_, values) => (self.b.alu(name, values)).ok_or_else(|| refused(pc, inst))?,
        };
        let value = self.output_modified(value, clamp, omod);
        self.vput(dst, value);
        Ok(())
    }

    /// A float result scaled by two, four or a half, then clamped to zero and one.
    fn output_modified(&mut self, value: Handle<E>, clamp: bool, omod: u8) -> Handle<E> {
        let mut value = value;
        if omod != 0 {
            let by = self.b.f32([2.0, 4.0, 0.5][usize::from(omod - 1) % 3]);
            let x = self.b.float(value);
            let scaled = self.b.bin(Bo::Multiply, x, by);
            value = self.b.bits(scaled);
        }
        if clamp {
            value = self.b.fmath(Mf::Saturate, &[value]);
        }
        value
    }

    /// `v_writelane_b32` and `v_readlane_b32` with a constant lane: how a program keeps a scalar
    /// register in a vector one. A lane nothing wrote that way is read as this lane's own value.
    fn lane(&mut self, pc: u32, inst: &Inst, name: &str, dst: u8, srcs: [Operand; 3]) -> Done {
        let Operand::Int(lane) = srcs[1] else {
            // A lane the program computes is the first that runs of some mask, which here is
            // this one, unless the register is one the program keeps scalars in.
            let from = match (name, srcs[0]) {
                ("v_readlane_b32", Operand::Vgpr(from)) => Some(from),
                _ => None,
            };
            let kept = |slot: &Reg| matches!(slot, Reg::Spill(n, _) if Some(*n) == from);
            let Some(from) = from.filter(|_| !self.spills.iter().any(kept)) else {
                return Err(form(pc, inst, "a lane the program computes"));
            };
            let mut value = self.vreg(pc, from)?;
            match self.subgroups {
                true => {
                    let index = self.sv(pc, srcs[1])?;
                    value = self.gather(value, index);
                }
                false => self.note("a lane read as this lane"),
            }
            self.put(Reg::S(dst & 0x7F), value);
            return Ok(());
        };
        let lane = lane as u8;
        // A fold's result is the same in every lane it reaches. Read at a lane other than the
        // first it is a step of the fold still, which joins the wave's two halves.
        if let ("v_readlane_b32", Operand::Vgpr(from)) = (name, srcs[0]) {
            if self.waved.get(&Reg::V(from)) == Some(&Waved::Folded) {
                let (value, to) = (self.vreg(pc, from)?, Reg::S(dst & 0x7F));
                self.put(to, value);
                if lane != 0 {
                    self.waved.insert(to, Waved::Foreign(Reg::V(from)));
                }
                return Ok(());
            }
        }
        if name == "v_writelane_b32" {
            let value = match self.computed_place(pc, srcs[0]) {
                Some(place) => place,
                None => self.sv(pc, srcs[0])?,
            };
            self.put(Reg::Spill(dst, lane), value);
            return Ok(());
        }
        let Operand::Vgpr(from) = srcs[0] else {
            return Err(form(pc, inst, "a source that is no vector register"));
        };
        // A table's word comes back as `sym` knows it: the register names it, and holds nothing.
        match self.state.spilled(from, lane).cloned() {
            Some(Sym::Word(at)) if at.index.is_some() => {
                let place = self.get(pc, Reg::Spill(from, lane));
                self.put_place(pc, dst & 0x7F, place, &at);
                return Ok(());
            }
            Some(Sym::Word(_)) => {
                self.defined.insert(Reg::S(dst & 0x7F));
                return Ok(());
            }
            _ => {}
        }
        let slot = Reg::Spill(from, lane);
        let value = match self.spills.contains(&slot) {
            true => self.get(pc, slot),
            false if self.shared_wave.threads.is_some() => {
                let (value, lane) = (self.vreg(pc, from)?, self.b.u32(u32::from(lane)));
                self.gather(value, lane)
            }
            false => {
                // ponytail: exact where the value is the same in every lane, as `v_readfirstlane_b32` is.
                self.note("a lane read as this lane");
                self.vreg(pc, from)?
            }
        };
        self.put(Reg::S(dst & 0x7F), value);
        Ok(())
    }

    /// A vector source as a 64-bit value's two dwords: a register and the next, or a constant extended.
    fn vv64(&mut self, pc: u32, operand: Operand) -> Result<[Handle<E>; 2], NotTranslated> {
        match operand {
            Operand::Vgpr(n) => Ok([self.vreg(pc, n)?, self.vreg(pc, n.wrapping_add(1))?]),
            _ => self.sv64(pc, operand),
        }
    }

    /// A comparison of two 64-bit values: the high dwords decide, the low ones where those are equal.
    fn wide_compare(&mut self, pc: u32, inst: &Inst, name: &str, a: Operand, b: Operand) -> Value {
        let rest = (name.strip_prefix("v_cmpx_")).or_else(|| name.strip_prefix("v_cmp_"));
        let Some((rel, kind)) = rest.and_then(|rest| rest.rsplit_once('_')) else {
            return Err(refused(pc, inst));
        };
        let ([al, ah], [bl, bh]) = (self.vv64(pc, a)?, self.vv64(pc, b)?);
        let (low_equal, high_equal) =
            (self.b.bin(Bo::Equal, al, bl), self.b.bin(Bo::Equal, ah, bh));
        let equal = self.b.bin(Bo::LogicalAnd, low_equal, high_equal);
        Ok(match rel {
            "eq" => equal,
            "lg" | "ne" => self.b.lnot(equal),
            "lt" | "le" | "gt" | "ge" => {
                let strict = if rel.starts_with('l') { "lt" } else { "gt" };
                let high = self.b.icmp(strict, kind == "i64", ah, bh);
                let low = self.b.icmp(rel, false, al, bl);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(refused(pc, inst));
                };
                let tied = self.b.bin(Bo::LogicalAnd, high_equal, low);
                self.b.bin(Bo::LogicalOr, high, tied)
            }
            _ => return Err(refused(pc, inst)),
        })
    }

    /// `v_fma_mix_f32`: a fused multiply-add whose every source is a whole float or a 16-bit half.
    #[allow(clippy::too_many_arguments)]
    fn fma_mix(
        &mut self,
        pc: u32,
        name: &str,
        dst: u8,
        srcs: [Operand; 3],
        (half_high, halved): (u8, u8),
        (neg, abs): (u8, u8),
        clamp: bool,
    ) -> Done {
        let mut values = Vec::with_capacity(3);
        for (i, src) in srcs.iter().enumerate() {
            let as_half = (halved >> i) & 1 != 0;
            let mut x = match (as_half, *src) {
                // An inline float constant read as a half is that value as a half.
                (true, Operand::Float(word)) => {
                    let bits = self.b.u32(half_word(word));
                    self.half(bits, false)
                }
                (true, _) => {
                    let bits = self.sv(pc, *src)?;
                    self.half(bits, (half_high >> i) & 1 != 0)
                }
                (false, _) => {
                    let bits = self.sv(pc, *src)?;
                    self.b.float(bits)
                }
            };
            if abs >> i & 1 != 0 {
                x = self.b.math(Mf::Abs, &[x]);
            }
            if neg >> i & 1 != 0 {
                x = self.b.un(Uo::Negate, x);
            }
            values.push(x);
        }
        let mut value = self.b.math(Mf::Fma, &values);
        if clamp {
            value = self.b.math(Mf::Saturate, &[value]);
        }
        let value = match name {
            "v_fma_mix_f32" => self.b.bits(value),
            // The 16-bit forms write one half of the destination and keep the other.
            _ => {
                self.note_half("v_fma");
                let wide = self.b.bits(value);
                let half = self.narrow(wide);
                let (low, old) = (self.b.u32(0xFFFF), self.raw(Reg::V(dst)));
                match name {
                    "v_fma_mixlo_f16" => {
                        let high = self.b.u32(0xFFFF_0000);
                        let kept = self.b.bin(Bo::And, old, high);
                        self.b.bin(Bo::InclusiveOr, kept, half)
                    }
                    _ => {
                        let sixteen = self.b.u32(16);
                        let raised = self.b.bin(Bo::ShiftLeft, half, sixteen);
                        let kept = self.b.bin(Bo::And, old, low);
                        self.b.bin(Bo::InclusiveOr, kept, raised)
                    }
                }
            }
        };
        self.vput(dst, value);
        Ok(())
    }

    /// `v_pk_*_f16`: one operation over the low halves of its sources and again over the high
    /// ones, each a 16-bit float; `sides` says for each result which half of each source it
    /// reads and which sources it negates.
    fn pk_half(
        &mut self,
        pc: u32,
        inst: &Inst,
        dst: u8,
        srcs: [Operand; 3],
        sides: [(u8, u8); 2],
        clamp: bool,
    ) -> Done {
        let name = inst.name();
        let count = inst.op().map_or(0, |op| usize::from(op.sources()));
        let wide = (name.strip_prefix("v_pk_"))
            .and_then(|rest| rest.strip_suffix("_f16"))
            .map(|kind| format!("v_{kind}_f32"))
            .ok_or_else(|| refused(pc, inst))?;
        let mut results = Vec::with_capacity(2);
        for (select, negate) in sides {
            let mut inputs = Vec::with_capacity(count);
            for (i, src) in srcs.iter().take(count).enumerate() {
                // An inline float constant is that value as a half, in the low half.
                let bits = match src {
                    Operand::Float(word) => self.b.u32(half_word(*word)),
                    _ => self.sv(pc, *src)?,
                };
                let mut x = self.half(bits, (select >> i) & 1 != 0);
                if (negate >> i) & 1 != 0 {
                    x = self.b.un(Uo::Negate, x);
                }
                inputs.push(self.b.bits(x));
            }
            let mut value = (self.b.alu(&wide, &inputs)).ok_or_else(|| refused(pc, inst))?;
            if clamp {
                value = self.b.fmath(Mf::Saturate, &[value]);
            }
            results.push(value);
        }
        self.note_half(name);
        let rounding = self.half_rounding();
        let packed = self.pack_halves(results[0], results[1], rounding);
        self.vput(dst, packed);
        Ok(())
    }

    /// `v_pack_b32_f16`: the chosen 16-bit half of each source, the first in the low half of the result.
    fn pack_b32(
        &mut self,
        pc: u32,
        dst: u8,
        srcs: [Operand; 3],
        (abs, neg, opsel): (u8, u8, u8),
    ) -> Done {
        let mut halves = Vec::with_capacity(2);
        for (i, src) in srcs.iter().take(2).enumerate() {
            // An inline float constant is a 16-bit one here, which the operand does not spell.
            let mut half = match src {
                Operand::Float(word) => self.b.u32(half_word(*word)),
                _ => self.sv(pc, *src)?,
            };
            if (opsel >> i) & 1 != 0 {
                let sixteen = self.b.u32(16);
                half = self.b.bin(Bo::ShiftRight, half, sixteen);
            }
            let low = self.b.u32(0xFFFF);
            half = self.b.bin(Bo::And, half, low);
            if (abs >> i) & 1 != 0 {
                let magnitude = self.b.u32(0x7FFF);
                half = self.b.bin(Bo::And, half, magnitude);
            }
            if (neg >> i) & 1 != 0 {
                let sign = self.b.u32(0x8000);
                half = self.b.bin(Bo::ExclusiveOr, half, sign);
            }
            halves.push(half);
        }
        let sixteen = self.b.u32(16);
        let raised = self.b.bin(Bo::ShiftLeft, halves[1], sixteen);
        let value = self.b.bin(Bo::InclusiveOr, halves[0], raised);
        self.vput(dst, value);
        Ok(())
    }

    /// The 16-bit interpolation: the second step reads one half of a parameter's component, interpolated,
    /// and leaves it as a 16-bit float; the first step's partial sum is only ever the second's input.
    fn interp_half(&mut self, pc: u32, inst: &Inst, name: &str, dst: u8, place: Operand) -> Done {
        if !self.fragment {
            return Err(refused(pc, inst));
        }
        if name != "v_interp_p2_f16" {
            self.defined.insert(Reg::V(dst));
            return Ok(());
        }
        // The first source field is no operand here: the attribute in bits 5:0, the component in 7:6, the half in 8.
        let code = match place {
            Operand::Sgpr(n) => u32::from(n),
            Operand::Int(n) if n >= 0 => 128 + n as u32,
            Operand::Int(n) => (192 - n) as u32,
            Operand::Reserved(code) => u32::from(code),
            Operand::Vgpr(n) => 256 + u32::from(n),
            _ => {
                return Err(form(
                    pc,
                    inst,
                    "an attribute field that reads as a status operand",
                ))
            }
        };
        let (attr, chan, high) = ((code & 0x3F) as u8, (code >> 6) & 3, (code >> 8) & 1 != 0);
        let Some(input) = self.program.inputs.get(usize::from(attr)) else {
            return Err(form(
                pc,
                inst,
                "an attribute its package's input table does not list",
            ));
        };
        let slot = input.slot;
        if self.attributes.contains_key(&slot) {
            return Err(form(pc, inst, "an input read both whole and as halves"));
        }
        let global = match self.halves.get(&(slot, high)) {
            Some(&global) => global,
            None => {
                let ty = vector(&mut self.module, VectorSize::Quad, Scalar::F32);
                let half = if high { "high" } else { "low" };
                let global = self.private(format!("attr{slot}_{half}"), ty);
                self.halves.insert((slot, high), global);
                global
            }
        };
        let pointer = self.global(global);
        let cell = self.b.index(pointer, chan);
        let value = self.b.load(cell);
        // The device interpolates the half as a float; the sum is rounded to 16 bits once, as the
        // hardware rounds its second step's.
        let wide = self.b.bits(value);
        let half = self.narrow(wide);
        self.vput_half(dst, half);
        Ok(())
    }

    /// The system value a pixel program finds in its `k`th vector register after the barycentric pair:
    /// the position components its signature reads, in order, then the front face as plus or minus one.
    fn system_input(&mut self, k: u8) -> Option<Handle<E>> {
        let program = self.program;
        let read: Vec<u32> = (0..4)
            .filter(|c| (program.position_read >> c) & 1 != 0)
            .collect();
        if let Some(&component) = read.get(usize::from(k)) {
            let position = self.position();
            let value = self.b.index(position, component);
            return Some(self.b.bits(value));
        }
        if !program.front_face || usize::from(k) != read.len() {
            return None;
        }
        let global = match self.front_face {
            Some(global) => global,
            None => {
                let ty = self.ty(TypeInner::Scalar(Scalar::BOOL));
                let global = self.private("front_facing".to_owned(), ty);
                self.front_face = Some(global);
                global
            }
        };
        let pointer = self.global(global);
        let front = self.b.load(pointer);
        let (yes, no) = (self.b.u32(1f32.to_bits()), self.b.u32((-1f32).to_bits()));
        Some(self.b.select(front, yes, no))
    }

    /// A sampler's global, by the table word its descriptor starts at or by the constant words the code wrote.
    fn sampler(
        &mut self,
        at: Option<Loc>,
        words: Option<[u32; 4]>,
        comparison: bool,
    ) -> Handle<naga::GlobalVariable> {
        let key = (at, words, comparison);
        if let Some(&global) = self.samplers.get(&key) {
            return global;
        }
        let name = format!("samp{}", self.samplers.len());
        let (inner, kind) = (
            TypeInner::Sampler { comparison },
            ResourceKind::Sampler { comparison },
        );
        let global = match &key.0 {
            Some(table) if sym::computed(table) => {
                self.bound_table(name, inner, naga::AddressSpace::Handle, kind, table.clone())
            }
            _ => self.bound(name, inner, naga::AddressSpace::Handle, kind, key.0.clone()),
        };
        if let Some(resource) = self.resources.last_mut() {
            resource.words = words;
        }
        self.samplers.insert(key, global);
        global
    }

    /// The samplers the four descriptor registers from `samp` may name, each as an expression
    /// with the condition a run takes it on: none for the first, which is taken when no other is.
    fn sampler_of(
        &mut self,
        pc: u32,
        samp: u8,
        compared: bool,
    ) -> Result<Vec<Chosen>, NotTranslated> {
        let found = (
            sym::descriptor(&self.state, samp, 4),
            sym::literals(&self.state, samp),
        );
        let (global, slot) = match found {
            (Some(at), _) => {
                let slot = self.entry(pc, samp, &at, SAMPLER_DWORDS);
                let at = if slot.is_some() { whole(&at) } else { at };
                (self.sampler(Some(at), None, compared), slot)
            }
            // The code wrote one of a few samplers, by the path it came: each is bound, and
            // the registers' words say which this run reads through.
            (None, Some(choices)) => {
                let mut samplers = Vec::with_capacity(choices.len());
                for (nth, words) in choices.iter().enumerate() {
                    let global = self.sampler(None, Some(*words), compared);
                    let mut when = None;
                    for k in (0..4u8).filter(|_| nth > 0) {
                        if choices
                            .iter()
                            .all(|other| other[usize::from(k)] == words[usize::from(k)])
                        {
                            continue;
                        }
                        let (held, word) = (
                            self.get(pc, Reg::S(samp + k)),
                            self.b.u32(words[usize::from(k)]),
                        );
                        let same = self.b.bin(Bo::Equal, held, word);
                        when = Some(match when {
                            Some(so_far) => self.b.bin(Bo::LogicalAnd, so_far, same),
                            None => same,
                        });
                    }
                    samplers.push((self.global(global), when));
                }
                return Ok(samplers);
            }
            (None, None) => {
                return Err(NotTranslated::Descriptor {
                    pc,
                    what: "a sampler whose descriptor it did not load from a table",
                })
            }
        };
        let mut sampler = self.global(global);
        if let Some(index) = slot {
            sampler = self.b.emit(E::Access {
                base: sampler,
                index,
            });
        }
        Ok(vec![(sampler, None)])
    }

    /// A mesh thread's number in its group, and its group's number.
    fn mesh_ids(&mut self, mesh: Mesh) -> (Handle<E>, Handle<E>) {
        let pointer = self.global(mesh.thread);
        let thread = self.b.load(pointer);
        let pointer = self.global(mesh.group);
        (thread, self.b.load(pointer))
    }

    /// `v_mbcnt_*` over a mask of every lane: how many lanes of the wave come before this one, in
    /// the mask's low half or in its high half, added to the second source.
    fn mbcnt(&mut self, pc: u32, inst: &Inst, name: &str, dst: u8, srcs: [Operand; 3]) -> Done {
        if srcs[0] != Operand::Int(-1) {
            // The lanes of a mask before this one, counted: the low half's instruction gives
            // the whole count, since a mask here holds this lane's bit and no other's.
            if !self.subgroups && !self.fragment {
                return Err(form(pc, inst, "a mask that is not every lane"));
            }
            let mut value = self.sv(pc, srcs[1])?;
            if name == "v_mbcnt_lo_u32_b32" {
                let (mask, one) = (self.sv(pc, srcs[0])?, self.b.u32(1));
                let bit = self.b.bin(Bo::And, mask, one);
                let how = naga::CollectiveOperation::ExclusiveScan;
                let before =
                    (self.collective("v_add_u32", how, bit)).ok_or_else(|| refused(pc, inst))?;
                value = self.b.bin(Bo::Add, value, before);
            }
            self.vput(dst, value);
            return Ok(());
        }
        let thread = match (self.mesh, self.subgroups) {
            (Some(mesh), _) => self.mesh_ids(mesh).0,
            (None, true) => self.lane_number(),
            // Each run of a geometry pair's body is the first lane of its wave.
            (None, false) if self.pair.is_some() => self.b.u32(0),
            // A pixel that is a wave of one lane is its first.
            (None, false) if self.fragment => {
                self.note("a lane's number in its wave read as the first");
                self.b.u32(0)
            }
            (None, false) => {
                return Err(form(
                    pc,
                    inst,
                    "a lane's number in a program that is one lane of no wave",
                ))
            }
        };
        let lanes = if self.program.wave32 { WAVE / 2 } else { WAVE };
        let (last, half) = (self.b.u32(lanes - 1), self.b.u32(WAVE / 2));
        let lane = self.b.bin(Bo::And, thread, last);
        let before = match name {
            "v_mbcnt_lo_u32_b32" => self.b.math(Mf::Min, &[lane, half]),
            _ => {
                let past = self.b.math(Mf::Max, &[lane, half]);
                self.b.bin(Bo::Subtract, past, half)
            }
        };
        let base = self.sv(pc, srcs[1])?;
        let value = self.b.bin(Bo::Add, before, base);
        self.vput(dst, value);
        Ok(())
    }

    /// A mesh thread's primitive: the dword of three vertex numbers and a cull bit, kept for the entry point.
    fn export_primitive(&mut self, pc: u32, en: u8, source: u8) -> Done {
        let Some(mesh) = self.mesh else {
            return Ok(());
        };
        if en & 1 == 0 {
            return Ok(());
        }
        let value = self.vreg(pc, source)?;
        let active = self.masked.then(|| self.active());
        if active.is_some() {
            self.b.open();
        }
        let pointer = self.global(mesh.prim);
        self.b.push(Statement::Store { pointer, value });
        let pointer = self.global(mesh.prim_set);
        let value = self.b.emit(E::Literal(Literal::Bool(true)));
        self.b.push(Statement::Store { pointer, value });
        if let Some(condition) = active {
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition,
                accept,
                reject: Block::new(),
            });
        }
        Ok(())
    }

    /// The part of a dword an `sdwa` selector names: a byte, a 16-bit half or the whole.
    fn sdwa_part(
        &mut self,
        pc: u32,
        inst: &Inst,
        value: Handle<E>,
        sel: u32,
        signed: bool,
    ) -> Value {
        Ok(match sel {
            0..=3 => self.field(value, 8 * sel, 8, signed),
            4 | 5 => self.field(value, 16 * (sel - 4), 16, signed),
            6 => value,
            _ => return Err(form(pc, inst, "a part selector that names nothing")),
        })
    }

    /// The sources of an instruction whose trailing word selects a part of each: the word names the
    /// first, `second` is the instruction's own register field.
    fn sdwa_sources(
        &mut self,
        pc: u32,
        inst: &Inst,
        word: u32,
        second: Option<u8>,
    ) -> Result<(Handle<E>, Option<Handle<E>>), NotTranslated> {
        // Bits 20, 21, 28 and 29 negate or take the magnitude of a source, read as a float.
        let register = |cx: &mut Self, number: u8, scalar: bool| match scalar {
            true => cx.sreg(pc, number),
            false => cx.vreg(pc, number),
        };
        let first = register(self, word as u8, (word >> 23) & 1 != 0)?;
        let first = self.sdwa_part(pc, inst, first, (word >> 16) & 7, (word >> 19) & 1 != 0)?;
        let first = self.modified(first, (word >> 21) & 1 != 0, (word >> 20) & 1 != 0);
        let second = match second {
            Some(number) => {
                let value = register(self, number, word >> 31 != 0)?;
                let part =
                    self.sdwa_part(pc, inst, value, (word >> 24) & 7, (word >> 27) & 1 != 0)?;
                Some(self.modified(part, (word >> 29) & 1 != 0, (word >> 28) & 1 != 0))
            }
            None => None,
        };
        Ok((first, second))
    }

    /// Writes a result to the part of the destination an `sdwa` word selects, the rest of it zero or kept.
    fn sdwa_put(&mut self, pc: u32, inst: &Inst, word: u32, dst: u8, value: Handle<E>) -> Done {
        let (sel, unused) = ((word >> 8) & 7, (word >> 11) & 3);
        // Bit 13 clamps a float result and bits 15:14 scale it.
        let (clamp, omod) = ((word >> 13) & 1 != 0, ((word >> 14) & 3) as u8);
        let mut value = value;
        if clamp || omod != 0 {
            let name = inst.name();
            let float_out = (name.ends_with("_f32") && !name.starts_with("v_cvt_"))
                || name.starts_with("v_cvt_f32_");
            if !float_out {
                return Err(form(pc, inst, "an output modifier on an integer result"));
            }
            value = self.output_modified(value, clamp, omod);
        }
        let (first, width) = match sel {
            0..=3 => (8 * sel, 8),
            4 | 5 => (16 * (sel - 4), 16),
            6 => {
                self.vput(dst, value);
                return Ok(());
            }
            _ => return Err(form(pc, inst, "a part selector that names nothing")),
        };
        let mask = ((1u32 << width) - 1) << first;
        let (by, bits) = (self.b.u32(first), self.b.u32(mask));
        let raised = self.b.bin(Bo::ShiftLeft, value, by);
        let placed = self.b.bin(Bo::And, raised, bits);
        let value = match unused {
            0 => placed,
            2 => {
                let (old, rest) = (self.raw(Reg::V(dst)), self.b.u32(!mask));
                let kept = self.b.bin(Bo::And, old, rest);
                self.b.bin(Bo::InclusiveOr, kept, placed)
            }
            _ => return Err(form(pc, inst, "a sign-extended part of the destination")),
        };
        self.vput(dst, value);
        Ok(())
    }

    /// Ends this lane from the function it began in.
    fn finish_lane(&mut self) {
        self.finish_lane_at(Place::default());
    }

    /// Ends this lane: a pixel lane the program switched off writes nothing. A function
    /// statements were moved to says that the lane ended, for the one that called it to end too.
    fn finish_lane_at(&mut self, place: Place) {
        if self.fragment && self.masked {
            let active = self.active();
            let condition = self.b.lnot(active);
            let mut accept = Block::new();
            accept.push(Statement::Kill, Span::UNDEFINED);
            self.b.push(Statement::If {
                condition,
                accept,
                reject: Block::new(),
            });
        }
        if place.moved {
            let ended = self.b.u32(ENDED_RETURN);
            self.put(ENDED, ended);
        }
        self.b.push(Statement::Return { value: None });
    }

    /// The blocks as one loop over a switch on the number of the block to run next, for a graph that
    /// no nest of ifs and loops reproduces.
    fn dispatch(&mut self, ends: &[End], blocks: Vec<Block>) {
        let entry = self.b.u32(0);
        self.put(Reg::Next, entry);
        self.b.open();
        let selector = self.raw(Reg::Next);
        let mut cases = Vec::with_capacity(blocks.len() + 1);
        for (index, (statements, end)) in blocks.into_iter().zip(ends).enumerate() {
            self.b.open();
            self.b.push(Statement::Block(statements));
            match *end {
                End::Return => self.finish_lane(),
                End::Jump(to) => {
                    let to = self.b.u32(to);
                    self.put(Reg::Next, to);
                }
                End::Branch { then, otherwise } => {
                    let condition = self.raw(Reg::Cond(index as u32));
                    // Two stores under an if: Direct3D's older compiler stops on a choice
                    // of two values assigned to the variable a switch in a loop reads.
                    let (then, otherwise) = (self.b.u32(then), self.b.u32(otherwise));
                    self.put(Reg::Next, otherwise);
                    self.when(condition, |cx| {
                        cx.put(Reg::Next, then);
                        Ok(())
                    })
                    .unwrap_or(());
                }
            }
            cases.push(naga::SwitchCase {
                value: naga::SwitchValue::U32(index as u32),
                body: self.b.close(),
                fall_through: false,
            });
        }
        self.b.open();
        self.b.push(Statement::Return { value: None });
        cases.push(naga::SwitchCase {
            value: naga::SwitchValue::Default,
            body: self.b.close(),
            fall_through: false,
        });
        self.b.push(Statement::Switch { selector, cases });
        let body = self.b.close();
        self.b.push(Statement::Loop {
            body,
            continuing: Block::new(),
            break_if: None,
        });
    }

    /// The private `vec4` a pixel input arrives in, by the attribute number the code names.
    fn attribute(
        &mut self,
        pc: u32,
        inst: &Inst,
        attr: u8,
        interpolated: bool,
    ) -> Result<Handle<naga::GlobalVariable>, NotTranslated> {
        let Some(input) = self.program.inputs.get(usize::from(attr)) else {
            return Err(form(
                pc,
                inst,
                "an attribute its package's input table does not list",
            ));
        };
        let slot = input.slot;
        if self.halves.keys().any(|(held, _)| *held == slot) {
            return Err(form(pc, inst, "an input read both whole and as halves"));
        }
        if let Some((global, any)) = self.attributes.get_mut(&slot) {
            *any |= interpolated;
            return Ok(*global);
        }
        let ty = vector(&mut self.module, VectorSize::Quad, Scalar::F32);
        let global = self.private(format!("attr{slot}"), ty);
        self.attributes.insert(slot, (global, interpolated));
        Ok(global)
    }

    fn vintrp(&mut self, pc: u32, inst: &Inst, name: &str, dst: u8, attr: u8, chan: u8) -> Done {
        if !self.fragment {
            return Err(refused(pc, inst));
        }
        // The first step's partial sum is only ever the second step's input, which reads the
        // whole interpolated value from the stage instead.
        // ponytail: the stage's own interpolation stands in for the barycentric pair the
        // instruction reads, so a centroid or sample pair reads as the declared one.
        if name == "v_interp_p1_f32" {
            self.defined.insert(Reg::V(dst));
            return Ok(());
        }
        let global = self.attribute(pc, inst, attr, name != "v_interp_mov_f32")?;
        let pointer = self.global(global);
        let cell = self.b.index(pointer, u32::from(chan & 3));
        let value = self.b.load(cell);
        let bits = self.b.bits(value);
        self.vput(dst, bits);
        Ok(())
    }

    fn export_global(&mut self, target: u8) -> Handle<naga::GlobalVariable> {
        if let Some(&global) = self.exports.get(&target) {
            return global;
        }
        let ty = vector(&mut self.module, VectorSize::Quad, Scalar::F32);
        let global = self.private(format!("out{target}"), ty);
        self.exports.insert(target, global);
        global
    }

    fn exp(
        &mut self,
        pc: u32,
        inst: &Inst,
        target: u8,
        en: u8,
        srcs: [u8; 4],
        compr: bool,
    ) -> Done {
        if self.mesh.is_some() && target == PRIM {
            return self.export_primitive(pc, en, srcs[0]);
        }
        if self.launch.is_some() {
            return Err(form(pc, inst, "a compute program"));
        }
        // An export of nothing is the one a wave with no vertex still has to make.
        if !self.fragment && en == 0 {
            return Ok(());
        }
        let colour = target < MRT_COUNT;
        let known = match self.fragment {
            true => colour || target == MRTZ || target == NULL_TARGET,
            false => target == POS0 || target == POS1 || target >= PARAM0,
        };
        if !known {
            return Err(form(pc, inst, "that target"));
        }
        if target == NULL_TARGET {
            return Ok(());
        }
        if compr && !colour {
            return Err(form(
                pc,
                inst,
                "packed halves to a target that is not a colour",
            ));
        }
        if target == MRTZ && en != 1 {
            return Err(form(pc, inst, "a stencil, a sample mask or an alpha"));
        }
        let mut parts = Vec::with_capacity(4);
        for i in 0..4u8 {
            // Packed, each source holds two components as halves, and its enable bit is the pair's first.
            let (source, enabled) = match compr {
                true => (srcs[usize::from(i / 2)], en >> (i & !1) & 1 != 0),
                false => (srcs[usize::from(i)], en >> i & 1 != 0),
            };
            parts.push(match (enabled, compr) {
                (false, _) => self.b.f32(0.0),
                (true, false) => {
                    let bits = self.vreg(pc, source)?;
                    self.b.float(bits)
                }
                (true, true) => {
                    let bits = self.vreg(pc, source)?;
                    self.half(bits, i % 2 == 1)
                }
            });
        }
        let masked = self.masked;
        let active = masked.then(|| self.active());
        if masked {
            self.b.open();
        }
        if target == MRTZ {
            let global = match self.depth {
                Some(global) => global,
                None => {
                    let ty = self.ty(TypeInner::Scalar(Scalar::F32));
                    let global = self.private("depth".to_owned(), ty);
                    self.depth = Some(global);
                    global
                }
            };
            let pointer = self.global(global);
            self.b.push(Statement::Store {
                pointer,
                value: parts[0],
            });
        } else {
            let ty = vector(&mut self.module, VectorSize::Quad, Scalar::F32);
            let global = self.export_global(target);
            let pointer = self.global(global);
            let value = self.b.emit(E::Compose {
                ty,
                components: parts,
            });
            self.b.push(Statement::Store { pointer, value });
            if let (Some(mesh), POS0) = (self.mesh, target) {
                let pointer = self.global(mesh.pos_set);
                let value = self.b.emit(E::Literal(Literal::Bool(true)));
                self.b.push(Statement::Store { pointer, value });
            }
        }
        if let Some(condition) = active {
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition,
                accept,
                reject: Block::new(),
            });
        }
        Ok(())
    }

    /// The register of an image instruction's `i`th address operand: listed one by one, or the `i`th from the first.
    fn address(&mut self, pc: u32, inst: &Inst, addrs: &[u8], i: usize) -> Value {
        let register = match addrs {
            [first] => u8::try_from(usize::from(*first) + i).ok(),
            listed => listed.get(i).copied(),
        };
        let register = register.ok_or_else(|| form(pc, inst, "an address past its registers"))?;
        self.vreg(pc, register)
    }

    /// The next address operand that fills a register whatever the address width: a bias, a
    /// reference, a gradient.
    fn whole_operand(&mut self, pc: u32, inst: &Inst, addrs: &[u8], at: &mut Cursor) -> Value {
        if at.high {
            (at.next, at.high) = (at.next + 1, false);
        }
        let bits = self.address(pc, inst, addrs, at.next)?;
        at.next += 1;
        Ok(bits)
    }

    /// The next coordinate, slice or level: a register, or with 16-bit addresses one half of one,
    /// the low half first.
    fn packed_operand(
        &mut self,
        pc: u32,
        inst: &Inst,
        addrs: &[u8],
        at: &mut Cursor,
    ) -> Result<(Handle<E>, Option<bool>), NotTranslated> {
        if !at.packed {
            return Ok((self.whole_operand(pc, inst, addrs, at)?, None));
        }
        let bits = self.address(pc, inst, addrs, at.next)?;
        let high = at.high;
        (at.next, at.high) = match high {
            true => (at.next + 1, false),
            false => (at.next, true),
        };
        Ok((bits, Some(high)))
    }

    /// The next coordinate as a float.
    fn float_operand(&mut self, pc: u32, inst: &Inst, addrs: &[u8], at: &mut Cursor) -> Value {
        Ok(match self.packed_operand(pc, inst, addrs, at)? {
            (bits, None) => self.b.float(bits),
            (bits, Some(high)) => self.half(bits, high),
        })
    }

    /// The next coordinate as a signed integer; a 16-bit one is not sign-extended.
    fn int_operand(&mut self, pc: u32, inst: &Inst, addrs: &[u8], at: &mut Cursor) -> Value {
        let bits = match self.packed_operand(pc, inst, addrs, at)? {
            (bits, None) => bits,
            (bits, Some(high)) => self.field(bits, 16 * u32::from(high), 16, false),
        };
        Ok(self.b.sint(bits))
    }

    /// A sample's texel offset: the constant its first address register was given, six signed
    /// bits an axis in its low three bytes.
    fn texel_offset(
        &mut self,
        pc: u32,
        inst: &Inst,
        addrs: &[u8],
        at: &mut Cursor,
        (coords, cube): (usize, bool),
    ) -> Value {
        let register = match addrs {
            [first] => u8::try_from(usize::from(*first) + at.next).ok(),
            listed => listed.get(at.next).copied(),
        };
        at.next += 1;
        let held = register.and_then(|n| self.state.moved(n).cloned());
        let Some(Sym::Const(word)) = held.filter(|_| !cube) else {
            return Err(form(
                pc,
                inst,
                "a texel offset the program computes, or one into a cube",
            ));
        };
        let mut parts = Vec::with_capacity(coords);
        for axis in 0..coords as u32 {
            let six = (word >> (8 * axis)) & 0x3F;
            let signed = ((six << 26) as i32) >> 26;
            parts.push(self.b.emit(E::Literal(Literal::I32(signed))));
        }
        Ok(match parts.as_slice() {
            [only] => *only,
            _ => {
                let ty = vector(&mut self.module, size_of(coords), Scalar::I32);
                self.b.emit(E::Compose {
                    ty,
                    components: parts,
                })
            }
        })
    }

    /// The four texels a gather at `level` reads of a two-dimensional image, as the bits of
    /// their `component`: those around the coordinate, in a gather's order.
    // ponytail: the edges are clamped whatever the sampler's addressing; a wrapping gather at a
    // stated level needs the sampler's words read at run time.
    fn gathered_texels(
        &mut self,
        image: Handle<E>,
        coordinate: Handle<E>,
        array_index: Option<Handle<E>>,
        (level, offset): (Handle<E>, Option<Handle<E>>),
        component: u32,
    ) -> Vec<Handle<E>> {
        self.note("a gather at a stated level read as its four texels, the edges clamped");
        let levels = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::NumLevels,
        });
        let (one, zero) = (self.b.u32(1), self.b.emit(E::Literal(Literal::I32(0))));
        let last = self.b.bin(Bo::Subtract, levels, one);
        let last = self.b.sint(last);
        let nearest = self.b.math(Mf::Round, &[level]);
        let level = self.b.convert(nearest, ScalarKind::Sint);
        let level = self.b.math(Mf::Clamp, &[level, zero, last]);
        let size = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::Size { level: Some(level) },
        });
        let (extent, half) = (self.b.convert(size, ScalarKind::Float), self.b.f32(0.5));
        let place = self.b.bin(Bo::Multiply, coordinate, extent);
        let ivec2 = vector(&mut self.module, VectorSize::Bi, Scalar::I32);
        let fvec2 = vector(&mut self.module, VectorSize::Bi, Scalar::F32);
        let halves = self.b.emit(E::Compose {
            ty: fvec2,
            components: vec![half, half],
        });
        let corner = self.b.bin(Bo::Subtract, place, halves);
        let corner = self.b.math(Mf::Floor, &[corner]);
        let corner = self.b.convert(corner, ScalarKind::Sint);
        let size = self.b.convert(size, ScalarKind::Sint);
        let (ones, zeros) = (
            self.b.emit(E::Literal(Literal::I32(1))),
            self.b.emit(E::Compose {
                ty: ivec2,
                components: vec![zero, zero],
            }),
        );
        let ones = self.b.emit(E::Compose {
            ty: ivec2,
            components: vec![ones, ones],
        });
        let furthest = self.b.bin(Bo::Subtract, size, ones);
        let mut parts = Vec::with_capacity(4);
        for (x, y) in [(0, 1), (1, 1), (1, 0), (0, 0)] {
            let (x, y) = (
                self.b.emit(E::Literal(Literal::I32(x))),
                self.b.emit(E::Literal(Literal::I32(y))),
            );
            let step = self.b.emit(E::Compose {
                ty: ivec2,
                components: vec![x, y],
            });
            let mut texel = self.b.bin(Bo::Add, corner, step);
            if let Some(offset) = offset {
                texel = self.b.bin(Bo::Add, texel, offset);
            }
            let texel = self.b.math(Mf::Clamp, &[texel, zeros, furthest]);
            let read = self.b.emit(E::ImageLoad {
                image,
                coordinate: texel,
                array_index,
                sample: None,
                level: Some(level),
            });
            let part = self.b.index(read, component);
            parts.push(self.b.bits(part));
        }
        parts
    }

    /// The image the eight descriptor registers from `rsrc` name, as an expression.
    fn image_of(&mut self, pc: u32, inst: &Inst, (rsrc, dim, compared): (u8, u8, bool)) -> Value {
        let (dimension, arrayed) = match dim {
            0 => (ImageDimension::D1, false),
            1 => (ImageDimension::D2, false),
            2 => (ImageDimension::D3, false),
            3 => (ImageDimension::Cube, false),
            5 => (ImageDimension::D2, true),
            _ => return Err(form(pc, inst, "that image dimension")),
        };
        let Some(at) = sym::descriptor(&self.state, rsrc, 8) else {
            return Err(NotTranslated::Descriptor {
                pc,
                what: "an image whose descriptor it did not load from a table",
            });
        };
        if self.told.contains_key(&at) {
            return Err(form(
                pc,
                inst,
                "an image the program also stores to, sampled or measured",
            ));
        }
        // A place a run computed picks one entry of the whole table, bound as an array of this kind of image.
        let slot = self.entry(pc, rsrc, &at, SLOT_DWORDS);
        let at = if slot.is_some() { whole(&at) } else { at };
        let texture = match self.textures.get(&(at.clone(), dim, compared)) {
            Some(&global) => global,
            None => {
                let class = match compared {
                    true => ImageClass::Depth { multi: false },
                    false => ImageClass::Sampled {
                        kind: ScalarKind::Float,
                        multi: false,
                    },
                };
                let name = format!("tex{}", self.textures.len());
                let inner = TypeInner::Image {
                    dim: dimension,
                    arrayed,
                    class,
                };
                let kind = ResourceKind::Texture { dim, compared };
                let space = naga::AddressSpace::Handle;
                let global = match slot {
                    Some(_) => self.bound_table(name, inner, space, kind, at.clone()),
                    None => self.bound(name, inner, space, kind, Some(at.clone())),
                };
                self.textures.insert((at, dim, compared), global);
                global
            }
        };
        let mut image = self.global(texture);
        if let Some(index) = slot {
            image = self.b.emit(E::Access { base: image, index });
        }
        Ok(image)
    }

    /// A direction from the face of a cube and the two coordinates on it the hardware samples by.
    fn cube_direction(&mut self, s: Handle<E>, t: Handle<E>, face: Handle<E>) -> Handle<E> {
        let (two, one) = (self.b.f32(2.0), self.b.f32(1.0));
        let mut across = [s, t];
        for coordinate in &mut across {
            let doubled = self.b.bin(Bo::Multiply, *coordinate, two);
            *coordinate = self.b.bin(Bo::Subtract, doubled, one);
        }
        let [u, v] = across;
        let (nu, nv, none) = (
            self.b.un(Uo::Negate, u),
            self.b.un(Uo::Negate, v),
            self.b.f32(-1.0),
        );
        let nearest = self.b.math(Mf::Round, &[face]);
        let face = self.b.convert(nearest, ScalarKind::Uint);
        let mut is = Vec::with_capacity(6);
        for number in 0..6 {
            let number = self.b.u32(number);
            is.push(self.b.bin(Bo::Equal, face, number));
        }
        // By face: (1, -v, -u), (-1, -v, u), (u, 1, v), (u, -1, -v), (u, -v, 1), (-u, -v, -1).
        let mut x = self.b.select(is[5], nu, u);
        x = self.b.select(is[1], none, x);
        x = self.b.select(is[0], one, x);
        let mut y = self.b.select(is[3], none, nv);
        y = self.b.select(is[2], one, y);
        let mut z = self.b.select(is[4], one, none);
        z = self.b.select(is[3], nv, z);
        z = self.b.select(is[2], v, z);
        z = self.b.select(is[1], u, z);
        z = self.b.select(is[0], nu, z);
        let ty = vector(&mut self.module, VectorSize::Tri, Scalar::F32);
        self.b.emit(E::Compose {
            ty,
            components: vec![x, y, z],
        })
    }

    /// Writes an image instruction's results: a register a component, or with 16-bit data a
    /// register a pair of them as halves, the first in the low half.
    fn put_texel(
        &mut self,
        pc: u32,
        inst: &Inst,
        (data, d16, floats): (u8, bool, bool),
        parts: &[Handle<E>],
    ) -> Done {
        if !d16 {
            for (k, &part) in parts.iter().enumerate() {
                self.vput(data.wrapping_add(k as u8), part);
            }
            return Ok(());
        }
        if !floats {
            return Err(form(pc, inst, "16-bit data that is no float"));
        }
        for k in 0..parts.len().div_ceil(2) {
            let low = parts[2 * k];
            let high = match parts.get(2 * k + 1) {
                Some(&bits) => bits,
                None => self.b.u32(0),
            };
            let packed = self.pack_halves(low, high, Rounding::Nearest);
            self.vput(data.wrapping_add(k as u8), packed);
        }
        Ok(())
    }

    /// The level of detail a sample at these coordinates takes, from their derivatives, and the same kept inside the image's levels.
    // ponytail: the isotropic level of the two axes' longer step; the sampler's own bias and clamps are not applied.
    fn level_of_detail(
        &mut self,
        image: Handle<E>,
        coordinate: Handle<E>,
    ) -> (Handle<E>, Handle<E>) {
        let size = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::Size { level: None },
        });
        let size = self.b.convert(size, ScalarKind::Float);
        let texels = self.b.bin(Bo::Multiply, coordinate, size);
        let mut longest = self.b.f32(0.0);
        for axis in [DerivativeAxis::X, DerivativeAxis::Y] {
            let step = self.b.emit(E::Derivative {
                axis,
                ctrl: DerivativeControl::Coarse,
                expr: texels,
            });
            let squared = self.b.math(Mf::Dot, &[step, step]);
            longest = self.b.math(Mf::Max, &[longest, squared]);
        }
        let (half, zero, one) = (self.b.f32(0.5), self.b.f32(0.0), self.b.u32(1));
        let log = self.b.math(Mf::Log2, &[longest]);
        let level = self.b.bin(Bo::Multiply, log, half);
        let levels = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::NumLevels,
        });
        let last = self.b.bin(Bo::Subtract, levels, one);
        let last = self.b.convert(last, ScalarKind::Float);
        let kept = self.b.math(Mf::Clamp, &[level, zero, last]);
        self.note("a level of detail computed from the coordinates' derivatives");
        (kept, level)
    }

    /// `image_get_resinfo`: the size of a level, the depth or the layers, and the levels.
    fn image_size(&mut self, dim: u8, image: Handle<E>, level: Handle<E>) -> [Handle<E>; 4] {
        let size = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::Size { level: Some(level) },
        });
        let one = self.b.u32(1);
        let width = match dim {
            0 | 4 => size,
            _ => self.b.index(size, 0),
        };
        let height = match dim {
            0 | 4 => one,
            _ => self.b.index(size, 1),
        };
        let depth = match dim {
            2 => self.b.index(size, 2),
            4 | 5 => self.b.emit(E::ImageQuery {
                image,
                query: naga::ImageQuery::NumLayers,
            }),
            _ => one,
        };
        let levels = self.b.emit(E::ImageQuery {
            image,
            query: naga::ImageQuery::NumLevels,
        });
        [width, height, depth, levels]
    }

    /// The texture the eight descriptor registers from `rsrc` name, bound to be stored to, and
    /// its format; `atomic` is the format an update in one step states of it.
    fn stored_image(
        &mut self,
        pc: u32,
        inst: &Inst,
        (rsrc, dim): (u8, u8),
        atomic: Option<naga::StorageFormat>,
    ) -> Result<(Handle<E>, naga::StorageFormat, bool), NotTranslated> {
        use naga::StorageAccess as Access;
        let (dimension, arrayed) = match dim {
            0 => (ImageDimension::D1, false),
            1 => (ImageDimension::D2, false),
            2 => (ImageDimension::D3, false),
            5 => (ImageDimension::D2, true),
            _ => return Err(form(pc, inst, "that image dimension")),
        };
        let Some(at) = sym::descriptor(&self.state, rsrc, 8) else {
            return Err(NotTranslated::Descriptor {
                pc,
                what: "an image whose descriptor it did not load from a table",
            });
        };
        let slot = self.entry(pc, rsrc, &at, SLOT_DWORDS);
        let at = if slot.is_some() { whole(&at) } else { at };
        // An update anywhere in the program states the format every use of the image has.
        let atomic = atomic.or_else(|| self.told.get(&at).copied().flatten());
        let (global, format) = match self.targets.get(&(at.clone(), dim)) {
            Some(&(global, format, updated)) => {
                if atomic.is_some_and(|stated| updated && stated != format) {
                    return Err(form(pc, inst, "an image updated as two formats"));
                }
                if let (false, Some(_), None) = (updated, atomic, slot) {
                    self.restated.insert(at.clone(), atomic);
                }
                (global, format)
            }
            None => {
                let format = (atomic.or_else(|| self.formats.get(&at).copied()))
                    .unwrap_or(naga::StorageFormat::Rgba16Float);
                let access = match (atomic.is_some(), self.told.contains_key(&at)) {
                    (true, _) => Access::ATOMIC | Access::LOAD | Access::STORE,
                    (false, true) => Access::LOAD | Access::STORE,
                    (false, false) => Access::STORE,
                };
                let inner = TypeInner::Image {
                    dim: dimension,
                    arrayed,
                    class: ImageClass::Storage { format, access },
                };
                let kind = ResourceKind::StorageTexture { dim, format };
                let name = format!("target{}", self.targets.len());
                let space = naga::AddressSpace::Handle;
                let global = match slot {
                    Some(_) => self.bound_table(name, inner, space, kind, at.clone()),
                    None => self.bound(name, inner, space, kind, Some(at.clone())),
                };
                if let Some(resource) = self.resources.last_mut() {
                    resource.written = true;
                }
                (self.targets).insert((at.clone(), dim), (global, format, atomic.is_some()));
                (global, format)
            }
        };
        if slot.is_none() {
            self.images_stored.insert(at, atomic);
        }
        let mut image = self.global(global);
        if let Some(index) = slot {
            image = self.b.emit(E::Access { base: image, index });
        }
        Ok((image, format, atomic.is_some()))
    }

    /// A texel's integer coordinates and its slice, from an image instruction's address registers.
    fn texel_place(
        &mut self,
        pc: u32,
        inst: &Inst,
        addrs: &[u8],
        (dim, a16): (u8, bool),
    ) -> Result<(Handle<E>, Option<Handle<E>>), NotTranslated> {
        let (coords, arrayed) = match dim {
            0 => (1usize, false),
            1 => (2, false),
            2 => (3, false),
            5 => (2, true),
            _ => return Err(form(pc, inst, "that image dimension")),
        };
        let mut at = Cursor {
            next: 0,
            high: false,
            packed: a16,
        };
        let mut parts = Vec::with_capacity(coords);
        for _ in 0..coords {
            parts.push(self.int_operand(pc, inst, addrs, &mut at)?);
        }
        let array_index = match arrayed {
            true => Some(self.int_operand(pc, inst, addrs, &mut at)?),
            false => None,
        };
        let coordinate = match (parts.len(), parts.first().copied()) {
            (1, Some(only)) => only,
            _ => {
                let ty = vector(&mut self.module, size_of(coords), Scalar::I32);
                self.b.emit(E::Compose {
                    ty,
                    components: parts,
                })
            }
        };
        Ok((coordinate, array_index))
    }

    /// `image_load` of an image the program also stores to, through its stored binding.
    fn load_stored(
        &mut self,
        pc: u32,
        inst: &Inst,
        name: &str,
        atomic: Option<naga::StorageFormat>,
    ) -> Done {
        let Inst::Mimg {
            addrs,
            data,
            rsrc,
            dmask,
            dim,
            a16,
            d16,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        if name != "image_load" {
            return Err(form(pc, inst, "a level of an image the program stores to"));
        }
        let (image, format, _) = self.stored_image(pc, inst, (*rsrc, *dim), atomic)?;
        let scalar = Scalar::from(format);
        let (coordinate, array_index) = self.texel_place(pc, inst, addrs, (*dim, *a16))?;
        let texel = self.b.emit(E::ImageLoad {
            image,
            coordinate,
            array_index,
            sample: None,
            level: None,
        });
        if scalar.width != 4 {
            // A 64-bit texel fills two registers, its low dword first.
            let (wide, by) = (self.b.index(texel, 0), self.b.u32(32));
            let high = self.b.bin(Bo::ShiftRight, wide, by);
            let mut parts = Vec::with_capacity(2);
            for (c, half) in [wide, high].into_iter().enumerate() {
                if (dmask >> c) & 1 != 0 {
                    parts.push(self.b.emit(E::As {
                        expr: half,
                        kind: ScalarKind::Uint,
                        convert: Some(4),
                    }));
                }
            }
            return self.put_texel(pc, inst, (*data, *d16, false), &parts);
        }
        let mut parts = Vec::with_capacity(4);
        for c in (0..4).filter(|c| (dmask >> c) & 1 != 0) {
            let part = self.b.index(texel, c);
            parts.push(match scalar.kind {
                ScalarKind::Uint => part,
                _ => self.b.bits(part),
            });
        }
        let floats = scalar.kind == ScalarKind::Float;
        self.put_texel(pc, inst, (*data, *d16, floats), &parts)
    }

    /// `image_store` and `image_atomic_*`: a texel written, or updated in one step, in an
    /// active lane.
    // ponytail: an update leaves its data register as it was; the hardware leaves the texel's
    // old value there, which no program read was seen to use.
    fn image_write(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        use naga::{AtomicFunction as F, StorageFormat as Format};
        let Inst::Mimg {
            addrs,
            data,
            rsrc,
            dmask,
            dim,
            a16,
            d16,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        if *d16 {
            return Err(form(pc, inst, "16-bit data stored to an image"));
        }
        // An update names a dword of unsigned or signed data, or with two registers a 64-bit one.
        let update = match (name.strip_prefix("image_atomic_"), *dmask) {
            (None, 0xF) => None,
            (None, _) => return Err(form(pc, inst, "a store of some components")),
            (Some(fun @ ("smin" | "smax")), 1) => Some((fun, Format::R32Sint)),
            (Some(fun), 1) => Some((fun, Format::R32Uint)),
            (Some(fun), 3) => Some((fun, Format::R64Uint)),
            (Some(_), _) => return Err(form(pc, inst, "an update of that many components")),
        };
        let stated = update.map(|(_, format)| format);
        let (image, format, updated) = self.stored_image(pc, inst, (*rsrc, *dim), stated)?;
        let (coordinate, array_index) = self.texel_place(pc, inst, addrs, (*dim, *a16))?;
        // A whole store to an image the program also updates: no update stores, so the texel
        // is cleared and then set, each in one step.
        if let (None, true) = (update, updated) {
            if Scalar::from(format).width != 4 {
                return Err(form(
                    pc,
                    inst,
                    "a store to a 64-bit image updated in one step",
                ));
            }
            let bits = self.vreg(pc, *data)?;
            let (zero, value) = match format {
                Format::R32Sint => (self.b.emit(E::Literal(Literal::I32(0))), self.b.sint(bits)),
                _ => (self.b.u32(0), bits),
            };
            return self.when_active(|cx| {
                for (fun, value) in [(F::And, zero), (F::InclusiveOr, value)] {
                    cx.b.push(Statement::ImageAtomic {
                        image,
                        coordinate,
                        array_index,
                        fun,
                        value,
                    });
                }
                Ok(())
            });
        }
        let statement = match update {
            None => {
                let scalar = Scalar::from(format);
                let mut components = Vec::with_capacity(4);
                for k in 0..4u8 {
                    let bits = self.vreg(pc, data.wrapping_add(k))?;
                    components.push(match scalar.kind {
                        ScalarKind::Float => self.b.float(bits),
                        ScalarKind::Sint => self.b.sint(bits),
                        _ => bits,
                    });
                }
                let ty = vector(&mut self.module, VectorSize::Quad, scalar);
                let value = self.b.emit(E::Compose { ty, components });
                Statement::ImageStore {
                    image,
                    coordinate,
                    array_index,
                    value,
                }
            }
            Some((fun, format)) => {
                let fun = match fun {
                    "add" => F::Add,
                    "and" => F::And,
                    "or" => F::InclusiveOr,
                    "xor" => F::ExclusiveOr,
                    "umin" | "smin" => F::Min,
                    "umax" | "smax" => F::Max,
                    _ => return Err(form(pc, inst, "an update no atomic texture makes")),
                };
                let low = self.vreg(pc, *data)?;
                let value = match format {
                    Format::R64Uint => {
                        if !matches!(fun, F::Min | F::Max) {
                            return Err(form(pc, inst, "an update no 64-bit atomic texture makes"));
                        }
                        let high = self.vreg(pc, data.wrapping_add(1))?;
                        let wide = |b: &mut Builder, expr| {
                            b.emit(E::As {
                                expr,
                                kind: ScalarKind::Uint,
                                convert: Some(8),
                            })
                        };
                        let (low, high) = (wide(&mut self.b, low), wide(&mut self.b, high));
                        let by = self.b.u32(32);
                        let raised = self.b.bin(Bo::ShiftLeft, high, by);
                        self.b.bin(Bo::InclusiveOr, low, raised)
                    }
                    Format::R32Sint => self.b.sint(low),
                    _ => low,
                };
                Statement::ImageAtomic {
                    image,
                    coordinate,
                    array_index,
                    fun,
                    value,
                }
            }
        };
        self.when_active(|cx| {
            cx.b.push(statement);
            Ok(())
        })
    }

    #[allow(clippy::too_many_lines)]
    fn image(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Mimg {
            addrs,
            data,
            rsrc,
            samp,
            dmask,
            dim,
            a16,
            d16,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let (coords, arrayed, cube) = match dim {
            0 => (1usize, false, false),
            1 => (2, false, false),
            2 => (3, false, false),
            3 => (2, false, true),
            5 => (2, true, false),
            _ => return Err(form(pc, inst, "that image dimension")),
        };
        let mut at = Cursor {
            next: 0,
            high: false,
            packed: *a16,
        };
        let enabled: Vec<u32> = (0..4).filter(|c| (dmask >> c) & 1 != 0).collect();
        if name == "image_store" || name.starts_with("image_atomic_") {
            return self.image_write(pc, inst, name);
        }
        if matches!(name, "image_load" | "image_load_mip") {
            if cube {
                return Err(form(pc, inst, "a cube read by its texels"));
            }
            // An image the program also stores to is read through the binding it is stored through.
            let stored =
                (sym::descriptor(&self.state, *rsrc, 8)).and_then(|at| self.told.get(&at).copied());
            if let Some(atomic) = stored {
                return self.load_stored(pc, inst, name, atomic);
            }
            let image = self.image_of(pc, inst, (*rsrc, *dim, false))?;
            // Integer coordinates, the slice of an array after them, then the level where the mnemonic takes one.
            let mut parts = Vec::with_capacity(coords);
            for _ in 0..coords {
                parts.push(self.int_operand(pc, inst, addrs, &mut at)?);
            }
            let array_index = match arrayed {
                true => Some(self.int_operand(pc, inst, addrs, &mut at)?),
                false => None,
            };
            let level = match name {
                "image_load_mip" => self.int_operand(pc, inst, addrs, &mut at)?,
                _ => self.b.emit(E::Literal(Literal::I32(0))),
            };
            let first = parts.first().copied();
            let coordinate = match (parts.len(), first) {
                (1, Some(only)) => only,
                _ => {
                    let ty = vector(&mut self.module, size_of(coords), Scalar::I32);
                    self.b.emit(E::Compose {
                        ty,
                        components: parts,
                    })
                }
            };
            // ponytail: the texel is read as floats; an integer format needs the descriptor's own format, known only at run time.
            self.note("an image loaded as floats");
            let texel = self.b.emit(E::ImageLoad {
                image,
                coordinate,
                array_index,
                sample: None,
                level: Some(level),
            });
            let mut parts = Vec::with_capacity(4);
            for &c in &enabled {
                let part = self.b.index(texel, c);
                parts.push(self.b.bits(part));
            }
            return self.put_texel(pc, inst, (*data, *d16, true), &parts);
        }
        if name == "image_get_resinfo" {
            let image = self.image_of(pc, inst, (*rsrc, *dim, false))?;
            let level = self.int_operand(pc, inst, addrs, &mut at)?;
            let sizes = self.image_size(*dim, image, level);
            let parts: Vec<_> = enabled.iter().map(|&c| sizes[c as usize]).collect();
            return self.put_texel(pc, inst, (*data, *d16, false), &parts);
        }
        let (rest, gather, lod) = match (
            name.strip_prefix("image_sample"),
            name.strip_prefix("image_gather4"),
            name,
        ) {
            (Some(rest), _, _) => (rest, false, false),
            (None, Some(rest), _) => (rest, true, false),
            (None, None, "image_get_lod") => ("", false, true),
            _ => return Err(refused(pc, inst)),
        };
        let mut tokens: Vec<&str> = rest.split('_').filter(|t| !t.is_empty()).collect();
        // A texel offset is the first address operand, and a least level the last.
        let shifted = tokens.last() == Some(&"o");
        if shifted {
            tokens.pop();
        }
        let clamped = tokens.last() == Some(&"cl");
        if clamped {
            tokens.pop();
        }
        let (compared, level) = match tokens.as_slice() {
            _ if clamped && tokens != ["d"] => return Err(refused(pc, inst)),
            [] => (false, "auto"),
            ["lz"] => (false, "zero"),
            ["l"] => (false, "exact"),
            ["b"] => (false, "bias"),
            ["d"] => (false, "gradient"),
            ["c"] => (true, "auto"),
            ["c", "lz"] => (true, "zero"),
            _ => return Err(refused(pc, inst)),
        };
        // A gather at a level the program states is read as its four texels.
        let texels = gather && level == "exact" && !compared;
        if gather && ((level != "zero" && !texels) || coords != 2) {
            return Err(form(
                pc,
                inst,
                "a gather at a level other than the first or of an image that is not two-dimensional",
            ));
        }
        if clamped && coords != 2 {
            return Err(form(
                pc,
                inst,
                "a least level on an image that is not two-dimensional",
            ));
        }
        if !self.fragment && lod {
            return Err(form(pc, inst, "an implicit level outside a pixel program"));
        }
        // Only a pixel has the neighbours an implicit level is measured against: elsewhere the
        // threads of a quad would have to be read, and the first level stands in.
        let flat = !self.fragment && matches!(level, "auto" | "bias");
        if flat {
            self.note("an implicit level outside a pixel program read as the first level");
        }
        if level == "gradient" && *a16 {
            return Err(form(pc, inst, "gradients with 16-bit addresses"));
        }
        if cube && (gather || lod || compared) {
            return Err(form(pc, inst, "a cube gathered, compared or measured"));
        }
        let image = self.image_of(pc, inst, (*rsrc, *dim, compared))?;
        let samplers = match texels {
            true => Vec::new(),
            false => self.sampler_of(pc, *samp, compared)?,
        };
        // The address operands come in this order: offset, bias, reference, gradients, coordinates, level.
        let offset = match shifted {
            true => Some(self.texel_offset(pc, inst, addrs, &mut at, (coords, cube))?),
            false => None,
        };
        let bias = match level {
            "bias" => {
                let bits = self.whole_operand(pc, inst, addrs, &mut at)?;
                Some(self.b.float(bits))
            }
            _ => None,
        };
        let depth_ref = match compared {
            true => {
                let bits = self.whole_operand(pc, inst, addrs, &mut at)?;
                Some(self.b.float(bits))
            }
            false => None,
        };
        let mut vectors = Vec::with_capacity(3);
        let groups = if level == "gradient" { 3 } else { 1 };
        // Where a cube's coordinates lie on its face, and which face.
        let mut on_face = None;
        for group in 0..groups {
            // A cube's third address is its face, which its gradients have none of.
            let listed = match (cube, group + 1 < groups) {
                (true, false) => 3,
                _ => coords,
            };
            let mut parts = Vec::with_capacity(listed);
            for _ in 0..listed {
                parts.push(self.float_operand(pc, inst, addrs, &mut at)?);
            }
            let listed_parts = (
                parts.first().copied(),
                parts.get(1).copied(),
                parts.get(2).copied(),
            );
            vectors.push(match listed_parts {
                (Some(only), None, _) => only,
                (Some(s), Some(t), Some(face)) if cube => {
                    on_face = Some((s, t, face));
                    self.cube_direction(s, t, face)
                }
                _ => {
                    let ty = vector(&mut self.module, size_of(listed), Scalar::F32);
                    self.b.emit(E::Compose {
                        ty,
                        components: parts,
                    })
                }
            });
        }
        let coordinate = vectors[groups - 1];
        if lod {
            if coords != 2 {
                return Err(form(
                    pc,
                    inst,
                    "the level of an image that is not two-dimensional",
                ));
            }
            let (kept, raw) = self.level_of_detail(image, coordinate);
            let mut parts = Vec::with_capacity(2);
            for &c in &enabled {
                let part = match c {
                    0 => kept,
                    1 => raw,
                    _ => self.b.f32(0.0),
                };
                parts.push(self.b.bits(part));
            }
            return self.put_texel(pc, inst, (*data, *d16, true), &parts);
        }
        let array_index = match arrayed {
            true => {
                let slice = self.float_operand(pc, inst, addrs, &mut at)?;
                let nearest = self.b.math(Mf::Round, &[slice]);
                Some(self.b.convert(nearest, ScalarKind::Sint))
            }
            false => None,
        };
        if texels {
            let level = self.float_operand(pc, inst, addrs, &mut at)?;
            let Some(&component) = enabled.first().filter(|_| enabled.len() == 1) else {
                return Err(form(pc, inst, "a gather of more than one component"));
            };
            let parts =
                self.gathered_texels(image, coordinate, array_index, (level, offset), component);
            return self.put_texel(pc, inst, (*data, *d16, true), &parts);
        }
        // A least level on a read by gradients: the gradients stretched until the level they
        // give is that one, and where they are nothing the level itself.
        let least = match clamped {
            true => Some(self.float_operand(pc, inst, addrs, &mut at)?),
            false => None,
        };
        let mut still = None;
        if let Some(least) = least {
            let size = self.b.emit(E::ImageQuery {
                image,
                query: naga::ImageQuery::Size { level: None },
            });
            let size = self.b.convert(size, ScalarKind::Float);
            let mut longest = self.b.f32(0.0);
            for step in [vectors[0], vectors[1]] {
                let texels = self.b.bin(Bo::Multiply, step, size);
                let squared = self.b.math(Mf::Dot, &[texels, texels]);
                longest = self.b.math(Mf::Max, &[longest, squared]);
            }
            let (half, zero) = (self.b.f32(0.5), self.b.f32(0.0));
            let log = self.b.math(Mf::Log2, &[longest]);
            let given = self.b.bin(Bo::Multiply, log, half);
            let short = self.b.bin(Bo::Subtract, least, given);
            let short = self.b.math(Mf::Max, &[short, zero]);
            let stretch = self.b.math(Mf::Exp2, &[short]);
            for step in vectors.iter_mut().take(2) {
                *step = self.b.bin(Bo::Multiply, *step, stretch);
            }
            still = Some((self.b.bin(Bo::LessEqual, longest, zero), least));
            self.note("a least level on a read by gradients made by stretching the gradients");
        }
        // A cube's gradients are steps on its face: each is how far the direction moves by it.
        if let (true, Some((s, t, face))) = (level == "gradient", on_face) {
            for step in vectors.iter_mut().take(2) {
                let (ds, dt) = (self.b.index(*step, 0), self.b.index(*step, 1));
                let (s, t) = (self.b.bin(Bo::Add, s, ds), self.b.bin(Bo::Add, t, dt));
                let moved = self.cube_direction(s, t, face);
                *step = self.b.bin(Bo::Subtract, moved, coordinate);
            }
        }
        let level = match level {
            // The first level by its number: Direct3D reads a level named as the first only
            // where a sample compares, and a gather takes no other.
            "zero" if !compared && !gather => SampleLevel::Exact(self.b.f32(0.0)),
            "zero" => SampleLevel::Zero,
            "exact" => SampleLevel::Exact(self.float_operand(pc, inst, addrs, &mut at)?),
            "bias" if flat => SampleLevel::Exact(bias.unwrap_or(coordinate)),
            "bias" => SampleLevel::Bias(bias.unwrap_or(coordinate)),
            "gradient" => SampleLevel::Gradient {
                x: vectors[0],
                y: vectors[1],
            },
            _ if flat && !compared => SampleLevel::Exact(self.b.f32(0.0)),
            _ if flat => SampleLevel::Zero,
            _ => SampleLevel::Auto,
        };
        // A gather reads the one component its mask names, of the four texels around the coordinate.
        let component = match (gather, compared, enabled.as_slice()) {
            (false, _, _) | (true, true, _) => None,
            (true, false, &[0]) => Some(naga::SwizzleComponent::X),
            (true, false, &[1]) => Some(naga::SwizzleComponent::Y),
            (true, false, &[2]) => Some(naga::SwizzleComponent::Z),
            (true, false, &[3]) => Some(naga::SwizzleComponent::W),
            (true, false, _) => return Err(form(pc, inst, "a gather of more than one component")),
        };
        let gathered = match (gather, compared) {
            (true, true) => Some(naga::SwizzleComponent::X),
            _ => component,
        };
        // Read through each sampler the registers may name, and kept from the one they do.
        let mut texel = None;
        for (sampler, when) in samplers {
            let mut read = self.b.emit(E::ImageSample {
                image,
                sampler,
                gather: gathered,
                coordinate,
                array_index,
                offset,
                level,
                depth_ref,
                clamp_to_edge: false,
            });
            if let Some((flat, least)) = still {
                let at_least = self.b.emit(E::ImageSample {
                    image,
                    sampler,
                    gather: gathered,
                    coordinate,
                    array_index,
                    offset,
                    level: SampleLevel::Exact(least),
                    depth_ref,
                    clamp_to_edge: false,
                });
                read = self.b.select(flat, at_least, read);
            }
            texel = Some(match (texel, when) {
                (Some(other), Some(when)) => self.b.select(when, read, other),
                _ => read,
            });
        }
        let Some(texel) = texel else {
            return Err(refused(pc, inst));
        };
        // The enabled components land in consecutive registers from `data`; a comparison gives
        // one, a gather the four texels.
        let mut parts = Vec::with_capacity(4);
        if gather {
            for c in 0..4 {
                let part = self.b.index(texel, c);
                parts.push(self.b.bits(part));
            }
        } else {
            for &c in &enabled {
                let part = match compared {
                    true if c == 0 => texel,
                    true => self.b.f32(0.0),
                    false => self.b.index(texel, c),
                };
                parts.push(self.b.bits(part));
            }
        }
        self.put_texel(pc, inst, (*data, *d16, true), &parts)
    }

    fn inst(&mut self, pc: u32, inst: &Inst) -> Done {
        let name = inst.name();
        match inst {
            Inst::Sopp { .. } => self.sopp(pc, inst, name),
            Inst::Sop1 { dst, src, .. } => {
                let made = self.makes_mask(name, &[*src]);
                let done = self.sop1(pc, inst, name, *dst, *src);
                self.keep_mask(*dst, made, done)
            }
            Inst::Sop2 {
                dst, src0, src1, ..
            } => {
                let made = self.makes_mask(name, &[*src0, *src1]);
                let done = self.sop2(pc, inst, name, *dst, *src0, *src1);
                self.keep_mask(*dst, made, done)
            }
            Inst::Sopk {
                dst, imm, imm32, ..
            } => self.sopk(pc, inst, name, *dst, *imm, *imm32),
            Inst::Sopc { src0, src1, .. } => self.sopc(pc, inst, name, *src0, *src1),
            Inst::Smem {
                base,
                data,
                offset,
                soffset,
                ..
            } => self.smem(pc, inst, name, *base, *data, *offset, *soffset),
            Inst::Vop1 {
                dst, src, extra, ..
            } => self.vop1(pc, inst, name, *dst, *src, *extra),
            Inst::Vop2 {
                dst,
                src0,
                src1,
                extra,
                ..
            } => self.vop2(pc, inst, name, *dst, *src0, *src1, *extra),
            // A comparison of parts of its registers; bit 15 of the word says a mask register other
            // than the comparison's own takes the result, and bits 14:8 which.
            Inst::Vopc {
                src0: Operand::Sdwa,
                src1,
                extra: Some(word),
                ..
            } => {
                half_parts(pc, inst, name, *word)?;
                let (a, b) = self.sdwa_sources(pc, inst, *word, Some(*src1))?;
                let b = b.ok_or_else(|| refused(pc, inst))?;
                let condition = self.compared(pc, inst, name, a, b)?;
                let dst = match (word >> 15) & 1 {
                    0 => VCC,
                    _ => ((word >> 8) & 0x7F) as u8,
                };
                self.put_compare(name, dst, condition);
                Ok(())
            }
            Inst::Vopc { src0, src1, .. } if name.ends_with("_u64") || name.ends_with("_i64") => {
                let condition = self.wide_compare(pc, inst, name, *src0, Operand::Vgpr(*src1))?;
                self.put_compare(name, VCC, condition);
                Ok(())
            }
            Inst::Vopc { src0, src1, .. } => {
                let a = match src0 {
                    Operand::Float(word) if name.ends_with("_f16") => self.b.u32(half_word(*word)),
                    _ => self.sv(pc, *src0)?,
                };
                let b = self.vreg(pc, *src1)?;
                let condition = self.compared(pc, inst, name, a, b)?;
                self.put_compare(name, VCC, condition);
                Ok(())
            }
            Inst::Vop3 { dst, srcs, .. }
                if name.starts_with("v_interp_p") && name.ends_with("_f16") =>
            {
                self.interp_half(pc, inst, name, *dst, srcs[0])
            }
            Inst::Vop3 {
                op,
                dst,
                srcs,
                abs,
                neg,
                opsel,
                clamp,
                omod,
            } => self.vop3(
                pc,
                inst,
                name,
                *dst,
                *srcs,
                (*abs, *neg, *opsel),
                (*clamp, *omod),
                usize::from(op.sources()),
            ),
            Inst::Vop3p {
                dst,
                srcs,
                opsel_lo,
                opsel_hi,
                neg_lo,
                neg_hi,
                clamp,
                ..
            } if matches!(
                name,
                "v_fma_mix_f32" | "v_fma_mixlo_f16" | "v_fma_mixhi_f16"
            ) =>
            {
                self.fma_mix(
                    pc,
                    name,
                    *dst,
                    *srcs,
                    (*opsel_lo, *opsel_hi),
                    (*neg_lo, *neg_hi),
                    *clamp,
                )
            }
            Inst::Vop3p {
                dst,
                srcs,
                opsel_lo,
                opsel_hi,
                neg_lo,
                neg_hi,
                clamp,
                ..
            } if matches!(name, "v_pk_lshlrev_b16" | "v_pk_lshrrev_b16") => {
                // Each half of the second source moved by the low four bits of a half of the first.
                let (count, value) = (self.sv(pc, srcs[0])?, self.sv(pc, srcs[1])?);
                let (low, most, sixteen) = (self.b.u32(0xFFFF), self.b.u32(15), self.b.u32(16));
                let mut halves = Vec::with_capacity(2);
                for select in [*opsel_lo, *opsel_hi] {
                    let mut parts = [count, value];
                    for (i, part) in parts.iter_mut().enumerate() {
                        if (select >> i) & 1 != 0 {
                            *part = self.b.bin(Bo::ShiftRight, *part, sixteen);
                        }
                        *part = self.b.bin(Bo::And, *part, low);
                    }
                    let by = self.b.bin(Bo::And, parts[0], most);
                    halves.push(match name {
                        "v_pk_lshrrev_b16" => self.b.bin(Bo::ShiftRight, parts[1], by),
                        _ => {
                            let moved = self.b.bin(Bo::ShiftLeft, parts[1], by);
                            self.b.bin(Bo::And, moved, low)
                        }
                    });
                }
                let high = self.b.bin(Bo::ShiftLeft, halves[1], sixteen);
                let packed = self.b.bin(Bo::InclusiveOr, halves[0], high);
                self.vput(*dst, packed);
                Ok(())
            }
            Inst::Vop3p {
                dst,
                srcs,
                opsel_lo,
                opsel_hi,
                neg_lo,
                neg_hi,
                clamp,
                ..
            } if name.starts_with("v_pk_") && name.ends_with("_f16") => self.pk_half(
                pc,
                inst,
                *dst,
                *srcs,
                [(*opsel_lo, *neg_lo), (*opsel_hi, *neg_hi)],
                *clamp,
            ),
            Inst::Vintrp {
                dst, attr, chan, ..
            } => self.vintrp(pc, inst, name, *dst, *attr, *chan),
            Inst::Exp {
                target,
                en,
                srcs,
                compr,
                ..
            } => self.exp(pc, inst, *target, *en, *srcs, *compr),
            Inst::Ds {
                addr,
                dst,
                offset0,
                offset1,
                ..
            } if name == "ds_swizzle_b32" => {
                // The 16-bit pattern's top bit says it permutes each quad's four lanes, two bits a lane.
                if offset1 & 0x80 == 0 {
                    let pattern = u32::from(*offset0) | (u32::from(*offset1) << 8);
                    return self.swizzle_bits(pc, inst, *dst, *addr, pattern);
                }
                let quad = [0, 2, 4, 6].map(|low| offset0 >> low & 3);
                self.permute(pc, inst, *dst, *addr, quad)
            }
            Inst::Ds { .. } if self.pair.is_some() => self.pair_memory(pc, inst, name),
            Inst::Ds { .. } => self.shared_memory(pc, inst, name),
            Inst::Mimg { .. } => self.image(pc, inst, name),
            Inst::Mubuf { .. } => self.mubuf(pc, inst, name),
            Inst::Mtbuf { .. } => self.mtbuf(pc, inst, name),
            Inst::Flat { .. } => self.flat(pc, inst, name),
            _ => Err(refused(pc, inst)),
        }
    }

    /// Lowers one block alone: every value crosses blocks in a local, so its statements stand by themselves.
    fn block(
        &mut self,
        index: u32,
        block: &super::cfg::Block,
        state: State,
    ) -> Result<Lowering, NotTranslated> {
        self.index = index;
        self.state = state;
        self.used.clear();
        self.defined.clear();
        self.versions.clear();
        self.permuted.clear();
        self.waved.clear();
        self.shared_wave.masks.clear();
        self.carry = None;
        self.b.open();
        self.b.open();
        for (pc, inst) in &block.insts {
            self.shared_wave.begin();
            self.inst(*pc, inst)?;
            sym::step_at(&mut self.state, *pc, inst, self.program.wave32);
            // Between two instructions nothing is held but in locals, a sum's carry aside.
            if self.carry.is_none() && self.b.written() >= RUN {
                self.b.close_run();
                self.b.open();
            }
        }
        self.swizzles_past(block);
        // A neighbour's value still held where the block ends may be read by another block.
        let left = (self.permuted.keys()).map(|&n| (index, n));
        self.rebuilt.extend(left.filter(|_| self.fragment));
        self.b.close_run();
        Ok(Lowering {
            block: self.b.close(),
            used: std::mem::take(&mut self.used),
            defined: std::mem::take(&mut self.defined),
            maybe: std::mem::take(&mut self.maybe),
        })
    }

    /// A statement that leaves or reruns a loop: itself inside a loop of this function, else
    /// the word of it left for the function that called this one.
    fn leave_loop(&mut self, place: Place, statement: Statement, ended: u32) -> Done {
        match (place.loops > 0, place.moved) {
            (true, _) => self.b.push(statement),
            (false, true) => {
                let ended = self.b.u32(ended);
                self.put(ENDED, ended);
                self.b.push(Statement::Return { value: None });
            }
            (false, false) => {
                return Err(NotTranslated::Structure(
                    "a loop is left where none is open".to_owned(),
                ))
            }
        }
        Ok(())
    }

    /// `if condition { inside }`, the statements `inside` writes.
    fn when(&mut self, condition: Handle<E>, inside: impl FnOnce(&mut Self) -> Done) -> Done {
        self.b.open();
        let written = inside(self);
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition,
            accept,
            reject: Block::new(),
        });
        written
    }

    /// The structured statements of `nodes`, each block's own statements taken from `blocks`;
    /// `parts` holds what the blocks of each function statements were moved to were lowered in.
    fn flow(
        &mut self,
        nodes: &[Flow],
        blocks: &mut [Option<Block>],
        parts: &mut [Part],
        place: Place,
    ) -> Done {
        // A long list is written as blocks one after another, as a long run of a block's own
        // statements is: each construct reads its condition just before itself.
        if nodes.len() <= LIST {
            return self.flow_list(nodes, blocks, parts, place);
        }
        for list in nodes.chunks(LIST) {
            self.b.open();
            let written = self.flow_list(list, blocks, parts, place);
            self.b.close_run();
            written?;
        }
        Ok(())
    }

    fn flow_list(
        &mut self,
        nodes: &[Flow],
        blocks: &mut [Option<Block>],
        parts: &mut [Part],
        place: Place,
    ) -> Done {
        for node in nodes {
            match node {
                Flow::Code(block) => {
                    let statements = (blocks.get_mut(*block as usize))
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            NotTranslated::Structure(format!("block {block} is emitted twice"))
                        })?;
                    self.b.push(Statement::Block(statements));
                }
                Flow::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    let condition = self.raw(Reg::Cond(*cond));
                    let accept = self.nested(then, blocks, parts, place)?;
                    let reject = self.nested(otherwise, blocks, parts, place)?;
                    self.b.push(Statement::If {
                        condition,
                        accept,
                        reject,
                    });
                }
                Flow::Loop(body) => {
                    let inside = Place {
                        loops: place.loops + 1,
                        ..place
                    };
                    let body = self.nested(body, blocks, parts, inside)?;
                    self.b.push(Statement::Loop {
                        body,
                        continuing: Block::new(),
                        break_if: None,
                    });
                }
                Flow::Break => self.leave_loop(place, Statement::Break, ENDED_BREAK)?,
                Flow::Continue => self.leave_loop(place, Statement::Continue, ENDED_CONTINUE)?,
                Flow::Return => self.finish_lane_at(place),
                Flow::Leave(to) => {
                    let to = self.b.u32(to + 1);
                    self.put(LEAVING, to);
                }
                Flow::Arrive(to) => {
                    let (held, this) = (self.raw(LEAVING), self.b.u32(to + 1));
                    let here = self.b.bin(Bo::Equal, held, this);
                    self.when(here, |cx| {
                        let nowhere = cx.b.u32(0);
                        cx.put(LEAVING, nowhere);
                        Ok(())
                    })?;
                }
                Flow::Guard(body) => {
                    let (held, nowhere) = (self.raw(LEAVING), self.b.u32(0));
                    let condition = self.b.bin(Bo::Equal, held, nowhere);
                    let accept = self.nested(body, blocks, parts, place)?;
                    self.b.push(Statement::If {
                        condition,
                        accept,
                        reject: Block::new(),
                    });
                }
                Flow::Unwind(loops) => {
                    let loops = self.b.u32(*loops);
                    self.put(LOOPS_LEFT, loops);
                }
                Flow::Unwound => {
                    let left = self.raw(LOOPS_LEFT);
                    let any = self.b.ne0(left);
                    self.when(any, |cx| {
                        let one = cx.b.u32(1);
                        let fewer = cx.b.bin(Bo::Subtract, left, one);
                        cx.put(LOOPS_LEFT, fewer);
                        cx.leave_loop(place, Statement::Break, ENDED_BREAK)
                    })?;
                }
                Flow::Again => {
                    let one = self.b.u32(1);
                    self.put(AGAIN, one);
                }
                Flow::Resume => {
                    let again = self.raw(AGAIN);
                    let asked = self.b.ne0(again);
                    self.when(asked, |cx| {
                        let zero = cx.b.u32(0);
                        cx.put(AGAIN, zero);
                        cx.leave_loop(place, Statement::Continue, ENDED_CONTINUE)
                    })?;
                }
                Flow::Call(number, body) => self.moved(*number, body, blocks, parts, place)?,
            }
        }
        Ok(())
    }

    /// Statements moved to a function of their own: the function, its call, and after the call
    /// whatever it left for its caller to do, a loop to leave or run again or the lane to end.
    fn moved(
        &mut self,
        number: usize,
        body: &[Flow],
        blocks: &mut [Option<Block>],
        parts: &mut [Part],
        place: Place,
    ) -> Done {
        let missing = || NotTranslated::Structure(format!("function {number} has no blocks"));
        let part = parts.get_mut(number).ok_or_else(missing)?;
        std::mem::swap(&mut self.b, &mut part.0);
        std::mem::swap(&mut self.locals, &mut part.1);
        let inside = Place {
            loops: 0,
            moved: true,
        };
        let written = self.flow(body, blocks, parts, inside);
        let part = parts.get_mut(number).ok_or_else(missing)?;
        let builder = std::mem::replace(&mut self.b, std::mem::take(&mut part.0));
        std::mem::swap(&mut self.locals, &mut part.1);
        written?;
        let mut function = builder.finish();
        function.name = Some(format!("part{number}"));
        let function = self.module.functions.append(function, Span::UNDEFINED);
        self.b.push(Statement::Call {
            function,
            arguments: Vec::new(),
            result: None,
        });
        let ended = self.raw(ENDED);
        if place.loops == 0 {
            // No loop of this function is open: whatever the call left is this one's caller's too.
            let any = self.b.ne0(ended);
            return self.when(any, |cx| {
                cx.b.push(Statement::Return { value: None });
                Ok(())
            });
        }
        for (how, statement) in [
            (ENDED_BREAK, Statement::Break),
            (ENDED_CONTINUE, Statement::Continue),
        ] {
            let how = self.b.u32(how);
            let so = self.b.bin(Bo::Equal, ended, how);
            self.when(so, |cx| {
                let zero = cx.b.u32(0);
                cx.put(ENDED, zero);
                cx.b.push(statement);
                Ok(())
            })?;
        }
        let how = self.b.u32(ENDED_RETURN);
        let so = self.b.bin(Bo::Equal, ended, how);
        self.when(so, |cx| {
            cx.b.push(Statement::Return { value: None });
            Ok(())
        })
    }

    fn nested(
        &mut self,
        nodes: &[Flow],
        blocks: &mut [Option<Block>],
        parts: &mut [Part],
        place: Place,
    ) -> Result<Block, NotTranslated> {
        self.b.open();
        let lowered = self.flow(nodes, blocks, parts, place);
        let block = self.b.close();
        lowered.map(|()| block)
    }

    /// The axis of its group's number a compute program finds in scalar register `r`.
    fn group_axis(&self, r: u8) -> Option<u32> {
        let launch = self.launch?;
        let nth = r.checked_sub(launch.user_registers)?;
        (0..3u32)
            .filter(|axis| (launch.group_axes >> axis) & 1 != 0)
            .nth(usize::from(nth))
    }

    /// One axis of a compute thread's number (`which` 0) or of its group's (1).
    fn launch_number(&mut self, which: usize, axis: u32) -> Handle<E> {
        let Some(ids) = self.ids else {
            return self.b.u32(0);
        };
        let pointer = self.global(ids[which]);
        let cell = self.b.index(pointer, axis);
        self.b.load(cell)
    }

    /// Runs what `inside` writes in an active lane only, where the program masks lanes.
    fn when_active(&mut self, inside: impl FnOnce(&mut Self) -> Done) -> Done {
        let active = self.masked.then(|| self.active());
        if active.is_some() {
            self.b.open();
        }
        let done = inside(self);
        if let Some(condition) = active {
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition,
                accept,
                reject: Block::new(),
            });
        }
        done
    }

    /// The dwords a compute group's threads share.
    fn shared_words(&mut self, launch: super::Launch) -> Handle<naga::GlobalVariable> {
        if let Some(global) = self.shared {
            return global;
        }
        let base = self.ty(TypeInner::Scalar(Scalar::U32));
        let dwords = NonZeroU32::new(launch.shared_dwords).unwrap_or(NonZeroU32::MIN);
        let ty = self.ty(TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(dwords),
            stride: 4,
        });
        let variable = naga::GlobalVariable {
            name: Some("shared".to_owned()),
            space: naga::AddressSpace::WorkGroup,
            binding: None,
            ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        };
        let global = (self.module.global_variables).append(variable, Span::UNDEFINED);
        self.shared = Some(global);
        global
    }

    /// Stores each value through its pointer, in an active lane only where the program masks lanes.
    fn guarded(&mut self, stores: Vec<(Handle<E>, Handle<E>)>) {
        let active = self.masked.then(|| self.active());
        if active.is_some() {
            self.b.open();
        }
        for (pointer, value) in stores {
            self.b.push(Statement::Store { pointer, value });
        }
        if let Some(condition) = active {
            let accept = self.b.close();
            self.b.push(Statement::If {
                condition,
                accept,
                reject: Block::new(),
            });
        }
    }

    /// The memory a compute group's threads share: dwords read and written at a byte address,
    /// one or two an instruction, the two-address forms a dword or 64 dwords apart a step.
    fn shared_memory(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Ds {
            addr,
            data0,
            data1,
            dst,
            offset0,
            offset1,
            gds,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let Some(launch) = self.launch else {
            return Err(match self.mesh {
                Some(_) => form(
                    pc,
                    inst,
                    "the memory a group's threads share, in a program run a thread at a time",
                ),
                None => refused(pc, inst),
            });
        };
        if *gds {
            return Err(form(pc, inst, "the memory every group shares"));
        }
        let whole = u32::from(*offset0) | (u32::from(*offset1) << 8);
        // Each lane takes the value of the lane its address numbers, four bytes a lane.
        if name == "ds_bpermute_b32" {
            let (bytes, value) = (self.vreg(pc, *addr)?, self.vreg(pc, *data0)?);
            let (by, two) = (self.b.u32(whole), self.b.u32(2));
            let bytes = self.b.bin(Bo::Add, bytes, by);
            let lane = self.b.bin(Bo::ShiftRight, bytes, two);
            let value = self.gather(value, lane);
            self.vput(*dst, value);
            return Ok(());
        }
        if let Some((fun, returns)) = shared_update(name) {
            let global = self.shared_words(launch);
            let (base, by, two) = (self.vreg(pc, *addr)?, self.b.u32(whole), self.b.u32(2));
            let bytes = self.b.bin(Bo::Add, base, by);
            let index = self.b.bin(Bo::ShiftRight, bytes, two);
            // A compare-and-store looks for its first datum and stores its second.
            let (value, compare) = match fun {
                "cmpswap" => (self.vreg(pc, *data1)?, Some(self.vreg(pc, *data0)?)),
                _ => (self.vreg(pc, *data0)?, None),
            };
            let signed = match fun {
                "smin" => Some(false),
                "smax" => Some(true),
                _ => None,
            };
            let fun = buffers::atomic_function(fun, compare);
            if fun.is_none() && signed.is_none() {
                return Err(refused(pc, inst));
            }
            self.shared_atomic = true;
            let dst = *dst;
            return self.when_active(|cx| {
                let pointer = cx.global(global);
                let cell = cx.b.emit(E::Access {
                    base: pointer,
                    index,
                });
                let old = match (fun, signed) {
                    (Some(fun), _) => cx.atomic(cell, fun, value),
                    (None, greatest) => cx.atomic_signed(cell, greatest == Some(true), value),
                };
                if returns {
                    cx.vput(dst, old);
                }
                Ok(())
            });
        }
        // A byte or a 16-bit part of a dword. A store clears the part and sets it in two
        // updates, so a thread that stores the dword's other part at once loses nothing.
        // The `d16` forms read into one half of the register and keep the other, and store
        // from its high half.
        let (plain, half) = match name.split_once("_d16") {
            Some((plain, "")) => (plain, Some(false)),
            Some((plain, "_hi")) => (plain, Some(true)),
            _ => (name, None),
        };
        let part = match plain {
            "ds_read_u8" => Some((false, 8)),
            "ds_read_u16" => Some((false, 16)),
            "ds_write_b8" => Some((true, 8)),
            "ds_write_b16" => Some((true, 16)),
            _ => None,
        };
        if let Some((write, bits)) = part {
            let global = self.shared_words(launch);
            let (base, by) = (self.vreg(pc, *addr)?, self.b.u32(whole));
            let bytes = self.b.bin(Bo::Add, base, by);
            let (two, three, eight) = (self.b.u32(2), self.b.u32(3), self.b.u32(8));
            let index = self.b.bin(Bo::ShiftRight, bytes, two);
            let within = self.b.bin(Bo::And, bytes, three);
            let shift = self.b.bin(Bo::Multiply, within, eight);
            let ones = self.b.u32((1 << bits) - 1);
            if !write {
                let pointer = self.global(global);
                let cell = self.b.emit(E::Access {
                    base: pointer,
                    index,
                });
                let word = self.b.load(cell);
                let moved = self.b.bin(Bo::ShiftRight, word, shift);
                let mut value = self.b.bin(Bo::And, moved, ones);
                if let Some(high) = half {
                    let (old, sixteen) = (self.vreg(pc, *dst)?, self.b.u32(16));
                    let kept = self.b.u32(if high { 0xFFFF } else { 0xFFFF_0000 });
                    let kept = self.b.bin(Bo::And, old, kept);
                    if high {
                        value = self.b.bin(Bo::ShiftLeft, value, sixteen);
                    }
                    value = self.b.bin(Bo::InclusiveOr, kept, value);
                }
                self.vput(*dst, value);
                return Ok(());
            }
            let mut data = self.vreg(pc, *data0)?;
            if half == Some(true) {
                let sixteen = self.b.u32(16);
                data = self.b.bin(Bo::ShiftRight, data, sixteen);
            }
            let kept = self.b.bin(Bo::And, data, ones);
            let (set, mask) = (
                self.b.bin(Bo::ShiftLeft, kept, shift),
                self.b.bin(Bo::ShiftLeft, ones, shift),
            );
            let hole = self.b.not(mask);
            self.shared_atomic = true;
            return self.when_active(|cx| {
                let pointer = cx.global(global);
                let cell = cx.b.emit(E::Access {
                    base: pointer,
                    index,
                });
                cx.atomic(cell, naga::AtomicFunction::And, hole);
                cx.atomic(cell, naga::AtomicFunction::InclusiveOr, set);
                Ok(())
            });
        }
        let (write, unit) = match name {
            "ds_read_b32" => (false, None),
            "ds_write_b32" => (true, None),
            "ds_read2_b32" => (false, Some(4)),
            "ds_write2_b32" => (true, Some(4)),
            "ds_read2st64_b32" => (false, Some(256)),
            "ds_write2st64_b32" => (true, Some(256)),
            _ => return Err(refused(pc, inst)),
        };
        let offsets = match unit {
            None => vec![u32::from(*offset0) | (u32::from(*offset1) << 8)],
            Some(unit) => vec![u32::from(*offset0) * unit, u32::from(*offset1) * unit],
        };
        let global = self.shared_words(launch);
        let base = self.vreg(pc, *addr)?;
        let mut cells = Vec::with_capacity(2);
        for offset in offsets {
            let (by, two) = (self.b.u32(offset), self.b.u32(2));
            let bytes = self.b.bin(Bo::Add, base, by);
            let index = self.b.bin(Bo::ShiftRight, bytes, two);
            let pointer = self.global(global);
            cells.push(self.b.emit(E::Access {
                base: pointer,
                index,
            }));
        }
        if write {
            let mut stores = Vec::with_capacity(2);
            for (cell, data) in cells.into_iter().zip([*data0, *data1]) {
                stores.push((cell, self.vreg(pc, data)?));
            }
            self.guarded(stores);
            return Ok(());
        }
        // Both are read before either register is written: the address may be one of them.
        let loaded: Vec<_> = cells.into_iter().map(|cell| self.b.load(cell)).collect();
        for (k, value) in loaded.into_iter().enumerate() {
            self.vput(dst.wrapping_add(k as u8), value);
        }
        Ok(())
    }

    /// Gives each register the entry reads before any write its value, or refuses the read.
    /// `handed` are the registers some read finds unwritten on every path: any other one is
    /// unwritten only on paths that switch its lane off, and reads as zero there.
    fn live_ins(&mut self, live: &BTreeMap<Reg, u32>, handed: &BTreeSet<Reg>) -> Done {
        let (one, zero) = (self.b.u32(1), self.b.u32(0));
        self.put(Reg::S(EXEC), one);
        self.put(Reg::S(EXEC + 1), zero);
        for (&reg, &pc) in live {
            if let Some(value) = self.pair_live_in(reg, pc, handed)? {
                self.put(reg, value);
                continue;
            }
            match reg {
                Reg::S(r) if r == EXEC || r == EXEC + 1 => {}
                // A mesh thread's launch number is its group's first plus its own; the wave it is in
                // sits in the top byte of the third scalar register.
                Reg::V(5) | Reg::S(3) if self.mesh.is_some() => {
                    let Some(mesh) = self.mesh else { continue };
                    let (thread, group) = self.mesh_ids(mesh);
                    let value = match reg {
                        Reg::V(_) => {
                            let threads = self.b.u32(MESH_THREADS);
                            let first = self.b.bin(Bo::Multiply, group, threads);
                            self.b.bin(Bo::Add, first, thread)
                        }
                        _ => {
                            let (lanes, top) = (self.b.u32(WAVE.trailing_zeros()), self.b.u32(24));
                            let wave = self.b.bin(Bo::ShiftRight, thread, lanes);
                            self.b.bin(Bo::ShiftLeft, wave, top)
                        }
                    };
                    self.put(reg, value);
                }
                // A compute thread's number in its group, an axis a register, and its group's
                // number in the scalar registers after the user's.
                Reg::V(n) if self.ids.is_some() && n < 3 => {
                    let value = self.launch_number(0, u32::from(n));
                    self.put(reg, value);
                }
                Reg::S(r) if self.group_axis(r).is_some() => {
                    let axis = self.group_axis(r).unwrap_or(0);
                    let value = self.launch_number(1, axis);
                    self.put(reg, value);
                }
                Reg::S(r) if r < USER_REGISTERS => {
                    let global = match self.user_data {
                        Some(global) => global,
                        None => {
                            let inner = self.rows(u32::from(USER_REGISTERS) / 4);
                            let global = self.bound(
                                "user_data".to_owned(),
                                inner,
                                naga::AddressSpace::Uniform,
                                ResourceKind::UserData,
                                None,
                            );
                            self.user_data = Some(global);
                            global
                        }
                    };
                    let pointer = self.global(global);
                    let row = self.b.index(pointer, u32::from(r) / 4);
                    let cell = self.b.index(row, u32::from(r) % 4);
                    let value = self.b.load(cell);
                    self.put(reg, value);
                }
                Reg::V(n) if self.fragment && n >= SYSTEM_BASE => {
                    let value = match self.system_input(n - SYSTEM_BASE) {
                        Some(value) => value,
                        None if !handed.contains(&reg) => zero,
                        None => {
                            return Err(NotTranslated::LiveIn {
                                pc,
                                register: reg.name(),
                            })
                        }
                    };
                    self.put(reg, value);
                }
                // The pair a pixel interpolates by, read as numbers: its weights of the second
                // and the third vertex of its triangle.
                Reg::V(n) if self.fragment && n < SYSTEM_BASE => {
                    let global = match self.barycentric {
                        Some(global) => global,
                        None => {
                            let ty = vector(&mut self.module, VectorSize::Tri, Scalar::F32);
                            let global = self.private("barycentric".to_owned(), ty);
                            self.barycentric = Some(global);
                            global
                        }
                    };
                    let pointer = self.global(global);
                    let cell = self.b.index(pointer, u32::from(n) + 1);
                    let weight = self.b.load(cell);
                    let value = self.b.bits(weight);
                    self.put(reg, value);
                    self.note(
                        "the pair a pixel interpolates by read as its weights of two vertices",
                    );
                }
                // A vertex program's first and fourth vector registers: the vertex's number and the instance's.
                Reg::V(0 | INSTANCE) if self.vertex => {
                    let held = match reg {
                        Reg::V(0) => self.vertex_index,
                        _ => self.instance_index,
                    };
                    let global = match held {
                        Some(global) => global,
                        None => {
                            let ty = self.ty(TypeInner::Scalar(Scalar::U32));
                            match reg {
                                Reg::V(0) => {
                                    let global = self.private("vertex_index".to_owned(), ty);
                                    self.vertex_index = Some(global);
                                    global
                                }
                                _ => {
                                    let global = self.private("instance_index".to_owned(), ty);
                                    self.instance_index = Some(global);
                                    global
                                }
                            }
                        }
                    };
                    let pointer = self.global(global);
                    let value = self.b.load(pointer);
                    self.put(reg, value);
                }
                _ if !handed.contains(&reg) => self.put(reg, zero),
                // A compute thread is handed its numbers and nothing else: any other vector
                // register it reads unwritten holds what the hardware left there, such as the
                // components a four-component store names past the ones its buffer keeps.
                Reg::V(_) if self.ids.is_some() => self.put(reg, zero),
                _ => {
                    return Err(NotTranslated::LiveIn {
                        pc,
                        register: reg.name(),
                    })
                }
            }
        }
        Ok(())
    }
}

/// A set of the program's registers, a bit each by its place among them.
type Bits = Vec<u64>;

/// Adds to `into` what `from` holds and `without` does not; whether that added anything.
fn add_bits(into: &mut Bits, from: &Bits, without: Option<&Bits>) -> bool {
    let mut changed = false;
    for (k, (slot, &more)) in into.iter_mut().zip(from).enumerate() {
        let new = *slot | (more & !without.map_or(0, |bits| bits[k]));
        changed |= new != *slot;
        *slot = new;
    }
    changed
}

/// Whether a block after `block` reads `reg` as that block left it.
fn read_after(lowered: &[Lowering], successors: &[Vec<usize>], block: usize, reg: Reg) -> bool {
    let mut work: Vec<usize> = successors.get(block).cloned().unwrap_or_default();
    let mut seen = BTreeSet::new();
    while let Some(at) = work.pop() {
        let Some(l) = lowered.get(at).filter(|_| seen.insert(at)) else {
            continue;
        };
        if l.used.contains_key(&reg) {
            return true;
        }
        if !l.defined.contains(&reg) {
            work.extend(successors.get(at).into_iter().flatten());
        }
    }
    false
}

/// What the blocks' sets say of the registers the program is given: `handed` are those some
/// block reads with no write on any path to the read (a read that only some paths leave
/// unwritten is of a lane those paths switched off), and `live` those the entry may read before
/// anything writes them, each with the first word that reads it.
// Each block is taken up again only when a set that reaches it grew, and a set is a word a
// 64 registers: the sets of every block of a long program settle in a pass or two.
fn register_flow(
    lowered: &[Lowering],
    successors: &[Vec<usize>],
) -> (BTreeSet<Reg>, BTreeMap<Reg, u32>) {
    let regs: Vec<Reg> = (lowered.iter())
        .flat_map(|l| l.used.keys().chain(&l.defined).chain(&l.maybe))
        .copied()
        .collect::<BTreeSet<Reg>>()
        .into_iter()
        .collect();
    let bits = |of: &mut dyn Iterator<Item = &Reg>| {
        let mut set: Bits = vec![0; regs.len().div_ceil(64)];
        for reg in of {
            if let Ok(place) = regs.binary_search(reg) {
                set[place / 64] |= 1 << (place % 64);
            }
        }
        set
    };
    let has = |set: &Bits, reg: &Reg| {
        (regs.binary_search(reg)).is_ok_and(|place| set[place / 64] >> (place % 64) & 1 != 0)
    };
    let used: Vec<Bits> = (lowered.iter().map(|l| bits(&mut l.used.keys()))).collect();
    let defined: Vec<Bits> = (lowered.iter().map(|l| bits(&mut l.defined.iter()))).collect();
    let writes: Vec<Bits> = (lowered.iter())
        .map(|l| bits(&mut l.defined.iter().chain(&l.maybe)))
        .collect();

    // What some path to a block's entry wrote.
    let mut written: Vec<Bits> = vec![bits(&mut std::iter::empty()); lowered.len()];
    let mut work: BTreeSet<usize> = (0..lowered.len()).collect();
    while let Some(block) = work.pop_first() {
        let mut out = written[block].clone();
        add_bits(&mut out, &writes[block], None);
        for &next in &successors[block] {
            if add_bits(&mut written[next], &out, None) {
                work.insert(next);
            }
        }
    }
    let handed = (lowered.iter().zip(&written))
        .flat_map(|(l, written)| l.used.keys().filter(move |reg| !has(written, reg)))
        .copied()
        .collect();

    // What a block's entry may read before anything writes it.
    let mut before = vec![Vec::new(); lowered.len()];
    for (block, nexts) in successors.iter().enumerate() {
        for &next in nexts {
            before[next].push(block);
        }
    }
    let mut live = used;
    let mut work: BTreeSet<usize> = (0..lowered.len()).collect();
    while let Some(block) = work.pop_last() {
        let reads = live[block].clone();
        for &from in &before[block] {
            if add_bits(&mut live[from], &reads, Some(&defined[from])) {
                work.insert(from);
            }
        }
    }
    let mut first: BTreeMap<Reg, u32> = BTreeMap::new();
    for l in lowered.iter().rev() {
        for (&reg, &pc) in &l.used {
            if live.first().is_some_and(|entry| has(entry, &reg)) {
                // The earliest block's read names the register.
                first.insert(reg, pc);
            }
        }
    }
    (handed, first)
}

/// Whether any instruction of the graph changes the lane mask.
fn masks_lanes(cfg: &Cfg) -> bool {
    cfg.blocks.values().flat_map(|b| &b.insts).any(|(_, inst)| {
        let name = inst.name();
        let writes_exec = match inst {
            Inst::Sop1 { dst, .. } | Inst::Sop2 { dst, .. } | Inst::Sopk { dst, .. } => {
                matches!(dst, Operand::ExecLo | Operand::ExecHi)
            }
            _ => false,
        };
        writes_exec || name.starts_with("v_cmpx") || name.contains("exec")
    })
}

/// Every lane of a vector register an instruction of the graph writes a scalar into.
fn spilled(cfg: &Cfg) -> BTreeSet<Reg> {
    (cfg.blocks.values().flat_map(|b| &b.insts))
        .filter_map(|(_, inst)| match inst {
            Inst::Vop3 { dst, srcs, .. } if inst.name() == "v_writelane_b32" => match srcs[1] {
                Operand::Int(lane) => Some(Reg::Spill(*dst, lane as u8)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The deepest nest of ifs and loops a module is given. naga's validator and its writers
/// recurse a frame for each, and a frame of an unoptimized build is tens of kilobytes: 24
/// nested ifs validate on a first thread's megabyte and 32 overflow it.
const MAX_NESTING: usize = 16;

/// How deep a function nests where a program's statements are spread over several: what is
/// left of [`MAX_NESTING`] is for the statements a block itself holds.
const SPREAD_NESTING: usize = 12;

/// The longest program lowered. A module costs time and memory by the word: a made-up program
/// of sums, products and a branch every 24 words takes about 18 microseconds and 1.6 kilobytes
/// a word to lower and validate in an unoptimized build, so five seconds and 420 megabytes at
/// this bound and twice both at the next power of two; a file may state more.
const MAX_WORDS: usize = 1 << 18;
const TOO_LONG: &str = "a program of more than 262,144 words: longer than any the game ships, \
    and a module of it would take this front end more time and memory than a file is owed";

/// How a block hands control on, by block number.
#[derive(Debug, Clone, Copy)]
enum End {
    Return,
    Jump(u32),
    Branch { then: u32, otherwise: u32 },
}

/// The naga module of a pixel or vertex program that stands alone.
pub fn lower(program: &Program, cfg: &Cfg) -> Result<Lowered, NotTranslated> {
    lower_with(program, cfg, &Options::default())
}

/// The naga module of a pixel or vertex program, told what its neighbours need of it.
pub fn lower_with(
    program: &Program,
    cfg: &Cfg,
    options: &Options,
) -> Result<Lowered, NotTranslated> {
    if program.words.len() > MAX_WORDS {
        return Err(NotTranslated::Pipeline(TOO_LONG));
    }
    // Half of a geometry pair, given the other: the two joined into one program.
    if let Some(joined) = (options.pair.as_deref()).and_then(|pair| geometry::joined(program, pair))
    {
        let (program, split) = joined?;
        let cfg =
            Cfg::build(&program.words).map_err(|why| NotTranslated::Structure(why.to_string()))?;
        return lower_as(&program, &cfg, options, Some(&split));
    }
    lower_as(program, cfg, options, None)
}

/// [`lower_with`], of a program that may be the joined halves of a geometry pair.
fn lower_as(
    program: &Program,
    cfg: &Cfg,
    options: &Options,
    split: Option<&geometry::Split>,
) -> Result<Lowered, NotTranslated> {
    let (mut told, mut eager, mut shared) = (StoredImages::new(), false, None);
    loop {
        match lower_pass(program, cfg, options, (&told, eager, shared), split) {
            // A compute group that is one wave, lowered onto its device's subgroup: again,
            // with the wave in the memory the group shares, which every device runs alike.
            Ok(Pass::Done(lowered)) => {
                let again = lowered.subgroups && !options.wave && shared.is_none();
                match wave::one_wave(program).filter(|_| again) {
                    Some(threads) => shared = Some(threads),
                    None => return Ok(*lowered),
                }
            }
            // Each round names an image no earlier one did, so the rounds end.
            Ok(Pass::Again(more)) => told.extend(more),
            Err(NotTranslated::Descriptor {
                what: MIXED_NUMBER, ..
            }) if !eager => eager = true,
            Err(stopped) => return Err(stopped),
        }
    }
}

/// What stops a lowering that reads a table's words only where a descriptor is used: a
/// register that holds a table's word on some paths, computed with as a number.
const MIXED_NUMBER: &str = "a table's word as a number";

/// One lowering, which reads the images `told` names through the bindings they are stored through.
#[allow(clippy::too_many_lines)]
fn lower_pass(
    program: &Program,
    cfg: &Cfg,
    options: &Options,
    (told, eager, shared): (&StoredImages, bool, Option<u32>),
    split: Option<&geometry::Split>,
) -> Result<Pass, NotTranslated> {
    let stage = match program.kind {
        ShaderKind::Pixel => naga::ShaderStage::Fragment,
        // A mesh program runs as a vertex program: its body once for a primitive and once for the
        // vertex at this corner of it, since a thread places one of each.
        ShaderKind::Vertex | ShaderKind::Mesh => naga::ShaderStage::Vertex,
        ShaderKind::Compute if program.launch.is_some_and(|launch| !launch.group_size) => {
            naga::ShaderStage::Compute
        }
        ShaderKind::Geometry => return Err(NotTranslated::Pipeline(GEOMETRY)),
        other => return Err(NotTranslated::Stage(format!("{other:?}").to_lowercase())),
    };
    let fragment = stage == naga::ShaderStage::Fragment;
    if (cfg.blocks.values().flat_map(|b| &b.insts))
        .any(|(_, inst)| inst.name().starts_with("image_bvh"))
    {
        return Err(NotTranslated::Pipeline(RAY_TRACING));
    }

    // How each block hands control on, by block number.
    let starts: Vec<u32> = cfg.blocks.keys().copied().collect();
    let place = |start: u32| starts.binary_search(&start).unwrap_or(0) as u32;
    let mut successors = Vec::with_capacity(starts.len());
    let mut ends = Vec::with_capacity(starts.len());
    for block in cfg.blocks.values() {
        let end = match block.terminator {
            Terminator::End => End::Return,
            Terminator::Jump(to) | Terminator::Fall(to) => End::Jump(place(to)),
            Terminator::Branch { taken, fallthrough } => End::Branch {
                then: place(taken),
                otherwise: place(fallthrough),
            },
            // A vertex program ends on a jump through a register only as half of a geometry
            // pair, where the jump is its return.
            Terminator::Indirect if program.kind == ShaderKind::Vertex => {
                return Err(NotTranslated::Pipeline(VERTEX_HALF));
            }
            Terminator::Indirect => {
                let pc = block.insts.last().map_or(block.start, |(pc, _)| *pc);
                return Err(NotTranslated::Indirect { pc });
            }
        };
        successors.push(match end {
            End::Jump(to) => vec![to as usize],
            End::Branch { then, otherwise } => vec![then as usize, otherwise as usize],
            End::Return => Vec::new(),
        });
        ends.push(end);
    }
    // A graph with a loop entered at two blocks still runs, block by block, in a dispatch loop.
    let (nodes, unstructured) = match flow::structure(&ends) {
        Ok(nodes) => (Some(nodes), None),
        Err(why) => (None, Some(why)),
    };

    let mut cx = Cx {
        program,
        module: naga::Module::default(),
        b: Builder::new(),
        fragment,
        vertex: program.kind == ShaderKind::Vertex,
        masked: masks_lanes(cfg),
        locals: HashMap::new(),
        spread: nodes
            .as_deref()
            .is_some_and(|nodes| flow::nesting(nodes) > MAX_NESTING),
        registers: HashMap::new(),
        index: 0,
        state: sym::entry(),
        used: BTreeMap::new(),
        defined: BTreeSet::new(),
        maybe: BTreeSet::new(),
        versions: HashMap::new(),
        permuted: HashMap::new(),
        rebuilt: Vec::new(),
        subgroups: stage == naga::ShaderStage::Compute || (fragment && options.wave),
        wave_ops: false,
        waved: HashMap::new(),
        carry: None,
        half_mode: None,
        lane: None,
        shared_wave: wave::Shared::of(shared),
        mesh: None,
        half_slots: &options.half_slots,
        attributes: BTreeMap::new(),
        halves: BTreeMap::new(),
        frag_coord: None,
        front_face: None,
        barycentric: None,
        fetched: BTreeMap::new(),
        vertex_index: None,
        instance_index: None,
        exports: BTreeMap::new(),
        depth: None,
        user_data: None,
        buffers: BTreeMap::new(),
        textures: BTreeMap::new(),
        samplers: BTreeMap::new(),
        storage: BTreeMap::new(),
        blocks: BTreeMap::new(),
        tables: BTreeMap::new(),
        targets: BTreeMap::new(),
        images_stored: StoredImages::new(),
        restated: StoredImages::new(),
        told,
        formats: &options.storage_formats,
        eager,
        pair: None,
        atomics: BTreeSet::new(),
        shared_atomic: false,
        code: None,
        data_start: (cfg.blocks.values().flat_map(|b| &b.insts))
            .map(|(pc, _)| pc + 1)
            .max()
            .unwrap_or(0),
        reads_code: (cfg.blocks.values().flat_map(|b| &b.insts))
            .any(|(_, inst)| inst.name() == "s_getpc_b64"),
        spills: spilled(cfg),
        launch: program.launch,
        ids: None,
        shared: None,
        stored: BTreeSet::new(),
        resources: Vec::new(),
        notes: BTreeMap::new(),
    };
    if (cfg.blocks.values().flat_map(|b| &b.insts)).any(|(_, inst)| writes_float_mode(inst)) {
        let flag = cx.ty(TypeInner::Scalar(Scalar::BOOL));
        cx.half_mode = Some(cx.private("halves_toward_zero".to_owned(), flag));
    }
    if program.kind == ShaderKind::Mesh {
        cx.shared_wave.plan(cfg);
        let word = cx.ty(TypeInner::Scalar(Scalar::U32));
        let flag = cx.ty(TypeInner::Scalar(Scalar::BOOL));
        cx.mesh = Some(Mesh {
            thread: cx.private("thread".to_owned(), word),
            group: cx.private("group".to_owned(), word),
            prim: cx.private("prim".to_owned(), word),
            prim_set: cx.private("prim_set".to_owned(), flag),
            pos_set: cx.private("pos_set".to_owned(), flag),
        });
    }
    if let Some(split) = split {
        cx.pair = Some(cx.pair_declare(split));
    }
    if program.launch.is_some() {
        let ty = vector(&mut cx.module, VectorSize::Tri, Scalar::U32);
        cx.ids = Some([
            cx.private("thread".to_owned(), ty),
            cx.private("group".to_owned(), ty),
        ]);
    }
    // A nest deeper than whoever reads the module is sure to follow is spread over functions,
    // none of which nests that deep; each block is lowered in the function it lands in.
    let mut functions = 1;
    let nodes = match cx.spread {
        true => nodes.map(|nodes| flow::outlined(nodes, SPREAD_NESTING, 0, &mut functions)),
        false => nodes,
    };
    let mut homes = vec![0usize; starts.len()];
    flow::homes(nodes.as_deref().unwrap_or_default(), 0, &mut homes);
    let mut parts: Vec<Part> = (0..functions)
        .map(|_| (Builder::new(), HashMap::new()))
        .collect();
    let mut states = sym::solve_for(cfg, program.wave32);
    let mut lowered = Vec::with_capacity(starts.len());
    for (index, (start, block)) in cfg.blocks.iter().enumerate() {
        let state = states.remove(start).unwrap_or_else(sym::entry);
        // The first function is the one the lowering holds; another is swapped in for its blocks.
        let part = parts.get_mut(homes[index]).filter(|_| homes[index] != 0);
        let Some(part) = part else {
            lowered.push(cx.block(index as u32, block, state)?);
            continue;
        };
        std::mem::swap(&mut cx.b, &mut part.0);
        std::mem::swap(&mut cx.locals, &mut part.1);
        let block = cx.block(index as u32, block, state);
        std::mem::swap(&mut cx.b, &mut part.0);
        std::mem::swap(&mut cx.locals, &mut part.1);
        lowered.push(block?);
    }
    // An image loaded as a texture that the program also stores to: its loads have to go
    // through the stored binding, which only another lowering can give them.
    let clash: StoredImages = (cx.images_stored.iter())
        .filter(|(at, _)| {
            !told.contains_key(*at) && cx.textures.keys().any(|(read, _, _)| read == *at)
        })
        .chain(&cx.restated)
        .map(|(at, atomic)| (at.clone(), *atomic))
        .collect();
    if !clash.is_empty() {
        return Ok(Pass::Again(clash));
    }
    cx.settle_swizzles(&successors)?;
    for (block, n) in std::mem::take(&mut cx.rebuilt) {
        if read_after(&lowered, &successors, block as usize, Reg::V(n)) {
            cx.note(REBUILT);
        }
    }
    let (handed, live) = register_flow(&lowered, &successors);
    // Nothing in the entry block is written yet, so the stores land before every block's statements.
    cx.used.clear();
    cx.defined.clear();
    cx.live_ins(&live, &handed)?;
    match nodes {
        Some(nodes) => {
            let mut blocks: Vec<Option<Block>> =
                lowered.into_iter().map(|l| Some(l.block)).collect();
            cx.flow(&nodes, &mut blocks, &mut parts, Place::default())?;
        }
        None => {
            cx.note(match unstructured {
                Some(flow::Unstructured::Shape) => {
                    "a dispatch loop for a graph nested deeper than a module is given"
                }
                _ => "a dispatch loop for a graph that does not nest",
            });
            cx.dispatch(&ends, lowered.into_iter().map(|l| l.block).collect());
        }
    }
    if stage == naga::ShaderStage::Vertex && !cx.exports.contains_key(&POS0) {
        return Err(NotTranslated::NoPosition);
    }

    let Cx {
        mut module,
        b,
        mesh,
        half_slots,
        attributes,
        halves,
        frag_coord,
        front_face,
        barycentric,
        fetched,
        vertex_index,
        instance_index,
        exports,
        depth,
        ids,
        stored,
        buffers,
        mut resources,
        notes,
        lane,
        wave_ops,
        atomics,
        shared,
        shared_atomic,
        pair,
        shared_wave,
        ..
    } = cx;
    // What the program updates in one step holds atomic dwords, which its other reads and
    // stores go through too.
    if !atomics.is_empty() || shared_atomic {
        let atomic = insert(&mut module, TypeInner::Atomic(Scalar::U32));
        let words = insert(
            &mut module,
            TypeInner::Array {
                base: atomic,
                size: naga::ArraySize::Dynamic,
                stride: 4,
            },
        );
        for global in atomics {
            module.global_variables[global].ty = words;
        }
        if let (true, Some(global)) = (shared_atomic, shared) {
            let dwords = program.launch.map_or(0, |launch| launch.shared_dwords);
            let ty = insert(
                &mut module,
                TypeInner::Array {
                    base: atomic,
                    size: naga::ArraySize::Constant(
                        NonZeroU32::new(dwords).unwrap_or(NonZeroU32::MIN),
                    ),
                    stride: 4,
                },
            );
            module.global_variables[global].ty = ty;
        }
    }
    // A constant buffer is declared as long as the furthest row any read of it reaches.
    for (at, (global, rows)) in buffers {
        let base = vector(&mut module, VectorSize::Quad, Scalar::U32);
        let ty = insert(
            &mut module,
            TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(NonZeroU32::new(rows).unwrap_or(NonZeroU32::MIN)),
                stride: 16,
            },
        );
        module.global_variables[global].ty = ty;
        for resource in &mut resources {
            if resource.at.as_ref() == Some(&at)
                && matches!(resource.kind, ResourceKind::ConstantBuffer { .. })
            {
                resource.kind = ResourceKind::ConstantBuffer { rows };
            }
        }
    }
    // A buffer the program stores to is bound to be written.
    for global in stored {
        let variable = &mut module.global_variables[global];
        variable.space = naga::AddressSpace::Storage {
            access: naga::StorageAccess::LOAD | naga::StorageAccess::STORE,
        };
        let place = variable
            .binding
            .as_ref()
            .map(|binding| binding.binding as usize);
        if let Some(resource) = place.and_then(|place| resources.get_mut(place)) {
            resource.written = true;
        }
    }
    let mut function = b.finish();
    function.name = Some("main_body".into());
    let function = module.functions.append(function, Span::UNDEFINED);

    // The entry point: the stage's inputs into the private variables, the body, then the exports out of theirs.
    let vec4 = vector(&mut module, VectorSize::Quad, Scalar::F32);
    let mut w = Builder::new();
    let mut arguments = Vec::new();
    let mut argument = |w: &mut Builder, name: String, ty, binding| {
        let index = arguments.len() as u32;
        arguments.push(naga::FunctionArgument {
            name: Some(name),
            ty,
            binding: Some(binding),
        });
        w.emit(E::FunctionArgument(index))
    };
    let location = |location: u32, flat: bool| naga::Binding::Location {
        location,
        interpolation: Some(match flat {
            true => naga::Interpolation::Flat,
            false => naga::Interpolation::Perspective,
        }),
        sampling: (!flat).then_some(naga::Sampling::Center),
        blend_src: None,
        per_primitive: false,
    };
    let store = |w: &mut Builder, global, value| {
        let pointer = w.emit(E::GlobalVariable(global));
        w.push(Statement::Store { pointer, value });
    };
    for (&slot, &(global, interpolated)) in &attributes {
        let binding = location(u32::from(slot), !interpolated);
        let value = argument(&mut w, format!("attr{slot}"), vec4, binding);
        store(&mut w, global, value);
    }
    for (&(slot, high), &global) in &halves {
        let at = u32::from(slot) + if high { HIGH_HALVES } else { 0 };
        let half = if high { "high" } else { "low" };
        let value = argument(
            &mut w,
            format!("attr{slot}_{half}"),
            vec4,
            location(at, false),
        );
        store(&mut w, global, value);
    }
    if let Some(global) = frag_coord {
        let binding = naga::Binding::BuiltIn(naga::BuiltIn::Position { invariant: false });
        let value = argument(&mut w, "frag_coord".to_owned(), vec4, binding);
        store(&mut w, global, value);
    }
    if let Some(global) = barycentric {
        let ty = vector(&mut module, VectorSize::Tri, Scalar::F32);
        let binding = naga::Binding::BuiltIn(naga::BuiltIn::Barycentric { perspective: true });
        let value = argument(&mut w, "barycentric".to_owned(), ty, binding);
        store(&mut w, global, value);
    }
    if let Some(global) = front_face {
        let ty = insert(&mut module, TypeInner::Scalar(Scalar::BOOL));
        let binding = naga::Binding::BuiltIn(naga::BuiltIn::FrontFacing);
        let value = argument(&mut w, "front_facing".to_owned(), ty, binding);
        store(&mut w, global, value);
    }
    for (&register, &(global, scalar)) in &fetched {
        let ty = vector(&mut module, VectorSize::Quad, scalar);
        let binding = naga::Binding::Location {
            location: register,
            interpolation: None,
            sampling: None,
            blend_src: None,
            per_primitive: false,
        };
        let value = argument(&mut w, format!("in{register}"), ty, binding);
        store(&mut w, global, value);
    }
    for (global, name, built_in) in [
        (vertex_index, "vertex_index", naga::BuiltIn::VertexIndex),
        (
            instance_index,
            "instance_index",
            naga::BuiltIn::InstanceIndex,
        ),
    ] {
        let Some(global) = global else { continue };
        let ty = insert(&mut module, TypeInner::Scalar(Scalar::U32));
        let binding = naga::Binding::BuiltIn(built_in);
        let value = argument(&mut w, name.to_owned(), ty, binding);
        store(&mut w, global, value);
    }
    if let Some([thread, group]) = ids {
        let ty = vector(&mut module, VectorSize::Tri, Scalar::U32);
        for (global, name, built_in) in [
            (thread, "thread", naga::BuiltIn::LocalInvocationId),
            (group, "group", naga::BuiltIn::WorkGroupId),
        ] {
            let binding = naga::Binding::BuiltIn(built_in);
            let value = argument(&mut w, name.to_owned(), ty, binding);
            store(&mut w, global, value);
        }
    }
    if let Some(global) = lane {
        let ty = insert(&mut module, TypeInner::Scalar(Scalar::U32));
        let value = match fragment {
            true => {
                let binding = naga::Binding::BuiltIn(naga::BuiltIn::SubgroupInvocationId);
                argument(&mut w, "lane".to_owned(), ty, binding)
            }
            // Every thread of a compute subgroup runs here, so the ones before this one number
            // it; the stage's own number is refused for a group of more than one axis.
            false => {
                let one = w.u32(1);
                let result = w.result(E::SubgroupOperationResult { ty });
                w.push(Statement::SubgroupCollectiveOperation {
                    op: naga::SubgroupOperation::Add,
                    collective_op: naga::CollectiveOperation::ExclusiveScan,
                    argument: one,
                    result,
                });
                result
            }
        };
        store(&mut w, global, value);
    }
    let call = || Statement::Call {
        function,
        arguments: Vec::new(),
        result: None,
    };
    // A mesh thread that reads other threads' values: the body is run for each of them first.
    let others = |w: &mut Builder, thread: Handle<E>| {
        let (Some(mesh), Some((run, none))) = (mesh, shared_wave.swizzle_run()) else {
            return;
        };
        for (which, pattern) in shared_wave.swizzle_runs() {
            let from = wave::swizzled(w, thread, pattern);
            store(w, mesh.thread, from);
            let which = w.u32(which);
            store(w, run, which);
            w.push(call());
        }
        let none = w.u32(none);
        store(w, run, none);
    };
    match mesh {
        None => match &pair {
            Some(pair) => pair.entry(&mut module, &mut w, &mut argument, function, &exports),
            None => w.push(call()),
        },
        // Instance `g` is group `g`; its vertices come three to a primitive, in the order of the
        // threads that place them.
        Some(mesh) => {
            let word = insert(&mut module, TypeInner::Scalar(Scalar::U32));
            let binding = naga::Binding::BuiltIn(naga::BuiltIn::VertexIndex);
            let vertex = argument(&mut w, "vertex_index".to_owned(), word, binding);
            let binding = naga::Binding::BuiltIn(naga::BuiltIn::InstanceIndex);
            let instance = argument(&mut w, "instance_index".to_owned(), word, binding);
            store(&mut w, mesh.group, instance);
            let three = w.u32(3);
            let (primitive, corner) = (
                w.bin(Bo::Divide, vertex, three),
                w.bin(Bo::Modulo, vertex, three),
            );
            let no = w.emit(E::Literal(Literal::Bool(false)));
            others(&mut w, primitive);
            store(&mut w, mesh.thread, primitive);
            store(&mut w, mesh.prim_set, no);
            w.push(call());
            // The primitive's dword holds its three vertex numbers ten bits apart and a cull bit on top.
            let pointer = w.emit(E::GlobalVariable(mesh.prim));
            let prim = w.load(pointer);
            let pointer = w.emit(E::GlobalVariable(mesh.prim_set));
            let made = w.load(pointer);
            let (ten, nine_bits, top) = (w.u32(10), w.u32(0x1FF), w.u32(31));
            let by = w.bin(Bo::Multiply, corner, ten);
            let shifted = w.bin(Bo::ShiftRight, prim, by);
            let index = w.bin(Bo::And, shifted, nine_bits);
            let sign = w.bin(Bo::ShiftRight, prim, top);
            let culled = w.ne0(sign);
            others(&mut w, index);
            store(&mut w, mesh.thread, index);
            store(&mut w, mesh.pos_set, no);
            w.push(call());
            // A primitive no thread made, a culled one, or a corner no thread placed: the same point
            // outside the clip volume, so the triangle has no area.
            let pointer = w.emit(E::GlobalVariable(mesh.pos_set));
            let placed = w.load(pointer);
            let (unmade, unplaced) = (w.lnot(made), w.lnot(placed));
            let gone = w.bin(Bo::LogicalOr, unmade, culled);
            let gone = w.bin(Bo::LogicalOr, gone, unplaced);
            if let Some(&position) = exports.get(&POS0) {
                let (two, one) = (w.f32(2.0), w.f32(1.0));
                let outside = w.emit(E::Compose {
                    ty: vec4,
                    components: vec![two, two, two, one],
                });
                w.open();
                store(&mut w, position, outside);
                let accept = w.close();
                w.push(Statement::If {
                    condition: gone,
                    accept,
                    reject: Block::new(),
                });
            }
        }
    }

    let mut members = Vec::new();
    let mut components = Vec::new();
    let mut offset = 0u32;
    for (&target, &global) in &exports {
        if target == POS1 && !fragment {
            // Four clip distances: a component the program did not write is zero, which clips nothing.
            let pointer = w.emit(E::GlobalVariable(global));
            let whole = w.load(pointer);
            let scalar = insert(&mut module, TypeInner::Scalar(Scalar::F32));
            let ty = insert(
                &mut module,
                TypeInner::Array {
                    base: scalar,
                    size: naga::ArraySize::Constant(NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN)),
                    stride: 4,
                },
            );
            members.push(naga::StructMember {
                name: Some("clip_distances".to_owned()),
                ty,
                binding: Some(naga::Binding::BuiltIn(naga::BuiltIn::ClipDistance)),
                offset,
            });
            offset += 16;
            let parts = (0..4).map(|c| w.index(whole, c)).collect();
            components.push(w.emit(E::Compose {
                ty,
                components: parts,
            }));
            continue;
        }
        let (name, binding) = match target {
            POS0 => (
                "position".to_owned(),
                naga::Binding::BuiltIn(naga::BuiltIn::Position { invariant: false }),
            ),
            _ if fragment => (
                format!("mrt{target}"),
                naga::Binding::Location {
                    location: u32::from(target),
                    interpolation: None,
                    sampling: None,
                    blend_src: None,
                    per_primitive: false,
                },
            ),
            _ => (
                format!("param{}", target - PARAM0),
                location(u32::from(target - PARAM0), false),
            ),
        };
        let pointer = w.emit(E::GlobalVariable(global));
        let whole = w.load(pointer);
        let slot = target.wrapping_sub(PARAM0);
        if fragment || target < PARAM0 || !half_slots.contains(&slot) {
            members.push(naga::StructMember {
                name: Some(name),
                ty: vec4,
                binding: Some(binding),
                offset,
            });
            offset += 16;
            components.push(whole);
            continue;
        }
        // Each component holds two 16-bit floats, which the rasterizer has to interpolate apart.
        let mut sides = [Vec::with_capacity(4), Vec::with_capacity(4)];
        for c in 0..4 {
            let part = w.index(whole, c);
            let bits = w.bits(part);
            let sixteen = w.u32(16);
            let high = w.bin(Bo::ShiftRight, bits, sixteen);
            for (half, parts) in [bits, high].into_iter().zip(sides.iter_mut()) {
                let wide = w.float_of_half(half);
                parts.push(w.float(wide));
            }
        }
        for (side, parts) in sides.into_iter().enumerate() {
            let (half, base) = [("low", 0), ("high", HIGH_HALVES)][side];
            members.push(naga::StructMember {
                name: Some(format!("{name}_{half}")),
                ty: vec4,
                binding: Some(location(base + u32::from(slot), false)),
                offset,
            });
            offset += 16;
            components.push(w.emit(E::Compose {
                ty: vec4,
                components: parts,
            }));
        }
    }
    if let Some(global) = depth {
        let ty = insert(&mut module, TypeInner::Scalar(Scalar::F32));
        members.push(naga::StructMember {
            name: Some("depth".to_owned()),
            ty,
            binding: Some(naga::Binding::BuiltIn(naga::BuiltIn::FragDepth)),
            offset,
        });
        offset += 4;
        let pointer = w.emit(E::GlobalVariable(global));
        components.push(w.load(pointer));
    }
    let result = if members.is_empty() {
        w.push(Statement::Return { value: None });
        None
    } else {
        let ty = module.types.insert(
            naga::Type {
                name: Some("Output".into()),
                inner: TypeInner::Struct {
                    members,
                    span: offset.next_multiple_of(16),
                },
            },
            Span::UNDEFINED,
        );
        let value = w.emit(E::Compose { ty, components });
        w.push(Statement::Return { value: Some(value) });
        Some(naga::FunctionResult { ty, binding: None })
    };
    let mut function = w.finish();
    function.name = Some("main".into());
    function.arguments = arguments;
    function.result = result;
    module.entry_points.push(naga::EntryPoint {
        name: "main".into(),
        stage,
        early_depth_test: None,
        workgroup_size: (program.launch)
            .filter(|_| stage == naga::ShaderStage::Compute)
            .map_or([0; 3], |launch| launch.threads.map(|n| n.max(1))),
        workgroup_size_overrides: None,
        function,
        mesh_info: None,
        task_payload: None,
        incoming_ray_payload: None,
    });
    Ok(Pass::Done(Box::new(Lowered {
        module,
        resources,
        approximations: notes,
        half_slots: halves.keys().map(|(slot, _)| *slot).collect(),
        subgroups: wave_ops || lane.is_some(),
    })))
}
