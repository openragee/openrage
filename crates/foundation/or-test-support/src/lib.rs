use std::path::{Path, PathBuf};

pub mod ledger;
pub mod rpf9;

pub const FIXTURE_ROOT_ENV: &str = "OPENRAGE_FIXTURES";

/// Where a whole game install is, for the questions a corpus is the wrong
/// denominator for.
pub const GAME_ROOT_ENV: &str = "OPENRAGE_GAME_ROOT";

pub const TARGET_GEN8: &str = "gen8";
pub const TARGET_GEN9: &str = "gen9";

pub const KNOWN_TARGETS: &[&str] = &[TARGET_GEN8, TARGET_GEN9];

pub const DEFAULT_TARGET: &str = TARGET_GEN8;

pub const SIBLING_CORPUS_DIR: &str = "openrage-corpus";

pub const SHARED_CORPUS_CRATE: &str = "or-io-verify";

pub const SHARED_CORPUS_PATH: &str = "crates/io/or-io-verify/tests/fixtures";

/// The shared corpus's directory in [`SIBLING_CORPUS_DIR`], named for the crate
/// that owned it before [`SHARED_CORPUS_CRATE`]; every staged corpus uses it.
pub const SHARED_CORPUS_SIBLING_DIR: &str = "or-asset";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusOrigin {
    Override(PathBuf),
    InTree(PathBuf),
    Sibling(PathBuf),
    Absent(PathBuf),
}

impl CorpusOrigin {
    pub fn path(&self) -> &Path {
        match self {
            Self::Override(p) | Self::InTree(p) | Self::Sibling(p) | Self::Absent(p) => p,
        }
    }

    pub fn found(&self) -> bool {
        !matches!(self, Self::Absent(_))
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Override(p) => format!("{} (from {FIXTURE_ROOT_ENV})", p.display()),
            Self::InTree(p) => format!("{} (staged in-tree)", p.display()),
            Self::Sibling(p) => format!("{} (sibling {SIBLING_CORPUS_DIR})", p.display()),
            Self::Absent(p) => format!("{} (nothing staged)", p.display()),
        }
    }
}

pub fn corpus_origin(manifest_dir: &str) -> CorpusOrigin {
    let in_tree = Path::new(manifest_dir).join("tests/fixtures");
    resolve(in_tree, manifest_dir, Path::new(manifest_dir).file_name())
}

pub fn shared_corpus_origin(manifest_dir: &str) -> CorpusOrigin {
    let in_tree = match workspace_root(manifest_dir) {
        Some(root) => root.join(SHARED_CORPUS_PATH),
        None => Path::new(manifest_dir).join("tests/fixtures"),
    };
    resolve(in_tree, manifest_dir, None)
}

pub fn fixtures_root(manifest_dir: &str) -> PathBuf {
    corpus_origin(manifest_dir).path().to_path_buf()
}

pub fn shared_corpus_root(manifest_dir: &str) -> PathBuf {
    shared_corpus_origin(manifest_dir).path().to_path_buf()
}

pub fn corpus_origin_for(crate_name: &str, manifest_dir: &str) -> CorpusOrigin {
    let in_tree = Path::new(manifest_dir).join("tests/fixtures");
    resolve(in_tree, manifest_dir, Some(crate_name.as_ref()))
}

/// Corpus for a crate that owns one and has its own override variable.
///
/// It takes the same routes as [`corpus_origin_for`], with two differences:
///
/// - **[`FIXTURE_ROOT_ENV`] is not consulted.** One path cannot name two
///   crates' corpora, and every existing setting of that variable names
///   the shared corpus. Honouring it here would hand a metadata suite a
///   directory of drawables.
/// - **`subdir` is appended to the discovered routes and not to `own_env`,**
///   because an operator pointing a variable at a corpus points it at the
///   directory holding the files, and that is what the existing variable means.
pub fn owned_corpus_origin(
    crate_name: &str,
    manifest_dir: &str,
    own_env: &str,
    subdir: &str,
) -> CorpusOrigin {
    if let Some(raw) = std::env::var_os(own_env) {
        if !raw.is_empty() {
            let root = PathBuf::from(raw);
            assert!(
                root.is_dir(),
                "{own_env} is set to `{}`, which is not a directory.\n\
                 Every suite reading it would have skipped and reported success.",
                root.display()
            );
            return CorpusOrigin::Override(root);
        }
    }

    let in_tree = Path::new(manifest_dir).join("tests/fixtures").join(subdir);
    if has_content(&in_tree) {
        return CorpusOrigin::InTree(in_tree);
    }
    match sibling_corpus(manifest_dir, Some(crate_name.as_ref())) {
        Some(root) if root.ends_with(crate_name) => {
            let with_subdir = root.join(subdir);
            if with_subdir.is_dir() {
                CorpusOrigin::Sibling(with_subdir)
            } else {
                CorpusOrigin::Absent(in_tree)
            }
        }
        _ => CorpusOrigin::Absent(in_tree),
    }
}

/// The directory in [`SIBLING_CORPUS_DIR`] holding mod files, which the shared corpus's suites must never see.
pub const MODS_CORPUS_SIBLING_DIR: &str = "mods";

/// The staged mod files, found only beside or above the workspace.
///
/// ⚠ Absent under [`FIXTURE_ROOT_ENV`], so the clean-checkout run sees none.
pub fn mods_corpus_origin(manifest_dir: &str) -> CorpusOrigin {
    let corpus = workspace_root(manifest_dir)
        .and_then(|root| corpus_beside_or_above(&root))
        .map(|corpus| corpus.join(MODS_CORPUS_SIBLING_DIR));
    match corpus {
        Some(mods) if override_root().is_none() && mods.is_dir() => CorpusOrigin::Sibling(mods),
        Some(mods) => CorpusOrigin::Absent(mods),
        None => CorpusOrigin::Absent(Path::new(manifest_dir).join(MODS_CORPUS_SIBLING_DIR)),
    }
}

fn resolve(
    in_tree: PathBuf,
    manifest_dir: &str,
    crate_dir: Option<&std::ffi::OsStr>,
) -> CorpusOrigin {
    if let Some(root) = override_root() {
        return CorpusOrigin::Override(root);
    }
    if has_content(&in_tree) {
        return CorpusOrigin::InTree(in_tree);
    }
    match sibling_corpus(manifest_dir, crate_dir) {
        Some(root) => CorpusOrigin::Sibling(root),
        None => CorpusOrigin::Absent(in_tree),
    }
}

fn override_root() -> Option<PathBuf> {
    let raw = std::env::var_os(FIXTURE_ROOT_ENV)?;
    if raw.is_empty() {
        return None;
    }
    let root = PathBuf::from(raw);
    assert!(
        root.is_dir(),
        "{FIXTURE_ROOT_ENV} is set to `{}`, which is not a directory.\n\
         Every fixture-backed suite would have skipped and reported success.\n\
         Fix the path, or unset the variable to let the corpus be discovered.",
        root.display()
    );
    Some(root)
}

/// The corpus repository, searched beside the workspace root and then upwards.
///
/// ⚠ A lane under `../openrage-lanes/` or `.claude/worktrees/` does not have
/// the corpus beside it. Checking only `root.parent()` finds nothing there, so every
/// fixture-backed suite in a lane skips, and **a suite that skips reports
/// green**. The first ancestor checked is the parent, so a plain clone
/// resolves as it always did.
fn corpus_beside_or_above(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .skip(1)
        .map(|dir| dir.join(SIBLING_CORPUS_DIR))
        .find(|corpus| corpus.is_dir())
}

fn sibling_corpus(manifest_dir: &str, crate_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let corpus = corpus_beside_or_above(&workspace_root(manifest_dir)?)?;
    if let Some(own) = crate_dir.map(|name| corpus.join(name)) {
        if own.is_dir() {
            return Some(own);
        }
    }
    let shared = corpus.join(SHARED_CORPUS_SIBLING_DIR);
    shared.is_dir().then_some(shared)
}

/// Whether `dir` holds anything that could be a fixture.
///
/// Existence is not the question. `or-io-verify/tests/fixtures/` is committed with
/// a `README.md` in it, and its winning would point every suite at an empty
/// directory and suppress the sibling checkout behind it.
///
/// Dotfiles and `.md` files are skipped as categories, never by name: a
/// blocklist of `*.md` and `.gitignore` let `.gitkeep` through, and
/// the metadata corpus's 6,072 fixtures stayed invisible while the suite
/// reported success in 0.00 s.
fn has_content(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if is_dir(&entry, &path) {
            if has_content(&path) {
                return true;
            }
        } else if !name.to_ascii_lowercase().ends_with(".md") {
            return true;
        }
    }
    false
}

pub fn openrage_root() -> Option<PathBuf> {
    workspace_root(env!("CARGO_MANIFEST_DIR"))
}

pub fn workspace_root(start: &str) -> Option<PathBuf> {
    let mut dir: Option<&Path> = Some(Path::new(start));
    while let Some(current) = dir {
        let manifest = current.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if text
                .lines()
                .any(|line| line.trim_start().starts_with("[workspace]"))
            {
                return Some(current.to_path_buf());
            }
        }
        dir = current.parent();
    }
    None
}

#[derive(Debug, Clone)]
pub struct Corpus {
    root: PathBuf,
    extension: String,
    title: Option<String>,
    target: Option<String>,
    all_targets: bool,
    limit: Option<usize>,
}

impl Corpus {
    pub fn new(manifest_dir: &str, extension: &str) -> Self {
        Self::with_root(fixtures_root(manifest_dir), extension)
    }

    pub fn with_root(root: impl Into<PathBuf>, extension: &str) -> Self {
        Self {
            root: root.into(),
            extension: extension.trim_start_matches('.').to_ascii_lowercase(),
            title: None,
            target: None,
            all_targets: false,
            limit: None,
        }
    }

    pub fn title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
    }

    pub fn target(mut self, target: &str) -> Self {
        self.target = Some(target.to_string());
        self.all_targets = false;
        self
    }

    pub fn all_targets(mut self) -> Self {
        self.all_targets = true;
        self.target = None;
        self
    }

    /// Cap how many files are returned.
    pub fn limit(mut self, max: usize) -> Self {
        self.limit = Some(max);
        self
    }

    pub fn search_dir(&self) -> PathBuf {
        let mut dir = self.root.clone();
        if let Some(title) = &self.title {
            dir = dir.join(title);
        }
        if let Some(target) = &self.target {
            dir = dir.join(target);
        }
        dir
    }

    pub fn find(&self) -> Vec<PathBuf> {
        let wanted = self.target.as_deref().unwrap_or(DEFAULT_TARGET);

        let mut candidates = Vec::new();
        collect(&self.root, &self.extension, &mut candidates);

        let mut out: Vec<PathBuf> = candidates
            .into_iter()
            .filter(|path| {
                let segments = relative_segments(&self.root, path);
                if !self.all_targets && target_of(&segments) != wanted {
                    return false;
                }
                match &self.title {
                    Some(title) => segments.iter().any(|s| s == title),
                    None => true,
                }
            })
            .collect();

        out.sort();
        if let Some(limit) = self.limit {
            out.truncate(limit);
        }
        out
    }

    pub fn find_or_skip(&self, what: &str) -> Option<Vec<PathBuf>> {
        let files = self.find();
        if files.is_empty() {
            skip(what, &self.search_dir(), &self.extension);
            None
        } else {
            Some(files)
        }
    }

    pub fn report(&self) -> String {
        let files = self.find();
        if files.is_empty() {
            format!(
                "corpus: 0 .{} files under {}",
                self.extension,
                self.search_dir().display()
            )
        } else {
            format!(
                "corpus: {} .{} files under {}",
                files.len(),
                self.extension,
                self.search_dir().display()
            )
        }
    }
}

#[track_caller]
pub fn skip(what: &str, dir: &Path, extension: &str) {
    ledger::record(
        ledger::SkipReason::Corpus,
        what,
        format_args!(
            "no .{extension} fixtures under {}; check out {SIBLING_CORPUS_DIR} beside this \
             workspace, stage a corpus there, or point {FIXTURE_ROOT_ENV} at one",
            dir.display()
        ),
    );
}

#[track_caller]
pub fn skip_corpus(what: &str, detail: impl std::fmt::Display) {
    ledger::record(ledger::SkipReason::Corpus, what, detail);
}

#[track_caller]
pub fn skip_device(what: &str, detail: impl std::fmt::Display) {
    ledger::record(ledger::SkipReason::Device, what, detail);
}

#[track_caller]
pub fn skip_install(what: &str, detail: impl std::fmt::Display) {
    ledger::record(ledger::SkipReason::Install, what, detail);
}

/// Names a metadata schema snapshot file, or `none` for the clean-checkout state.
pub const SCHEMA_ENV: &str = "OPENRAGE_SCHEMA";

/// The shared schema store's file under [`SIBLING_CORPUS_DIR`]; the same
/// file `or_title::schema::STORE_FILE` names.
pub const SCHEMA_STORE: &str = "or-meta/v3/schema_snapshot.bin";

/// The schema snapshot a suite installs: [`SCHEMA_ENV`]'s file, else the
/// shared store above the workspace; `None` for `none` or when none is staged.
///
/// ⚠ A path and not a library, so this crate gains no dependency; and never
/// the user's configuration directory, which a test must not read.
pub fn schema_snapshot() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(SCHEMA_ENV) {
        let value = raw.to_string_lossy().trim().to_string();
        if matches!(value.to_ascii_lowercase().as_str(), "none" | "off") {
            return None;
        }
        if !value.is_empty() {
            let path = PathBuf::from(value);
            assert!(
                path.is_file(),
                "{SCHEMA_ENV} is set to `{}`, which is not a file.\n\
                 Every suite needing a schema would have skipped and reported success.",
                path.display()
            );
            return Some(path);
        }
    }
    corpus_beside_or_above(&openrage_root()?)
        .map(|corpus| corpus.join(SCHEMA_STORE))
        .filter(|path| path.is_file())
}

/// Record that `what` needs a schema snapshot and none is staged.
#[track_caller]
pub fn skip_schema(what: &str) {
    ledger::record(
        ledger::SkipReason::Corpus,
        what,
        format_args!(
            "no metadata schema snapshot: run `just snapshot <psc-root>`, or point \
             {SCHEMA_ENV} at one"
        ),
    );
}

/// Every game install `OPENRAGE_GAME_ROOT` names, as a `;`-separated list of
/// install roots, in the order listed; an entry that is not a directory is
/// dropped.
///
/// ⚠ There is no discovery here on purpose: a test that goes looking through
/// Program Files is a test whose population changes when somebody installs a
/// game, and a measurement wants a denominator it was told.
pub fn game_roots() -> Vec<PathBuf> {
    let Some(raw) = std::env::var_os(GAME_ROOT_ENV) else {
        return Vec::new();
    };
    raw.to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

#[macro_export]
macro_rules! require_corpus {
    ($files:expr, $what:expr) => {
        if $files.is_empty() {
            $crate::skip_corpus($what, "no fixtures matched");
            return;
        }
    };
}

fn relative_segments(root: &Path, path: &Path) -> Vec<String> {
    path.strip_prefix(root)
        .unwrap_or(path)
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

fn target_of(segments: &[String]) -> &str {
    for segment in segments {
        if let Some(target) = KNOWN_TARGETS.iter().find(|t| *t == segment) {
            return target;
        }
    }
    DEFAULT_TARGET
}

/// `path.is_dir()` without its stat: the listing already carries the type,
/// and only a link (a staged corpus is often a junction) needs following.
fn is_dir(entry: &std::fs::DirEntry, path: &Path) -> bool {
    match entry.file_type() {
        Ok(t) if t.is_symlink() => path.is_dir(),
        Ok(t) => t.is_dir(),
        Err(_) => path.is_dir(),
    }
}

fn collect(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_dir(&entry, &path) {
            collect(&path, extension, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        {
            out.push(path);
        }
    }
}

pub fn matched_targets_in(
    root: impl Into<PathBuf>,
    title: &str,
    targets: &[&str],
    extension: &str,
) -> Vec<(String, Vec<(String, PathBuf)>)> {
    let root = root.into();
    let mut groups: std::collections::BTreeMap<String, Vec<(String, PathBuf)>> = Default::default();

    for target in targets {
        for path in Corpus::with_root(root.clone(), extension)
            .title(title)
            .target(target)
            .find()
        {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            groups
                .entry(name.to_string())
                .or_default()
                .push((target.to_string(), path.clone()));
        }
    }

    groups
        .into_iter()
        .filter(|(_, paths)| paths.len() == targets.len())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_corpus_resolves_to_a_directory_that_exists() {
        if std::env::var_os(FIXTURE_ROOT_ENV).is_some() {
            return;
        }

        let root = shared_corpus_root(env!("CARGO_MANIFEST_DIR"));
        assert!(
            root.is_dir(),
            "the shared corpus should resolve to a real directory, got {}\n\
             if the corpus moved, {SHARED_CORPUS_PATH} is the one line to change",
            root.display()
        );
    }

    #[test]
    fn the_workspace_root_is_the_one_declaring_a_workspace() {
        let root = workspace_root(env!("CARGO_MANIFEST_DIR")).expect("a workspace above us");
        assert!(
            root.join("Cargo.toml").is_file(),
            "the root should have a manifest"
        );
        assert_ne!(
            root,
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "this crate's own directory is not the workspace root"
        );
    }

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_override<T>(value: Option<&Path>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(FIXTURE_ROOT_ENV);
        match value {
            Some(path) => std::env::set_var(FIXTURE_ROOT_ENV, path),
            None => std::env::remove_var(FIXTURE_ROOT_ENV),
        }

        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));

        match previous {
            Some(value) => std::env::set_var(FIXTURE_ROOT_ENV, value),
            None => std::env::remove_var(FIXTURE_ROOT_ENV),
        }
        match out {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    #[test]
    fn the_override_wins_over_discovery() {
        let scratch = tempfile::tempdir().unwrap();
        let elsewhere = scratch.path().to_path_buf();

        let origin = with_override(Some(&elsewhere), || {
            shared_corpus_origin(env!("CARGO_MANIFEST_DIR"))
        });

        assert_eq!(origin, CorpusOrigin::Override(elsewhere));
    }

    #[test]
    #[cfg_attr(
        all(windows, debug_assertions),
        ignore = "Cranelift cannot unwind a panic on Windows: run it with `cargo test --release`"
    )]
    fn an_override_naming_nothing_fails_rather_than_skipping_quietly() {
        let scratch = tempfile::tempdir().unwrap();
        let missing = scratch.path().join("no-such-corpus-dir");
        assert!(!missing.exists(), "the test needs a path that is not there");

        let panicked = with_override(Some(&missing), || {
            std::panic::catch_unwind(|| fixtures_root(env!("CARGO_MANIFEST_DIR"))).is_err()
        });

        assert!(
            panicked,
            "a {FIXTURE_ROOT_ENV} pointing at nothing should fail, not resolve"
        );
    }

    #[test]
    fn an_empty_override_is_the_same_as_an_unset_one() {
        let discovered = with_override(None, || corpus_origin(env!("CARGO_MANIFEST_DIR")));
        let emptied = with_override(Some(Path::new("")), || {
            corpus_origin(env!("CARGO_MANIFEST_DIR"))
        });

        assert_eq!(emptied, discovered);
    }

    #[test]
    fn repository_furniture_does_not_count_as_a_staged_corpus() {
        // Repository furniture: a README, a `.gitkeep` and a `.gitignore`.
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().to_path_buf();
        std::fs::create_dir_all(dir.join("gen9")).unwrap();
        std::fs::write(dir.join("README.md"), "not a fixture").unwrap();
        std::fs::write(dir.join(".gitkeep"), "not a fixture").unwrap();
        std::fs::write(dir.join(".gitignore"), "*").unwrap();

        assert!(!has_content(&dir), "furniture only");

        std::fs::write(dir.join("gen9/thing.ydr"), [0u8; 4]).unwrap();
        assert!(
            has_content(&dir),
            "one real file anywhere below makes it a corpus"
        );
    }

    #[test]
    fn an_empty_shared_shadow_with_no_sibling_corpus_is_a_skip() {
        let scratch = tempfile::tempdir().unwrap();
        let workspace = scratch.path().to_path_buf();
        let verify = workspace.join("crates/io/or-io-verify");
        let shadow = verify.join("tests/fixtures");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(workspace.join("Cargo.toml"), "[workspace]\n").unwrap();
        std::fs::write(shadow.join("README.md"), "not a fixture").unwrap();
        assert!(
            corpus_beside_or_above(&workspace).is_none(),
            "the test needs no {SIBLING_CORPUS_DIR} above {}",
            workspace.display()
        );

        let origin = with_override(None, || shared_corpus_origin(verify.to_str().unwrap()));

        assert_eq!(
            origin,
            CorpusOrigin::Absent(shadow),
            "the shared shadow is or-io-verify's, and an empty one is \
             absent rather than a corpus"
        );
        assert!(
            Corpus::with_root(origin.path(), "ydr").find().is_empty(),
            "nothing found is what makes `find_or_skip` record a skip"
        );
    }

    #[test]
    fn discovery_finds_the_sibling_corpus_when_one_is_checked_out() {
        let origin = with_override(None, || shared_corpus_origin(env!("CARGO_MANIFEST_DIR")));

        let Some(sibling) = workspace_root(env!("CARGO_MANIFEST_DIR"))
            .and_then(|root| root.parent().map(|p| p.join(SIBLING_CORPUS_DIR)))
            .filter(|p| p.is_dir())
        else {
            eprintln!("SKIPPED: no {SIBLING_CORPUS_DIR} beside this workspace");
            return;
        };

        assert_eq!(
            origin,
            CorpusOrigin::Sibling(sibling.join(SHARED_CORPUS_SIBLING_DIR)),
            "a sibling corpus should be found without {FIXTURE_ROOT_ENV}"
        );
    }

    #[test]
    fn a_crate_with_its_own_corpus_directory_gets_that_one() {
        // The metadata corpus has its own `or-meta` directory and must not be
        // handed the shared corpus's drawables.
        let Some(corpus) = workspace_root(env!("CARGO_MANIFEST_DIR"))
            .and_then(|root| root.parent().map(|p| p.join(SIBLING_CORPUS_DIR)))
            .filter(|p| p.join("or-meta").is_dir())
        else {
            eprintln!("SKIPPED: no sibling corpus with an or-meta directory");
            return;
        };

        let origin = with_override(None, || {
            corpus_origin_for("or-meta", env!("CARGO_MANIFEST_DIR"))
        });

        assert_eq!(origin, CorpusOrigin::Sibling(corpus.join("or-meta")));
    }
}
