use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use or_io::container::rpf::crypto::{
    extract_keys_from_magic, extract_keys_with_snapshot, ExtractionMode,
};
use or_io::container::rpf::{
    extract_rpf9_body, fingerprint_executable, read_rpf9_body, BodyForm, Defrag,
    ExtractionProgress, Gta5RetailCryptoProvider, KeysCache, NoOpCryptoProvider, Rpf9BodyKind,
    Rpf9Content, Rpf9Family, Rpf9Header, Rpf9Resource, Rpf9Role, Rpf9Scan, Rpf9ScanFind, Rpf9Walk,
    Rpf9ZlibRun, RpfArchive, RpfBuilder, RpfCryptoProvider, RpfEncryption, RpfEntry, RpfHeader,
    RpfNode, RpfVersion, RPF9_HEADER_SIZE, RPF9_MAGIC, RPF9_MISSING_RUN,
};

use crate::project::Failure::{self, Run, Usage};

pub fn run(args: &[String]) -> ExitCode {
    if args.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }
    let result = match args[0].as_str() {
        "info" => cmd_info(&args[1..]).map_err(Run),
        "list" => cmd_list(&args[1..]),
        "extract" => cmd_extract(&args[1..]),
        "create" => cmd_create(&args[1..]).map_err(Run),
        "defrag" => cmd_defrag(&args[1..]).map_err(Run),
        "import" => cmd_import(&args[1..]).map_err(Run),
        "mv" => cmd_mv(&args[1..]).map_err(Run),
        "rm" => cmd_rm(&args[1..]).map_err(Run),
        "extract-keys" => cmd_extract_keys(&args[1..]).map_err(Run),
        "scan" => cmd_scan(&args[1..]),
        "scan-body" => cmd_scan_body(&args[1..]),
        "scan-open" => cmd_scan_open(&args[1..]),
        "scan-extract" => cmd_scan_extract(&args[1..]),
        "scan-export" => cmd_scan_export(&args[1..]),
        "scan-import" => super::gta6_import::cmd_scan_import(&args[1..]).map_err(Run),
        "scan-from-xml" => super::gta6_meta_import::cmd_scan_from_xml(&args[1..]).map_err(Run),
        "scan-texture" => super::gta6_texture::cmd_scan_texture(&args[1..]).map_err(Run),
        "scan-relayout" => super::gta6_relayout::cmd_scan_relayout(&args[1..]).map_err(Run),
        "scan-census" => super::gta6_meta_census::cmd_scan_census(&args[1..]).map_err(Run),
        "scan-particles" => super::gta6_particles::cmd_scan_particles(&args[1..]).map_err(Run),
        "scan-particle-copies" => {
            super::gta6_particle_copies::cmd_scan_particle_copies(&args[1..]).map_err(Run)
        }
        "scan-bound-edits" => {
            super::gta6_bound_edits::cmd_scan_bound_edits(&args[1..]).map_err(Run)
        }
        "scan-fragment-regroup" => {
            super::gta6_fragment_regroup::cmd_scan_fragment_regroup(&args[1..]).map_err(Run)
        }
        "scan-map-entities" => {
            super::gta6_map_entities::cmd_scan_map_entities(&args[1..]).map_err(Run)
        }
        "scan-expression-tracks" => {
            super::gta6_expression_tracks::cmd_scan_expression_tracks(&args[1..]).map_err(Run)
        }
        "scan-group-map" => super::gta6_group_map::cmd_scan_group_map(&args[1..]).map_err(Run),
        "scan-maps" => super::gta6_map::cmd_scan_maps(&args[1..]).map_err(Run),
        "map-entity" => super::gta6_map_entity::cmd_map_entity(&args[1..]).map_err(Run),
        "scan-names" => super::gta6_names::cmd_scan_names(&args[1..]).map_err(Run),
        "scan-world" => super::gta6_world::cmd_scan_world(&args[1..]).map_err(Run),
        "scan-lights" => super::gta6_world::cmd_scan_lights(&args[1..]).map_err(Run),
        "scan-shaders" => super::gta6_translate::cmd_scan_shaders(&args[1..]).map_err(Run),
        "scan-rdna" => super::gta6_rdna::cmd_scan_rdna(&args[1..]).map_err(Run),
        "scan-rdna-diff" => super::gta6_rdna_diff::cmd_scan_rdna_diff(&args[1..]).map_err(Run),
        // Run by `scan-rdna` on itself, and so not in the usage.
        "scan-rdna-worker" => super::gta6_rdna_run::cmd_worker(&args[1..]).map_err(Run),
        "scan-shading" => super::gta6_shading::cmd_scan_shading(&args[1..]).map_err(Run),
        "scan-effect-use" => super::gta6_shading::cmd_scan_effect_use(&args[1..]).map_err(Run),
        "scan-rdna-bind" => super::gta6_rdna_bind::cmd_scan_rdna_bind(&args[1..]).map_err(Run),
        "scan-install" => super::gta6_install::cmd_scan_install(&args[1..]),
        "scan-expressions" => super::gta6_expressions::cmd_scan_expressions(&args[1..]),
        "scan-radio" => super::gta6_uidata::cmd_scan_radio(&args[1..]).map_err(Run),
        "scan-dialogue" => super::gta6_uidata::cmd_scan_dialogue(&args[1..]).map_err(Run),
        "trace" => super::gta6_trace::cmd_trace(&args[1..]),
        "scan-anim" => split_limit(&args[1..])
            .map_err(Usage)
            .and_then(|(limit, args)| super::gta6_anim::cmd_scan_anim(&args, limit)),
        "-h" | "--help" | "help" => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("error: Unknown command: {}", other);
            eprintln!();
            print_usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Usage(e)) => {
            eprintln!("error: {}", e);
            ExitCode::from(2)
        }
        Err(Run(e)) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         openrage-cli rpf info <archive.rpf> [--keys <keys.bin>]\n  \
         openrage-cli rpf list <archive.rpf> [--keys <keys.bin>]\n  \
         openrage-cli rpf list <archive.rpf> [--kind <family>] [--json]   (a GTA VI archive, RPF9: what `rpf scan` prints; --kind keeps the bodies of one family, of models, dictionaries, collision, navigation, animations, metadata, text, audio, movies, shaders, archives, other and unknown; --json prints one JSON document)\n  \
         openrage-cli rpf extract <archive.rpf> <out_dir> [--keys <keys.bin>]\n  \
         openrage-cli rpf extract <archive.rpf> <out_dir> [--kind <family>] [--as stored|inflated|converted] [--limit <bytes>] [--dry-run]   (a GTA VI archive, RPF9: what `rpf scan-extract` does; stored, the default, writes a headerless resource as it is stored, which scan-open reads at offset 0, and inflated writes it as `rpf scan-body` does, its virtual section then its physical; the two write every other body alike; converted writes a texture dictionary's textures as PNG and an audio container's streams as `awc export` does, each in a folder of the body's name, a model as a .glb and a metadata body as XML, and every other body stored)\n  \
         openrage-cli rpf create <src_dir> <out.rpf> [--version rpf0|rpf2|rpf3|rpf4|rpf6|rpf7|rpf8|img1|img2|img3] [--name-shift 0..3] [--encryption open|aes|ng] [--keys <keys.bin>]\n  \
         openrage-cli rpf defrag <archive.rpf> [--write] [--keys <keys.bin>]\n  \
         openrage-cli rpf import <archive.rpf> <src_file> <dest_path> [--keys <keys.bin>]\n  \
         openrage-cli rpf mv <archive.rpf> <from_path> <to_path> [--keys <keys.bin>]\n  \
         openrage-cli rpf rm <archive.rpf> <path> [--keys <keys.bin>]\n  \
         openrage-cli rpf extract-keys <gta5.exe> <out_keys.bin> [--aes-only] [--magic <magic.dat>]\n  \
         openrage-cli rpf scan <archive.rpf> [--from <offset>] [--to <offset>]   (RPF9: the bodies found without the table, those that start in the range; what the headers before --from state is not carried)\n  \
         openrage-cli rpf scan-body <archive.rpf> <offset> <out_file> [--limit <bytes>] [--stored | --xml | --gxt2 | --rewrite]   (--stored writes a resource as it is stored, which scan-open reads at offset 0; --xml writes a metadata body's XML; --gxt2 a text table's plain texts as GTA V's text table; --rewrite a text table as OpenRage writes it again)\n  \
         openrage-cli rpf scan-from-xml <archive.rpf|file> <offset> <edited.xml> <out> [--limit <bytes>]   (a scanned or loose GTA VI headerless metadata body with the tree of the XML scan-body --xml wrote, edited, written in, an array of structures, pointers or values whose size their type gives grown or shrunk to it; written as a loose resource that scan-open reads at offset 0)\n  \
         openrage-cli rpf scan-open <archive.rpf> <offset> [--limit <bytes>] [--effects <archive.rpf> <offset>] [--export <dir> [--format png|dds]] [--texts]   (one scanned body through the reader that fits, or the scan of a nested archive whose names the scan placed or a count of the entries its header states placed; --effects names a drawable's shaders by the SGD5 container there; --export writes a texture dictionary's textures that read, by name, or a drawable's embedded ones as hash_<key>, one file per face or slice; --texts lists a text table's every text by key)\n  \
         openrage-cli rpf scan-extract <archive.rpf> <out_dir> [--kind <kind>] [--role <role>] [--limit <bytes>] [--dry-run]   (every scanned body of a kind as `rpf scan` tallies it or of a family as `rpf list --kind` names it, or of a role, as <offset>.<ext>, each as scan-body --stored writes it)\n  \
         openrage-cli rpf scan-export <archive.rpf> <offset> <out.glb|out.gltf> [--every-lod] [--entry <code>] [--limit <bytes>] [--effects <archive.rpf> <offset>] [--textures <pack.ttpf|folder>]   (a scanned GTA VI drawable, fragment or dictionary entry to glTF, a dictionary's entry skinned to its own skeleton; --textures carries as images the textures the drawable embeds and those the pack, or a .ttpf in the folder, holds, its DiffuseTex map the base colour)\n  \
         openrage-cli rpf scan-import <archive.rpf|file> <offset> <mesh.glb|mesh.gltf> <out> [--entry <code>] [--limit <bytes>] [--fill-missing]   (a scanned or loose GTA VI drawable, fragment or dictionary entry with each geometry a primitive of the mesh names replaced from it, as mesh import replaces a GTA V one; channels the mesh cannot state are kept while the vertex count is, and the import is refused by name otherwise; --fill-missing gives a normal, tangent, colour, texcoord or skin the mesh omits at another vertex count its stated default, as mesh import --fill-missing does; written as a loose resource that scan-open reads at offset 0)\n  \
         openrage-cli rpf scan-texture <archive.rpf|file> <offset> <texture> <out> [--image <file>] [--add] [--remove] [--slice <n>] [--format <format>] [--mips <n>] [--filter box|lanczos3] [--quality fast|normal|high] [--srgb|--linear] [--allow-npot] [--limit <bytes>]   (a scanned or loose GTA VI texture dictionary's texture, as texture import and reencode edit a .ytd: from --image in its own size, format and mips, one slice of a cube, an array or a one-level volume by --slice, else given the image's size, --format and --mips; without --image re-encoded from its top mip; --add adds it from --image; a drawable's or fragment's embedded texture, named or hash_ and its key, from --image or re-encoded in any size, format and mips, the physical section packed anew, --add embedding one from --image and --remove unembedding it; written as a loose resource that scan-open reads at offset 0)\n  \
         openrage-cli rpf scan-anim <archive.rpf> <offset> [--json] [--on <archive.rpf> <offset> [--clip <name>] [--at <seconds>]] [--limit <bytes>]   (a GTA VI animation body as JSON, or bound to the skeleton of the drawable or fragment body --on names, with its pose at --at)\n  \
         openrage-cli rpf scan-expressions <archive.rpf> <offset> [--limit <bytes>] [--id <group> <track> <hash> --out <file> | --add <group> <track> <hash> --out <file> | --remove <group> <track> --out <file>]   (a GTA VI expression set's tracks group by group, each with its id, track word, components or mask and slots; --id gives track <track> of the group inputs, bones, controls, facial-outputs or outputs the id <hash>, written where it lies; --add adds a copy of the track under <hash> and --remove removes it, a bone's rotation and translation together, the set laid out anew; each writes the set as a loose resource, which scan-open reads at offset 0)\n  \
         openrage-cli rpf scan-radio <archive.rpf>... [--tracks] [--language <name>] [--limit <bytes>]   (the radio stations of the archives' GTA VI audio game data: the lists of stations, and each station's track lists and tracks, joined to the streamed sounds they play with each one's length and audio stream ids; a station is named by its key's text in the archives' text tables of the language, the first a change set among the archives registers unless --language names one; --tracks lists every track with the title and artists its text names)\n  \
         openrage-cli rpf scan-dialogue <archive.rpf>... [--out <file.tsv>] [--language <name>] [--limit <bytes>]   (the GTA VI dialogue lines the archives' conversation bodies state, each with its audio stream and its text in every text table that holds its label, with the table's language, tab-separated; a language is the name a change set among the archives registers its text archive under, else the archive's place among its file's; --language reads the tables of one; `awc export --dialogue` reads the file)\n  \
         openrage-cli rpf scan-relayout <archive.rpf> [--from <offset>] [--to <offset>] [--every <n>] [--count <n>] [--limit <bytes>]   (whether each n-th scanned resource's block map, blocks and RSC8 head are what the loose-resource writer derives from its sections, in memory; nothing is written)\n  \
         openrage-cli rpf scan-census <archive.rpf> [--from <offset>] [--to <offset>] [--every <n>] [--kind <kind>|all] [--not <kind>[,<kind>...]] [--sample <n>] [--jobs <n>] [--limit <bytes>]   (each n-th metadata row through scan-open, or with --kind each n-th row of a kind as `rpf scan` tallies it, of a family as `rpf list --kind` names it, or of all, but the rows of the kinds or families --not names with commas between them, as archives, each of which is scanned whole; at most --sample of each row evenly spaced, each in a process of its own, --jobs of them at a time: how many read and refused per row, the refusals by message with numbers as N, and the save lines; exits 1 when a reader of metadata refuses one; nothing is written)\n  \
         openrage-cli rpf scan-particles <archive.rpf> <offset> [--validate] [--limit <bytes>]   (a GTA VI particle effects list's effects, events, emitter and particle rules and behaviours, a line each; --validate says whether it reads and whether its values written back in place leave its section unchanged, and exits 1 when not)\n  \
         openrage-cli rpf scan-particle-copies <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI particle effects list's first effect, emitter and particle rule, first event that copies and first behaviour a rule lacks the class of, each copied and the copy removed again, in memory: a line each saying whether the list reads as stored; exits 1 when one does not; nothing is written)\n  \
         openrage-cli rpf scan-bound-edits <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI bound's, drawable's or fragment's collision given a vertex and a copy of a geometry's last primitive, and a copy of its first primitive child, each deleted again, in memory: a line each saying whether the bound reads as stored; exits 1 when one does not; nothing is written)\n  \
         openrage-cli rpf scan-fragment-regroup <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI fragment's first child that takes another group given it and written, then given its own group at its own place and written again, in memory: whether the virtual section is the stored one; exits 1 when not; nothing is written)\n  \
         openrage-cli rpf scan-map-entities <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI CMapData's first entity copied in as an orphan and the copy removed, and its first parent's child count raised and lowered, in memory: a line each saying whether the map reads, or its sections are, as stored; exits 1 when one does not; nothing is written)\n  \
         openrage-cli rpf scan-expression-tracks <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI expression set laid out anew unchanged, and in each group that takes a track a copy of its first track added and removed again, in memory: a line each saying whether the virtual section is the stored one; exits 1 when one is not; nothing is written)\n  \
         openrage-cli rpf scan-group-map <archive.rpf> <offset> [--limit <bytes>]   (a GTA VI PSO body's first map keyed by hashes given a pair, saved, read again and saved without it, in memory: whether the save is the stored body; exits 1 when not; nothing is written)\n  \
         openrage-cli rpf scan-maps <archive.rpf> [--from <offset>] [--to <offset>] [--limit <bytes>] [--list] [--check] [--extents] [--names]   (every GTA VI map body a scan finds, CMapData and CMapTypes, as its structure and counts, then how many maps, archetypes and entities the kept names name and how many maps more the naming rules name from them; --names keeps those maps' names with the kept ones; --list adds every entity, list and archetype, --check resolves each level-of-detail link among the maps found and counts the elements naming each entity against its numChildren, --extents recomputes each map's entities' box from the archetypes found)\n  \
         openrage-cli rpf map-entity add|remove <archive.rpf> <offset> <entity> <out_dir> [--orphan] [--limit <bytes>]   (a GTA VI map body's entity copied to the end, as a sibling of its source or with --orphan as an orphan, or removed, with its parent map's child count and every map of the archive that names it as a parent renumbered; each body that changed written to the folder as <offset>.rsc, a loose resource that scan-open reads at offset 0)\n  \
         openrage-cli rpf scan-names [<archive.rpf>...] [--prefix <bytes>] [--threads <count>] [--names <file>]... [--pack <file.ttpf or folder>]...   (the names the bodies of GTA VI archives spell, name.#dr and its like and plain ones, read from the first bytes of each body, 16 MiB unless --prefix says, in one pass over each archive on every core unless --threads says, with a line a second on stderr, and kept with those of every archive read before in the configuration folder, where the GTA VI commands and the app find the name a hash or a scanned body has; --names holds each line of a file of names you supply as a plain name, a line that starts with // left out; then how many names are held by ending; --pack counts the keys of a texture pack they name, by a name held and by the naming rules)\n  \
         openrage-cli rpf scan-world <archive.rpf> [--from <offset>] [--to <offset>] [--limit <bytes>] [--shard <bytes>] [--list]   (a census of every GTA VI world-data body a scan finds, a shard of the archive and a body at a time: maps, entities, lights, interiors, entity groups, the lane graph's nodes, links and junctions, and the bodies no reader read; a line per shard, then the totals and the lane graph joined; --list adds a line per body)\n  \
         openrage-cli rpf scan-lights <archive.rpf> <offset> [--limit <bytes>]   (the lights of one GTA VI map body: each light effect of an entity or an archetype and its lights, an entity's own with where they stand in the world)\n  \
         openrage-cli rpf scan-shaders <archive.rpf> <offset> [--limit <bytes>]   (every program of the SGD5 effect container there through the DXIL front end, as counts by outcome and by refusal)\n  \
         openrage-cli rpf scan-rdna <archive.rpf> <offset> [--program <n>] [--json <file>] [--stage <text>] [--reason <text>] [--sample <n>] [--backends] [--cache] [--fresh] [--jobs <n>] [--worker-memory <MB>] [--memory <MB>] [--batch <n>] [--budget <ms>] [--limit <bytes>]   (every program of the SGD5 effect container there through the native-code front end, translated by worker processes side by side so that a program that crashes, panics, runs past --budget or asks for more than --worker-memory is an outcome and never the end of the run; the workers are half the machine's threads at most and no more than fit --memory, a quarter of the machine's where it is not given: per stage, how many decode to a control-flow graph, how many translate, and the refusals by name; the inflated programs are kept under the configuration directory and --fresh reads the archive again; --cache keeps the translations there too; --json writes a record a block, a line each; --stage and --reason list the blocks of a stage whose outcome, reason or back-end refusal holds the text, --sample of them; --backends writes every valid module as SPIR-V, HLSL, MSL and WGSL and counts the refusals; --program takes that program alone and prints its instructions and, where it translates, its WGSL)\n  \
         openrage-cli rpf scan-rdna-diff <before.json> <after.json>   (what changed between two scan-rdna --json runs: the blocks newly translated, refused, invalid, panicked, timed out or crashed, the reasons that grew or shrank in each stage, and the translations that got slower)\n  \
         openrage-cli rpf scan-shading <archive.rpf> <offset> <effect or 0xhash> [--wgsl <folder>] [--hardware] [--limit <bytes>]   (one effect of the SGD5 effect container there, technique by technique: the stages of its first pass and the targets its pixel program exports, then the pass that shades a model translated, with what fills each binding and the places its code reads of it; --wgsl writes the two translated stages there as WGSL; --hardware binds as for a hardware adapter, which refuses no stage for its size)\n  \
         openrage-cli rpf scan-effect-use <archive.rpf> <shaders.rpf> <offset> [--from <offset>] [--to <offset>] [--every <n>] [--limit <bytes>]   (the drawables of a GTA VI archive, or each n-th model body, by the effects their geometries name, against the SGD5 effect container at the offset of the second archive: per effect how many drawables name it, how many of its geometries its own passes draw, and why the rest do not, then the reasons counted)\n  \
         openrage-cli rpf scan-rdna-bind <archive.rpf> <offset> [--technique <name>] [--limit <bytes>]   (each effect of the SGD5 effect container there, its vertex and pixel passes through the native-code front end and its binder: how many translate in both stages, join every resource to a binding record and a name, and bind for the renderer, and what stopped the rest by name; --technique reports instead the first pass of that technique in each effect that has it: whether its two programs translate, and how many of the bindings they declare a model's file, the view and the material fill against those only the game fills)\n  \
         openrage-cli rpf trace <archive.rpf> <offset> [--entry <code>] [--effects <archive.rpf> <offset>] [--level <n>] [--reduced] [--split median|area] [--size <n>] [--png <out.png>] [--limit <bytes>]   (what the game gives its ray tracer of the drawable, fragment or dictionary entry there, traced on the CPU by the reference walk of the ray-tracing programs and printed as depth digits or written as a PNG: one structure a model the ray tracer draws, the geometries whose shader draws in the opaque or the cutout bucket, of the level the game takes for a plain archetype, with --reduced for an archetype it traces a level down, or of --level; --effects names the SGD5 container whose effects say which geometries are described twice; --split names how the builder parts a box's nodes, about the median or by surface area)\n  \
         openrage-cli rpf scan-install <folder> [--kind <kind>] [--sample <n>]   (every GTA VI archive under the folder scanned in turn, cancellably: per archive and in all, how many bodies of each kind `rpf scan` tallies, or only those of a kind or family as `rpf scan-extract --kind` takes it; --sample adds under each archive at most n offsets of each kind, evenly spaced, for `rpf scan-open`; nothing is opened)\n"
    );
}

type ParsedKeysArgs = (
    Arc<dyn RpfCryptoProvider>,
    Option<Arc<or_io::container::Gta5Keys>>,
    Vec<String>,
);

pub(crate) fn split_keys_arg(args: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut keys_path: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--keys" => {
                let p = args
                    .get(i + 1)
                    .ok_or_else(|| "--keys needs a path".to_string())?;
                keys_path = Some(PathBuf::from(p));
                i += 2;
            }
            other => {
                filtered.push(other.to_string());
                i += 1;
            }
        }
    }
    Ok((keys_path, filtered))
}

pub(crate) fn resolve_read_provider(
    archive_path: &Path,
    keys_override: Option<&Path>,
) -> Result<Arc<dyn RpfCryptoProvider>, String> {
    // Before any key is looked for: no provider opens RPF9, and its folder may hold a `common.rpf`.
    if describe_rpf9(archive_path)? {
        return Err(rpf9_refusal());
    }
    if let Some(p) = keys_override {
        let cache = KeysCache::read_from(p)
            .map_err(|e| format!("read keys cache: {e}"))?
            .ok_or_else(|| format!("keys cache not found: {}", p.display()))?;
        let keys = cache
            .into_keys()
            .map_err(|e| format!("decode keys cache: {e}"))?;
        return Ok(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))));
    }
    let root =
        find_install_root(archive_path).or_else(|| super::search::resolve_game_root(None).ok());
    match root {
        Some(root) if root.is_dir() => {
            let resolved = super::keys::resolve_crypto(&root, None);
            for note in &resolved.notes {
                eprintln!("note: {note}");
            }
            Ok(resolved.provider)
        }
        _ => Ok(Arc::new(NoOpCryptoProvider::new())),
    }
}

/// How a GTA VI archive's entries are listed: with a key snapshot, and with none.
fn rpf9_listing() -> String {
    format!(
        "its table is enciphered: with a key snapshot ({} set, or `rpf9 list <archive> --keys \
         <snapshot>`) `rpf9 list` lists its entries, by name where a harvested vocabulary or \
         --names folder holds the name and by hash otherwise, and with none `rpf list` lists the \
         bodies a scan finds",
        or_io::container::rpf::GTA6_SNAPSHOT_ENV
    )
}

/// Why a verb that reads or writes GTA V's archive generations refuses a GTA VI one, and what to run.
fn rpf9_refusal() -> String {
    format!(
        "This is an RPF9 archive (GTA VI), which this verb does not read or write, and {}. \
         `rpf9 copy`, `replace`, `add`, `remove`, `rename` and `defrag` write a changed copy to a \
         new file with the same key snapshot, and the game refuses that copy: only its maker can \
         sign the table",
        rpf9_listing()
    )
}

fn find_install_root(archive_path: &Path) -> Option<PathBuf> {
    let mut dir = archive_path.parent();
    while let Some(d) = dir {
        if d.join("common.rpf").is_file() || d.join("RDR2.exe").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

fn parse_keys_arg_full(args: &[String]) -> Result<ParsedKeysArgs, String> {
    let mut filtered = Vec::with_capacity(args.len());
    let mut i = 0;
    let mut keys_path: Option<PathBuf> = None;
    while i < args.len() {
        match args[i].as_str() {
            "--keys" => {
                let p = args
                    .get(i + 1)
                    .ok_or_else(|| "--keys needs a path".to_string())?;
                keys_path = Some(PathBuf::from(p));
                i += 2;
            }
            other => {
                filtered.push(other.to_string());
                i += 1;
            }
        }
    }
    let (provider, keys): (
        Arc<dyn RpfCryptoProvider>,
        Option<Arc<or_io::container::Gta5Keys>>,
    ) = match keys_path {
        Some(p) => {
            let cache = KeysCache::read_from(&p)
                .map_err(|e| format!("read keys cache: {e}"))?
                .ok_or_else(|| format!("keys cache not found: {}", p.display()))?;
            let keys = cache
                .into_keys()
                .map_err(|e| format!("decode keys cache: {e}"))?;
            let arc = Arc::new(keys);
            (
                Arc::new(Gta5RetailCryptoProvider::new(Arc::clone(&arc))),
                Some(arc),
            )
        }
        None => (Arc::new(NoOpCryptoProvider::new()), None),
    };
    Ok((provider, keys, filtered))
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = split_keys_arg(args)?;
    let path = args
        .first()
        .ok_or_else(|| "missing <archive.rpf>".to_string())?;
    let path_buf = Path::new(path);
    if describe_rpf9(path_buf)? {
        println!("  listing       {}", rpf9_listing());
        return Ok(());
    }
    let provider = resolve_read_provider(path_buf, keys_override.as_deref())?;

    let mut file = fs::File::open(path_buf).map_err(|e| format!("open({path}): {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("stat({path}): {e}"))?
        .len();
    let mut head = [0u8; 4];
    file.read_exact(&mut head)
        .map_err(|e| format!("read({path}): {e}"))?;
    let version =
        RpfVersion::detect(head).ok_or_else(|| format!("{path} starts with no archive magic"))?;
    println!("{version} archive: {}", path);
    // RPF7's header is read first, so a table that will not open still says which mode it is in.
    if version == RpfVersion::Rpf7 {
        file.rewind().map_err(|e| format!("read({path}): {e}"))?;
        let header = RpfHeader::read(&mut file).map_err(|e| format!("header parse: {e}"))?;
        print_header(&header);
    }
    println!("  archive size  {size} bytes");

    match open(path_buf, provider) {
        Ok(archive) => {
            if version != RpfVersion::Rpf7 {
                print_header(&archive.header);
            }
            println!("  files         {}", archive.file_count());
            println!("  folders       {}", archive.dir_count());
            if let or_io::container::TocDecryptOutcome::RecoveredNg { ng_key_index } =
                archive.toc_outcome
            {
                println!(
                    "  ng recovery   key index {ng_key_index} (nominal key did not validate; \
                     archive likely renamed)"
                );
            }
        }
        Err(e) => {
            println!("  files         (cannot enumerate; TOC parse failed)");
            println!("  folders       (cannot enumerate; TOC parse failed)");
            println!("  toc error     {}", e);
            return Err(format!(
                "header parsed OK but TOC decode failed: {e}.\n\
                 Hint: the header above shows which encryption mode the archive \
                 uses; cross-reference with your loaded keys."
            ));
        }
    }
    Ok(())
}

/// Print what an RPF9 archive's plaintext header states; false when `path` is not RPF9.
fn describe_rpf9(path: &Path) -> Result<bool, String> {
    // A file that will not open is not RPF9 here; the open that follows reports it.
    let Ok(file) = fs::File::open(path) else {
        return Ok(false);
    };
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    file.take(RPF9_HEADER_SIZE as u64)
        .read_to_end(&mut head)
        .map_err(|e| format!("read({}): {e}", path.display()))?;
    if !head.starts_with(&RPF9_MAGIC.to_le_bytes()) {
        return Ok(false);
    }
    let size = fs::metadata(path)
        .map_err(|e| format!("stat({}): {e}", path.display()))?
        .len();
    println!("RPF9 archive (GTA VI): {}", path.display());
    println!("  archive size  {size} bytes");
    let header = Rpf9Header::parse(&head).map_err(|e| format!("header parse: {e}"))?;
    println!(
        "  table end     {:#X}, encrypted; its parts start at 0x40, {:#X} and {:#X}, and its index at {:#X}",
        header.toc_end,
        0x40 + u64::from(header.part_b_offset),
        0x40 + u64::from(header.part_c_offset),
        header.index_offset
    );
    println!(
        "  layout        {}, index kind {} (kind 1 indexes the entry rows only; what the layout \
         byte and the other kinds select is not known)",
        header.layout, header.index_kind
    );
    println!(
        "  names         {} bytes, encrypted, ending the archive",
        header.names_size
    );
    println!("  entries       {}", header.entry_count);
    println!("  index width   {} byte(s)", header.index_width);
    let per_archive: String = header
        .per_archive
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("  per archive   {per_archive}");
    Ok(true)
}

/// What `scan-body` writes at most unless `--limit` says otherwise.
const SCAN_BODY_LIMIT: u64 = 1 << 30;

fn scan_label(kind: &Rpf9BodyKind) -> &'static str {
    match kind {
        Rpf9BodyKind::Archive(_) => "archive",
        Rpf9BodyKind::Zlib(run) => match run.content {
            Rpf9Content::Xml => "zlib xml",
            Rpf9Content::Text => "zlib text",
            Rpf9Content::Pso => "zlib pso",
            Rpf9Content::Rbf => "zlib rbf",
            Rpf9Content::ResourceShaped => "zlib resource-shaped",
            Rpf9Content::ClipRoot => "zlib clip root",
            Rpf9Content::PipelineCache => "zlib pipeline cache",
            Rpf9Content::TextTable => "zlib text table",
            Rpf9Content::AudioData => "zlib audio game data",
            Rpf9Content::Identified(body) => body.label(),
            Rpf9Content::Other => "zlib other",
        },
        Rpf9BodyKind::StoredRsc8 { .. } => "stored rsc8",
        Rpf9BodyKind::Bink2 { .. } => "bink 2",
        Rpf9BodyKind::Awc { .. } => "awc",
        Rpf9BodyKind::Sgd5 { .. } => "sgd5",
        Rpf9BodyKind::Names { .. } => "names",
        Rpf9BodyKind::FirstEntry { .. } => "first entry",
        Rpf9BodyKind::RawDeflate {
            content: Rpf9Content::Pso,
        } => "raw deflate pso",
        Rpf9BodyKind::RawDeflate { .. } => "raw deflate rbf",
    }
}

/// The tally row of a find: a body inside an audio container counts apart, in its bytes.
pub(crate) fn scan_row(find: &Rpf9ScanFind) -> String {
    match (find.container, &find.kind) {
        (Some(_), Rpf9BodyKind::StoredRsc8 { .. }) => "stored rsc8 in awc".to_string(),
        (
            _,
            Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                signature: Some(s), ..
            }),
        ) => headerless_row(*s),
        (_, kind) => scan_label(kind).to_string(),
    }
}

/// A headerless resource's row: its role, or its signature where no role is known.
fn headerless_row(signature: or_io::container::rpf::Rpf9Signature) -> String {
    match signature.role() {
        Some(role) => format!("headerless {}", role.label()),
        None => format!("headerless {signature}"),
    }
}

pub(crate) fn scan_line(find: &Rpf9ScanFind) -> String {
    let end = match (find.stored_len, find.inflated_len) {
        (Some(s), Some(i)) => format!("stored {s} bytes, inflated {i}"),
        _ => "end not known".to_string(),
    };
    let what = match &find.kind {
        Rpf9BodyKind::Archive(h) => format!(
            "{} entries, table to {:#X}, names {} bytes; {}{}",
            h.entry_count,
            find.offset + u64::from(h.toc_end),
            h.names_size,
            match find.offset {
                0 => "extent the whole file, which its names end",
                _ => "extent where a names row for it ends it, else not known",
            },
            h.broken_rule().map_or(String::new(), |why| format!(
                "; its header breaks a rule every header measured holds: {why}"
            ))
        ),
        Rpf9BodyKind::Names { archive } => format!(
            "the encrypted names of the archive at {archive:#X}, which end it at {:#X}, {} bytes from its header; {end}",
            find.offset + find.span,
            find.offset + find.span - archive
        ),
        // The scan states a first entry's end only for the placeholder (scan-recount F1).
        Rpf9BodyKind::FirstEntry { .. } if find.stored_len.is_some() => format!(
            "an empty archive's placeholder: the 16 bytes FORCE_EMPTY_RPF\\0, the one entry of an \
             archive that holds nothing else, its names after it at {:#X}; {end}",
            find.offset + find.span
        ),
        Rpf9BodyKind::FirstEntry { head } => format!(
            "the first entry after the table, where the header puts it; its first bytes {}, which no detector here names; {end}",
            head.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
        Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) => format!("{}; {end}", resource_line(resource)),
        Rpf9BodyKind::Zlib(run) => format!(
            "{} chunk(s), {} stored piece(s), first chunk {} -> {} bytes, run inflates to {}; {end}",
            run.chunks,
            run.stored_pieces,
            run.first_chunk_len,
            run.first_chunk_inflated,
            run.inflated
        ),
        Rpf9BodyKind::StoredRsc8 { version, .. } => match find.container {
            Some(at) => format!("version {version}; {end}; a chunk of the container at {at:#X}"),
            None => format!("version {version}; {end}"),
        },
        Rpf9BodyKind::Bink2 { revision, .. } => {
            format!("revision {}; {end}", char::from(*revision))
        }
        Rpf9BodyKind::Awc {
            flags,
            streams,
            chunks,
        } => format!("flags {flags:#06X}, {streams} stream(s), {chunks} chunk(s); {end}"),
        Rpf9BodyKind::Sgd5 { head_layouts } => format!(
            "{head_layouts} head layout(s), to the next entry's page {:#X}; {end}",
            find.offset + find.span
        ),
        Rpf9BodyKind::RawDeflate { .. } => format!(
            "a raw DEFLATE stream with no zlib head, one entry of an archive whose first entry starts as many such streams as it states entries; {end}"
        ),
    };
    format!("  {:#012X}  {:<20}  {what}", find.offset, scan_row(find))
}

/// A resource's block map counts, each block found, and how far its blocks were followed.
fn resource_line(resource: &Rpf9Resource) -> String {
    let blocks: Vec<String> = (resource.blocks.iter().flatten())
        .map(|b| {
            let section = if b.physical { "p" } else { "v" };
            match b.zlib {
                true => format!(
                    "{section} {:#X} zlib {} -> {}",
                    b.offset, b.stored_len, b.inflated_len
                ),
                false => format!("{section} {:#X} stored {}", b.offset, b.stored_len),
            }
        })
        .collect();
    let walk = match resource.walk {
        Rpf9Walk::Whole => "whole".to_string(),
        Rpf9Walk::NotOnDisk { at } => format!("not on disk from {at:#X}"),
        Rpf9Walk::Refused { at, why } => format!("refused at {at:#X}: {why}"),
        Rpf9Walk::TailNotDeflate { at } => format!(
            "its zlib stream stops being deflate data at {at:#X} with no piece missing before it: its tail is replaced or protected"
        ),
        Rpf9Walk::PointersPast { section, targets } => format!(
            "whole; {targets} pointer words aim just past its {section} section: values, or a block of another resource"
        ),
    };
    let map = match resource.virtual_blocks {
        0 => "block map not read".to_string(),
        v => format!("block map v={v} p={}", resource.physical_blocks),
    };
    format!(
        "resource, {map}: {}; virtual {} bytes, physical {} bytes; {walk}",
        blocks.join(", "),
        resource.virtual_len,
        resource.physical_len
    )
}

#[derive(Default)]
struct ScanTally {
    count: u64,
    end_known: u64,
    span: u64,
    inflated: u64,
}

fn print_tallies(tallies: &std::collections::BTreeMap<String, ScanTally>) {
    println!(
        "    {:<20}  {:>9}  {:>9}  {:>15}  {:>15}",
        "kind", "count", "end known", "bytes", "inflated"
    );
    for (label, t) in tallies {
        println!(
            "    {label:<20}  {:>9}  {:>9}  {:>15}  {:>15}",
            t.count, t.end_known, t.span, t.inflated
        );
    }
}

/// Whether the archive's head is all zero, as a piece a download has not delivered reads.
fn head_not_arrived(path: &Path) -> Result<bool, String> {
    let file = fs::File::open(path).map_err(|e| format!("open({}): {e}", path.display()))?;
    let mut head = Vec::with_capacity(RPF9_HEADER_SIZE);
    file.take(RPF9_HEADER_SIZE as u64)
        .read_to_end(&mut head)
        .map_err(|e| format!("read({}): {e}", path.display()))?;
    Ok(head.len() == RPF9_HEADER_SIZE && head.iter().all(|b| *b == 0))
}

fn cmd_scan(args: &[String]) -> Result<(), Failure> {
    let (from, args) = split_offset_flag(args, "--from").map_err(Usage)?;
    let (to, args) = split_offset_flag(&args, "--to").map_err(Usage)?;
    let path = args
        .first()
        .ok_or_else(|| Usage("missing <archive.rpf>".to_string()))?;
    if !describe_rpf9(Path::new(path))? {
        if !head_not_arrived(Path::new(path))? {
            return Err(Run(format!("{path} does not start with an RPF9 head")));
        }
        println!("archive: {path}");
        println!(
            "  head          missing: its first {RPF9_HEADER_SIZE} bytes are zero, as a piece a \
             download has not delivered reads, so whether this is an RPF9 archive and what its \
             header states are not known; the bytes present are scanned"
        );
    }
    let range = from.unwrap_or(0)..to.unwrap_or(u64::MAX);
    // A range that holds no byte is the arguments' error; any other is the scan's.
    let failure = if range.is_empty() { Usage } else { Run };
    list_scan(path, range).map_err(failure)
}

/// The `flag <offset>` an arguments list may hold, and its other words.
pub(crate) fn split_offset_flag(
    args: &[String],
    flag: &str,
) -> Result<(Option<u64>, Vec<String>), String> {
    let Some(i) = args.iter().position(|a| a == flag) else {
        return Ok((None, args.to_vec()));
    };
    let word = args.get(i + 1).ok_or(format!("{flag} needs an offset"))?;
    let mut rest = args.to_vec();
    rest.drain(i..i + 2);
    Ok((Some(parse_offset(word)?), rest))
}

/// Every body of the archive at `path` that starts in `range`, then the tallies of what was found.
fn list_scan(path: &str, range: std::ops::Range<u64>) -> Result<(), String> {
    use std::collections::BTreeMap;
    if range.start >= range.end {
        return Err(format!(
            "the range {:#X} to {:#X} holds no byte",
            range.start, range.end
        ));
    }
    println!("  scan          {}", Rpf9ScanFind::ORIGIN);
    if range != (0..u64::MAX) {
        println!(
            "  range         the bodies that start from {:#X} up to {:#X}",
            range.start, range.end
        );
    }
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let mut scan = Rpf9Scan::new(file, range.clone()).map_err(|e| e.to_string())?;
    // Keyed by the header a section starts at; `None` before any header the scan found.
    let mut sections: BTreeMap<Option<u64>, BTreeMap<String, ScanTally>> = BTreeMap::new();
    let mut totals: BTreeMap<String, ScanTally> = BTreeMap::new();
    let mut section = None;
    // Resource starts, blocks listed under them, and starts with a block not on disk or refused.
    let (mut starts, mut continuations, mut not_on_disk, mut refused) = (0, 0, 0, 0);
    let mut tail_not_deflate = 0;
    // Where each nested archive whose names were placed ends.
    let (mut ends, mut off_rule) = (BTreeMap::new(), 0);
    // A whole file's finds, kept small, for the names placed by count after the scan.
    let whole = range == (0..u64::MAX);
    let mut counted = or_io::container::rpf::Rpf9NamesByCount::default();
    for find in scan.by_ref() {
        let find = find.map_err(|e| e.to_string())?;
        println!("{}", scan_line(&find));
        if whole {
            counted.push(&find);
        }
        match find.kind {
            Rpf9BodyKind::Archive(header) => {
                section = Some(find.offset);
                off_rule += usize::from(header.broken_rule().is_some());
            }
            Rpf9BodyKind::Names { archive } => {
                ends.insert(archive, find.offset + find.span);
            }
            _ => {}
        }
        if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = find.kind
        {
            starts += 1;
            continuations += resource.continuation_blocks();
            match resource.walk {
                Rpf9Walk::Whole | Rpf9Walk::PointersPast { .. } => {}
                Rpf9Walk::NotOnDisk { .. } => not_on_disk += 1,
                Rpf9Walk::TailNotDeflate { .. } => tail_not_deflate += 1,
                Rpf9Walk::Refused { .. } => refused += 1,
            }
        }
        let inflated = match find.kind {
            Rpf9BodyKind::Zlib(run) => run.inflated,
            Rpf9BodyKind::StoredRsc8 { extent: n, .. } | Rpf9BodyKind::Bink2 { length: n, .. } => n,
            Rpf9BodyKind::Awc { .. } => find.stored_len.unwrap_or(0),
            Rpf9BodyKind::RawDeflate { .. } => find.inflated_len.unwrap_or(0),
            Rpf9BodyKind::Archive(_)
            | Rpf9BodyKind::Sgd5 { .. }
            | Rpf9BodyKind::Names { .. }
            | Rpf9BodyKind::FirstEntry { .. } => 0,
        };
        let label = scan_row(&find);
        for tally in [
            sections
                .entry(section)
                .or_default()
                .entry(label.clone())
                .or_default(),
            totals.entry(label).or_default(),
        ] {
            tally.count += 1;
            tally.end_known += u64::from(find.stored_len.is_some());
            tally.span += find.span;
            tally.inflated += inflated;
        }
    }
    if whole {
        let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
        for names in counted.place(file).map_err(|e| e.to_string())? {
            println!(
                "{}; placed by counting its header's entries",
                scan_line(&names)
            );
            if let Rpf9BodyKind::Names { archive } = names.kind {
                ends.insert(archive, names.offset + names.span);
            }
        }
    }
    println!("summary of a scan without the table: a section is the bytes from one archive header to the next, and which archive a body belongs to is in the encrypted table; a row \"in awc\" counts bytes inside the awc row's");
    for (offset, tallies) in &sections {
        match offset {
            Some(0) => println!("  section from the header at 0x0 (top level)"),
            Some(offset) => match ends.get(offset) {
                Some(end) => println!(
                    "  section from the header at {offset:#X}, whose archive ends with its names at {end:#X}"
                ),
                None => println!(
                    "  section from the header at {offset:#X}, whose archive's extent is not known"
                ),
            },
            None => println!("  bytes before the first header the scan found"),
        }
        print_tallies(tallies);
    }
    println!("  every section");
    print_tallies(&totals);
    let nested = sections
        .keys()
        .filter(|s| matches!(s, Some(o) if *o > 0))
        .count();
    println!(
        "  archives      {nested} nested header(s); {} with names placed after their last entry, which give their extent; the others' extent is not known; {off_rule} header(s) that break a rule every header measured holds",
        ends.len()
    );
    println!(
        "  resources     {starts} start(s), with {continuations} continuation block(s) listed under them, not as bodies; {not_on_disk} with a block in a piece a download has not delivered; {tail_not_deflate} whose zlib stream stops being deflate data with no piece missing before it; {refused} whose blocks do not hold to the measured layout"
    );
    println!(
        "  missing       {} bytes in {} run(s) of at least {} zero bytes no body claims: pieces a download has not delivered, not content",
        scan.missing_bytes(),
        scan.missing_runs(),
        RPF9_MISSING_RUN
    );
    let scanned = range.end.min(scan.file_len()).saturating_sub(range.start);
    let present = scanned.saturating_sub(scan.missing_bytes());
    println!(
        "  unclaimed     {} bytes, {} of them zero; {:.2}% of the {present} bytes present",
        scan.unclaimed_bytes(),
        scan.unclaimed_zero_bytes(),
        100.0 * scan.unclaimed_bytes() as f64 / present.max(1) as f64
    );
    println!(
        "  zlib heads that fail partway  {}, {} of them with no zero page before the failure: from it on their bytes are not their deflate data, so their tails are replaced or protected",
        scan.broken_chunks(),
        scan.tails_not_deflate()
    );
    Ok(())
}

pub(crate) fn parse_offset(word: &str) -> Result<u64, String> {
    match word.strip_prefix("0x").or_else(|| word.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => word.parse(),
    }
    .map_err(|e| format!("{word} is not an offset: {e}"))
}

/// The `--limit <bytes>` a scanned-body verb takes, and its other words.
pub(crate) fn split_limit(args: &[String]) -> Result<(u64, Vec<String>), String> {
    match args.iter().position(|a| a == "--limit") {
        Some(i) => {
            let word = args.get(i + 1).ok_or("--limit needs a byte count")?;
            let mut rest = args.to_vec();
            rest.drain(i..i + 2);
            Ok((parse_offset(word)?, rest))
        }
        None => Ok((SCAN_BODY_LIMIT, args.to_vec())),
    }
}

/// An archive's path and the offset of a body in it.
type AtOffset = (String, u64);

/// The archive and offset `--effects` names, and the other words.
pub(super) fn split_effects(args: &[String]) -> Result<(Option<AtOffset>, Vec<String>), String> {
    match args.iter().position(|a| a == "--effects") {
        Some(i) => {
            let (Some(path), Some(word)) = (args.get(i + 1), args.get(i + 2)) else {
                return Err("--effects needs an archive and an offset".to_string());
            };
            let effects = (path.clone(), parse_offset(word)?);
            let mut rest = args.to_vec();
            rest.drain(i..i + 3);
            Ok((Some(effects), rest))
        }
        None => Ok((None, args.to_vec())),
    }
}

/// The effects of the `SGD5` container at `at` of the archive at `path`, which name a drawable's shaders.
fn effects_at_offset(
    (path, at): AtOffset,
    limit: u64,
) -> Result<or_io::gta5::shaders::shader_defs::ShaderDefinitions, String> {
    use or_io::gta5::shaders::sgd5::{shader_definitions, Sgd5File};
    let file = fs::File::open(&path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(file, at, limit).map_err(|e| e.to_string())?;
    if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
        return Err(format!("--effects {at:#X} is not an SGD5 effect container"));
    }
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {at:#X} does not read: {e}"))?;
    Ok(shader_definitions(
        "gta6",
        "gta6",
        std::slice::from_ref(&container),
    ))
}

/// Whether `flag` is among `args`, and the other words.
fn split_flag(args: &[String], flag: &str) -> (bool, Vec<String>) {
    let rest: Vec<String> = args.iter().filter(|a| *a != flag).cloned().collect();
    (rest.len() < args.len(), rest)
}

/// The value after `flag`, and the other words.
pub(crate) fn split_value(
    args: &[String],
    flag: &str,
) -> Result<(Option<String>, Vec<String>), String> {
    match args.iter().position(|a| a == flag) {
        Some(i) => {
            let value = args.get(i + 1).ok_or(format!("{flag} needs a value"))?;
            let mut rest = args.to_vec();
            let value = value.clone();
            rest.drain(i..i + 2);
            Ok((Some(value), rest))
        }
        None => Ok((None, args.to_vec())),
    }
}

/// The XML of a scanned metadata body: a PSO or RBF document, or a headerless RBM resource.
fn metadata_xml(find: &Rpf9ScanFind, bytes: &[u8]) -> Result<String, String> {
    if let Rpf9BodyKind::RawDeflate { .. } = find.kind {
        let instance = super::meta::read_instance(bytes, or_io::meta::ErrorMode::Lenient)?.0;
        return or_io::meta::write_meta(&instance).map_err(|e| e.to_string());
    }
    let Rpf9BodyKind::Zlib(run) = find.kind else {
        return Err(format!(
            "the body at {:#X} is no metadata document",
            find.offset
        ));
    };
    let instance = match (run.content, run.resource) {
        (Rpf9Content::Pso | Rpf9Content::Rbf, _) => {
            super::meta::read_instance(bytes, or_io::meta::ErrorMode::Lenient)?.0
        }
        (Rpf9Content::ResourceShaped | Rpf9Content::TextTable, Some(resource))
            if bytes.get(0x10..0x14) == Some(b"0DRP") =>
        {
            let (virtual_section, physical) = resource.sections(bytes);
            // The names of GTA VI's snapshot are installed by the first ask for the schema.
            super::meta::schema();
            or_io::meta::binary::rbm::gta6::open(virtual_section, physical)
                .map_err(|e| format!("GTA VI's RBM reader does not read this body: {e}"))?
                .0
        }
        _ => {
            return Err(format!(
                "the body at {:#X} is no metadata document",
                find.offset
            ))
        }
    };
    or_io::meta::write_meta(&instance).map_err(|e| e.to_string())
}

fn cmd_scan_body(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = split_limit(args).map_err(Usage)?;
    let (stored, args) = split_flag(&args, "--stored");
    let (xml, args) = split_flag(&args, "--xml");
    let (gxt2, args) = split_flag(&args, "--gxt2");
    let (rewrite, args) = split_flag(&args, "--rewrite");
    let forms = [stored, xml, gxt2, rewrite].iter().filter(|&&f| f).count();
    let ([path, offset, out], 0 | 1) = (args.as_slice(), forms) else {
        return Err(Usage(
            "usage: rpf scan-body <archive.rpf> <offset> <out_file> \
             [--stored | --xml | --gxt2 | --rewrite]"
                .to_string(),
        ));
    };
    let offset = parse_offset(offset).map_err(Usage)?;
    if super::navmesh_edit::same_file(path, out) {
        return Err(Usage(format!(
            "the output is the archive {path}: name another file"
        )));
    }
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let read = if stored {
        extract_rpf9_body
    } else {
        read_rpf9_body
    };
    let (find, mut bytes) = match read(&file, offset, limit) {
        Ok(read) => read,
        // A first entry of unstated end, or a nested archive: the bytes a scan gives it.
        Err(e) if !(xml || gxt2 || rewrite) => {
            return super::gta6_spans::write_refused(&file, offset, &e, out).map_err(Run)
        }
        Err(e) => return Err(Run(e.to_string())),
    };
    if xml {
        bytes = metadata_xml(&find, &bytes)?.into_bytes();
    }
    if gxt2 || rewrite {
        bytes = super::gta6_text::converted(&find, &bytes, gxt2)?;
    }
    fs::write(out, &bytes).map_err(|e| format!("write({out}): {e}"))?;
    println!("{}", scan_line(&find));
    println!(
        "  wrote {} bytes to {out}; {}",
        bytes.len(),
        Rpf9ScanFind::ORIGIN
    );
    if xml {
        println!("  the XML of what OpenRage reads of the body; `rpf scan-from-xml` writes a headerless one back, edited, as a loose resource");
        return Ok(());
    }
    if gxt2 {
        println!(
            "  the table's plain texts as GTA V's text table, which `convert` and `info` read"
        );
        return Ok(());
    }
    if rewrite {
        println!("  the table as OpenRage writes it: `rpf scan-open {out} 0` reads it again");
        return Ok(());
    }
    if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    {
        if stored {
            println!(
                "  the resource as it is stored, from its first block to its last block's end: `rpf scan-open {out} 0` reads it again"
            );
            return Ok(());
        }
        println!(
            "  the first {} bytes are the virtual section, the rest the physical section",
            resource.virtual_len
        );
    } else if find.stored_len.is_none() {
        println!(
            "  these are the bytes of the clear run; whether the body ends with it is not known"
        );
    }
    Ok(())
}

/// Every body a scan finds that `--kind` and `--role` admit, each as `scan-body --stored` writes
/// it, cancellably; a name comes only from the body's offset and kind.
fn cmd_scan_extract(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = split_limit(args).map_err(Usage)?;
    let (dry_run, args) = split_flag(&args, "--dry-run");
    let (kind, args) = split_value(&args, "--kind").map_err(Usage)?;
    let (role, args) = split_value(&args, "--role").map_err(Usage)?;
    let [path, out_dir] = args.as_slice() else {
        return Err(Usage(
            "usage: rpf scan-extract <archive.rpf> <out_dir> [--kind <kind>] \
             [--role <role>] [--limit <bytes>] [--dry-run]"
                .to_string(),
        ));
    };
    let admits = |find: &Rpf9ScanFind| {
        let found_role = match find.kind {
            Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
            _ => None,
        };
        kind.as_ref().is_none_or(|k| {
            *k == scan_row(find) || Rpf9Family::parse(k) == Some(Rpf9Family::of(find))
        }) && role
            .as_ref()
            .is_none_or(|r| found_role.is_some_and(|f| f.label() == r))
    };
    extract_scanned(path, out_dir, limit, dry_run, Form::Stored, admits).map_err(Run)
}

/// Whether a read gives a find's bytes: a header, the names after a table and a first entry whose end is not stated have no end a read knows.
fn is_read(find: &Rpf9ScanFind) -> bool {
    match find.kind {
        Rpf9BodyKind::Archive(_) | Rpf9BodyKind::Names { .. } => false,
        Rpf9BodyKind::FirstEntry { .. } => find.stored_len.is_some(),
        _ => true,
    }
}

/// The form `rpf extract --as` names: a body as it is stored, as a read returns it, or as a person uses it.
#[derive(PartialEq)]
enum Form {
    Stored,
    Inflated,
    Converted,
}

/// Writes the body `find` is into `out_dir` in the form a person uses, by what the single-body
/// verbs run: a texture dictionary's textures as PNG and an audio container's streams, each in a
/// folder of the body's name, a model as a `.glb` and a metadata body as XML. `Ok(false)` for a
/// body with no such form, which is then written stored.
fn convert(
    file: &fs::File,
    find: &Rpf9ScanFind,
    limit: u64,
    out_dir: &Path,
) -> Result<bool, String> {
    use or_interchange::gltf::{self, Container};
    use or_io::gta6::headerless::{read, Entry, Resource};
    let name = find.file_name();
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let body = || read_rpf9_body(file, find.offset, limit).map_err(|e| e.to_string());
    let write = |name: String, bytes: &[u8]| {
        fs::write(out_dir.join(&name), bytes).map_err(|e| format!("write({name}): {e}"))
    };
    match find.kind {
        Rpf9BodyKind::Awc { .. } => {
            let key = or_io::awc::AwcKey::default();
            let awc = or_io::awc::AwcFile::parse(&body()?.1, key).map_err(|e| e.to_string())?;
            let dir = out_dir.join(stem);
            fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all: {e}"))?;
            let decoders = or_media::Decoders::default();
            let (written, skipped) = super::awc::export_streams(&awc, key, false, &decoders, &dir);
            if written == 0 {
                let _ = fs::remove_dir(&dir);
                return Err(format!(
                    "no stream of this container is written: {}",
                    skipped.join("; ")
                ));
            }
            println!("wrote {written} file(s) into {}", dir.display());
            for why in &skipped {
                println!("  skipped {why}");
            }
        }
        Rpf9BodyKind::Zlib(run) => match (run.content, run.resource, run.signature) {
            (Rpf9Content::Pso | Rpf9Content::Rbf, ..)
            | (Rpf9Content::ResourceShaped, Some(_), None) => {
                let (find, bytes) = body()?;
                // A resource with no signature is a metadata body only where it starts as an RBM does.
                if run.resource.is_some() && bytes.get(0x10..0x14) != Some(b"0DRP") {
                    return Ok(false);
                }
                write(
                    format!("{stem}.xml"),
                    metadata_xml(&find, &bytes)?.as_bytes(),
                )?;
            }
            (Rpf9Content::ResourceShaped, Some(resource), Some(signature)) => {
                match signature.role() {
                    Some(Rpf9Role::Drawable | Rpf9Role::DrawableWithBound) => {
                        let bytes = body()?.1;
                        let scene = super::gta6_export::export(
                            &resource, &bytes, signature, None, false, None, stem,
                        )?;
                        let files = gltf::write(&scene, stem, Container::Glb);
                        write(format!("{stem}.glb"), &files.primary)?;
                    }
                    Some(Rpf9Role::Dictionary) => {
                        let bytes = body()?.1;
                        let (virtual_section, physical) = resource.sections(&bytes);
                        let opened = read(signature, virtual_section, Some(physical));
                        let Resource::Dictionary(dict) = opened.map_err(|e| e.to_string())? else {
                            return Ok(false);
                        };
                        let texture =
                            |(_, entry): &(u32, _)| matches!(entry, Ok(Entry::Texture(_)));
                        if !dict.entries.iter().any(texture) {
                            return Ok(false);
                        }
                        let dir = out_dir.join(stem);
                        super::ttpf::export_dictionary(dict, &dir, false).inspect_err(|_| {
                            let _ = fs::remove_dir(&dir);
                        })?;
                    }
                    _ => return Ok(false),
                }
            }
            _ => return Ok(false),
        },
        _ => return Ok(false),
    }
    Ok(true)
}

/// The bodies of the archive at `path` a scan finds and `admits` takes, each written into
/// `out_dir` in `form`.
fn extract_scanned(
    path: &str,
    out_dir: &str,
    limit: u64,
    dry_run: bool,
    form: Form,
    admits: impl Fn(&Rpf9ScanFind) -> bool,
) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let cancelled = super::jobs::on_interrupt();
    let mut finds = Vec::new();
    let mut spans = super::gta6_spans::Spans::default();
    for find in Rpf9Scan::new(&file, 0..u64::MAX).map_err(|e| e.to_string())? {
        if cancelled.is_set() {
            return Err("cancelled".to_string());
        }
        let find = find.map_err(|e| e.to_string())?;
        spans.push(&find);
        if admits(&find) && !matches!(find.kind, Rpf9BodyKind::Names { .. }) {
            finds.push(find);
        }
    }
    // A first entry of unstated end and a nested archive are written as the span the list gives.
    let file_len = file.metadata().map_err(|e| format!("{path}: {e}"))?.len();
    let ends = spans.ends(&file, file_len)?;
    // ⚠ The archive at offset 0 is the file itself; a nested one with no names row has no end.
    let unplaced = (finds.iter())
        .filter(|f| matches!(f.kind, Rpf9BodyKind::Archive(_)) && f.offset > 0)
        .filter(|f| !ends.contains_key(&f.offset))
        .count();
    finds.retain(|find| is_read(find) || ends.contains_key(&find.offset));
    if unplaced > 0 {
        println!("{unplaced} nested archive(s) left out: no names row ends them");
    }
    let out_dir = PathBuf::from(out_dir);
    if !dry_run {
        fs::create_dir_all(&out_dir).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let read = if form == Form::Inflated {
        read_rpf9_body
    } else {
        extract_rpf9_body
    };
    let progress = or_job::Progress::new();
    let mut ticker = super::jobs::Ticker::new(&progress);
    let mut converted = 0;
    let report = or_job::run(finds, &cancelled, &progress, |find| {
        ticker.tick();
        let name = find.file_name();
        progress.now_on(name.clone());
        if dry_run {
            return or_job::Step::Done(0);
        }
        if let (false, Some(&end)) = (is_read(&find), ends.get(&find.offset)) {
            let out = out_dir.join(&name);
            return match super::gta6_spans::write_span(&file, find.offset, end, &out) {
                Ok(len) => or_job::Step::Done(len),
                Err(e) => or_job::Step::Failed(format!("{name}: {e}")),
            };
        }
        if form == Form::Converted {
            match convert(&file, &find, limit, &out_dir) {
                Ok(true) => {
                    converted += 1;
                    return or_job::Step::Done(0);
                }
                Ok(false) => {}
                Err(e) => return or_job::Step::Failed(format!("{name}: {e}")),
            }
        }
        match read(&file, find.offset, limit) {
            Ok((_, bytes)) => match fs::write(out_dir.join(&name), &bytes) {
                Ok(()) => or_job::Step::Done(bytes.len() as u64),
                Err(e) => or_job::Step::Failed(format!("write({name}): {e}")),
            },
            Err(e) => or_job::Step::Failed(format!("{name}: {e}")),
        }
    });
    ticker.finish(&report);
    for failure in &report.failures {
        eprintln!("warn: {failure}");
    }
    if report.cancelled {
        println!("{}", report.summary());
        return Err("cancelled".to_string());
    }
    let (written, errors) = (report.counts.succeeded, report.counts.failed);
    if dry_run {
        println!(
            "Would extract {written} body(ies) into {}",
            out_dir.display()
        );
    } else if form == Form::Converted {
        println!(
            "Extracted {written} body(ies) into {}: {converted} converted, {} stored; {errors} error(s); {}",
            out_dir.display(),
            written - converted,
            Rpf9ScanFind::ORIGIN
        );
    } else {
        println!(
            "Extracted {written} body(ies) ({} bytes) into {}; {errors} error(s); {}",
            report.counts.bytes,
            out_dir.display(),
            Rpf9ScanFind::ORIGIN
        );
    }
    if errors > 0 {
        Err(format!("{errors} extraction error(s)"))
    } else {
        Ok(())
    }
}

/// The `--entry <code>` a dictionary's export takes, and the other words.
fn split_entry(args: &[String]) -> Result<(Option<u32>, Vec<String>), String> {
    match args.iter().position(|a| a == "--entry") {
        Some(i) => {
            let word = args
                .get(i + 1)
                .ok_or("--entry needs a dictionary entry's code")?;
            let code = u32::try_from(parse_offset(word)?)
                .map_err(|_| format!("{word} is not a 32-bit entry code"))?;
            let mut rest = args.to_vec();
            rest.drain(i..i + 2);
            Ok((Some(code), rest))
        }
        None => Ok((None, args.to_vec())),
    }
}

/// `rpf scan-export`: a scanned GTA VI drawable, fragment or dictionary entry to glTF, as `mesh
/// export` writes a GTA V drawable.
fn cmd_scan_export(args: &[String]) -> Result<(), Failure> {
    use or_interchange::gltf::{self, Container};
    let (limit, args) = split_limit(args).map_err(Usage)?;
    let (effects_at, args) = split_effects(&args).map_err(Usage)?;
    let (entry, args) = split_entry(&args).map_err(Usage)?;
    let (textures, args) = super::gta6_export::split_textures(&args).map_err(Usage)?;
    let every_lod = args.iter().any(|a| a == "--every-lod");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--every-lod").collect();
    let [path, offset, out] = args[..] else {
        return Err(Usage(
            "usage: rpf scan-export <archive.rpf> <offset> <out.glb|out.gltf> \
             [--every-lod] [--entry <code>] [--limit <bytes>] [--effects <archive.rpf> <offset>] \
             [--textures <pack.ttpf|folder>]"
                .to_string(),
        ));
    };
    let out = Path::new(out);
    let container = match out.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("glb") => Container::Glb,
        Some(e) if e.eq_ignore_ascii_case("gltf") => Container::Separate,
        _ => return Err(Usage("the output must end in .glb or .gltf".to_string())),
    };
    let offset = parse_offset(offset).map_err(Usage)?;
    let effects = effects_at
        .map(|at| effects_at_offset(at, limit))
        .transpose()?;
    let packs = (textures.as_deref())
        .map(super::gta6_export::open_packs)
        .transpose()?;
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return Err(Run(format!(
            "the body at {offset:#X} is no headerless GTA VI resource"
        )));
    };
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("mesh");
    let scene = super::gta6_export::export_textured(
        &resource,
        &bytes,
        signature,
        entry,
        every_lod,
        effects.as_ref(),
        stem,
        packs.as_deref(),
    )?;
    let files = gltf::write(&scene, stem, container);
    fs::write(out, &files.primary).map_err(|e| format!("write({}): {e}", out.display()))?;
    let directory = out.parent().unwrap_or(Path::new("."));
    for (name, bytes) in &files.sidecars {
        let at = directory.join(name);
        fs::write(&at, bytes).map_err(|e| format!("write({}): {e}", at.display()))?;
    }
    println!(
        "{path} at {offset:#X} -> {} ({} meshes, {} bones, {} vertices, {} triangles, {} materials)",
        out.display(),
        scene.objects.iter().map(|o| o.meshes.len()).sum::<usize>(),
        scene.objects.iter().map(|o| o.bones.len()).sum::<usize>(),
        scene.vertex_count(),
        scene.triangle_count(),
        scene.materials.len(),
    );
    for line in or_interchange::report_lines(&scene.report) {
        println!("  note: {line}");
    }
    Ok(())
}

fn cmd_scan_open(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = split_limit(args).map_err(Usage)?;
    let (effects_at, args) = split_effects(&args).map_err(Usage)?;
    let (export, args) = super::ttpf::split_export(&args).map_err(Usage)?;
    let (texts, args) = split_flag(&args, "--texts");
    let [path, offset] = args.as_slice() else {
        return Err(Usage(
            "usage: rpf scan-open <archive.rpf> <offset> [--limit <bytes>] \
             [--effects <archive.rpf> <offset>] [--export <dir> [--format png|dds]] [--texts]"
                .to_string(),
        ));
    };
    let offset = parse_offset(offset).map_err(Usage)?;
    if texts {
        return super::gta6_text::print_texts(path, offset, limit).map_err(Run);
    }
    scan_open(path, offset, limit, effects_at, export).map_err(Run)
}

/// The body at `offset` of the archive at `path` through the reader that fits it, or its textures exported.
fn scan_open(
    path: &str,
    offset: u64,
    limit: u64,
    effects_at: Option<AtOffset>,
    export: Option<super::ttpf::Export>,
) -> Result<(), String> {
    use or_io::container::rpf::rsc8::Rsc8;
    let effects = match effects_at {
        Some(at) => Some(effects_at_offset(at, limit)?),
        None => None,
    };
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = match read_rpf9_body(&file, offset, limit) {
        Ok(read) => read,
        Err(or_io::container::rpf::Rpf9ScanError::ExtentNotKnown { .. }) if offset > 0 => {
            return open_nested(path, &file, offset)
        }
        Err(e) => match carried_row(&file, offset)? {
            Some(find) => (find, Vec::new()),
            None => return Err(e.to_string()),
        },
    };
    println!("{}", scan_line(&find));
    println!("  {}", Rpf9ScanFind::ORIGIN);
    if let Some(export) = &export {
        return super::ttpf::export_scanned(&find, &bytes, export);
    }
    let name = format!("{path} at {offset:#X}");
    match find.kind {
        Rpf9BodyKind::Zlib(run) => match run.content {
            Rpf9Content::Xml | Rpf9Content::Text => {
                if find.stored_len.is_none() {
                    println!("  whether the body ends with this run is not known");
                }
                // A registration document prints as `dlc inspect` prints one; other text as it is.
                if run.content == Rpf9Content::Xml && super::dlc::inspect_document(&bytes).is_ok() {
                    return Ok(());
                }
                use std::io::Write;
                std::io::stdout()
                    .write_all(&bytes)
                    .map_err(|e| format!("write: {e}"))?;
            }
            Rpf9Content::Pso => {
                super::meta::print_info(&name, &bytes, or_io::meta::ErrorMode::Lenient)
                    .map_err(|e| format!("the PSO reader does not read this body: {e}"))?
            }
            Rpf9Content::Rbf => print_rbf(&name, &bytes)?,
            Rpf9Content::ResourceShaped => match (run.resource, run.signature) {
                (Some(resource), Some(signature)) => {
                    super::gta6_resource::print(&resource, &bytes, signature, effects.as_ref())?
                }
                (Some(resource), None) => open_headerless(&resource, &bytes, &name)?,
                // An offset table with no block map is named by its head, as object trees.
                (None, _) => return Err(NOT_A_START.to_string()),
            },
            Rpf9Content::ClipRoot => {
                let Some(resource) = run.resource else {
                    return Err(NOT_A_START.to_string());
                };
                let (virtual_section, physical) = resource.sections(&bytes);
                println!(
                    "  resource      headerless: a virtual section of {} bytes in {} block(s){}",
                    virtual_section.len(),
                    resource.virtual_blocks,
                    if physical.is_empty() {
                        ", and no physical section"
                    } else {
                        "; its physical section is not read"
                    }
                );
                super::gta6_anim::print(virtual_section, false)?
            }
            Rpf9Content::PipelineCache => super::gta6_shaders::print_pipeline_cache(&bytes)?,
            Rpf9Content::TextTable => {
                let resource = run.resource.ok_or(NOT_A_START)?;
                super::gta6_text::print_table(resource.sections(&bytes).0)?
            }
            Rpf9Content::AudioData => {
                if find.stored_len.is_none() {
                    println!("  whether the body ends with this run is not known");
                }
                super::gta6_text::print_audio(&bytes)?
            }
            Rpf9Content::Identified(body) => super::gta6_other_bodies::print(body, &bytes)?,
            Rpf9Content::Other => {
                if !super::gta6_name_pool::print(&bytes) {
                    return Err(
                        "what this body holds is not known: its first chunk inflates to \
                            neither text, PSO nor a resource's shape"
                            .to_string(),
                    );
                }
            }
        },
        Rpf9BodyKind::StoredRsc8 { .. } => {
            let rsc = Rsc8::open(&bytes).map_err(|e| e.to_string())?;
            print_stored_rsc8(&rsc)?;
        }
        Rpf9BodyKind::Awc { .. } => {
            let awc = or_io::awc::AwcFile::parse(&bytes, or_io::awc::AwcKey::default())
                .map_err(|e| e.to_string())?;
            super::awc::print_info(&name, &awc);
            super::gta6_anim::print_awc_chunks(&awc);
            if (0..awc.streams.len()).any(|s| awc.opus_streams(s).is_ok_and(|g| !g.is_empty())) {
                println!("  the streams are Opus: `awc decode-check` decodes them in memory");
            }
        }
        Rpf9BodyKind::Bink2 { revision, length } => {
            let word = |at: usize| {
                u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
            };
            let revision = char::from(revision);
            println!("  movie         Bink 2, revision {revision}, {length} bytes");
            println!(
                "  frames        {}, the largest {} bytes",
                word(0x08),
                word(0x0C)
            );
            println!("  picture       {} x {}", word(0x14), word(0x18));
            println!("  frame rate    {}/{}", word(0x1C), word(0x20));
            println!("  video flags   {:#X}", word(0x24));
            match or_media::bik::audio_tracks(&bytes) {
                Ok(tracks) => {
                    println!("  sound tracks  {}", tracks.len());
                    let decoders = or_media::Decoders::default();
                    for (i, t) in tracks.iter().enumerate() {
                        println!(
                            "    flags state {}, {} channel(s), {} Hz, {} sample frames",
                            t.codec.label(),
                            t.channels,
                            t.sample_rate,
                            t.frames
                        );
                        if let Err(e) = or_media::bik::audio_track(&bytes, i, &decoders) {
                            println!("    does not play: {e}");
                        }
                    }
                }
                Err(e) => println!("  sound tracks  {} stated; not read: {e}", word(0x28)),
            }
        }
        Rpf9BodyKind::Archive(_) => {
            return Err(format!("the archive at {offset:#X} has no known extent"))
        }
        Rpf9BodyKind::Sgd5 { .. } => super::gta6_shaders::print_sgd5(&bytes)?,
        // The placeholder's line says all it holds.
        Rpf9BodyKind::FirstEntry { .. } if find.stored_len.is_some() => {}
        Rpf9BodyKind::FirstEntry { .. } => super::gta6_scripts::print_span(&file, offset, limit)?,
        Rpf9BodyKind::Names { .. } => {
            return Err(format!(
                "the bytes at {offset:#X} are not read: {}",
                scan_line(&find)
            ))
        }
        Rpf9BodyKind::RawDeflate {
            content: Rpf9Content::Rbf,
        } => print_rbf(&name, &bytes)?,
        Rpf9BodyKind::RawDeflate { .. } => {
            super::meta::print_info(&name, &bytes, or_io::meta::ErrorMode::Lenient)
                .map_err(|e| format!("the metadata reader does not read this body: {e}"))?
        }
    }
    Ok(())
}

/// An RBF body, of a zlib run or a raw DEFLATE stream: its interface tree, then its metadata.
fn print_rbf(name: &str, bytes: &[u8]) -> Result<(), String> {
    super::gta6_uidata::print_tree(bytes)?;
    super::meta::print_scanned_rbf(name, bytes)
        .map_err(|e| format!("the RBF reader does not read this body: {e}"))
}

/// The scan of the archive nested at `offset`, from its header to the end of its names.
fn open_nested(path: &str, file: &fs::File, offset: u64) -> Result<(), String> {
    let end = super::gta6_spans::archive_end(file, offset)?;
    let end = end.ok_or_else(|| {
        format!(
            "the archive nested at {offset:#X} has no known extent: neither names a scan places \
             after its last entry nor names placed by counting the entries its header states are \
             found, and where it ends is in the encrypted table of the archive that holds it"
        )
    })?;
    println!("nested RPF9 archive at {offset:#X}, ending with its names at {end:#X}");
    list_scan(path, offset..end)
}

/// Where the archive nested at `offset` ends: past the names a scan from its header places after
/// its last entry; `None` when another header or the file's end comes first.
pub(crate) fn nested_end(file: &fs::File, offset: u64) -> Result<Option<u64>, String> {
    let scan = Rpf9Scan::new(file, offset..u64::MAX).map_err(|e| e.to_string())?;
    for find in scan {
        let find = find.map_err(|e| e.to_string())?;
        match find.kind {
            Rpf9BodyKind::Names { archive } if archive == offset => {
                return Ok(Some(find.offset + find.span))
            }
            Rpf9BodyKind::Archive(_) if find.offset != offset => return Ok(None),
            _ => {}
        }
    }
    Ok(None)
}

/// The names or first-entry row a scan from the file's start lists at `offset`: what the headers
/// before it state places it, so a read of the bytes at `offset` alone never finds it.
fn carried_row(file: &fs::File, offset: u64) -> Result<Option<Rpf9ScanFind>, String> {
    let scan = Rpf9Scan::new(file, 0..offset.saturating_add(1)).map_err(|e| e.to_string())?;
    for find in scan {
        let find = find.map_err(|e| e.to_string())?;
        let carried = matches!(
            find.kind,
            Rpf9BodyKind::Names { .. } | Rpf9BodyKind::FirstEntry { .. }
        );
        if carried && find.offset == offset {
            return Ok(Some(find));
        }
    }
    Ok(None)
}

/// Why a run with four zero bytes first that starts no resource is not opened.
const NOT_A_START: &str = "this run starts with four zero bytes but no headerless resource starts \
     here: its first 16 bytes state no block map, or neither a zero slot nor a page start comes \
     before it; a block of a resource is read from the resource's first block";

/// A headerless resource through GTA VI's RBM reader, from `bytes`, a read of `resource`.
fn open_headerless(resource: &Rpf9Resource, bytes: &[u8], name: &str) -> Result<(), String> {
    if bytes.get(0x10..0x14) != Some(b"0DRP") {
        return Err(
            "this body starts with four zero bytes but has no 0DRP at 0x10, so what it is and \
             which reader fits it are not known"
                .to_string(),
        );
    }
    let (virtual_section, physical) = resource.sections(bytes);
    // The names of GTA VI's snapshot are installed by the first ask for the schema.
    super::meta::schema();
    let (instance, layout) = match or_io::meta::binary::rbm::gta6::open(virtual_section, physical) {
        Ok(opened) => opened,
        Err(e) => {
            let why = format!("GTA VI's RBM reader does not read this body's two sections: {e}");
            // A map body's typed reader needs the class table and blocks alone, not the tree.
            return match super::gta6_map::print_body(virtual_section, physical) {
                Some(Ok(())) => {
                    println!("  tree          {why}");
                    Ok(())
                }
                Some(Err(map)) => Err(format!("{why}; {map}")),
                None => Err(why),
            };
        }
    };
    let facts =
        or_io::meta::binary::rbm::gta6::header_facts(virtual_section).map_err(|e| e.to_string())?;
    let document = (
        instance,
        Some(layout),
        or_io::meta::Diagnostics::new(or_io::meta::ErrorMode::Strict),
    );
    super::meta::print_document(name, "GTA VI RBM, headerless", bytes.len(), &document);
    if let (instance, Some(layout), _) = &document {
        let stored = [virtual_section, physical].concat();
        super::meta::print_gta6_save(layout, instance, &stored);
        super::gta6_meta_names::print_text_names(layout, instance);
    }
    if let Some(or_io::meta::binary::ContainerLayout::Rbm(rbm)) =
        document.1.as_ref().map(|l| &l.container)
    {
        print_headerless_rbm(rbm, &facts, resource);
        super::gta6_ids::print(virtual_section);
    }
    super::gta6_anim_scene::print_body(virtual_section, physical)?;
    super::gta6_ped_catalogue::print_body(virtual_section, physical)?;
    super::gta6_map::print_body(virtual_section, physical).unwrap_or(Ok(()))
}

/// The header words, block counts, string table and physical section of a headerless RBM resource.
fn print_headerless_rbm(
    rbm: &or_io::meta::binary::layout::RbmLayout,
    facts: &or_io::meta::binary::rbm::gta6::HeaderFacts,
    resource: &Rpf9Resource,
) {
    let h = &rbm.header;
    println!(
        "  header        root block {}; 0x16 {}, 0x17 {}, 0x4E {}; 0x50 {}",
        h.root_block_id,
        h.has_encrypted_strings,
        h.unknown_17h,
        h.unknown_4eh,
        match facts.word_50h {
            0 => "zero".to_string(),
            p => format!("a pointer, {p:#X}, to the table of ids below"),
        }
    );
    println!(
        "  block map     {} virtual and {} physical block(s)",
        facts.virtual_blocks, facts.physical_blocks
    );
    match facts.strings {
        Some((entries, blobs)) => println!(
            "  strings       {entries} entries, {blobs} blob bytes; the blobs are not read, since no key for them is known"
        ),
        None => println!("  strings       none"),
    }
    let values: usize = rbm.model.enums.values().map(|e| e.values.len()).sum();
    let physical: Vec<_> = rbm
        .model
        .data_blocks
        .iter()
        .filter(|b| !b.data.is_empty() && b.data_addr >> 28 == 6)
        .collect();
    let end = physical
        .iter()
        .map(|b| (b.data_addr & 0x0FFF_FFFF) + b.data.len() as u64)
        .max()
        .unwrap_or(0);
    println!(
        "  data          {values} enum values; {} physical block(s) ending at {end:#X}",
        physical.len()
    );
    let stored = |zlib| {
        resource
            .blocks
            .iter()
            .flatten()
            .filter(|b| b.physical && b.zlib == zlib)
            .count()
    };
    if resource.physical_len > 0 {
        println!(
            "  physical      {} bytes: {} block(s) zlib, {} stored raw",
            resource.physical_len,
            stored(true),
            stored(false)
        );
    }
}

/// A stored resource's header facts, then a version 65 resource through GTA VI's animation reader
/// and another version's clip-like root as the block GTA V's declared `CrBlockStream` reads at 0xB0.
fn print_stored_rsc8(rsc: &or_io::container::rpf::rsc8::Rsc8) -> Result<(), String> {
    use or_io::desc::{FieldValue, Image, ScalarValue, TypeName, Value};
    const BLOCK: usize = 0xB0;
    let id = rsc.resource_id();
    println!("  resource      RSC8, version byte {id:#04X} ({id}), stored, no key");
    println!(
        "  sections      virtual {} bytes, physical {} bytes",
        rsc.virtual_size(),
        rsc.physical_size()
    );
    if id == or_io::gta6::anim::RSC8_VERSION {
        return super::gta6_anim::print(
            &rsc.data()[..rsc.virtual_size().min(rsc.data().len())],
            true,
        );
    }
    let data = rsc.data();
    // The one root these resources are known to have: a zero vft slot and zero bytes
    // from 0x10 to 0x38, with a block stream at 0xB0.
    if data.len() < BLOCK + 0x20
        || data[..8].iter().any(|b| *b != 0)
        || data[0x10..0x38].iter().any(|b| *b != 0)
    {
        return Err(
            "its root is not the clip-like root measured on these resources; what it is is not \
             known"
                .to_string(),
        );
    }
    let pack = or_io::gta5::formats::format("ycd")
        .ok_or("the ycd declaration is missing")?
        .description(or_io::Generation::Gen8)
        .map_err(|e| e.to_string())?;
    let aligned = or_io::decl::Aligned::from_bytes(data);
    let block = or_io::desc::read_value(
        &pack,
        TypeName(or_io::decl::intern("CrBlockStream")),
        Image::new(aligned.bytes(), rsc.virtual_size()),
        BLOCK,
    )
    .map_err(|e| format!("GTA V's CrBlockStream does not read at {BLOCK:#X}: {e}"))?;
    let record = block
        .record()
        .ok_or("GTA V's CrBlockStream read as no record")?;
    println!(
        "  root          the clip-like root; its name, maps and the fields before {BLOCK:#X} \
         are not read, since no declaration states them"
    );
    println!("  block at {BLOCK:#X}  GTA V's CrBlockStream");
    for (field, value) in record.as_ref().fields() {
        let shown = match value {
            FieldValue::Scalar(ScalarValue::Int(v)) => v.to_string(),
            FieldValue::Scalar(v) => format!("{v:?}"),
            FieldValue::Child(Some(Value::Bytes(b) | Value::Opaque(b))) => {
                format!("{} trailing bytes", b.len())
            }
            FieldValue::Child(Some(Value::Array { elements, .. })) => {
                format!("{} trailing elements", elements.len())
            }
            _ => continue,
        };
        println!("    {:<20}  {shown}", field.name);
    }
    Ok(())
}

fn print_header(header: &RpfHeader) {
    println!(
        "  encryption    {:?} ({:#010X})",
        header.encryption,
        header.encryption.to_raw()
    );
    println!("  entries       {}", header.entry_count);
    println!(
        "  name heap     {} bytes (shift {})",
        header.name_heap_size, header.name_shift
    );
    if header.xcompress {
        println!("  xcompress     yes (Xenon archive)");
    }
}

/// Whether the file at `path` starts with RPF9's magic: a GTA VI archive, which a scan reads.
pub(crate) fn is_rpf9(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(path).is_ok_and(|mut file| file.read_exact(&mut magic).is_ok())
        && magic == RPF9_MAGIC.to_le_bytes()
}

/// The family `--kind` names, or the words it takes.
fn parse_family(word: &str) -> Result<Rpf9Family, String> {
    Rpf9Family::parse(word).ok_or_else(|| {
        let words: Vec<&str> = Rpf9Family::ALL.iter().map(|f| f.word()).collect();
        format!(
            "--kind: unknown family `{word}`. One of: {}",
            words.join(", ")
        )
    })
}

/// `rpf list` on a GTA VI archive: what `rpf scan` prints, the bodies of one family, or one JSON document.
fn list_rpf9(args: &[String]) -> Result<(), Failure> {
    let (json, args) = split_flag(args, "--json");
    let (kind, args) = split_value(&args, "--kind").map_err(Usage)?;
    let family = (kind.as_deref().map(parse_family).transpose()).map_err(Usage)?;
    let [path] = args.as_slice() else {
        return Err(Usage(
            "usage: rpf list <archive.rpf> [--kind <family>] [--json]".to_string(),
        ));
    };
    if !json && family.is_none() {
        return cmd_scan(&args);
    }
    for line in listed_rpf9(path, family, json)? {
        println!("{line}");
    }
    Ok(())
}

/// The lines [`list_rpf9`] prints for the archive at `path`: one JSON document, or a line per body.
pub(crate) fn listed_rpf9(
    path: &str,
    family: Option<Rpf9Family>,
    json: bool,
) -> Result<Vec<String>, Failure> {
    use std::collections::BTreeMap;
    let mut out = Vec::new();
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let mut scan = Rpf9Scan::new(&file, 0..u64::MAX).map_err(|e| e.to_string())?;
    if let (Some(family), false) = (family, json) {
        out.push(format!(
            "  kind          {}: only the bodies of that family; {}",
            family.word(),
            Rpf9ScanFind::ORIGIN
        ));
    }
    // Where each nested archive whose names were placed ends, and the runs a download has not delivered.
    let (mut finds, mut ends, mut missing) = (Vec::new(), BTreeMap::new(), Vec::new());
    let (mut after, mut counted) = (0, 0);
    let mut by_count = or_io::container::rpf::Rpf9NamesByCount::default();
    // The spans `rpf extract` writes the bodies a read gives no end as.
    let mut spans = super::gta6_spans::Spans::default();
    loop {
        let find = scan.next().transpose().map_err(|e| e.to_string())?;
        if let Some(find) = &find {
            by_count.push(find);
            spans.push(find);
        }
        // A run is counted once the scan is past it, at the next find or at the file's end.
        if scan.missing_bytes() > counted {
            missing.push(serde_json::json!({
                "from": after,
                "to": find.map_or(scan.file_len(), |find| find.offset),
                "bytes": scan.missing_bytes() - counted,
            }));
            counted = scan.missing_bytes();
        }
        let Some(find) = find else {
            break;
        };
        after = after.max(find.offset + find.span);
        if let Rpf9BodyKind::Names { archive } = find.kind {
            ends.insert(archive, find.offset + find.span);
        }
        if family.is_some_and(|f| f != Rpf9Family::of(&find)) {
            continue;
        }
        match json {
            true => finds.push(find),
            false => out.push(scan_line(&find)),
        }
    }
    // The names placed by counting their header's entries, as `rpf scan` lists them.
    for names in by_count.place(&file).map_err(|e| e.to_string())? {
        if let Rpf9BodyKind::Names { archive } = names.kind {
            ends.insert(archive, names.offset + names.span);
        }
        if family.is_some_and(|f| f != Rpf9Family::of(&names)) {
            continue;
        }
        match json {
            true => finds.insert(finds.partition_point(|f| f.offset <= names.offset), names),
            false => out.push(format!(
                "{}; placed by counting its header's entries",
                scan_line(&names)
            )),
        }
    }
    if !json {
        return Ok(out);
    }
    let spanned = spans.ends(&file, scan.file_len())?;
    let bodies: Vec<serde_json::Value> = (finds.iter())
        .map(|find| {
            let nested_in = (ends.range(..find.offset).next_back())
                .filter(|(_, end)| find.offset < **end)
                .map(|(archive, _)| *archive);
            serde_json::json!({
                "offset": find.offset,
                "kind": Rpf9Family::of(find).word(),
                "found_as": scan_row(find),
                "stored": find.stored_len,
                "inflated": find.inflated_len,
                "file": (is_read(find) || spanned.contains_key(&find.offset))
                    .then(|| find.file_name()),
                "nested_in": nested_in,
            })
        })
        .collect();
    let document = serde_json::json!({
        "archive": path,
        "length": scan.file_len(),
        "bodies": bodies,
        "missing": missing,
    });
    out.push(serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?);
    Ok(out)
}

fn cmd_list(args: &[String]) -> Result<(), Failure> {
    let (keys_override, args) = split_keys_arg(args).map_err(Usage)?;
    let path = args
        .first()
        .ok_or_else(|| Usage("missing <archive.rpf>".to_string()))?;
    if is_rpf9(Path::new(path)) {
        return list_rpf9(&args);
    }
    let provider = resolve_read_provider(Path::new(path), keys_override.as_deref())?;
    let archive = open(Path::new(path), provider)?;
    println!("/");
    archive.walk(|node, depth| {
        if depth == 0 {
            return;
        }
        let indent = "  ".repeat(depth - 1);
        let icon = match node {
            RpfNode::Directory { .. } => "📁",
            RpfNode::ResourceFile { .. } => "📄",
            RpfNode::BinaryFile { .. } => "📦",
        };
        let detail = match archive.entries.get(node.entry_index() as usize) {
            Some(RpfEntry::Binary(b)) => {
                if b.is_compressed() {
                    format!(
                        " [{} -> {} bytes, compressed]",
                        b.uncompressed_size, b.consumed_size
                    )
                } else {
                    format!(" [{} bytes]", b.uncompressed_size)
                }
            }
            Some(RpfEntry::Resource(r)) => format!(" [{} bytes, RSC7]", r.consumed_size),
            _ => String::new(),
        };
        println!("{}{} {}{}", indent, icon, node.name(), detail);
    });
    Ok(())
}

/// `rpf extract` on a GTA VI archive: the scanned bodies of a family, each as `rpf scan-extract`
/// writes it, with `--as inflated` as `rpf scan-body` does, or with `--as converted` as the verb
/// that exports a body of its kind does.
fn extract_rpf9(args: &[String]) -> Result<(), Failure> {
    let (limit, args) = split_limit(args).map_err(Usage)?;
    let (dry_run, args) = split_flag(&args, "--dry-run");
    let (kind, args) = split_value(&args, "--kind").map_err(Usage)?;
    let (form, args) = split_value(&args, "--as").map_err(Usage)?;
    let family = (kind.as_deref().map(parse_family).transpose()).map_err(Usage)?;
    let form = match form.as_deref() {
        None | Some("stored") => Form::Stored,
        Some("inflated") => Form::Inflated,
        Some("converted") => Form::Converted,
        Some(other) => {
            return Err(Usage(format!(
                "--as: unknown form `{other}`. One of: stored, inflated, converted"
            )))
        }
    };
    let [path, out_dir] = args.as_slice() else {
        return Err(Usage(
            "usage: rpf extract <archive.rpf> <out_dir> [--kind <family>] \
             [--as stored|inflated|converted] [--limit <bytes>] [--dry-run]"
                .to_string(),
        ));
    };
    println!(
        "  form          {}, and every other body as a read returns it",
        match form {
            Form::Inflated => "inflated: a headerless resource is written as its virtual section then its physical",
            Form::Stored => "stored: a headerless resource is written as it is stored, which `rpf scan-open <file> 0` reads again",
            Form::Converted => "converted: a texture dictionary is written as its textures in PNG and an audio container as its streams, each in a folder of its name, a model as a .glb and a metadata body as XML; any other headerless resource is written as it is stored",
        }
    );
    let admits = |find: &Rpf9ScanFind| family.is_none_or(|f| f == Rpf9Family::of(find));
    extract_scanned(path, out_dir, limit, dry_run, form, admits).map_err(Run)
}

/// Extract, in one of three named forms, optionally filtered, cancellably.
///
/// ⚠ A resource's raw bytes lack their RSC7 header — the page flags live in
/// the entry table, not the body — so `--mode as-stored` on a `.ydr` writes a
/// file of the right length that nothing will open. Those files are counted
/// and reported once.
fn cmd_extract(args: &[String]) -> Result<(), Failure> {
    let (keys_override, args) = split_keys_arg(args).map_err(Usage)?;
    let archive_path = args
        .first()
        .ok_or_else(|| Usage("missing <archive.rpf>".to_string()))?;
    if is_rpf9(Path::new(archive_path)) {
        return extract_rpf9(&args);
    }
    let out_dir = args
        .get(1)
        .ok_or_else(|| Usage("missing <out_dir>".to_string()))?;

    let mut form = BodyForm::Decoded;
    let mut filter: Option<or_title::vfs::Pattern> = None;
    let mut dry_run = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| Usage("--mode needs a value".to_string()))?;
                form = BodyForm::parse(v).ok_or_else(|| {
                    Usage(format!(
                        "--mode: unknown form `{v}`. One of: as-stored (or raw), \
                         decrypted, decoded"
                    ))
                })?;
                i += 2;
            }
            "--filter" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| Usage("--filter needs a glob".to_string()))?;
                filter = Some(or_title::vfs::Pattern::new(v));
                i += 2;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => return Err(Usage(format!("unknown argument: {other}"))),
        }
    }

    let provider = resolve_read_provider(Path::new(archive_path), keys_override.as_deref())?;
    let archive = open(Path::new(archive_path), provider)?;
    let out_dir = PathBuf::from(out_dir);
    if !dry_run {
        fs::create_dir_all(&out_dir).map_err(|e| format!("create_dir_all: {}", e))?;
    }

    let cancelled = super::jobs::on_interrupt();
    let progress = or_job::Progress::new();
    let mut ticker = super::jobs::Ticker::new(&progress);

    let mut filtered: u32 = 0;
    let mut not_openable: u32 = 0;

    let indices: Vec<u32> = (0..archive.entries.len() as u32).collect();
    let report = or_job::run(indices, &cancelled, &progress, |entry_index| {
        ticker.tick();
        let Some(entry) = archive.entries.get(entry_index as usize) else {
            return or_job::Step::Skipped;
        };
        let path = match archive.entry_path(entry_index) {
            Some(p) if !p.is_empty() => p,
            _ => return or_job::Step::Skipped,
        };
        progress.now_on(path.clone());

        if let RpfEntry::Directory(_) = entry {
            if dry_run {
                return or_job::Step::Skipped;
            }
            let dest = out_dir.join(&path);
            return match fs::create_dir_all(&dest) {
                Ok(()) => or_job::Step::Skipped,
                Err(e) => or_job::Step::Failed(format!("create_dir_all({}): {e}", dest.display())),
            };
        }

        if let Some(pattern) = &filter {
            if !pattern.matches_path(&path) {
                filtered += 1;
                return or_job::Step::Skipped;
            }
        }
        if !form.is_loadable_for(entry.kind()) {
            not_openable += 1;
        }
        if dry_run {
            return or_job::Step::Done(0);
        }

        let dest = out_dir.join(&path);
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match archive.read_form(entry_index, form) {
            Ok(bytes) => match fs::write(&dest, &bytes) {
                Ok(()) => or_job::Step::Done(bytes.len() as u64),
                Err(e) => or_job::Step::Failed(format!("write({}): {e}", dest.display())),
            },
            Err(e) => or_job::Step::Failed(format!("extract {path}: {e}")),
        }
    });
    ticker.finish(&report);

    for failure in &report.failures {
        eprintln!("warn: {failure}");
    }
    let written = report.counts.succeeded;
    let errors = report.counts.failed;

    if report.cancelled {
        println!("{}", report.summary());
        return Err(Run("cancelled".to_string()));
    }
    if dry_run {
        println!(
            "Would extract {written} file(s) in `{}` form ({filtered} filtered out)",
            form.name()
        );
    } else {
        println!(
            "Extracted {written} file(s) ({} bytes) in `{}` form into {}; \
             {filtered} filtered out, {errors} error(s)",
            report.counts.bytes,
            form.name(),
            out_dir.display(),
        );
    }
    if not_openable > 0 {
        println!(
            "  ⚠ {not_openable} of those are not openable in `{}` form — a binary \
             entry stays deflated and a resource has no RSC7 header, because its \
             page flags live in the entry table. Use `--mode decoded` for files \
             something else has to read.",
            form.name()
        );
    }
    if errors > 0 {
        Err(Run(format!("{} extraction error(s)", errors)))
    } else {
        Ok(())
    }
}

fn cmd_create(args: &[String]) -> Result<(), String> {
    let (provider, keys, args) = parse_keys_arg_full(args)?;
    let src_dir = args
        .first()
        .ok_or_else(|| "missing <src_dir>".to_string())?;
    let out_path = args.get(1).ok_or_else(|| "missing <out.rpf>".to_string())?;
    let mut name_shift = 0u8;
    let mut encryption = RpfEncryption::Open;
    let mut version = RpfVersion::Rpf7;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--version needs a value".to_string())?;
                version = RpfVersion::ALL
                    .into_iter()
                    .find(|candidate| candidate.name().eq_ignore_ascii_case(v))
                    .ok_or_else(|| format!("--version: unknown archive version '{v}'"))?;
                i += 2;
            }
            "--name-shift" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--name-shift needs a value".to_string())?;
                name_shift = v
                    .parse::<u8>()
                    .map_err(|e| format!("--name-shift: {}", e))?;
                i += 2;
            }
            "--encryption" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "--encryption needs a value".to_string())?;
                encryption = match v.to_ascii_lowercase().as_str() {
                    "open" => RpfEncryption::Open,
                    "aes" => RpfEncryption::Aes,
                    "ng" => RpfEncryption::Ng,
                    other => {
                        return Err(format!(
                            "--encryption: unknown mode '{other}' (expected open|aes|ng)"
                        ))
                    }
                };
                i += 2;
            }
            other => return Err(format!("unknown option: {}", other)),
        }
    }

    if version != RpfVersion::Rpf7 && (encryption.needs_crypto() || name_shift != 0) {
        return Err(format!(
            "--name-shift and --encryption aes|ng are RPF7's; {version} is written in the clear"
        ));
    }
    if encryption.needs_crypto() && !provider.supports(encryption) {
        return Err(format!(
            "--encryption {:?} requires --keys; pass `--keys <path>` to a cache built by `extract-keys`",
            encryption
        ));
    }

    // An AES-only key cache (`extract-keys --aes-only`) has no NG decrypt
    // tables, so it passes `supports` and still cannot encrypt NG.
    if matches!(encryption, RpfEncryption::Ng | RpfEncryption::Other(_)) {
        let retail = keys.as_ref().ok_or_else(|| {
            "--encryption ng requires --keys; pass `--keys <path>` to a cache built by \
             `extract-keys`"
                .to_string()
        })?;
        if !retail.has_ng() {
            return Err(
                "--encryption ng requires NG decrypt tables, and your --keys cache has none. \
                 No shipped executable carries them: re-run `openrage-cli rpf extract-keys`, \
                 which takes them from your magic.dat beside the keys cache or else the key \
                 snapshot bundled with OpenRage, or pass `--magic <magic.dat>` to name one."
                    .to_string(),
            );
        }
        eprintln!(
            "note: NG encrypt: first save derives ~25 MiB of inverse tables (~10 s). \
             Subsequent saves with the same keys reuse the cached state."
        );
    }

    let src_dir = PathBuf::from(src_dir);
    if !src_dir.is_dir() {
        return Err(format!("not a directory: {}", src_dir.display()));
    }

    let mut builder = RpfBuilder::new(version)
        .name_shift(name_shift)
        .encrypted(encryption, provider);
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(src_dir.clone());
    let mut count: usize = 0;
    while let Some(dir) = queue.pop_front() {
        let read = fs::read_dir(&dir).map_err(|e| format!("read_dir({}): {}", dir.display(), e))?;
        for entry in read {
            let entry = entry.map_err(|e| format!("dir entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                queue.push_back(path);
                continue;
            }
            let rel = path
                .strip_prefix(&src_dir)
                .map_err(|e| format!("strip_prefix: {}", e))?
                .to_string_lossy()
                .replace('\\', "/");
            builder
                .insert_file(&rel, &path)
                .map_err(|e| format!("add {}: {}", rel, e))?;
            count += 1;
        }
    }

    let out_path = PathBuf::from(out_path);
    builder
        .save(&out_path)
        .map_err(|e| format!("save: {}", e))?;
    println!(
        "Wrote {} ({} files, {version}, encryption={:?}, name-shift={})",
        out_path.display(),
        count,
        encryption,
        name_shift
    );
    Ok(())
}

/// Rebuild an archive with no slack, and say what that would cost first.
///
/// ⚠ It reports by default and writes only with `--write`: a bug in writing an
/// archive costs a user their install. The rebuild is [`Defrag`], which the
/// editor's Defragment runs too.
fn cmd_defrag(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = split_keys_arg(args)?;
    let archive_path = args
        .first()
        .ok_or_else(|| "missing <archive.rpf>".to_string())?;
    let mut write = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--write" => write = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let path = Path::new(archive_path);
    let provider = resolve_read_provider(path, keys_override.as_deref())?;
    let archive = open(path, provider)?;

    let before = fs::metadata(path)
        .map_err(|e| format!("stat({}): {e}", path.display()))?
        .len();

    let defrag = Defrag::plan(&archive, |_, _| {}).map_err(|e| e.to_string())?;
    let after = defrag.planned;

    let delta = after as i64 - before as i64;
    println!("{}", path.display());
    println!(
        "  {} file(s), {} byte(s) of payload",
        defrag.files, defrag.payload
    );
    println!(
        "  encryption {:?}, name_shift {}",
        archive.header.encryption, archive.header.name_shift
    );
    println!("  now {before} byte(s), after a rebuild {after} ({delta:+})");
    if delta < 0 {
        println!("  would reclaim {} byte(s)", -delta);
    } else if delta == 0 {
        println!("  nothing to reclaim");
    } else {
        println!(
            "  ⚠ the rebuild is LARGER. That is not slack — check whether every \
             entry's compression decision survived the round trip before writing."
        );
    }

    if !write {
        println!("  (preview only; pass --write to commit)");
        return Ok(());
    }

    let backup = path.with_extension("rpf.bak");
    if defrag.write_over(path).map_err(|e| e.to_string())? {
        println!("  backed up to {}", backup.display());
    } else {
        println!("  {} exists; leaving it alone", backup.display());
    }
    println!("  written");
    Ok(())
}

fn cmd_extract_keys(args: &[String]) -> Result<(), String> {
    let mut positional: Vec<&String> = Vec::new();
    let mut mode = ExtractionMode::Full;
    let mut magic: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--aes-only" => mode = ExtractionMode::AesOnly,
            "--magic" => {
                let p = iter
                    .next()
                    .ok_or_else(|| "--magic needs a path".to_string())?;
                magic = Some(PathBuf::from(p));
            }
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            _ => positional.push(arg),
        }
    }
    if magic.is_some() && mode == ExtractionMode::AesOnly {
        return Err(
            "--aes-only and --magic are mutually exclusive (--magic already implies AES + decode)"
                .to_string(),
        );
    }
    let exe_path = positional
        .first()
        .ok_or_else(|| "missing <gta5.exe>".to_string())?;
    let out_path = positional
        .get(1)
        .ok_or_else(|| "missing <out_keys.bin>".to_string())?;
    let exe_path = PathBuf::from(exe_path.as_str());
    let out_path = PathBuf::from(out_path.as_str());

    let progress_cb = move |p: ExtractionProgress| {
        eprintln!("  [{}] {} / {}", p.stage, p.found, p.total);
    };
    let fingerprint = fingerprint_executable(&exe_path)
        .map_err(|e| format!("fingerprint {}: {e}", exe_path.display()))?;
    let keys = if let Some(magic_path) = magic {
        extract_keys_from_magic(&exe_path, &magic_path, &progress_cb)
            .map_err(|e| format!("magic decode from {}: {e}", magic_path.display()))?
    } else {
        let snapshot = crate::keys::user_snapshot();
        extract_keys_with_snapshot(&exe_path, mode, &snapshot, &progress_cb)
            .map_err(|e| format!("extract from {}: {e}", exe_path.display()))?
    };
    let cache = KeysCache::build(exe_path.clone(), fingerprint.clone(), &keys);
    cache
        .write_to(&out_path)
        .map_err(|e| format!("write {}: {e}", out_path.display()))?;
    println!(
        "Wrote {} (sha1 {}, size {})",
        out_path.display(),
        fingerprint.sha1_hex,
        fingerprint.size
    );
    for line in keys.provenance.report() {
        println!("  {line}");
    }
    Ok(())
}

fn cmd_import(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = split_keys_arg(args)?;
    let archive_path = args
        .first()
        .ok_or_else(|| "missing <archive.rpf>".to_string())?;
    let src_file = args
        .get(1)
        .ok_or_else(|| "missing <src_file>".to_string())?;
    let dest_path = args
        .get(2)
        .ok_or_else(|| "missing <dest_path>".to_string())?;

    let data = fs::read(src_file).map_err(|e| format!("read({src_file}): {e}"))?;
    let mut builder = edit(archive_path, keys_override.as_deref())?;
    builder
        .insert(dest_path, data)
        .map_err(|e| format!("add({dest_path}): {e}"))?;
    save(&builder, archive_path)?;
    println!("Imported {src_file} -> {dest_path}");
    Ok(())
}

fn cmd_mv(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = split_keys_arg(args)?;
    let archive_path = args
        .first()
        .ok_or_else(|| "missing <archive.rpf>".to_string())?;
    let from = args
        .get(1)
        .ok_or_else(|| "missing <from_path>".to_string())?;
    let to = args.get(2).ok_or_else(|| "missing <to_path>".to_string())?;

    let mut builder = edit(archive_path, keys_override.as_deref())?;
    let moved = builder
        .rename(from, to)
        .map_err(|e| format!("move({from}): {e}"))?;
    if moved == 0 {
        return Err(format!("Path not found: {from}"));
    }
    save(&builder, archive_path)?;
    println!("Moved {from} -> {to}");
    Ok(())
}

fn cmd_rm(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = split_keys_arg(args)?;
    let archive_path = args
        .first()
        .ok_or_else(|| "missing <archive.rpf>".to_string())?;
    let target = args.get(1).ok_or_else(|| "missing <path>".to_string())?;

    let mut builder = edit(archive_path, keys_override.as_deref())?;
    let removed = builder.remove(target);
    if removed == 0 {
        return Err(format!("Path not found: {target}"));
    }
    save(&builder, archive_path)?;
    println!("Removed {removed} entry(s) at {target}");
    Ok(())
}

fn open(path: &Path, keys: Arc<dyn RpfCryptoProvider>) -> Result<RpfArchive, String> {
    RpfArchive::open_with(path, keys).map_err(|e| format!("parse: {e}"))
}

/// A builder holding the archive at `path`, to change and save back, keeping its encryption.
fn edit(path: &str, keys_override: Option<&Path>) -> Result<RpfBuilder, String> {
    let path = Path::new(path);
    let keys = resolve_read_provider(path, keys_override)?;
    open(path, keys)?.edit().map_err(|e| format!("edit: {e}"))
}

fn save(builder: &RpfBuilder, path: &str) -> Result<(), String> {
    builder.save(path).map_err(|e| format!("save: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_body_refuses_to_write_over_the_archive_it_reads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.rpf");
        fs::write(&path, [1u8; 64]).expect("written");
        let p = path.to_string_lossy().into_owned();
        let args = [p.clone(), "0x10".to_string(), p];
        assert!(matches!(cmd_scan_body(&args), Err(Usage(_))));
        assert_eq!(fs::read(&path).expect("kept"), [1u8; 64]);
    }

    /// A 64-byte RPF9 head of layout 4 that holds every measured rule: `entries` entries, the
    /// table to `toc_end` with its index rows at its end, and `names` bytes of names.
    fn head(toc_end: u32, entries: u32, names: u32) -> Vec<u8> {
        let mut head = vec![0u8; RPF9_HEADER_SIZE];
        let index = toc_end - (entries * 9).next_multiple_of(16);
        for (at, word) in [
            (0x00, RPF9_MAGIC),
            (0x04, toc_end),
            (0x08, index),
            (0x0C, names),
            (0x1C, 0x150),
            (0x24, entries),
            (0x30, 0x10),
        ] {
            head[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        head[0x10..0x18].copy_from_slice(&[0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07, 0x18]);
        head[0x19] = 0x7A;
        head[0x1A] = 4;
        head[0x20] = 5;
        head[0x22] = 1;
        head[0x2C] = 1;
        head[0x2D] = 0x0F;
        head[0x2E] = 0x09;
        head
    }

    /// A 64-byte RPF9 head with a table and a names blob after it.
    fn rpf9(len: usize) -> Vec<u8> {
        let mut bytes: Vec<u8> = (0..0x300u32).map(|i| (i * 37 + 11) as u8).collect();
        bytes[..RPF9_HEADER_SIZE].copy_from_slice(&head(0x2D0, 5, 0x30));
        bytes.truncate(len);
        bytes
    }

    fn words(args: &[&Path]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_rpf_verbs_describe_and_scan_an_rpf9_archive_and_refuse_to_change_it() {
        let dir = tempfile::tempdir().expect("a folder");
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, rpf9(0x300)).expect("written");
        let out = dir.path().join("out");

        assert!(describe_rpf9(&archive).expect("described"));
        for verb in [
            vec![Path::new("info"), &archive],
            vec![Path::new("list"), &archive],
            vec![Path::new("extract"), &archive, &out],
        ] {
            assert_eq!(run(&words(&verb)), ExitCode::SUCCESS, "{verb:?}");
        }
        for verb in [
            vec![Path::new("defrag"), &archive],
            vec![Path::new("rm"), &archive, Path::new("a.xml")],
        ] {
            assert_eq!(run(&words(&verb)), ExitCode::FAILURE, "{verb:?}");
        }
        assert_eq!(
            resolve_read_provider(&archive, None).err(),
            Some(rpf9_refusal())
        );
        let said = rpf9_refusal();
        for words in [
            "rpf9 list",
            "rpf9 copy",
            "by hash",
            or_io::container::rpf::GTA6_SNAPSHOT_ENV,
            "the game refuses",
        ] {
            assert!(said.contains(words), "{said}");
        }
    }

    /// Where the table of [`rpf9_with_a_body`]'s head ends, and so where its body starts.
    const BODY_AT: u32 = 0x1A0;

    /// An RPF9 head that holds every measured rule, its table, and one zlib XML body after it.
    fn rpf9_with_a_body(text: &[u8]) -> (Vec<u8>, u64) {
        let toc_end = BODY_AT;
        let mut bytes = vec![0x5Au8; toc_end as usize];
        bytes[..RPF9_HEADER_SIZE].copy_from_slice(&head(toc_end, 1, 0x40));
        // A zlib stream of one stored deflate block, under the `78 DA` head the scan looks for.
        let len = text.len() as u16;
        bytes.extend([0x78, 0xDA, 0x01]);
        bytes.extend(len.to_le_bytes());
        bytes.extend((!len).to_le_bytes());
        bytes.extend_from_slice(text);
        let (a, b) = text.iter().fold((1u32, 0u32), |(a, b), &c| {
            let a = (a + u32::from(c)) % 65_521;
            (a, (b + a) % 65_521)
        });
        bytes.extend(((b << 16) | a).to_be_bytes());
        (bytes, u64::from(toc_end))
    }

    /// `text` as a zlib stream of one stored deflate block.
    fn zlib(text: &[u8]) -> Vec<u8> {
        let (bytes, at) = rpf9_with_a_body(text);
        bytes[at as usize..].to_vec()
    }

    /// `bodies` after `head`, each on the next 4 KiB page, and where each starts.
    fn with_bodies(mut bytes: Vec<u8>, bodies: &[Vec<u8>]) -> (Vec<u8>, Vec<String>) {
        let mut offsets = Vec::new();
        for body in bodies {
            bytes.resize(bytes.len().next_multiple_of(0x1000), 0);
            offsets.push(format!("{:#X}", bytes.len()));
            bytes.extend_from_slice(body);
        }
        (bytes, offsets)
    }

    /// A stored RSC8 whose virtual section is the clip-like root: zeros to 0x38 but a pointer at
    /// 0x08, and GTA V's 0x20-byte block stream header at 0xB0 with `data` trailing it.
    fn clip_like_rsc8(data: &[u8]) -> Vec<u8> {
        let size = (0xD0 + data.len()).next_multiple_of(16);
        let mut section = vec![0u8; size];
        section[0x08..0x10].copy_from_slice(&(0x5000_0000u64 + 0xC0).to_le_bytes());
        let block = &mut section[0xB0..0xD0];
        for (at, word) in [
            (0x00, 0x1234_5678u32),
            (0x04, data.len() as u32),
            (0x10, data.len() as u32 + 32),
        ] {
            block[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
        block[0x14..0x16].copy_from_slice(&2u16.to_le_bytes());
        block[0x16..0x18].copy_from_slice(&3u16.to_le_bytes());
        block[0x1E] = 11;
        section[0xD0..0xD0 + data.len()].copy_from_slice(data);
        let mut bytes = b"RSC8".to_vec();
        for word in [0x1F41u32, size as u32 | 4, 1] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend(section);
        bytes
    }

    /// An `ADAT` container of one stream holding a `data` and a 0x76 chunk.
    fn adat() -> Vec<u8> {
        let tables = 16 + 4 + 2 * 8;
        let mut bytes = b"ADAT".to_vec();
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(0xFF02u16.to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        bytes.extend((tables as u32).to_le_bytes());
        bytes.extend((2u32 << 29 | 0x123).to_le_bytes());
        for (kind, size, at) in [
            (0x55u64, 64u64, tables as u64),
            (0x76, 32, tables as u64 + 64),
        ] {
            bytes.extend((kind << 56 | size << 28 | at).to_le_bytes());
        }
        bytes.extend((0..96).map(|i| i as u8 | 1));
        bytes
    }

    fn bink2() -> Vec<u8> {
        let mut bytes = b"KB2n".to_vec();
        for word in [0x100u32 - 8, 2, 0x40, 2, 1920, 1080, 30, 1, 0x4_0013, 0] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.resize(0x100, 0x11);
        bytes
    }

    #[test]
    fn scan_open_reads_each_kind_through_its_reader_and_refuses_what_is_not_known() {
        let dir = tempfile::tempdir().expect("a folder");
        let (head, _) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        let mut lform = b"PSIN\0\0\0\x10\0\0\0\0\0\0\0\0".to_vec();
        lform.extend(b"CHKS\0\0\0\x14\0\0\0\x24\0\0\0\0\0\0\0\0");
        let mut not_clip = clip_like_rsc8(&[7; 0x40]);
        not_clip[16 + 0x20] = 1;
        let (bytes, at) = with_bodies(
            head,
            &[
                zlib(b"plain names\r\nand more\r\n"),
                clip_like_rsc8(&[7; 0x40]),
                adat(),
                bink2(),
                zlib(&lform),
                zlib(&[0; 0x40]),
                zlib(&[0x11; 0x40]),
                not_clip,
            ],
        );
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");
        let open = |at: &str| run(&words(&[Path::new("scan-open"), &archive, Path::new(at)]));
        for (i, read) in [(BODY_AT, true)]
            .into_iter()
            .map(|(a, r)| (format!("{a:#X}"), r))
            .chain(
                at.iter()
                    .cloned()
                    .zip([true, true, true, true, false, false, false, false]),
            )
        {
            let want = if read {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
            assert_eq!(open(&i), want, "the body at {i}");
        }
        assert_eq!(open("0x0"), ExitCode::FAILURE, "a nested archive's extent");
    }

    #[test]
    fn scan_open_lists_a_pool_of_names_and_refuses_a_body_that_is_none() {
        let dir = tempfile::tempdir().expect("a folder");
        let (head, _) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        let (bytes, at) = with_bodies(
            head,
            &[
                zlib(b"first_name\0second_name\0"),
                zlib(b"first_name\0\x01\x02\0"),
            ],
        );
        let archive = dir.path().join("audio.rpf");
        fs::write(&archive, &bytes).expect("written");
        let open = |at: &str| run(&words(&[Path::new("scan-open"), &archive, Path::new(at)]));
        assert_eq!(open(&at[0]), ExitCode::SUCCESS, "a pool of names");
        assert_eq!(open(&at[1]), ExitCode::FAILURE, "bytes no name holds");
    }

    #[test]
    fn scan_open_finds_the_first_entry_row_a_scan_lists_after_the_table_which_a_read_alone_does_not(
    ) {
        let dir = tempfile::tempdir().expect("a folder");
        let mut bytes = vec![0x5Au8; BODY_AT as usize];
        bytes[..RPF9_HEADER_SIZE].copy_from_slice(&head(BODY_AT, 1, 0x40));
        // Bytes no detector names, where the header puts the first entry.
        bytes.extend([0xA7; 0x40]);
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");
        let file = fs::File::open(&archive).expect("opened");
        let at = u64::from(BODY_AT);
        assert!(
            read_rpf9_body(&file, at, SCAN_BODY_LIMIT).is_err(),
            "a read of the bytes alone finds nothing there"
        );
        let find = (carried_row(&file, at).expect("scanned")).expect("a scan lists a row there");
        assert!(
            matches!(find.kind, Rpf9BodyKind::FirstEntry { .. }),
            "{find:?}"
        );
        assert_eq!(carried_row(&file, at + 16).expect("scanned"), None);
        let opened = run(&words(&[
            Path::new("scan-open"),
            &archive,
            Path::new("0x1A0"),
        ]));
        // The span from the first entry is listed: its one piece, as long as the names the header
        // states, is those names, so no opaque body is left to refuse.
        assert_eq!(
            opened,
            ExitCode::SUCCESS,
            "the first entry's span is listed"
        );
    }

    #[test]
    fn a_first_entry_that_is_the_empty_archive_placeholder_is_named_so_and_not_opaque() {
        // As the scan lists it in a nested archive of one entry: its 16 bytes' end stated.
        let find = Rpf9ScanFind {
            offset: 0xFA61_C280,
            kind: Rpf9BodyKind::FirstEntry { head: *b"FORC" },
            span: 16,
            stored_len: Some(16),
            inflated_len: Some(16),
            container: None,
        };
        let line = scan_line(&find);
        assert!(line.contains("FORCE_EMPTY_RPF"), "{line}");
        assert!(!line.contains("no detector"), "{line}");
    }

    #[test]
    fn the_empty_archive_placeholder_is_written_by_every_body_verb_and_opens_alone() {
        use or_io::container::rpf::EMPTY_ARCHIVE_PLACEHOLDER;
        let dir = tempfile::tempdir().expect("a folder");
        let mut bytes = vec![0x5Au8; BODY_AT as usize];
        bytes[..RPF9_HEADER_SIZE].copy_from_slice(&head(BODY_AT, 1, 0x40));
        bytes.extend_from_slice(EMPTY_ARCHIVE_PLACEHOLDER);
        bytes.extend([0xA7; 0x40]);
        let archive = dir.path().join("empty.rpf");
        fs::write(&archive, &bytes).expect("written");
        let file = fs::File::open(&archive).expect("opened");
        let offset = u64::from(BODY_AT);
        let (find, read) = read_rpf9_body(&file, offset, u64::MAX).expect("read");
        assert_eq!(find.stored_len, Some(16));
        assert_eq!(read, EMPTY_ARCHIVE_PLACEHOLDER.to_vec());
        assert!(is_read(&find));
        let at = format!("{offset:#X}");
        let one = dir.path().join("one.bin");
        let scan_body = [Path::new("scan-body"), &archive, Path::new(&at), &one];
        assert_eq!(run(&words(&scan_body)), ExitCode::SUCCESS);
        assert_eq!(fs::read(&one).expect("scan-body"), read);
        for verb in ["scan-extract", "extract"] {
            let out = dir.path().join(verb);
            assert_eq!(
                run(&words(&[Path::new(verb), &archive, &out])),
                ExitCode::SUCCESS,
                "{verb}"
            );
            let written = fs::read(out.join(find.file_name())).expect(verb);
            assert_eq!(written, read, "{verb}");
        }
        let open = [Path::new("scan-open"), &archive, Path::new(&at)];
        assert_eq!(run(&words(&open)), ExitCode::SUCCESS);
    }

    #[test]
    fn scan_open_takes_an_effects_archive_and_offset_and_refuses_one_that_is_no_effect_container() {
        let args: Vec<String> = ["a.rpf", "--effects", "b.rpf", "0x2000", "0x100"]
            .map(String::from)
            .to_vec();
        let (at, rest) = split_effects(&args).expect("parsed");
        assert_eq!(at, Some(("b.rpf".to_string(), 0x2000)));
        assert_eq!(rest, ["a.rpf", "0x100"].map(String::from));
        assert!(split_effects(&["--effects".to_string(), "b.rpf".to_string()]).is_err());

        let dir = tempfile::tempdir().expect("a folder");
        let (bytes, body) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");
        let body = format!("{body:#X}");
        let at = Path::new(&body);
        let opened = run(&words(&[Path::new("scan-open"), &archive, at]));
        assert_eq!(opened, ExitCode::SUCCESS);
        let flag = Path::new("--effects");
        let refused = run(&words(&[
            Path::new("scan-open"),
            &archive,
            at,
            flag,
            &archive,
            at,
        ]));
        assert_eq!(
            refused,
            ExitCode::FAILURE,
            "the XML body is no SGD5 container"
        );
    }

    #[test]
    fn scan_takes_a_range_and_scan_open_lists_a_nested_archive_whose_names_were_placed() {
        let (top, _) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        let mut nested = head(0x1B0, 2, 0x40);
        nested.resize(0x1B0, 0x5A);
        nested.extend(zlib(b"<?xml version=\"1.0\"?>\n<CMapTypes/>\n"));
        nested.resize(nested.len().next_multiple_of(16), 0);
        nested.extend([0x33; 0x40]);
        let mut last = head(0x1A0, 1, 0x40);
        last.resize(0x1A0, 0x5A);
        let nested_len = nested.len() as u64;
        let (bytes, at) = with_bodies(top, &[nested, last]);
        assert_eq!(at, ["0x1000", "0x2000"]);
        let dir = tempfile::tempdir().expect("a folder");
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");

        let file = fs::File::open(&archive).expect("opened");
        assert_eq!(nested_end(&file, 0x1000), Ok(Some(0x1000 + nested_len)));
        assert_eq!(nested_end(&file, 0x2000), Ok(None), "no names of its own");
        let rpf = |verb: &str, extra: &[&str]| {
            let mut args = vec![verb.to_string(), archive.to_string_lossy().into_owned()];
            args.extend(extra.iter().map(|w| w.to_string()));
            run(&args)
        };
        assert_eq!(rpf("scan-open", &["0x1000"]), ExitCode::SUCCESS);
        assert_eq!(rpf("scan-open", &["0x2000"]), ExitCode::FAILURE);
        assert_eq!(
            rpf("scan", &["--from", "0x1000", "--to", "0x2000"]),
            ExitCode::SUCCESS
        );
        for refused in [&["--from", "0x2000", "--to", "0x1000"][..], &["--to"]] {
            assert_eq!(rpf("scan", refused), ExitCode::from(2), "{refused:?}");
        }
        let args = ["a.rpf", "--from", "4096"].map(String::from);
        let (from, rest) = split_offset_flag(&args, "--from").expect("parsed");
        assert_eq!((from, rest), (Some(4096), vec!["a.rpf".to_string()]));
    }

    #[test]
    fn scan_export_takes_an_entry_code_and_refuses_a_body_that_is_no_headerless_resource() {
        let args: Vec<String> = ["a.rpf", "--entry", "0x1234ABCD", "0x100"]
            .map(String::from)
            .to_vec();
        let (entry, rest) = split_entry(&args).expect("parsed");
        assert_eq!(entry, Some(0x1234_ABCD));
        assert_eq!(rest, ["a.rpf", "0x100"].map(String::from));
        assert!(split_entry(&["--entry".to_string(), "0x1_0000_0000".to_string()]).is_err());

        let dir = tempfile::tempdir().expect("a folder");
        let (bytes, body) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");
        let body = format!("{body:#X}");
        let at = Path::new(&body);
        for (out, code) in [
            ("out.glb", ExitCode::FAILURE),
            ("out.obj", ExitCode::from(2)),
        ] {
            let out = dir.path().join(out);
            let refused = run(&words(&[Path::new("scan-export"), &archive, at, &out]));
            assert_eq!(refused, code, "{}", out.display());
            assert!(!out.exists(), "nothing is written");
        }
    }

    #[test]
    fn scan_runs_on_an_archive_whose_head_has_not_arrived() {
        let dir = tempfile::tempdir().expect("a folder");
        let (mut bytes, _) = rpf9_with_a_body(b"<?xml version=\"1.0\"?>\n<SSetupData/>\n");
        bytes[..RPF9_HEADER_SIZE].fill(0);
        let archive = dir.path().join("story000.rpf");
        fs::write(&archive, &bytes).expect("written");
        assert_eq!(
            run(&words(&[Path::new("scan"), &archive])),
            ExitCode::SUCCESS
        );
        bytes[0x10] = 1;
        fs::write(&archive, &bytes).expect("written");
        assert_eq!(
            run(&words(&[Path::new("scan"), &archive])),
            ExitCode::FAILURE
        );
    }

    #[test]
    fn scan_lists_an_rpf9_archive_and_scan_body_writes_a_body_inflated() {
        let dir = tempfile::tempdir().expect("a folder");
        let text = b"<?xml version=\"1.0\"?>\n<SSetupData>\n</SSetupData>\n";
        let (bytes, body_at) = rpf9_with_a_body(text);
        let archive = dir.path().join("common.rpf");
        fs::write(&archive, &bytes).expect("written");
        let out = dir.path().join("body.xml");
        let at_word = format!("{BODY_AT:#X}");
        let at = Path::new(&at_word);
        assert_eq!(body_at, u64::from(BODY_AT));

        assert_eq!(
            run(&words(&[Path::new("scan"), &archive])),
            ExitCode::SUCCESS
        );
        assert_eq!(
            run(&words(&[Path::new("scan-body"), &archive, at, &out])),
            ExitCode::SUCCESS
        );
        assert_eq!(fs::read(&out).expect("the body"), text);

        let short = Path::new("16");
        let refused = dir.path().join("refused.bin");
        for verb in [
            vec![Path::new("scan-body"), &archive, Path::new("0x0"), &refused],
            vec![
                Path::new("scan-body"),
                &archive,
                Path::new("0x90"),
                &refused,
            ],
            vec![
                Path::new("scan-body"),
                &archive,
                at,
                &refused,
                Path::new("--limit"),
                short,
            ],
        ] {
            assert_eq!(run(&words(&verb)), ExitCode::FAILURE, "{verb:?}");
        }
        assert!(!refused.exists(), "a refused body was written");

        let other = dir.path().join("other.rpf");
        fs::write(&other, b"8FPR and the rest").expect("written");
        assert_eq!(run(&words(&[Path::new("scan"), &other])), ExitCode::FAILURE);
    }

    #[test]
    fn only_an_rpf9_head_is_described_and_a_short_one_is_refused() {
        let dir = tempfile::tempdir().expect("a folder");
        let other = dir.path().join("other.rpf");
        fs::write(&other, b"8FPR and the rest").expect("written");
        assert_eq!(describe_rpf9(&other), Ok(false));
        assert_eq!(describe_rpf9(&dir.path().join("absent.rpf")), Ok(false));

        let short = dir.path().join("short.rpf");
        fs::write(&short, rpf9(RPF9_HEADER_SIZE - 1)).expect("written");
        assert!(describe_rpf9(&short).is_err());
    }
}
