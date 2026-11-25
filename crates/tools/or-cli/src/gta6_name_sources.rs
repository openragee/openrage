use std::path::Path;

use or_io::container::rpf::{Gta6Archive, NameResolver};

pub(crate) fn load(directory: Option<&Path>) -> NameResolver {
    let local = or_title::config::config_dir().map(|root| root.join("gta6_names"));
    NameResolver::load_with_sources(directory, local.as_deref())
}

pub(crate) fn resolve(archive: &mut Gta6Archive, names: &NameResolver) {
    archive.resolve_names(names);
    if let Err(error) = names.cache_local_entries(&archive.entries) {
        tracing::warn!(%error, "could not update the local GTA VI names cache");
    }
}
