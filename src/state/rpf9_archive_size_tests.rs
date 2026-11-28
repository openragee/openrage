//! Details sizes a nested GTA VI archive as its column does, past 4 GiB too. Install-gated: it
//! needs the key snapshot and the build, and skips without them.

use std::path::PathBuf;

use or_io::container::rpf::Gta6Context;
use or_io::container::RpfArchive;

use crate::state::files_view::measured_bytes;
use crate::state::rpf_archive_open::gta6_session;
use crate::state::rpf_session::RpfEditSession;

/// The build's largest archive under 64 GiB holds the build's one row past 4 GiB, a nested archive.
#[test]
fn details_sizes_a_nested_archive_as_its_column_does_past_4_gib_too() {
    const WHAT: &str = "Details sizes a nested GTA VI archive as its column does";
    let _serial = crate::ui::theme::serial();
    let build_root = std::env::var_os("OPENRAGE_GTA6_BUILD").map(PathBuf::from);
    let (Some(build_root), Ok(keys)) = (build_root, crate::state::rpf9_scan::rpf9_table::keys())
    else {
        or_test_support::skip_install(WHAT, "set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS");
        return;
    };
    let _reading = gta6_session::hold(Gta6Context {
        keys,
        names: crate::state::rpf9_scan::rpf9_table::resolver(),
    });
    let largest = std::fs::read_dir(&build_root)
        .expect("the build root lists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpf")))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .filter(|(len, _)| *len < 64 << 30)
        .max()
        .map(|(_, p)| p)
        .expect("a top-level archive");
    let archive =
        RpfArchive::open_with(&largest, gta6_session::crypto()).expect("the archive opens");
    let session = RpfEditSession::from_archive(archive).expect("the tree lists");
    let mut past = 0usize;
    for id in session.children_of(session.root_id()) {
        let Some(size) = session.archive_size(id) else {
            continue;
        };
        assert_eq!(Some(size), measured_bytes(&session, id), "row {id}");
        past += usize::from(size > u64::from(u32::MAX));
    }
    // Measured on the build 1.0.2.1: one row past 4 GiB, 14 159 487 680 bytes.
    assert_eq!(past, 1);
}
