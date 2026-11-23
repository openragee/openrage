//! GTA VI's dialogue lines. Conversation data states a dialogue key and one of its line labels
//! as two NUL-terminated names in a row. The n-th distinct label of a key in a body is spoken
//! by the audio stream named by the key, `_` and n in two digits, and a label's hash is its key
//! in the text tables, which joins a line's audio to its subtitle in each table.
//!
//! The pairs are found by their shape in a body's bytes, not through the body's structure. A
//! line's number is stated nowhere: a key that keeps one take of several has a stream whose
//! number the labels do not give, and [`Dialogue::settle`] takes it from the streams that
//! exist. A text table's language is `text_language`'s to tell.

use std::collections::{BTreeMap, BTreeSet};

use super::hash::name_hash32;
use super::text::{TextEntry, TextItem, TextTable};

/// One line of dialogue: its key, its place among the key's labels of its body from 1, and its
/// label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogueLine {
    pub key: String,
    pub number: usize,
    pub label: String,
}

impl DialogueLine {
    /// The name of the audio stream that speaks the line.
    pub fn wave(&self) -> String {
        format!("{}_{:02}", self.key, self.number)
    }

    /// The audio container stream the line's wave is: its name hash's low 29 bits.
    pub fn stream_id(&self) -> u32 {
        name_hash32(self.wave().as_bytes()) & 0x1FFF_FFFF
    }

    /// The label's key in a text table.
    pub fn label_hash(&self) -> u32 {
        name_hash32(self.label.as_bytes())
    }

    /// The speaker's initials, where the label ends with one or two capitals, `_` and a number.
    // No field states a speaker. Three capitals there are a scene's word more often than initials.
    pub fn speaker(&self) -> Option<&str> {
        let mut parts = self.label.rsplit('_');
        let number = parts.next()?;
        let code = parts.next()?;
        let numbered = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
        let coded = (1..=2).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_uppercase());
        (numbered && coded).then_some(code)
    }

    /// The line's text in `table`, where the table holds its label.
    pub fn subtitle<'a>(&self, table: &'a TextTable) -> Option<&'a str> {
        table.find(self.label_hash()).map(first_text)
    }

    /// The comments an exported stream of the line carries: its name, its label, its speaker
    /// where the label names one, and `subtitle` without its markup where one is given.
    pub fn comments(&self, subtitle: Option<&str>) -> Vec<(String, String)> {
        let mut out = vec![
            ("TITLE".to_string(), self.wave()),
            ("LABEL".to_string(), self.label.clone()),
        ];
        if let Some(speaker) = self.speaker() {
            out.push(("SPEAKER".to_string(), speaker.to_string()));
        }
        if let Some(text) = subtitle {
            out.push(("SUBTITLE".to_string(), plain(text)));
        }
        out
    }
}

/// An entry's text, the first form of one with grammatical variants.
fn first_text(entry: &TextEntry) -> &str {
    match &entry.item {
        TextItem::Text(text) => text,
        TextItem::Variants(v) => v.forms.first().map_or("", |form| form.text.as_str()),
    }
}

/// The texts of `table` whose label's hash is one of `wanted`, each with that hash.
pub fn subtitles_in<'a>(
    table: &'a TextTable,
    wanted: &'a BTreeSet<u32>,
) -> impl Iterator<Item = (u32, &'a str)> {
    (table.entries.iter())
        .filter(move |e| wanted.contains(&e.key))
        .map(|e| (e.key, first_text(e)))
}

/// `text` without its `~...~` markup; a line break's token becomes a space.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('~') {
        let Some(len) = rest[open + 1..].find('~') else {
            break;
        };
        out.push_str(&rest[..open]);
        if &rest[open + 1..open + 1 + len] == "n" {
            out.push(' ');
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    out
}

fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether `name` has a dialogue key's shape: a root, `AU_`, then four capitals or digits.
fn is_key(name: &[u8]) -> bool {
    let Some((root, tail)) = name.split_last_chunk::<7>() else {
        return false;
    };
    let coded = |b: &u8| b.is_ascii_uppercase() || b.is_ascii_digit();
    !root.is_empty() && tail[..3] == *b"AU_" && tail[3..].iter().all(coded)
}

/// Each dialogue key of `body` with the label after it, in the order the body states them.
// A key is the name that ends a NUL-terminated run of bytes; its label is the whole of the
// next run, a name that is no key and that a NUL ends too.
pub fn pairs(body: &[u8]) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let (mut before, mut current): (Option<&[u8]>, Option<&[u8]>) = (None, None);
    for next in body.split(|b| *b == 0) {
        // A run is known to end with a NUL once another follows it.
        if let (Some(run), Some(label)) = (before, current) {
            let start = run.iter().rposition(|b| !is_name(*b)).map_or(0, |i| i + 1);
            let key = &run[start..];
            let labelled = !label.is_empty() && label.iter().all(|b| is_name(*b));
            if is_key(key) && labelled && !is_key(label) {
                if let (Ok(key), Ok(label)) = (std::str::from_utf8(key), std::str::from_utf8(label))
                {
                    out.push((key, label));
                }
            }
        }
        (before, current) = (current, Some(next));
    }
    out
}

/// The dialogue keys found so far, each with its distinct labels in the order they were found
/// and the number each has in the body that first stated it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialogue {
    keys: BTreeMap<String, Vec<(usize, String)>>,
}

impl Dialogue {
    /// Adds the pairs of `body`, and says how many it held. A key's labels number from 1 in each
    /// body, in the order the body first states them; a label a body added before keeps its number.
    pub fn add(&mut self, body: &[u8]) -> usize {
        let found = pairs(body);
        let mut stated: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for &(key, label) in &found {
            let labels = stated.entry(key).or_default();
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        for (key, labels) in stated {
            let kept = self.keys.entry(key.to_string()).or_default();
            for (i, label) in labels.into_iter().enumerate() {
                if !kept.iter().any(|(_, l)| l == label) {
                    kept.push((i + 1, label.to_string()));
                }
            }
        }
        found.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Renumbers the lines of each key that `held` holds no stream for under a number its
    /// labels state, where it holds as many streams of the key, numbered 00 to 99, as the key
    /// has labels: the labels take those streams' numbers in order. Says how many lines changed.
    // A key that keeps one take of several is the case: one label, and one stream of any number.
    pub fn settle(&mut self, held: impl Fn(u32) -> bool) -> usize {
        let stream = |key: &str, number: usize| {
            name_hash32(format!("{key}_{number:02}").as_bytes()) & 0x1FFF_FFFF
        };
        let mut changed = 0;
        for (key, labels) in &mut self.keys {
            if labels.iter().all(|(number, _)| held(stream(key, *number))) {
                continue;
            }
            let numbers: Vec<usize> = (0..100).filter(|n| held(stream(key, *n))).collect();
            if numbers.len() != labels.len() {
                continue;
            }
            // A stable sort: labels two bodies numbered alike stay in the order they were found.
            labels.sort_by_key(|(number, _)| *number);
            for ((number, _), now) in labels.iter_mut().zip(numbers) {
                changed += usize::from(*number != now);
                *number = now;
            }
        }
        changed
    }

    /// Every line, key by key and in each key's label order.
    pub fn lines(&self) -> impl Iterator<Item = DialogueLine> + '_ {
        self.keys.iter().flat_map(|(key, labels)| {
            labels.iter().map(move |(number, label)| DialogueLine {
                key: key.clone(),
                number: *number,
                label: label.clone(),
            })
        })
    }
}

#[cfg(test)]
#[path = "dialogue_tests.rs"]
mod tests;
