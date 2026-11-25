//! What a file is, in a modder's words and units, read from its owned value:
//! the first thing the inspector shows of it.

use or_io::codec::mesh::VertexSource;
use or_io::decl::{Gen8, Gen9};
use or_io::gta5::{
    CLightAttr, DrawableTarget, GrmShaderGroup, GtaFragType, PhBoundAny, RmcDrawable,
};

use super::*;

/// A summary's rows: a label, and a value in words and units.
pub(super) type Facts = Vec<(String, String)>;

const DETAIL: [&str; 4] = [
    "High detail",
    "Medium detail",
    "Low detail",
    "Very low detail",
];

/// The heading, name and edition of `doc`, what its owned value holds, then `extra`.
pub(super) fn draw_summary(ui: &mut egui::Ui, doc: &crate::state::Document, extra: Facts) {
    ui.heading(doc.kind.info().noun);
    ui.separator();
    let mut facts = vec![("Name".to_string(), doc.file_name())];
    facts.extend(edition(doc).map(|e| ("Edition".to_string(), e)));
    if let DocPayload::Resource(asset) = &doc.payload {
        facts.extend(asset_facts(asset));
    }
    facts.extend(extra);
    draw_facts_grid(ui, "summary", &facts);
}

/// `facts` in the body font, which fits a sentence in the panel's width where a monospace one does not.
pub(super) fn draw_facts_grid(ui: &mut egui::Ui, id: &str, facts: &Facts) {
    grid(ui, id, |ui| {
        for (label, value) in facts {
            if !filter_matches(ui.ctx(), label) {
                continue;
            }
            ui.label(
                egui::RichText::new(label)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            let shown = ui
                .add(egui::Label::new(value.as_str()).sense(egui::Sense::click()))
                .on_hover_text("Click to copy.");
            if shown.clicked() {
                ui.ctx().copy_text(value.clone());
            }
            ui.end_row();
        }
    });
}

/// `Legacy, version 165`: the edition a resource's own version names, and the file's own version once a save would write another.
pub(super) fn edition(doc: &crate::state::Document) -> Option<String> {
    // A GTA VI body is of neither GTA V edition, whatever its resource's version reads as.
    if crate::state::rpf9_scan::from_gta_vi(doc.path.as_deref()) {
        return None;
    }
    let version = doc.resource.as_ref()?.seal.version();
    let format = or_io::gta5::formats::format(doc.kind.extension())?;
    let target = format.target_of_version(version).ok()?;
    let edition = format!("{}, version {version}", target.edition_label());
    Some(match doc.opened_version() {
        Some(opened) if opened != version => {
            format!("{edition} when saved; the file is version {opened}")
        }
        _ => edition,
    })
}

fn asset_facts(asset: &or_io::Asset) -> Facts {
    use or_io::Asset;
    match asset {
        Asset::Drawable(d) => drawable_facts(&d.base, &d.lights, d.bound.as_deref()),
        Asset::DrawableGen9(d) => drawable_facts(&d.base, &d.lights, d.bound.as_deref()),
        Asset::Fragment(f) => fragment_facts(f),
        Asset::FragmentGen9(f) => fragment_facts(f),
        Asset::TextureDict(d) => {
            texture_facts(d.entries.iter().flatten().map(|t| t.backing_store.len()))
        }
        Asset::TextureDictGen9(d) => {
            texture_facts(d.entries.iter().flatten().map(|t| t.pixels.len()))
        }
        Asset::DrawableDictGen9(d) => vec![("Drawables".into(), d.entries.len().to_string())],
        Asset::Bounds(b) => {
            let header = crate::render::bound_meshes::header(b);
            let (min, max) = (
                header.bounding_box_min(),
                &header.bounding_box_max_xyz_margin_w,
            );
            vec![
                ("Collision".into(), bound_words(b)),
                (
                    "Dimensions".into(),
                    size(max.x - min[0], max.y - min[1], max.z - min[2]),
                ),
                ("Radius".into(), metres(header.radius_around_centroid)),
            ]
        }
        _ => Vec::new(),
    }
}

/// `4.8 × 9.55 × 0.38 m`.
fn size(x: f32, y: f32, z: f32) -> String {
    format!("{} × {} × {}", number(x), number(y), metres(z))
}

fn texture_facts(sizes: impl Iterator<Item = usize>) -> Facts {
    let (count, bytes) = sizes.fold((0, 0), |(n, b), size| (n + 1, b + size));
    vec![
        ("Textures".into(), count.to_string()),
        ("Memory".into(), or_job::human_bytes(bytes as u64)),
    ]
}

/// What a target's shader is called, and the textures it samples.
pub(super) trait Shaded: DrawableTarget + Sized {
    fn effect(shader: &Self::InstanceData) -> u32;
    fn textures(group: &GrmShaderGroup<Self>, shader: &Self::InstanceData) -> Vec<String>;
}

impl Shaded for Gen8 {
    fn effect(shader: &or_io::gta5::gen8::GrcInstanceData) -> u32 {
        shader.basis_hash.get()
    }

    fn textures(
        group: &GrmShaderGroup<Gen8>,
        shader: &or_io::gta5::gen8::GrcInstanceData,
    ) -> Vec<String> {
        (shader.textures().filter_map(|slot| group.slot_name(slot)))
            .map(str::to_string)
            .collect()
    }
}

impl Shaded for Gen9 {
    fn effect(shader: &or_io::gta5::gen9::GrcInstanceDataG9) -> u32 {
        shader.name.get()
    }

    fn textures(
        group: &GrmShaderGroup<Gen9>,
        shader: &or_io::gta5::gen9::GrcInstanceDataG9,
    ) -> Vec<String> {
        (shader.textures().filter_map(|slot| group.slot_name(slot)))
            .map(str::to_string)
            .collect()
    }
}

/// A drawable's levels of detail, shaders and their textures, size, rig, lights and collision.
pub(super) fn drawable_facts<T: Shaded>(
    drawable: &RmcDrawable<T>,
    lights: &[CLightAttr],
    bound: Option<&PhBoundAny>,
) -> Facts {
    let lods = &drawable.lod_group;
    let mut facts: Facts = (lods.lods.iter().zip(lods.lod_distances))
        .zip(DETAIL)
        .filter(|((lod, _), _)| !lod.models.is_empty())
        .map(|((lod, distance), detail)| {
            let vertices: usize = (lod.models.iter())
                .flat_map(|m| m.geometries.iter())
                .map(|g| g.vertex_buffer.vertex_declared())
                .sum();
            let models = match lod.models.len() {
                1 => String::new(),
                n => format!("{n} models, "),
            };
            (
                detail.to_string(),
                format!("{models}{vertices} vertices, to {}", metres(distance)),
            )
        })
        .collect();

    let group = drawable.shader_group.as_deref();
    let shaders = group.map_or(&[][..], |g| &g.shaders[..]);
    facts.push(("Shaders".into(), counted(shaders.len())));
    // A shader, then each texture it samples on a row of its own.
    for shader in shaders {
        let mut label = format!("  {}", shader_name(T::effect(shader)));
        let textures = group.map(|g| T::textures(g, shader)).unwrap_or_default();
        if textures.is_empty() {
            facts.push((label, "no textures".into()));
            continue;
        }
        for texture in textures {
            facts.push((std::mem::take(&mut label), texture));
        }
    }

    let (min, max) = (&lods.bounds_box.min, &lods.bounds_box.max);
    facts.push((
        "Dimensions".into(),
        size(max.x - min.x, max.y - min.y, max.z - min.z),
    ));
    facts.push((
        "Radius".into(),
        metres(lods.bounds_sphere.center_and_radius.w),
    ));
    let bones = drawable.skeleton.as_deref().map_or(0, |s| s.bones.len());
    facts.push(("Skeleton".into(), counted_as(bones, "bone")));
    facts.push(("Lights".into(), counted(lights.len())));
    facts.push(("Collision".into(), bound.map_or("none".into(), bound_words)));
    let embedded = (group.and_then(|g| g.texture_dictionary.as_deref()))
        .map_or(0, |d| d.entries.iter().flatten().count());
    facts.push(("Embedded textures".into(), counted(embedded)));
    facts
}

/// A fragment's drawable, then its other drawables, physics parts, glass and cloth.
fn fragment_facts<T: Shaded>(fragment: &GtaFragType<T>) -> Facts {
    let mut facts = match fragment.common_drawable.as_deref() {
        Some(d) => drawable_facts(&d.base, &fragment.lights, d.bound.as_deref()),
        None => vec![("Drawable".into(), "none".into())],
    };
    facts.push((
        "Extra drawables".into(),
        counted(fragment.extra_drawables.len()),
    ));
    facts.push(("Physics parts".into(), counted(fragment.child_count())));
    facts.push((
        "Glass windows".into(),
        counted(fragment.glass_pane_model_infos.len()),
    ));
    facts.push(("Cloth".into(), counted(fragment.env_cloth.len())));
    facts
}

/// `composite, 3 parts`: a bound's type and what it is made of.
pub(super) fn bound_words(bound: &PhBoundAny) -> String {
    let kind = crate::ui::panels::hierarchy::bound_type_name(bound);
    match bound {
        PhBoundAny::PhBoundComposite(c) => format!("{kind}, {}", plural(c.bounds.len(), "part")),
        _ => match crate::ui::panels::hierarchy::bound_geometry(bound) {
            Some(g) => format!("{kind}, {}", plural(g.polygons.len(), "polygon")),
            None => kind,
        },
    }
}

/// `none`, or the number.
fn counted(n: usize) -> String {
    if n == 0 {
        "none".into()
    } else {
        n.to_string()
    }
}

/// `none`, or `n` of `noun`.
fn counted_as(n: usize, noun: &str) -> String {
    if n == 0 {
        "none".into()
    } else {
        plural(n, noun)
    }
}

/// `1 model`, `2 models`.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// A length to two decimals at most, with no trailing zero.
fn number(v: f32) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub(super) fn metres(v: f32) -> String {
    format!("{} m", number(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_reads_in_metres_without_trailing_zeros() {
        assert_eq!(metres(9998.0), "9998 m");
        assert_eq!(metres(1.25), "1.25 m");
        assert_eq!(metres(0.3), "0.3 m");
        assert_eq!(plural(1, "model"), "1 model");
        assert_eq!(plural(3, "model"), "3 models");
    }
}
