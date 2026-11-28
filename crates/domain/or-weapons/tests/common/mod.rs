//! What the suites that measure the weapon family on a mounted install share: the mount, the weapon files, and a reading of two XML texts that says what the second lost of the first.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_title::game::Game;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};

/// The install mounted with every pack its content list names, none when it cannot be.
pub fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

/// Every `.meta` of the install with `weapon` in its name, by the logical path it is mounted at, and its bytes.
pub fn weapon_files(game: &MountedGame) -> Vec<(String, Vec<u8>)> {
    let mut named = Vec::new();
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".meta") && leaf.contains("weapon") {
            named.push(logical.to_string());
        }
    });
    named
        .into_iter()
        .filter_map(|logical| game.vfs.open(&logical).map(|bytes| (logical, bytes)))
        .collect()
}

/// The name of the root element of an XML text, none for anything else.
pub fn root_of(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>")? + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + 3..];
        } else {
            break;
        }
    }
    let tag = rest.strip_prefix('<')?;
    let end = tag.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    Some(tag[..end].to_string())
}

/// One element of an XML text as it states it.
struct Element {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    kids: Vec<Element>,
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `text` as the tree of its elements; the declaration and the comments are left out.
fn read(text: &str) -> Element {
    let mut stack = vec![Element {
        tag: String::new(),
        attrs: Vec::new(),
        text: String::new(),
        kids: Vec::new(),
    }];
    let mut rest = text.trim_start_matches('\u{feff}');
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            stack.last_mut().unwrap().text.push_str(&unescape(rest));
            break;
        };
        stack
            .last_mut()
            .unwrap()
            .text
            .push_str(&unescape(&rest[..open]));
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>").expect("a declaration ends") + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->").expect("a comment ends") + 3..];
        } else if let Some(after) = rest.strip_prefix("</") {
            rest = &after[after.find('>').expect("a tag ends") + 1..];
            let done = stack.pop().expect("an end tag closes an element");
            stack.last_mut().unwrap().kids.push(done);
        } else {
            let end = rest.find('>').expect("a tag ends");
            let inner = &rest[1..end];
            rest = &rest[end + 1..];
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(inner) => (inner, true),
                None => (inner, false),
            };
            let tag_end = inner
                .find(|c: char| c.is_whitespace())
                .unwrap_or(inner.len());
            let mut element = Element {
                tag: inner[..tag_end].to_string(),
                attrs: Vec::new(),
                text: String::new(),
                kids: Vec::new(),
            };
            let mut attrs = inner[tag_end..].trim();
            while let Some(eq) = attrs.find('=') {
                let name = attrs[..eq].trim().to_string();
                let quote = attrs[eq + 1..].trim_start();
                let delimiter = quote.chars().next().expect("an attribute has a value");
                let value_end = quote[1..].find(delimiter).expect("a value ends") + 1;
                element.attrs.push((name, unescape(&quote[1..value_end])));
                attrs = quote[value_end + 1..].trim_start();
            }
            if empty {
                stack.last_mut().unwrap().kids.push(element);
            } else {
                stack.push(element);
            }
        }
    }
    stack.swap_remove(0).kids.swap_remove(0)
}

/// A value as a number when it is one, so `25` and `25.000000` are the same value.
fn normal(value: &str) -> String {
    let value = value.trim();
    match value.parse::<f64>() {
        Ok(number) if number.is_finite() => format!("n{}", (number as f32).to_bits()),
        _ => format!("s{value}"),
    }
}

/// Every element of the tree, one line each: its place, its attributes and its text.
fn flatten(element: &Element, place: &str, into: &mut Vec<String>) {
    let mut attrs: Vec<String> = element
        .attrs
        .iter()
        .map(|(name, value)| format!("{name}={}", normal(value)))
        .collect();
    attrs.sort();
    let text = element.text.trim();
    into.push(format!(
        "{place} [{}] {}",
        attrs.join(" "),
        if text.is_empty() { "" } else { text }
    ));
    let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
    for kid in &element.kids {
        let at = counted.entry(kid.tag.as_str()).or_default();
        flatten(kid, &format!("{place}/{}[{at}]", kid.tag), into);
        *at += 1;
    }
}

/// The elements `before` states that `after` does not, and the elements `after` states that `before` does not, each as a line.
pub fn lost(before: &[u8], after: &[u8]) -> Vec<String> {
    let lines = |bytes: &[u8]| {
        let mut lines = Vec::new();
        let tree = read(&String::from_utf8_lossy(bytes));
        flatten(&tree, &tree.tag, &mut lines);
        lines
    };
    let (mut was, mut is) = (lines(before), lines(after));
    was.sort();
    is.sort();
    let mut differences = Vec::new();
    let (mut a, mut b) = (0, 0);
    while a < was.len() || b < is.len() {
        match (was.get(a), is.get(b)) {
            (Some(x), Some(y)) if x == y => {
                a += 1;
                b += 1;
            }
            (Some(x), Some(y)) if x < y => {
                differences.push(format!("-{x}"));
                a += 1;
            }
            (Some(_), Some(y)) => {
                differences.push(format!("+{y}"));
                b += 1;
            }
            (Some(x), None) => {
                differences.push(format!("-{x}"));
                a += 1;
            }
            (None, Some(y)) => {
                differences.push(format!("+{y}"));
                b += 1;
            }
            (None, None) => break,
        }
    }
    differences
}
