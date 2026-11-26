use std::path::{Path, PathBuf};

include!("fingerprint.rs");

const VENDOR: &str = "vendor/dxbc-spirv";
/// The translator's source folders; its Direct3D 9 front end is not built.
const SOURCE_DIRS: [&str; 5] = ["dxbc", "ir", "ir/passes", "spirv", "util"];

fn sources(dir: &Path) -> Vec<PathBuf> {
    sources_of(dir, "cpp")
}

fn sources_of(dir: impl AsRef<Path>, extension: &str) -> Vec<PathBuf> {
    let dir = dir.as_ref();
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    found.sort();
    found
}

/// FNV-1a over the files, in order: a name for this build of the translator, since its shim and layout change what it writes.
fn fingerprint(paths: &[&str]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for path in paths {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        for byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=fingerprint.rs");
    println!("cargo:rerun-if-changed=src/layout.rs");
    println!(
        "cargo:rustc-env=OR_SHADER_FRONT_FINGERPRINT={:016x}",
        fingerprint(&["src/shim.cpp", "src/layout.rs"])
    );
    // The native front end's own sources: a translation kept on disk is of the build that wrote it.
    println!("cargo:rerun-if-changed=src/rdna");
    println!(
        "cargo:rustc-env=OR_RDNA_FINGERPRINT={:016x}",
        tree_fingerprint(Path::new("src/rdna"))
    );
    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    if std::env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        return;
    }
    let vendor = Path::new(VENDOR);
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .warnings(false)
        .include(vendor)
        .include(vendor.join("submodules/spirv_headers/include"))
        .define("DXBC_SPV_ENABLE_SM5", None)
        .define("DXBC_SPV_ENABLE_SPIRV", None)
        .file("src/shim.cpp");
    for dir in SOURCE_DIRS {
        build.files(sources(&vendor.join(dir)));
    }
    build.compile("or_shader_front");
}
