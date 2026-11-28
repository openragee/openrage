//! The deferred frame read as targets and passes, held to the source: every place of the renderer
//! that begins a pass or copies a texture is entered here with what draws through it, and the
//! steps of the two frames `render_deferred` draws are listed in order with the kind of each, so
//! that the kinds a declared list of passes needs are counted and not estimated.
//!
//! A pass added to or taken from the source, a call that records into the frame added to, taken
//! from or moved within `render_deferred`, or a label this list names that its file no longer
//! holds, fails here until the list is read again; a step's kind, program and count are a reading
//! entered by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_renderer::game_shaders::deferred::{
    DEPTH_STENCIL_FORMAT, GBUFFER_FORMAT, GBUFFER_TARGETS, HDR_FORMAT,
};
use or_renderer::game_shaders::room_mask::MOST_BOXES;
use or_renderer::game_shaders::shadow::MAP_FORMAT;
use or_renderer::game_shaders::tone::BLOOM_SIZES;

/// What draws through a place that begins a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draws {
    /// The deferred frame of `render_deferred`.
    Frame,
    /// What a viewport draws after any frame, over it: lines, models and particles.
    Over,
    /// A GTA VI model's own passes in Shaded, which are not `render_deferred`'s.
    Model,
    /// Not a frame: a texture made once, a read-back, a tool or the window's clear.
    Apart,
}

/// Each source file that begins a render pass, a compute pass or a copy between textures, with
/// how many places of each it holds.
const SITES: [(&str, Draws, [usize; 3]); 21] = [
    ("game_shaders/ambient.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/bloom.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/deferred.rs", Draws::Frame, [2, 0, 0]),
    ("game_shaders/dome.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/frame.rs", Draws::Apart, [3, 0, 0]),
    ("game_shaders/grade.rs", Draws::Frame, [1, 0, 1]),
    ("game_shaders/meshlets.rs", Draws::Model, [3, 0, 0]),
    // The forward surfaces over the lit frame, and before them what each lets through of it.
    ("game_shaders/meshlets/forward.rs", Draws::Model, [2, 0, 0]),
    ("game_shaders/meshlets/sun_mask.rs", Draws::Model, [1, 0, 0]),
    ("game_shaders/shadow.rs", Draws::Frame, [2, 0, 0]),
    ("game_shaders/stand_in.rs", Draws::Apart, [2, 0, 0]),
    // The sun's mask of a model written by rays, a tile at a time.
    ("game_shaders/trace/pass.rs", Draws::Model, [0, 3, 1]),
    ("stencil/mod.rs", Draws::Apart, [1, 0, 0]),
    ("surface.rs", Draws::Apart, [1, 0, 0]),
    // The pass every step over the lit frame opens, then the three a viewport draws over a frame.
    ("viewport.rs", Draws::Over, [4, 0, 0]),
    ("viewport/game_frame.rs", Draws::Frame, [4, 0, 1]),
    ("viewport/meter.rs", Draws::Frame, [0, 1, 0]),
    ("viewport/post.rs", Draws::Frame, [1, 0, 1]),
    ("world/cards_bake.rs", Draws::Apart, [2, 0, 2]),
    ("world/shadow_terrain.rs", Draws::Frame, [2, 0, 0]),
    // A helper of tests, compiled for them alone.
    ("game_shaders/test_pack.rs", Draws::Apart, [1, 0, 0]),
];

/// The calls counted in a file, in the order of a site's three numbers.
const CALLS: [&str; 3] = [
    ".begin_render_pass(",
    ".begin_compute_pass(",
    ".copy_texture_to_texture(",
];

/// The frames `render_deferred` draws today.
const LEGACY: u8 = 1;
const WORLD: u8 = 2;
const BOTH: u8 = LEGACY | WORLD;

/// What a step is, as a list of passes would state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// A scene's surfaces, each by the program of its own material for the pass's route.
    Surfaces,
    /// The host's own buckets, drawn by its fallback shader.
    Buckets,
    /// One program over the whole of its target.
    Screen,
    /// One program over a mesh the host keeps, once for each record of a list the frame wrote.
    Instances,
    /// One texture copied into another.
    Copy,
    /// A compute program whose counts are read back a frame or two late.
    Measure,
}

/// Where a step's program comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum From {
    /// The game's own program, of the pack.
    Game,
    /// A shader of OpenRage's.
    Host,
    /// No program.
    None,
}

/// How often a step is drawn in a frame that draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Times {
    Once,
    /// As many as stated.
    Fixed(usize),
    /// Once for each step of the bloom's own list, `plan` of its module.
    BloomPlan,
    /// Once for each box that lamps light, at most [`MOST_BOXES`], and once more.
    Rooms,
}

/// Why a step cannot be a row of a list of passes as it stands; none where it can.
type Named = Option<&'static str>;

/// One step of a frame: the call of `render_deferred` that records it, the pass it is drawn in by
/// that pass's label and the file that holds the label, what it writes, which frames draw it, its
/// kind, its program and how often.
struct Step {
    call: &'static str,
    label: &'static str,
    file: &'static str,
    writes: &'static str,
    frames: u8,
    kind: Kind,
    from: From,
    times: Times,
    named: Named,
}

const fn step(
    call: &'static str,
    (label, file): (&'static str, &'static str),
    writes: &'static str,
    frames: u8,
    kind: Kind,
    from: From,
) -> Step {
    Step {
        call,
        label,
        file,
        writes,
        frames,
        kind,
        from,
        times: Times::Once,
        named: None,
    }
}

const FRAME: &str = "viewport/game_frame.rs";

/// The steps of the deferred frame in the order `render_deferred` records them.
fn steps() -> Vec<Step> {
    use Kind::*;
    let tiles = "four tiles of one target, each with its viewport, its view and the instances the CPU culled to it";
    let rooms = "as many pairs of passes as the frame has boxes, the stencil marks kept or cleared by whether two boxes share their lamps";
    let history = "two targets swapped from frame to frame, a camera offset every earlier step takes, and a history thrown away by how far the camera went";
    let read_back = "counts read back a frame or two late, which set a later frame's exposure, and no pass while a read is pending";
    let sun_turned = "drawn only when the sun has turned since the texture was last marched";
    vec![
        step(
            "scene.record_gbuffer",
            ("viewport-gbuffer-pass", FRAME),
            "G-buffer, depth and stencil",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "world.draw_gbuffer",
            ("viewport-gbuffer-pass", FRAME),
            "G-buffer, depth and stencil",
            WORLD,
            Buckets,
            From::Host,
        ),
        step(
            "scene.record_shadow",
            ("sun-shadow-cast", "game_shaders/shadow.rs"),
            "shadow map",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        Step {
            named: Some(tiles),
            ..step(
                "world.draw_shadow",
                ("sun-shadow-cast", "game_shaders/shadow.rs"),
                "shadow map",
                WORLD,
                Buckets,
                From::Host,
            )
        },
        step(
            "passes.record_shadow_receive",
            ("sun-shadow-receive", "game_shaders/shadow.rs"),
            "G-buffer 2, alpha",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            named: Some(sun_turned),
            ..step(
                "terrain.record",
                ("world-terrain-bake", "world/shadow_terrain.rs"),
                "terrain shadow",
                WORLD,
                Screen,
                From::Host,
            )
        },
        step(
            "terrain.record",
            ("world-terrain-receive", "world/shadow_terrain.rs"),
            "G-buffer 2, alpha",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "passes.record_ambient",
            ("ssao-downscale", "game_shaders/ambient.rs"),
            "half depth",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_ambient",
            ("ssao-process", "game_shaders/ambient.rs"),
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_ambient",
            ("ssao-blur-x", "game_shaders/ambient.rs"),
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_ambient",
            ("ssao-blur-y", "game_shaders/ambient.rs"),
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_ambient",
            ("ssao-upscale", "game_shaders/ambient.rs"),
            "G-buffer 3, red and green",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_lighting",
            ("viewport-lighting-pass", FRAME),
            "lit frame",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            times: Times::Fixed(3),
            ..step(
                "passes.record_lighting",
                ("viewport-lighting-pass", FRAME),
                "lit frame",
                BOTH,
                Instances,
                From::Game,
            )
        },
        Step {
            times: Times::Rooms,
            named: Some(rooms),
            ..step(
                "passes.record_rooms",
                ("room-mask", "game_shaders/deferred.rs"),
                "stencil",
                WORLD,
                Instances,
                From::Host,
            )
        },
        Step {
            times: Times::Rooms,
            named: Some(rooms),
            ..step(
                "passes.record_rooms",
                ("room-lights", "game_shaders/deferred.rs"),
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "passes.record_sky_noise",
            ("sky-noise-pass", "game_shaders/dome.rs"),
            "cloud noise",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            times: Times::Fixed(3),
            ..step(
                "passes.record_sky",
                ("viewport-sky-pass", FRAME),
                "lit frame",
                BOTH,
                Surfaces,
                From::Game,
            )
        },
        step(
            "passes.record_sky",
            ("viewport-sky-pass", FRAME),
            "lit frame",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "scene.record_water_fog",
            ("viewport-water-fog-pass", FRAME),
            "lit frame and a sink",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "world.draw_glow",
            ("viewport-glow-pass", FRAME),
            "lit frame",
            WORLD,
            Buckets,
            From::Host,
        ),
        Step {
            times: Times::Fixed(2),
            ..step(
                "passes.record_distant_lights",
                ("viewport-glow-pass", FRAME),
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "encoder.copy_texture_to_texture",
            ("refraction", FRAME),
            "copy of the lit frame",
            BOTH,
            Copy,
            From::None,
        ),
        step(
            "scene.record_blended",
            ("viewport-blended-pass", FRAME),
            "lit frame",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "world.draw_lit",
            ("viewport-blended-pass", FRAME),
            "lit frame",
            WORLD,
            Buckets,
            From::Host,
        ),
        Step {
            times: Times::Fixed(2),
            ..step(
                "passes.record_distant_lights",
                ("viewport-blended-pass", FRAME),
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "passes.record_fog",
            ("viewport-fog-volume-pass", FRAME),
            "lit frame",
            BOTH,
            Instances,
            From::Game,
        ),
        Step {
            named: Some(history),
            ..step(
                "post.record_temporal",
                ("post", "viewport/post.rs"),
                "history",
                WORLD,
                Screen,
                From::Host,
            )
        },
        step(
            "post.record_temporal",
            ("post", "viewport/post.rs"),
            "lit frame",
            WORLD,
            Copy,
            From::None,
        ),
        Step {
            times: Times::Fixed(2),
            named: Some(read_back),
            ..step(
                "meter.record",
                ("viewport-meter", "viewport/meter.rs"),
                "counts",
                BOTH,
                Measure,
                From::Host,
            )
        },
        step(
            "grade.record",
            ("grade", "game_shaders/grade.rs"),
            "scratch",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "grade.record",
            ("grade", "game_shaders/grade.rs"),
            "lit frame",
            WORLD,
            Copy,
            From::None,
        ),
        Step {
            // The halvings, the doublings and the composite.
            times: Times::Fixed(BLOOM_SIZES + (BLOOM_SIZES - 1) + 1),
            ..step(
                "post.record_bloom",
                ("post", "viewport/post.rs"),
                "bloom sizes, scratch",
                WORLD,
                Screen,
                From::Host,
            )
        },
        Step {
            times: Times::Fixed(2),
            ..step(
                "post.record_bloom",
                ("post", "viewport/post.rs"),
                "local light, lit frame",
                WORLD,
                Copy,
                From::None,
            )
        },
        Step {
            times: Times::BloomPlan,
            ..step(
                "passes.record_bloom",
                ("bloom", "game_shaders/bloom.rs"),
                "bloom buffers, lit frame",
                LEGACY,
                Screen,
                From::Game,
            )
        },
        step(
            "passes.record_resolve",
            ("viewport-resolve-pass", FRAME),
            "mapped frame or screen",
            BOTH,
            Screen,
            From::Game,
        ),
        // Where nothing was drawn: the sky's frame mapped in place of the background, else the bloom that spills there.
        step(
            "passes.record_background",
            ("viewport-resolve-pass", FRAME),
            "mapped frame or screen",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "passes.record_halo",
            ("viewport-resolve-pass", FRAME),
            "mapped frame or screen",
            LEGACY,
            Screen,
            From::Game,
        ),
        step(
            "post.record_sharpen",
            ("post", "viewport/post.rs"),
            "screen",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "passes.record_antialias",
            ("viewport-antialias-pass", FRAME),
            "screen",
            LEGACY,
            Screen,
            From::Game,
        ),
    ]
}

fn source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `dir`, but the tests'.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir)
        .expect("the source is read")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

/// How many of each of [`CALLS`] the code of `text` holds, its own `mod tests` left out.
fn calls(text: &str) -> [usize; 3] {
    let code = text
        .split("#[cfg(test)]\nmod tests {")
        .next()
        .unwrap_or(text);
    CALLS.map(|call| code.matches(call).count())
}

/// The call of `render_deferred` that opens the pass both casts of the sun's shadow draw in.
const OPENS: &str = "passes.record_shadow_cast";

/// The calls in the body of `render_deferred` that record into the frame, in the order the source holds them, each as the last name of what it is called on and its own.
fn recorded(text: &str) -> Vec<String> {
    let body = (text.split("fn render_deferred(").nth(1)).expect("the frame's function is there");
    // The function's own closing brace is the first at a method's depth.
    let body = body.split("\n    }\n").next().unwrap_or(body);
    let name = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut calls = Vec::new();
    for (dot, _) in body.match_indices('.') {
        let after = &body[dot + 1..];
        let len = after.bytes().take_while(|b| name(*b)).count();
        let (method, rest) = after.split_at(len);
        let records = method == "record"
            || method.starts_with("record_")
            || method.starts_with("draw_")
            || method == "copy_texture_to_texture";
        if !records || !rest.starts_with('(') {
            continue;
        }
        let on = body[..dot].trim_end().trim_end_matches(')');
        let start = on.bytes().rposition(|b| !name(b)).map_or(0, |at| at + 1);
        calls.push(format!("{}.{method}", &on[start..]));
    }
    calls
}

#[test]
fn every_place_that_begins_a_pass_is_entered_with_what_draws_through_it() {
    let root = source();
    let mut found = Vec::new();
    files(&root, &mut found);
    let mut held: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    for path in found {
        let text = std::fs::read_to_string(&path).expect("a source file reads");
        let counted = calls(&text);
        if counted != [0; 3] {
            let relative = path.strip_prefix(&root).expect("under the source");
            let name = relative.to_string_lossy().replace('\\', "/");
            held.insert(name, counted);
        }
    }
    let entered: BTreeMap<String, [usize; 3]> = (SITES.iter())
        .map(|(file, _, counts)| ((*file).to_string(), *counts))
        .collect();
    assert_eq!(
        held, entered,
        "the source's passes (left) are not the ones entered (right): read the pass that changed and enter it"
    );
}

#[test]
fn the_frame_is_listed_in_the_passes_the_source_labels_and_the_kinds_it_needs_are_counted() {
    let root = source();
    let steps = steps();
    let frame_files: BTreeSet<&str> = (SITES.iter())
        .filter(|(_, draws, _)| *draws == Draws::Frame)
        .map(|(file, _, _)| *file)
        .collect();
    for step in &steps {
        assert!(
            frame_files.contains(step.file),
            "{}: {} is not a file of the frame",
            step.label,
            step.file
        );
        let text = std::fs::read_to_string(root.join(step.file)).expect("a source file reads");
        assert!(
            text.contains(&format!("\"{}\"", step.label)),
            "{} no longer labels a pass {:?}",
            step.file,
            step.label
        );
    }
    // Every file of the frame is named by a step: a pass of the frame the list leaves out fails.
    let listed: BTreeSet<&str> = steps.iter().map(|s| s.file).collect();
    assert_eq!(listed, frame_files);
    // The steps are in the order the frame's function records them, and none of its calls is left out.
    let text = std::fs::read_to_string(root.join(FRAME)).expect("the frame's source reads");
    let recorded: Vec<String> = (recorded(&text).into_iter())
        .filter(|call| call != OPENS)
        .collect();
    let mut listed: Vec<&str> = steps.iter().map(|s| s.call).collect();
    listed.dedup();
    assert_eq!(
        recorded, listed,
        "what render_deferred records (left) is not what the steps list (right): read the call that changed and enter it"
    );
    // What a call is made on says whose program draws: a scene's surfaces by the game's, the world's buckets by the host's, and a copy by none.
    for step in &steps {
        let whose = match step.call.split_once('.') {
            Some(("scene", _)) => (Kind::Surfaces, From::Game),
            Some(("world", _)) => (Kind::Buckets, From::Host),
            Some(("encoder", _)) => (Kind::Copy, From::None),
            _ => continue,
        };
        assert_eq!((step.kind, step.from), whose, "{}", step.call);
    }

    // The targets a pass kind states are of the formats the frame is made in.
    eprintln!(
        "targets: {GBUFFER_TARGETS} of {GBUFFER_FORMAT:?} with {DEPTH_STENCIL_FORMAT:?}; the lit frame {HDR_FORMAT:?}; the shadow map {MAP_FORMAT:?}"
    );
    let mut kinds: BTreeMap<u8, BTreeSet<Kind>> = BTreeMap::new();
    let mut named: BTreeMap<u8, BTreeSet<&'static str>> = BTreeMap::new();
    let mut by_program: BTreeMap<(u8, bool), usize> = BTreeMap::new();
    for step in &steps {
        for frame in [LEGACY, WORLD] {
            if step.frames & frame == 0 {
                continue;
            }
            kinds.entry(frame).or_default().insert(step.kind);
            if let Some(why) = step.named {
                named.entry(frame).or_default().insert(why);
            }
            if step.from != From::None {
                *by_program
                    .entry((frame, step.from == From::Game))
                    .or_default() += 1;
            }
        }
        let frames = match step.frames {
            LEGACY => "legacy",
            WORLD => "world",
            _ => "both",
        };
        eprintln!(
            "{:<26} {frames:<6} {:<9?} {:<5?} {:?} -> {}{}",
            step.label,
            step.kind,
            step.from,
            step.times,
            step.writes,
            step.named
                .map_or(String::new(), |why| format!("  [named: {why}]"))
        );
    }
    for (frame, name) in [(LEGACY, "GTA V Legacy"), (WORLD, "the GTA VI World screen")] {
        eprintln!(
            "{name}: {} kinds {:?}; {} named; {} step rows of the game's programs, {} of OpenRage's",
            kinds[&frame].len(),
            kinds[&frame],
            named.get(&frame).map_or(0, BTreeSet::len),
            by_program.get(&(frame, true)).copied().unwrap_or(0),
            by_program.get(&(frame, false)).copied().unwrap_or(0),
        );
    }
    // What the reference states: the kinds each frame needs, and the passes no kind states.
    assert_eq!(kinds[&LEGACY].len(), 5, "{:?}", kinds[&LEGACY]);
    assert_eq!(kinds[&WORLD].len(), 6, "{:?}", kinds[&WORLD]);
    assert!(!kinds[&LEGACY].contains(&Kind::Buckets));
    assert_eq!(named.get(&LEGACY).map_or(0, BTreeSet::len), 1);
    assert_eq!(named[&WORLD].len(), 5);
    let rooms = steps.iter().filter(|s| s.times == Times::Rooms).count();
    eprintln!("the rooms' two passes are drawn once a box, for {MOST_BOXES} boxes at most");
    assert_eq!(rooms, 2);
}
