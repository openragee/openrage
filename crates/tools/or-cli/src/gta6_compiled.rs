//! `openrage-cli script compiled`: what a decrypted GTA VI script body holds, read as a compiled program.
//!
//! ⚠ The body is the inflated plaintext an archive's content cipher yields (`rpf9 extract`), not
//! the stored entry. A native is printed by its RVA only: no name is read or shipped.

use std::path::PathBuf;
use std::process::ExitCode;

use or_io::gta6::script::compiled::Compiled;

pub fn run(args: &[String]) -> ExitCode {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut list_natives = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "--natives" => list_natives = true,
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
    let mut failed = false;
    for path in &paths {
        if let Err(why) = show(path, list_natives) {
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

fn show(path: &std::path::Path, list_natives: bool) -> Result<(), String> {
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
        let natives = program.natives().map_err(|e| e.to_string())?;
        for n in natives {
            println!(
                "  {:#08X}  native RVA {:#010X}  args {}  results {}",
                n.offset, n.rva, n.args, n.returns
            );
        }
    }
    Ok(())
}

fn print_usage() {
    println!("Usage: openrage-cli script compiled [--natives] BODY...");
    println!();
    println!(
        "Reads a decrypted GTA VI script body: its header, its instructions and its native calls."
    );
    println!(
        "  --natives   list every native call: code offset, native RVA, argument and result counts"
    );
}
