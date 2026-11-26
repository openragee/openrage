// The name of a build of the native front end, shared by the build script and the test that holds it.

/// FNV-1a over every `.rs` file under `dir`, those of the folders inside it too, each with its
/// path from `dir`, in the order of those paths. Carriage returns are left out: a checkout's
/// line endings do not make another translator.
pub fn tree_fingerprint(dir: &std::path::Path) -> u64 {
    fn walk(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for path in entries.filter_map(|entry| entry.ok().map(|e| e.path())) {
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    let mut named: Vec<(String, std::path::PathBuf)> = (files.into_iter())
        .map(|path| {
            let within = path.strip_prefix(dir).unwrap_or(&path);
            (within.to_string_lossy().replace('\\', "/"), path)
        })
        .collect();
    named.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (name, path) in named {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for byte in name.bytes().chain([0]).chain(bytes) {
            if byte != b'\r' {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hash
}
