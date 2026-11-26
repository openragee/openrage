//! `openrage-cli mcp`: the Model Context Protocol over stdio, so an AI agent drives OpenRage with no window.
//! Its tools are the routes of `serve`, every command of this binary, and the editor's frames drawn from a script.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::serve::{self, LineRead, Session};

#[path = "mcp_gta6.rs"]
mod gta6;

/// The name a client lists this server under.
const NAME: &str = "openrage";

/// The revisions answered as asked; a client that asks for another is answered with the last.
const PROTOCOLS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

/// How much of each stream of a command an answer carries.
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// How long a command runs when the call does not say.
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;

const INSTRUCTIONS: &str = "OpenRage reads, edits and writes the files of GTA V, Legacy and Enhanced, with no window. \
`mount` an install once, then `search`, `list`, `locate` and `extract` read it. `cli` runs every other command \
(inspect, convert, meta, texture, mesh, rpf, project, wardrobe and the rest) on files named by absolute path. \
`shots` draws the editor from a script and answers PNGs. The mounted install is only read, and a command writes \
only the paths it is given.";

/// A parameter: its name (`!` after a required one), its JSON type (`strings` is an array of strings) and what it means.
type Param = (&'static str, &'static str, &'static str);

const PROJECT: Param = ("project!", "string", "path of the .openrage-proj");
const MODULE: Param = (
    "module",
    "string",
    "which clothing content, when the project holds several",
);
const TIMEOUT: Param = (
    "timeoutSeconds",
    "integer",
    "stop after this long; default 600",
);

/// The routes of `serve` offered as tools, each under the route's own name.
const ROUTES: &[(&str, &str, &[Param])] = &[
    (
        "mount",
        "Mount a GTA V install so search, packs, list, locate and extract can read it. Call once: it stays mounted, \
         and a second call replaces it. Answers the edition, the DLC packs mounted and any warnings.",
        &[
            ("game", "string", "install folder; default: the game selected in the OpenRage app"),
            ("dlc", "string", "DLC cutoff: `basegame`, a build number, or a pack name; default: every pack"),
            ("keys", "string", "a keys cache file; default: the title's own cache, made from the install's executable"),
        ],
    ),
    ("status", "Whether an install is mounted, and which.", &[]),
    ("unmount", "Drop the mounted install.", &[]),
    (
        "search",
        "Find files of the mounted install by name, extension or data-file type; give at least one of pattern, ext \
         and fileTypes. A hit has its logical path and the archive it resolves to. `truncated` true means the sweep \
         stopped at maxHits, so `count` is not a total.",
        &[
            ("pattern", "string", "case-insensitive substring, or a glob with * and ?, of the file's name"),
            ("ext", "strings", "extensions to keep, such as [\"ydr\", \"ytd\"]"),
            ("fileTypes", "strings", "data-file types such as WEAPONINFO_FILE"),
            ("device", "string", "keep hits whose device, such as dlc_mpHeist3, contains this"),
            ("raw", "boolean", "crawl every .rpf on disk instead of the mounted view; needs no mount"),
            ("countOnly", "boolean", "answer the count without the hits"),
            ("limit", "integer", "how many hits to answer; ask for a page, an install is large"),
            ("offset", "integer", "the first hit of the page"),
            ("maxHits", "integer", "where the sweep stops; default 100000"),
        ],
    ),
    ("packs", "The DLC packs of the mounted install, in the engine's mount order.", &[]),
    (
        "list",
        "The names in one folder of the mounted install.",
        &[("path!", "string", "a folder such as common:/data/ai")],
    ),
    (
        "locate",
        "The archive a file of the mounted install resolves to, after the title update and the DLC packs overlay it.",
        &[("path!", "string", "a logical path such as common:/data/ai/weapons.meta")],
    ),
    (
        "extract",
        "Copy one file out of the mounted install. Give `out`; `inline` answers base64 instead, for a file up to 8 MiB.",
        &[
            ("path!", "string", "a logical path, as a search hit gives it"),
            ("out", "string", "the file to write"),
            ("inline", "boolean", "answer the bytes as base64 and write nothing"),
        ],
    ),
    (
        "extract-many",
        "Copy files out of the mounted install into a folder, each under the name of its device. A file that fails \
         is listed in `errors` and does not stop the rest.",
        &[
            ("paths!", "strings", "logical paths"),
            ("outDir!", "string", "the folder to write into"),
        ],
    ),
    (
        "wardrobe-add",
        "Add clothes to the clothing pack of a project, as dropping them on the Wardrobe does. Changes the project, \
         never the install. Give `from` or `package`.",
        &[
            PROJECT,
            MODULE,
            ("from", "strings", "files and folders to add"),
            ("package", "string", "a garment package folder"),
        ],
    ),
    (
        "wardrobe-set",
        "Change garments of the clothing pack of a project. Besides `garments`, give any of: label, group, ped (male or \
         female), slot, position, skin (universal or race), heels, hairScale, hairHeight, stickyness, audio, audio2, \
         firstPerson (each null for none), flags (a number), flagOn and flagOff (the engine's names such as \
         NOT_IN_CAR), propMask, renderFlags, addTag, removeTag, and remove true to delete them.",
        &[PROJECT, MODULE, ("garments!", "strings", "garment ids, as wardrobe.json and wardrobe-add give them")],
    ),
    (
        "wardrobe-fix",
        "Apply every safe fix to the clothing pack of a project, and answer what changed and what is left.",
        &[PROJECT, MODULE],
    ),
    (
        "wardrobe-waive",
        "Record why a rule does not apply to the clothing pack of a project, or to one garment of it.",
        &[
            PROJECT,
            MODULE,
            ("code!", "string", "the rule's code, as `cli` [\"wardrobe\", \"rules\"] lists them"),
            ("reason!", "string", "why it is waived"),
            ("garment", "string", "the one garment it is waived for"),
        ],
    ),
];

const SHOTS: &str = "Draw the OpenRage editor with no window from a script of steps, one per line, and answer each \
`shot` as a PNG: the way to see a model, a texture or a screen, and to do what only the editor does. Each call is a \
fresh editor, so a script states the whole flow. Coordinates are points on a 1600x900 screen. Steps: \
`game legacy|enhanced [FOLDER]` makes an install active; `open PATH` opens a file or a folder; `project PATH` opens a \
project; `workspace NAME`; `mode wireframe|solid|material|shaded`; `click X Y`, `rclick X Y`, `dclick X Y`, \
`hover X Y`, `scroll X Y DY`; `type TEXT`; `key NAME [ctrl] [shift] [alt]`; `wait SECONDS`; `size W H`; \
`theme dark|light`; `shot NAME` saves NAME.png. A wrong line fails with its number before anything is drawn. Needs a \
GPU adapter and the `openrage` editor beside this binary.";

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None => serve_stdio(),
        Some("install") => install(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("Error: unknown option: {other}");
            eprintln!();
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn print_usage() {
    eprintln!("MCP server: an AI agent drives OpenRage over stdio, with no window.");
    eprintln!();
    eprintln!("Usage: openrage-cli mcp");
    eprintln!("         Serve the Model Context Protocol on stdin and stdout; logs go to stderr.");
    eprintln!("       openrage-cli mcp install [claude-code] [claude-desktop] [cursor]");
    eprintln!(
        "         Register this binary with the clients found on this machine, or with those"
    );
    eprintln!("         named, and print the entry any other client takes.");
    eprintln!();
    eprintln!("See docs/reference/MCP_Server.md for the tools.");
}

fn serve_stdio() -> ExitCode {
    serve::init_stderr_tracing();
    let mut session = Session::default();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut reader = stdin.lock();
    let mut raw = Vec::new();
    loop {
        raw.clear();
        let response = match serve::read_capped_line(&mut reader, &mut raw) {
            LineRead::Eof => break,
            LineRead::Empty => continue,
            LineRead::TooLong => Some(failure(Value::Null, -32600, "request too long".into())),
            LineRead::Line => answer(&String::from_utf8_lossy(&raw), &mut session),
        };
        if let Some(response) = response {
            if !serve::write_line(&mut out, &response) {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

fn failure(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The answer to one message, and none to a notification.
fn answer(line: &str, session: &mut Session) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(e) => return Some(failure(Value::Null, -32700, format!("parse error: {e}"))),
    };
    let id = message.get("id").cloned()?;
    let params = message.get("params");
    let result = match message.get("method").and_then(Value::as_str) {
        Some("initialize") => Ok(initialized(params)),
        Some("ping") => Ok(json!({})),
        Some("tools/list") => Ok(json!({ "tools": tools() })),
        Some("tools/call") => call(params, session),
        Some(other) => Err((-32601, format!("method not found: {other}"))),
        None => Err((-32600, "not a request".to_string())),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => failure(id, code, message),
    })
}

fn initialized(params: Option<&Value>) -> Value {
    let asked = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = asked
        .filter(|v| PROTOCOLS.contains(v))
        .unwrap_or(PROTOCOLS[PROTOCOLS.len() - 1]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn tool(name: &str, about: &str, params: &[Param]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (key, kind, means) in params {
        let key = match key.strip_suffix('!') {
            Some(key) => {
                required.push(key);
                key
            }
            None => key,
        };
        let mut schema = match *kind {
            "strings" => json!({ "type": "array", "items": { "type": "string" } }),
            kind => json!({ "type": kind }),
        };
        schema["description"] = json!(means);
        properties.insert(key.to_string(), schema);
    }
    json!({
        "name": name,
        "description": about,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
    })
}

fn tools() -> Vec<Value> {
    let mut tools: Vec<Value> = ROUTES
        .iter()
        .map(|(name, about, params)| tool(name, about, params))
        .collect();
    // Named from the dispatch's own list, so a new command is offered without a line here.
    let commands: Vec<&str> = super::COMMANDS
        .iter()
        .copied()
        .filter(|c| !matches!(*c, "serve" | "mcp"))
        .collect();
    tools.push(tool(
        "cli",
        &format!(
            "Run any openrage-cli command and answer what it printed and its exit code: 0 success, 1 a runtime \
             error, 2 a usage error. Each call is one process, so name files by absolute path. `args` [\"help\"] says \
             what each command does, and [COMMAND, \"--help\"] gives its usage. An answer keeps the last {} KiB of \
             each stream, so a long output is paged or written to a file: [\"script\", FILE, \"--functions\", \
             \"--json\", \"--offset\", N, \"--limit\", N] answers one page of a compiled script's functions. \
             Commands: {}.",
            MAX_OUTPUT_BYTES / 1024,
            commands.join(", ")
        ),
        &[
            ("args!", "strings", "the command line after `openrage-cli`, one word per item"),
            ("cwd", "string", "the folder to run in"),
            TIMEOUT,
        ],
    ));
    tools.push(tool(
        "shots",
        SHOTS,
        &[
            ("steps!", "string", "the script, one step per line"),
            (
                "outDir",
                "string",
                "the folder the PNGs go to; default: a new one under the temp folder",
            ),
            TIMEOUT,
        ],
    ));
    tools.extend(gta6::tools());
    tools
}

fn text(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

fn call(params: Option<&Value>, session: &mut Session) -> Result<Value, (i64, String)> {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params
        .and_then(|p| p.get("arguments"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let outcome = match name {
        "cli" => cli(&args),
        "shots" => shots(&args),
        route if ROUTES.iter().any(|(name, ..)| *name == route) => route_of(route, args, session),
        rpf9 if gta6::is_tool(rpf9) => gta6::call(rpf9, &args),
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };
    Ok(match outcome {
        Ok(content) => json!({ "content": content, "isError": false }),
        Err(why) => json!({ "content": [text(why)], "isError": true }),
    })
}

/// A route of `serve`, asked as a client of that protocol asks it and answered without its envelope.
fn route_of(
    route: &str,
    mut args: Map<String, Value>,
    session: &mut Session,
) -> Result<Vec<Value>, String> {
    args.remove("id");
    args.insert("cmd".into(), json!(route));
    let (mut response, _) = serve::handle_line(&Value::Object(args).to_string(), session);
    if response["ok"] == json!(true) {
        if let Some(fields) = response.as_object_mut() {
            fields.remove("ok");
        }
        Ok(vec![text(response.to_string())])
    } else {
        Err(response["error"].as_str().unwrap_or("failed").to_string())
    }
}

fn timeout_of(args: &Map<String, Value>) -> Duration {
    Duration::from_secs(
        args.get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    )
}

fn cli(args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let line: Vec<String> = args
        .get("args")
        .and_then(Value::as_array)
        .and_then(|words| words.iter().map(|w| w.as_str().map(String::from)).collect())
        .ok_or("cli: 'args' (an array of strings) is required")?;
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    let cwd = args.get("cwd").and_then(Value::as_str);
    let (succeeded, said) = run_child(&exe, &line, cwd, timeout_of(args))?;
    if succeeded {
        Ok(vec![text(said)])
    } else {
        Err(said)
    }
}

fn shots(args: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let steps = args
        .get("steps")
        .and_then(Value::as_str)
        .ok_or("shots: 'steps' (the script) is required")?;
    let editor = std::env::current_exe()
        .map_err(|e| format!("cannot find this binary: {e}"))?
        .with_file_name(format!("openrage{}", std::env::consts::EXE_SUFFIX));
    if !editor.is_file() {
        return Err(format!(
            "shots draws with the editor, and {} is not there",
            editor.display()
        ));
    }
    let dir = match args.get("outDir").and_then(Value::as_str) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            std::env::temp_dir().join(format!("openrage-shots-{stamp}"))
        }
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let script = dir.join("steps.txt");
    std::fs::write(&script, steps).map_err(|e| format!("write {}: {e}", script.display()))?;
    let line = [
        "--self-check".to_string(),
        "--shots".to_string(),
        script.display().to_string(),
        dir.display().to_string(),
    ];
    let started = std::time::SystemTime::now();
    let (succeeded, said) = run_child(&editor, &line, None, timeout_of(args))?;
    if !succeeded {
        return Err(said);
    }
    // ⚠ Read from the folder, in the order written, and not from what the editor printed: that text is cut at a ceiling a long run passes.
    let mut saved: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)
        .map_err(|e| format!("read {}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .filter(|(modified, _)| *modified >= started)
        .collect();
    saved.sort();
    let mut content = vec![text(said)];
    let mut inline = 0;
    for (_, path) in saved {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        content.push(text(path.display().to_string()));
        inline += bytes.len();
        // Past the ceiling a picture is answered by its path alone.
        if inline <= serve::MAX_INLINE_BYTES {
            content.push(json!({
                "type": "image",
                "data": serve::base64_encode(&bytes),
                "mimeType": "image/png",
            }));
        }
    }
    Ok(content)
}

/// The last [`MAX_OUTPUT_BYTES`] of a stream and how many bytes came before them, read on a thread so a full pipe never stalls the child.
///
/// ⚠ The end is kept and not the start: a command that fails says why last, after whatever it logged.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<(Vec<u8>, u64)> {
    std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut total = 0;
        let mut chunk = [0; 8192];
        let Some(mut pipe) = pipe else {
            return (tail, 0);
        };
        while let Ok(read @ 1..) = pipe.read(&mut chunk) {
            total += read as u64;
            tail.extend_from_slice(&chunk[..read]);
            if tail.len() > 2 * MAX_OUTPUT_BYTES {
                tail.drain(..tail.len() - MAX_OUTPUT_BYTES);
            }
        }
        if tail.len() > MAX_OUTPUT_BYTES {
            tail.drain(..tail.len() - MAX_OUTPUT_BYTES);
        }
        let dropped = total - tail.len() as u64;
        (tail, dropped)
    })
}

/// Whether `exe` exited 0, and what it said: its exit code, then each stream that held anything.
fn run_child(
    exe: &Path,
    args: &[String],
    cwd: Option<&str>,
    limit: Duration,
) -> Result<(bool, String), String> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
    let streams = [
        ("stdout", drain(child.stdout.take())),
        ("stderr", drain(child.stderr.take())),
    ];
    let deadline = Instant::now() + limit;
    let status = loop {
        // The deadline is read first, so a limit of nothing stops every child, however fast it ends.
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // ⚠ The readers are not joined: a process the child started can hold the pipes open past the kill.
            return Err(format!(
                "stopped after {} s; give a larger timeoutSeconds",
                limit.as_secs()
            ));
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut said = match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "ended by a signal".to_string(),
    };
    for (name, reader) in streams {
        let (tail, dropped) = reader.join().unwrap_or_default();
        if !tail.is_empty() {
            said += &format!("\n--- {name} ---\n");
        }
        if dropped > 0 {
            said += &format!("[the first {dropped} byte(s) are not shown]\n");
        }
        said += String::from_utf8_lossy(&tail).trim_end();
    }
    Ok((status.success(), said))
}

/// The clients `install` registers with.
const CLIENTS: &[&str] = &["claude-code", "claude-desktop", "cursor"];

/// What a client's configuration holds for this server: this binary by its full path.
fn entry(exe: &str) -> Value {
    json!({ "command": exe, "args": ["mcp"] })
}

/// `text`, a client's configuration file or nothing yet, with this server in its `mcpServers` and the rest as it was.
fn registered(text: &str, exe: &str) -> Result<String, String> {
    let mut config: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|e| format!("is not JSON: {e}"))?
    };
    config
        .as_object_mut()
        .ok_or("is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("holds an `mcpServers` that is not an object")?
        .insert(NAME.into(), entry(exe));
    serde_json::to_string_pretty(&config).map_err(|e| e.to_string())
}

/// The configuration file of each client that keeps `mcpServers` in a JSON file, where the client's folder exists.
fn config_files() -> Vec<(&'static str, PathBuf)> {
    let mut claude: Vec<PathBuf> = dirs::config_dir()
        .map(|dir| dir.join("Claude"))
        .into_iter()
        .collect();
    // ⚠ A Claude Desktop installed as a Windows package reads its own copy of the roaming folder, not `%APPDATA%`.
    let packages = dirs::data_local_dir().map(|dir| dir.join("Packages"));
    if let Some(Ok(entries)) = packages.map(std::fs::read_dir) {
        claude.extend(
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
                .map(|e| e.path().join("LocalCache").join("Roaming").join("Claude")),
        );
    }
    let mut found: Vec<(&'static str, PathBuf)> = claude
        .into_iter()
        .filter(|dir| dir.is_dir())
        .map(|dir| ("claude-desktop", dir.join("claude_desktop_config.json")))
        .collect();
    if let Some(cursor) = dirs::home_dir().map(|home| home.join(".cursor")) {
        if cursor.is_dir() {
            found.push(("cursor", cursor.join("mcp.json")));
        }
    }
    found
}

fn register_in(file: &Path, exe: &str) -> Result<(), String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    std::fs::write(file, registered(&text, exe)?).map_err(|e| e.to_string())
}

/// Claude Code keeps its servers behind its own command, which replaces an earlier registration of this name.
fn register_with_claude_code(exe: &str) -> Result<(), String> {
    let run = |args: &[&str]| {
        // `claude.cmd` is the npm install on Windows, which a bare name does not find.
        ["claude", "claude.cmd"].iter().find_map(|program| {
            Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .output()
                .ok()
        })
    };
    let _ = run(&["mcp", "remove", "--scope", "user", NAME]);
    let added = run(&["mcp", "add", "--scope", "user", NAME, "--", exe, "mcp"])
        .ok_or("no `claude` on PATH")?;
    if added.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&added.stderr).trim().to_string())
    }
}

fn install(clients: &[String]) -> ExitCode {
    if let Some(unknown) = clients.iter().find(|c| !CLIENTS.contains(&c.as_str())) {
        return super::usage_error(&format!(
            "`{unknown}` is not a client this registers with: {}",
            CLIENTS.join(", ")
        ));
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe.display().to_string(),
        Err(e) => {
            eprintln!("Error: cannot find this binary: {e}");
            return ExitCode::from(1);
        }
    };
    let files = config_files();
    for client in CLIENTS {
        if !clients.is_empty() && !clients.iter().any(|c| c == client) {
            continue;
        }
        let outcomes: Vec<(String, Result<(), String>)> = if *client == "claude-code" {
            vec![("user scope".into(), register_with_claude_code(&exe))]
        } else {
            files
                .iter()
                .filter(|(name, _)| name == client)
                .map(|(_, file)| (file.display().to_string(), register_in(file, &exe)))
                .collect()
        };
        if outcomes.is_empty() {
            println!("{client}: not found on this machine");
        }
        for (place, outcome) in outcomes {
            match outcome {
                Ok(()) => println!("{client}: registered ({place}); restart it to see the tools"),
                Err(why) => println!("{client}: not registered ({place}): {why}"),
            }
        }
    }
    println!();
    println!("Any other client starts this command and speaks MCP over its stdio:");
    println!("  \"{exe}\" mcp");
    println!("which a client that keeps `mcpServers` in a JSON file holds as:");
    let snippet = json!({ "mcpServers": { NAME: entry(&exe) } });
    println!(
        "{}",
        serde_json::to_string_pretty(&snippet).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(session: &mut Session, message: Value) -> Value {
        answer(&message.to_string(), session).expect("a request is answered")
    }

    #[test]
    fn a_client_initializes_lists_the_tools_and_calls_a_route() {
        let mut s = Session::default();
        let hello = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
        );
        assert_eq!(hello["id"], json!(1));
        assert_eq!(hello["result"]["protocolVersion"], json!("2025-03-26"));
        assert!(hello["result"]["capabilities"]["tools"].is_object());
        let newer = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2999-01-01"}}),
        );
        assert_eq!(newer["result"]["protocolVersion"], json!("2025-06-18"));

        // A notification is not answered, and a method this does not have is an error and not silence.
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(answer(&note.to_string(), &mut s).is_none());
        let unknown = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}),
        );
        assert_eq!(unknown["error"]["code"], json!(-32601));

        let listed = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
        );
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), ROUTES.len() + 2 + gta6::TOOLS.len());
        let search = tools.iter().find(|t| t["name"] == "search").unwrap();
        assert_eq!(
            search["inputSchema"]["properties"]["ext"]["type"],
            json!("array")
        );
        let list = tools.iter().find(|t| t["name"] == "list").unwrap();
        assert_eq!(list["inputSchema"]["required"], json!(["path"]));
        let cli = tools.iter().find(|t| t["name"] == "cli").unwrap();
        assert!(cli["description"].as_str().unwrap().contains("wardrobe"));

        let status = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"status","arguments":{}}}),
        );
        assert_eq!(status["result"]["isError"], json!(false));
        let body: Value =
            serde_json::from_str(status["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"mounted":false,"gameRoot":null,"edition":null,"dlc":null,"packs":null,"roots":null})
        );

        // A route's refusal is the tool's error, and a tool this does not have is the protocol's.
        let unmounted = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list","arguments":{"path":"common:/data"}}}),
        );
        assert_eq!(unmounted["result"]["isError"], json!(true));
        assert!(unmounted["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not mounted"));
        let missing = ask(
            &mut s,
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"shutdown"}}),
        );
        assert_eq!(missing["error"]["code"], json!(-32602));
    }

    #[test]
    fn every_route_offered_as_a_tool_is_one_serve_answers() {
        let mut s = Session::default();
        // ⚠ `mount` with no `game` mounts the install the app has selected on this machine, so it is given a folder that is not there.
        let nowhere = std::env::temp_dir().join("openrage-mcp-no-such-install");
        let mut args = Map::new();
        args.insert("game".into(), json!(nowhere.display().to_string()));
        for (route, ..) in ROUTES {
            let said = match route_of(route, args.clone(), &mut s) {
                Ok(content) => content[0]["text"].to_string(),
                Err(why) => why,
            };
            assert!(!said.contains("unknown command"), "`{route}`: {said}");
        }
    }

    #[test]
    fn a_command_is_run_and_its_output_and_exit_code_answered_and_one_that_overruns_is_stopped() {
        // The test binary is not `openrage-cli`, so the child is cargo, which the test runs under.
        let cargo = PathBuf::from(std::env::var_os("CARGO").expect("run under cargo"));
        let long = Duration::from_secs(120);
        let (ok, said) = run_child(&cargo, &["--version".into()], None, long).unwrap();
        assert!(ok, "{said}");
        assert!(
            said.starts_with("exit code 0\n--- stdout ---\ncargo "),
            "{said}"
        );
        let (ok, said) = run_child(&cargo, &["no-such-command".into()], None, long).unwrap();
        assert!(!ok);
        assert!(said.contains("--- stderr ---"), "{said}");
        let stopped = run_child(&cargo, &["--version".into()], None, Duration::ZERO).unwrap_err();
        assert!(stopped.contains("stopped after"), "{stopped}");
    }

    #[test]
    fn registering_adds_this_server_and_keeps_what_the_file_held() {
        let held = r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#;
        let config: Value =
            serde_json::from_str(&registered(held, "C:\\or\\openrage-cli.exe").unwrap()).unwrap();
        assert_eq!(config["theme"], json!("dark"));
        assert_eq!(config["mcpServers"]["other"]["command"], json!("x"));
        assert_eq!(
            config["mcpServers"]["openrage"],
            json!({"command":"C:\\or\\openrage-cli.exe","args":["mcp"]})
        );
        let fresh: Value = serde_json::from_str(&registered("", "cli").unwrap()).unwrap();
        assert_eq!(fresh["mcpServers"]["openrage"]["command"], json!("cli"));
        // A file that is not what a client writes is left alone.
        assert!(registered("[1]", "cli").is_err());
        assert!(registered("{not json", "cli").is_err());
        assert!(registered(r#"{"mcpServers":3}"#, "cli").is_err());
    }
}
