//! A cap on the turns of every loop of a stage, so that a loop the binder judged wrongly costs a
//! device a bounded number of turns an invocation and never its display.

use naga::{
    BinaryOperator as Bo, Expression as E, Handle, Literal as L, Scalar, Span, Statement as S,
    TypeInner,
};

use super::loops::Ends;

/// How many times the turns a loop was shown to end within it may take before it is left.
pub const MARGIN: u32 = 4;
/// The fewest turns a loop is let take.
pub const LEAST_CAP: u32 = 64;
/// The turns a loop not shown to end is let take each time it is entered.
pub const UNSHOWN_CAP: u32 = 1024;
/// The turns every loop of an invocation is let take together.
// Loops inside loops multiply: this bounds what an invocation runs whatever the nesting.
pub const INVOCATION_TURNS: u32 = 1 << 14;
const _: () = assert!(UNSHOWN_CAP <= INVOCATION_TURNS);

/// The names of what [`watch`] adds: the flag of a capped invocation and its turns so far.
pub const CAPPED: &str = "loop_capped";
pub const TURNS: &str = "loop_turns";

/// The cap of a loop that ends as `ends`; `None` for one the binder refuses.
pub fn cap(ends: Ends) -> Option<u32> {
    match ends {
        Ends::Dead => Some(LEAST_CAP),
        Ends::Literal(turns) | Ends::Known(turns) => {
            Some(turns.saturating_mul(MARGIN).max(LEAST_CAP))
        }
        Ends::Unshown => Some(UNSHOWN_CAP),
        Ends::Game(_) | Ends::Runaway => None,
    }
}

/// What [`watch`] added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Watched {
    pub loops: u32,
    pub expressions: u32,
}

/// `module` with every loop held to its cap of `caps`, in the order of [`super::loops::loops`].
// A loop leaves once it has turned its cap since it was entered, once every loop of the
// invocation has turned `INVOCATION_TURNS` times, or once a loop of the invocation was so left;
// where `discard`, a fragment entry point then discards. `Err` where `caps` is not one a loop.
// A capped vertex or mesh invocation finishes with what it holds: every loop it enters after is
// left at its first turn, so it ends, and a wrong place for a vertex costs a wrong triangle.
pub fn watch(module: &mut naga::Module, caps: &[u32], discard: bool) -> Result<Watched, String> {
    let found = super::loops::loops(module);
    if found != caps.len() {
        return Err(format!(
            "the stage holds {found} loops where {} were judged",
            caps.len()
        ));
    }
    if caps.is_empty() {
        return Ok(Watched::default());
    }
    let ty = |module: &mut naga::Module, scalar| {
        module.types.insert(
            naga::Type {
                name: None,
                inner: TypeInner::Scalar(scalar),
            },
            Span::UNDEFINED,
        )
    };
    let (u32_ty, bool_ty) = (ty(module, Scalar::U32), ty(module, Scalar::BOOL));
    let global = |module: &mut naga::Module, name: &str, ty, init| {
        let init = module.global_expressions.append(init, Span::UNDEFINED);
        module.global_variables.append(
            naga::GlobalVariable {
                name: Some(name.to_owned()),
                space: naga::AddressSpace::Private,
                binding: None,
                ty,
                init: Some(init),
                memory_decorations: Default::default(),
            },
            Span::UNDEFINED,
        )
    };
    let capped = global(module, CAPPED, bool_ty, E::Literal(L::Bool(false)));
    let turns = global(module, TURNS, u32_ty, E::Literal(L::U32(0)));
    let mut watch = Watch {
        u32_ty,
        capped,
        turns,
        caps,
        next: 0,
        added: Watched::default(),
    };
    for (_, function) in module.functions.iter_mut() {
        watch.function(function, false);
    }
    for entry in module.entry_points.iter_mut() {
        let fragment = discard && entry.stage == naga::ShaderStage::Fragment;
        watch.function(&mut entry.function, fragment);
    }
    Ok(watch.added)
}

struct Watch<'a> {
    u32_ty: Handle<naga::Type>,
    capped: Handle<naga::GlobalVariable>,
    turns: Handle<naga::GlobalVariable>,
    caps: &'a [u32],
    /// The loop met next, in the order of the caps.
    next: usize,
    added: Watched,
}

/// The expressions of one function the guards share.
#[derive(Clone, Copy)]
struct Shared {
    capped: Handle<E>,
    turns: Handle<E>,
    budget: Handle<E>,
    zero: Handle<E>,
    one: Handle<E>,
    yes: Handle<E>,
}

impl Watch<'_> {
    fn function(&mut self, function: &mut naga::Function, discard: bool) {
        let before = function.expressions.len();
        let mut add = |e| function.expressions.append(e, Span::UNDEFINED);
        let shared = Shared {
            capped: add(E::GlobalVariable(self.capped)),
            turns: add(E::GlobalVariable(self.turns)),
            budget: add(E::Literal(L::U32(INVOCATION_TURNS))),
            zero: add(E::Literal(L::U32(0))),
            one: add(E::Literal(L::U32(1))),
            yes: add(E::Literal(L::Bool(true))),
        };
        let loops = self.next;
        let body = std::mem::take(&mut function.body);
        let mut body = self.block(function, &shared, body, discard);
        if discard && !matches!(body.last(), Some(S::Return { .. })) {
            for statement in killed(function, &shared) {
                body.push(statement, Span::UNDEFINED);
            }
        }
        function.body = body;
        // A function with no loop holds what was made for one unused, as a validator takes.
        if self.next > loops || discard {
            self.added.expressions += (function.expressions.len() - before) as u32;
        }
    }

    fn block(
        &mut self,
        function: &mut naga::Function,
        shared: &Shared,
        block: naga::Block,
        discard: bool,
    ) -> naga::Block {
        let mut out = naga::Block::with_capacity(block.len());
        for (statement, span) in block.span_into_iter() {
            match statement {
                S::Block(inner) => {
                    let inner = self.block(function, shared, inner, discard);
                    out.push(S::Block(inner), span);
                }
                S::If {
                    condition,
                    accept,
                    reject,
                } => {
                    let accept = self.block(function, shared, accept, discard);
                    let reject = self.block(function, shared, reject, discard);
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
                            body: self.block(function, shared, case.body, discard),
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
                    let cap = self.caps[self.next];
                    let at = self.next;
                    self.next += 1;
                    let mut body = self.block(function, shared, body, discard);
                    let continuing = self.block(function, shared, continuing, discard);
                    let local = function.local_variables.append(
                        naga::LocalVariable {
                            name: Some(format!("{TURNS}{at}")),
                            ty: self.u32_ty,
                            init: None,
                        },
                        Span::UNDEFINED,
                    );
                    let local = (function.expressions).append(E::LocalVariable(local), span);
                    out.push(
                        S::Store {
                            pointer: local,
                            value: shared.zero,
                        },
                        span,
                    );
                    let mut guarded = guard(function, shared, local, cap);
                    guarded.append(&mut body);
                    out.push(
                        S::Loop {
                            body: guarded,
                            continuing,
                            break_if,
                        },
                        span,
                    );
                    self.added.loops += 1;
                }
                S::Return { value } if discard => {
                    for statement in killed(function, shared) {
                        out.push(statement, span);
                    }
                    out.push(S::Return { value }, span);
                }
                other => out.push(other, span),
            }
        }
        out
    }
}

/// The statements that discard a capped fragment invocation.
fn killed(function: &mut naga::Function, shared: &Shared) -> [S; 2] {
    let held = (function.expressions).append(
        E::Load {
            pointer: shared.capped,
        },
        Span::UNDEFINED,
    );
    [
        S::Emit(naga::Range::new_from_bounds(held, held)),
        S::If {
            condition: held,
            accept: naga::Block::from_vec(vec![S::Kill]),
            reject: naga::Block::new(),
        },
    ]
}

/// The statements a watched loop's turn starts with: leave, marking the invocation capped, once
/// it was capped, the loop has turned `cap` times or the invocation its budget; else count.
fn guard(
    function: &mut naga::Function,
    shared: &Shared,
    local: Handle<E>,
    cap: u32,
) -> naga::Block {
    let e = &mut function.expressions;
    let mut add = |x| e.append(x, Span::UNDEFINED);
    let cap = add(E::Literal(L::U32(cap)));
    let binary = |op, left, right| E::Binary { op, left, right };
    let own = add(E::Load { pointer: local });
    let full = add(binary(Bo::GreaterEqual, own, cap));
    let held = add(E::Load {
        pointer: shared.capped,
    });
    let total = add(E::Load {
        pointer: shared.turns,
    });
    let spent = add(binary(Bo::GreaterEqual, total, shared.budget));
    let either = add(binary(Bo::LogicalOr, held, full));
    let leave = add(binary(Bo::LogicalOr, either, spent));
    let next = add(binary(Bo::Add, own, shared.one));
    let counted = add(binary(Bo::Add, total, shared.one));
    let range = naga::Range::new_from_bounds;
    naga::Block::from_vec(vec![
        S::Emit(range(own, leave)),
        S::If {
            condition: leave,
            accept: naga::Block::from_vec(vec![
                S::Store {
                    pointer: shared.capped,
                    value: shared.yes,
                },
                S::Break,
            ]),
            reject: naga::Block::new(),
        },
        S::Emit(range(next, counted)),
        S::Store {
            pointer: local,
            value: next,
        },
        S::Store {
            pointer: shared.turns,
            value: counted,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fragment stage whose loop no exit leaves, beside one a helper runs to its argument.
    const ENDLESS: &str = "
@group(0) @binding(0) var<uniform> draw: vec4<u32>;
@group(0) @binding(1) var t: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
fn run(n: u32) -> u32 {
    var i: u32 = 0u;
    loop { if i >= n { break; } i = i + 1u; }
    return i;
}
@fragment fn main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    var n: u32 = run(draw.y);
    loop { if draw.x == 7u { break; } n = n + 1u; }
    if n > 3u { return vec4<f32>(1.0); }
    return textureSample(t, s, at.xy) * f32(n);
}";

    fn parsed() -> naga::Module {
        naga::front::wgsl::parse_str(ENDLESS).expect("the WGSL parses")
    }

    /// The loops of each function whose turn starts with the guard, in order.
    fn guarded(module: &naga::Module) -> Vec<bool> {
        fn of(block: &naga::Block, out: &mut Vec<bool>) {
            for statement in block {
                match statement {
                    S::Block(b) => of(b, out),
                    S::If { accept, reject, .. } => {
                        of(accept, out);
                        of(reject, out);
                    }
                    S::Loop { body, .. } => {
                        let leaves = matches!(body.get(1), Some(S::If { accept, .. })
                            if matches!(accept.last(), Some(S::Break)));
                        out.push(matches!(body.first(), Some(S::Emit(_))) && leaves);
                        of(body, out);
                    }
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for (_, function) in module.functions.iter() {
            of(&function.body, &mut out);
        }
        for entry in &module.entry_points {
            of(&entry.function.body, &mut out);
        }
        out
    }

    #[test]
    fn every_loop_is_held_to_its_cap_and_a_capped_fragment_is_discarded() {
        let mut module = parsed();
        let watched = watch(&mut module, &[100, UNSHOWN_CAP], true).expect("watched");
        assert_eq!(watched.loops, 2);
        assert!(watched.expressions > 0);
        assert_eq!(guarded(&module), [true, true]);
        // A discard before each return of the fragment entry point.
        let returns = module.entry_points[0].function.body.iter().filter(
            |s| matches!(s, S::If { accept, .. } if matches!(accept.first(), Some(S::Kill))),
        );
        assert_eq!(returns.count(), 1);
        let caps: Vec<u32> = (module.functions.iter().map(|(_, f)| f))
            .chain(module.entry_points.iter().map(|e| &e.function))
            .flat_map(|f| {
                f.expressions
                    .iter()
                    .map(|(_, e)| e.clone())
                    .collect::<Vec<_>>()
            })
            .filter_map(|e| match e {
                E::Literal(L::U32(cap)) if cap == 100 || cap == UNSHOWN_CAP => Some(cap),
                _ => None,
            })
            .collect();
        assert_eq!(caps, [100, UNSHOWN_CAP]);
    }

    #[test]
    fn a_watched_stage_is_valid_and_every_backend_a_device_takes_writes_it() {
        use naga::valid::{Capabilities, ValidationFlags, Validator};
        let mut module = parsed();
        watch(&mut module, &[100, UNSHOWN_CAP], true).expect("watched");
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .expect("the validator takes the watched stage");
        let wgsl =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
                .expect("WGSL");
        assert!(wgsl.contains(CAPPED) && wgsl.contains("discard"), "{wgsl}");
        naga::back::spv::write_vec(&module, &info, &Default::default(), None).expect("SPIR-V");
        let mut hlsl = String::new();
        let options = naga::back::hlsl::Options::default();
        naga::back::hlsl::Writer::new(&mut hlsl, &options, &Default::default())
            .write(&module, &info, None)
            .expect("HLSL");
        let mut msl = String::new();
        let options = naga::back::msl::Options::default();
        let pipeline = naga::back::msl::PipelineOptions::default();
        naga::back::msl::Writer::new(&mut msl)
            .write(&module, &info, &options, &pipeline)
            .expect("MSL");
    }

    #[test]
    fn a_stage_whose_loops_are_not_those_judged_is_refused() {
        let mut module = parsed();
        assert!(watch(&mut module, &[100], true).is_err());
        let mut none = naga::Module::default();
        assert_eq!(watch(&mut none, &[], true), Ok(Watched::default()));
    }

    /// The caps a module's guards hold, in order.
    fn caps(module: &naga::Module) -> Vec<u32> {
        let mut out = Vec::new();
        for function in (module.functions.iter().map(|(_, f)| f))
            .chain(module.entry_points.iter().map(|e| &e.function))
        {
            for (_, e) in function.expressions.iter() {
                if let E::Binary {
                    op: Bo::GreaterEqual,
                    right,
                    ..
                } = e
                {
                    if let E::Literal(L::U32(cap)) = function.expressions[*right] {
                        if cap != INVOCATION_TURNS {
                            out.push(cap);
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_fed_stage_is_judged_before_its_buffers_are_rewritten_and_watched_as_drawn() {
        use crate::rdna::ResourceKind;
        use crate::rdna_bind::budget::Budget;
        use crate::rdna_bind::shading::feed_within;
        use crate::rdna_bind::trees::{tests::stage_reading, DRAW_STATE};
        use or_io::gta5::shaders::sgd5::Sgd5Table;
        let counting = "
@group(0) @binding(0) var<uniform> state: array<vec4<u32>, 2>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    var i: u32 = 0u;
    loop { if i >= state[1].y { break; } i = i + 1u; }
    return vec4<f32>(f32(i));
}";
        let rows = ResourceKind::ConstantBuffer { rows: 2 };
        let (native, lowered, table) = stage_reading(DRAW_STATE, 68, rows, counting);
        // No constant buffer: the buffer at rest becomes the stage's own noughts.
        let budget = Budget {
            uniform: 0,
            ..Budget::DEFAULT
        };
        let (stage, fitted) =
            feed_within(&native, lowered, &table, &Sgd5Table::default(), &budget).expect("fed");
        assert_eq!(fitted.own.len(), 1, "{fitted:?}");
        let module = &stage.module;
        assert!(module
            .global_variables
            .iter()
            .all(|(_, g)| g.binding.is_none()));
        // Judged by the noughts it was bound as (one test, then out), not as an unknown own
        // variable, whose cap would be that of a loop not shown to end.
        assert_eq!(caps(module), [LEAST_CAP]);
        assert_eq!(guarded(module), [true]);
    }

    #[test]
    fn a_cap_is_its_turns_with_a_margin_and_none_for_a_refused_loop() {
        assert_eq!(cap(Ends::Literal(5)), Some(LEAST_CAP));
        assert_eq!(cap(Ends::Known(100)), Some(400));
        assert_eq!(cap(Ends::Dead), Some(LEAST_CAP));
        assert_eq!(cap(Ends::Unshown), Some(UNSHOWN_CAP));
        assert_eq!(cap(Ends::Game(7)), None);
        assert_eq!(cap(Ends::Runaway), None);
    }
}
