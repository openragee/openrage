//! A unit as Metal's shading language with no Metal device: naga's writer, the counts wgpu's
//! Metal backend holds a pipeline to, and a unit's buffers brought within them.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use naga::ShaderStage;
use or_renderer::game_shaders::pipeline::{entry_of, vertex_streams};
use or_shader_front::rdna_bind::budget::{
    self, Bound, Budget, Fitted, METAL_BUFFERS, METAL_VERTEX_BUFFERS,
};

use super::{groups, needs, Stages};
use crate::device_check::one_line;
use crate::rdna_report::{missing_feature, Adapter, Outcome};

/// The versions of Metal's shading language the text is written for, each with the first macOS
/// and iOS that take it: what wgpu asks naga for on a Metal device of that system. The second
/// is the oldest on which wgpu's Metal backend updates a texture in a step.
pub const METAL_VERSIONS: [((u8, u8), &str); 2] =
    [((3, 0), "macOS 13, iOS 16"), ((3, 1), "macOS 14, iOS 17")];

/// What Metal binds a stage, as wgpu's Metal backend states it of a Mac: buffers, textures and
/// samplers.
const METAL_STAGE: [(&str, u32); 3] = [
    ("buffers", METAL_BUFFERS),
    ("textures", 128),
    ("samplers", 16),
];

/// The vertex buffers a pipeline of the vertex stage `vertex` is created and drawn with: one
/// where the stage takes an input of a vertex, none where it takes none, as a mesh program
/// lowered to a vertex stage does.
pub(super) fn vertex_buffers(vertex: &naga::Module) -> u32 {
    let streams =
        entry_of(vertex, ShaderStage::Vertex).and_then(|entry| vertex_streams(vertex, entry, &[]));
    streams.map_or(0, |streams| u32::from(!streams.vertex.is_empty()))
}

fn stage_name(stage: wgpu::ShaderStages) -> &'static str {
    match stage {
        wgpu::ShaderStages::VERTEX => "vertex",
        wgpu::ShaderStages::FRAGMENT => "pixel",
        _ => "compute",
    }
}

/// Where each resource of a stage of `module` goes in Metal's tables, numbered as wgpu's Metal
/// backend numbers a pipeline layout: in the order of the groups and of the bindings in each, an
/// array as one buffer of arguments, then a buffer of sizes where the stage binds a buffer of
/// dwords, alone or in an array, or is a vertex stage. With them, the buffers the stage takes
/// of the table. `None` where the renderer's layouts do not take the module, which is then
/// written with made-up numbers.
///
/// # Errors
/// Where the stage binds more than [`METAL_STAGE`], which the backend refuses the layout for.
pub(super) fn metal_resources(
    module: &naga::Module,
    stage: wgpu::ShaderStages,
) -> Result<Option<(naga::back::msl::EntryPointResources, u32)>, String> {
    use naga::back::msl::{BindSamplerTarget, BindTarget};
    let Some((groups, _)) = groups(&[(module, stage)]) else {
        return Ok(None);
    };
    let mut resources = naga::back::msl::EntryPointResources::default();
    let (mut bound, mut sized) = ([0u32; 3], stage == wgpu::ShaderStages::VERTEX);
    let mut next = |kind: usize| {
        bound[kind] += 1;
        (bound[kind] - 1) as u8
    };
    for (group, entries) in &groups {
        let mut entries: Vec<_> = entries.iter().map(|(entry, _)| entry).collect();
        entries.sort_by_key(|entry| entry.binding);
        for entry in entries {
            let mut target = BindTarget::default();
            let storage = |ty: &wgpu::BindingType| match ty {
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    ..
                } => Some(*read_only),
                _ => None,
            };
            sized |= storage(&entry.ty).is_some();
            match entry.ty {
                _ if entry.count.is_some() => target.buffer = Some(next(0)),
                wgpu::BindingType::Buffer { .. } => {
                    target.buffer = Some(next(0));
                    target.mutable = storage(&entry.ty) == Some(false);
                }
                wgpu::BindingType::Sampler(_) => {
                    target.sampler = Some(BindSamplerTarget::Resource(next(2)));
                }
                wgpu::BindingType::StorageTexture { access, .. } => {
                    target.texture = Some(next(1));
                    target.mutable = access != wgpu::StorageTextureAccess::ReadOnly;
                }
                _ => target.texture = Some(next(1)),
            }
            let at = naga::ResourceBinding {
                group: *group,
                binding: entry.binding,
            };
            resources.resources.insert(at, target);
        }
    }
    if sized {
        resources.sizes_buffer = Some(next(0));
    }
    match (METAL_STAGE.iter().zip(bound)).find(|((_, most), bound)| bound > most) {
        Some(((what, most), bound)) => Err(format!(
            "Metal binds {most} {what} a stage, and the {} stage binds {bound}",
            stage_name(stage)
        )),
        None => Ok(Some((resources, bound[0]))),
    }
}

/// Why wgpu's Metal backend creates no pipeline of a vertex stage that takes `layout` buffers
/// of Metal's table and is drawn with `vertex_buffers` vertex buffers, where it does not.
// ⚠ The backend checks this of every pipeline with a vertex stage, one drawn with no vertex
// buffer too, after the layout's own count: `METAL_VERTEX_BUFFERS` says why it is not 31.
pub(super) fn vertex_stage_refused(layout: u32, vertex_buffers: u32) -> Option<String> {
    (layout + vertex_buffers > METAL_VERTEX_BUFFERS).then(|| {
        format!(
            "wgpu's Metal backend gives the vertex stage of a pipeline {METAL_VERTEX_BUFFERS} buffers, its vertex buffers among them, and the vertex stage binds {layout} and is drawn with {vertex_buffers}"
        )
    })
}

/// `module` as Metal's shading language of `version`, written with the checks a Metal device's
/// writer is given and with its resources at `resources`.
fn metal_source(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    version: (u8, u8),
    resources: Option<naga::back::msl::EntryPointResources>,
) -> Result<String, String> {
    use naga::back::msl;
    use naga::proc::{BoundsCheckPolicies, BoundsCheckPolicy};
    let entry = module.entry_points.first().map(|entry| entry.name.clone());
    let options = msl::Options {
        lang_version: version,
        fake_missing_bindings: resources.is_none(),
        per_entry_point_map: (entry.zip(resources).into_iter()).collect(),
        bounds_check_policies: BoundsCheckPolicies {
            index: BoundsCheckPolicy::Restrict,
            buffer: BoundsCheckPolicy::Restrict,
            image_load: BoundsCheckPolicy::Restrict,
            binding_array: BoundsCheckPolicy::Unchecked,
        },
        zero_initialize_workgroup_memory: true,
        force_loop_bounding: true,
        ..Default::default()
    };
    let (text, written) =
        msl::write_string(module, info, &options, &msl::PipelineOptions::default())
            .map_err(|e| one_line(&e.to_string()))?;
    // The writer gives the text and says apart which entry points it could not write.
    match written.entry_point_names.into_iter().find_map(Result::err) {
        Some(e) => Err(format!(
            "an entry point is not written: {}",
            one_line(&e.to_string())
        )),
        None => Ok(text),
    }
}

/// The stages of a pipeline, each with the stage it is and the word a kept file is named by.
fn each_stage(stages: &Stages) -> Vec<(&naga::Module, wgpu::ShaderStages, &'static str)> {
    use wgpu::ShaderStages as At;
    match stages {
        Stages::Compute(module) => vec![(module, At::COMPUTE, "compute")],
        Stages::Render(vertex, pixel) => std::iter::once((vertex, At::VERTEX, "vertex"))
            .chain((pixel.iter()).map(|pixel| (pixel, At::FRAGMENT, "pixel")))
            .collect(),
    }
}

/// Whether naga writes `stages` as Metal's shading language, validated for what its writer
/// supports and counted against what wgpu's Metal backend takes: each stage's resources when
/// it lays the pipeline out, then the vertex stage's buffers with the vertex buffers when it
/// creates the pipeline. All of Metal that a machine with no Metal device can check. A unit
/// that updates a texture in a step is skipped by the version of [`METAL_VERSIONS`] that does;
/// naga writes the same text for both. With `keep`, a folder and the unit's name, each stage's
/// text is saved there under the version it was written for, for a Mac's own compiler.
// ponytail: the text is not compiled here; only Metal's compiler says whether it builds.
pub(super) fn metal_text(stages: &Stages, keep: Option<(&Path, &str)>) -> Outcome {
    use naga::back::msl;
    use naga::valid::{ValidationFlags, Validator};
    let write = || -> Result<usize, Outcome> {
        let stages = each_stage(stages);
        let mut laid_out = Vec::new();
        for (module, stage, _) in &stages {
            let info = Validator::new(ValidationFlags::all(), msl::supported_capabilities())
                .validate(module)
                .map_err(|refusal| {
                    let said = format!("{:?}", refusal.into_inner());
                    match missing_feature(&said) {
                        Some(capability) => Outcome::Skipped(capability),
                        None => Outcome::Rejected(one_line(&said)),
                    }
                })?;
            let resources = metal_resources(module, *stage).map_err(Outcome::Rejected)?;
            laid_out.push((info, resources));
        }
        if let (Some((vertex, wgpu::ShaderStages::VERTEX, _)), Some((_, Some((_, layout))))) =
            (stages.first(), laid_out.first())
        {
            let refused = vertex_stage_refused(*layout, vertex_buffers(vertex));
            refused.map_or(Ok(()), |why| Err(Outcome::Rejected(why)))?;
        }
        let mut newest = 0;
        for ((module, _, tag), (info, resources)) in stages.iter().zip(laid_out) {
            let at = usize::from(needs(module).contains(wgpu::Features::TEXTURE_ATOMIC));
            let (version, _) = METAL_VERSIONS[at];
            let resources = resources.map(|(resources, _)| resources);
            let text =
                metal_source(module, &info, version, resources).map_err(Outcome::Rejected)?;
            if let Some((dir, name)) = keep {
                let dir = dir.join(format!("{}.{}", version.0, version.1));
                let file = dir.join(format!("{name}.{tag}.metal"));
                std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(&file, text))
                    .map_err(|e| Outcome::Rejected(format!("{}: {e}", file.display())))?;
            }
            newest = newest.max(at);
        }
        Ok(newest)
    };
    match catch_unwind(AssertUnwindSafe(write)) {
        Ok(Ok(0)) => Outcome::Created,
        Ok(Ok(at)) => {
            let ((major, minor), systems) = METAL_VERSIONS[at];
            Outcome::Skipped(format!("Metal's language {major}.{minor} ({systems})"))
        }
        Ok(Err(outcome)) => outcome,
        Err(_) => Outcome::Rejected("naga's MSL writer panicked".into()),
    }
}

/// The adapter a run of [`super::Ways::msl`] is counted under.
pub fn metal_text_adapter() -> Adapter {
    Adapter {
        name: "naga's MSL writer, no device".into(),
        backend: "Metal".into(),
        shader_compiler: "text".into(),
        ..Default::default()
    }
}

/// The adapter a run of [`super::Ways::fit`] is counted under: the same text of each unit
/// with its buffers brought within what wgpu's Metal backend takes.
pub fn metal_fitted_adapter() -> Adapter {
    Adapter {
        name: "naga's MSL writer, buffers fitted, no device".into(),
        ..metal_text_adapter()
    }
}

/// What wgpu's Metal backend gives the stage `stage` of a pipeline whose vertex stage is drawn
/// with `vertex_buffers` vertex buffers.
pub(super) fn metal_budget(stage: wgpu::ShaderStages, vertex_buffers: u32) -> Budget {
    Budget::metal(stage == wgpu::ShaderStages::VERTEX, vertex_buffers)
}

/// What a report notes of a stage the binder fed for a device with default limits, where it
/// did something to the stage's buffers: how many of the bindings the editor leaves at nought
/// became the stage's own noughts, and how many read another's buffer.
pub(super) fn fed_notes(stage: &str, fitted: &Fitted) -> Vec<String> {
    let said = |count: usize, what: &str| {
        (count > 0).then(|| format!("as fed, {stage} stage: {count} {what}"))
    };
    let own = said(
        fitted.own.len(),
        "constant buffers left at nought are its own noughts",
    );
    let shared = said(
        fitted.shared.len(),
        "buffers left at nought read another's buffer",
    );
    own.into_iter().chain(shared).collect()
}

/// What a report notes of the stage `module`, brought within `budget` as `fitted` says: the
/// buffers merged and into how many, and what it still binds where it is past the budget all
/// the same, with the bytes of its smallest and its largest constant buffer.
pub(super) fn fit_notes(
    stage: &str,
    fitted: &Fitted,
    budget: &Budget,
    module: &naga::Module,
) -> Vec<String> {
    let after = Bound::of(module);
    let mut notes = Vec::new();
    let noughts = fitted.own.len() + fitted.shared.len();
    if noughts > 0 {
        notes.push(format!(
            "fitted, {stage} stage: {noughts} buffers left at nought are its own noughts or share a buffer"
        ));
    }
    for listed in [false, true] {
        let merged = || {
            fitted
                .merged
                .iter()
                .filter(|m| m.listed.is_some() == listed)
        };
        let parts: usize = merged().map(|m| m.parts.len()).sum();
        if parts > 0 {
            let what = match listed {
                true => "buffers of dwords",
                false => "constant buffers",
            };
            notes.push(format!(
                "fitted, {stage} stage: {parts} {what} merged into {}",
                merged().count()
            ));
        }
    }
    if !budget.holds(after) {
        let bytes = (module.global_variables.iter())
            .filter(|(_, global)| global.binding.is_some())
            .filter(|(_, global)| global.space == naga::AddressSpace::Uniform)
            .map(|(_, global)| module.types[global.ty].inner.size(module.to_ctx()));
        let (least, most) = bytes.fold((u32::MAX, 0), |(least, most), bytes| {
            (least.min(bytes), most.max(bytes))
        });
        notes.push(format!(
            "fitted, {stage} stage: still past Metal's buffers, with {} constant buffers of {} to {most} bytes, {} buffers of dwords, {} bound arrays",
            after.uniform,
            least.min(most),
            after.storage,
            after.arrays
        ));
    }
    notes
}

/// `stages` with each stage's buffers brought within what wgpu's Metal backend takes, by
/// merging them, and what a report notes of it: nothing says which the editor leaves at nought.
pub(super) fn fitted(stages: &Stages) -> (Stages, Vec<String>) {
    let mut notes = Vec::new();
    let mut fit = |module: &naga::Module, stage: wgpu::ShaderStages, vertex_buffers: u32| {
        let mut module = module.clone();
        let budget = metal_budget(stage, vertex_buffers);
        let done = budget::within(&mut module, &[], &budget);
        notes.extend(fit_notes(stage_name(stage), &done, &budget, &module));
        module
    };
    let stages = match stages {
        Stages::Compute(module) => Stages::Compute(fit(module, wgpu::ShaderStages::COMPUTE, 0)),
        Stages::Render(vertex, pixel) => Stages::Render(
            fit(vertex, wgpu::ShaderStages::VERTEX, vertex_buffers(vertex)),
            (pixel.as_ref()).map(|pixel| fit(pixel, wgpu::ShaderStages::FRAGMENT, 0)),
        ),
    };
    (stages, notes)
}

/// What a report notes of each stage of `stages` that is past what wgpu's Metal backend takes
/// of buffers: how many it binds, as the backend counts them.
pub(super) fn past_notes(stages: &Stages) -> Vec<String> {
    let mut notes = Vec::new();
    for (module, stage, _) in each_stage(stages) {
        let vertex_buffers = match stage {
            wgpu::ShaderStages::VERTEX => vertex_buffers(module),
            _ => 0,
        };
        let bound = Bound::of(module);
        if !metal_budget(stage, vertex_buffers).holds(bound) {
            notes.push(format!(
                "past Metal's buffers, {} stage: {} buffers ({} constant buffers, {} buffers of dwords, {} bound arrays{})",
                stage_name(stage),
                bound.together(),
                bound.uniform,
                bound.storage,
                bound.arrays,
                if bound.sized { ", the buffer of sizes" } else { "" }
            ));
        }
    }
    notes
}
