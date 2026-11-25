//! A stage's buffers held to what a device gives a stage: the buffers the editor leaves at
//! nought first, then buffers merged into one that each is read from at its own place.

use std::collections::{BTreeMap, BTreeSet};

use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{
    AddressSpace, Arena, ArraySize, BinaryOperator as Bo, Block, Expression as E, Function,
    GlobalVariable, Handle, Literal, MathFunction, Module, Range, Scalar, ScalarKind, Span,
    Statement as S, StorageAccess, Type, TypeInner, VectorSize,
};

use super::plain::{call_in_place, each_block};

type Ex = Handle<E>;
type Global = Handle<GlobalVariable>;
/// A group and a binding in it.
type At = (u32, u32);

/// The buffers wgpu's Metal backend gives a stage, all kinds together.
pub const METAL_BUFFERS: u32 = 31;
/// The buffers wgpu's Metal backend gives the vertex stage of a pipeline, its vertex buffers
/// among them.
// ⚠ Not 31: the backend numbers a pipeline's vertex buffers down from the most vertex buffers
// wgpu takes, 16, and refuses a pipeline whose vertex stage's buffers reach them, whether or
// not it draws with any.
pub const METAL_VERTEX_BUFFERS: u32 = 16;

/// The buffers a device gives a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Constant buffers: wgpu's `max_uniform_buffers_per_shader_stage`.
    pub uniform: u32,
    /// Buffers of dwords: wgpu's `max_storage_buffers_per_shader_stage`.
    pub storage: u32,
    /// The buffers of every kind ([`Bound::together`]), where a backend numbers them from one
    /// table.
    pub together: Option<u32>,
    /// The bytes a constant buffer binds: wgpu's `max_uniform_buffer_binding_size`.
    pub uniform_bytes: u32,
}

impl Budget {
    /// What a device with wgpu's default limits gives a stage.
    pub const DEFAULT: Self = Self {
        uniform: 12,
        storage: 8,
        together: None,
        uniform_bytes: 65536,
    };

    /// What wgpu's Metal backend gives a stage: the `vertex` stage of a pipeline drawn with
    /// `vertex_buffers` vertex buffers, or another stage.
    pub fn metal(vertex: bool, vertex_buffers: u32) -> Self {
        let together = match vertex {
            true => METAL_VERTEX_BUFFERS.saturating_sub(vertex_buffers),
            false => METAL_BUFFERS,
        };
        Self {
            uniform: METAL_BUFFERS,
            storage: METAL_BUFFERS,
            together: Some(together),
            ..Self::DEFAULT
        }
    }

    /// How many buffers `bound` is past it: constant buffers, buffers of dwords, all together.
    fn past(&self, bound: Bound) -> [u32; 3] {
        [
            bound.uniform.saturating_sub(self.uniform),
            bound.storage.saturating_sub(self.storage),
            (self.together).map_or(0, |most| bound.together().saturating_sub(most)),
        ]
    }

    /// Whether a device that gives a stage this much takes a stage that binds `bound`.
    pub fn holds(&self, bound: Bound) -> bool {
        self.past(bound) == [0; 3]
    }
}

/// The buffers a stage binds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bound {
    pub uniform: u32,
    pub storage: u32,
    /// Bound arrays, of textures, samplers or buffers: a backend with one table of buffers
    /// binds each as a buffer of arguments.
    pub arrays: u32,
    /// Whether such a backend also binds a buffer of the buffers' lengths: the stage binds a
    /// buffer of dwords, or is a vertex stage.
    pub sized: bool,
}

impl Bound {
    /// What the first entry point's stage of `module` binds.
    pub fn of(module: &Module) -> Self {
        let vertex = (module.entry_points.first())
            .is_some_and(|entry| entry.stage == naga::ShaderStage::Vertex);
        let mut bound = Self {
            sized: vertex,
            ..Self::default()
        };
        for (_, global) in module.global_variables.iter() {
            if global.binding.is_none() {
                continue;
            }
            let array = matches!(
                module.types[global.ty].inner,
                TypeInner::BindingArray { .. }
            );
            bound.sized |= matches!(global.space, AddressSpace::Storage { .. });
            match global.space {
                _ if array => bound.arrays += 1,
                AddressSpace::Uniform => bound.uniform += 1,
                AddressSpace::Storage { .. } => bound.storage += 1,
                _ => {}
            }
        }
        bound
    }

    /// Every buffer a backend with one table of them binds the stage.
    pub fn together(&self) -> u32 {
        self.uniform + self.storage + self.arrays + u32::from(self.sized)
    }
}

/// One buffer a merged buffer holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// The group and binding it had.
    pub was: (u32, u32),
    /// The byte it starts at in a merged constant buffer; its place in the list a merged buffer
    /// of dwords starts with.
    pub offset: u32,
    /// Its bytes in a merged constant buffer; nought in a buffer of dwords, whose parts are as
    /// long as what fills them.
    pub bytes: u32,
}

/// Buffers of a stage merged into one, which whoever fills the stage fills from its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The group and binding of the one buffer.
    pub at: (u32, u32),
    /// The buffers it holds, in their order in it.
    pub parts: Vec<Part>,
    /// For a buffer of dwords, the bytes of one element. The buffer then starts with a list of
    /// one element a part and one more: the first dword of element `k` is the element part `k`
    /// starts at, counted from the buffer's first, and that of the element after the last
    /// part's is where the last part ends. Each part holds one element at least.
    pub listed: Option<u32>,
    /// The reads of a part at a computed place, each of which is now a call that adds where
    /// the part starts and holds the place to the part's length.
    pub calls: u32,
}

/// What was done to a stage to bring it within a budget.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fitted {
    /// The bindings left at nought that became the stage's own variables of noughts.
    pub own: Vec<(u32, u32)>,
    /// Each binding left at nought that now reads another's buffer of noughts, with that other.
    pub shared: Vec<((u32, u32), (u32, u32))>,
    pub merged: Vec<Merged>,
}

/// The most bytes of a constant buffer left at nought that a stage holds as its own variable.
// ⚠ Each thread of a stage has its own copy: with one of 64 KiB, the size a buffer read at a
// computed place is declared with, a Vulkan device was lost at the first draw.
const OWN_NOUGHTS: u32 = 4096;

fn place(global: &GlobalVariable) -> Option<(u32, u32)> {
    global.binding.as_ref().map(|b| (b.group, b.binding))
}

fn functions(module: &Module) -> impl Iterator<Item = &Function> {
    (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function))
}

fn functions_mut(module: &mut Module) -> impl Iterator<Item = &mut Function> {
    (module.functions.iter_mut().map(|(_, function)| function)).chain(
        module
            .entry_points
            .iter_mut()
            .map(|entry| &mut entry.function),
    )
}

/// Unbind `global`, which nothing reads any more: a variable of the stage's own that holds a
/// dword, since a variable of the stage's own has a stated size.
fn unbound(module: &mut Module, global: Global) {
    let dword = dword(module);
    let global = module.global_variables.get_mut(global);
    (global.ty, global.space, global.binding) = (dword, AddressSpace::Private, None);
}

/// While `module` binds more than `budget` gives, make the smallest constant buffers it binds
/// at `noughts` the stage's own variables of noughts, which bind nothing and read the same.
// Only as many as it takes: a driver may fold what a program computes from noughts it can see,
// a division by one say, otherwise than the device computes it from a buffer.
fn own_noughts(module: &mut Module, noughts: &[(u32, u32)], budget: &Budget, done: &mut Fitted) {
    let past = budget.past(Bound::of(module));
    let mut own: Vec<_> = (module.global_variables.iter())
        .filter(|(_, global)| global.space == AddressSpace::Uniform)
        .filter(|(_, global)| place(global).is_some_and(|at| noughts.contains(&at)))
        .map(|(handle, global)| (module.types[global.ty].inner.size(module.to_ctx()), handle))
        .filter(|(size, _)| *size <= OWN_NOUGHTS)
        .collect();
    own.sort();
    for (_, handle) in own.into_iter().take(past[0].max(past[2]) as usize) {
        let global = module.global_variables.get_mut(handle);
        done.own.extend(place(global));
        (global.space, global.binding) = (AddressSpace::Private, None);
    }
}

/// While `module` binds more than `budget` gives, have a buffer it binds at `noughts` read the
/// buffer of another of them of the same kind and type: both hold noughts, of one length.
fn shared_noughts(module: &mut Module, noughts: &[(u32, u32)], budget: &Budget, done: &mut Fitted) {
    let read_only = |space: AddressSpace| match space {
        AddressSpace::Uniform => Some(0),
        AddressSpace::Storage { access } if access == StorageAccess::LOAD => Some(1),
        _ => None,
    };
    let mut alike: BTreeMap<(usize, Handle<Type>), Vec<Global>> = BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        let array = matches!(
            module.types[global.ty].inner,
            TypeInner::BindingArray { .. }
        );
        let nought = place(global).is_some_and(|at| noughts.contains(&at));
        if let (Some(kind), true, false) = (read_only(global.space), nought, array) {
            alike.entry((kind, global.ty)).or_default().push(handle);
        }
    }
    for ((kind, _), globals) in alike {
        let past = budget.past(Bound::of(module));
        let Some((kept, others)) = globals.split_first() else {
            continue;
        };
        for other in others.iter().take(past[kind].max(past[2]) as usize) {
            for function in functions_mut(module) {
                for (_, expression) in function.expressions.iter_mut() {
                    if *expression == E::GlobalVariable(*other) {
                        *expression = E::GlobalVariable(*kept);
                    }
                }
            }
            let pair =
                place(&module.global_variables[*other]).zip(place(&module.global_variables[*kept]));
            done.shared.extend(pair);
            unbound(module, *other);
        }
    }
}

/// How a function reads a part of a merged buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Read {
    /// The element at a place the code computes.
    Computed,
    /// The element at a stated place.
    Stated(u32),
    /// The part's length in elements.
    Length,
}

/// One read of a part.
struct Site {
    at: Ex,
    part: usize,
    read: Read,
    /// The place, where the code computes it.
    index: Option<Ex>,
}

/// What a function does with the parts of a buffer to be merged.
#[derive(Default)]
struct Reads {
    /// The expressions that name a part.
    named: Vec<Ex>,
    sites: Vec<Site>,
    /// The loads through what a site that becomes a call gives, which is then a value.
    loads: Vec<Ex>,
}

/// What `function` does with `parts`; `None` where it loads one whole, which is more than
/// reading an element or asking its length. With `stated_stay`, a read at a stated place stays
/// a read of the buffer, as in a constant buffer; every other read becomes a call.
// A valid module hands no buffer to a function: naga takes no pointer to one as an argument.
fn reads_of(function: &Function, parts: &[Global], stated_stay: bool) -> Option<Reads> {
    let mut reads = Reads::default();
    let mut named = BTreeMap::new();
    for (at, expression) in function.expressions.iter() {
        if let E::GlobalVariable(global) = expression {
            if let Some(part) = parts.iter().position(|p| p == global) {
                named.insert(at, part);
            }
        }
    }
    // What a call gives, and what reads on from it.
    let mut given = BTreeSet::new();
    for (at, expression) in function.expressions.iter() {
        let (base, read, index) = match *expression {
            E::Access { base, index } => (base, Read::Computed, Some(index)),
            E::AccessIndex { base, index } => (base, Read::Stated(index), None),
            E::ArrayLength(array) => (array, Read::Length, None),
            E::Load { pointer } if named.contains_key(&pointer) => return None,
            E::Load { pointer } if given.contains(&pointer) => {
                reads.loads.push(at);
                continue;
            }
            _ => continue,
        };
        match named.get(&base) {
            Some(&part) => {
                let stays = stated_stay && matches!(read, Read::Stated(_));
                if !stays && read != Read::Length {
                    given.insert(at);
                }
                reads.sites.push(Site {
                    at,
                    part,
                    read,
                    index,
                });
            }
            None if given.contains(&base) && read != Read::Length => drop(given.insert(at)),
            None => {}
        }
    }
    reads.named = named.into_keys().collect();
    Some(reads)
}

/// Whether every function of `module` only reads elements of `global` or asks its length.
fn read_plainly(module: &Module, global: Global) -> bool {
    functions(module).all(|function| reads_of(function, &[global], false).is_some())
}

fn validated(module: &Module) -> Result<ModuleInfo, String> {
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map_err(|e| format!("{e:?}"))
}

fn dword(module: &mut Module) -> Handle<Type> {
    module.types.insert(
        Type {
            name: None,
            inner: TypeInner::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    )
}

/// A function that reads one part of a merged buffer, as it is written.
struct Reader {
    function: Function,
    /// The first expression a statement has to evaluate.
    first: Option<Ex>,
}

impl Reader {
    fn new(argument: Option<Handle<Type>>, result: Handle<Type>) -> Self {
        let function = Function {
            name: Some("merged_part".into()),
            arguments: (argument.into_iter())
                .map(|ty| naga::FunctionArgument {
                    name: None,
                    ty,
                    binding: None,
                })
                .collect(),
            result: Some(naga::FunctionResult {
                ty: result,
                binding: None,
            }),
            ..Default::default()
        };
        Self {
            function,
            first: None,
        }
    }

    /// An expression no statement evaluates: an argument, a binding, a number. Each comes
    /// before the first computed one.
    fn stated(&mut self, expression: E) -> Ex {
        (self.function.expressions).append(expression, Span::UNDEFINED)
    }

    fn number(&mut self, number: u32) -> Ex {
        self.stated(E::Literal(Literal::U32(number)))
    }

    /// An expression the function's one statement evaluates.
    fn computed(&mut self, expression: E) -> Ex {
        let at = (self.function.expressions).append(expression, Span::UNDEFINED);
        self.first.get_or_insert(at);
        at
    }

    fn less(&mut self, left: Ex, right: Ex) -> Ex {
        self.computed(E::Binary {
            op: Bo::Subtract,
            left,
            right,
        })
    }

    /// The function, which gives `value`, the last expression it computes.
    fn giving(mut self, value: Ex) -> Function {
        let mut body = Block::new();
        if let Some(first) = self.first {
            body.push(
                S::Emit(Range::new_from_bounds(first, value)),
                Span::UNDEFINED,
            );
        }
        body.push(S::Return { value: Some(value) }, Span::UNDEFINED);
        self.function.body = body;
        self.function
    }
}

/// The parts of one merged buffer, and where each lies in it.
struct Parts {
    /// The part that becomes the merged buffer.
    merged: Global,
    globals: Vec<Global>,
    /// A constant buffer's parts: the row each starts at and its rows. Empty for a buffer of
    /// dwords, whose parts are found by its list.
    rows: Vec<(u32, u32)>,
    /// The type of one element, and whether it is a row of four dwords.
    element: Handle<Type>,
    wide: bool,
}

impl Parts {
    /// The first dword of element `entry` of `buffer`: an entry of its list.
    fn listed(&self, f: &mut Reader, buffer: Ex, entry: u32) -> Ex {
        let mut cell = f.computed(E::AccessIndex {
            base: buffer,
            index: entry,
        });
        if self.wide {
            cell = f.computed(E::AccessIndex {
                base: cell,
                index: 0,
            });
        }
        f.computed(E::Load { pointer: cell })
    }

    /// The function that reads `part` as `read` says, a computed place given as `index`.
    fn reader(
        &self,
        module: &mut Module,
        part: usize,
        read: Read,
        index: Option<Handle<Type>>,
    ) -> Function {
        let dword = dword(module);
        let result = match read {
            Read::Length => dword,
            _ => self.element,
        };
        let signed = index.is_some_and(|ty| {
            matches!(module.types[ty].inner, TypeInner::Scalar(s) if s.kind == ScalarKind::Sint)
        });
        let mut f = Reader::new(index, result);
        let buffer = f.stated(E::GlobalVariable(self.merged));
        let one = f.number(1);
        let place = match read {
            Read::Computed => f.stated(E::FunctionArgument(0)),
            Read::Stated(place) => f.number(place),
            Read::Length => one,
        };
        let stated =
            (self.rows.get(part)).map(|&(first, rows)| (f.number(first), f.number(rows - 1)));
        let place = match signed {
            true => f.computed(E::As {
                expr: place,
                kind: ScalarKind::Uint,
                convert: Some(4),
            }),
            false => place,
        };
        let (first, last) = match stated {
            Some(stated) => stated,
            None => {
                let first = self.listed(&mut f, buffer, part as u32);
                let end = self.listed(&mut f, buffer, part as u32 + 1);
                let length = f.less(end, first);
                if read == Read::Length {
                    return f.giving(length);
                }
                (first, f.less(length, one))
            }
        };
        // A place past the part's end reads its last element, as a device holds a place past a
        // buffer's end to the buffer.
        let held = f.computed(E::Math {
            fun: MathFunction::Min,
            arg: place,
            arg1: Some(last),
            arg2: None,
            arg3: None,
        });
        let index = f.computed(E::Binary {
            op: Bo::Add,
            left: first,
            right: held,
        });
        let cell = f.computed(E::Access {
            base: buffer,
            index,
        });
        let value = f.computed(E::Load { pointer: cell });
        f.giving(value)
    }
}

/// The type of an expression of a function, placed in `module`.
fn type_of(module: &mut Module, info: &naga::valid::FunctionInfo, at: Ex) -> Handle<Type> {
    match info[at].ty.clone() {
        naga::proc::TypeResolution::Handle(handle) => handle,
        naga::proc::TypeResolution::Value(inner) => {
            (module.types).insert(Type { name: None, inner }, Span::UNDEFINED)
        }
    }
}

/// Make the buffers `parts` of `module` one buffer, bound where the first of them was: a read
/// of a part at a stated place of a constant buffer is moved to the part's place, and every
/// other read becomes a call of a function that finds the part. `Err` says why the buffers are
/// left as they are, with `module` untouched; `Ok` is how many reads became calls.
fn merge(module: &mut Module, parts: &Parts) -> Result<u32, String> {
    let mut work = module.clone();
    let info = validated(&work)?;
    let stated_stay = !parts.rows.is_empty();
    // The functions in the module's order, the entry points after them, with what each reads.
    let count = work.functions.len();
    let handles: Vec<_> = work.functions.iter().map(|(handle, _)| handle).collect();
    let mut found = Vec::new();
    for function in functions(&work) {
        let reads =
            reads_of(function, &parts.globals, stated_stay).ok_or("a part is read whole")?;
        found.push(reads);
    }
    // One function for each way a part is read, which the functions that call it follow.
    let mut readers: BTreeMap<(usize, Read, Option<Handle<Type>>), usize> = BTreeMap::new();
    let mut made: Vec<Function> = Vec::new();
    // Each call: the function it is in, the read it stands for, the function called, the place.
    let mut calls = Vec::new();
    for (at, reads) in found.iter().enumerate() {
        for site in &reads.sites {
            if stated_stay && matches!(site.read, Read::Stated(_)) {
                continue;
            }
            if stated_stay && site.read == Read::Length {
                return Err("the code asks a constant buffer its length".into());
            }
            let types = match handles.get(at) {
                Some(handle) => &info[*handle],
                None => info.get_entry_point(at - count),
            };
            let index = site.index.map(|index| type_of(&mut work, types, index));
            let key = (site.part, site.read, index);
            let reader = match readers.get(&key) {
                Some(reader) => *reader,
                None => {
                    made.push(parts.reader(&mut work, site.part, site.read, index));
                    readers.insert(key, made.len() - 1);
                    made.len() - 1
                }
            };
            calls.push((at, site.at, reader, site.index));
        }
    }
    // A function is called only by one after it: the new ones come first, and the old ones move.
    let mut functions_first = Arena::new();
    let new: Vec<Handle<Function>> = (made.into_iter())
        .map(|function| functions_first.append(function, Span::UNDEFINED))
        .collect();
    let moved: Vec<Handle<Function>> = (work.functions.drain())
        .map(|(_, function, span)| functions_first.append(function, span))
        .collect();
    work.functions = functions_first;
    let renumber = |function: &mut Function| {
        for (_, expression) in function.expressions.iter_mut() {
            if let E::CallResult(called) = expression {
                *called = moved[called.index()];
            }
        }
        each_block(&mut function.body, &mut |block| {
            for statement in block.iter_mut() {
                if let S::Call { function, .. } = statement {
                    *function = moved[function.index()];
                }
            }
        });
    };
    for handle in &moved {
        renumber(work.functions.get_mut(*handle));
    }
    (work.entry_points.iter_mut()).for_each(|entry| renumber(&mut entry.function));

    for (at, reads) in found.iter().enumerate() {
        let function = match moved.get(at) {
            Some(handle) => work.functions.get_mut(*handle),
            None => &mut work.entry_points[at - count].function,
        };
        for &named in &reads.named {
            *function.expressions.get_mut(named) = E::GlobalVariable(parts.merged);
        }
        for site in &reads.sites {
            let first = parts.rows.get(site.part).map(|(first, _)| *first);
            if let (Some(first), E::AccessIndex { index, .. }) =
                (first, function.expressions.get_mut(site.at))
            {
                *index += first;
            }
        }
        // What a call gives is a value: a load through it is the value itself.
        for &load in &reads.loads {
            if let E::Load { pointer } = function.expressions[load] {
                *function.expressions.get_mut(load) = E::Binary {
                    op: Bo::InclusiveOr,
                    left: pointer,
                    right: pointer,
                };
            }
        }
    }
    for &(at, site, reader, index) in &calls {
        let function = match moved.get(at) {
            Some(handle) => work.functions.get_mut(*handle),
            None => &mut work.entry_points[at - count].function,
        };
        *function.expressions.get_mut(site) = E::CallResult(new[reader]);
        let call = S::Call {
            function: new[reader],
            arguments: index.into_iter().collect(),
            result: Some(site),
        };
        let mut placed = false;
        let expressions = &function.expressions;
        each_block(&mut function.body, &mut |block| {
            placed = placed || call_in_place(block, expressions, site, &call);
        });
        if !placed {
            return Err("no statement of the code evaluates a read of a part".into());
        }
    }
    let total: u32 = parts.rows.iter().map(|(_, rows)| rows).sum();
    if let Some(total) = std::num::NonZeroU32::new(total) {
        let whole = work.types.insert(
            Type {
                name: None,
                inner: TypeInner::Array {
                    base: parts.element,
                    size: ArraySize::Constant(total),
                    stride: 16,
                },
            },
            Span::UNDEFINED,
        );
        work.global_variables.get_mut(parts.merged).ty = whole;
    }
    for &part in parts.globals.iter().filter(|part| **part != parts.merged) {
        unbound(&mut work, part);
    }
    validated(&work).map_err(|why| format!("the merged stage is not valid: {why}"))?;
    *module = work;
    Ok(calls.len() as u32)
}

/// The rows of `ty` where it is rows of four dwords of a stated count, as a lowered stage
/// declares a constant buffer.
fn rows_of(module: &Module, ty: Handle<Type>) -> Option<(Handle<Type>, u32)> {
    match module.types[ty].inner {
        TypeInner::Array {
            base,
            size: ArraySize::Constant(rows),
            stride: 16,
        } if is_row(module, base) => Some((base, rows.get())),
        _ => None,
    }
}

fn is_row(module: &Module, ty: Handle<Type>) -> bool {
    module.types[ty].inner
        == TypeInner::Vector {
            size: VectorSize::Quad,
            scalar: Scalar::U32,
        }
}

/// While `module` binds more than `budget` gives, merge the smallest constant buffers it binds
/// into one, as far as the bytes a constant buffer binds reach.
fn merge_constant_buffers(module: &mut Module, budget: &Budget, done: &mut Fitted) {
    let past = budget.past(Bound::of(module));
    let over = past[0].max(past[2]) as usize;
    // Each with its rows first, which it is sorted by, then where it is bound.
    let mut buffers: Vec<(u32, At, Global, Handle<Type>)> = (module.global_variables.iter())
        .filter(|(_, global)| global.space == AddressSpace::Uniform)
        .filter_map(|(handle, global)| {
            let (row, rows) = rows_of(module, global.ty)?;
            Some((rows, place(global)?, handle, row))
        })
        .filter(|(_, _, handle, _)| read_plainly(module, *handle))
        .collect();
    buffers.sort();
    let mut bytes = 0u32;
    buffers.truncate(over + 1);
    buffers.retain(|(rows, ..)| {
        bytes = bytes.saturating_add(rows * 16);
        bytes <= budget.uniform_bytes
    });
    let (Some(&(_, at, merged, element)), true) = (buffers.first(), over > 0 && buffers.len() > 1)
    else {
        return;
    };
    let mut first = 0;
    let rows: Vec<(u32, u32)> = (buffers.iter())
        .map(|(rows, ..)| {
            first += rows;
            (first - rows, *rows)
        })
        .collect();
    let parts = Parts {
        merged,
        globals: buffers.iter().map(|(_, _, handle, _)| *handle).collect(),
        rows: rows.clone(),
        element,
        wide: true,
    };
    if let Ok(calls) = merge(module, &parts) {
        done.merged.push(Merged {
            at,
            parts: (buffers.iter().zip(&rows))
                .map(|((_, was, ..), (first, rows))| Part {
                    was: *was,
                    offset: first * 16,
                    bytes: rows * 16,
                })
                .collect(),
            listed: None,
            calls,
        });
    }
}

/// While `module` binds more than `budget` gives, merge buffers of dwords it only reads, of
/// one element type, into one that starts with the list of where each lies.
fn merge_buffers(module: &mut Module, budget: &Budget, done: &mut Fitted) {
    let mut alike: BTreeMap<Handle<Type>, Vec<(At, Global)>> = BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        let read_only = matches!(global.space, AddressSpace::Storage { access } if access == StorageAccess::LOAD);
        let listed = matches!(
            module.types[global.ty].inner,
            TypeInner::Array { base, size: ArraySize::Dynamic, .. }
                if is_row(module, base) || module.types[base].inner == TypeInner::Scalar(Scalar::U32)
        );
        if let (true, true, Some(at)) = (read_only, listed, place(global)) {
            if read_plainly(module, handle) {
                alike.entry(global.ty).or_default().push((at, handle));
            }
        }
    }
    // The type most buffers have first.
    let mut alike: Vec<_> = alike.into_iter().collect();
    alike.sort_by_key(|(_, buffers)| std::cmp::Reverse(buffers.len()));
    for (ty, mut buffers) in alike {
        let past = budget.past(Bound::of(module));
        let over = past[1].max(past[2]) as usize;
        buffers.sort();
        buffers.truncate(over + 1);
        let (Some(&(at, merged)), true) = (buffers.first(), over > 0 && buffers.len() > 1) else {
            continue;
        };
        let TypeInner::Array { base, stride, .. } = module.types[ty].inner else {
            continue;
        };
        let parts = Parts {
            merged,
            globals: buffers.iter().map(|(_, handle)| *handle).collect(),
            rows: Vec::new(),
            element: base,
            wide: is_row(module, base),
        };
        if let Ok(calls) = merge(module, &parts) {
            done.merged.push(Merged {
                at,
                parts: (buffers.iter().zip(0..))
                    .map(|((was, _), offset)| Part {
                        was: *was,
                        offset,
                        bytes: 0,
                    })
                    .collect(),
                listed: Some(stride),
                calls,
            });
        }
    }
}

/// Keep `module` to the buffers `budget` gives a stage by what it binds at `noughts`, the
/// bindings the editor leaves at nought, alone: the smallest constant buffers of them become
/// the stage's own noughts, then buffers of them of one type read one buffer. Whoever fills
/// the stage fills it as before.
pub fn noughts_within(module: &mut Module, noughts: &[(u32, u32)], budget: &Budget) -> Fitted {
    let mut done = Fitted::default();
    own_noughts(module, noughts, budget, &mut done);
    shared_noughts(module, noughts, budget, &mut done);
    done
}

/// Keep `module` to the buffers `budget` gives a stage: by its `noughts` first
/// ([`noughts_within`]), and where that is not enough by merging buffers, the smallest constant
/// buffers first and then buffers of dwords, which whoever fills the stage then fills as
/// [`Fitted::merged`] says. A stage still past the budget is left so: [`Budget::holds`] says.
// What a merge costs: a read at a stated place of a constant buffer costs nothing; every other
// read is a call that adds the part's start and holds the place to the part's length (a `min`
// and an add, and for a buffer of dwords two loads of the list and two subtractions); and the
// parts are written as one buffer, so a part that changes is written at its place in it.
pub fn within(module: &mut Module, noughts: &[(u32, u32)], budget: &Budget) -> Fitted {
    let mut done = noughts_within(module, noughts, budget);
    merge_constant_buffers(module, budget, &mut done);
    merge_buffers(module, budget, &mut done);
    done
}

#[cfg(test)]
mod tests;
