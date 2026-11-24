//! `openrage-cli ttpf`: list a GTA VI texture pack's textures, export the readable ones, replace
//! one into a new pack, and plan a save against the file's own table.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::codec::texture::{decode_level, save_png, to_dds};
use or_io::gta6::bcpack::calibrate::{load_cache, save_cache};
use or_io::gta6::bcpack::Decode;
use or_io::gta6::hash::name_hash64;
use or_io::gta6::texture_names::{name_keys, use_label};
use or_io::gta6::ttpf::{
    calibrate_packs_from, coverage, Format, Texture, TexturePack, KIND_STORED, KIND_ZLIB,
};

/// A cube's faces in the order a pack stores them, D3D's.
const FACES: [&str; 6] = ["px", "nx", "py", "ny", "pz", "nz"];

/// Where `ttpf calibrate` keeps the coder's tables it fits: `bcpack` under the configuration directory.
fn calibration_dir() -> Option<PathBuf> {
    or_title::config::config_dir().map(|dir| dir.join("bcpack"))
}

/// The pack at `path`, its bodies that do not decode exactly drawn through a calibration that read it, if one is kept.
fn open(path: &Path) -> Result<TexturePack, String> {
    let mut pack = TexturePack::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let tables = calibration_dir().and_then(|dir| load_cache(&dir, pack.member_key()));
    pack.set_calibration(tables.map(std::sync::Arc::new));
    Ok(pack)
}

/// How a texture with a body through the coder was decoded, for a line that lists or exports it.
fn decode_note(how: &Decode) -> String {
    if how.is_exact() {
        return "exact".to_string();
    }
    format!(
        "calibrated: an estimate through tables fitted on this machine, {:.1}% of its coded \
         blocks through a slot or symbol not known",
        how.unknown_percent()
    )
}

fn kind_name(kind: u8) -> String {
    match kind {
        KIND_STORED => "stored".into(),
        KIND_ZLIB => "zlib".into(),
        // The kind is the BC number of the blocks, stored through the BCPack coder.
        1 | 3..=7 => format!("BC{kind} coder"),
        other => format!("kind {other}"),
    }
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: ttpf info <file.ttpf> [--name NAME]... [--names FILE]";
    let (mut path, mut names) = (None, HashMap::new());
    let mut stems = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" | "-n" => {
                let name = args.get(i + 1).ok_or("--name needs a value")?;
                names.insert(name_hash64(name.as_bytes()), name.clone());
                i += 2;
            }
            "--names" => {
                let file = args.get(i + 1).ok_or("--names needs a file")?;
                let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
                stems.push_str(&text);
                stems.push('\n');
                i += 2;
            }
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other if path.is_none() => {
                path = Some(PathBuf::from(other));
                i += 1;
            }
            _ => return Err(USAGE.into()),
        }
    }
    let path = path.ok_or(USAGE)?;
    let pack = open(&path)?;
    // Each line names a texture or a drawable, whose stem the naming rules expand as the app does.
    let keys: HashSet<u64> = (pack.entries.iter()).map(|e| e.key).collect();
    let lines = stems.lines().map(str::trim).filter(|l| !l.is_empty());
    for (key, (name, _)) in name_keys(lines, &keys) {
        names.entry(key).or_insert(name);
    }
    let sentinel = pack.rows.last().map_or(0, |row| row.offset);
    let state = if pack.is_complete() {
        "complete".to_string()
    } else if pack.size_on_disk() == sentinel {
        "full length but sparse, so pieces of a download may still be missing".to_string()
    } else {
        format!("short: {} of {sentinel} bytes on disk", pack.size_on_disk())
    };
    println!(
        "{}: {} texture(s) in {} rows; {state}\n",
        path.display(),
        pack.entries.len(),
        pack.rows.len()
    );
    println!(
        "{:>6} {:<22} {:>11} {:<22} {:<20} READ",
        "INDEX", "NAME", "SIZE", "FORMAT", "BODIES"
    );
    for (index, entry) in pack.entries.iter().enumerate() {
        let name = match names.get(&entry.key) {
            _ if !entry.on_disk => "not on disk".to_string(),
            Some(name) => name.clone(),
            // A texture with no name is said with the model a harvest found to name it by key.
            None => match super::gta6_names::texture_uses().get(&entry.key) {
                Some(used) => format!("hash_{:016X} ({})", entry.key, use_label(used)),
                None => format!("hash_{:016X}", entry.key),
            },
        };
        let format = match entry.format() {
            Some(format) => format!("{} ({:#05x})", format.name(), entry.format_code),
            None => format!("{:#05x}", entry.format_code),
        };
        let mut kinds: Vec<String> = Vec::new();
        let mut coded = false;
        for row in pack.rows_of(index).into_iter().flatten() {
            let kind = pack.rows[row].kind;
            coded |= kind != KIND_STORED && kind != KIND_ZLIB;
            let name = kind_name(kind);
            if !kinds.contains(&name) {
                kinds.push(name);
            }
        }
        let read = match pack.texture_noted(index) {
            Ok((texture, how)) => {
                let mips = match texture.faces {
                    1 => format!("yes, {} mip(s)", texture.levels),
                    faces => format!("yes, {faces} slices of {} mip(s)", texture.levels),
                };
                // A texture with a body through the coder says whether its decode is exact.
                match coded {
                    true => format!("{mips}, {}", decode_note(&how)),
                    false => mips,
                }
            }
            Err(e) => format!("no: {e}"),
        };
        println!(
            "{index:>6} {name:<22} {:>11} {format:<22} {:<20} {read}",
            format!("{}x{}", entry.width, entry.height),
            kinds.join(", ")
        );
    }
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<(), String> {
    let mut as_dds = false;
    let mut only: Option<usize> = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                let raw = args.get(i + 1).ok_or("--format needs a value")?;
                as_dds = match raw.to_ascii_lowercase().as_str() {
                    "dds" => true,
                    "png" => false,
                    other => return Err(format!("--format expects png or dds, got '{other}'")),
                };
                i += 2;
            }
            "--index" | "-i" => {
                let raw = args.get(i + 1).ok_or("--index needs a value")?;
                only = Some(
                    raw.parse()
                        .map_err(|_| format!("--index expects a number, got '{raw}'"))?,
                );
                i += 2;
            }
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => {
                positional.push(PathBuf::from(other));
                i += 1;
            }
        }
    }
    let [input, out_dir] = positional.as_slice() else {
        return Err(
            "usage: ttpf export <file.ttpf> <outdir> [--format png|dds] [--index N]".into(),
        );
    };

    let pack = open(input)?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;
    let indices: Vec<usize> = match only {
        Some(index) => vec![index],
        None => (0..pack.entries.len()).collect(),
    };
    let (mut written, mut refused) = (0usize, 0usize);
    let mut estimates = 0usize;
    for index in indices {
        let (texture, how) = match pack.texture_noted(index) {
            Ok(read) => read,
            Err(e) if only.is_some() => return Err(format!("texture {index}: {e}")),
            Err(e) => {
                eprintln!("texture {index}: {e}");
                refused += 1;
                continue;
            }
        };
        let cube = pack.entries[index].dimension == 2;
        // An export of an estimate says so on its line.
        let note = match how.is_exact() {
            true => String::new(),
            false => format!("; {}", decode_note(&how)),
        };
        estimates += usize::from(!how.is_exact());
        write_faces(out_dir, &index.to_string(), texture, cube, as_dds, &note)?;
        written += 1;
    }
    if written == 0 {
        return Err(format!("no texture of {} is readable", input.display()));
    }
    println!(
        "\nExported {written} texture(s) to {}; {refused} not readable",
        out_dir.display()
    );
    if estimates > 0 {
        println!(
            "{estimates} of them are calibrated: estimates through tables fitted on this \
             machine, not the pack's stored blocks"
        );
    }
    Ok(())
}

/// Every `.ttpf` under `dir`, into `out`.
fn packs_under(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let listed = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in listed.flatten() {
        let path = entry.path();
        if path.is_dir() {
            packs_under(&path, out)?;
        } else if (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("ttpf")) {
            out.push(path);
        }
    }
    Ok(())
}

/// Fit the coder's tables from every pack under a folder, and keep them for those packs.
fn cmd_calibrate(args: &[String]) -> Result<(), String> {
    let (folder, covered) = match args {
        [folder] => (folder, false),
        [folder, flag] if flag == "--coverage" => (folder, true),
        _ => return Err("usage: ttpf calibrate <folder> [--coverage]".into()),
    };
    let mut paths = Vec::new();
    packs_under(Path::new(folder), &mut paths)?;
    paths.sort();
    let mut packs = Vec::new();
    for path in &paths {
        match TexturePack::open(path) {
            Ok(pack) => packs.push(pack),
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
    }
    if packs.is_empty() {
        return Err(format!("no texture pack opens under {folder}"));
    }
    let mut said = u64::MAX;
    let mut progress = |done: u64, total: u64| {
        let percent = 100 * done / total.max(1);
        if percent != said {
            said = percent;
            eprint!("\r{percent:>3}%");
        }
    };
    // A run cut short is gone on from by the next one, at the pass it was in.
    let dir = calibration_dir().ok_or("no configuration directory to keep the calibration in")?;
    let found =
        calibrate_packs_from(&packs, Some(&dir), &mut progress, &|| false).ok_or("stopped")?;
    eprintln!();
    println!("Calibrated from {} pack(s) under {folder}\n", packs.len());
    println!(
        "{:<5} {:>7} {:>7} {:>10} {:>10} {:>6} {:>6} {:>10}  AGAINST THE PLAIN MIPS",
        "MODE", "CHAINS", "BODIES", "BLOCKS", "NOT KNOWN", "SLOTS", "PAIRS", "QUADRANTS"
    );
    for summary in &found.summaries {
        let mismatch = match (summary.mean_mismatch(), summary.mean_alpha_mismatch()) {
            (Some(mean), _) if summary.kind == 6 => format!("{mean:.2} stops off"),
            (Some(mean), Some(alpha)) => format!("colour {mean:.2}, alpha {alpha:.2} of 255 off"),
            (Some(mean), None) => format!("{mean:.2} of 255 off"),
            (None, _) => "no coded mip beside a plain one".to_string(),
        };
        println!(
            "{:<5} {:>7} {:>7} {:>10} {:>9.2}% {:>6} {:>6} {:>10}  {mismatch}",
            format!("BC{}", summary.kind),
            summary.textures,
            summary.bodies,
            summary.blocks,
            100.0 * summary.unknown as f64 / summary.blocks.max(1) as f64,
            summary.slots,
            summary.pairs,
            summary.quadrants
        );
    }
    let path = save_cache(&dir, &found.tables, &found.members)
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    println!("\nKept in {}", path.display());
    if !covered {
        return Ok(());
    }
    // How every coded texture of these packs then reads, each coded body decoded once.
    let tables = std::sync::Arc::new(found.tables);
    for pack in &mut packs {
        pack.set_calibration(Some(tables.clone()));
    }
    println!(
        "\n{:<5} {:>9} {:>10} {:>8} {:>12} {:>12}  THROUGH A SLOT NOT KNOWN",
        "MODE", "EXACT", "CALIBRATED", "REFUSED", "BLOCKS", "FITTED"
    );
    for of_kind in coverage(&packs, &|| false).ok_or("stopped")? {
        println!(
            "{:<5} {:>9} {:>10} {:>8} {:>12} {:>12}  {:.2}% of the fitted blocks",
            format!("BC{}", of_kind.kind),
            of_kind.exact,
            of_kind.calibrated,
            of_kind.refused,
            of_kind.blocks,
            of_kind.fitted,
            100.0 * of_kind.unknown as f64 / of_kind.fitted.max(1) as f64
        );
    }
    println!(
        "The tables are fitted on this machine, not read from a source: a texture drawn through \
         them is an estimate, and `ttpf info` and `ttpf export` say which are."
    );
    Ok(())
}

/// Write `texture` into `out_dir` as `<stem>.png` or `.dds`, a cube's faces, in their stored
/// order, and an array's or a volume's slices as files of their own.
fn write_faces(
    out_dir: &Path,
    stem: &str,
    texture: Texture,
    cube: bool,
    as_dds: bool,
    note: &str,
) -> Result<(), String> {
    let extension = if as_dds { "dds" } else { "png" };
    let faces: Vec<(String, Texture)> = match texture.faces {
        1 => vec![(format!("{stem}.{extension}"), texture)],
        faces => (0..faces)
            .filter_map(|f| {
                // A cube array's slices run cube after cube, six faces each.
                let face = FACES[f as usize % 6];
                let name = match (cube, faces) {
                    (true, 6) => face.to_string(),
                    (true, _) => format!("{}_{face}", f / 6),
                    _ => f.to_string(),
                };
                Some((format!("{stem}_{name}.{extension}"), texture.face(f)?))
            })
            .collect(),
    };
    for (name, face) in faces {
        let path = out_dir.join(name);
        if as_dds {
            to_dds(&face).and_then(|dds| dds.save(&path))
        } else {
            decode_level(&face, 0).and_then(|image| save_png(&path, &image))
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        println!(
            "{}  ({}x{} {}, {} mip(s){note})",
            path.display(),
            face.width,
            face.height,
            face.format.name(),
            face.levels
        );
    }
    Ok(())
}

/// A folder to export to, and whether as DDS.
pub(crate) type Export = (PathBuf, bool);

/// Where `--export DIR` and `--format png|dds` ask a scanned dictionary's textures to be written, and the other words.
pub(crate) fn split_export(args: &[String]) -> Result<(Option<Export>, Vec<String>), String> {
    let (mut out, mut as_dds, mut rest) = (None, None, Vec::new());
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--export" => {
                out = Some(PathBuf::from(
                    words.next().ok_or("--export needs a folder")?,
                ))
            }
            "--format" => {
                as_dds = match words.next().map(|f| f.to_ascii_lowercase()).as_deref() {
                    Some("dds") => Some(true),
                    Some("png") => Some(false),
                    _ => return Err("--format expects png or dds".into()),
                }
            }
            _ => rest.push(word.clone()),
        }
    }
    match (out, as_dds) {
        (Some(out), as_dds) => Ok((Some((out, as_dds.unwrap_or(false))), rest)),
        (None, Some(_)) => Err("--format applies to --export".into()),
        (None, None) => Ok((None, rest)),
    }
}

/// Write each texture of the scanned dictionary `resource` holds in `bytes` that reads into `out`, by its name, as `ttpf export` writes a pack's.
pub(crate) fn export_scanned(
    find: &or_io::container::rpf::Rpf9ScanFind,
    bytes: &[u8],
    (out, as_dds): &Export,
) -> Result<(), String> {
    use or_io::container::rpf::{Rpf9BodyKind, Rpf9ZlibRun};
    use or_io::gta6::headerless::{read, read_drawable_head, Dictionary, Entry, Resource};
    let not_one = || {
        "--export writes the textures of a scanned texture dictionary, or those a drawable's \
         shader group embeds, and this body is neither"
            .to_string()
    };
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = &find.kind
    else {
        return Err(not_one());
    };
    let (virtual_section, physical) = resource.sections(bytes);
    let fragment = match read(*signature, virtual_section, Some(physical)) {
        Ok(Resource::Dictionary(dict)) => return export_dictionary(dict, out, *as_dds),
        Ok(Resource::Drawable(_)) => false,
        Ok(Resource::DrawableWithBound(_)) => true,
        _ => return Err(not_one()),
    };
    // An embedded texture is named `hash_` and its key, its record holding no name.
    let head = read_drawable_head(virtual_section, Some(physical), fragment)
        .map_err(|e| format!("the drawable's embedded textures are not read: {e}"))?;
    if head.textures.is_empty() {
        return Err("this drawable's shader group embeds no texture".to_string());
    }
    let entries = (head.textures.into_iter())
        .map(|t| match t {
            Ok(t) => (t.hash, Ok(Entry::Texture(t.texture))),
            Err(e) => (0, Err(e)),
        })
        .collect();
    export_dictionary(Dictionary { entries }, out, *as_dds)
}

/// Write each texture of `dict` that reads into `out`, by its name.
pub(crate) fn export_dictionary(
    dict: or_io::gta6::headerless::Dictionary,
    out: &Path,
    as_dds: bool,
) -> Result<(), String> {
    use or_io::gta6::headerless::Entry;
    std::fs::create_dir_all(out).map_err(|e| format!("failed to create {}: {e}", out.display()))?;
    let (mut written, mut refused) = (0usize, 0usize);
    let mut taken = std::collections::HashSet::new();
    for (_, entry) in dict.entries {
        let Ok(Entry::Texture(t)) = entry else {
            continue;
        };
        match t.texture {
            Ok(texture) => {
                // A name is the file's stem only where it names no folder.
                let named: String = (t.name.chars())
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || "_-.".contains(c) {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                // Two names may make one stem, and a folder may not tell case apart.
                let (mut stem, mut n) = (named.clone(), 1);
                while !taken.insert(stem.to_ascii_lowercase()) {
                    n += 1;
                    stem = format!("{named}_{n}");
                }
                write_faces(out, &stem, texture, t.dimension == 2, as_dds, "")?;
                written += 1;
            }
            Err(e) => {
                eprintln!("texture {:?}: {e}", t.name);
                refused += 1;
            }
        }
    }
    if written == 0 {
        return Err(format!(
            "no texture of this dictionary reads; {refused} do not"
        ));
    }
    println!(
        "\nExported {written} texture(s) to {}; {refused} not readable",
        out.display()
    );
    Ok(())
}

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

fn cmd_replace(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: ttpf replace <file.ttpf> <#index|hash_KEY|name> <image> \
                         -o <out.ttpf> [--format F] [--mips N] [--quality Q] [--filter F] \
                         [--srgb|--linear] [--slice N]";
    let (slice, args) = crate::rpf::split_value(args, "--slice")?;
    let slice = (slice.map(|word| word.parse::<u32>()).transpose())
        .map_err(|_| "--slice expects a number".to_string())?;
    let (opts, positional) = crate::texture::parse_flags(&args)?;
    let [input, texture, image] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    let output = opts.output.clone().ok_or(USAGE)?;
    let input = PathBuf::from(input);
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok();
    if output.exists() && same(&input, &output) {
        return Err(format!(
            "{} is the pack the rows are read from; write the new pack to another path",
            output.display()
        ));
    }
    let mut pack = open(&input)?;
    let index = index_of(&pack, texture)
        .ok_or_else(|| format!("no texture '{texture}' in {}", input.display()))?;
    let image = or_io::authoring::texture_import::load(Path::new(image))
        .map_err(|e| format!("{image}: {e}"))?;
    let was = pack.entries[index];
    let options = opts.build_options(was.format().is_some_and(|f| f.is_srgb()));
    // An image of another size, a format or a mip count plans the texture's rows anew.
    let reshaped = opts.format.is_some()
        || opts.mips.is_some()
        || (image.width, image.height) != (was.width, was.height);
    let mips = if reshaped && slice.is_some() {
        return Err("--slice replaces one slice in the texture's own size, format and mips".into());
    } else if reshaped {
        let format = match opts.format {
            Some(grc) => Format::of_grc(grc, options.srgb)
                .ok_or_else(|| format!("{grc:?} is no format a GTA VI pack is measured to hold"))?,
            None => was.format().ok_or_else(|| {
                format!("texture {index}'s format is not a measured one; name one with --format")
            })?,
        };
        pack.reshape(index, &image, format, &options)
    } else {
        // A cube's face, an array's slice or a one-level volume's z-plane, 0 by default.
        pack.replace_slice(index, slice.unwrap_or(0), &image, &options)
    };
    let mips = mips.map_err(|e| format!("texture {index}: {e}"))?;
    // ⚠ Written beside the output and renamed over it: an output that is the input under another
    // path, such as a hard link, is then never truncated while the rows are read from it.
    let mut temp = output.clone().into_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    if same(&input, &temp) {
        return Err(format!(
            "{} is the pack the rows are read from",
            temp.display()
        ));
    }
    let written = std::fs::File::create(&temp)
        .map_err(|e| e.to_string())
        .and_then(|file| {
            let mut out = std::io::BufWriter::new(file);
            pack.save_to(&mut out).map_err(|e| e.to_string())?;
            std::io::Write::flush(&mut out).map_err(|e| e.to_string())
        })
        .and_then(|()| std::fs::rename(&temp, &output).map_err(|e| e.to_string()));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("{}: {e}", output.display()));
    }
    println!(
        "texture {index} replaced with {mips} mip(s); wrote {}",
        output.display()
    );
    if pack.is_relaid() {
        println!(
            "the textures after it moved to fit its new size, format or mips: no game has been \
             seen to load a pack written this way"
        );
    }
    Ok(())
}

/// What a save would write against what the file holds, per row and per entry, reading no body.
fn cmd_plan(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("usage: ttpf plan <file.ttpf>...".into());
    }
    for path in args {
        let planned = open(Path::new(path)).and_then(|pack| {
            let layout = pack.save_layout().map_err(|e| format!("{path}: {e}"))?;
            Ok((pack, layout))
        });
        let (pack, layout) = match planned {
            Ok(planned) => planned,
            Err(e) => {
                println!("{e}");
                continue;
            }
        };
        let differ: Vec<usize> = (0..pack.rows.len())
            .filter(|&row| layout.rows[row] != pack.rows[row])
            .collect();
        println!(
            "{path}: {} rows, {} as planned, {} not; starts carried: {} where the file departs \
             from the 4 KiB rule, {} the rule does not decide",
            pack.rows.len(),
            pack.rows.len() - differ.len(),
            differ.len(),
            layout.departs.len(),
            layout.undecided.len()
        );
        for &row in differ.iter().take(8) {
            let (was, now) = (pack.rows[row], layout.rows[row]);
            println!(
                "  row {row}: offset {:#x} pad {:#x} kind {}, planned {:#x} pad {:#x} kind {}",
                was.offset, was.pad, was.kind, now.offset, now.pad, now.kind
            );
        }
        for &row in layout.departs.iter().take(8) {
            let previous = pack.rows[row - 1];
            println!(
                "  departs at row {row}: offset {:#x}, the row before at {:#x} with pad {:#x}",
                pack.rows[row].offset, previous.offset, previous.pad
            );
        }
        let (mut same, mut other, mut unjudged) = (0, Vec::new(), 0);
        for (index, entry) in pack.entries.iter().enumerate() {
            match pack.codes_by_rule(index) {
                Some(codes) if codes == (entry.row_codes, entry.small_rows, entry.align_pages) => {
                    same += 1
                }
                Some(_) => other.push(index),
                None => unjudged += 1,
            }
        }
        println!(
            "  {} entries: row codes as the rule gives {same}, not {}, not judged {unjudged}",
            pack.entries.len(),
            other.len()
        );
        for &index in other.iter().take(8) {
            let entry = pack.entries[index];
            println!(
                "  entry {index}: codes {:?} {} {} against the rule's {:?}",
                entry.row_codes,
                entry.small_rows,
                entry.align_pages,
                pack.codes_by_rule(index)
            );
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("GTA VI texture packs (.ttpf): a texture is keyed by the 64-bit hash of its");
    eprintln!("lower-cased name, which the pack does not hold, and is numbered by its index");
    eprintln!();
    eprintln!("Usage: openrage-cli ttpf <subcommand> [args...]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  info <file.ttpf> [--name NAME]... [--names FILE]");
    eprintln!("        List every texture: its name where a --name hashes to its key, or a");
    eprintln!("        line of FILE does, or a name the texture naming rules give from a");
    eprintln!("        line's stem (a drawable's name), else hash_<key>; size, format, body");
    eprintln!("        codecs, and whether it reads");
    eprintln!("  export <file.ttpf> <outdir> [--format png|dds] [--index N]");
    eprintln!("        Write the readable textures, or texture N, as <index>.png or <index>.dds;");
    eprintln!("        a cube as <index>_px, _nx, _py, _ny, _pz and _nz, one file per face, and");
    eprintln!("        an array as <index>_0, _1 and on, one file per slice.");
    eprintln!("        DDS carries every mip read; PNG the top mip.");
    eprintln!("  replace <file.ttpf> <#index|hash_KEY|name> <image> -o <out.ttpf>");
    eprintln!("        [--format F] [--mips N] [--quality fast|normal|high]");
    eprintln!("        [--filter box|lanczos3] [--srgb|--linear] [--slice N]");
    eprintln!("        --slice N replaces a cube's face, an array's slice or a one-level");
    eprintln!("        volume's z-plane alone, in the texture's own size, format and mips.");
    eprintln!("        Encode the image into the texture's format, size and mips, tiled as the");
    eprintln!("        pack lays it, and write a new pack to <out.ttpf>, never over the input:");
    eprintln!("        every other row as the file stores it, the replaced rows as zlib. An");
    eprintln!("        image of another size, --format or --mips plans the texture's rows anew");
    eprintln!("        as the pack's own textures of that shape have them and moves every");
    eprintln!("        texture after it; no game has been seen to load a pack written so.");
    eprintln!("  calibrate <folder> [--coverage]");
    eprintln!("        Fit the BCPack coder's preset and index tables for BC1, BC3, BC4, BC5,");
    eprintln!("        BC6H and BC7");
    eprintln!("        from every .ttpf under <folder>, against the plain mips beside the coded");
    eprintln!("        ones, and keep them in the configuration directory for those packs. The");
    eprintln!("        tables are in no file and no source gives them, so they are estimates;");
    eprintln!("        the table printed says how far each mode's coded mips are from the plain");
    eprintln!("        mips beside them. Name the folder that holds every pack of the game: a");
    eprintln!("        slot is fitted only from the packs read. --coverage then decodes every");
    eprintln!("        coded body of those packs once and counts, by mode, the textures that");
    eprintln!(
        "        read exactly, as estimates, or not at all, which takes longer than the fit."
    );
    eprintln!("  plan <file.ttpf>...");
    eprintln!("        Print what a save would write against what the file holds: rows whose");
    eprintln!("        offset, pad or kind differ, rows whose start is carried, and entries");
    eprintln!("        whose row-size codes are not the rule's. Reads no body.");
    eprintln!();
    eprintln!("A format code is a DXGI number shifted left by 3, and flags. Read: BC1, BC2, BC3,");
    eprintln!("BC4, BC5, BC7, BC6H unsigned and signed (signed as the floats it decodes to, which");
    eprintln!("has no DDS form here), RGBA8, BGRA8, R8, A8, R16, R16F, R32F, RGBA16 and RGBA16F,");
    eprintln!("in stored or zlib bodies: 2D textures, a top mip over 4 MiB from its 4 MiB rows,");
    eprintln!("cubes of BC blocks from their last row, cubes of 4-byte pixels with each face the");
    eprintln!("lone level of one tile, arrays in 64 KiB tiles, and volumes in 4 KiB tiles slice");
    eprintln!("by slice, each z-slab holding the chain; with every mip of the chain whose tail");
    eprintln!("slot is measured (1-, 4-, 8- and 16-byte elements), and a lone level in one tile.");
    eprintln!("A body of codec kind 1 (BC1 coder in the BODIES column) holds BC1 blocks through");
    eprintln!("the console's BCPack coder and decodes exactly, but for one that reads a preset");
    eprintln!("endpoint no value is known for. That one, and bodies of codec kinds 3 to 7 (BC3,");
    eprintln!(
        "BC4, BC5, BC6H and BC7 through the coder), read once `ttpf calibrate` has fitted the"
    );
    eprintln!("coder's tables from the packs they are in: such a texture is an estimate, and");
    eprintln!("info and export say calibrated for it, with the share of its coded blocks drawn");
    eprintln!("through a slot or symbol the fit did not solve. A save never writes an estimate:");
    eprintln!("it carries the coded bodies as the file stores them. A signed BC6H texture is");
    eprintln!("drawn through the tables the unsigned ones fitted: no plain mip lies beside one,");
    eprintln!("and what checks it is that its blocks' ends stay in range as an encoder's do.");
    eprintln!("Refused, each by name: bodies of codec kinds 3 to 7 with no calibration kept;");
    eprintln!("cubes of 1-, 2- and 8-byte pixels; arrays in 4 KiB tiles;");
    eprintln!("volumes in tile modes other than 1; a mip in a tail with no measured slot (2-byte");
    eprintln!("elements, and past the slots measured); other DXGI numbers; entries and bytes");
    eprintln!("not on disk.");
    eprintln!("Replace takes a 2D texture OpenRage encodes (BC1 to BC5, BC7 with bc7-encode,");
    eprintln!("RGBA8, BGRA8, R8, A8) and refuses, each by name: cubes, arrays and volumes; a");
    eprintln!("tail with mips no measured slot places; a top mip over rows that are not 4 MiB");
    eprintln!("each; row-size codes not the rule's; an image of another size; a pack not");
    eprintln!("wholly on disk. A save carries every other row as the file stores it.");
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(command) = args.first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    let result = match command.as_str() {
        "info" => cmd_info(rest),
        "export" => cmd_export(rest),
        "replace" => cmd_replace(rest),
        "plan" => cmd_plan(rest),
        "calibrate" => cmd_calibrate(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("Error: unknown subcommand '{other}'");
            eprintln!();
            print_usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("Error: {message}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use or_io::gta6::headerless::{Dictionary, DictionaryTexture, Entry, HeaderlessError};
    use or_io::gta6::ttpf::Format;

    use super::*;

    /// A dictionary texture `name` of `slices` 4x4 BC1 sRGB slices, slice s all `0x10 + s`, or one that does not read.
    fn texture(name: &str, slices: u8, reads: bool) -> (u32, Result<Entry, HeaderlessError>) {
        let texture = Texture {
            width: 4,
            height: 4,
            format: Format::of_dxgi(72).expect("BC1 sRGB"),
            levels: 1,
            faces: slices.into(),
            pixels: (0..slices).flat_map(|s| [0x10 + s; 8]).collect(),
        };
        let entry = DictionaryTexture {
            name: name.to_string(),
            width: 4,
            height: 4,
            depth: slices.into(),
            dimension: 1,
            format: 72,
            mip_levels: 1,
            pixels_at: 0,
            pixels_len: 8 * u64::from(slices),
            texture: match reads {
                true => Ok(texture),
                false => Err(HeaderlessError::TextureFormat(34)),
            },
        };
        (0, Ok(Entry::Texture(entry)))
    }

    #[test]
    fn a_scanned_dictionary_exports_each_texture_that_reads_by_name_one_file_a_slice() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let entries = vec![
            texture("zq_glow", 1, true),
            texture("zq/array", 2, true),
            texture("zq_dark", 1, false),
            texture("zq?GLOW", 1, true),
        ];
        export_dictionary(Dictionary { entries }, dir.path(), true).expect("exports");
        let mut written: Vec<String> = std::fs::read_dir(dir.path())
            .expect("listed")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        written.sort();
        assert_eq!(
            written,
            [
                "zq_GLOW_2.dds",
                "zq_array_0.dds",
                "zq_array_1.dds",
                "zq_glow.dds"
            ]
        );
        let slice = or_io::dds::Dds::open(dir.path().join("zq_array_1.dds")).expect("a DDS");
        assert_eq!(slice.data, [0x11; 8], "slice 1 alone");
        assert_eq!(slice.dxt10.map(|x| x.dxgi_format), Some(72), "sRGB kept");

        let none = Dictionary {
            entries: vec![texture("zq_dark", 1, false)],
        };
        assert!(export_dictionary(none, dir.path(), false).is_err());
    }

    #[test]
    fn export_and_its_format_are_split_from_the_other_words() {
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (export, rest) = split_export(&words(&[
            "a.rpf", "--export", "out", "0x10", "--format", "DDS",
        ]))
        .expect("splits");
        assert_eq!(export, Some((PathBuf::from("out"), true)));
        assert_eq!(rest, words(&["a.rpf", "0x10"]));
        assert_eq!(split_export(&words(&["a.rpf"])).expect("splits").0, None);
        assert!(split_export(&words(&["a.rpf", "--format", "png"])).is_err());
        assert!(split_export(&words(&["--export", "out", "--format", "tga"])).is_err());
    }
}
