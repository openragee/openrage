//! A GTA VI model's shading pass for the renderer: the mesh and the pixel program of the pass
//! translated as one, each with what fills its bindings.

use std::collections::BTreeSet;

use or_io::gta5::shaders::sgd5::Sgd5Table;
use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna::link::{self, Linked};
use or_shader_front::rdna_bind::picks::{deferred_targets, picks};
use or_shader_front::rdna_bind::shading::{feed, parameter_of, parameter_slots, sources, Source};

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

/// The expressions of `module` that load a texel of a texture.
fn texel_loads(module: &naga::Module) -> usize {
    let functions = (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    functions
        .flat_map(|function| function.expressions.iter())
        .filter(|(_, expression)| matches!(expression, naga::Expression::ImageLoad { .. }))
        .count()
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
        // What only the game fills of each stage, by the hash of its name.
        for (word, container, table) in [
            ("mesh", &a, &first.variables),
            ("pixel", &b, &pixel.variables),
        ] {
            let Ok(native) = native(container) else {
                continue;
            };
            let mesh = native.program.kind == ShaderKind::Mesh;
            let filled = sources(&native.records, mesh, table, &effect.parameters);
            let game: BTreeSet<String> = (filled.iter())
                .filter_map(|source| match source {
                    Source::Game(hash) => Some(format!("{hash:08X}")),
                    _ => None,
                })
                .collect();
            let game: Vec<&str> = game.iter().map(String::as_str).collect();
            println!("    {word} game {}", game.join(" "));
        }
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
                    let globals = stage.module.global_variables.iter();
                    let buffers = globals.filter(|(_, global)| {
                        global.space == naga::AddressSpace::Uniform && global.binding.is_some()
                    });
                    println!(
                        "  {word}: binds {} constant buffers, loads texels at {} places, departs from the hardware by {:?}",
                        buffers.count(),
                        texel_loads(&stage.module),
                        stage.approximations
                    );
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

/// The pass of `passes` that draws a geometry whose vertex data is `vertex_data`: the one whose
/// mesh program reads the most streams among those reading none the data's table of streams
/// leaves empty.
pub fn drawing_pass<'p, P: std::borrow::Borrow<Gta6ShadingPass>>(
    passes: &'p [P],
    vertex_data: &[u8],
) -> Option<&'p P> {
    let held = |slot: &u32| {
        let at = 4 * *slot as usize;
        (vertex_data.get(at..at + 4)).is_some_and(|word| word != [0; 4])
    };
    // The first of those that read as many: `max_by_key` keeps the last.
    (passes.iter().rev())
        .filter(|pass| pass.borrow().mesh.streams.iter().all(held))
        .max_by_key(|pass| pass.borrow().mesh.streams.len())
}

/// The workers a census runs side by side.
const WORKERS: usize = 8;

/// `each` over `items` on [`WORKERS`] threads, the results in the items' order.
fn side_by_side<T: Sync, R: Send>(items: &[T], each: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..WORKERS.min(items.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(at) else {
                            return mine;
                        };
                        mine.push((at, each(item)));
                    }
                })
            })
            .collect();
        (workers.into_iter())
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    out.sort_by_key(|(at, _)| *at);
    out.into_iter().map(|(_, result)| result).collect()
}

/// Whether an effect's own passes draw a model, as the editor decides it.
enum Verdict {
    Draws(Vec<Gta6ShadingPass>),
    /// The first refusal of a pass that writes the deferred targets.
    Refused(String),
    /// No technique's first pass writes them: what its first mesh pass writes instead.
    NoDeferredPass(String),
}

/// One drawable of a census: the effect and the vertex data's table of streams of each geometry
/// of its first level that reads.
type Named = Vec<(u32, Vec<u8>)>;

/// `rpf scan-effect-use`: the drawables of an archive, or each n-th, by the effects their
/// geometries name, against what each effect's own passes are to the editor.
pub(crate) fn cmd_scan_effect_use(args: &[String]) -> Result<(), String> {
    use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Role, Rpf9Scan};
    use or_io::gta5::shaders::sgd5::Sgd5File;
    use or_io::gta5::shaders::sgd5_pack::first_passes;
    use or_io::gta6::hash::name_hash32;
    use or_io::gta6::headerless::{read, Drawable, Entry, Resource};
    use std::collections::BTreeMap;

    let (limit, args) = super::rpf::split_limit(args)?;
    let (from, args) = super::rpf::split_offset_flag(&args, "--from")?;
    let (to, args) = super::rpf::split_offset_flag(&args, "--to")?;
    let (every, args) = super::rpf::split_offset_flag(&args, "--every")?;
    let [path, effects, offset] = args.as_slice() else {
        return Err(
            "usage: rpf scan-effect-use <archive.rpf> <shaders.rpf> <offset> [--from <offset>] [--to <offset>] [--every <n>] [--limit <bytes>]"
                .to_string(),
        );
    };
    let every = every.unwrap_or(1).max(1);
    let offset = super::rpf::parse_offset(offset)?;
    let file = std::fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let cancel = super::jobs::on_interrupt();
    let mut rows = Vec::new();
    let mut seen = 0u64;
    // ⚠ The scan panics on some bytes of a large archive: it is taken up again past them, by
    // steps that double, and what was passed over is said.
    let (mut start, end) = (from.unwrap_or(0), to.unwrap_or(u64::MAX));
    let (mut step, mut passed) = (0u64, 0u64);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let scanned = 'scan: loop {
        let mut scan = match Rpf9Scan::new(&file, start..end) {
            Ok(scan) => scan,
            Err(why) => break Err(why.to_string()),
        };
        loop {
            if cancel.is_set() {
                break 'scan Err("cancelled".to_string());
            }
            let next = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scan.next()));
            let find = match next {
                Ok(None) => break 'scan Ok(()),
                Ok(Some(Err(why))) => break 'scan Err(why.to_string()),
                Ok(Some(Ok(find))) => find,
                Err(_) => {
                    step = (2 * step).max(16);
                    passed += step;
                    start += step;
                    continue 'scan;
                }
            };
            step = 0;
            start = find.offset + find.span.max(16);
            let Rpf9BodyKind::Zlib(run) = find.kind else {
                continue;
            };
            let (Rpf9Content::ResourceShaped, Some(resource), Some(signature)) =
                (run.content, run.resource, run.signature)
            else {
                continue;
            };
            let model = matches!(
                signature.role(),
                Some(Rpf9Role::Drawable | Rpf9Role::DrawableWithBound | Rpf9Role::Dictionary)
            );
            if model {
                seen += 1;
                if (seen - 1).is_multiple_of(every) {
                    rows.push((find.offset, resource, signature));
                }
            }
        }
    };
    std::panic::set_hook(hook);
    scanned?;
    if passed > 0 {
        println!("  scan          {passed} bytes passed over where the scan panicked");
    }
    let named = |drawable: &Drawable| -> Named {
        let level = drawable.levels.iter().flatten().find(|l| !l.is_empty());
        (level.into_iter().flatten())
            .flat_map(|model| model.geometries.iter().flatten())
            .filter(|geometry| geometry.positions.is_ok())
            .filter_map(|geometry| {
                let shader = drawable.shaders.get(usize::from(geometry.shader))?;
                let table = geometry.vertex_data.get(..4 * 23).unwrap_or_default();
                Some((shader.name_hash, table.to_vec()))
            })
            .collect()
    };
    let opened = side_by_side(&rows, |(at, resource, signature)| -> Vec<(u64, Named)> {
        if cancel.is_set() {
            return Vec::new();
        }
        let Ok((_, bytes)) = read_rpf9_body(&file, *at, limit) else {
            return Vec::new();
        };
        let (virtual_section, physical) = resource.sections(&bytes);
        match read(*signature, virtual_section, Some(physical)) {
            Ok(Resource::Drawable(d)) => vec![(*at, named(&d))],
            Ok(Resource::DrawableWithBound(r)) => vec![(*at, named(&r.drawable))],
            Ok(Resource::Dictionary(dict)) => (dict.entries.iter())
                .filter_map(|(_, entry)| match entry {
                    Ok(Entry::Drawable(d)) => Some((*at, named(d))),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    });
    let drawables: Vec<(u64, Named)> = (opened.into_iter().flatten())
        .filter(|(_, named)| !named.is_empty())
        .collect();
    println!(
        "  census        {} drawables with a geometry that reads, of {} of {seen} model bodies (every {every})",
        drawables.len(),
        rows.len()
    );

    let source = std::fs::File::open(effects).map_err(|e| format!("open({effects}): {e}"))?;
    let (_, bytes) = read_rpf9_body(&source, offset, limit).map_err(|e| e.to_string())?;
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {offset:#X} does not read: {e}"))?;
    let mut used: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
    for (_, named) in &drawables {
        let mut effects: Vec<u32> = named.iter().map(|(effect, _)| *effect).collect();
        for effect in &effects {
            used.entry(*effect).or_default().1 += 1;
        }
        effects.sort_unstable();
        effects.dedup();
        for effect in effects {
            used.entry(effect).or_default().0 += 1;
        }
    }
    let hashes: Vec<u32> = used.keys().copied().collect();
    let verdicts = side_by_side(&hashes, |hash| {
        let Some(effect) =
            (container.effects.iter()).find(|e| name_hash32(e.name.as_bytes()) == *hash)
        else {
            return (
                format!("0x{hash:08X}"),
                Verdict::Refused("the container holds no such effect".to_string()),
            );
        };
        let inflated = |program: &or_io::gta5::shaders::sgd5::Sgd5Program| {
            program.inflate(bytes.get(program.stream.clone())?).ok()
        };
        let (mut passes, mut refused, mut other) = (Vec::new(), None, None);
        let mut seen = BTreeSet::new();
        for (first, pixel) in first_passes(&container, effect) {
            if cancel.is_set() || !seen.insert(first.at) {
                continue;
            }
            let (Some(a), Some(b)) = (inflated(first), inflated(pixel)) else {
                continue;
            };
            if is_gta6_shading_pass(&a, &b) {
                let tables = [&first.variables, &pixel.variables];
                match translate_gta6_shading(&a, &b, tables, &effect.parameters) {
                    Ok(pass) => passes.push(pass),
                    Err(why) => refused = refused.or(Some(why)),
                }
            } else if let (Ok(a), Ok(b)) = (native(&a), native(&b)) {
                if a.program.kind == ShaderKind::Mesh && other.is_none() {
                    let mut targets: Vec<(u8, u8)> = picks(&b.program.words).exports;
                    targets.dedup();
                    other = Some(format!(
                        "its mesh pass {} + {} writes (target, mask) {targets:?}",
                        first.name, pixel.name
                    ));
                }
            }
        }
        let verdict = match (passes.is_empty(), refused, other) {
            (false, ..) => Verdict::Draws(passes),
            (true, Some(why), _) => Verdict::Refused(why),
            (true, None, Some(other)) => Verdict::NoDeferredPass(other),
            (true, None, None) => Verdict::NoDeferredPass("it has no mesh pass".to_string()),
        };
        (effect.name.clone(), verdict)
    });
    let verdicts: BTreeMap<u32, (String, Verdict)> = hashes.iter().copied().zip(verdicts).collect();
    let draws = |effect: u32, table: &[u8]| match verdicts.get(&effect) {
        Some((_, Verdict::Draws(passes))) => drawing_pass(passes, table).is_some(),
        _ => false,
    };
    let mut drawn: BTreeMap<u32, u64> = BTreeMap::new();
    let (mut whole, mut partly, mut geometries, mut all) = (0u64, 0u64, 0u64, 0u64);
    let mut first_of: BTreeMap<Vec<u32>, u64> = BTreeMap::new();
    for (at, named) in &drawables {
        let mut count = 0;
        for (effect, table) in named {
            if draws(*effect, table) {
                count += 1;
                *drawn.entry(*effect).or_default() += 1;
            }
        }
        all += named.len() as u64;
        geometries += count as u64;
        whole += u64::from(count == named.len());
        partly += u64::from(count > 0 && count < named.len());
        let mut effects: Vec<u32> = named.iter().map(|(effect, _)| *effect).collect();
        effects.sort_unstable();
        effects.dedup();
        first_of.entry(effects).or_insert(*at);
    }
    println!(
        "  bound         {whole} of {} drawables draw every geometry by its effect's own passes, {partly} more draw some; {geometries} of {all} geometries",
        drawables.len()
    );
    let mut ranked: Vec<(u32, (u64, u64))> = used.into_iter().collect();
    ranked.sort_by_key(|(_, (drawables, _))| std::cmp::Reverse(*drawables));
    let mut reasons: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (hash, (drawables, geometries)) in &ranked {
        let (name, verdict) = &verdicts[hash];
        let said = match verdict {
            Verdict::Draws(passes) => format!("draws by {} passes", passes.len()),
            Verdict::Refused(why) => format!("refused: {why}"),
            Verdict::NoDeferredPass(why) => format!("no deferred pass: {why}"),
        };
        let reason = match verdict {
            Verdict::Draws(_) => None,
            Verdict::Refused(why) => Some(format!("refused: {why}")),
            Verdict::NoDeferredPass(_) => Some("no deferred pass".to_string()),
        };
        if let Some(reason) = reason {
            let reason = reasons.entry(reason).or_default();
            reason.0 += 1;
            reason.1 += geometries;
        }
        // A drawable that names this effect and no other, else the first that names it.
        let sample = (first_of.iter())
            .find(|(effects, _)| effects.as_slice() == [*hash])
            .or_else(|| first_of.iter().find(|(effects, _)| effects.contains(hash)))
            .map_or(String::new(), |(_, at)| format!(", at {at:#X}"));
        println!(
            "  {name}: {drawables} drawables, {} of {geometries} geometries drawn, {said}{sample}",
            drawn.get(hash).copied().unwrap_or(0)
        );
    }
    println!("  reasons       by effects and geometries");
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by_key(|(_, (_, geometries))| std::cmp::Reverse(*geometries));
    for (reason, (effects, geometries)) in reasons {
        println!("  {effects} effects, {geometries} geometries: {reason}");
    }
    Ok(())
}
