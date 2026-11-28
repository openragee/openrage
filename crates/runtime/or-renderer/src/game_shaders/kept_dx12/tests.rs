use super::*;

fn key<'a>(hlsl: &'a str, arguments: &'a [String], compiler: &'a str, adapter: &'a str) -> Key<'a> {
    Key {
        hlsl,
        arguments,
        compiler,
        adapter,
    }
}

#[test]
fn a_kept_stage_is_keyed_by_its_source_arguments_compiler_and_adapter() {
    let plain = arguments("main", "vs_6_6", false);
    let half = arguments("main", "vs_6_6", true);
    let other_profile = arguments("main", "vs_6_5", false);
    let first = key("float4 a;", &plain, "dxc 1.8", "a card 1.0").digest();
    assert_eq!(
        first,
        key("float4 a;", &plain, "dxc 1.8", "a card 1.0").digest()
    );
    let changed = [
        key("float4 b;", &plain, "dxc 1.8", "a card 1.0").digest(),
        key("float4 a;", &half, "dxc 1.8", "a card 1.0").digest(),
        key("float4 a;", &other_profile, "dxc 1.8", "a card 1.0").digest(),
        key("float4 a;", &plain, "dxc 1.7", "a card 1.0").digest(),
        key("float4 a;", &plain, "dxc 1.8", "a card 1.1").digest(),
    ];
    for (at, other) in changed.iter().enumerate() {
        assert_ne!(
            *other, first,
            "the part changed at {at} leaves the key as it was"
        );
    }
    // A part's bytes moved into its neighbour make another key.
    assert_ne!(digest(&["ab", "c"]), digest(&["a", "bc"]));
}

#[test]
fn a_kept_stage_reads_back_whole_and_a_cut_changed_or_foreign_file_is_refused() {
    let dxil = b"DXBC and the rest of a stage".to_vec();
    let mine = digest(&["mine"]);
    let file = encode(&dxil, &mine);
    assert_eq!(decode(&file, &mine), Some(dxil.as_slice()));
    // Cut short, at any length.
    for cut in [0, 4, MAGIC.len(), MAGIC.len() + DIGEST, file.len() - 1] {
        assert_eq!(decode(&file[..cut], &mine), None, "cut at {cut}");
    }
    // Another key's stage, a byte changed, and a file of another form.
    assert_eq!(decode(&file, &digest(&["theirs"])), None);
    let mut changed = file.clone();
    *changed.last_mut().expect("the file holds the stage") ^= 1;
    assert_eq!(decode(&changed, &mine), None);
    let mut other_form = file.clone();
    other_form[MAGIC.len() - 1] ^= 1;
    assert_eq!(decode(&other_form, &mine), None);
    // A file holding no stage at all.
    assert_eq!(decode(&encode(&[], &mine), &mine), None);
}

#[test]
fn a_stage_is_kept_in_a_file_that_reads_back_and_one_cut_short_is_refused() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("deeper").join("a.dxil");
    let mine = digest(&["mine"]);
    store(&file, &encode(b"a stage", &mine));
    let bytes = std::fs::read(&file).expect("the stage was kept");
    assert_eq!(decode(&bytes, &mine), Some(b"a stage".as_slice()));
    std::fs::write(&file, &bytes[..bytes.len() - 2]).expect("the file is cut");
    let bytes = std::fs::read(&file).expect("the file is there");
    assert_eq!(decode(&bytes, &mine), None);
    // Nothing is left beside it.
    let beside = std::fs::read_dir(file.parent().expect("a folder")).expect("it lists");
    assert_eq!(beside.count(), 1);
}

#[test]
fn only_a_direct3d_12_device_that_takes_compiled_stages_is_handed_them() {
    use wgpu::Backend::{Dx12, Metal, Vulkan};
    assert!(used_on(Dx12, FEATURES));
    assert!(used_on(Dx12, wgpu::Features::all()));
    assert!(!used_on(Dx12, wgpu::Features::empty()));
    for backend in [Vulkan, Metal] {
        assert!(!used_on(backend, wgpu::Features::all()), "{backend:?}");
    }
}

#[test]
fn a_pass_refused_with_a_stage_handed_deletes_it_and_is_built_again_and_one_built_keeps_it() {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let file = folder.path().join("a.dxil");
    std::fs::write(&file, b"a stage").expect("the stage is written");
    let hand = || HANDED.with(|handed| handed.borrow_mut().push(file.clone()));
    // A pass that was built forgets what it was handed and keeps the file.
    hand();
    assert!(!forget_handed(false));
    assert!(file.is_file());
    // Nothing was handed for a pass refused after that: it is not built again.
    assert!(!forget_handed(true));
    hand();
    assert!(forget_handed(true));
    assert!(!file.exists());
    // The pass is built again with no stage of this module, and the next one with them.
    let again: Result<(), ()> = without(|| {
        assert!(OFF.with(Cell::get));
        Err(())
    });
    assert_eq!(again, Err(()));
    assert!(!OFF.with(Cell::get));
}

/// A uniform buffer, a storage buffer, a texture and a sampler in group 0, and an array of
/// textures, a read-write buffer, a comparison sampler and a sampler in group 1.
fn made_up_groups() -> Vec<Option<Vec<wgpu::BindGroupLayoutEntry>>> {
    let entry = |binding, ty, count: Option<u32>| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty,
        count: count.and_then(NonZeroU32::new),
    };
    let buffer = |ty| wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let texture = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let sampler = wgpu::BindingType::Sampler;
    let read_only = wgpu::BufferBindingType::Storage { read_only: true };
    let read_write = wgpu::BufferBindingType::Storage { read_only: false };
    vec![
        // Out of order, as a layout's entries may be given.
        Some(vec![
            entry(3, sampler(wgpu::SamplerBindingType::Filtering), None),
            entry(0, buffer(wgpu::BufferBindingType::Uniform), None),
            entry(1, buffer(read_only), None),
            entry(2, texture, None),
        ]),
        Some(vec![
            entry(0, texture, Some(4)),
            entry(1, buffer(read_write), None),
            entry(2, sampler(wgpu::SamplerBindingType::Comparison), None),
            entry(3, sampler(wgpu::SamplerBindingType::Filtering), None),
        ]),
        None,
        Some(vec![entry(
            0,
            buffer(wgpu::BufferBindingType::Uniform),
            None,
        )]),
    ]
}

#[test]
fn registers_are_numbered_on_across_groups_by_kind_with_each_groups_samplers_after_its_views() {
    let options = options(&made_up_groups(), hlsl::ShaderModel::V6_6).expect("it is laid out");
    let at = |group, binding| {
        let target = options.binding_map[&naga::ResourceBinding { group, binding }];
        (target.space, target.register, target.binding_array_size)
    };
    assert_eq!(at(0, 0), (0, 0, None), "the uniform buffer is b0");
    assert_eq!(at(0, 1), (0, 0, None), "the storage buffer is t0");
    assert_eq!(at(0, 2), (0, 1, None), "the texture is t1");
    assert_eq!(at(0, 3), (255, 0, None), "the sampler is the group's first");
    let first = options.sampler_buffer_binding_map[&hlsl::SamplerIndexBufferKey { group: 0 }];
    assert_eq!(first.register, 2, "group 0's samplers are t2");
    assert_eq!(at(1, 0), (0, 3, Some(4)), "the textures are t3 to t6");
    assert_eq!(at(1, 1), (0, 0, None), "the read-write buffer is u0");
    assert_eq!(at(1, 2), (255, 0, None));
    assert_eq!(at(1, 3), (255, 1, None));
    let second = options.sampler_buffer_binding_map[&hlsl::SamplerIndexBufferKey { group: 1 }];
    assert_eq!(second.register, 7, "group 1's samplers are t7");
    assert_eq!(
        at(3, 0),
        (0, 1, None),
        "the uniform buffer of group 3 is b1"
    );
    let constants = options
        .special_constants_binding
        .expect("drawing constants");
    assert_eq!(
        (constants.space, constants.register),
        (0, 2),
        "after every group"
    );
    assert_eq!(
        options.sampler_heap_target.comparison_samplers.register,
        2048
    );
    assert!(!options.fake_missing_bindings);
}

#[test]
fn a_binding_whose_registers_are_not_laid_out_here_is_refused() {
    let dynamic = wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: None,
        },
        count: None,
    };
    assert!(options(&[Some(vec![dynamic])], hlsl::ShaderModel::V6_6).is_err());
}

fn parsed(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the test's stage parses")
}

#[test]
fn a_pass_is_written_as_hlsl_with_each_stage_at_the_registers_of_the_passs_layout() {
    let vertex = parsed(
        "struct View { m: mat4x4<f32> }
        @group(0) @binding(0) var<uniform> view: View;
        struct Out { @builtin(position) at: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) unread: f32 }
        @vertex fn shade_vertex(@builtin(vertex_index) i: u32) -> Out {
            var out: Out;
            out.at = view.m * vec4<f32>(f32(i), 0.0, 0.0, 1.0);
            out.uv = vec2<f32>(0.5);
            out.unread = 1.0;
            return out;
        }",
    );
    let fragment = parsed(
        "@group(1) @binding(0) var picture: texture_2d<f32>;
        @group(1) @binding(1) var picked: sampler;
        @fragment fn shade_pixel(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
            return textureSample(picture, picked, uv);
        }",
    );
    let model = hlsl::ShaderModel::V6_6;
    let written =
        hlsl_of((&vertex, &fragment), ShaderStage::Vertex, model).expect("the vertex stage");
    assert_eq!(written.entry, "shade_vertex");
    assert!(written.source.contains("register(b0"), "{}", written.source);
    // The vertex index is offset by the drawing constants, after the one uniform buffer.
    assert!(written.source.contains("register(b1"), "{}", written.source);
    let written =
        hlsl_of((&vertex, &fragment), ShaderStage::Fragment, model).expect("the pixel stage");
    assert_eq!(written.entry, "shade_pixel");
    assert!(written.source.contains("register(t0"), "{}", written.source);
    assert!(written.source.contains("register(t1"), "{}", written.source);
    assert_eq!(profile(ShaderStage::Fragment, model), "ps_6_6");
    // A stage that binds in the other stage's groups is not written.
    assert!(hlsl_of((&fragment, &vertex), ShaderStage::Vertex, model).is_err());
}

#[test]
fn the_shader_model_follows_the_compilers_version_up_to_the_highest_kept() {
    use hlsl::ShaderModel as M;
    assert_eq!(shader_model(1, 0), M::V6_0);
    assert_eq!(shader_model(1, 5), M::V6_5);
    assert_eq!(shader_model(1, 8), MOST);
    assert_eq!(shader_model(2, 0), MOST);
}

#[test]
fn dxc_is_given_the_backends_arguments_without_its_debugging_pair() {
    let given = arguments("main", "ps_6_6", false);
    assert_eq!(
        given,
        [
            "-E",
            "main",
            "-T",
            "ps_6_6",
            "-HV",
            "2018",
            "-no-warnings",
            "-Ges"
        ]
    );
    assert!(!given
        .iter()
        .any(|argument| argument == "-Zi" || argument == "-Od"));
    let half = arguments("main", "ps_6_6", true);
    assert_eq!(half.last().map(String::as_str), Some("-enable-16bit-types"));
}

#[test]
fn the_backend_whose_registers_are_laid_out_here_is_the_one_the_workspace_builds() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.lock"))
        .expect("the workspace's lock file")
        .replace("\r\n", "\n");
    let built = format!("name = \"wgpu-hal\"\nversion = \"{MIRRORED}\"\n");
    assert!(
        lock.contains(&built),
        "wgpu-hal is no longer {MIRRORED}: read its Direct3D 12 pipeline layout again before changing MIRRORED"
    );
}

#[test]
fn a_function_of_a_choice_between_bare_floats_compiles_for_direct3d_12() {
    // A layer chosen among constants and rounded, as a translated stage samples an array by.
    let vertex = parsed(
        "@vertex fn shade_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            return vec4<f32>(f32(i));
        }",
    );
    let mut fragment = parsed(
        "@fragment fn shade_pixel(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
            let layer = select(select(0.0, 1.0, at.x > 1.0), 2.0, at.y > 1.0);
            return vec4<f32>(round(layer), floor(select(3.0, 4.0, at.z > 0.5)), 0.0, 1.0);
        }",
    );
    typed_literals(&mut fragment);
    let model = hlsl::ShaderModel::V6_0;
    let written = hlsl_of((&vertex, &fragment), ShaderStage::Fragment, model).expect("written");
    let Some(dxc) = dxc::compiler() else {
        panic!("no DXC: set WGPU_DX12_COMPILER=dxc with dxcompiler.dll on PATH");
    };
    let arguments = arguments(
        &written.entry,
        &profile(ShaderStage::Fragment, model),
        false,
    );
    let compiled = dxc.compile(&written.source, &arguments);
    assert!(compiled.is_ok(), "{compiled:?}\n{}", written.source);
}
