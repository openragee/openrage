//! The rules of `hashes mine`: candidates made from the names the tables' rows already have, each
//! spelled out only where its hash is an unresolved entry's.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use or_io::gta6::hash::NameHasher;

use super::{stem_of, Miner, Source};

/// The rounds of rules over the names each round found, fewer where a round names nothing.
const ROUNDS: usize = 4;
/// The extensions and suffixes the rules try, the commonest first.
const RULE_TOP: usize = 64;
/// The twins whose names the twin rule tries for one unnamed archive, the most rows shared first.
const TWINS: usize = 3;
/// The longest ending after a `_` the twin rule spells out letter by letter.
const SHORT_ENDING: usize = 3;
/// The candidates the nested-archive rule tries for one unnamed archive at most.
// ponytail: a table with many named directories joins only the ones met first; the report counts
// the archives cut short.
pub(super) const NESTED_CAP: usize = 1 << 17;
/// The longest ending a named archive adds to a stem of its rows.
const MAX_ENDING: usize = 24;
/// The stem-and-ending candidates the sibling rule hashes in one table at most.
// ponytail: a table past this tries only the stems met first, alphabetically; the report counts
// the tables cut short.
pub(super) const SIBLING_CAP: usize = 1 << 22;
/// The three-digit numbers the sibling rule tries after a stem and a `_` in every directory, past
/// the two digits a sibling's own numbering reaches at the default range.
const WIDE_NUMBERS: std::ops::Range<u32> = 100..1000;
/// The named rows of one table, spread over it, that the head rule learns its heads from.
const HEAD_SAMPLE: usize = 3;

impl Miner {
    /// Tries, before the last component of [`HEAD_SAMPLE`] named rows of each table with an
    /// unresolved row, each head of [`short_heads`]; each head that names a row there is then tried
    /// before the last component of every named row of that table.
    pub(super) fn head_rules(&mut self) {
        let heads = short_heads();
        let was = std::mem::replace(&mut self.source, Source::Rules);
        for at in 0..self.tree.len() {
            let rows = &self.tree[at].rows;
            if !rows.iter().any(|&row| self.unresolved(row)) {
                continue;
            }
            let mut named: Vec<String> = (rows.iter())
                .filter_map(|&row| self.name64(row))
                .map(str::to_ascii_lowercase)
                .collect();
            named.sort_unstable();
            named.dedup();
            let step = (named.len() / HEAD_SAMPLE).max(1);
            let mut learnt = BTreeSet::new();
            for name in named.iter().step_by(step) {
                for head in &heads {
                    if self.try_head(name, head) {
                        learnt.insert(head.as_str());
                    }
                }
            }
            for head in learnt {
                for name in &named {
                    self.try_head(name, head);
                }
            }
        }
        self.source = was;
    }

    /// Whether `head` before the last component of `name` names an unresolved entry, now found.
    fn try_head(&mut self, name: &str, head: &str) -> bool {
        let (dir, last) = name.split_at(name.rfind('/').map_or(0, |cut| cut + 1));
        let state = NameHasher::default().with(dir.as_bytes());
        let hash = state.with(head.as_bytes()).with(last.as_bytes()).finish();
        if !self.unresolved(hash) {
            return false;
        }
        self.try_entry(&format!("{dir}{head}{last}"));
        self.found64.contains_key(&hash)
    }

    /// Tries each unnamed nested archive as a directory a named archive of the same table lies in,
    /// joined with a stem the archive's named rows share or with that directory's last component,
    /// then with each ending a named archive adds to such a stem and each extension named archives carry.
    pub(super) fn nested_rules(&mut self) {
        let mut dirs: HashMap<Option<usize>, BTreeSet<String>> = HashMap::new();
        let (mut endings, mut exts, mut unnamed) = (Vec::new(), BTreeSet::new(), Vec::new());
        for table in &self.tree {
            let Some(from) = table.from else {
                continue;
            };
            let stems = shared_stems(table.rows.iter().filter_map(|&hash| self.name64(hash)));
            let Some(archive) = self.name64(from) else {
                if self.entry.contains_key(&from) {
                    unnamed.push((from, table.parent, stems));
                }
                continue;
            };
            let archive = archive.to_ascii_lowercase();
            let stem = stem_of(&archive);
            exts.insert(archive[stem.len()..].to_string());
            let (dir, last) = stem.split_at(stem.rfind('/').map_or(0, |at| at + 1));
            let ends: BTreeSet<&str> = (stems.iter().map(String::as_str).chain(dir_stem(dir)))
                .filter_map(|s| last.strip_prefix(s))
                .filter(|end| end.starts_with('_') && end.len() <= MAX_ENDING)
                .collect();
            endings.extend(ends.into_iter().map(str::to_owned));
            dirs.entry(table.parent)
                .or_default()
                .insert(dir.to_string());
        }
        let mut endings = ranked(counted(endings.into_iter()), 2);
        endings.insert(0, String::new());
        let was = std::mem::replace(&mut self.source, Source::Rules);
        for (from, parent, stems) in unnamed {
            let Some(dirs) = dirs.get(&parent) else {
                continue;
            };
            let mut budget = NESTED_CAP;
            'archive: for dir in dirs {
                for stem in stems.iter().map(String::as_str).chain(dir_stem(dir)) {
                    for end in &endings {
                        for ext in &exts {
                            if self.found64.contains_key(&from) {
                                break 'archive;
                            }
                            if budget == 0 {
                                self.nested_capped.insert(from);
                                break 'archive;
                            }
                            budget -= 1;
                            self.try_name(&format!("{dir}{stem}{end}{ext}"));
                        }
                    }
                }
            }
        }
        self.source = was;
    }

    /// Tries each unnamed nested archive as the name of a named one whose rows it shares, the most
    /// rows shared first, with each ending of a `_` and up to [`SHORT_ENDING`] letters or digits.
    pub(super) fn twin_rules(&mut self) {
        let mut holders: HashMap<u64, Vec<usize>> = HashMap::new();
        for (at, table) in self.tree.iter().enumerate() {
            if table.from.is_some_and(|from| self.name64(from).is_some()) {
                for &row in table.rows.iter().filter(|&&row| row != 0) {
                    holders.entry(row).or_default().push(at);
                }
            }
        }
        let endings = short_endings();
        let was = std::mem::replace(&mut self.source, Source::Rules);
        for at in 0..self.tree.len() {
            let Some(from) = (self.tree[at].from)
                .filter(|from| self.entry.contains_key(from) && !self.found64.contains_key(from))
            else {
                continue;
            };
            let mut shared: HashMap<usize, usize> = HashMap::new();
            for row in &self.tree[at].rows {
                for &twin in holders.get(row).into_iter().flatten() {
                    *shared.entry(twin).or_default() += 1;
                }
            }
            let mut twins: Vec<(usize, usize)> = shared.into_iter().collect();
            twins.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            'twins: for (twin, _) in twins.into_iter().take(TWINS) {
                let Some(name) = (self.tree[twin].from).and_then(|f| self.name64(f)) else {
                    continue;
                };
                let name = name.to_ascii_lowercase();
                let state = NameHasher::default().with(name.as_bytes());
                for end in &endings {
                    if state.with(end.as_bytes()).finish() == from {
                        self.try_entry(&format!("{name}{end}"));
                        break 'twins;
                    }
                }
            }
        }
        self.source = was;
    }

    /// Tries each unresolved row of a table as a stem that named rows of one of its directories
    /// share, with each ending those rows add to such a stem, its last run of digits renumbered
    /// below `range`, and with a `_` and each of [`WIDE_NUMBERS`]; a candidate is spelled out only
    /// where its hash is an unresolved entry's.
    pub(super) fn sibling_rules(&mut self, range: u32) {
        let was = std::mem::replace(&mut self.source, Source::Rules);
        for at in 0..self.tree.len() {
            let rows = &self.tree[at].rows;
            if !rows.iter().any(|&row| self.unresolved(row)) {
                continue;
            }
            let mut dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for name in rows.iter().filter_map(|&row| self.name64(row)) {
                let name = name.to_ascii_lowercase();
                let (dir, last) = name.split_at(name.rfind('/').map_or(0, |cut| cut + 1));
                dirs.entry(dir.to_string())
                    .or_default()
                    .push(last.to_string());
            }
            let mut budget = SIBLING_CAP;
            'table: for (dir, lasts) in dirs {
                let (stems, endings) = sibling_parts(&lasts, range);
                let at_dir = NameHasher::default().with(dir.as_bytes());
                for stem in &stems {
                    let at_stem = at_dir.with(stem.as_bytes());
                    for end in &endings {
                        if budget == 0 {
                            self.siblings_capped.insert(at);
                            break 'table;
                        }
                        budget -= 1;
                        if self.unresolved(at_stem.with(end.as_bytes()).finish()) {
                            self.try_entry(&format!("{dir}{stem}{end}"));
                        }
                    }
                }
            }
        }
        self.source = was;
    }

    /// The rules over the names resolved, `extra` and those found, then over what each round
    /// found, until a round names nothing or [`ROUNDS`] have run.
    pub(super) fn rules(&mut self, extra: impl IntoIterator<Item = String>, range: u32) {
        self.source = Source::Rules;
        let mut work: Vec<String> = self.resolved.clone();
        work.extend(extra);
        work.extend(self.order.iter().cloned());
        let exts = work.iter().filter_map(|name| {
            let ext = &name[stem_of(name).len()..];
            (2..=8)
                .contains(&ext.len())
                .then(|| ext.to_ascii_lowercase())
        });
        let exts = ranked(counted(exts), 1);
        let suffixes = known_suffixes(&work);
        let mut expanded = HashSet::new();
        for _ in 0..ROUNDS {
            let mark = self.order.len();
            // Each round, since a name the other rules found gives a directory new siblings.
            self.sibling_rules(range);
            for name in &work {
                if expanded.insert(name.clone()) {
                    rule_candidates(name, &exts, &suffixes, range, &mut |c| self.try_name(c));
                }
            }
            self.nested_rules();
            self.twin_rules();
            self.head_rules();
            work = self.order[mark..].to_vec();
            if work.is_empty() {
                break;
            }
        }
        self.source = Source::Install;
    }
}

fn counted(items: impl Iterator<Item = String>) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        *counts.entry(item).or_default() += 1;
    }
    counts
}

/// The [`RULE_TOP`] commonest items of `counts` seen at least `least` times.
fn ranked(counts: HashMap<String, usize>, least: usize) -> Vec<String> {
    let mut ranked: Vec<(String, usize)> =
        counts.into_iter().filter(|(_, n)| *n >= least).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(RULE_TOP)
        .map(|(item, _)| item)
        .collect()
}

/// The `_xxx` endings that turn one stem of `names` into another, seen twice or more.
pub(super) fn known_suffixes(names: &[String]) -> Vec<String> {
    let mut stems: Vec<&str> = names.iter().map(|n| stem_of(n)).collect();
    stems.sort_unstable();
    stems.dedup();
    let mut endings = Vec::new();
    for (i, a) in stems.iter().enumerate() {
        for b in stems[i + 1..]
            .iter()
            .take(32)
            .take_while(|b| b.starts_with(a))
        {
            let rest = &b[a.len()..];
            if (2..=12).contains(&rest.len()) && rest.starts_with('_') && !rest.contains('/') {
                endings.push(rest.to_string());
            }
        }
    }
    ranked(counted(endings.into_iter()), 2)
}

/// What the rules make of `name`: its stem with each extension of `exts`, its stem without a suffix
/// of `suffixes` and with each, and where the stem's last component holds digits, its last run of
/// digits as each number below `range` that many digits wide, with the name's own extension and without.
pub(super) fn rule_candidates(
    name: &str,
    exts: &[String],
    suffixes: &[String],
    range: u32,
    each: &mut impl FnMut(&str),
) {
    let stem = stem_of(name);
    let ext = &name[stem.len()..];
    for other in exts {
        each(&format!("{stem}{other}"));
    }
    let base = (suffixes.iter().find(|s| stem.ends_with(s.as_str())))
        .map_or(stem, |s| &stem[..stem.len() - s.len()]);
    each(base);
    for suffix in suffixes {
        each(&format!("{base}{suffix}"));
        if !ext.is_empty() {
            each(&format!("{base}{suffix}{ext}"));
        }
    }
    let from = stem.rfind('/').map_or(0, |at| at + 1);
    let Some(last) = stem[from..].bytes().rposition(|b| b.is_ascii_digit()) else {
        return;
    };
    let end = from + last + 1;
    let digits = stem[..end]
        .bytes()
        .rev()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits > 4 {
        return;
    }
    let (head, tail) = (&stem[..end - digits], &stem[end..]);
    for n in 0..range.min(10u32.pow(digits as u32)) {
        let numbered = format!("{head}{n:0digits$}{tail}");
        each(&numbered);
        if !ext.is_empty() {
            each(&format!("{numbered}{ext}"));
        }
    }
}

/// The `_` prefixes of the last components of `names` that two of them share, or every one where
/// there is one name, case-folded.
pub(super) fn shared_stems<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let (mut counts, mut named) = (HashMap::<String, usize>::new(), 0);
    for name in names {
        named += 1;
        let name = name.to_ascii_lowercase();
        let stem = stem_of(&name);
        let last = &stem[stem.rfind('/').map_or(0, |at| at + 1)..];
        for (at, _) in last.match_indices('_').chain([(last.len(), "")]) {
            *counts.entry(last[..at].to_string()).or_default() += 1;
        }
    }
    let mut stems: Vec<String> = (counts.into_iter())
        .filter(|(stem, n)| !stem.is_empty() && (named == 1 || *n >= 2))
        .map(|(stem, _)| stem)
        .collect();
    stems.sort_unstable();
    stems
}

/// The stems the last components `lasts` of one directory share, and the bare stem then each
/// ending those components add to a stem, the commonest first, with each renumbering of its last
/// run of digits below `range`, then a `_` and each of [`WIDE_NUMBERS`].
pub(super) fn sibling_parts(lasts: &[String], range: u32) -> (Vec<String>, Vec<String>) {
    let stems = shared_stems(lasts.iter().map(String::as_str));
    let seen = lasts.iter().flat_map(|last| {
        (stems.iter())
            .filter_map(move |stem| last.strip_prefix(stem.as_str()))
            .filter(|end| end.len() <= MAX_ENDING && end.starts_with(['_', '.']))
            .map(str::to_owned)
    });
    let (mut endings, mut held) = (vec![String::new()], HashSet::from([String::new()]));
    for end in ranked(counted(seen), 1) {
        if held.insert(end.clone()) {
            endings.push(end.clone());
        }
        // The rules' numbering of a name, given the ending alone.
        rule_candidates(&end, &[], &[], range, &mut |each| {
            if held.insert(each.to_string()) {
                endings.push(each.to_string());
            }
        });
    }
    for n in WIDE_NUMBERS {
        let end = format!("_{n}");
        if held.insert(end.clone()) {
            endings.push(end);
        }
    }
    (stems, endings)
}

/// The last component of `dir`, a directory that ends in `/`.
pub(super) fn dir_stem(dir: &str) -> Option<&str> {
    let dir = dir.strip_suffix('/')?;
    Some(&dir[dir.rfind('/').map_or(0, |at| at + 1)..]).filter(|last| !last.is_empty())
}

/// Every `_` followed by one to [`SHORT_ENDING`] lower-case letters or digits, the shortest first.
pub(super) fn short_endings() -> Vec<String> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let (mut endings, mut longest) = (Vec::new(), vec!["_".to_string()]);
    for _ in 0..SHORT_ENDING {
        longest = (longest.iter())
            .flat_map(|head| {
                ALPHABET
                    .iter()
                    .map(move |&b| format!("{head}{}", char::from(b)))
            })
            .collect();
        endings.extend(longest.iter().cloned());
    }
    endings
}

/// Every one to [`SHORT_ENDING`] lower-case letters or digits followed by an `@`, then by a `_`,
/// the shortest first.
pub(super) fn short_heads() -> Vec<String> {
    let endings = short_endings();
    (['@', '_'].iter())
        .flat_map(|tail| {
            endings
                .iter()
                .map(move |end| format!("{}{tail}", &end[1..]))
        })
        .collect()
}
