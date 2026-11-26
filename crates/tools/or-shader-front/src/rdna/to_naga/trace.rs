//! A program that intersects rays itself, lowered as its code states it: the structure it
//! walks is memory, one buffer of dwords the binder fills (the arena), and the instruction that
//! tests a ray against one node is a call of one function of the module.

use naga::{
    BinaryOperator as Bo, Block, Expression as E, Handle, MathFunction as Mf, Scalar, Span,
    Statement, TypeInner, VectorSize,
};
use or_io::gta6::accel::layout::{
    BOX_BOUNDS, BOX_BOUNDS_STRIDE, BOX_CHILDREN, CORNERS, INVALID, TRIANGLE_ID,
    TRIANGLE_VERTEX_STRIDE, TYPE_BOX32, UNIT,
};
use or_io::gta6::accel::MISS;

use std::collections::{BTreeMap, BTreeSet};

use super::flow::{self, Flow};
use super::{
    form, refused, vector, Builder, Cx, Done, End, NotTranslated, Reg, ResourceKind, EXEC,
    MAX_NESTING, SPREAD_NESTING,
};
use crate::rdna::cfg::{self, Cfg, Terminator};
use crate::rdna::decode::Inst;
use crate::rdna::sym::Sym;

/// The words an intersection's descriptor states after the structure's address: the children
/// of a box sorted nearest first, the whole address range, and a triangle answered as a
/// distance's numerator and denominator with two barycentric numerators. The function below
/// is written to this contract and no other.
const MODES: [u32; 3] = [0x8000_0000, 0xFFFF_FFFF, 0x8900_03FF];
const OTHER_MODES: &str = "a descriptor that does not state sorted children, the whole address \
    range and barycentrics";
/// The 64-bit form's mnemonic.
const WIDE: &str = "image_bvh64_intersect_ray";
/// The instructions a walk keeps its stack with, in the memory its group shares. A program that
/// tests nodes and holds any other instruction of that memory has lanes that read what other
/// lanes stored: its own compiler left out the barrier between the two, since its group is one
/// wave in step. A lane an invocation stops there ([`NOT_THE_STACK`]), and the program is
/// lowered with its wave kept whole, which puts the barrier back.
// ponytail: told by the instruction's name, not by its address: memory that is no stack, stored
// and loaded with these four alone, passes as a lane's own. No program of the game does that;
// the wave kept whole meets before every instruction of that memory, whatever its name.
const STACK: [&str; 4] = [
    "ds_write_b32",
    "ds_write2_b32",
    "ds_read2_b32",
    "ds_wrxchg_rtn_b32",
];
pub(super) const NOT_THE_STACK: &str = "memory of the group that is no walk's stack: another \
    lane's store is read with no barrier between the two";
/// The first node type that is no triangle.
const BOX_TYPES: u32 = 4;
/// The least direction component a slab is divided by, and the least determinant a triangle is
/// kept with: the least normal float, 2^-126.
const MIN: f32 = f32::MIN_POSITIVE;
/// The greatest determinant a triangle is kept with, 2^126.
const DET_MAX: f32 = 1.0 / MIN;
/// The share of a box's greatest distance from the ray's origin it is widened by, 2^-20.
const BOX_GROW: f32 = 1.0 / 1_048_576.0;
/// The same for a triangle's own bounds, 2^-21: half a box's, so a box that holds a triangle
/// holds its widened bounds with the rounding of both to spare.
const TRI_GROW: f32 = 1.0 / 2_097_152.0;

/// Every number of the arena's layout the test of a node is written from, in a fixed order: a
/// kept translation is of these as it is of this front end's own sources, and they are none of
/// them.
pub fn arena_layout() -> Vec<u32> {
    let mut numbers = vec![
        BOX_BOUNDS,
        BOX_BOUNDS_STRIDE,
        BOX_CHILDREN,
        INVALID,
        TRIANGLE_ID,
        TRIANGLE_VERTEX_STRIDE,
        TYPE_BOX32,
        UNIT,
    ];
    numbers.extend(CORNERS.iter().flatten());
    numbers.extend(MISS);
    numbers
}

/// One instruction that tests a ray against a node, and where its call is in the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intersection {
    /// The word of the instruction.
    pub word: u32,
    /// Whether its node is the 64-bit form's, which holds the structure's address too.
    pub wide: bool,
    /// The function of the module the call is in.
    pub function: String,
    /// The ifs and loops of the program's structured form around it, whatever function each is in.
    pub depth: u32,
    /// The first word of each loop around it, the outermost first.
    pub loops: Vec<u32>,
}

/// What a lowering holds of a program's ray tracing.
#[derive(Default)]
pub(super) struct Trace {
    /// Whether the program's structure is the arena ([`super::Options::arena`]).
    pub(super) on: bool,
    /// Whether the program tests a ray against a node anywhere.
    rays: bool,
    arena: Option<Handle<naga::GlobalVariable>>,
    function: Option<Handle<naga::Function>>,
    /// Each intersection lowered: its block's number, its word, and whether it is the 64-bit form.
    sites: Vec<(u32, u32, bool)>,
    /// The walk lowered alone, where the lowering is of one.
    walk: Option<Walk>,
}

/// A walk lowered alone: the block every way out of it lands on, the registers left there,
/// those it was given, and the two buffers.
struct Walk {
    exit: u32,
    left: Vec<Reg>,
    given: Vec<Reg>,
    buffers: [Option<Handle<naga::GlobalVariable>>; 2],
}

/// One walk of a program, to be lowered alone ([`super::Options::region`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Region {
    /// The word the walk starts at: the first of its loop, as [`Intersection::loops`] names
    /// a loop, or of what is run before it.
    pub first: u32,
    /// The word the program goes on at once the walk is over: nothing from it on is the walk's.
    pub exit: u32,
    /// The registers left in a buffer where the walk ends, by name: `v4`, `s20`.
    pub left: Vec<String>,
}

/// The registers of a walk lowered alone, by slot: invocation `i` is given slot `k` in dword
/// `i * given.len() + k` of one buffer and leaves slot `k` in dword `i * left.len() + k` of the
/// other. An invocation's number is its group's along x times a group's threads, plus its
/// thread's number in the group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registers {
    pub given: Vec<String>,
    pub left: Vec<String>,
}

/// A loop cut out of a program's graph: a graph of its own, and the graph it is of.
pub(super) struct Cut<'a> {
    pub(super) cfg: Cfg,
    pub(super) whole: &'a Cfg,
    exit: u32,
    left: Vec<Reg>,
}

const NO_BLOCK: NotTranslated = NotTranslated::Pipeline(
    "a walk lowered alone: no block of the program starts at a word named, or one before it would",
);
const NO_REGISTER: NotTranslated =
    NotTranslated::Pipeline("a walk lowered alone: a register named to be left is none");
/// The block every way out of a walk lowered alone lands on: past any word of a program.
const EXIT: u32 = 1 << 30;

/// How each block of `cfg` hands control on, by block number; `None` with a jump through a register.
fn ends_of(cfg: &Cfg, starts: &[u32]) -> Option<Vec<End>> {
    let place = |start: u32| starts.binary_search(&start).unwrap_or(0) as u32;
    (cfg.blocks.values())
        .map(|block| match block.terminator {
            Terminator::End => Some(End::Return),
            Terminator::Jump(to) | Terminator::Fall(to) => Some(End::Jump(place(to))),
            Terminator::Branch { taken, fallthrough } => Some(End::Branch {
                then: place(taken),
                otherwise: place(fallthrough),
            }),
            Terminator::Indirect => None,
        })
        .collect()
}

/// The blocks of `whole` that `region` names, as a graph of its own: those reached from its
/// first word without passing its exit, one before them that goes to the first where the first
/// is not the lowest, and one that every way out lands on and ends.
pub(super) fn cut<'a>(whole: &'a Cfg, region: &Region) -> Result<Cut<'a>, NotTranslated> {
    if !(whole.blocks.contains_key(&region.first) && whole.blocks.contains_key(&region.exit)) {
        return Err(NO_BLOCK);
    }
    let (mut inside, mut next) = (BTreeSet::new(), vec![region.first]);
    while let Some(start) = next.pop() {
        if start == region.exit || !inside.insert(start) {
            continue;
        }
        match whole.blocks[&start].terminator {
            Terminator::End => {}
            Terminator::Jump(to) | Terminator::Fall(to) => next.push(to),
            Terminator::Branch { taken, fallthrough } => next.extend([taken, fallthrough]),
            Terminator::Indirect => return Err(NO_BLOCK),
        }
    }
    let within = |to: u32| if inside.contains(&to) { to } else { EXIT };
    let mut blocks = BTreeMap::new();
    for start in &inside {
        let mut block = whole.blocks[start].clone();
        block.terminator = match block.terminator {
            // A program that ends inside its walk leaves it there.
            Terminator::End => Terminator::Jump(EXIT),
            Terminator::Jump(to) => Terminator::Jump(within(to)),
            Terminator::Fall(to) => Terminator::Fall(within(to)),
            Terminator::Branch { taken, fallthrough } => Terminator::Branch {
                taken: within(taken),
                fallthrough: within(fallthrough),
            },
            Terminator::Indirect => return Err(NO_BLOCK),
        };
        blocks.insert(*start, block);
    }
    let mut ends = vec![(EXIT, Terminator::End)];
    // The lowest block of a graph is where it is entered.
    let lowest = inside.first().copied().unwrap_or(region.first);
    if lowest != region.first {
        let before = lowest.checked_sub(1).ok_or(NO_BLOCK)?;
        ends.push((before, Terminator::Jump(region.first)));
    }
    for (start, terminator) in ends {
        let block = cfg::Block {
            start,
            insts: Vec::new(),
            terminator,
        };
        blocks.insert(start, block);
    }
    let named = |name: &String| {
        let registers = (0..=255).flat_map(|n| [Reg::V(n), Reg::S(n)]);
        registers.chain([Reg::Scc]).find(|reg| reg.name() == *name)
    };
    let left: Option<Vec<Reg>> = region.left.iter().map(named).collect();
    Ok(Cut {
        cfg: Cfg { blocks },
        whole,
        exit: EXIT,
        left: left.ok_or(NO_REGISTER)?,
    })
}

impl Trace {
    pub(super) fn of(on: bool, cfg: &Cfg, walk: Option<&Cut>) -> Self {
        Self {
            on,
            walk: walk.map(|walk| Walk {
                exit: walk.exit,
                left: walk.left.clone(),
                given: Vec::new(),
                buffers: [None; 2],
            }),
            rays: (cfg.blocks.values().flat_map(|b| &b.insts)).any(|(_, inst)| tests_a_node(inst)),
            ..Self::default()
        }
    }

    /// The registers of the walk lowered alone, where the lowering is of one.
    pub(super) fn registers(&self) -> Option<Registers> {
        let names = |registers: &[Reg]| registers.iter().map(|reg| reg.name()).collect();
        self.walk.as_ref().map(|walk| Registers {
            given: names(&walk.given),
            left: names(&walk.left),
        })
    }

    /// Whether `name`, an instruction of the memory a group shares, is one this lowering has no
    /// order for among the lanes ([`NOT_THE_STACK`]).
    pub(super) fn unordered(&self, name: &str) -> bool {
        self.rays && !STACK.contains(&name)
    }

    /// Where each intersection lowered is in the structured form `nodes`, whose blocks start
    /// at the words of `starts`; all in the one function where the blocks run in a dispatch loop.
    pub(super) fn placed(&self, nodes: Option<&[Flow]>, starts: &[u32]) -> Vec<Intersection> {
        let mut found = Vec::with_capacity(self.sites.len());
        match nodes {
            Some(nodes) => self.place(nodes, starts, (0, 0), &mut Vec::new(), &mut found),
            None => found.extend(self.sites.iter().map(|&(_, word, wide)| Intersection {
                word,
                wide,
                function: function_name(0),
                depth: 0,
                loops: Vec::new(),
            })),
        }
        found.sort_by_key(|site| site.word);
        found
    }

    fn place(
        &self,
        nodes: &[Flow],
        starts: &[u32],
        (home, depth): (usize, u32),
        loops: &mut Vec<u32>,
        found: &mut Vec<Intersection>,
    ) {
        for node in nodes {
            match node {
                Flow::Code(block) => {
                    let here = self.sites.iter().filter(|site| site.0 == *block);
                    found.extend(here.map(|&(_, word, wide)| Intersection {
                        word,
                        wide,
                        function: function_name(home),
                        depth,
                        loops: loops.clone(),
                    }));
                }
                Flow::If {
                    then, otherwise, ..
                } => {
                    self.place(then, starts, (home, depth + 1), loops, found);
                    self.place(otherwise, starts, (home, depth + 1), loops, found);
                }
                Flow::Loop(body) => {
                    let first = first_block(body).and_then(|block| starts.get(block as usize));
                    loops.push(first.copied().unwrap_or(u32::MAX));
                    self.place(body, starts, (home, depth + 1), loops, found);
                    loops.pop();
                }
                Flow::Guard(body) => self.place(body, starts, (home, depth + 1), loops, found),
                Flow::Call(number, body) => {
                    self.place(body, starts, (*number, depth), loops, found);
                }
                _ => {}
            }
        }
    }
}

/// Whether `inst` tests a ray against a node.
pub(super) fn tests_a_node(inst: &Inst) -> bool {
    inst.name().starts_with("image_bvh")
}

/// Where a lowering of the graph `cfg` puts each instruction that tests a ray against a node,
/// whether or not the rest of the program lowers: the structured form, and the functions a deep
/// one is spread over, follow from the graph alone. A graph that does not nest runs in a dispatch
/// loop of one function, as the waves of a group of several do, which this does not ask about.
pub fn intersections(cfg: &Cfg) -> Vec<Intersection> {
    let starts: Vec<u32> = cfg.blocks.keys().copied().collect();
    let ends = ends_of(cfg, &starts);
    let mut trace = Trace::default();
    for (index, block) in cfg.blocks.values().enumerate() {
        let tests = (block.insts.iter()).filter(|(_, inst)| tests_a_node(inst));
        (trace.sites).extend(tests.map(|(pc, inst)| (index as u32, *pc, inst.name() == WIDE)));
    }
    let nodes = ends
        .and_then(|ends| flow::structure(&ends).ok())
        .map(|nodes| match flow::nesting(&nodes) > MAX_NESTING {
            true => flow::outlined(nodes, SPREAD_NESTING, 0, &mut 1),
            false => nodes,
        });
    trace.placed(nodes.as_deref(), &starts)
}

/// The name of the function the statements numbered `home` are written in.
fn function_name(home: usize) -> String {
    match home {
        0 => "main_body".to_owned(),
        number => format!("part{number}"),
    }
}

/// The block a structured form runs first.
fn first_block(nodes: &[Flow]) -> Option<u32> {
    nodes.iter().find_map(|node| match node {
        Flow::Code(block) => Some(*block),
        Flow::If { cond, .. } => Some(*cond),
        Flow::Loop(body) | Flow::Guard(body) | Flow::Call(_, body) => first_block(body),
        _ => None,
    })
}

impl Cx<'_> {
    /// One of the two buffers of a walk lowered alone: the registers it is given, or those it
    /// leaves.
    fn registers_buffer(&mut self, left: bool) -> Handle<naga::GlobalVariable> {
        let held = self
            .trace
            .walk
            .as_ref()
            .and_then(|walk| walk.buffers[usize::from(left)]);
        if let Some(global) = held {
            return global;
        }
        let base = self.ty(TypeInner::Scalar(Scalar::U32));
        let inner = TypeInner::Array {
            base,
            size: naga::ArraySize::Dynamic,
            stride: 4,
        };
        let space = naga::AddressSpace::Storage {
            access: naga::StorageAccess::LOAD,
        };
        let name = if left {
            "registers_left"
        } else {
            "registers_given"
        };
        let global = self.bound(name.to_owned(), inner, space, ResourceKind::Registers, None);
        if left {
            self.stored.insert(global);
        }
        if let Some(walk) = &mut self.trace.walk {
            walk.buffers[usize::from(left)] = Some(global);
        }
        global
    }

    /// The pointer to slot `slot` of this invocation's row of `count` dwords of `global`.
    fn register_slot(
        &mut self,
        global: Handle<naga::GlobalVariable>,
        (slot, count): (usize, usize),
    ) -> Handle<E> {
        let threads = self.launch.map_or([1; 3], |launch| launch.threads);
        let threads = self.b.u32(threads.iter().map(|&n| n.max(1)).product());
        let (group, thread) = (self.launch_number(1, 0), self.wave_thread());
        let before = self.b.bin(Bo::Multiply, group, threads);
        let invocation = self.b.bin(Bo::Add, before, thread);
        let (count, slot) = (self.b.u32(count as u32), self.b.u32(slot as u32));
        let row = self.b.bin(Bo::Multiply, invocation, count);
        let index = self.b.bin(Bo::Add, row, slot);
        let pointer = self.global(global);
        self.b.emit(E::Access {
            base: pointer,
            index,
        })
    }

    /// A walk lowered alone is given every register it reads before it writes it: those of
    /// `live` that are a program's own, which are loaded here. The lane mask is not one: an
    /// invocation of a walk is a lane that runs.
    pub(super) fn region_take(&mut self, live: &BTreeMap<Reg, u32>) -> BTreeSet<Reg> {
        if self.trace.walk.is_none() {
            return BTreeSet::new();
        }
        let given: Vec<Reg> = (live.keys().copied())
            .filter(|reg| match *reg {
                Reg::S(r) => r != EXEC && r != EXEC + 1,
                Reg::V(_) | Reg::Scc | Reg::Spill(..) => true,
                _ => false,
            })
            .collect();
        if !given.is_empty() {
            let global = self.registers_buffer(false);
            for (slot, reg) in given.iter().enumerate() {
                let cell = self.register_slot(global, (slot, given.len()));
                let value = self.b.load(cell);
                self.put(*reg, value);
            }
        }
        if let Some(walk) = &mut self.trace.walk {
            walk.given.clone_from(&given);
        }
        given.into_iter().collect()
    }

    /// Where a walk lowered alone ends, the registers named are left in its second buffer.
    pub(super) fn region_leave(&mut self, block: &cfg::Block) {
        let left = match &self.trace.walk {
            Some(walk) if walk.exit == block.start && !walk.left.is_empty() => walk.left.clone(),
            _ => return,
        };
        let global = self.registers_buffer(true);
        for (slot, reg) in left.iter().enumerate() {
            let value = self.get(block.start, *reg);
            let pointer = self.register_slot(global, (slot, left.len()));
            self.b.push(Statement::Store { pointer, value });
        }
    }

    /// The arena's dword at a byte address: the structures a program walks lie in one buffer,
    /// and an address is a byte offset into it, below 4 GiB, so the high dword of an address
    /// the code carries in a register pair is nought and is not read.
    pub(super) fn arena_word(&mut self, bytes: Handle<E>) -> Handle<E> {
        let global = match self.trace.arena {
            Some(global) => global,
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
                let global =
                    self.bound("arena".to_owned(), inner, space, ResourceKind::Arena, None);
                self.trace.arena = Some(global);
                global
            }
        };
        let two = self.b.u32(2);
        let index = self.b.bin(Bo::ShiftRight, bytes, two);
        let pointer = self.global(global);
        let cell = self.b.emit(E::Access {
            base: pointer,
            index,
        });
        self.b.load(cell)
    }

    /// `image_bvh_intersect_ray` and its 64-bit form: the node, the ray's far distance, its
    /// origin, its direction and the direction's inverse go to the module's one test of a
    /// node, and the four dwords it answers to the instruction's four registers.
    pub(super) fn intersect(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Mimg {
            addrs,
            data,
            rsrc,
            dmask,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        if *dmask != 0xF {
            return Err(form(pc, inst, "fewer than its four results"));
        }
        for (k, mode) in (1..).zip(MODES) {
            if self.state.get(usize::from(*rsrc) + k) != Some(&Sym::Const(mode)) {
                return Err(form(pc, inst, OTHER_MODES));
            }
        }
        let wide = name == WIDE;
        // The structure's address in units of 256 bytes, nought where the node holds the address.
        let base = self.sreg(pc, *rsrc)?;
        let node = self.address(pc, inst, addrs, 0)?;
        // The 64-bit node's high dword is nought for a structure in the arena.
        let mut next = if wide { 2 } else { 1 };
        let far = self.address(pc, inst, addrs, next)?;
        let far = self.b.float(far);
        let ty = vector(&mut self.module, VectorSize::Tri, Scalar::F32);
        let mut arguments = vec![base, node, far];
        for _ in 0..3 {
            let mut components = Vec::with_capacity(3);
            for _ in 0..3 {
                next += 1;
                let bits = self.address(pc, inst, addrs, next)?;
                components.push(self.b.float(bits));
            }
            arguments.push(self.b.emit(E::Compose { ty, components }));
        }
        let function = self.node_test();
        let result = self.b.result(E::CallResult(function));
        self.b.push(Statement::Call {
            function,
            arguments,
            result: Some(result),
        });
        for k in 0..4 {
            let value = self.b.index(result, k);
            self.vput(data.wrapping_add(k as u8), value);
        }
        self.trace.sites.push((self.index, pc, wide));
        Ok(())
    }

    /// The module's test of a ray against one node, written on its first use:
    /// `bvh_intersect(base, node, far, origin, direction, inverse)`. The node's bytes are at
    /// `(base << 8) + ((node & ~7) << 3)` of the arena and `node & 7` is its type. A box
    /// answers its children the ray meets before `far`, the nearest first, then `0xFFFFFFFF`
    /// for the others; a triangle answers the bits of four floats, the distance's numerator
    /// and denominator and the numerators of two barycentrics, and `(+inf, 1, 0, 0)` for a miss.
    ///
    /// The arithmetic is `or_io::gta6::accel::intersect_ray`'s, operation for operation: single
    /// precision, one rounded operation where one is written, every test in its positive form.
    /// A device that fuses a product into a sum answers other bits; the tests' sides are read by
    /// comparing products, which no fusing changes.
    fn node_test(&mut self) -> Handle<naga::Function> {
        if let Some(function) = self.trace.function {
            return function;
        }
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let float = self.ty(TypeInner::Scalar(Scalar::F32));
        let three = vector(&mut self.module, VectorSize::Tri, Scalar::F32);
        let four = vector(&mut self.module, VectorSize::Quad, Scalar::U32);
        let outer = std::mem::replace(&mut self.b, Builder::new());
        self.node_body(four);
        let mut function = std::mem::replace(&mut self.b, outer).finish();
        function.name = Some("bvh_intersect".to_owned());
        function.arguments = [
            ("base", word),
            ("node", word),
            ("far", float),
            ("origin", three),
            ("direction", three),
            ("inverse", three),
        ]
        .into_iter()
        .map(|(name, ty)| naga::FunctionArgument {
            name: Some(name.to_owned()),
            ty,
            binding: None,
        })
        .collect();
        function.result = Some(naga::FunctionResult {
            ty: four,
            binding: None,
        });
        let function = self.module.functions.append(function, Span::UNDEFINED);
        self.trace.function = Some(function);
        function
    }

    /// The statements of [`Cx::node_test`], written where the builder stands.
    fn node_body(&mut self, four: Handle<naga::Type>) {
        let [base, node, far] = [0, 1, 2].map(|k| self.b.emit(E::FunctionArgument(k)));
        let [origin, direction, inverse] = [3, 4, 5].map(|k| {
            let whole = self.b.emit(E::FunctionArgument(k));
            [0, 1, 2].map(|part| self.b.index(whole, part))
        });
        let (eight, three, six) = (self.b.u32(8), self.b.u32(3), self.b.u32(UNIT.ilog2()));
        let structure = self.b.bin(Bo::ShiftLeft, base, eight);
        let unit = self.b.bin(Bo::ShiftRight, node, three);
        let within = self.b.bin(Bo::ShiftLeft, unit, six);
        let at = self.b.bin(Bo::Add, structure, within);
        let (types, boxes, wide) = (self.b.u32(7), self.b.u32(BOX_TYPES), self.b.u32(TYPE_BOX32));
        let kind = self.b.bin(Bo::And, node, types);
        let triangle = self.b.bin(Bo::Less, kind, boxes);
        let boxed = self.b.bin(Bo::Equal, kind, wide);
        let ray = (origin, direction, inverse);

        self.b.open();
        let answer = self.triangle_test(four, (at, kind), ray);
        self.b.push(Statement::Return {
            value: Some(answer),
        });
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: triangle,
            accept,
            reject: Block::new(),
        });

        self.b.open();
        let answer = self.box_test(four, at, far, ray);
        self.b.push(Statement::Return {
            value: Some(answer),
        });
        let accept = self.b.close();
        self.b.push(Statement::If {
            condition: boxed,
            accept,
            reject: Block::new(),
        });

        // An instance, a box of halves, which no structure here holds, and a node of no kind.
        let none = self.b.u32(INVALID);
        let none = self.b.emit(E::Compose {
            ty: four,
            components: vec![none; 4],
        });
        self.b.push(Statement::Return { value: Some(none) });
    }

    /// Three floats of the arena from the byte `first`.
    fn arena_floats(&mut self, first: Handle<E>) -> [Handle<E>; 3] {
        [0, 4, 8].map(|by| {
            let by = self.b.u32(by);
            let bytes = self.b.bin(Bo::Add, first, by);
            let bits = self.arena_word(bytes);
            self.b.float(bits)
        })
    }

    /// A box node's answer: its children the ray meets before `far`, by the distance each is
    /// entered at and then by their place in the node, and no node for the others.
    fn box_test(
        &mut self,
        four: Handle<naga::Type>,
        at: Handle<E>,
        far: Handle<E>,
        (origin, direction, inverse): Ray,
    ) -> Handle<E> {
        let none = self.b.u32(INVALID);
        let mut children = Vec::with_capacity(4);
        for i in 0..4 {
            let by = self.b.u32(BOX_CHILDREN + 4 * i);
            let bytes = self.b.bin(Bo::Add, at, by);
            let child = self.arena_word(bytes);
            let present = self.b.bin(Bo::NotEqual, child, none);
            let bounds = BOX_BOUNDS + BOX_BOUNDS_STRIDE * i;
            let (least, greatest) = (self.b.u32(bounds), self.b.u32(bounds + 12));
            let (least, greatest) = (
                self.b.bin(Bo::Add, at, least),
                self.b.bin(Bo::Add, at, greatest),
            );
            let (least, greatest) = (self.arena_floats(least), self.arena_floats(greatest));
            let f = &mut self.b;
            let low = [0, 1, 2].map(|k| f.bin(Bo::Subtract, least[k], origin[k]));
            let high = [0, 1, 2].map(|k| f.bin(Bo::Subtract, greatest[k], origin[k]));
            // The origin's own coordinates are in it for the rounding of a ray turned into an
            // instance's space, which is of their size.
            let mut most = f.f32(0.0);
            for value in origin {
                most = f.greater_size(most, value);
            }
            for k in 0..3 {
                most = f.greater_size(most, low[k]);
                most = f.greater_size(most, high[k]);
            }
            let grow = f.fbits(BOX_GROW);
            let grow = f.bin(Bo::Multiply, most, grow);
            let mut stretch = (f.fbits(f32::NEG_INFINITY), f.fbits(f32::INFINITY));
            let mut met = present;
            for k in 0..3 {
                let (low, high) = (
                    f.bin(Bo::Subtract, low[k], grow),
                    f.bin(Bo::Add, high[k], grow),
                );
                // False for bounds that are not numbers, or inside out.
                let ordered = f.bin(Bo::LessEqual, low, high);
                let (crossed, narrowed) =
                    f.narrow((low, high), (direction[k], inverse[k]), stretch);
                stretch = narrowed;
                met = f.both(met, ordered);
                met = f.both(met, crossed);
            }
            let (near, beyond) = stretch;
            let zero = f.f32(0.0);
            for holds in [
                f.bin(Bo::GreaterEqual, beyond, zero),
                f.bin(Bo::GreaterEqual, beyond, near),
                f.bin(Bo::Less, near, far),
            ] {
                met = f.both(met, holds);
            }
            children.push((met, near, child));
        }
        // A child's place in the answer: the children met that are nearer, and those as near
        // that come before it in the node.
        let f = &mut self.b;
        let (zero, one) = (f.u32(0), f.u32(1));
        let mut places = Vec::with_capacity(4);
        for (i, &(_, near, _)) in children.iter().enumerate() {
            let mut place = zero;
            for (j, &(met, other, _)) in children.iter().enumerate() {
                if i == j {
                    continue;
                }
                let mut before = f.bin(Bo::Less, other, near);
                if j < i {
                    let level = f.bin(Bo::Equal, other, near);
                    before = f.bin(Bo::LogicalOr, before, level);
                }
                let before = f.both(met, before);
                let counted = f.select(before, one, zero);
                place = f.bin(Bo::Add, place, counted);
            }
            places.push(place);
        }
        let components = (0..4)
            .map(|slot| {
                let slot = f.u32(slot);
                let mut answer = none;
                for (&(met, _, child), &place) in children.iter().zip(&places) {
                    let here = f.bin(Bo::Equal, place, slot);
                    let here = f.both(met, here);
                    answer = f.select(here, child, answer);
                }
                answer
            })
            .collect();
        f.emit(E::Compose {
            ty: four,
            components,
        })
    }

    /// A triangle's answer, by a test that leaves no crack along an edge two triangles share:
    /// the corners are taken from the origin and sheared so the ray runs along its greatest
    /// axis, and each edge's side is read by comparing its two products.
    #[allow(clippy::too_many_lines)]
    fn triangle_test(
        &mut self,
        four: Handle<naga::Type>,
        (at, kind): (Handle<E>, Handle<E>),
        (origin, direction, inverse): Ray,
    ) -> Handle<E> {
        // The vertices the node's type makes this triangle's corners.
        let mut corners = [[at; 3]; 3];
        for (c, corner) in corners.iter_mut().enumerate() {
            let mut vertex = self.b.u32(TRIANGLE_VERTEX_STRIDE * CORNERS[3][c]);
            for k in (0..3u32).rev() {
                let (this, of) = (
                    self.b.u32(k),
                    self.b.u32(TRIANGLE_VERTEX_STRIDE * CORNERS[k as usize][c]),
                );
                let is = self.b.bin(Bo::Equal, kind, this);
                vertex = self.b.select(is, of, vertex);
            }
            let first = self.b.bin(Bo::Add, at, vertex);
            *corner = self.arena_floats(first);
        }
        let by = self.b.u32(TRIANGLE_ID);
        let bytes = self.b.bin(Bo::Add, at, by);
        let ids = self.arena_word(bytes);

        let f = &mut self.b;
        // The axis the ray runs most along, the lowest on a tie.
        let sizes = direction.map(|part| f.math(Mf::Abs, &[part]));
        let (zero, one, two, three) = (f.u32(0), f.u32(1), f.u32(2), f.u32(3));
        let second = f.bin(Bo::Greater, sizes[1], sizes[0]);
        let (mut kz, most) = (
            f.select(second, one, zero),
            f.select(second, sizes[1], sizes[0]),
        );
        let third = f.bin(Bo::Greater, sizes[2], most);
        kz = f.select(third, two, kz);
        let along = f.pick(direction, kz);
        let (least, size) = (f.fbits(MIN), f.math(Mf::Abs, &[along]));
        let mut hit = f.bin(Bo::GreaterEqual, size, least);
        // The other two, exchanged where the ray runs down its axis so a winding stays one.
        let nothing = f.f32(0.0);
        let down = f.bin(Bo::Less, along, nothing);
        let next = [one, two].map(|by| {
            let sum = f.bin(Bo::Add, kz, by);
            f.bin(Bo::Modulo, sum, three)
        });
        let (kx, ky) = (
            f.select(down, next[1], next[0]),
            f.select(down, next[0], next[1]),
        );
        let axes = [kx, ky, kz];
        let unit = f.f32(1.0);
        let (across, up) = (f.pick(direction, kx), f.pick(direction, ky));
        let (sx, sy, sz) = (
            f.bin(Bo::Divide, across, along),
            f.bin(Bo::Divide, up, along),
            f.bin(Bo::Divide, unit, along),
        );
        // Each corner from the origin on the three axes, then sheared.
        let from = axes.map(|axis| f.pick(origin, axis));
        let q = corners.map(|corner| {
            [0, 1, 2].map(|j| {
                let part = f.pick(corner, axes[j]);
                f.bin(Bo::Subtract, part, from[j])
            })
        });
        let p = q.map(|q| {
            let (x, y) = (f.bin(Bo::Multiply, sx, q[2]), f.bin(Bo::Multiply, sy, q[2]));
            [
                f.bin(Bo::Subtract, q[0], x),
                f.bin(Bo::Subtract, q[1], y),
                f.bin(Bo::Multiply, sz, q[2]),
            ]
        });
        // An edge's two products, the edge opposite corner 0 first.
        let products = [(2, 1), (0, 2), (1, 0)].map(|(a, b): (usize, usize)| {
            (
                f.bin(Bo::Multiply, p[a][0], p[b][1]),
                f.bin(Bo::Multiply, p[a][1], p[b][0]),
            )
        });
        let mut sides = [hit; 2];
        for (side, op) in sides.iter_mut().zip([Bo::GreaterEqual, Bo::LessEqual]) {
            let [first, second, third] = products.map(|(a, b)| f.bin(op, a, b));
            let all = f.both(first, second);
            *side = f.both(all, third);
        }
        let inside = f.bin(Bo::LogicalOr, sides[0], sides[1]);
        hit = f.both(hit, inside);
        let [u, v, w] = products.map(|(a, b)| f.bin(Bo::Subtract, a, b));
        let sum = f.bin(Bo::Add, u, v);
        let determinant = f.bin(Bo::Add, sum, w);
        let (size, greatest) = (f.math(Mf::Abs, &[determinant]), f.fbits(DET_MAX));
        for kept in [
            f.bin(Bo::GreaterEqual, size, least),
            f.bin(Bo::LessEqual, size, greatest),
        ] {
            hit = f.both(hit, kept);
        }
        let parts =
            [(u, p[0][2]), (v, p[1][2]), (w, p[2][2])].map(|(a, b)| f.bin(Bo::Multiply, a, b));
        let sum = f.bin(Bo::Add, parts[0], parts[1]);
        let numerator = f.bin(Bo::Add, sum, parts[2]);
        let (size, greatest) = (f.math(Mf::Abs, &[numerator]), f.fbits(f32::MAX));
        let finite = f.bin(Bo::LessEqual, size, greatest);
        hit = f.both(hit, finite);
        // The triangle's own bounds from the origin, widened, and the stretch of the ray inside
        // them: a ray that passes beside them is no hit, however thin a triangle points at it.
        let bounds = [0, 1, 2].map(|j| {
            let (mut low, mut high) = (q[0][j], q[0][j]);
            for corner in &q[1..] {
                let under = f.bin(Bo::Less, corner[j], low);
                low = f.select(under, corner[j], low);
                let over = f.bin(Bo::Greater, corner[j], high);
                high = f.select(over, corner[j], high);
            }
            (low, high)
        });
        let mut most = nothing;
        for (low, high) in bounds {
            most = f.greater_size(most, low);
            most = f.greater_size(most, high);
        }
        let grow = f.fbits(TRI_GROW);
        let grow = f.bin(Bo::Multiply, most, grow);
        let mut stretch = (f.fbits(f32::NEG_INFINITY), f.fbits(f32::INFINITY));
        for (j, (low, high)) in bounds.into_iter().enumerate() {
            let (low, high) = (f.bin(Bo::Subtract, low, grow), f.bin(Bo::Add, high, grow));
            let along = (f.pick(direction, axes[j]), f.pick(inverse, axes[j]));
            let (crossed, narrowed) = f.narrow((low, high), along, stretch);
            stretch = narrowed;
            hit = f.both(hit, crossed);
        }
        let (near, beyond) = stretch;
        let crossed = f.bin(Bo::GreaterEqual, beyond, near);
        hit = f.both(hit, crossed);
        // The distance brought into that stretch, so no test of a box that holds the triangle
        // contradicts it, whatever the two roundings.
        let t = f.bin(Bo::Divide, numerator, determinant);
        let (early, late) = (f.bin(Bo::Less, t, near), f.bin(Bo::Greater, t, beyond));
        let (first, last) = (
            f.bin(Bo::Multiply, near, determinant),
            f.bin(Bo::Multiply, beyond, determinant),
        );
        let numerator = f.select(late, last, numerator);
        let numerator = f.select(early, first, numerator);
        // The id byte names the places, in this triangle, of the two corners whose weights
        // the model asks for.
        let eight = f.u32(8);
        let shift = f.bin(Bo::Multiply, kind, eight);
        let id = f.bin(Bo::ShiftRight, ids, shift);
        let weights = [u, v, w];
        let first = f.bin(Bo::And, id, three);
        let second = f.bin(Bo::ShiftRight, id, two);
        let second = f.bin(Bo::And, second, three);
        let [first, second] = [first, second].map(|place| {
            let place = f.math(Mf::Min, &[place, two]);
            f.pick(weights, place)
        });
        let answer = [numerator, determinant, first, second].map(|part| f.bits(part));
        let missed = MISS.map(|part| f.u32(part));
        let components = (0..4)
            .map(|k| f.select(hit, answer[k], missed[k]))
            .collect();
        f.emit(E::Compose {
            ty: four,
            components,
        })
    }
}

/// A ray's origin, its direction and the direction's inverse, a float a part.
type Ray = ([Handle<E>; 3], [Handle<E>; 3], [Handle<E>; 3]);

impl Builder {
    /// A float by its bits: a writer prints a pattern exactly, and a module holds no literal
    /// that is not a finite number.
    fn fbits(&mut self, value: f32) -> Handle<E> {
        let bits = self.u32(value.to_bits());
        self.float(bits)
    }

    fn both(&mut self, a: Handle<E>, b: Handle<E>) -> Handle<E> {
        self.bin(Bo::LogicalAnd, a, b)
    }

    /// `most`, or the size of `value` where that is greater, by a compare.
    fn greater_size(&mut self, most: Handle<E>, value: Handle<E>) -> Handle<E> {
        let size = self.math(Mf::Abs, &[value]);
        let greater = self.bin(Bo::Greater, size, most);
        self.select(greater, size, most)
    }

    /// The part of `parts` the axis `axis` names.
    fn pick(&mut self, parts: [Handle<E>; 3], axis: Handle<E>) -> Handle<E> {
        let (zero, one) = (self.u32(0), self.u32(1));
        let (first, second) = (
            self.bin(Bo::Equal, axis, zero),
            self.bin(Bo::Equal, axis, one),
        );
        let rest = self.select(second, parts[1], parts[2]);
        self.select(first, parts[0], rest)
    }

    /// One axis of a widened box narrows the stretch of the ray inside it: whether the ray
    /// can be in the slab at all, and the stretch after it.
    ///
    /// A direction component below the least normal float is not divided by: `1 / 0` is
    /// infinite and its product with a plane at the origin's own coordinate is not a number.
    /// Such an axis is tested by the origin lying in the closed slab and bounds nothing.
    fn narrow(
        &mut self,
        (low, high): (Handle<E>, Handle<E>),
        (direction, inverse): (Handle<E>, Handle<E>),
        (near, beyond): (Handle<E>, Handle<E>),
    ) -> (Handle<E>, (Handle<E>, Handle<E>)) {
        let (least, size) = (self.fbits(MIN), self.math(Mf::Abs, &[direction]));
        let moving = self.bin(Bo::GreaterEqual, size, least);
        let (p, q) = (
            self.bin(Bo::Multiply, low, inverse),
            self.bin(Bo::Multiply, high, inverse),
        );
        let first = self.bin(Bo::Less, p, q);
        let (n, f) = (self.select(first, p, q), self.select(first, q, p));
        let later = self.bin(Bo::Greater, n, near);
        let later = self.both(moving, later);
        let sooner = self.bin(Bo::Less, f, beyond);
        let sooner = self.both(moving, sooner);
        let stretch = (self.select(later, n, near), self.select(sooner, f, beyond));
        let zero = self.f32(0.0);
        let (under, over) = (
            self.bin(Bo::LessEqual, low, zero),
            self.bin(Bo::GreaterEqual, high, zero),
        );
        let within = self.both(under, over);
        (self.bin(Bo::LogicalOr, moving, within), stretch)
    }
}
