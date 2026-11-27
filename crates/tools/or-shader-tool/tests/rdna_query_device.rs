//! The test of a node answered from the device's own ray query, run on the machine's software
//! adapter against the same test answered from the arena's bytes: a walk of a made-up scene
//! answered by each finds the same triangle of the same instance at the same distance, to the
//! device's rounding. No hardware adapter is asked for, and an adapter with no ray query is a
//! skip, said in the ledger.
#![cfg(feature = "device-check")]

use std::borrow::Cow;

use naga::{Expression as E, Handle, Span, Statement as S};
use or_io::gta6::accel::layout::QUERY_TOP;
use or_io::gta6::accel::{
    walk_lane_with, Arena, Hit, Instance, Probe, Ray, Structure, Triangle, Walk,
};
use or_renderer::game_shaders::trace::query::{self, Structures};
use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna::{self, Launch, Options, Program, ResourceKind};
use wgpu::util::DeviceExt;

/// A made-up program: the lane's number, a dword at its own address of the group's memory and
/// back, a branch of its own, and one test of a node of the structure its descriptor names.
const A_LANE_S_OWN: [u32; 35] = [
    0x7e04_02f2,
    0x7e06_02f2,
    0x7e08_02f2,
    0x7e0a_02f2,
    0x7e0c_02f2,
    0x7e0e_02f2,
    0x7e10_02f2,
    0x7e12_02f2,
    0x7e14_02f2,
    0x7e16_02f2,
    0xd765_0014,
    0x0001_00c1,
    0xd766_0014,
    0x0002_28c1,
    0x3400_2882,
    0x4a02_28bc,
    0xd834_0000,
    0x0100_0100,
    0x7e02_0287,
    0xd8b4_0000,
    0x0100_0100,
    0x7e2c_0301,
    0xbe98_047e,
    0x7da2_289f,
    0xbf88_0001,
    0x7e30_0281,
    0xbefe_0418,
    0xbe88_0380,
    0xbe89_0b81,
    0xbe8a_03c1,
    0xbe8b_03ff,
    0x8900_03ff,
    0xf19c_0f01,
    0x0082_0000,
    0xbf81_0000,
];

/// The dwords of one test of a node: base, node, far, origin, direction, inverse.
const ROW: usize = 12;
/// The most tests one walk of the made-up scene asks: one more is a fault of the walk.
const MOST: usize = 512;

/// The software adapter's device and queue, opened with its ray query; `None`, said in the
/// ledger, where the machine has no software adapter or it offers no ray query.
fn software(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::DX12;
    }
    descriptor.flags = wgpu::InstanceFlags::empty();
    let instance = wgpu::Instance::new(or_renderer::instance::with_dxc(descriptor));
    let asked = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: true,
    };
    let adapter = match pollster::block_on(instance.request_adapter(&asked)) {
        Ok(adapter) if adapter.get_info().device_type == wgpu::DeviceType::Cpu => adapter,
        Ok(adapter) => {
            let name = adapter.get_info().name;
            or_test_support::skip_device(test, format_args!("{name} is no software adapter"));
            return None;
        }
        Err(why) => {
            or_test_support::skip_device(test, format_args!("no software adapter: {why}"));
            return None;
        }
    };
    let info = adapter.get_info();
    if !query::offered(info.backend, adapter.features()) {
        let name = info.name;
        or_test_support::skip_device(test, format_args!("{name} offers no ray query"));
        return None;
    }
    let mut required_limits = adapter.limits();
    let (required_features, experimental_features) =
        query::device_needs(&adapter, &mut required_limits);
    let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rdna-query"),
        required_features,
        required_limits,
        experimental_features,
        ..Default::default()
    }));
    Some(device.expect("the software adapter's device with its ray query"))
}

fn program() -> Program {
    Program {
        kind: ShaderKind::Compute,
        slot: 0,
        words: A_LANE_S_OWN.to_vec(),
        wave32: false,
        inputs: Vec::new(),
        vertex_inputs: Vec::new(),
        position_read: 0,
        front_face: false,
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 128,
        }),
    }
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn ty(module: &mut naga::Module, inner: naga::TypeInner) -> Handle<naga::Type> {
    module
        .types
        .insert(naga::Type { name: None, inner }, Span::UNDEFINED)
}

/// `expression` appended to `f`, emitted at the end of `block` where it has to be.
fn put(f: &mut naga::Function, block: &mut naga::Block, expression: E) -> Handle<E> {
    let early = expression.needs_pre_emit();
    let at = f.expressions.append(expression, Span::UNDEFINED);
    if !early {
        block.push(
            S::Emit(naga::Range::new_from_bounds(at, at)),
            Span::UNDEFINED,
        );
    }
    at
}

/// The dword `index` of the buffer `pointer` points at.
fn dword(
    f: &mut naga::Function,
    block: &mut naga::Block,
    pointer: Handle<E>,
    index: Handle<E>,
) -> Handle<E> {
    let cell = put(
        f,
        block,
        E::Access {
            base: pointer,
            index,
        },
    );
    put(f, block, E::Load { pointer: cell })
}

/// `module` with a compute entry `replay` of one invocation: the tests of a node its buffer
/// `probes` holds, after their count, made one after another as one invocation's, and the
/// answer of the last in `answers`.
fn with_replay(mut module: naga::Module) -> naga::Module {
    use naga::{Literal, ScalarKind, TypeInner, VectorSize};
    let (test, _) = (module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("bvh_intersect"))
        .expect("the module tests nodes");
    let word = ty(&mut module, TypeInner::Scalar(naga::Scalar::U32));
    let three = ty(
        &mut module,
        TypeInner::Vector {
            size: VectorSize::Tri,
            scalar: naga::Scalar::F32,
        },
    );
    let four = ty(
        &mut module,
        TypeInner::Vector {
            size: VectorSize::Quad,
            scalar: naga::Scalar::U32,
        },
    );
    let words = ty(
        &mut module,
        TypeInner::Array {
            base: word,
            size: naga::ArraySize::Dynamic,
            stride: 4,
        },
    );
    let mut buffer = |name: &str, binding, access| {
        let global = naga::GlobalVariable {
            name: Some(name.to_owned()),
            space: naga::AddressSpace::Storage { access },
            binding: Some(naga::ResourceBinding { group: 1, binding }),
            ty: words,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        };
        module.global_variables.append(global, Span::UNDEFINED)
    };
    let probes = buffer("probes", 0, naga::StorageAccess::LOAD);
    let answers = buffer(
        "answers",
        1,
        naga::StorageAccess::LOAD | naga::StorageAccess::STORE,
    );

    let mut f = naga::Function {
        name: Some("replay".to_owned()),
        ..naga::Function::default()
    };
    let local = |f: &mut naga::Function, name: &str, ty| {
        let variable = naga::LocalVariable {
            name: Some(name.to_owned()),
            ty,
            init: None,
        };
        f.local_variables.append(variable, Span::UNDEFINED)
    };
    let (i, last) = (local(&mut f, "i", word), local(&mut f, "last", four));
    let mut body = naga::Block::new();
    let b = &mut body;
    let i = put(&mut f, b, E::LocalVariable(i));
    let last = put(&mut f, b, E::LocalVariable(last));
    let (probes, answers) = (
        put(&mut f, b, E::GlobalVariable(probes)),
        put(&mut f, b, E::GlobalVariable(answers)),
    );
    let literal = |f: &mut naga::Function, b: &mut naga::Block, word: u32| {
        put(f, b, E::Literal(Literal::U32(word)))
    };
    let zero = literal(&mut f, b, 0);
    b.push(
        S::Store {
            pointer: i,
            value: zero,
        },
        Span::UNDEFINED,
    );
    let nothing = put(&mut f, b, E::ZeroValue(four));
    b.push(
        S::Store {
            pointer: last,
            value: nothing,
        },
        Span::UNDEFINED,
    );

    let mut turn = naga::Block::new();
    let t = &mut turn;
    let count = dword(&mut f, t, probes, zero);
    let at = put(&mut f, t, E::Load { pointer: i });
    let done = put(
        &mut f,
        t,
        E::Binary {
            op: naga::BinaryOperator::GreaterEqual,
            left: at,
            right: count,
        },
    );
    let mut leave = naga::Block::new();
    leave.push(S::Break, Span::UNDEFINED);
    t.push(
        S::If {
            condition: done,
            accept: leave,
            reject: naga::Block::new(),
        },
        Span::UNDEFINED,
    );
    let row = literal(&mut f, t, ROW as u32);
    let one = literal(&mut f, t, 1);
    let first = put(
        &mut f,
        t,
        E::Binary {
            op: naga::BinaryOperator::Multiply,
            left: at,
            right: row,
        },
    );
    let first = put(
        &mut f,
        t,
        E::Binary {
            op: naga::BinaryOperator::Add,
            left: first,
            right: one,
        },
    );
    let read = |f: &mut naga::Function, t: &mut naga::Block, k: u32| {
        let by = literal(f, t, k);
        let index = put(
            f,
            t,
            E::Binary {
                op: naga::BinaryOperator::Add,
                left: first,
                right: by,
            },
        );
        dword(f, t, probes, index)
    };
    let (base, node) = (read(&mut f, t, 0), read(&mut f, t, 1));
    let float = |f: &mut naga::Function, t: &mut naga::Block, k: u32| {
        let bits = read(f, t, k);
        put(
            f,
            t,
            E::As {
                expr: bits,
                kind: ScalarKind::Float,
                convert: None,
            },
        )
    };
    let far = float(&mut f, t, 2);
    let mut arguments = vec![base, node, far];
    for from in [3, 6, 9] {
        let components = (from..from + 3).map(|k| float(&mut f, t, k)).collect();
        arguments.push(put(
            &mut f,
            t,
            E::Compose {
                ty: three,
                components,
            },
        ));
    }
    let result = f.expressions.append(E::CallResult(test), Span::UNDEFINED);
    t.push(
        S::Call {
            function: test,
            arguments,
            result: Some(result),
        },
        Span::UNDEFINED,
    );
    t.push(
        S::Store {
            pointer: last,
            value: result,
        },
        Span::UNDEFINED,
    );
    let next = put(
        &mut f,
        t,
        E::Binary {
            op: naga::BinaryOperator::Add,
            left: at,
            right: one,
        },
    );
    t.push(
        S::Store {
            pointer: i,
            value: next,
        },
        Span::UNDEFINED,
    );
    b.push(
        S::Loop {
            body: turn,
            continuing: naga::Block::new(),
            break_if: None,
        },
        Span::UNDEFINED,
    );

    let answer = put(&mut f, b, E::Load { pointer: last });
    for k in 0..4 {
        let part = put(
            &mut f,
            b,
            E::AccessIndex {
                base: answer,
                index: k,
            },
        );
        let index = literal(&mut f, b, k);
        let cell = put(
            &mut f,
            b,
            E::Access {
                base: answers,
                index,
            },
        );
        b.push(
            S::Store {
                pointer: cell,
                value: part,
            },
            Span::UNDEFINED,
        );
    }
    f.body = body;
    module.entry_points.push(naga::EntryPoint {
        name: "replay".to_owned(),
        stage: naga::ShaderStage::Compute,
        early_depth_test: None,
        workgroup_size: [1, 1, 1],
        workgroup_size_overrides: None,
        function: f,
        mesh_info: None,
        task_payload: None,
        incoming_ray_payload: None,
    });
    module
}

/// One body of the test of a node on the device: every test a walk asks is answered by running
/// the tests it asked before again, in order, then it, in one invocation, so that what the
/// device's query keeps of a walk is what it would keep had the walk run there.
struct Replay {
    pipeline: wgpu::ComputePipeline,
    groups: [wgpu::BindGroup; 2],
    probes: wgpu::Buffer,
    answers: wgpu::Buffer,
    readback: wgpu::Buffer,
    asked: Vec<u32>,
}

impl Replay {
    fn new(
        device: &wgpu::Device,
        lowered: &rdna::Lowered,
        arena: &[u32],
        structures: Option<&Structures>,
    ) -> Self {
        let module = with_replay(lowered.module.clone());
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("replay"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let refused = pollster::block_on(validation.pop());
        assert!(refused.is_none(), "{refused:?}");
        let storage = |contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents,
                usage: wgpu::BufferUsages::STORAGE | usage,
            })
        };
        let arena = storage(&bytes(arena), wgpu::BufferUsages::empty());
        let probes = storage(&[0; 4 * (1 + ROW * MOST)], wgpu::BufferUsages::COPY_DST);
        let answers = storage(&[0; 16], wgpu::BufferUsages::COPY_SRC);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = |kind: ResourceKind| {
            let found = lowered.resources.iter().find(|r| r.kind == kind);
            found.map(|r| r.binding)
        };
        let mut first = vec![wgpu::BindGroupEntry {
            binding: binding(ResourceKind::Arena).expect("the module reads the arena"),
            resource: arena.as_entire_binding(),
        }];
        if let Some(structures) = structures {
            let at = binding(ResourceKind::DeviceStructure).expect("the module asks the device");
            first.push(structures.entry(at));
        }
        let group = |index: u32, entries: &[wgpu::BindGroupEntry]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(index),
                entries,
            })
        };
        let second = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: probes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: answers.as_entire_binding(),
            },
        ];
        let groups = [group(0, &first), group(1, &second)];
        Self {
            pipeline,
            groups,
            probes,
            answers,
            readback,
            asked: Vec::new(),
        }
    }

    /// The answer of `probe`, the next test of the walk under way.
    fn answer(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        probe: &Probe,
    ) -> [u32; 4] {
        self.asked.push(probe.base >> 8);
        self.asked.push(probe.node);
        self.asked.push(probe.t_max.to_bits());
        for part in [probe.origin, probe.direction, probe.inverse] {
            self.asked.extend(part.map(f32::to_bits));
        }
        assert_eq!(
            self.asked.len() % ROW,
            0,
            "a test of a node is {ROW} dwords"
        );
        let count = self.asked.len() / ROW;
        assert!(count <= MOST, "a walk of more than {MOST} tests");
        let mut words = vec![count as u32];
        words.extend(&self.asked);
        queue.write_buffer(&self.probes, 0, &bytes(&words));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.groups[0], &[]);
            pass.set_bind_group(1, &self.groups[1], &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.answers, 0, &self.readback, 0, 16);
        queue.submit([encoder.finish()]);
        self.readback.map_async(wgpu::MapMode::Read, .., |_| ());
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the replay runs to its end");
        let answer = {
            let bytes = self.readback.get_mapped_range(..);
            std::array::from_fn(|k| {
                u32::from_le_bytes([
                    bytes[4 * k],
                    bytes[4 * k + 1],
                    bytes[4 * k + 2],
                    bytes[4 * k + 3],
                ])
            })
        };
        self.readback.unmap();
        answer
    }
}

/// A sheet of `n` by `n` squares from -1 to 1, two triangles a square that share its diagonal.
fn sheet(n: usize) -> Vec<Triangle> {
    let at = |i: usize| -1.0 + 2.0 * i as f32 / n as f32;
    let mut triangles = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let corner = |dx: usize, dy: usize| [at(x + dx), at(y + dy), 0.0];
            let (a, b, c, d) = (corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1));
            for corners in [[a, b, c], [a, c, d]] {
                triangles.push(Triangle {
                    corners,
                    geometry: 0,
                    primitive: triangles.len() as u32,
                    opaque: true,
                });
            }
        }
    }
    triangles
}

/// Two sheets one behind another and one beside them, and a lone triangle between: the arena
/// as built, the arena a walk through the query is given, the top level, and the near sheet.
fn scene() -> (Vec<u32>, Vec<u32>, Structure, Vec<Triangle>) {
    let mut arena = Arena::new();
    let near = sheet(4);
    let sheet = arena.bottom_level(&near).unwrap();
    let lone = Triangle {
        corners: [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let lone = arena.bottom_level(&[lone]).unwrap();
    let placed = |bottom: Structure, [x, y, z]: [f32; 3]| Instance {
        bottom,
        object_to_world: [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]],
        id: 0,
        mask: 0xFF,
        flags: 0,
        contribution: 0,
    };
    let instances = [
        placed(sheet, [0.0, 0.0, 0.0]),
        placed(sheet, [0.5, 0.25, 2.0]),
        placed(lone, [0.25, 0.0, -1.0]),
        placed(sheet, [3.0, 0.0, 1.0]),
    ];
    let top = arena.top_level(&instances).unwrap();
    let asked = query::asked_words(&arena, &top).expect("a top level");
    assert_eq!(asked[(QUERY_TOP / 4) as usize], top.base);
    (arena.words().to_vec(), asked, top, near)
}

/// Whether two hits are one triangle of one instance at one distance, to the device's rounding.
fn same(a: &Hit, b: &Hit) -> bool {
    let near = (a.t - b.t).abs() <= 1e-5 * a.t.abs().max(1.0);
    (a.leaf, a.instance) == (b.leaf, b.instance) && near
}

#[test]
fn a_walk_answered_by_the_device_s_query_finds_what_the_arena_s_test_finds() {
    let test = "the device's ray query under a walk on the software adapter";
    let Some((device, queue)) = software(test) else {
        return;
    };
    let gpu = (&device, &queue);
    let (plain, asked, top, near) = scene();
    let lower = |query| {
        let options = Options {
            arena: true,
            query,
            ..Options::default()
        };
        rdna::translate_with(&program(), &options).expect("the program lowers")
    };
    let structures = Structures::build(&device, &queue, &asked, top.base).expect("the structures");
    let mut arena = Replay::new(&device, &lower(false), &plain, None);
    let mut device_query = Replay::new(&device, &lower(true), &asked, Some(&structures));
    let (mut hits, mut apart) = (0, Vec::new());
    for (number, triangle) in near.iter().enumerate().step_by(2) {
        // At the middle of a triangle of the near sheet, from before the lone triangle.
        let middle: [f32; 3] =
            std::array::from_fn(|k| triangle.corners.iter().map(|c| c[k]).sum::<f32>() / 3.0);
        let origin = [0.1 * number as f32 - 0.7, 0.05, -5.0];
        let ray = Ray {
            origin,
            direction: std::array::from_fn(|k| middle[k] - origin[k]),
            t_min: 0.0,
            t_max: 20.0,
        };
        for first_hit in [false, true] {
            let walk = Walk {
                first_hit,
                ..Walk::default()
            };
            let walked = |replay: &mut Replay, words: &[u32]| {
                replay.asked.clear();
                let mut body = |probe: &Probe| replay.answer(gpu, probe);
                walk_lane_with(words, top.base, &ray, &walk, &mut body, &mut |_| true).hit
            };
            let by_arena = walked(&mut arena, &plain);
            let by_query = walked(&mut device_query, &asked);
            match (&by_arena, &by_query) {
                (Some(a), Some(b)) if same(a, b) || first_hit => hits += 1,
                (None, None) => {}
                _ => apart.push((number, first_hit, by_arena, by_query)),
            }
        }
    }
    assert!(apart.is_empty(), "{apart:?}");
    assert!(hits > 8, "{hits} hits");
}
