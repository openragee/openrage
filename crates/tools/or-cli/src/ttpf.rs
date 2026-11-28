//! `openrage-cli ttpf`: list a GTA VI texture pack's textures, export the readable ones, replace
//! one into a new pack, and plan a save against the file's own table.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::codec::texture::{decode_level, save_png, to_dds};
use or_io::gta6::bcpack::calibrate::{
    keep_supplied, load_cache, save_cache, Among, Probed, Summary,
};
use or_io::gta6::bcpack::{Checked, Decode, Reading, Standing};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::texture_names::{name_keys, use_label};
use or_io::gta6::ttpf::{
    calibrate_packs_from, coverage, keep_checked, probe_packs, score_packs, Coverage, Format,
    Texture, TexturePack, KIND_STORED, KIND_ZLIB,
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

/// How the tables file `pack` reads through stands once `all` is counted, what a listing or an
/// export read of the pack: kept beside the file, so that a later read of the pack says it too.
///
/// ⚠ A listing words its textures by this, after every one of them is read: a texture that read
/// clean before another counted a block against the file was read through the same file.
fn standing_after(pack: &TexturePack, all: &Decode) -> Standing {
    match calibration_dir().map(|dir| pack.keep_read(&dir, all)) {
        Some(Ok(standing)) => standing,
        Some(Err(e)) => {
            eprintln!("what this read counted against the tables supplied is not kept: {e}");
            pack.standing_after(all)
        }
        None => pack.standing_after(all),
    }
}

/// What a texture with a body through the coder may be called, for a line that lists or exports
/// it: `standing` is how the tables file its pack reads through stands.
fn decode_note(how: &Decode, standing: Standing) -> String {
    match how.reading(standing) {
        Reading::Exact => "exact".to_string(),
        Reading::BuiltIn => "NOT called exact: through BC1's built-in tables alone, which \
                             nothing confirms value for value"
            .to_string(),
        Reading::Unseconded { blocks } => format!(
            "NOT called exact: {blocks} of its coded blocks read a value BC1's built-in tables \
             give otherwise, or not at all"
        ),
        Reading::Calibrated => format!(
            "calibrated: an estimate through tables fitted on this machine, {:.1}% of its coded \
             blocks through a slot or symbol not known",
            how.unknown_percent()
        ),
        Reading::Unchecked => "NOT called exact yet: through the tables supplied, which no \
                               check has held to blocks of its format"
            .to_string(),
        Reading::Unruled => "NOT called exact: through the tables supplied, and no rule of its \
                             blocks' format can show a wrong table"
            .to_string(),
        Reading::Disagrees { disagree, blocks } => format!(
            "NOT exact: {disagree} of its {blocks} coded blocks read a value no table gives or \
             hold what no encoder writes, through the tables supplied"
        ),
        Reading::NotTheCoders { .. } => {
            "NOT exact: blocks were counted against the tables supplied, which are not this \
             coder's"
                .to_string()
        }
        // Nothing through the coder was rebuilt, or a body under the ones that were was not.
        Reading::CodedUnread => match how.supplied {
            0 => "NOT called exact: its plain levels were read and its coded levels were not",
            _ => {
                "NOT called exact: a coded body of it was not read, and only the levels above \
                 that body were"
            }
        }
        .to_string(),
    }
}

/// How the tables file a pack reads through stands, in sentences.
fn standing_note(standing: Standing) -> String {
    let counted = |found: Checked| {
        format!(
            "found {} of {} coded blocks that read a value no table gives or hold what no \
             encoder writes",
            found.disagree, found.blocks
        )
    };
    if let Some(failed) = standing.failed() {
        return format!(
            "They are NOT this coder's tables: a check {}.",
            counted(failed)
        );
    }
    if let Some(here) = standing.failed_in_pack() {
        return format!(
            "They are NOT this coder's tables for this pack: a read of its textures {}, \
             and no texture of it read through them is called exact.",
            counted(here)
        );
    }
    let of = |kind: u8, name: &str| match standing.held(kind) {
        Some(blocks) => format!(
            "A check held {blocks} {name} blocks to the rules of their format, and none broke \
             one."
        ),
        None => format!("No check has read a {name} block through them."),
    };
    format!(
        "{} {} A BC6H or BC7 texture is called exact only once `ttpf tables --check <folder>` \
         has held the tables of its format and none of its own blocks breaks a rule; a BC1 \
         texture only where BC1's built-in tables give the same values; a BC3, BC4 or BC5 \
         texture never, since no rule of those formats can show a wrong table.",
        of(6, "BC6H"),
        of(7, "BC7")
    )
}

/// What the textures of one pack just read said against the tables supplied, where any did.
fn pack_verdict(all: &Decode) -> Option<String> {
    (all.supplied > 0 && all.disagree() > 0).then(|| {
        format!(
            "The tables supplied are not this coder's tables for this pack: {} of {} coded \
             blocks read a value no table gives or hold what no encoder writes. That is kept \
             with the pack, and no texture of it read through them is called exact.",
            all.disagree(),
            all.supplied
        )
    })
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
    // A pack with no row table states no length: its file's is the one it has.
    let sentinel = (pack.rows.last()).map_or(pack.size_on_disk(), |row| row.offset);
    let state = if pack.is_complete() {
        "complete".to_string()
    } else if pack.size_on_disk() == sentinel {
        "full length but sparse, so pieces of a download may still be missing".to_string()
    } else {
        format!("short: {} of {sentinel} bytes on disk", pack.size_on_disk())
    };
    println!(
        "{}: {} texture(s) for {} in {} rows; {state}\n",
        path.display(),
        pack.entries.len(),
        pack.platform().label(),
        pack.rows.len()
    );
    // A line up to what its texture's read is called, and the read where the texture has a
    // body through the coder: the word waits for what every texture of the pack counted.
    let mut lines: Vec<(String, Option<Decode>)> = Vec::new();
    let mut all = Decode::default();
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
        let (read, how) = match pack.texture_noted(index) {
            Ok((texture, how)) => {
                let mips = match texture.faces {
                    1 => format!("yes, {} mip(s)", texture.levels),
                    faces => format!("yes, {faces} slices of {} mip(s)", texture.levels),
                };
                all.add(how);
                // A texture with a body through the coder says what its read may be called.
                (mips, coded.then_some(how))
            }
            Err(e) => (format!("no: {e}"), None),
        };
        let line = format!(
            "{index:>6} {name:<22} {:>11} {format:<22} {:<20} {read}",
            format!("{}x{}", entry.width, entry.height),
            kinds.join(", ")
        );
        lines.push((line, how));
    }
    let standing = standing_after(&pack, &all);
    if (pack.rows.iter()).any(|row| pack.reads_supplied(row.kind)) {
        println!(
            "Coded bodies read through the tables supplied. {}\n",
            standing_note(standing)
        );
    }
    println!(
        "{:>6} {:<22} {:>11} {:<22} {:<20} READ",
        "INDEX", "NAME", "SIZE", "FORMAT", "BODIES"
    );
    for (line, how) in lines {
        match how {
            Some(how) => println!("{line}, {}", decode_note(&how, standing)),
            None => println!("{line}"),
        }
    }
    if let Some(verdict) = pack_verdict(&all) {
        println!("\n{verdict}");
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
    let mut refused = 0usize;
    let (mut estimates, mut doubted, mut unseconded, mut partly) = (0usize, 0usize, 0usize, 0);
    // The files of each texture written and how it read: what its read is called waits for
    // what every texture exported counted.
    let mut wrote: Vec<(Vec<String>, Decode)> = Vec::new();
    let mut all = Decode::default();
    let mut failed = None;
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
        all.add(how);
        match write_faces(out_dir, &index.to_string(), texture, cube, as_dds) {
            Ok(files) => wrote.push((files, how)),
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    let standing = standing_after(&pack, &all);
    let written = wrote.len();
    for (files, how) in wrote {
        // An export that is not exact says so on its line.
        let reading = how.reading(standing);
        let note = match reading {
            Reading::Exact => String::new(),
            _ => format!("; {}", decode_note(&how, standing)),
        };
        estimates += usize::from(reading == Reading::Calibrated);
        unseconded += usize::from(matches!(
            reading,
            Reading::BuiltIn | Reading::Unseconded { .. }
        ));
        doubted += usize::from(matches!(
            reading,
            Reading::Unchecked
                | Reading::Unruled
                | Reading::Disagrees { .. }
                | Reading::NotTheCoders { .. }
        ));
        partly += usize::from(reading == Reading::CodedUnread);
        for file in files {
            println!("{file}{note})");
        }
    }
    if let Some(e) = failed {
        return Err(e);
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
    if unseconded > 0 {
        println!(
            "{unseconded} of them are BC1 textures not called exact: BC1's built-in tables were \
             fitted and nothing confirms them value for value, so a block is called exact only \
             where a tables file you supply (`ttpf tables <file>`) gives the same value"
        );
    }
    if partly > 0 {
        println!(
            "{partly} of them are not called exact: a body of each through the coder was not \
             read, and only the levels above that body are written"
        );
    }
    if doubted > 0 {
        println!(
            "{doubted} of them read through the tables supplied and are not called exact. {}",
            standing_note(standing)
        );
    }
    if let Some(verdict) = pack_verdict(&all) {
        println!("{verdict}");
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

/// Every pack under `folder` that opens and whose envelope has the BCPack coder, in the order of their paths.
fn open_under(folder: &str) -> Result<Vec<TexturePack>, String> {
    let mut paths = Vec::new();
    packs_under(Path::new(folder), &mut paths)?;
    paths.sort();
    let mut packs = Vec::new();
    for path in &paths {
        match TexturePack::open(path) {
            Ok(pack) if pack.envelope().has_coder() => packs.push(pack),
            Ok(_) => {}
            Err(e) => eprintln!("{}: {e}", path.display()),
        }
    }
    match packs.is_empty() {
        true => Err(format!(
            "no texture pack whose envelope has the BCPack coder opens under {folder}"
        )),
        false => Ok(packs),
    }
}

/// Says how far a run is, a line that is written over.
fn percent_said() -> impl FnMut(u64, u64) {
    let mut said = u64::MAX;
    move |done: u64, total: u64| {
        let percent = 100 * done / total.max(1);
        if percent != said {
            said = percent;
            eprint!("\r{percent:>3}%");
        }
    }
}

/// How far each mode's coded mips are from the plain mips beside them, a line a mode.
fn print_summaries(summaries: &[Summary]) {
    println!(
        "{:<5} {:>7} {:>7} {:>10} {:>10} {:>6} {:>6} {:>10}  AGAINST THE PLAIN MIPS",
        "MODE", "CHAINS", "BODIES", "BLOCKS", "NOT KNOWN", "SLOTS", "PAIRS", "QUADRANTS"
    );
    for summary in summaries {
        let mismatch = match (summary.mean_mismatch(), summary.mean_alpha_mismatch()) {
            (Some(mean), _) if summary.kind == 6 => format!("{mean:.4} stops off"),
            (Some(mean), Some(alpha)) => format!("colour {mean:.4}, alpha {alpha:.4} of 255 off"),
            (Some(mean), None) => format!("{mean:.4} of 255 off"),
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
}

/// How every coded texture of a set of packs reads, a line a mode.
fn print_coverage(found: &[Coverage]) {
    println!(
        "{:<5} {:>9} {:>9} {:>9} {:>10} {:>8} {:>12} {:>12} {:>10} {:>8}",
        "MODE",
        "CLEAN",
        "DISAGREE",
        "ONE TABLE",
        "CALIBRATED",
        "REFUSED",
        "BLOCKS",
        "FITTED",
        "NOT KNOWN",
        "STRAY"
    );
    for of_kind in found {
        println!(
            "{:<5} {:>9} {:>9} {:>9} {:>10} {:>8} {:>12} {:>12} {:>10} {:>8}",
            format!("BC{}", of_kind.kind),
            of_kind.clean,
            of_kind.disagree,
            of_kind.unseconded,
            of_kind.calibrated,
            of_kind.refused,
            of_kind.blocks,
            of_kind.fitted,
            of_kind.unknown,
            of_kind.stray
        );
    }
    for of_kind in found.iter().filter(|of_kind| of_kind.unseconded_blocks > 0) {
        println!(
            "BC{}: {} bodies hold {} blocks only one table stands behind.",
            of_kind.kind, of_kind.unseconded_bodies, of_kind.unseconded_blocks
        );
    }
    println!(
        "CLEAN, DISAGREE, ONE TABLE, CALIBRATED and REFUSED count textures: those none of whose \
         coded blocks is fitted or counted here, those through supplied tables with a block \
         counted as NOT KNOWN or STRAY, BC1 textures with a block only one table stands behind, \
         those through fitted tables, and those with a body that does not read. Only one table \
         stands behind a BC1 block read through the built-in tables alone, and behind one read \
         through supplied tables where it reads a preset slot or a quadrant symbol the built-in \
         tables give another value for, or none. NOT KNOWN counts \
         the blocks drawn through a preset slot or an index symbol no table gave a value. STRAY \
         counts the BC6H and BC7 blocks no encoder writes: an end out of its base's range, an \
         anchor's index past half its range, a shared p-bit two ends disagree on. No rule holds \
         a BC1, BC3, BC4 or BC5 block, so their STRAY is 0 whatever the tables."
    );
}

/// The fewest coded levels that have to read a value for a probe to say how it stands: the
/// blocks of one level are of one picture, and under that a few pictures decide.
const PROBE_LEVELS: u64 = 8;

/// What moving each value of each table one step did, a line a table and a band of how many
/// levels read a value.
fn print_probe(probed: &Probed) {
    println!(
        "{:<12} {:<16} {:>7} {:>7} {:>7} {:>7}",
        "TABLE", "READ BY", "VALUES", "LEAST", "TIED", "BEATEN"
    );
    let bands: [(&str, u64, u64); 4] = [
        ("no level", 0, 0),
        ("1 to 7 levels", 1, PROBE_LEVELS - 1),
        ("8 to 63 levels", PROBE_LEVELS, 63),
        ("64 or more", 64, u64::MAX),
    ];
    for table in &probed.tables {
        let name = format!(
            "BC{} {}",
            table.kind,
            match (table.stored, table.indices) {
                (true, _) => "stored",
                (false, true) => "indices",
                (false, false) => "ends",
            }
        );
        for (band, from, to) in bands {
            let read = || (table.values.iter()).filter(|moved| (from..=to).contains(&moved.levels));
            let stood = |how: Among| read().filter(|moved| moved.among() == Some(how)).count();
            let values = read().count();
            if values > 0 {
                println!(
                    "{name:<12} {band:<16} {values:>7} {:>7} {:>7} {:>7}",
                    stood(Among::Least),
                    stood(Among::Tied),
                    stood(Among::Beaten)
                );
            }
        }
    }
    println!(
        "A value is a field of a preset slot, an index of a preset slot, or a pixel of an index \
         table's symbol. Each was moved one step down and one step up, a step being the least \
         one the block that reads it answers to, and the blocks that read it were held against \
         the plain mips beside them each way. LEAST: the table's value meets them better than \
         both; TIED: as well as the better; BEATEN: a value beside it meets them better. A \
         plain mip is another image and the blocks of one level are of one picture, so a value \
         few levels read says little: under {PROBE_LEVELS} levels it is not determined. The \
         rows named stored move the indices of \
         the plain mips' own blocks the same way, each under the symbol the table gives the \
         pattern it holds: those blocks are right, so what is BEATEN there is the measure and \
         not the table."
    );
    if probed.bc1_blocks > 0 {
        let mean = |sum: f64| sum / probed.bc1_compared.max(1) as f64;
        println!(
            "BC1: {} of the blocks read read a value the built-in tables give otherwise; they \
             are {:.4} of 255 off the plain mips through the tables supplied and {:.4} through \
             the built-in ones.",
            probed.bc1_blocks,
            mean(probed.bc1_through),
            mean(probed.bc1_built_in)
        );
    }
}

/// Keep a tables file the user names as the coder's tables, or check what is kept against a folder's packs.
fn cmd_tables(args: &[String]) -> Result<(), String> {
    const USAGE: &str = "usage: ttpf tables <file> | ttpf tables --check <folder> [--probe]";
    let dir = calibration_dir().ok_or("no configuration directory to keep the tables in")?;
    let (folder, probe) = match args {
        [flag, folder] if flag == "--check" => (folder, false),
        [flag, folder, probe] if flag == "--check" && probe == "--probe" => (folder, true),
        [file] if !file.starts_with('-') => {
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let path = keep_supplied(&dir, &bytes).map_err(|e| format!("{file}: {e}"))?;
            // The file kept may be one a check has read through before.
            let kept = load_cache(&dir, 0).ok_or("the tables kept do not read back")?;
            println!(
                "Kept in {} as the tables supplied, digest {:016x}: every pack now reads BC1, \
                 BC3, BC4, BC5, BC6H and BC7 bodies through them, in place of a calibration. \
                 That the file reads says its layout is a tables file's, not that its values \
                 are this coder's. {}",
                path.display(),
                kept.digest(),
                standing_note(kept.standing())
            );
            return Ok(());
        }
        _ => return Err(USAGE.into()),
    };
    let mut packs = open_under(folder)?;
    let Some(tables) = load_cache(&dir, packs[0].member_key()) else {
        // With no tables kept only BC1 reads, through its built-in ones.
        println!(
            "{} pack(s) under {folder}, with no tables kept: neither supplied ones nor a \
             calibration\n",
            packs.len()
        );
        print_coverage(&coverage(&packs, &|| false).ok_or("stopped")?);
        return Ok(());
    };
    let supplied = [1, 3, 4, 5, 6, 7]
        .into_iter()
        .all(|kind| tables.supplied(kind));
    println!(
        "{} pack(s) under {folder}, through {}\n",
        packs.len(),
        match supplied {
            true => format!("the tables supplied, digest {:016x}", tables.digest()),
            false => "the calibration kept for them".to_string(),
        }
    );
    let tables = std::sync::Arc::new(tables);
    for pack in &mut packs {
        pack.set_calibration(Some(tables.clone()));
    }
    // Every coded body decoded once, then the levels a calibration reads drawn against the plain ones.
    let found = coverage(&packs, &|| false).ok_or("stopped")?;
    print_coverage(&found);
    if let Some((slots, symbols)) = tables.bc1_apart_counts() {
        println!(
            "BC1's tables as supplied and as built in give another value for {slots} preset \
             slots and {symbols} quadrant symbols: a BC1 block that reads one of those is one \
             only the tables supplied stand behind."
        );
    }
    println!();
    if supplied {
        // What the check found is kept by mode with the file's digest, beside what earlier
        // checks of the same file found, and every later read says it.
        let standing =
            keep_checked(&dir, &tables, &found).map_err(|e| format!("{}: {e}", dir.display()))?;
        if standing.failed().is_some() {
            return Err(standing_note(standing));
        }
        println!(
            "{} A count of none shows that the entries of BC6H's and BC7's preset lists that \
             these blocks read are in this coder's order, with BC7's p-bits and the anchors' top \
             bits. It does not show each end value of each slot, nor the place of an entry few \
             blocks read, nor anything of a BC3, BC4 or BC5 table, which no rule sees and no \
             second table confirms. BC1's tables are held against the built-in ones above. \
             --probe holds each value to the plain mips.\n",
            standing_note(standing)
        );
    }
    let scored = score_packs(&packs, &tables, &mut percent_said(), &|| false).ok_or("stopped")?;
    eprintln!();
    print_summaries(&scored);
    if probe {
        // The rules above see an entry's place, a p-bit and an anchor's top bit; this holds
        // each value to the pixels.
        let probed = probe_packs(&packs, &tables, &mut percent_said(), &|| false);
        eprintln!();
        println!();
        print_probe(&probed.ok_or("stopped")?);
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
    let mut packs = open_under(folder)?;
    let mut progress = percent_said();
    // A run cut short is gone on from by the next one, at the pass it was in.
    let dir = calibration_dir().ok_or("no configuration directory to keep the calibration in")?;
    let found =
        calibrate_packs_from(&packs, Some(&dir), &mut progress, &|| false).ok_or("stopped")?;
    eprintln!();
    println!("Calibrated from {} pack(s) under {folder}\n", packs.len());
    print_summaries(&found.summaries);
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
    println!();
    print_coverage(&coverage(&packs, &|| false).ok_or("stopped")?);
    println!(
        "The tables are fitted on this machine, not read from a source: a texture drawn through \
         them is an estimate, and `ttpf info` and `ttpf export` say which are."
    );
    Ok(())
}

/// Write `texture` into `out_dir` as `<stem>.png` or `.dds`, a cube's faces, in their stored
/// order, and an array's or a volume's slices as files of their own; a line for each file
/// written, open for what the caller says of the texture before it closes the bracket.
fn write_faces(
    out_dir: &Path,
    stem: &str,
    texture: Texture,
    cube: bool,
    as_dds: bool,
) -> Result<Vec<String>, String> {
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
    let mut written = Vec::new();
    for (name, face) in faces {
        let path = out_dir.join(name);
        if as_dds {
            to_dds(&face).and_then(|dds| dds.save(&path))
        } else {
            decode_level(&face, 0).and_then(|image| save_png(&path, &image))
        }
        .map_err(|e| format!("{}: {e}", path.display()))?;
        written.push(format!(
            "{}  ({}x{} {}, {} mip(s)",
            path.display(),
            face.width,
            face.height,
            face.format.name(),
            face.levels
        ));
    }
    Ok(written)
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
                for file in write_faces(out, &stem, texture, t.dimension == 2, as_dds)? {
                    println!("{file})");
                }
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
    if let Some(kind) = or_io::gta6::ttpf_author::bcpack_kind(&pack, index) {
        return Err(or_io::gta6::ttpf_author::AuthorError::BcPack { index, kind }.to_string());
    }
    let image = or_io::authoring::texture_import::load(Path::new(image))
        .map_err(|e| format!("{image}: {e}"))?;
    let was = pack.entries[index];
    let options = opts.build_options(was.format().is_some_and(|f| f.is_srgb()));
    // An image of another size, a format or a mip count plans the texture's rows anew.
    let reshaped = opts.format.is_some()
        || opts.mips.is_some()
        || (image.width, image.height) != (was.width, was.height);
    let mips = if reshaped {
        let format = match opts.format {
            Some(grc) => Format::of_grc(grc, options.srgb)
                .ok_or_else(|| format!("{grc:?} is no format a GTA VI pack is measured to hold"))?,
            None => was.format().ok_or_else(|| {
                format!("texture {index}'s format is not a measured one; name one with --format")
            })?,
        };
        // The image is for slice --slice, 0 by default; every other slice takes its own top mip
        // resampled to the image's size.
        let shown = slice.unwrap_or(0);
        (pack.slice_sources(index, Some(&image), shown, options.filter))
            .and_then(|sources| pack.reshape_slices(index, &sources, format, &options))
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
    eprintln!("        volume's z-plane alone, in the texture's own size, format and mips; with");
    eprintln!("        another size, --format or --mips it names the slice the image is for,");
    eprintln!("        and every other slice is encoded from its own top mip resampled to the");
    eprintln!("        image's size.");
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
    eprintln!("        read with nothing against them, as estimates, or not at all, which takes");
    eprintln!("        longer than the fit.");
    eprintln!("  tables <file> | tables --check <folder> [--probe]");
    eprintln!("        Read the BCPack coder's preset and index tables from <file>, a tables file");
    eprintln!("        you supply in the layout they are published in, and keep them in the");
    eprintln!("        configuration directory. Every pack of every install then reads its coded");
    eprintln!("        bodies through them, in place of a calibration. OpenRage ships no such");
    eprintln!("        file and vouches for none, and a file that reads is not thereby the");
    eprintln!("        coder's: --check decodes every coded body of the packs under <folder>");
    eprintln!("        through the tables kept for them, supplied or fitted, and counts by mode");
    eprintln!("        the blocks through a value no table gives and the blocks no encoder");
    eprintln!("        writes, then says how far the coded mips are from the plain mips beside");
    eprintln!("        them. Tables that are right leave none of either; supplied tables that");
    eprintln!("        leave one are reported as not this coder's, and the command fails. What");
    eprintln!("        a check found is kept by mode with the file's digest, and a failure stands");
    eprintln!("        whatever a later check of other packs finds. The count said of a failure");
    eprintln!("        is one check's own, and the count held of a mode the largest check's. An");
    eprintln!("        info or an export that counts such a block fails the file for that pack:");
    eprintln!("        the count is kept with the pack and the file's digest, and no texture of");
    eprintln!("        the pack is called exact through that file, in that listing or after. info");
    eprintln!("        says exact for a texture every coded body of which was read, and export");
    eprintln!("        says nothing, only on one of two grounds. A BC6H or");
    eprintln!("        BC7 texture: none of its blocks breaks a rule of its format and a check");
    eprintln!("        has held the tables of that format. A BC1 texture: every value it reads");
    eprintln!("        is the same in the file and in BC1's built-in tables. A BC3, BC4 or BC5");
    eprintln!("        texture is never called exact, since no rule of those formats can show a");
    eprintln!("        wrong table. Otherwise they say how many blocks disagree, how many BC1");
    eprintln!("        blocks read a value the two tables give differently, or that the tables");
    eprintln!("        are not checked for the format. Through the blocks that read an entry");
    eprintln!("        the rules see its place in its list, a p-bit and an anchor's top bit, and");
    eprintln!("        not each end value. --probe then moves each value of each table");
    eprintln!("        one step down and one step up and counts the values that meet the plain");
    eprintln!(
        "        mips beside the coded ones better than both, as well, or worse, by how many"
    );
    eprintln!("        coded levels read each: a value few levels read is not determined by it.");
    eprintln!("        Of a folder with more than 1,024 BC1 textures it reads one in eight.");
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
    eprintln!("the console's BCPack coder. With a tables file kept by `ttpf tables`, a body of");
    eprintln!("every coder kind reads through it, exactly as far as its check says. Without one,");
    eprintln!("kind 1 decodes through its built-in tables, which were fitted and which nothing");
    eprintln!("confirms value for value: such a texture is said to rest on them and is not called");
    eprintln!("exact. With a tables file kept, a BC1 texture says how many of its blocks read a");
    eprintln!("value the two tables give differently. A read stops at the first body that does");
    eprintln!("not read and gives the levels above it: a texture none of whose coded bodies was");
    eprintln!("read says that its plain levels were read and its coded levels were not, one a");
    eprintln!("coded body of which was not read says that, and neither is called exact. A body");
    eprintln!("that reads a preset endpoint the built-in tables have no value for, and bodies of");
    eprintln!("codec kinds 3 to 7 (BC3,");
    eprintln!(
        "BC4, BC5, BC6H and BC7 through the coder), read once `ttpf calibrate` has fitted the"
    );
    eprintln!("coder's tables from the packs they are in: such a texture is an estimate, and");
    eprintln!("info and export say calibrated for it, with the share of its coded blocks drawn");
    eprintln!("through a slot or symbol the fit did not solve. A save never writes an estimate:");
    eprintln!("it carries the coded bodies as the file stores them. A signed BC6H texture is");
    eprintln!("drawn through the tables the unsigned ones fitted: no plain mip lies beside one,");
    eprintln!("and what checks it is that its blocks' ends stay in range as an encoder's do.");
    eprintln!("Refused, each by name: bodies of codec kinds 3 to 7 with no tables kept;");
    eprintln!("cubes of 1-, 2- and 8-byte pixels; arrays in 4 KiB tiles;");
    eprintln!("volumes in tile modes other than 1; a mip in a tail with no measured slot (2-byte");
    eprintln!("elements, and past the slots measured); other DXGI numbers; entries and bytes");
    eprintln!("not on disk.");
    eprintln!("Replace takes a texture OpenRage encodes (BC1 to BC5, BC7 with bc7-encode,");
    eprintln!("RGBA8, BGRA8, R8, A8) and refuses, each by name: another shape for an array in");
    eprintln!("4 KiB tiles, and a volume of several mips; a tail");
    eprintln!("with mips no measured slot places; a top mip over rows that are not 4 MiB each;");
    eprintln!("row-size codes not the rule's; a pack not wholly on disk. A save carries every");
    eprintln!("other row as the file stores it.");
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
        "tables" => cmd_tables(rest),
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
