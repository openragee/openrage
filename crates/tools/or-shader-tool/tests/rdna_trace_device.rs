//! A program that tests rays against nodes, lowered a lane an invocation, built into a pipeline
//! on the machine's software adapter: where a device's own shader compiler first reads the test
//! of a node and a walk's module, and where a module that meets in a dispatch loop is run. No
//! hardware adapter is asked for.
//!
//! Each test is a process of its own under the runner, which ends it at the tier's limit. The
//! instance asks for no debug layer and no validation layer of the backend.
#![cfg(feature = "device-check")]

use std::borrow::Cow;
use std::path::Path;
use std::time::Instant;

use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna::{self, Launch, Options, Program, Region};
use or_shader_front::rdna_corpus::Corpus;

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

/// A made-up program whose graph does not nest: its entry branches into the middle of a loop
/// that the block after it enters at its head, and the loop's second block holds a barrier.
const MEETS_IN_A_LOOP_ENTERED_TWICE: [u32; 9] = [
    0x7e06_02f2,
    0xbf06_8080,
    0xbf85_0001,
    0x7e06_02f0,
    0xbf8a_0000,
    0xbf07_8080,
    0xbf85_fffc,
    0x7e08_0303,
    0xbf81_0000,
];

/// The software adapter's device and queue, with the adapter's name and what compiles Direct3D
/// 12's shaders; `None`, said in the ledger, where the machine has no software adapter.
fn software(test: &str) -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::DX12;
    }
    descriptor.flags = wgpu::InstanceFlags::empty();
    let descriptor = or_renderer::instance::with_dxc(descriptor);
    let compiler = format!("{:?}", descriptor.backend_options.dx12.shader_compiler);
    let instance = wgpu::Instance::new(descriptor);
    let asked = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        compatible_surface: None,
        force_fallback_adapter: true,
    };
    let adapter = match pollster::block_on(instance.request_adapter(&asked)) {
        Ok(adapter) if adapter.get_info().device_type == wgpu::DeviceType::Cpu => adapter,
        Ok(adapter) => {
            let info = adapter.get_info();
            or_test_support::skip_device(
                test,
                format_args!("{} is no software adapter", info.name),
            );
            return None;
        }
        Err(why) => {
            or_test_support::skip_device(test, format_args!("no software adapter: {why}"));
            return None;
        }
    };
    let info = adapter.get_info();
    let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rdna-trace"),
        required_limits: adapter.limits(),
        ..Default::default()
    }));
    let (device, queue) = device.expect("the software adapter's device");
    Some((
        device,
        queue,
        format!("{} ({:?}), {compiler}", info.name, info.backend),
    ))
}

/// Builds `module`'s compute pipeline on `device`: the milliseconds it took, or what the device
/// or its compiler said.
fn built(device: &wgpu::Device, module: &naga::Module) -> Result<u128, String> {
    made(device, module).map(|(_, took)| took)
}

/// [`built`], with the pipeline.
fn made(
    device: &wgpu::Device,
    module: &naga::Module,
) -> Result<(wgpu::ComputePipeline, u128), String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let started = Instant::now();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: None,
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let took = started.elapsed().as_millis();
    let internal = pollster::block_on(internal.pop());
    let validation = pollster::block_on(validation.pop());
    match validation.or(internal) {
        None => Ok((pipeline, took)),
        Some(error) => Err(error.to_string()),
    }
}

/// A compute program of one wave of 64 lanes, a thread a lane.
fn compute(words: &[u32], shared_dwords: u32) -> Program {
    Program {
        kind: ShaderKind::Compute,
        slot: 0,
        words: words.to_vec(),
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
            shared_dwords,
        }),
    }
}

/// The adapter ends its process on a barrier in an arm of a `switch` of three ways or more,
/// taken or not, once the program runs: so the blocks of a dispatch loop that meets stand
/// under `if`s, and this module is run, one group of it, and not only built.
#[test]
fn a_barrier_in_a_block_of_a_graph_that_does_not_nest_runs_on_the_software_adapter() {
    let test = "a barrier in a dispatch loop on the software adapter";
    let Some((device, queue, _)) = software(test) else {
        return;
    };
    let program = compute(&MEETS_IN_A_LOOP_ENTERED_TWICE, 0);
    let lowered = rdna::translate(&program).unwrap();
    let dispatched = "a dispatch loop for a graph that does not nest";
    assert!(lowered.approximations.contains_key(dispatched));
    let (pipeline, _) = made(&device, &lowered.module).unwrap();
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.dispatch_workgroups(1, 1, 1);
    }
    queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the group runs to its end");
    let refused = pollster::block_on(validation.pop());
    assert!(refused.is_none(), "{refused:?}");
}

fn traced(program: &Program, region: Option<Region>) -> Result<rdna::Lowered, String> {
    let options = Options {
        arena: true,
        region,
        ..Options::default()
    };
    rdna::translate_with(program, &options).map_err(|why| why.to_string())
}

#[test]
fn a_lane_s_own_module_with_the_test_of_a_node_is_built_on_the_software_adapter() {
    let test = "a module a lane an invocation on the software adapter";
    let Some((device, _, _)) = software(test) else {
        return;
    };
    let lowered = traced(&compute(&A_LANE_S_OWN, 128), None).unwrap();
    assert!(!lowered.subgroups);
    assert!((lowered.module.functions.iter())
        .any(|(_, function)| function.name.as_deref() == Some("bvh_intersect")));
    built(&device, &lowered.module).unwrap();
}

/// The value of `key` in a walk's file.
fn stated<'a>(text: &'a str, key: &str) -> &'a str {
    let lines = text.lines().filter_map(|line| line.split_once('='));
    let found = lines.into_iter().find(|(name, _)| name.trim() == key);
    found
        .unwrap_or_else(|| panic!("the walk's file states no {key}"))
        .1
        .trim()
}

fn number(text: &str) -> u32 {
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.unwrap_or_else(|_| panic!("{text} is no number"))
}

/// The walks `OPENRAGE_RDNA_WALKS` names, of the user's own build: each walk lowered alone, and
/// the program it is of lowered whole, built on the software adapter. What the device said of
/// each is written beside the walk's file, with `.device.out` after its name; a walk alone
/// that is not built fails the test, and a whole program is only recorded, since it binds
/// more than a device with nothing bound to it lays out.
#[test]
fn a_walk_of_the_build_lowered_alone_is_built_on_the_software_adapter() {
    let test = "a walk of the build on the software adapter";
    let Some(files) = std::env::var_os("OPENRAGE_RDNA_WALKS") else {
        or_test_support::skip_install(test, "OPENRAGE_RDNA_WALKS names no file of a walk");
        return;
    };
    let Some((device, _, adapter)) = software(test) else {
        return;
    };
    for file in std::env::split_paths(&files) {
        let text = std::fs::read_to_string(&file).expect("the walk's file");
        let mut corpus = Corpus::open(Path::new(stated(&text, "corpus"))).expect("the corpus");
        let index = number(stated(&text, "program")) as usize;
        let bytes = corpus.program(index).ok().flatten().expect("the program");
        let program = rdna::unwrap(&bytes)
            .expect("the program's blocks")
            .swap_remove(0);
        // A walk a wave takes together leaves no instance and no stack word of a lane's own.
        let of_a_wave = text
            .lines()
            .any(|line| line.replace(' ', "") == "form=wave");
        let left: &[&str] = match of_a_wave {
            true => &["leaf", "t", "count"],
            false => &["leaf", "t", "instance", "count", "stack"],
        };
        let region = Region {
            first: number(stated(&text, "first")),
            exit: number(stated(&text, "exit")),
            left: (left.iter())
                .map(|what| stated(&text, what).to_owned())
                .collect(),
        };
        let mut report = format!("{adapter}\n");
        let alone = traced(&program, Some(region)).expect("the walk lowers");
        let made = built(&device, &alone.module);
        report += &format!("the walk alone: {made:?}\n");
        let whole = traced(&program, None).and_then(|lowered| built(&device, &lowered.module));
        report += &format!("the whole program: {whole:?}\n");
        let mut out = file.clone().into_os_string();
        out.push(".device.out");
        std::fs::write(&out, report).expect("what the device said, written");
        made.unwrap();
    }
}
