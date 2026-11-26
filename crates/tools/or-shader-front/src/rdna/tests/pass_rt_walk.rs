//! A walk of the user's own build, lowered alone and run here against the reference walk, ray
//! for ray: whether it hit, the distance's bits, the leaf, the instance, the turns it took and
//! the stack word it left.
//!
//! Nothing of a build is in the tree. `OPENRAGE_RDNA_WALKS` names the files, one a walk and
//! `;` between them, that say which program of which kept corpus it is, where its walk starts
//! and ends and which registers hold what; without it the test checks nothing and says so. What
//! each walk came to is written beside its file, with `.out` after its name.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use or_io::gta6::accel::layout::{
    offset, FLAG_FLIP_WINDING, FLAG_FORCE_NON_OPAQUE, FLAG_FORCE_OPAQUE, FLAG_NO_CULL, HEADER_ROOT,
    INSTANCE_BOTTOM, INSTANCE_CONTRIBUTION_FLAGS, TRIANGLE_WORD,
};
use or_io::gta6::accel::{
    walk_lane, word, Arena, Cull, Hit, Instance, Opacity, Outcome, Ray, Structure, Triangle, Walk,
};

use super::interp;
use super::*;
use crate::rdna_corpus::Corpus;

/// What one file says of a walk.
pub(super) struct Spec {
    pub(super) words: BTreeMap<String, String>,
}

impl Spec {
    pub(super) fn read(path: &Path) -> Self {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|why| panic!("{}: {why}", path.display()));
        let lines = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'));
        let words = lines
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
            .collect();
        Self { words }
    }

    pub(super) fn text(&self, key: &str) -> &str {
        (self.words.get(key)).unwrap_or_else(|| panic!("the walk's file states no {key}"))
    }

    pub(super) fn number(&self, key: &str) -> u32 {
        number(self.text(key)).unwrap_or_else(|| panic!("{key} is no number"))
    }

    /// The pairs `name:value` of a key, an empty list where the file has none.
    /// Whether the walk is the form a wave takes together, which has a check of its own.
    pub(super) fn of_a_wave(&self) -> bool {
        self.words.get("form").is_some_and(|form| form == "wave")
    }

    /// The program the file names, from the corpus it names.
    pub(super) fn program(&self) -> Program {
        let mut corpus = Corpus::open(Path::new(self.text("corpus"))).expect("the corpus");
        let bytes = (corpus.program(self.number("program") as usize))
            .ok()
            .flatten()
            .expect("the program");
        let block = self.words.get("block").map_or(0, |_| self.number("block"));
        unwrap(&bytes)
            .expect("the program's blocks")
            .swap_remove(block as usize)
    }

    pub(super) fn pairs(&self, key: &str) -> Vec<(&str, &str)> {
        let all = self.words.get(key).map_or("", String::as_str);
        (all.split_whitespace())
            .map(|pair| pair.split_once(':').expect("a pair with no colon"))
            .collect()
    }
}

pub(super) fn number(text: &str) -> Option<u32> {
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// A fixed sequence of floats in `[0, 1)`.
struct Sequence(u32);

impl Sequence {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1 << 24) as f32
    }

    fn within(&mut self, size: f32) -> f32 {
        (self.next() - 0.5) * size
    }

    fn point(&mut self, size: f32) -> [f32; 3] {
        std::array::from_fn(|_| self.within(size))
    }
}

pub(super) struct Scene {
    pub(super) name: &'static str,
    pub(super) arena: Arena,
    pub(super) top: Structure,
    pub(super) rays: Vec<Ray>,
}

/// What a scene is built with other than as stated: the same nodes at the same places, with
/// one thing a walk reads of them changed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Changed {
    Nothing,
    /// Every triangle's opaque bit the other way.
    OpaqueBits,
    /// No instance forces its triangles opaque or not opaque.
    ForceFlags,
}

fn triangle(corners: [[f32; 3]; 3], primitive: usize, opaque: bool) -> Triangle {
    Triangle {
        corners,
        geometry: 0,
        primitive: primitive as u32,
        opaque,
    }
}

/// `n` by `n` quads across eight units, lifted by a bump of the grid's own.
fn grid(n: usize, opaque: bool) -> Vec<Triangle> {
    let at = |i: usize, j: usize| {
        let line = |i: usize| i as f32 / n as f32 * 8.0 - 4.0;
        [line(i), line(j), 0.09 * ((i * 7 + j * 3) % 6) as f32]
    };
    let mut triangles = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let quad = [at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)];
            for corners in [[quad[0], quad[1], quad[2]], [quad[1], quad[3], quad[2]]] {
                triangles.push(triangle(corners, triangles.len(), opaque));
            }
        }
    }
    triangles
}

/// `n` loose triangles through a cube of six units, each third of them opaque.
fn soup(sequence: &mut Sequence, n: usize) -> Vec<Triangle> {
    (0..n)
        .map(|k| {
            let centre = sequence.point(6.0);
            let corners = std::array::from_fn(|_| {
                let by = sequence.point(0.9);
                std::array::from_fn(|axis| centre[axis] + by[axis])
            });
            // No two of them share an edge, so a geometry's number pairs none.
            Triangle {
                geometry: k as u32 % 5,
                ..triangle(corners, k, k % 3 == 0)
            }
        })
        .collect()
}

/// `count` triangles stacked along z a quarter apart, each across the z axis.
fn stack(count: usize) -> Vec<Triangle> {
    (0..count)
        .map(|k| {
            let z = k as f32 * 0.25;
            // Wound to face a ray that comes up the axis.
            triangle([[-1.0, -1.0, z], [-1.0, 2.0, z], [2.0, -1.0, z]], k, true)
        })
        .collect()
}

/// Turns whose rows hold one 1 or -1: a product by one is exact however it is summed.
const QUARTERS: [[[f32; 3]; 3]; 4] = [
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]],
    [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
];

fn turn(sequence: &mut Sequence) -> [[f32; 3]; 3] {
    let (a, b) = (sequence.next() * 6.0, sequence.next() * 6.0);
    let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
    [
        [ca, -sa * cb, sa * sb],
        [sa, ca * cb, -ca * sb],
        [0.0, sb, cb],
    ]
}

/// Instances of a grid, a soup and a lone triangle about the origin: some under a mask no
/// walk here asks for, and each of the four flags on some.
pub(super) fn placed(name: &'static str, seed: u32, exact: bool, changed: Changed) -> Scene {
    let mut sequence = Sequence(seed);
    let mut arena = Arena::new();
    let mut models = [grid(10, true), soup(&mut sequence, 400), grid(1, false)];
    if changed == Changed::OpaqueBits {
        for triangle in models.iter_mut().flatten() {
            triangle.opaque = !triangle.opaque;
        }
    }
    let bottoms = models
        .each_ref()
        .map(|model| arena.bottom_level(model).expect("a bottom level"));
    let forces = (FLAG_FORCE_OPAQUE | FLAG_FORCE_NON_OPAQUE) as u8;
    let instances: Vec<Instance> = (0..18)
        .map(|i| {
            let turned = match exact {
                true => QUARTERS[i % 4],
                false => turn(&mut sequence),
            };
            // Whole numbers of sixteenths: a turn of them is exact too.
            let at = sequence.point(24.0).map(|x| (x * 16.0).round() / 16.0);
            Instance {
                bottom: bottoms[i % 3],
                object_to_world: std::array::from_fn(|row| {
                    [turned[row][0], turned[row][1], turned[row][2], at[row]]
                }),
                id: i as u32,
                // A bit no walk of the build asks for.
                mask: if i % 5 == 4 { 0x02 } else { 0xFF },
                flags: [
                    0,
                    FLAG_NO_CULL,
                    FLAG_FLIP_WINDING,
                    FLAG_FORCE_OPAQUE,
                    FLAG_FORCE_NON_OPAQUE,
                    0,
                    0,
                ][i % 7] as u8
                    & match changed {
                        Changed::ForceFlags => !forces,
                        _ => 0xFF,
                    },
                contribution: 3 * i as u32,
            }
        })
        .collect();
    let top = arena.top_level(&instances).expect("a top level");
    let rays = (0..384)
        .map(|k| {
            let origin = sequence.point(60.0);
            let to = sequence.point(26.0);
            Ray {
                origin,
                direction: std::array::from_fn(|axis| to[axis] - origin[axis]),
                t_min: 0.0,
                // One ray in four stops short of where it is aimed.
                t_max: if k % 4 == 3 { 0.6 } else { 1.5 },
            }
        })
        .collect();
    Scene {
        name,
        arena,
        top,
        rays,
    }
}

/// One instance of a stack of triangles a ray along z meets the boxes of at every level: more
/// children left waiting than a stack of sixteen holds. Half the rays pass beside every
/// triangle and walk the whole of it.
pub(super) fn deep() -> Scene {
    let mut sequence = Sequence(5);
    let mut arena = Arena::new();
    let bottom = arena.bottom_level(&stack(1 << 12)).expect("a bottom level");
    let instance = Instance {
        bottom,
        object_to_world: std::array::from_fn(|row| {
            [
                QUARTERS[0][row][0],
                QUARTERS[0][row][1],
                QUARTERS[0][row][2],
                0.0,
            ]
        }),
        id: 0,
        mask: 0xFF,
        flags: 0,
        contribution: 0,
    };
    let top = arena.top_level(&[instance]).expect("a top level");
    let rays = (0..64)
        .map(|k| {
            let beside = k % 2 == 1;
            let at = if beside { 1.3 } else { -0.4 };
            Ray {
                origin: [at + sequence.within(0.3), at + sequence.within(0.3), -1.0],
                direction: [sequence.within(0.01), sequence.within(0.01), 2000.0],
                t_min: 0.0,
                t_max: 1.0,
            }
        })
        .collect();
    Scene {
        name: "a stack deeper than sixteen",
        arena,
        top,
        rays,
    }
}

/// What a walk's rays came to, over one scene.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Tally {
    rays: u32,
    hits: u32,
    /// Rays whose every field is the reference's.
    equal: u32,
    /// Rays that hit or miss as the reference, on the same leaf of the same instance.
    same_triangle: u32,
    climbs: u32,
    deepest: u32,
    turns: u64,
    /// Rays whose stack word the reference has no form of, and was not held to it.
    unstacked: u32,
    /// The first rays that differ: what the walk left, then what the reference came to.
    differing: Vec<[[u32; 5]; 2]>,
}

/// The walk the file states, lowered alone from the program's words with `ring` entries a
/// lane and the words `patches` change.
struct Lowering {
    /// The bytes from one entry of a lane's stack to the next: four a lane of the wave.
    step: u32,
    module: naga::Module,
    registers: Registers,
    threads: [u32; 3],
    shared: u32,
    walk: Walk,
    commits: Commits,
}

/// What a walk's own code decides of a triangle that is not opaque.
enum Commits {
    /// It commits every one.
    All,
    /// It commits one whose record of a table holds none of the bits of `reject` in its first
    /// dword: the record numbered the instance's contribution plus the geometry's number, in
    /// the buffer the module names `buffer`.
    Table { buffer: String, reject: u32 },
}

/// The records of a walk's table of geometries: more than a scene numbers.
const RECORDS: u32 = 256;
/// The dwords before a bound buffer's first record, the first of them a record's bytes.
const TABLE_HEADER: [u32; 4] = [8, 0, 0, 0];

/// The first dword of record `index`: nought, a bit a walk rejects, bits it does not ask
/// about, and another it rejects, in turn.
fn record(index: u32, reject: u32) -> u32 {
    let lowest = reject & reject.wrapping_neg();
    let highest = 1 << (31 - reject.leading_zeros().min(31));
    [0, lowest, !reject & 0x00FF_00FF, highest][index as usize % 4]
}

/// A table's dwords as a module's buffer of them is bound.
fn table(reject: u32) -> Vec<u32> {
    let mut words = TABLE_HEADER.to_vec();
    for index in 0..RECORDS {
        words.extend([record(index, reject), 0]);
    }
    words
}

/// Whether the walk's own code commits `hit`, on a triangle of the arena `words` whose top
/// level is at `top`.
fn commits(commits: &Commits, words: &[u32], top: u32, hit: &Hit) -> bool {
    match commits {
        Commits::All => true,
        Commits::Table { reject, .. } => {
            let instance = top + offset(hit.instance);
            let bottom = word(words, instance, INSTANCE_BOTTOM) << 8;
            let contribution = word(words, instance, INSTANCE_CONTRIBUTION_FLAGS) & 0xFF_FFFF;
            let geometry = word(words, bottom + offset(hit.leaf), TRIANGLE_WORD) >> 8;
            record(contribution + geometry, *reject) & reject == 0
        }
    }
}

fn lower(spec: &Spec, ring: u32, patched: bool) -> Lowering {
    let mut program = spec.program();
    let (first, exit) = (spec.number("first"), spec.number("exit"));
    // The stack's address mask is a literal of the walk: another ring is another literal.
    let (mask, step) = (spec.number("ring"), spec.number("step"));
    let entries = (mask + 4) / step;
    let wanted = (ring * step - 1) & !3;
    let mut changed = 0;
    for at in first..exit {
        if program.words[at as usize] == mask && ring != entries {
            program.words[at as usize] = wanted;
            changed += 1;
        }
    }
    assert!(
        ring == entries || changed > 0,
        "the walk holds no literal {mask:#x}"
    );
    if patched {
        for (at, change) in spec.pairs("control") {
            let (and, or) = change.split_once('/').expect("a mask and a value");
            let word = &mut program.words[number(at).unwrap() as usize];
            *word = (*word & number(and).unwrap()) | number(or).unwrap();
        }
    }
    let left = ["leaf", "t", "instance", "count", "stack"].map(|what| spec.text(what).to_owned());
    let options = Options {
        arena: true,
        region: Some(Region {
            first,
            exit,
            left: left.to_vec(),
        }),
        ..Options::default()
    };
    let lowered = translate_with(&program, &options).expect("the walk lowers");
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&lowered.module)
        .expect("a module the validator takes");
    assert!(!lowered.subgroups, "the walk is not a lane an invocation");
    let launch = program.launch.expect("a compute program");
    let cull = match spec.text("cull") {
        "none" => Cull::None,
        "back" => Cull::Back,
        "front" => Cull::Front,
        other => panic!("{other} is no culling"),
    };
    let opacity = match spec.text("opacity") {
        "instance" => Opacity::Instance,
        "opaque" => Opacity::Opaque,
        "nonopaque" => Opacity::NonOpaque,
        other => panic!("{other} is no opacity"),
    };
    Lowering {
        step,
        module: lowered.module,
        registers: lowered.region.expect("a walk's registers"),
        threads: launch.threads.map(|n| n.max(1)),
        shared: launch.shared_dwords,
        walk: Walk {
            ring,
            lane: 0,
            mask: spec.number("mask") as u8,
            cull,
            opacity,
            first_hit: spec.text("first_hit") == "true",
            bound: spec.number("bound"),
        },
        commits: match spec.text("commits") {
            "all" => Commits::All,
            "table" => Commits::Table {
                buffer: spec.text("table").to_owned(),
                reject: spec.number("reject"),
            },
            other => panic!("{other} is no rule of what a walk commits"),
        },
    }
}

/// How a run of a walk is held to the reference: as it stands, or with one thing wrong on one
/// side, which has to show.
struct Held<'a> {
    /// Every lane is given the first lane's stack address.
    shared_lane: bool,
    /// The scene the reference walks, where it is not the module's: one built [`Changed`].
    reference: Option<&'a Scene>,
    /// What the reference takes a triangle's opacity from, where it is not the walk's own.
    opacity: Option<Opacity>,
    /// The reference commits every triangle that is not opaque, whatever the walk's code says.
    commit_all: bool,
}

const AS_IT_STANDS: Held = Held {
    shared_lane: false,
    reference: None,
    opacity: None,
    commit_all: false,
};

/// Runs the lowered walk over `scene`, a ray a lane and a group at a time, and holds each ray
/// to the reference.
fn compare(spec: &Spec, lowering: &Lowering, scene: &Scene, held: &Held) -> Tally {
    let words = scene.arena.words();
    let shared_lane = held.shared_lane;
    let reference = held.reference.map_or(words, |other| other.arena.words());
    let records = match &lowering.commits {
        Commits::All => Vec::new(),
        Commits::Table { reject, .. } => table(*reject),
    };
    let lanes = lowering.threads.iter().product::<u32>() as usize;
    let given = &lowering.registers.given;
    let slots = given.len();
    let stated: BTreeMap<&str, &str> = spec.pairs("given").into_iter().collect();
    let mut tally = Tally::default();
    for rays in scene.rays.chunks(lanes) {
        let mut rows = vec![0u32; lanes * slots];
        for (lane, ray) in rays.iter().enumerate() {
            let stack = if shared_lane { 0 } else { lane as u32 * 4 };
            for (slot, register) in given.iter().enumerate() {
                let what = stated.get(register.as_str()).copied().unwrap_or("zero");
                let float = |part: f32| part.to_bits();
                rows[lane * slots + slot] = match what {
                    "zero" => 0,
                    "one" => 1,
                    "ones" => u32::MAX,
                    "root" => word(words, scene.top.base, HEADER_ROOT),
                    "top" => scene.top.base >> 8,
                    "top_bytes" => scene.top.base,
                    "lane4" => stack,
                    "far" => float(ray.t_max),
                    "ox" => float(ray.origin[0]),
                    "oy" => float(ray.origin[1]),
                    "oz" => float(ray.origin[2]),
                    "dx" => float(ray.direction[0]),
                    "dy" => float(ray.direction[1]),
                    "dz" => float(ray.direction[2]),
                    other => number(other).unwrap_or_else(|| panic!("{other} is no value")),
                };
            }
        }
        // Where its stack starts each lane left no entry before its walk.
        let mut shared = vec![0u32; lowering.shared as usize];
        shared[..lanes].fill(u32::MAX);
        let left = vec![0u32; lanes * 5];
        let mut buffers = vec![
            ("arena", words),
            ("registers_given", &rows[..]),
            ("registers_left", &left[..]),
            ("shared", &shared[..]),
        ];
        if let Commits::Table { buffer, .. } = &lowering.commits {
            buffers.push((buffer.as_str(), &records[..]));
        }
        let (_, after) =
            interp::run_bound(&lowering.module, lowering.threads, &|_, _| None, &buffers);
        for (lane, ray) in rays.iter().enumerate() {
            let walk = Walk {
                lane: lane as u32,
                opacity: held.opacity.unwrap_or(lowering.walk.opacity),
                ..lowering.walk
            };
            let mut candidate = |hit: &Hit| {
                held.commit_all || commits(&lowering.commits, reference, scene.top.base, hit)
            };
            let want = walk_lane(reference, scene.top.base, ray, &walk, &mut candidate);
            let wanted = expected(&want, ray, &walk);
            let mut got: [u32; 5] = std::array::from_fn(|k| after[2][lane * 5 + k]);
            // A walk that hit nothing leaves the last instance it was in, which is no result.
            if got[0] == 0 {
                got[2] = 0;
            }
            // The reference keeps the stack word of a wave of 32 lanes.
            if lowering.step != 0x80 {
                match lane < 32 {
                    true => got[4] = halved(got[4]),
                    false => {
                        got[4] = wanted[4];
                        tally.unstacked += 1;
                    }
                }
            }
            tally.rays += 1;
            tally.hits += u32::from(want.hit.is_some());
            tally.equal += u32::from(got == wanted);
            if got != wanted && tally.differing.len() < 6 {
                tally.differing.push([got, wanted]);
            }
            let same = got[0] == wanted[0] && (got[0] == 0 || got[2] == wanted[2]);
            tally.same_triangle += u32::from(same);
            tally.climbs += want.climbs;
            tally.deepest = tally.deepest.max(want.deepest);
            tally.turns += u64::from(want.turns);
        }
    }
    tally
}

/// The stack word of a wave of 64 lanes, whose entries are 256 bytes apart, as a wave of 32
/// keeps it with entries 128 bytes apart: the same lane, and above it the count of entries
/// halved, from nought or from the mark a climb leaves; the depth an instance was entered at
/// likewise.
fn halved(word: u32) -> u32 {
    let half = |field: u32, mark: u32| match field >= mark / 2 {
        true => (mark as i32 + (field as i32 - mark as i32) / 2) as u32,
        false => field / 2,
    };
    let (lane, count, entered) = (word & 0x7C, (word >> 7) & 0x1FFF, word >> 20);
    lane | (half(count, 0x800) << 7 & 0xF_FF80) | half(entered, 0x800) << 20
}

/// The leaf, the distance's bits, the instance, the count of turns and the stack word a walk
/// leaves where the reference came to `want`.
fn expected(want: &Outcome, ray: &Ray, walk: &Walk) -> [u32; 5] {
    // A walk counts a turn where it finds nothing left to test, and none where a hit ends it.
    let ended_on_a_hit = walk.first_hit && want.hit.is_some();
    let count = want.turns + u32::from(!ended_on_a_hit);
    match want.hit {
        Some(hit) => [hit.leaf, hit.t.to_bits(), hit.instance, count, want.stack],
        None => [0, ray.t_max.to_bits(), 0, count, want.stack],
    }
}

#[test]
fn a_walk_of_the_build_lowered_alone_equals_the_reference_walk_ray_for_ray() {
    let Some(files) = std::env::var_os("OPENRAGE_RDNA_WALKS") else {
        or_test_support::skip_install(
            "a walk of the build against the reference walk",
            "OPENRAGE_RDNA_WALKS names no file of a walk",
        );
        return;
    };
    for file in std::env::split_paths(&files) {
        let spec = Spec::read(&file);
        if spec.of_a_wave() {
            continue;
        }
        let mut report = String::new();
        let entries = (spec.number("ring") + 4) / spec.number("step");
        for ring in [entries, 4] {
            let lowering = lower(&spec, ring, false);
            let scenes = [
                placed(
                    "instances a quarter turn apart",
                    12345,
                    true,
                    Changed::Nothing,
                ),
                placed(
                    "instances turned by any angle",
                    777,
                    false,
                    Changed::Nothing,
                ),
                deep(),
            ];
            for scene in &scenes {
                let tally = compare(&spec, &lowering, scene, &AS_IT_STANDS);
                let _ = writeln!(report, "ring {ring}, {}: {tally:x?}", scene.name);
                assert!(tally.hits > 0 && tally.hits < tally.rays, "{tally:?}");
                assert_eq!(
                    tally.equal, tally.rays,
                    "ring {ring}, {}: {tally:x?}",
                    scene.name
                );
            }
            // The deep scene overruns the ring: its pops climb by the parent table.
            let climbed = compare(&spec, &lowering, &scenes[2], &AS_IT_STANDS);
            assert!(climbed.climbs > 0, "{climbed:?}");
            // Controls that must differ: every lane on the first lane's stack, and the walk
            // with the instructions the file names changed.
            let on_one = Held {
                shared_lane: true,
                ..AS_IT_STANDS
            };
            let crossed = compare(&spec, &lowering, &scenes[0], &on_one);
            let _ = writeln!(report, "ring {ring}, every lane on one stack: {crossed:?}");
            assert!(crossed.equal < crossed.rays, "{crossed:?}");
            if spec.words.contains_key("control") {
                let other = lower(&spec, ring, true);
                let broken = compare(&spec, &other, &scenes[0], &AS_IT_STANDS);
                let _ = writeln!(report, "ring {ring}, the control: {broken:?}");
                assert!(broken.equal < broken.rays, "{broken:?}");
            }
            // A walk whose own code decides a triangle that is not opaque: the reference with
            // a triangle's opaque bit the other way, with no instance forcing either, with
            // every triangle taken as opaque, and with every candidate committed, each alone,
            // has to come to other hits than the walk does.
            if matches!(lowering.commits, Commits::Table { .. }) {
                let turned = placed("", 12345, true, Changed::OpaqueBits);
                let unforced = placed("", 12345, true, Changed::ForceFlags);
                let wrong = [
                    (
                        "the opaque bits the other way",
                        Held {
                            reference: Some(&turned),
                            ..AS_IT_STANDS
                        },
                    ),
                    (
                        "no instance forcing opacity",
                        Held {
                            reference: Some(&unforced),
                            ..AS_IT_STANDS
                        },
                    ),
                    (
                        "every triangle opaque",
                        Held {
                            opacity: Some(Opacity::Opaque),
                            ..AS_IT_STANDS
                        },
                    ),
                    (
                        "every candidate committed",
                        Held {
                            commit_all: true,
                            ..AS_IT_STANDS
                        },
                    ),
                ];
                for (what, held) in &wrong {
                    let differs = compare(&spec, &lowering, &scenes[0], held);
                    let _ = writeln!(report, "ring {ring}, {what}: {differs:?}");
                    assert!(differs.equal < differs.rays, "{what}: {differs:?}");
                }
            }
        }
        let mut out = file.clone().into_os_string();
        out.push(".out");
        std::fs::write(&out, report).expect("the walk's figures written");
    }
}
