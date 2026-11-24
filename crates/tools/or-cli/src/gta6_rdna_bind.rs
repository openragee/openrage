//! `rpf scan-rdna-bind`: each effect's vertex and pixel passes through the native front end and its
//! binder, as counts: the passes whose two programs translate, whose every resource joins a binding
//! record and a name, and that the renderer can be handed, with what stopped the rest by name.
//! With `--technique`, the first pass of that technique in each effect instead: whether its two
//! programs translate, and what fills each binding they declare.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind};
use or_io::gta5::shaders::sgd5::{Sgd5File, Sgd5Program, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::{first_passes, technique_programs, vertex_pixel_passes};
use or_shader_front::dxil::ShaderKind;
use or_shader_front::rdna_bind::picks::{deferred_targets, material_textures, picks};
use or_shader_front::rdna_bind::shading::{sources, Source};
use or_shader_front::rdna_bind::{self, Join, Native, Unbound};

/// What one pair of a vertex and a pixel program came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Pair {
    translated: bool,
    /// Every resource of both modules is tied to a binding record and a name, or is a sampler.
    joined: bool,
    bound: bool,
    /// The stage and the census word of what stopped it first.
    stopped: Option<String>,
    /// The descriptors read from a place the program states: how many, how many with one binding
    /// record there, and how many of those the program's table names once.
    places: [usize; 3],
}

/// Counts `joins` into `places`; whether each is tied to a parameter or needs none.
fn tally(joins: &[Join<'_>], places: &mut [usize; 3]) -> bool {
    for join in joins {
        let reached = match join {
            Join::Named { .. } => 3,
            Join::NoName { .. } => 2,
            Join::NoRecord { .. } => 1,
            Join::Sampler | Join::UserData | Join::Words | Join::Picked => 0,
        };
        for count in &mut places[..reached] {
            *count += 1;
        }
    }
    joins.iter().all(Join::joins)
}

/// `vertex` and `pixel` translated as one pass, joined and bound; `tables` are their own variable
/// tables and `effect` the first table of the effect that runs them.
fn pair(vertex: &Native, pixel: &Native, tables: [&Sgd5Table; 2], effect: &Sgd5Table) -> Pair {
    let mut out = Pair::default();
    let stop = |stage: &str, why: &Unbound| Some(format!("{stage} {}", why.name()));
    let fragment = match rdna_bind::lower(pixel, &BTreeSet::new()) {
        Ok(lowered) => lowered,
        Err(why) => {
            out.stopped = stop("pixel", &why);
            return out;
        }
    };
    // The vertex stage exports as halves what the pixel stage reads as halves.
    let geometry = match rdna_bind::lower(vertex, &fragment.half_slots) {
        Ok(lowered) => lowered,
        Err(why) => {
            out.stopped = stop("vertex", &why);
            return out;
        }
    };
    out.translated = true;
    let vertex_joined = tally(
        &rdna_bind::joins(vertex, &geometry, tables[0]),
        &mut out.places,
    );
    let pixel_joined = tally(
        &rdna_bind::joins(pixel, &fragment, tables[1]),
        &mut out.places,
    );
    out.joined = vertex_joined && pixel_joined;
    let bound = rdna_bind::bind_lowered(vertex, geometry, tables[0], Some(effect))
        .map_err(|why| stop("vertex", &why))
        .and_then(|_| {
            rdna_bind::bind_lowered(pixel, fragment, tables[1], Some(effect))
                .map_err(|why| stop("pixel", &why))
        });
    match bound {
        Ok(_) => out.bound = true,
        Err(why) => out.stopped = why,
    }
    out
}

/// How the passes of an effect, or of a container, came out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Counts {
    passes: usize,
    translated: usize,
    joined: usize,
    bound: usize,
    /// Passes whose bank-0 program is of another stage, or whose programs do not read.
    other: usize,
}

impl Counts {
    fn add(&mut self, pair: &Pair) {
        self.passes += 1;
        self.translated += usize::from(pair.translated);
        self.joined += usize::from(pair.joined);
        self.bound += usize::from(pair.bound);
    }

    fn merge(&mut self, other: &Counts) {
        self.passes += other.passes;
        self.translated += other.translated;
        self.joined += other.joined;
        self.bound += other.bound;
        self.other += other.other;
    }
}

/// The program at `program`'s stream of `bytes`, ready to translate; `None` where it does not
/// read or is of another stage than vertex and pixel.
fn read(program: &Sgd5Program, bytes: &[u8]) -> Option<Native> {
    let container = program.inflate(bytes.get(program.stream.clone())?).ok()?;
    rdna_bind::native(&container).ok()
}

/// The reasons printed at most, the commonest first.
const TOP_REASONS: usize = 16;

/// The `--technique <name>` a census takes, and its other words.
fn split_technique(args: &[String]) -> Result<(Option<String>, Vec<String>), String> {
    match args.iter().position(|a| a == "--technique") {
        Some(i) => {
            let name = args.get(i + 1).ok_or("--technique needs a name")?.clone();
            let mut rest = args.to_vec();
            rest.drain(i..i + 2);
            Ok((Some(name), rest))
        }
        None => Ok((None, args.to_vec())),
    }
}

/// Whether a source is of a kind a census counts under one word.
type Kind = fn(&Source) -> bool;

/// How many of `filled` each kind of source fills, as words: `2 vertex data, 1 the game's`.
fn described(filled: &[Source]) -> String {
    let kinds: [(&str, Kind); 8] = [
        ("vertex data", |s| *s == Source::VertexData),
        ("meshlets", |s| *s == Source::Meshlets),
        ("view", |s| *s == Source::View),
        ("material", |s| *s == Source::Material),
        ("draw words", |s| *s == Source::DrawWords),
        ("samplers", |s| *s == Source::Sampler),
        ("the game's", |s| matches!(s, Source::Game(_))),
        ("unnamed", |s| *s == Source::Unnamed),
    ];
    let parts: Vec<String> = (kinds.iter())
        .map(|(word, is)| (filled.iter().filter(|s| is(s)).count(), word))
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect();
    match parts.is_empty() {
        true => "no binding".to_string(),
        false => parts.join(", "),
    }
}

/// How many shading passes have the two things a renderer needs of them established.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Settled {
    /// Every number either program picks a texture by names one texture of the effect.
    numbers: usize,
    /// Of those, the passes that also load a table entry at a place no such number gives.
    other: usize,
    /// The pixel program writes the targets whose contents are established.
    targets: usize,
    both: usize,
    /// Why the numbers of the rest are not settled, by stage and reason.
    unsettled: BTreeMap<String, usize>,
}

impl Settled {
    fn add(&mut self, geometry: &Native, fragment: &Native, effect: &Sgd5Table) {
        let picked = [geometry, fragment].map(|native| picks(&native.program.words));
        let mut numbers = true;
        for (native, picked) in [geometry, fragment].into_iter().zip(&picked) {
            let kind = native.program.kind;
            if let Err(why) = material_textures(picked, &native.records, kind, effect) {
                numbers = false;
                let stage = if kind == ShaderKind::Pixel {
                    "pixel"
                } else {
                    "first stage"
                };
                *self.unsettled.entry(format!("{stage}: {why}")).or_default() += 1;
            }
        }
        let targets = deferred_targets(&picked[1].exports);
        self.numbers += usize::from(numbers);
        self.other += usize::from(numbers && picked.iter().any(|p| p.other > 0));
        self.targets += usize::from(targets);
        self.both += usize::from(numbers && targets);
    }

    fn said(&self) -> String {
        format!(
            "{} whose every texture number names one texture of the effect in both stages ({} of them also pick by another number); {} whose pixel targets are the four whole and the motion; {} both",
            self.numbers, self.other, self.targets, self.both
        )
    }
}

/// What lowering a program came to, as words.
fn outcome(lowered: &Result<(), Unbound>) -> String {
    match lowered {
        Ok(()) => "translates".to_string(),
        Err(why) => format!("is refused ({})", why.name()),
    }
}

/// `rpf scan-rdna-bind --technique`: per effect that has the technique, the two programs of its
/// first pass: whether each translates and what fills its bindings, then the totals and the
/// bindings only the game fills by the hash of their names.
fn shading(container: &Sgd5File, bytes: &[u8], technique: &str) {
    let mut natives: HashMap<usize, Option<Native>> = HashMap::new();
    let mut game: BTreeMap<(&str, u32), usize> = BTreeMap::new();
    let (mut effects, mut translated, mut whole) = (0usize, 0usize, 0usize);
    let mut settled = Settled::default();
    // The passes whose first stage, and whose pixel stage, holds a loop not shown to end.
    let mut unending = [0usize; 2];
    for effect in &container.effects {
        let Some((first, pixel)) = technique_programs(container, effect, technique) else {
            continue;
        };
        effects += 1;
        for program in [first, pixel] {
            natives
                .entry(program.at)
                .or_insert_with(|| read(program, bytes));
        }
        let (Some(Some(geometry)), Some(Some(fragment))) =
            (natives.get(&first.at), natives.get(&pixel.at))
        else {
            println!("  {}: a program of the pass does not read", effect.name);
            continue;
        };
        let mesh = geometry.program.kind == ShaderKind::Mesh;
        let stage = if mesh { "mesh" } else { "vertex" };
        let lowered = rdna_bind::lower(fragment, &BTreeSet::new());
        // The first stage exports as halves what the pixel stage reads as halves.
        let halves = match &lowered {
            Ok(lowered) => lowered.half_slots.clone(),
            Err(_) => BTreeSet::new(),
        };
        let lowered = [rdna_bind::lower(geometry, &halves), lowered];
        // The loops of each stage a device may never leave, as far as its lowered code shows.
        let loops = lowered.each_ref().map(|lowered| match lowered {
            Ok(lowered) => lowered.loops_not_shown_to_end(),
            Err(_) => 0,
        });
        for (held, loops) in unending.iter_mut().zip(loops) {
            *held += usize::from(loops > 0);
        }
        let outcomes = lowered.map(|lowered| lowered.map(|_| ()));
        let filled = [
            sources(
                &geometry.records,
                mesh,
                &first.variables,
                &effect.parameters,
            ),
            sources(
                &fragment.records,
                false,
                &pixel.variables,
                &effect.parameters,
            ),
        ];
        println!(
            "  {}: {stage} {}, {}; pixel {}, {}; loops not shown to end {}, {}",
            effect.name,
            outcome(&outcomes[0]),
            described(&filled[0]),
            outcome(&outcomes[1]),
            described(&filled[1]),
            loops[0],
            loops[1],
        );
        settled.add(geometry, fragment, &effect.parameters);
        translated += usize::from(outcomes.iter().all(Result::is_ok));
        whole += usize::from(filled.iter().flatten().all(|s| s.held()));
        for (word, filled) in [stage, "pixel"].into_iter().zip(&filled) {
            for source in filled {
                if let Source::Game(hash) = source {
                    *game.entry((word, *hash)).or_default() += 1;
                }
            }
        }
    }
    println!("  effects       {effects} with the technique {technique}");
    println!(
        "  passes        {translated} translate in both stages; {whole} whose every binding a model's file, the view or the material fills"
    );
    println!(
        "  loops         {} passes hold a loop not shown to end in the first stage, {} in the pixel stage",
        unending[0], unending[1]
    );
    println!("  settled       {}", settled.said());
    for (why, count) in settled.unsettled.iter().take(TOP_REASONS) {
        println!("    not settled, {why}: {count} passes");
    }
    let mut names: Vec<_> = game.iter().collect();
    names.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!(
        "  the game      fills {} distinct binding(s), by stage and the hash of the name:",
        names.len()
    );
    for ((word, hash), count) in names.iter().take(TOP_REASONS) {
        println!("    {word} 0x{hash:08X}: {count} passes");
    }
    if names.len() > TOP_REASONS {
        println!("    and {} other(s)", names.len() - TOP_REASONS);
    }
}

/// `rpf scan-rdna-bind`: per effect of the `SGD5` container at an offset, its vertex and pixel
/// passes, how many translate in both stages, join every resource and bind, printed as counts.
pub(crate) fn cmd_scan_rdna_bind(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let (technique, args) = split_technique(&args)?;
    let [path, offset] = args.as_slice() else {
        return Err(
            "usage: rpf scan-rdna-bind <archive.rpf> <offset> [--technique <name>] [--limit <bytes>]"
                .to_string(),
        );
    };
    let offset = super::rpf::parse_offset(offset)?;
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
        return Err(format!(
            "the body at {offset:#X} is not an SGD5 effect container"
        ));
    }
    let container = Sgd5File::open(&bytes)
        .map_err(|e| format!("the SGD5 container at {offset:#X} does not read: {e}"))?;
    if let Some(technique) = technique {
        shading(&container, &bytes, &technique);
        return Ok(());
    }

    let mut natives: HashMap<usize, Option<Native>> = HashMap::new();
    let mut pairs: HashMap<(usize, usize), Pair> = HashMap::new();
    let (mut total, mut effects) = (Counts::default(), 0usize);
    let mut stopped: BTreeMap<String, usize> = BTreeMap::new();
    for effect in &container.effects {
        let mut mine = Counts::default();
        for (_, vertex, pixel) in vertex_pixel_passes(&container, effect) {
            for program in [vertex, pixel] {
                natives
                    .entry(program.at)
                    .or_insert_with(|| read(program, &bytes));
            }
            let (Some(Some(v)), Some(Some(p))) = (natives.get(&vertex.at), natives.get(&pixel.at))
            else {
                mine.other += 1;
                continue;
            };
            let tables = [&vertex.variables, &pixel.variables];
            let came = pairs
                .entry((vertex.at, pixel.at))
                .or_insert_with(|| pair(v, p, tables, &effect.parameters));
            mine.add(came);
            if let Some(why) = &came.stopped {
                *stopped.entry(why.clone()).or_default() += 1;
            }
        }
        if mine.passes > 0 {
            effects += 1;
            println!(
                "  {}: {} vertex and pixel passes, {} translate, {} join every resource, {} bound",
                effect.name, mine.passes, mine.translated, mine.joined, mine.bound
            );
        }
        total.merge(&mine);
    }
    let places = pairs.values().fold([0usize; 3], |sum, pair| {
        [
            sum[0] + pair.places[0],
            sum[1] + pair.places[1],
            sum[2] + pair.places[2],
        ]
    });
    // The pass that shades a model, by shape: the first technique whose pass is a mesh program
    // and a pixel program that writes the targets whose contents are established.
    let (mut settled, mut shaded) = (Settled::default(), 0usize);
    for effect in &container.effects {
        for (first, pixel) in first_passes(&container, effect) {
            for program in [first, pixel] {
                natives
                    .entry(program.at)
                    .or_insert_with(|| read(program, &bytes));
            }
            let (Some(Some(geometry)), Some(Some(fragment))) =
                (natives.get(&first.at), natives.get(&pixel.at))
            else {
                continue;
            };
            if geometry.program.kind == ShaderKind::Mesh
                && deferred_targets(&picks(&fragment.program.words).exports)
            {
                shaded += 1;
                settled.add(geometry, fragment, &effect.parameters);
                break;
            }
        }
    }
    println!("  effects       {effects} with a vertex and pixel pass");
    println!(
        "  shading       {shaded} of {} effects have a mesh pass that writes the settled targets: {}",
        container.effects.len(),
        settled.said()
    );
    println!(
        "  passes        {}: {} translate in both stages, {} join every resource, {} bound; {} more whose bank-0 program is of another stage or does not read",
        total.passes, total.translated, total.joined, total.bound, total.other
    );
    println!(
        "  pairs         {} distinct: {} descriptors read from a stated place, {} with one binding record there, {} named once by the program's table",
        pairs.len(),
        places[0],
        places[1],
        places[2]
    );
    let mut reasons: Vec<_> = stopped.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (reason, count) in reasons.iter().take(TOP_REASONS) {
        println!("    stopped at {reason}: {count} passes");
    }
    if reasons.len() > TOP_REASONS {
        println!("    and {} other reason(s)", reasons.len() - TOP_REASONS);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta5::shaders::sgd5::Sgd5Resource;
    use or_shader_front::rdna_bind::{Class, Record};

    #[test]
    fn a_join_counts_as_far_as_it_reached_and_only_a_name_or_a_sampler_joins() {
        let record = Record {
            class: Class::Srv,
            register: 9,
            root: 8,
            offset: 32,
            dwords: 8,
            stride: 4,
        };
        let named = Sgd5Resource {
            code: 0x02,
            flags: 0,
            slot: 9,
            members: 0,
            buffer_size: 0,
            name_hash: 0xA000_0001,
            semantic_hash: 0xA000_0001,
        };
        let named = Join::Named {
            record,
            named: &named,
        };
        let mut places = [0; 3];
        assert!(tally(&[named, Join::Sampler], &mut places));
        assert_eq!(places, [1, 1, 1]);
        let no_record = Join::NoRecord {
            root: 8,
            offset: 64,
        };
        assert!(!tally(
            &[named, Join::NoName { record }, no_record],
            &mut places
        ));
        assert_eq!(places, [4, 3, 2]);
        // What the game picks or passes has no place to count, and joins nothing.
        assert!(!tally(&[Join::Picked], &mut places));
        assert!(!tally(&[Join::UserData], &mut places));
        assert_eq!(places, [4, 3, 2]);
    }

    #[test]
    fn the_technique_flag_is_taken_out_of_the_words_and_sources_are_counted_by_kind() {
        let words =
            |args: &[&str]| -> Vec<String> { args.iter().map(|a| (*a).to_owned()).collect() };
        let split = split_technique(&words(&["a.rpf", "--technique", "shade", "0x10"]));
        assert_eq!(
            split,
            Ok((Some("shade".to_owned()), words(&["a.rpf", "0x10"])))
        );
        assert!(split_technique(&words(&["--technique"])).is_err());
        assert_eq!(split_technique(&words(&["a"])), Ok((None, words(&["a"]))));

        let filled = [
            Source::Game(7),
            Source::VertexData,
            Source::Game(9),
            Source::View,
            Source::Sampler,
        ];
        assert_eq!(
            described(&filled),
            "1 vertex data, 1 view, 1 samplers, 2 the game's"
        );
        assert_eq!(described(&[]), "no binding");
        assert_eq!(outcome(&Ok(())), "translates");
        assert_eq!(outcome(&Err(Unbound::UserData)), "is refused (user-data)");
    }

    #[test]
    fn a_pass_is_counted_once_at_each_step_it_reached() {
        let mut counts = Counts::default();
        counts.add(&Pair {
            translated: true,
            joined: true,
            ..Pair::default()
        });
        counts.add(&Pair::default());
        let mut total = Counts {
            other: 2,
            ..Counts::default()
        };
        total.merge(&counts);
        assert_eq!(
            total,
            Counts {
                passes: 2,
                translated: 1,
                joined: 1,
                bound: 0,
                other: 2,
            }
        );
    }
}
