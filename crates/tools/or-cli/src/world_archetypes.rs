//! `world archetypes`: a `.ytyp` generated from the drawables it declares.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_io::container::resource::ResourceType;
use or_io::decl::{Gen8, Gen9};
use or_io::gta5::{File as _, GtaDrawable, GtaFragType, PgDictionary};
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, ErrorMode};
use or_io::Generation;
use or_world::generate::{self, Agreement, AssetKind, Authoring, Closeness, DrawableFacts};
use or_world::map_types::MapTypes;

macro_rules! facts_of {
    ($lod:expr, $name:expr, $kind:expr, $dict:expr) => {{
        let g = &$lod;
        let (b, s) = (&g.bounds_box, &g.bounds_sphere.center_and_radius);
        DrawableFacts {
            name: $name,
            kind: $kind,
            dictionary: $dict,
            bb_min: Vec3::new(b.min.x, b.min.y, b.min.z),
            bb_max: Vec3::new(b.max.x, b.max.y, b.max.z),
            bs_centre: Vec3::new(s.x, s.y, s.z),
            bs_radius: s.w,
        }
    }};
}

/// A dictionary entry's own name where it hashes to its key, else the key.
///
/// ⚠ A `.ydd` stores entries by hash alone, so the name is recoverable only
/// when the drawable happens to carry it.
fn entry_name(key: u32, drawable_name: &str) -> String {
    let stem = drawable_name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !stem.is_empty() && or_io::string_hash(stem.as_bytes()) == key {
        stem
    } else {
        format!("0x{key:08X}")
    }
}

/// Every drawable one file declares, at the target its RSC7 version names.
fn read(path: &Path) -> Result<Vec<DrawableFacts>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let resource = match ext.as_str() {
        "ydr" => ResourceType::Ydr,
        "ydd" => ResourceType::Ydd,
        "yft" => ResourceType::Yft,
        other => return Err(format!("not a drawable: .{other}")),
    };
    let gen9 = crate::target_of_resource(resource, &bytes) == Generation::Gen9;
    let e = |e: or_io::Error| e.to_string();
    macro_rules! drawable {
        ($t:ty) => {{
            let d = GtaDrawable::<$t>::from_bytes(&bytes).map_err(e)?;
            vec![facts_of!(d.lod_group, stem, AssetKind::Drawable, None)]
        }};
    }
    macro_rules! dictionary {
        ($t:ty) => {{
            let dict = PgDictionary::<GtaDrawable<$t>>::from_bytes(&bytes).map_err(e)?;
            (dict.codes.iter().zip(&dict.entries))
                .filter_map(|(key, d)| {
                    let d = d.as_deref()?;
                    let name = entry_name(*key, &d.name);
                    let kind = AssetKind::DrawableDictionary;
                    Some(facts_of!(d.lod_group, name, kind, Some(stem.clone())))
                })
                .collect()
        }};
    }
    macro_rules! fragment {
        ($t:ty) => {{
            let f = GtaFragType::<$t>::from_bytes(&bytes).map_err(e)?;
            let d = f
                .common_drawable
                .as_deref()
                .ok_or("the fragment has no drawable")?;
            vec![facts_of!(d.lod_group, stem, AssetKind::Fragment, None)]
        }};
    }
    Ok(match (resource, gen9) {
        (ResourceType::Ydr, false) => drawable!(Gen8),
        (ResourceType::Ydr, true) => drawable!(Gen9),
        (ResourceType::Ydd, false) => dictionary!(Gen8),
        (ResourceType::Ydd, true) => dictionary!(Gen9),
        (_, false) => fragment!(Gen8),
        (_, true) => fragment!(Gen9),
    })
}

fn gather(inputs: &[&String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for input in inputs {
        let path = PathBuf::from(input);
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                out.extend(entries.flatten().map(|e| e.path()).filter(|p| {
                    p.extension().and_then(|s| s.to_str()).is_some_and(|e| {
                        ["ydr", "ydd", "yft"]
                            .iter()
                            .any(|w| w.eq_ignore_ascii_case(e))
                    })
                }));
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

pub fn run(args: &[String]) -> ExitCode {
    let value = |flag| super::world::flag_value(args, flag);
    let positional: Vec<&String> = {
        let mut skip = false;
        args.iter()
            .filter(|a| {
                if skip {
                    skip = false;
                    return false;
                }
                if a.starts_with("--") {
                    skip = !a.contains('=');
                    return false;
                }
                true
            })
            .collect()
    };
    let (Some(name), Some((out, inputs))) = (value("--name"), positional.split_last()) else {
        eprintln!(
            "Error: world archetypes [--like <donor.ytyp>] --name <types-name> \
             <drawable-or-dir>... <out.ytyp>"
        );
        return ExitCode::from(2);
    };
    let donor = value("--like");
    if donor.is_some() && inputs.is_empty() {
        eprintln!("Error: no drawables given");
        return ExitCode::from(2);
    }

    let mut authoring = Authoring::default();
    if let Some(d) = value("--lod-dist").and_then(|v| v.parse().ok()) {
        authoring.lod_dist = d;
    }
    authoring.texture_dictionary = value("--txd").map(str::to_string);

    let mut facts = Vec::new();
    for path in gather(inputs) {
        match read(&path) {
            Ok(found) => facts.extend(found),
            Err(e) => {
                eprintln!("Error: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let Some(donor) = donor else {
        return from_nothing(args, name, out, &facts, &authoring);
    };
    let bytes = match std::fs::read(donor) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {donor}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    let (donor_tree, layout) =
        match parse_meta_binary_full(crate::meta::schema(), &bytes, &mut diags) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("Error: {donor}: {e}");
                return ExitCode::FAILURE;
            }
        };
    let Some(spelling) = generate::asset_type_enum_of(&donor_tree) else {
        eprintln!("Error: {donor} holds no archetype to take the asset-type enum from");
        return ExitCode::FAILURE;
    };
    let archetypes = facts
        .iter()
        .map(|f| generate::archetype(f, &authoring, &spelling))
        .collect();
    let name_hash = super::world::name_or_hash(name);
    let tree = match generate::type_file(&donor_tree, name_hash, archetypes) {
        Ok(tree) => tree,
        Err(e) => {
            eprintln!("Error: {donor}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let encoded = match or_world::write::encode(&tree, &layout) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    written(out, &encoded, &facts, &authoring, name_hash)
}

/// The type file `name` made from nothing, declaring one archetype for each of `facts`.
fn from_nothing(
    args: &[String],
    name: &str,
    out: &str,
    facts: &[DrawableFacts],
    authoring: &Authoring,
) -> ExitCode {
    let format = match super::world_new::container(args) {
        Ok(format) => format,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::from(2);
        }
    };
    let mut types = or_world::create::types(name);
    types.archetypes.extend(
        facts
            .iter()
            .map(|f| or_world::create::archetype(f, authoring)),
    );
    let encoded = match or_world::create::types_file(&types, format) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let status = written(out, &encoded, facts, authoring, types.name.hash);
    if status == ExitCode::SUCCESS && !facts.is_empty() {
        println!(
            "  Made from nothing, as PSO: no archetype states an assetType, which the typed \
             family does not name yet."
        );
    }
    status
}

/// Reads `encoded` back, holds each archetype to the drawable it was generated from, and writes it to `out`.
fn written(
    out: &str,
    encoded: &[u8],
    facts: &[DrawableFacts],
    authoring: &Authoring,
    name_hash: u32,
) -> ExitCode {
    // ⚠ Read back before writing. An encoder whose output its own reader
    // does not return is one the game will not open either.
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    let back = match parse_meta_binary_full(crate::meta::schema(), encoded, &mut diags) {
        Ok((back, _)) => back,
        Err(e) => {
            eprintln!("Error: the generated file does not read back: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(types) = MapTypes::new(&back) else {
        eprintln!("Error: the generated file does not read back as a CMapTypes");
        return ExitCode::FAILURE;
    };
    let read_back: Vec<_> = types.archetypes().collect();
    let exact = read_back
        .iter()
        .zip(facts)
        .filter(|(a, f)| {
            Agreement::of(a, f).is_some_and(|g| {
                [g.bb_min, g.bb_max, g.bs_centre, g.bs_radius]
                    .iter()
                    .all(|c| *c == Closeness::Exact)
            })
        })
        .count();
    if read_back.len() != facts.len() || exact != facts.len() {
        eprintln!(
            "Error: {} of {} archetypes read back with the bounds they were generated from",
            exact,
            facts.len()
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(out, encoded) {
        eprintln!("Error: {out}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "{out}: {} bytes, name {name_hash:#010X}, {} archetype(s), every one reading back \
         with its drawable's bounds",
        encoded.len(),
        facts.len()
    );
    println!(
        "  lodDist {} and textureDictionary {} are authoring decisions no drawable records",
        authoring.lod_dist,
        authoring
            .texture_dictionary
            .as_deref()
            .unwrap_or("<the asset's own name>")
    );
    ExitCode::SUCCESS
}
