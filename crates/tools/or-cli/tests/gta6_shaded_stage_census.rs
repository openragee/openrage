//! How many stages of the effects a sample of GTA VI's drawables names Shaded translates, and why
//! it refuses the rest: each effect's own mesh and pixel passes as the app translates them, and
//! where none of those draws, its vertex and pixel passes; the table is printed, by reason.
//!
//! Install-gated: it skips cleanly without the build.
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`, `shaders.rpf` and `pack/*/*.rpf`

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_cli::{
    gta6_view_passes, is_gta6_shading_pass, translate_gta6_mesh_stage, translate_gta6_pixel_stage,
    translate_gta6_program, Gta6PixelStage,
};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Role, Rpf9Scan};
use or_io::gta5::shaders::sgd5::{Sgd5Effect, Sgd5File, Sgd5Program};
use or_io::gta5::shaders::sgd5_pack::vertex_pixel_passes;
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{read, Drawable, Entry, Resource};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const SHADERS: &str = "shaders.rpf";
/// Each n-th model body a scan finds is sampled.
const EVERY: u64 = 4;
const MOST_DRAWABLES: usize = 2_000;
const SCAN_BUDGET: Duration = Duration::from_secs(240);
const TRANSLATE_BUDGET: Duration = Duration::from_secs(360);
const BODY_LIMIT: u64 = 1 << 30;
const CONTAINER_BUDGET: Duration = Duration::from_secs(240);
/// The most bytes of the containers kept at once.
const CONTAINER_BYTES: usize = 4 << 30;
const PANICKED: &str = "the translation panicked";

/// One stage's outcomes: translated, and refused by reason.
#[derive(Default)]
struct Stages {
    translated: usize,
    refused: BTreeMap<String, usize>,
}

impl Stages {
    fn count(&mut self, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.translated += 1,
            Err(why) => *self.refused.entry(why).or_default() += 1,
        }
    }
}

/// By (path, stage): `own` is an effect's mesh and pixel passes, `vertex-pixel` the passes the
/// app translates for an effect none of whose own passes draws.
type Tally = BTreeMap<(&'static str, &'static str), Stages>;

fn archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let rpf = path.extension().and_then(|e| e.to_str()) == Some("rpf");
            let shaders = (path.file_name().and_then(|n| n.to_str()))
                .is_some_and(|n| n.eq_ignore_ascii_case(SHADERS));
            if rpf && !shaders {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in (std::fs::read_dir(root.join("pack")).into_iter().flatten()).flatten() {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    // The smaller first, so that the time spent scanning reaches more archives.
    out.sort_by_key(|path| std::fs::metadata(path).map_or(u64::MAX, |m| m.len()));
    out
}

/// The effects every n-th drawable of the build's archives names, and how many drawables were
/// sampled.
fn sampled_effects(root: &Path) -> (usize, BTreeSet<u32>) {
    let started = Instant::now();
    let (mut drawables, mut effects, mut seen) = (0usize, BTreeSet::new(), 0u64);
    let take = |drawable: &Drawable, effects: &mut BTreeSet<u32>| {
        effects.extend(drawable.shaders.iter().map(|s| s.name_hash));
        1
    };
    'archives: for path in archives(root) {
        let (Ok(scanned), Ok(reader)) = (File::open(&path), File::open(&path)) else {
            continue;
        };
        let Ok(scan) = Rpf9Scan::new(scanned, 0..u64::MAX) else {
            continue;
        };
        for find in scan {
            if drawables >= MOST_DRAWABLES || started.elapsed() > SCAN_BUDGET {
                break 'archives;
            }
            let Ok(find) = find else {
                break;
            };
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
            if !model {
                continue;
            }
            seen += 1;
            if !(seen - 1).is_multiple_of(EVERY) {
                continue;
            }
            let Ok((_, bytes)) = read_rpf9_body(&reader, find.offset, BODY_LIMIT) else {
                continue;
            };
            let (virtual_section, physical) = resource.sections(&bytes);
            match read(signature, virtual_section, Some(physical)) {
                Ok(Resource::Drawable(d)) => drawables += take(&d, &mut effects),
                Ok(Resource::DrawableWithBound(r)) => drawables += take(&r.drawable, &mut effects),
                Ok(Resource::Dictionary(dict)) => {
                    for (_, entry) in &dict.entries {
                        if let Ok(Entry::Drawable(d)) = entry {
                            drawables += take(d, &mut effects);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    println!(
        "  sampled       {drawables} drawables of {seen} model bodies (every {EVERY}) in {:?}, naming {} effects",
        started.elapsed(),
        effects.len()
    );
    (drawables, effects)
}

/// The `SGD5` containers of `shaders.rpf` that hold one of `wanted`'s effects, with the bytes their
/// programs' streams lie in, read until the budget of time or bytes is spent.
fn containers(archive: &Path, wanted: &BTreeSet<u32>) -> Vec<(Vec<u8>, Sgd5File)> {
    let (Ok(scanned), Ok(reader)) = (File::open(archive), File::open(archive)) else {
        return Vec::new();
    };
    let Ok(scan) = Rpf9Scan::new(scanned, 0..u64::MAX) else {
        return Vec::new();
    };
    let (started, mut kept, mut out) = (Instant::now(), 0usize, Vec::new());
    for find in scan.map_while(Result::ok) {
        if started.elapsed() > CONTAINER_BUDGET || kept > CONTAINER_BYTES {
            break;
        }
        if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
            continue;
        }
        let Ok((_, bytes)) = read_rpf9_body(&reader, find.offset, BODY_LIMIT) else {
            continue;
        };
        let Ok(file) = Sgd5File::open(&bytes) else {
            continue;
        };
        let held = |e: &Sgd5Effect| wanted.contains(&name_hash32(e.name.as_bytes()));
        if file.effects.iter().any(held) {
            kept += bytes.len();
            out.push((bytes, file));
        }
    }
    out
}

/// `f`, with a panic said as a refusal of its own.
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(PANICKED.to_string()))
}

/// `effect`'s stages counted into `tally` as the app translates them for Shaded.
fn translate_effect(bytes: &[u8], file: &Sgd5File, effect: &Sgd5Effect, tally: &mut Tally) {
    let inflated = |program: &Sgd5Program| {
        let stream = bytes.get(program.stream.clone())?;
        program.inflate(stream).ok()
    };
    let parameters = &effect.parameters;
    let mut seen = HashSet::new();
    let mut pixels: HashMap<usize, Result<Gta6PixelStage, String>> = HashMap::new();
    let mut draws = false;
    for (first, pixel) in gta6_view_passes(file, effect) {
        if seen.contains(&first.at) {
            continue;
        }
        let (Some(mesh), Some(fragment)) = (inflated(first), inflated(pixel)) else {
            continue;
        };
        if !is_gta6_shading_pass(&mesh, &fragment) {
            continue;
        }
        seen.insert(first.at);
        let stage = pixels.entry(pixel.at).or_insert_with(|| {
            let stage =
                guarded(|| translate_gta6_pixel_stage(&fragment, &pixel.variables, parameters));
            (tally.entry(("own", "pixel")).or_default())
                .count(stage.as_ref().map(|_| ()).map_err(Clone::clone));
            stage
        });
        let Ok(stage) = &*stage else {
            *(tally.entry(("own", "vertex")).or_default().refused)
                .entry("not tried: its pass's pixel stage is refused".to_string())
                .or_default() += 1;
            continue;
        };
        let pass =
            guarded(|| translate_gta6_mesh_stage(&mesh, &first.variables, parameters, stage));
        draws |= pass.is_ok();
        (tally.entry(("own", "vertex")).or_default()).count(pass.map(|_| ()));
    }
    if draws {
        return;
    }
    let mut read = HashSet::new();
    for (_, vertex, pixel) in vertex_pixel_passes(file, effect) {
        for (program, stage) in [(vertex, "vertex"), (pixel, "pixel")] {
            if !read.insert(program.at) {
                continue;
            }
            let Some(stream) = bytes.get(program.stream.clone()) else {
                continue;
            };
            let outcome = guarded(|| translate_gta6_program(program, stream, Some(parameters)))
                .and_then(|done| done.translated.map(|_| ()).map_err(|why| why.name()));
            (tally.entry(("vertex-pixel", stage)).or_default()).count(outcome);
        }
    }
}

#[test]
fn shaded_stages_of_sampled_drawables_are_counted_by_outcome_and_reason() {
    let Some(root) = std::env::var_os(BUILD_ENV).map(PathBuf::from) else {
        skip_install(
            "shaded_stages_of_sampled_drawables_are_counted_by_outcome_and_reason",
            format!("{BUILD_ENV} is unset"),
        );
        return;
    };
    let (drawables, effects) = sampled_effects(&root);
    assert!(drawables > 0, "no drawable of the build was sampled");

    let containers = containers(&root.join(SHADERS), &effects);
    println!(
        "  containers    {} effect containers read",
        containers.len()
    );
    let started = Instant::now();
    let (mut tally, mut found, mut done) = (Tally::new(), 0usize, 0usize);
    for hash in &effects {
        if started.elapsed() > TRANSLATE_BUDGET {
            break;
        }
        done += 1;
        let named = |effect: &&Sgd5Effect| name_hash32(effect.name.as_bytes()) == *hash;
        let held = (containers.iter())
            .find_map(|(bytes, file)| Some((bytes, file, file.effects.iter().find(named)?)));
        if let Some((bytes, file, effect)) = held {
            found += 1;
            translate_effect(bytes, file, effect, &mut tally);
        }
    }
    println!(
        "  effects       {done} of {} looked for in {:?}, {found} held by a container",
        effects.len(),
        started.elapsed()
    );
    for ((path, stage), stages) in &tally {
        let refused: usize = stages.refused.values().sum();
        println!(
            "  {path:<13} {stage:<6} {} translated, {refused} refused",
            stages.translated
        );
        let mut reasons: Vec<(&String, &usize)> = stages.refused.iter().collect();
        reasons.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        for (reason, count) in reasons {
            println!("    {count:>6}  {reason}");
        }
    }
    for stages in tally.values() {
        for reason in stages.refused.keys() {
            assert!(!reason.trim().is_empty(), "a refusal carries no reason");
        }
    }
}
