//! A GTA VI data XML edited in its form, saved as the text it was read from with only the bytes of
//! each edited value replaced: its comments, declaration, spellings and line endings stay.

use std::ops::Range;

use or_io::meta::instance::parser::{parse_typed_or_tree, spanned_root, SpannedElement};
use or_io::meta::{normalize_member_name, MetaInstance, SchemaLibrary, SourceFormat, Value};

use crate::state::MetaDocument;

/// The bytes `meta`'s form edits save as when it is a GTA VI data XML whose every edit is a value:
/// its source text with those values spliced in; `None` where the text is written again.
pub(super) fn spliced(meta: &MetaDocument) -> Option<Vec<u8>> {
    let xml = meta.gta6_library && meta.source_format == SourceFormat::Xml;
    // Text typed and not read yet is the modder's own, saved as typed.
    if !xml || !meta.is_edited() || meta.parsed_at_text_revision < meta.text_revision {
        return None;
    }
    let edited = &meta.parsed.as_ref()?.instance;
    let typed = meta.text.encode() != meta.raw_bytes
        && or_io::meta::write_meta(edited).ok() != Some(meta.text.text.to_string());
    if typed {
        return None;
    }
    match splice(meta.schema(), &meta.raw_bytes, edited) {
        Ok(bytes) => Some(bytes),
        Err(why) => {
            tracing::warn!(
                %why,
                "the metadata XML is written again from its fields, so a GTA VI archive saved with \
                 it is rebuilt, which the game refuses because only its maker can sign the table"
            );
            None
        }
    }
}

/// `original` with each value `edited` changes written over that value's bytes and every other
/// byte kept; refused, naming why, where an element is added, removed or moved, or the text has
/// no place for a new value.
fn splice(
    library: &SchemaLibrary,
    original: &[u8],
    edited: &MetaInstance,
) -> Result<Vec<u8>, String> {
    let (read, _) = parse_typed_or_tree(library, original).map_err(|e| e.to_string())?;
    let root = spanned_root(original).map_err(|e| e.to_string())?;
    let mut splice = Splice {
        src: original,
        edits: Vec::new(),
    };
    splice.instance(&read, edited, &root)?;
    splice.edits.sort_by_key(|(at, _)| at.start);
    let mut out = Vec::with_capacity(original.len());
    let mut from = 0;
    for (at, text) in &splice.edits {
        if at.start < from {
            return Err("two edited values share their bytes".into());
        }
        out.extend_from_slice(&original[from..at.start]);
        out.extend_from_slice(text.as_bytes());
        from = at.end;
    }
    out.extend_from_slice(&original[from..]);
    // The rules above follow the reader's; reading the result back is what proves them.
    let (again, _) = parse_typed_or_tree(library, &out).map_err(|e| e.to_string())?;
    if again != *edited {
        return Err("the text with the new values does not read back as the edit".into());
    }
    Ok(out)
}

struct Splice<'a> {
    src: &'a [u8],
    edits: Vec<(Range<usize>, String)>,
}

fn moved(at: &SpannedElement) -> String {
    format!("<{}> has an element added, removed or moved", at.tag)
}

fn unplaced(at: &SpannedElement) -> String {
    format!("<{}> has no text to hold its new value", at.tag)
}

impl Splice<'_> {
    fn instance(
        &mut self,
        read: &MetaInstance,
        edited: &MetaInstance,
        at: &SpannedElement,
    ) -> Result<(), String> {
        let names = |i: &MetaInstance| i.fields.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        if read.type_name != edited.type_name
            || read.extras != edited.extras
            || names(read) != names(edited)
        {
            return Err(moved(at));
        }
        for (i, ((name, was), (_, now))) in read.fields.iter().zip(&edited.fields).enumerate() {
            if was == now {
                continue;
            }
            // The reader takes a structure's attribute members first, then its elements in order.
            let nth = read.fields[..i].iter().filter(|(n, _)| n == name).count();
            let named = |key: &str| key == name.as_str() || normalize_member_name(key) == *name;
            let attrs: Vec<_> = at.attrs.iter().filter(|(key, _)| named(key)).collect();
            if let Some((_, span)) = attrs.get(nth) {
                let text = parts(now).ok_or_else(|| unplaced(at))?.join(" ");
                self.put(span.clone(), &text, true);
                continue;
            }
            // An element naming an item by `ref` is kept whole by the reader, not read as a member.
            let child = (at.children.iter())
                .filter(|c| named(&c.tag) && !c.attrs.iter().any(|(k, _)| k == "ref"))
                .nth(nth - attrs.len())
                .ok_or_else(|| moved(at))?;
            self.value(was, now, child)?;
        }
        Ok(())
    }

    fn value(&mut self, was: &Value, now: &Value, at: &SpannedElement) -> Result<(), String> {
        match (was, now) {
            (Value::Struct(was), Value::Struct(now))
            | (Value::Pointer(Some(was)), Value::Pointer(Some(now))) => self.instance(was, now, at),
            (Value::Array(was), Value::Array(now)) if was.len() == now.len() => {
                // A byte array's items are its text, which no element splits.
                if at.children.len() != was.len() {
                    return Err(unplaced(at));
                }
                for ((was, now), at) in was.iter().zip(now).zip(&at.children) {
                    if was != now {
                        self.value(was, now, at)?;
                    }
                }
                Ok(())
            }
            (Value::Map(was), Value::Map(now)) if was.len() == now.len() => {
                // One `<Item key="…">` per entry, else a key element then a value element.
                let keyed = at.children.len() == was.len();
                if !keyed && at.children.len() != 2 * was.len() {
                    return Err(unplaced(at));
                }
                for (i, ((was_key, was), (now_key, now))) in was.iter().zip(now).enumerate() {
                    let (key_at, value_at) = if keyed {
                        (&at.children[i], &at.children[i])
                    } else {
                        (&at.children[2 * i], &at.children[2 * i + 1])
                    };
                    if was_key != now_key && keyed {
                        let span = (key_at.attrs.iter()).find(|(k, _)| k == "key");
                        let (_, span) = span.ok_or_else(|| unplaced(key_at))?;
                        let text = parts(now_key).ok_or_else(|| unplaced(key_at))?.join(" ");
                        self.put(span.clone(), &text, true);
                    } else if was_key != now_key {
                        self.value(was_key, now_key, key_at)?;
                    }
                    if was != now {
                        self.value(was, now, value_at)?;
                    }
                }
                Ok(())
            }
            (Value::Array(_), Value::Array(_))
            | (Value::Map(_), Value::Map(_))
            | (Value::Pointer(_), Value::Pointer(_)) => Err(moved(at)),
            _ if std::mem::discriminant(was) == std::mem::discriminant(now) => {
                self.leaf(was, now, at)
            }
            _ => Err(moved(at)),
        }
    }

    /// A value read from an element: from its text where that is not blank, else its `value`
    /// attribute, else one attribute per component, as the reader takes it.
    fn leaf(&mut self, was: &Value, now: &Value, at: &SpannedElement) -> Result<(), String> {
        // Only a number keeps the form it was written in; any other text is put as it now reads.
        let numeric = matches!(
            now,
            Value::Float(_) | Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_) | Value::Mat34(_)
        );
        let (Some(was), Some(now)) = (parts(was), parts(now)) else {
            return Err(unplaced(at));
        };
        if !at.children.is_empty() || was.len() != now.len() {
            return Err(unplaced(at));
        }
        let (body, src) = (at.content.clone(), self.src);
        let text = |span: &Range<usize>| std::str::from_utf8(&src[span.clone()]).ok();
        if let Some(span) = body
            .clone()
            .filter(|s| text(s).is_some_and(|t| !t.trim().is_empty()))
        {
            if numeric && self.put_tokens(span.clone(), &was, &now, false) {
                return Ok(());
            }
            // The whole body where it is the value as written, else the value inside its blanks.
            let whole = text(&span) == Some(was.join(" ").as_str());
            let span = if whole { span } else { self.trimmed(span) };
            self.put(span, &now.join(" "), false);
            return Ok(());
        }
        if let Some((_, span)) = at.attrs.iter().find(|(k, _)| k == "value") {
            if !(numeric && self.put_tokens(span.clone(), &was, &now, true)) {
                self.put(span.clone(), &now.join(" "), true);
            }
            return Ok(());
        }
        let names: &[&str] = match was.len() {
            1 => &[],
            _ if at.attrs.iter().any(|(k, _)| k == "r") => &["r", "g", "b", "a"],
            _ => &["x", "y", "z", "w"],
        };
        if names.is_empty() || !at.attrs.iter().any(|(k, _)| names.contains(&k.as_str())) {
            let span = body.ok_or_else(|| unplaced(at))?;
            self.put(span, &now.join(" "), false);
            return Ok(());
        }
        for ((was, now), name) in was.iter().zip(&now).zip(names) {
            if was != now {
                let span = (at.attrs.iter()).find(|(k, _)| k == name);
                let (_, span) = span.ok_or_else(|| unplaced(at))?;
                let one = |s: &String| std::slice::from_ref(s).to_vec();
                if !(numeric && self.put_tokens(span.clone(), &one(was), &one(now), true)) {
                    self.put(span.clone(), now, true);
                }
            }
        }
        Ok(())
    }

    /// Splice each changed component over its own token of `span`, written in that token's form;
    /// false, nothing put, where the span does not hold one token per component.
    fn put_tokens(
        &mut self,
        span: Range<usize>,
        was: &[String],
        now: &[String],
        attribute: bool,
    ) -> bool {
        let separator = |b: &u8| b.is_ascii_whitespace() || *b == b',';
        let mut tokens = Vec::new();
        let mut at = span.start;
        while at < span.end {
            let start = at
                + (self.src[at..span.end].iter())
                    .take_while(|b| separator(b))
                    .count();
            let end = start
                + (self.src[start..span.end].iter())
                    .take_while(|b| !separator(b))
                    .count();
            if start < end {
                tokens.push(start..end);
            }
            at = end;
        }
        if tokens.len() != was.len() || was.len() != now.len() {
            return false;
        }
        for ((was, now), token) in was.iter().zip(now).zip(tokens) {
            if was != now {
                let original = std::str::from_utf8(&self.src[token.clone()]).unwrap_or_default();
                let text = in_the_form_of(original, now);
                self.put(token, &text, attribute);
            }
        }
        true
    }

    fn trimmed(&self, span: Range<usize>) -> Range<usize> {
        let bytes = &self.src[span.clone()];
        let lead = bytes.iter().take_while(|b| b.is_ascii_whitespace()).count();
        let trail = (bytes[lead..].iter().rev())
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        span.start + lead..span.end - trail
    }

    fn put(&mut self, span: Range<usize>, text: &str, attribute: bool) {
        let mut escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        if attribute {
            escaped = escaped.replace('"', "&quot;");
        }
        self.edits.push((span, escaped));
    }
}

/// `now` with as many decimals as the number `original` was written with, so a value of the same
/// length keeps its length; as it is where `original` is no decimal or `now` no number. The read
/// back the splice ends on refuses a form that loses the value.
fn in_the_form_of(original: &str, now: &str) -> String {
    let decimals = (original.split_once('.'))
        .filter(|_| !original.contains(['e', 'E']) && original.parse::<f64>().is_ok())
        .map(|(_, fraction)| fraction.len());
    match (decimals, now.parse::<f64>()) {
        (Some(n), Ok(v)) => format!("{v:.n$}"),
        _ => now.to_owned(),
    }
}

/// A value as the text of each of its components; `None` for one that holds elements.
fn parts(value: &Value) -> Option<Vec<String>> {
    // A number with no point would read back as an integer where no schema types it.
    fn float(v: impl std::fmt::Display) -> String {
        let s = v.to_string();
        if s.contains(['.', 'e', 'E', 'N', 'i']) {
            s
        } else {
            s + ".0"
        }
    }
    let one = |s: String| Some(vec![s]);
    match value {
        Value::Bool(b) => one(b.to_string()),
        Value::Int(v) => one(v.to_string()),
        Value::UInt(v) => one(v.to_string()),
        Value::Float(v) => one(float(v)),
        Value::String(s) | Value::Raw(s) => one(s.clone()),
        Value::Hash(h) => one(h.to_xml_text()),
        Value::Enum { variant, .. } => one(variant.clone()),
        Value::Bitset { bits, .. } => one(format!("0x{bits:X}")),
        Value::Vec2(v) => Some(v.iter().map(float).collect()),
        Value::Vec3(v) => Some(v.iter().map(float).collect()),
        Value::Vec4(v) => Some(v.iter().map(float).collect()),
        Value::Mat34(m) => Some(m.iter().flatten().map(float).collect()),
        Value::Color(c) => Some(
            [16, 8, 0, 24]
                .map(|at| ((c >> at) & 0xFF).to_string())
                .into(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use or_io::meta::instance::parser::parse_typed_or_tree;
    use or_io::meta::instance::HashedString;
    use or_io::meta::par_tree::library_from_par_tree;
    use or_io::meta::{SchemaLibrary, Value};

    use super::splice;

    /// A made-up dump: a root with a hash string, an enum, a u32, a float and an array of u32s.
    fn library() -> SchemaLibrary {
        let json = r#"{"structs":[
          {"name":"CZqChangeSet","markup":"$struct$ =@CZqChangeSet@\r\n{\r\n\t$string$ id;\r\n\t$enum$ @ZqFlag@ flags;\r\n\t$uint$ count;\r\n\t$float$ weight;\r\n\t$array$<$uint$> items;\r\n};\r\n"}
        ],"enums":[
          {"name":"ZqFlag","markup":"$enum$ =@ZqFlag@\r\n{\r\n\tZqDefault = 1,\r\n\tZqPersistent = 2,\r\n};\r\n"}
        ]}"#;
        library_from_par_tree(json.as_bytes()).expect("the dump reads")
    }

    /// A comment, a standalone declaration, CRLF, an enum by name, an element the library lacks
    /// between two members, and an empty element written open and closed.
    const XML: &str = concat!(
        "<?xml version=\"1.0\" encoding=\"utf-8\" standalone=\"yes\"?>\r\n",
        "<!-- zq hand edit -->\r\n",
        "<CZqChangeSet>\r\n",
        "  <id>zq_core</id>\r\n",
        "  <zqUnheard></zqUnheard>\r\n",
        "  <flags>ZqPersistent</flags>\r\n",
        "  <count value=\"12\"/>\r\n",
        "  <weight value=\"0.5\"/>\r\n",
        "  <items>\r\n    <Item value=\"7\"/>\r\n    <Item value=\"9\"/>\r\n  </items>\r\n",
        "</CZqChangeSet>\r\n",
    );

    fn read() -> or_io::meta::MetaInstance {
        parse_typed_or_tree(&library(), XML.as_bytes())
            .expect("it reads")
            .0
    }

    fn edited(name: &str, value: Value) -> Vec<u8> {
        let mut inst = read();
        inst.set(name, value);
        splice(&library(), XML.as_bytes(), &inst).expect("it splices")
    }

    /// The offsets at which `a` and `b`, of one length, differ.
    fn differing(a: &[u8], b: &[u8]) -> Vec<usize> {
        assert_eq!(a.len(), b.len());
        (0..a.len()).filter(|&i| a[i] != b[i]).collect()
    }

    #[test]
    fn a_number_edited_at_its_length_changes_only_its_own_bytes() {
        let out = edited("count", Value::UInt(34));
        let at = XML.find("\"12\"").expect("the value") + 1;
        assert_eq!(differing(XML.as_bytes(), &out), [at, at + 1]);
        // The writer's form would have lost the comment, the declaration and the enum's name.
        let rewritten = or_io::meta::write_meta(&read()).expect("it writes");
        assert!(!rewritten.contains("standalone") && !rewritten.contains("\r\n"));
    }

    #[test]
    fn every_kind_of_value_is_spliced_where_it_was_read() {
        let enum_ = Value::Enum {
            type_name: "ZqFlag".into(),
            variant: "ZqDefault".into(),
        };
        let out = edited("flags", enum_);
        let want = XML.replace(">ZqPersistent<", ">ZqDefault<");
        assert_eq!(String::from_utf8(out).expect("text"), want);
        let out = edited("id", Value::Hash(HashedString::new("zq_anim")));
        assert_eq!(out, XML.replace("zq_core", "zq_anim").as_bytes());
        let out = edited("weight", Value::Float(2.0));
        assert_eq!(out, XML.replace("\"0.5\"", "\"2.0\"").as_bytes());
        let items = Value::Array(vec![Value::UInt(7), Value::UInt(10)]);
        let out = edited("items", items);
        assert_eq!(out, XML.replace("\"9\"", "\"10\"").as_bytes());
    }

    #[test]
    fn a_number_is_written_with_the_decimals_it_was_read_with() {
        let six = XML.replace("\"0.5\"", "\"0.500000\"");
        let mut inst = parse_typed_or_tree(&library(), six.as_bytes())
            .expect("it reads")
            .0;
        inst.set("weight", Value::Float(0.75));
        let out = splice(&library(), six.as_bytes(), &inst).expect("it splices");
        assert_eq!(out, six.replace("\"0.500000\"", "\"0.750000\"").as_bytes());
        assert_eq!(super::in_the_form_of("0.500000", "0.75"), "0.750000");
        assert_eq!(super::in_the_form_of("12", "13"), "13");
        assert_eq!(super::in_the_form_of("1e3", "2.5"), "2.5");
    }

    #[test]
    fn a_longer_value_moves_the_length_by_exactly_the_difference() {
        let out = edited("id", Value::Hash(HashedString::new("zq_core_animation")));
        assert_eq!(out.len(), XML.len() + "_animation".len());
        assert_eq!(out, XML.replace("zq_core", "zq_core_animation").as_bytes());
    }

    #[test]
    fn a_structural_edit_is_refused_and_says_so() {
        let mut inst = read();
        inst.set("items", Value::Array(vec![Value::UInt(7)]));
        let why = splice(&library(), XML.as_bytes(), &inst).expect_err("an item removed");
        assert!(why.contains("added, removed or moved"), "{why}");
        let mut inst = read();
        inst.fields.swap(0, 1);
        let why = splice(&library(), XML.as_bytes(), &inst).expect_err("two members moved");
        assert!(why.contains("added, removed or moved"), "{why}");
    }

    /// The first string a depth-first walk of `inst` finds that ends in a lowercase letter and
    /// holds no character the text escapes, with that letter changed: an edit at its length.
    fn one_letter_changed(inst: &mut or_io::meta::MetaInstance) -> bool {
        fn changed(s: &str) -> Option<String> {
            let last = s.chars().last().filter(|c| c.is_ascii_lowercase())?;
            let plain = !s.contains(['&', '<', '>', '"']);
            plain.then(|| {
                format!(
                    "{}{}",
                    &s[..s.len() - 1],
                    if last == 'a' { 'b' } else { 'a' }
                )
            })
        }
        fn value(v: &mut Value) -> bool {
            match v {
                Value::String(s) => changed(s).map(|new| *s = new).is_some(),
                Value::Hash(h) if h.to_xml_text() == h.raw => (changed(&h.raw))
                    .map(|new| *h = HashedString::new(new))
                    .is_some(),
                Value::Struct(i) | Value::Pointer(Some(i)) => one_letter_changed(i),
                Value::Array(items) => items.iter_mut().any(value),
                _ => false,
            }
        }
        inst.fields.iter_mut().any(|(_, v)| value(v))
    }

    /// Every data XML among the small rows of the build's three smallest root archives, one
    /// letter of a string changed, splices to its length with one byte different, or is refused.
    /// Install-gated: it needs the key snapshot and the build, and skips without them.
    #[test]
    fn a_data_xml_of_the_build_edited_at_its_length_changes_one_byte() {
        use or_io::container::rpf::Gta6Mount;
        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        const CAP: u64 = 0x40000;
        const DOCS: usize = 16;
        let build =
            (std::env::var_os(BUILD_ENV).map(std::path::PathBuf::from)).filter(|p| p.is_dir());
        let keys = (build.as_ref()).and_then(|_| crate::state::rpf9_scan::rpf9_table::keys().ok());
        let (Some(build), Some(keys)) = (build, keys) else {
            or_test_support::skip_install(
                "gta6 data XML splice",
                format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"),
            );
            return;
        };
        let default = crate::state::meta_schema();
        let library = or_io::meta::names_library().unwrap_or(default);
        let mut archives: Vec<(u64, std::path::PathBuf)> = (std::fs::read_dir(&build).into_iter())
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
            .collect();
        archives.sort();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let (mut spliced, mut refused) = (0usize, Vec::new());
        'walk: for (_, disk) in archives.iter().take(3) {
            let Ok(mut file) = std::fs::File::open(disk) else {
                continue;
            };
            let Ok(mount) = Gta6Mount::open_root(&mut file, &keys) else {
                continue;
            };
            for entry in &mount.archive.entries {
                if spliced + refused.len() >= DOCS || std::time::Instant::now() > deadline {
                    break 'walk;
                }
                let small = (entry.real.or(entry.stored)).is_some_and(|s| s > 0 && s <= CAP);
                let Some(body) = small
                    .then(|| mount.read_entry_decrypted(&mut file, entry, &keys).ok())
                    .flatten()
                else {
                    continue;
                };
                let head = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&body);
                if !head.trim_ascii_start().starts_with(b"<") {
                    continue;
                }
                let Ok((mut inst, _)) = parse_typed_or_tree(library, &body) else {
                    continue;
                };
                if !one_letter_changed(&mut inst) {
                    continue;
                }
                let at = format!("{} row {}", disk.display(), entry.index);
                match splice(library, &body, &inst) {
                    Ok(out) => {
                        assert_eq!(out.len(), body.len(), "{at} keeps its length");
                        let differ = (0..out.len()).filter(|&i| out[i] != body[i]).count();
                        assert_eq!(differ, 1, "{at} differs in the one letter edited");
                        spliced += 1;
                    }
                    Err(why) => refused.push(format!("{at}: {why}")),
                }
            }
        }
        assert!(
            spliced > 0,
            "no data XML of the build's smallest archives spliced; refused: {refused:?}"
        );
    }
}
