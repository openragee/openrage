// wgpu's auto-trait chain (`Device: Send`) is deeper than the default 128 on this nightly.
#![recursion_limit = "256"]
#[macro_use]
mod i18n;
mod app;
mod audio;
mod bink;
#[cfg(test)]
mod default_view_strings;
mod diagnostics;
#[cfg(test)]
mod doc_citations;
#[cfg(test)]
mod doc_comments;
#[cfg(test)]
mod doc_counts;
#[cfg(test)]
mod doc_kinds;
#[cfg(test)]
mod doc_target_sample;
#[cfg(test)]
mod edition_strings;
mod photo;
mod photo_serve;
mod photo_timing;
mod platform;
mod render;
mod self_check;
#[cfg(test)]
mod shell_budget;
mod state;
#[cfg(test)]
mod suppressions;
mod ui;
mod video;

fn main() {
    // ⚠ Before anything starts: a packager asks, and the answer is the whole output.
    if std::env::args().nth(1).as_deref() == Some("--file-types") {
        println!("{}", platform::associations::file_types_json());
        return;
    }

    if std::env::args().nth(1).as_deref() == Some("--photo-serve") {
        diagnostics::install_for_protocol();
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(photo_serve::run(&args));
    }

    diagnostics::install();

    if std::env::args().nth(1).as_deref() == Some("--photo") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(photo::run(&args));
    }

    // ⚠ Before the event loop, because the whole point of it is that no
    // window opens and nothing takes the desktop's focus.
    if let Some(options) = self_check::options_from(std::env::args().skip(1)) {
        std::process::exit(self_check::run(options));
    }

    tracing::info!("Starting OpenRage");
    // ⚠ Before any panel builds the hash index, which reads the user's mined names only once.
    state::read_the_mined_names();

    if let Err(e) = app::run(state::command_line_paths(std::env::args_os().skip(1))) {
        tracing::error!("Application error: {}", e);
        std::process::exit(1);
    }
}
