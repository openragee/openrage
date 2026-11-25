//! The one clock and the one random source of what moves from frame to frame: the wall's, or, while
//! a shots run fixes them, a fixed step a frame and a generator started from a seed.
//!
//! ⚠ An `Instant` from [`now`] is measured against another from [`now`], never with
//! `Instant::elapsed`: a fixed clock runs ahead of or behind the wall's.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// What a fixed clock stands on: when it was fixed and how long one frame is.
struct Fixed {
    at: Instant,
    step: Duration,
}

static FIXED: OnceLock<Fixed> = OnceLock::new();
/// The frames a fixed clock has drawn.
static FRAMES: AtomicU64 = AtomicU64::new(0);
/// The frame a fixed clock's game time last started again at.
static GAME_FROM: AtomicU64 = AtomicU64::new(0);
/// The generator's state.
static RANDOM: AtomicU64 = AtomicU64::new(0);
/// When the wall's game time began: the first read of it.
static WALL_START: OnceLock<Instant> = OnceLock::new();

/// Fix the clock for the rest of the process at `step` a frame, each frame counted by [`tick`], and start [`random`] again from `seed`.
pub fn fix(step: Duration, seed: u64) {
    let _ = FIXED.set(Fixed {
        at: Instant::now(),
        step,
    });
    RANDOM.store(seed, Ordering::Relaxed);
}

/// Whether a shots run has fixed the clock.
pub fn is_fixed() -> bool {
    FIXED.get().is_some()
}

/// Move a fixed clock on by one frame; nothing on the wall's.
pub fn tick() {
    if is_fixed() {
        FRAMES.fetch_add(1, Ordering::Relaxed);
    }
}

/// The frame time: the wall's, or the instant the clock was fixed plus a step a frame drawn since.
pub fn now() -> Instant {
    match FIXED.get() {
        Some(fixed) => fixed.at + steps(fixed.step, FRAMES.load(Ordering::Relaxed)),
        None => Instant::now(),
    }
}

/// The game time in seconds, which wind, waves and twinkling run on: the wall's since it was first read, or a step a frame since [`restart_game`].
pub fn game_seconds() -> f64 {
    match FIXED.get() {
        Some(fixed) => {
            let frames = FRAMES.load(Ordering::Relaxed) - GAME_FROM.load(Ordering::Relaxed);
            steps(fixed.step, frames).as_secs_f64()
        }
        None => WALL_START.get_or_init(Instant::now).elapsed().as_secs_f64(),
    }
}

/// Start a fixed clock's game time again from nought, so that a frame drawn a given number of frames later stands at one time whatever came before; nothing on the wall's.
pub fn restart_game() {
    GAME_FROM.store(FRAMES.load(Ordering::Relaxed), Ordering::Relaxed);
}

fn steps(step: Duration, frames: u64) -> Duration {
    step.saturating_mul(u32::try_from(frames).unwrap_or(u32::MAX))
}

/// The next of 64 random bits: SplitMix64 from the seed a fixed clock was given, else from nought.
// ⚠ Drawn by threads that race, the same seed still gives two runs apart: draw where the order is fixed.
pub fn random() -> u64 {
    const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut z = RANDOM
        .fetch_add(GOLDEN, Ordering::Relaxed)
        .wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A random number from 0 up to 1, never 1.
pub fn random_unit() -> f64 {
    (random() >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // ⚠ One test, because the clock is the process's: once fixed it stays fixed.
    #[test]
    fn a_fixed_clock_moves_a_step_a_frame_and_its_random_from_its_seed() {
        let wall = now();
        assert!(!is_fixed());
        tick();
        assert!(now() >= wall, "the wall's clock runs as the wall does");

        fix(Duration::from_millis(16), 7);
        let (first, game) = (now(), game_seconds());
        tick();
        tick();
        assert_eq!(now().duration_since(first), Duration::from_millis(32));
        assert!((game_seconds() - game - 0.032).abs() < 1e-9);
        restart_game();
        assert_eq!(
            game_seconds(),
            0.0,
            "a restart puts the game time at nought"
        );
        tick();
        assert!((game_seconds() - 0.016).abs() < 1e-9);

        let drawn: Vec<u64> = (0..4).map(|_| random()).collect();
        fix(Duration::from_millis(16), 7);
        let again: Vec<u64> = (0..4).map(|_| random()).collect();
        assert_eq!(drawn, again, "one seed, one sequence");
        assert!(drawn.windows(2).all(|w| w[0] != w[1]));
        assert!((0..100)
            .map(|_| random_unit())
            .all(|u| (0.0..1.0).contains(&u)));
    }
}
