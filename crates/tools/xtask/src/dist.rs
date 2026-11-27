//! `cargo xtask dist <windows|linux|macos>`: one package of the app per OS, made by one code path.
//!
//! The formats a package names (a desktop entry's MIME types, a bundle's document types) are asked
//! of the built app with `openrage --file-types`, so they are the ones it opens and cannot drift
//! from the Settings page. What each OS gets:
//!
//! - **Linux:** a tarball with `bin/`, a desktop entry whose `StartupWMClass` is the application id
//!   the window sets, a shared-mime-info package, an icon and an `install.sh`, and a `.deb` of the
//!   same files under `/usr`.
//! - **macOS:** `OpenRage.app` (a universal binary, its document types and an `.icns`), signed
//!   ad hoc, and a `.dmg`. Not notarised: a first run is a right-click, Open.
//! - **Windows:** a zip of the executables.
//!
//! Each package carries the licences and a `.sha256` file beside it.

mod deb;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};

use crate::tiers::target_directory;

/// The application id the window asks winit for, which a desktop entry's `StartupWMClass` matches.
const APP_ID: &str = "openrage";

/// Reverse-DNS bundle identifier on the project's GitHub organisation, until a domain is owned.
const BUNDLE_ID: &str = "io.github.openragee.openrage";

/// The oldest macOS an Apple-silicon Mac runs.
const MACOS_MINIMUM: &str = "11.0";

pub fn run(args: &[String]) -> Result<ExitCode> {
    let mut it = args.iter();
    let target = it
        .next()
        .context("dist takes windows, linux or macos")?
        .clone();
    let mut options = Options::default();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| -> Result<String> {
            Ok(it
                .next()
                .with_context(|| format!("{flag} needs a value"))?
                .clone())
        };
        match arg.as_str() {
            "--out" => options.out = Some(PathBuf::from(value("--out")?)),
            "--version" => options.version = Some(value("--version")?),
            "--bin-dir" => options.bin_dir = Some(PathBuf::from(value("--bin-dir")?)),
            "--file-types" => options.file_types = Some(PathBuf::from(value("--file-types")?)),
            "--icon" => options.icon = Some(PathBuf::from(value("--icon")?)),
            "--no-sign" => options.sign = false,
            "--require-helper" => options.require_helper = true,
            other => bail!("dist: unknown argument `{other}`"),
        }
    }

    let root = crate::git::here()?;
    let out = options.out.clone().unwrap_or_else(|| {
        target_directory()
            .unwrap_or_else(|_| root.join("target"))
            .join("dist")
    });
    std::fs::create_dir_all(&out)?;

    let artifacts = match target.as_str() {
        "linux" => package(Target::Linux, &root, &out, &options)?,
        "macos" => package(Target::Macos, &root, &out, &options)?,
        "windows" => package(Target::Windows, &root, &out, &options)?,
        other => bail!("dist: no package for `{other}`; windows, linux or macos"),
    };
    for artifact in &artifacts {
        let sum = sha256_file(artifact)?;
        let name = artifact
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("artifact");
        std::fs::write(
            artifact.with_file_name(format!("{name}.sha256")),
            format!("{sum}  {name}\n"),
        )?;
        println!("{}  {sum}", artifact.display());
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Clone)]
struct Options {
    out: Option<PathBuf>,
    version: Option<String>,
    /// A folder already holding `openrage` and `openrage-cli`, and `or-shader-tool` if it is to be packaged, so nothing is built.
    bin_dir: Option<PathBuf>,
    /// A `--file-types` output to read instead of running the built app, which a cross build cannot.
    file_types: Option<PathBuf>,
    icon: Option<PathBuf>,
    sign: bool,
    /// Fail where the helper program that bakes the game's shaders cannot be built, instead of packaging without it.
    require_helper: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            out: None,
            version: None,
            bin_dir: None,
            file_types: None,
            icon: None,
            sign: true,
            require_helper: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Linux,
    Macos,
    Windows,
}

/// What a package is made from.
struct Inputs {
    version: String,
    openrage: PathBuf,
    cli: PathBuf,
    /// The program that bakes the game's shaders for the editor, which carries the C++ translator and sits beside it; `None` where it could not be built.
    helper: Option<PathBuf>,
    icon_png: Vec<u8>,
    types: Vec<FileType>,
    licences: Vec<PathBuf>,
    /// The folders of GTA VI names the editor reads with no setting, those the tree has.
    names: Vec<PathBuf>,
}

fn package(target: Target, root: &Path, out: &Path, options: &Options) -> Result<Vec<PathBuf>> {
    let host_ok = match target {
        Target::Macos => cfg!(target_os = "macos"),
        Target::Windows => cfg!(windows),
        Target::Linux => cfg!(target_os = "linux"),
    };
    if !host_ok {
        bail!("this package is made on the OS it is for: its tools and its binary are that OS's");
    }
    let version = match &options.version {
        Some(v) => v.clone(),
        None => workspace_version(root)?,
    };
    let (openrage, cli, helper) = match &options.bin_dir {
        Some(dir) => (
            dir.join(bin_name("openrage")),
            dir.join(bin_name("openrage-cli")),
            Some(dir.join(bin_name(HELPER))).filter(|p| p.is_file()),
        ),
        None => build(target, root, out, options.require_helper)?,
    };
    for bin in [&openrage, &cli] {
        if !bin.is_file() {
            bail!(
                "{} is not there; build first or name --bin-dir",
                bin.display()
            );
        }
    }
    let types = match &options.file_types {
        Some(file) => parse_file_types(&std::fs::read_to_string(file)?)?,
        None => ask_for_file_types(&openrage)?,
    };
    let icon_png = match &options.icon {
        Some(path) => std::fs::read(path)?,
        None => std::fs::read(find_icon(root)?).context("reading the application icon")?,
    };
    let licences = ["LICENSE", "LICENSE-third-party.md", "README.md"]
        .iter()
        .map(|name| root.join(name))
        .filter(|p| p.is_file())
        .collect();
    let inputs = Inputs {
        version,
        openrage,
        cli,
        helper,
        icon_png,
        types,
        licences,
        names: (NAMES_FOLDERS
            .iter()
            .map(|name| root.join("assets").join(name)))
        .filter(|p| p.is_dir())
        .collect(),
    };

    match target {
        Target::Linux => linux(&inputs, out),
        Target::Windows => Ok(vec![windows(&inputs, out)?]),
        Target::Macos => macos(&inputs, out, options.sign),
    }
}

/// The program the editor bakes the game's shaders with.
const HELPER: &str = "or-shader-tool";

fn bin_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn workspace_version(root: &Path) -> Result<String> {
    let out = Command::new("cargo")
        .current_dir(root)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .context("could not run `cargo metadata`")?;
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == "openrage")
        .and_then(|p| p["version"].as_str())
        .map(str::to_string)
        .context("the openrage package names no version")
}

// ------------------------------------------------------------------ building

/// Builds the release binaries; on a Mac, once per architecture, joined with `lipo`.
fn build(
    target: Target,
    root: &Path,
    out: &Path,
    require_helper: bool,
) -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let cargo = |extra: &[&str]| -> Result<()> {
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["build", "--release", "-p", "openrage", "--bins"])
            .args(extra)
            .status()
            .context("could not run cargo")?;
        if !status.success() {
            bail!("cargo build failed");
        }
        Ok(())
    };
    // The helper carries a C++ translator, so it builds apart from the app and can fail apart from it
    let helper_cargo = |extra: &[&str]| -> Result<()> {
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["build", "--release", "-p", "or-shader-tool"])
            .args(["--features", "native-front", "--bin", HELPER])
            .args(extra)
            .status()
            .context("could not run cargo")?;
        if !status.success() {
            bail!("cargo build of {HELPER} failed");
        }
        Ok(())
    };
    let target_dir = target_directory()?;

    if target != Target::Macos {
        cargo(&[])?;
        let release = target_dir.join("release");
        let helper = helper_or_none(
            helper_cargo(&[]).map(|()| release.join(bin_name(HELPER))),
            require_helper,
        )?;
        return Ok((
            release.join(bin_name("openrage")),
            release.join(bin_name("openrage-cli")),
            helper,
        ));
    }

    let triples = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
    for triple in triples {
        cargo(&["--target", triple])?;
    }
    let helpers_built = triples
        .iter()
        .try_for_each(|triple| helper_cargo(&["--target", triple]));
    let universal = out.join("universal");
    std::fs::create_dir_all(&universal)?;
    let join = |name: &str| -> Result<()> {
        let status = Command::new("lipo")
            .arg("-create")
            .arg("-output")
            .arg(universal.join(name))
            .args(
                triples
                    .iter()
                    .map(|t| target_dir.join(t).join("release").join(name)),
            )
            .status()
            .context("could not run lipo")?;
        if !status.success() {
            bail!("lipo failed for {name}");
        }
        Ok(())
    };
    for name in ["openrage", "openrage-cli"] {
        join(name)?;
    }
    let helper = helper_or_none(
        helpers_built
            .and_then(|()| join(HELPER))
            .map(|()| universal.join(HELPER)),
        require_helper,
    )?;
    Ok((
        universal.join("openrage"),
        universal.join("openrage-cli"),
        helper,
    ))
}

/// The helper where it was built, or `None` with a warning where it was not, unless it is required.
fn helper_or_none(built: Result<PathBuf>, require: bool) -> Result<Option<PathBuf>> {
    match built {
        Ok(path) => Ok(Some(path)),
        Err(error) if require => Err(error.context("the helper program was required")),
        Err(error) => {
            eprintln!(
                "warning: the package has no {HELPER}, so its editor cannot bake the game's shaders for the viewport's game-shaders mode: {error:#}"
            );
            Ok(None)
        }
    }
}

/// The 256 px icon the build script renders from the brand symbol.
fn find_icon(root: &Path) -> Result<PathBuf> {
    icon_under(target_directory().unwrap_or_else(|_| root.join("target")))
}

fn icon_under(target: PathBuf) -> Result<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack = vec![target];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                // Only the build scripts' output folders: `target` holds hundreds of thousands of files.
                // Cargo names one `build/openrage-<hash>/out`, or `build/openrage/<hash>/out`.
                let wanted = name == "release"
                    || name == "debug"
                    || name == "build"
                    || name == "openrage"
                    || name.starts_with("openrage-")
                    || name == "out"
                    || name.ends_with("-apple-darwin")
                    || dir.file_name().is_some_and(|d| d == "openrage");
                if wanted {
                    stack.push(path);
                }
            } else if name == "app_logo_256.png" {
                found.push(path);
            }
        }
    }
    found.sort();
    found
        .pop()
        .context("no app_logo.png under target; build the app first or pass --icon")
}

// ----------------------------------------------------------------- file types

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileType {
    ext: String,
    noun: String,
    suggested: bool,
}

fn parse_file_types(json: &str) -> Result<Vec<FileType>> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(json).context("--file-types did not print a JSON array")?;
    let types: Vec<FileType> = rows
        .iter()
        .filter_map(|row| {
            Some(FileType {
                ext: row["ext"].as_str()?.to_string(),
                noun: row["noun"].as_str()?.to_string(),
                suggested: row["suggested"].as_bool().unwrap_or(false),
            })
        })
        .collect();
    if types.is_empty() {
        bail!("the app named no file types, so a package would claim none");
    }
    Ok(types)
}

fn ask_for_file_types(openrage: &Path) -> Result<Vec<FileType>> {
    let out = Command::new(openrage)
        .arg("--file-types")
        .output()
        .with_context(|| {
            format!(
                "could not run {} --file-types; pass --file-types FILE",
                openrage.display()
            )
        })?;
    if !out.status.success() {
        bail!("{} --file-types failed", openrage.display());
    }
    parse_file_types(&String::from_utf8_lossy(&out.stdout))
}

/// The formats a package claims: the game's own, never a generic one other programs have the better
/// claim to (`xml`, `txt`, `png`).
fn claimed(types: &[FileType]) -> impl Iterator<Item = &FileType> {
    types.iter().filter(|t| t.suggested)
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ------------------------------------------------------------------- generated

/// The desktop entry: what a launcher lists, which window it belongs to and what it opens.
fn desktop_entry(types: &[FileType]) -> String {
    let mimes: String = claimed(types)
        .map(|t| format!("application/x-openrage-{};", t.ext))
        .collect();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=OpenRage\n\
         GenericName=Asset editor for the RAGE engine\n\
         Comment=Read, edit and write the files GTA V ships\n\
         Exec=openrage %F\n\
         Icon=openrage\n\
         Terminal=false\n\
         Categories=Development;Graphics;\n\
         StartupWMClass={APP_ID}\n\
         MimeType={mimes}\n"
    )
}

/// A shared-mime-info package: one type per claimed extension, so a desktop can offer the app for it.
fn mime_package(types: &[FileType]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for t in claimed(types) {
        let _ = write!(
            out,
            "  <mime-type type=\"application/x-openrage-{ext}\">\n    <comment>{noun}</comment>\n    <glob pattern=\"*.{ext}\"/>\n  </mime-type>\n",
            ext = t.ext,
            noun = xml(&t.noun),
        );
    }
    out.push_str("</mime-info>\n");
    out
}

/// `Info.plist` for the bundle, with a document type for each claimed format.
fn info_plist(version: &str, types: &[FileType]) -> String {
    let mut documents = String::new();
    for t in claimed(types) {
        let _ = write!(
            documents,
            "    <dict>\n      <key>CFBundleTypeName</key><string>{}</string>\n      <key>CFBundleTypeExtensions</key><array><string>{}</string></array>\n      <key>CFBundleTypeRole</key><string>Editor</string>\n      <key>LSHandlerRank</key><string>Alternate</string>\n    </dict>\n",
            xml(&t.noun),
            xml(&t.ext)
        );
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>CFBundleName</key><string>OpenRage</string>\n\
           <key>CFBundleDisplayName</key><string>OpenRage</string>\n\
           <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>\n\
           <key>CFBundleExecutable</key><string>openrage</string>\n\
           <key>CFBundleIconFile</key><string>openrage</string>\n\
           <key>CFBundlePackageType</key><string>APPL</string>\n\
           <key>CFBundleShortVersionString</key><string>{version}</string>\n\
           <key>CFBundleVersion</key><string>{version}</string>\n\
           <key>LSMinimumSystemVersion</key><string>{MACOS_MINIMUM}</string>\n\
           <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>\n\
           <key>NSHighResolutionCapable</key><true/>\n\
           <key>CFBundleDocumentTypes</key>\n\
           <array>\n{documents}  </array>\n\
         </dict>\n\
         </plist>\n"
    )
}

/// An `.icns` holding one 256 px PNG (`ic08`), which macOS reads and scales for the Dock and Finder.
fn icns(png: &[u8]) -> Vec<u8> {
    let chunk = 8 + png.len();
    let total = 8 + chunk;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(b"ic08");
    out.extend_from_slice(&(chunk as u32).to_be_bytes());
    out.extend_from_slice(png);
    out
}

fn install_script() -> &'static str {
    "#!/bin/sh\n\
     # Installs under ~/.local, or under the first argument, then refreshes the desktop's caches.\n\
     set -eu\n\
     here=$(cd \"$(dirname \"$0\")\" && pwd)\n\
     prefix=${1:-$HOME/.local}\n\
     mkdir -p \"$prefix/bin\" \"$prefix/share\"\n\
     cp \"$here/bin/\"* \"$prefix/bin/\"\n\
     cp -R \"$here/share/.\" \"$prefix/share/\"\n\
     command -v update-mime-database >/dev/null 2>&1 && update-mime-database \"$prefix/share/mime\" || true\n\
     command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database \"$prefix/share/applications\" || true\n\
     echo \"OpenRage is in $prefix/bin; the app needs libasound2 and a Vulkan driver.\"\n"
}

// -------------------------------------------------------------------- staging

fn copy_executable(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to.parent().context("a destination has a folder")?)?;
    std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("a file has a folder")?)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn copy_licences(inputs: &Inputs, dir: &Path) -> Result<()> {
    for file in &inputs.licences {
        std::fs::copy(
            file,
            dir.join(file.file_name().context("a licence has a name")?),
        )?;
    }
    Ok(())
}

/// The folders of `assets/` a package ships, each under its own name where the executable looks for it.
const NAMES_FOLDERS: [&str; 2] = ["gta6_names", "gta6_script_names"];

/// Copies each names folder into `to`, where the executable looks for it.
fn copy_names(inputs: &Inputs, to: &Path) -> Result<()> {
    for names in &inputs.names {
        let into = to.join(names.file_name().context("a folder has a name")?);
        std::fs::create_dir_all(&into)?;
        for entry in std::fs::read_dir(names)? {
            let file = entry?.path();
            if file.is_file() {
                std::fs::copy(
                    &file,
                    into.join(file.file_name().context("a file has a name")?),
                )?;
            }
        }
    }
    Ok(())
}

fn fresh(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

fn stage_linux(inputs: &Inputs, dir: &Path) -> Result<()> {
    fresh(dir)?;
    copy_executable(&inputs.openrage, &dir.join("bin/openrage"))?;
    copy_executable(&inputs.cli, &dir.join("bin/openrage-cli"))?;
    if let Some(helper) = &inputs.helper {
        copy_executable(helper, &dir.join("bin").join(HELPER))?;
    }
    write(
        &dir.join("share/applications/openrage.desktop"),
        desktop_entry(&inputs.types),
    )?;
    write(
        &dir.join("share/mime/packages/openrage.xml"),
        mime_package(&inputs.types),
    )?;
    write(
        &dir.join("share/icons/hicolor/256x256/apps/openrage.png"),
        &inputs.icon_png,
    )?;
    write(&dir.join("install.sh"), install_script())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("install.sh"),
            std::fs::Permissions::from_mode(0o755),
        )?;
    }
    copy_names(inputs, &dir.join("share/openrage"))?;
    copy_licences(inputs, dir)
}

/// The staged folder as a Debian package lays it out: the executables in `/usr/bin`, the desktop's files in `/usr/share`, and the licences with the documentation.
fn deb_files(stage: &Path) -> Result<Vec<deb::File>> {
    fn walk(dir: &Path, below: &str, mode: u32, out: &mut Vec<deb::File>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{below}/{name}");
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &path, mode, out)?;
            } else {
                out.push(deb::File {
                    path,
                    mode,
                    bytes: std::fs::read(entry.path())?,
                });
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(&stage.join("bin"), "usr/bin", 0o755, &mut files)?;
    walk(&stage.join("share"), "usr/share", 0o644, &mut files)?;
    // Debian looks for the licence in a file called `copyright`
    for (from, to) in [
        ("LICENSE", "copyright"),
        ("LICENSE-third-party.md", "LICENSE-third-party.md"),
        ("README.md", "README.md"),
    ] {
        if let Ok(bytes) = std::fs::read(stage.join(from)) {
            files.push(deb::File {
                path: format!("usr/share/doc/openrage/{to}"),
                mode: 0o644,
                bytes,
            });
        }
    }
    Ok(files)
}

/// The seconds a package is stamped with: `SOURCE_DATE_EPOCH` where a build sets it, else now.
fn package_time() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        })
}

fn linux(inputs: &Inputs, out: &Path) -> Result<Vec<PathBuf>> {
    let name = format!(
        "openrage-{}-linux-{}",
        inputs.version,
        std::env::consts::ARCH
    );
    let dir = out.join(&name);
    stage_linux(inputs, &dir)?;
    let archive = out.join(format!("{name}.tar.gz"));
    run_tool(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(out)
            .arg(&name),
    )?;
    let package = deb::deb(
        &inputs.version,
        std::env::consts::ARCH,
        &deb_files(&dir)?,
        package_time(),
    )?;
    let debian = out.join(format!(
        "openrage_{}_{}.deb",
        inputs.version,
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            _ => "amd64",
        }
    ));
    std::fs::write(&debian, package)?;
    Ok(vec![archive, debian])
}

fn stage_macos(inputs: &Inputs, app: &Path) -> Result<()> {
    fresh(app)?;
    let contents = app.join("Contents");
    copy_executable(&inputs.openrage, &contents.join("MacOS/openrage"))?;
    copy_executable(&inputs.cli, &contents.join("MacOS/openrage-cli"))?;
    // Beside the editor, which is where it looks for it
    if let Some(helper) = &inputs.helper {
        copy_executable(helper, &contents.join("MacOS").join(HELPER))?;
    }
    write(
        &contents.join("Info.plist"),
        info_plist(&inputs.version, &inputs.types),
    )?;
    write(
        &contents.join("Resources/openrage.icns"),
        icns(&inputs.icon_png),
    )?;
    copy_names(inputs, &contents.join("Resources"))?;
    copy_licences(inputs, &contents.join("Resources"))
}

fn macos(inputs: &Inputs, out: &Path, sign: bool) -> Result<Vec<PathBuf>> {
    let app = out.join("OpenRage.app");
    stage_macos(inputs, &app)?;
    if sign {
        // Ad hoc: a signature with no identity, so the app runs but is not notarised; a first run is
        // a right-click, Open. `--deep` signs the CLI beside the app.
        run_tool(
            Command::new("codesign")
                .args(["--force", "--deep", "--sign", "-"])
                .arg(&app),
        )?;
    }

    let staging = out.join("dmg");
    fresh(&staging)?;
    run_tool(Command::new("cp").arg("-R").arg(&app).arg(&staging))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))?;
    let dmg = out.join(format!("OpenRage-{}-macos.dmg", inputs.version));
    let _ = std::fs::remove_file(&dmg);
    run_tool(
        Command::new("hdiutil")
            .args([
                "create",
                "-volname",
                "OpenRage",
                "-ov",
                "-format",
                "UDZO",
                "-srcfolder",
            ])
            .arg(&staging)
            .arg(&dmg),
    )?;
    Ok(vec![dmg])
}

fn stage_windows(inputs: &Inputs, dir: &Path) -> Result<()> {
    fresh(dir)?;
    copy_executable(&inputs.openrage, &dir.join("openrage.exe"))?;
    copy_executable(&inputs.cli, &dir.join("openrage-cli.exe"))?;
    if let Some(helper) = &inputs.helper {
        copy_executable(helper, &dir.join(format!("{HELPER}.exe")))?;
    }
    copy_names(inputs, dir)?;
    copy_licences(inputs, dir)
}

fn windows(inputs: &Inputs, out: &Path) -> Result<PathBuf> {
    let name = format!("openrage-{}-windows-x64", inputs.version);
    let dir = out.join(&name);
    stage_windows(inputs, &dir)?;
    let archive = out.join(format!("{name}.zip"));
    let _ = std::fs::remove_file(&archive);
    // Windows 10's own `tar` writes a zip when the name says so.
    run_tool(
        Command::new("tar")
            .arg("-a")
            .arg("-c")
            .arg("-f")
            .arg(&archive)
            .arg("-C")
            .arg(out)
            .arg(&name),
    )?;
    Ok(archive)
}

fn run_tool(command: &mut Command) -> Result<()> {
    let shown = format!("{command:?}");
    let status = command
        .status()
        .with_context(|| format!("could not run {shown}"))?;
    if !status.success() {
        bail!("{shown} failed");
    }
    Ok(())
}

// --------------------------------------------------------------------- SHA-256

fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_hex(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    ))
}

/// SHA-256 as FIPS 180-4 states it; written here and not imported because nothing else in this
/// crate needs a hash, and tested against the system's own `shasum`.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());

    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<FileType> {
        parse_file_types(
            r#"[{"ext":"ytd","noun":"Texture dictionary","suggested":true},
                {"ext":"ydr","noun":"Drawable & more","suggested":true},
                {"ext":"xml","noun":"XML","suggested":false}]"#,
        )
        .expect("a list of file types")
    }

    #[test]
    fn the_icon_is_found_in_either_layout_of_a_build_scripts_output() {
        for out in [
            "release/build/openrage-0123abcd/out",
            "release/build/openrage/0123abcd/out",
            "x86_64-apple-darwin/release/build/openrage/0123abcd/out",
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let dir = scratch.path().join(out);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("app_logo_256.png"), b"png").unwrap();
            let found = icon_under(scratch.path().to_path_buf()).expect(out);
            assert_eq!(found, dir.join("app_logo_256.png"));
        }
    }

    #[test]
    fn the_file_types_come_from_the_app_and_a_package_claims_only_the_games_own() {
        let types = types();
        assert_eq!(types.len(), 3);
        let desktop = desktop_entry(&types);
        assert!(
            desktop.contains("MimeType=application/x-openrage-ytd;application/x-openrage-ydr;\n")
        );
        assert!(
            !desktop.contains("xml"),
            "a generic format was claimed: {desktop}"
        );
        assert!(desktop.contains(&format!("StartupWMClass={APP_ID}")));
        assert!(desktop.contains("Exec=openrage %F"));
        assert!(
            parse_file_types("[]").is_err(),
            "a package that claims nothing is a mistake"
        );
    }

    #[test]
    fn the_mime_package_and_the_plist_are_well_formed_and_escape_what_a_noun_holds() {
        let types = types();
        let mime = mime_package(&types);
        assert!(mime.contains(r#"<glob pattern="*.ytd"/>"#));
        assert!(mime.contains("Drawable &amp; more"));
        assert!(!mime.contains("*.xml"));
        let plist = info_plist("1.2.3", &types);
        assert!(plist.contains("<key>CFBundleShortVersionString</key><string>1.2.3</string>"));
        assert!(plist.contains("<string>ytd</string>") && !plist.contains("<string>xml</string>"));
        assert!(plist.contains(BUNDLE_ID));
        assert_eq!(mime.matches("<mime-type ").count(), 2);
        assert_eq!(plist.matches("<key>CFBundleTypeName</key>").count(), 2);
    }

    #[test]
    fn an_icns_is_a_header_and_one_png_chunk_of_the_length_it_says() {
        let png = vec![7u8; 1000];
        let bytes = icns(&png);
        assert_eq!(&bytes[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len()
        );
        assert_eq!(&bytes[8..12], b"ic08");
        assert_eq!(
            u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize,
            8 + png.len()
        );
        assert_eq!(&bytes[16..], png.as_slice());
    }

    /// Stand-ins for the built app, so staging is tested on any OS.
    fn inputs(dir: &Path) -> Inputs {
        let (openrage, cli) = (dir.join("openrage"), dir.join("openrage-cli"));
        std::fs::write(&openrage, b"app").unwrap();
        std::fs::write(&cli, b"cli").unwrap();
        let licence = dir.join("LICENSE");
        std::fs::write(&licence, b"MIT").unwrap();
        Inputs {
            version: "0.1.0".into(),
            openrage,
            cli,
            helper: None,
            icon_png: vec![1, 2, 3],
            types: types(),
            licences: vec![licence],
            names: Vec::new(),
        }
    }

    #[test]
    fn a_linux_package_has_the_binaries_the_desktop_files_the_icon_and_an_installer() {
        let scratch = tempfile::tempdir().unwrap();
        let inputs = inputs(scratch.path());
        let dir = scratch.path().join("stage");
        stage_linux(&inputs, &dir).unwrap();
        for file in [
            "bin/openrage",
            "bin/openrage-cli",
            "share/applications/openrage.desktop",
            "share/mime/packages/openrage.xml",
            "share/icons/hicolor/256x256/apps/openrage.png",
            "install.sh",
            "LICENSE",
        ] {
            assert!(dir.join(file).is_file(), "{file} is missing");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.join("bin/openrage"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0o111
            );
        }
    }

    #[test]
    fn a_debian_package_puts_the_executables_in_usr_bin_the_desktops_files_in_usr_share_and_no_installer(
    ) {
        let scratch = tempfile::tempdir().unwrap();
        let mut inputs = inputs(scratch.path());
        let helper = scratch.path().join(HELPER);
        std::fs::write(&helper, b"helper").unwrap();
        inputs.helper = Some(helper);
        let dir = scratch.path().join("stage");
        stage_linux(&inputs, &dir).unwrap();
        let files = deb_files(&dir).unwrap();
        let mode = |path: &str| {
            files
                .iter()
                .find(|f| f.path == path)
                .unwrap_or_else(|| panic!("{path} is not in the package"))
                .mode
        };
        assert_eq!(mode("usr/bin/openrage"), 0o755);
        assert_eq!(mode("usr/bin/openrage-cli"), 0o755);
        assert_eq!(mode("usr/bin/or-shader-tool"), 0o755);
        assert_eq!(mode("usr/share/applications/openrage.desktop"), 0o644);
        assert_eq!(mode("usr/share/mime/packages/openrage.xml"), 0o644);
        assert_eq!(
            mode("usr/share/icons/hicolor/256x256/apps/openrage.png"),
            0o644
        );
        assert_eq!(mode("usr/share/doc/openrage/copyright"), 0o644);
        assert!(
            files.iter().all(|f| !f.path.contains("install.sh")),
            "a package is its own installer"
        );
    }

    #[test]
    fn a_macos_bundle_has_its_executable_plist_icon_and_licences_where_the_system_looks() {
        let scratch = tempfile::tempdir().unwrap();
        let inputs = inputs(scratch.path());
        let app = scratch.path().join("OpenRage.app");
        stage_macos(&inputs, &app).unwrap();
        for file in [
            "Contents/MacOS/openrage",
            "Contents/MacOS/openrage-cli",
            "Contents/Info.plist",
            "Contents/Resources/openrage.icns",
            "Contents/Resources/LICENSE",
        ] {
            assert!(app.join(file).is_file(), "{file} is missing");
        }
        // The system's own parser, where there is one: a plist that is well-formed XML can still be wrong.
        if cfg!(target_os = "macos") {
            let lint = Command::new("plutil")
                .arg("-lint")
                .arg(app.join("Contents/Info.plist"))
                .output()
                .unwrap();
            assert!(
                lint.status.success(),
                "{}",
                String::from_utf8_lossy(&lint.stdout)
            );
        }
    }

    #[test]
    fn a_windows_package_has_both_executables() {
        let scratch = tempfile::tempdir().unwrap();
        let inputs = inputs(scratch.path());
        let dir = scratch.path().join("stage");
        stage_windows(&inputs, &dir).unwrap();
        assert!(dir.join("openrage.exe").is_file() && dir.join("openrage-cli.exe").is_file());
    }

    /// `inputs` with a stand-in for the helper program beside the editor's.
    fn with_a_helper(dir: &Path) -> Inputs {
        let mut inputs = inputs(dir);
        let helper = dir.join(bin_name(HELPER));
        std::fs::write(&helper, b"helper").unwrap();
        inputs.helper = Some(helper);
        inputs
    }

    #[test]
    fn the_helper_that_bakes_the_games_shaders_sits_beside_the_editor_in_each_package() {
        let scratch = tempfile::tempdir().unwrap();
        let with = with_a_helper(scratch.path());
        let without = inputs(scratch.path());

        let linux = scratch.path().join("linux");
        stage_linux(&with, &linux).unwrap();
        assert_eq!(
            std::fs::read(linux.join("bin/or-shader-tool")).unwrap(),
            b"helper"
        );
        stage_linux(&without, &linux).unwrap();
        assert!(!linux.join("bin/or-shader-tool").exists());

        // The editor looks in the folder of its own executable
        let app = scratch.path().join("OpenRage.app");
        stage_macos(&with, &app).unwrap();
        assert_eq!(
            std::fs::read(app.join("Contents/MacOS/or-shader-tool")).unwrap(),
            b"helper"
        );
        stage_macos(&without, &app).unwrap();
        assert!(!app.join("Contents/MacOS/or-shader-tool").exists());

        let windows = scratch.path().join("windows");
        stage_windows(&with, &windows).unwrap();
        assert_eq!(
            std::fs::read(windows.join("or-shader-tool.exe")).unwrap(),
            b"helper"
        );
        stage_windows(&without, &windows).unwrap();
        assert!(!windows.join("or-shader-tool.exe").exists());
    }

    #[test]
    fn a_helper_that_could_not_be_built_is_left_out_with_a_warning_unless_it_is_required() {
        let built = || Ok(PathBuf::from("target/release/or-shader-tool"));
        assert_eq!(
            helper_or_none(built(), false).unwrap(),
            Some(PathBuf::from("target/release/or-shader-tool"))
        );
        assert_eq!(
            helper_or_none(built(), true).unwrap(),
            Some(PathBuf::from("target/release/or-shader-tool"))
        );
        let failed = || Err(anyhow::anyhow!("no C++ compiler"));
        assert_eq!(helper_or_none(failed(), false).unwrap(), None);
        let error = helper_or_none(failed(), true).unwrap_err();
        assert!(
            format!("{error:#}").contains("no C++ compiler"),
            "{error:#}"
        );
    }

    #[test]
    fn sha256_agrees_with_the_systems_own_over_inputs_that_cross_a_block() {
        let scratch = tempfile::tempdir().unwrap();
        for len in [0usize, 1, 55, 56, 63, 64, 65, 1000] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let file = scratch.path().join(format!("f{len}"));
            std::fs::write(&file, &bytes).unwrap();
            let theirs = Command::new("shasum")
                .args(["-a", "256"])
                .arg(&file)
                .output()
                .or_else(|_| Command::new("sha256sum").arg(&file).output());
            let Ok(theirs) = theirs else {
                return; // no system hash to compare with; the vectors above are the only oracle
            };
            let theirs = String::from_utf8_lossy(&theirs.stdout);
            assert_eq!(
                sha256_hex(&bytes),
                // ⚠ `sha256sum` writes a leading backslash when the file name it was given holds one.
                theirs
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('\\'),
                "{len} bytes"
            );
        }
    }
}
