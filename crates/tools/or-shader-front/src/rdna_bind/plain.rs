//! A stage's array of textures lowered to plain textures: a program picks an entry of its heap
//! by a number it reads when it runs, and the editor knows the few entries a material fills, so
//! each sample of the array becomes a call that tells them apart by that number. A device then
//! needs no arrays of textures to draw the stage.

use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{
    Arena, Block, Expression as E, Function, Handle, Module, Range, SampleLevel, Span,
    Statement as S, Type, TypeInner,
};

type Ex = Handle<E>;

/// Where a function of a module lies.
#[derive(Clone, Copy)]
enum Place {
    Function(usize),
    Entry(usize),
}

/// One sample or load of an entry of the array.
struct Site {
    place: Place,
    /// The sample or the load.
    at: Ex,
    /// The expression that takes the entry, and the number it takes it by.
    access: Ex,
    index: Ex,
    signed: bool,
    sampler: Option<Handle<naga::GlobalVariable>>,
    /// What it reads other than the image and the sampler, in the order `rebuilt` visits them.
    operands: Vec<Ex>,
    operand_types: Vec<Handle<Type>>,
    index_type: Handle<Type>,
    result: Handle<Type>,
}

/// `expression`, a sample or a load, of `image` through `sampler`, each other operand passed
/// through `operand` in a fixed order; `None` for another expression, or a sample with an offset.
fn rebuilt(
    expression: &E,
    image: Ex,
    sampler: Option<Ex>,
    operand: &mut dyn FnMut(Ex) -> Ex,
) -> Option<E> {
    Some(match *expression {
        E::ImageSample {
            gather,
            coordinate,
            array_index,
            offset: None,
            level,
            depth_ref,
            clamp_to_edge,
            ..
        } => E::ImageSample {
            image,
            sampler: sampler?,
            gather,
            coordinate: operand(coordinate),
            array_index: array_index.map(&mut *operand),
            offset: None,
            level: match level {
                SampleLevel::Auto => SampleLevel::Auto,
                SampleLevel::Zero => SampleLevel::Zero,
                SampleLevel::Exact(h) => SampleLevel::Exact(operand(h)),
                SampleLevel::Bias(h) => SampleLevel::Bias(operand(h)),
                SampleLevel::Gradient { x, y } => SampleLevel::Gradient {
                    x: operand(x),
                    y: operand(y),
                },
            },
            depth_ref: depth_ref.map(&mut *operand),
            clamp_to_edge,
        },
        E::ImageLoad {
            coordinate,
            array_index,
            sample,
            level,
            ..
        } => E::ImageLoad {
            image,
            coordinate: operand(coordinate),
            array_index: array_index.map(&mut *operand),
            sample: sample.map(&mut *operand),
            level: level.map(&mut *operand),
        },
        _ => return None,
    })
}

fn function_of(module: &Module, place: Place) -> Option<&Function> {
    match place {
        Place::Function(at) => (module.functions.iter().nth(at)).map(|(_, f)| f),
        Place::Entry(at) => module.entry_points.get(at).map(|e| &e.function),
    }
}

/// Every sample and load of an entry of `global` in `module`; `Err` where the code uses the
/// array in another way.
fn sites(
    module: &mut Module,
    info: &ModuleInfo,
    global: Handle<naga::GlobalVariable>,
) -> Result<Vec<Site>, String> {
    let places: Vec<Place> = (0..module.functions.len())
        .map(Place::Function)
        .chain((0..module.entry_points.len()).map(Place::Entry))
        .collect();
    let mut out = Vec::new();
    for place in places {
        let Some(function) = function_of(module, place) else {
            continue;
        };
        let types = match place {
            Place::Function(at) => {
                let (handle, _) = module.functions.iter().nth(at).ok_or("no function")?;
                &info[handle]
            }
            Place::Entry(at) => info.get_entry_point(at),
        };
        let is_array = |h: Ex| function.expressions[h] == E::GlobalVariable(global);
        let entry = |h: Ex| match function.expressions[h] {
            E::Access { base, index } if is_array(base) => Some(index),
            _ => None,
        };
        // What is found here, with the types still to be placed in the module.
        let mut found = Vec::new();
        for (at, expression) in function.expressions.iter() {
            let image = match *expression {
                E::ImageSample { image, .. } | E::ImageLoad { image, .. } => Some(image),
                E::ImageQuery { image, .. } if entry(image).is_some() || is_array(image) => {
                    return Err("the code asks an entry of the array its size".into());
                }
                E::AccessIndex { base, .. } if is_array(base) => {
                    return Err("the code takes an entry of the array at a stated place".into());
                }
                _ => None,
            };
            let Some((access, index)) = image.and_then(|image| Some((image, entry(image)?))) else {
                continue;
            };
            let sampler = match *expression {
                E::ImageSample { sampler, .. } => match function.expressions[sampler] {
                    E::GlobalVariable(sampler) => Some(sampler),
                    _ => return Err("a sample's sampler is not a binding".into()),
                },
                _ => None,
            };
            let mut operands = Vec::new();
            let mut keep = |h: Ex| {
                operands.push(h);
                h
            };
            if rebuilt(expression, access, Some(access), &mut keep).is_none() {
                return Err("a sample of the array has an offset".into());
            }
            let signed = match types[index].ty.inner_with(&module.types) {
                TypeInner::Scalar(scalar) => scalar.kind == naga::ScalarKind::Sint,
                _ => return Err("an entry of the array is not taken by a number".into()),
            };
            let resolved = |h: Ex| types[h].ty.clone();
            found.push((
                at,
                access,
                index,
                signed,
                sampler,
                operands.iter().map(|h| resolved(*h)).collect::<Vec<_>>(),
                resolved(index),
                resolved(at),
                operands,
            ));
        }
        for (at, access, index, signed, sampler, operand_types, index_type, result, operands) in
            found
        {
            let mut placed = |resolution: naga::proc::TypeResolution| match resolution {
                naga::proc::TypeResolution::Handle(handle) => handle,
                naga::proc::TypeResolution::Value(inner) => {
                    (module.types).insert(Type { name: None, inner }, Span::UNDEFINED)
                }
            };
            out.push(Site {
                place,
                at,
                access,
                index,
                signed,
                sampler,
                operand_types: operand_types.into_iter().map(&mut placed).collect(),
                index_type: placed(index_type),
                result: placed(result),
                operands,
            });
        }
    }
    Ok(out)
}

/// The function that does what `site` did for whichever of `textures` its first argument
/// names: each texture with the number of its entry, the first of them for any other number.
fn pick(site: &Site, expression: &E, textures: &[(u32, Handle<naga::GlobalVariable>)]) -> Function {
    let argument = |ty| naga::FunctionArgument {
        name: None,
        ty,
        binding: None,
    };
    let mut function = Function {
        name: Some("pick_texture".into()),
        arguments: std::iter::once(site.index_type)
            .chain(site.operand_types.iter().copied())
            .map(argument)
            .collect(),
        result: Some(naga::FunctionResult {
            ty: site.result,
            binding: None,
        }),
        ..Default::default()
    };
    let expressions = &mut function.expressions;
    let number = expressions.append(E::FunctionArgument(0), Span::UNDEFINED);
    let passed: Vec<Ex> = (1..=site.operands.len() as u32)
        .map(|i| expressions.append(E::FunctionArgument(i), Span::UNDEFINED))
        .collect();
    let sampler = (site.sampler).map(|s| expressions.append(E::GlobalVariable(s), Span::UNDEFINED));
    let of = |expressions: &mut Arena<E>, texture| {
        let image = expressions.append(E::GlobalVariable(texture), Span::UNDEFINED);
        let mut next = passed.iter().copied();
        let mut operand = |_| next.next().unwrap_or(number);
        // `sites` took only what `rebuilt` rebuilds.
        let value =
            rebuilt(expression, image, sampler, &mut operand).unwrap_or(E::ZeroValue(site.result));
        let value = expressions.append(value, Span::UNDEFINED);
        let mut block = Block::new();
        block.push(
            S::Emit(Range::new_from_bounds(value, value)),
            Span::UNDEFINED,
        );
        block.push(S::Return { value: Some(value) }, Span::UNDEFINED);
        block
    };
    let mut body = Block::new();
    for (entry, texture) in textures.iter().skip(1) {
        let literal = match site.signed {
            true => naga::Literal::I32(*entry as i32),
            false => naga::Literal::U32(*entry),
        };
        let entry = expressions.append(E::Literal(literal), Span::UNDEFINED);
        let condition = expressions.append(
            E::Binary {
                op: naga::BinaryOperator::Equal,
                left: number,
                right: entry,
            },
            Span::UNDEFINED,
        );
        let accept = of(expressions, *texture);
        body.push(
            S::Emit(Range::new_from_bounds(condition, condition)),
            Span::UNDEFINED,
        );
        body.push(
            S::If {
                condition,
                accept,
                reject: Block::new(),
            },
            Span::UNDEFINED,
        );
    }
    if let Some((_, texture)) = textures.first() {
        body.extend_block(of(expressions, *texture));
    }
    function.body = body;
    function
}

/// Each block of `block`, itself first.
pub(super) fn each_block(block: &mut Block, f: &mut dyn FnMut(&mut Block)) {
    f(block);
    for statement in block.iter_mut() {
        match statement {
            S::Block(inner) => each_block(inner, f),
            S::If { accept, reject, .. } => {
                each_block(accept, f);
                each_block(reject, f);
            }
            S::Switch { cases, .. } => {
                for case in cases {
                    each_block(&mut case.body, f);
                }
            }
            S::Loop {
                body, continuing, ..
            } => {
                each_block(body, f);
                each_block(continuing, f);
            }
            _ => {}
        }
    }
}

/// Put `call` where `block` emits `at`, which the call gives from now on; whether it was there.
pub(super) fn call_in_place(block: &mut Block, expressions: &Arena<E>, at: Ex, call: &S) -> bool {
    let holds = |statement: &S| match statement {
        S::Emit(range) => range.index_range().contains(&(at.index() as u32)),
        _ => false,
    };
    let Some(place) = block.iter().position(holds) else {
        return false;
    };
    let S::Emit(range) = &block[place] else {
        return false;
    };
    let (whole, here) = (range.index_range(), at.index() as u32);
    let part = |range: std::ops::Range<u32>| {
        (!range.is_empty()).then(|| S::Emit(Range::from_index_range(range, expressions)))
    };
    let parts = [
        part(whole.start..here),
        Some(call.clone()),
        part(here + 1..whole.end),
    ];
    block.splice(
        place..=place,
        Block::from_vec(parts.into_iter().flatten().collect()),
    );
    true
}

/// Make the array of textures `module` binds at `at` (group, binding) one plain texture for
/// each number of `entries`, bound in `group` from binding `first` on in that order: a sample or
/// a load of the array's entry `n` reads the texture of the number `n`, and that of the first
/// number for any other. `Err` says why the array is left as it is, with `module` untouched.
pub fn plain_textures(
    module: &mut Module,
    at: (u32, u32),
    entries: &[u32],
    group: u32,
    first: u32,
) -> Result<(), String> {
    let validate = |module: &Module| {
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(module)
            .map_err(|e| format!("{e:?}"))
    };
    if entries.is_empty() {
        return Err("no entry of the array is known".into());
    }
    let mut work = module.clone();
    let found = work
        .global_variables
        .iter()
        .find(|(_, global)| (global.binding.as_ref()).is_some_and(|b| (b.group, b.binding) == at));
    let (global, _) = found.ok_or("no binding there")?;
    let TypeInner::BindingArray { base, .. } = work.types[work.global_variables[global].ty].inner
    else {
        return Err("the binding is no array".into());
    };
    let info = validate(&work)?;
    let sites = sites(&mut work, &info, global)?;

    // The array itself becomes the first texture; the others are bound after it.
    let array = work.global_variables[global].clone();
    let mut textures = Vec::with_capacity(entries.len());
    for (binding, entry) in (first..).zip(entries) {
        let bound = Some(naga::ResourceBinding { group, binding });
        let texture = match textures.is_empty() {
            true => {
                let first = work.global_variables.get_mut(global);
                (first.ty, first.binding) = (base, bound);
                global
            }
            false => work.global_variables.append(
                naga::GlobalVariable {
                    name: None,
                    binding: bound,
                    ty: base,
                    ..array.clone()
                },
                Span::UNDEFINED,
            ),
        };
        textures.push((*entry, texture));
    }

    // A function is called only by one after it: the new ones come first, and the old ones move.
    let mut functions = Arena::new();
    let picks: Vec<Handle<Function>> = (sites.iter())
        .map(|_| functions.append(Function::default(), Span::UNDEFINED))
        .collect();
    let moved: Vec<Handle<Function>> = (work.functions.drain())
        .map(|(_, function, span)| functions.append(function, span))
        .collect();
    work.functions = functions;
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
    let handles: Vec<Handle<Function>> = work.functions.iter().map(|(h, _)| h).collect();
    for handle in handles {
        renumber(work.functions.get_mut(handle));
    }
    (work.entry_points.iter_mut()).for_each(|entry| renumber(&mut entry.function));

    for (site, pick_at) in sites.iter().zip(&picks) {
        let function = match site.place {
            Place::Function(at) => work.functions.get_mut(moved[at]),
            Place::Entry(at) => &mut work.entry_points[at].function,
        };
        let picked = pick(site, &function.expressions[site.at], &textures);
        // Nothing reads the entry now: it is left as a number, which an emit may hold.
        *function.expressions.get_mut(site.access) = E::Unary {
            op: naga::UnaryOperator::BitwiseNot,
            expr: site.index,
        };
        *function.expressions.get_mut(site.at) = E::CallResult(*pick_at);
        let call = S::Call {
            function: *pick_at,
            arguments: std::iter::once(site.index)
                .chain(site.operands.iter().copied())
                .collect(),
            result: Some(site.at),
        };
        let mut placed = false;
        let expressions = &function.expressions;
        each_block(&mut function.body, &mut |block| {
            placed = placed || call_in_place(block, expressions, site.at, &call);
        });
        if !placed {
            return Err("no statement of the code evaluates a sample of the array".into());
        }
        *work.functions.get_mut(*pick_at) = picked;
    }
    validate(&work).map_err(|why| format!("the lowered stage is not valid: {why}"))?;
    *module = work;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARRAYED: &str = "
@group(0) @binding(0) var heap: binding_array<texture_2d<f32>, 256>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<uniform> numbers: vec4<u32>;

fn shade(uv: vec2<f32>) -> vec4<f32> {
    let colour = textureSample(heap[numbers.x / 8u], bilinear, uv);
    var total = colour;
    if uv.x > 0.5 {
        total += textureSampleLevel(heap[numbers.y], bilinear, uv, 0.0);
    }
    return total + textureLoad(heap[numbers.z], vec2<i32>(0, 0), 0);
}

@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return shade(uv);
}
";

    fn lowered() -> Module {
        let mut module = naga::front::wgsl::parse_str(ARRAYED).unwrap();
        plain_textures(&mut module, (0, 0), &[0, 3, 5], 1, 7).unwrap();
        module
    }

    #[test]
    fn an_array_of_textures_becomes_one_texture_for_each_entry_a_material_fills() {
        let module = lowered();
        let mut bound: Vec<(u32, u32, bool)> = (module.global_variables.iter())
            .filter_map(|(_, global)| {
                let binding = global.binding.as_ref()?;
                let array = matches!(
                    module.types[global.ty].inner,
                    TypeInner::BindingArray { .. }
                );
                let image = matches!(module.types[global.ty].inner, TypeInner::Image { .. });
                (array || image).then_some((binding.group, binding.binding, array))
            })
            .collect();
        bound.sort_unstable();
        assert_eq!(bound, [(1, 7, false), (1, 8, false), (1, 9, false)]);
        // Three uses of the array, each a function of its own before the one that calls it.
        assert_eq!(module.functions.len(), 4);
        let text = super::super::shading::wgsl(&module).unwrap();
        assert!(!text.contains("binding_array"), "{text}");
        assert!(text.contains("== 3u") && text.contains("== 5u"), "{text}");
        assert!(
            text.contains("textureSampleLevel") && text.contains("textureLoad"),
            "{text}"
        );
    }

    #[test]
    fn an_array_the_code_asks_its_size_or_no_entry_is_known_of_is_left_as_it_is() {
        let asked = ARRAYED.replace(
            "return total +",
            "let size = textureDimensions(heap[numbers.w]); return vec4<f32>(f32(size.x)) + total +",
        );
        let mut module = naga::front::wgsl::parse_str(&asked).unwrap();
        let before = format!("{module:?}");
        assert!(plain_textures(&mut module, (0, 0), &[0, 3], 1, 7).is_err());
        assert!(plain_textures(&mut module, (0, 0), &[], 1, 7).is_err());
        assert!(plain_textures(&mut module, (0, 2), &[0], 1, 7).is_err());
        assert_eq!(format!("{module:?}"), before);
    }
}
