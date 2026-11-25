use std::process::ExitCode;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod addonped;
mod analyse;
mod archive_census;
mod awc;
mod awc_decode;
mod bounds_census;
mod bulk;
mod census_cap;
mod convert;
mod deflate;
mod depends;
mod describe;
mod dlc;
mod fixups;
mod formats;
mod gfx;
mod gta6_anim;
mod gta6_anim_scene;
mod gta6_bound_edits;
mod gta6_embedded_texture;
mod gta6_export;
mod gta6_expression_tracks;
mod gta6_expressions;
mod gta6_fragment;
mod gta6_fragment_regroup;
mod gta6_group_map;
mod gta6_hashes;
mod gta6_ids;
mod gta6_import;
mod gta6_install;
mod gta6_inverse;
mod gta6_keys;
mod gta6_map;
mod gta6_map_entities;
mod gta6_map_entity;
mod gta6_matcher;
mod gta6_meta_census;
mod gta6_meta_import;
mod gta6_meta_names;
mod gta6_mission;
mod gta6_model;
mod gta6_motion_detail;
mod gta6_name_pool;
mod gta6_names;
mod gta6_names_mine;
mod gta6_navmesh;
mod gta6_navmesh_edit;
mod gta6_other_bodies;
mod gta6_particle_copies;
mod gta6_particles;
mod gta6_ped_catalogue;
mod gta6_project;
mod gta6_rdna;
mod gta6_rdna_backends;
mod gta6_rdna_bind;
mod gta6_rdna_diff;
mod gta6_rdna_limits;
mod gta6_rdna_pairs;
mod gta6_rdna_run;
mod gta6_relayout;
mod gta6_resource;
mod gta6_save;
mod gta6_scripts;
mod gta6_shaders;
mod gta6_shading;
mod gta6_spans;
mod gta6_text;
mod gta6_texture;
mod gta6_texture_replace;
mod gta6_translate;
mod gta6_uidata;
mod gta6_world;
mod import;
mod inspect;
mod jobs;
mod keys;
mod maps;
mod maps_changes;
mod mcp;
mod mesh;
mod meta;
mod names;
mod navmesh_census;
mod navmesh_edit;
mod particles;
mod photo;
mod project;
mod project_acceptance;
mod project_checks;
mod project_resource;
mod project_steps;
mod registry;
mod retarget;
mod rpf;
mod rpf9;
mod rpf9_diff;
mod rpf9_write;
mod script;
mod search;
mod serve;
mod source;
mod texture;
mod texture_census;
mod ttpf;
mod unfixtured_census;
mod verify;
mod version_census;
mod vertex_census;
mod wardrobe;
mod wardrobe_dump;
mod wardrobe_exchange;
mod wardrobe_fix;
mod wardrobe_maps;
mod wardrobe_mesh;
mod wardrobe_places;
mod wardrobe_power;
mod weapons;
mod world;
mod world_archetypes;
mod world_edit;
/// The install built by hand that the world verbs' tests share with `or-depends`' own.
#[cfg(test)]
#[path = "../../../domain/or-depends/tests/world_fixture/mod.rs"]
mod world_fixture;
mod world_ground;
mod world_index;
mod world_new;

pub use depends::DependencyIndex;
pub use dlc::{create_dlc_pack, dlc_pack_of_folder, dlc_refusals};
pub use gta6_export::carry_images as gta6_carry_images;
pub use gta6_export::{scene as gta6_scene, Names as Gta6Names};
pub use gta6_import::{replace_gta6_mesh, replace_gta6_mesh_with};
pub use gta6_model::{replace_gta6_model, ModelLods, ModelReplace};
pub use gta6_name_pool::listing as gta6_name_pool;
pub use gta6_shading::{
    drawing_pass as gta6_drawing_pass, gta6_kept, is_gta6_shading_pass, translate_gta6_mesh_stage,
    translate_gta6_pixel_stage, translate_gta6_shading, view_passes as gta6_view_passes,
    Fed as Gta6Fed, Feed as Gta6Feed, Gta6PixelStage, Gta6ShadingPass, Stage as Gta6Stage,
};
pub use gta6_translate::{translate_gta6_program, Gta6Program, Gta6Refusal, DXIL_FRONT};
pub use mesh::{export_scene, replace_mesh, Exported};
pub use project_resource::check_resource_folder;
pub use retarget::{retarget_file, Retargeted, CONVERTED as RETARGETED};
pub use script::listing as script_listing;
pub use script::provenance as script_provenance;
pub use script::source as script_source;
pub use script::{download_natives, downloaded_natives, natives_dir};
pub use texture::export_pngs;

/// The target a file's own RSC7 version names, never the install it came from; Legacy when it names none.
fn target_of(path: &std::path::Path, data: &[u8]) -> or_io::Generation {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(or_io::container::resource::ResourceType::from_extension)
        .map_or(or_io::Generation::Gen8, |resource| {
            target_of_resource(resource, data)
        })
}

/// The target a file's RSC7 version names for `resource`, and Legacy when it names none.
fn target_of_resource(
    resource: or_io::container::resource::ResourceType,
    data: &[u8],
) -> or_io::Generation {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    Rsc7::peek(data)
        .ok()
        .and_then(|seal| resource.generation_of_version(seal.version()))
        .unwrap_or(or_io::Generation::Gen8)
}

/// A resource's decompressed pages and virtual size, for a view of what the file states.
fn pages(data: &[u8]) -> Result<(or_io::decl::Aligned, usize), String> {
    use or_io::container::resource::{ResourceEnvelope, Rsc7};
    let opened = Rsc7::open(data).map_err(|e| e.to_string())?;
    Ok((
        or_io::decl::Aligned::from_bytes(&opened.data),
        opened.virtual_size as usize,
    ))
}

/// Report a bad argument, with the usage error's exit code.
fn usage_error(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    ExitCode::from(2)
}

/// Every command `run` dispatches, for a caller that writes a command line and checks its first word.
pub const COMMANDS: &[&str] = &[
    "awc",
    "bulk",
    "convert",
    "deflate",
    "gfx",
    "depends",
    "describe",
    "inspect",
    "mesh",
    "model",
    "meta",
    "world",
    "project",
    "wardrobe",
    "addonped",
    "particles",
    "maps",
    "weapons",
    "photo",
    "rpf",
    "keys",
    "script",
    "search",
    "hash",
    "names",
    "serve",
    "mcp",
    "source",
    "texture",
    "versions",
    "gaps",
    "formats",
    "analyse",
    "archives",
    "channels",
    "bounds",
    "navmesh",
    "dlc",
    "dlc-list",
];

/// Whether `args` is a command line `run` would take, read by the parser of its command and without running it; an `Err` says what is refused.
///
/// ⚠ Only the commands the editor states a line for, for Copy as command, are read; any other is refused as one this does not check.
pub fn accepts(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or("no command")?;
    let after = rest.get(1..).unwrap_or_default();
    match (command.as_str(), rest.first().map(String::as_str)) {
        ("bulk", _) => bulk::accepts(rest),
        ("convert", Some("retarget")) => retarget::accepts(after),
        ("mesh", Some("export")) => mesh::accepts_export(after),
        ("mesh", Some("import")) => mesh::accepts_import(after),
        ("dlc", Some("create")) => dlc::accepts_create(after),
        ("project", Some("build")) => project::accepts_build(after),
        _ => Err(format!("`{}` is not a line this reads", args.join(" "))),
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(command) = args.first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    match command.as_str() {
        "awc" => with_cli_tracing(|| awc::run(rest)),
        "bulk" => with_cli_tracing(|| bulk::run(rest)),
        "convert" => with_cli_tracing(|| convert::run(rest)),
        "deflate" => deflate::run(rest),
        "gfx" => with_cli_tracing(|| gfx::run(rest)),
        "depends" => with_cli_tracing(|| depends::run(rest)),
        "describe" => with_cli_tracing(|| describe::run(rest)),
        "inspect" => with_cli_tracing(|| inspect::run(rest)),
        "mesh" => with_cli_tracing_at("warn", || mesh::run(rest)),
        "model" => with_cli_tracing_at("warn", || gta6_model::run(rest)),
        "meta" => with_cli_tracing(|| meta::run(rest)),
        "world" => with_cli_tracing(|| world::run(rest)),
        "project" => with_cli_tracing(|| match rest.split_first() {
            Some((verb, rest)) if verb == "import" => import::run(rest),
            _ => project::run(rest),
        }),
        "wardrobe" => with_cli_tracing(|| wardrobe::run(rest)),
        "addonped" => with_cli_tracing(|| addonped::run(rest)),
        "particles" => with_cli_tracing(|| particles::run(rest)),
        "maps" => with_cli_tracing(|| maps::run(rest)),
        "weapons" => with_cli_tracing(|| weapons::run(rest)),
        "photo" => photo::run(rest),
        "rpf" => rpf::run(rest),
        "rpf9" | "gta6-rpf9" => with_cli_tracing(|| rpf9::run(rest)),
        "rpf9-diff" => with_cli_tracing(|| rpf9_diff::run(rest)),
        "script" => with_cli_tracing(|| script::run(rest)),
        "search" => {
            let level = if rest.iter().any(|a| a == "--verify") {
                "warn"
            } else {
                "info"
            };
            with_cli_tracing_at(level, || search::run(rest))
        }
        "hash" => names::run_hash(rest),
        "names" => with_cli_tracing(|| names::run_names(rest)),
        "hashes" if rest.first().is_some_and(|w| w == "mine") => {
            with_cli_tracing(|| gta6_names_mine::run(rest))
        }
        "serve" => serve::run(rest),
        "mcp" => mcp::run(rest),
        "source" => with_cli_tracing(|| source::run(rest)),
        "texture" if rest.first().is_some_and(|w| w == "replace") => {
            with_cli_tracing(|| gta6_texture_replace::run(&rest[1..]))
        }
        "texture" => with_cli_tracing(|| texture::run(rest)),
        "ttpf" => ttpf::run(rest),
        "gta6-save" => gta6_save::run(rest),
        "gta6-project" => with_cli_tracing(|| gta6_project::run(rest)),
        "keys" => with_cli_tracing(|| gta6_keys::run(rest)),
        "hashes" => with_cli_tracing_at("warn", || gta6_hashes::run(rest)),
        "gta6-mission" => gta6_mission::run(rest),
        "versions" => with_cli_tracing(|| version_census::run(rest)),
        "gaps" => with_cli_tracing(|| unfixtured_census::run(rest)),
        "formats" => with_cli_tracing_at("warn", || formats::run(rest)),
        "analyse" | "analyze" => with_cli_tracing(|| analyse::run(rest)),
        "archives" => with_cli_tracing(|| archive_census::run(rest)),
        "channels" => with_cli_tracing_at("warn", || vertex_census::run(rest)),
        "bounds" => with_cli_tracing_at("warn", || bounds_census::run(rest)),
        "navmesh" => with_cli_tracing_at("warn", || match rest.first().map(String::as_str) {
            Some("disable" | "delete") => navmesh_edit::run(rest),
            Some("scan") => gta6_navmesh::run(&rest[1..]),
            _ => navmesh_census::run(rest),
        }),
        "dlc" => with_cli_tracing(|| dlc::run(rest)),
        "dlc-list" | "dlc_list" => with_cli_tracing(|| search::run_dlc_list(rest)),
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "-V" | "--version" | "version" => {
            println!("openrage-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Error: Unknown command '{}'", other);
            eprintln!();
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn with_cli_tracing(f: impl FnOnce() -> ExitCode) -> ExitCode {
    with_cli_tracing_at("info", f)
}

/// ⚠ A per-file `INFO` line is fine for one file and is the whole cost at
/// volume: the first whole-install `search --verify` spent 787 s, nearly all
/// of it writing two log lines per file to stderr for 321,200 files.
/// `RUST_LOG` still wins, because the default is a default.
fn with_cli_tracing_at(reader_level: &str, f: impl FnOnce() -> ExitCode) -> ExitCode {
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(format!("or_io={reader_level}").parse().unwrap())
                .add_directive("openrage_cli=debug".parse().unwrap()),
        )
        .try_init();
    f()
}

fn print_usage() {
    eprintln!("OpenRage command-line tools");
    eprintln!();
    eprintln!("Usage: openrage-cli <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!(
        "  awc        Audio wave containers: info, export to WAV, replace from WAV, census, which"
    );
    eprintln!("  bulk       Extract from a whole install, by pattern");
    eprintln!("  convert    Load/write GTA V resource files (roundtrip, info, verify, retarget)");
    eprintln!("  describe   What each file of an install is, without opening it");
    eprintln!("  deflate    Extract raw decompressed data from RSC7 files");
    eprintln!("  gfx        Scaleform movies: list tags, export to SWF, import from SWF");
    eprintln!("  inspect    Deep inspection: stats, textures, drawables, diff, layout");
    eprintln!("  mesh       Export a drawable to glTF, and census what an export assumes");
    eprintln!(
        "  model      GTA VI models: replace a loose model's meshes from a glTF, as a new file"
    );
    eprintln!("  meta       Metadata (PSO/RBM/RBF0/XML): info, to-xml, from-xml, roundtrip");
    eprintln!("  world      World data (.ymap/.ytyp): info, check, extents, states, create, delete, prefab, new-map, archetypes; index and at over an install");
    eprintln!("  depends    What an asset needs and where a model is placed: of,");
    eprintln!("             placements, census, rules");
    eprintln!(
        "  project    Projects: create, import game files, plan, build and verify a resource, a DLC or a package"
    );
    eprintln!("  wardrobe   Clothing packs: validate, plan, build, diff, import, fix and optimise textures");
    eprintln!("  addonped   Addon peds: validate and verify");
    eprintln!("  particles  Particles: list, validate and verify");
    eprintln!("  maps       Maps: list, validate, verify, changes of a map from its base, apply operations, delete entities, keep a manifest of the game and list doors");
    eprintln!("  weapons    Weapons: validate and verify");
    eprintln!("  photo      A WebP picture of each model in a folder or the game, with no window");
    eprintln!(
        "  rpf        RPF7 archives: info, list, extract, create, keys; list and extract also"
    );
    eprintln!("             read a GTA VI archive (RPF9) by scanning it, as scan, scan-body,");
    eprintln!("             scan-open, scan-extract, scan-export and scan-anim do; extract --as");
    eprintln!("             converted writes its textures as PNG, models as glTF, metadata as XML");
    eprintln!("             and audio as Ogg Opus");
    eprintln!(
        "  rpf9       GTA VI archives (RPF9): tree deciphers an archive's table of contents and"
    );
    eprintln!("             lists its entries, with --names, --keys, --groups and --nested");
    eprintln!(
        "  script     Inspect, disassemble or decompile a .ysc; natives named from a supplied list"
    );
    eprintln!("  search     Find files by name across an install (raw or mounted DLC sim)");
    eprintln!("  hash       Hash names the way RAGE does (both functions, hex or decimal)");
    eprintln!("  names      Build a hash -> name index over an install and query it");
    eprintln!(
        "  hashes     Census how many hashes of an install resolve to a name, or mine names for those that do not"
    );
    eprintln!("  serve      Persistent NDJSON server (mount once, many commands over stdio)");
    eprintln!("  mcp        MCP server for AI agents over stdio; `mcp install` registers it with a client");
    eprintln!("  source     One source folder: replace a drawable source's geometry from a glTF, GLB or FBX");
    eprintln!("  texture    Author textures: export/import PNG+DDS, re-encode BCn, PSNR");
    eprintln!("             texture replace puts an image into a GTA VI pack, as a new file");
    eprintln!("  ttpf       GTA VI texture packs: list, export the readable textures, replace one");
    eprintln!("  gta6-save  GTA VI saves: info, get or set a value of a story save, key of a name");
    eprintln!(
        "  gta6-project  A GTA VI mod as a project: build edits into copies of its archives, check, revert"
    );
    eprintln!(
        "  gta6-mission  The mission state tables in an inflated body of a GTA VI script archive"
    );
    eprintln!(
        "  keys       GTA VI key snapshot from the game executable: keys gta6 --exe <exe> [--out <file>] [--force]"
    );
    eprintln!("  versions   Census the RSC7 resource versions in an install (Legacy vs Enhanced)");
    eprintln!("  gaps       Census the layouts no fixture exercises, over a whole install");
    eprintln!("  formats    Survey an install: what is in it, and what OpenRage can open");
    eprintln!(
        "  analyse    What a resource says about its own shape, for a format no reader covers"
    );
    eprintln!("  archives   Census the archives of an install for escrow-wrapped resources");
    eprintln!(
        "  channels   Census the vertex channels shipped geometry carries, and their encodings"
    );
    eprintln!("  bounds     Census whether a bounding volume recomputed from the vertices matches the shipped one");
    eprintln!("  navmesh    Census navmesh polygon flags, adjacency bits and cell references;");
    eprintln!("             disable or delete one polygon of a .ynv");
    eprintln!("  dlc        Build a DLC pack from a folder, or read one back");
    eprintln!("  dlc-list   List the DLC packs in engine mount order, up to a cutoff (--dlc) or a");
    eprintln!("             pack of the DLC list (--level), as story or multiplayer (--mode)");
    eprintln!();
    eprintln!("Run `openrage-cli <command> --help` for command-specific usage.");
    eprintln!("`openrage-cli --version` prints the version.");
    eprintln!();
    eprintln!("Exit codes: 0 success, 1 runtime error, 2 usage error.");
}

#[cfg(test)]
mod tests {
    /// ⚠ The list stands beside the dispatch and is not made from it: this holds each name to an arm of `run` and to a line of the usage.
    #[test]
    fn every_command_listed_is_dispatched_and_in_the_usage() {
        let source = include_str!("lib.rs");
        for command in super::COMMANDS {
            let arm = format!("\"{command}\" =>");
            let first_of_two = format!("\"{command}\" |");
            assert!(
                source.contains(&arm) || source.contains(&first_of_two),
                "`{command}` has no arm in `run`"
            );
            let usage = format!("\"  {command} ");
            assert!(source.contains(&usage), "`{command}` is not in the usage");
        }
    }
}
