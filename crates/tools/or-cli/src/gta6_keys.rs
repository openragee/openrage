//! `keys gta6`: read the GTA VI key snapshot out of the user's own executable and write it masked,
//! beside the keys cache where the archive readers look for it. Also the keys the `rpf9` verbs read
//! with, from `--keys`, `--exe` or the environment.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::gta6_extract::extract_snapshot;
use or_io::container::rpf::{
    decode_masked, encode_masked, is_masked, GtaViKeys, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV,
    GTA6_SNAPSHOT_FILE_NAME,
};

/// The keys an `rpf9` verb reads with: the snapshot `--keys` names (unmasked with `--exe` when
/// masked), else the user's snapshot or the keys the executable yields, cached masked beside the
/// keys cache. Without `--exe`, the executable is the one `OPENRAGE_GTA6_EXE` names.
pub(crate) fn load(snapshot: Option<&Path>, exe: Option<&Path>) -> Result<GtaViKeys, String> {
    if let Some(path) = snapshot {
        return match exe {
            Some(exe) => open_with(path, exe),
            None => GtaViKeys::from_file(path).map_err(|e| e.to_string()),
        };
    }
    GtaViKeys::for_install_with(or_title::config::config_dir().as_deref(), exe)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!(
                "No GTA VI keys: pass --exe with the path to game_scarlett_master.exe, pass --keys \
                 with a key snapshot, or set {GTA6_SNAPSHOT_ENV}."
            )
        })
}

/// The snapshot at `path`, unmasked with the executable at `exe` when it is masked.
fn open_with(path: &Path, exe: &Path) -> Result<GtaViKeys, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !is_masked(&bytes) {
        return GtaViKeys::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()));
    }
    let exe_bytes =
        std::fs::read(exe).map_err(|e| format!("cannot read {}: {e}", exe.display()))?;
    let plain =
        decode_masked(&bytes, &exe_bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    GtaViKeys::parse(&plain).map_err(|e| format!("{}: {e}", path.display()))
}

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
                "it opens only with that executable: pass --exe {} to the rpf9 verbs, or set {GTA6_EXE_ENV} to it",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An index-only `ORGTAVIK` snapshot of synthetic tables, with no key.
    fn index_only_snapshot() -> Vec<u8> {
        let mut out = b"ORGTAVIK".to_vec();
        out.extend(1u32.to_le_bytes());
        out.extend(0x4342u32.to_le_bytes());
        out.extend([7u8; 32 + 16]);
        for len in [0x1000u32, 0x4000] {
            out.extend(len.to_le_bytes());
            out.extend((0..len).map(|i| (i % 251) as u8));
        }
        out.extend(0u32.to_le_bytes());
        out
    }

    #[test]
    fn a_masked_keys_file_opens_with_its_executable_and_no_other() {
        let dir = tempfile::tempdir().expect("a folder");
        let exe = dir.path().join("game_scarlett_master.exe");
        let other = dir.path().join("other.exe");
        std::fs::write(&exe, b"the keying executable").expect("write");
        std::fs::write(&other, b"another executable").expect("write");
        let plain = index_only_snapshot();
        GtaViKeys::parse(&plain).expect("the synthetic snapshot parses");
        let masked = dir.path().join("masked.dat");
        std::fs::write(&masked, encode_masked(&plain, b"the keying executable")).expect("write");
        let keys = open_with(&masked, &exe).expect("its executable opens it");
        assert_eq!(keys.exe_sha256(), &[7u8; 32]);
        let err = open_with(&masked, &other)
            .err()
            .expect("another executable is refused");
        assert!(err.contains("masked.dat"), "{err}");

        let plain_file = dir.path().join("plain.dat");
        std::fs::write(&plain_file, &plain).expect("write");
        assert!(
            open_with(&plain_file, &other).is_ok(),
            "a plain snapshot needs no executable"
        );
    }
}
