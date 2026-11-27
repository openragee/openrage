//! The deferred frame read as targets and passes, held to the source: every place of the renderer
//! that begins a pass or copies a texture is entered here with what draws through it, and the
//! steps of the two frames `render_deferred` draws are listed in order with the kind of each, so
//! that the kinds a declared list of passes needs are counted and not estimated.
//!
//! A pass added to or taken from the source, or a label this list names that its file no longer
//! holds, fails here until the list is read again.

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
const SITES: [(&str, Draws, [usize; 3]); 19] = [
    ("game_shaders/ambient.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/bloom.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/deferred.rs", Draws::Frame, [2, 0, 0]),
    ("game_shaders/dome.rs", Draws::Frame, [1, 0, 0]),
    ("game_shaders/frame.rs", Draws::Apart, [3, 0, 0]),
    ("game_shaders/grade.rs", Draws::Frame, [1, 0, 1]),
    ("game_shaders/meshlets.rs", Draws::Model, [3, 0, 0]),
    ("game_shaders/meshlets/sun_mask.rs", Draws::Model, [1, 0, 0]),
    ("game_shaders/shadow.rs", Draws::Frame, [2, 0, 0]),
    ("game_shaders/stand_in.rs", Draws::Apart, [2, 0, 0]),
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

/// One step of a frame: the pass it is drawn in by that pass's label and the file that holds the
/// label, what it writes, which frames draw it, its kind, its program and how often.
struct Step {
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
    label: &'static str,
    file: &'static str,
    writes: &'static str,
    frames: u8,
    kind: Kind,
    from: From,
) -> Step {
    Step {
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
            "viewport-gbuffer-pass",
            FRAME,
            "G-buffer, depth and stencil",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "viewport-gbuffer-pass",
            FRAME,
            "G-buffer, depth and stencil",
            WORLD,
            Buckets,
            From::Host,
        ),
        step(
            "sun-shadow-cast",
            "game_shaders/shadow.rs",
            "shadow map",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        Step {
            named: Some(tiles),
            ..step(
                "sun-shadow-cast",
                "game_shaders/shadow.rs",
                "shadow map",
                WORLD,
                Buckets,
                From::Host,
            )
        },
        step(
            "sun-shadow-receive",
            "game_shaders/shadow.rs",
            "G-buffer 2, alpha",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            named: Some(sun_turned),
            ..step(
                "world-terrain-bake",
                "world/shadow_terrain.rs",
                "terrain shadow",
                WORLD,
                Screen,
                From::Host,
            )
        },
        step(
            "world-terrain-receive",
            "world/shadow_terrain.rs",
            "G-buffer 2, alpha",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "ssao-downscale",
            "game_shaders/ambient.rs",
            "half depth",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "ssao-process",
            "game_shaders/ambient.rs",
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "ssao-blur-x",
            "game_shaders/ambient.rs",
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "ssao-blur-y",
            "game_shaders/ambient.rs",
            "half occlusion",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "ssao-upscale",
            "game_shaders/ambient.rs",
            "G-buffer 3, red and green",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "viewport-lighting-pass",
            FRAME,
            "lit frame",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            times: Times::Fixed(3),
            ..step(
                "viewport-lighting-pass",
                FRAME,
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
                "room-mask",
                "game_shaders/deferred.rs",
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
                "room-lights",
                "game_shaders/deferred.rs",
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "sky-noise-pass",
            "game_shaders/dome.rs",
            "cloud noise",
            BOTH,
            Screen,
            From::Game,
        ),
        Step {
            times: Times::Fixed(3),
            ..step(
                "viewport-sky-pass",
                FRAME,
                "lit frame",
                BOTH,
                Surfaces,
                From::Game,
            )
        },
        step(
            "viewport-sky-pass",
            FRAME,
            "lit frame",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "viewport-water-fog-pass",
            FRAME,
            "lit frame and a sink",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "viewport-glow-pass",
            FRAME,
            "lit frame",
            WORLD,
            Buckets,
            From::Host,
        ),
        Step {
            times: Times::Fixed(2),
            ..step(
                "viewport-glow-pass",
                FRAME,
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "refraction",
            FRAME,
            "copy of the lit frame",
            BOTH,
            Copy,
            From::None,
        ),
        step(
            "viewport-blended-pass",
            FRAME,
            "lit frame",
            LEGACY,
            Surfaces,
            From::Game,
        ),
        step(
            "viewport-blended-pass",
            FRAME,
            "lit frame",
            WORLD,
            Buckets,
            From::Host,
        ),
        Step {
            times: Times::Fixed(2),
            ..step(
                "viewport-blended-pass",
                FRAME,
                "lit frame",
                WORLD,
                Instances,
                From::Game,
            )
        },
        step(
            "viewport-fog-volume-pass",
            FRAME,
            "lit frame",
            BOTH,
            Instances,
            From::Game,
        ),
        Step {
            named: Some(history),
            ..step(
                "post",
                "viewport/post.rs",
                "history",
                WORLD,
                Screen,
                From::Host,
            )
        },
        step(
            "post",
            "viewport/post.rs",
            "lit frame",
            WORLD,
            Copy,
            From::None,
        ),
        Step {
            times: Times::Fixed(2),
            named: Some(read_back),
            ..step(
                "viewport-meter",
                "viewport/meter.rs",
                "counts",
                BOTH,
                Measure,
                From::Host,
            )
        },
        step(
            "grade",
            "game_shaders/grade.rs",
            "scratch",
            WORLD,
            Screen,
            From::Host,
        ),
        step(
            "grade",
            "game_shaders/grade.rs",
            "lit frame",
            WORLD,
            Copy,
            From::None,
        ),
        Step {
            times: Times::BloomPlan,
            ..step(
                "bloom",
                "game_shaders/bloom.rs",
                "bloom buffers, lit frame",
                LEGACY,
                Screen,
                From::Game,
            )
        },
        Step {
            // The halvings, the doublings and the composite.
            times: Times::Fixed(BLOOM_SIZES + (BLOOM_SIZES - 1) + 1),
            ..step(
                "post",
                "viewport/post.rs",
                "bloom sizes, scratch",
                WORLD,
                Screen,
                From::Host,
            )
        },
        Step {
            times: Times::Fixed(2),
            ..step(
                "post",
                "viewport/post.rs",
                "local light, lit frame",
                WORLD,
                Copy,
                From::None,
            )
        },
        step(
            "viewport-resolve-pass",
            FRAME,
            "mapped frame or screen",
            BOTH,
            Screen,
            From::Game,
        ),
        step(
            "viewport-antialias-pass",
            FRAME,
            "screen",
            LEGACY,
            Screen,
            From::Game,
        ),
        step(
            "post",
            "viewport/post.rs",
            "screen",
            WORLD,
            Screen,
            From::Host,
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
