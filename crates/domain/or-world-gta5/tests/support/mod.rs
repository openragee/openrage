//! The schema a world suite reads with, installed at run time.

#![allow(dead_code)]

use or_io::meta::binary::hash_index::HashIndex;
use or_io::meta::SchemaLibrary;

/// The default library, with the staged snapshot installed when there is one.
pub fn schema() -> &'static SchemaLibrary {
    // ⚠ Once a process: finding the snapshot walks the directories above
    // the workspace, and the library is asked for once a field.
    static INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| {
        if let Some(path) = or_test_support::schema_snapshot() {
            if let Err(e) = or_io::meta::install_snapshot(&path) {
                panic!(
                    "the staged schema snapshot {} is unusable: {e}",
                    path.display()
                );
            }
        }
    });
    or_io::meta::default_library()
}

/// The default hash index, after [`schema`] has had its chance to install.
pub fn hash_index() -> &'static HashIndex {
    schema();
    or_io::meta::binary::hash_index::default_hash_index()
}

/// The installed schema, or `None` having recorded that `what` skipped.
pub fn schema_or_skip(what: &str) -> Option<&'static SchemaLibrary> {
    let library = schema();
    if library.structs.is_empty() {
        or_test_support::skip_schema(what);
        return None;
    }
    Some(library)
}
