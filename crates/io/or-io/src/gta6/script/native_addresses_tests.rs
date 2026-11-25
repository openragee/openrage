//! Checks of the native address table on listings written here, so no game byte is involved; and,
//! gated on the user's listings, a build over them.

use super::*;
use crate::gta5::script::natives::NativeDb;

/// A listing declaring `declarations` in a `Native.PLAYER` class.
fn listing(declarations: &[&str]) -> String {
    let members: String = declarations
        .iter()
        .map(|d| format!("            {d}\n"))
        .collect();
    format!(
        "[VmProgram(0x00000001U)]\npublic static class Program_00000001\n{{\n    public static \
         class Native\n    {{\n        public static class PLAYER\n        {{\n{members}        \
         }}\n    }}\n}}\n"
    )
}

fn native(name: &str, hash: u64, address: u32) -> String {
    format!(
        "public static void {name}(VmValue p0) => Vm.Native(0x{hash:X}UL, 0x{address:X}U, 0, p0);"
    )
}

/// A folder of two listings: one names the native at 0x100, both declare the hash-only one at 0x200.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    let first = listing(&[
        &native("_0x00000000000000AA", 0xAA, 0x100),
        &native("_0x00000000000000BB", 0xBB, 0x200),
    ]);
    let second = listing(&[
        &native("SET_THING", 0xAA, 0x100),
        &native("_0x00000000000000BB", 0xBB, 0x200),
    ]);
    std::fs::write(dir.path().join("a.ysc.cs"), first).expect("write a listing");
    std::fs::write(dir.path().join("b.ysc.cs"), second).expect("write a listing");
    std::fs::write(dir.path().join("notes.txt"), "not a listing").expect("write a file");
    dir
}

#[test]
fn an_address_resolves_to_its_hash_and_the_name_a_listing_gives_it() {
    let dir = folder();
    let table = NativeAddresses::load(dir.path(), None).expect("the folder reads");
    assert_eq!(table.len(), 2);
    assert_eq!(table.hash_of(0x100), Some(0xAA));
    assert_eq!(
        table.name_of(0x100),
        Some("SET_THING"),
        "a name replaces a hash-only one"
    );
    assert_eq!(table.hash_of(0x200), Some(0xBB));
    assert_eq!(
        table.name_of(0x200),
        None,
        "a hash-only native names nothing"
    );
    assert_eq!((table.hash_of(0x300), table.name_of(0x300)), (None, None));
}

#[test]
fn the_names_registry_names_a_native_by_its_hash_before_the_listing() {
    let dir = folder();
    let mut table = NativeAddresses::load(dir.path(), None).expect("the folder reads");
    let db = NativeDb::parse("test", b"0xBB FROM_REGISTRY\n0xAA ALSO_REGISTRY\n")
        .expect("the list reads");
    table.names.natives = Some(db);
    assert_eq!(table.name_of(0x200), Some("FROM_REGISTRY"));
    assert_eq!(table.name_of(0x100), Some("ALSO_REGISTRY"));
}

#[test]
fn a_disagreement_or_a_malformed_listing_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let a = listing(&[&native("SET_THING", 0xAA, 0x100)]);
    std::fs::write(dir.path().join("a.ysc.cs"), &a).expect("write");
    let other_hash = listing(&[&native("SET_THING", 0xAB, 0x100)]);
    std::fs::write(dir.path().join("b.ysc.cs"), other_hash).expect("write");
    let error = NativeAddresses::load(dir.path(), None).expect_err("two hashes at one address");
    let NativeAddressesError::Conflict {
        listing: file,
        address,
        ..
    } = &error
    else {
        panic!("{error}");
    };
    assert_eq!((file.as_str(), *address), ("b.ysc.cs", 0x100));

    let other_name = listing(&[&native("SET_OTHER", 0xAA, 0x100)]);
    std::fs::write(dir.path().join("b.ysc.cs"), other_name).expect("write");
    let error = NativeAddresses::load(dir.path(), None).expect_err("two names at one address");
    assert!(error.to_string().contains("`SET_OTHER`"), "{error}");

    std::fs::write(dir.path().join("b.ysc.cs"), "int x;\n").expect("write");
    let error = NativeAddresses::load(dir.path(), None).expect_err("a listing that does not open");
    let NativeAddressesError::Listing { listing, .. } = &error else {
        panic!("{error}");
    };
    assert_eq!(listing, "b.ysc.cs");

    let empty = tempfile::tempdir().expect("a temp dir");
    let error = NativeAddresses::load(empty.path(), None).expect_err("no listing");
    assert!(
        matches!(error, NativeAddressesError::Empty { .. }),
        "{error}"
    );
}

#[test]
fn the_cache_is_read_for_the_same_files_and_rebuilt_when_one_changes() {
    let dir = folder();
    let config = tempfile::tempdir().expect("a temp dir");
    let config_dir = config.path().join("config");
    let built = NativeAddresses::load(dir.path(), Some(&config_dir)).expect("the folder reads");
    let cache = config_dir.join(CACHE_FILE);
    let text = std::fs::read_to_string(&cache).expect("the cache is written");
    assert!(text.starts_with(CACHE_HEAD), "{text}");
    assert!(
        text.contains("00000100\t00000000000000aa\tSET_THING\n"),
        "{text}"
    );

    // A cache whose rows differ from the listings, under the right head, is what is read.
    let head = text.lines().next().expect("a head line");
    std::fs::write(
        &cache,
        format!("{head}\n00000100\t00000000000000aa\tCACHED\n"),
    )
    .expect("write");
    let cached = NativeAddresses::load(dir.path(), Some(&config_dir)).expect("the cache reads");
    assert_eq!((cached.len(), cached.name_of(0x100)), (1, Some("CACHED")));

    // A changed listing changes the fingerprint, so the folder is read again.
    let third = listing(&[&native("THIRD", 0xCC, 0x300)]);
    std::fs::write(dir.path().join("c.ysc.cs"), third).expect("write");
    let rebuilt = NativeAddresses::load(dir.path(), Some(&config_dir)).expect("the folder reads");
    assert_eq!(rebuilt.len(), built.len() + 1);
    assert_eq!(rebuilt.name_of(0x100), Some("SET_THING"));
    assert_eq!(rebuilt.name_of(0x300), Some("THIRD"));

    // A cache row that does not read is passed over for the folder.
    let head = std::fs::read_to_string(&cache).expect("the cache is rewritten");
    let head = head.lines().next().expect("a head line").to_owned();
    std::fs::write(&cache, format!("{head}\nnot a row\n")).expect("write");
    let reread = NativeAddresses::load(dir.path(), Some(&config_dir)).expect("the folder reads");
    assert_eq!(reread.len(), 3);
}

#[test]
fn the_users_listings_declare_their_natives_by_address() {
    let Some(folder) = std::env::var_os("OPENRAGE_LIFTED_LISTINGS")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
    else {
        or_test_support::skip_install(
            "the native address table over lifted listings",
            "set OPENRAGE_LIFTED_LISTINGS",
        );
        return;
    };
    let table = NativeAddresses::load(&folder, None).expect("the listings read");
    let named = table
        .by_address
        .keys()
        .filter(|a| table.name_of(**a).is_some());
    let named = named.count();
    assert!(
        named > 0 && named < table.len(),
        "{named} of {}",
        table.len()
    );
}
