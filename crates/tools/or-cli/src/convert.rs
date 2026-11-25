use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{error, info};

use or_io::audio_world_sectors::{AudSector, AudioWorldSectors};
use or_io::cache_loader::CacheFile;
use or_io::codec::anim::AnimationSource as _;
use or_io::container::resource::ResourceType;
use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::{Gen8, Gen9, Raw, View};
use or_io::distant_lights::DistantLights;
use or_io::gta5::data::water::CWaterData;
use or_io::gta5::drawable::DrawableTarget;
use or_io::gta5::pose_db::PoseMatcherDatabase;
use or_io::gta5::shaders::fxc::{FxcArchive, GrcEffect};
use or_io::gta5::text::Gxt2;
use or_io::gta5::File as _;
use or_io::gta5::{
    CNavMesh, CNavMeshQuadTree, CPathRegion, CVehicleRecording, CrClipAny, CrClipDictionary,
    CrExpressions, CrExpressionsDictionary, CrFrameFilterAny, CrFrameFilterDictionary, FragType,
    FwWaypointRecordingRoute, GtaDrawable, GtaFragType, PgDictionary, PtxFxList, RmcDrawable,
};
use or_io::height_map::HeightMap;
use or_io::mrf::{MvNetworkDef, NodeBody};
use or_io::nav_precalc::NavPrecalc;
use or_io::social_club_metadata::SocialClubMetadata;
use or_io::water_map::WaterMap;
use or_io::world_blanket::WorldBlanket;
use or_io::Generation;

use crate::inspect::{bound_header, truncate_name};
use crate::texture::Listing;

// A `ResourceType`, or a format with no RSC7 header and so no version, no
// generation and no paged layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subject {
    Resource(ResourceType),
    Gxt2,
    Ypdb,
    Fxc,
    HeightMap,
    DistantLights,
    WaterMap,
    WaterData,
    WorldBlanket,
    NavPrecalc,
    Mrf,
    CacheFile,
    SocialClubMetadata,
    AudioWorldSectors,
}

impl Subject {
    fn from_extension(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if name.ends_with(".dat") {
            if name.starts_with("heightmap") {
                return Some(Self::HeightMap);
            }
            if name.starts_with("distantlights") {
                return Some(Self::DistantLights);
            }
            if name.starts_with("waterheight") {
                return Some(Self::WaterMap);
            }
            if name.starts_with("worldblanket") {
                return Some(Self::WorldBlanket);
            }
            if name.starts_with("navprecalc") {
                return Some(Self::NavPrecalc);
            }
            if name.ends_with("_cache_y.dat") {
                return Some(Self::CacheFile);
            }
            if name == "metadata.dat" {
                return Some(Self::SocialClubMetadata);
            }
            if name == "audioworldsectorsinfo.dat" {
                return Some(Self::AudioWorldSectors);
            }
        }
        if or_io::gta5::data::water::is_water_file(&name) {
            return Some(Self::WaterData);
        }
        let extension = path.extension().and_then(OsStr::to_str)?;
        if extension.eq_ignore_ascii_case("gxt2") {
            return Some(Self::Gxt2);
        }
        if extension.eq_ignore_ascii_case("ypdb") {
            return Some(Self::Ypdb);
        }
        if extension.eq_ignore_ascii_case("fxc") {
            return Some(Self::Fxc);
        }
        if extension.eq_ignore_ascii_case("mrf") {
            return Some(Self::Mrf);
        }
        ResourceType::from_extension(extension).map(Self::Resource)
    }

    fn as_resource(self) -> Option<ResourceType> {
        match self {
            Self::Resource(resource) => Some(resource),
            Self::Gxt2 | Self::Ypdb | Self::Fxc | Self::HeightMap | Self::DistantLights => None,
            Self::WaterMap | Self::WorldBlanket | Self::NavPrecalc | Self::Mrf => None,
            Self::WaterData => None,
            Self::CacheFile | Self::SocialClubMetadata | Self::AudioWorldSectors => None,
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Resource(resource) => {
                format!("{} ({})", resource, resource.description())
            }
            Self::Gxt2 => "GXT2 (Global Text Table)".to_string(),
            Self::Ypdb => "YPDB (Pose matcher database)".to_string(),
            Self::Fxc => "FXC (Shader effect archive)".to_string(),
            Self::HeightMap => "Height map (terrain height grid)".to_string(),
            Self::DistantLights => "Distant lights (far traffic and street lights)".to_string(),
            Self::WaterMap => "Water height map (river, lake and pool tiles)".to_string(),
            Self::WaterData => "Water (water, calming and wave quads)".to_string(),
            Self::WorldBlanket => "World blanket (a square grid of 16-bit cells)".to_string(),
            Self::NavPrecalc => "Nav precalc (NAVP cells and a BITS block)".to_string(),
            Self::Mrf => "MRF (Move network definition)".to_string(),
            Self::CacheFile => "Cache file (streaming store extents)".to_string(),
            Self::SocialClubMetadata => "Social Club metadata (metadata.dat)".to_string(),
            Self::AudioWorldSectors => "Audio world sectors (100 x 100 grid)".to_string(),
        }
    }
}

/// The refusal for a format OpenRage reads and never writes: a `.fxc` is
/// `fxc.exe`'s output, which `or-shader-tool` bakes rather than rewrites, and
/// nothing here authors `.ysc` bytecode. Not "no reader": the reader produced
/// the report the caller just saw.
fn no_writer(subject: Subject) -> String {
    format!(
        "{}: OpenRage reads this and does not write it",
        subject.name()
    )
}

fn no_reader(subject: Subject) -> String {
    format!(
        "{}: recognised, but OpenRage cannot read it yet",
        subject.name()
    )
}

// Which edition a file came from, read from its own RSC7 header.
struct FileProvenance {
    version: Option<u32>,
    generation: Option<Generation>,
}

impl FileProvenance {
    fn detect(subject: Subject, data: &[u8]) -> Self {
        // GXT2 and other non-RSC7 formats have no resource version.
        let Ok(header) = Rsc7::peek(data) else {
            return Self {
                version: None,
                generation: None,
            };
        };
        let generation = subject
            .as_resource()
            .and_then(|rt| rt.generation_of_version(header.version()));
        Self {
            version: Some(header.version()),
            generation,
        }
    }
}

fn cmd_info(path: &Path, json: bool) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;

    let resource_type = Subject::from_extension(path)
        .ok_or_else(|| format!("Unknown file type: {}", path.display()))?;

    let provenance = FileProvenance::detect(resource_type, &data);

    if json {
        let mut doc = info_json(resource_type, path, &data)?;
        if let Some(m) = doc.as_object_mut() {
            m.insert("file".into(), json!(path.display().to_string()));
            m.insert("type".into(), json!(resource_type.name()));
            m.insert("size".into(), json!(data.len()));
            if let Some(version) = provenance.version {
                m.insert("version".into(), json!(version));
            }
            if let Some(generation) = provenance.generation {
                m.insert("generation".into(), json!(generation.slug()));
                m.insert("edition".into(), json!(generation.edition_label()));
            }
        }
        print_json_doc(&doc);
        return Ok(());
    }

    println!("File: {}", path.display());
    println!("Type: {}", resource_type.name());
    println!("Size: {} bytes", data.len());
    if let Some(version) = provenance.version {
        match provenance.generation {
            Some(generation) => {
                println!(
                    "Edition: {} (version {version})",
                    generation.edition_label()
                )
            }
            None => println!("Edition: unrecognised (version {version})"),
        }
    }
    println!();

    match resource_type {
        Subject::Resource(ResourceType::Ytd) => info_ytd(path, &data),
        Subject::Resource(ResourceType::Ydr) => info_ydr(&data),
        Subject::Resource(ResourceType::Ydd) => info_ydd(&data),
        Subject::Resource(ResourceType::Yft) => info_yft(&data),
        Subject::Resource(ResourceType::Ypt) => info_ypt(&data),
        Subject::Resource(ResourceType::Ybn) => info_ybn(&data),
        Subject::Resource(ResourceType::Yld) => info_yld(&data),
        Subject::Resource(ResourceType::Ycd) => info_ycd(&data),
        Subject::Resource(ResourceType::Yed) => info_yed(&data),
        Subject::Resource(ResourceType::Yfd) => info_yfd(&data),
        Subject::Resource(ResourceType::Ybd) => info_ybd(&data),
        Subject::Resource(ResourceType::Ynd) => info_ynd(&data),
        Subject::Resource(ResourceType::Ynv) => info_ynv(&data),
        Subject::Resource(ResourceType::Ysc) => info_ysc(&data),
        Subject::Resource(ResourceType::Yvr) => info_yvr(&data),
        Subject::Resource(ResourceType::Ywr) => info_ywr(&data),
        Subject::Gxt2 => info_gxt2(&data),
        Subject::Ypdb => info_ypdb(path, &data),
        Subject::Fxc => info_fxc(path, &data),
        Subject::HeightMap => info_height_map(&data),
        Subject::DistantLights => info_distant_lights(&data),
        Subject::WaterMap => info_water_map(&data),
        Subject::WaterData => info_water_data(&data),
        Subject::WorldBlanket => info_world_blanket(&data),
        Subject::NavPrecalc => info_nav_precalc(&data),
        Subject::Mrf => info_mrf(&data),
        Subject::CacheFile | Subject::SocialClubMetadata | Subject::AudioWorldSectors => {
            println!("{}:", resource_type.name());
            for (key, value) in dat_table_json(resource_type, &data)?
                .as_object()
                .into_iter()
                .flatten()
            {
                println!("  {key}: {value}");
            }
            Ok(())
        }
        unread @ Subject::Resource(ResourceType::Ypm) => Err(no_reader(unread)),
    }
}

fn info_json(resource_type: Subject, path: &Path, data: &[u8]) -> Result<Value, String> {
    match resource_type {
        Subject::Resource(ResourceType::Ytd) => {
            let dict = Listing::from_bytes(path, data)
                .map_err(|e| format!("Failed to parse YTD: {}", e))?;
            let textures: Vec<Value> = dict
                .textures()
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "width": t.width,
                        "height": t.height,
                        "mips": t.mip_count,
                        "format": format!("{:?}", t.pixels.pixel_format()),
                        "dataSize": t.pixels.payload().len(),
                    })
                })
                .collect();
            Ok(json!({ "textureCount": textures.len(), "textures": textures }))
        }
        Subject::Resource(ResourceType::Ydr) => {
            let d = GtaDrawable::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Failed to parse YDR: {}", e))?;
            let sphere = &d.lod_group.bounds_sphere.center_and_radius;
            let bbox = &d.lod_group.bounds_box;
            let lods: Vec<Value> = d
                .lod_group
                .lods
                .iter()
                .enumerate()
                .filter(|(_, lod)| !lod.models.is_empty())
                .map(|(i, l)| json!({ "level": i, "models": l.models.len() }))
                .collect();
            Ok(json!({
                "name": d.name,
                "hasGeometry": has_models(&d),
                "boundsSphere": {
                    "center": [sphere.x, sphere.y, sphere.z],
                    "radius": sphere.w,
                },
                "boundsBox": {
                    "min": [bbox.min.x, bbox.min.y, bbox.min.z],
                    "max": [bbox.max.x, bbox.max.y, bbox.max.z],
                },
                "lods": lods,
            }))
        }
        Subject::Resource(ResourceType::Ydd) => {
            let dict = PgDictionary::<GtaDrawable>::from_bytes(data)
                .map_err(|e| format!("Failed to parse YDD: {}", e))?;
            let entries: Vec<Value> = entries_of(&dict)
                .map(|(hash, d)| {
                    json!({
                        "hash": format!("{:#010X}", hash),
                        "name": d.name,
                        "hasGeometry": has_models(d),
                        "hasSkeleton": d.skeleton.is_some(),
                        "hasBound": d.bound.is_some(),
                        "lights": d.lights.len(),
                        "lods": active_lods(d),
                    })
                })
                .collect();
            Ok(json!({ "drawableCount": entries.len(), "drawables": entries }))
        }
        Subject::Resource(ResourceType::Yft) => {
            let frag = GtaFragType::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Failed to parse YFT: {}", e))?;
            Ok(json!({
                "name": frag.tune_name,
                "hasCommonDrawable": frag.common_drawable.is_some(),
                "extraDrawables": frag.extra_drawables.len(),
                "drawableCount": frag.drawables().len(),
                "hasPhysics": frag.physics_lod_group.is_some(),
                "physicsChildren": frag.child_count(),
                "boneCount": fragment_bones(&frag),
                "lights": frag.lights.len(),
                "envCloth": frag.env_cloth.len(),
                "glassWindows": frag.glass_pane_model_infos.len(),
            }))
        }
        Subject::Resource(ResourceType::Ypt) => {
            let fx = PtxFxList::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Failed to parse YPT: {}", e))?;
            let effects = effect_names(&fx);
            Ok(json!({
                "version": rsc7_version(data),
                "effectCount": effects.len(),
                "effects": effects,
            }))
        }
        Subject::Resource(ResourceType::Ybn) => {
            let bound = or_io::gta5::PhBoundAny::from_bytes(data)
                .map_err(|e| format!("Failed to parse YBN: {}", e))?;
            Ok(json!({ "boundType": format!("{:?}", bound_header(&bound).r#type) }))
        }
        Subject::Resource(ResourceType::Yld) => {
            let report = cloth_report(data).map_err(|e| format!("Failed to parse YLD: {}", e))?;
            let entries: Vec<Value> = report
                .entries
                .iter()
                .map(|e| {
                    json!({
                        "hash": format!("{:#010X}", e.name_hash),
                        "name": e.name,
                        "lods": e.populated_lods,
                        "vertices": e.vertex_count,
                        "edges": e.edge_count,
                        "customEdges": e.custom_edge_count,
                        "pinnedVertices": e.pinned_vertex_count,
                        "boneIds": e.bone_id_count,
                        "bindings": e.binding_count,
                        "poses": e.pose_count,
                        "triIndices": e.tri_index_count,
                        "hasBoundComposite": e.has_bound_composite,
                        "hasBridge": e.has_bridge,
                        "hasMorphController": e.has_morph_controller,
                        "hasClothBound": e.has_cloth_bound,
                    })
                })
                .collect();
            Ok(json!({
                "clothCount": report.entry_count,
                "hashesSorted": report.hashes_sorted,
                "virtualSize": report.virtual_size,
                "cloths": entries,
                "warnings": report.diagnostics.messages()
                    .iter()
                    .map(|d| serde_json::json!({ "key": d.key, "message": d.message, "at": d.location }))
                    .collect::<Vec<_>>(),
            }))
        }
        Subject::Resource(ResourceType::Ycd) => {
            let dict = CrClipDictionary::from_bytes(data)
                .map_err(|e| format!("Failed to parse YCD: {}", e))?;
            let clips: Vec<Value> = or_anim::clip::clips(&dict)
                .map(|(key, clip)| {
                    let header = or_anim::clip::header(clip);
                    let animation = or_anim::clip::primary(&dict, clip);
                    json!({
                        "key": format!("{:#010X}", key),
                        "name": header.name.data,
                        "type": header.r#type.raw(),
                        "duration": or_anim::clip::duration(clip),
                        "looped": or_anim::clip::is_looped(clip),
                        "tags": header.tags.as_ref().map_or(0, |t| t.tags.len()),
                        "properties": header
                            .properties
                            .as_ref()
                            .map_or(0, |p| p.properties.entries().count()),
                        "frames": animation.map_or(0, |a| a.num_frames),
                        "dofs": animation.map_or(0, |a| a.dofs.len()),
                        "bones": animation.map_or(0, |a| or_anim::clip::bone_ids(a).len()),
                        "compact": animation.is_some_and(or_anim::clip::is_compact),
                    })
                })
                .collect();
            Ok(json!({
                "clipCount": clips.len(),
                "animationCount": animation_count(&dict),
                "bucketCount": dict.clips.hash.len(),
                "baseNameKeys": dict.base_name_keys.get(),
                "clips": clips,
            }))
        }
        Subject::Resource(ResourceType::Yed) => {
            let expressions: Vec<Value> = expression_summaries(data)
                .map_err(|e| format!("Failed to parse YED: {}", e))?
                .iter()
                .map(|s| {
                    json!({
                        "key": format!("{:#010X}", s.key),
                        "name": s.name,
                        "shortName": s.short_name,
                        "tracks": s.track_count,
                        "bones": s.bone_count,
                        "variables": s.variable_count,
                        "springs": s.spring_count,
                        "streams": s.stream_count,
                        "instructions": s.instruction_count,
                        "maxStreamSize": s.declared_max_stream_size,
                    })
                })
                .collect();
            Ok(json!({ "expressionCount": expressions.len(), "expressions": expressions }))
        }
        Subject::Resource(ResourceType::Ybd) => {
            let dict = or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(data)
                .map_err(|e| format!("Failed to parse YBD: {}", e))?;
            let bounds: Vec<Value> = dict
                .codes
                .iter()
                .zip(dict.entries.iter())
                .map(|(hash, b)| {
                    json!({
                        "nameHash": format!("{hash:#010X}"),
                        "boundType": owned_bound_type(b.as_deref()),
                    })
                })
                .collect();
            Ok(json!({ "format": "ybd", "bounds": bounds }))
        }
        Subject::Resource(ResourceType::Ynv) => {
            let mesh =
                CNavMesh::from_bytes(data).map_err(|e| format!("Failed to parse YNV: {}", e))?;
            let s = MeshSummary::of(&mesh);
            Ok(json!({
                "vertexCount": s.vertices,
                "indexCount": s.indices,
                "adjacencyCount": s.adjacency,
                "polyCount": s.polys,
                "sectorCount": s.sectors,
                "sectorLeafCount": s.sector_leaves,
                "portalCount": s.portals,
            }))
        }
        Subject::Resource(ResourceType::Yfd) => {
            let dict = CrFrameFilterDictionary::from_bytes(data)
                .map_err(|e| format!("Failed to parse YFD: {}", e))?;
            let filters: Vec<Value> = filter_rows(&dict)
                .map(|s| {
                    json!({
                        "nameHash": format!("{:#010X}", s.name_hash),
                        "type": s.filter_type_name,
                        "entries": s.entry_count,
                        "weights": s.weight_count,
                        "bones": s.bone_count,
                        "entriesSorted": s.entries_sorted,
                    })
                })
                .collect();
            Ok(json!({ "filterCount": dict.entries.len(), "filters": filters }))
        }
        Subject::Resource(ResourceType::Ynd) => {
            let region =
                CPathRegion::from_bytes(data).map_err(|e| format!("Failed to parse YND: {}", e))?;
            let summary = RegionSummary::of(&region);
            Ok(json!({
                "areaId": summary.area_id,
                "nodeCount": summary.nodes,
                "vehicleNodes": summary.vehicle_nodes,
                "pedNodes": summary.ped_nodes,
                "linkCount": summary.links,
                "outboundLinks": summary.outbound_links,
                "junctionCount": summary.junctions,
                "junctionRefCount": summary.junction_refs,
                "heightmapBytes": summary.heightmap_bytes,
            }))
        }
        Subject::Resource(ResourceType::Yvr) => {
            let recording = CVehicleRecording::from_bytes(data)
                .map_err(|e| format!("Failed to parse YVR: {}", e))?;
            Ok(json!({
                "frameCount": recording.states.len(),
                "durationMs": recording_duration(&recording),
            }))
        }
        Subject::Resource(ResourceType::Ywr) => {
            let route = FwWaypointRecordingRoute::from_bytes(data)
                .map_err(|e| format!("Failed to parse YWR: {}", e))?;
            Ok(json!({ "waypointCount": route.waypoints.len() }))
        }
        Subject::Gxt2 => {
            let table =
                Gxt2::from_bytes(data).map_err(|e| format!("Failed to parse GXT2: {}", e))?;
            let entries: Vec<Value> = table
                .entries
                .iter()
                .map(|e| json!({ "hash": format!("{:#010X}", e.hash), "value": e.value }))
                .collect();
            Ok(json!({ "entryCount": table.len(), "entries": entries }))
        }
        Subject::Fxc => {
            let effect =
                GrcEffect::parse(data).map_err(|e| format!("Failed to parse FXC: {}", e))?;
            let archive =
                FxcArchive::parse(data).map_err(|e| format!("Failed to scan FXC: {}", e))?;
            let variables: Vec<Value> = effect
                .variables
                .iter()
                .map(|v| {
                    json!({
                        "name": v.name,
                        "semantic": v.semantic,
                        "type": format!("{:?}", v.var_type),
                        "count": v.count,
                        "slot": v.slot,
                        "cbuffer": format!("{:#010X}", v.cbuffer),
                    })
                })
                .collect();
            Ok(json!({
                "vertexType": effect.vertex_type,
                "drawBucket": effect.draw_bucket(),
                "presets": effect.presets.iter().map(|p| &p.name).collect::<Vec<_>>(),
                "programs": effect.programs.len(),
                "cbuffers": effect.cbuffers.len(),
                "techniques": effect.techniques.len(),
                "variables": variables,
                "dxbcBlobs": archive.len(),
                "trailingBytes": GrcEffect::trailing_bytes(data).unwrap_or(0),
            }))
        }
        Subject::Ypdb => {
            let db = PoseMatcherDatabase::from_bytes(data)
                .map_err(|e| format!("Failed to parse YPDB: {}", e))?;
            let samples: Vec<Value> = db
                .samples
                .iter()
                .map(|s| {
                    json!({
                        "clipSet": format!("{:#010X}", s.clip_set),
                        "clip": format!("{:#010X}", s.clip),
                        "offset": s.offset,
                        "points": s.cloud.points.len(),
                    })
                })
                .collect();
            Ok(json!({
                "signature": format!("{:#010X}", db.signature),
                "boneTags": db.bone_tags,
                "sampleCount": db.samples.len(),
                "samples": samples,
            }))
        }
        Subject::Resource(ResourceType::Ysc) => {
            let program = or_io::gta5::script::ScrProgram::from_bytes(data)
                .map_err(|e| format!("Failed to parse YSC: {e}"))?;
            // ⚠ Natives are hashes and the key says so, because a consumer
            // reading `natives` as names would be silently wrong.
            Ok(json!({
                "name": program.name,
                "nameHash": format!("{:#010X}", program.name_hash),
                "globalsSignature": format!("{:#010X}", program.globals_signature),
                "codeLength": program.code_length,
                "staticCount": program.statics.len(),
                "parameterCount": program.parameter_count,
                "nativeHashes": program
                    .native_hashes()
                    .iter()
                    .map(|h| format!("{h:#018X}"))
                    .collect::<Vec<_>>(),
                "stringCount": program.strings.len(),
            }))
        }
        Subject::HeightMap => {
            let map = HeightMap::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "columns": map.num_cols,
                "rows": map.num_rows,
                "cellSize": map.cell_size(),
                "boundsMin": map.bounds_min,
                "boundsMax": map.bounds_max,
                "flags": map.flags,
                "cellsWithHeight": map.max.iter().filter(|&&h| h != 0).count(),
                "waterMask": !map.water_mask.is_empty(),
            }))
        }
        Subject::CacheFile | Subject::SocialClubMetadata | Subject::AudioWorldSectors => {
            dat_table_json(resource_type, data)
        }
        Subject::DistantLights => {
            let lights = DistantLights::open(data).map_err(|e| e.to_string())?;
            let groups: Vec<Value> = lights
                .groups
                .iter()
                .map(|g| {
                    json!({
                        "centre": [g.centre_x, g.centre_y, g.centre_z],
                        "radius": g.radius,
                        "points": g.point_count,
                        "lanes": g.num_lanes(),
                        "twoWay": g.two_way(),
                        "streetLights": g.use_streetlights(),
                    })
                })
                .collect();
            Ok(json!({
                "blocks": lights.blocks(),
                "pointCount": lights.points.len(),
                "groupCount": groups.len(),
                "groups": groups,
            }))
        }
        Subject::WaterMap => {
            let map = WaterMap::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "columns": map.num_tile_cols,
                "rows": map.num_tile_rows(),
                "tileSize": [map.tile_size_x, map.tile_size_y],
                "offset": [map.offset_x, map.offset_y],
                "tilesWithWater": water_tiles(&map),
                "riverPoints": map.river_points.len(),
                "rivers": map.rivers.len(),
                "lakeBoxes": map.lake_boxes.len(),
                "lakes": map.lakes.len(),
                "pools": map.pools.len(),
            }))
        }
        Subject::WaterData => {
            let water = CWaterData::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "waterQuads": water.water_quads.len(),
                "calmingQuads": water.calming_quads.len(),
                "waveQuads": water.wave_quads.len(),
                "heights": water_heights(&water),
                "unnamed": water.unnamed(),
            }))
        }
        Subject::WorldBlanket => {
            let blanket = WorldBlanket::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "side": blanket.side(),
                "nonZeroCells": blanket.cells.iter().filter(|&&c| c != 0).count(),
            }))
        }
        Subject::NavPrecalc => {
            let precalc = NavPrecalc::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "header": precalc.header,
                "cells": precalc.cells.len(),
                "cellsWithEntries": precalc.cells.iter().filter(|c| c.entries() > 0).count(),
                "entries": precalc.cells.iter().map(|c| c.entries()).sum::<usize>(),
                "bitsBytes": precalc.bits.len(),
            }))
        }
        Subject::Mrf => {
            let network = MvNetworkDef::open(data).map_err(|e| e.to_string())?;
            let counts: serde_json::Map<String, Value> = network
                .node_counts()
                .into_iter()
                .map(|(kind, n)| (kind.name().to_string(), json!(n)))
                .collect();
            let machines: Vec<Value> = state_machines(&network)
                .into_iter()
                .map(|m| {
                    json!({
                        "id": format!("{:#010x}", m.id),
                        "kind": m.kind,
                        "states": m.states.iter().map(|&(id, transitions)| json!({
                            "id": format!("{id:#010x}"),
                            "transitions": transitions,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({
                "nodeCount": network.nodes.len(),
                "nodeCounts": counts,
                "requests": network.requests.len(),
                "flags": network.flags.len(),
                "stateMachines": machines,
            }))
        }
        unread @ Subject::Resource(ResourceType::Ypm) => Err(no_reader(unread)),
    }
}

/// A state machine node, as `convert info` lists it.
struct Machine {
    id: u32,
    kind: &'static str,
    /// Each state's name hash and how many transitions leave it.
    states: Vec<(u32, usize)>,
}

fn state_machines(network: &MvNetworkDef) -> Vec<Machine> {
    let edges = network.edges();
    network
        .nodes
        .iter()
        .zip(&edges)
        .filter_map(|(node, node_edges)| {
            let states = match &node.body {
                NodeBody::StateMachine(sm) => &sm.states,
                NodeBody::InlinedStateMachine(ism) => &ism.states,
                _ => return None,
            };
            let states = states
                .iter()
                .zip(&node_edges.states)
                .map(|(s, at)| (s.id, at.map_or(0, |n| edges[n].transitions.len())))
                .collect();
            Some(Machine {
                id: node.id,
                kind: node.node_type.name(),
                states,
            })
        })
        .collect()
}

fn print_json_doc(doc: &Value) {
    match serde_json::to_string_pretty(doc) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("Error: serialize: {e}"),
    }
}

fn info_ytd(path: &Path, data: &[u8]) -> Result<(), String> {
    let dict =
        Listing::from_bytes(path, data).map_err(|e| format!("Failed to parse YTD: {}", e))?;
    let textures = dict.textures();

    println!("Texture Dictionary Contents:");
    println!("  Texture count: {}", textures.len());
    println!();

    if textures.is_empty() {
        println!("  (empty dictionary)");
    } else {
        println!(
            "  {:<40} {:>8} {:>8} {:>6} {:>12} {:>12}",
            "Name", "Width", "Height", "Mips", "Format", "Data Size"
        );
        println!("  {}", "-".repeat(92));

        for texture in &textures {
            let format_str = format!("{:?}", texture.pixels.pixel_format());
            println!(
                "  {:<40} {:>8} {:>8} {:>6} {:>12} {:>12}",
                truncate_name(texture.name, 40),
                texture.width,
                texture.height,
                texture.mip_count,
                format_str,
                texture.pixels.payload().len()
            );
        }
    }

    Ok(())
}

/// Whether any level of detail holds a model.
pub(crate) fn has_models<T: DrawableTarget>(drawable: &RmcDrawable<T>) -> bool {
    active_lods(drawable) > 0
}

/// How many levels of detail hold a model.
pub(crate) fn active_lods<T: DrawableTarget>(drawable: &RmcDrawable<T>) -> usize {
    (drawable.lod_group.lods.iter())
        .filter(|l| !l.models.is_empty())
        .count()
}

/// The bones a fragment's common drawable carries.
pub(crate) fn fragment_bones<T: DrawableTarget>(frag: &FragType<T>) -> usize {
    (frag.common_drawable.as_deref())
        .and_then(|d| d.skeleton.as_deref())
        .map_or(0, |s| s.bones.len())
}

/// A drawable dictionary's entries, each with its hash.
pub(crate) fn entries_of(
    dict: &PgDictionary<GtaDrawable>,
) -> impl Iterator<Item = (u32, &GtaDrawable)> + '_ {
    (dict.codes.iter().zip(&dict.entries)).filter_map(|(h, d)| Some((*h, d.as_deref()?)))
}

fn info_ydr(data: &[u8]) -> Result<(), String> {
    let drawable =
        GtaDrawable::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YDR: {}", e))?;
    let sphere = &drawable.lod_group.bounds_sphere.center_and_radius;
    let bbox = &drawable.lod_group.bounds_box;

    println!("Drawable Contents:");
    println!("  Name: {}", drawable.name);
    println!("  Has geometry: {}", has_models(&drawable));
    println!();

    println!("  Bounding Sphere:");
    println!(
        "    Center: ({:.3}, {:.3}, {:.3})",
        sphere.x, sphere.y, sphere.z
    );
    println!("    Radius: {:.3}", sphere.w);
    println!();

    println!("  Bounding Box:");
    println!(
        "    Min: ({:.3}, {:.3}, {:.3})",
        bbox.min.x, bbox.min.y, bbox.min.z
    );
    println!(
        "    Max: ({:.3}, {:.3}, {:.3})",
        bbox.max.x, bbox.max.y, bbox.max.z
    );
    println!();

    println!("  LOD Levels:");
    for (i, level) in drawable.lod_group.lods.iter().enumerate() {
        if !level.models.is_empty() {
            println!("    LOD {}: {} model(s)", i, level.models.len());
        }
    }

    Ok(())
}

fn info_ydd(data: &[u8]) -> Result<(), String> {
    let dict = PgDictionary::<GtaDrawable>::from_bytes(data)
        .map_err(|e| format!("Failed to parse YDD: {}", e))?;

    println!("Drawable Dictionary Contents:");
    println!("  Drawable count: {}", entries_of(&dict).count());
    println!();

    if entries_of(&dict).next().is_none() {
        println!("  (empty dictionary)");
        return Ok(());
    }

    println!(
        "  {:<40} {:>12} {:>6} {:>6} {:>8} {:>7} {:>7}",
        "Name", "Hash", "LODs", "Geoms", "Skeleton", "Bound", "Lights"
    );
    println!("  {}", "-".repeat(92));

    for (hash, d) in entries_of(&dict) {
        let geoms: usize = (d.lod_group.lods.iter())
            .flat_map(|l| l.models.iter())
            .map(|m| m.geometries.len())
            .sum();
        println!(
            "  {:<40} {:>12} {:>6} {:>6} {:>8} {:>7} {:>7}",
            truncate_name(&d.name, 40),
            format!("{:#010X}", hash),
            active_lods(d),
            geoms,
            d.skeleton.is_some(),
            d.bound.is_some(),
            d.lights.len()
        );
    }

    Ok(())
}

fn info_yft(data: &[u8]) -> Result<(), String> {
    let frag =
        GtaFragType::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YFT: {}", e))?;

    println!("Fragment Contents:");
    println!("  Name: {}", frag.tune_name);
    println!(
        "  Common drawable: {}",
        frag.common_drawable
            .as_deref()
            .map(|d| {
                if d.name.is_empty() {
                    "(unnamed)".to_string()
                } else {
                    d.name.clone()
                }
            })
            .unwrap_or_else(|| "(none)".to_string())
    );
    println!("  Extra drawables: {}", frag.extra_drawables.len());
    println!("  Env cloth: {}", frag.env_cloth.len());
    println!("  Lights: {}", frag.lights.len());
    println!("  Glass windows: {}", frag.glass_pane_model_infos.len());
    println!("  Has physics: {}", frag.physics_lod_group.is_some());
    println!("  Physics children: {}", frag.child_count());
    println!("  Bones: {}", fragment_bones(&frag));
    println!(
        "  Bounding sphere: center=({:.3}, {:.3}, {:.3}) radius={:.3}",
        frag.bounding_sphere.x,
        frag.bounding_sphere.y,
        frag.bounding_sphere.z,
        frag.bounding_sphere.w
    );

    Ok(())
}

/// The names of a particle list's effect rules, in dictionary order.
pub(crate) fn effect_names<T: DrawableTarget>(list: &PtxFxList<T>) -> Vec<&str> {
    (list.effect_rule_dict.as_deref())
        .map(|d| {
            d.entries
                .iter()
                .flatten()
                .map(|e| e.name.as_str())
                .collect()
        })
        .unwrap_or_default()
}

fn info_ypt(data: &[u8]) -> Result<(), String> {
    let list =
        PtxFxList::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YPT: {}", e))?;
    let effects = effect_names(&list);

    println!("Particle Effects List:");
    println!("  Version: {}", rsc7_version(data).unwrap_or(0));
    println!("  Effect rules: {}", effects.len());
    println!();

    if !effects.is_empty() {
        println!("  Effect Rules (first 20):");
        for (i, effect) in effects.iter().enumerate() {
            if i >= 20 {
                println!("    ... and {} more", effects.len() - 20);
                break;
            }
            println!("    - {}", effect);
        }
    }

    Ok(())
}

/// What one path region holds, as a flat row.
struct RegionSummary {
    nodes: usize,
    vehicle_nodes: usize,
    ped_nodes: usize,
    links: usize,
    /// Links whose target region is not this one.
    outbound_links: usize,
    junctions: usize,
    junction_refs: usize,
    heightmap_bytes: usize,
    /// The region every node in the file names, where they agree.
    area_id: Option<u16>,
}

impl RegionSummary {
    fn of(region: &CPathRegion) -> Self {
        let mut areas: Vec<u16> = region.nodes.iter().map(|n| n.address.region).collect();
        areas.sort_unstable();
        areas.dedup();
        let area_id = match areas.as_slice() {
            [only] => Some(*only),
            _ => None,
        };
        Self {
            nodes: region.nodes.len(),
            vehicle_nodes: region.num_nodes_car_nodes as usize,
            ped_nodes: region.num_nodes_ped_nodes as usize,
            links: region.links.len(),
            outbound_links: area_id.map_or(0, |area| {
                (region.links.iter())
                    .filter(|l| l.other_node.region != area)
                    .count()
            }),
            junctions: region.virtual_junctions.len(),
            junction_refs: region.junction_map.junction_map.data.len(),
            heightmap_bytes: region.height_samples.len(),
            area_id,
        }
    }
}

/// What one navmesh cell holds, as a flat row.
struct MeshSummary {
    vertices: usize,
    indices: usize,
    adjacency: usize,
    polys: usize,
    /// Nodes in the sector quadtree, and how many of them are leaves.
    sectors: usize,
    sector_leaves: usize,
    /// Polygon ids the leaves list, a polygon once per leaf it is in.
    sector_poly_ids: usize,
    portals: usize,
    portal_links: usize,
    /// The size of the box a vertex quantises into, from the quadtree's root.
    box_size: Option<[f32; 3]>,
}

impl MeshSummary {
    fn of(mesh: &CNavMesh) -> Self {
        fn walk(node: &CNavMeshQuadTree, s: &mut MeshSummary) {
            s.sectors += 1;
            if let Some(leaf) = &node.leaf_data {
                s.sector_leaves += 1;
                s.sector_poly_ids += leaf.polys.len();
            }
            for child in node.children.iter().flatten() {
                walk(child, s);
            }
        }
        let mut s = Self {
            vertices: mesh.compressed_vertex_array.as_ref().map_or(0, |a| a.len()),
            indices: mesh.vertex_index_array.as_ref().map_or(0, |a| a.len()),
            adjacency: mesh.adjacent_polys_array.as_ref().map_or(0, |a| a.len()),
            polys: mesh.polys_array.as_ref().map_or(0, |a| a.len()),
            sectors: 0,
            sector_leaves: 0,
            sector_poly_ids: 0,
            portals: mesh.special_links.len(),
            portal_links: mesh.special_links_indices.len(),
            box_size: mesh.quad_tree.as_ref().map(|t| {
                [
                    t.maxs.x - t.mins.x,
                    t.maxs.y - t.mins.y,
                    t.maxs.z - t.mins.z,
                ]
            }),
        };
        if let Some(root) = &mesh.quad_tree {
            walk(root, &mut s);
        }
        s
    }
}

fn info_ynd(data: &[u8]) -> Result<(), String> {
    let region =
        CPathRegion::from_bytes(data).map_err(|e| format!("Failed to parse YND: {}", e))?;
    let summary = RegionSummary::of(&region);

    println!("Path node region:");
    match summary.area_id {
        Some(area) => println!("  Area: {area}"),
        None => println!("  Area: mixed (its nodes do not agree on one)"),
    }
    println!(
        "  Nodes: {} ({} vehicle, {} pedestrian)",
        summary.nodes, summary.vehicle_nodes, summary.ped_nodes
    );
    println!(
        "  Links: {} ({} leaving this region)",
        summary.links, summary.outbound_links
    );
    println!(
        "  Junctions: {} ({} references, {} heightmap bytes)",
        summary.junctions, summary.junction_refs, summary.heightmap_bytes
    );
    println!();

    Ok(())
}

/// Milliseconds from a recording's start to its last frame.
fn recording_duration(recording: &CVehicleRecording) -> u32 {
    recording
        .states
        .last()
        .map_or(0, |state| state.time_in_recording)
}

fn info_yvr(data: &[u8]) -> Result<(), String> {
    let recording =
        CVehicleRecording::from_bytes(data).map_err(|e| format!("Failed to parse YVR: {}", e))?;
    println!("Vehicle recording:");
    println!("  Frames: {}", recording.states.len());
    println!("  Duration: {} ms", recording_duration(&recording));
    println!();
    Ok(())
}

fn info_ywr(data: &[u8]) -> Result<(), String> {
    let route = FwWaypointRecordingRoute::from_bytes(data)
        .map_err(|e| format!("Failed to parse YWR: {}", e))?;
    println!("Waypoint recording:");
    println!("  Waypoints: {}", route.waypoints.len());
    println!();
    Ok(())
}

fn info_ynv(data: &[u8]) -> Result<(), String> {
    let mesh = CNavMesh::from_bytes(data).map_err(|e| format!("Failed to parse YNV: {}", e))?;
    let s = MeshSummary::of(&mesh);

    println!("Navigation mesh:");
    println!("  Vertices: {}", s.vertices);
    println!("  Polygons: {}", s.polys);
    println!(
        "  Indices: {} ({} adjacency entries)",
        s.indices, s.adjacency
    );
    println!(
        "  Sectors: {} ({} leaves, {} polygon ids)",
        s.sectors, s.sector_leaves, s.sector_poly_ids
    );
    println!("  Portals: {} ({} links)", s.portals, s.portal_links);
    if let Some(size) = s.box_size {
        println!("  Cell: {:.1} x {:.1} x {:.1}", size[0], size[1], size[2]);
    }
    println!();

    Ok(())
}

fn info_ybn(data: &[u8]) -> Result<(), String> {
    let bound = or_io::gta5::PhBoundAny::from_bytes(data)
        .map_err(|e| format!("Failed to parse YBN: {}", e))?;

    println!("Physics bound:");
    println!("  Type: {:?}", bound_header(&bound).r#type);
    println!();

    Ok(())
}

/// A bound slot's type, or `(none)` for an empty one.
fn owned_bound_type(bound: Option<&or_io::gta5::PhBoundAny>) -> String {
    bound.map_or("(none)".into(), |b| format!("{:?}", bound_header(b).r#type))
}

/// The keys a `.yld` report's findings carry.
mod yld_keys {
    pub const HASHES_UNSORTED: &str = "YLD_HASHES_UNSORTED";
    pub const NO_CLOTH_CONTROLLER: &str = "YLD_NO_CLOTH_CONTROLLER";
    pub const NO_BOUND_COMPOSITE: &str = "YLD_NO_BOUND_COMPOSITE";
    pub const MORPH_CONTROLLER_UNVERIFIED: &str = "YLD_MORPH_CONTROLLER_UNVERIFIED";
    pub const NO_BRIDGE: &str = "YLD_NO_BRIDGE";
    pub const LOD_ABOVE_ZERO_POPULATED: &str = "YLD_LOD_ABOVE_ZERO_POPULATED";
}

/// One cloth of a `.yld`, as a flat row.
pub(crate) struct ClothEntrySummary {
    pub name_hash: u32,
    pub name: String,
    pub populated_lods: Vec<usize>,
    pub vertex_count: u32,
    pub edge_count: u32,
    pub custom_edge_count: usize,
    pub pinned_vertex_count: u32,
    pub bone_id_count: usize,
    pub binding_count: usize,
    pub pose_count: usize,
    pub tri_index_count: usize,
    pub has_bound_composite: bool,
    pub has_bridge: bool,
    pub has_morph_controller: bool,
    pub has_cloth_bound: bool,
}

/// What a `.yld` holds, and what in it the game would not expect.
pub(crate) struct YldValidation {
    pub virtual_size: usize,
    pub entry_count: usize,
    pub hashes_sorted: bool,
    pub entries: Vec<ClothEntrySummary>,
    pub diagnostics: or_io::Diagnostics,
}

impl YldValidation {
    pub fn total_vertices(&self) -> u32 {
        self.entries.iter().map(|e| e.vertex_count).sum()
    }

    pub fn total_edges(&self) -> u32 {
        self.entries.iter().map(|e| e.edge_count).sum()
    }
}

/// What `info`, `verify` and `verify-stats` report of a `.yld`, from the owned dictionary.
pub(crate) fn cloth_report(data: &[u8]) -> Result<YldValidation, String> {
    use or_io::{Diagnostic, Diagnostics};
    let virtual_size = Rsc7::peek(data).map_err(|e| e.to_string())?.virtual_size() as usize;
    let dict = or_io::gta5::PgDictionary::<or_io::gta5::CharacterCloth>::from_bytes(data)
        .map_err(|e| e.to_string())?;

    let mut diagnostics = Diagnostics::new();
    let hashes_sorted = dict.codes.windows(2).all(|w| w[0] <= w[1]);
    if !hashes_sorted {
        diagnostics.warn(
            yld_keys::HASHES_UNSORTED,
            "hash table is not ascending; the engine binary-searches it and would miss entries",
        );
    }

    let mut entries = Vec::with_capacity(dict.entries.len());
    for (index, (&hash, cloth)) in dict.codes.iter().zip(&dict.entries).enumerate() {
        let Some(cloth) = cloth else { continue };
        let controller = cloth.cloth_controller.as_ref();
        let at = |lod: Option<usize>| match lod {
            Some(lod) => format!("entry {index} ({hash:#010X}) LOD {lod}"),
            None => format!("entry {index} ({hash:#010X})"),
        };
        let levels = || {
            controller
                .into_iter()
                .flat_map(|c| c.base.cloth())
                .flatten()
                .map(|v| &**v)
        };

        if controller.is_none() {
            diagnostics.push(
                Diagnostic::warning(yld_keys::NO_CLOTH_CONTROLLER, "no cloth controller")
                    .at(at(None)),
            );
        }
        if cloth.bound_composite.is_none() {
            diagnostics.push(
                Diagnostic::warning(
                    yld_keys::NO_BOUND_COMPOSITE,
                    "no collision composite; every retail file has one",
                )
                .at(at(None)),
            );
        }
        if let Some(ctrl) = controller {
            if ctrl.base.morph_controller.is_some() {
                diagnostics.push(
                    Diagnostic::warning(
                        yld_keys::MORPH_CONTROLLER_UNVERIFIED,
                        "has a morph controller; phMapData/phMorphData layouts are \
                         engine-derived and unverified",
                    )
                    .at(at(None)),
                );
            }
            if ctrl.base.bridge.is_none() {
                diagnostics.push(
                    Diagnostic::warning(yld_keys::NO_BRIDGE, "no sim-to-render bridge")
                        .at(at(None)),
                );
            }
            for (lod, slot) in ctrl.base.cloth().into_iter().enumerate().skip(1) {
                if slot.is_some() {
                    diagnostics.push(
                        Diagnostic::info(
                            yld_keys::LOD_ABOVE_ZERO_POPULATED,
                            "populated; only LOD 0 occurs in the reference corpus",
                        )
                        .at(at(Some(lod))),
                    );
                }
            }
        }

        // ⚠ An owned level's vertex and edge counts are the lengths of their
        // runs, written from them at save, so there is no stated count to disagree.
        let sum = |f: fn(&or_io::gta5::PhVerletCloth) -> usize| levels().map(f).sum::<usize>();
        entries.push(ClothEntrySummary {
            name_hash: hash,
            name: controller
                .and_then(|c| std::str::from_utf8(c.base.name()).ok())
                .unwrap_or("")
                .to_string(),
            populated_lods: controller
                .map(|c| {
                    c.base
                        .cloth()
                        .iter()
                        .enumerate()
                        .filter(|(_, slot)| slot.is_some())
                        .map(|(lod, _)| lod)
                        .collect()
                })
                .unwrap_or_default(),
            vertex_count: sum(|v| v.cloth_data.vertex_positions.len()) as u32,
            edge_count: sum(|v| v.edge_data.len()) as u32,
            custom_edge_count: sum(|v| v.custom_edge_data.len()),
            pinned_vertex_count: levels().map(|v| v.num_pinned_verts as u32).sum(),
            bone_id_count: cloth.bone_id.len(),
            binding_count: controller.map_or(0, |c| c.binding_info.len()),
            pose_count: cloth.poses.len(),
            tri_index_count: controller.map_or(0, |c| c.tri_indices.len()),
            has_bound_composite: cloth.bound_composite.is_some(),
            has_bridge: controller.is_some_and(|c| c.base.bridge.is_some()),
            has_morph_controller: controller.is_some_and(|c| c.base.morph_controller.is_some()),
            has_cloth_bound: levels()
                .any(|v| v.custom_bound.is_some() || v.virtual_bound.is_some()),
        });
    }

    Ok(YldValidation {
        virtual_size,
        entry_count: dict.entries.len(),
        hashes_sorted,
        entries,
        diagnostics,
    })
}

fn info_yld(data: &[u8]) -> Result<(), String> {
    let report = cloth_report(data).map_err(|e| format!("Failed to parse YLD: {}", e))?;

    println!("Cloth dictionary:");
    println!("  Entries:      {}", report.entry_count);
    println!("  Virtual heap: {} bytes", report.virtual_size);
    println!("  Hashes sorted: {}", report.hashes_sorted);
    println!();

    if report.entries.is_empty() {
        println!("  (empty dictionary)");
        return Ok(());
    }

    println!(
        "  {:<10} {:<20} {:>5} {:>7} {:>7} {:>6} {:>6}",
        "Hash", "Name", "LODs", "Verts", "Edges", "Pinned", "Bones"
    );
    println!("  {}", "-".repeat(74));
    for entry in &report.entries {
        let lods = entry
            .populated_lods
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "  {:#010X} {:<20} {:>5} {:>7} {:>7} {:>6} {:>6}",
            entry.name_hash,
            truncate_name(&entry.name, 20),
            if lods.is_empty() { "-".into() } else { lods },
            entry.vertex_count,
            entry.edge_count,
            entry.pinned_vertex_count,
            entry.bone_id_count,
        );
    }
    println!();

    for entry in &report.entries {
        println!(
            "  {}: bound={} bridge={} morph={} clothBound={} bindings={} tris={} poses={} customEdges={}",
            if entry.name.is_empty() {
                format!("{:#010X}", entry.name_hash)
            } else {
                entry.name.clone()
            },
            entry.has_bound_composite,
            entry.has_bridge,
            entry.has_morph_controller,
            entry.has_cloth_bound,
            entry.binding_count,
            entry.tri_index_count,
            entry.pose_count,
            entry.custom_edge_count,
        );
    }

    if !report.diagnostics.is_empty() {
        println!();
        println!("Warnings:");
        for finding in report.diagnostics.messages() {
            println!("  - {finding}");
        }
    }
    println!();

    Ok(())
}

/// What an expression set states about itself without decoding a single opcode.
struct ExpressionSummary {
    key: u32,
    name: String,
    short_name: String,
    track_count: usize,
    bone_count: usize,
    variable_count: usize,
    spring_count: usize,
    stream_count: usize,
    instruction_count: usize,
    longest_stream: usize,
    declared_max_stream_size: u32,
}

impl ExpressionSummary {
    fn of(expr: &CrExpressions, instruction_count: usize) -> Self {
        let name = expr.name.data.as_str();
        let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let short_name = file.rfind('.').map_or(file, |dot| &file[..dot]);
        let mut bones: Vec<u16> = expr.tracks.iter().map(|t| t.id).collect();
        bones.sort_unstable();
        bones.dedup();
        let streams = || expr.expression_streams.iter().flatten();
        Self {
            key: or_io::gta5::expression_dict::key_of(name),
            name: name.to_string(),
            short_name: short_name.to_ascii_lowercase(),
            track_count: expr.tracks.len(),
            bone_count: bones.len(),
            variable_count: expr.variables.len(),
            spring_count: expr.motions.len(),
            stream_count: expr.expression_streams.len(),
            instruction_count,
            longest_stream: streams().map(|s| s.size()).max().unwrap_or(0),
            declared_max_stream_size: expr.max_packed_size,
        }
    }
}

/// Every expression set of a `.yed`, in dictionary order.
fn expression_summaries(data: &[u8]) -> Result<Vec<ExpressionSummary>, String> {
    let dict = CrExpressionsDictionary::from_bytes(data).map_err(|e| e.to_string())?;
    let operations = operation_counts(data)?;
    Ok((dict.entries.iter().flatten())
        .zip(operations)
        .map(|(expr, ops)| ExpressionSummary::of(expr, ops))
        .collect())
}

/// Each expression set's operations, one per instruction.
///
/// ⚠ Read from the file: a program's header states its operation count, and
/// the owned value keeps it hidden because the runs are carried as one.
fn operation_counts(data: &[u8]) -> Result<Vec<usize>, String> {
    let (pages, virtual_size) = crate::pages(data)?;
    let image = or_io::desc::Image::new(pages.bytes(), virtual_size);
    let e = |e: or_io::decl::ViewError| e.to_string();
    let dict = View::<Raw<CrExpressionsDictionary>>::at_offset(image, 0).map_err(e)?;
    let entries = dict.entries().map_err(e)?;
    let mut out = Vec::new();
    for i in 0..entries.len() {
        let Some(set) = entries.follow(i).map_err(e)? else {
            continue;
        };
        let streams = set.expression_streams().map_err(e)?;
        let mut operations = 0;
        for j in 0..streams.len() {
            if let Some(stream) = streams.follow(j).map_err(e)? {
                operations += usize::from(stream.num_operations);
            }
        }
        out.push(operations);
    }
    Ok(out)
}

fn info_yed(data: &[u8]) -> Result<(), String> {
    let summaries =
        expression_summaries(data).map_err(|e| format!("Failed to parse YED: {}", e))?;

    println!("Expression dictionary:");
    println!("  Expressions: {}", summaries.len());
    println!();

    if summaries.is_empty() {
        println!("  (empty dictionary)");
        return Ok(());
    }

    println!(
        "  {:<10} {:<28} {:>7} {:>6} {:>5} {:>7} {:>8} {:>9}",
        "Key", "Name", "Tracks", "Bones", "Vars", "Springs", "Programs", "Instrs"
    );
    println!("  {}", "-".repeat(88));
    for s in &summaries {
        println!(
            "  {:#010X} {:<28} {:>7} {:>6} {:>5} {:>7} {:>8} {:>9}",
            s.key,
            truncate_name(&s.short_name, 28),
            s.track_count,
            s.bone_count,
            s.variable_count,
            s.spring_count,
            s.stream_count,
            s.instruction_count,
        );
    }
    println!();
    println!("  The compiled programs are carried verbatim and are not disassembled.");
    Ok(())
}

fn info_ybd(data: &[u8]) -> Result<(), String> {
    let dict =
        or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(data).map_err(|e| {
            // ⚠ The seal's message names the version; this adds what it cannot
            // say, which is that the refusal is about the bounds rather than the
            // dictionary around them.
            match rsc7_version(data).and_then(why_a_version_is_refused) {
                Some(why) => format!("{e}\n  {why}"),
                None => format!("Failed to parse YBD: {e}"),
            }
        })?;

    println!();
    println!("Bound dictionary:");
    println!("  Bounds:       {}", dict.entries.len());
    println!(
        "  Keys sorted:  {}",
        dict.codes.windows(2).all(|w| w[0] <= w[1])
    );
    println!();
    println!("  Name hash    Type");
    println!("  {}", "-".repeat(34));
    for (hash, bound) in dict.codes.iter().zip(&dict.entries) {
        println!("  {hash:#010X}   {}", owned_bound_type(bound.as_deref()));
    }
    Ok(())
}

/// What to tell a user whose `.ybd` is at a version this build does not read:
/// the refusal is about the bounds rather than the dictionary around them.
fn why_a_version_is_refused(version: u32) -> Option<String> {
    use or_io::container::resource::YBD_FILE_VERSION;
    (version != YBD_FILE_VERSION).then(|| {
        format!(
            "a .ybd at version {version} holds phBound at version {version}, and only \
             version {YBD_FILE_VERSION} is described. The dictionary around it would read at \
             any version, which is why this is refused rather than attempted."
        )
    })
}

/// The RSC7 version, for a message about a file this build will not open.
fn rsc7_version(data: &[u8]) -> Option<u32> {
    (data.len() >= 8 && &data[0..4] == b"RSC7")
        .then(|| u32::from_le_bytes(data[4..8].try_into().expect("four bytes")))
}

fn convert_ybd(data: &[u8]) -> Result<Vec<u8>, String> {
    let dict = or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(data)
        .map_err(|e| format!("Failed to parse YBD: {e}"))?;
    println!("  {} bounds", dict.entries.len());
    dict.to_bytes()
        .map_err(|e| format!("Failed to write YBD: {e}"))
}

/// What one frame filter masks, as a flat row.
struct FilterSummary {
    name_hash: u32,
    filter_type_name: &'static str,
    entry_count: usize,
    weight_count: usize,
    bone_count: usize,
    entries_sorted: bool,
}

/// Each filter of a `.yfd` with its hash, in dictionary order.
fn filter_rows(dict: &CrFrameFilterDictionary) -> impl Iterator<Item = FilterSummary> + '_ {
    use or_io::gta5::EFrameFilterType as T;
    (dict.codes.iter().zip(&dict.entries)).filter_map(|(hash, filter)| {
        let filter: &CrFrameFilterAny = filter.as_deref()?;
        let multi = filter.as_multi_weight();
        let kind = match filter {
            CrFrameFilterAny::CrFrameFilterMultiWeight(f) => f.base.r#type,
            CrFrameFilterAny::Base(f) => f.r#type,
        };
        let mut bones: Vec<u16> = multi
            .map(|f| f.track_id_indices.iter().map(|e| e.id).collect())
            .unwrap_or_default();
        bones.sort_unstable();
        bones.dedup();
        Some(FilterSummary {
            name_hash: *hash,
            filter_type_name: match kind.get() {
                T::Bone => "bone",
                T::BoneBasic => "bone-basic",
                T::BoneMultiWeight => "bone-multi-weight",
                T::MultiWeight => "multi-weight",
                T::TrackMultiWeight => "track-multi-weight",
                T::Mover => "mover",
                _ => "unknown",
            },
            entry_count: multi.map_or(0, |f| f.track_id_indices.len()),
            weight_count: multi.map_or(0, |f| f.weights.len()),
            bone_count: bones.len(),
            entries_sorted: multi.is_none_or(|f| {
                (f.track_id_indices.windows(2)).all(|w| w[0].sort_key() <= w[1].sort_key())
            }),
        })
    })
}

fn info_yfd(data: &[u8]) -> Result<(), String> {
    let dict = CrFrameFilterDictionary::from_bytes(data)
        .map_err(|e| format!("Failed to parse YFD: {}", e))?;

    println!("Frame filter dictionary:");
    println!("  Filters: {}", dict.entries.len());
    println!();

    if dict.entries.is_empty() {
        println!("  (empty dictionary)");
        return Ok(());
    }

    println!(
        "  {:<10} {:<20} {:>8} {:>8} {:>6} {:>7}",
        "Hash", "Type", "Entries", "Weights", "Bones", "Sorted"
    );
    println!("  {}", "-".repeat(66));
    for s in filter_rows(&dict) {
        println!(
            "  {:#010X} {:<20} {:>8} {:>8} {:>6} {:>7}",
            s.name_hash,
            s.filter_type_name,
            s.entry_count,
            s.weight_count,
            s.bone_count,
            s.entries_sorted,
        );
    }
    Ok(())
}

/// ⚠ **Legacy only, by nature.** A retail Enhanced install ships **zero**
/// `.fxc` (`openrage-cli formats --ext=fxc`), because Gen9 moved the shader
/// parameter model into the resource itself, so nothing here takes a target
/// parameter.
fn info_fxc(_path: &Path, data: &[u8]) -> Result<(), String> {
    let effect = GrcEffect::parse(data).map_err(|e| format!("Failed to parse FXC: {}", e))?;
    let archive = FxcArchive::parse(data).map_err(|e| format!("Failed to scan FXC: {}", e))?;

    println!("Shader effect:");
    println!("  Vertex type:  {:#010X}", effect.vertex_type);
    if let Some(bucket) = effect.draw_bucket() {
        println!("  Draw bucket:  {bucket}");
    }
    println!("  Presets:      {}", effect.presets.len());
    println!("  Programs:     {}", effect.programs.len());
    println!("  Cbuffers:     {}", effect.cbuffers.len());
    println!("  Variables:    {}", effect.variables.len());
    println!("  Techniques:   {}", effect.techniques.len());
    println!(
        "  DXBC blobs:   {} ({} bytes)",
        archive.len(),
        archive.total_bytes()
    );
    match GrcEffect::trailing_bytes(data) {
        Ok(0) => println!("  Consumed:     every byte"),
        Ok(n) => println!("  Consumed:     all but {n} trailing byte(s)"),
        Err(_) => {}
    }
    println!();

    let counts = archive.stage_counts(data);
    println!(
        "  Blobs by stage: VS {} · PS {} · GS {} · HS {} · DS {} · CS {} · ?? {}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
    );
    println!();

    if effect.variables.is_empty() {
        println!("  (no declared variables)");
        return Ok(());
    }

    println!(
        "  {:<28} {:<28} {:<12} {:>5} {:>5}",
        "Name", "Semantic", "Type", "Count", "Slot"
    );
    println!("  {}", "-".repeat(84));
    for var in &effect.variables {
        println!(
            "  {:<28} {:<28} {:<12} {:>5} {:>5}",
            truncate_name(&var.name, 28),
            truncate_name(&var.semantic, 28),
            format!("{:?}", var.var_type),
            var.count,
            var.slot,
        );
    }
    Ok(())
}

/// ⚠ A native prints as a hash because that is all this build can recover.
/// See `openrage-cli script --natives`, which says so on the same output.
fn info_ysc(data: &[u8]) -> Result<(), String> {
    let program = or_io::gta5::script::ScrProgram::from_bytes(data)
        .map_err(|e| format!("Failed to parse YSC: {e}"))?;

    println!("Script program:");
    println!("  Name:      {}", program.name);
    println!("  Name hash: {:#010X}", program.name_hash);
    println!("  Globals:   signature {:#010X}", program.globals_signature);
    println!(
        "  Code:      {} bytes over {} page(s)",
        program.code_length,
        or_io::gta5::script::ScrProgram::pages_for(program.code_length as usize)
    );
    println!(
        "  Statics:   {} ({} argument(s))",
        program.statics.len(),
        program.parameter_count
    );
    println!("  Natives:   {} call site(s)", program.natives.len());
    println!(
        "  Strings:   {} over {} byte(s)",
        program.strings.len(),
        program.strings_length
    );
    Ok(())
}

fn info_ypdb(_path: &Path, data: &[u8]) -> Result<(), String> {
    let db = PoseMatcherDatabase::from_bytes(data)
        .map_err(|e| format!("Failed to parse YPDB: {}", e))?;

    println!("Pose matcher database:");
    println!("  Signature:   {:#010X}", db.signature);
    println!("  Samples:     {}", db.samples.len());
    println!("  Cloud points:{:>5}", db.point_count());
    println!("  Bone tags:   {}", db.bone_tags.len());
    println!();

    if db.samples.is_empty() {
        println!("  (no samples)");
        return Ok(());
    }

    println!(
        "  {:<12} {:<12} {:>8} {:>8}",
        "Clip set", "Clip", "Offset", "Points"
    );
    println!("  {}", "-".repeat(44));
    for sample in &db.samples {
        println!(
            "  {:#010X}   {:#010X}   {:>7.3}s {:>8}",
            sample.clip_set,
            sample.clip,
            sample.offset,
            sample.cloud.points.len()
        );
    }
    Ok(())
}

/// The animations a clip dictionary holds.
pub(crate) fn animation_count(dict: &CrClipDictionary) -> usize {
    (dict.anim_dictionary.as_deref()).map_or(0, |d| d.animations.entries().count())
}

/// The clips whose first animation is compact, which nothing decodes.
fn compact_clips(dict: &CrClipDictionary) -> usize {
    or_anim::clip::clips(dict)
        .filter(|(_, clip)| {
            or_anim::clip::primary(dict, clip).is_some_and(or_anim::clip::is_compact)
        })
        .count()
}

fn info_ycd(data: &[u8]) -> Result<(), String> {
    let dict =
        CrClipDictionary::from_bytes(data).map_err(|e| format!("Failed to parse YCD: {}", e))?;
    let clips: Vec<(u32, &CrClipAny)> = or_anim::clip::clips(&dict).collect();

    println!("Clip dictionary:");
    println!("  Clips:          {}", clips.len());
    println!("  Animations:     {}", animation_count(&dict));
    println!("  Hash buckets:   {}", dict.clips.hash.len());
    println!("  Base-name keys: {}", dict.base_name_keys.get());
    println!();

    if clips.is_empty() {
        println!("  (empty dictionary)");
        return Ok(());
    }

    println!(
        "  {:<10} {:<28} {:>8} {:>7} {:>5} {:>6} {:>5}",
        "Key", "Name", "Duration", "Frames", "DOFs", "Bones", "Tags"
    );
    println!("  {}", "-".repeat(80));
    for (key, clip) in &clips {
        let header = or_anim::clip::header(clip);
        let animation = or_anim::clip::primary(&dict, clip);
        println!(
            "  {:#010X} {:<28} {:>7.3}s {:>7} {:>5} {:>6} {:>5}",
            key,
            truncate_name(&header.name.data, 28),
            or_anim::clip::duration(clip),
            animation.map_or(0, |a| a.num_frames),
            animation.map_or(0, |a| a.dofs.len()),
            animation.map_or(0, |a| or_anim::clip::bone_ids(a).len()),
            header.tags.as_ref().map_or(0, |t| t.tags.len()),
        );
    }
    println!();

    let compact = compact_clips(&dict);
    if compact > 0 {
        println!(
            "  Note: {compact} clip(s) use compact (compressed) animation blocks. They read and \
             write verbatim but cannot be decoded or sampled."
        );
        println!();
    }

    Ok(())
}

fn info_height_map(data: &[u8]) -> Result<(), String> {
    let map = HeightMap::open(data).map_err(|e| format!("Failed to parse height map: {e}"))?;
    let [cell_x, cell_y] = map.cell_size();
    println!("Height map:");
    println!(
        "  Grid: {} x {} cells of {cell_x} x {cell_y}",
        map.num_cols, map.num_rows
    );
    println!("  Bounds min: {:?}", map.bounds_min);
    println!("  Bounds max: {:?}", map.bounds_max);
    println!(
        "  Cells with a height: {}",
        map.max.iter().filter(|&&h| h != 0).count()
    );
    println!(
        "  Stored: {}{}",
        if map.flags & HeightMap::RLE_DATA != 0 {
            "one span per row"
        } else {
            "the whole grid"
        },
        if map.water_mask.is_empty() {
            ""
        } else {
            ", with a water mask"
        }
    );
    Ok(())
}

fn info_mrf(data: &[u8]) -> Result<(), String> {
    let network =
        MvNetworkDef::open(data).map_err(|e| format!("Failed to parse move network: {e}"))?;
    let counts: Vec<String> = network
        .node_counts()
        .into_iter()
        .map(|(kind, n)| format!("{} {n}", kind.name()))
        .collect();
    let machines = state_machines(&network);
    println!("Move network:");
    println!("  Nodes: {} ({})", network.nodes.len(), counts.join(", "));
    println!(
        "  Requests: {}, flags: {}",
        network.requests.len(),
        network.flags.len()
    );
    println!("  State machines: {}", machines.len());
    for m in machines.iter().take(50) {
        println!("    {:#010x} {}, {} states", m.id, m.kind, m.states.len());
        for (id, transitions) in &m.states {
            println!("      {id:#010x} ({transitions} transitions)");
        }
    }
    if machines.len() > 50 {
        println!("    ... and {} more", machines.len() - 50);
    }
    Ok(())
}

fn info_distant_lights(data: &[u8]) -> Result<(), String> {
    let lights =
        DistantLights::open(data).map_err(|e| format!("Failed to parse distant lights: {e}"))?;
    let groups = &lights.groups;
    println!("Distant lights:");
    println!("  Blocks: {0} x {0}", lights.blocks());
    println!("  Points: {}", lights.points.len());
    println!(
        "  Groups: {} ({} two-way, {} with street lights)",
        groups.len(),
        groups.iter().filter(|g| g.two_way()).count(),
        groups.iter().filter(|g| g.use_streetlights()).count()
    );
    println!();
    println!(
        "  {:>18} {:>6} {:>6} {:>5}  Kind",
        "Centre", "Radius", "Points", "Lanes"
    );
    for g in groups.iter().take(50) {
        println!(
            "  {:>18} {:>6} {:>6} {:>5}  {}",
            format!("{},{},{}", g.centre_x, g.centre_y, g.centre_z),
            g.radius,
            g.point_count,
            g.num_lanes(),
            match (g.use_streetlights(), g.two_way()) {
                (true, _) => "street lights",
                (false, true) => "two-way traffic",
                (false, false) => "one-way traffic",
            }
        );
    }
    if groups.len() > 50 {
        println!("    ... and {} more", groups.len() - 50);
    }
    Ok(())
}

/// The tiles that hold at least one reference.
fn water_tiles(map: &WaterMap) -> usize {
    (0..map.num_tile_rows())
        .flat_map(|row| (0..map.num_tile_cols).map(move |col| (col, row)))
        .filter(|&(col, row)| map.tile_refs(col, row).is_some())
        .count()
}

fn info_water_map(data: &[u8]) -> Result<(), String> {
    let map = WaterMap::open(data).map_err(|e| format!("Failed to parse water height map: {e}"))?;
    println!("Water height map:");
    println!(
        "  Grid: {} x {} tiles of {} x {} from ({}, {}), rows running south",
        map.num_tile_cols,
        map.num_tile_rows(),
        map.tile_size_x,
        map.tile_size_y,
        map.offset_x,
        map.offset_y
    );
    println!("  Tiles with water: {}", water_tiles(&map));
    println!(
        "  Rivers: {} ({} points), lakes: {} ({} boxes), pools: {}",
        map.rivers.len(),
        map.river_points.len(),
        map.lakes.len(),
        map.lake_boxes.len(),
        map.pools.len()
    );
    Ok(())
}

/// The lowest and the highest surface among the water quads, or `None` with none.
fn water_heights(water: &CWaterData) -> Option<[f32; 2]> {
    let heights = water.water_quads.iter().map(|quad| quad.z);
    heights.fold(None, |range, z| match range {
        None => Some([z, z]),
        Some([low, high]) => Some([low.min(z), high.max(z)]),
    })
}

fn info_water_data(data: &[u8]) -> Result<(), String> {
    let water = CWaterData::open(data).map_err(|e| format!("Failed to parse water: {e}"))?;
    println!("Water:");
    println!("  Water quads: {}", water.water_quads.len());
    if let Some([low, high]) = water_heights(&water) {
        println!("  Surfaces from {low} to {high}");
    }
    println!("  Calming quads: {}", water.calming_quads.len());
    println!("  Wave quads: {}", water.wave_quads.len());
    for path in water.unnamed() {
        println!("  Kept as it came: {path}");
    }
    Ok(())
}

fn info_world_blanket(data: &[u8]) -> Result<(), String> {
    let blanket =
        WorldBlanket::open(data).map_err(|e| format!("Failed to parse world blanket: {e}"))?;
    println!("World blanket:");
    println!("  Grid: {0} x {0} cells", blanket.side());
    println!(
        "  Non-zero cells: {} (the file states no meaning for a cell)",
        blanket.cells.iter().filter(|&&c| c != 0).count()
    );
    Ok(())
}

fn info_nav_precalc(data: &[u8]) -> Result<(), String> {
    let precalc =
        NavPrecalc::open(data).map_err(|e| format!("Failed to parse nav precalc: {e}"))?;
    println!("Nav precalc:");
    println!("  Header values: {:?}", precalc.header);
    println!(
        "  Cells: {} ({} with entries, {} entries)",
        precalc.cells.len(),
        precalc.cells.iter().filter(|c| c.entries() > 0).count(),
        precalc.cells.iter().map(|c| c.entries()).sum::<usize>()
    );
    println!("  BITS block: {} bytes", precalc.bits.len());
    Ok(())
}

/// A cache file's, `metadata.dat`'s or the audio sector grid's summary.
fn dat_table_json(subject: Subject, data: &[u8]) -> Result<Value, String> {
    match subject {
        Subject::CacheFile => {
            let cache = CacheFile::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "version": cache.version,
                "fileDates": cache.file_dates.len(),
                "mapDataStore": cache.map_data_store.len(),
                "interiorProxies": cache.interior_proxies.len(),
                "boundsStore": cache.bounds_store.len(),
            }))
        }
        Subject::SocialClubMetadata => {
            let metadata = SocialClubMetadata::open(data).map_err(|e| e.to_string())?;
            Ok(json!({
                "formatWord": metadata.format_word,
                "built": metadata.built,
                "bodyBytes": metadata.body.len(),
            }))
        }
        Subject::AudioWorldSectors => {
            let grid = AudioWorldSectors::open(data).map_err(|e| e.to_string())?;
            let count = |f: fn(&AudSector) -> bool| grid.sectors.iter().filter(|s| f(s)).count();
            Ok(json!({
                "sectors": grid.sectors.len(),
                "waterSectors": count(|s| s.is_water_sector),
                "withBuildings": count(|s| s.num_buildings != 0),
                "withTrees": count(|s| s.num_trees != 0),
                "withHighwayNodes": count(|s| s.num_highway_nodes != 0),
            }))
        }
        other => Err(format!("{} is not a .dat table", other.name())),
    }
}

/// A cache file, `metadata.dat` or the audio sector grid, opened and saved back.
fn resave_dat_table(subject: Subject, data: &[u8]) -> Result<Vec<u8>, String> {
    match subject {
        Subject::CacheFile => CacheFile::open(data).and_then(|c| c.save()),
        Subject::SocialClubMetadata => SocialClubMetadata::open(data).and_then(|m| m.save()),
        Subject::AudioWorldSectors => AudioWorldSectors::open(data).and_then(|g| g.save()),
        other => return Err(format!("{} is not a .dat table", other.name())),
    }
    .map_err(|e| e.to_string())
}

fn info_gxt2(data: &[u8]) -> Result<(), String> {
    let table = Gxt2::from_bytes(data).map_err(|e| format!("Failed to parse GXT2: {}", e))?;

    println!("Global Text Table:");
    println!("  Entry count: {}", table.len());
    println!();

    if table.is_empty() {
        println!("  (empty table)");
    } else {
        println!("  {:<12} Value", "Hash");
        println!("  {}", "-".repeat(60));
        for entry in table.entries.iter().take(50) {
            println!(
                "  {:#010X}  {}",
                entry.hash,
                truncate_name(&entry.value, 48)
            );
        }
        if table.len() > 50 {
            println!("    ... and {} more", table.len() - 50);
        }
    }

    Ok(())
}

fn cmd_convert(input: &Path, output: &Path) -> Result<(), String> {
    let t_total = Instant::now();

    let t0 = Instant::now();
    let input_data = fs::read(input).map_err(|e| format!("Failed to read input: {}", e))?;

    let resource_type = Subject::from_extension(input)
        .ok_or_else(|| format!("Unknown input file type: {}", input.display()))?;

    if let Some(out_type) = Subject::from_extension(output) {
        if out_type != resource_type {
            return Err(format!(
                "Output type ({:?}) must match input type ({:?})",
                out_type, resource_type
            ));
        }
    }

    println!("Converting: {} -> {}", input.display(), output.display());
    println!("Type: {}", resource_type.name());
    let t_read_file = t0.elapsed();
    println!("Input size: {} bytes", input_data.len());

    let output_data = match resource_type {
        Subject::Resource(ResourceType::Ytd) => convert_ytd(input, &input_data)?,
        Subject::Resource(ResourceType::Ydr) => convert_ydr(&input_data)?,
        Subject::Resource(ResourceType::Ydd) => convert_ydd(&input_data)?,
        Subject::Resource(ResourceType::Yft) => convert_yft(&input_data)?,
        Subject::Resource(ResourceType::Ypt) => convert_ypt(&input_data)?,
        Subject::Resource(ResourceType::Ybn) => convert_ybn(&input_data)?,
        Subject::Resource(ResourceType::Yld) => convert_yld(&input_data)?,
        Subject::Resource(ResourceType::Ycd) => convert_ycd(&input_data)?,
        Subject::Resource(ResourceType::Yed) => convert_yed(&input_data)?,
        Subject::Resource(ResourceType::Yfd) => convert_yfd(&input_data)?,
        Subject::Resource(ResourceType::Ybd) => convert_ybd(&input_data)?,
        Subject::Resource(ResourceType::Ynd) => or_io::gta5::CPathRegion::from_bytes(&input_data)
            .map_err(|e| format!("Failed to parse YND: {}", e))?
            .to_bytes()
            .map_err(|e| format!("Failed to write YND: {}", e))?,
        Subject::Resource(ResourceType::Ynv) => or_io::gta5::CNavMesh::from_bytes(&input_data)
            .map_err(|e| format!("Failed to parse YNV: {}", e))?
            .to_bytes()
            .map_err(|e| format!("Failed to write YNV: {}", e))?,
        Subject::Resource(ResourceType::Yvr) => CVehicleRecording::from_bytes(&input_data)
            .map_err(|e| format!("Failed to parse YVR: {}", e))?
            .to_bytes()
            .map_err(|e| format!("Failed to write YVR: {}", e))?,
        Subject::Resource(ResourceType::Ywr) => FwWaypointRecordingRoute::from_bytes(&input_data)
            .map_err(|e| format!("Failed to parse YWR: {}", e))?
            .to_bytes()
            .map_err(|e| format!("Failed to write YWR: {}", e))?,
        unread @ Subject::Resource(ResourceType::Ypm) => {
            return Err(no_reader(unread));
        }
        Subject::Gxt2 => convert_gxt2(input, &input_data)?,
        Subject::Ypdb => convert_ypdb(input, &input_data)?,
        Subject::HeightMap => HeightMap::open(&input_data)
            .and_then(|map| map.save())
            .map_err(|e| e.to_string())?,
        Subject::DistantLights => DistantLights::open(&input_data)
            .and_then(|lights| lights.save())
            .map_err(|e| e.to_string())?,
        Subject::WaterMap => WaterMap::open(&input_data)
            .and_then(|map| map.save())
            .map_err(|e| e.to_string())?,
        Subject::WaterData => CWaterData::open(&input_data)
            .and_then(|water| water.save())
            .map_err(|e| e.to_string())?,
        Subject::WorldBlanket => WorldBlanket::open(&input_data)
            .map_err(|e| e.to_string())?
            .save(),
        Subject::NavPrecalc => NavPrecalc::open(&input_data)
            .and_then(|precalc| precalc.save())
            .map_err(|e| e.to_string())?,
        Subject::Mrf => MvNetworkDef::open(&input_data)
            .and_then(|network| network.save())
            .map_err(|e| e.to_string())?,
        Subject::CacheFile | Subject::SocialClubMetadata | Subject::AudioWorldSectors => {
            resave_dat_table(resource_type, &input_data)?
        }
        // ⚠ `.ysc` reads and has no writer, deliberately: nothing here can
        // author bytecode, so a writer would only echo what the reader just
        // read. See `or_io::gta5::script`.
        readonly @ (Subject::Fxc | Subject::Resource(ResourceType::Ysc)) => {
            return Err(no_writer(readonly))
        }
    };

    println!("Output size: {} bytes", output_data.len());

    let size_diff = output_data.len() as i64 - input_data.len() as i64;
    let size_pct = (output_data.len() as f64 / input_data.len() as f64) * 100.0;
    println!(
        "Size difference: {:+} bytes ({:.1}% of original)",
        size_diff, size_pct
    );

    let t0 = Instant::now();
    fs::write(output, &output_data).map_err(|e| format!("Failed to write output: {}", e))?;
    let t_write_file = t0.elapsed();

    println!();
    println!("Verification: Re-reading output...");
    let t0 = Instant::now();
    verify_data(resource_type, output, &output_data)?;
    let t_verify = t0.elapsed();

    println!();
    println!("Timing:");
    print_step_timing("read file", t_read_file);
    print_step_timing("write file", t_write_file);
    print_step_timing("verify", t_verify);
    print_step_timing("total", t_total.elapsed());

    println!();
    println!("Success! Output written to: {}", output.display());

    Ok(())
}

fn convert_ytd(path: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict =
        Listing::from_bytes(path, data).map_err(|e| format!("Failed to parse YTD: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        texture_count = dict.textures().len(),
        "Parsed texture dictionary"
    );

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YTD: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ydr(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let drawable =
        GtaDrawable::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YDR: {}", e))?;
    let t_parse = t0.elapsed();

    info!(name = %drawable.name, has_geometry = has_models(&drawable), "Parsed drawable");

    let t_write = Instant::now();
    let output = drawable
        .to_bytes()
        .map_err(|e| format!("Failed to write YDR: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ydd(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict = PgDictionary::<GtaDrawable>::from_bytes(data)
        .map_err(|e| format!("Failed to parse YDD: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        drawable_count = dict.entries.len(),
        with_geometry = entries_of(&dict).filter(|(_, d)| has_models(d)).count(),
        "Parsed drawable dictionary"
    );

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YDD: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_yft(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let frag =
        GtaFragType::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YFT: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        name = %frag.tune_name,
        drawables = frag.drawables().len(),
        bones = fragment_bones(&frag),
        physics_children = frag.child_count(),
        "Parsed fragment"
    );

    let t_write = Instant::now();
    let output = frag
        .to_bytes()
        .map_err(|e| format!("Failed to write YFT: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ybn(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let bound = or_io::gta5::PhBoundAny::from_bytes(data)
        .map_err(|e| format!("Failed to parse YBN: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        bound_type = ?bound_header(&bound).r#type,
        "Parsed physics bound"
    );

    let t_write = Instant::now();
    let output = bound
        .to_bytes()
        .map_err(|e| format!("Failed to write YBN: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_yld(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict = or_io::gta5::PgDictionary::<or_io::gta5::CharacterCloth>::from_bytes(data)
        .map_err(|e| format!("Failed to parse YLD: {}", e))?;
    let t_parse = t0.elapsed();

    info!(cloth_count = dict.entries.len(), "Parsed cloth dictionary");

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YLD: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_yed(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict = or_io::gta5::CrExpressionsDictionary::from_bytes(data)
        .map_err(|e| format!("Failed to parse YED: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        expression_count = dict.entries.len(),
        "Parsed expression dictionary"
    );

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YED: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_yfd(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict = or_io::gta5::CrFrameFilterDictionary::from_bytes(data)
        .map_err(|e| format!("Failed to parse YFD: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        filter_count = dict.entries.len(),
        "Parsed frame filter dictionary"
    );

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YFD: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ypdb(_path: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let db = PoseMatcherDatabase::from_bytes(data)
        .map_err(|e| format!("Failed to parse YPDB: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        sample_count = db.samples.len(),
        "Parsed pose matcher database"
    );

    let t_write = Instant::now();
    let output = db
        .to_bytes()
        .map_err(|e| format!("Failed to write YPDB: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ycd(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let dict =
        CrClipDictionary::from_bytes(data).map_err(|e| format!("Failed to parse YCD: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        clip_count = or_anim::clip::clips(&dict).count(),
        animation_count = animation_count(&dict),
        "Parsed clip dictionary"
    );

    let t_write = Instant::now();
    let output = dict
        .to_bytes()
        .map_err(|e| format!("Failed to write YCD: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_ypt(data: &[u8]) -> Result<Vec<u8>, String> {
    let t0 = Instant::now();
    let list =
        PtxFxList::<Gen8>::from_bytes(data).map_err(|e| format!("Failed to parse YPT: {}", e))?;
    let t_parse = t0.elapsed();

    info!(
        effect_count = effect_names(&list).len(),
        has_texture_dictionary = list.texture_dict.is_some(),
        has_drawable_dictionary = list.model_dict.is_some(),
        "Parsed particle effects"
    );

    let t_write = Instant::now();
    let output = list
        .to_bytes()
        .map_err(|e| format!("Failed to write YPT: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn convert_gxt2(_path: &Path, data: &[u8]) -> Result<Vec<u8>, String> {
    // GXT2 is a plain binary blob (not RSC7), so there's no decompress step.
    let t0 = Instant::now();
    let table = Gxt2::from_bytes(data).map_err(|e| format!("Failed to parse GXT2: {}", e))?;
    let t_parse = t0.elapsed();

    info!(entry_count = table.len(), "Parsed global text table");

    let t_write = Instant::now();
    let output = table
        .to_bytes()
        .map_err(|e| format!("Failed to write GXT2: {}", e))?;
    let t_write = t_write.elapsed();

    print_convert_timing(Duration::ZERO, t_parse, t_write);
    Ok(output)
}

fn cmd_verify(path: &Path, json: bool) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;

    let resource_type = Subject::from_extension(path)
        .ok_or_else(|| format!("Unknown file type: {}", path.display()))?;

    if json {
        let base = json!({
            "file": path.display().to_string(),
            "type": resource_type.name(),
            "size": data.len(),
        });
        let mut doc = base;
        match verify_stats(resource_type, path, &data) {
            Ok(stats) => {
                let m = doc.as_object_mut().unwrap();
                m.insert("valid".into(), json!(true));
                if let Value::Object(s) = stats {
                    for (k, v) in s {
                        m.insert(k, v);
                    }
                }
                if let Some(fixups) = fixup_summary(path, &data) {
                    m.insert("fixups".into(), fixups);
                }
                print_json_doc(&doc);
                return Ok(());
            }
            Err(e) => {
                let m = doc.as_object_mut().unwrap();
                m.insert("valid".into(), json!(false));
                m.insert("error".into(), json!(e.clone()));
                print_json_doc(&doc);
                return Err(e);
            }
        }
    }

    println!("Verifying: {}", path.display());
    println!("Type: {}", resource_type.name());
    println!("Size: {} bytes", data.len());

    verify_data(resource_type, path, &data)?;

    if let Ok(report) = super::fixups::scan_bytes(path, &data) {
        if report.is_clean() {
            println!("  OK - no aliased geometry buffers, no page over the game's limit");
        }
        if !report.aliased_blocks.is_empty() {
            println!(
                "  WARNING - {} aliased geometry block(s); the game would reject this file",
                report.aliased_blocks.len()
            );
            for block in &report.aliased_blocks {
                println!("    {block}");
            }
            println!("    Repair with: openrage-cli convert repair <in> <out>");
        }
        for page in &report.oversized_pages {
            println!("  WARNING - {page}; the game would reject this file");
        }
        if !report.oversized_pages.is_empty() {
            println!("    Repair with: openrage-cli convert repair <in> <out> --drop-copies");
        }
    }

    println!();
    println!("Verification passed!");

    Ok(())
}

fn fixup_summary(path: &Path, data: &[u8]) -> Option<Value> {
    let report = super::fixups::scan_bytes(path, data).ok()?;
    Some(json!({
        "clean": report.is_clean(),
        "aliasedBlocks": report.aliased_blocks.len(),
        "oversizedPages": report.oversized_pages.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        "blocks": report
            .aliased_blocks
            .iter()
            .map(|b| json!({
                "sourceAddress": format!("{:#X}", b.source_address),
                "kind": b.kind.to_string(),
                "sites": b.sites.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>(),
    }))
}

fn cmd_repair(
    input: &Path,
    output: &Path,
    drop_copies: bool,
    fix_counts: bool,
    fix_palettes: bool,
    place: Option<or_interchange::doctor::place::Frame>,
) -> Result<(), String> {
    let data = fs::read(input).map_err(|e| format!("Failed to read file: {}", e))?;

    // ⚠ The scan reads the file as the game would, and refuses a model whose counts contradict their runs: with --fix-counts that is what is being mended.
    let before = match super::fixups::scan_bytes(input, &data) {
        Ok(report) => report,
        Err(why) if fix_counts => {
            println!("  not scanned before the repair: {why}");
            super::fixups::FixupReport::default()
        }
        Err(why) => return Err(why),
    };
    println!("Repairing: {}", input.display());
    if before.aliased_blocks.is_empty() {
        println!("  no aliased geometry buffers");
    } else {
        println!("  {} aliased block(s) found", before.aliased_blocks.len());
        for block in &before.aliased_blocks {
            println!("    {block}");
        }
    }
    for page in &before.oversized_pages {
        println!("  {page}");
    }
    if !before.oversized_pages.is_empty() && !drop_copies {
        println!("  a level that copies the high one is the usual cause: --drop-copies removes it");
    }

    let (outcome, bytes) =
        super::fixups::repair_bytes(input, &data, drop_copies, fix_counts, fix_palettes, place)?;
    fs::write(output, &bytes).map_err(|e| format!("Failed to write output: {}", e))?;

    println!("  {}", outcome.describe());
    if let Ok(after) = super::fixups::scan_bytes(output, &bytes) {
        for page in &after.oversized_pages {
            println!(
                "  still {page}: no level copies the high one, so the model itself is too large"
            );
        }
        if !before.oversized_pages.is_empty() && after.oversized_pages.is_empty() {
            println!("  the page now fits the game's limit");
        }
    }
    println!(
        "Wrote {} ({} bytes, was {})",
        output.display(),
        bytes.len(),
        data.len()
    );
    Ok(())
}

fn verify_stats(resource_type: Subject, path: &Path, data: &[u8]) -> Result<Value, String> {
    let enhanced = resource_type
        .as_resource()
        .is_some_and(|r| crate::target_of_resource(r, data).is_gen9());

    match resource_type {
        Subject::Resource(ResourceType::Ytd) => {
            let dict =
                Listing::from_bytes(path, data).map_err(|e| format!("Parse failed: {}", e))?;
            let textures = dict.textures().len();
            Ok(match enhanced {
                true => json!({ "textures": textures, "edition": "Enhanced" }),
                false => json!({ "textures": textures }),
            })
        }
        Subject::Resource(ResourceType::Ydr) if enhanced => {
            let d = GtaDrawable::<Gen9>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "name": d.name,
                "hasGeometry": has_models(&d),
                "edition": "Enhanced",
            }))
        }
        Subject::Resource(ResourceType::Ydr) => {
            let d = GtaDrawable::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "name": d.name, "hasGeometry": has_models(&d) }))
        }
        Subject::Resource(ResourceType::Ydd) if enhanced => {
            let dict = PgDictionary::<GtaDrawable<Gen9>>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            let each = || dict.entries.iter().flatten();
            Ok(json!({
                "drawables": each().count(),
                "withGeometry": each().filter(|d| has_models(d)).count(),
                "edition": "Enhanced",
            }))
        }
        Subject::Resource(ResourceType::Ydd) => {
            let dict = PgDictionary::<GtaDrawable>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "drawables": entries_of(&dict).count(),
                "withGeometry": entries_of(&dict).filter(|(_, d)| has_models(d)).count(),
            }))
        }
        Subject::Resource(ResourceType::Yft) if enhanced => {
            let frag = GtaFragType::<Gen9>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "name": frag.tune_name,
                "drawables": frag.drawables().len(),
                "bones": fragment_bones(&frag),
                "physicsChildren": frag.child_count(),
            }))
        }
        Subject::Resource(ResourceType::Yft) => {
            let frag = GtaFragType::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "name": frag.tune_name,
                "drawables": frag.drawables().len(),
                "bones": fragment_bones(&frag),
                "physicsChildren": frag.child_count(),
            }))
        }
        Subject::Resource(ResourceType::Ypt) => {
            let list =
                PtxFxList::<Gen8>::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "effects": effect_names(&list).len(),
                "textures": list.texture_dict.as_ref().map_or(0, |d| d.entries.len()),
                "drawables": list.model_dict.as_ref().map_or(0, |d| d.entries.len()),
            }))
        }
        Subject::Resource(ResourceType::Ybn) => {
            let b = or_io::gta5::PhBoundAny::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "boundType": format!("{:?}", bound_header(&b).r#type) }))
        }
        Subject::Resource(ResourceType::Yld) => {
            let r = cloth_report(data).map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "cloths": r.entry_count,
                "vertices": r.total_vertices(),
                "edges": r.total_edges(),
                "warnings": r.diagnostics.len(),
            }))
        }
        Subject::Resource(ResourceType::Ycd) => {
            let d =
                CrClipDictionary::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "clips": or_anim::clip::clips(&d).count(),
                "animations": animation_count(&d),
                "buckets": d.clips.hash.len(),
            }))
        }
        Subject::Resource(ResourceType::Yed) => {
            let d = or_io::gta5::CrExpressionsDictionary::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            let each = || d.entries.iter().flatten();
            Ok(json!({
                "expressions": d.entries.len(),
                "streams": each().map(|e| e.expression_streams.len()).sum::<usize>(),
                "tracks": each().map(|e| e.tracks.len()).sum::<usize>(),
            }))
        }
        Subject::Resource(ResourceType::Ybd) => {
            let d = or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "bounds": d.entries.len() }))
        }
        Subject::Resource(ResourceType::Ynv) => {
            let m = MeshSummary::of(
                &CNavMesh::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?,
            );
            Ok(json!({
                "vertices": m.vertices,
                "polys": m.polys,
                "indices": m.indices,
            }))
        }
        Subject::Resource(ResourceType::Yfd) => {
            let d = or_io::gta5::CrFrameFilterDictionary::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "filters": d.entries.len(),
                "entries": d
                    .entries
                    .iter()
                    .flatten()
                    .filter_map(|f| f.as_multi_weight())
                    .map(|f| f.track_id_indices.len())
                    .sum::<usize>(),
            }))
        }
        Subject::Resource(ResourceType::Ynd) => {
            let region = or_io::gta5::CPathRegion::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "nodes": region.nodes.len(),
                "links": region.links.len(),
                "junctions": region.virtual_junctions.len(),
            }))
        }
        Subject::Resource(ResourceType::Yvr) => {
            let recording =
                CVehicleRecording::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "frames": recording.states.len() }))
        }
        Subject::Resource(ResourceType::Ywr) => {
            let route = FwWaypointRecordingRoute::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "waypoints": route.waypoints.len() }))
        }
        Subject::Gxt2 => {
            let t = Gxt2::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({ "entries": t.len() }))
        }
        Subject::Fxc => {
            let effect = GrcEffect::parse(data).map_err(|e| format!("Parse failed: {}", e))?;
            let archive = FxcArchive::parse(data).map_err(|e| format!("Scan failed: {}", e))?;
            Ok(json!({
                "programs": effect.programs.len(),
                "variables": effect.variables.len(),
                "dxbcBlobs": archive.len(),
            }))
        }
        Subject::Ypdb => {
            let db = PoseMatcherDatabase::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            Ok(json!({
                "samples": db.samples.len(),
                "points": db.point_count(),
                "boneTags": db.bone_tags.len(),
            }))
        }
        Subject::Resource(ResourceType::Ysc) => {
            let program = or_io::gta5::script::ScrProgram::from_bytes(data)
                .map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({
                "name": program.name,
                "codeLength": program.code_length,
                "statics": program.statics.len(),
                "natives": program.natives.len(),
                "strings": program.strings.len(),
            }))
        }
        Subject::HeightMap => {
            let map = HeightMap::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({ "columns": map.num_cols, "rows": map.num_rows }))
        }
        Subject::DistantLights => {
            let lights = DistantLights::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({ "points": lights.points.len(), "groups": lights.groups.len() }))
        }
        Subject::WaterMap => {
            let map = WaterMap::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({ "columns": map.num_tile_cols, "rows": map.num_tile_rows() }))
        }
        Subject::WaterData => {
            let water = CWaterData::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({
                "waterQuads": water.water_quads.len(),
                "calmingQuads": water.calming_quads.len(),
                "waveQuads": water.wave_quads.len(),
            }))
        }
        Subject::WorldBlanket => {
            let blanket = WorldBlanket::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({ "side": blanket.side() }))
        }
        Subject::NavPrecalc => {
            let precalc = NavPrecalc::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(json!({ "cells": precalc.cells.len(), "bitsBytes": precalc.bits.len() }))
        }
        Subject::Mrf => {
            let network = MvNetworkDef::open(data).map_err(|e| format!("Parse failed: {e}"))?;
            Ok(
                json!({ "nodes": network.nodes.len(), "stateMachines": state_machines(&network).len() }),
            )
        }
        Subject::CacheFile | Subject::SocialClubMetadata | Subject::AudioWorldSectors => {
            dat_table_json(resource_type, data).map_err(|e| format!("Parse failed: {e}"))
        }
        unread @ Subject::Resource(ResourceType::Ypm) => Err(no_reader(unread)),
    }
}

/// A format's structural checks over a file's own pages, which an open does not run.
fn checked<E: std::fmt::Display>(
    data: &[u8],
    check: impl FnOnce(or_io::desc::Image<'_>, usize) -> Result<(), E>,
) -> Result<(), String> {
    let (pages, virtual_size) = crate::pages(data)?;
    check(or_io::desc::Image::new(pages.bytes(), virtual_size), 0)
        .map_err(|e| format!("Validation failed: {e}"))
}

fn verify_data(resource_type: Subject, path: &Path, data: &[u8]) -> Result<(), String> {
    match resource_type {
        Subject::Resource(ResourceType::Ytd) => {
            let dict =
                Listing::from_bytes(path, data).map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid YTD ({} textures)", dict.textures().len());
        }
        Subject::Resource(ResourceType::Ydr) => {
            let drawable = GtaDrawable::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YDR (name: {}, has_geometry: {})",
                drawable.name,
                has_models(&drawable)
            );
        }
        Subject::Resource(ResourceType::Ydd) => {
            let dict = PgDictionary::<GtaDrawable>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YDD ({} drawables, {} with geometry)",
                entries_of(&dict).count(),
                entries_of(&dict).filter(|(_, d)| has_models(d)).count()
            );
        }
        Subject::Resource(ResourceType::Yft)
            if crate::target_of_resource(ResourceType::Yft, data).is_gen9() =>
        {
            let frag = GtaFragType::<Gen9>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YFT, Enhanced (name: {}, drawables: {}, bones: {}, physics children: {})",
                frag.tune_name,
                frag.drawables().len(),
                fragment_bones(&frag),
                frag.child_count()
            );
        }
        Subject::Resource(ResourceType::Yft) => {
            let frag = GtaFragType::<Gen8>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YFT (name: {}, drawables: {}, bones: {}, physics children: {})",
                frag.tune_name,
                frag.drawables().len(),
                fragment_bones(&frag),
                frag.child_count()
            );
        }
        Subject::Resource(ResourceType::Ypt) => {
            let list =
                PtxFxList::<Gen8>::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YPT ({} effects, textures: {}, drawables: {})",
                effect_names(&list).len(),
                list.texture_dict.as_ref().map_or(0, |d| d.entries.len()),
                list.model_dict.as_ref().map_or(0, |d| d.entries.len())
            );
        }
        Subject::Resource(ResourceType::Ybn) => {
            let bound = or_io::gta5::PhBoundAny::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid YBN (type: {:?})", bound_header(&bound).r#type);
        }
        Subject::Resource(ResourceType::Ynd) => {
            let region =
                CPathRegion::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            // The reader does not run these checks; only a write does.
            checked(data, or_io::gta5::path_region::validate)?;
            println!(
                "  OK - Valid YND ({} nodes, {} links, {} junctions)",
                region.nodes.len(),
                region.links.len(),
                region.virtual_junctions.len()
            );
        }
        Subject::Resource(ResourceType::Yld) => {
            let report = cloth_report(data).map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YLD ({} cloth, {} vertices, {} constraints)",
                report.entry_count,
                report.total_vertices(),
                report.total_edges()
            );
            for finding in report.diagnostics.messages() {
                println!("  WARN - {finding}");
            }
        }
        Subject::Resource(ResourceType::Ycd) => {
            let dict =
                CrClipDictionary::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            checked(data, or_io::gta5::clip_dict::validate)?;
            println!(
                "  OK - Valid YCD ({} clips, {} animations, {} hash buckets)",
                or_anim::clip::clips(&dict).count(),
                animation_count(&dict),
                dict.clips.hash.len()
            );
            let compact = compact_clips(&dict);
            if compact > 0 {
                println!("  WARN - {compact} clip(s) use compact blocks and cannot be decoded");
            }
        }
        Subject::Resource(ResourceType::Yed) => {
            let summaries =
                expression_summaries(data).map_err(|e| format!("Parse failed: {}", e))?;
            let streams: usize = summaries.iter().map(|s| s.stream_count).sum();
            println!(
                "  OK - Valid YED ({} expressions, {streams} programs)",
                summaries.len()
            );
            // The engine allocates one scratch buffer of this size per
            // expression, so a stream longer than the declared maximum
            // overruns it at evaluation time rather than at load.
            for s in &summaries {
                if s.longest_stream > s.declared_max_stream_size as usize {
                    println!(
                        "  WARN - {}: a program is {} bytes and max_stream_size is {}",
                        s.short_name, s.longest_stream, s.declared_max_stream_size
                    );
                }
            }
        }
        Subject::Resource(ResourceType::Ybd) => {
            let dict = or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid YBD ({} bounds)", dict.entries.len());
            // The engine binary-searches the key table, so an unsorted one
            // loads and reports bounds it holds as absent.
            if dict.codes.windows(2).any(|w| w[0] > w[1]) {
                println!("  WARNING: keys are not ascending; lookups will miss");
            }
        }
        Subject::Resource(ResourceType::Ynv) => {
            let mesh = CNavMesh::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            // The reader does not run these checks; only a write does.
            checked(data, or_io::gta5::navmesh::validate)?;
            let s = MeshSummary::of(&mesh);
            println!(
                "  OK - Valid YNV ({} polygons, {} vertices, {} indices)",
                s.polys, s.vertices, s.indices
            );
        }
        Subject::Resource(ResourceType::Yfd) => {
            let dict = CrFrameFilterDictionary::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            let rows: Vec<FilterSummary> = filter_rows(&dict).collect();
            let entries: usize = rows.iter().map(|s| s.entry_count).sum();
            println!(
                "  OK - Valid YFD ({} filters, {entries} entries)",
                dict.entries.len()
            );
            // The engine binary-searches the entry table, so an unsorted one
            // loads and masks the wrong bones.
            for s in rows.iter().filter(|s| !s.entries_sorted) {
                println!("  WARN - {:#010X}: entry table is not sorted", s.name_hash);
            }
        }
        Subject::Resource(ResourceType::Yvr) => {
            let recording =
                CVehicleRecording::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid YVR ({} frames)", recording.states.len());
        }
        Subject::Resource(ResourceType::Ywr) => {
            let route = FwWaypointRecordingRoute::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid YWR ({} waypoints)", route.waypoints.len());
        }
        Subject::Gxt2 => {
            let table = Gxt2::from_bytes(data).map_err(|e| format!("Parse failed: {}", e))?;
            println!("  OK - Valid GXT2 ({} entries)", table.len());
        }
        Subject::HeightMap
        | Subject::DistantLights
        | Subject::WaterMap
        | Subject::WaterData
        | Subject::WorldBlanket
        | Subject::NavPrecalc
        | Subject::Mrf
        | Subject::CacheFile
        | Subject::SocialClubMetadata
        | Subject::AudioWorldSectors => {
            let stats = verify_stats(resource_type, path, data)?;
            println!("  OK - Valid {} ({stats})", resource_type.name());
        }
        Subject::Fxc => {
            let effect = GrcEffect::parse(data).map_err(|e| format!("Parse failed: {}", e))?;
            let archive = FxcArchive::parse(data).map_err(|e| format!("Scan failed: {}", e))?;
            println!(
                "  OK - Valid FXC ({} programs, {} variables, {} DXBC blobs)",
                effect.programs.len(),
                effect.variables.len(),
                archive.len()
            );
            // A `.fxc` carries no table of contents, so the only thing that
            // says the parse landed on the right fields is that it consumed
            // the file exactly.
            match GrcEffect::trailing_bytes(data) {
                Ok(0) => {}
                Ok(n) => println!("  WARN - {n} byte(s) past the end of the effect"),
                Err(e) => println!("  WARN - could not measure the consume: {e}"),
            }
        }
        Subject::Ypdb => {
            let db = PoseMatcherDatabase::from_bytes(data)
                .map_err(|e| format!("Parse failed: {}", e))?;
            println!(
                "  OK - Valid YPDB ({} samples, {} cloud points, {} bone tags)",
                db.samples.len(),
                db.point_count(),
                db.bone_tags.len()
            );
        }
        Subject::Resource(ResourceType::Ysc) => {
            let program = or_io::gta5::script::ScrProgram::from_bytes(data)
                .map_err(|e| format!("Parse failed: {e}"))?;
            println!(
                "  OK - Valid YSC (name: {}, {} bytes of code, {} native call(s))",
                program.name,
                program.code_length,
                program.natives.len()
            );
        }
        unread @ Subject::Resource(ResourceType::Ypm) => {
            return Err(no_reader(unread));
        }
    }
    Ok(())
}

fn cmd_batch(args: &[&String], json: bool) -> ExitCode {
    let mut dir: Option<&str> = None;
    let mut exts: Vec<String> = Vec::new();
    let mut out_dir: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(list) = a.strip_prefix("--ext=") {
            exts = list
                .split(',')
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
        } else if a == "--out" {
            i += 1;
            match args.get(i) {
                Some(v) => out_dir = Some(v.as_str()),
                None => {
                    eprintln!("Error: --out needs a directory");
                    return ExitCode::from(2);
                }
            }
        } else if a.starts_with("--") {
            eprintln!("Error: unknown option {a}");
            return ExitCode::from(2);
        } else if dir.is_none() {
            dir = Some(a);
        } else {
            eprintln!("Error: unexpected argument {a}");
            return ExitCode::from(2);
        }
        i += 1;
    }

    let Some(dir) = dir else {
        eprintln!("Error: Missing directory argument");
        eprintln!("Usage: openrage-cli convert batch <dir> [--ext=a,b] [--out <dir>] [--json]");
        return ExitCode::from(2);
    };

    let mut files = Vec::new();
    if let Err(e) = collect_convertible(Path::new(dir), &exts, &mut files) {
        eprintln!("Error: {e}");
        return ExitCode::from(1);
    }
    files.sort();

    if let Some(out) = out_dir {
        if let Err(e) = fs::create_dir_all(out) {
            eprintln!("Error: create {out}: {e}");
            return ExitCode::from(1);
        }
    }

    let mut results = Vec::new();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut total_in = 0u64;
    let mut total_out = 0u64;

    for path in &files {
        let name = path.display().to_string();
        let rt = match Subject::from_extension(path) {
            Some(rt) => rt,
            None => continue,
        };
        let outcome = roundtrip_one(rt, path, out_dir);
        match outcome {
            Ok((in_len, out_len)) => {
                passed += 1;
                total_in += in_len as u64;
                total_out += out_len as u64;
                let delta = (out_len as f64 - in_len as f64) / in_len.max(1) as f64 * 100.0;
                if json {
                    results.push(json!({
                        "file": name, "ok": true, "type": rt.name(),
                        "inBytes": in_len, "outBytes": out_len, "deltaPct": delta,
                    }));
                } else {
                    println!("  PASS  {name}  {in_len} -> {out_len} ({delta:+.1}%)");
                }
            }
            Err(e) => {
                failed += 1;
                if json {
                    results
                        .push(json!({ "file": name, "ok": false, "type": rt.name(), "error": e }));
                } else {
                    println!("  FAIL  {name}: {e}");
                }
            }
        }
    }

    if json {
        let delta = if total_in > 0 {
            (total_out as f64 - total_in as f64) / total_in as f64 * 100.0
        } else {
            0.0
        };
        print_json_doc(&json!({
            "directory": dir,
            "total": files.len(),
            "passed": passed,
            "failed": failed,
            "inBytes": total_in,
            "outBytes": total_out,
            "deltaPct": delta,
            "results": results,
        }));
    } else {
        println!();
        println!("{} file(s): {passed} passed, {failed} failed", files.len());
        if total_in > 0 {
            let delta = (total_out as f64 - total_in as f64) / total_in as f64 * 100.0;
            println!("Aggregate size: {total_in} -> {total_out} bytes ({delta:+.1}%)");
        }
    }

    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_reencode(args: &[String]) -> Result<(), String> {
    let input = Path::new(&args[0]);
    let mut output: Option<&Path> = None;
    let mut policy = or_io::codec::anim::EncodePolicy::Lossless;

    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--policy" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--policy needs a value: raw or quantize[:bits]".to_string())?;
            policy = match value.split_once(':') {
                Some(("quantize", bits)) => or_io::codec::anim::EncodePolicy::Quantize {
                    bits: bits
                        .parse()
                        .map_err(|_| format!("'{bits}' is not a bit count"))?,
                },
                _ => match value.as_str() {
                    "raw" | "lossless" => or_io::codec::anim::EncodePolicy::Lossless,
                    "quantize" => or_io::codec::anim::EncodePolicy::Quantize { bits: 16 },
                    other => return Err(format!("Unknown policy '{other}'")),
                },
            };
            index += 2;
        } else if arg.starts_with("--") {
            return Err(format!("Unknown option {arg}"));
        } else {
            output = Some(Path::new(arg));
            index += 1;
        }
    }

    if !matches!(
        Subject::from_extension(input),
        Some(Subject::Resource(ResourceType::Ycd))
    ) {
        return Err("reencode only applies to .ycd files".to_string());
    }

    let data = fs::read(input).map_err(|e| format!("Failed to read {}: {e}", input.display()))?;
    let dict = CrClipDictionary::from_bytes(&data).map_err(|e| format!("Parse failed: {e}"))?;

    println!("\n=== RE-ENCODE {} ===\n", input.display());
    println!(
        "Policy:      {}",
        match policy {
            or_io::codec::anim::EncodePolicy::Lossless => "raw (lossless)".to_string(),
            or_io::codec::anim::EncodePolicy::Quantize { bits } => format!("quantize {bits} bits"),
        }
    );

    let mut rebuilt = dict.clone();
    let mut animations = 0usize;
    let mut skipped = 0usize;
    let mut worst = 0.0f32;
    let mut before_bytes = 0usize;
    let mut after_bytes = 0usize;

    let block_bytes = |a: &or_io::gta5::CrAnimation| {
        a.blocks
            .iter()
            .flatten()
            .map(|b| b.data.len())
            .sum::<usize>()
    };
    for (key, clip) in or_anim::clip::clips(&dict) {
        let Some(animation) = or_anim::clip::primary(&dict, clip) else {
            continue;
        };
        before_bytes += block_bytes(animation);

        if or_anim::clip::is_compact(animation) {
            skipped += 1;
            after_bytes += block_bytes(animation);
            continue;
        }

        let mut samples: Vec<Option<Vec<[f32; 4]>>> = Vec::with_capacity(animation.dofs.len());
        for dof in 0..animation.dofs.len() {
            let mut frames = Vec::with_capacity(usize::from(animation.num_frames));
            let mut ok = true;
            for frame in 0..animation.num_frames {
                let time = animation.internal_frame_to_time(frame as f32);
                match or_io::codec::anim::decode_dof(animation, dof, time) {
                    Ok(Some(s)) => frames.push(s.components),
                    Ok(None) => frames.push([0.0; 4]),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            samples.push(if ok { Some(frames) } else { None });
        }
        let undecodable = samples.iter().filter(|s| s.is_none()).count();

        let mut animation_copy = animation.clone();
        let sample_ref = samples.clone();
        or_io::codec::anim::reencode_animation(&mut animation_copy, policy, |dof, frame| {
            sample_ref
                .get(dof)
                .and_then(|frames| frames.as_ref())
                .and_then(|frames| frames.get(frame))
                .copied()
                .unwrap_or([0.0; 4])
        })
        .map_err(|e| format!("Re-encode failed: {e}"))?;

        after_bytes += block_bytes(&animation_copy);

        for (dof, before) in samples.iter().enumerate() {
            let Some(before) = before else { continue };
            let components =
                or_io::codec::anim::format_type::component_count(animation.dofs[dof].r#type).min(4);
            for (frame, expected) in before.iter().enumerate() {
                let time = animation_copy.internal_frame_to_time(frame as f32);
                let Ok(Some(got)) = or_io::codec::anim::decode_dof(&animation_copy, dof, time)
                else {
                    return Err(format!(
                        "clip {key:#010X} DOF {dof} was re-encoded into a block that does not decode"
                    ));
                };
                for (a, b) in expected.iter().zip(got.components.iter()).take(components) {
                    worst = worst.max((a - b).abs());
                }
            }
        }

        // ⚠ Only a clip that plays one animation takes the re-encoded blocks.
        if let CrClipAny::CrClipAnimation(c) = clip {
            if let Some(slot) = c
                .animation
                .key
                .and_then(|k| or_anim::clip::animation_mut(&mut rebuilt, k))
            {
                *slot = animation_copy;
            }
        }
        animations += 1;
        if undecodable > 0 {
            println!("  {key:#010X}: {undecodable} DOF(s) could not be decoded and were zeroed");
        }
    }

    println!("Animations:  {animations} re-encoded, {skipped} compact (skipped)");
    println!(
        "Block bytes: {before_bytes} -> {after_bytes} ({:+.1}%)",
        if before_bytes > 0 {
            (after_bytes as f64 - before_bytes as f64) / before_bytes as f64 * 100.0
        } else {
            0.0
        }
    );
    println!("Worst drift: {worst}");

    if let Some(output) = output {
        let bytes = rebuilt
            .to_bytes()
            .map_err(|e| format!("Save failed: {e}"))?;
        fs::write(output, &bytes)
            .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
        let reloaded =
            CrClipDictionary::from_bytes(&bytes).map_err(|e| format!("Reload failed: {e}"))?;
        println!(
            "Wrote {} ({} bytes, {} clips reloaded)",
            output.display(),
            bytes.len(),
            or_anim::clip::clips(&reloaded).count()
        );
    } else {
        println!("(no output path given; nothing written)");
    }
    println!();
    Ok(())
}

fn collect_convertible(
    dir: &Path,
    exts: &[String],
    out: &mut Vec<std::path::PathBuf>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_convertible(&path, exts, out)?;
            continue;
        }
        let Some(ext) = path.extension().and_then(OsStr::to_str) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if !exts.is_empty() && !exts.contains(&ext) {
            continue;
        }
        if Subject::from_extension(&path).is_some() {
            out.push(path);
        }
    }
    Ok(())
}

fn roundtrip_one(
    rt: Subject,
    path: &Path,
    out_dir: Option<&str>,
) -> Result<(usize, usize), String> {
    let input = fs::read(path).map_err(|e| format!("read: {e}"))?;

    let enhanced = rt
        .as_resource()
        .is_some_and(|r| crate::target_of_resource(r, &input).is_gen9());

    let output = match rt {
        Subject::Resource(ResourceType::Ytd) => Listing::from_bytes(path, &input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ydr) if enhanced => GtaDrawable::<Gen9>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ydr) => GtaDrawable::<Gen8>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ydd) if enhanced => {
            PgDictionary::<GtaDrawable<Gen9>>::from_bytes(&input)
                .map_err(|e| format!("parse: {e}"))?
                .to_bytes()
                .map_err(|e| format!("write: {e}"))?
        }
        Subject::Resource(ResourceType::Ydd) => PgDictionary::<GtaDrawable>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Yft) if enhanced => GtaFragType::<Gen9>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Yft) => GtaFragType::<Gen8>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ypt) => PtxFxList::<Gen8>::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ybn) => or_io::gta5::PhBoundAny::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ynd) => or_io::gta5::CPathRegion::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Yld) => {
            or_io::gta5::PgDictionary::<or_io::gta5::CharacterCloth>::from_bytes(&input)
                .map_err(|e| format!("parse: {e}"))?
                .to_bytes()
                .map_err(|e| format!("write: {e}"))?
        }
        Subject::Resource(ResourceType::Ycd) => CrClipDictionary::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Yed) => {
            or_io::gta5::CrExpressionsDictionary::from_bytes(&input)
                .map_err(|e| format!("parse: {e}"))?
                .to_bytes()
                .map_err(|e| format!("write: {e}"))?
        }
        Subject::Resource(ResourceType::Yfd) => {
            or_io::gta5::CrFrameFilterDictionary::from_bytes(&input)
                .map_err(|e| format!("parse: {e}"))?
                .to_bytes()
                .map_err(|e| format!("write: {e}"))?
        }
        Subject::Resource(ResourceType::Ybd) => {
            or_io::gta5::PgDictionary::<or_io::gta5::PhBoundAny>::from_bytes(&input)
                .map_err(|e| format!("parse: {e}"))?
                .to_bytes()
                .map_err(|e| format!("write: {e}"))?
        }
        Subject::Resource(ResourceType::Ynv) => or_io::gta5::CNavMesh::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Yvr) => CVehicleRecording::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Resource(ResourceType::Ywr) => FwWaypointRecordingRoute::from_bytes(&input)
            .map_err(|e| format!("parse: {e}"))?
            .to_bytes()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Gxt2 => {
            let t = Gxt2::from_bytes(&input).map_err(|e| format!("parse: {e}"))?;
            t.to_bytes().map_err(|e| format!("write: {e}"))?
        }
        Subject::Ypdb => {
            let db = PoseMatcherDatabase::from_bytes(&input).map_err(|e| format!("parse: {e}"))?;
            db.to_bytes().map_err(|e| format!("write: {e}"))?
        }
        Subject::HeightMap => HeightMap::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::DistantLights => DistantLights::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::WaterMap => WaterMap::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::WaterData => CWaterData::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::WorldBlanket => WorldBlanket::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save(),
        Subject::NavPrecalc => NavPrecalc::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::Mrf => MvNetworkDef::open(&input)
            .map_err(|e| format!("parse: {e}"))?
            .save()
            .map_err(|e| format!("write: {e}"))?,
        Subject::CacheFile | Subject::SocialClubMetadata | Subject::AudioWorldSectors => {
            resave_dat_table(rt, &input)?
        }
        readonly @ (Subject::Fxc | Subject::Resource(ResourceType::Ysc)) => {
            return Err(no_writer(readonly))
        }
        unread @ Subject::Resource(ResourceType::Ypm) => {
            return Err(no_reader(unread));
        }
    };

    verify_stats(rt, path, &output).map_err(|e| format!("reload: {e}"))?;

    if let Some(dir) = out_dir {
        let name = path.file_name().unwrap_or_else(|| OsStr::new("out"));
        let dest = Path::new(dir).join(name);
        fs::write(&dest, &output).map_err(|e| format!("write {}: {e}", dest.display()))?;
    }

    Ok((input.len(), output.len()))
}

fn print_step_timing(label: &str, duration: Duration) {
    let ms = duration.as_secs_f64() * 1000.0;
    println!("  {:<14} {:>8.1}ms", label, ms);
}

fn print_convert_timing(decompress: Duration, parse: Duration, write: Duration) {
    println!("  Step timing:");
    print_step_timing("  decompress", decompress);
    print_step_timing("  parse", parse);
    print_step_timing("  serialize", write);
    print_step_timing("  subtotal", decompress + parse + write);
}

fn take_json_flag(args: &[String]) -> (bool, Vec<&String>) {
    let mut json = false;
    let rest = args
        .iter()
        .filter(|a| {
            if a.as_str() == "--json" {
                json = true;
                false
            } else {
                true
            }
        })
        .collect();
    (json, rest)
}

fn print_usage(program: &str) {
    eprintln!("RAGE Resource Tool - Load and write GTA V resource files");
    eprintln!();
    eprintln!("Usage:");
    eprintln!(
        "  {} <input> <output>         Convert/roundtrip a file",
        program
    );
    eprintln!(
        "  {} info <file> [--json]     Show file information",
        program
    );
    eprintln!(
        "  {} verify <file> [--json]   Verify file can be read",
        program
    );
    eprintln!(
        "  {} batch <dir> [options]    Roundtrip every supported file under <dir>",
        program
    );
    eprintln!(
        "  {} repair <in> <out> [--drop-copies]  Strip aliased LOD levels from a drawable; with --drop-copies also a level that copies the high one",
        program
    );
    eprintln!(
        "  {} retarget <dir> --out <dir>  Convert a folder between Legacy and Enhanced",
        program
    );
    eprintln!();
    eprintln!("Batch options:");
    eprintln!("  --ext=<a,b,...>   Only these extensions (default: all supported)");
    eprintln!("  --out <dir>       Keep converted output here (default: discard)");
    eprintln!("  --json            Machine-readable per-file results");
    eprintln!();
    eprintln!("Supported formats:");
    eprintln!("  .ytd  Texture Dictionary (v13)");
    eprintln!("  .ydr  Drawable/Model (v165)");
    eprintln!("  .ydd  Drawable Dictionary (v165)");
    eprintln!("  .yft  Fragment (v162)");
    eprintln!("  .ypt  Particle Template (v68)");
    eprintln!("  .ybn  Physics bounds (v43)");
    eprintln!("  .yld  Cloth dictionary (v8)");
    eprintln!("  .ycd  Clip dictionary (v46)");
    eprintln!("  .yed  Expression dictionary");
    eprintln!("  .yfd  Frame filter dictionary");
    eprintln!("  .ybd  Bound dictionary");
    eprintln!("  .ynd  Path node region");
    eprintln!("  .ynv  Navigation mesh");
    eprintln!("  .yvr  Vehicle recording");
    eprintln!("  .ywr  Waypoint recording");
    eprintln!("  .ysc  Script program (read only)");
    eprintln!("  .gxt2 Global text table (hash-keyed strings)");
    eprintln!("  .ypdb Pose matcher database");
    eprintln!("  .fxc  Shader effect archive (read only)");
    eprintln!("  .mrf  Move network definition");
    eprintln!("  heightmap*.dat      Terrain height map");
    eprintln!("  distantlights*.dat  Distant lights");
    eprintln!("  waterheight.dat     Water height map");
    eprintln!("  water.xml, water_*.xml  Water, calming and wave quads");
    eprintln!("  worldblanket.dat    World blanket");
    eprintln!("  navprecalc.dat      Nav precalc");
    eprintln!("  *_cache_y.dat       Cache file (map data, interior proxy and bounds extents)");
    eprintln!("  metadata.dat        Social Club metadata");
    eprintln!("  audioworldsectorsinfo.dat  Audio world sectors");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {} original.ytd converted.ytd", program);
    eprintln!("  {} info textures.ytd", program);
    eprintln!("  {} verify particles.ypt", program);
    eprintln!("  {} batch ./corpus --ext=yft", program);
    eprintln!();
    eprintln!("Testing workflow:");
    eprintln!("  1. Extract a file from GTA V using OpenIV or CodeWalker");
    eprintln!("  2. Run: {} original.ytd converted.ytd", program);
    eprintln!("  3. Open converted.ytd in CodeWalker to verify");
    eprintln!("  4. Place in mods folder and test in GTA V");
}

pub fn run(args: &[String]) -> ExitCode {
    let program = "openrage-cli convert";

    if args.is_empty() {
        print_usage(program);
        return ExitCode::from(2);
    }

    let result = match args[0].as_str() {
        "info" => {
            let (json, rest) = take_json_flag(&args[1..]);
            let Some(file) = rest.first() else {
                eprintln!("Error: Missing file argument");
                eprintln!("Usage: {} info <file> [--json]", program);
                return ExitCode::from(2);
            };
            cmd_info(Path::new(file), json)
        }
        "verify" => {
            let (json, rest) = take_json_flag(&args[1..]);
            let Some(file) = rest.first() else {
                eprintln!("Error: Missing file argument");
                eprintln!("Usage: {} verify <file> [--json]", program);
                return ExitCode::from(2);
            };
            cmd_verify(Path::new(file), json)
        }
        "batch" => {
            let (json, rest) = take_json_flag(&args[1..]);
            return cmd_batch(&rest, json);
        }
        // as-prop <component.ydd> <out.ydd> <opaque stock prop.ydd> <alpha stock prop.ydd> <12 numbers: origin, then the rotation by rows>
        "as-prop" => {
            if args.len() != 6 {
                eprintln!("Usage: {} as-prop <component.ydd> <out.ydd> <opaque stock prop.ydd> <alpha stock prop.ydd> <ox,oy,oz,r00..r22>", program);
                return ExitCode::from(2);
            }
            let n: Vec<f32> = args[5]
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            if n.len() != 12 {
                eprintln!("Error: the frame takes 12 numbers: origin, then the rotation by rows");
                return ExitCode::from(2);
            }
            let frame = or_interchange::doctor::place::Frame {
                origin: [n[0], n[1], n[2]],
                axes: [[n[3], n[4], n[5]], [n[6], n[7], n[8]], [n[9], n[10], n[11]]],
            };
            let read = |p: &String| fs::read(p).map_err(|e| format!("{p}: {e}"));
            let made = (|| {
                let (data, opaque, alpha) = (read(&args[1])?, read(&args[3])?, read(&args[4])?);
                let (said, bytes) = super::fixups::as_prop_bytes(&data, &opaque, &alpha, frame)?;
                fs::write(&args[2], &bytes).map_err(|e| format!("{}: {e}", args[2]))?;
                println!(
                    "{said}\nWrote {} ({} bytes, was {})",
                    args[2],
                    bytes.len(),
                    data.len()
                );
                Ok::<(), String>(())
            })();
            return match made {
                Ok(()) => ExitCode::SUCCESS,
                Err(why) => {
                    eprintln!("Error: {why}");
                    ExitCode::FAILURE
                }
            };
        }
        "retarget" => return crate::retarget::run(&args[1..]),
        "repair" => {
            let drop_copies = args[1..].iter().any(|a| a == "--drop-copies");
            let fix_counts = args[1..].iter().any(|a| a == "--fix-counts");
            let fix_palettes = args[1..].iter().any(|a| a == "--fix-palettes");
            // --place ox,oy,oz,r00,r01,r02,r10,r11,r12,r20,r21,r22: the frame (origin, then the rotation by rows) the mesh is carried into
            let place_at = args[1..].iter().position(|a| a == "--place");
            let place = place_at.and_then(|i| args.get(i + 2)).map(|text| {
                let n: Vec<f32> = text
                    .split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect();
                (n.len() == 12).then(|| or_interchange::doctor::place::Frame {
                    origin: [n[0], n[1], n[2]],
                    axes: [[n[3], n[4], n[5]], [n[6], n[7], n[8]], [n[9], n[10], n[11]]],
                })
            });
            if matches!(place, Some(None)) {
                eprintln!("Error: --place takes 12 numbers: origin, then the rotation by rows");
                return ExitCode::from(2);
            }
            let place = place.flatten();
            let rest: Vec<&String> = args[1..]
                .iter()
                .enumerate()
                .filter(|(i, a)| {
                    *a != "--drop-copies"
                        && *a != "--fix-counts"
                        && *a != "--fix-palettes"
                        && *a != "--place"
                        && !place_at.is_some_and(|p| *i == p + 1)
                })
                .map(|(_, a)| a)
                .collect();
            if rest.len() < 2 {
                eprintln!("Error: Missing arguments");
                eprintln!(
                    "Usage: {} repair <input> <output> [--drop-copies] [--fix-counts] [--fix-palettes]",
                    program
                );
                return ExitCode::from(2);
            }
            cmd_repair(
                Path::new(rest[0]),
                Path::new(rest[1]),
                drop_copies,
                fix_counts,
                fix_palettes,
                place,
            )
        }
        "reencode" => {
            if args.len() < 2 {
                eprintln!("Error: Missing arguments");
                eprintln!(
                    "Usage: {} reencode <input.ycd> [output.ycd] [--policy raw|quantize[:bits]]",
                    program
                );
                return ExitCode::from(2);
            }
            cmd_reencode(&args[1..])
        }
        "-h" | "--help" | "help" => {
            print_usage(program);
            return ExitCode::SUCCESS;
        }
        input => {
            if args.len() < 2 {
                eprintln!("Error: Missing output argument");
                eprintln!("Usage: {} <input> <output>", program);
                return ExitCode::from(2);
            }
            cmd_convert(Path::new(input), Path::new(&args[1]))
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{}", e);
            eprintln!("Error: {}", e);
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ A GXT2 value is UTF-8 text in any language, so a cut must land on a
    /// character boundary.
    #[test]
    fn a_long_non_ascii_name_is_cut_on_a_character() {
        let name = "é".repeat(30);
        assert_eq!(truncate_name(&name, 40), name);
        assert_eq!(truncate_name(&name, 10), format!("{}...", "é".repeat(7)));
    }

    #[test]
    fn a_heightmap_and_distant_lights_report_by_file_name() {
        let map = HeightMap {
            version: 1,
            flags: HeightMap::RLE_DATA,
            num_cols: 2,
            num_rows: 2,
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [100.0, 50.0, 800.0],
            max: vec![0, 7, 9, 0],
            min: vec![0, 1, 2, 0],
            water_mask: Vec::new(),
        };
        let path = Path::new("levels/heightmapheistisland.dat");
        let subject = Subject::from_extension(path).expect("routed by name");
        assert_eq!(subject, Subject::HeightMap);
        let bytes = map.save().expect("saves");
        let info = info_json(subject, path, &bytes).expect("reports");
        assert_eq!(info["cellSize"], json!([50.0, 25.0]));
        assert_eq!(info["cellsWithHeight"], json!(2));

        let path = Path::new("distantlights_hd.dat");
        assert_eq!(Subject::from_extension(path), Some(Subject::DistantLights));
        let cells = or_io::distant_lights::BLOCKS_HD.pow(2);
        let lights = DistantLights {
            block_first_group: vec![0; cells],
            block_num_coastal_groups: vec![0; cells],
            block_num_inland_groups: vec![0; cells],
            points: Vec::new(),
            groups: Vec::new(),
        };
        let info = info_json(Subject::DistantLights, path, &lights.save().expect("saves"))
            .expect("reports");
        assert_eq!(info["blocks"], json!(32));
        assert_eq!(Subject::from_extension(Path::new("popcycle.dat")), None);
    }

    #[test]
    fn a_water_map_world_blanket_and_nav_precalc_report_by_file_name() {
        use or_io::nav_precalc::NavPrecalcCell;
        use or_io::water_map::{WaterMapRefId, WaterMapTileRow};

        let map = WaterMap {
            num_tile_cols: 2,
            tile_rows: vec![WaterMapTileRow {
                first: 1,
                count: 1,
                start: 0,
            }],
            tiles: vec![0],
            ref_ids: vec![WaterMapRefId(0x8000)],
            ..WaterMap::default()
        };
        let blanket = WorldBlanket {
            cells: vec![0, 1, 1, 0],
        };
        let precalc = NavPrecalc {
            cells: vec![NavPrecalcCell {
                values: [1, 3],
                groups: [vec![[0; 4]], vec![], vec![], vec![[1; 4]; 2]],
            }],
            ..NavPrecalc::default()
        };
        for (name, subject, bytes, key, value) in [
            (
                "common/data/levels/gta5/waterheight.dat",
                Subject::WaterMap,
                map.save().expect("saves"),
                "tilesWithWater",
                json!(1),
            ),
            (
                "worldblanket.dat",
                Subject::WorldBlanket,
                blanket.save(),
                "nonZeroCells",
                json!(2),
            ),
            (
                "navprecalc.dat",
                Subject::NavPrecalc,
                precalc.save().expect("saves"),
                "entries",
                json!(3),
            ),
        ] {
            let path = Path::new(name);
            assert_eq!(Subject::from_extension(path), Some(subject), "{name}");
            let info = info_json(subject, path, &bytes).expect("reports");
            assert_eq!(info[key], value, "{name}: {info}");
        }
    }

    #[test]
    fn a_water_file_reports_its_quads_and_what_it_keeps_by_file_name() {
        let quad = |x: i16, z: &str| {
            format!(
                r#"<Item><minX value="{x}" /><maxX value="{}" /><minY value="0" /><maxY value="4" />{z}</Item>"#,
                x + 8
            )
        };
        let text = format!(
            "<WaterData><WaterQuads>{}{}</WaterQuads><WaveQuads>{}</WaveQuads></WaterData>",
            quad(0, r#"<z value="2.5" /><ocean value="true" />"#),
            quad(8, r#"<z value="-1" />"#),
            quad(0, ""),
        );
        for name in ["common/data/levels/gta5/water.xml", "Water_Island.xml"] {
            let subject = Subject::from_extension(Path::new(name));
            assert_eq!(subject, Some(Subject::WaterData), "{name}");
        }
        assert_eq!(Subject::from_extension(Path::new("watertune.xml")), None);

        let path = Path::new("water.xml");
        let info = info_json(Subject::WaterData, path, text.as_bytes()).expect("reports");
        assert_eq!(info["waterQuads"], json!(2), "{info}");
        assert_eq!(info["calmingQuads"], json!(0), "{info}");
        assert_eq!(info["waveQuads"], json!(1), "{info}");
        assert_eq!(info["heights"], json!([-1.0, 2.5]), "{info}");
        assert_eq!(
            info["unnamed"],
            json!(["WaterData/WaterQuads/Item/ocean"]),
            "{info}"
        );
        let stats = verify_stats(Subject::WaterData, path, text.as_bytes()).expect("verifies");
        assert_eq!(stats["waterQuads"], json!(2), "{stats}");
        let refused = info_json(Subject::WaterData, path, b"<WaterTune/>").unwrap_err();
        assert_eq!(
            refused,
            "the document's root is <WaterTune>, not <WaterData>"
        );
    }

    #[test]
    fn a_move_network_reports_its_node_kinds_and_state_machines() {
        use or_io::mrf::{
            MvNodeDef, MvTransition, NodeType, StateBaseDef, StateDef, StateMachineDef, StateNode,
            VERSION,
        };
        let state = StateDef {
            base: StateBaseDef {
                transitions_offset: 0x40 - 0x1c,
                ..StateBaseDef::default()
            },
            child_count: 0,
            param_inputs_offset: 0,
            events_offset: 0,
            param_outputs_offset: 0,
            operations_offset: 0,
            transitions: vec![MvTransition {
                flags: 0,
                tag: 0,
                duration: 0.5,
                signal: 0,
                transition_weight_signal: 0,
                target: -(0x40 + 0x14),
                conditions: Vec::new(),
                filter: None,
            }],
            param_inputs: Vec::new(),
            events: Vec::new(),
            param_outputs: Vec::new(),
            operations: Vec::new(),
        };
        let mut network = MvNetworkDef {
            version: VERSION,
            xrefs: Vec::new(),
            requests: Vec::new(),
            flags: Vec::new(),
            nodes: vec![
                MvNodeDef {
                    node_type: NodeType::StateMachine,
                    index: 0,
                    id: 0x11,
                    body: NodeBody::StateMachine(StateMachineDef {
                        base: StateBaseDef::default(),
                        states: vec![StateNode { id: 0x22, node: 0 }],
                        transitions: Vec::new(),
                    }),
                },
                MvNodeDef {
                    node_type: NodeType::State,
                    index: 0,
                    id: 0x22,
                    body: NodeBody::State(state),
                },
            ],
            string_table: Vec::new(),
        };
        let at = network.node_offsets();
        let NodeBody::StateMachine(machine) = &mut network.nodes[0].body else {
            unreachable!()
        };
        machine.states[0].node = at[1] as i32 - (at[0] + 0x24) as i32;

        let path = Path::new("anim/networkdefs.rpf/taskexitvehicle.mrf");
        let subject = Subject::from_extension(path).expect("routed by extension");
        assert_eq!(subject, Subject::Mrf);
        let bytes = network.save().expect("saves");
        let info = info_json(subject, path, &bytes).expect("reports");
        assert_eq!(info["nodeCounts"], json!({ "StateMachine": 1, "State": 1 }));
        assert_eq!(
            info["stateMachines"],
            json!([{
                "id": "0x00000011",
                "kind": "StateMachine",
                "states": [{ "id": "0x00000022", "transitions": 1 }],
            }])
        );
        assert_eq!(
            verify_stats(subject, path, &bytes).expect("verifies"),
            json!({ "nodes": 2, "stateMachines": 1 })
        );
    }

    #[test]
    fn the_cache_files_metadata_and_audio_sectors_report_by_file_name() {
        let cache = or_io::cache_loader::CacheFile {
            bounds_store: vec![Default::default(); 3],
            ..Default::default()
        };
        let grid = AudioWorldSectors::default();
        let metadata = SocialClubMetadata {
            format_word: 1,
            built: "2024-02-01T15:54:51".into(),
            body: vec![0; 5],
        };
        for (name, subject, bytes, key, value) in [
            (
                "data/gta5_cache_y.dat",
                Subject::CacheFile,
                cache.save().expect("saves"),
                "boundsStore",
                json!(3),
            ),
            (
                "MP2023_01_3100635916_cache_y.dat",
                Subject::CacheFile,
                cache.save().expect("saves"),
                "version",
                json!(46),
            ),
            (
                "x64/metadata.dat",
                Subject::SocialClubMetadata,
                metadata.save().expect("saves"),
                "bodyBytes",
                json!(5),
            ),
            (
                "levels/gta5/audioworldsectorsinfo.dat",
                Subject::AudioWorldSectors,
                grid.save().expect("saves"),
                "sectors",
                json!(10_000),
            ),
        ] {
            let path = Path::new(name);
            assert_eq!(Subject::from_extension(path), Some(subject), "{name}");
            let info = info_json(subject, path, &bytes).expect("reports");
            assert_eq!(info[key], value, "{name}: {info}");
            assert_eq!(resave_dat_table(subject, &bytes).expect("resaves"), bytes);
        }
        assert_eq!(Subject::from_extension(Path::new("cache_y.dat")), None);
    }

    #[test]
    fn a_vehicle_and_a_waypoint_recording_report_convert_and_verify() {
        use or_io::gta5::{CVehicleStateEachFrame, RouteData};

        let recording = CVehicleRecording {
            states: (0..3)
                .map(|i| CVehicleStateEachFrame {
                    time_in_recording: i * 100,
                    coors_x: i as f32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let route = FwWaypointRecordingRoute {
            waypoints: (0..4)
                .map(|i| RouteData {
                    xyz: [i as f32, 1.0, 2.0],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let dir = tempfile::tempdir().expect("tempdir");
        for (ext, bytes, key, count) in [
            ("yvr", recording.to_bytes().expect("saves"), "frameCount", 3),
            ("ywr", route.to_bytes().expect("saves"), "waypointCount", 4),
        ] {
            let path = dir.path().join(format!("a.{ext}"));
            fs::write(&path, &bytes).expect("written");
            let subject = Subject::from_extension(&path).expect("recognised");
            let info = info_json(subject, &path, &bytes).expect("info reads it");
            assert_eq!(info[key], json!(count), "{ext}: {info}");
            cmd_info(&path, false).expect("info reads it");
            cmd_verify(&path, false).expect("verify passes");
            assert_eq!(
                roundtrip_one(subject, &path, None),
                Ok((bytes.len(), bytes.len()))
            );

            let out = dir.path().join(format!("b.{ext}"));
            cmd_convert(&path, &out).expect("convert round-trips");
            assert_eq!(fs::read(&out).expect("converted"), bytes, "{ext}");
        }
    }
}
