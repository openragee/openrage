//! `openrage-cli script compiled`: what a decrypted GTA VI script body holds, read as a compiled program.
//!
//! ⚠ The body is the inflated plaintext an archive's content cipher yields (`rpf9 extract`), not
//! the stored entry. A native is named only from the user's own lifted listings (`--listings`, or
//! the table a run with them cached); no name is shipped. An instruction's mnemonic is the one
//! `script disasm` prints: both read the instruction set of `or_io::gta6::script::opcode`.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::script::compiled::disasm;
use or_io::gta6::script::compiled::Compiled;
use or_io::gta6::script::compiled_write::{set_float, set_int};
use or_io::gta6::script::native_addresses::NativeAddresses;

use super::{failed, misused, note, printed};

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
            "-h" | "--help" | "help" => return printed(USAGE, ExitCode::SUCCESS),
            "--natives" => list_natives = true,
            "--listing" => listing = true,
            "--listings" => match args.next() {
                Some(dir) => listings = Some(PathBuf::from(dir)),
                None => return misused("--listings needs a folder"),
            },
            flag @ ("--set-int" | "--set-float") => {
                let Some(e) = args.next().and_then(|a| edit(flag, a)) else {
                    return misused(&format!(
                        "{flag} needs OFFSET=VALUE, the offset a listing prints"
                    ));
                };
                edits.push(e);
            }
            "--set-string" => {
                return misused(
                    "--set-string is refused: every string constant of the build's compiled programs names the same 0x28-byte object, so no string text was found to replace",
                );
            }
            "--out" => match args.next() {
                Some(path) => out = Some(PathBuf::from(path)),
                None => return misused("--out needs a new file"),
            },
            other if other.starts_with('-') => {
                note(USAGE.trim_end());
                return misused(&format!("unknown option {other}"));
            }
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        note(USAGE.trim_end());
        return ExitCode::from(2);
    }
    if !edits.is_empty() || out.is_some() {
        let (Some(out), [path], false) = (out, paths.as_slice(), edits.is_empty()) else {
            return misused(
                "an edit takes one BODY, at least one --set-int or --set-float, and --out NEW",
            );
        };
        return match write_edited(path, &edits, &out) {
            Ok(said) => printed(&said, ExitCode::SUCCESS),
            Err(why) => failed(&format!("{}: {why}", path.display())),
        };
    }
    let natives = match load_natives(listings.as_deref()) {
        Ok(natives) => natives,
        Err(why) => return failed(&why),
    };
    // One text for the run: a reader that closes the pipe ends it with no line and no failure.
    let mut out = String::new();
    let mut code = ExitCode::SUCCESS;
    for path in &paths {
        if let Err(why) = show(&mut out, path, list_natives, listing, natives.as_ref()) {
            code = failed(&format!("{}: {why}", path.display()));
        }
    }
    printed(&out, code)
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

/// Writes what the body at `path` holds to `out`; what it wrote before a refusal stays.
fn show(
    out: &mut String,
    path: &Path,
    list_natives: bool,
    listing: bool,
    natives: Option<&NativeAddresses>,
) -> Result<(), String> {
    let body = std::fs::read(path).map_err(|e| e.to_string())?;
    let program = Compiled::open(&body).map_err(|e| e.to_string())?;
    let h = program.header;
    let _ = writeln!(out, "{}", path.display());
    let _ = writeln!(out, "  program hash  {:#010X}", h.hash);
    let _ = writeln!(out, "  types         {}", h.types);
    let _ = writeln!(
        out,
        "  code          {} bytes at body offset {:#X}",
        h.code_len, h.code_start
    );
    let _ = writeln!(out, "  natives       {} stated", h.natives);
    match program.facts() {
        Ok(f) => {
            let _ = writeln!(out, "  instructions  {}", f.instructions);
            let _ = writeln!(
                out,
                "  native calls  {} to {} distinct natives",
                f.native_calls, f.natives_found
            );
            if f.natives_found != h.natives as usize {
                out.push_str(
                    "  ⚠ the calls name a different count of natives than the header states\n",
                );
            }
        }
        Err(e) => {
            let _ = writeln!(out, "  decode stopped: {e}");
        }
    }
    if list_natives {
        let calls = program.natives().map_err(|e| e.to_string())?;
        for n in calls {
            let name = natives.and_then(|t| t.name_of(n.rva)).unwrap_or("");
            let line = format!(
                "  {:#08X}  native RVA {:#010X}  args {}  results {}  {name}",
                n.offset, n.rva, n.args, n.returns
            );
            let _ = writeln!(out, "{}", line.trim_end());
        }
    }
    if listing {
        let listed = disasm::listing(&program, natives).map_err(|e| e.to_string())?;
        out.push_str(&listed.text);
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

/// Apply every edit to the body at `path` and write it to the new file `out`; what to say of it.
fn write_edited(path: &Path, edits: &[Edit], out: &Path) -> Result<String, String> {
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
    Ok(format!(
        "Wrote {}: {} edits, {} bytes, the source body's length.\n{EDITED}\n",
        out.display(),
        edits.len(),
        body.len()
    ))
}

/// What an edited body is and where it can go.
const EDITED: &str = "This is an edited script body, not an archive. Put it into a patched copy \
with `rpf9 patch ARCHIVE --entry PATH --with BODY --out NEW`: a patched copy keeps every table \
byte-identical, so its table signature still matches and no check read in the game's code rejects \
it, but no run of the game was made, so whether the game runs the edited program is not known.";

const USAGE: &str = "\
Usage: openrage-cli script compiled [--natives] [--listing] [--listings DIR] BODY...
       openrage-cli script compiled BODY --set-int OFFSET=VALUE --set-float OFFSET=VALUE ... --out NEW

Reads a decrypted GTA VI script body: its header, its instructions and its native calls.
  --natives        list every native call: code offset, native RVA, argument and result counts, name where known
  --listing        print every instruction by function: offset, opcode, mnemonic where one is established, operands
  --listings DIR   name natives from the lifted listings in DIR (cached; later runs without it use the cache)
  --set-int OFFSET=VALUE    set the integer the push at code OFFSET pushes; it must fit the push's immediate
  --set-float OFFSET=VALUE  set the 32-bit float a 4-byte push at code OFFSET pushes
  --out NEW        the new file an edited body goes to; a branch, or a value the push cannot hold, is refused
  An edit keeps the body's length, so it can go into a patched copy with rpf9 patch.

This reads the code with none of the program's tables, so it lists a body `script` refuses.
`script disasm` and `script decompile` read the same file through its tables: functions, types and calls resolved.
";

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
