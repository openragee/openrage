//! `openrage-cli texture replace`: one texture of a GTA VI texture pack replaced from a PNG, a DDS
//! or any other import format, in its own size, format and mips, written to a new pack.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::hash::name_hash64;
use or_io::gta6::ttpf::TexturePack;
use or_io::gta6::ttpf_author::{replace_from_image, Replace};

const USAGE: &str = "usage: texture replace <dict.ttpf> <name|#index|hash_KEY> <image> \
                     --out <new.ttpf> [--resize] [--dds-mips] [--slice N] \
                     [--quality fast|normal|high] [--filter box|lanczos3] [--srgb|--linear]";

/// Said after every write.
const STRUCTURAL: &str = "The new pack is a structural result written to a new file: no game has \
                          been seen to load it.";

/// The texture `#N`, `hash_<key>` or a name hashing to its key names.
fn index_of(pack: &TexturePack, texture: &str) -> Option<usize> {
    if let Some(index) = texture.strip_prefix('#') {
        return index.parse().ok().filter(|&i| i < pack.entries.len());
    }
    let key = match texture.strip_prefix("hash_") {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => name_hash64(texture.as_bytes()),
    };
    (pack.entries.iter()).position(|e| e.on_disk && e.key == key)
}

fn same(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
}

fn replace(args: &[String]) -> Result<(), String> {
    let (out, args) = crate::rpf::split_value(args, "--out")?;
    let (slice, args) = crate::rpf::split_value(&args, "--slice")?;
    let slice = (slice.map(|word| word.parse::<u32>()).transpose())
        .map_err(|_| "--slice expects a number".to_string())?
        .unwrap_or(0);
    let (resize, dds_mips) = (
        args.iter().any(|a| a == "--resize"),
        args.iter().any(|a| a == "--dds-mips"),
    );
    let args: Vec<String> = (args.into_iter())
        .filter(|a| a != "--resize" && a != "--dds-mips")
        .collect();
    let (opts, positional) = crate::texture::parse_flags(&args)?;
    if opts.format.is_some() || opts.mips.is_some() {
        return Err(
            "texture replace keeps the texture's format and mips; ttpf replace gives it others"
                .into(),
        );
    }
    let [input, name, image] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    let output = (out.map(PathBuf::from).or(opts.output.clone())).ok_or(USAGE)?;
    let input = PathBuf::from(input);
    if output.exists() && same(&input, &output) {
        return Err(format!(
            "{} is the pack being read, which is never written; give --out a new file",
            output.display()
        ));
    }
    let mut pack = TexturePack::open(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let index = index_of(&pack, name)
        .ok_or_else(|| format!("no texture '{name}' in {}", input.display()))?;
    let srgb = pack.entries[index].format().is_some_and(|f| f.is_srgb());
    let how = Replace {
        build: opts.build_options(srgb),
        resize,
        dds_mips,
    };
    let mips = replace_from_image(&mut pack, index, slice, Path::new(image), &how)
        .map_err(|e| e.to_string())?;
    write_new(&pack, &input, &output)?;
    println!(
        "texture {index} ({name}) replaced with {mips} mip(s), its name and key kept; wrote {}",
        output.display()
    );
    println!("{STRUCTURAL}");
    Ok(())
}

/// `pack` saved to `output` through a file beside it renamed over it, so an output that is the input under another path is never truncated while its rows are read.
fn write_new(pack: &TexturePack, input: &Path, output: &Path) -> Result<(), String> {
    let mut temp = output.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    if same(input, &temp) {
        return Err(format!("{} is the pack being read", temp.display()));
    }
    let written = std::fs::File::create(&temp)
        .map_err(|e| e.to_string())
        .and_then(|file| {
            let mut out = std::io::BufWriter::new(file);
            pack.save_to(&mut out).map_err(|e| e.to_string())?;
            std::io::Write::flush(&mut out).map_err(|e| e.to_string())
        })
        .and_then(|()| std::fs::rename(&temp, output).map_err(|e| e.to_string()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{}: {e}", output.display()));
    }
    Ok(())
}

fn print_usage() {
    eprintln!("{USAGE}");
    eprintln!();
    eprintln!(
        "Replace one texture of a GTA VI texture pack (.ttpf) from an image (PNG, DDS, JPEG,"
    );
    eprintln!("TGA, BMP, TIFF, PSD), encoded into the texture's own size, format and mips, its");
    eprintln!("mips made from the image's top level; the texture keeps its name, key and entry.");
    eprintln!(
        "  --resize    resample an image of another size to the texture's, else it is refused"
    );
    eprintln!("  --dds-mips  copy a DDS's own mips block for block: the DDS must be of the");
    eprintln!("              texture's format and size and hold at least as many mips");
    eprintln!("  --slice N   a cube's face, an array's slice or a one-level volume's z-plane");
    eprintln!("A texture stored through the BCPack coder is refused by name. {STRUCTURAL}");
}

pub fn run(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    if args.is_empty() {
        print_usage();
        return ExitCode::from(2);
    }
    match replace(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Error: {message}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_input_is_never_the_output_and_format_and_mips_are_not_changed_here() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let pack = dir.path().join("pack.ttpf");
        std::fs::write(&pack, b"not read").expect("written");
        let pack = pack.to_string_lossy().into_owned();
        let refused = replace(&words(&[&pack, "zq_ab", "red.png", "--out", &pack]))
            .expect_err("written in place");
        assert!(refused.contains("never written"), "{refused}");
        let refused = replace(&words(&[
            &pack, "zq_ab", "red.png", "--out", "new.ttpf", "-f", "BC7",
        ]))
        .expect_err("another format");
        assert!(refused.contains("ttpf replace"), "{refused}");
        let refused = replace(&words(&[&pack, "zq_ab", "red.png"])).expect_err("no --out");
        assert!(refused.starts_with("usage"), "{refused}");
    }
}
