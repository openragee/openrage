//! The app's checks of the names cache it shares with the CLI, which
//! `NameResolver::load_cached` keeps in the user's configuration.
#![cfg(test)]

mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use or_io::container::rpf::NameResolver;

    #[test]
    fn the_resolver_is_built_once_a_run() {
        use super::super::{resolver, warm_names};
        warm_names();
        let calls: Vec<_> = (0..4).map(|_| std::thread::spawn(resolver)).collect();
        let first = resolver();
        for call in calls {
            let other = call.join().expect("a call returns");
            assert!(std::sync::Arc::ptr_eq(&first, &other), "built again");
        }
        assert!(std::sync::Arc::ptr_eq(&first, &resolver()), "built again");
    }

    /// Env-gated: the names folder `OPENRAGE_GTA6_NAMES` names, loaded cold into an empty
    /// configuration and then warm from the cache that kept, which reads no source and hashes no
    /// name; prints both times.
    #[test]
    fn the_names_load_warm_from_the_cache_faster_than_cold() {
        let Some(dir) = std::env::var_os(or_io::container::rpf::GTA6_NAMES_ENV) else {
            or_test_support::skip_install(
                "the_names_load_warm_from_the_cache_faster_than_cold",
                "set OPENRAGE_GTA6_NAMES",
            );
            return;
        };
        let (dir, config) = (PathBuf::from(dir), tempfile::tempdir().expect("config"));
        let started = Instant::now();
        let cold = NameResolver::load_cached(Some(&dir), Some(config.path()));
        let cold_took = started.elapsed();
        let read = NameResolver::sources_read_on_this_thread();
        let hashed = NameResolver::names_hashed_on_this_thread();
        let started = Instant::now();
        let warm = NameResolver::load_cached(Some(&dir), Some(config.path()));
        let warm_took = started.elapsed();
        println!(
            "names load: {} names, cold {cold_took:?}, warm {warm_took:?}",
            cold.len()
        );
        assert_eq!(NameResolver::sources_read_on_this_thread(), read);
        assert_eq!(NameResolver::names_hashed_on_this_thread(), hashed);
        assert_eq!(warm.len(), cold.len());
        assert_eq!(warm.vocabulary().counts(), cold.vocabulary().counts());
        assert!(
            warm_took < cold_took,
            "warm {warm_took:?}, cold {cold_took:?}"
        );
    }
}
