//! The GTA VI (RPF9) archive tools of `openrage-cli mcp`: each runs the `openrage-cli rpf9` verb it
//! names in a process of its own, so a tool answers exactly what the verb prints.

use serde_json::{Map, Value};

use super::{run_child, text, timeout_of, tool, Param, TIMEOUT};

/// What every write tool says of the archive it writes.
const WRITES: &str = "It writes a new archive at `out` and never changes the source. The game \
refuses the result: it checks a signature over the table when it mounts an archive, and only the \
game's maker can sign one, so this is a structural write.";

const ARCHIVE: Param = ("archive!", "string", "absolute path of the .rpf");
const INNER: Param = (
    "inner",
    "string",
    "nested archives to walk into, slash-separated, each named as `entry` names one",
);
const ENTRY: Param = (
    "entry!",
    "string",
    "the entry: its recovered name, its 0x hash or its decimal index, as rpf9-list gives them",
);
const OUT: Param = (
    "out!",
    "string",
    "absolute path of the new archive; it must not exist",
);
const NAMES: Param = (
    "names",
    "string",
    "a folder of recovered-names tables; without one an entry shows as its hash",
);
const KEYS: Param = (
    "keys",
    "string",
    "a GTA VI key snapshot file; default: the one the install supplies",
);
const EXE: Param = (
    "exe",
    "string",
    "the GTA VI executable a masked key snapshot opens with",
);
const FOLDER: Param = (
    "folder!",
    "string",
    "absolute path of a GTA VI install folder",
);

/// The parameters given to the verb as positional words, in the order a tool lists them; every
/// other string parameter is given as `--name value`.
const POSITIONAL: &[&str] = &["archive", "folder", "outDir", "inner"];

/// A tool: its name, the words after `rpf9`, what it does, whether it writes an archive, and its
/// parameters.
type Tool = (
    &'static str,
    &'static [&'static str],
    &'static str,
    bool,
    &'static [Param],
);

pub(super) const TOOLS: &[Tool] = &[
    (
        "rpf9-list",
        &["list", "--json"],
        "List the entries of a GTA VI archive, or of one nested in it, as JSON: index, name, hash, \
         start, stored and real size, and whether the body is enciphered.",
        false,
        &[ARCHIVE, INNER, NAMES, KEYS, EXE, TIMEOUT],
    ),
    (
        "rpf9-find",
        &["find"],
        "Find entries across every archive of a GTA VI install folder by name substring, by hash or \
         by kind, and answer each hit's archive, entry and size.",
        false,
        &[
            FOLDER,
            ("name", "string", "a substring of the entry's recovered name"),
            ("hash", "string", "the entry's 64-bit name hash, in hex"),
            ("kind", "string", "archive, resource, media, or a class byte as 0xNN"),
            NAMES,
            KEYS,
            EXE,
            TIMEOUT,
        ],
    ),
    (
        "rpf9-read",
        &["extract", "--decrypt"],
        "Read one entry of a GTA VI archive: its body, deciphered and inflated, is written into \
         `outDir` under the entry's name.",
        false,
        &[
            ARCHIVE,
            ("outDir!", "string", "absolute path of the folder to write into"),
            INNER,
            ENTRY,
            NAMES,
            KEYS,
            EXE,
            TIMEOUT,
        ],
    ),
    (
        "rpf9-census",
        &["census"],
        "Count the archives of a GTA VI install folder, nested ones included: entries, nested \
         archives, enciphered bodies and the kinds they inflate to. Writes counts only, as \
         rpf9-census.md in `out`.",
        false,
        &[
            FOLDER,
            (
                "out!",
                "string",
                "absolute path of the folder rpf9-census.md is written into",
            ),
            NAMES,
            KEYS,
            EXE,
            TIMEOUT,
        ],
    ),
    (
        "rpf9-copy",
        &["copy"],
        "Copy a GTA VI archive, deciphering and re-enciphering every table.",
        true,
        &[ARCHIVE, OUT, KEYS, EXE, TIMEOUT],
    ),
    (
        "rpf9-replace",
        &["replace"],
        "Replace the body of one entry of a GTA VI archive with the bytes of a file.",
        true,
        &[
            ARCHIVE,
            INNER,
            ENTRY,
            ("with!", "string", "absolute path of the file holding the new body"),
            OUT,
            (
                "resource",
                "boolean",
                "true for an entry with a 20-byte row: `with` is then the resource's loose form",
            ),
            NAMES,
            KEYS,
            EXE,
            TIMEOUT,
        ],
    ),
    (
        "rpf9-remove",
        &["remove"],
        "Remove one entry of a GTA VI archive.",
        true,
        &[ARCHIVE, INNER, ENTRY, OUT, NAMES, KEYS, EXE, TIMEOUT],
    ),
    (
        "rpf9-rename",
        &["rename"],
        "Rename one entry of a GTA VI archive: both of its name hashes change.",
        true,
        &[
            ARCHIVE,
            INNER,
            ENTRY,
            ("name!", "string", "the entry's new path"),
            OUT,
            NAMES,
            KEYS,
            EXE,
            TIMEOUT,
        ],
    ),
    (
        "rpf9-defrag",
        &["defrag"],
        "Pack the bodies of a GTA VI archive in order, closing the gaps between them; only the \
         starts and the table change.",
        true,
        &[ARCHIVE, OUT, KEYS, EXE, TIMEOUT],
    ),
];

pub(super) fn tools() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|(name, _, about, writes, params)| {
            if *writes {
                tool(name, &format!("{about} {WRITES}"), params)
            } else {
                tool(name, about, params)
            }
        })
        .collect()
}

pub(super) fn is_tool(name: &str) -> bool {
    TOOLS.iter().any(|t| t.0 == name)
}

pub(super) fn call(name: &str, args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    answer(name, args, |line| {
        let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
        run_child(&exe, line, None, timeout_of(args))
    })
}

/// The tool's command line run by `run`, which answers whether it succeeded and what it said.
fn answer(
    name: &str,
    args: &Map<String, Value>,
    run: impl FnOnce(&[String]) -> Result<(bool, String), String>,
) -> Result<Vec<Value>, String> {
    let (_, verb, _, _, params) = TOOLS
        .iter()
        .find(|t| t.0 == name)
        .ok_or_else(|| format!("unknown tool: {name}"))?;
    let (succeeded, said) = run(&line_of(verb, params, args)?)?;
    if succeeded {
        Ok(vec![text(said)])
    } else {
        Err(said)
    }
}

/// `rpf9`, the verb's words, then each string parameter given: a positional one as its value, any
/// other as `--name value`; a boolean parameter given as true is the flag `--name`.
fn line_of(
    verb: &[&str],
    params: &[Param],
    args: &Map<String, Value>,
) -> Result<Vec<String>, String> {
    let mut line: Vec<String> = std::iter::once("rpf9")
        .chain(verb.iter().copied())
        .map(String::from)
        .collect();
    for (key, kind, _) in params {
        if *kind == "boolean" {
            if args.get(*key).and_then(Value::as_bool) == Some(true) {
                line.push(format!("--{key}"));
            }
            continue;
        }
        if *kind != "string" {
            continue;
        }
        let (key, required) = match key.strip_suffix('!') {
            Some(key) => (key, true),
            None => (*key, false),
        };
        let Some(value) = args.get(key) else {
            if required {
                return Err(format!("'{key}' is required"));
            }
            continue;
        };
        let value = value
            .as_str()
            .ok_or_else(|| format!("'{key}' is a string"))?;
        if !POSITIONAL.contains(&key) {
            line.push(format!("--{key}"));
        }
        line.push(value.to_string());
    }
    Ok(line)
}

#[cfg(test)]
#[path = "mcp_gta6_tests.rs"]
mod tests;
