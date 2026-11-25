//! Counts a document states, recomputed rather than restated.
//!
//! [`shell_budget`](crate::shell_budget) states the principle this module
//! applies to everything that is not a line count: *a figure quoted as an
//! illustration is the one nobody updates, because nothing fails when it
//! rots.* The other half — **list the members
//! rather than count them**, so a row added on one side of a merge cannot pass
//! by leaving a total unchanged.
//!
//! # What lives here and what does not
//!
//! A count belongs here when it is *derivable and stable under counting
//! convention*. The R2 documentation sweep separated forty-eight stale claims
//! into three groups and only the first is this module's:
//!
//! - **Derivable.** Recompute it and assert the relationship the documentation's
//!   argument depends on. That is everything below, plus
//!   [`shell_budget`](crate::shell_budget) for file sizes.
//! - **Definitional.** How many places match on `DocPayload`; how many
//!   `#[record]` declarations exist; how many references a type's variants
//!   have. Every one of these gave a different answer per counting rule — one
//!   of them six answers in a single audit — so the figure is an argument
//!   dressed as a measurement. **Those are deleted from the documentation, not
//!   asserted here.** Adding one would be picking a convention and calling it a
//!   fact.
//! - **Corpus-backed.** Measured over `../openrage-corpus`, which is not
//!   committed and which CI cannot reach. Those stay prose, and name the
//!   suite and the revision they were measured at, so a reader knows the
//!   figure is dated.
//!
//! # Counts nobody brought here
//!
//! [`figures`] reads every committed document for the counts it states, and holds each one: a
//! test asserts it, its paragraph names the revision or suite it was measured at, or the ledger
//! there waives it with a reason.
//!
//! # Why the assertions are shaped the way they are
//!
//! None of them asserts a number. Each asserts the **property** a document
//! makes its argument from, because that is what has to stay true — and
//! because a test that pins a total is a second place for the total to be
//! wrong. `the_shell_is_flat_rather_than_deep` is the clearest case: six
//! different figures for `AppState`'s field count have appeared in this
//! repository's prose, and not one of them was the claim. The claim was that
//! the struct is flat.
//!
//! ```text
//! cargo test --bin openrage doc_counts
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod figures;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel))
        .unwrap_or_else(|e| panic!("{rel} is named by a document and by this test: {e}"))
}

fn body_of(text: &str, header: &str) -> String {
    let start = text
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` is not in this file any more"));
    let open = text[start..]
        .find('{')
        .unwrap_or_else(|| panic!("`{header}` has no body"))
        + start;

    let chars: Vec<char> = text[open..].chars().collect();
    let mut out = String::new();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut block = 1usize;
            i += 2;
            while i < chars.len() && block > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    block += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    block -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        if c == '"' {
            i += 1;
            while i < chars.len() {
                match chars[i] {
                    '\\' => i += 1,
                    '"' => break,
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '{' {
            depth += 1;
            i += 1;
            if depth == 1 {
                continue;
            }
        } else if c == '}' {
            depth -= 1;
            i += 1;
            if depth == 0 {
                break;
            }
        } else {
            i += 1;
        }
        out.push(c);
    }
    out
}

fn members(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in body.chars() {
        match c {
            '{' | '(' | '[' | '<' => depth += 1,
            '}' | ')' | ']' | '>' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            push_member(&current, &mut out);
            current.clear();
            continue;
        }
        current.push(c);
    }
    push_member(&current, &mut out);
    out
}

fn push_member(raw: &str, out: &mut Vec<(String, String)>) {
    let cleaned: String = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.trim().trim_start_matches("pub ").trim();
    if cleaned.is_empty() {
        return;
    }
    let name: String = cleaned
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return;
    }
    let rest = cleaned[name.len()..].trim_start_matches(':').trim();
    out.push((name, rest.to_string()));
}

fn document_kind_variants() -> Vec<String> {
    let text = read("src/state/document/kind.rs");
    members(&body_of(&text, "pub enum DocumentKind"))
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn doc_payload_variants() -> Vec<String> {
    let text = read("src/state/document/payload.rs");
    members(&body_of(&text, "pub enum DocPayload"))
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

const OPEN_KIND: &str = "Registered";

/// The one payload every owned format opens into, and the named kinds it holds.
const OWNED_PAYLOAD: &str = "Resource";
const OWNED_KINDS: &[&str] = &[
    "Ytd", "Ydr", "Ydd", "Yft", "Ycd", "Yed", "Yfd", "Ynd", "Ynv", "Ybn", "Yld", "Ypt",
];

#[test]
fn document_kind_names_these_formats_and_one_it_does_not() {
    assert_eq!(
        document_kind_variants(),
        vec![
            "Ypt",
            "Ytd",
            "Ydr",
            "Ydd",
            "Yft",
            "Ybn",
            "Yld",
            "Ycd",
            "Yed",
            "Yfd",
            "Ynd",
            "Ynv",
            "Ypdb",
            "Fxc",
            "Gxt2",
            "Bik",
            "Awc",
            "HeightMap",
            "DistantLights",
            "WaterMap",
            "WorldBlanket",
            "NavPrecalc",
            "DatTable",
            "Rel",
            "NameTable",
            "Gfx",
            "MoveNetwork",
            "Dds",
            "Ttpf",
            "Rpf",
            "Rpf9",
            "Gta6Anim",
            "Gta6Drawable",
            "Gta6Text",
            "Gta6AudioData",
            "Gta6Expressions",
            "Gta6Save",
            "Gta6Script",
            "GameRoot",
            "Folder",
            "Text",
            "Meta",
            "Hex",
            "Compare",
            OPEN_KIND,
        ],
        "`DocumentKind`'s variants are stated in docs/reference/Crate_Reference.md and \
         src/state/capabilities.rs. Update the list here and the sentence \
         there in one change — and note that several documents distinguish \
         the *named* kinds from the open `Registered` one, so say which you \
         mean."
    );
}

/// The list the tables are checked against names every variant but the open one.
///
/// ⚠ **`DocumentKind::NAMED` is the last hand-written copy of this enum, and
/// it only earns that by being checked against the declaration.** There were
/// three — `kinds::KINDS`'s pin, `capabilities::BUILTIN`'s pin and
/// `from_variant_name` — and a sixteenth variant satisfied all three, because
/// a list is what people forget to add to.
#[test]
fn the_named_kinds_are_the_variants_the_enum_declares() {
    let declared: Vec<String> = document_kind_variants()
        .into_iter()
        .filter(|v| v != OPEN_KIND)
        .collect();
    let listed: Vec<String> = crate::state::DocumentKind::NAMED
        .iter()
        .map(|kind| format!("{kind:?}"))
        .collect();

    assert_eq!(
        declared, listed,
        "`DocumentKind::NAMED` is what `kinds`, `capabilities` and \
         `document_io` are checked for completeness against, so a variant \
         missing from it is a kind with no table row, no capability row and \
         no reader that nothing else fails on."
    );
}

/// The enum is open, and exactly one variant is what opens it.
///
/// `capabilities.rs` used to say `DocumentKind` was a closed enum and that
/// this was the ceiling a runtime-registered format stopped at. The ceiling
/// lifted when `Registered` landed, and that bullet survived in four
/// documents afterwards — so the property is asserted rather than described.
#[test]
fn exactly_one_document_kind_carries_a_resource_the_enum_does_not_name() {
    let open: Vec<String> = document_kind_variants()
        .into_iter()
        .filter(|v| v == OPEN_KIND)
        .collect();
    assert_eq!(
        open,
        vec![OPEN_KIND.to_string()],
        "a format the enum does not name keys `DocumentKind::{OPEN_KIND}`. If \
         that variant has been renamed or removed, \
         `src/state/capabilities.rs` and docs/reference/Crate_Reference.md both argue \
         from its existence."
    );
}

#[test]
fn every_named_kind_has_a_payload_and_there_is_no_surplus() {
    let kinds: BTreeSet<String> = document_kind_variants()
        .into_iter()
        .filter(|v| v != OPEN_KIND)
        .collect();
    let payloads: BTreeSet<String> = doc_payload_variants().into_iter().collect();

    let without_payload: Vec<&String> = kinds
        .iter()
        .filter(|k| {
            !payloads.contains(*k)
                && *k != "GameRoot"
                && *k != "Folder"
                && !OWNED_KINDS.contains(&k.as_str())
        })
        .collect();
    assert!(
        without_payload.is_empty(),
        "these document kinds have no payload variant and are not in \
         `OWNED_KINDS`: {without_payload:?}"
    );
    let stale: Vec<&&str> = OWNED_KINDS
        .iter()
        .filter(|k| payloads.contains(**k))
        .collect();
    assert!(
        stale.is_empty(),
        "these kinds have a payload variant of their own, so `OWNED_KINDS` \
         should not list them: {stale:?}"
    );

    let surplus: Vec<&String> = payloads
        .iter()
        .filter(|p| !kinds.contains(*p) && *p != OPEN_KIND && *p != OWNED_PAYLOAD)
        .collect();
    assert!(
        surplus.is_empty(),
        "every `DocPayload` variant beyond the named kinds should be \
         `{OPEN_KIND}` or `{OWNED_PAYLOAD}`: an Enhanced file opens as the owned \
         root `or_io::Asset` names, never a clone of its Legacy payload, and \
         these are neither: {surplus:?}"
    );
}

#[test]
fn every_field_of_format_capabilities_but_the_key_is_a_cell() {
    let text = read("src/state/capabilities.rs");
    let fields = members(&body_of(&text, "pub struct FormatCapabilities"));

    assert!(
        fields.len() > 10,
        "the capability table parsed as {} field(s), which is not it — the \
         parse is wrong, not the struct",
        fields.len()
    );

    let (key, cells): (Vec<_>, Vec<_>) = fields.iter().partition(|(name, _)| name == "kind");
    assert_eq!(
        key.len(),
        1,
        "the table is keyed by exactly one field, `kind`; found {:?}",
        key
    );

    let not_a_cell: Vec<&String> = cells
        .iter()
        .filter(|(_, ty)| !ty.starts_with("Option<"))
        .map(|(name, _)| name)
        .collect();
    assert!(
        not_a_cell.is_empty(),
        "a capability cell is an `Option`, because `None` is the answer for \
         most cells for most formats and that is the table's whole argument. \
         These are not: {not_a_cell:?}"
    );

    let tax = read("docs/reference/Crate_Reference.md");
    let unnamed: Vec<&String> = cells
        .iter()
        .filter(|(name, _)| !tax.contains(&format!("`{name}`")))
        .map(|(name, _)| name)
        .collect();
    assert!(
        unnamed.is_empty(),
        "docs/reference/Crate_Reference.md enumerates the capability cells and does \
         not name these: {unnamed:?}. Add them to the list there, or stop \
         enumerating."
    );
}

fn rust_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for top in ["src", "crates"] {
        walk(&repo_root().join(top), &mut |path| {
            if path.extension().is_some_and(|e| e == "rs") {
                out.push(path.to_path_buf());
            }
        });
    }
    out.sort();
    out
}

fn walk(dir: &Path, f: &mut impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            // ⚠ `is_dir` follows links, and the ignored `corpus` is a junction
            // to the whole corpus.
            if crate::doc_citations::is_ignored_dir(&path) {
                continue;
            }
            if path.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                matches!(
                    n.as_ref(),
                    "target" | ".claude" | ".git" | "node_modules" | ".cursor"
                ) || n.starts_with('_')
            }) {
                continue;
            }
            if path.file_name().is_some_and(|n| n == "extra")
                && path
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n == "docs")
            {
                continue;
            }
            walk(&path, f);
        } else {
            f(&path);
        }
    }
}

#[test]
fn the_shell_is_flat_rather_than_deep() {
    let state = read("src/state/mod.rs");
    let fields = members(&body_of(&state, "pub struct AppState"));
    assert!(
        fields.len() > 20,
        "`AppState` parsed as {} field(s). It is the app's whole state sheet; \
         that is the parse failing, not the struct shrinking.",
        fields.len()
    );

    let mut declared: BTreeSet<String> = BTreeSet::new();
    for path in rust_sources() {
        if path.ends_with("state/mod.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for keyword in ["struct ", "enum ", "type "] {
            for (index, _) in text.match_indices(keyword) {
                let name: String = text[index + keyword.len()..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.chars().next().is_some_and(char::is_uppercase) {
                    declared.insert(name);
                }
            }
        }
    }
    assert!(
        declared.len() > 200,
        "the type index found {} declaration(s) across src/ and crates/, \
         which is too few to be this workspace",
        declared.len()
    );

    let is_handle = |ty: &str| {
        ty.split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| declared.contains(word))
    };
    let inline: Vec<&String> = fields
        .iter()
        .filter(|(_, ty)| !is_handle(ty))
        .map(|(name, _)| name)
        .collect();

    assert!(
        inline.len() * 2 < fields.len(),
        "AGENTS.md says `AppState` is deliberately flat — most fields a \
         handle to a type that already has its own module and its own \
         invariants. {} of {} fields are now a bare primitive or a type \
         declared in state/mod.rs itself: {inline:?}. Either the shape has \
         changed and that paragraph wants rewriting, or state is being kept \
         here that belongs in a module.",
        inline.len(),
        fields.len()
    );
}

/// Every `Rnn`/`Rnna` the roadmap's checkbox list names, in order.
fn roadmap_checkboxes(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line
                .trim_start()
                .strip_prefix("- [ ] [")
                .or_else(|| line.trim_start().strip_prefix("- [x] ["))?;
            roadmap_id(rest)
        })
        .collect()
}

/// Every `Rnn`/`Rnna` with a detail section of its own.
///
/// ⚠ **Any heading depth, because the document uses two and both resolve.**
/// R33's parts are `###` and R34's are `####`; a reader following either
/// anchor lands, so a check that admitted only one depth would report a dead
/// link that is not dead. It found exactly that on its first contact with new
/// work, which is the right failure for the wrong reason.
fn roadmap_sections(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("##")?.trim_start_matches('#');
            // `### R8 …`, and `### ✅ R40a …` for a part that closed.
            roadmap_id(rest.strip_prefix(' ')?.trim_start_matches(['✅', ' ']))
        })
        .collect()
}

/// `R40a The shader definition database…` → `R40a`, and nothing else is one.
fn roadmap_id(rest: &str) -> Option<String> {
    let word = rest.split_whitespace().next()?;
    let digits = word.strip_prefix('R')?;
    let number: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if number.is_empty() {
        return None;
    }
    // A part takes exactly one lowercase letter; `R33a` is an item and
    // `R33abc` or `R33-` is prose that happens to start with an R.
    let suffix = &digits[number.len()..];
    let is_a_part = suffix.len() == 1 && suffix.chars().all(|c| c.is_ascii_lowercase());
    if !suffix.is_empty() && !is_a_part {
        return None;
    }
    Some(format!("R{number}{suffix}"))
}

/// The roadmap's checkbox list and its detail sections are the same set, bar
/// programme headers — asserted because the *count* of items is written nowhere.
///
/// ⚠ **This replaced three documents stating three different totals.** The
/// roadmap said ninety-two over `R1`–`R60`, `AGENTS.md` said ninety-three over
/// `R1`–`R61`, and the file already carried `R62`; within the same evening two
/// rows were decomposed and one was added, so any literal written that day was
/// wrong by the next morning. The total is deleted rather than corrected, per
/// [the rule](../docs/guides/Documentation_Standard.md#figures), and what a reader actually needs from it
/// — that the list is a complete index of the sections, and that following a
/// checkbox lands somewhere — is a set comparison, which is stable under every
/// counting convention a total is not.
#[test]
fn the_roadmap_is_an_index_of_itself() {
    let text = read("docs/direction/Roadmap.md");
    let boxes: BTreeSet<String> = roadmap_checkboxes(&text).into_iter().collect();
    let sections: BTreeSet<String> = roadmap_sections(&text).into_iter().collect();

    assert!(
        boxes.len() > 50 && sections.len() > 50,
        "found {} checkbox(es) and {} section(s), which is not this roadmap — \
         the parse is broken rather than the document",
        boxes.len(),
        sections.len()
    );

    let dangling: Vec<&String> = boxes.difference(&sections).collect();
    assert!(
        dangling.is_empty(),
        "these checkboxes link to a detail section that is not in the file: \
         {dangling:?}"
    );

    // The other direction is not equality: a decomposed number keeps a section
    // so its anchor still resolves, and is a programme header rather than a
    // row. That is legitimate exactly when its lettered parts are scheduled.
    let mut headers_without_parts = Vec::new();
    for section in sections.difference(&boxes) {
        let has_parts = boxes
            .iter()
            .any(|b| b.len() == section.len() + 1 && b.starts_with(section.as_str()));
        if !has_parts {
            headers_without_parts.push(section);
        }
    }
    assert!(
        headers_without_parts.is_empty(),
        "these detail sections are in no checkbox list and have no lettered \
         parts in one either, so nothing schedules them: \
         {headers_without_parts:?}"
    );
}

#[test]
fn the_index_check_can_tell_the_three_shapes_apart() {
    // The sweep above reports clean, and clean is what a broken parse reports
    // too. So the three cases it rules on are put to it as text: a row with a
    // section, a row without one, and a section with no row whose parts are
    // scheduled.
    let text = "\
- [x] [R1 Landed](#r1-landed)
- [ ] [R9 Dangling](#r9-dangling)
- [ ] [R40a A part](#r40a-a-part)
  - [ ] [R33a Nested rows count too](#r33a-nested-rows-count-too)
- [ ] not a row at all
### R1 Landed
### ✅ R40a A part
### R40 A programme header
#### R33a A part one heading deeper
### R50 A header nobody scheduled
## Appendix A — not an item
";
    let boxes: BTreeSet<String> = roadmap_checkboxes(text).into_iter().collect();
    let sections: BTreeSet<String> = roadmap_sections(text).into_iter().collect();

    assert_eq!(
        boxes,
        ["R1", "R33a", "R40a", "R9"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "a nested row is a row, and a bullet with no link is not"
    );
    assert!(
        sections.contains("R40a"),
        "a `### ✅ Rnn` heading is still that item's section"
    );
    assert!(
        sections.contains("R33a"),
        "a `####` heading is a section too — the document uses both depths and \
         both anchors resolve"
    );

    let dangling: Vec<&String> = boxes.difference(&sections).collect();
    assert_eq!(
        dangling,
        vec!["R9"],
        "a checkbox whose section is missing is the case that breaks a link"
    );

    let orphans: Vec<&String> = sections
        .difference(&boxes)
        .filter(|s| {
            !boxes
                .iter()
                .any(|b| b.len() == s.len() + 1 && b.starts_with(s.as_str()))
        })
        .collect();
    assert_eq!(
        orphans,
        vec!["R50"],
        "R40 is a programme header because R40a is scheduled; R50 is not, \
         because nothing of its is"
    );
}

const R66D_TABLE_HEADER: &str =
    "| Format | Module | Targets | Declared | Owned | At a fixed point |";

/// Each root `or-io` opens owned, as its `impl File` or `impl Resource` line spells it, with the file stem declaring it.
///
/// ⚠ A generic head (`impl<T> File for T`) is a blanket over a parameter and names no root.
fn owned_roots_or_io_declares() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    walk(&repo_root().join("crates/io/or-io/src"), &mut |path| {
        if path.extension().is_none_or(|e| e != "rs") {
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        for line in text.lines().map(str::trim) {
            for trait_for in ["File for ", "Resource for "] {
                let Some(at) = line.find(trait_for) else {
                    continue;
                };
                let head = &line[..at];
                if !head.starts_with("impl ") || !(head.ends_with(' ') || head.ends_with("::")) {
                    continue;
                }
                let root = line[at + trait_for.len()..].trim_end_matches('{').trim();
                out.insert(root.to_string(), stem.clone());
            }
        }
    });
    out
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn backticked(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Where the R66d table's Owned column and the roots `or-io` owns disagree, one line each.
fn owned_column_disagreements(roadmap: &str, declared: &BTreeMap<String, String>) -> Vec<String> {
    let headers: Vec<usize> = roadmap
        .lines()
        .enumerate()
        .filter(|(_, line)| line.trim() == R66D_TABLE_HEADER)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        headers.len(),
        1,
        "the R66d table's header row must be exactly one line of the roadmap: \
         `{R66D_TABLE_HEADER}`"
    );
    let columns = table_cells(R66D_TABLE_HEADER);
    let column = |name: &str| columns.iter().position(|c| c == name).unwrap_or_default();
    let (module_at, owned_at) = (column("Module"), column("Owned"));

    let mut out = Vec::new();
    let mut named: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows = 0usize;
    for line in roadmap.lines().skip(headers[0] + 1) {
        if !line.trim_start().starts_with('|') {
            break;
        }
        if line.trim_start().starts_with("|---") {
            continue;
        }
        rows += 1;
        let row = table_cells(line);
        if row.len() != columns.len() {
            out.push(format!(
                "a row has {} cells, not {}: {line}",
                row.len(),
                columns.len()
            ));
            continue;
        }
        let (format, owned) = (&row[0], &row[owned_at]);
        let modules = backticked(&row[module_at]);
        let roots = backticked(owned);
        if roots.is_empty() && !matches!(owned.as_str(), "" | "—") {
            out.push(format!("{format}: the Owned cell `{owned}` names no root"));
        }
        for root in roots {
            match declared.get(&root) {
                None => out.push(format!(
                    "{format}: `{root}` has no `impl File` or `impl Resource` in crates/io/or-io/src"
                )),
                Some(stem) if !modules.contains(stem) => out.push(format!(
                    "{format}: `{root}` is declared in `{stem}`, which is not this row's module"
                )),
                Some(_) => {}
            }
            *named.entry(root).or_default() += 1;
        }
    }
    assert!(
        rows > 5,
        "the R66d table parsed as {rows} row(s), which is not it — the parse is \
         broken rather than the table"
    );

    for root in declared.keys() {
        match named.get(root) {
            None => out.push(format!(
                "`{root}` has an `impl File` or `impl Resource` in or-io and no Owned cell"
            )),
            Some(1) => {}
            Some(n) => out.push(format!("`{root}` is named by {n} Owned cells")),
        }
    }
    out
}

/// Every Owned cell of the R66d table names a root `or-io` owns, and every owned root has its cell.
#[test]
fn the_owned_column_is_the_resources_or_io_declares() {
    let declared = owned_roots_or_io_declares();
    assert!(
        !declared.is_empty(),
        "no `impl File for` or `impl Resource for` was found under crates/io/or-io/src, which is \
         the search failing rather than every owned format going"
    );
    let wrong = owned_column_disagreements(&read("docs/direction/Roadmap.md"), &declared);
    assert!(
        wrong.is_empty(),
        "docs/direction/Roadmap.md's R66d table and or-io's owned roots disagree:\n  {}\n\n\
         A format lane names its root in its own Owned cell, in backticks as \
         its `impl File for` or `impl Resource for` line spells it, in the row of the module that \
         declares it.",
        wrong.join("\n  ")
    );
}

#[test]
fn the_owned_column_check_can_tell_its_shapes_apart() {
    let table = format!(
        "\
{R66D_TABLE_HEADER}
|---|---|---|---|---|---|
| `.a` | `alpha` | Gen8 | #1 | `Alpha`, #2 | 1 of 1 |
| `.b` | `beta` | Gen8 | #1 | #3 | 1 of 1 |
| `.c` | `gamma` | Gen8 | #1 | `Stray` | 1 of 1 |
| `.d` | `delta` | Gen8 | #1 | `Moved` | 1 of 1 |
| `.e` | `epsilon` | both | #1 | — | not recorded |
| `.f` | `zeta` | Gen8 | #1 | | 1 of 1 |

After the table.
"
    );
    let declared: BTreeMap<String, String> = [
        ("Alpha", "alpha"),
        ("Moved", "elsewhere"),
        ("Unlisted", "zeta"),
    ]
    .into_iter()
    .map(|(root, stem)| (root.to_string(), stem.to_string()))
    .collect();

    assert_eq!(
        owned_column_disagreements(&table, &declared),
        vec![
            "`.b`: the Owned cell `#3` names no root",
            "`.c`: `Stray` has no `impl File` or `impl Resource` in crates/io/or-io/src",
            "`.d`: `Moved` is declared in `elsewhere`, which is not this row's module",
            "`Unlisted` has an `impl File` or `impl Resource` in or-io and no Owned cell",
        ],
        "a cell with no root, a root nothing owns, a root in another module's \
         row and a root with no cell are each named; a dash and an empty cell \
         are not"
    );
}

#[test]
fn the_workspace_members_are_the_layers_they_live_in() {
    let manifest = read("Cargo.toml");
    let start = manifest
        .find("members = [")
        .expect("the workspace manifest lists its members");
    let end = manifest[start..]
        .find(']')
        .expect("the members list is closed")
        + start;
    let declared: BTreeSet<String> = manifest[start..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.split('#').next().unwrap_or("").trim();
            line.strip_prefix('"')?
                .split('"')
                .next()
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .collect();

    let mut on_disk = BTreeSet::new();
    let crates = repo_root().join("crates");
    for layer in std::fs::read_dir(&crates)
        .expect("crates/ exists")
        .flatten()
    {
        if !layer.path().is_dir() {
            continue;
        }
        for member in std::fs::read_dir(layer.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if member.path().join("Cargo.toml").is_file() {
                let layer_name = layer.file_name().to_string_lossy().to_string();
                let crate_name = member.file_name().to_string_lossy().to_string();
                on_disk.insert(format!("crates/{layer_name}/{crate_name}"));
            }
        }
    }

    assert!(
        on_disk.len() > 15,
        "found {} crate(s) under crates/, which is not this workspace",
        on_disk.len()
    );
    assert_eq!(
        declared, on_disk,
        "every crate under crates/<layer>/ is a workspace member and every \
         member is one of those. A crate missing from `members` is not built, \
         not linted and not tested; a member with no directory does not \
         resolve."
    );

    let stray: Vec<&String> = declared
        .iter()
        .filter(|m| m.split('/').count() != 3)
        .collect();
    assert!(
        stray.is_empty(),
        "the layer scheme is `crates/<layer>/<crate>`, so a crate's directory \
         names the layer it belongs to. These do not fit it: {stray:?}"
    );
}

/// Whether a manifest lists `krate` as a dependency of any kind.
fn lists_dependency(manifest: &str, krate: &str) -> bool {
    manifest.lines().any(|line| {
        let line = line.trim_start();
        line.strip_prefix(krate)
            .is_some_and(|rest| rest.starts_with('.') || rest.trim_start().starts_with('='))
    })
}

/// Each "Only the app depends on `X`" in `doc` that a manifest in `manifests` (crate, text) refutes.
fn sole_dependent_disagreements(doc: &str, manifests: &[(String, String)]) -> Vec<String> {
    const CLAIM: &str = "only the app depends on `";
    let flat = doc
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    flat.match_indices(CLAIM)
        .filter_map(|(at, _)| {
            let krate = flat[at + CLAIM.len()..].split('`').next()?;
            let users: Vec<&str> = manifests
                .iter()
                .filter(|(_, text)| lists_dependency(text, krate))
                .map(|(name, _)| name.as_str())
                .collect();
            (!users.is_empty())
                .then(|| format!("`{krate}` is not the app's alone: {}", users.join(", ")))
        })
        .collect()
}

#[test]
fn a_crate_the_reference_gives_to_the_app_alone_is_listed_by_no_other_crate() {
    let mut manifests = Vec::new();
    for layer in std::fs::read_dir(repo_root().join("crates"))
        .expect("crates/ exists")
        .flatten()
    {
        for member in std::fs::read_dir(layer.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Ok(text) = std::fs::read_to_string(member.path().join("Cargo.toml")) {
                manifests.push((member.file_name().to_string_lossy().into_owned(), text));
            }
        }
    }
    assert!(
        manifests.len() > 15,
        "found {} manifest(s)",
        manifests.len()
    );
    assert_eq!(
        sole_dependent_disagreements(&read("docs/reference/Crate_Reference.md"), &manifests),
        Vec::<String>::new(),
        "a dependency edge was added or the claim was written without looking: \
         `cargo tree -p <crate> -e normal --depth 1` lists the edges"
    );
}

#[test]
fn the_sole_dependent_check_can_tell_a_held_claim_from_a_refuted_one() {
    let manifests = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, text)| (name.to_string(), text.to_string()))
            .collect()
    };
    let crates = manifests(&[
        ("or-cli", "[dependencies]\nor-source.workspace = true\n"),
        (
            "or-wardrobe",
            "[dependencies]\nor-source = { path = \"x\" }\n",
        ),
        (
            "or-world",
            "[dependencies]\nor-source-extra.workspace = true\n",
        ),
        ("or-anim", "[dependencies]\nor-world.workspace = true\n"),
    ]);
    let doc = "Only the\n  app depends on `or-source`. And only the app depends on `or-world`; \
               Only the app depends on `or-anim`, and Only the app depends on `or-sim`.";
    assert_eq!(
        sole_dependent_disagreements(doc, &crates),
        [
            "`or-source` is not the app's alone: or-cli, or-wardrobe",
            "`or-world` is not the app's alone: or-anim",
        ],
        "a claim across a line break is read, a longer crate name does not count as the crate, \
         and a crate nothing lists is left alone"
    );
}
