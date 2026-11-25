//! An oracle for the passes: a function's blocks are run on made-up inputs before and after a
//! pass, and what the two runs did is compared.
//!
//! A run records every call with the cells it is given, every write through a pointer and what
//! is returned. A call yields made-up cells, changes all memory a pointer reaches and every frame
//! cell from one whose address was handed out on, which is the most a pass may assume of it.
//! ⚠ A load is no event: two loads may change places, and one nothing reads may go.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::cfg::{self, Terminator};
use super::ir::{
    BinOp, Callee, Expr, FunctionKey, Kind, Returns, Scalar, Stmt, UnOp, Var, VarInfo, Width,
};
use super::listing::Listing;
use super::passes;
use super::project::Signature;
use super::tree;
use super::{Context, Level, Names, Options, Project, Stats};

#[path = "exec_corpus_tests.rs"]
mod corpus;

const LOCALS: u64 = 0xA1 << 56;
const PARAMS: u64 = 0xA2 << 56;
const TEMPS: u64 = 0xA3 << 56;
const GLOBALS: u64 = 0xA4 << 56;
const STATICS: u64 = 0xA5 << 56;
const HEAP: u64 = 0xA6 << 56;

/// What a run may take: statements and jumps, events, and the seeds past the first [`SEEDS`] that are tried for blocks none has entered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub steps: usize,
    pub events: usize,
    pub more: u64,
}

impl Limits {
    /// What a test written by hand runs at.
    pub(crate) const DEFAULT: Limits = Limits {
        steps: 3000,
        events: 200,
        more: 0,
    };
    /// What the suite over a folder runs at: some of its functions hold more than 3,000 statements.
    pub(crate) const CORPUS: Limits = Limits {
        steps: 20_000,
        events: 2000,
        more: 60,
    };
}

/// The seeds in a row that enter no new block before no more are tried.
const STALE: u64 = 8;

/// The most cells one value is made of, and the most a frame holds.
// ⚠ A count is what the input states: past these a run is recorded as not made, never sized.
const CELLS: u32 = 4096;
const FRAME_CELLS: u32 = 1 << 16;

fn mix(x: u64) -> u64 {
    let x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn mix2(a: u64, b: u64) -> u64 {
    mix(mix(a) ^ b)
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Whether an address is one of the function's own cells: a local's, a parameter's or a temporary's.
fn framed(addr: u64) -> bool {
    matches!(addr >> 56, 0xA1..=0xA3)
}

/// A cell as something that works on its value alone sees it: an object by its type and not by which it is.
fn typed(cell: u64) -> u64 {
    match cell >> 56 {
        0xA6 => cell & !0xFFFF_0000,
        _ => cell,
    }
}

/// A cell that is no address this oracle hands out.
fn plain(value: u64) -> u64 {
    match value >> 56 {
        0xA0..=0xAF => value & 0x00FF_FFFF_FFFF_FFFF,
        _ => value,
    }
}

/// A made-up cell: often zero or one, so that a test of it goes both ways over a few seeds.
fn made(h: u64) -> u64 {
    let v = mix(h);
    plain(match h % 16 {
        0..=3 => 0,
        4..=6 => 1,
        7 => 2 + v % 14,
        8 => u64::MAX,
        9 => 0xFFFF_FFFF,
        10 => v & 0xFFFF_FFFF,
        11 => v as i32 as i64 as u64,
        _ => v,
    })
}

/// What a run did that anything outside the function can see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// A call, the cells passed, and a digest of the frame cells the callee can read.
    Call {
        callee: u64,
        args: Vec<u64>,
        seen: u64,
    },
    Store {
        addr: u64,
        bytes: u32,
        value: u64,
    },
    Return(Vec<u64>),
    Unreachable,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Run {
    pub events: Vec<Event>,
    /// The run was stopped before the function left.
    pub cut: bool,
    /// What the oracle could not run, where it met such a thing.
    pub unsupported: Option<String>,
    /// The blocks the run entered, by `BlockId`.
    pub reached: Vec<bool>,
    /// The cells handed back by script functions whose result the project gives no kind.
    // Such a function may return constants alone, whose range no table keeps: a function that
    // hands its result on is then proved of a kind the made-up cell need not be.
    pub unstated: HashSet<u64>,
}

enum Val {
    One(u64),
    Many(Vec<u64>),
}

impl Val {
    fn one(&self) -> u64 {
        match self {
            Val::One(cell) => *cell,
            Val::Many(cells) => cells.first().copied().unwrap_or(0),
        }
    }

    fn push_to(self, out: &mut Vec<u64>) {
        match self {
            Val::One(cell) => out.push(cell),
            Val::Many(cells) => out.extend(cells),
        }
    }
}

/// What an intrinsic does, read off the runtime the listings come with.
enum Does {
    /// Works on its operands alone.
    Works,
    Reads,
    Allocates,
    Calls,
}

fn does(name: &str) -> Does {
    let base = name.split('<').next().unwrap_or(name);
    let converts = base.split_once("_to_").is_some_and(|(from, to)| {
        Scalar::from_name(from).is_some() && Scalar::from_name(to).is_some()
    });
    const WORKS: [&str; 8] = [
        "abs_",
        "min_",
        "max_",
        "clamp_",
        "sign_",
        "lerp_",
        "round_half_up_",
        "packed_vector_",
    ];
    const FLOATS: [&str; 11] = [
        "sqrtf", "sinf", "cosf", "tanf", "asinf", "acosf", "atanf", "atan2f", "powf", "floorf",
        "ceilf",
    ];
    if converts || WORKS.iter().any(|p| base.starts_with(p)) || FLOATS.contains(&base) {
        Does::Works
    } else if base.starts_with("array_get_")
        || base.starts_with("bytes_equal_")
        || matches!(base, "array_length" | "array_element_address" | "try_cast")
    {
        Does::Reads
    } else if matches!(base, "box" | "box_at" | "stack_alloc_zeroed") {
        Does::Allocates
    } else {
        Does::Calls
    }
}

/// The type the runtime reads cell `index` of an intrinsic's arguments as, where it reads less than the cell.
fn read_as(name: &str, index: usize) -> Option<Scalar> {
    const FLOATS: [&str; 11] = [
        "sqrtf", "sinf", "cosf", "tanf", "asinf", "acosf", "atanf", "atan2f", "powf", "floorf",
        "ceilf",
    ];
    const STEMS: [&str; 7] = [
        "abs",
        "min",
        "max",
        "clamp",
        "lerp",
        "sign",
        "round_half_up",
    ];
    let base = name.split('<').next().unwrap_or(name);
    if let Some((from, to)) = base.split_once("_to_") {
        if let (Some(from), Some(_)) = (Scalar::from_name(from), Scalar::from_name(to)) {
            return (index == 0).then_some(from);
        }
    }
    if FLOATS.contains(&base) {
        return Some(Scalar::F32);
    }
    if let Some((stem, ty)) = base.rsplit_once('_') {
        if let (true, Some(ty)) = (STEMS.contains(&stem), Scalar::from_name(ty)) {
            return Some(ty);
        }
    }
    match (base, index) {
        (array, 1) if array.starts_with("array_") => Some(Scalar::I32),
        ("packed_vector_from_xyz", 0..=2)
        | ("packed_vector_splat", 0)
        | ("packed_vector_scale", 2) => Some(Scalar::F32),
        _ => None,
    }
}

/// The type the runtime computes an intrinsic's one cell at, where it is narrower than the cell.
fn yields_as(name: &str) -> Option<Scalar> {
    const FLOATS: [&str; 11] = [
        "sqrtf", "sinf", "cosf", "tanf", "asinf", "acosf", "atanf", "atan2f", "powf", "floorf",
        "ceilf",
    ];
    const STEMS: [&str; 7] = [
        "abs",
        "min",
        "max",
        "clamp",
        "lerp",
        "sign",
        "round_half_up",
    ];
    let base = name.split('<').next().unwrap_or(name);
    if let Some((from, to)) = base.split_once("_to_") {
        if let (Some(_), Some(to)) = (Scalar::from_name(from), Scalar::from_name(to)) {
            return Some(to);
        }
    }
    if FLOATS.contains(&base) || base.starts_with("packed_vector_dot") {
        return Some(Scalar::F32);
    }
    if let Some((stem, ty)) = base.rsplit_once('_') {
        if let (true, Some(ty)) = (STEMS.contains(&stem), Scalar::from_name(ty)) {
            return Some(if stem == "sign" { Scalar::I32 } else { ty });
        }
    }
    if base == "array_length" {
        return Some(Scalar::I32);
    }
    base.strip_prefix("array_get_").and_then(Scalar::from_name)
}

fn callee_id(callee: &Callee) -> u64 {
    match callee {
        Callee::Local { at, .. } => mix2(1, u64::from(*at)),
        Callee::Import { program, at, .. } => mix2(mix2(2, u64::from(*program)), u64::from(*at)),
        Callee::Native { hash, .. } => mix2(3, *hash),
        Callee::Builtin { ty, method, .. } => mix2(mix2(4, u64::from(*ty)), u64::from(*method)),
        Callee::Virtual { ty, slot, .. } => mix2(mix2(5, text_hash(ty)), u64::from(*slot)),
        Callee::Intrinsic { name, ty, .. } => {
            let ty = ty.as_ref().map_or(0, |ty| text_hash(&ty.name));
            mix2(mix2(6, text_hash(name)), ty)
        }
    }
}

fn var_addr(var: Var) -> u64 {
    match var {
        Var::Local(n) => LOCALS + 8 * u64::from(n),
        Var::Param(n) => PARAMS + 8 * u64::from(n),
        Var::Value { at, index } => TEMPS | (u64::from(at) << 28) | (u64::from(index & 0x7F) << 20),
        Var::Stack { at, index } => {
            TEMPS | (1 << 55) | (u64::from(at) << 28) | (u64::from(index & 0x7F) << 20)
        }
        Var::Global {
            program,
            offset,
            statics,
        } => {
            let space = if statics { STATICS } else { GLOBALS };
            space | (u64::from(program) << 24) | u64::from(offset)
        }
    }
}

struct Machine<'f> {
    f: &'f cfg::Function,
    /// What states the cells a script function returns where a call of it does not.
    project: &'f Project,
    seed: u64,
    /// The cells written, by address; one never written reads as its default.
    cells: HashMap<u64, u64>,
    /// The cells each temporary of several cells was last given, by its address.
    widths: HashMap<u64, usize>,
    /// The calls made, which is also how often memory changed under the function.
    calls: u64,
    allocs: HashMap<u64, u64>,
    /// The objects an event has shown, numbered as they were first shown.
    // ⚠ Two allocations may change places, so an object is known by where it first went, not by when it was made.
    shown: HashMap<u64, u64>,
    /// The lowest local and parameter whose address was handed out, and the lowest cell of each temporary.
    low_local: Option<u64>,
    low_param: Option<u64>,
    low_temps: BTreeMap<u64, u64>,
    run: Run,
    steps: usize,
    limits: Limits,
}

impl<'f> Machine<'f> {
    fn unsupported(&mut self, what: &str) {
        if self.run.unsupported.is_none() {
            self.run.unsupported = Some(what.to_owned());
        }
    }

    /// Whether a value of `cells` cells is made; one of more than [`CELLS`] is what the oracle does not run.
    fn sized(&mut self, cells: u32) -> bool {
        if cells > CELLS {
            self.unsupported("a value of more cells than the oracle makes");
        }
        cells <= CELLS
    }

    fn cell(&self, addr: u64) -> u64 {
        if let Some(cell) = self.cells.get(&addr) {
            return *cell;
        }
        match addr >> 56 {
            0xA1 | 0xA3 => 0,
            0xA2 => made(mix2(self.seed, addr)),
            // A new object is zeroes until a callee has had it.
            0xA6 => match self.shown.get(&(addr & !0xFFFF)) {
                Some(id) => made(mix2(
                    mix2(self.seed, HEAP | (id << 16) | (addr & 0xFFFF)),
                    self.calls,
                )),
                None => 0,
            },
            _ => made(mix2(mix2(self.seed, addr), self.calls)),
        }
    }

    /// A cell as an event states it: an object by the order it was first shown in.
    fn stated(&mut self, cell: u64) -> u64 {
        if cell >> 56 != 0xA6 {
            return cell;
        }
        let next = self.shown.len() as u64 + 1;
        let id = *self.shown.entry(cell & !0xFFFF).or_insert(next);
        HEAP | (id << 16) | (cell & 0xFFFF)
    }

    fn read(&self, addr: u64, bytes: u32) -> u64 {
        if addr & 7 == 0 && bytes == 8 {
            return self.cell(addr);
        }
        let mut bits = 0u64;
        for i in 0..u64::from(bytes.min(8)) {
            let at = addr.wrapping_add(i);
            let byte = (self.cell(at & !7) >> ((at & 7) * 8)) & 0xFF;
            bits |= byte << (i * 8);
        }
        // Bytes of two made-up cells must not come together as an address of the frame's.
        match framed(addr) {
            true => bits,
            false => plain(bits),
        }
    }

    /// Writes the low `bytes` bytes of `value`; a write outside the frame is an event.
    fn write(&mut self, addr: u64, value: u64, bytes: u32) {
        let bytes = bytes.min(8);
        if !framed(addr) {
            let value = match bytes {
                8 => value,
                n => value & ((1u64 << (n * 8)) - 1),
            };
            if framed(value) && bytes == 8 {
                self.escape(value);
            }
            let (addr, value) = (self.stated(addr), self.stated(value));
            self.run.events.push(Event::Store { addr, bytes, value });
        }
        if addr & 7 == 0 && bytes == 8 {
            self.cells.insert(addr, value);
            return;
        }
        for i in 0..u64::from(bytes) {
            let at = addr.wrapping_add(i);
            let shift = (at & 7) * 8;
            let old = self.cell(at & !7);
            let new = (old & !(0xFF << shift)) | (((value >> (i * 8)) & 0xFF) << shift);
            self.cells.insert(at & !7, new);
        }
    }

    /// Notes that an address of the frame's went where a callee may find it.
    fn escape(&mut self, addr: u64) {
        let lower = |low: &mut Option<u64>, n: u64| *low = Some(low.map_or(n, |old| old.min(n)));
        match addr >> 56 {
            0xA1 => lower(&mut self.low_local, (addr - LOCALS) / 8),
            0xA2 => lower(&mut self.low_param, (addr - PARAMS) / 8),
            0xA3 => {
                let base = addr & !0xF_FFFF;
                let cell = (addr & 0xF_FFFF) / 8;
                let low = self.low_temps.entry(base).or_insert(cell);
                *low = (*low).min(cell);
            }
            _ => {}
        }
    }

    /// The frame cells a callee can reach.
    fn exposed(&self) -> Vec<u64> {
        let mut cells = Vec::new();
        if let Some(low) = self.low_local {
            cells.extend((low..u64::from(self.f.head.locals)).map(|n| LOCALS + 8 * n));
        }
        if let Some(low) = self.low_param {
            cells.extend((low..u64::from(self.f.head.params)).map(|n| PARAMS + 8 * n));
        }
        for (base, low) in &self.low_temps {
            let width = self.widths.get(base).copied().unwrap_or(1) as u64;
            cells.extend((*low..width.max(*low + 1)).map(|n| base + 8 * n));
        }
        cells
    }

    fn width(&self, var: Var) -> Option<usize> {
        let info = self.f.head.vars.get(&var)?;
        let temporary = var.is_temporary();
        match (&info.kind, info.width) {
            (_, Width::Cells) if temporary => {
                Some(self.widths.get(&var_addr(var)).copied().unwrap_or(1))
            }
            (Kind::Aggregate(cells), _) => Some((*cells).min(CELLS) as usize),
            // A struct held in place, a parameter as much as a local.
            (Kind::Value { cells, .. }, Width::Cells) => Some((*cells).min(CELLS) as usize),
            (Kind::Vector | Kind::Handle, Width::Cells) => Some(2),
            _ => None,
        }
    }

    fn assign(&mut self, target: Var, value: Val) {
        let addr = var_addr(target);
        if matches!(target, Var::Global { .. }) {
            self.write(addr, value.one(), 8);
            return;
        }
        if self.width(target).is_none() {
            self.cells.insert(addr, value.one());
            return;
        }
        let cells = match value {
            Val::One(cell) => vec![cell],
            Val::Many(cells) => cells,
        };
        if target.is_temporary() {
            self.widths.insert(addr, cells.len());
        }
        for (n, cell) in cells.into_iter().enumerate() {
            self.cells.insert(addr + 8 * n as u64, cell);
        }
    }

    fn alloc(&mut self, what: u64) -> u64 {
        let count = self.allocs.entry(what).or_insert(0);
        *count += 1;
        HEAP | ((what & 0xFF_FFFF) << 32) | ((*count & 0xFFFF) << 16)
    }

    /// What the project states of the script function a call reaches.
    fn signature(&self, callee: &Callee) -> Option<&'f Signature> {
        let key = match callee {
            Callee::Local { at, .. } => FunctionKey::new(self.f.head.program, *at),
            Callee::Import { program, at, .. } => FunctionKey::new(*program, *at),
            _ => return None,
        };
        self.project.signatures.get(&key)
    }

    fn call(&mut self, callee: &Callee, args: &[Expr], returns: Option<u32>) -> Val {
        // ⚠ A call that states no cells yields those its callee's signature states: a pass that
        // writes them into the call must find the run it is held against made of the same cells.
        let stated = self.signature(callee);
        let returns = returns.or_else(|| stated.map(|held| held.returns));
        if returns.is_some_and(|cells| !self.sized(cells)) {
            return Val::One(0);
        }
        let mut cells = Vec::with_capacity(args.len());
        for arg in args {
            match (arg, self.eval(arg)) {
                // A call that states no count is passed where one cell goes.
                (Expr::Call { returns: None, .. }, value) => cells.push(value.one()),
                (_, value) => value.push_to(&mut cells),
            }
        }
        let id = callee_id(callee);
        if let Callee::Intrinsic { name, .. } = callee {
            // An intrinsic sees of each cell what the runtime reads of it, whatever it then does.
            for (n, cell) in cells.iter_mut().enumerate() {
                if let Some(ty) = read_as(name, n) {
                    *cell = cast(ty, *cell);
                }
            }
            // A conversion between integers is what the runtime computes of the cell, and nothing made up.
            if let Some(exact) = converted(name, cells.first().copied()) {
                return Val::One(exact);
            }
            let of_args = cells.iter().fold(id, |h, cell| mix2(h, typed(*cell)));
            // What the runtime computes comes out as the type it computed at.
            let shaped = |value: Val| match (yields_as(name), value) {
                (Some(ty), Val::One(cell)) => Val::One(cast(ty, cell)),
                (_, value) if name.starts_with("bytes_equal_") => Val::One(value.one() & 1),
                (_, value) => value,
            };
            match does(name) {
                Does::Works => return shaped(self.yielded(of_args, returns)),
                Does::Reads if name.starts_with("try_cast") => {
                    let kept = mix(of_args) & 1 == 1;
                    return Val::One(if kept {
                        cells.first().copied().unwrap_or(0)
                    } else {
                        0
                    });
                }
                Does::Reads => return shaped(self.yielded(mix2(of_args, self.calls), returns)),
                Does::Allocates => return Val::One(self.alloc(id)),
                Does::Calls => {}
            }
        }
        for cell in &cells {
            if framed(*cell) {
                self.escape(*cell);
            }
        }
        let cells: Vec<u64> = cells.into_iter().map(|cell| self.stated(cell)).collect();
        let exposed = self.exposed();
        let mut seen = 0;
        for addr in &exposed {
            let cell = self.cell(*addr);
            seen = mix2(mix2(seen, *addr), self.stated(cell));
        }
        self.run.events.push(Event::Call {
            callee: id,
            args: cells,
            seen,
        });
        let n = self.calls;
        self.calls += 1;
        let shown = &self.shown;
        self.cells.retain(|addr, _| {
            framed(*addr) || (addr >> 56 == 0xA6 && !shown.contains_key(&(addr & !0xFFFF)))
        });
        for addr in exposed {
            let new = made(mix2(mix2(self.seed ^ 0xC10B, addr), n));
            self.cells.insert(addr, new);
        }
        // ⚠ A function whose own code proves what its one cell is returns a cell widened as
        // that: a pass may read `f() != 0` as `f()`, or `as_u32(f())` as `f()`, on that proof,
        // which a made-up cell of any bits would deny. `check_blocks` holds the proof to the code.
        match (stated, self.yielded(mix2(mix2(self.seed, id), n), returns)) {
            (Some(stated), Val::One(cell)) if stated.result != Kind::Unknown => {
                Val::One(widened(&stated.result, cell))
            }
            (_, Val::One(cell)) => {
                if matches!(callee, Callee::Local { .. } | Callee::Import { .. }) {
                    self.run.unstated.insert(cell);
                }
                Val::One(cell)
            }
            (_, made) => made,
        }
    }

    /// The cells something yields: as many as it states, four where it states none.
    fn yielded(&self, h: u64, returns: Option<u32>) -> Val {
        match returns {
            Some(1) => Val::One(made(mix2(h, 0))),
            Some(cells) => Val::Many((0..u64::from(cells)).map(|k| made(mix2(h, k))).collect()),
            None => Val::Many((0..4).map(|k| made(mix2(h, k))).collect()),
        }
    }

    fn truth(&mut self, e: &Expr) -> bool {
        self.eval(e).one() != 0
    }

    fn eval(&mut self, e: &Expr) -> Val {
        Val::One(match e {
            Expr::Const(c) => c.bits,
            Expr::Var(var) => match (self.width(*var), var) {
                (_, Var::Global { .. }) => self.read(var_addr(*var), 8),
                (Some(width), _) => {
                    let addr = var_addr(*var);
                    return Val::Many((0..width as u64).map(|n| self.cell(addr + 8 * n)).collect());
                }
                (None, _) => self.cell(var_addr(*var)),
            },
            Expr::GuidEmpty => return Val::Many(vec![0, 0]),
            // A source reading changes nothing of what is evaluated.
            Expr::Sugar { e, .. } => return self.eval(e),
            Expr::Cell { base, index } => match self.eval(base) {
                Val::One(cell) if *index == 0 => cell,
                Val::One(_) => 0,
                Val::Many(cells) => cells.get(*index as usize).copied().unwrap_or(0),
            },
            Expr::Cells(items) => {
                let mut cells = Vec::with_capacity(items.len());
                for item in items {
                    self.eval(item).push_to(&mut cells);
                }
                return Val::Many(cells);
            }
            Expr::Cast { to, e } => cast(*to, self.eval(e).one()),
            Expr::Truth(e) => u64::from(self.truth(e)),
            Expr::Unary { op, e: inner } => {
                let kind = inner.listed_kind();
                let v = self.eval(inner).one();
                match (op, kind) {
                    (UnOp::Not, _) => u64::from(v == 0),
                    (UnOp::BitNot, Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32) => {
                        !(v as i32) as i64 as u64
                    }
                    (UnOp::BitNot, Kind::U32) => u64::from(!(v as u32)),
                    (UnOp::BitNot, Kind::I64 | Kind::U64) => !v,
                    (UnOp::Neg, Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32) => {
                        (v as i32).wrapping_neg() as i64 as u64
                    }
                    (UnOp::Neg, Kind::U32) => (-(i64::from(v as u32))) as u64,
                    (UnOp::Neg, Kind::I64) => (v as i64).wrapping_neg() as u64,
                    (UnOp::Neg, Kind::F32) => u64::from(v as u32 ^ (1 << 31)),
                    (UnOp::Neg, Kind::F64) => v ^ (1 << 63),
                    _ => {
                        self.unsupported("a unary operator over an untyped operand");
                        0
                    }
                }
            }
            Expr::Binary { op, at, l, r } => self.binary(*op, at, l, r),
            Expr::Select { cond, then, els } => {
                return match self.truth(cond) {
                    true => self.eval(then),
                    false => self.eval(els),
                }
            }
            Expr::Call {
                callee,
                args,
                returns,
            } => return self.call(callee, args, *returns),
            Expr::Load { ty, addr, offset } => {
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let raw = self.read(addr, ty.bytes());
                match ty {
                    Scalar::I8 => raw as i8 as i64 as u64,
                    Scalar::I16 => raw as i16 as i64 as u64,
                    Scalar::I32 => raw as i32 as i64 as u64,
                    _ => raw,
                }
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => {
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let cells = bytes.div_ceil(8);
                if !self.sized(cells) {
                    return Val::One(0);
                }
                let load = |n: u32| {
                    self.read(addr.wrapping_add(8 * u64::from(n)), (*bytes - 8 * n).min(8))
                };
                return Val::Many((0..cells).map(load).collect());
            }
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } => {
                if !self.sized(bytes.div_ceil(8)) {
                    return Val::One(0);
                }
                let mut cells = Vec::new();
                self.eval(value).push_to(&mut cells);
                let byte = |at: u32| -> u64 {
                    let cell = cells.get(at as usize / 8).copied().unwrap_or(0);
                    (cell >> ((at % 8) * 8)) & 0xFF
                };
                let out = (0..bytes.div_ceil(8)).map(|n| {
                    let take = (*bytes - 8 * n).min(8);
                    (0..take).fold(0, |bits, i| {
                        bits | (byte(offset.wrapping_add(8 * n + i)) << (i * 8))
                    })
                });
                return Val::Many(out.collect());
            }
            Expr::ByteOffset { addr, offset } => {
                let base = self.eval(addr).one();
                base.wrapping_add(self.eval(offset).one())
            }
            Expr::AddrOf { var, cell } => var_addr(*var).wrapping_add(8 * u64::from(*cell)),
            Expr::ZeroSlots(cells) => {
                return match self.sized(*cells) {
                    true => Val::Many(vec![0; *cells as usize]),
                    false => Val::One(0),
                }
            }
            Expr::Vector3(parts) => {
                let x = self.eval(&parts[0]).one() & 0xFFFF_FFFF;
                let y = self.eval(&parts[1]).one() & 0xFFFF_FFFF;
                let z = self.eval(&parts[2]).one() & 0xFFFF_FFFF;
                return Val::Many(vec![x | (y << 32), z]);
            }
            Expr::NewObject(ty) => self.alloc(text_hash(&ty.name)),
            Expr::NewArray { ty, len, .. } => {
                self.eval(len);
                self.alloc(mix2(7, text_hash(&ty.name)))
            }
            Expr::FunctionRef { program, at } => {
                plain(mix2(mix2(8, u64::from(*program)), u64::from(*at)))
            }
            Expr::BuiltinFunctionRef { ty, method } => {
                plain(mix2(mix2(9, u64::from(*ty)), u64::from(*method)))
            }
            Expr::VirtualFunctionRef { receiver, ty, slot } => {
                let receiver = self.eval(receiver).one();
                plain(mix2(
                    mix2(mix2(10, typed(receiver)), text_hash(&ty.name)),
                    u64::from(*slot),
                ))
            }
            Expr::StringConstant { program, offset } => {
                plain(mix2(mix2(11, u64::from(*program)), u64::from(*offset)))
            }
            Expr::BuiltinGlobal(n) => {
                let h = mix2(12, u64::from(*n));
                return Val::Many(vec![plain(mix2(h, 0)), plain(mix2(h, 1))]);
            }
        })
    }

    fn binary(&mut self, op: BinOp, at: &Kind, l: &Expr, r: &Expr) -> u64 {
        match op {
            BinOp::LogicalAnd => return u64::from(self.truth(l) && self.truth(r)),
            BinOp::LogicalOr => return u64::from(self.truth(l) || self.truth(r)),
            _ => {}
        }
        let (a, b) = (self.eval(l).one(), self.eval(r).one());
        let ordered = |o: Option<std::cmp::Ordering>| match (op, o) {
            (BinOp::Eq, o) => Some(o.is_some_and(std::cmp::Ordering::is_eq)),
            (BinOp::Ne, o) => Some(!o.is_some_and(std::cmp::Ordering::is_eq)),
            (BinOp::Lt, o) => Some(o.is_some_and(std::cmp::Ordering::is_lt)),
            (BinOp::Le, o) => Some(o.is_some_and(std::cmp::Ordering::is_le)),
            (BinOp::Gt, o) => Some(o.is_some_and(std::cmp::Ordering::is_gt)),
            (BinOp::Ge, o) => Some(o.is_some_and(std::cmp::Ordering::is_ge)),
            _ => None,
        };
        let done = match at {
            Kind::Bool => {
                let (a, b) = (a != 0, b != 0);
                match op {
                    BinOp::And => Some(u64::from(a & b)),
                    BinOp::Or => Some(u64::from(a | b)),
                    BinOp::Xor => Some(u64::from(a ^ b)),
                    _ => ordered(Some(a.cmp(&b))).map(u64::from),
                }
            }
            Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32 => {
                let (a, b) = (a as i32, b as i32);
                let value = match op {
                    BinOp::Add => Some(a.wrapping_add(b)),
                    BinOp::Sub => Some(a.wrapping_sub(b)),
                    BinOp::Mul => Some(a.wrapping_mul(b)),
                    BinOp::Div => Some(if b == 0 { 0 } else { a.wrapping_div(b) }),
                    BinOp::Rem => Some(if b == 0 { 0 } else { a.wrapping_rem(b) }),
                    BinOp::And => Some(a & b),
                    BinOp::Or => Some(a | b),
                    BinOp::Xor => Some(a ^ b),
                    BinOp::Shl => Some(a.wrapping_shl(b as u32)),
                    BinOp::Shr => Some(a.wrapping_shr(b as u32)),
                    _ => None,
                };
                match value {
                    Some(value) => Some(value as i64 as u64),
                    None => ordered(Some(a.cmp(&b))).map(u64::from),
                }
            }
            Kind::U32 | Kind::Hash => {
                let (a, c) = (a as u32, b as u32);
                let value = match op {
                    BinOp::Add => Some(a.wrapping_add(c)),
                    BinOp::Sub => Some(a.wrapping_sub(c)),
                    BinOp::Mul => Some(a.wrapping_mul(c)),
                    BinOp::Div => Some(a.checked_div(c).unwrap_or(0)),
                    BinOp::Rem => Some(a.checked_rem(c).unwrap_or(0)),
                    BinOp::And => Some(a & c),
                    BinOp::Or => Some(a | c),
                    BinOp::Xor => Some(a ^ c),
                    BinOp::Shl => Some(a.wrapping_shl(b as u32)),
                    BinOp::Shr => Some(a.wrapping_shr(b as u32)),
                    _ => None,
                };
                match value {
                    Some(value) => Some(u64::from(value)),
                    None => ordered(Some(a.cmp(&c))).map(u64::from),
                }
            }
            Kind::I64 => {
                let (a, c) = (a as i64, b as i64);
                let value = match op {
                    BinOp::Add => Some(a.wrapping_add(c)),
                    BinOp::Sub => Some(a.wrapping_sub(c)),
                    BinOp::Mul => Some(a.wrapping_mul(c)),
                    BinOp::Div => Some(if c == 0 { 0 } else { a.wrapping_div(c) }),
                    BinOp::Rem => Some(if c == 0 { 0 } else { a.wrapping_rem(c) }),
                    BinOp::And => Some(a & c),
                    BinOp::Or => Some(a | c),
                    BinOp::Xor => Some(a ^ c),
                    BinOp::Shl => Some(a.wrapping_shl(b as u32)),
                    BinOp::Shr => Some(a.wrapping_shr(b as u32)),
                    _ => None,
                };
                match value {
                    Some(value) => Some(value as u64),
                    None => ordered(Some(a.cmp(&c))).map(u64::from),
                }
            }
            Kind::U64 => {
                let value = match op {
                    BinOp::Add => Some(a.wrapping_add(b)),
                    BinOp::Sub => Some(a.wrapping_sub(b)),
                    BinOp::Mul => Some(a.wrapping_mul(b)),
                    BinOp::Div => Some(a.checked_div(b).unwrap_or(0)),
                    BinOp::Rem => Some(a.checked_rem(b).unwrap_or(0)),
                    BinOp::And => Some(a & b),
                    BinOp::Or => Some(a | b),
                    BinOp::Xor => Some(a ^ b),
                    BinOp::Shl => Some(a.wrapping_shl(b as u32)),
                    BinOp::Shr => Some(a.wrapping_shr(b as u32)),
                    _ => None,
                };
                value.or_else(|| ordered(Some(a.cmp(&b))).map(u64::from))
            }
            Kind::F32 => {
                let (a, b) = (f32::from_bits(a as u32), f32::from_bits(b as u32));
                let value = match op {
                    BinOp::Add => Some(a + b),
                    BinOp::Sub => Some(a - b),
                    BinOp::Mul => Some(a * b),
                    BinOp::Div => Some(a / b),
                    BinOp::Rem => Some(a % b),
                    _ => None,
                };
                match value {
                    Some(value) => Some(u64::from(value.to_bits())),
                    None => ordered(a.partial_cmp(&b)).map(u64::from),
                }
            }
            Kind::F64 => {
                let (a, b) = (f64::from_bits(a), f64::from_bits(b));
                let value = match op {
                    BinOp::Add => Some(a + b),
                    BinOp::Sub => Some(a - b),
                    BinOp::Mul => Some(a * b),
                    BinOp::Div => Some(a / b),
                    BinOp::Rem => Some(a % b),
                    _ => None,
                };
                match value {
                    Some(value) => Some(value.to_bits()),
                    None => ordered(a.partial_cmp(&b)).map(u64::from),
                }
            }
            // Whole cells, which only equality compares.
            _ => match op {
                BinOp::Eq => Some(u64::from(a == b)),
                BinOp::Ne => Some(u64::from(a != b)),
                _ => None,
            },
        };
        match done {
            Some(cell) => cell,
            None => {
                self.unsupported("an operator at a kind the oracle does not work at");
                0
            }
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { target, value } => {
                let value = self.eval(value);
                self.assign(*target, value);
            }
            Stmt::AssignMany { targets, values } => {
                let values: Vec<Val> = values.iter().map(|value| self.eval(value)).collect();
                for (target, value) in targets.iter().zip(values) {
                    self.assign(*target, value);
                }
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let value = self.eval(value).one();
                self.write(addr, value, kind.bytes());
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } => {
                if !self.sized(bytes.div_ceil(8)) {
                    return;
                }
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let mut cells = Vec::new();
                self.eval(value).push_to(&mut cells);
                for n in 0..bytes.div_ceil(8) {
                    let cell = cells.get(n as usize).copied().unwrap_or(0);
                    self.write(
                        addr.wrapping_add(8 * u64::from(n)),
                        cell,
                        (*bytes - 8 * n).min(8),
                    );
                }
            }
            Stmt::StoreF32Slots {
                addr,
                offset,
                value,
            } => {
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let mut cells = Vec::new();
                self.eval(value).push_to(&mut cells);
                for (n, cell) in cells.into_iter().enumerate() {
                    self.write(addr.wrapping_add(4 * n as u64), cell, 4);
                }
            }
            Stmt::ZeroBytes { addr, bytes } => {
                if !self.sized(bytes.div_ceil(8)) {
                    return;
                }
                let addr = self.eval(addr).one();
                for n in 0..bytes.div_ceil(8) {
                    self.write(
                        addr.wrapping_add(8 * u64::from(n)),
                        0,
                        (*bytes - 8 * n).min(8),
                    );
                }
            }
            Stmt::StepI32 { addr, offset, down } => {
                let addr = self.eval(addr).one().wrapping_add(u64::from(*offset));
                let old = self.read(addr, 4) as i32;
                let new = if *down {
                    old.wrapping_sub(1)
                } else {
                    old.wrapping_add(1)
                };
                self.write(addr, new as u32 as u64, 4);
            }
            Stmt::Eval(e) => {
                self.eval(e);
            }
        }
    }

    fn go(&mut self) {
        let blocks = &self.f.cfg.blocks;
        let mut b = cfg::ENTRY;
        loop {
            let Some(block) = blocks.get(b) else {
                self.unsupported("a jump to no block");
                return;
            };
            if let Some(entered) = self.run.reached.get_mut(b) {
                *entered = true;
            }
            for stmt in &block.stmts {
                self.stmt(stmt);
                self.steps += 1;
            }
            self.steps += 1;
            let limits = self.limits;
            let over = self.steps > limits.steps || self.run.events.len() > limits.events;
            if over || self.run.unsupported.is_some() {
                self.run.cut = true;
                return;
            }
            match &block.term {
                Terminator::Jump(to) => b = *to,
                Terminator::Branch { cond, then, els } => {
                    b = if self.truth(cond) { *then } else { *els };
                }
                Terminator::Switch {
                    value,
                    cases,
                    default,
                } => {
                    let value = i64::from(self.eval(value).one() as i32);
                    let case = cases.iter().find(|case| case.0 == value);
                    b = case.map_or(*default, |case| case.1);
                }
                Terminator::Return(value) => {
                    let mut cells = Vec::new();
                    if let Some(value) = value {
                        match (self.f.head.returns, self.eval(value)) {
                            (Returns::Cell, value) => cells.push(value.one()),
                            (_, value) => value.push_to(&mut cells),
                        }
                    }
                    let cells = cells.into_iter().map(|cell| self.stated(cell)).collect();
                    self.run.events.push(Event::Return(cells));
                    return;
                }
                Terminator::Unreachable => {
                    self.run.events.push(Event::Unreachable);
                    return;
                }
            }
        }
    }
}

fn cast(to: Scalar, bits: u64) -> u64 {
    match to {
        Scalar::I8 => bits as i8 as i64 as u64,
        Scalar::U8 => u64::from(bits as u8),
        Scalar::I16 => bits as i16 as i64 as u64,
        Scalar::U16 => u64::from(bits as u16),
        Scalar::I32 => bits as i32 as i64 as u64,
        Scalar::U32 | Scalar::F32 => u64::from(bits as u32),
        Scalar::I64 | Scalar::U64 | Scalar::F64 => bits,
    }
}

/// Runs a function's blocks on the inputs `seed` makes up.
pub(crate) fn run(f: &cfg::Function, seed: u64) -> Run {
    run_within(f, seed, Limits::DEFAULT)
}

/// [`run`], within `limits`.
pub(crate) fn run_within(f: &cfg::Function, seed: u64, limits: Limits) -> Run {
    run_in(f, seed, limits, Project::empty())
}

/// [`run_within`], with the signatures of `project` for the cells of a call that states none.
pub(crate) fn run_in(f: &cfg::Function, seed: u64, limits: Limits, project: &Project) -> Run {
    let mut machine = Machine {
        f,
        project,
        seed: mix(seed),
        cells: HashMap::new(),
        widths: HashMap::new(),
        calls: 0,
        allocs: HashMap::new(),
        shown: HashMap::new(),
        low_local: None,
        low_param: None,
        low_temps: BTreeMap::new(),
        run: Run {
            reached: vec![false; f.cfg.blocks.len()],
            ..Run::default()
        },
        steps: 0,
        limits,
    };
    if f.head.locals > FRAME_CELLS || f.head.params > FRAME_CELLS {
        machine.unsupported("a frame of more cells than the oracle makes");
    } else if !f.cfg.blocks.is_empty() {
        machine.go();
    }
    machine.run
}

/// What the conversion `name` from one integer to another makes of a cell: the cell read as the one and widened as the other.
fn converted(name: &str, cell: Option<u64>) -> Option<u64> {
    let (from, to) = name.split_once("_to_")?;
    let (from, to) = (Scalar::from_name(from)?, Scalar::from_name(to)?);
    let integer = |ty: Scalar| !matches!(ty, Scalar::F32 | Scalar::F64);
    (integer(from) && integer(to)).then_some(cast(to, cast(from, cell?)))
}

/// A cell as a value of `kind` widens to it; any cell for a kind that fills one or says nothing of its bits.
fn widened(kind: &Kind, cell: u64) -> u64 {
    match kind {
        Kind::Bool => cell & 1,
        Kind::I8 => cast(Scalar::I8, cell),
        Kind::U8 => cast(Scalar::U8, cell),
        Kind::I16 => cast(Scalar::I16, cell),
        Kind::U16 => cast(Scalar::U16, cell),
        Kind::I32 => cast(Scalar::I32, cell),
        Kind::U32 | Kind::Hash | Kind::F32 => cast(Scalar::U32, cell),
        _ => cell,
    }
}

/// How two events differ, in words a tally can count.
fn differing(was: &Event, is: &Event) -> &'static str {
    let low = |a: &u64, b: &u64| *a as u32 == *b as u32;
    match (was, is) {
        (
            Event::Call {
                callee: a, args: x, ..
            },
            Event::Call {
                callee: b, args: y, ..
            },
        ) => match x.iter().zip(y).find(|(p, q)| p != q) {
            _ if a != b => "another callee",
            _ if x.len() != y.len() => "a call of other cells",
            Some((p, q)) if low(p, q) => "the upper half of an argument",
            Some(_) => "an argument",
            None => "what a callee can see of the frame",
        },
        (
            Event::Store {
                addr: a,
                bytes: m,
                value: v,
            },
            Event::Store {
                addr: b,
                bytes: n,
                value: w,
            },
        ) => match (a == b, m == n) {
            (false, _) => "a store elsewhere",
            (true, false) => "a store of other bytes",
            (true, true) if low(v, w) => "the upper half of a stored value",
            (true, true) => "a stored value",
        },
        (Event::Return(x), Event::Return(y)) if x.len() != y.len() => "a return of other cells",
        (Event::Return(x), Event::Return(y)) => match x.iter().zip(y).all(|(p, q)| low(p, q)) {
            true => "the upper half of a returned value",
            false => "a returned value",
        },
        _ => "another kind of event",
    }
}

/// Whether two runs did the same, as far as both went.
pub(crate) fn same(was: &Run, is: &Run) -> Result<(), String> {
    let shared = was.events.len().min(is.events.len());
    for n in 0..shared {
        if was.events[n] != is.events[n] {
            return Err(format!(
                "event {n}, {}: {:X?} became {:X?}",
                differing(&was.events[n], &is.events[n]),
                was.events[n],
                is.events[n]
            ));
        }
    }
    if !was.cut && !is.cut && was.events.len() != is.events.len() {
        return Err(format!(
            "{} event(s) became {}",
            was.events.len(),
            is.events.len()
        ));
    }
    Ok(())
}

/// The seeds each function is run on.
pub(crate) const SEEDS: u64 = 4;

/// What checking one function came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Checked {
    /// The function held something the oracle does not run: nothing was compared.
    pub skipped: bool,
    /// The seeds every form of the function was run on.
    pub seeds: u32,
    /// The blocks of the form read that a seed entered, and the blocks it has.
    // ⚠ A statement no seed reaches is compared by nothing.
    pub reached: u32,
    pub blocks: u32,
}

/// Runs a function as it was read and after the passes, by the four first and then by every one; `Err` names the pass whose result does otherwise.
pub(crate) fn check(f: tree::Function, names: &Names) -> Result<Checked, String> {
    let done = check_in(f.clone(), &Context::baseline(names), Limits::DEFAULT)?;
    let every = Context::new(names, Project::empty(), Options::default());
    check_in(f, &every, Limits::DEFAULT)?;
    Ok(done)
}

/// [`check_blocks`] of a function in the structured form, as a listing reads.
pub(crate) fn check_in(
    f: tree::Function,
    cx: &Context<'_>,
    limits: Limits,
) -> Result<Checked, String> {
    if f.head.incomplete.is_some() {
        return Ok(Checked {
            skipped: true,
            ..Checked::default()
        });
    }
    let read = cfg::lower(f).map_err(|e| format!("does not lower: {e}"))?;
    check_blocks(&read, cx, limits)
}

/// Runs a function's blocks as a front end made them and after the passes `cx` asks; `Err` names the pass that makes the runs differ once it is switched on after those before it.
// ⚠ The way back from a source construct is `cfg::lower`, which writes each as the statements
// it holds: a construct made wrongly differs here like any other rewrite.
pub(crate) fn check_blocks(
    read: &cfg::Function,
    cx: &Context<'_>,
    limits: Limits,
) -> Result<Checked, String> {
    // The seeds are chosen on the form read, once: every other form is run on exactly those.
    let mut was: Vec<(u64, Run)> = Vec::new();
    let mut reached = vec![false; read.cfg.blocks.len()];
    let mut stale = 0;
    for seed in 0..SEEDS + limits.more {
        // More seeds are for blocks none has entered yet: with none left, or none found, no more.
        if stale >= STALE || (seed >= SEEDS && reached.iter().all(|entered| *entered)) {
            break;
        }
        let run = run_in(read, seed, limits, cx.project);
        let enters = |(now, before): (&bool, &bool)| *now && !*before;
        if seed < SEEDS || run.reached.iter().zip(&reached).any(enters) {
            for (before, now) in reached.iter_mut().zip(&run.reached) {
                *before |= *now;
            }
            was.push((seed, run));
            stale = 0;
        } else {
            stale += 1;
        }
    }
    let mut checked = Checked {
        skipped: was.iter().any(|(_, run)| run.unsupported.is_some()),
        seeds: was.len() as u32,
        reached: reached.iter().filter(|entered| **entered).count() as u32,
        blocks: reached.len() as u32,
    };
    if checked.skipped {
        return Ok(checked);
    }
    // ⚠ A callee's result is made up as its signature says it widens, so the signature of this
    // function is held to what its own code returned: a proof that is wrong is found here.
    let own = FunctionKey::new(read.head.program, read.head.at);
    if let Some(stated) = cx.project.signatures.get(&own) {
        for (seed, run) in &was {
            let one = run.events.iter().find_map(|event| match event {
                Event::Return(cells) if cells.len() == 1 => cells.first().copied(),
                _ => None,
            });
            let told = |cell: &u64| widened(&stated.result, *cell) == *cell;
            if let Some(cell) = one.filter(|cell| !told(cell) && !run.unstated.contains(cell)) {
                let kind = &stated.result;
                return Err(format!(
                    "signature, seed {seed}: {cell:X} is returned, and is no {kind:?}"
                ));
            }
        }
    }
    // `Err` where a run differs, `Ok(false)` where the passes left what the oracle does not run.
    let holds = |options: Options| -> Result<bool, String> {
        let cx = Context::new(cx.names, cx.project, options);
        let mut done = super::blocks_in(read.clone(), &cx, &mut Stats::default());
        // A void function returns by running off its end, which the blocks must say.
        if done.head.returns == Returns::Void {
            done.body.push(tree::Stmt::Return(None));
        }
        let back = cfg::lower(done).map_err(|e| format!("does not lower: {e}"))?;
        for (seed, was) in &was {
            let is = run_in(&back, *seed, limits, cx.project);
            if is.unsupported.is_some() {
                return Ok(false);
            }
            same(was, &is).map_err(|why| format!("seed {seed}: {why}"))?;
        }
        Ok(true)
    };
    if let Ok(ran) = holds(cx.options) {
        checked.skipped = !ran;
        return Ok(checked);
    }
    // The passes switched on one after another, in their order: the first set that differs
    // ends in the pass to read. With none on, the blocks only go to labels and jumps and back.
    let mut some = Options {
        level: cx.options.level,
        ..Options::at(Level::Lifted)
    };
    holds(some).map_err(|why| format!("no pass, {why}"))?;
    for pass in cx.options.passes.iter() {
        some = some.with(pass);
        holds(some).map_err(|why| format!("{}, {why}", pass.name()))?;
    }
    Err("the passes do not give the same result twice".to_owned())
}

/// What holding two forms of one function against each other came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Agreed {
    /// Either form holds what the oracle does not run: nothing was compared.
    pub skipped: bool,
    /// Every block of both forms was entered by a seed.
    // ⚠ Without it the two do the same only as far as the seeds went: a block none entered is compared by nothing.
    pub whole: bool,
}

/// Whether two forms of one function do the same on every seed, with the signatures of `project`; more seeds are tried while one enters a block of either form that none had.
pub(crate) fn agree(
    a: &cfg::Function,
    b: &cfg::Function,
    limits: Limits,
    project: &Project,
) -> Result<Agreed, String> {
    let mut entered = (
        vec![false; a.cfg.blocks.len()],
        vec![false; b.cfg.blocks.len()],
    );
    let whole = |entered: &(Vec<bool>, Vec<bool>)| entered.0.iter().chain(&entered.1).all(|e| *e);
    let mut stale = 0;
    for seed in 0..SEEDS + limits.more {
        if stale >= STALE || (seed >= SEEDS && whole(&entered)) {
            break;
        }
        let was = run_in(a, seed, limits, project);
        let is = run_in(b, seed, limits, project);
        if was.unsupported.is_some() || is.unsupported.is_some() {
            return Ok(Agreed {
                skipped: true,
                whole: false,
            });
        }
        same(&was, &is).map_err(|why| format!("seed {seed}: {why}"))?;
        let mut news = false;
        for (held, run) in [(&mut entered.0, &was), (&mut entered.1, &is)] {
            for (before, now) in held.iter_mut().zip(&run.reached) {
                news |= *now && !*before;
                *before |= *now;
            }
        }
        stale = match news || seed < SEEDS {
            true => 0,
            false => stale + 1,
        };
    }
    Ok(Agreed {
        skipped: false,
        whole: whole(&entered),
    })
}

/// Checks every function of a listing written by hand.
fn checked(text: &str) -> Result<(), String> {
    let listing = Listing::open(text).map_err(|e| e.to_string())?;
    for f in listing.functions() {
        let f = f.map_err(|e| e.to_string())?;
        let name = f.head.name.clone();
        let done = check(f, &Names::default()).map_err(|why| format!("{name}: {why}"))?;
        if done.skipped {
            return Err(format!("{name}: the oracle does not run it"));
        }
    }
    Ok(())
}

/// A listing of one function of two parameters and eight locals, written by hand, with natives to call.
const FRAME: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static VmValue func_000010(VmValue arg0, VmValue arg1) // 0x000010
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[8];
        VmValue v_000010_0 = default; VmValue v_000018_1 = default; VmValue v_000020_2 = default;
        VmValues v_000028_3 = null!; VmValues v_000030_4 = null!;
        VmValue stack_000040_0 = default; VmValue stack_000040_1 = default; VmValue stack_000050_0 = default;
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class System
        {
            public static class Delegate
            {
                public static void Ctor(VmValue p0, VmValue p1, VmValue p2) => Vm.Builtin(2, 0, 0, p0, p1, p2);
            }
            public static class Guid
            {
                public static VmValue op_Equality(VmValue p0, VmValue p1, VmValue p2, VmValue p3) => Vm.Builtin(3, 1, 1, p0, p1, p2, p3)[0];
            }
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static VmValues PAIR() => Vm.Native(0x0000000000000002UL, 0x00000002U, 2);
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
            public static void USE2(VmValue p0, VmValue p1) => Vm.Native(0x0000000000000004UL, 0x00000004U, 0, p0, p1);
        }
    }
    public static class Operations
    {
        public static VmValue clamp_f32(VmValue p0, VmValue p1, VmValue p2) => Vm.Intrinsic("clamp_f32", 1, p0, p1, p2)[0];
    }
}
"#;

/// The readable source of the function that holds `body`, which the passes must also keep the doings of.
fn source(body: &str) -> String {
    holds(body);
    let done = super::decompile(&FRAME.replace("BODY", body), &Names::default());
    done.expect("the listing decompiles").text
}

/// Asserts that the source of `body` holds each of `with` and none of `without`.
fn reads(body: &str, with: &[&str], without: &[&str]) {
    let text = source(body);
    for wanted in with {
        assert!(text.contains(wanted), "no `{wanted}` in:\n{text}");
    }
    for unwanted in without {
        assert!(!text.contains(unwanted), "`{unwanted}` in:\n{text}");
    }
}

fn holds(body: &str) {
    if let Err(why) = checked(&FRAME.replace("BODY", body)) {
        panic!("{why}\nin:\n{body}");
    }
}

fn function(body: &str) -> cfg::Function {
    let text = FRAME.replace("BODY", body);
    let listing = Listing::open(&text).expect("the listing opens");
    let f = listing.functions().next().expect("a function");
    cfg::lower(f.expect("the function reads")).expect("the function lowers")
}

fn differ(was: &str, is: &str) -> bool {
    let (was, is) = (function(was), function(is));
    (0..SEEDS).any(|seed| same(&run(&was, seed), &run(&is, seed)).is_err())
}

#[test]
fn the_oracle_tells_two_orders_of_calls_apart() {
    assert!(differ(
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); return v_000010_0;",
        "Native.DEMO.USE(p1); v_000010_0 = Native.DEMO.GET(p0); return v_000010_0;",
    ));
    assert!(!differ(
        "v_000010_0 = Native.DEMO.GET(p0); return v_000010_0;",
        "return Native.DEMO.GET(p0);",
    ));
}

#[test]
fn the_oracle_sees_a_load_moved_past_a_call_and_past_a_store() {
    assert!(differ(
        "v_000010_0 = load_u64(p0, 0x10); Native.DEMO.USE(p1); return v_000010_0;",
        "Native.DEMO.USE(p1); v_000010_0 = load_u64(p0, 0x10); return v_000010_0;",
    ));
    assert!(differ(
        "v_000010_0 = load_u64(p0, 0x10); store_u64(p0, 0x10, 5UL); return v_000010_0;",
        "store_u64(p0, 0x10, 5UL); v_000010_0 = load_u64(p0, 0x10); return v_000010_0;",
    ));
}

#[test]
fn the_oracle_lets_a_callee_read_and_write_a_local_whose_address_it_was_given() {
    // The callee reads the cell: assigning it after the call is another program.
    assert!(differ(
        "local_3 = 7; Native.DEMO.USE(Vm.Address(locals, 3)); return 0;",
        "Native.DEMO.USE(Vm.Address(locals, 3)); local_3 = 7; return 0;",
    ));
    // The callee writes the cell, and the one after it: a read before the call is not a read after it.
    assert!(differ(
        "local_4 = 7; Native.DEMO.USE(Vm.Address(locals, 3)); return local_4;",
        "local_4 = 7; v_000010_0 = local_4; Native.DEMO.USE(Vm.Address(locals, 3)); return v_000010_0;",
    ));
    // The cell before it is out of the callee's reach.
    assert!(!differ(
        "local_2 = 7; Native.DEMO.USE(Vm.Address(locals, 3)); return local_2;",
        "local_2 = 7; v_000010_0 = local_2; Native.DEMO.USE(Vm.Address(locals, 3)); return v_000010_0;",
    ));
}

#[test]
fn the_oracle_tells_a_narrow_store_from_a_whole_one() {
    assert!(differ(
        "local_1 = p0; store_i32(Vm.Address(locals, 1), 0x0, 5); return local_1;",
        "local_1 = p0; local_1 = 5; return local_1;",
    ));
}

#[test]
fn the_passes_keep_what_these_functions_do() {
    // Temporaries, a join, a loop with a step, a switch, a value of two cells and an out buffer.
    for body in [
        "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = load_i32(p1, 0x10); Native.DEMO.USE2(v_000018_1, v_000010_0); return v_000010_0;",
        "if (((as_u32(p0) == 0))) { stack_000040_0 = Native.DEMO.GET(p1); } else { stack_000040_0 = 7; } local_2 = stack_000040_0; Native.DEMO.USE(local_2); return local_2;",
        "local_1 = 0; while (Truth(true)) { if (((as_i32(local_1) >= as_i32(p0)))) { break; } else { } Native.DEMO.USE(local_1); local_1++; } return local_1;",
        "switch (as_i32(((as_i32(p0) - 3)))) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; case 2: return 5; default: break; } return p1;",
        "v_000028_3 = Native.DEMO.PAIR(); local_4 = v_000028_3[0]; local_5 = v_000028_3[1]; Native.DEMO.USE2(local_4, local_5); return local_5;",
        "v_000028_3 = zero_slots(2); Native.DEMO.USE(Vm.Address(v_000028_3, 0)); local_1 = v_000028_3[1]; Native.DEMO.USE(local_1); return v_000028_3[0];",
        "v_000028_3 = zero_slots(1); store_i32(Vm.Address(v_000028_3, 0), 0x0, p0); Native.DEMO.USE(v_000028_3[0]); return 0;",
        "local_3 = load_u64(p0, 0x8); store_i32(local_3, 0x10, ((as_i32(load_i32(local_3, 0x10)) + as_i32(1)))); Native.DEMO.USE(Vm.Address(locals, 3)); return local_3;",
    ] {
        holds(body);
    }
}

#[test]
fn a_local_given_a_temporarys_reads_is_not_then_counted_by_its_old_reads() {
    // `local_0` takes the value and the read of the temporary; counted by the reads it had
    // before, it would look read in this block alone and be renamed away under the `return`.
    holds(
        "v_000010_0 = Native.DEMO.GET(p0); local_0 = v_000010_0; local_1 = v_000010_0;          if (((as_i32(local_0) == as_i32(3)))) { local_1 = 1; } else { }          Native.DEMO.USE(local_1); return local_0;",
    );
}

#[test]
fn a_local_only_ever_cleared_is_zero_where_it_is_read() {
    reads(
        "zero_bytes(Vm.Address(locals, 1), 4); Native.DEMO.USE(local_1); return 0;",
        &["    DEMO.USE(0);"],
        &["zero_bytes", "local_1"],
    );
    // Assigned something else on one path, it is a variable.
    reads(
        "zero_bytes(Vm.Address(locals, 1), 4); if (((as_u32(p0) != 0))) { local_1 = p1; } else { } Native.DEMO.USE(local_1); return 0;",
        &["DEMO.USE(local_1);"],
        &[],
    );
    // Its address handed to a call, the callee may have written it.
    reads(
        "zero_bytes(Vm.Address(locals, 1), 4); Native.DEMO.USE(Vm.Address(locals, 1)); Native.DEMO.USE(local_1); return 0;",
        &["DEMO.USE(&local_1);", "DEMO.USE(local_1);"],
        &[],
    );
}

#[test]
fn a_test_of_constants_goes_one_way() {
    reads(
        "local_1 = 1; if (((as_u32(local_1) == 0))) { Native.DEMO.USE(1); } else { Native.DEMO.USE(2); } return 0;",
        &["    DEMO.USE(2);"],
        &["DEMO.USE(1)", "if ("],
    );
    reads(
        "if (((as_u32(p0) == 0))) { Native.DEMO.USE(1); } else { Native.DEMO.USE(2); } return 0;",
        &["if (", "DEMO.USE(1);", "DEMO.USE(2);"],
        &[],
    );
}

#[test]
fn a_clear_of_whole_cells_makes_one_value_of_the_locals_it_clears() {
    reads(
        "zero_bytes(Vm.Address(locals, 2), 16); store_i32(Vm.Address(locals, 2), 0x8, p0); Native.DEMO.USE(Vm.Address(locals, 2)); return 0;",
        &["var[2] local_2", "local_2 = default;", "local_2.f_8 = ", "DEMO.USE(&local_2);"],
        &["zero_bytes"],
    );
    // One of the locals named apart, the clear stays the clear of cells it is.
    reads(
        "zero_bytes(Vm.Address(locals, 2), 16); local_3 = p0; Native.DEMO.USE2(local_2, local_3); return 0;",
        &["zero_bytes(&local_2, 16);"],
        &["var[2]"],
    );
}

#[test]
fn an_address_a_local_only_loads_and_stores_through_reaches_nothing_else() {
    // `local_1` holds the address of `local_0` and is only stored through: the store is one of
    // `local_0`, and `local_4` is out of every pointer's reach.
    reads(
        "local_1 = Vm.Address(locals, 0); store_i32(local_1, 0x0, p0); local_4 = Native.DEMO.GET(p1); Native.DEMO.USE2(local_4, local_0); return 0;",
        &["DEMO.USE2(DEMO.GET(p1), local_0);"],
        &["local_1", "local_4"],
    );
    // Handed to a call, the address reaches every local from `local_0` on.
    reads(
        "local_1 = Vm.Address(locals, 0); Native.DEMO.USE(local_1); local_4 = Native.DEMO.GET(p1); Native.DEMO.USE2(local_4, local_0); return 0;",
        &["var get = DEMO.GET(p1);", "DEMO.USE2(get, local_0);"],
        &[],
    );
    // A holder above an address that escapes can be read by the callee, and what it holds
    // escapes with it: a call may write `local_3`, so the read of it stays before the call.
    reads(
        "local_5 = Vm.Address(locals, 3); local_3 = 7; Native.DEMO.USE(Vm.Address(locals, 4)); v_000010_0 = ((as_i32(local_3) + as_i32(1))); Native.DEMO.USE(p0); Native.DEMO.USE(v_000010_0); return 0;",
        &["local_3 + 1;\n    DEMO.USE(p0);"],
        &[],
    );
    // A store through the holder writes the local it holds the address of: a read of that local
    // does not move past it.
    holds(
        "local_1 = Vm.Address(locals, 0); local_0 = 5; v_000010_0 = ((as_i32(local_0) + as_i32(1))); store_i32(local_1, 0x0, p0); store_i32(local_1, 0x4, p1); Native.DEMO.USE(v_000010_0); return local_0;",
    );
}

#[test]
fn an_object_made_and_constructed_side_by_side_is_one_construction() {
    let made =
        "v_000010_0 = Vm.NewObject(new VmType(\"System.Delegate\", 48, 0x80000002U, 0x000C2000U));";
    let ctor = "Builtins.System.Delegate.Ctor(v_000010_0, p0, Vm.Function(0x00000001U, 0x000010));";
    reads(
        &format!("{made} {ctor} Native.DEMO.USE2(v_000010_0, v_000010_0); return 0;"),
        &[" = new System.Delegate(p0, &func_000010);"],
        &["Delegate.Ctor("],
    );
    // With a call between the two, the object is in a callee's hands before it is constructed.
    reads(
        &format!(
            "{made} Native.DEMO.USE(v_000010_0); {ctor} Native.DEMO.USE(v_000010_0); return 0;"
        ),
        &[" = new System.Delegate();", "System.Delegate.Ctor("],
        &[],
    );
}

#[test]
fn the_equality_of_two_values_of_several_cells_is_written_with_its_sign() {
    let pairs = "v_000028_3 = Native.DEMO.PAIR(); local_1 = v_000028_3[0]; local_2 = v_000028_3[1]; v_000030_4 = Native.DEMO.PAIR(); local_3 = v_000030_4[0]; local_4 = v_000030_4[1];";
    let equal = "Builtins.System.Guid.op_Equality(local_1, local_2, local_3, local_4)";
    reads(
        &format!(
            "{pairs} if (((as_u32({equal}) == 0))) {{ Native.DEMO.USE(1); }} else {{ }} return 0;"
        ),
        &["if (DEMO.PAIR() != DEMO.PAIR())"],
        &["op_Equality"],
    );
    // Four cells that are no two values stay the call they are.
    reads(
        "if (((as_u32(Builtins.System.Guid.op_Equality(p0, p1, p0, p1)) != 0))) { Native.DEMO.USE(1); } else { } return 0;",
        &["System.Guid.op_Equality(p0, p1, p0, p1)"],
        &[],
    );
}

#[test]
fn a_cell_that_is_a_negative_int_widened_reads_as_that_int() {
    reads(
        "Native.DEMO.USE(0xFFFFFFFF8E7B3F2C); Native.DEMO.USE(0xFFFFFFFFFFFFFFFE); Native.DEMO.USE(0x1FFFFFFFF); return 0;",
        &["DEMO.USE((int)0x8E7B3F2C);", "DEMO.USE(0x1FFFFFFFF);"],
        &["0xFFFFFFFF8E7B3F2C"],
    );
}

#[test]
fn a_float_an_intrinsic_yields_is_stored_and_read_with_no_cast() {
    reads(
        "store_i32(p0, 0x10, Operations.clamp_f32(load_i32(p0, 0x14), 0, 1.0f)); return 0;",
        &["p0->f_10 = clamp_f32("],
        &["(int)clamp_f32", "(float)clamp_f32"],
    );
}

#[test]
fn a_long_chain_of_tests_is_written_one_test_a_line() {
    let test = |n: u32| format!("((as_i32(load_i32(p0, 0x10)) == as_i32({n})))");
    let chain: Vec<String> = (1..=7).map(test).collect();
    reads(
        &format!(
            "if ({}) {{ Native.DEMO.USE(1); }} else {{ }} return 0;",
            chain.join(" || ")
        ),
        &[
            "    if ((int)p0->f_10 == 1\n        || (int)p0->f_10 == 2\n",
            "        || (int)p0->f_10 == 7)\n",
        ],
        &[],
    );
    // A short one stays on its line.
    reads(
        "if (((as_i32(p0) == as_i32(1))) || ((as_i32(p0) == as_i32(2)))) { Native.DEMO.USE(1); } else { } return 0;",
        &["    if (p0 == 1 || p0 == 2)\n"],
        &[],
    );
}

#[test]
fn tests_of_different_bits_of_one_variable_are_no_one_switch() {
    let arm = |test: &str, n: u32, rest: &str| {
        format!("if ((({test} == as_i32({n})))) {{ Native.DEMO.USE({n}); }} else {{ {rest} }}")
    };
    // One of the four reads a byte of the variable: a switch on the variable would test it wrong.
    let mixed = arm(
        "as_i32(p0)",
        1,
        &arm(
            "as_i32(as_u8(p0))",
            2,
            &arm("as_i32(p0)", 3, &arm("as_i32(p0)", 4, "")),
        ),
    );
    reads(&format!("{mixed} return 0;"), &["p0 == 2"], &["switch"]);
    let same = arm(
        "as_i32(p0)",
        1,
        &arm(
            "as_i32(p0)",
            2,
            &arm("as_i32(p0)", 3, &arm("as_i32(p0)", 4, "")),
        ),
    );
    reads(
        &format!("{same} return 0;"),
        &["switch (p0)", "case 4:"],
        &[],
    );
}

#[test]
fn a_switch_on_a_cut_difference_keeps_its_difference() {
    // `(byte)(p0 - 3)` is no `p0` with three added to each case: the cut comes after the difference.
    reads(
        "switch (as_i32(as_u8(((as_i32(p0) - as_i32(3)))))) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; case 2: Native.DEMO.USE(3); break; default: break; } return 0;",
        &["- 3", "case 0:"],
        &["case 5:"],
    );
    reads(
        "switch (as_i32(((as_i32(p0) - as_i32(3))))) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; case 2: Native.DEMO.USE(3); break; default: break; } return 0;",
        &["switch (p0)", "case 5:"],
        &["- 3"],
    );
}

#[test]
fn a_group_that_holds_a_whole_value_is_not_said_to_be_as_wide_as_its_items() {
    // Two items, three cells: the count of items is no width.
    reads(
        "v_000028_3 = Native.DEMO.PAIR(); v_000030_4 = new VmValues(v_000028_3[0], v_000028_3[1], p0); Native.DEMO.USE(Vm.Address(v_000030_4, 0)); return 0;",
        &["var[] v_000030_4 = "],
        &["var[2] v_000030_4"],
    );
}

#[test]
fn whole_cells_loaded_and_stored_are_a_field_as_wide() {
    reads(
        "v_000028_3 = load_bytes(p0, 0x20, 16); store_bytes(p1, 0x8, 16, new VmValues(v_000028_3[0], v_000028_3[1])); return 0;",
        &["    p1->f_8 = (var[2])p0->f_20;"],
        &["load_bytes", "store_bytes"],
    );
    // Twelve bytes are no whole cells: the store says how many it writes.
    reads(
        "store_bytes(p1, 0x8, 12, new VmValues(p0, p1)); return 0;",
        &["store_bytes(p1, 0x8, 12, { p0, p1 });"],
        &[],
    );
}

#[test]
fn a_copy_of_a_whole_value_reads_as_the_value_it_copies() {
    reads(
        "v_000028_3 = Native.DEMO.PAIR(); local_4 = v_000028_3[0]; local_5 = v_000028_3[1]; local_6 = local_4; local_7 = local_5; Native.DEMO.USE2(local_6, local_7); Native.DEMO.USE2(local_4, local_5); return 0;",
        &["var[2] pair = DEMO.PAIR();", "DEMO.USE2(pair);\n    DEMO.USE2(pair);"],
        &["local_6"],
    );
}

#[test]
fn a_value_of_a_billion_cells_is_not_made() {
    // The count is the listing's word: the oracle records that it did not run, and sizes nothing.
    let f = function("v_000028_3 = zero_slots(4000000000); return 0;");
    assert!(run(&f, 0).unsupported.is_some());
    let small = function("v_000028_3 = zero_slots(2); return 0;");
    assert!(run(&small, 0).unsupported.is_none());
}

#[test]
fn a_run_marks_the_blocks_it_enters() {
    let f = function(
        "if (((as_u32(p0) == 0))) { Native.DEMO.USE(1); } else { Native.DEMO.USE(2); } return 0;",
    );
    // One arm of the two is left out by any one run, and so by what that run compares.
    let entered = |seed| run(&f, seed).reached.iter().filter(|e| **e).count();
    assert_eq!(entered(0) + 1, f.cfg.blocks.len());
    let checked = check_blocks(&f, &Context::baseline(&Names::default()), Limits::DEFAULT);
    let checked = checked.expect("the passes keep it");
    assert_eq!(checked.blocks as usize, f.cfg.blocks.len());
    assert!(checked.reached >= entered(0) as u32 && checked.reached <= checked.blocks);
    assert_eq!(u64::from(checked.seeds), SEEDS);
}

#[test]
fn a_parameter_that_is_one_value_of_two_cells_is_passed_whole() {
    let mut f = function("Native.DEMO.USE(p0); return 0;");
    let passed = |f: &cfg::Function| match run(f, 0).events.first() {
        Some(Event::Call { args, .. }) => args.len(),
        other => panic!("no call: {other:?}"),
    };
    assert_eq!(passed(&f), 1);
    let mut pair = VarInfo::new(Width::Cells);
    pair.kind = Kind::Value {
        ty: "type_0000000000000001".to_owned(),
        cells: 2,
    };
    f.head.vars.insert(Var::Param(0), pair);
    assert_eq!(passed(&f), 2);
}

#[test]
fn two_forms_that_differ_do_not_agree() {
    let was = function("v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); return v_000010_0;");
    let is = function("Native.DEMO.USE(p1); v_000010_0 = Native.DEMO.GET(p0); return v_000010_0;");
    let none = Project::empty();
    let all = Agreed {
        skipped: false,
        whole: true,
    };
    assert_eq!(agree(&was, &was.clone(), Limits::DEFAULT, none), Ok(all));
    assert!(agree(&was, &is, Limits::DEFAULT, none).is_err());
}

/// The blocks as a body of labels and jumps, one label a block, for reading them.
fn flat(f: &cfg::Function) -> tree::Function {
    let label = |b: cfg::BlockId| tree::Label::Block(b as u32);
    let go = |b: cfg::BlockId| vec![tree::Stmt::Goto(label(b))];
    let mut body = Vec::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        body.push(tree::Stmt::Label(label(b)));
        body.extend(block.stmts.iter().cloned().map(tree::Stmt::Simple));
        body.push(match &block.term {
            Terminator::Jump(to) => tree::Stmt::Goto(label(*to)),
            Terminator::Branch { cond, then, els } => tree::Stmt::If {
                cond: cond.clone(),
                then: go(*then),
                els: go(*els),
            },
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let case = |at: Option<i64>, to: cfg::BlockId| tree::Case {
                    labels: vec![at],
                    body: go(to),
                };
                let mut all: Vec<tree::Case> =
                    cases.iter().map(|(at, to)| case(Some(*at), *to)).collect();
                all.push(case(None, *default));
                tree::Stmt::Switch {
                    value: value.clone(),
                    cases: all,
                }
            }
            Terminator::Return(value) => tree::Stmt::Return(value.clone()),
            Terminator::Unreachable => tree::Stmt::Unreachable,
        });
    }
    tree::Function {
        head: f.head.clone(),
        body,
    }
}

#[test]
#[ignore = "prints the blocks of the function OPENRAGE_LIFTED_SHOW names as `listing path|function`; run by name"]
fn shows_the_blocks_of_one_function_after_each_pass() {
    let named = std::env::var("OPENRAGE_LIFTED_SHOW").expect("the function is named");
    let (path, wanted) = named.split_once('|').expect("`path|function`");
    let text = std::fs::read_to_string(path).expect("the listing is text");
    let listing = Listing::open(&text).expect("the listing opens");
    let names = Names::default();
    for f in listing.functions() {
        let f = f.expect("the function reads");
        if f.head.name != wanted {
            continue;
        }
        let shown = |f: &cfg::Function| {
            let mut writer = super::print::ListingWriter::new(f.head.program, &[]);
            writer.function(&flat(f));
            let text = writer.finish();
            let body: Vec<&str> = text
                .lines()
                .filter(|line| !line.contains("ref VmValue ") && !line.contains(" = default;"))
                .filter(|line| !line.contains(" = null!;"))
                .skip(3)
                .take_while(|line| !line.contains("    public static class "))
                .collect();
            body.join("\n")
        };
        let show =
            |stage: &str, f: &cfg::Function| eprintln!("======== {stage} ========\n{}", shown(f));
        let mut stats = Stats::default();
        let mut f = cfg::lower(f).expect("the function lowers");
        let read = f.clone();
        show("read", &f);
        passes::values::run(&mut f, &names, &mut stats);
        show("values", &f);
        let valued = f.clone();
        passes::types::run(&mut f, &names, &mut stats);
        show("types", &f);
        // The events of the first seed two stages part on, up to where they part.
        for (stage, is) in [("values", &valued), ("types", &f)] {
            for seed in 0..SEEDS {
                let (was, now) = (run(&read, seed), run(is, seed));
                if same(&was, &now).is_err() {
                    let shared = was.events.iter().zip(&now.events);
                    let parted = shared.take_while(|(a, b)| a == b).count();
                    eprintln!("======== {stage}, seed {seed}: parts at event {parted} ========");
                    for n in parted.saturating_sub(6)..=parted {
                        eprintln!(
                            "{n}: {:X?}
   {:X?}",
                            was.events.get(n),
                            now.events.get(n)
                        );
                    }
                    break;
                }
            }
        }
        // Every pass switched on after those before it, as `check_blocks` does, with the project
        // the corpus suites are given: the blocks each leaves where it changes them, and the
        // event its result first parts from the function read at.
        let whole = corpus::whole_project();
        let project = whole.as_ref().unwrap_or(Project::empty());
        let mut some = Options {
            level: Level::Source,
            ..Options::at(Level::Lifted)
        };
        let mut last = String::new();
        let mut before = read.clone();
        for pass in Options::default().passes.iter() {
            some = some.with(pass);
            let cx = Context::new(&names, project, some);
            let mut done = super::blocks_in(read.clone(), &cx, &mut Stats::default());
            if done.head.returns == Returns::Void {
                done.body.push(tree::Stmt::Return(None));
            }
            let back = cfg::lower(done).expect("the function lowers back");
            let text = shown(&back);
            if text != last {
                eprintln!("======== with {} ========\n{text}", pass.name());
                last = text;
            }
            // What the listing's syntax does not write: the statements whose debug form the pass changed.
            let stmts = |f: &cfg::Function| -> Vec<String> {
                let all = f.cfg.blocks.iter().flat_map(|block| {
                    let term = format!("{:?}", block.term);
                    (block.stmts.iter().map(|stmt| format!("{stmt:?}"))).chain([term])
                });
                all.collect()
            };
            let (old, new) = (stmts(&before), stmts(&back));
            if old.len() == new.len() {
                let changed = old.iter().zip(&new).filter(|(a, b)| a != b);
                for (a, b) in changed.take(12) {
                    let (a, b): (String, String) =
                        (a.chars().take(900).collect(), b.chars().take(900).collect());
                    eprintln!(
                        "---- {}: a statement was\n{a}\n---- and is\n{b}",
                        pass.name()
                    );
                }
            }
            before = back.clone();
            // Every seed the corpus suites may run a function on.
            for seed in 0..SEEDS + Limits::CORPUS.more {
                let was = run_in(&read, seed, Limits::CORPUS, project);
                let now = run_in(&back, seed, Limits::CORPUS, project);
                if same(&was, &now).is_err() {
                    let shared = was.events.iter().zip(&now.events);
                    let parted = shared.take_while(|(a, b)| a == b).count();
                    eprintln!(
                        "======== with {}, seed {seed}: parts at event {parted} ========",
                        pass.name()
                    );
                    for n in parted.saturating_sub(3)..=parted {
                        let (a, b) = (was.events.get(n), now.events.get(n));
                        eprintln!("{n}: {a:X?}\n   {b:X?}");
                    }
                    break;
                }
            }
        }
    }
}
