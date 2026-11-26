//! Cancellation and progress for a job that runs long enough to need both.
//!
//! Two consumers: `openrage-cli`, which renders progress as a line on stderr,
//! and the editor, which renders it as a bar in a frame. Neither can use the
//! other's, so what is shared is the counting and the stopping rather than the
//! drawing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A flag something else sets when the user asks to stop.
///
/// ⚠ Shared between clones, because the thing that sets it is never the thing
/// that reads it — a console handler, or a button in a frame, against a worker.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// How far a job has got, readable while it runs.
///
/// ⚠ **Counters rather than a stream of events.** A frame needs the current
/// numbers and nothing else, so a channel would make the reader drain thousands
/// of messages to learn one of them, and would grow without bound if it did
/// not.
#[derive(Clone, Default)]
pub struct Progress(Arc<Counters>);

#[derive(Default)]
struct Counters {
    total: AtomicU64,
    attempted: AtomicU64,
    succeeded: AtomicU64,
    skipped: AtomicU64,
    failed: AtomicU64,
    bytes: AtomicU64,
    item: Mutex<String>,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the job is working on now, for a reader that shows one line.
    ///
    /// Set by the work, not by [`run`]; a job with nothing useful to name
    /// does not call this.
    pub fn now_on(&self, item: impl Into<String>) {
        if let Ok(mut slot) = self.0.item.lock() {
            *slot = item.into();
        }
    }

    /// Publish `done` of `total`, for a job that is no loop [`run`] counts, such as a copy in bytes.
    pub fn reached(&self, done: u64, total: u64) {
        self.0.total.store(total, Ordering::Relaxed);
        self.0.attempted.store(done, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let c = &self.0;
        Snapshot {
            total: c.total.load(Ordering::Relaxed),
            attempted: c.attempted.load(Ordering::Relaxed),
            succeeded: c.succeeded.load(Ordering::Relaxed),
            skipped: c.skipped.load(Ordering::Relaxed),
            failed: c.failed.load(Ordering::Relaxed),
            bytes: c.bytes.load(Ordering::Relaxed),
            item: c
                .item
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| String::new()),
        }
    }

    fn begin(&self, total: u64) {
        self.0.total.store(total, Ordering::Relaxed);
    }

    fn record(&self, step: &Step) {
        let c = &self.0;
        c.attempted.fetch_add(1, Ordering::Relaxed);
        match step {
            Step::Done(bytes) => {
                c.succeeded.fetch_add(1, Ordering::Relaxed);
                c.bytes.fetch_add(*bytes, Ordering::Relaxed);
            }
            Step::Skipped => {
                c.skipped.fetch_add(1, Ordering::Relaxed);
            }
            Step::Failed(_) => {
                c.failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The counts at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub total: u64,
    pub attempted: u64,
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes: u64,
    pub item: String,
}

impl Snapshot {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.attempted as f32 / self.total as f32).clamp(0.0, 1.0)
    }
}

/// What one item did.
///
/// ⚠ `Failed` carries a message rather than being an error the caller returns,
/// because returning one is what makes a four-hour run die three hours in on
/// the first malformed file.
pub enum Step {
    Done(u64),
    Skipped,
    Failed(String),
}

/// How many failure messages a report keeps.
///
/// ⚠ The *count* is not capped; the messages are. A run over 300,000 entries
/// where most are unreadable would otherwise hold 300,000 strings to print
/// twenty of.
pub const FAILURES_KEPT: usize = 32;

/// What a finished — or abandoned — job did.
#[derive(Clone, Debug)]
pub struct Report {
    pub counts: Snapshot,
    pub cancelled: bool,
    pub failures: Vec<String>,
    pub elapsed: Duration,
}

impl Report {
    /// One sentence, for a toast or a terminal.
    pub fn summary(&self) -> String {
        let c = &self.counts;
        let mut out = if self.cancelled {
            format!(
                "Stopped after {} of {} — {} written",
                c.attempted, c.total, c.succeeded
            )
        } else {
            format!("{} of {} written", c.succeeded, c.total)
        };
        out.push_str(&format!(" ({})", human_bytes(c.bytes)));
        if c.skipped > 0 {
            out.push_str(&format!(", {} skipped", c.skipped));
        }
        if c.failed > 0 {
            out.push_str(&format!(", {} failed", c.failed));
        }
        out
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Run `each` over every item, counting what fails and stopping when asked.
///
/// ⚠ A per-item failure is **counted, never propagated**; an interrupt is
/// checked **between items** and stops the run with the count so far. A naïve
/// `?` in a loop gets both wrong — it dies on the first bad file and ignores
/// Ctrl-C.
///
/// The iterator is `ExactSizeIterator` so the total comes from the work rather
/// than from a caller who has to remember to declare it.
pub fn run<I, T>(
    items: I,
    cancel: &Cancel,
    progress: &Progress,
    mut each: impl FnMut(T) -> Step,
) -> Report
where
    I: IntoIterator<Item = T>,
    I::IntoIter: ExactSizeIterator,
{
    let started = Instant::now();
    let items = items.into_iter();
    progress.begin(items.len() as u64);

    let mut failures: Vec<String> = Vec::new();
    let mut cancelled = false;

    for item in items {
        // Before the item, not after: a run cancelled at the first check must
        // do no work at all.
        if cancel.is_set() {
            cancelled = true;
            break;
        }
        let step = each(item);
        if let Step::Failed(why) = &step {
            if failures.len() < FAILURES_KEPT {
                failures.push(why.clone());
            }
        }
        progress.record(&step);
    }

    Report {
        counts: progress.snapshot(),
        cancelled,
        failures,
        elapsed: started.elapsed(),
    }
}

/// The workers a job runs side by side on this machine: its cores less the two a frame and the system keep, and one at least.
pub fn workers() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    cores.saturating_sub(2).max(1)
}

/// `each` over `items` on at most `workers` threads, the results in the items' order: `None` for an item reached once `cancel` is set.
// ⚠ A panic of `each` is the caller's: it is raised again on the thread that called.
pub fn side_by_side<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    cancel: &Cancel,
    each: impl Fn(&T) -> R + Sync,
) -> Vec<Option<R>> {
    let next = AtomicUsize::new(0);
    let drain = || {
        let mut mine = Vec::new();
        while !cancel.is_set() {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(at) else { break };
            mine.push((at, each(item)));
        }
        mine
    };
    let threads = workers.min(items.len());
    let done: Vec<(usize, R)> = match threads {
        // One worker is the caller's own thread.
        0 | 1 => drain(),
        _ => std::thread::scope(|scope| {
            let spawned: Vec<_> = (0..threads).map(|_| scope.spawn(drain)).collect();
            (spawned.into_iter())
                .flat_map(|worker| {
                    (worker.join()).unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        }),
    };
    let mut out: Vec<Option<R>> = items.iter().map(|_| None).collect();
    for (at, result) in done {
        out[at] = Some(result);
    }
    out
}

type Task = Box<dyn FnOnce() + Send>;

struct Waiting {
    tasks: VecDeque<Task>,
    /// The threads that run a task now, or are about to look for one.
    alive: usize,
}

struct Shared {
    waiting: Mutex<Waiting>,
    idle: Box<dyn Fn() + Send + Sync>,
}

/// Workers for tasks handed over time: at most `limit` run at once, and `idle` runs each time the last of them finds none left.
pub struct Pool {
    shared: Arc<Shared>,
    limit: usize,
    stack: usize,
}

impl Pool {
    /// A pool of at most `limit` workers, each on a stack of `stack` bytes.
    pub fn new(limit: usize, stack: usize, idle: impl Fn() + Send + Sync + 'static) -> Self {
        let waiting = Waiting {
            tasks: VecDeque::new(),
            alive: 0,
        };
        Self {
            shared: Arc::new(Shared {
                waiting: Mutex::new(waiting),
                idle: Box::new(idle),
            }),
            limit: limit.max(1),
            stack,
        }
    }

    /// Have a worker run `task` once one is free.
    ///
    /// # Errors
    ///
    /// The system gave no thread, and no worker is left to take the task.
    pub fn run(&self, task: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
        let mut waiting = self.shared.waiting();
        waiting.tasks.push_back(Box::new(task));
        if waiting.alive >= self.limit {
            return Ok(());
        }
        waiting.alive += 1;
        let shared = Arc::clone(&self.shared);
        let spawned = (std::thread::Builder::new().name("pool-worker".into()))
            .stack_size(self.stack)
            .spawn(move || shared.drain());
        match spawned {
            Ok(_) => Ok(()),
            Err(error) => {
                waiting.alive -= 1;
                // A worker that runs still takes it; with none, nobody would.
                if waiting.alive > 0 {
                    return Ok(());
                }
                waiting.tasks.pop_back();
                Err(error)
            }
        }
    }
}

impl Shared {
    fn waiting(&self) -> std::sync::MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn drain(&self) {
        loop {
            let next = {
                let mut waiting = self.waiting();
                let next = waiting.tasks.pop_front();
                if next.is_none() {
                    waiting.alive -= 1;
                }
                next.ok_or(waiting.alive == 0)
            };
            match next {
                // ⚠ A task that panics must not take its worker's place in the count with it.
                Ok(task) => drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))),
                Err(last) => {
                    if last {
                        (self.idle)();
                    }
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool of `limit` workers that counts each time it falls idle, and what says so.
    fn counting(limit: usize) -> (Pool, std::sync::mpsc::Receiver<()>) {
        let (fell, idle) = std::sync::mpsc::channel();
        let fell = Mutex::new(fell);
        let pool = Pool::new(limit, 1 << 20, move || {
            let _ = fell.lock().map(|fell| fell.send(()));
        });
        (pool, idle)
    }

    #[test]
    fn a_pool_runs_no_more_tasks_at_once_than_its_limit_and_falls_idle_once_a_batch() {
        let (pool, idle) = counting(2);
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let wait = Arc::new(Mutex::new(wait));
        // The tasks begun, and those ended.
        let counts = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
        for _ in 0..6 {
            let (wait, counts) = (Arc::clone(&wait), Arc::clone(&counts));
            let task = move || {
                counts.0.fetch_add(1, Ordering::SeqCst);
                // Held until every task of the batch is handed over.
                let _ = wait.lock().map(|wait| wait.recv());
                counts.1.fetch_add(1, Ordering::SeqCst);
            };
            pool.run(task).expect("a worker");
        }
        let begun = || counts.0.load(Ordering::SeqCst);
        let started = Instant::now();
        while begun() < 2 && started.elapsed() < Duration::from_secs(20) {
            std::thread::yield_now();
        }
        // Long enough for a third worker to begin a task, were there one.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(begun(), 2, "tasks at once of a limit of 2");
        for _ in 0..6 {
            go.send(()).expect("a task waits");
        }
        idle.recv_timeout(Duration::from_secs(20)).expect("idle");
        assert_eq!(counts.1.load(Ordering::SeqCst), 6, "idle before the last");
        let again = idle.recv_timeout(Duration::from_millis(200));
        assert!(again.is_err(), "idle twice in one batch");
    }

    #[test]
    fn items_run_side_by_side_on_no_more_workers_than_asked_and_come_back_in_order() {
        let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let items: Vec<u32> = (0..24).collect();
        let doubled = side_by_side(&items, 3, &Cancel::new(), |n| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            running.fetch_sub(1, Ordering::SeqCst);
            2 * n
        });
        let expected: Vec<Option<u32>> = items.iter().map(|n| Some(2 * n)).collect();
        assert_eq!(doubled, expected);
        let most = most.load(Ordering::SeqCst);
        assert!((2..=3).contains(&most), "{most} ran at once of 3 asked");
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        assert!(workers() >= 1 && (cores < 3 || workers() + 2 <= cores));
    }

    #[test]
    fn an_item_reached_after_the_stop_is_left_out() {
        let cancel = Cancel::new();
        let done = side_by_side(&[1, 2, 3, 4], 1, &cancel, |n| {
            if *n == 2 {
                cancel.set();
            }
            *n
        });
        assert_eq!(done, vec![Some(1), Some(2), None, None]);
    }

    #[test]
    fn a_flag_is_shared_between_its_clones() {
        let a = Cancel::new();
        let b = a.clone();
        assert!(!a.is_set());
        b.set();
        assert!(
            a.is_set(),
            "the handler holds a clone, so setting one must be visible through \
             the other or the job never sees the interrupt"
        );
    }

    #[test]
    fn a_failing_item_is_counted_and_the_run_continues() {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let items: Vec<u32> = (0..7).collect();

        let report = run(items.clone(), &cancel, &progress, |n| {
            if n % 3 == 0 {
                Step::Failed(format!("item {n} is malformed"))
            } else {
                Step::Done(10)
            }
        });

        assert_eq!(
            report.counts.attempted,
            items.len() as u64,
            "three of seven items failed and the run stopped early; a failure \
             is counted, never propagated"
        );
        assert_eq!(report.counts.failed, 3);
        assert_eq!(report.counts.succeeded, 4);
        assert_eq!(report.counts.bytes, 40);
        assert!(!report.cancelled);
        assert_eq!(report.failures.len(), 3, "each failure states which item");
    }

    #[test]
    fn cancelling_stops_the_run_and_reports_how_far_it_got() {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let stop_after = 4u64;

        let report = run(0..50u32, &cancel, &progress, |_| {
            if progress.snapshot().attempted + 1 >= stop_after {
                cancel.set();
            }
            Step::Done(1)
        });

        assert!(report.cancelled, "the run must report that it was stopped");
        assert_eq!(
            report.counts.attempted, stop_after,
            "the flag was set on item {stop_after}, so that many ran and no more"
        );
        assert!(
            report.counts.attempted > 0,
            "nothing ran at all, so this proves the cancel path and not the \
             loop it is supposed to interrupt"
        );
        assert_eq!(report.counts.succeeded, stop_after);
    }

    #[test]
    fn a_flag_already_set_does_no_work() {
        let cancel = Cancel::new();
        cancel.set();
        let progress = Progress::new();
        let mut ran = 0u32;
        let report = run(0..10u32, &cancel, &progress, |_| {
            ran += 1;
            Step::Done(0)
        });
        assert_eq!(ran, 0, "an item ran after the job was already cancelled");
        assert_eq!(report.counts.total, 10, "the total is still what was asked");
        assert!(report.cancelled);
    }

    #[test]
    fn many_failures_do_not_become_many_strings() {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let n = (FAILURES_KEPT * 4) as u32;

        let report = run(0..n, &cancel, &progress, |i| {
            Step::Failed(format!("item {i} would not read"))
        });

        assert_eq!(
            report.counts.failed, n as u64,
            "every failure is counted, whatever the cap on messages"
        );
        assert_eq!(
            report.failures.len(),
            FAILURES_KEPT,
            "a report that kept one string per failure would hold {n} of them \
             to print twenty"
        );
    }

    #[test]
    fn progress_is_readable_from_another_thread_while_the_job_runs() {
        use std::sync::mpsc::channel;

        let cancel = Cancel::new();
        let progress = Progress::new();
        let (release_tx, release_rx) = channel::<()>();
        let (reached_tx, reached_rx) = channel::<()>();

        let worker_progress = progress.clone();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            run(0..3u32, &worker_cancel, &worker_progress, |i| {
                worker_progress.now_on(format!("item {i}"));
                if i == 1 {
                    let _ = reached_tx.send(());
                    let _ = release_rx.recv();
                }
                Step::Done(100)
            })
        });

        reached_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the worker must reach the second item");
        let mid = progress.snapshot();
        assert_eq!(
            mid.total, 3,
            "the total is published before the first item, or a bar has nothing \
             to divide by for the whole run"
        );
        assert_eq!(
            mid.attempted, 1,
            "one item finished before the second began"
        );
        assert_eq!(mid.item, "item 1", "the reader sees what is in flight");

        let _ = release_tx.send(());
        let report = worker.join().expect("worker");
        assert_eq!(report.counts.attempted, 3);
        assert_eq!(report.counts.bytes, 300);
    }

    #[test]
    fn a_job_no_loop_counts_publishes_how_far_it_has_got() {
        let progress = Progress::new();
        progress.reached(256, 1024);
        let snap = progress.snapshot();
        assert_eq!((snap.attempted, snap.total), (256, 1024));
        assert_eq!(snap.fraction(), 0.25);
    }

    #[test]
    fn a_fraction_of_nothing_is_not_a_division_by_zero() {
        let progress = Progress::new();
        let report = run(Vec::<u32>::new(), &Cancel::new(), &progress, |_| {
            Step::Done(0)
        });
        assert_eq!(report.counts.fraction(), 0.0);
        assert_eq!(report.counts.total, 0);
        assert!(report.summary().contains("0 of 0"));
    }

    #[test]
    fn a_stopped_run_says_so_in_its_summary() {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let report = run(0..8u32, &cancel, &progress, |i| {
            if i == 2 {
                cancel.set();
            }
            Step::Done(2048)
        });
        let summary = report.summary();
        assert!(
            summary.starts_with("Stopped after 3 of 8"),
            "a cancelled run must not read like a finished one: {summary}"
        );
        assert!(summary.contains("6.0 KiB"), "{summary}");
    }
}
