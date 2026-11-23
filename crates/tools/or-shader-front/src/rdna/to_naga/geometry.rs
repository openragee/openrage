//! A geometry pair as one vertex program that pulls. The vertex half leaves its vertex in the
//! memory its wave shares and returns to the pipeline; the geometry half reads its triangle's
//! three vertices there and leaves the three it emits there too, for code the pipeline holds to
//! export. Here the two are one body run four times for each vertex drawn: the vertex half for
//! each corner of the input triangle, pulled by index from buffers, then the geometry half, and
//! the entry point exports the vertex this invocation is from where the geometry half left it.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use naga::{
    BinaryOperator as Bo, Expression as E, Handle, Scalar, Statement, TypeInner, VectorSize,
};

use super::{
    form, insert, refused, vector, Builder, Cx, Done, NotTranslated, Reg, ResourceKind, PARAM0,
    POS0,
};
use crate::dxil::container::{Container, ContainerError};
use crate::dxil::ShaderKind;
use crate::rdna::cfg::{Cfg, Terminator};
use crate::rdna::decode::{Inst, Operand};
use crate::rdna::sym::{self, Loc, Sym};
use crate::rdna::Program;

const S_ENDPGM: u32 = 0xBF81_0000;
const S_CBRANCH_SCC1: u32 = 0xBF85_0000;
/// The system value of a position in a signature.
const SV_POSITION: u32 = 1;
/// The shared memory of one wave's vertices: a vertex half writes at its lane's place past its
/// wave's number, in bits 27:24 of `s3`, times this.
const WAVE_BYTES: u32 = 2048;
/// Where the geometry half leaves what it emits: past the three waves the corners are run as.
const EMITTED_BYTES: u32 = 3 * WAVE_BYTES;
/// The pipeline's own table, in the user register a pair's programs find it in, and the word of
/// it that states where the emitted vertices start, in units of 256 bytes.
const PIPELINE_TABLE: u8 = 8;
const EMITTED_WORD: u32 = 0xB8;
/// The run of the body that is the geometry half; runs 0 to 2 are the vertex half at a corner.
const GEOMETRY_RUN: u32 = 3;
/// `s3` as every run finds it: one vertex and one primitive in the wave, in its low two bytes.
const ONE_OF_EACH: u32 = 0x0101;
/// In a vertex half the vertex's number, the instance's and the first fetched component are
/// five vector registers further than in a vertex program: the geometry half's come first.
const VERTEX_ID: u8 = 5;
const INSTANCE: u8 = 8;
const FETCH_BASE: u32 = 9;
/// The messages of a geometry program that emits one triangle: three vertices, the strip's end, done.
const ONE_TRIANGLE: [u16; 5] = [0x22, 0x22, 0x22, 0x12, 0x03];

const NO_GRAPH: &str = "a geometry pair whose other half has no graph: its words do not follow \
    from the first to an end";
const NO_RETURN: &str = "a geometry pair whose vertex half does not end by returning: its last \
    instruction is not one jump through the address the pipeline left in s6";
const OWN_WORDS: &str = "a geometry pair that reads its own words or exports by itself: the \
    halves are joined into one program, which moves the words, and the pair's one export is the \
    emitted vertex";
const NOT_ONE_TRIANGLE: &str = "a geometry program that does not emit one triangle: branches, \
    or other messages than three vertices, the strip's end and done";
const OTHER_LAYOUT: &str = "a geometry program whose emitted vertices do not fill its output \
    signature: a write outside the registers of the vertex it precedes, or no position";
const WAVES_DIFFER: &str = "a geometry pair whose halves were compiled for waves of two widths";
const TOO_LONG: &str = "a geometry pair whose vertex half is longer than a branch reaches";

/// One register of the vertices a geometry program emits, from its container's output signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Emitted {
    pub register: u32,
    pub position: bool,
}

/// The other half of a geometry pair, and what the geometry program's container states it emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub other: Program,
    pub emitted: Vec<Emitted>,
}

/// The registers of the output signature of the geometry program in `dxbc`, lowest first.
pub fn emitted(dxbc: &[u8]) -> Result<Vec<Emitted>, ContainerError> {
    let outputs = Container::read(dxbc)?.outputs()?;
    let mut registers = BTreeMap::new();
    for element in outputs {
        *registers.entry(element.register).or_default() |= element.system_value == SV_POSITION;
    }
    Ok((registers.into_iter())
        .map(|(register, position)| Emitted { register, position })
        .collect())
}

/// Where the joined program's geometry half is, and what it emits.
pub(super) struct Split {
    emitted: Vec<Emitted>,
    /// The dwords of an emitted vertex.
    stride: u32,
}

/// What the lowering of a joined pair keeps: the run and the pulled vertex the entry point sets
/// before each run of the body, the memory the halves share, and the buffers pulled from.
pub(super) struct Pairing {
    run: Handle<naga::GlobalVariable>,
    vertex: Handle<naga::GlobalVariable>,
    instance: Handle<naga::GlobalVariable>,
    shared: Handle<naga::GlobalVariable>,
    indices: Handle<naga::GlobalVariable>,
    vertices: Option<Handle<naga::GlobalVariable>>,
    /// Each emitted register with the export target it leaves through.
    targets: Vec<(u8, u32)>,
    stride: u32,
}

/// Whether a shared-memory instruction writes, the dwords it moves at each address, and the
/// bytes each address is past the instruction's base.
fn cells(name: &str, offset0: u8, offset1: u8) -> Option<(bool, u32, Vec<u32>)> {
    let (first, second) = (u32::from(offset0), u32::from(offset1));
    let one = vec![first | second << 8];
    Some(match name {
        "ds_read_b32" => (false, 1, one),
        "ds_write_b32" => (true, 1, one),
        "ds_read_b64" => (false, 2, one),
        "ds_write_b64" => (true, 2, one),
        "ds_read2_b32" => (false, 1, vec![first * 4, second * 4]),
        "ds_write2_b32" => (true, 1, vec![first * 4, second * 4]),
        "ds_read2_b64" => (false, 2, vec![first * 8, second * 8]),
        "ds_write2_b64" => (true, 2, vec![first * 8, second * 8]),
        _ => return None,
    })
}

fn graph(half: &Program) -> Result<Cfg, NotTranslated> {
    let cfg = Cfg::build(&half.words).map_err(|_| NotTranslated::Pipeline(NO_GRAPH))?;
    let own = (cfg.blocks.values().flat_map(|block| &block.insts))
        .any(|(_, inst)| matches!(inst, Inst::Exp { .. }) || inst.name() == "s_getpc_b64");
    match own {
        true => Err(NotTranslated::Pipeline(OWN_WORDS)),
        false => Ok(cfg),
    }
}

/// The vertex half's words with its return to the pipeline made the program's end.
fn returning(vertex: &Program) -> Result<Vec<u32>, NotTranslated> {
    let cfg = graph(vertex)?;
    let no_return = NotTranslated::Pipeline(NO_RETURN);
    let mut jumps = (cfg.blocks.values()).filter(|b| b.terminator == Terminator::Indirect);
    let (Some(block), None) = (jumps.next(), jumps.next()) else {
        return Err(no_return);
    };
    let Some((at, Inst::Sop1 { src, .. })) = block.insts.last() else {
        return Err(no_return);
    };
    let last = (cfg.blocks.values().flat_map(|block| &block.insts)).map(|(pc, _)| *pc);
    let mut state = (sym::solve_for(&cfg, vertex.wave32).remove(&block.start))
        .ok_or(NotTranslated::Pipeline(NO_RETURN))?;
    for (pc, inst) in &block.insts[..block.insts.len() - 1] {
        sym::step_at(&mut state, *pc, inst, vertex.wave32);
    }
    let back = sym::register(*src).and_then(|r| state.get(usize::from(r)));
    if last.max() != Some(*at) || back != Some(&Sym::User(6)) {
        return Err(no_return);
    }
    let mut words = vertex.words[..*at as usize].to_vec();
    words.push(S_ENDPGM);
    Ok(words)
}

/// The dwords of a vertex `geometry` emits, where it emits one triangle laid out as `emitted` says:
/// register `r` in the four dwords at `4 * r` of its vertex, the vertices one after another.
fn stride(geometry: &Program, emitted: &[Emitted]) -> Result<u32, NotTranslated> {
    let cfg = graph(geometry)?;
    // ponytail: every geometry program of the build is one block; one that branches around its
    // emits needs them counted along each path, and a vertex count the renderer is told.
    let (Some(block), 1) = (cfg.blocks.values().next(), cfg.blocks.len()) else {
        return Err(NotTranslated::Pipeline(NOT_ONE_TRIANGLE));
    };
    let other = NotTranslated::Pipeline(OTHER_LAYOUT);
    let Some(last) = emitted.iter().map(|e| e.register).max() else {
        return Err(other);
    };
    if !emitted.iter().any(|e| e.position) || last >= 32 {
        return Err(other);
    }
    let stride = 4 * (last + 1);
    let mut messages = Vec::new();
    for (_, inst) in &block.insts {
        match inst {
            Inst::Sopp { imm, .. } if inst.name() == "s_sendmsg" => messages.push(*imm),
            Inst::Ds {
                offset0, offset1, ..
            } => {
                let vertex = messages.len() as u32;
                let outside = |(write, dwords, at): (bool, u32, Vec<u32>)| {
                    let within = (vertex * stride * 4)..((vertex + 1) * stride * 4);
                    write
                        && (at.iter())
                            .any(|at| !within.contains(at) || at + 4 * dwords > within.end)
                };
                if cells(inst.name(), *offset0, *offset1).is_some_and(outside) {
                    return Err(other);
                }
            }
            _ => {}
        }
    }
    match messages == ONE_TRIANGLE {
        true => Ok(stride),
        false => Err(NotTranslated::Pipeline(NOT_ONE_TRIANGLE)),
    }
}

/// The one program a pair is lowered as, where `program` and `pair` are its two halves: a branch
/// to the geometry half on the scalar condition, the vertex half, the geometry half.
pub(super) fn joined(
    program: &Program,
    pair: &Pair,
) -> Option<Result<(Program, Split), NotTranslated>> {
    let (vertex, geometry) = match (program.kind, pair.other.kind) {
        (ShaderKind::Vertex, ShaderKind::Geometry) => (program, &pair.other),
        (ShaderKind::Geometry, ShaderKind::Vertex) => (&pair.other, program),
        _ => return None,
    };
    Some((|| {
        let body = returning(vertex)?;
        let stride = stride(geometry, &pair.emitted)?;
        if vertex.wave32 != geometry.wave32 {
            return Err(NotTranslated::Pipeline(WAVES_DIFFER));
        }
        let over = u16::try_from(body.len())
            .ok()
            .filter(|over| *over <= i16::MAX as u16)
            .ok_or(NotTranslated::Pipeline(TOO_LONG))?;
        let mut words = vec![S_CBRANCH_SCC1 | u32::from(over)];
        words.extend(body);
        words.extend(&geometry.words);
        let joined = Program {
            kind: ShaderKind::Vertex,
            words,
            ..vertex.clone()
        };
        let split = Split {
            emitted: pair.emitted.clone(),
            stride,
        };
        Ok((joined, split))
    })())
}

impl Cx<'_> {
    /// The globals of a joined pair, with an export target for each emitted register: the
    /// position's, and for any other the parameter of the register's own number.
    pub(super) fn pair_declare(&mut self, split: &Split) -> Pairing {
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let dwords = EMITTED_BYTES / 4 + 3 * split.stride;
        let shared = self.ty(TypeInner::Array {
            base: word,
            size: naga::ArraySize::Constant(NonZeroU32::new(dwords).unwrap_or(NonZeroU32::MIN)),
            stride: 4,
        });
        let mut targets = Vec::with_capacity(split.emitted.len());
        for emitted in &split.emitted {
            // A pixel program after a pair names each input's parameter by its register.
            let target = match emitted.position {
                true => POS0,
                false => PARAM0 + emitted.register as u8,
            };
            self.export_global(target);
            targets.push((target, emitted.register));
        }
        Pairing {
            run: self.private("pair_run".to_owned(), word),
            vertex: self.private("pair_vertex".to_owned(), word),
            instance: self.private("pair_instance".to_owned(), word),
            shared: self.private("pair_shared".to_owned(), shared),
            indices: self.pulled("pull_indices", 4),
            vertices: None,
            targets,
            stride: split.stride,
        }
    }

    /// A buffer of dwords the pair pulls from, `stride` bytes an element.
    fn pulled(&mut self, name: &str, stride: u32) -> Handle<naga::GlobalVariable> {
        let base = self.ty(TypeInner::Scalar(Scalar::U32));
        let inner = TypeInner::Array {
            base,
            size: naga::ArraySize::Dynamic,
            stride: 4,
        };
        let space = naga::AddressSpace::Storage {
            access: naga::StorageAccess::LOAD,
        };
        let kind = ResourceKind::Buffer {
            stride: Some(stride),
        };
        self.bound(name.to_owned(), inner, space, kind, None)
    }

    fn pair_global(&mut self, global: Handle<naga::GlobalVariable>) -> Handle<E> {
        let pointer = self.global(global);
        self.b.load(pointer)
    }

    /// What a register the pipeline fills holds on entry to a run of a joined pair's body;
    /// `None` for one read as any program's is.
    pub(super) fn pair_live_in(
        &mut self,
        reg: Reg,
        pc: u32,
        handed: &BTreeSet<Reg>,
    ) -> Result<Option<Handle<E>>, NotTranslated> {
        let Some(pair) = &self.pair else {
            return Ok(None);
        };
        let (run, vertex, instance) = (pair.run, pair.vertex, pair.instance);
        Ok(Some(match reg {
            // The branch the joined program opens on: taken to the geometry half.
            Reg::Scc => {
                let (run, last) = (self.pair_global(run), self.b.u32(GEOMETRY_RUN));
                let geometry = self.b.bin(Bo::Equal, run, last);
                self.b.flag(geometry)
            }
            // Each corner's vertex half runs as the first lane of its own wave.
            Reg::S(3) => {
                let (run, top, low) = (
                    self.pair_global(run),
                    self.b.u32(24),
                    self.b.u32(ONE_OF_EACH),
                );
                let wave = self.b.bin(Bo::ShiftLeft, run, top);
                self.b.bin(Bo::InclusiveOr, wave, low)
            }
            // The emitted vertices' place is the table's word alone; the return address is not followed.
            Reg::S(2 | 6 | 7) => self.b.u32(0),
            // Where the corners' vertices are, in dwords: the first and second in the halves of
            // one register, the third in the next.
            Reg::V(0) => self.b.u32((WAVE_BYTES / 4) << 16),
            Reg::V(1) => self.b.u32(2 * WAVE_BYTES / 4),
            Reg::V(VERTEX_ID) => self.pair_global(vertex),
            Reg::V(INSTANCE) => self.pair_global(instance),
            Reg::V(_) if handed.contains(&reg) => {
                return Err(NotTranslated::LiveIn {
                    pc,
                    register: reg.name(),
                })
            }
            Reg::V(_) => self.b.u32(0),
            _ => return Ok(None),
        }))
    }

    /// The word of the pipeline's table that states where the emitted vertices start.
    pub(super) fn pair_word(&mut self, at: &Loc) -> Option<Handle<E>> {
        let stated = self.pair.is_some()
            && at.root == PIPELINE_TABLE
            && at.path.is_empty()
            && at.offset == EMITTED_WORD
            && at.index.is_none();
        stated.then(|| self.b.u32(EMITTED_BYTES >> 8))
    }

    /// The vertex half's fetch call: each input's components from the pulled vertex's row of the
    /// vertex buffer, four dwords an input in the signature's order.
    pub(super) fn pair_fetch(&mut self, pc: u32, inst: &Inst) -> Done {
        let program = self.program;
        let count = program.vertex_inputs.len() as u32;
        let Some(pair) = &mut self.pair else {
            return Err(refused(pc, inst));
        };
        let vertex = pair.vertex;
        let buffer = match pair.vertices {
            Some(buffer) => buffer,
            None => {
                let buffer = self.pulled("pull_vertices", 16 * count);
                if let Some(pair) = &mut self.pair {
                    pair.vertices = Some(buffer);
                }
                buffer
            }
        };
        let (vertex, count) = (self.pair_global(vertex), self.b.u32(count));
        let row = self.b.bin(Bo::Multiply, vertex, count);
        for (ordinal, input) in program.vertex_inputs.iter().enumerate() {
            let first = FETCH_BASE + 4 * input.register;
            if first + 3 > u32::from(u8::MAX) {
                return Err(form(
                    pc,
                    inst,
                    "an input register past the vector registers",
                ));
            }
            let (ordinal, four) = (self.b.u32(ordinal as u32), self.b.u32(4));
            let element = self.b.bin(Bo::Add, row, ordinal);
            let dword = self.b.bin(Bo::Multiply, element, four);
            for c in 0..4u32 {
                // A component the element does not hold reads as the format's default: 0, 0, 0, 1.
                let part = match (input.mask >> c & 1 != 0, c, input.component_type) {
                    (true, _, _) => {
                        let (by, pointer) = (self.b.u32(c), self.global(buffer));
                        let index = self.b.bin(Bo::Add, dword, by);
                        let cell = self.b.emit(E::Access {
                            base: pointer,
                            index,
                        });
                        self.b.load(cell)
                    }
                    (false, 3, 1 | 2) => self.b.u32(1),
                    (false, 3, _) => self.b.u32(1f32.to_bits()),
                    (false, _, _) => self.b.u32(0),
                };
                self.vput((first + c) as u8, part);
            }
        }
        if let Inst::Sop1 { dst, .. } = inst {
            self.define_scalars(*dst, 2);
        }
        Ok(())
    }

    /// `s_bfm_b64`: as many set bits as the first source's low six say, moved up by the second's.
    pub(super) fn field_mask64(
        &mut self,
        pc: u32,
        dst: Operand,
        count: Operand,
        by: Operand,
    ) -> Done {
        let (count, by) = (self.sv(pc, count)?, self.sv(pc, by)?);
        let (most, word, zero) = (self.b.u32(63), self.b.u32(32), self.b.u32(0));
        let count = self.b.bin(Bo::And, count, most);
        let low = self.b.low_bits(count);
        let (wide, over) = (
            self.b.bin(Bo::GreaterEqual, count, word),
            self.b.bin(Bo::Subtract, count, word),
        );
        let over = self.b.select(wide, over, zero);
        let high = self.b.low_bits(over);
        let value = self.shifted64("s_lshl_b64", by, [low, high]);
        self.sput64(pc, dst, value)
    }

    /// The memory the halves share: dwords read and written at a byte address, one or two a
    /// place, at one place or two.
    pub(super) fn pair_memory(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let (
            Inst::Ds {
                addr,
                data0,
                data1,
                dst,
                offset0,
                offset1,
                gds,
                ..
            },
            Some(pair),
        ) = (inst, &self.pair)
        else {
            return Err(refused(pc, inst));
        };
        let shared = pair.shared;
        if *gds {
            return Err(form(pc, inst, "the memory every group shares"));
        }
        let Some((write, dwords, places)) = cells(name, *offset0, *offset1) else {
            return Err(refused(pc, inst));
        };
        let base = self.vreg(pc, *addr)?;
        let mut pointers = Vec::with_capacity(4);
        for place in places {
            for k in 0..dwords {
                let (by, two) = (self.b.u32(place + 4 * k), self.b.u32(2));
                let bytes = self.b.bin(Bo::Add, base, by);
                let index = self.b.bin(Bo::ShiftRight, bytes, two);
                let pointer = self.global(shared);
                pointers.push(self.b.emit(E::Access {
                    base: pointer,
                    index,
                }));
            }
        }
        if write {
            let mut stores = Vec::with_capacity(pointers.len());
            for (k, pointer) in pointers.into_iter().enumerate() {
                let (datum, k) = ([*data0, *data1][k / dwords as usize], k as u32 % dwords);
                stores.push((pointer, self.vreg(pc, datum.wrapping_add(k as u8))?));
            }
            self.guarded(stores);
            return Ok(());
        }
        // All are read before any register is written: the address may be one of them.
        let loaded: Vec<_> = (pointers.into_iter())
            .map(|pointer| self.b.load(pointer))
            .collect();
        for (k, value) in loaded.into_iter().enumerate() {
            self.vput(dst.wrapping_add(k as u8), value);
        }
        Ok(())
    }
}

/// How the entry point takes an input of its stage: by name, type and binding.
type Argument<'a> =
    dyn FnMut(&mut Builder, String, Handle<naga::Type>, naga::Binding) -> Handle<E> + 'a;

impl Pairing {
    /// The entry point's runs of `body`: the vertex half for each corner of the triangle this
    /// vertex is of, its vertex pulled through the index buffer, then the geometry half; and
    /// each export target filled from the emitted vertex this one is.
    pub(super) fn entry(
        &self,
        module: &mut naga::Module,
        w: &mut Builder,
        argument: &mut Argument,
        body: Handle<naga::Function>,
        exports: &BTreeMap<u8, Handle<naga::GlobalVariable>>,
    ) {
        let word = insert(module, TypeInner::Scalar(Scalar::U32));
        let vec4 = vector(module, VectorSize::Quad, Scalar::F32);
        let built_in = |built_in| naga::Binding::BuiltIn(built_in);
        let store = |w: &mut Builder, global, value| {
            let pointer = w.emit(E::GlobalVariable(global));
            w.push(Statement::Store { pointer, value });
        };
        let run = |w: &mut Builder, run: u32| {
            let number = w.u32(run);
            store(w, self.run, number);
            w.push(Statement::Call {
                function: body,
                arguments: Vec::new(),
                result: None,
            });
        };
        let drawn = argument(
            w,
            "vertex_index".to_owned(),
            word,
            built_in(naga::BuiltIn::VertexIndex),
        );
        let instance = argument(
            w,
            "instance_index".to_owned(),
            word,
            built_in(naga::BuiltIn::InstanceIndex),
        );
        store(w, self.instance, instance);
        let three = w.u32(3);
        let corner = w.bin(Bo::Modulo, drawn, three);
        let first = w.bin(Bo::Subtract, drawn, corner);
        for k in 0..3 {
            let by = w.u32(k);
            let index = w.bin(Bo::Add, first, by);
            let pointer = w.emit(E::GlobalVariable(self.indices));
            let cell = w.emit(E::Access {
                base: pointer,
                index,
            });
            let pulled = w.load(cell);
            store(w, self.vertex, pulled);
            run(w, k);
        }
        run(w, GEOMETRY_RUN);
        let (stride, start) = (w.u32(self.stride), w.u32(EMITTED_BYTES / 4));
        let past = w.bin(Bo::Multiply, corner, stride);
        let vertex = w.bin(Bo::Add, start, past);
        for &(target, register) in &self.targets {
            let Some(&export) = exports.get(&target) else {
                continue;
            };
            let components = (0..4)
                .map(|c| {
                    let by = w.u32(4 * register + c);
                    let index = w.bin(Bo::Add, vertex, by);
                    let pointer = w.emit(E::GlobalVariable(self.shared));
                    let cell = w.emit(E::Access {
                        base: pointer,
                        index,
                    });
                    let bits = w.load(cell);
                    w.float(bits)
                })
                .collect();
            let value = w.emit(E::Compose {
                ty: vec4,
                components,
            });
            store(w, export, value);
        }
    }
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
