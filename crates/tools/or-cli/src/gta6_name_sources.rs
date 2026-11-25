use std::path::Path;

use or_io::container::rpf::NameResolver;

pub(crate) fn load(directory: Option<&Path>) -> NameResolver {
    let local = or_title::config::config_dir().map(|root| root.join("gta6_names"));
    NameResolver::load_with_sources(directory, local.as_deref())
}
