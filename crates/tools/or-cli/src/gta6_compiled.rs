//! `openrage-cli script compiled`: what a decrypted GTA VI script body holds, read as a compiled program.
//!
//! ⚠ The body is the inflated plaintext an archive's content cipher yields (`rpf9 extract`), not
//! the stored entry. A native is named only from the user's own lifted listings (`--listings`, or
//! the table a run with them cached); no name is shipped.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::script::compiled::disasm;
use or_io::gta6::script::compiled::Compiled;
use or_io::gta6::script::native_addresses::NativeAddresses;

pub fn run(args: &[String]) -> ExitCode {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut list_natives = false;
    let mut listing = false;
    let mut listings: Option<PathBuf> = None;
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

fn print_usage() {
    println!(
        "Usage: openrage-cli script compiled [--natives] [--listing] [--listings DIR] BODY..."
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
}
