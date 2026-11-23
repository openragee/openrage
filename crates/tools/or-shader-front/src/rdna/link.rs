//! The two translated stages of a pass made to agree: every component the pixel stage reads is carried
//! in as few locations as hold them, numbered from nought, and the stage before it writes those and no other.

use naga::{
    Binding, Block, BuiltIn, Expression as E, Handle, Interpolation, Literal, Sampling, Scalar,
    ShaderStage, Span, Statement, TypeInner, VectorSize,
};

/// The location a lowered stage gives the high halves of a parameter of 16-bit floats, past that of its low halves.
pub const HIGH_HALVES: u32 = 32;

/// One component a pixel stage reads, and where a linked pass carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carried {
    /// The parameter slot the pixel program names.
    pub slot: u8,
    /// Whether it is one of the slot's high 16-bit halves.
    pub high: bool,
    pub component: u8,
    pub flat: bool,
    /// The location, and the component of its vector.
    pub location: u32,
    pub at: u8,
    /// Whether the stage before writes it; one it does not write is nought.
    pub fed: bool,
}

/// How the stages of a pass were joined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Linked {
    pub carried: Vec<Carried>,
    /// The location the clip distances cross at, where the pixel stage discards by them.
    pub clip: Option<u32>,
    /// The locations the pass crosses with.
    pub locations: u32,
}

/// What a link is told of the pass and of the device.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The slot the pixel stage reads each parameter of the stage before at, parameter `n` at
    /// place `n`; a parameter past the list is read at its own number.
    pub slots: Vec<u32>,
    /// Whether the device clips by distances the vertex stage gives. Without it they cross as
    /// one more location and the pixel stage discards where one is under nought.
    pub clip_distances: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("the module has no {0:?} entry point")]
    NoEntry(ShaderStage),
    #[error("the {0:?} stage's interface is not one the translator writes: {1}")]
    Shape(ShaderStage, &'static str),
}

fn vec4(module: &mut naga::Module) -> Handle<naga::Type> {
    let inner = TypeInner::Vector {
        size: VectorSize::Quad,
        scalar: Scalar::F32,
    };
    (module.types).insert(naga::Type { name: None, inner }, Span::UNDEFINED)
}

/// One location a pixel stage takes, the private variable its entry point stores it to, and the components read of that.
struct Input {
    argument: usize,
    location: u32,
    class: (Option<Interpolation>, Option<Sampling>),
    pointer: Handle<E>,
    read: [bool; 4],
}

fn inputs(module: &naga::Module, at: usize) -> Result<Vec<Input>, LinkError> {
    let shape = |why| LinkError::Shape(ShaderStage::Fragment, why);
    let entry = &module.entry_points[at].function;
    let mut inputs = Vec::new();
    let mut globals = Vec::new();
    for (argument, stated) in entry.arguments.iter().enumerate() {
        let Some(Binding::Location {
            location,
            interpolation,
            sampling,
            ..
        }) = stated.binding
        else {
            continue;
        };
        let float4 = TypeInner::Vector {
            size: VectorSize::Quad,
            scalar: Scalar::F32,
        };
        if module.types[stated.ty].inner != float4 {
            return Err(shape("an input that is not four floats"));
        }
        let stored = entry.body.iter().find_map(|statement| match statement {
            Statement::Store { pointer, value }
                if entry.expressions[*value] == E::FunctionArgument(argument as u32) =>
            {
                match entry.expressions[*pointer] {
                    E::GlobalVariable(global) => Some((*pointer, global)),
                    _ => None,
                }
            }
            _ => None,
        });
        let (pointer, global) = stored.ok_or(shape("an input its entry point does not keep"))?;
        globals.push(global);
        inputs.push(Input {
            argument,
            location,
            class: (interpolation, sampling),
            pointer,
            read: [false; 4],
        });
    }
    let functions = (module.functions.iter().map(|(_, function)| function)).chain([entry]);
    for function in functions {
        let input = |pointer: Handle<E>| match function.expressions[pointer] {
            E::GlobalVariable(global) => globals.iter().position(|held| *held == global),
            _ => None,
        };
        for (_, expression) in function.expressions.iter() {
            match expression {
                E::AccessIndex { base, index } => {
                    if let (Some(input), true) = (input(*base), *index < 4) {
                        inputs[input].read[*index as usize] = true;
                    }
                }
                E::Load { pointer } | E::Access { base: pointer, .. } => {
                    if let Some(input) = input(*pointer) {
                        inputs[input].read = [true; 4];
                    }
                }
                _ => {}
            }
        }
    }
    Ok(inputs)
}

/// The pixel stage's inputs packed: its arguments replaced, each private variable filled from
/// where its components now are. Gives what is carried and each location's interpolation.
type Packed = (Vec<Carried>, Vec<(Option<Interpolation>, Option<Sampling>)>);

fn pack_inputs(module: &mut naga::Module, at: usize) -> Result<Packed, LinkError> {
    let inputs = inputs(module, at)?;
    let float4 = vec4(module);
    // A class of interpolation fills its own vectors, in the order the classes are first read.
    let mut classes: Vec<(Option<Interpolation>, Option<Sampling>)> = Vec::new();
    for input in inputs.iter().filter(|input| input.read.contains(&true)) {
        if !classes.contains(&input.class) {
            classes.push(input.class);
        }
    }
    let mut carried = Vec::new();
    let mut locations = Vec::new();
    // Per input, where each of its components is: the location and the component there.
    let mut places = vec![[None; 4]; inputs.len()];
    for class in classes {
        let mut filled = 4;
        let of_class = |input: &&(usize, &Input)| input.1.class == class;
        let mut ordered: Vec<(usize, &Input)> = inputs.iter().enumerate().collect();
        ordered.sort_by_key(|(_, input)| (input.location % HIGH_HALVES, input.location));
        for (index, input) in ordered.iter().filter(of_class) {
            for component in (0..4).filter(|c| input.read[*c]) {
                if filled == 4 {
                    locations.push(class);
                    filled = 0;
                }
                let location = locations.len() as u32 - 1;
                places[*index][component] = Some((location, filled));
                carried.push(Carried {
                    slot: (input.location % HIGH_HALVES) as u8,
                    high: input.location >= HIGH_HALVES,
                    component: component as u8,
                    flat: class.0 == Some(Interpolation::Flat),
                    location,
                    at: filled as u8,
                    fed: false,
                });
                filled += 1;
            }
        }
    }

    let function = &mut module.entry_points[at].function;
    let old: Vec<usize> = inputs.iter().map(|input| input.argument).collect();
    // The packed vectors come first; the arguments that are no location keep their order after them.
    let mut arguments: Vec<naga::FunctionArgument> = (locations.iter().enumerate())
        .map(
            |(location, (interpolation, sampling))| naga::FunctionArgument {
                name: Some(format!("carried{location}")),
                ty: float4,
                binding: Some(Binding::Location {
                    location: location as u32,
                    interpolation: *interpolation,
                    sampling: *sampling,
                    blend_src: None,
                    per_primitive: false,
                }),
            },
        )
        .collect();
    let mut renumbered = vec![None; function.arguments.len()];
    for (index, argument) in function.arguments.drain(..).enumerate() {
        if !old.contains(&index) {
            renumbered[index] = Some(arguments.len() as u32);
            arguments.push(argument);
        }
    }
    function.arguments = arguments;
    for (_, expression) in function.expressions.iter_mut() {
        if let E::FunctionArgument(index) = expression {
            *expression = match renumbered[*index as usize] {
                Some(now) => E::FunctionArgument(now),
                // Nothing reads it once its store is gone.
                None => E::ZeroValue(float4),
            };
        }
    }
    let vectors: Vec<Handle<E>> = (0..locations.len() as u32)
        .map(|location| {
            (function.expressions).append(E::FunctionArgument(location), Span::UNDEFINED)
        })
        .collect();
    let nought = (function.expressions).append(E::Literal(Literal::F32(0.0)), Span::UNDEFINED);
    let before = function.expressions.len();
    let mut stores = Block::new();
    for (input, places) in inputs.iter().zip(&places) {
        let components = (places.iter())
            .map(|place| match place {
                Some((location, at)) => function.expressions.append(
                    E::AccessIndex {
                        base: vectors[*location as usize],
                        index: *at,
                    },
                    Span::UNDEFINED,
                ),
                None => nought,
            })
            .collect();
        let value = function.expressions.append(
            E::Compose {
                ty: float4,
                components,
            },
            Span::UNDEFINED,
        );
        let pointer = input.pointer;
        stores.push(Statement::Store { pointer, value }, Span::UNDEFINED);
    }
    let pointers: Vec<Handle<E>> = inputs.iter().map(|input| input.pointer).collect();
    let mut body = Block::new();
    if function.expressions.len() > before {
        let emitted = function.expressions.range_from(before);
        body.push(Statement::Emit(emitted), Span::UNDEFINED);
    }
    body.extend_block(stores);
    for statement in std::mem::take(&mut function.body).iter().cloned() {
        // The stores of the arguments that are gone.
        let gone = matches!(&statement, Statement::Store { pointer, value }
            if pointers.contains(pointer) && matches!(function.expressions[*value], E::ZeroValue(_)));
        if !gone {
            body.push(statement, Span::UNDEFINED);
        }
    }
    function.body = body;
    Ok((carried, locations))
}

fn entry(module: &naga::Module, stage: ShaderStage) -> Result<usize, LinkError> {
    (module.entry_points.iter())
        .position(|entry| entry.stage == stage)
        .ok_or(LinkError::NoEntry(stage))
}

/// A pixel stage with no stage before it: its inputs packed as [`link`] packs them.
pub fn pack(pixel: &mut naga::Module) -> Result<Linked, LinkError> {
    let at = entry(pixel, ShaderStage::Fragment)?;
    let (carried, locations) = pack_inputs(pixel, at)?;
    Ok(Linked {
        carried,
        clip: None,
        locations: locations.len() as u32,
    })
}

/// The pixel stage discards where a clip distance, carried at `location`, is under nought.
// Carried with perspective: a distance is linear over the triangle in clip space, so that is the
// interpolation whose sign at a pixel is the sign a clipper would have cut by.
fn discard_clipped(module: &mut naga::Module, at: usize, location: u32) {
    let float4 = vec4(module);
    let function = &mut module.entry_points[at].function;
    let index = function.arguments.len() as u32;
    function.arguments.push(naga::FunctionArgument {
        name: Some("clip_distances".to_owned()),
        ty: float4,
        binding: Some(Binding::Location {
            location,
            interpolation: Some(Interpolation::Perspective),
            sampling: Some(Sampling::Center),
            blend_src: None,
            per_primitive: false,
        }),
    });
    let expressions = &mut function.expressions;
    let distances = expressions.append(E::FunctionArgument(index), Span::UNDEFINED);
    let nought = expressions.append(E::ZeroValue(float4), Span::UNDEFINED);
    let before = expressions.len();
    let under = expressions.append(
        E::Binary {
            op: naga::BinaryOperator::Less,
            left: distances,
            right: nought,
        },
        Span::UNDEFINED,
    );
    let condition = expressions.append(
        E::Relational {
            fun: naga::RelationalFunction::Any,
            argument: under,
        },
        Span::UNDEFINED,
    );
    let mut body = Block::new();
    body.push(
        Statement::Emit(expressions.range_from(before)),
        Span::UNDEFINED,
    );
    body.push(
        Statement::If {
            condition,
            accept: Block::from_vec(vec![Statement::Kill]),
            reject: Block::new(),
        },
        Span::UNDEFINED,
    );
    body.extend_block(std::mem::take(&mut function.body));
    function.body = body;
}

/// Make `pixel` and the vertex stage `producer` one pass: the components `pixel` reads are packed
/// into locations numbered from nought, `producer` writes those from the parameters
/// [`Options::slots`] pairs them with, nought where it has none, and drops what nothing reads.
pub fn link(
    producer: &mut naga::Module,
    pixel: &mut naga::Module,
    options: &Options,
) -> Result<Linked, LinkError> {
    let shape = |why| LinkError::Shape(ShaderStage::Vertex, why);
    let (vertex, fragment) = (
        entry(producer, ShaderStage::Vertex)?,
        entry(pixel, ShaderStage::Fragment)?,
    );
    // Read before anything is changed, so a refusal leaves both modules as they were.
    let result = (producer.entry_points[vertex].function.result.as_ref())
        .ok_or(shape("no result"))?
        .ty;
    let TypeInner::Struct { members, .. } = producer.types[result].inner.clone() else {
        return Err(shape("a result that is no structure"));
    };
    let function = &producer.entry_points[vertex].function;
    let returned = match function.body.last() {
        Some(Statement::Return { value: Some(value) }) => *value,
        _ => return Err(shape("a body that does not end by returning its result")),
    };
    let values = match &function.expressions[returned] {
        E::Compose { ty, components } if *ty == result && components.len() == members.len() => {
            components.clone()
        }
        _ => return Err(shape("a result that is not composed of its members")),
    };

    let (mut carried, classes) = pack_inputs(pixel, fragment)?;
    let clipped = members
        .iter()
        .any(|member| member.binding == Some(Binding::BuiltIn(BuiltIn::ClipDistance)));
    let clip = (clipped && !options.clip_distances).then_some(classes.len() as u32);
    if let Some(location) = clip {
        discard_clipped(pixel, fragment, location);
    }

    let float4 = vec4(producer);
    let function = &mut producer.entry_points[vertex].function;
    let nought = (function.expressions).append(E::Literal(Literal::F32(0.0)), Span::UNDEFINED);
    let before = function.expressions.len();
    // The parameter a pixel location reads, as the value its member is composed of.
    let written = |location: u32| {
        members.iter().zip(&values).find_map(|(member, value)| {
            let Some(Binding::Location { location: at, .. }) = member.binding else {
                return None;
            };
            let (low, high) = (at % HIGH_HALVES, at / HIGH_HALVES * HIGH_HALVES);
            let slot = options.slots.get(low as usize).copied().unwrap_or(low);
            (high + slot == location).then_some(*value)
        })
    };
    let mut kept = Vec::new();
    let mut components = Vec::new();
    let mut offset = 0;
    let mut member = |name: String, ty, binding, value| {
        kept.push(naga::StructMember {
            name: Some(name),
            ty,
            binding: Some(binding),
            offset,
        });
        offset += 16;
        components.push(value);
    };
    for (stated, value) in members.iter().zip(&values) {
        let name = stated.name.clone().unwrap_or_default();
        match &stated.binding {
            Some(Binding::Location { .. }) | None => {}
            Some(Binding::BuiltIn(BuiltIn::ClipDistance)) if clip.is_some() => {
                let parts = (0..4)
                    .map(|index| {
                        let base = *value;
                        (function.expressions)
                            .append(E::AccessIndex { base, index }, Span::UNDEFINED)
                    })
                    .collect();
                let value = function.expressions.append(
                    E::Compose {
                        ty: float4,
                        components: parts,
                    },
                    Span::UNDEFINED,
                );
                let binding = Binding::Location {
                    location: clip.unwrap_or(0),
                    interpolation: Some(Interpolation::Perspective),
                    sampling: Some(Sampling::Center),
                    blend_src: None,
                    per_primitive: false,
                };
                member(name, float4, binding, value);
            }
            Some(binding) => member(name, stated.ty, binding.clone(), *value),
        }
    }
    for (location, (interpolation, sampling)) in classes.iter().enumerate() {
        let location = location as u32;
        let mut parts = [nought; 4];
        for carried in carried.iter_mut().filter(|c| c.location == location) {
            let from = u32::from(carried.slot) + if carried.high { HIGH_HALVES } else { 0 };
            if let Some(base) = written(from) {
                let index = u32::from(carried.component);
                parts[usize::from(carried.at)] =
                    (function.expressions).append(E::AccessIndex { base, index }, Span::UNDEFINED);
                carried.fed = true;
            }
        }
        let value = function.expressions.append(
            E::Compose {
                ty: float4,
                components: parts.to_vec(),
            },
            Span::UNDEFINED,
        );
        let binding = Binding::Location {
            location,
            interpolation: *interpolation,
            sampling: *sampling,
            blend_src: None,
            per_primitive: false,
        };
        member(format!("carried{location}"), float4, binding, value);
    }
    let ty = producer.types.insert(
        naga::Type {
            name: Some("Linked".into()),
            inner: TypeInner::Struct {
                members: kept,
                span: offset.max(16),
            },
        },
        Span::UNDEFINED,
    );
    let function = &mut producer.entry_points[vertex].function;
    let value = (function.expressions).append(E::Compose { ty, components }, Span::UNDEFINED);
    let emitted = function.expressions.range_from(before);
    let last = function.body.len() - 1;
    function.body.cull(last..);
    (function.body).push(Statement::Emit(emitted), Span::UNDEFINED);
    let value = Some(value);
    (function.body).push(Statement::Return { value }, Span::UNDEFINED);
    if let Some(result) = &mut function.result {
        result.ty = ty;
    }
    Ok(Linked {
        carried,
        clip,
        locations: classes.len() as u32 + u32::from(clip.is_some()),
    })
}
