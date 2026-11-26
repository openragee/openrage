use std::path::{Path, PathBuf};

use or_io::container::rpf::{NameResolver, GTA6_NAMES_ENV};
use or_io::gta6::names::Vocabulary;

/// The names of `directory`, or the folder [`GTA6_NAMES_ENV`] names, and of the user's harvest, through the cache the app keeps, then the bundled words.
pub(crate) fn load(directory: Option<&Path>) -> NameResolver {
    // A test that runs a verb in process never reads or rewrites the user's own cache.
    let config = if cfg!(test) {
        None
    } else {
        or_title::config::config_dir()
    };
    load_from(directory, config.as_deref())
}

fn load_from(directory: Option<&Path>, config: Option<&Path>) -> NameResolver {
    let external = std::env::var_os(GTA6_NAMES_ENV).map(PathBuf::from);
    let mut names = NameResolver::load_cached(directory.or(external.as_deref()), config);
    names.merge_vocabulary(&Vocabulary::bundled());
    names
}

#[cfg(test)]
mod tests {
    use std::fs;

    use or_io::gta6::hash::name_hash64;

    use super::*;

    #[test]
    fn a_cache_the_app_kept_is_read_without_reading_the_vocabulary() {
        let names = tempfile::tempdir().expect("names");
        let config = tempfile::tempdir().expect("config");
        let vocabulary = names.path().join("vocabulary.txt");
        fs::write(&vocabulary, "x/one.ydr\nx/two.ytd\n").expect("write");
        // What the app's resolver keeps.
        NameResolver::load_cached(Some(names.path()), Some(config.path()));

        let before = NameResolver::sources_read_on_this_thread();
        let loaded = load_from(Some(names.path()), Some(config.path()));
        assert_eq!(NameResolver::sources_read_on_this_thread(), before);
        assert_eq!(loaded.lookup(name_hash64(b"x/two.ytd")), Some("x/two.ytd"));
        assert_eq!(loaded.lookup(name_hash64(b"diffuse")), Some("diffuse"));

        fs::write(&vocabulary, "x/one.ydr\nx/three.ybn\n").expect("change");
        let loaded = load_from(Some(names.path()), Some(config.path()));
        assert_eq!(NameResolver::sources_read_on_this_thread(), before + 1);
        assert_eq!(
            loaded.lookup(name_hash64(b"x/three.ybn")),
            Some("x/three.ybn")
        );
    }
}
