//! The functions of one program worked on the threads of a pool the caller made, where it made one.
//!
//! ⚠ Only a pool [`pooled`] made is used: a pass recurses as deep as a function's statements
//! nest, past the stack of any other pool's threads, so elsewhere the work stays on the calling thread.

/// The stack each thread of a pool [`pooled`] makes runs on: a pass recurses as deep as a function's statements nest.
pub const STACK: usize = 256 << 20;

#[cfg(feature = "rayon")]
std::thread_local! {
    /// Whether this thread is one of a pool [`pooled`] made.
    static DEEP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `work` on a pool of `threads` threads of [`STACK`], over which [`map_with`] spreads the functions of a program; on the calling thread where no pool is made.
pub fn pooled<R: Send>(threads: usize, work: impl FnOnce() -> R + Send) -> R {
    #[cfg(feature = "rayon")]
    {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .stack_size(STACK)
            .start_handler(|_| DEEP.with(|deep| deep.set(true)))
            .build();
        if let Ok(pool) = pool {
            return pool.install(work);
        }
    }
    let _ = threads;
    work()
}

/// `each` of every item, in the items' order, with a state `init` makes for each run of items one thread works: on the threads of a pool [`pooled`] made where the caller runs on one, else one after another.
pub fn map_with<T: Send, S, R: Send>(
    items: Vec<T>,
    init: impl Fn() -> S + Sync + Send,
    each: impl Fn(&mut S, T) -> R + Sync + Send,
) -> Vec<R> {
    #[cfg(feature = "rayon")]
    if items.len() > 1 && DEEP.with(std::cell::Cell::get) {
        use rayon::prelude::*;
        return items.into_par_iter().map_init(init, each).collect();
    }
    let mut state = init();
    items
        .into_iter()
        .map(|item| each(&mut state, item))
        .collect()
}

/// `a` and `b` at once on a pool [`pooled`] made where the caller runs on one, else `a` then `b`.
pub fn join<A: Send, B: Send>(
    a: impl FnOnce() -> A + Send,
    b: impl FnOnce() -> B + Send,
) -> (A, B) {
    #[cfg(feature = "rayon")]
    if DEEP.with(std::cell::Cell::get) {
        return rayon::join(a, b);
    }
    (a(), b())
}

/// `each` of every item, in the items' order, each item a task of its own where [`map_with`] would spread them, so that a long item holds up no other.
pub fn map<T: Send, R: Send>(items: Vec<T>, each: impl Fn(T) -> R + Sync + Send) -> Vec<R> {
    #[cfg(feature = "rayon")]
    if items.len() > 1 && DEEP.with(std::cell::Cell::get) {
        use rayon::prelude::*;
        return items.into_par_iter().with_max_len(1).map(each).collect();
    }
    items.into_iter().map(each).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_keeps_the_order_of_its_items_in_a_pool_and_out_of_one() {
        let items: Vec<u32> = (0..1000).collect();
        let doubled: Vec<u32> = items.iter().map(|n| n * 2).collect();
        assert_eq!(map(items.clone(), |n| n * 2), doubled);
        assert_eq!(pooled(4, || map(items.clone(), |n| n * 2)), doubled);
        let counted = pooled(4, || {
            map_with(
                items,
                || 0u32,
                |seen, n| {
                    *seen += 1;
                    n * 2 + u32::from(*seen == 0)
                },
            )
        });
        assert_eq!(counted, doubled);
    }
}
