//! `openrage-cli script compiled`: what a decrypted GTA VI script body holds, read as a compiled program.
//!
//! ⚠ The body is the inflated plaintext an archive's content cipher yields (`rpf9 extract`), not
//! the stored entry. A native is named only from the user's own lifted listings (`--listings`, or
//! the table a run with them cached); no name is shipped.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::script::compiled::disasm;
use or_io::gta6::script::compiled::Compiled;
use or_io::gta6::script::compiled_write::{set_float, set_int};
use or_io::gta6::script::native_addresses::NativeAddresses;

/// An edit `--set-int` or `--set-float` asks for: a code offset and the value its push takes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Edit {
    Int(usize, i128),
    Float(usize, f32),
}

pub fn run(args: &[String]) -> ExitCode {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut list_natives = false;
    let mut listing = false;
    let mut listings: Option<PathBuf> = None;
    let mut edits: Vec<Edit> = Vec::new();
    let mut out: Option<PathBuf> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--natives" => list_natives = true,
            "--listing" => listing = true,
            "--listings" => match args.next() {
                Some(dir) => listings = Some(PathBuf::from(dir)),
                None => {
                    eprintln!("Error: --listings needs a folder");
                    return ExitCode::from(2);
                }
            },
            flag @ ("--set-int" | "--set-float") => {
                let Some(e) = args.next().and_then(|a| edit(flag, a)) else {
                    eprintln!("Error: {flag} needs OFFSET=VALUE, the offset a listing prints");
                    return ExitCode::from(2);
                };
                edits.push(e);
            }
            "--set-string" => {
                eprintln!(
                    "Error: --set-string is refused: every string constant of the build's compiled programs names the same 0x28-byte object, so no string text was found to replace"
                );
                return ExitCode::from(2);
            }
            "--out" => match args.next() {
                Some(path) => out = Some(PathBuf::from(path)),
                None => {
                    eprintln!("Error: --out needs a new file");
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("Error: unknown option {other}");
                print_usage();
                return ExitCode::from(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }
    if !edits.is_empty() || out.is_some() {
        let (Some(out), [path], false) = (out, paths.as_slice(), edits.is_empty()) else {
            eprintln!(
                "Error: an edit takes one BODY, at least one --set-int or --set-float, and --out NEW"
            );
            return ExitCode::from(2);
        };
        return match write_edited(path, &edits, &out) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("Error: {}: {why}", path.display());
                ExitCode::FAILURE
            }
        };
    }
    let natives = match load_natives(listings.as_deref()) {
        Ok(natives) => natives,
        Err(why) => {
            eprintln!("Error: {why}");
            return ExitCode::FAILURE;
        }
    };
    let mut failed = false;
    for path in &paths {
        if let Err(why) = show(path, list_natives, listing, natives.as_ref()) {
            eprintln!("Error: {}: {why}", path.display());
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// The natives the listings in `listings` declare, cached in the configuration folder; without a folder, the table cached there last.
fn load_natives(listings: Option<&Path>) -> Result<Option<NativeAddresses>, String> {
    let config = or_title::config::config_dir();
    match listings {
        Some(folder) => NativeAddresses::load(folder, config.as_deref())
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(config.as_deref().and_then(NativeAddresses::cached)),
    }
}

fn show(
    path: &Path,
    list_natives: bool,
    listing: bool,
    natives: Option<&NativeAddresses>,
) -> Result<(), String> {
    let body = std::fs::read(path).map_err(|e| e.to_string())?;
    let program = Compiled::open(&body).map_err(|e| e.to_string())?;
    let h = program.header;
    println!("{}", path.display());
    println!("  program hash  {:#010X}", h.hash);
    println!("  types         {}", h.types);
    println!(
        "  code          {} bytes at body offset {:#X}",
        h.code_len, h.code_start
    );
    println!("  natives       {} stated", h.natives);
    match program.facts() {
        Ok(f) => {
            println!("  instructions  {}", f.instructions);
            println!(
                "  native calls  {} to {} distinct natives",
                f.native_calls, f.natives_found
            );
            if f.natives_found != h.natives as usize {
                println!("  ⚠ the calls name a different count of natives than the header states");
            }
        }
        Err(e) => println!("  decode stopped: {e}"),
    }
    if list_natives {
        let calls = program.natives().map_err(|e| e.to_string())?;
        for n in calls {
            let name = natives.and_then(|t| t.name_of(n.rva)).unwrap_or("");
            let line = format!(
                "  {:#08X}  native RVA {:#010X}  args {}  results {}  {name}",
                n.offset, n.rva, n.args, n.returns
            );
            println!("{}", line.trim_end());
        }
    }
    if listing {
        let out = disasm::listing(&program, natives).map_err(|e| e.to_string())?;
        print!("{}", out.text);
    }
    Ok(())
}

/// `OFFSET=VALUE` of `--set-int` or `--set-float`; an offset or integer may be `0x` hex.
fn edit(flag: &str, arg: &str) -> Option<Edit> {
    let (offset, value) = arg.split_once('=')?;
    let offset = usize::try_from(number(offset)?).ok()?;
    match flag {
        "--set-int" => Some(Edit::Int(offset, number(value)?)),
        _ => value.parse().ok().map(|v| Edit::Float(offset, v)),
    }
}

/// A decimal or `0x` hex integer, with an optional minus: from `i64::MIN` up to `u64::MAX`, never wrapped.
fn number(text: &str) -> Option<i128> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let magnitude = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u64>().ok()?,
    };
    let value = i128::from(magnitude);
    let value = if negative { -value } else { value };
    (value >= i128::from(i64::MIN)).then_some(value)
}

/// Apply every edit to the body at `path` and write it to the new file `out`.
fn write_edited(path: &Path, edits: &[Edit], out: &Path) -> Result<(), String> {
    let mut body = std::fs::read(path).map_err(|e| e.to_string())?;
    for edit in edits {
        let done = match *edit {
            Edit::Int(offset, value) => set_int(&mut body, offset, value),
            Edit::Float(offset, value) => set_float(&mut body, offset, value),
        };
        done.map_err(|e| e.to_string())?;
    }
    let mut file = (std::fs::File::options().write(true).create_new(true))
        .open(out)
        .map_err(|e| format!("{}: {e} (an edit goes only to a new file)", out.display()))?;
    file.write_all(&body).map_err(|e| e.to_string())?;
    println!(
        "Wrote {}: {} edits, {} bytes, the source body's length.",
        out.display(),
        edits.len(),
        body.len()
    );
    println!("{EDITED}");
    Ok(())
}

/// What an edited body is and where it can go.
const EDITED: &str = "This is an edited script body, not an archive. Put it into a patched copy \
with `rpf9 patch ARCHIVE --entry PATH --with BODY --out NEW`: a patched copy keeps every table \
byte-identical, so its table signature still matches and no check read in the game's code rejects \
it, but no run of the game was made, so whether the game runs the edited program is not known.";

fn print_usage() {
    println!(
        "Usage: openrage-cli script compiled [--natives] [--listing] [--listings DIR] BODY..."
    );
    println!(
        "       openrage-cli script compiled BODY --set-int OFFSET=VALUE --set-float OFFSET=VALUE ... --out NEW"
    );
    println!();
    println!(
        "Reads a decrypted GTA VI script body: its header, its instructions and its native calls."
    );
    println!(
        "  --natives        list every native call: code offset, native RVA, argument and result counts, name where known"
    );
    println!(
        "  --listing        print every instruction by function: offset, opcode, mnemonic where one is established, operands"
    );
    println!(
        "  --listings DIR   name natives from the lifted listings in DIR (cached; later runs without it use the cache)"
    );
    println!(
        "  --set-int OFFSET=VALUE    set the integer the push at code OFFSET pushes; it must fit the push's immediate"
    );
    println!(
        "  --set-float OFFSET=VALUE  set the 32-bit float a 4-byte push at code OFFSET pushes"
    );
    println!(
        "  --out NEW        the new file an edited body goes to; a branch, or a value the push cannot hold, is refused"
    );
    println!(
        "  An edit keeps the body's length, so it can go into a patched copy with rpf9 patch."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_reads_its_offset_and_value() {
        assert_eq!(edit("--set-int", "0x1A=-3"), Some(Edit::Int(0x1A, -3)));
        assert_eq!(edit("--set-int", "26=0x10"), Some(Edit::Int(26, 16)));
        assert_eq!(
            edit("--set-int", "0=0xFFFFFFFFFFFFFFFF"),
            Some(Edit::Int(0, u64::MAX.into()))
        );
        assert_eq!(edit("--set-int", "0x1A=-0xFFFFFFFFFFFFFFFF"), None);
        assert_eq!(
            edit("--set-int", "0=-0x8000000000000000"),
            Some(Edit::Int(0, i64::MIN.into()))
        );
        assert_eq!(edit("--set-float", "3=2.5"), Some(Edit::Float(3, 2.5)));
        assert_eq!(edit("--set-int", "3"), None);
        assert_eq!(edit("--set-int", "-1=2"), None);
        assert_eq!(edit("--set-float", "3=x"), None);
    }
}
