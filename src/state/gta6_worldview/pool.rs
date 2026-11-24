//! Workers that serve the nearest request first: the queue is what is wanted now, set anew as the
//! camera moves, so a request no longer wanted is never served.

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};

struct Queue<Q> {
    /// What is wanted, each with how far it is from the camera.
    wanted: Vec<(f32, Q)>,
    /// What a worker holds, or has answered and the owner has not taken yet.
    held: HashSet<Q>,
    stopped: bool,
}

struct Shared<Q> {
    queue: Mutex<Queue<Q>>,
    woken: Condvar,
}

/// A pool of workers, each answering one request at a time.
pub(super) struct Pool<Q, A> {
    shared: Arc<Shared<Q>>,
    answers: Receiver<(Q, A)>,
}

impl<Q: Copy + Eq + Hash + Send + 'static, A: Send + 'static> Pool<Q, A> {
    /// `workers` threads, each with a state of its own made by `state`, answering with `answer`.
    pub(super) fn new<S>(
        workers: usize,
        state: impl Fn() -> S + Send + Sync + 'static,
        answer: impl Fn(&mut S, Q) -> A + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                wanted: Vec::new(),
                held: HashSet::new(),
                stopped: false,
            }),
            woken: Condvar::new(),
        });
        let (send, answers) = mpsc::channel();
        let (state, answer) = (Arc::new(state), Arc::new(answer));
        for _ in 0..workers.max(1) {
            let (shared, send) = (Arc::clone(&shared), send.clone());
            let (state, answer) = (Arc::clone(&state), Arc::clone(&answer));
            std::thread::spawn(move || {
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
                            if let Some(at) = nearest {
                                let (_, request) = queue.wanted.swap_remove(at);
                                queue.held.insert(request);
                                break request;
                            }
                            queue = (shared.woken.wait(queue)).unwrap_or_else(|p| p.into_inner());
                        }
                    };
                    // A panic answers nothing: the request stays held and is not asked again.
                    let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        answer(&mut own, request)
                    }));
                    if let Ok(answered) = answered {
                        if send.send((request, answered)).is_err() {
                            return;
                        }
                    }
                }
            });
        }
        Self { shared, answers }
    }

    /// Ask for `wanted`, each with its distance, in place of what was asked before.
    pub(super) fn want(&self, wanted: Vec<(f32, Q)>) {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.wanted = wanted;
        drop(queue);
        self.shared.woken.notify_all();
    }

    /// The answers that arrived, while `more` says to take another.
    pub(super) fn take(&self, mut more: impl FnMut() -> bool) -> Vec<(Q, A)> {
        let mut taken = Vec::new();
        while more() {
            match self.answers.try_recv() {
                Ok(answer) => taken.push(answer),
                Err(_) => break,
            }
        }
        if !taken.is_empty() {
            let mut queue = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
            for (request, _) in &taken {
                queue.held.remove(request);
            }
        }
        taken
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
