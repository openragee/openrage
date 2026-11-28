//! The sun's shadow on a GTA VI model traced by one of the game's own walks, translated and run as
//! a compute pass over the depth target: the arena it walks, the pass, and the governor of its rays.

pub mod arena;
pub mod governor;
pub mod pass;
pub mod query;

pub use arena::SceneArena;
pub use governor::{Governor, Rate, SAMPLES};
pub use pass::{
    cast_held_off, cast_of, Cast, Frame, PixelView, Role, TracedMask, View, WalkModule,
};

/// The variable the developer's aids are read from, which [`traced_aid`] reads too.
const AIDS: &str = "OPENRAGE_GTA6_SHADED";
/// The stack a worker builds on: the validator and a device's compiler recurse a frame for every
/// nested block of a walk's module.
const BUILD_STACK: usize = 64 << 20;

/// The one worker an arena or a traced pass is built on, so that no frame waits for either.
pub fn worker() -> &'static or_job::Pool {
    static WORKER: std::sync::OnceLock<or_job::Pool> = std::sync::OnceLock::new();
    WORKER.get_or_init(|| or_job::Pool::new(1, BUILD_STACK, || {}))
}

/// The walk file the aid `traced` of `stated` names, empty where it names none; `None` without the aid.
pub fn traced_of(stated: &str) -> Option<String> {
    let words = stated.split(',').map(str::trim);
    let mut asked = None;
    for word in words {
        match word.split_once('=') {
            Some(("traced", file)) => asked = Some(file.to_owned()),
            None if word == "traced" => asked = Some(String::new()),
            _ => {}
        }
    }
    asked
}

/// The developer's aid `traced`, or `traced=FILE`, of `OPENRAGE_GTA6_SHADED`: the sun's mask is written by the game's walk where the device runs it.
pub fn traced_aid() -> Option<String> {
    static ASKED: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    (ASKED.get_or_init(|| traced_of(&std::env::var(AIDS).unwrap_or_default()))).clone()
}

/// Whether the aids of `stated` have the traced walk test its nodes by the device's ray query where the device offers one: `traced` without `no-query` or `plain-device`.
pub fn query_of(stated: &str) -> bool {
    let mut words = stated.split(',').map(str::trim);
    traced_of(stated).is_some() && !words.any(|word| word == "no-query" || word == "plain-device")
}

/// [`query_of`] the developer's aids of `OPENRAGE_GTA6_SHADED`.
pub fn query_aid() -> bool {
    static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ASKED.get_or_init(|| query_of(&std::env::var(AIDS).unwrap_or_default()))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod watchdog_tests;
