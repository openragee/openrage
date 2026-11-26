//! Whether GTA VI's keys are held and how many names the resolver holds, asked on a worker so a
//! frame never waits on unmasking the keys or reading the names.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::state::rpf9_scan::rpf9_table;

/// What the app knows of GTA VI's keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Gta6Keys {
    /// Not known yet: a worker is asking.
    Asking,
    /// The keys are held; `names` is how many names the resolver holds once it is built.
    Held { names: Option<usize> },
    /// No keys, and the sentence that says why.
    Missing(String),
}

/// How long an answer stands before it is asked again: the install folder set may change.
const AGAIN: Duration = Duration::from_secs(2);

static ANSWER: Mutex<(Option<Instant>, Gta6Keys)> = Mutex::new((None, Gta6Keys::Asking));
static ASKING: AtomicBool = AtomicBool::new(false);

fn answer() -> MutexGuard<'static, (Option<Instant>, Gta6Keys)> {
    ANSWER.lock().unwrap_or_else(|p| p.into_inner())
}

/// What is known of GTA VI's keys now; a worker asks again once the answer is [`AGAIN`] old.
pub(crate) fn gta6_keys() -> Gta6Keys {
    let mut held = answer();
    let stale = held.0.is_none_or(|at| at.elapsed() >= AGAIN);
    if stale && !ASKING.swap(true, Ordering::Relaxed) {
        held.0 = Some(Instant::now());
        std::thread::spawn(|| {
            let now = match rpf9_table::keys() {
                Err(why) => Gta6Keys::Missing(why),
                Ok(_) => {
                    // The count said before stands while the resolver is built.
                    let before = match &answer().1 {
                        Gta6Keys::Held { names } => *names,
                        _ => None,
                    };
                    answer().1 = Gta6Keys::Held { names: before };
                    let names = Some(rpf9_table::resolver().len());
                    Gta6Keys::Held { names }
                }
            };
            answer().1 = now;
            ASKING.store(false, Ordering::Relaxed);
        });
    }
    held.1.clone()
}
