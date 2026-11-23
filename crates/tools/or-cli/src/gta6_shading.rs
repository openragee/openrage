//! A GTA VI model's shading pass for the renderer: the mesh and the pixel program of the pass
//! translated as one, each with what fills its bindings.

use std::collections::BTreeSet;

use or_io::gta5::shaders::sgd5::Sgd5Table;
use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna::link::{self, Linked};
use or_shader_front::rdna_bind::picks::{deferred_targets, picks};
use or_shader_front::rdna_bind::shading::{feed, parameter_of, parameter_slots};

pub use or_shader_front::rdna_bind::kept as gta6_kept;
pub use or_shader_front::rdna_bind::shading::{Fed, Feed, Stage};
use or_shader_front::rdna_bind::{self, Native};

/// The two stages of a model's shading pass, the mesh program lowered to a vertex stage, linked:
/// the vertex stage writes the locations the pixel stage reads, numbered from nought, and no other.
#[derive(Debug, Clone)]
pub struct Gta6ShadingPass {
    pub mesh: Stage,
    pub pixel: Stage,
    /// What crosses between the two, and where.
    pub linked: Linked,
}

fn native(container: &[u8]) -> Result<Native, String> {
    rdna_bind::native(container).map_err(|why| why.to_string())
}

/// Whether the inflated containers `first` and `pixel` are a mesh program and a pixel program
/// that writes the targets whose contents are established: the pass that shades a model.
pub fn is_gta6_shading_pass(first: &[u8], pixel: &[u8]) -> bool {
    let (Ok(first), Ok(pixel)) = (native(first), native(pixel)) else {
        return false;
    };
    first.program.kind == ShaderKind::Mesh
        && pixel.program.kind == ShaderKind::Pixel
        && deferred_targets(&picks(&pixel.program.words).exports)
}

/// The pass of the inflated containers `first` and `pixel` translated, with what fills each
/// binding; the tables are the programs' own, in that order, and `effect` is the first table of
/// the effect that runs them. `Err` says why the renderer cannot be handed it.
pub fn translate_gta6_shading(
    first: &[u8],
    pixel: &[u8],
    tables: [&Sgd5Table; 2],
    effect: &Sgd5Table,
) -> Result<Gta6ShadingPass, String> {
    let pixel = translate_gta6_pixel_stage(pixel, tables[1], effect)?;
    translate_gta6_mesh_stage(first, tables[0], effect, &pixel)
}

/// The pixel stage of a model's shading pass, which several passes of an effect share: the
/// stage, and the slots it reads as 16-bit halves. A caller keeps it for the next pass that
/// runs the same program.
#[derive(Debug, Clone)]
pub struct Gta6PixelStage {
    stage: Stage,
    half_slots: BTreeSet<u8>,
}

fn said(stage: &str, why: &dyn std::fmt::Display) -> String {
    format!("{stage} program: {why}")
}

/// The inflated container `pixel` translated as the pixel stage of a shading pass; `table` is
/// its own and `effect` the first table of the effect that runs it.
pub fn translate_gta6_pixel_stage(
    pixel: &[u8],
    table: &Sgd5Table,
    effect: &Sgd5Table,
) -> Result<Gta6PixelStage, String> {
    let pixel = native(pixel)?;
    let fragment =
        rdna_bind::kept::lower(&pixel, &BTreeSet::new()).map_err(|why| said("pixel", &why))?;
    Ok(Gta6PixelStage {
        half_slots: fragment.half_slots.clone(),
        stage: feed(&pixel, fragment, table, effect).map_err(|why| said("pixel", &why))?,
    })
}

/// The pass whose mesh program is the inflated container `first` and whose pixel stage is
/// `pixel`; `table` is the mesh program's own.
pub fn translate_gta6_mesh_stage(
    first: &[u8],
    table: &Sgd5Table,
    effect: &Sgd5Table,
    pixel: &Gta6PixelStage,
) -> Result<Gta6ShadingPass, String> {
    let first = native(first)?;
    // The mesh stage exports as halves what the pixel stage reads as halves.
    let slots = parameter_slots(&first.outputs);
    let halves: BTreeSet<u8> = (pixel.half_slots.iter())
        .filter_map(|slot| parameter_of(&slots, *slot))
        .collect();
    let geometry = rdna_bind::kept::lower(&first, &halves).map_err(|why| said("mesh", &why))?;
    let mut mesh = feed(&first, geometry, table, effect).map_err(|why| said("mesh", &why))?;
    let mut pixel = pixel.stage.clone();
    // The pixel stage is several passes': each pass links a copy of its own. The mesh stage's
    // parameters are at the pixel stage's slots already, and a clip distance crosses as a
    // location the pixel stage discards by, so that no device has to clip by distances.
    let (vertex, fragment) = (
        std::sync::Arc::make_mut(&mut mesh.module),
        std::sync::Arc::make_mut(&mut pixel.module),
    );
    let linked = link::link(vertex, fragment, &link::Options::default())
        .map_err(|why| said("pass", &why))?;
    Ok(Gta6ShadingPass {
        mesh,
        pixel,
        linked,
    })
}

/// `module` as HLSL, written as the Direct3D 12 backend has it written.
fn hlsl(module: &naga::Module) -> Result<String, String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .map_err(|e| format!("{e:?}"))?;
    let options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_1,
        fake_missing_bindings: true,
        ..Default::default()
    };
    let pipeline = naga::back::hlsl::PipelineOptions::default();
    let mut text = String::new();
    (naga::back::hlsl::Writer::new(&mut text, &options, &pipeline).write(module, &info, None))
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// `module` as the Metal shading language, written on a thread with a deep stack: the writer
/// recurses along a stage's expressions.
fn msl(module: &std::sync::Arc<naga::Module>) -> Result<String, String> {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    let module = std::sync::Arc::clone(module);
    let write = move || {
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .map_err(|e| format!("{e:?}"))?;
        let options = naga::back::msl::Options {
            lang_version: (3, 0),
            fake_missing_bindings: true,
            ..Default::default()
        };
        let pipeline = naga::back::msl::PipelineOptions::default();
        (naga::back::msl::write_string(&module, &info, &options, &pipeline))
            .map(|(text, _)| text)
            .map_err(|e| e.to_string())
    };
    (std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(write))
    .map_err(|e| e.to_string())?
    .join()
    .unwrap_or_else(|_| Err("the writer panicked".to_string()))
}

/// `rpf scan-shading`: one effect of the `SGD5` container at an offset, technique by technique:
/// the stages of its first pass and the targets its pixel program exports, then the pass that
/// shades a model translated, with what fills each binding.
pub(crate) fn cmd_scan_shading(args: &[String]) -> Result<(), String> {
    use or_io::container::rpf::read_rpf9_body;
    use or_io::gta5::shaders::sgd5::Sgd5File;
    use or_io::gta5::shaders::sgd5_pack::technique_programs;
    use or_io::gta6::hash::name_hash32;

    let (limit, mut args) = super::rpf::split_limit(args)?;
    let wgsl = match args.iter().position(|a| a == "--wgsl") {
        Some(at) if at + 1 < args.len() => {
            let folder = args.remove(at + 1);
            args.remove(at);
            Some(std::path::PathBuf::from(folder))
        }
        _ => None,
    };
    let [path, offset, wanted] = args.as_slice() else {
        return Err(
            "usage: rpf scan-shading <archive.rpf> <offset> <effect or 0xhash> [--wgsl <folder>] [--limit <bytes>]"
                .to_string(),
        );
    };
    let offset = super::rpf::parse_offset(offset)?;
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (_, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {offset:#X} does not read: {e}"))?;
    let hash = wanted
        .strip_prefix("0x")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok());
    let effect = (container.effects.iter())
        .find(|e| e.name == *wanted || Some(name_hash32(e.name.as_bytes())) == hash)
        .ok_or_else(|| format!("the container holds no effect {wanted}"))?;
    println!("  effect        {}", effect.name);
    for texture in &effect.parameters.resources {
        if texture.slot >= 256 {
            println!(
                "  parameter     code {:#04X} register {} name 0x{:08X} stated 0x{:08X}",
                texture.code, texture.slot, texture.name_hash, texture.semantic_hash
            );
        }
    }
    let mut shading: Vec<_> = Vec::new();
    for technique in &effect.techniques {
        let Some((first, pixel)) = technique_programs(&container, effect, &technique.name) else {
            println!("  {}: no pass of two programs", technique.name);
            continue;
        };
        let inflated = |program: &or_io::gta5::shaders::sgd5::Sgd5Program| {
            let stream = bytes.get(program.stream.clone())?;
            program.inflate(stream).ok()
        };
        let (Some(a), Some(b)) = (inflated(first), inflated(pixel)) else {
            println!("  {}: a program does not inflate", technique.name);
            continue;
        };
        let (kind, exports) = match (native(&a), native(&b)) {
            (Ok(a), Ok(b)) => (
                format!("{:?} + {:?}", a.program.kind, b.program.kind),
                format!("{:?}", picks(&b.program.words).exports),
            ),
            (a, b) => (
                format!("unread: {:?} {:?}", a.err(), b.err()),
                String::new(),
            ),
        };
        let is = is_gta6_shading_pass(&a, &b);
        println!(
            "  {}: {kind}, exports {exports}{}",
            technique.name,
            if is { ", shades a model" } else { "" }
        );
        if is
            && shading.iter().all(
                |(_, _, seen, _): &(_, _, &or_io::gta5::shaders::sgd5::Sgd5Program, _)| {
                    seen.at != first.at
                },
            )
        {
            shading.push((a, b, first, pixel));
        }
    }
    if shading.is_empty() {
        println!("  shading       no technique's first pass shades a model");
        return Ok(());
    }
    for (a, b, first, pixel) in shading {
        println!("  pass          {} + {}", first.name, pixel.name);
        if let (Ok(mesh), Ok(fragment)) = (native(&a), native(&b)) {
            let element = |e: &or_io::gta5::shaders::spirv_pack::SignatureElement| {
                format!(
                    "{}{}@{}/{}",
                    e.semantic, e.index, e.register, e.system_value
                )
            };
            let gives: Vec<String> = mesh.outputs.iter().map(element).collect();
            let takes: Vec<String> = fragment.inputs.iter().map(element).collect();
            let slots: Vec<(u8, u8)> = (fragment.program.inputs.iter())
                .map(|input| (input.slot, input.flags))
                .collect();
            println!("  mesh gives    {}", gives.join(" "));
            println!("  pixel takes   {}", takes.join(" "));
            println!("  pixel slots   {slots:?} (slot, flags)");
        }
        let started = std::time::Instant::now();
        let tables = [&first.variables, &pixel.variables];
        match translate_gta6_shading(&a, &b, tables, &effect.parameters) {
            Err(why) => {
                println!("  shading       not translated: {why}");
                // What each stage binds, for the binding the refusal is about.
                for (word, container, table) in [("mesh", &a, tables[0]), ("pixel", &b, tables[1])]
                {
                    let Ok(native) = native(container) else {
                        continue;
                    };
                    match rdna_bind::lower(&native, &BTreeSet::new()) {
                        Err(why) => println!("  {word}: not lowered: {why}"),
                        Ok(lowered) => {
                            let kind = native.program.kind;
                            for resource in &lowered.resources {
                                let at = rdna_bind::numbered(rdna_bind::place(resource), kind);
                                let join = rdna_bind::join(at, &native.records, table);
                                println!(
                                    "  {word}: binding {} {:?} count {:?} at {:?}: {:?}",
                                    resource.binding,
                                    resource.kind,
                                    resource.count,
                                    resource.at,
                                    std::mem::discriminant(&join),
                                );
                            }
                        }
                    }
                }
            }
            Ok(pass) => {
                println!("  shading       translated in {:?}", started.elapsed());
                let unfed = pass.linked.carried.iter().filter(|c| !c.fed).count();
                println!(
                    "  linked        {} locations, {} components, {unfed} the mesh stage does not write, clip at {:?}",
                    pass.linked.locations,
                    pass.linked.carried.len(),
                    pass.linked.clip
                );
                for (word, stage) in [("mesh", &pass.mesh), ("pixel", &pass.pixel)] {
                    println!(
                        "  {word}: textures {:X?}, streams {:?}",
                        stage.textures, stage.streams
                    );
                    if let Some(folder) = &wgsl {
                        let text = or_shader_front::rdna_bind::shading::wgsl(&stage.module)?;
                        let file =
                            folder.join(format!("{}-{}-{word}.wgsl", effect.name, first.name));
                        std::fs::write(&file, text)
                            .map_err(|e| format!("{}: {e}", file.display()))?;
                        // As Direct3D's compiler and Metal's are given it, for what they refuse.
                        let written =
                            [("hlsl", hlsl(&stage.module)), ("metal", msl(&stage.module))];
                        for (language, text) in written {
                            match text {
                                Ok(text) => {
                                    let _ = std::fs::write(file.with_extension(language), text);
                                }
                                Err(why) => println!("  {word}: not written as {language}: {why}"),
                            }
                        }
                    }
                    let reads = or_shader_front::rdna_bind::shading::reads(&stage.module);
                    for fed in &stage.feeds {
                        let said = match &fed.feed {
                            Feed::View(layout) | Feed::Material(layout) => format!(
                                "{} {} bytes, {} variables",
                                if matches!(fed.feed, Feed::View(_)) {
                                    "view"
                                } else {
                                    "material"
                                },
                                layout.size,
                                layout.variables.len()
                            ),
                            Feed::Zero(Some(hash)) => format!("noughts, the game's 0x{hash:08X}"),
                            other => format!("{other:?}"),
                        };
                        println!(
                            "    ({}, {}) {said}{}",
                            fed.group,
                            fed.binding,
                            fed.header
                                .map_or(String::new(), |h| format!(", header {h}"))
                        );
                        if let Some(reads) = reads.get(&(fed.group, fed.binding)) {
                            let reads: Vec<&str> = reads.iter().map(String::as_str).collect();
                            println!("      reads {}", reads.join(" "));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
