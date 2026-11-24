//! `keys gta6`: read the GTA VI key snapshot out of the user's own executable and write it masked,
//! beside the keys cache where the archive readers look for it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_extract::extract_snapshot;
use or_io::container::rpf::{encode_masked, GtaViKeys, GTA6_EXE_ENV, GTA6_SNAPSHOT_FILE_NAME};

const USAGE: &str =
    "usage: openrage-cli keys gta6 --exe <game_scarlett_master.exe> [--out <file>] [--force]";

pub(crate) fn run(args: &[String]) -> ExitCode {
    let Some((verb, rest)) = args.split_first() else {
        return usage("keys needs a title: gta6");
    };
    if verb != "gta6" {
        return usage(&format!("keys does not know {verb:?}; it takes gta6"));
    }
    let mut exe = None;
    let mut out = None;
    let mut force = false;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--exe" => exe = it.next().map(PathBuf::from),
            "--out" => out = it.next().map(PathBuf::from),
            "--force" => force = true,
            other => return usage(&format!("unknown argument {other:?}")),
        }
    }
    let Some(exe) = exe else {
        return usage("--exe needs the GTA VI executable");
    };
    let named = out.is_some();
    let Some(out) =
        out.or_else(|| or_title::config::config_dir().map(|dir| dir.join(GTA6_SNAPSHOT_FILE_NAME)))
    else {
        return usage("there is no configuration folder to write to; pass --out");
    };
    // A snapshot already beside the keys cache may be a plaintext one the readers open today.
    if !named && !force && out.exists() {
        eprintln!(
            "Error: {} already exists; name it with --out, or pass --force, to replace it",
            out.display()
        );
        return ExitCode::FAILURE;
    }
    match write_masked(&exe, &out) {
        Ok(len) => {
            println!(
                "wrote the masked GTA VI key snapshot to {} ({len} bytes)",
                out.display()
            );
            println!(
                "it opens only with that executable: set {GTA6_EXE_ENV} to {}",
                exe.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// Extract the snapshot from the executable at `exe_path`, mask it with that executable, write it to `out`.
fn write_masked(exe_path: &Path, out: &Path) -> Result<usize, String> {
    let exe =
        std::fs::read(exe_path).map_err(|e| format!("cannot read {}: {e}", exe_path.display()))?;
    let plain = extract_snapshot(&exe).map_err(|e| format!("{}: {e}", exe_path.display()))?;
    GtaViKeys::parse(&plain).map_err(|e| format!("the extracted snapshot does not parse: {e}"))?;
    let masked = encode_masked(&plain, &exe);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(out, &masked).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(masked.len())
}
