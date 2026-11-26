use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    // Only the real command line reads the user's mined names; a test running a verb never does.
    or_cli::read_mined_parser_names();
    or_cli::run(&args[1..])
}
