//! The cross products of `hashes cross`: names made of the parts of the names already known, hashed
//! on every thread and kept where a hash is an unresolved entry's. No body is read.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use or_io::gta6::hash::{name_hash64, NameHasher};

use super::{Miner, Source};

/// The rounds of stems and endings at most; a round that names nothing ends them.
const ROUNDS: usize = 6;
/// The most frequent endings a round joins to every stem.
// ponytail: a fixed count; rank by hits per ending once a run shows which endings pay.
const ENDINGS: usize = 30_000;
/// The most frequent words a round puts between a stem and a number.
const WORDS: usize = 400;
/// A component longer than this is no entry's name.
const MAX_NAME: usize = 120;
/// Below this many candidates a product is hashed on one thread.
const ONE_THREAD: f64 = 2e6;
const SEPARATORS: &[u8] = b"_@-.+";

/// The unresolved hashes in an open table probed linearly; an empty slot is 0, which names no entry.
struct Targets {
    slots: Vec<u64>,
    mask: usize,
}

impl Targets {
    fn new(hashes: impl Iterator<Item = u64>) -> Self {
        let hashes: Vec<u64> = hashes.filter(|hash| *hash != 0).collect();
        let size = (hashes.len() * 4).next_power_of_two().max(16);
        let mut slots = vec![0u64; size];
        for hash in hashes {
            let mut at = hash as usize & (size - 1);
            while slots[at] != 0 && slots[at] != hash {
                at = (at + 1) & (size - 1);
            }
            slots[at] = hash;
        }
        Self {
            slots,
            mask: size - 1,
        }
    }

    fn has(&self, hash: u64) -> bool {
        let mut at = hash as usize & self.mask;
        loop {
            match self.slots[at] {
                slot if slot == hash => return true,
                0 => return false,
                _ => at = (at + 1) & self.mask,
            }
        }
    }
}

/// Every concatenation of one item of each of `lists`, in their order, whose hash is in `targets`.
/// The longest list is shared out to the threads; an item's hash state is made once for all that follow it.
fn product(targets: &Targets, lists: &[&[String]]) -> Vec<String> {
    if lists.is_empty() || lists.iter().any(|list| list.is_empty()) {
        return Vec::new();
    }
    let split = (0..lists.len())
        .max_by_key(|&at| lists[at].len())
        .unwrap_or(0);
    let size: f64 = lists.iter().map(|list| list.len() as f64).product();
    let threads = if size < ONE_THREAD {
        1
    } else {
        std::thread::available_parallelism().map_or(4, |n| n.get())
    };
    let step = (lists[split].len() / (threads * 8)).clamp(1, 64);
    let next = AtomicUsize::new(0);
    let hits = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut found = Vec::new();
                let mut picked = Vec::with_capacity(lists.len());
                loop {
                    let from = next.fetch_add(step, Ordering::Relaxed);
                    if from >= lists[split].len() {
                        break;
                    }
                    let to = (from + step).min(lists[split].len());
                    let mut mine = lists.to_vec();
                    mine[split] = &lists[split][from..to];
                    walk(
                        targets,
                        &mine,
                        NameHasher::default(),
                        &mut picked,
                        &mut found,
                    );
                }
                hits.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .append(&mut found);
            });
        }
    });
    hits.into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn walk<'a>(
    targets: &Targets,
    lists: &[&'a [String]],
    state: NameHasher,
    picked: &mut Vec<&'a str>,
    found: &mut Vec<String>,
) {
    let Some((first, rest)) = lists.split_first() else {
        if targets.has(state.finish()) {
            found.push(picked.concat());
        }
        return;
    };
    for item in *first {
        picked.push(item.as_str());
        walk(targets, rest, state.with(item.as_bytes()), picked, found);
        picked.pop();
    }
}

/// Adds the lower-case components of the path `text`, each whole and without what follows its first `.`.
fn components(text: &str, names: &mut BTreeSet<String>) {
    for part in text.split(['/', '\\']) {
        if part.is_empty() || part.len() > MAX_NAME {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if let Some((stem, _)) = lower.split_once('.') {
            if !stem.is_empty() {
                names.insert(stem.to_string());
            }
        }
        names.insert(lower);
    }
}

fn without_digits(name: &str) -> &str {
    name.trim_end_matches(|c: char| c.is_ascii_digit())
}

/// The stems and the endings of `names`: each name cut at every separator, which goes to either
/// side, and before the digits it ends with; the [`ENDINGS`] most frequent endings, most frequent first.
fn parts(names: &BTreeSet<String>) -> (Vec<String>, Vec<String>) {
    let mut stems: BTreeSet<&str> = BTreeSet::new();
    let mut endings: HashMap<&str, usize> = HashMap::new();
    for name in names {
        let bare = without_digits(name);
        stems.insert(name.as_str());
        stems.insert(bare);
        *endings.entry(&name[bare.len()..]).or_default() += 1;
        for (at, byte) in name.bytes().enumerate().skip(1) {
            if SEPARATORS.contains(&byte) {
                stems.insert(&name[..at]);
                stems.insert(&name[..=at]);
                *endings.entry(&name[at..]).or_default() += 1;
                *endings.entry(&name[at + 1..]).or_default() += 1;
            }
        }
    }
    stems.remove("");
    endings.remove("");
    let mut ranked: Vec<(&str, usize)> = endings.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.truncate(ENDINGS);
    (
        stems.into_iter().map(str::to_string).collect(),
        ranked
            .into_iter()
            .map(|(ending, _)| ending.to_string())
            .collect(),
    )
}

/// The [`WORDS`] most frequent words of `names` between separators, each after a `_`.
fn words(names: &BTreeSet<String>) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in names {
        for word in name.split(|c: char| c.is_ascii() && SEPARATORS.contains(&(c as u8))) {
            if !word.is_empty() && !word.bytes().all(|b| b.is_ascii_digit()) {
                *counts.entry(word).or_default() += 1;
            }
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    ranked.truncate(WORDS);
    ranked
        .into_iter()
        .map(|(word, _)| format!("_{word}"))
        .collect()
}

/// The numberings a stem takes: none, 0 to 300 bare and after a `_`, and two and three padded digits after a `_`.
fn numbers() -> Vec<String> {
    let mut all = BTreeSet::from([String::new()]);
    for n in 0..=300 {
        all.insert(n.to_string());
        all.insert(format!("_{n}"));
    }
    all.extend((0..100).map(|n| format!("_{n:02}")));
    all.extend((0..1000).map(|n| format!("_{n:03}")));
    all.into_iter().collect()
}

/// Every string of `least` to `most` decimal digits.
fn digit_strings(least: usize, most: usize) -> Vec<String> {
    let of_width = |width: usize| {
        (0..10u32.pow(width as u32)).map(move |n| match width {
            0 => String::new(),
            _ => format!("{n:0width$}"),
        })
    };
    (least..=most).flat_map(of_width).collect()
}

impl Miner {
    /// Tries every concatenation of `lists` against the entries still unresolved; the names kept
    /// join `names`, and how many there were.
    fn cross_pass(&mut self, names: &mut BTreeSet<String>, lists: &[&[String]]) -> usize {
        let open = (self.entry.keys().copied()).filter(|hash| !self.found64.contains_key(hash));
        let targets = Targets::new(open);
        let before = self.found64.len();
        for hit in product(&targets, lists) {
            self.try_entry(&hit);
            if self.found64.contains_key(&name_hash64(hit.as_bytes())) {
                names.insert(hit);
            }
        }
        self.found64.len() - before
    }

    /// Names the unresolved entries of the tables read from the names known, those of `extra`
    /// among them: each name's path components, then every stem with every frequent ending, with
    /// a number and with a frequent word and a number, again while a round names something, then
    /// the names that are only digits. The report's lines.
    pub(super) fn cross(&mut self, extra: impl IntoIterator<Item = String>) -> Vec<String> {
        let was = std::mem::replace(&mut self.source, Source::Rules);
        let mut names = BTreeSet::new();
        for name in self.resolved.iter().chain(self.named64.values()) {
            components(name, &mut names);
        }
        for name in extra {
            components(&name, &mut names);
        }
        let mut lines = Vec::new();
        let known: Vec<String> = names.iter().cloned().collect();
        let found = self.cross_pass(&mut names, &[known.as_slice()]);
        lines.push(format!(
            "  cross         {found} named by the components of {} names known",
            known.len()
        ));
        let numbers = numbers();
        let counts: Vec<String> = (0..100).map(|n| format!("_{n}")).collect();
        for round in 1..=ROUNDS {
            let (stems, endings) = parts(&names);
            let words = words(&names);
            let found = self.cross_pass(&mut names, &[stems.as_slice(), endings.as_slice()])
                + self.cross_pass(&mut names, &[stems.as_slice(), numbers.as_slice()])
                + self.cross_pass(
                    &mut names,
                    &[stems.as_slice(), words.as_slice(), counts.as_slice()],
                );
            lines.push(format!(
                "  cross         round {round}: {found} named by {} stems with {} endings, {} numberings and {} words",
                stems.len(),
                endings.len(),
                numbers.len(),
                words.len()
            ));
            if found == 0 {
                break;
            }
        }
        let (heads, tails) = (digit_strings(1, 5), digit_strings(0, 5));
        let found = self.cross_pass(&mut names, &[heads.as_slice(), tails.as_slice()]);
        lines.push(format!(
            "  cross         {found} named by one to ten digits"
        ));
        self.source = was;
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn a_product_keeps_the_concatenations_whose_hash_is_a_target() {
        let targets =
            Targets::new([name_hash64(b"ss_33_instances_4"), name_hash64(b"core_12")].into_iter());
        let (stems, mids, ends) = (
            list(&["ss_33", "core", "ds_01"]),
            list(&["", "_instances"]),
            list(&["_4", "_12", "_13"]),
        );
        let mut hits = product(
            &targets,
            &[stems.as_slice(), mids.as_slice(), ends.as_slice()],
        );
        hits.sort();
        assert_eq!(hits, ["core_12", "ss_33_instances_4"]);
        assert!(product(&targets, &[stems.as_slice(), &[]]).is_empty());
    }

    #[test]
    fn a_name_is_cut_at_its_separators_and_before_its_digits() {
        let names = BTreeSet::from(["ss_33_long_0".to_string(), "core12".to_string()]);
        let (stems, endings) = parts(&names);
        for stem in ["ss", "ss_", "ss_33", "ss_33_long_", "core", "core12"] {
            assert!(stems.iter().any(|s| s == stem), "{stem} in {stems:?}");
        }
        for ending in ["_33_long_0", "long_0", "_0", "0", "12"] {
            assert!(
                endings.iter().any(|e| e == ending),
                "{ending} in {endings:?}"
            );
        }
        assert!(!stems.iter().any(String::is_empty) && !endings.iter().any(String::is_empty));
    }

    #[test]
    fn the_digit_strings_reach_every_width_once() {
        let digits = digit_strings(0, 2);
        assert_eq!(digits.len(), 1 + 10 + 100);
        assert!(digits.iter().any(String::is_empty) && digits.iter().any(|d| d == "07"));
        assert_eq!(digit_strings(1, 1).len(), 10);
    }
}
