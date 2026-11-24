//! A script run in a child process, which its parent ends where a frame does not finish.

use std::io::BufRead;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Set in the child, which draws; the process without it watches.
const CHILD: &str = "OPENRAGE_SHOTS_CHILD";
/// The seconds a frame may take, which also asks for the watch on a hardware adapter.
const LIMIT: &str = "OPENRAGE_FRAME_LIMIT";
/// A software adapter compiles a translated program of thirty thousand lines in minutes.
const SOFTWARE_LIMIT: Duration = Duration::from_secs(600);
/// What the child writes once a frame is drawn; the parent shows every other line.
const BEAT: &str = "\u{1}frame";

/// The seconds `stated` names, or the software adapter's limit where it was asked for.
fn limit_of(stated: Option<&str>, software: bool) -> Option<Duration> {
    let stated = stated.and_then(|seconds| seconds.trim().parse().ok());
    stated
        .map(Duration::from_secs)
        .or(software.then_some(SOFTWARE_LIMIT))
}

/// How long this process waits for a frame of a child it runs the script in, or `None` where it
/// draws the script itself.
pub(super) fn limit() -> Option<Duration> {
    if std::env::var_os(CHILD).is_some() {
        return None;
    }
    limit_of(
        std::env::var(LIMIT).ok().as_deref(),
        or_renderer::instance::software_asked(),
    )
}

/// Tells a watching parent that a frame is drawn.
pub(super) fn beat() {
    if std::env::var_os(CHILD).is_some() {
        println!("{BEAT}");
    }
}

/// What became of a child, from the lines it wrote.
#[derive(Debug, PartialEq)]
enum Ending {
    /// Its lines ended: it left by itself.
    Left,
    /// No frame for the limit, during the step last shown.
    Stuck(String),
}

/// Shows the child's `lines` until they end or none comes for `limit`.
fn watch(lines: &mpsc::Receiver<String>, limit: Duration) -> Ending {
    let mut step = String::new();
    loop {
        match lines.recv_timeout(limit) {
            Ok(line) if line == BEAT => {}
            Ok(line) => {
                println!("{line}");
                if line.starts_with("> ") {
                    step = line;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ending::Left,
            Err(mpsc::RecvTimeoutError::Timeout) => return Ending::Stuck(step),
        }
    }
}

/// This program run again with the same arguments as the child that draws; the exit code.
pub(super) fn run(limit: Duration) -> i32 {
    let mut command = match std::env::current_exe() {
        Ok(program) => Command::new(program),
        Err(e) => {
            println!("cannot find this program to run the script in a child: {e}");
            return 1;
        }
    };
    command.args(std::env::args_os().skip(1)).env(CHILD, "1");
    // The child writes where this process does, which removes a scratch folder of its own.
    if let Some(root) = crate::state::prefs::config_root() {
        command.env("OPENRAGE_CONFIG_DIR", root);
    }
    let mut child = match command.stdout(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) => {
            println!("cannot run the script in a child: {e}");
            return 1;
        }
    };
    let (send, lines) = mpsc::channel();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
    }
    match watch(&lines, limit) {
        Ending::Stuck(step) => {
            let _ = child.kill();
            let _ = child.wait();
            println!(
                "NOT FINISHED: no frame was drawn for {} s during `{}`: the run was ended",
                limit.as_secs(),
                match step.trim_start_matches("> ") {
                    "" => "the start",
                    step => step,
                }
            );
            1
        }
        Ending::Left => match child.wait() {
            Ok(status) if status.success() => 0,
            Ok(status) => {
                println!("the run ended by itself: {status}");
                status.code().unwrap_or(1)
            }
            Err(e) => {
                println!("the run's end was not read: {e}");
                1
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_is_the_one_stated_or_the_software_adapters() {
        assert_eq!(limit_of(None, false), None);
        assert_eq!(limit_of(None, true), Some(SOFTWARE_LIMIT));
        assert_eq!(limit_of(Some("5"), true), Some(Duration::from_secs(5)));
        assert_eq!(limit_of(Some("5"), false), Some(Duration::from_secs(5)));
    }

    #[test]
    fn a_child_that_stops_writing_frames_is_stuck_at_its_last_step_and_one_that_ends_has_left() {
        let (send, lines) = mpsc::channel();
        for line in ["> open a", BEAT, "said", "> shot b", BEAT] {
            send.send(line.to_owned()).unwrap();
        }
        let stuck = watch(&lines, Duration::from_millis(50));
        assert_eq!(stuck, Ending::Stuck("> shot b".to_owned()));
        send.send(BEAT.to_owned()).unwrap();
        drop(send);
        assert_eq!(watch(&lines, Duration::from_millis(50)), Ending::Left);
    }
}
