//! The rules a GTA VI texture's name follows from a name its drawable states. A texture key is the
//! 64-bit hash of the lower-cased name, so a rule's candidate names a key only where it hashes to
//! one; nothing here is a list of names.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::hash::{name_hash32, name_hash64, NameHasher};
use super::names::TextureUse;

/// The map kinds a texture's name ends in, after `_`: albedo, albedo with alpha, normal, the
/// three data maps, emissive, tint, palette, dirt, eyes, mask and flow; then four measured to end
/// names the others leave, whose maps were not looked at.
const MAP_KINDS: [&str; 17] = [
    "ab", "abal", "nm", "ma", "mb", "mc", "em", "tn", "pal", "dirt", "eyes", "mask", "flow",
    "tint", "id", "ani", "ctrl",
];
/// The level-of-detail tokens a stem can end in, the first none.
const LODS: [&str; 9] = [
    "",
    "_lod",
    "_slod",
    "_slod1",
    "_slod2",
    "_slod3",
    "_slod4",
    "_batchlod",
    "_batchslod",
];
/// A ped component texture's map kinds.
const PED_MAPS: [&str; 7] = ["ab", "tn", "ma", "nm", "mb", "em", "mc"];
/// A ped component's variants `c0` to `c3` and texture sets `000` to `039`.
const PED_VARIANTS: u32 = 4;
const PED_SETS: u32 = 40;
/// A numbered variant runs from 0 to 99; the first 30 also take a letter `a` to `h`.
const NUMBERS: u32 = 100;
const LETTERED: u32 = 30;
/// The longest stem the rules make names from: a name the file states is bounded only by its
/// section, and every candidate copies the stem.
const MAX_STEM: usize = 128;
/// The underscores the token rules split a stem at, the first ones: each split makes over a
/// thousand names.
const MAX_SPLITS: usize = 8;

/// The shader parameters that bind a texture, by the names their hashes are of.
const TEXTURE_PARAMETERS: [&str; 7] = [
    "DiffuseTex",
    "BumpTex",
    "SpecularTex",
    "SpecularTex2",
    "TintTexture",
    "LookupTexture",
    "MaterialId",
];

/// The name of the texture parameter whose hash is `hash`, where it is one of those named.
pub fn parameter_name(hash: u32) -> Option<&'static str> {
    (TEXTURE_PARAMETERS.into_iter()).find(|name| name_hash32(name.as_bytes()) == hash)
}

/// What a texture no name is known for is said to be: the first model that names it by its key and
/// the parameter that binds it there, as `model:Parameter`, with how many models more name it. It
/// is no name: it hashes to no key.
pub fn use_label(used: &TextureUse) -> String {
    let parameter = match parameter_name(used.parameter) {
        Some(name) => name.to_string(),
        None => format!("{:#010X}", used.parameter),
    };
    match used.models.saturating_sub(1) {
        0 => format!("{}:{parameter}", used.model),
        more => format!("{}:{parameter} +{more}", used.model),
    }
}

/// The rule that gives a name, in the order a key several rules name keeps the first of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// The stem itself.
    Stem,
    /// The stem, `_` and a map kind.
    MapKind,
    /// The stem with its level-of-detail token swapped for another or none, then a map kind.
    LodSwap,
    /// The stem with its last number, and a letter after it, changed, then a map kind.
    Numbered,
    /// The stem's first or last tokens, then a map kind.
    Closure,
    /// The stem's first tokens, `_c<N>_<NNN>_` and a ped map kind.
    PedComponent,
}

impl Rule {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stem => "the drawable's stem",
            Self::MapKind => "stem and map kind",
            Self::LodSwap => "level of detail swapped",
            Self::Numbered => "numbered variant",
            Self::Closure => "first or last tokens",
            Self::PedComponent => "ped component",
        }
    }
}

/// A drawable's name without its `.#dr` or `.#ft` extension, lower-cased.
pub fn stem(name: &str) -> String {
    let stem = (name.rsplit_once('.'))
        .filter(|(_, extension)| extension.starts_with('#'))
        .map_or(name, |(stem, _)| stem);
    stem.to_ascii_lowercase()
}

/// Each name the rules give from `stem`, with its rule, handed to `each` as it is made.
pub fn candidates(stem: &str, each: &mut impl FnMut(Rule, &str)) {
    let mut name = String::new();
    bases(stem, &mut |_, _| true, &mut |rule, base, kinds| {
        spell(rule, base, kinds, &mut name, each)
    });
}

/// A part of a stem that other stems share, and that alone decides the names made from it.
#[derive(Debug, Clone, Copy)]
enum Shared {
    /// What stands around a stem's last number, with the number's width.
    Numbered,
    /// The tokens before an underscore.
    First,
    /// The tokens after an underscore.
    Last,
}

/// Each base the rules give from `stem`, with its rule and the kinds that end its names after `_`
/// (none where the base is itself the name), handed to `each`; `once` says whether the names of a
/// shared part are still to make.
fn bases(
    stem: &str,
    once: &mut impl FnMut(Shared, &str) -> bool,
    each: &mut impl FnMut(Rule, &str, &[&str]),
) {
    if stem.len() > MAX_STEM {
        return;
    }
    each(Rule::Stem, stem, &[]);
    kinds_and_lods(stem, each);

    if let Some(end) = stem.rfind(|c: char| c.is_ascii_digit()) {
        let start = stem[..end]
            .rfind(|c: char| !c.is_ascii_digit())
            .map_or(0, |i| i + 1);
        let width = end + 1 - start;
        // A letter that ends the number's token belongs to the number.
        let rest = &stem[end + 1..];
        let rest = match rest.as_bytes() {
            [c, ..]
                if c.is_ascii_lowercase()
                    && matches!(rest.as_bytes().get(1), None | Some(b'_')) =>
            {
                &rest[1..]
            }
            _ => rest,
        };
        let head = &stem[..start];
        // No name holds a NUL, so one parts the head from the rest.
        let count = match once(Shared::Numbered, &format!("{head}\0{width}\0{rest}")) {
            true => 10u32
                .checked_pow(width as u32)
                .map_or(NUMBERS, |p| p.min(NUMBERS)),
            false => 0,
        };
        for n in 0..count {
            let plain = format!("{head}{n:0width$}{rest}");
            each(Rule::Numbered, &plain, &MAP_KINDS);
            let letters = if n < LETTERED { "abcdefgh" } else { "" };
            for letter in letters.chars() {
                let variant = format!("{head}{n:0width$}{letter}{rest}");
                each(Rule::Numbered, &variant, &MAP_KINDS);
            }
        }
    }

    for (at, _) in stem.match_indices('_').take(MAX_SPLITS) {
        let (first, last) = (&stem[..at], &stem[at + 1..]);
        let first_is_new = once(Shared::First, first);
        if first_is_new && !first.is_empty() {
            each(Rule::Closure, first, &MAP_KINDS);
        }
        if !last.is_empty() && once(Shared::Last, last) {
            each(Rule::Closure, last, &MAP_KINDS);
        }
        let variants = if first_is_new { PED_VARIANTS } else { 0 };
        for c in 0..variants {
            for set in 0..PED_SETS {
                let component = format!("{first}_c{c}_{set:03}");
                each(Rule::PedComponent, &component, &PED_MAPS);
            }
        }
    }
}

/// The stem with each map kind, then the stem with its level-of-detail token swapped with each
/// map kind.
fn kinds_and_lods(stem: &str, each: &mut impl FnMut(Rule, &str, &[&str])) {
    each(Rule::MapKind, stem, &MAP_KINDS);
    let base = (LODS[1..].iter())
        .find_map(|lod| stem.strip_suffix(lod))
        .unwrap_or(stem);
    let mut swapped = String::new();
    for lod in LODS {
        swapped.clear();
        swapped.push_str(base);
        swapped.push_str(lod);
        if swapped != stem {
            each(Rule::LodSwap, &swapped, &MAP_KINDS);
        }
    }
}

/// Each name the map kind and level-of-detail rules give from `stem`, a name a body spells with
/// no ending, as a material or a model a metadata document names: the two rules cheap enough to
/// run on every name held.
pub fn plain_candidates(stem: &str, each: &mut impl FnMut(Rule, &str)) {
    if stem.len() <= MAX_STEM {
        let mut name = String::new();
        kinds_and_lods(stem, &mut |rule, base, kinds| {
            spell(rule, base, kinds, &mut name, each)
        });
    }
}

/// `base` where there is no kind, else `base`, `_` and each of `kinds`, built in `name`.
fn spell(
    rule: Rule,
    base: &str,
    kinds: &[&str],
    name: &mut String,
    each: &mut impl FnMut(Rule, &str),
) {
    if kinds.is_empty() {
        return each(rule, base);
    }
    for kind in kinds {
        name.clear();
        name.push_str(base);
        name.push('_');
        name.push_str(kind);
        each(rule, name);
    }
}

/// The keys the rules name, each with its name and the rule that gave it.
pub type Named = HashMap<u64, (String, Rule)>;

/// The names a worker takes at a time, after which it tells how many and asks whether to go on.
const BLOCK: usize = if cfg!(test) { 16 } else { 512 };
/// The shared parts a worker remembers having made the names of.
const REMEMBERED: usize = 1 << 16;

/// The keys sought, behind a bit a key that tells most candidates from them without a lookup.
struct Probe<'a> {
    keys: &'a HashSet<u64>,
    /// Empty where the names are too few to pay for setting the bits.
    bits: Vec<u64>,
    mask: u64,
}

impl<'a> Probe<'a> {
    fn new(keys: &'a HashSet<u64>, names: usize) -> Self {
        // A key is a hash, so its low bits spread: 32 bits a key send one candidate in 32 to the set.
        let slots = (keys.len() * 32).next_power_of_two().max(64);
        let mask = slots as u64 - 1;
        // A name makes a hundred candidates and more, and setting the bits costs a step a key.
        let mut bits = match names.saturating_mul(16) < keys.len() {
            true => Vec::new(),
            false => vec![0u64; slots / 64],
        };
        if !bits.is_empty() {
            for key in keys {
                let at = key & mask;
                bits[(at >> 6) as usize] |= 1 << (at & 63);
            }
        }
        Self { keys, bits, mask }
    }

    fn holds(&self, key: u64) -> bool {
        let at = key & self.mask;
        let bit = |word: &u64| ((word >> (at & 63)) & 1) == 1;
        // No bits set stand for every bit set.
        (self.bits.get((at >> 6) as usize)).is_none_or(bit) && self.keys.contains(&key)
    }
}

/// The shared parts whose names a worker has made, by hash; emptied when full, which costs only
/// making some names again.
#[derive(Default)]
struct Seen(HashSet<u64>);

impl Seen {
    /// Whether the names of the shared part `text` are still to make; they are not after this.
    fn once(&mut self, part: Shared, text: &str) -> bool {
        if self.0.len() >= REMEMBERED {
            self.0.clear();
        }
        let tag = (part as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0.insert(name_hash64(text.as_bytes()) ^ tag)
    }
}

/// Holds the name `name` gives for `key` in `found` where none is held, or where `rule` comes
/// before the rule that gave the one held.
fn keep(found: &mut Named, key: u64, rule: Rule, name: impl FnOnce() -> String) {
    match found.entry(key) {
        Entry::Vacant(slot) => {
            slot.insert((name(), rule));
        }
        Entry::Occupied(mut held) if rule < held.get().1 => {
            held.insert((name(), rule));
        }
        Entry::Occupied(_) => {}
    }
}

/// Holds in `found` each key of `probe` a name the rules give from `name` hashes to.
// A base is hashed once and each kind ends that hash; no name is built but one that names a key.
fn seek(name: &str, plain: bool, probe: &Probe<'_>, seen: &mut Seen, found: &mut Named) {
    let mut hold = |rule: Rule, base: &str, kinds: &[&str]| {
        let start = NameHasher::default().with(base.as_bytes());
        if kinds.is_empty() && probe.holds(start.finish()) {
            keep(found, start.finish(), rule, || base.to_string());
        }
        let start = start.with(b"_");
        for kind in kinds {
            let key = start.with(kind.as_bytes()).finish();
            if probe.holds(key) {
                keep(found, key, rule, || format!("{base}_{kind}"));
            }
        }
    };
    match plain {
        true => {
            let lower = name.to_ascii_lowercase();
            if lower.len() <= MAX_STEM {
                kinds_and_lods(&lower, &mut hold);
            }
        }
        false => bases(
            &stem(name),
            &mut |part, text| seen.once(part, text),
            &mut hold,
        ),
    }
}

/// Each key of `keys` a candidate from `names` hashes to, with its name and the rule that gave
/// it, the first of the rules in their order where several give it.
pub fn name_keys<'a>(names: impl IntoIterator<Item = &'a str>, keys: &HashSet<u64>) -> Named {
    name_keys_by(names, keys, false, &|_| true)
}

/// As [`name_keys`], from names spelled with no ending, by [`plain_candidates`].
pub fn name_keys_of_plain<'a>(
    names: impl IntoIterator<Item = &'a str>,
    keys: &HashSet<u64>,
) -> Named {
    name_keys_by(names, keys, true, &|_| true)
}

/// As [`name_keys`], or as [`name_keys_of_plain`] where `plain`, on every core: `go_on` is told
/// how many names more are done and says whether to go on, and a stop gives what was found.
// One pass over the names whatever the keys. The names are sorted so those that share a part
// stand together, each worker takes blocks of them in turn and makes a shared part's names
// once, and a worker holds its finds and the parts it remembers: nothing a candidate.
pub fn name_keys_by<'a>(
    names: impl IntoIterator<Item = &'a str>,
    keys: &HashSet<u64>,
    plain: bool,
    go_on: &(dyn Fn(usize) -> bool + Sync),
) -> Named {
    if keys.is_empty() {
        return Named::new();
    }
    let mut names: Vec<&str> = names.into_iter().collect();
    names.sort_unstable();
    names.dedup();
    let (probe, next) = (Probe::new(keys, names.len()), AtomicUsize::new(0));
    let work = || {
        let (mut found, mut seen) = (Named::new(), Seen::default());
        loop {
            let from = next.fetch_add(BLOCK, Ordering::Relaxed);
            let block = names.get(from..).unwrap_or_default();
            let block = &block[..block.len().min(BLOCK)];
            for name in block {
                seek(name, plain, &probe, &mut seen, &mut found);
            }
            if block.is_empty() || !go_on(block.len()) {
                return found;
            }
        }
    };
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let workers = cores.min(names.len().div_ceil(BLOCK));
    if workers <= 1 {
        // No thread is started for one, as where none can be.
        return work();
    }
    let found: Vec<Named> = std::thread::scope(|scope| {
        let started: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
        let ended = started.into_iter().map(|worker| match worker.join() {
            Ok(found) => found,
            Err(panic) => std::panic::resume_unwind(panic),
        });
        ended.collect()
    });
    let mut all = Named::new();
    for (key, (name, rule)) in found.into_iter().flatten() {
        keep(&mut all, key, rule, move || name);
    }
    all
}

/// As [`name_keys`], with each key a name `vocabulary` holds hashes to named by it: a name a
/// body spells outranks a rule's candidate.
pub fn name_keys_with<'a>(
    vocabulary: &super::names::Vocabulary,
    names: impl IntoIterator<Item = &'a str>,
    keys: &HashSet<u64>,
) -> HashMap<u64, (String, Rule)> {
    let mut found = name_keys(names, keys);
    for key in keys {
        if let Some(name) = vocabulary.name64(*key) {
            found.insert(*key, (name.to_string(), Rule::Stem));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(drawable: &str, textures: &[&str]) -> HashMap<u64, (String, Rule)> {
        let keys = textures.iter().map(|t| name_hash64(t.as_bytes())).collect();
        name_keys([drawable], &keys)
    }

    fn rule_of(found: &HashMap<u64, (String, Rule)>, texture: &str) -> Option<Rule> {
        let (name, rule) = found.get(&name_hash64(texture.as_bytes()))?;
        assert_eq!(name, texture);
        Some(*rule)
    }

    /// Made-up names that share their first tokens, what stands around their numbers and their
    /// levels of detail, enough of them for several blocks.
    fn made_up_names() -> Vec<String> {
        let mut names = Vec::new();
        for group in 0..4 {
            for n in 0..9 {
                names.push(format!("zq{group}_hut_{n:02}.#dr"));
                names.push(format!("zq{group}_hut_{n:02}_lod.#dr"));
                names.push(format!("Zq{group}_Mast_top_{n}a.#ft"));
            }
        }
        assert!(names.len() > 4 * BLOCK);
        names
    }

    /// The keys named a name at a time: every candidate spelled whole and hashed whole.
    fn named_one_at_a_time(names: &[String], keys: &HashSet<u64>, plain: bool) -> Named {
        let mut found = Named::new();
        for name in names {
            let mut hold = |rule, candidate: &str| {
                let key = name_hash64(candidate.as_bytes());
                if keys.contains(&key) {
                    keep(&mut found, key, rule, || candidate.to_string());
                }
            };
            match plain {
                true => plain_candidates(&name.to_ascii_lowercase(), &mut hold),
                false => candidates(&stem(name), &mut hold),
            }
        }
        found
    }

    #[test]
    fn many_names_are_named_in_one_pass_as_they_are_a_name_at_a_time() {
        let names = made_up_names();
        let stems: Vec<String> = names.iter().map(|name| stem(name)).collect();
        // Each stem is a key, and its albedo, and one candidate in 997, and a word no name hashes
        // to: few enough for the names to pay for the probe's bits.
        let mut keys: HashSet<u64> = stems.iter().map(|s| name_hash64(s.as_bytes())).collect();
        keys.extend(
            stems
                .iter()
                .map(|s| name_hash64(format!("{s}_ab").as_bytes())),
        );
        keys.insert(7);
        // A first token's albedo and a level of detail no name states, so those rules name a key.
        keys.extend(["zq0_ab", "zq0_hut_00_slod_ab"].map(|name| name_hash64(name.as_bytes())));
        let mut made = 0usize;
        for one in &stems {
            candidates(one, &mut |_, candidate| {
                made += 1;
                if made.is_multiple_of(997) {
                    keys.insert(name_hash64(candidate.as_bytes()));
                }
            });
        }

        assert!(!Probe::new(&keys, names.len()).bits.is_empty());
        assert!(Probe::new(&keys, 1).bits.is_empty());

        let told = AtomicUsize::new(0);
        let go_on = |more: usize| {
            told.fetch_add(more, Ordering::Relaxed);
            true
        };
        let found = name_keys_by(names.iter().map(String::as_str), &keys, false, &go_on);
        assert_eq!(found, named_one_at_a_time(&names, &keys, false));
        assert_eq!(told.into_inner(), names.len());
        for rule in [
            Rule::Stem,
            Rule::MapKind,
            Rule::LodSwap,
            Rule::Numbered,
            Rule::Closure,
            Rule::PedComponent,
        ] {
            assert!(found.values().any(|(_, by)| *by == rule), "{rule:?}");
        }
        // A name given twice is run once.
        let twice = names.iter().chain(&names).map(String::as_str);
        assert_eq!(name_keys(twice, &keys), found);

        let plain = name_keys_of_plain(stems.iter().map(String::as_str), &keys);
        assert!(!plain.is_empty());
        assert_eq!(plain, named_one_at_a_time(&stems, &keys, true));
    }

    #[test]
    fn a_part_several_stems_share_makes_its_names_once() {
        let count = |stems: &[&str]| {
            let (mut seen, mut made) = (Seen::default(), 0usize);
            for stem in stems {
                bases(
                    stem,
                    &mut |part, text| seen.once(part, text),
                    &mut |_, _, kinds| made += kinds.len().max(1),
                );
            }
            made
        };
        let alone = count(&["zq_hut_03"]);
        let mut whole = 0usize;
        candidates("zq_hut_03", &mut |_, _| whole += 1);
        assert_eq!(alone, whole);
        // A sibling by its number makes its own stem and levels of detail, and its last tokens
        // after each of its two underscores.
        let own = 1 + MAP_KINDS.len() * LODS.len();
        assert_eq!(
            count(&["zq_hut_03", "zq_hut_04"]),
            alone + own + 2 * MAP_KINDS.len()
        );
        // The same stem again makes them alone.
        assert_eq!(count(&["zq_hut_03", "zq_hut_03"]), alone + own);
        // A part forgotten is made again, and never left out.
        let mut seen = Seen::default();
        assert!(seen.once(Shared::First, "zq"));
        assert!(!seen.once(Shared::First, "zq"));
        assert!(seen.once(Shared::Last, "zq"));
        for n in 0..REMEMBERED {
            seen.once(Shared::Numbered, &n.to_string());
        }
        assert!(seen.once(Shared::First, "zq"));
    }

    #[test]
    fn a_pass_told_to_stop_ends_after_the_block_each_worker_holds() {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        // Longer than a stem the rules take, so no name is made: only the blocks are counted.
        let long = "q".repeat(MAX_STEM + 1);
        let names: Vec<String> = (0..(cores + 2) * BLOCK)
            .map(|n| format!("{long}{n}"))
            .collect();
        let told = AtomicUsize::new(0);
        let stop = |more: usize| {
            told.fetch_add(more, Ordering::Relaxed);
            false
        };
        let names = names.iter().map(String::as_str);
        assert!(name_keys_by(names, &HashSet::from([7]), false, &stop).is_empty());
        let told = told.into_inner();
        assert!((BLOCK..=cores * BLOCK).contains(&told), "{told}");
    }

    #[test]
    fn a_stem_is_the_name_without_its_extension_lower_cased() {
        assert_eq!(stem("Zq_Shed_07.#dr"), "zq_shed_07");
        assert_eq!(stem("zq_crane.#ft"), "zq_crane");
        assert_eq!(stem("zq.v2_hut"), "zq.v2_hut");
    }

    #[test]
    fn each_rule_names_a_texture_from_a_made_up_drawable_name() {
        let cases = [
            ("zq_shed_07.#dr", "zq_shed_07", Rule::Stem),
            ("zq_shed_07.#dr", "zq_shed_07_nm", Rule::MapKind),
            ("zq_shed_07.#dr", "zq_shed_07_tint", Rule::MapKind),
            ("zq_shed_07.#dr", "zq_shed_07_lod_ma", Rule::LodSwap),
            ("zq_shed_07_lod.#dr", "zq_shed_07_slod2_abal", Rule::LodSwap),
            ("zq_shed_07_lod.#dr", "zq_shed_07_em", Rule::LodSwap),
            ("zq_crate_01a.#dr", "zq_crate_05c_mb", Rule::Numbered),
            ("zq_crate_01a.#dr", "zq_crate_42_mb", Rule::Numbered),
            ("zq_tower_top_part.#dr", "zq_tower_ab", Rule::Closure),
            ("zq_tower_top_part.#dr", "top_part_tn", Rule::Closure),
            (
                "jacketzq_mr1_003_long_full.#dr",
                "jacketzq_mr1_003_long_c1_012_tn",
                Rule::PedComponent,
            ),
            (
                "jacketzq_mr1_003_long_alt_00.#dr",
                "jacketzq_mr1_003_long_c0_000_ab",
                Rule::PedComponent,
            ),
        ];
        for (drawable, texture, rule) in cases {
            let found = named(drawable, &[texture]);
            assert_eq!(
                rule_of(&found, texture),
                Some(rule),
                "{drawable} -> {texture}"
            );
        }
    }

    #[test]
    fn a_name_spelled_with_no_ending_names_by_map_kind_and_level_of_detail_alone() {
        let keys: HashSet<u64> = [
            "zq_brick_wall_nm",
            "zq_brick_wall_lod_ab",
            "zq_brick_wall",
            "zq_brick_ab",
            "zq_brick_wall_07_ab",
        ]
        .iter()
        .map(|t| name_hash64(t.as_bytes()))
        .collect();
        let found = name_keys_of_plain(["Zq_Brick_Wall"], &keys);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(rule_of(&found, "zq_brick_wall_nm"), Some(Rule::MapKind));
        assert_eq!(rule_of(&found, "zq_brick_wall_lod_ab"), Some(Rule::LodSwap));
        // The rules a model's name takes besides name the stem itself and its first tokens too.
        assert_eq!(name_keys(["zq_brick_wall"], &keys).len(), 4);
        let mut made = 0usize;
        plain_candidates("zq_brick_wall", &mut |_, _| made += 1);
        assert_eq!(made, MAP_KINDS.len() * LODS.len());
        plain_candidates(&"q".repeat(MAX_STEM + 1), &mut |_, _| made = 0);
        assert_ne!(made, 0, "a stem too long makes no name");
    }

    #[test]
    fn a_texture_with_no_name_is_said_by_the_model_that_names_it_and_its_parameter() {
        let diffuse = name_hash32(b"DiffuseTex");
        assert_eq!(parameter_name(diffuse), Some("DiffuseTex"));
        assert_eq!(parameter_name(name_hash32(b"bumptex")), Some("BumpTex"));
        assert_eq!(parameter_name(7), None);
        let mut used = TextureUse {
            model: "zq_shed_07".to_string(),
            parameter: diffuse,
            models: 1,
        };
        assert_eq!(use_label(&used), "zq_shed_07:DiffuseTex");
        (used.parameter, used.models) = (0x1234, 3);
        assert_eq!(use_label(&used), "zq_shed_07:0x00001234 +2");
    }

    #[test]
    fn a_name_that_fits_no_rule_stays_a_key() {
        let outside = [
            "zq_shed_07_unknownkind",
            "zq_barn_07_ab",
            "zq_crate_45c_mb",
            "zq_crate_100_mb",
            "jacketzq_mr1_003_long_c4_000_tn",
            "jacketzq_mr1_003_long_c0_040_tn",
        ];
        for drawable in [
            "zq_shed_07.#dr",
            "zq_crate_01a.#dr",
            "jacketzq_mr1_003_long_full.#dr",
        ] {
            assert!(named(drawable, &outside).is_empty(), "{drawable}");
        }
    }

    #[test]
    fn a_long_stem_or_one_of_many_tokens_makes_a_bounded_count_of_names() {
        let count = |stem: &str, rules: &[Rule]| {
            let mut n = 0usize;
            candidates(stem, &mut |rule, _| n += usize::from(rules.contains(&rule)));
            n
        };
        let every = [
            Rule::Stem,
            Rule::MapKind,
            Rule::LodSwap,
            Rule::Numbered,
            Rule::Closure,
            Rule::PedComponent,
        ];
        let tokens: Vec<String> = (0..30).map(|k| format!("t{k}")).collect();
        let stem = tokens.join("_");
        assert!(stem.len() <= MAX_STEM);
        let per_split = 2 * MAP_KINDS.len() + (PED_VARIANTS * PED_SETS) as usize * PED_MAPS.len();
        assert_eq!(
            count(&stem, &[Rule::Closure, Rule::PedComponent]),
            MAX_SPLITS * per_split
        );

        assert_eq!(count(&format!("a{}", "_".repeat(65_536)), &every), 0);
        assert_eq!(count(&"q".repeat(MAX_STEM + 1), &every), 0);
        assert_eq!(count(&"q".repeat(MAX_STEM), &[Rule::Stem]), 1);
    }

    #[test]
    fn only_a_held_key_is_named_and_the_first_rule_keeps_it() {
        let held = name_hash64(b"zq_shed_07_ab");
        let found = name_keys(["zq_shed_07.#dr", "zq_shed.#dr"], &HashSet::from([held, 7]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[&held], ("zq_shed_07_ab".to_string(), Rule::MapKind));
        assert!(name_keys(["zq_shed_07"], &HashSet::new()).is_empty());
    }

    #[test]
    fn a_name_the_vocabulary_holds_names_a_key_no_rule_reaches() {
        let mut vocabulary = crate::gta6::names::Vocabulary::default();
        vocabulary.add("zq_odd_texture_name", None);
        let (held, ruled) = (
            name_hash64(b"zq_odd_texture_name"),
            name_hash64(b"zq_shed_07_nm"),
        );
        let keys = HashSet::from([held, ruled, 7]);
        let found = name_keys_with(&vocabulary, ["zq_shed_07.#dr"], &keys);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[&held],
            ("zq_odd_texture_name".to_string(), Rule::Stem)
        );
        assert_eq!(found[&ruled], ("zq_shed_07_nm".to_string(), Rule::MapKind));
        assert_eq!(name_keys(["zq_shed_07.#dr"], &keys).len(), 1);
    }
}
