//! The architecture the core and its adapters are held to: a module of the core names no game, no
//! title's crate and no file extension of a game, leans on no module that does and holds no
//! static; the core depends on no crate of its own layer or above; and no adapter crate depends on
//! another adapter, on a crate of the runtime, on a tool or on the app.
//!
//! ⚠ Each rule has a ledger of what broke it the day it was written: the modules of one title's
//! model that still live in the core, and the shared modules that lean on them. An entry that no
//! longer breaks its rule fails the test, so a ledger only shrinks.
//!
//! ⚠ The check reads text: a module that is one title's model and states neither a game's name
//! nor one of its extensions is not found by it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The modules of the core that name a game, by their path under its `src`.
const NAMES_A_GAME: &[&str] = &[
    "archetypes.rs",
    "collision/bound.rs",
    "content_flags.rs",
    "execute.rs",
    "extension.rs",
    "extents.rs",
    "field.rs",
    "generate.rs",
    "group.rs",
    "keys.rs",
    "lib.rs",
    "manifest.rs",
    "manifest_build.rs",
    "map_data.rs",
    "map_types.rs",
    "partition.rs",
    "plan.rs",
    "prefab.rs",
    "state.rs",
    "streaming_rules.rs",
    "transform.rs",
    "xml.rs",
];

/// The modules of the core that name no game and lean on one that does: the module, and the one it leans on.
const LEANS_ON: &[(&str, &str)] = &[
    ("collision/bvh.rs", "extents.rs"),
    ("collision/set.rs", "extents.rs"),
    ("collision/shape.rs", "extents.rs"),
    ("create.rs", "archetypes.rs"),
    ("create.rs", "content_flags.rs"),
    ("create.rs", "generate.rs"),
    ("create.rs", "map_data.rs"),
    ("create.rs", "map_types.rs"),
    ("create.rs", "transform.rs"),
    ("doors/find.rs", "extension.rs"),
    ("doors/find.rs", "field.rs"),
    ("doors/find.rs", "keys.rs"),
    ("doors/find.rs", "map_data.rs"),
    ("doors/find.rs", "map_types.rs"),
    ("interior_portals.rs", "streaming_rules.rs"),
    ("scenario.rs", "field.rs"),
    ("streaming.rs", "streaming_rules.rs"),
    ("validate.rs", "field.rs"),
    ("validate.rs", "group.rs"),
    ("validate.rs", "keys.rs"),
    ("validate.rs", "map_data.rs"),
    ("validate.rs", "map_types.rs"),
    ("write.rs", "archetypes.rs"),
    ("write.rs", "content_flags.rs"),
    ("write.rs", "extents.rs"),
    ("write.rs", "field.rs"),
    ("write.rs", "keys.rs"),
    ("write.rs", "map_data.rs"),
    ("write.rs", "map_types.rs"),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Every `.rs` file under `folder`, by its path from it, in path order.
fn rust_files(folder: &Path) -> Vec<String> {
    let (mut found, mut pending) = (Vec::new(), vec![folder.to_path_buf()]);
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let under = path.strip_prefix(folder).expect("a file under the folder");
                found.push(under.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

/// The file extensions of the games: those of every declared format, and those of the metadata, script and archive files.
fn extensions() -> Vec<&'static str> {
    let declared = or_io::gta5::formats::FORMATS
        .iter()
        .map(|format| format.ext);
    declared
        .chain(["ymap", "ytyp", "ymf", "ymt", "ysc", "rpf"])
        .collect()
}

/// Whether `line` holds `word` with no letter, digit or underscore after it.
fn ends_a_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(at, _)| {
        let after = line[at + word.len()..].chars().next();
        !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// `source` as the lines it reads as: a string a backslash carries onto the next line stands on one, under the number of its first.
fn joined(source: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut carried = false;
    for (at, line) in source.lines().enumerate() {
        let (text, carries) = match line.trim_end().strip_suffix('\\') {
            Some(head) => (head, true),
            None => (line, false),
        };
        match (carried, out.last_mut()) {
            (true, Some((_, last))) => last.push_str(text.trim_start()),
            _ => out.push((at + 1, text.to_owned())),
        }
        carried = carries;
    }
    out
}

/// `line` with each string literal that follows another after a comma made one with it, as `concat!` makes them.
fn literals_joined(line: &str) -> String {
    let (mut out, mut rest) = (String::with_capacity(line.len()), line);
    while let Some(at) = rest.find('"') {
        let after = rest[at + 1..].trim_start().strip_prefix(',');
        match after.and_then(|after| after.trim_start().strip_prefix('"')) {
            Some(next) => {
                out.push_str(&rest[..at]);
                rest = next;
            }
            None => {
                out.push_str(&rest[..=at]);
                rest = &rest[at + 1..];
            }
        }
    }
    out + rest
}

/// What of `source` names a game, each with its line, in the order of the lines.
// ⚠ A name is read as the compiler reads it, not as it is laid out: a path broken before its `::`,
// a string carried over a line's end and the literals `concat!` joins state it all the same.
fn games_named(source: &str) -> Vec<(usize, String)> {
    let extensions = extensions();
    let mut found = BTreeSet::new();
    let (mut dense, mut line_of) = (String::new(), Vec::new());
    for (at, line) in source.lines().enumerate() {
        dense.extend(line.chars().filter(|c| !c.is_whitespace()));
        line_of.resize(dense.len(), at + 1);
    }
    for (at, _) in dense.match_indices("Game::") {
        found.insert((line_of[at], "a constant of `Game`".to_owned()));
    }
    for (at, line) in joined(source) {
        let lower = line.to_lowercase();
        for text in [literals_joined(&lower), lower] {
            let mut name = |what: &str| found.insert((at, what.to_owned()));
            for game in ["gta", "rdr", "red dead"] {
                if text.contains(game) {
                    name(&format!("the game in `{game}`"));
                }
            }
            if ["or_world_", "or-world-"]
                .iter()
                .any(|prefix| text.contains(prefix))
            {
                name("a title's crate");
            }
            for ext in &extensions {
                if ends_a_word(&text, &format!(".{ext}")) || text.contains(&format!("\"{ext}\"")) {
                    name(&format!("the extension `.{ext}`"));
                }
            }
        }
    }
    found.into_iter().collect()
}

/// The lines of `source` that declare a static or a thread's own: a world is opened and handed on, and no process keeps one.
fn statics(source: &str) -> Vec<usize> {
    let declares = |line: &str| {
        let mut item = line.trim_start();
        while let Some(attribute) = item.strip_prefix("#[") {
            let after = attribute.split_once(']').map_or("", |(_, after)| after);
            item = after.trim_start();
        }
        if let Some(public) = item.strip_prefix("pub") {
            item = match public.strip_prefix('(') {
                Some(scope) => scope.split_once(')').map_or("", |(_, after)| after),
                None => public,
            };
        }
        let item = item.trim_start();
        item == "static"
            || ["static ", "thread_local!", "lazy_static!"]
                .iter()
                .any(|declared| item.starts_with(declared))
    };
    let lines = source.lines().enumerate();
    lines
        .filter(|(_, line)| declares(line))
        .map(|(at, _)| at + 1)
        .collect()
}

/// The modules of the crate `source` names by path, each as the files it may be: `a.rs`, `a/b.rs`.
fn leans_on(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices("crate::") {
        let path = &source[at + "crate::".len()..];
        let ident = |text: &str| -> String {
            (text
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_'))
            .collect()
        };
        let first = ident(path);
        let second = path[first.len()..].strip_prefix("::").map(ident);
        found.insert(format!("{first}.rs"));
        if let Some(second) = second.filter(|second| !second.is_empty()) {
            found.insert(format!("{first}/{second}.rs"));
        }
    }
    found
}

/// What breaks the core's rules under `src`, and each ledger entry that breaks its rule no more.
fn faults_of_the_core(src: &Path, names_a_game: &[&str], leaning: &[(&str, &str)]) -> Vec<String> {
    let mut faults = Vec::new();
    let files = rust_files(src);
    let mut still_leaning = BTreeSet::new();
    for file in &files {
        let source = std::fs::read_to_string(src.join(file)).expect("a source file reads");
        let named = games_named(&source);
        match (names_a_game.contains(&file.as_str()), named.first()) {
            (false, Some(_)) => faults
                .extend((named.iter()).map(|(line, what)| format!("{file}:{line} names {what}"))),
            (true, None) => faults.push(format!(
                "{file} names no game any more: take it out of the ledger"
            )),
            (true, Some(_)) => continue,
            (false, None) => {}
        }
        let held = statics(&source);
        faults.extend(
            held.iter()
                .map(|line| format!("{file}:{line} holds a static")),
        );
        for module in leans_on(&source) {
            if !names_a_game.contains(&module.as_str()) || module == *file {
                continue;
            }
            match leaning.contains(&(file.as_str(), module.as_str())) {
                true => {
                    still_leaning.insert((file.clone(), module));
                }
                false => faults.push(format!("{file} leans on {module}, which names a game")),
            }
        }
    }
    for entry in names_a_game
        .iter()
        .filter(|entry| !files.iter().any(|f| f == *entry))
    {
        faults.push(format!(
            "{entry} is in the ledger and is no file of the core"
        ));
    }
    for (file, module) in leaning {
        if !still_leaning.contains(&(file.to_string(), module.to_string())) {
            faults.push(format!(
                "{file} leans on {module} no more: take the pair out of the ledger"
            ));
        }
    }
    faults
}

/// The name a manifest gives its package.
fn package_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("name"))?;
    Some(line.split('"').nth(1)?.to_owned())
}

/// The crates a manifest depends on, in any of its dependency tables: each by its key, and by the package it names where that is another.
fn dependencies(manifest: &str) -> BTreeSet<String> {
    let package = |line: &str| -> Option<String> {
        let (_, named) = line.split_once("package")?;
        let named = named.trim_start().strip_prefix('=')?;
        Some(named.split('"').nth(1)?.to_owned())
    };
    let (mut found, mut inside, mut of_one) = (BTreeSet::new(), false, false);
    for line in manifest.lines().map(str::trim) {
        if let Some(table) = line.strip_prefix('[') {
            let table = table.trim_end_matches(']');
            // A table of one dependency names it in its own header.
            let one = table.rsplit_once("dependencies.");
            found.extend(one.map(|(_, one)| one.to_owned()));
            (inside, of_one) = (table.ends_with("dependencies"), one.is_some());
        } else if (inside || of_one) && !line.starts_with('#') {
            let key: String = (line.chars())
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if inside && !key.is_empty() {
                found.insert(key);
            }
            found.extend(package(line));
        }
    }
    found
}

/// The packages of the layer `layer` under `root`, by the name each manifest gives.
fn packages(root: &Path, layer: &str) -> BTreeSet<String> {
    let crates = std::fs::read_dir(root.join("crates").join(layer));
    (crates.into_iter().flatten().flatten())
        .filter_map(|entry| package_name(&entry.path().join("Cargo.toml")))
        .collect()
}

/// The packages above the domain layer under `root`: the runtime's, the tools and the app.
fn above_the_domain(root: &Path) -> BTreeSet<String> {
    let mut above = packages(root, "runtime");
    above.extend(packages(root, "tools"));
    above.extend(package_name(&root.join("Cargo.toml")));
    above
}

/// What the core under `root` depends on that it may not: a crate of its own layer or above.
fn faults_of_the_cores_manifest(root: &Path) -> Vec<String> {
    let manifest = root.join("crates/domain/or-world/Cargo.toml");
    let manifest = std::fs::read_to_string(manifest).expect("the core's manifest reads");
    let mut forbidden = above_the_domain(root);
    forbidden.extend(packages(root, "domain"));
    forbidden.remove("or-world");
    let depends = dependencies(&manifest);
    (depends.intersection(&forbidden))
        .map(|dependency| format!("or-world depends on {dependency}"))
        .collect()
}

/// What an adapter crate under `root` depends on that it may not: another adapter, a crate of the runtime, a tool or the app.
fn faults_of_the_adapters(root: &Path) -> Vec<String> {
    let domain = root.join("crates/domain");
    let mut adapters: Vec<String> = std::fs::read_dir(&domain)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("or-world-"))
        .collect();
    adapters.sort();
    let forbidden = above_the_domain(root);
    let mut faults = Vec::new();
    for adapter in &adapters {
        let folder = domain.join(adapter);
        let manifest = std::fs::read_to_string(folder.join("Cargo.toml")).unwrap_or_default();
        for dependency in dependencies(&manifest) {
            let another = dependency != *adapter && adapters.contains(&dependency);
            if another || forbidden.contains(&dependency) {
                faults.push(format!("{adapter} depends on {dependency}"));
            }
        }
        for file in rust_files(&folder) {
            let source = std::fs::read_to_string(folder.join(&file)).unwrap_or_default();
            for other in adapters.iter().filter(|other| *other != adapter) {
                if ends_a_word(&source, &other.replace('-', "_")) {
                    faults.push(format!("{adapter}/{file} names {other}"));
                }
            }
        }
    }
    faults
}

#[test]
fn the_core_names_no_game_outside_its_ledger() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let faults = faults_of_the_core(&src, NAMES_A_GAME, LEANS_ON);
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn no_adapter_crate_depends_on_another_or_above_itself() {
    let faults = faults_of_the_adapters(&workspace());
    assert!(
        faults.is_empty(),
        "{} faults:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn the_core_depends_on_no_crate_of_its_own_layer_or_above() {
    let root = workspace();
    let above = above_the_domain(&root);
    assert!(
        above.contains("or-renderer") && above.contains("xtask") && above.len() > 6,
        "the crates above the domain were read as {above:?}"
    );
    assert_eq!(faults_of_the_cores_manifest(&root), [] as [String; 0]);
}

#[test]
fn a_planted_edge_from_the_core_to_its_own_layer_or_above_fails_the_core() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let root = scratch.path();
    let package = |name: &str| format!("[package]\nname = \"{name}\"\n");
    write(root, "Cargo.toml", &package("the-app"));
    write(
        root,
        "crates/domain/or-maps/Cargo.toml",
        &package("or-maps"),
    );
    write(
        root,
        "crates/runtime/or-renderer/Cargo.toml",
        &package("or-renderer"),
    );
    write(root, "crates/io/or-io/Cargo.toml", &package("or-io"));
    let core = |dependencies: &str| {
        let manifest = format!(
            "{}\n[dependencies]\nor-io.workspace = true\n{dependencies}",
            package("or-world")
        );
        write(root, "crates/domain/or-world/Cargo.toml", &manifest);
        faults_of_the_cores_manifest(root)
    };
    assert_eq!(core(""), [] as [String; 0]);
    assert_eq!(
        core("or-maps.workspace = true\n"),
        ["or-world depends on or-maps"]
    );
    assert_eq!(
        core("\n[dev-dependencies]\nor-renderer.workspace = true\nthe-app = { path = \"../../..\" }\n"),
        ["or-world depends on or-renderer", "or-world depends on the-app"]
    );
    assert_eq!(
        core("maps = { package = \"or-maps\", path = \"../or-maps\" }\n"),
        ["or-world depends on or-maps"]
    );
}

#[test]
fn the_forest_takes_how_a_parent_waits_from_the_policy() {
    let read = |file: &str| std::fs::read_to_string(workspace().join(file)).expect("a file reads");
    let forest = read("crates/domain/or-world/src/streaming.rs");
    assert!(
        !forest.contains("with_exact_children") && forest.contains("pub fn waiting_for("),
        "the forest states one title's rule as a constructor of its own again"
    );
    // The one driver outside the core states its title's rule by the same policy value.
    let outside = read("crates/domain/or-depends/src/world_forest.rs");
    assert!(outside.contains("Forest::waiting_for(Children::Stated)"));
}

fn write(folder: &Path, file: &str, text: &str) {
    let path = folder.join(file);
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder is made");
    std::fs::write(path, text).expect("a file is written");
}

#[test]
fn a_planted_game_fails_the_core_and_a_stale_ledger_entry_fails_too() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let src = scratch.path();
    write(
        src,
        "clean.rs",
        "pub fn yaw(pose: &Pose) -> f32 { pose.yaw }\n",
    );
    write(src, "model/old.rs", "//! What a `.ymap` places.\n");
    assert_eq!(
        faults_of_the_core(src, &["model/old.rs"], &[]),
        [] as [String; 0]
    );

    write(
        src,
        "planted.rs",
        "fn game() -> Game {\n    Game::GtaVLegacy\n}\n",
    );
    let faults = faults_of_the_core(src, &["model/old.rs"], &[]);
    assert_eq!(faults.len(), 2, "{faults:?}");
    assert!(
        faults[0].starts_with("planted.rs:2 names a constant of `Game`"),
        "{faults:?}"
    );

    for (planted, what) in [
        ("let ext = \"ytyp\";\n", "the extension `.ytyp`"),
        ("// as Red Dead Redemption does\n", "the game in `red dead`"),
        ("use or_world_two::Thing;\n", "a title's crate"),
        ("//! reads a .ybn file\n", "the extension `.ybn`"),
        // Broken across lines, a name is still stated.
        (
            "fn game() -> u8 {\n    let _ = Game\n        ::from_slug(\"x\");\n    0\n}\n",
            "a constant of `Game`",
        ),
        (
            "const EXT: &str = \"ym\\\n    ap\";\n",
            "the extension `.ymap`",
        ),
        (
            "const EXT: &str = concat!(\"ym\", \"ap\");\n",
            "the extension `.ymap`",
        ),
        (
            "const NAME: &str = \"rd\\\n    r2\";\n",
            "the game in `rdr`",
        ),
    ] {
        write(src, "planted.rs", planted);
        let faults = faults_of_the_core(src, &["model/old.rs"], &[]);
        let line = 1 + usize::from(planted.starts_with("fn game"));
        assert_eq!(faults, [format!("planted.rs:{line} names {what}")]);
    }

    for planted in [
        "static OPENED: Mutex<Option<World>> = Mutex::new(None);\n",
        "    pub(crate) static mut OPENED: usize = 0;\n",
        "thread_local! { static OPENED: Cell<usize> = Cell::new(0); }\n",
        "pub(in crate::driver) static OPENED: usize = 0;\n",
        "#[allow(dead_code)] pub(crate)static OPENED: usize = 0;\n",
        "pub static\n    OPENED: usize = 0;\n",
    ] {
        write(src, "planted.rs", planted);
        let faults = faults_of_the_core(src, &["model/old.rs"], &[]);
        assert_eq!(faults, ["planted.rs:1 holds a static"], "{planted}");
    }
    write(
        src,
        "planted.rs",
        "fn held() -> &'static str {\n    \"static\"\n}\n",
    );
    assert_eq!(
        faults_of_the_core(src, &["model/old.rs"], &[]),
        [] as [String; 0]
    );

    write(src, "planted.rs", "use crate::model::old::Thing;\n");
    let faults = faults_of_the_core(src, &["model/old.rs"], &[]);
    assert_eq!(
        faults,
        ["planted.rs leans on model/old.rs, which names a game"]
    );
    let leaning = [("planted.rs", "model/old.rs")];
    assert_eq!(
        faults_of_the_core(src, &["model/old.rs"], &leaning),
        [] as [String; 0]
    );

    write(src, "planted.rs", "pub fn nothing() {}\n");
    let faults = faults_of_the_core(src, &["model/old.rs"], &leaning);
    assert_eq!(
        faults,
        ["planted.rs leans on model/old.rs no more: take the pair out of the ledger"]
    );
    write(src, "model/old.rs", "//! What a map places.\n");
    let faults = faults_of_the_core(src, &["model/old.rs", "gone.rs"], &[]);
    assert_eq!(
        faults,
        [
            "model/old.rs names no game any more: take it out of the ledger",
            "gone.rs is in the ledger and is no file of the core",
        ]
    );
}

#[test]
fn a_planted_import_of_another_adapter_fails_the_adapters() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let root = scratch.path();
    write(root, "Cargo.toml", "[package]\nname = \"the-app\"\n");
    for above in ["tools/a-tool", "runtime/or-renderer", "runtime/a-runtime"] {
        let name = above.rsplit('/').next().expect("a name");
        write(
            root,
            &format!("crates/{above}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\n"),
        );
    }
    let manifest = |name: &str, dependencies: &str| {
        format!("[package]\nname = \"{name}\"\n\n[dependencies]\nor-world.workspace = true\n{dependencies}")
    };
    let one = "crates/domain/or-world-one";
    write(
        root,
        "crates/domain/or-world-two/Cargo.toml",
        &manifest("or-world-two", ""),
    );
    write(
        root,
        "crates/domain/or-world-two/src/lib.rs",
        "pub struct Thing;\n",
    );
    write(
        root,
        &format!("{one}/Cargo.toml"),
        &manifest("or-world-one", "or-io.workspace = true\n"),
    );
    write(
        root,
        &format!("{one}/src/lib.rs"),
        "use or_world::contract::WorldTitle;\n",
    );
    assert_eq!(faults_of_the_adapters(root), [] as [String; 0]);

    write(
        root,
        &format!("{one}/src/lib.rs"),
        "use or_world_two::Thing;\n",
    );
    assert_eq!(
        faults_of_the_adapters(root),
        ["or-world-one/src/lib.rs names or-world-two"]
    );
    write(root, &format!("{one}/src/lib.rs"), "");

    for (planted, what) in [
        ("or-world-two.workspace = true\n", "or-world-two"),
        ("or-renderer = { workspace = true }\n", "or-renderer"),
        ("a-tool.workspace = true\n", "a-tool"),
        ("a-runtime.workspace = true\n", "a-runtime"),
        (
            "\n[dev-dependencies]\nthe-app = { path = \"../../..\" }\n",
            "the-app",
        ),
        (
            "\n[dependencies.or-world-two]\nworkspace = true\n",
            "or-world-two",
        ),
        // Under another name, a dependency is the package it names.
        (
            "two = { package = \"or-world-two\", path = \"../or-world-two\" }\n",
            "or-world-two",
        ),
        (
            "\n[dependencies.draw]\npath = \"../../runtime/or-renderer\"\npackage = \"or-renderer\"\n",
            "or-renderer",
        ),
    ] {
        write(
            root,
            &format!("{one}/Cargo.toml"),
            &manifest("or-world-one", planted),
        );
        assert_eq!(
            faults_of_the_adapters(root),
            [format!("or-world-one depends on {what}")]
        );
    }
}
