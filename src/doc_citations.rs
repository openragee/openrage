use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const ELSEWHERE: &[(&str, &str)] = &[
    (
        "docs/extra/",
        "gitignored: the RAGE binary-layout specifications are the private tier",
    ),
    ("docs/extra/rage-base/", "same"),
    (
        "crates/asset/or-meta/data/schema_snapshot.bin",
        "gitignored and in no commit at this path; `docs/guides/Private_Tier.md` \
         records where the snapshot was tracked and how it was retracted",
    ),
    (
        "crates/format/or-io/",
        "a pre-V2 path; `docs/guides/Private_Tier.md` describes a retraction that \
         happened at it",
    ),
    ("crates/format/or-meta/", "same"),
    (
        "src/state/gta6_worldview/index.rs",
        "where the plans of the world say the GTA VI index stood before R71b moved it to `crates/domain/or-world-gta6`",
    ),
    (
        "src/state/gta6_worldview/index_keep.rs",
        "same",
    ),
    (
        "src/state/gta6_worldview/index_states.rs",
        "same",
    ),
    (
        "src/state/gta6_worldview/index_types.rs",
        "where the plans say a reader stood before R71b moved it to `or_io::gta6::map`",
    ),
    (
        "src/state/gta6_worldview/lights_index.rs",
        "same",
    ),
    (
        "src/state/gta6_worldview/lights_lod.rs",
        "same",
    ),
    (
        "src/state/gta6_worldview/stream.rs",
        "where the plan of the world says the selecting half of the GTA VI viewer stood before R71b moved it to `crates/domain/or-world-gta6`",
    ),
    (
        "crates/or-io/src/rage_types/frag.rs",
        "one of the 20 files in `archive/ydd-clothing-preview`; `or-io` is the \
         pre-split facade crate, and the path not resolving is the roadmap's \
         point — R46 is a rewrite rather than a port",
    ),
    (
        "jbib_059_u",
        "a corpus filename, named where R46 records how a garment pairs to its \
         textures",
    ),
    (
        "rage_cbinst_matrices",
        "a constant-buffer name in the game's own HLSL, read out of a `.fxc`'s RDEF \
         chunk at runtime; `or-shader-tool reflect` prints it and nothing declares it",
    ),
    (
        "pg_base_field",
        "a field that never existed; the lesson box is about its absence",
    ),
    (
        "rage_layout_tests",
        "deleted with the macro; docs cite the bare name without `!`",
    ),
];

/// Paths and names deleted with `or-asset`, `or-format-api`, `or-macros`,
/// pack discovery and the provenance lint's crate list in R66f, which a document may still cite where it records
/// what that code measured or held, at the revision or pull request it names.
///
/// ⚠ A tombstone, not a permanent permit: rewriting a citation away drops the
/// entry, through [`the_permit_list_holds_nothing_nobody_cites`]. An entry
/// resolves in no code, though a comment may still name it.
const DELETED_IN_R66F: &[&str] = &[
    "a_broken_layout_cannot_pass_this_gate",
    "a_moved_offset_in_the_lod_group_binds_and_only_the_compiled_reader_notices",
    "an_enhanced_texture_view_pointer_rebuilds_at_its_own_view",
    "crates/asset/or-asset/tests/a_material_names_its_parameters.rs",
    "crates/asset/or-format-api",
    "every_corpus_legacy_texture_decodes_as_the_compiled_reader_decodes_it",
    "from_resource_with_options",
    "no_shared_type_diverges_in_offsets_alone",
    "physical_size_and_template",
    "read_raw_bound",
    "scripts/or_io_crates.txt",
    "the_gap_is_named_and_still_open",
    "what_a_behaviour_record_looks_like",
    "what_a_pack_read_costs",
    "what_a_whole_file_record_read_costs_against_the_compiled_reader",
    "what_the_permanent_write_rows_rest_on",
];

/// Names a document may cite that live in this tree only inside a comment.
///
/// ⚠ Separate from [`ELSEWHERE`] because the two say different things and one
/// cannot stand in for the other. An `ELSEWHERE` entry says *nothing in this
/// tree carries this name*; an entry here says *something does, and it is
/// prose*. Putting one of these in `ELSEWHERE` fails
/// [`the_permit_list_only_holds_things_that_are_missing`], because it resolves.
const IN_A_COMMENT_ONLY: &[(&str, &str)] = &[];

const IN_TREE: &[&str] = &[
    "src/",
    "crates/",
    "docs/",
    "tools/",
    "scripts/",
    "assets/",
    "plugins/",
    ".github/",
    "external/",
];

const PATTERN: &[char] = &[
    '*', '{', '}', '<', '>', '~', '?', '|', '[', ']', '(', ')', ' ', '\t', '\\',
];

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The directories git ignores, which no commit carries and so no citation may
/// rest on; an ignored `repomix/` alone has held 142,066 files.
///
/// ⚠ Asked of git rather than parsed from `.gitignore`, so nested ignore files
/// and `info/exclude` count too. Where git cannot answer, nothing is skipped.
pub(crate) fn is_ignored_dir(path: &Path) -> bool {
    static IGNORED: OnceLock<BTreeSet<PathBuf>> = OnceLock::new();
    IGNORED
        .get_or_init(|| {
            let listed = std::process::Command::new("git")
                .args(["ls-files", "-oi", "--exclude-standard", "--directory"])
                .current_dir(repo_root())
                .output();
            let Ok(listed) = listed else {
                return BTreeSet::new();
            };
            if !listed.status.success() {
                return BTreeSet::new();
            }
            String::from_utf8_lossy(&listed.stdout)
                .lines()
                .filter_map(|line| line.strip_suffix('/'))
                .map(|dir| repo_root().join(dir))
                .collect()
        })
        .contains(path)
}

/// Every file and directory [`walk`] reaches from the root, walked once.
pub(crate) fn walked() -> &'static [PathBuf] {
    static WALKED: OnceLock<Vec<PathBuf>> = OnceLock::new();
    WALKED.get_or_init(|| {
        let mut paths = Vec::new();
        walk(repo_root(), &mut paths);
        paths
    })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git" | "target" | "node_modules" | ".claude"
        ) {
            continue;
        }
        if name.starts_with('_') || name == ".cursor" {
            continue;
        }
        if name == "extra"
            && path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "docs")
        {
            continue;
        }
        // The listing's type is unfollowed, so a link reads as a link.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if is_ignored_dir(&path) {
                continue;
            }
            out.push(path.clone());
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn tree() -> BTreeSet<String> {
    walked()
        .iter()
        .map(|p| {
            p.strip_prefix(repo_root())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

/// The same text with its comments gone and its string literals left alone.
///
/// ⚠ **A string literal counts as code here and a comment does not**, and the
/// line is the compiler: a suite that passes its own name to `skip_corpus` is
/// referring to it, a `//` that mentions it is not. Raw strings are tracked by
/// their hash count because a `\` does not escape inside one, and getting that
/// wrong would swallow live code and report it as prose.
///
/// It scans bytes: every delimiter is ASCII, and no byte of a multi-byte
/// UTF-8 character equals one, so only whole characters are ever cut.
fn without_comments(text: &str) -> String {
    let b = text.as_bytes();
    let at = |i: usize| b.get(i).copied();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];

        if c == b'/' && at(i + 1) == Some(b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && at(i + 1) == Some(b'*') {
            let mut depth = 1usize;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && at(i + 1) == Some(b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && at(i + 1) == Some(b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'r' || c == b'b' {
            let r = if c == b'b' && at(i + 1) == Some(b'r') {
                i + 1
            } else {
                i
            };
            let mut hashes = 0;
            let mut j = r + 1;
            while at(j) == Some(b'#') {
                hashes += 1;
                j += 1;
            }
            if at(r) == Some(b'r') && at(j) == Some(b'"') {
                let mut k = j + 1;
                loop {
                    if k >= b.len() {
                        break;
                    }
                    if b[k] == b'"' && (1..=hashes).all(|h| at(k + h) == Some(b'#')) {
                        k += hashes + 1;
                        break;
                    }
                    k += 1;
                }
                let end = k.min(b.len());
                out.extend_from_slice(&b[i..end]);
                i = end;
                continue;
            }
        }
        if c == b'\'' && at(i + 2) == Some(b'\'') {
            out.extend_from_slice(&b[i..=i + 2]);
            i += 3;
            continue;
        }
        if c == b'\'' && at(i + 1) == Some(b'\\') {
            let mut k = i + 2;
            while k < b.len() && b[k] != b'\'' {
                k += 1;
            }
            let end = (k + 1).min(b.len());
            out.extend_from_slice(&b[i..end]);
            i = end;
            continue;
        }
        if c == b'"' {
            out.push(c);
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                out.push(b[i]);
                i += 1;
                if b[i - 1] == b'"' {
                    break;
                }
            }
            continue;
        }

        out.push(c);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn words_of(text: &str, into: &mut BTreeSet<String>) {
    let mut current = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            current.push(c);
        } else if !current.is_empty() {
            into.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        into.insert(current);
    }
}

/// Every word in the workspace, and the subset of them the compiler sees.
fn vocabularies() -> (BTreeSet<String>, BTreeSet<String>) {
    static BUILT: OnceLock<(BTreeSet<String>, BTreeSet<String>)> = OnceLock::new();
    BUILT.get_or_init(build_vocabularies).clone()
}

fn build_vocabularies() -> (BTreeSet<String>, BTreeSet<String>) {
    const SOURCE: &[&str] = &["rs", "toml", "ps1", "sh", "yml", "wgsl"];
    const BRACED: &[&str] = &["rs", "wgsl"];
    let (mut all, mut code) = (BTreeSet::new(), BTreeSet::new());
    for path in walked() {
        if path.starts_with(repo_root().join("external")) || path.ends_with("doc_citations.rs") {
            continue;
        }
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if let Some(stem) = name.strip_suffix(".rs") {
            all.insert(stem.to_string());
            code.insert(stem.to_string());
        }
        all.insert(name.clone());
        code.insert(name.clone());
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        if !SOURCE.contains(&extension.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        words_of(&text, &mut all);
        let stripped = if BRACED.contains(&extension.as_str()) {
            without_comments(&text)
        } else {
            text.lines()
                .map(|line| line.split('#').next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n")
        };
        words_of(&stripped, &mut code);
    }
    (all, code)
}

fn vocabulary() -> BTreeSet<String> {
    vocabularies().0
}

fn documents() -> Vec<(String, String)> {
    walked()
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .filter(|p| !p.starts_with(repo_root().join("external")))
        .filter_map(|p| {
            let rel = p
                .strip_prefix(repo_root())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            std::fs::read_to_string(p).ok().map(|t| (rel, t))
        })
        .collect()
}

fn outside_code_blocks(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn backticked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (index, chunk) in text.split('`').enumerate() {
        if index % 2 == 1 && !chunk.contains('\n') {
            out.push(chunk.trim().to_string());
        }
    }
    out
}

fn without_locator(token: &str) -> String {
    let base = match token.find("::") {
        Some(at) => &token[..at],
        None => token,
    };
    let bytes: Vec<char> = base.chars().collect();
    let mut cut = bytes.len();
    let mut i = bytes.len();
    while i > 0 {
        let c = bytes[i - 1];
        if c.is_ascii_digit() || c == ',' || c == '-' || c == '\u{2013}' || c == '\u{2014}' {
            i -= 1;
            continue;
        }
        if c == ':' && i < bytes.len() {
            cut = i - 1;
        }
        break;
    }
    bytes[..cut].iter().collect()
}

fn is_name(token: &str) -> bool {
    let token = token.strip_suffix("()").unwrap_or(token);
    let token = token.strip_suffix('!').unwrap_or(token);
    if token.matches('_').count() < 2 {
        return false;
    }
    let mut parts = token.split('_');
    parts.all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    }) && token.starts_with(|c: char| c.is_ascii_lowercase())
}

fn permitted(token: &str) -> bool {
    ELSEWHERE.iter().any(|(t, _)| *t == token) || DELETED_IN_R66F.contains(&token)
}

type Citation = (String, String);

fn citations() -> (Vec<Citation>, Vec<Citation>) {
    static BUILT: OnceLock<(Vec<Citation>, Vec<Citation>)> = OnceLock::new();
    BUILT.get_or_init(build_citations).clone()
}

fn build_citations() -> (Vec<Citation>, Vec<Citation>) {
    let (mut paths, mut names) = (Vec::new(), Vec::new());
    for (document, text) in documents() {
        for token in backticked(&outside_code_blocks(&text)) {
            if token.is_empty() || token.contains(PATTERN) || token.contains("/.../") {
                continue;
            }
            let base = without_locator(&token);
            if IN_TREE.iter().any(|root| base.starts_with(root)) {
                paths.push((base, document.clone()));
            } else if is_name(&base) {
                let base = base
                    .trim_end_matches("()")
                    .trim_end_matches('!')
                    .to_string();
                names.push((base, document.clone()));
            }
        }
    }
    (paths, names)
}

#[test]
fn every_in_tree_path_a_document_cites_exists() {
    let tree = tree();
    let (paths, _) = citations();
    assert!(
        paths.len() > 100,
        "only {} in-tree path citation(s) were found across the documentation, \
         which is too few — the walk or the backtick parse is wrong",
        paths.len()
    );

    let mut missing: Vec<String> = paths
        .iter()
        .filter(|(path, _)| !tree.contains(path.trim_end_matches('/')) && !permitted(path))
        .map(|(path, document)| format!("  {document}: `{path}`"))
        .collect();
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "these documents cite a path in this repository that does not \
         exist:\n{}\n\n\
         Fix the citation. If the path is genuinely gone and the sentence is \
         a tombstone for it, add it to ELSEWHERE in src/doc_citations.rs with \
         that reason.",
        missing.join("\n")
    );
}

#[test]
fn every_name_a_document_cites_resolves() {
    let vocabulary = vocabulary();
    let (_, names) = citations();
    assert!(
        names.len() > 300,
        "only {} name citation(s) were found across the documentation, which \
         is too few — the backtick parse is wrong",
        names.len()
    );

    let mut missing: Vec<String> = names
        .iter()
        .filter(|(name, _)| !vocabulary.contains(name) && !permitted(name))
        .map(|(name, document)| format!("  {document}: `{name}`"))
        .collect();
    missing.sort();
    missing.dedup();

    assert!(
        missing.is_empty(),
        "these documents cite a name that appears nowhere in this \
         workspace:\n{}\n\n\
         A renamed test is the most common kind of stale citation here and \
         the cheapest to fix. If the name belongs to the pack repository or \
         is not a Rust item at all, add it to ELSEWHERE in \
         src/doc_citations.rs with that reason.",
        missing.join("\n")
    );
}

/// A citation a comment can satisfy is half a check, and this is the other half.
///
/// [`every_name_a_document_cites_resolves`] resolves against every word in
/// every source file, **comments included**, on purpose: a name that
/// survives only in a comment is still a name, and
/// deleting the last comment mentioning something should fail the documents
/// citing it. The cost is that a *wrong* name in a comment satisfies a
/// document that repeats it, and the two then agree with each other and with
/// nothing else.
///
/// Found by exactly that route.
/// `crates/asset/or-asset/tests/what_the_gen9_collapse_still_needs.rs` named
/// `or_asset::target`'s guard as `each_leaf_resolves_to_a_different_type_per_target`;
/// the function was `every_leaf_differs_between_the_two_targets`, and
/// a handoff document cited the comment's spelling. Two more of the
/// same shape were sitting beside it. Three of 373 cited names, on the sweep
/// that added this test.
///
/// ⚠ **What it does not catch**, so nobody reads it as more than it is: a name
/// this tree never mentions at all is [`every_name_a_document_cites_resolves`]'s
/// job, and a name a document gets wrong in a way some *unrelated* comment
/// happens to spell is caught by neither. This one closes the case where the
/// citation and its only evidence are the same sentence.
#[test]
fn no_cited_name_rests_on_a_comment_alone() {
    let (all, code) = vocabularies();
    let (_, names) = citations();

    let mut prose: Vec<String> = names
        .iter()
        .filter(|(name, _)| {
            all.contains(name)
                && !code.contains(name)
                && !IN_A_COMMENT_ONLY.iter().any(|(t, _)| t == name)
                && !DELETED_IN_R66F.contains(&name.as_str())
        })
        .map(|(name, document)| format!("  {document}: `{name}`"))
        .collect();
    prose.sort();
    prose.dedup();

    assert!(
        prose.is_empty(),
        "these documents cite a name that exists in this workspace only \
         inside a comment:\n{}\n\n\
         The comment is the citation's only evidence, so the two can be wrong \
         together — which is how a renamed test stays cited for months. Check \
         the comment against the code before believing either. If the name \
         belongs to the pack repository or to something genuinely outside \
         this tree, add it to IN_A_COMMENT_ONLY in src/doc_citations.rs with \
         that reason — not to ELSEWHERE, which says the opposite.",
        prose.join("\n")
    );
}

/// The stripper decides which half of the check a name lands in, so it is
/// measured rather than trusted.
#[test]
fn a_comment_is_stripped_and_a_string_literal_is_not() {
    let sample = r##"
        // only_in_a_line_comment
        /// only_in_a_doc_comment
        /* only_in_a_block_comment */
        fn f() {
            let a = "only_in_a_string";
            let b = r"only_in_a_raw_string";
            let c = r#"only_in_a_hashed_raw_string"#;
            let d = '"'; // a char literal that looks like a quote opens
            let e = in_real_code();
        }
    "##;
    let code = without_comments(sample);

    for gone in [
        "only_in_a_line_comment",
        "only_in_a_doc_comment",
        "only_in_a_block_comment",
    ] {
        assert!(!code.contains(gone), "`{gone}` survived the strip");
    }
    for kept in [
        "only_in_a_string",
        "only_in_a_raw_string",
        "only_in_a_hashed_raw_string",
        "in_real_code",
    ] {
        assert!(
            code.contains(kept),
            "`{kept}` was stripped, and a stripper that eats code reports it \
             as prose — which fails the gate on something real"
        );
    }
}

#[test]
fn the_permit_list_only_holds_things_that_are_missing() {
    let tree = tree();
    let (all, code) = vocabularies();

    let mut stale = Vec::new();
    for (token, reason) in ELSEWHERE {
        let resolves = if IN_TREE.iter().any(|root| token.starts_with(root)) {
            tree.contains(token.trim_end_matches('/'))
        } else {
            all.contains(*token)
        };
        if resolves {
            stale.push(format!("  ELSEWHERE `{token}` — permitted as: {reason}"));
        }
    }
    for (token, reason) in IN_A_COMMENT_ONLY {
        if code.contains(*token) {
            stale.push(format!(
                "  IN_A_COMMENT_ONLY `{token}` — permitted as: {reason}"
            ));
        }
    }
    for token in DELETED_IN_R66F {
        let resolves = if IN_TREE.iter().any(|root| token.starts_with(root)) {
            tree.contains(token.trim_end_matches('/'))
        } else {
            code.contains(*token)
        };
        if resolves {
            stale.push(format!("  DELETED_IN_R66F `{token}` — it is in the tree"));
        }
    }

    assert!(
        stale.is_empty(),
        "these permit entries resolve in the tree now, so the reason \
         beside each is no longer true:\n{}\n\n\
         Delete the entry. The citation it was permitting is a real citation \
         again and should be checked like every other one.",
        stale.join("\n")
    );
}

#[test]
fn the_permit_list_holds_nothing_nobody_cites() {
    let (paths, names) = citations();
    let cited: BTreeSet<&str> = paths
        .iter()
        .chain(names.iter())
        .map(|(token, _)| token.as_str())
        .collect();

    let unused: Vec<String> = ELSEWHERE
        .iter()
        .chain(IN_A_COMMENT_ONLY.iter())
        .filter(|(token, _)| !cited.contains(token))
        .map(|(token, reason)| format!("  `{token}` — permitted as: {reason}"))
        .chain(
            DELETED_IN_R66F
                .iter()
                .filter(|token| !cited.contains(*token))
                .map(|token| format!("  `{token}` — a tombstone of R66f")),
        )
        .collect();

    assert!(
        unused.is_empty(),
        "no document cites these any more, so permitting them says nothing:\n{}\n\n\
         Delete the entries.",
        unused.join("\n")
    );
}
