//! Workers that serve the nearest request first: the queue is what is wanted now, set anew as the
//! camera moves, so a request no longer wanted is never served.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// The soonest each key of `asks` is needed by, where a key is asked for more than once.
pub fn soonest<K: Eq + Hash>(asks: impl Iterator<Item = (K, f32)>) -> HashMap<K, f32> {
    let mut out = HashMap::new();
    for (key, soon) in asks {
        let at = out.entry(key).or_insert(soon);
        *at = at.min(soon);
    }
    out
}

struct Queue<Q> {
    /// What is wanted, each with how far it is from the camera.
    wanted: Vec<(f32, Q)>,
    /// What a worker holds, or has answered and the owner has not taken yet.
    held: HashSet<Q>,
    /// What the answers not taken yet weigh.
    waiting: u64,
    stopped: bool,
}

struct Shared<Q> {
    queue: Mutex<Queue<Q>>,
    woken: Condvar,
    /// The most the answers not taken may weigh before the workers wait for the owner to take some.
    most: u64,
}

/// A pool of workers, each answering one request at a time.
pub struct Pool<Q, A> {
    shared: Arc<Shared<Q>>,
    answers: Receiver<(Q, A, u64, Instant)>,
    /// Answers drawn from the workers that [`Pool::take_soonest`] has not taken yet, each with its weight and when it was answered.
    arrived: Mutex<Vec<(Q, A, u64, Instant)>>,
    /// The answers taken soonest first, the seconds they waited from answered to taken in all, and the most.
    waited: Mutex<(usize, f64, f32)>,
}

/// What `work` gives, or `failed` where it panics: a worker whose read panics answers that one
/// item as one that did not read, and goes on.
// ⚠ Only a build that unwinds gets here: the dev and test profiles abort on a panic on Windows.
pub fn or_on_panic<T>(what: &str, work: impl FnOnce() -> T, failed: T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        tracing::error!(what, "a worker of the world stopped on an internal error");
        failed
    })
}

impl<Q: Copy + Eq + Hash + Send + 'static, A: Default + Send + 'static> Pool<Q, A> {
    /// `workers` threads, each with a state of its own made by `state`, answering with `answer`:
    /// an answer that panics is answered `A::default()`.
    pub fn new<S>(
        workers: usize,
        state: impl Fn() -> S + Send + Sync + 'static,
        answer: impl Fn(&mut S, Q) -> A + Send + Sync + 'static,
    ) -> Self {
        Self::bounded(workers, (u64::MAX, |_| 0), state, answer)
    }

    /// As [`Self::new`], but no worker starts another request while the answers the owner has not taken weigh `most` or more, each weighed by `weigh`: what is read waits on the device's uploads rather than piling up in memory.
    pub fn bounded<S>(
        workers: usize,
        (most, weigh): (u64, fn(&A) -> u64),
        state: impl Fn() -> S + Send + Sync + 'static,
        answer: impl Fn(&mut S, Q) -> A + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                wanted: Vec::new(),
                held: HashSet::new(),
                waiting: 0,
                stopped: false,
            }),
            woken: Condvar::new(),
            most,
        });
        let (send, answers) = mpsc::channel();
        let (state, answer) = (Arc::new(state), Arc::new(answer));
        for _ in 0..workers.max(1) {
            let (shared, send) = (Arc::clone(&shared), send.clone());
            let (state, answer) = (Arc::clone(&state), Arc::clone(&answer));
            std::thread::spawn(move || {
                below_normal();
                let mut own = state();
                loop {
                    let request = {
                        let mut queue = shared.queue.lock().unwrap_or_else(|p| p.into_inner());
                        loop {
                            if queue.stopped {
                                return;
                            }
                            let nearest = (queue.wanted.iter().enumerate())
                                .filter(|(_, (_, q))| !queue.held.contains(q))
                                .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
                                .map(|(at, _)| at);
                            let room = queue.waiting < shared.most;
                            if let (Some(at), true) = (nearest, room) {
                                let (_, request) = queue.wanted.swap_remove(at);
                                queue.held.insert(request);
                                break request;
                            }
                            queue = (shared.woken.wait(queue)).unwrap_or_else(|p| p.into_inner());
                        }
                    };
                    let answered =
                        or_on_panic("a read", || answer(&mut own, request), A::default());
                    let weight = weigh(&answered);
                    let mut queue = shared.queue.lock().unwrap_or_else(|p| p.into_inner());
                    queue.waiting = queue.waiting.saturating_add(weight);
                    drop(queue);
                    if send
                        .send((request, answered, weight, Instant::now()))
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
        Self {
            shared,
            answers,
            arrived: Mutex::new(Vec::new()),
            waited: Mutex::default(),
        }
    }

    /// The answer that arrived whose request `soon` says is needed first, the least soonest.
    // ponytail: every answer waiting is looked at on each take; a heap if a backlog of thousands shows in the frame.
    pub fn take_soonest(&self, soon: impl Fn(&Q) -> f32) -> Option<(Q, A)> {
        let mut arrived = self.arrived.lock().unwrap_or_else(|p| p.into_inner());
        arrived.extend(self.answers.try_iter());
        let first = (arrived.iter().enumerate())
            .min_by(|a, b| soon(&a.1 .0).total_cmp(&soon(&b.1 .0)))
            .map(|(at, _)| at)?;
        let (request, answer, weight, answered) = arrived.swap_remove(first);
        drop(arrived);
        let waited = answered.elapsed().as_secs_f32();
        let mut stats = self.waited.lock().unwrap_or_else(|p| p.into_inner());
        *stats = (
            stats.0 + 1,
            stats.1 + f64::from(waited),
            stats.2.max(waited),
        );
        drop(stats);
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.held.remove(&request);
        queue.waiting = queue.waiting.saturating_sub(weight);
        drop(queue);
        self.shared.woken.notify_all();
        Some((request, answer))
    }

    /// [`Self::want`], less each ask for what `alike` finds held already under another request: what is being read, or was and is not taken.
    pub fn want_unheld<K: Eq + Hash>(&self, mut wanted: Vec<(f32, Q)>, alike: impl Fn(&Q) -> K) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        let held: HashSet<K> = queue.held.iter().map(&alike).collect();
        wanted.retain(|(_, q)| queue.held.contains(q) || !held.contains(&alike(q)));
        queue.wanted = wanted;
        drop(queue);
        self.shared.woken.notify_all();
    }

    /// Ask for nothing, and let go of every answer that arrived and is not taken; what those weighed.
    pub fn forget(&self) -> u64 {
        let mut arrived = self.arrived.lock().unwrap_or_else(|p| p.into_inner());
        arrived.extend(self.answers.try_iter());
        let gone: Vec<(Q, A, u64, Instant)> = arrived.drain(..).collect();
        drop(arrived);
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.wanted.clear();
        let mut weighed = 0;
        for (request, _, weight, _) in &gone {
            queue.held.remove(request);
            queue.waiting = queue.waiting.saturating_sub(*weight);
            weighed += weight;
        }
        drop(queue);
        self.shared.woken.notify_all();
        weighed
    }

    /// Look at each answer that arrived and is not taken yet.
    pub fn each_arrived(&self, mut look: impl FnMut(&Q, &A)) {
        let mut arrived = self.arrived.lock().unwrap_or_else(|p| p.into_inner());
        arrived.extend(self.answers.try_iter());
        for (request, answer, ..) in arrived.iter() {
            look(request, answer);
        }
    }

    /// The answers taken soonest first, the seconds they waited from answered to taken in all, and the most.
    pub fn waited(&self) -> (usize, f64, f32) {
        *self.waited.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Ask for `wanted`, each with its distance, in place of what was asked before.
    pub fn want(&self, wanted: Vec<(f32, Q)>) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.wanted = wanted;
        drop(queue);
        self.shared.woken.notify_all();
    }

    /// The answers that arrived, while `more` says to take another.
    pub fn take(&self, mut more: impl FnMut() -> bool) -> Vec<(Q, A)> {
        let mut taken = Vec::new();
        while more() {
            match self.answers.try_recv() {
                Ok(answer) => taken.push(answer),
                Err(_) => break,
            }
        }
        if taken.is_empty() {
            return Vec::new();
        }
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        for (request, _, weight, _) in &taken {
            queue.held.remove(request);
            queue.waiting = queue.waiting.saturating_sub(*weight);
        }
        drop(queue);
        self.shared.woken.notify_all();
        taken
            .into_iter()
            .map(|(request, answer, ..)| (request, answer))
            .collect()
    }
}

/// Run the calling thread below normal priority: a worker reading while the camera moves then waits for the frame's own thread and the layout's, not they for it.
fn below_normal() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentThread() -> isize;
            fn SetThreadPriority(thread: isize, priority: i32) -> i32;
        }
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        // SAFETY: the handle of the calling thread is a pseudo handle, valid while the thread runs and never closed, and the call reads nothing else.
        let set = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
        if set == 0 {
            tracing::debug!("a world reader's priority was not lowered");
        }
    }
}

impl<Q, A> Drop for Pool<Q, A> {
    fn drop(&mut self) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.stopped = true;
        drop(queue);
        self.shared.woken.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn what_arrived_is_taken_by_how_soon_it_is_needed_now_and_not_as_it_arrived() {
        let pool = Pool::new(1, || (), |_, request: u32| request * 10);
        // Read nearest first: 5 then 7.
        pool.want(vec![(1.0, 5), (2.0, 7)]);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let mut arrived = pool.arrived.lock().unwrap_or_else(|p| p.into_inner());
            arrived.extend(pool.answers.try_iter());
            if arrived.len() == 2 || std::time::Instant::now() > until {
                break;
            }
            drop(arrived);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // The camera turned: 7 is needed first now.
        let soon = |request: &u32| if *request == 7 { 0.0 } else { 1.0 };
        assert_eq!(pool.take_soonest(soon), Some((7, 70)));
        assert_eq!(pool.take_soonest(soon), Some((5, 50)));
        assert_eq!(pool.take_soonest(soon), None);
        assert_eq!(soonest([(1, 3.0), (1, 2.0), (4, 9.0)].into_iter())[&1], 2.0);
    }

    /// Take answers from `pool` until `count` have come, or a few seconds pass.
    fn wait_for(pool: &Pool<u32, u64>, count: usize, got: &mut Vec<u32>) {
        let started = Instant::now();
        while got.len() < count && started.elapsed() < Duration::from_secs(5) {
            got.extend(pool.take(|| true).into_iter().map(|(q, _)| q));
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_bounded_pool_starts_nothing_while_what_waits_to_be_taken_weighs_its_most() {
        let pool: Pool<u32, u64> = Pool::bounded(1, (10, |weight| *weight), || (), |_, _| 6);
        pool.want(vec![(1.0, 1), (2.0, 2), (3.0, 3)]);
        // Two answers weigh 12, past the most of 10: the third waits until they are taken.
        let started = Instant::now();
        while pool.shared.queue.lock().map_or(0, |q| q.waiting) < 12
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(50));
        let queue = pool.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(queue.waiting, 12);
        assert_eq!(queue.wanted.len(), 1, "the third is not started");
        drop(queue);
        let mut got = Vec::new();
        wait_for(&pool, 3, &mut got);
        assert_eq!(got, [1, 2, 3]);
    }

    #[test]
    fn what_is_held_under_one_request_is_not_asked_for_again_under_another_alike() {
        // At its most once one has arrived: nothing after it starts, and it stays held.
        let pool: Pool<(u32, u32), u64> = Pool::bounded(1, (6, |w| *w), || (), |_, _| 6);
        pool.want(vec![(1.0, (7, 256))]);
        let started = Instant::now();
        while pool.shared.queue.lock().map_or(0, |q| q.waiting) < 6
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(2));
        }
        pool.want_unheld(vec![(1.0, (7, 512)), (2.0, (8, 256))], |(key, _)| *key);
        let queue = pool.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(queue.wanted, vec![(2.0, (8, 256))]);
    }

    #[test]
    fn forgetting_asks_for_nothing_and_lets_go_of_what_arrived_with_its_weight() {
        // At its most once two have arrived: what is asked for after them is not started.
        let pool: Pool<u32, u64> = Pool::bounded(1, (12, |weight| *weight), || (), |_, _| 6);
        pool.want(vec![(1.0, 1), (2.0, 2)]);
        let started = Instant::now();
        while pool.shared.queue.lock().map_or(0, |q| q.waiting) < 12
            && started.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(2));
        }
        pool.want(vec![(3.0, 3), (4.0, 4)]);
        assert_eq!(pool.forget(), 12);
        std::thread::sleep(Duration::from_millis(50));
        let queue = pool.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            (queue.waiting, queue.wanted.len(), queue.held.len()),
            (0, 0, 0)
        );
        drop(queue);
        assert_eq!(pool.take_soonest(|_| 0.0), None);
    }

    #[test]
    #[cfg_attr(
        all(windows, debug_assertions),
        ignore = "Cranelift cannot unwind a panic on Windows: run it on a build that unwinds"
    )]
    fn a_reader_that_panics_on_one_body_answers_it_unread_and_its_worker_reads_on() {
        // One worker, so that what comes after the panic is read by the worker that panicked.
        let pool: Pool<u32, Option<u32>> = Pool::new(
            1,
            || (),
            |_, body| {
                assert_ne!(body, 2, "a reader made to panic on body 2");
                Some(body * 10)
            },
        );
        pool.want(vec![(1.0, 1), (2.0, 2), (3.0, 3)]);
        let (mut answered, until) = (Vec::new(), Instant::now() + Duration::from_secs(20));
        while answered.len() < 3 && Instant::now() < until {
            answered.extend(pool.take(|| true));
            std::thread::sleep(Duration::from_millis(1));
        }
        answered.sort_unstable();
        assert_eq!(answered, vec![(1, Some(10)), (2, None), (3, Some(30))]);
    }
}
