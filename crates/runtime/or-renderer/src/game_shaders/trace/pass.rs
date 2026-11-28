//! The compute pass that writes the sun's mask by rays: the editor's rays made from the depth
//! target, a lane each of the game's own walk, lowered alone, and the walk's leaf read back into
//! the mask, a tile of rays at a time.

use std::borrow::Cow;
use std::num::NonZeroU64;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use glam::{Mat4, Vec3, Vec4};
use or_io::gta6::accel::layout::HEADER_ROOT;
use or_io::gta6::accel::{word, Ray, Triangle};
use wgpu::util::DeviceExt;

use super::arena::SceneArena;
use super::governor::{Governor, Rate, SAMPLES};
use super::query;

/// The most rays one dispatch of the walk is handed: no dispatch is unbounded.
pub const TILE_RAYS: u32 = 1 << 16;
/// How far a ray starts off the surface along its normal, as a share of its distance from the camera.
// The editor's own: no note establishes the game's, whose rays start where its programs say.
pub const BIAS: f32 = 1.0 / 1024.0;
/// The threads of a group of the passes around the walk.
const GROUP: u32 = 64;
/// The bytes of a tile's words.
const TILE_BYTES: u64 = 80;
/// The bit of the mask's tile word that asks for a pixel's share to be rebuilt from its neighbours.
const REBUILD: u32 = 1 << 14;
/// The pixels a side of a tile of the mask's words.
const MASK_TILE: u32 = 16;
const NO_ENTRY: &str = "the walk's module has no compute entry point";
/// Why the walk is not built on AMD's Direct3D 12 driver.
const NESTED_ON_AMD: &str = "the walk holds a loop inside a loop, and AMD's Direct3D 12 driver \
    ends the program while it builds such a stage: it is traced under Vulkan (WGPU_BACKEND=vulkan)";

/// The game's walk lowered alone, a lane an invocation, as the translation cache gave it.
#[derive(Debug, Clone)]
pub struct WalkModule {
    pub module: Arc<naga::Module>,
    /// What each dword of an invocation's row of `registers_given` holds, by the word its walk file states.
    pub given: Vec<String>,
    /// The dwords of an invocation's row of `registers_left`: the committed leaf first, nought for a miss.
    pub left: usize,
    pub threads: [u32; 3],
    pub shared_dwords: u32,
    /// The turns the walk's file states its own count ends it at.
    pub bound: u32,
    /// The loops its lowering does not show to end, which the pass runs only with every loop watched ([`watchdog`]).
    pub unbounded: u32,
}

impl WalkModule {
    /// The invocations of a group, each a lane of the walk.
    pub fn lanes(&self) -> u32 {
        self.threads.iter().product::<u32>().max(1)
    }

    /// The rows of a tile: [`TILE_RAYS`] rounded up to whole groups of the walk.
    fn rows(&self) -> u32 {
        TILE_RAYS.div_ceil(self.lanes()) * self.lanes()
    }
}

/// What the rays pass writes into a dword of an invocation's row: a value, or a part of its ray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Value(u32),
    /// The lane's first stack entry, four bytes a lane of its group.
    Lane4,
    Far,
    Origin(u32),
    Direction(u32),
}

impl Role {
    /// The role of a register a walk file states as `word`, over `arena`.
    pub fn of(word: &str, arena: &SceneArena) -> Result<Self, String> {
        let axis = |name: &str| {
            ["x", "y", "z"]
                .iter()
                .position(|a| *a == name)
                .map(|k| k as u32)
        };
        Ok(match word {
            "zero" => Role::Value(0),
            "one" => Role::Value(1),
            "ones" => Role::Value(u32::MAX),
            "root" => Role::Value(arena.root()),
            "top" => Role::Value(arena.top.base >> 8),
            "top_bytes" => Role::Value(arena.top.base),
            "lane4" => Role::Lane4,
            "far" => Role::Far,
            _ => {
                let parsed = match word.strip_prefix("0x") {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => word.parse().ok(),
                };
                match (word.split_at_checked(1), parsed) {
                    (_, Some(value)) => Role::Value(value),
                    (Some(("o", a)), _) if axis(a).is_some() => Role::Origin(axis(a).unwrap_or(0)),
                    (Some(("d", a)), _) if axis(a).is_some() => {
                        Role::Direction(axis(a).unwrap_or(0))
                    }
                    _ => {
                        return Err(format!(
                            "the walk is given {word}, which the pass does not make"
                        ))
                    }
                }
            }
        })
    }

    /// The pair the rays pass reads: a code, and the value of a [`Role::Value`].
    fn words(self) -> [u32; 2] {
        match self {
            Role::Value(value) => [0, value],
            Role::Lane4 => [1, 0],
            Role::Far => [2, 0],
            Role::Origin(axis) => [3 + axis, 0],
            Role::Direction(axis) => [6 + axis, 0],
        }
    }
}

/// The roles of the dwords of a row of `walk` over `arena`.
pub fn roles(walk: &WalkModule, arena: &SceneArena) -> Result<Vec<Role>, String> {
    walk.given
        .iter()
        .map(|word| Role::of(word, arena))
        .collect()
}

/// Where the walk's module binds what the pass fills, in its group nought.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkBindings {
    pub arena: u32,
    pub given: u32,
    pub left: u32,
    /// The device's top level, which a walk whose test of a node asks the device's query binds.
    pub structures: Option<u32>,
}

/// The bindings of `module`; `Err` where it binds anything else, which the pass would leave empty.
pub fn bindings_of(module: &naga::Module) -> Result<WalkBindings, String> {
    let (mut arena, mut given, mut left, mut structures) = (None, None, None, None);
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        let name = global.name.as_deref().unwrap_or("");
        let structure = matches!(
            module.types[global.ty].inner,
            naga::TypeInner::AccelerationStructure { .. }
        );
        let slot = match name {
            _ if structure => &mut structures,
            "arena" => &mut arena,
            "registers_given" => &mut given,
            "registers_left" => &mut left,
            _ => {
                return Err(format!(
                    "the walk binds {name}, which the pass does not fill"
                ))
            }
        };
        if binding.group != 0 {
            return Err(format!("the walk binds {name} outside its first group"));
        }
        *slot = Some(binding.binding);
    }
    match (arena, given, left) {
        (Some(arena), Some(given), Some(left)) => Ok(WalkBindings {
            arena,
            given,
            left,
            structures,
        }),
        _ => Err("the walk binds no arena or no registers".into()),
    }
}

/// Where the device's top level is bound in the walk's group: `Err` where the walk binds one and the pass has none (`given` false) to give.
pub fn structure_slot(bindings: &WalkBindings, given: bool) -> Result<Option<u32>, String> {
    match (bindings.structures, given) {
        (Some(_), false) => {
            Err("the walk asks the device's query, and no device structure is built".into())
        }
        (structures, _) => Ok(structures),
    }
}
/// The arena words a walk is given where they are not the scene's own, and its registers' roles.
pub type Given = (Option<Arc<Vec<u32>>>, Vec<Role>);

/// The words the walk's arena buffer holds where they are not `arena`'s own, and the roles of its registers: a walk through the device's query (`queried`) is given the arena as [`query::asked_words`] states it, and the root that arena's header states.
pub fn given_arena(walk: &WalkModule, arena: &SceneArena, queried: bool) -> Result<Given, String> {
    let mut roles = roles(walk, arena)?;
    if !queried {
        return Ok((None, roles));
    }
    let words = query::asked_words(&arena.arena, &arena.top)
        .ok_or("the scene's top level states no boxes for the device's query")?;
    let root = word(&words, arena.top.base, HEADER_ROOT);
    for (stated, role) in walk.given.iter().zip(&mut roles) {
        if stated == "root" {
            *role = Role::Value(root);
        }
    }
    Ok((Some(Arc::new(words)), roles))
}

/// `module` with every lane's first stack entry of its group's memory set to -1 before the walk, as the program stores it before the words the walk is cut from.
// ⚠ A device clears the memory to nought, which a walk pops as a node; every invocation stores
// the same value to each of the first `lanes` dwords, and the barrier after them holds every
// lane's own pushes until all have.
pub fn seed_stack(module: &naga::Module, lanes: u32) -> naga::Module {
    use naga::{Expression as E, Span, Statement as S};
    let mut module = module.clone();
    let shared = module.global_variables.iter().find(|(_, global)| {
        global.space == naga::AddressSpace::WorkGroup && global.name.as_deref() == Some("shared")
    });
    let Some((shared, ty)) = shared.map(|(handle, global)| (handle, global.ty)) else {
        return module;
    };
    let dwords = match &module.types[ty].inner {
        naga::TypeInner::Array {
            size: naga::ArraySize::Constant(count),
            ..
        } => Some(count.get()),
        _ => None,
    };
    let Some(dwords) = dwords else {
        return module;
    };
    let mut entries = module.entry_points.iter_mut();
    let Some(entry) = entries.find(|entry| entry.stage == naga::ShaderStage::Compute) else {
        return module;
    };
    let function = &mut entry.function;
    let base = (function.expressions).append(E::GlobalVariable(shared), Span::UNDEFINED);
    let value =
        (function.expressions).append(E::Literal(naga::Literal::U32(u32::MAX)), Span::UNDEFINED);
    let mut seeded = naga::Block::new();
    for lane in 0..lanes.min(dwords) {
        let pointer =
            (function.expressions).append(E::AccessIndex { base, index: lane }, Span::UNDEFINED);
        seeded.push(
            S::Emit(naga::Range::new_from_bounds(pointer, pointer)),
            Span::UNDEFINED,
        );
        seeded.push(S::Store { pointer, value }, Span::UNDEFINED);
    }
    seeded.push(
        S::ControlBarrier(naga::Barrier::WORK_GROUP),
        Span::UNDEFINED,
    );
    seeded.append(&mut function.body);
    function.body = seeded;
    module
}

/// Why the pass refuses a walk whose loops are not all held by the watchdog.
pub const LOOP_UNSHOWN: &str = "the walk holds a loop its lowered code does not show to end";
/// How many times what bounds a real walk's loop the watchdog lets it turn.
pub const WATCHDOG_MARGIN: u64 = 4;
/// The names [`watchdog`] gives the flag of a capped invocation and the count of them.
const CAPPED: &str = "watchdog_capped";
const CAPPED_COUNT: &str = "watchdog_count";

/// The turns a loop of a walk may take each time it is entered before the watchdog leaves it.
// A real walk's own loop ends by its count against `bound`, one turn past it at most. The node test
// of body S is arithmetic with no loop; that of body Q loops over the candidates the device hands
// over or the boxes above one, each of which takes a node, and so more than a dword, of the arena
// it is given as `query_dwords` (nought for body S). `WATCHDOG_MARGIN` times the greater.
pub fn watchdog_cap(bound: u32, query_dwords: usize) -> u32 {
    let real = (u64::from(bound) + 1).max(query_dwords as u64);
    real.saturating_mul(WATCHDOG_MARGIN)
        .min(u64::from(u32::MAX - 1)) as u32
}

/// A walk's module with every loop held by the watchdog, and what it holds.
#[derive(Debug, Clone)]
pub struct Watched {
    pub module: naga::Module,
    /// The loops given a counter, and the loops of the result that hold none ([`unwatched`]).
    pub loops: u32,
    pub unwatched: u32,
    /// The stores to `registers_left` that leave nought where the walk was capped.
    pub stores: u32,
    /// The binding in group nought of the count of capped invocations, an atomic `u32`.
    pub binding: u32,
}

/// `module` with every loop of every function given a counter of its turns since it was entered:
/// at `cap` turns, or once any loop of the invocation was capped, it leaves, the invocation's
/// first capping is counted in the atomic of group nought it binds, and each store to
/// `registers_left` after it leaves nought, a miss. A walk that ends before the cap computes what
/// it did.
pub fn watchdog(module: &naga::Module, cap: u32) -> Result<Watched, String> {
    use naga::{AddressSpace as A, Expression as E, Scalar, Span, TypeInner as T};
    let mut module = module.clone();
    let u32_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Scalar(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    let bool_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Scalar(Scalar::BOOL),
        },
        Span::UNDEFINED,
    );
    let atomic_ty = module.types.insert(
        naga::Type {
            name: None,
            inner: T::Atomic(Scalar::U32),
        },
        Span::UNDEFINED,
    );
    let no =
        (module.global_expressions).append(E::Literal(naga::Literal::Bool(false)), Span::UNDEFINED);
    let capped = module.global_variables.append(
        naga::GlobalVariable {
            name: Some(CAPPED.to_owned()),
            space: A::Private,
            binding: None,
            ty: bool_ty,
            init: Some(no),
            memory_decorations: Default::default(),
        },
        Span::UNDEFINED,
    );
    let binding = (module.global_variables.iter())
        .filter_map(|(_, global)| global.binding.as_ref())
        .filter(|binding| binding.group == 0)
        .map(|binding| binding.binding + 1)
        .max()
        .unwrap_or(0);
    let count = module.global_variables.append(
        naga::GlobalVariable {
            name: Some(CAPPED_COUNT.to_owned()),
            space: A::Storage {
                access: naga::StorageAccess::LOAD | naga::StorageAccess::STORE,
            },
            binding: Some(naga::ResourceBinding { group: 0, binding }),
            ty: atomic_ty,
            init: None,
            memory_decorations: Default::default(),
        },
        Span::UNDEFINED,
    );
    let left = (module.global_variables.iter())
        .find(|(_, global)| global.name.as_deref() == Some("registers_left"))
        .map(|(handle, global)| (handle, global.ty))
        .ok_or("the walk binds no registers_left for the watchdog to leave a miss in")?;
    match module.types[left.1].inner {
        T::Array { base, .. } if module.types[base].inner == T::Scalar(Scalar::U32) => {}
        _ => return Err("the walk's registers_left is no array of u32".into()),
    }
    let mut watch = Watch {
        u32_ty,
        cap,
        capped,
        count,
        left: left.0,
        loops: 0,
        stores: 0,
        shared: None,
    };
    for (_, function) in module.functions.iter_mut() {
        watch.function(function);
    }
    for entry in module.entry_points.iter_mut() {
        watch.function(&mut entry.function);
    }
    let unwatched = unwatched(&module);
    Ok(Watched {
        loops: watch.loops,
        unwatched,
        stores: watch.stores,
        binding,
        module,
    })
}

/// What [`watchdog`] gives each function.
struct Watch {
    u32_ty: naga::Handle<naga::Type>,
    cap: u32,
    capped: naga::Handle<naga::GlobalVariable>,
    count: naga::Handle<naga::GlobalVariable>,
    left: naga::Handle<naga::GlobalVariable>,
    loops: u32,
    stores: u32,
    /// The current function's, made with its first loop: a function with none binds no count.
    shared: Option<Shared>,
}

/// The expressions of one function the watchdog's loops share.
#[derive(Clone, Copy)]
struct Shared {
    capped: naga::Handle<naga::Expression>,
    count: naga::Handle<naga::Expression>,
    cap: naga::Handle<naga::Expression>,
    zero: naga::Handle<naga::Expression>,
    one: naga::Handle<naga::Expression>,
    yes: naga::Handle<naga::Expression>,
}

impl Watch {
    fn function(&mut self, function: &mut naga::Function) {
        self.shared = None;
        let body = std::mem::take(&mut function.body);
        function.body = self.block(function, body);
    }

    fn shared(&mut self, function: &mut naga::Function) -> Shared {
        use naga::{Expression as E, Literal as L, Span};
        let (capped, count, cap) = (self.capped, self.count, self.cap);
        *self.shared.get_or_insert_with(|| {
            let mut add = |e| function.expressions.append(e, Span::UNDEFINED);
            Shared {
                capped: add(E::GlobalVariable(capped)),
                count: add(E::GlobalVariable(count)),
                cap: add(E::Literal(L::U32(cap))),
                zero: add(E::Literal(L::U32(0))),
                one: add(E::Literal(L::U32(1))),
                yes: add(E::Literal(L::Bool(true))),
            }
        })
    }

    fn block(&mut self, function: &mut naga::Function, block: naga::Block) -> naga::Block {
        use naga::{Expression as E, Span, Statement as S};
        let mut out = naga::Block::with_capacity(block.len());
        for (statement, span) in block.span_into_iter() {
            match statement {
                S::Block(inner) => out.push(S::Block(self.block(function, inner)), span),
                S::If {
                    condition,
                    accept,
                    reject,
                } => {
                    let accept = self.block(function, accept);
                    let reject = self.block(function, reject);
                    out.push(
                        S::If {
                            condition,
                            accept,
                            reject,
                        },
                        span,
                    );
                }
                S::Switch { selector, cases } => {
                    let cases = (cases.into_iter())
                        .map(|case| naga::SwitchCase {
                            body: self.block(function, case.body),
                            ..case
                        })
                        .collect();
                    out.push(S::Switch { selector, cases }, span);
                }
                S::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    let body = self.block(function, body);
                    let continuing = self.block(function, continuing);
                    let turns = function.local_variables.append(
                        naga::LocalVariable {
                            name: Some(format!("watchdog_turns{}", self.loops)),
                            ty: self.u32_ty,
                            init: None,
                        },
                        Span::UNDEFINED,
                    );
                    self.loops += 1;
                    let shared = self.shared(function);
                    let turns =
                        (function.expressions).append(E::LocalVariable(turns), Span::UNDEFINED);
                    out.push(
                        S::Store {
                            pointer: turns,
                            value: shared.zero,
                        },
                        span,
                    );
                    let mut guarded = guard(function, &shared, turns);
                    let mut body = body;
                    guarded.append(&mut body);
                    out.push(
                        S::Loop {
                            body: guarded,
                            continuing,
                            break_if,
                        },
                        span,
                    );
                }
                S::Store { pointer, value } if root(function, pointer) == Some(self.left) => {
                    let e = &mut function.expressions;
                    let capped = e.append(E::GlobalVariable(self.capped), Span::UNDEFINED);
                    let zero = e.append(E::Literal(naga::Literal::U32(0)), Span::UNDEFINED);
                    let held = e.append(E::Load { pointer: capped }, Span::UNDEFINED);
                    let value = e.append(
                        E::Select {
                            condition: held,
                            accept: zero,
                            reject: value,
                        },
                        Span::UNDEFINED,
                    );
                    out.push(S::Emit(naga::Range::new_from_bounds(held, value)), span);
                    out.push(S::Store { pointer, value }, span);
                    self.stores += 1;
                }
                other => out.push(other, span),
            }
        }
        out
    }
}

/// The global a pointer of `function` is into, through the elements it picks.
fn root(
    function: &naga::Function,
    mut pointer: naga::Handle<naga::Expression>,
) -> Option<naga::Handle<naga::GlobalVariable>> {
    use naga::Expression as E;
    loop {
        pointer = match function.expressions[pointer] {
            E::GlobalVariable(global) => return Some(global),
            E::Access { base, .. } | E::AccessIndex { base, .. } => base,
            _ => return None,
        };
    }
}

/// The statements a watched loop's turn starts with: leave once the invocation was capped or the
/// loop has turned `cap` times, counting the invocation's first capping; else count the turn.
fn guard(
    function: &mut naga::Function,
    shared: &Shared,
    turns: naga::Handle<naga::Expression>,
) -> naga::Block {
    use naga::{BinaryOperator as B, Expression as E, Span, Statement as S};
    let e = &mut function.expressions;
    let mut add = |x| e.append(x, Span::UNDEFINED);
    let range = naga::Range::new_from_bounds;
    let now = add(E::Load { pointer: turns });
    let full = add(E::Binary {
        op: B::GreaterEqual,
        left: now,
        right: shared.cap,
    });
    let held = add(E::Load {
        pointer: shared.capped,
    });
    let leave = add(E::Binary {
        op: B::LogicalOr,
        left: held,
        right: full,
    });
    let again = add(E::Load {
        pointer: shared.capped,
    });
    let first = add(E::Unary {
        op: naga::UnaryOperator::LogicalNot,
        expr: again,
    });
    let was = add(E::Load { pointer: turns });
    let next = add(E::Binary {
        op: B::Add,
        left: was,
        right: shared.one,
    });
    let counted = naga::Block::from_vec(vec![S::Atomic {
        pointer: shared.count,
        fun: naga::AtomicFunction::Add,
        value: shared.one,
        result: None,
    }]);
    let accept = naga::Block::from_vec(vec![
        S::Emit(range(again, first)),
        S::If {
            condition: first,
            accept: counted,
            reject: naga::Block::new(),
        },
        S::Store {
            pointer: shared.capped,
            value: shared.yes,
        },
        S::Break,
    ]);
    let mut block = naga::Block::new();
    for statement in [
        S::Emit(range(now, leave)),
        S::If {
            condition: leave,
            accept,
            reject: naga::Block::new(),
        },
        S::Emit(range(was, next)),
        S::Store {
            pointer: turns,
            value: next,
        },
    ] {
        block.push(statement, Span::UNDEFINED);
    }
    block
}

/// The loops of `module` whose turn does not start with the watchdog's leave: the accounting
/// [`watchdog`] is held to, read from the module as it stands.
pub fn unwatched(module: &naga::Module) -> u32 {
    use naga::{BinaryOperator as B, Expression as E, Statement as S};
    let capped = (module.global_variables.iter())
        .find(|(_, global)| global.name.as_deref() == Some(CAPPED))
        .map(|(handle, _)| handle);
    let watched = |function: &naga::Function, body: &naga::Block| {
        let Some(S::If {
            condition, accept, ..
        }) = body.get(1)
        else {
            return false;
        };
        let E::Binary {
            op: B::LogicalOr,
            left,
            ..
        } = function.expressions[*condition]
        else {
            return false;
        };
        let E::Load { pointer } = function.expressions[left] else {
            return false;
        };
        let held =
            matches!(function.expressions[pointer], E::GlobalVariable(g) if Some(g) == capped);
        held && matches!(body.first(), Some(S::Emit(_))) && matches!(accept.last(), Some(S::Break))
    };
    fn of(
        function: &naga::Function,
        block: &naga::Block,
        watched: &dyn Fn(&naga::Function, &naga::Block) -> bool,
    ) -> u32 {
        (block.iter())
            .map(|statement| match statement {
                S::Block(block) => of(function, block, watched),
                S::If { accept, reject, .. } => {
                    of(function, accept, watched) + of(function, reject, watched)
                }
                S::Switch { cases, .. } => cases
                    .iter()
                    .map(|case| of(function, &case.body, watched))
                    .sum(),
                S::Loop {
                    body, continuing, ..
                } => {
                    u32::from(!watched(function, body))
                        + of(function, body, watched)
                        + of(function, continuing, watched)
                }
                _ => 0,
            })
            .sum()
    }
    (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function))
        .map(|function| of(function, &function.body, &watched))
        .sum()
}

/// Why a device with `limits` does not run `walk` over an arena of `arena_bytes`; `None` where it does.
pub fn refused_by(limits: &wgpu::Limits, walk: &WalkModule, arena_bytes: u64) -> Option<String> {
    let [x, y, z] = walk.threads;
    let lanes = walk.lanes();
    let rows = u64::from(walk.rows());
    let binds = limits.max_storage_buffer_binding_size;
    let checks = [
        (limits.max_compute_workgroups_per_dimension == 0, "the device runs no compute pass".to_owned()),
        (
            limits.max_compute_invocations_per_workgroup < lanes.max(GROUP)
                || limits.max_compute_workgroup_size_x < x.max(GROUP)
                || limits.max_compute_workgroup_size_y < y
                || limits.max_compute_workgroup_size_z < z,
            format!("the device runs no group of {x} by {y} by {z}, the walk's"),
        ),
        (
            limits.max_compute_workgroups_per_dimension < (rows / u64::from(lanes)).max(u64::from(TILE_RAYS / GROUP)) as u32,
            "the device runs fewer groups a dispatch than a tile of rays takes".to_owned(),
        ),
        (
            u64::from(limits.max_compute_workgroup_storage_size) < u64::from(walk.shared_dwords) * 4,
            format!("the device's group shares fewer bytes than the walk's {}", walk.shared_dwords * 4),
        ),
        (
            limits.max_storage_buffers_per_shader_stage < 3 || limits.max_storage_textures_per_shader_stage < 1,
            "the device binds fewer storage buffers or textures to a stage than the pass does".to_owned(),
        ),
        (
            binds < arena_bytes || binds < rows * walk.given.len() as u64 * 4,
            format!("the device binds a storage buffer of fewer bytes than the arena's {arena_bytes} or a tile's registers"),
        ),
    ];
    checks
        .into_iter()
        .find_map(|(refused, why)| refused.then_some(why))
}

/// What a frame is drawn by that moves the mask: when any of it changes the samples start again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub view_proj: [f32; 16],
    pub towards: [f32; 3],
    /// The tangent of half the angle the sun's disc is seen under.
    pub disc: f32,
    /// The share of the sun's light the clouds let through.
    pub clouds: f32,
}

/// What the passes read of a frame of the model's targets.
pub struct Frame<'a> {
    pub depth: &'a wgpu::TextureView,
    pub normal: &'a wgpu::TextureView,
    /// The light the resolve reads, whose layout the rays pass takes from it.
    pub light: &'a wgpu::Buffer,
    /// The mask's roots, which the pass's own target is copied to.
    pub roots: &'a wgpu::Texture,
}

/// The editor's ray of one pixel, none where its share needs none, and the share of the sun's light the pixel takes where the ray reaches the sun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cast {
    pub ray: Option<Ray>,
    pub share: f32,
}

/// What the rays pass reads of a frame besides its targets, as the CPU takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelView {
    /// From the clip space back to the world.
    pub unproject: Mat4,
    pub towards: Vec3,
    pub camera: Vec3,
    pub disc: f32,
    pub clouds: f32,
}

/// The point of the unit disc sample `k` of a pixel is jittered by: its middle for the first.
pub fn disc_point(k: u32, pixel: [u32; 2]) -> [f32; 2] {
    if k == 0 {
        return [0.0, 0.0];
    }
    let mixed = (pixel[0] as f32 * 0.067_110_56 + pixel[1] as f32 * 0.005_837_15).fract();
    let turn = std::f32::consts::TAU * (52.982_919 * mixed).fract();
    let r = ((k as f32 + 0.5) / SAMPLES as f32).sqrt();
    let a = k as f32 * 2.399_963_2 + turn;
    [r * a.cos(), r * a.sin()]
}

/// The ray the rays pass makes for `pixel` of a frame of `size` whose depth there is `depth` on a surface facing `normal`, at sample `sample`, under an arena bounded by `sphere`; the CPU's statement of the pass's own arithmetic, the ground left out.
pub fn cast_of(
    view: &PixelView,
    at: ([u32; 2], (u32, u32)),
    surface: (f32, Vec3),
    sample: u32,
    sphere: ([f32; 3], f32),
) -> Cast {
    cast_held_off(view, at, surface, sample, sphere, 0.0)
}

/// [`cast_of`] of a pass whose rays start `lift` along their way ([`TracedMask::hold_off`]).
pub fn cast_held_off(
    view: &PixelView,
    (pixel, size): ([u32; 2], (u32, u32)),
    (depth, normal): (f32, Vec3),
    sample: u32,
    sphere: ([f32; 3], f32),
    lift: f32,
) -> Cast {
    if depth <= 0.0 {
        return Cast {
            ray: None,
            share: 1.0,
        };
    }
    let at = (Vec3::new(pixel[0] as f32, pixel[1] as f32, 0.0) + 0.5)
        / Vec3::new(size.0 as f32, size.1 as f32, 1.0)
        * 2.0
        - 1.0;
    let world = view.unproject * Vec4::new(at.x, -at.y, depth, 1.0);
    let place = world.truncate() / world.w;
    let towards = view.towards.normalize();
    if normal.dot(towards) <= 0.0 {
        return Cast {
            ray: None,
            share: 0.0,
        };
    }
    let mut across = towards.cross(Vec3::Z);
    if across.length_squared() < 1e-6 {
        across = towards.cross(Vec3::X);
    }
    let across = across.normalize();
    let down = towards.cross(across);
    let [u, v] = disc_point(sample, pixel).map(|x| x * view.disc);
    let way = (towards + u * across + v * down).normalize();
    let origin = place + normal * (BIAS * place.distance(view.camera)) + way * lift;
    let far = origin.distance(Vec3::from(sphere.0)) + sphere.1;
    Cast {
        ray: Some(Ray {
            origin: origin.to_array(),
            direction: way.to_array(),
            t_min: 0.0,
            t_max: far,
        }),
        share: view.clouds,
    }
}

/// The tile words a frame's rays are cut into, shared by the two programs that read them.
const TILE_WGSL: &str = r#"
struct TraceTile {
    first: u32, count: u32, padded: u32, slots: u32,
    width: u32, height: u32, step: u32, sample: u32,
    phase_x: u32, phase_y: u32, lanes: u32, blocks: u32,
    // `lift`: how far along its way a ray starts, in the world's units.
    left: u32, bias: f32, samples: u32, lift: f32,
    // The middle of what the arena bounds, and the distance to its farthest corner.
    centre: vec4<f32>,
}
"#;

/// The rays of a tile, after the program that lights the frame, whose light, depth, normal,
/// ground and normal's decoding it takes: each a row of the walk's registers, and the share of the
/// sun's light the pixel takes if the ray reaches the sun.
const RAYS_WGSL: &str = r#"
@group(2) @binding(0) var<uniform> traced_tile: TraceTile;
@group(2) @binding(1) var<storage, read> trace_roles: array<vec2<u32>>;
@group(2) @binding(2) var<storage, read_write> trace_rows: array<u32>;
@group(2) @binding(3) var<storage, read_write> trace_shares: array<f32>;

struct Cast {
    origin: vec3<f32>,
    way: vec3<f32>,
    far: f32,
    share: f32,
}

fn trace_disc(k: u32, pixel: vec2<u32>) -> vec2<f32> {
    if k == 0u {
        return vec2<f32>(0.0);
    }
    let mixed = fract(f32(pixel.x) * 0.06711056 + f32(pixel.y) * 0.00583715);
    let turn = 6.2831853 * fract(52.982919 * mixed);
    let r = sqrt((f32(k) + 0.5) / f32(traced_tile.samples));
    let a = f32(k) * 2.3999632 + turn;
    return r * vec2<f32>(cos(a), sin(a));
}

// A ray that walks nothing: no distance, and the pixel lit.
fn no_ray(share: f32) -> Cast {
    return Cast(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 1.0), 0.0, share);
}

fn cast_at(pixel: vec2<u32>) -> Cast {
    let texel = vec2<i32>(pixel);
    let size = vec2<f32>(f32(traced_tile.width), f32(traced_tile.height));
    let at = (vec2<f32>(pixel) + 0.5) / size * 2.0 - 1.0;
    let clip = vec2<f32>(at.x, -at.y);
    let far = textureLoad(depth, texel, 0);
    var place: vec3<f32>;
    var n: vec3<f32>;
    if far <= 0.0 {
        let ground = ground_at(clip);
        if light.laid.w <= 0.0 || !ground.met {
            return no_ray(1.0);
        }
        place = ground.surface.at;
        n = ground.surface.n;
    } else {
        let world = light.unproject * vec4<f32>(clip, far, 1.0);
        place = world.xyz / world.w;
        n = normal_of(textureLoad(normal, texel, 0));
    }
    let towards = normalize(light.towards.xyz);
    if dot(n, towards) <= 0.0 {
        return no_ray(0.0);
    }
    var across = cross(towards, vec3<f32>(0.0, 0.0, 1.0));
    if dot(across, across) < 1e-6 {
        across = cross(towards, vec3<f32>(1.0, 0.0, 0.0));
    }
    across = normalize(across);
    let down = cross(towards, across);
    let d = trace_disc(traced_tile.sample, pixel) * light.shadow.y;
    let way = normalize(towards + d.x * across + d.y * down);
    let origin = place + n * (traced_tile.bias * distance(place, light.camera.xyz)) + way * traced_tile.lift;
    let reach = distance(origin, traced_tile.centre.xyz) + traced_tile.centre.w;
    return Cast(origin, way, reach, light.clouds.x);
}

@compute @workgroup_size(64)
fn cast_rays(@builtin(global_invocation_id) id: vec3<u32>) {
    let r = id.x;
    if r >= traced_tile.padded {
        return;
    }
    // `cast` is a word WGSL keeps.
    var made = no_ray(1.0);
    if r < traced_tile.count {
        let g = traced_tile.first + r;
        let block = vec2<u32>(g % traced_tile.blocks, g / traced_tile.blocks);
        let picked = block * traced_tile.step + vec2<u32>(traced_tile.phase_x, traced_tile.phase_y);
        made = cast_at(min(picked, vec2<u32>(traced_tile.width - 1u, traced_tile.height - 1u)));
    }
    trace_shares[r] = made.share;
    for (var k = 0u; k < traced_tile.slots; k++) {
        let role = trace_roles[k];
        var value = role.y;
        switch role.x {
            case 1u: { value = (r % traced_tile.lanes) * 4u; }
            case 2u: { value = bitcast<u32>(made.far); }
            case 3u: { value = bitcast<u32>(made.origin.x); }
            case 4u: { value = bitcast<u32>(made.origin.y); }
            case 5u: { value = bitcast<u32>(made.origin.z); }
            case 6u: { value = bitcast<u32>(made.way.x); }
            case 7u: { value = bitcast<u32>(made.way.y); }
            case 8u: { value = bitcast<u32>(made.way.z); }
            default: {}
        }
        trace_rows[r * traced_tile.slots + k] = value;
    }
}
"#;

/// Each ray's walk read back into the mask: nought where it committed a hit, else its share,
/// over the block of pixels it stands for, as the mean of the samples so far, its root kept.
const MASK_WGSL: &str = r#"
@group(0) @binding(0) var<uniform> traced_tile: TraceTile;
@group(0) @binding(1) var<storage, read> trace_left: array<u32>;
@group(0) @binding(2) var<storage, read> trace_shares: array<f32>;
@group(0) @binding(3) var<storage, read_write> trace_sums: array<f32>;
@group(0) @binding(4) var trace_roots: texture_storage_2d<r32float, write>;

@compute @workgroup_size(64)
fn write_mask(@builtin(global_invocation_id) id: vec3<u32>) {
    let r = id.x;
    if r >= traced_tile.count {
        return;
    }
    let hit = trace_left[r * traced_tile.left] != 0u;
    let lit = select(trace_shares[r], 0.0, hit);
    let g = traced_tile.first + r;
    let corner = vec2<u32>(g % traced_tile.blocks, g / traced_tile.blocks) * traced_tile.step;
    let n = f32(traced_tile.sample + 1u);
    for (var y = 0u; y < traced_tile.step; y++) {
        for (var x = 0u; x < traced_tile.step; x++) {
            let p = corner + vec2<u32>(x, y);
            if p.x >= traced_tile.width || p.y >= traced_tile.height {
                continue;
            }
            let i = p.y * traced_tile.width + p.x;
            var sum = lit;
            if traced_tile.sample > 0u {
                sum += trace_sums[i];
            }
            trace_sums[i] = sum;
            textureStore(trace_roots, vec2<i32>(p), vec4<f32>(sqrt(sum / n), 0.0, 0.0, 1.0));
        }
    }
}
"#;

/// The rays pass's program: the program that lights the frame, then the rays.
pub fn rays_program() -> String {
    format!(
        "{}\n{TILE_WGSL}\n{RAYS_WGSL}",
        super::super::meshlets::resolve_program()
    )
}

/// The mask pass's program.
pub fn mask_program() -> String {
    format!("{TILE_WGSL}\n{MASK_WGSL}")
}

/// What is built once for a walk and an arena, off the frame.
struct Built {
    walk: wgpu::ComputePipeline,
    walk_group: wgpu::BindGroup,
    /// The device's structures a walk through its query reads, which only its group binds.
    _structures: Option<query::Structures>,
    rays: wgpu::ComputePipeline,
    rays_frame: wgpu::BindGroupLayout,
    rays_own: wgpu::BindGroupLayout,
    mask: wgpu::ComputePipeline,
    mask_layout: wgpu::BindGroupLayout,
    arena: wgpu::Buffer,
    roles: wgpu::Buffer,
    given: wgpu::Buffer,
    left: wgpu::Buffer,
    shares: wgpu::Buffer,
    /// The count of invocations the watchdog left, an atomic of the walk's group, and its copy read back.
    capped: wgpu::Buffer,
    readback: wgpu::Buffer,
    reading: Arc<Mutex<Reading>>,
}

/// Where the count of capped invocations is on its way back from the device.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Reading {
    #[default]
    Idle,
    /// A frame recorded its copy; the next maps it, once that frame was submitted.
    Copied,
    Mapping,
    Mapped,
}

/// What a frame of one size reads and writes.
struct Sized {
    size: (u32, u32),
    tiles: wgpu::Buffer,
    stride: u64,
    /// Each pixel's sum of samples, which only the mask's group reads.
    _sums: wgpu::Buffer,
    out: wgpu::Texture,
    rays_own: wgpu::BindGroup,
    mask_group: wgpu::BindGroup,
    /// The targets' group, made once a frame hands them over.
    frame: Option<wgpu::BindGroup>,
}

/// The pass as far as a worker has built it: nothing yet, then the pass or why the device has none.
type Slot = Arc<Mutex<Option<Result<Arc<Built>, String>>>>;

/// The rays of one tile: how many, and how many rows the walk is dispatched over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    count: u32,
    padded: u32,
}

/// The sun's mask written by rays of the game's walk over one model's arena.
pub struct TracedMask {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Whether the walk's test of a node asks the device's query, over structures built of the arena.
    queried: bool,
    walk: Arc<WalkModule>,
    arena: Arc<SceneArena>,
    roles: Vec<Role>,
    generation: u64,
    built: Slot,
    sized: Option<Sized>,
    governor: Governor,
    /// What the last frame was drawn by, and how often the arena was posed since it was built.
    seen: Option<Vec<u32>>,
    posed: u32,
    /// The byte ranges of the arena a pose rewrote that the device's copy lacks.
    owed: Vec<Range<u32>>,
    /// The rate of this frame and its tiles; none before the pass is built.
    plan: Option<(Rate, Vec<Tile>)>,
    rays: u64,
    /// The invocations the watchdog left since the pass was built, as last read back.
    capped: u64,
    /// Whether the mask's tile words last written ask for the rebuild.
    rebuild: Option<bool>,
    /// How far along its way a ray starts, in the world's units.
    lift: f32,
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn tile_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: NonZeroU64::new(TILE_BYTES),
        },
        count: None,
    }
}

fn texture(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn layout(
    device: &wgpu::Device,
    label: &str,
    entries: &[wgpu::BindGroupLayoutEntry],
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries,
    })
}

fn compute(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    layout: Option<&wgpu::PipelineLayout>,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout,
        module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(16).next_multiple_of(4),
        usage,
        mapped_at_creation: false,
    })
}

/// The passes of `walk` over `arena` with `roles`, built for `device`; over the words `asked` in
/// place of the arena's own, with the device's structures of them, for a walk through its query.
fn build(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    walk: &WalkModule,
    (arena, asked): (&SceneArena, Option<&[u32]>),
    roles: &[Role],
) -> Result<Built, String> {
    let bindings = bindings_of(&walk.module)?;
    let entry_point = walk_entry(&walk.module).ok_or(NO_ENTRY)?.to_owned();
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let started = std::time::Instant::now();
    let structures = match asked {
        Some(words) => Some(
            query::Structures::build(device, queue, words, arena.top.base)
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let structure_binding = structure_slot(&bindings, structures.is_some())?;
    let seeded = seed_stack(&walk.module, walk.lanes());
    let words = asked.unwrap_or(arena.words());
    let cap = watchdog_cap(walk.bound, asked.map_or(0, <[u32]>::len));
    let watched = watchdog(&seeded, cap)?;
    if watched.unwatched > 0 {
        return Err(LOOP_UNSHOWN.to_owned());
    }
    tracing::info!(
        loops = watched.loops,
        stores = watched.stores,
        unbounded = walk.unbounded,
        cap,
        "every loop of the traced walk is watched"
    );
    let (watched_loops, count_binding) = (watched.loops, watched.binding);
    let walk_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("trace-walk"),
        source: wgpu::ShaderSource::Naga(Cow::Owned(watched.module)),
    });
    let walk_pipeline = compute(device, &walk_module, &entry_point, None);
    let storage_usage = wgpu::BufferUsages::STORAGE;
    let rows = u64::from(walk.rows());
    let arena_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("trace-arena"),
        contents: bytemuck::cast_slice(words),
        usage: storage_usage | wgpu::BufferUsages::COPY_DST,
    });
    let capped = buffer(
        device,
        "trace-capped",
        4,
        storage_usage | wgpu::BufferUsages::COPY_SRC,
    );
    let readback = buffer(
        device,
        "trace-capped-read",
        4,
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    );
    // A walk given no register still binds a buffer of a pair.
    let mut pairs: Vec<[u32; 2]> = roles.iter().map(|role| role.words()).collect();
    if pairs.is_empty() {
        pairs.push(Role::Value(0).words());
    }
    let roles_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("trace-roles"),
        contents: bytemuck::cast_slice(&pairs),
        usage: storage_usage,
    });
    let given = buffer(
        device,
        "trace-given",
        rows * walk.given.len() as u64 * 4,
        storage_usage,
    );
    let left = buffer(
        device,
        "trace-left",
        rows * walk.left as u64 * 4,
        storage_usage,
    );
    let shares = buffer(device, "trace-shares", rows * 4, storage_usage);
    fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let walk_group = {
        let mut walk_entries = vec![
            entry(bindings.arena, &arena_buffer),
            entry(bindings.given, &given),
            entry(bindings.left, &left),
        ];
        // A module with no loop leaves the count unused, and its derived layout without it.
        if watched_loops > 0 {
            walk_entries.push(entry(count_binding, &capped));
        }
        // The walk's layout is derived from its module, which declares the top level's binding.
        if let (Some(binding), Some(structures)) = (structure_binding, &structures) {
            walk_entries.push(structures.entry(binding));
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace-walk"),
            layout: &walk_pipeline.get_bind_group_layout(0),
            entries: &walk_entries,
        })
    };
    let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
    let uniform = wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let rays_frame = layout(
        device,
        "trace-frame",
        &[
            texture(1, unfiltered),
            texture(2, wgpu::TextureSampleType::Depth),
            uniform,
        ],
    );
    let rays_own = layout(
        device,
        "trace-rays",
        &[
            tile_entry(),
            storage(1, true),
            storage(2, false),
            storage(3, false),
        ],
    );
    let rays_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("trace-rays"),
        bind_group_layouts: &[Some(&rays_frame), None, Some(&rays_own)],
        immediate_size: 0,
    });
    let rays_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("trace-rays"),
        source: wgpu::ShaderSource::Wgsl(rays_program().into()),
    });
    let rays = compute(device, &rays_module, "cast_rays", Some(&rays_layout));
    let written = wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    };
    let mask_layout = layout(
        device,
        "trace-mask",
        &[
            tile_entry(),
            storage(1, true),
            storage(2, true),
            storage(3, false),
            written,
        ],
    );
    let mask_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("trace-mask"),
        bind_group_layouts: &[Some(&mask_layout)],
        immediate_size: 0,
    });
    let mask_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("trace-mask"),
        source: wgpu::ShaderSource::Wgsl(mask_program().into()),
    });
    let mask = compute(
        device,
        &mask_module,
        "write_mask",
        Some(&mask_pipeline_layout),
    );
    let refused = pollster::block_on(validation.pop()).or(pollster::block_on(internal.pop()));
    tracing::info!(elapsed = ?started.elapsed(), refused = refused.is_some(), "the traced sun's passes were built for the device");
    if let Some(error) = refused {
        return Err(error.to_string());
    }
    Ok(Built {
        walk: walk_pipeline,
        walk_group,
        _structures: structures,
        rays,
        rays_frame,
        rays_own,
        mask,
        mask_layout,
        arena: arena_buffer,
        roles: roles_buffer,
        given,
        left,
        shares,
        capped,
        readback,
        reading: Arc::default(),
    })
}

/// The passes of `walk` over `arena`, given `asked` and `roles` as [`given_arena`] makes them, started on the worker.
fn started(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    walk: &Arc<WalkModule>,
    arena: &Arc<SceneArena>,
    (asked, roles): (Option<Arc<Vec<u32>>>, Vec<Role>),
) -> Result<Slot, String> {
    let built = Slot::default();
    let theirs = Arc::clone(&built);
    let (device, queue) = (device.clone(), queue.clone());
    let (walk, arena) = (Arc::clone(walk), Arc::clone(arena));
    let work = move || {
        let words = asked.as_deref().map(Vec::as_slice);
        let made = build((&device, &queue), &walk, (&*arena, words), &roles);
        *theirs.lock().unwrap_or_else(PoisonError::into_inner) = Some(made.map(Arc::new));
    };
    super::worker().run(work).map_err(|e| e.to_string())?;
    Ok(built)
}

/// The entry point a lowered walk's module runs as.
pub fn walk_entry(module: &naga::Module) -> Option<&str> {
    (module.entry_points.iter())
        .find(|entry| entry.stage == naga::ShaderStage::Compute)
        .map(|entry| entry.name.as_str())
}

impl TracedMask {
    /// The pass of `walk` over `arena`, the `generation`th built, its pipelines started on a worker; `Err` says why `device` does not run it.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        walk: Arc<WalkModule>,
        arena: Arc<SceneArena>,
        generation: u64,
    ) -> Result<Self, String> {
        let bytes = arena.words().len() as u64 * 4;
        if let Some(why) = refused_by(&device.limits(), &walk, bytes) {
            return Err(why);
        }
        if crate::instance::refused_by_driver(&device.adapter_info(), &walk.module).is_some() {
            return Err(NESTED_ON_AMD.to_owned());
        }
        let queried = bindings_of(&walk.module)?.structures.is_some();
        if queried && !query::offered(device.adapter_info().backend, device.features()) {
            return Err(query::QueryError::NotOffered.to_string());
        }
        walk_entry(&walk.module).ok_or(NO_ENTRY)?;
        let (asked, roles) = given_arena(&walk, &arena, queried)?;
        let built = started((device, queue), &walk, &arena, (asked, roles.clone()))?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            queried,
            walk,
            arena,
            roles,
            generation,
            built,
            sized: None,
            governor: Governor::default(),
            seen: None,
            posed: 0,
            owed: Vec::new(),
            plan: None,
            rays: 0,
            capped: 0,
            rebuild: None,
            lift: 0.0,
        })
    }

    /// Start each ray `lift` along its way, as far as the shadow map holds a surface clear of what stands before it towards the light.
    pub fn hold_off(&mut self, lift: f32) {
        self.lift = lift;
    }

    /// How far along its way a ray starts.
    pub fn lift(&self) -> f32 {
        self.lift
    }

    /// The arena's number, which a caller tells a new model's from by.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn built(&self) -> Option<Arc<Built>> {
        let held = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        held.as_ref().and_then(|built| built.as_ref().ok().cloned())
    }

    /// How far the pass is: `None` while a worker builds it, then `Ok` or why the device refused it.
    pub fn state(&self) -> Option<Result<(), String>> {
        let held = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        held.as_ref()
            .map(|built| built.as_ref().map(|_| ()).map_err(Clone::clone))
    }

    /// Whether this frame's mask is the pass's: the shadow map writes none then.
    pub fn writes(&self) -> bool {
        self.plan.is_some()
    }

    /// The rays this frame casts.
    pub fn rays(&self) -> u64 {
        self.rays
    }

    /// The rays the watchdog left unfinished, a miss each, since the pass was built: as far as the device has handed the count back.
    pub fn capped(&self) -> u64 {
        self.capped
    }

    /// The count of capped rays a frame copied, asked for or read where the device has handed it over.
    fn read_capped(&mut self, built: &Built) {
        let mut reading = built.reading.lock().unwrap_or_else(PoisonError::into_inner);
        match *reading {
            Reading::Copied => {
                *reading = Reading::Mapping;
                drop(reading);
                let theirs = Arc::clone(&built.reading);
                built
                    .readback
                    .slice(..4)
                    .map_async(wgpu::MapMode::Read, move |done| {
                        let next = if done.is_ok() {
                            Reading::Mapped
                        } else {
                            Reading::Idle
                        };
                        *theirs.lock().unwrap_or_else(PoisonError::into_inner) = next;
                    });
            }
            Reading::Mapped => {
                let count = {
                    let bytes = built.readback.slice(..4).get_mapped_range();
                    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                };
                built.readback.unmap();
                *reading = Reading::Idle;
                if u64::from(count) > self.capped {
                    tracing::warn!(
                        capped = count,
                        "the watchdog left rays of the traced walk unfinished, each a miss"
                    );
                }
                self.capped = u64::from(count);
            }
            Reading::Idle | Reading::Mapping => {}
        }
    }

    /// The targets were made anew: what reads them is made again with the next frame.
    pub fn resized(&mut self) {
        self.sized = None;
        self.rebuild = None;
    }

    /// The arena rewritten for `posed`, its triangles where a pose puts them, and the mask traced anew; `Err` leaves it as it was.
    pub fn refit(&mut self, posed: &[Triangle]) -> Result<(), String> {
        let mut arena = (*self.arena).clone();
        let ranges = arena.refit(posed).map_err(|e| e.to_string())?;
        let arena = Arc::new(arena);
        if self.queried {
            // wgpu refits no device structure, and a range written again would take back the
            // root the query's copy states: the pass of a walk through the query is built anew.
            // ponytail: its pipelines are compiled again too; a top level alone where only instances move.
            let (asked, roles) = given_arena(&self.walk, &arena, true)?;
            let devices = (&self.device, &self.queue);
            self.built = started(devices, &self.walk, &arena, (asked, roles.clone()))?;
            self.roles = roles;
            self.sized = None;
            self.rebuild = None;
        } else {
            self.owed.extend(ranges);
        }
        self.arena = arena;
        self.posed = self.posed.wrapping_add(1);
        Ok(())
    }

    fn sized_for(&mut self, built: &Built, size: (u32, u32)) -> Option<&Sized> {
        if self.sized.as_ref().is_some_and(|sized| sized.size == size) {
            return self.sized.as_ref();
        }
        let device = &self.device;
        let pixels = u64::from(size.0) * u64::from(size.1);
        if device.limits().max_storage_buffer_binding_size < pixels * 4 {
            return None;
        }
        let entries = pixels.div_ceil(u64::from(TILE_RAYS)).max(1);
        let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(TILE_BYTES);
        let usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let tiles = buffer(device, "trace-tiles", entries * stride, usage);
        let sums = buffer(
            device,
            "trace-sums",
            pixels * 4,
            wgpu::BufferUsages::STORAGE,
        );
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("trace-mask"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tile = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &tiles,
            offset: 0,
            size: NonZeroU64::new(TILE_BYTES),
        });
        let rays_own = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace-rays"),
            layout: &built.rays_own,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tile.clone(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: built.roles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: built.given.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: built.shares.as_entire_binding(),
                },
            ],
        });
        let view = out.create_view(&Default::default());
        let mask_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trace-mask"),
            layout: &built.mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: tile,
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: built.left.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: built.shares.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: sums.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        self.sized = Some(Sized {
            size,
            tiles,
            stride,
            _sums: sums,
            out,
            rays_own,
            mask_group,
            frame: None,
        });
        self.sized.as_ref()
    }

    /// Take in a frame of `size` drawn by `view`: its rays written for the device, and the mask's tile words `tiles` set to ask for the rebuild while the view moves.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        view: &View,
        size: (u32, u32),
        tiles: &wgpu::Texture,
    ) {
        self.plan = None;
        self.rays = 0;
        let Some(built) = self.built() else {
            return;
        };
        self.read_capped(&built);
        for range in std::mem::take(&mut self.owed) {
            let words = &self.arena.words()[(range.start / 4) as usize..(range.end / 4) as usize];
            queue.write_buffer(
                &built.arena,
                u64::from(range.start),
                bytemuck::cast_slice(words),
            );
        }
        let (stride, tiles_buffer) = match self.sized_for(&built, size) {
            Some(sized) => (sized.stride, sized.tiles.clone()),
            None => return,
        };
        let mut key: Vec<u32> = view.view_proj.iter().map(|x| x.to_bits()).collect();
        key.extend(view.towards.map(f32::to_bits));
        key.extend([
            view.disc.to_bits(),
            view.clouds.to_bits(),
            self.lift.to_bits(),
            size.0,
            size.1,
            self.posed,
        ]);
        key.extend([self.generation as u32, (self.generation >> 32) as u32]);
        let moved = self.seen.as_ref() != Some(&key);
        self.seen = Some(key);
        let rate = self.governor.next(moved);
        self.rays = rate.rays(size);
        let rebuild = matches!(rate, Rate::Quarter { .. });
        if rate != Rate::Converged && self.rebuild != Some(rebuild) {
            let (across, down) = (
                size.0.div_ceil(MASK_TILE).max(1),
                size.1.div_ceil(MASK_TILE).max(1),
            );
            let word = if rebuild { REBUILD } else { 0 };
            let words = vec![word; (across * down) as usize];
            queue.write_texture(
                tiles.as_image_copy(),
                bytemuck::cast_slice(&words),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(across * 4),
                    rows_per_image: Some(down),
                },
                wgpu::Extent3d {
                    width: across,
                    height: down,
                    depth_or_array_layers: 1,
                },
            );
            self.rebuild = Some(rebuild);
        }
        let cut = tiles_of(self.rays, self.walk.lanes());
        let (blocks, _) = rate.blocks(size);
        let (centre, radius) = self.arena.sphere();
        let mut bytes = vec![0u8; cut.len() * stride as usize];
        for (k, tile) in cut.iter().enumerate() {
            let [phase_x, phase_y] = rate.offset();
            let words: [u32; 20] = [
                k as u32 * TILE_RAYS,
                tile.count,
                tile.padded,
                self.roles.len() as u32,
                size.0,
                size.1,
                rate.step(),
                rate.sample(),
                phase_x,
                phase_y,
                self.walk.lanes(),
                blocks,
                self.walk.left as u32,
                BIAS.to_bits(),
                SAMPLES,
                self.lift.to_bits(),
                centre[0].to_bits(),
                centre[1].to_bits(),
                centre[2].to_bits(),
                radius.to_bits(),
            ];
            let at = k * stride as usize;
            bytes[at..at + TILE_BYTES as usize].copy_from_slice(bytemuck::cast_slice(&words));
        }
        if !bytes.is_empty() {
            queue.write_buffer(&tiles_buffer, 0, &bytes);
        }
        self.plan = Some((rate, cut));
    }

    /// Record this frame's tiles into `encoder` and copy the mask they write to `frame`'s roots: whether it was written.
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder, frame: &Frame<'_>) -> bool {
        let (Some((_, cut)), Some(built)) = (self.plan.clone(), self.built()) else {
            return false;
        };
        if cut.is_empty() {
            return false;
        }
        let lanes = self.walk.lanes();
        let device = self.device.clone();
        let Some(sized) = self.sized.as_mut() else {
            return false;
        };
        let frame_group = sized.frame.get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("trace-frame"),
                layout: &built.rays_frame,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(frame.normal),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(frame.depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: frame.light.as_entire_binding(),
                    },
                ],
            })
        });
        for (k, tile) in cut.iter().enumerate() {
            let offset = (k as u64 * sized.stride) as u32;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("trace-rays"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&built.rays);
            pass.set_bind_group(0, &*frame_group, &[]);
            pass.set_bind_group(2, &sized.rays_own, &[offset]);
            pass.dispatch_workgroups(tile.padded.div_ceil(GROUP), 1, 1);
            drop(pass);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("trace-walk"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&built.walk);
            pass.set_bind_group(0, &built.walk_group, &[]);
            pass.dispatch_workgroups(tile.padded / lanes, 1, 1);
            drop(pass);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("trace-mask"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&built.mask);
            pass.set_bind_group(0, &sized.mask_group, &[offset]);
            pass.dispatch_workgroups(tile.count.div_ceil(GROUP), 1, 1);
        }
        let (width, height) = sized.size;
        encoder.copy_texture_to_texture(
            sized.out.as_image_copy(),
            frame.roots.as_image_copy(),
            wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
        );
        let mut reading = built.reading.lock().unwrap_or_else(PoisonError::into_inner);
        if *reading == Reading::Idle {
            encoder.copy_buffer_to_buffer(&built.capped, 0, &built.readback, 0, 4);
            *reading = Reading::Copied;
        }
        true
    }
}

/// `rays` cut into tiles of at most [`TILE_RAYS`], each dispatched over whole groups of `lanes`.
fn tiles_of(rays: u64, lanes: u32) -> Vec<Tile> {
    let tiles = rays.div_ceil(u64::from(TILE_RAYS));
    (0..tiles)
        .map(|k| {
            let count = (rays - k * u64::from(TILE_RAYS)).min(u64::from(TILE_RAYS)) as u32;
            Tile {
                count,
                padded: count.div_ceil(lanes) * lanes,
            }
        })
        .collect()
}

#[cfg(test)]
pub(super) fn tiles_for_test(rays: u64, lanes: u32) -> Vec<(u32, u32)> {
    tiles_of(rays, lanes)
        .iter()
        .map(|tile| (tile.count, tile.padded))
        .collect()
}
