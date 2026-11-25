//! The facts GTA VI's headerless resource reader reads from a scanned body, as the text the body
//! opens to and as a drawable's inspector shows them, and a navigation mesh as the model the `.ynv`
//! view draws.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::{Rpf9Resource, Rpf9Role, Rpf9Signature};
use or_io::gta5::shaders::sgd5::{
    drawable_parameter, holds_floats, holds_integers, parameter_names, shader_definitions, Sgd5File,
};
use or_io::gta5::shaders::shader_defs::{self, ShaderDef, ShaderDefinitions};
use or_io::gta5::{
    AiSplitArray, CAdjacentNavmeshLookups, CNavMesh, CNavMeshCompressedVertex, CNavMeshQuadTree,
    CSpecialLinkInfo, SubArray, TNavMeshPoly, TNavMeshQuadTreeLeafData, TShortMinMax, Vector3,
};
use or_io::gta6::headerless::{
    read, read_navmesh, Bound, ChildBound, DictionaryTexture, Drawable, DrawableWithBound, Entry,
    HeaderlessError, NavNode, ParameterValue, Primitive, Resource, Shader, UnknownShape,
};

/// The role's name, or that it is not known.
pub(crate) fn role_name(role: Option<Rpf9Role>) -> &'static str {
    match role {
        Some(Rpf9Role::Bound) => tr!("bound"),
        Some(Rpf9Role::Drawable) => tr!("drawable"),
        Some(Rpf9Role::Dictionary) => tr!("dictionary"),
        Some(Rpf9Role::DrawableWithBound) => tr!("fragment"),
        Some(Rpf9Role::NavMesh) => tr!("navigation mesh"),
        Some(Rpf9Role::NavOctree) => tr!("navigation octree"),
        Some(Rpf9Role::AccelerationStructure) => tr!("acceleration structure"),
        Some(Rpf9Role::Cloth) => tr!("cloth"),
        Some(Rpf9Role::MorphTarget) => tr!("morph target"),
        Some(Rpf9Role::VertexBuffers) => tr!("vertex buffers"),
        Some(Rpf9Role::KeyedBlob) => tr!("keyed blob"),
        Some(Rpf9Role::KeyedBlobDescriptor) => tr!("keyed-blob descriptor"),
        Some(Rpf9Role::AssetIndex) => tr!("asset index"),
        Some(Rpf9Role::NeighbourTable) => tr!("neighbour table"),
        Some(Rpf9Role::ParticleEffects) => tr!("particle effects"),
        None => tr!("not known"),
    }
}

/// What the reader reads from `bytes`, a read of `resource`: its virtual, then its physical section.
pub(super) fn facts(
    resource: &Rpf9Resource,
    signature: Rpf9Signature,
    bytes: &[u8],
) -> Result<String, String> {
    let mut lines = vec![match signature.role() {
        Some(role) => tr!(
            "A headerless GTA VI resource of signature {signature}, whose role, inferred from its layout, is {role}.",
            signature = signature,
            role = role_name(Some(role)),
        ),
        None => tr!(
            "A headerless GTA VI resource of signature {signature}, whose role is not known.",
            signature = signature,
        ),
    }];
    let (virtual_section, physical) = resource.sections(bytes);
    lines.push(tr!(
        "Its block map counts {virtual_blocks} virtual and {physical_blocks} physical blocks, which hold {virtual_size} and {physical_size}.",
        virtual_blocks = resource.virtual_blocks,
        physical_blocks = resource.physical_blocks,
        virtual_size = or_job::human_bytes(virtual_section.len() as u64),
        physical_size = or_job::human_bytes(physical.len() as u64),
    ));
    let resource = read(signature, virtual_section, Some(physical)).map_err(|e| {
        tr!(
            "GTA VI's headerless resource reader does not read this body: {e}",
            e = e
        )
    })?;
    match resource {
        Resource::Bound(b) => bound_lines(&b, false, &mut lines),
        Resource::Drawable(d) => drawable_lines(&d, true, &mut lines),
        Resource::Dictionary(dict) => {
            let drawables = (dict.entries.iter())
                .filter(|(_, e)| matches!(e, Ok(Entry::Drawable(_))))
                .count();
            lines.push(tr!(
                "A dictionary of {entries} entries, {drawables} of them drawables.",
                entries = dict.entries.len(),
                drawables = drawables,
            ));
            for (code, entry) in &dict.entries {
                match entry {
                    Ok(Entry::Drawable(d)) => drawable_lines(d, true, &mut lines),
                    entry => lines.extend(entry_line(*code, entry)),
                }
            }
        }
        Resource::DrawableWithBound(r) => fragment_lines(&r, true, &mut lines),
        Resource::Instances(set) => {
            let structures = set.instances.iter().filter_map(|i| i.structure.as_ref());
            lines.push(tr!(
                "{instances} ray-tracing instances of key {key}, whose {structures} serialized acceleration structures hold {primitives} primitives in all.",
                instances = set.instances.len(),
                key = format!("{:016X}", set.key),
                structures = structures.clone().count(),
                primitives = structures.map(|s| u64::from(s.primitives)).sum::<u64>(),
            ));
            lines.push(tr!("Each structure's nodes are the console driver's, so no geometry is read from them.").to_string());
        }
        Resource::MorphTarget(m) => super::gta6_morph::morph_lines(&m, &mut lines),
        Resource::NavMesh(m) => {
            lines.push(tr!(
                "A navigation mesh of cell {cell}: {vertices} vertices, {polygons} polygons and {sides} side records, in a quadtree of {nodes} nodes.",
                cell = m.cell,
                vertices = m.vertices.len(),
                polygons = m.polygons_proper,
                sides = m.polygons.len().saturating_sub(m.polygons_proper),
                nodes = m.nodes,
            ));
            lines.extend(super::gta6_navmesh::cell_lines(&m));
        }
        Resource::NavOctree(t) => lines.push(tr!(
            "A navigation octree of cell {cell}: {nodes} nodes, {internal} of them internal, and {placeholder} placeholder, {open} open and {surface} surface leaves.",
            cell = t.cell,
            nodes = t.nodes,
            internal = t.internal,
            placeholder = t.placeholder,
            open = t.open,
            surface = t.surface,
        )),
        Resource::Cloth(c) => super::gta6_fragment::cloth_lines(&c, &mut lines),
        Resource::VertexBuffers(set) => lines.push(tr!(
            "{buffers} vertex buffers of {vertices} vertices in all, with no index buffer: which drawable's they are is not known.",
            buffers = set.buffers.len(),
            vertices = set.buffers.iter().map(|(n, _)| u64::from(*n)).sum::<u64>(),
        )),
        Resource::KeyedBlob(b) => lines.push(tr!(
            "A blob of {size} under the key {key}. What the key hashes and what the bytes hold are not known.",
            size = or_job::human_bytes(b.len),
            key = format!("{:016X}", b.key),
        )),
        Resource::KeyedBlobDescriptor(d) => super::gta6_small::descriptor_lines(&d, &mut lines),
        Resource::AssetIndex(a) => super::gta6_small::index_lines(&a, &mut lines),
        Resource::NeighbourTable(n) => super::gta6_small::neighbour_lines(&n, &mut lines),
        Resource::ParticleEffects(list) => lines.extend(super::gta6_particles::lines(&list)),
    }
    Ok(lines.join("\n"))
}

/// What a dictionary's entry other than a drawable is; `None` for a drawable.
fn entry_line(code: u32, entry: &Result<Entry, HeaderlessError>) -> Option<String> {
    let code = format!("{code:#010X}");
    Some(match entry {
        Ok(Entry::Drawable(_)) => return None,
        Ok(Entry::Texture(t)) => texture_line(t),
        Ok(Entry::FrameFilter { name, tracks }) => filter_lines(name, tracks),
        Ok(Entry::AnimationRecord { name, bones }) => record_lines(name, bones),
        Ok(Entry::Word64(word)) => tr!(
            "Entry {code} holds the 64-bit word {word}, whose role is not known.",
            code = code,
            word = format!("{word:016X}"),
        ),
        Err(e) => tr!("Entry {code} is not read: {e}", code = code, e = e),
    })
}

/// A frame filter's name, then a line per track: what it names, its weight, its group and its word at 0x2C.
fn filter_lines(name: &str, tracks: &[or_io::gta6::headerless::FilterTrack]) -> String {
    use or_io::gta6::anim::track_label;
    let mut lines = vec![tr_n!(
        "A frame filter named {name}, of {count} track:",
        "A frame filter named {name}, of {count} tracks:",
        tracks.len(),
        name = name
    )];
    lines.extend(tracks.iter().map(|t| {
        tr!(
            "Track {track}: weight {weight}, group {group}, word at 0x2C {word}.",
            track = track_label(t.track),
            weight = t.weight,
            group = track_label(t.group),
            word = format!("{:#010X}", t.word_2c),
        )
    }));
    lines.join("\n")
}

/// An animation record's name, then a line per bone: its name hash, its kind, its flags and its floats.
fn record_lines(name: &str, bones: &[or_io::gta6::headerless::BoneRecord]) -> String {
    let mut lines = vec![tr_n!(
        "An animation record named {name}, with settings for {count} skeleton bone.",
        "An animation record named {name}, with settings for {count} skeleton bones.",
        bones.len(),
        name = name
    )];
    lines.extend(bones.iter().map(|b| {
        let values: Vec<String> = b.values.iter().map(f32::to_string).collect();
        tr!(
            "Bone {bone}: kind {kind}, flags {flags}, values {values}.",
            bone = super::gta6_names::said32(b.bone),
            kind = b.kind,
            flags = format!("{:#010X}", b.flags),
            values = values.join(", "),
        )
    }));
    lines.join("\n")
}

fn fragment_lines(r: &DrawableWithBound, shaders: bool, lines: &mut Vec<String>) {
    lines.push(tr!("Its name is {name}.", name = r.name));
    drawable_lines(&r.drawable, shaders, lines);
    match &r.bound {
        Some(bound) => bound_lines(bound, false, lines),
        None => lines.push(tr!("It has no bound: its record at 0x10 is null.").to_string()),
    }
    super::gta6_fragment::fragment_lines(&r.fragment, lines);
}

/// The drawable a scanned body shows: its own, its fragment's, or the dictionary entry `selection` names.
pub(crate) fn drawable_of(resource: &Resource, selection: Option<u64>) -> Option<&Drawable> {
    use crate::ui::panels::selection_ids::{dict_entry_unpack_part, is_dict_entry_selection};
    match resource {
        Resource::Drawable(d) => Some(d),
        Resource::DrawableWithBound(r) => Some(&r.drawable),
        Resource::Dictionary(dict) => {
            let code = dict_entry_unpack_part(selection.filter(|s| is_dict_entry_selection(*s))?).0;
            (dict.entries.iter()).find_map(|(c, e)| match e {
                Ok(Entry::Drawable(d)) if *c == code => Some(&**d),
                _ => None,
            })
        }
        _ => None,
    }
}

/// Each drawable of a scanned dictionary, with the code that keys it, walked and not collected.
pub(crate) fn dictionary_drawables(
    resource: &Resource,
) -> impl Iterator<Item = (u32, &Drawable)> + '_ {
    let entries = match resource {
        Resource::Dictionary(dict) => dict.entries.as_slice(),
        _ => &[],
    };
    entries.iter().filter_map(|(code, e)| match e {
        Ok(Entry::Drawable(d)) => Some((*code, &**d)),
        _ => None,
    })
}

/// What the inspector says of a scanned body the model view draws, and of the drawable `selection`
/// shows, whose shaders it lists as [`material_rows`].
pub(crate) fn body_lines(resource: &Resource, selection: Option<u64>) -> Vec<String> {
    let mut lines = Vec::new();
    match resource {
        Resource::Bound(b) => bound_lines(b, true, &mut lines),
        Resource::DrawableWithBound(r) => fragment_lines(r, false, &mut lines),
        Resource::Dictionary(dict) => {
            for (code, entry) in &dict.entries {
                lines.extend(entry_line(*code, entry));
            }
            match drawable_of(resource, selection) {
                Some(d) => drawable_lines(d, false, &mut lines),
                None => lines.push(
                    tr!("No drawable is chosen: click one of the list to draw it.").to_string(),
                ),
            }
        }
        Resource::Drawable(d) => drawable_lines(d, false, &mut lines),
        Resource::Cloth(c) => super::gta6_fragment::cloth_lines(c, &mut lines),
        _ => {}
    }
    lines
}

fn texture_line(t: &DictionaryTexture) -> String {
    let what = format!(
        "{}\u{00d7}{}\u{00d7}{}, DXGI {}",
        t.width, t.height, t.depth, t.format
    );
    match &t.texture {
        Ok(_) => tr!(
            "A texture named {name}, {what}, of {mips} mips, which reads.",
            name = t.name,
            what = what,
            mips = t.mip_levels,
        ),
        Err(e) => tr!(
            "A texture named {name}, {what}, of {mips} mips, which does not read: {why}",
            name = t.name,
            what = what,
            mips = t.mip_levels,
            why = e,
        ),
    }
}

/// A scanned navigation mesh as the model the `.ynv` view draws, the cell kept for the inspector
/// under `path`; `None` where it does not read or fit.
pub(super) fn as_ynv(
    path: &std::path::Path,
    resource: &Rpf9Resource,
    signature: Rpf9Signature,
    bytes: &[u8],
) -> Option<CNavMesh> {
    if signature.role() != Some(Rpf9Role::NavMesh) {
        return None;
    }
    let m = read_navmesh(resource.sections(bytes).0).ok()?;
    let mesh = cell_view(&m)?;
    super::gta6_navmesh::remember(path, m);
    Some(mesh)
}

/// A cell as the model the `.ynv` view draws; `None` where it does not fit.
pub(super) fn cell_view(m: &or_io::gta6::headerless::NavMesh) -> Option<CNavMesh> {
    let vertices = (m.vertices.iter()).map(|q| CNavMeshCompressedVertex {
        data: *q,
        ..Default::default()
    });
    // ⚠ GTA V's edge names a polygon in fifteen bits: one past them leads nowhere in the view.
    let edges = (0..m.edges.len()).map(|i| super::gta6_navmesh_edit::view_edge(m, i));
    let mut runs = super::gta6_navmesh::link_runs(m);
    let mut polygons = Vec::with_capacity(m.polygons.len());
    for (p, run) in m.polygons.iter().zip(&mut runs) {
        let mut polygon = TNavMeshPoly {
            min_max: min_max(p.min_max),
            ..Default::default()
        };
        polygon.set_num_vertices(usize::from(p.corners)).ok()?;
        // Bits 0 and 7 are GTA V's small and water bits.
        polygon.set_as_integer1(p.flags);
        // ponytail: GTA V counts a polygon's links in three bits; the inspector lists them all.
        run.truncate(7);
        polygon.set_num_special_links(run.len()).ok()?;
        polygons.push(polygon);
    }
    let mut cells = [0; 32];
    cells[0] = m.cell;
    // Positions are quantised in the same box as the vertices; the heading has no field here.
    let links = (m.special_links.iter()).map(|l| CSpecialLinkInfo {
        link_from_pos_x: l.from[0],
        link_from_pos_y: l.from[1],
        link_from_pos_z: l.from[2],
        link_to_pos_x: l.to[0],
        link_to_pos_y: l.to[1],
        link_to_pos_z: l.to[2],
        original_link_from_poly: l.from_polygon,
        link_from_poly: l.from_polygon,
        original_link_to_poly: l.to_polygon,
        link_to_poly: l.to_polygon,
        ..Default::default()
    });
    let mesh = CNavMesh {
        index_of_mesh: m.cell,
        adjacent_meshes: CAdjacentNavmeshLookups {
            navmesh_indices: cells,
            ..Default::default()
        },
        compressed_vertex_array: Some(split(vertices.collect(), 6)),
        vertex_index_array: Some(split(m.indices.clone(), 2)),
        adjacent_polys_array: Some(split(edges.collect::<Result<_, _>>().ok()?, 8)),
        polys_array: Some(split(polygons, 48)),
        quad_tree: Some(Box::new(quad_tree(&m.tree))),
        special_links: links.collect(),
        special_links_indices: runs.concat(),
        ..Default::default()
    };
    Some(mesh)
}

/// `items` in sub-arrays of 16 KiB of `stride`-byte items, as a split array holds them.
fn split<T: or_io::decl::Owned + Clone>(items: Vec<T>, stride: usize) -> Box<AiSplitArray<T>>
where
    or_io::decl::Raw<T>: or_io::decl::Plain + or_io::decl::Element + or_io::decl::Param,
{
    let sub_arrays = (items.chunks(16384 / stride))
        .map(|chunk| SubArray {
            elements: chunk.to_vec(),
            ..Default::default()
        })
        .collect();
    Box::new(AiSplitArray {
        sub_arrays,
        ..Default::default()
    })
}

fn min_max([min_x, max_x, min_y, max_y, min_z, max_z]: [i16; 6]) -> TShortMinMax {
    TShortMinMax {
        min_x,
        max_x,
        min_y,
        max_y,
        min_z,
        max_z,
        ..Default::default()
    }
}

fn quad_tree(node: &NavNode) -> CNavMeshQuadTree {
    let vector = |[x, y, z]: [f32; 3]| Vector3 {
        x,
        y,
        z,
        ..Default::default()
    };
    let leaf = |polygons: &Vec<u16>| TNavMeshQuadTreeLeafData {
        polys: polygons.clone(),
        ..Default::default()
    };
    CNavMeshQuadTree {
        mins: vector(node.box_min),
        maxs: vector(node.box_max),
        leaf_data: node.polygons.as_ref().map(|p| Box::new(leaf(p))),
        children: (node.children.each_ref()).map(|c| c.as_deref().map(|c| Box::new(quad_tree(c)))),
        ..Default::default()
    }
}

/// A primitive child's shape; `None` for a geometry.
pub(crate) fn shape_name(shape: &ChildBound) -> Option<&'static str> {
    Some(match shape {
        ChildBound::Geometry(_) => return None,
        ChildBound::Sphere { .. } => tr!("sphere"),
        ChildBound::Capsule { .. } => tr!("capsule"),
        ChildBound::Box { .. } => tr!("box"),
        ChildBound::TaperedCapsule { .. } => tr!("tapered capsule"),
        ChildBound::Disc { .. } => tr!("disc"),
        ChildBound::Cylinder { .. } => tr!("cylinder"),
        ChildBound::Hull { .. } => tr!("hull of eight corners"),
        ChildBound::Frustum { .. } => tr!("frustum of two radii"),
    })
}

/// A bound's counts, its primitive children's dimensions and what is not read; where it is
/// `drawn`, also each geometry's primitives the view does not draw.
fn bound_lines(b: &Bound, drawn: bool, lines: &mut Vec<String>) {
    let (mut vertices, mut triangles, mut others, mut shapes) = (0, 0, 0, 0);
    let mut refused = Vec::new();
    let mut not_drawn = Vec::new();
    for (i, child) in b.children.iter().enumerate() {
        let g = match &child.bound {
            Some(Ok(ChildBound::Geometry(g))) => g,
            Some(Ok(shape)) => {
                shapes += 1;
                if let (Some(name), Some(([x, y, z], radius))) = (shape_name(shape), shape.extent())
                {
                    refused.push(tr!(
                        "Child {child} is a {shape}: half extents {x}, {y} and {z} m about its centre, radius {radius} m.",
                        child = i,
                        shape = name,
                        x = format!("{x:.4}"),
                        y = format!("{y:.4}"),
                        z = format!("{z:.4}"),
                        radius = format!("{radius:.4}"),
                    ));
                }
                continue;
            }
            Some(Err(e)) => {
                refused.push(tr!("Child {child}: {why}", child = i, why = e));
                continue;
            }
            None => continue,
        };
        vertices += g.vertices.len();
        let shapes = g.primitives.iter();
        let round = shapes.filter(|p| !matches!(p, Primitive::Triangle { .. }));
        let count = round.count();
        triangles += g.primitives.len() - count;
        others += count;
        if !g.unknown_shapes.is_empty() {
            let ends = (g.unknown_shapes.iter())
                .filter(|(_, why)| *why == UnknownShape::EndsOnOneVertex)
                .count();
            not_drawn.push(tr!(
                "Child {child}: {ends} cylinders name one vertex as both ends, so they state no axis and are drawn as a point marker; the BVH leaf of {off} cylinders disagrees with the box their fields give, and they are drawn from their ends.",
                child = i,
                ends = ends,
                off = g.unknown_shapes.len() - ends,
            ));
        }
    }
    if drawn {
        refused.extend(not_drawn);
    }
    lines.push(tr!(
        "A bound of {children} children: {vertices} vertices, {triangles} triangles and {others} spheres, capsules, boxes and cylinders.",
        children = b.children.len(),
        vertices = vertices,
        triangles = triangles,
        others = others,
    ));
    if shapes > 0 {
        lines.push(tr_n!(
            "{count} child is a primitive bound, drawn from its fields.",
            "{count} children are primitive bounds, drawn from their fields.",
            shapes
        ));
    }
    lines.extend(refused);
}

/// GTA VI's effects from each `SGD5` container the app has read, which name a drawable's shaders.
// ponytail: one database for the whole app, which no view empties.
static EFFECTS: Mutex<Option<ShaderDefinitions>> = Mutex::new(None);
/// How many times [`EFFECTS`] has learned a container's effects.
static EFFECTS_LEARNED: AtomicU64 = AtomicU64::new(0);

/// Whether [`EFFECTS`] has been given what an earlier run kept.
static KEPT_READ: AtomicBool = AtomicBool::new(false);

/// Where the effects the app has read are kept across runs.
// ⚠ Named anew when a kept effect gains a field an older file lacks: `gta6_effects.json` predates
// each constant's drawable place, which an absent value cannot tell from a buffer the engine fills.
fn kept_effects_path() -> Option<std::path::PathBuf> {
    super::prefs::config_root().map(|root| root.join("gta6_effects_placed.json"))
}

fn effects() -> MutexGuard<'static, Option<ShaderDefinitions>> {
    let mut known = EFFECTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !KEPT_READ.swap(true, Ordering::Relaxed) && known.is_none() {
        *known = kept_effects_path().and_then(|path| kept_effects(&path));
    }
    known
}

/// The effects kept at `path`; `None` where there are none or they do not read.
fn kept_effects(path: &std::path::Path) -> Option<ShaderDefinitions> {
    let text = std::fs::read_to_string(path).ok()?;
    ShaderDefinitions::from_json(&text)
        .inspect_err(|e| tracing::warn!("{}: {e}", path.display()))
        .ok()
}

/// Keep effects, as `text` states them, at `path` for the next run.
fn keep_effects(path: &std::path::Path, text: serde_json::Result<String>) -> Result<(), String> {
    let text = text.map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether the app knows any GTA VI effect, read this run or kept from an earlier one.
pub(super) fn effects_known() -> bool {
    effects().as_ref().is_some_and(|e| !e.shaders.is_empty())
}

/// The effect a shader's name hash names among those the app has read, else `hash_` and the hash.
pub(crate) fn effect_name(hash: u32) -> String {
    known_effect(hash).unwrap_or_else(|| format!("hash_{hash:08X}"))
}

/// The effect a shader's name hash names among those the app has read.
pub(crate) fn known_effect(hash: u32) -> Option<String> {
    (effects().as_ref()).and_then(|e| e.name_material(hash, &[]).shader)
}

/// The effect whose name hashes to `hash`, among those the app has read.
pub(crate) fn effect_def(hash: u32) -> Option<ShaderDef> {
    (effects().as_ref()).and_then(|e| e.shader(hash).cloned())
}

/// Keep the effects of `bytes`, an `SGD5` container, for the drawables' shaders; how many it holds.
pub(super) fn learn_effects(bytes: &[u8]) -> Result<usize, String> {
    let file = Sgd5File::open(bytes).map_err(|e| e.to_string())?;
    let learned = shader_definitions("gta6", "gta6", std::slice::from_ref(&file));
    let count = learned.shaders.len();
    let fresh: HashSet<u32> = learned.shaders.iter().map(|s| s.name_hash).collect();
    let mut effects = effects();
    let known = effects.get_or_insert_with(|| ShaderDefinitions::new("gta6", "gta6"));
    known.shaders.retain(|s| !fresh.contains(&s.name_hash));
    known.shaders.extend(learned.shaders);
    EFFECTS_LEARNED.fetch_add(1, Ordering::Relaxed);
    let text = (!known.shaders.is_empty()).then(|| known.to_json());
    drop(effects);
    if let (Some(path), Some(text)) = (kept_effects_path(), text) {
        if let Err(why) = keep_effects(&path, text) {
            tracing::warn!("the GTA VI effects read are not kept: {why}");
        }
    }
    Ok(count)
}

/// A stamp of what a drawable's lines read beside it: the effects, GTA V's parameter names and the texture names learned, the packs indexed, and the documents open.
pub(crate) fn drawable_lines_stamp(state: &super::AppState) -> u64 {
    let mut hasher = DefaultHasher::new();
    EFFECTS_LEARNED.load(Ordering::Relaxed).hash(&mut hasher);
    shader_defs::is_installed().hash(&mut hasher);
    super::ttpf_document::names_known().hash(&mut hasher);
    state.texture_packs_indexed().hash(&mut hasher);
    for doc in &state.documents {
        (doc.id, doc.payload_epoch()).hash(&mut hasher);
    }
    hasher.finish()
}

/// The name a texture key is looked up and drawn by: the one the app has learned, else `hash_` and the key.
pub(crate) fn texture_name(key: u64) -> String {
    super::ttpf_document::known_name(key)
        .map_or_else(|| format!("hash_{key:016X}"), |n| n.to_ascii_lowercase())
}

/// GTA V's parameter names by hash, from the shader database installed; empty with none.
fn gta5_names() -> HashMap<u32, String> {
    shader_defs::with_installed(parameter_names).unwrap_or_default()
}

/// A parameter's name where GTA V's names hash to `hash`, else `hash_` and the hash.
fn parameter_name(names: &HashMap<u32, String>, hash: u32) -> String {
    names
        .get(&hash)
        .cloned()
        .unwrap_or_else(|| format!("hash_{hash:08X}"))
}

/// A shader's heading, then each parameter as the inspector lists it: its name, what it holds, and
/// whether that is a warning.
pub(crate) type Material = (String, Vec<(String, String, bool)>);

/// Each shader of `d`, the drawable of `doc`, as GTA V's material inspector lists a material: its
/// effect, then a row per parameter, a texture with what its key resolves to as the model view
/// binds it, a constant with its words and place.
pub(crate) fn material_rows(
    state: &super::AppState,
    doc: super::DocumentId,
    d: &Drawable,
) -> Vec<Material> {
    let names = gta5_names();
    let effects = effects();
    let mut out = Vec::new();
    for (i, s) in d.shaders.iter().enumerate() {
        let effect = (effects.as_ref()).and_then(|e| e.shader(s.name_hash));
        let hash = format!("{:08X}", s.name_hash);
        let heading = match effect {
            Some(effect) => tr!(
                "Shader {index}: the effect {effect}, hash_{hash}",
                index = i,
                effect = effect.name,
                hash = hash,
            ),
            None => tr!(
                "Shader {index}: hash_{hash}, named only by its hash",
                index = i,
                hash = hash,
            ),
        };
        let mut rows = Vec::new();
        for (slot, key) in s.texture_keys.iter().enumerate() {
            let parameter = (s.parameters.iter()).find(|p| {
                matches!(p.value, ParameterValue::Texture { slot: at } if usize::from(at) == slot)
            });
            let name =
                parameter.map_or_else(|| "-".to_string(), |p| parameter_name(&names, p.hash));
            let (detail, warn) = texture_detail(state, doc, slot, *key);
            rows.push((name, detail, warn));
        }
        for p in &s.parameters {
            let (detail, warn) = match &p.value {
                ParameterValue::Texture { .. } => continue,
                ParameterValue::Constant {
                    buffer,
                    offset,
                    words,
                } => {
                    let (values, undeclared) = constant_values(words, effect, p.hash);
                    let at = tr!(
                        "buffer {buffer} +{offset}",
                        buffer = buffer,
                        offset = format!("{offset:#x}"),
                    );
                    match undeclared {
                        true => (
                            tr!(
                                "{values} · {at} · its effect does not declare it",
                                values = values,
                                at = at,
                            ),
                            true,
                        ),
                        false => (format!("{values} · {at}"), false),
                    }
                }
            };
            rows.push((parameter_name(&names, p.hash), detail, warn));
        }
        out.push((heading, rows));
    }
    out
}

/// Texture slot `slot` of `doc`'s drawable bound to `key`: the name the key is known by, and where
/// it resolves, or that none holds it. A key no pack of its install holds is no warning: nothing
/// the reader does would load it.
fn texture_detail(
    state: &super::AppState,
    doc: super::DocumentId,
    slot: usize,
    key: Option<u64>,
) -> (String, bool) {
    let Some(key) = key else {
        return (tr!("texture slot {slot} · unbound", slot = slot), false);
    };
    let known = texture_name(key);
    let found = super::texture_resolution::probe(state, None, None, &known).found;
    let name = match known.starts_with("hash_") {
        true => known.clone(),
        false => format!("{known} ({key:016X})"),
    };
    match (found, state.held_by_no_pack(doc, &known)) {
        (Some(origin), _) => (
            tr!(
                "texture slot {slot} · {name}, from {source}",
                slot = slot,
                name = name,
                source = origin.describe(),
            ),
            false,
        ),
        (None, Some(folder)) => (
            tr!(
                "texture slot {slot} · {name} is in no texture pack under {folder}: there is no texture to load for this key.",
                slot = slot,
                name = name,
                folder = folder.display(),
            ),
            false,
        ),
        (None, None) => (
            tr!(
                "texture slot {slot} · {name} is unresolved: no open texture pack and no dictionary the app has read holds it.",
                slot = slot,
                name = name,
            ),
            true,
        ),
    }
}

/// A constant's words, as floats or unsigned integers where its effect declares them so and in
/// hex elsewhere, beside the effect's default where it sets one; and whether a known effect leaves
/// it undeclared.
fn constant_values(words: &[u32], effect: Option<&ShaderDef>, hash: u32) -> (String, bool) {
    use super::gta6_constant_types as types;
    use or_io::gta5::shaders::shader_defs::ShaderParamValue;
    let def = effect.map(|e| drawable_parameter(e, hash));
    let declared = def.flatten();
    if let Some(stale) = declared.and_then(|d| types::stale(words, d)) {
        return (stale, false);
    }
    // Type 0x03 holds a table of 16 entries, which reads entry by entry.
    let remap = declared.is_some_and(types::is_remap);
    let integers = declared.is_some_and(|d| holds_integers(d) && !types::is_remap(d));
    let floats = declared.is_some_and(holds_floats);
    let shown = |words: &[u32]| {
        if let Some(entries) = types::remap_shown(words).filter(|_| remap) {
            return entries;
        }
        (words.iter())
            .map(|w| match (floats, integers) {
                (true, _) => f32::from_bits(*w).to_string(),
                (_, true) => w.to_string(),
                _ => format!("{w:#010X}"),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let default = declared.and_then(|d| d.default.as_ref()).map(|d| match d {
        ShaderParamValue::Floats(f) => f.iter().map(|f| f.to_bits()).collect(),
        ShaderParamValue::Uints(w) => w.clone(),
    });
    let values = match default {
        Some(default) if default == words => {
            tr!("{values}, the effect's default", values = shown(words))
        }
        Some(default) => tr!(
            "{values}, where the effect's default is {default}",
            values = shown(words),
            default = shown(&default),
        ),
        None => shown(words),
    };
    (values, matches!(def, Some(None)))
}

/// Each parameter of each of `shaders`: its name, and whether its effect declares it as floats.
pub(crate) fn parameter_rows(shaders: &[Shader]) -> Vec<Vec<(String, bool)>> {
    let (names, effects) = (gta5_names(), effects());
    let row = |s: &Shader, hash| {
        let effect = (effects.as_ref()).and_then(|e| e.shader(s.name_hash));
        let def = effect.and_then(|e| drawable_parameter(e, hash));
        (parameter_name(&names, hash), def.is_some_and(holds_floats))
    };
    (shaders.iter())
        .map(|s| s.parameters.iter().map(|p| row(s, p.hash)).collect())
        .collect()
}

/// Each constant of `s`, shader `index`, with its buffer, offset and words, as floats where its
/// effect declares floats; and whether the effect declares it, where the effect is known.
fn constant_lines(
    index: usize,
    s: &Shader,
    effect: Option<&ShaderDef>,
    names: &HashMap<u32, String>,
    lines: &mut Vec<String>,
) {
    for p in &s.parameters {
        let ParameterValue::Constant {
            buffer,
            offset,
            words,
        } = &p.value
        else {
            continue;
        };
        let (values, undeclared) = constant_values(words, effect, p.hash);
        let parameter = parameter_name(names, p.hash);
        lines.push(match undeclared {
            true => tr!(
                "Shader {shader}, constant {parameter}: buffer {buffer}, offset {offset}, {values}; its effect does not declare it.",
                shader = index,
                parameter = parameter,
                buffer = buffer,
                offset = offset,
                values = values,
            ),
            false => tr!(
                "Shader {shader}, constant {parameter}: buffer {buffer}, offset {offset}, {values}.",
                shader = index,
                parameter = parameter,
                buffer = buffer,
                offset = offset,
                values = values,
            ),
        });
    }
}

/// A drawable's counts, its shaders' lines where `shaders`, and what is not read.
fn drawable_lines(d: &Drawable, shaders: bool, lines: &mut Vec<String>) {
    let (mut geometries, mut triangles, mut positions) = (0, 0, 0);
    let mut refused = Vec::new();
    for (l, models) in d.levels.iter().enumerate() {
        for (m, model) in models.iter().flatten().enumerate() {
            for (g, geometry) in model.geometries.iter().enumerate() {
                geometries += 1;
                let why = match geometry {
                    Ok(geometry) => {
                        triangles += geometry.triangles.len();
                        match &geometry.positions {
                            Ok(_) => {
                                positions += 1;
                                continue;
                            }
                            Err(e) => e.to_string(),
                        }
                    }
                    Err(e) => e.to_string(),
                };
                refused.push(tr!(
                    "Level {level}, model {model}, geometry {geometry}: {why}",
                    level = l,
                    model = m,
                    geometry = g,
                    why = why,
                ));
            }
        }
    }
    let keys: usize = d
        .shaders
        .iter()
        .map(|s| s.texture_keys.iter().flatten().count())
        .sum();
    lines.push(tr!(
        "A drawable named {name}: {shaders} shaders with {keys} texture keys, {geometries} geometries, {triangles} triangles, and positions read in {positions} of the geometries.",
        name = d.name,
        shaders = d.shaders.len(),
        keys = keys,
        geometries = geometries,
        triangles = triangles,
        positions = positions,
    ));
    let effects = effects();
    let gta5 = gta5_names();
    let listed = if shaders { d.shaders.as_slice() } else { &[] };
    for (i, s) in listed.iter().enumerate() {
        let effect = (effects.as_ref()).and_then(|e| e.shader(s.name_hash));
        match effect {
            None => lines.push(tr!(
                "Shader {index} is named only by its hash, hash_{hash}, and binds {keys} texture keys.",
                index = i,
                hash = format!("{:08X}", s.name_hash),
                keys = s.texture_keys.iter().flatten().count(),
            )),
            Some(effect) => {
                let named = (s.parameters.iter())
                    .filter(|p| drawable_parameter(effect, p.hash).is_some())
                    .count();
                let names: Vec<String> = (s.parameters.iter())
                    .map(|p| parameter_name(&gta5, p.hash))
                    .collect();
                lines.push(tr!(
                    "Shader {index} is the effect {effect}, hash_{hash}, and binds {keys} texture keys. The effect declares {named} of its {count} parameters: {names}.",
                    index = i,
                    effect = effect.name,
                    hash = format!("{:08X}", s.name_hash),
                    keys = s.texture_keys.iter().flatten().count(),
                    named = named,
                    count = s.parameters.len(),
                    names = names.join(", "),
                ));
            }
        }
        constant_lines(i, s, effect, &gta5, lines);
    }
    drop(effects);
    lines.extend(
        d.embedded
            .as_deref()
            .map(super::gta6_fragment::surfaces_line),
    );
    lines.extend(refused);
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::shaders::shader_defs::{ShaderDef, ShaderParamDef, ShaderParamKind};
    use or_io::gta6::hash::name_hash32;
    use or_io::gta6::headerless::{BoundChild, BoundGeometry, Dictionary, Parameter};

    fn drawable_facts(d: &Drawable) -> Vec<String> {
        let mut lines = Vec::new();
        drawable_lines(d, true, &mut lines);
        lines
    }

    #[test]
    fn a_frame_filter_lists_each_track_by_name_and_an_animation_record_each_bone() {
        use or_io::gta6::headerless::{BoneRecord, FilterTrack};
        let hash = |name: &str| name_hash32(name.as_bytes());
        let tracks = [
            FilterTrack {
                track: hash("moverDeltaRotation"),
                weight: 0.25,
                group: hash("boneRotation"),
                word_2c: 7,
            },
            FilterTrack {
                track: 0x0BAD_F00D,
                weight: 1.0,
                group: hash("behavior"),
                word_2c: 0,
            },
        ];
        let filter = entry_line(
            1,
            &Ok(Entry::FrameFilter {
                name: "pack:/upper.iff".into(),
                tracks: tracks.to_vec(),
            }),
        );
        let lines: Vec<String> = filter
            .expect("a line")
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "A frame filter named pack:/upper.iff, of 2 tracks:",
                "Track moverDeltaRotation: weight 0.25, group boneRotation, word at 0x2C 0x00000007.",
                "Track hash_0BADF00D: weight 1, group behavior, word at 0x2C 0x00000000.",
            ]
        );
        let bones = [BoneRecord {
            bone: 0x1234_5678,
            kind: 3,
            flags: 0x10,
            values: std::array::from_fn(|i| i as f32 * 0.5),
        }];
        let record = entry_line(
            2,
            &Ok(Entry::AnimationRecord {
                name: "rig_a".into(),
                bones: bones.to_vec(),
            }),
        )
        .expect("a line");
        let bone = record.lines().nth(1).expect("a bone line");
        assert!(
            bone.starts_with("Bone hash_12345678: kind 3, flags 0x00000010, values 0, 0.5, 1,"),
            "{bone}"
        );
        assert!(bone.ends_with(", 13."), "{bone}");
    }

    fn drawable(name: &str) -> Drawable {
        Drawable {
            name: name.to_string(),
            box_min: [0.0; 3],
            box_max: [1.0; 3],
            shaders: Vec::new(),
            levels: [None, None, None, None],
            lod_distances: [9998.0; 4],
            bound: None,
            embedded: None,
            extra_skeletons: Vec::new(),
        }
    }

    #[test]
    fn a_drawn_bound_states_each_primitive_child_and_names_the_primitives_it_does_not_draw() {
        let geometry = BoundGeometry {
            box_min: [0.0; 3],
            box_max: [1.0; 3],
            vertices: vec![[0.0; 3]; 3],
            primitives: vec![
                Primitive::Triangle {
                    vertices: [0, 1, 2],
                    neighbours: [0xFFFF; 3],
                    edges: [0xFF; 3],
                },
                Primitive::Capsule {
                    ends: [0, 1],
                    radius: 0.5,
                },
                Primitive::Cylinder {
                    ends: [1, 1],
                    radius: 0.5,
                },
            ],
            material_ids: Vec::new(),
            primitive_materials: Vec::new(),
            bvh: None,
            unknown_shapes: vec![(2, UnknownShape::EndsOnOneVertex)],
        };
        let child = |bound| BoundChild {
            matrix: [[0.0; 4]; 4],
            flags: None,
            bound: Some(Ok(bound)),
            material_id: None,
        };
        let bound = Bound {
            radius: 1.0,
            box_min: [0.0; 3],
            box_max: [1.0; 3],
            centroid: [0.0; 3],
            children: vec![
                child(ChildBound::Geometry(geometry)),
                child(ChildBound::Sphere {
                    centre: [0.0; 3],
                    radius: 1.0,
                }),
            ],
            bvh: None,
        };
        let lines = body_lines(&Resource::Bound(bound), None);
        let text = lines.join("\n");
        assert!(
            text.contains("Child 0: 1 cylinders name one vertex as both ends, so they state no axis and are drawn as a point marker; the BVH leaf of 0 cylinders disagrees"),
            "{text}"
        );
        assert!(text.contains("1 triangles and 2 spheres"), "{text}");
        assert!(
            text.contains(
                "Child 1 is a sphere: half extents 1.0000, 1.0000 and 1.0000 m about its centre, radius 1.0000 m."
            ),
            "{text}"
        );
    }

    #[test]
    fn a_dictionary_s_lines_follow_the_drawable_chosen_and_say_when_none_is() {
        use crate::ui::panels::selection_ids::dict_entry_id;
        let code = name_hash32(b"prop_chosen");
        let dict = Resource::Dictionary(Dictionary {
            entries: vec![
                (
                    code,
                    Ok(Entry::Drawable(Box::new(drawable("prop_chosen.#dr")))),
                ),
                (code + 1, Ok(Entry::Word64(7))),
            ],
        });
        let none = body_lines(&dict, None).join("\n");
        assert!(none.contains("No drawable is chosen"), "{none}");
        assert!(none.contains("64-bit word"), "{none}");
        assert!(!none.contains("prop_chosen"), "{none}");
        let chosen = body_lines(&dict, Some(dict_entry_id(code))).join("\n");
        assert!(
            chosen.contains("A drawable named prop_chosen.#dr"),
            "{chosen}"
        );
        assert!(drawable_of(&dict, Some(dict_entry_id(code + 1))).is_none());
        assert_eq!(dictionary_drawables(&dict).count(), 1);
    }

    fn param(name_hash: u32, semantic_hash: Option<u32>, kind: ShaderParamKind) -> ShaderParamDef {
        ShaderParamDef {
            name: format!("hash_{name_hash:08X}"),
            name_hash,
            kind,
            array_len: 1,
            register: 0,
            cbuffer: None,
            drawable_place: None,
            semantic: None,
            semantic_hash,
            ui_name: None,
            default: None,
            preset_default: None,
        }
    }

    #[test]
    fn a_learned_effect_names_a_drawable_s_shader_and_joins_its_parameters_by_0x14() {
        let (texture, glow, key) = (0x1111_0001, 0x1111_0002, 0x0123_4567_89AB_CDEFu64);
        let effect = "effect_of_a_test";
        let shader = Shader {
            name_hash: name_hash32(effect.as_bytes()),
            texture_keys: vec![Some(key), None],
            parameters: vec![
                Parameter {
                    hash: texture,
                    value: ParameterValue::Texture { slot: 0 },
                },
                Parameter {
                    hash: glow,
                    value: ParameterValue::Constant {
                        buffer: 1,
                        offset: 4,
                        words: vec![1.5f32.to_bits()],
                    },
                },
            ],
        };
        let drawable = Drawable {
            shaders: vec![shader.clone()],
            ..drawable("d")
        };
        assert!(!drawable_facts(&drawable).iter().any(|l| l.contains(effect)));

        // The effect states the constant's hash as another parameter's name hash, which joins
        // nothing: a drawable's record names the hash at +0x14.
        let mut defs = ShaderDefinitions::new("gta6", "gta6");
        defs.shaders.push(ShaderDef {
            name: effect.to_string(),
            name_hash: name_hash32(effect.as_bytes()),
            effect: effect.to_string(),
            draw_bucket: None,
            vertex_type: 0,
            traits: Vec::new(),
            parameters: vec![
                param(texture, None, ShaderParamKind::Texture),
                param(glow, Some(0x2222_0002), ShaderParamKind::Float),
            ],
        });
        let hash = name_hash32(effect.as_bytes());
        assert_eq!(effect_name(hash), format!("hash_{hash:08X}"));
        *effects() = Some(defs.clone());
        assert_eq!(effect_name(hash), effect, "an export names the effect too");
        let after = drawable_facts(&drawable);
        *effects() = None;
        let summary = (after.iter())
            .find(|l| l.contains(effect))
            .expect("the shader is named by its effect");
        assert!(summary.contains("declares 1 of its 2"), "{summary}");
        assert!(summary.contains(&format!("hash_{glow:08X}")), "{summary}");
        let constant = (after.iter())
            .find(|l| l.contains("constant"))
            .expect("a constant's line");
        assert!(
            constant.contains("buffer 1, offset 4, 0x3FC00000; its effect does not declare"),
            "{constant}"
        );

        // Joined by its +0x14 hash, the constant reads as the float its effect declares, by the
        // name GTA V's names give that hash.
        defs.shaders[0].parameters[1] = param(0x3333_0003, Some(glow), ShaderParamKind::Float);
        let names = HashMap::from([(glow, "GlowScale".to_string())]);
        let mut lines = Vec::new();
        constant_lines(0, &shader, defs.shader(hash), &names, &mut lines);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("GlowScale: buffer 1, offset 4, 1.5."),
            "{}",
            lines[0]
        );
        // Beside the effect's default, said to differ where it does; an integer type reads unsigned.
        use or_io::gta5::shaders::shader_defs::ShaderParamValue;
        let set = Some(ShaderParamValue::Floats(vec![1.5]));
        defs.shaders[0].parameters[1].default = set;
        let shown = |defs: &ShaderDefinitions| {
            let mut lines = Vec::new();
            constant_lines(0, &shader, defs.shader(hash), &names, &mut lines);
            lines.remove(0)
        };
        let line = shown(&defs);
        assert!(line.contains(", 1.5, the effect's default."), "{line}");
        let mut int = param(0x3333_0003, Some(glow), ShaderParamKind::Unknown(5));
        int.default = Some(ShaderParamValue::Uints(vec![7]));
        defs.shaders[0].parameters[1] = int;
        let line = shown(&defs);
        assert!(
            line.contains(", 1069547520, where the effect's default is 7."),
            "{line}"
        );

        // No pack is open and no name is learned for the key.
        let materials = material_rows(&crate::state::AppState::new(), 0, &drawable);
        let [(heading, rows)] = materials.as_slice() else {
            panic!("one material: {materials:?}");
        };
        assert!(heading.contains("named only by its hash"), "{heading}");
        let [(slot, bound, true), (_, empty, false), (constant, value, false)] = rows.as_slice()
        else {
            panic!("a texture row that warns, an empty slot's, then a constant row: {rows:?}");
        };
        assert_eq!(empty, "texture slot 1 · unbound");
        assert_eq!(slot, &format!("hash_{texture:08X}"));
        assert!(bound.starts_with("texture slot 0 · "), "{bound}");
        assert!(bound.contains(&format!("hash_{key:016X}")), "{bound}");
        assert!(bound.contains("unresolved"), "{bound}");
        assert_eq!(constant, &format!("hash_{glow:08X}"));
        assert_eq!(value, "0x3FC00000 · buffer 1 +0x4");
    }

    #[test]
    fn effects_kept_by_one_run_are_read_back_by_the_next_and_a_damaged_file_is_none() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("kept").join("gta6_effects.json");
        assert!(kept_effects(&path).is_none(), "nothing is kept yet");
        let mut defs = ShaderDefinitions::new("gta6", "gta6");
        defs.shaders.push(ShaderDef {
            name: "effect_kept".to_string(),
            name_hash: name_hash32(b"effect_kept"),
            effect: "effect_kept".to_string(),
            draw_bucket: None,
            vertex_type: 0,
            traits: Vec::new(),
            parameters: vec![param(0x1111_0001, None, ShaderParamKind::Texture)],
        });
        keep_effects(&path, defs.to_json()).expect("kept");
        let read = kept_effects(&path).expect("read back");
        assert_eq!(read.shaders.len(), 1);
        assert_eq!(read.shaders[0].name, "effect_kept");
        assert_eq!(read.shaders[0].parameters[0].name_hash, 0x1111_0001);
        std::fs::write(&path, b"{ not json").expect("damaged");
        assert!(kept_effects(&path).is_none());
    }

    #[test]
    fn the_inspector_s_lines_leave_a_drawable_s_shaders_to_its_material_rows() {
        let shader = Shader {
            name_hash: 0xABCD_0123,
            texture_keys: vec![None],
            parameters: vec![Parameter {
                hash: 0x1111_0001,
                value: ParameterValue::Texture { slot: 0 },
            }],
        };
        let drawable = Drawable {
            shaders: vec![shader],
            ..drawable("d")
        };
        let lines = body_lines(&Resource::Drawable(drawable), None).join("\n");
        assert!(lines.contains("A drawable named d"), "{lines}");
        assert!(!lines.contains("Shader 0"), "{lines}");
    }
}
