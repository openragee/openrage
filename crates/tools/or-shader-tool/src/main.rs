#[cfg(feature = "render-drawable")]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[cfg(feature = "native-front")]
use or_shader_tool::pack_protocol::{throttled, Event};
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod rdna_command;

fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("or_shader_tool=info".parse().unwrap())
                // The device check counts what wgpu refuses, and each refusal also logs its whole shader.
                .add_directive("wgpu=off".parse().unwrap())
                .add_directive("wgpu_core=off".parse().unwrap())
                .add_directive("wgpu_hal=off".parse().unwrap()),
        )
        .init();

    let mut args: Vec<String> = std::env::args().collect();
    let program = args.remove(0);

    if args.is_empty() {
        print_usage(&program);
        return ExitCode::from(1);
    }

    match args.remove(0).as_str() {
        "defs" => run_defs(&args, &program),
        "inventory" => run_inventory(&args),
        "spv-check" => run_spv_check(&args),
        "translate-check" => run_translate_check(&args),
        "pass-check" => run_pass_check(&args),
        "bake-pack" => run_bake_pack(&args),
        "pack-identity" => run_pack_identity(),
        "pack-digest" => run_pack_digest(&args),
        "backend-check" => run_backend_check(&args),
        "render-drawable" => run_render_drawable(&args),
        "render-census" => run_render_census(&args),
        "rdna-devices" => rdna_command::run(&args),
        "-h" | "--help" | "help" => {
            print_usage(&program);
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("Unknown command: {other}");
            print_usage(&program);
            ExitCode::from(1)
        }
    }
}

fn run_defs(args: &[String], program: &str) -> ExitCode {
    let Some(sub) = args.first().map(String::as_str) else {
        print_defs_usage(program);
        return ExitCode::from(1);
    };
    match sub {
        "build" => run_defs_build(&args[1..]),
        "show" => run_defs_show(&args[1..]),
        "find" => run_defs_find(&args[1..]),
        "check" => run_defs_check(&args[1..]),
        "diff" => run_defs_diff(&args[1..]),
        other => {
            eprintln!("Unknown defs command: {other}");
            print_defs_usage(program);
            ExitCode::from(1)
        }
    }
}

fn run_defs_build(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut title = "gta5".to_string();
    let mut target = "gen8".to_string();
    let mut i = 0;
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned();
        match args[i].as_str() {
            "--input" => {
                input = value(i).map(PathBuf::from);
                i += 1;
            }
            "--out" => {
                out = value(i).map(PathBuf::from);
                i += 1;
            }
            "--title" => {
                title = value(i).unwrap_or(title);
                i += 1;
            }
            "--target" => {
                target = value(i).unwrap_or(target);
                i += 1;
            }
            other => {
                eprintln!("Error: unknown flag {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }

    let (Some(input), Some(out)) = (input, out) else {
        eprintln!("Error: --input and --out are both required");
        return ExitCode::from(1);
    };

    let is_install = input.join("common.rpf").exists();
    let built = if is_install {
        or_shader_tool::defs::build_from_install(&input, &title, &target)
    } else {
        or_shader_tool::defs::build_from_directory(&input, &title, &target)
    };
    let built = match built {
        Ok(b) => b,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    let size = match or_shader_tool::defs::write(&out, &built.definitions) {
        Ok(n) => n,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    println!();
    println!("Shader definitions written to {}", out.display());
    println!(
        "  {} shaders, {} parameters, {} KiB",
        built.definitions.shaders.len(),
        built.definitions.parameter_count(),
        size / 1024
    );
    if !built.unresolved.is_empty() {
        println!(
            "  {} presets name an effect this input does not hold:",
            built.unresolved.len()
        );
        for u in built.unresolved.iter().take(10) {
            println!("    - {} -> {}", u.preset, u.effect);
        }
        if built.unresolved.len() > 10 {
            println!("    … and {} more", built.unresolved.len() - 10);
        }
    }
    let refused = !built.unreadable.is_empty() || !built.incomplete.is_empty();
    if !built.unreadable.is_empty() {
        println!("  {} effects would not parse:", built.unreadable.len());
        for (name, why) in built.unreadable.iter().take(10) {
            println!("    - {name}: {why}");
        }
    }
    if !built.incomplete.is_empty() {
        println!(
            "  {} effects were not read to the end, so the reader is wrong somewhere:",
            built.incomplete.len()
        );
        for (name, left) in built.incomplete.iter().take(10) {
            println!("    - {name}: {left} bytes past the last table");
        }
    }
    if refused {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run_defs_show(args: &[String]) -> ExitCode {
    let Some(path) = args.first().map(PathBuf::from) else {
        eprintln!("Error: missing database path");
        return ExitCode::from(1);
    };
    let defs = match or_shader_tool::defs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    println!(
        "{} / {} (schema {}, build {})",
        defs.title,
        defs.target,
        defs.schema_version,
        if defs.source_build.is_empty() {
            "<unset>"
        } else {
            &defs.source_build
        }
    );
    println!(
        "  {} shaders, {} parameters",
        defs.shaders.len(),
        defs.parameter_count()
    );

    let Some(name) = args.get(1) else {
        println!();
        for s in defs.shaders.iter().take(40) {
            println!(
                "  {:<40} {:<24} bucket {:<4} {} params",
                s.name,
                s.effect,
                s.draw_bucket
                    .map(|b| b.to_string())
                    .unwrap_or_else(|| "-".into()),
                s.parameters.len()
            );
        }
        if defs.shaders.len() > 40 {
            println!("  … and {} more", defs.shaders.len() - 40);
        }
        return ExitCode::SUCCESS;
    };

    let Some(shader) = defs.shader_named(name) else {
        eprintln!("No shader named `{name}`");
        return ExitCode::from(1);
    };
    println!();
    println!("  {} (effect {})", shader.name, shader.effect);
    println!("    hash        0x{:08X}", shader.name_hash);
    println!("    vertex type 0x{:08X}", shader.vertex_type);
    if let Some(bucket) = shader.draw_bucket {
        println!("    bucket      {bucket}");
    }
    for t in &shader.traits {
        println!("    trait       {} = {}", t.name, t.value);
    }
    println!();
    for p in &shader.parameters {
        let default = p
            .preset_default
            .as_ref()
            .or(p.default.as_ref())
            .map(|v| format!("{v:?}"))
            .unwrap_or_default();
        println!(
            "    0x{:08X}  {:<28} {:<10} reg {:<3} {}",
            p.name_hash,
            p.name,
            p.kind.as_str(),
            p.register,
            default
        );
    }
    ExitCode::SUCCESS
}

fn run_defs_find(args: &[String]) -> ExitCode {
    let (Some(path), Some(needle)) = (args.first().map(PathBuf::from), args.get(1)) else {
        eprintln!("Error: defs find needs a database and a parameter name or hash");
        return ExitCode::from(1);
    };
    let defs = match or_shader_tool::defs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    let wanted = match needle
        .strip_prefix("0x")
        .or_else(|| needle.strip_prefix("0X"))
    {
        Some(hex) => match u32::from_str_radix(hex, 16) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Error: {needle} is not a hash: {e}");
                return ExitCode::from(1);
            }
        },
        None => or_io::gta5::shaders::shader_defs::hash_name(needle),
    };

    let mut shaders = Vec::new();
    let mut found: Option<&or_io::gta5::shaders::shader_defs::ShaderParamDef> = None;
    for shader in &defs.shaders {
        if let Some(p) = shader.parameter(wanted) {
            shaders.push(shader.name.as_str());
            found.get_or_insert(p);
        }
    }

    match found {
        Some(p) => {
            println!(
                "0x{:08X}  {}  {}  declared by {} of {} shaders",
                wanted,
                p.name,
                p.kind.as_str(),
                shaders.len(),
                defs.shaders.len()
            );
            for name in shaders.iter().take(12) {
                println!("  - {name}");
            }
            if shaders.len() > 12 {
                println!("  … and {} more", shaders.len() - 12);
            }
            ExitCode::SUCCESS
        }
        None => {
            println!("0x{wanted:08X}  no shader in this database declares it");
            ExitCode::from(1)
        }
    }
}

fn run_defs_check(args: &[String]) -> ExitCode {
    let Some(db) = args.first().map(PathBuf::from) else {
        eprintln!("Error: defs check needs a database and at least one file or directory");
        return ExitCode::from(1);
    };
    let defs = match or_shader_tool::defs::read(&db) {
        Ok(d) => d,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    let roots: Vec<PathBuf> = args[1..].iter().map(PathBuf::from).collect();
    let files = or_shader_tool::defs::material_files(&roots);
    if files.is_empty() {
        eprintln!("Error: nothing under those paths names a shader");
        return ExitCode::from(1);
    }

    let cov = or_io::gta5::shaders::shader_defs::Coverage::of(
        &defs,
        &files,
        or_io::gta5::shaders::shader_defs::materials::materials_in,
    );
    println!();
    println!(
        "Checked {} of {} files against {}",
        cov.files_read,
        files.len(),
        db.display()
    );
    println!(
        "  {}",
        cov.files_by_format
            .iter()
            .map(|(ext, n)| format!("{n} .{ext}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  {} materials, {} parameters, {} named ({:.1}%)",
        cov.materials,
        cov.entries,
        cov.named,
        cov.named_fraction() * 100.0
    );
    for (path, why) in cov.files_refused.iter().take(10) {
        println!("  refused {}: {why}", path.display());
    }
    if cov.files_refused.len() > 10 {
        println!("  … and {} more refused", cov.files_refused.len() - 10);
    }
    if !cov.unknown_shaders.is_empty() {
        println!(
            "  {} materials name a shader this database does not describe, over {} hashes:",
            cov.unknown_shaders.values().sum::<usize>(),
            cov.unknown_shaders.len()
        );
        for (hash, n) in cov.unknown_shaders.iter().take(20) {
            println!("    - 0x{hash:08X} x{n}");
        }
    }

    let elsewhere = cov.declared_by_another_shader();
    if !elsewhere.is_empty() {
        println!(
            "  {} hashes are declared by some other shipped effect, so the material is \
             pointed at the wrong shader rather than the name being unknown:",
            elsewhere.len()
        );
        for (hash, row) in elsewhere.iter().take(20) {
            println!(
                "    - 0x{hash:08X} x{} — {} declares it; bound by {}",
                row.occurrences,
                row.declared_by.first().map(String::as_str).unwrap_or("?"),
                join_capped(&row.bound_by, 4)
            );
        }
    }

    let nowhere = cov.undeclared_anywhere();
    if !nowhere.is_empty() {
        println!(
            "  {} hashes no shipped effect declares anywhere — {} occurrences, {} of them \
             texture slots. This is drift between the shipped effects and the shipped files:",
            nowhere.len(),
            nowhere.iter().map(|(_, r)| r.occurrences).sum::<usize>(),
            nowhere.iter().map(|(_, r)| r.bindings).sum::<usize>()
        );
        for (hash, row) in &nowhere {
            println!(
                "    - 0x{hash:08X} x{} across {} shaders: {}",
                row.occurrences,
                row.bound_by.len(),
                join_capped(&row.bound_by, 6)
            );
        }
    }

    if cov.unknown_shaders.is_empty() && cov.residue.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn join_capped(names: &std::collections::BTreeSet<String>, limit: usize) -> String {
    let mut out: Vec<&str> = names.iter().take(limit).map(String::as_str).collect();
    let extra = names.len().saturating_sub(limit);
    let tail;
    if extra > 0 {
        tail = format!("… +{extra}");
        out.push(&tail);
    }
    out.join(", ")
}

fn run_defs_diff(args: &[String]) -> ExitCode {
    let mut sides: Vec<PathBuf> = Vec::new();
    let mut title = "gta5".to_string();
    let mut target = "gen8".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                title = args.get(i + 1).cloned().unwrap_or(title);
                i += 1;
            }
            "--target" => {
                target = args.get(i + 1).cloned().unwrap_or(target);
                i += 1;
            }
            other if other.starts_with("--") => {
                eprintln!("Error: unknown flag {other}");
                return ExitCode::from(1);
            }
            other => sides.push(PathBuf::from(other)),
        }
        i += 1;
    }

    let [before, after] = sides.as_slice() else {
        eprintln!("Error: defs diff needs exactly two sides");
        return ExitCode::from(1);
    };

    let before_subject = or_shader_tool::defs::Subject::of(before);
    let after_subject = or_shader_tool::defs::Subject::of(after);
    let (before, after) = match (
        before_subject.resolve(&title, &target),
        after_subject.resolve(&title, &target),
    ) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };

    let diff = or_io::gta5::shaders::shader_defs::ShaderDefsDiff::of(&before, &after);
    println!();
    println!("{} -> {}", before_subject.label(), after_subject.label());
    if diff.is_empty() {
        println!(
            "  no change: {} shaders, {} parameters on both sides",
            before.shaders.len(),
            before.parameter_count()
        );
        return ExitCode::SUCCESS;
    }

    let counts = diff.counts();
    println!(
        "  {} shaders added, {} removed, {} changed; {} parameters added, {} removed, {} changed",
        counts.shaders_added,
        counts.shaders_removed,
        counts.shaders_changed,
        counts.parameters_added,
        counts.parameters_removed,
        counts.parameters_changed
    );
    println!();
    for change in &diff.header {
        println!(
            "  ! {}: {} -> {}",
            change.field, change.before, change.after
        );
    }
    for delta in &diff.shaders {
        match delta {
            or_io::gta5::shaders::shader_defs::ShaderDelta::Added { name, parameters } => {
                println!("  + {name} ({parameters} parameters)");
            }
            or_io::gta5::shaders::shader_defs::ShaderDelta::Removed { name, parameters } => {
                println!("  - {name} ({parameters} parameters)");
            }
            or_io::gta5::shaders::shader_defs::ShaderDelta::Changed {
                name,
                changes,
                parameters,
            } => {
                println!("  ~ {name}");
                for c in changes {
                    println!("      {}: {} -> {}", c.field, c.before, c.after);
                }
                for p in parameters {
                    match p {
                        or_io::gta5::shaders::shader_defs::ParamDelta::Added { name } => {
                            println!("      + {name}");
                        }
                        or_io::gta5::shaders::shader_defs::ParamDelta::Removed { name } => {
                            println!("      - {name}");
                        }
                        or_io::gta5::shaders::shader_defs::ParamDelta::Changed {
                            name,
                            changes,
                        } => {
                            for c in changes {
                                println!("      ~ {name}.{}: {} -> {}", c.field, c.before, c.after);
                            }
                        }
                    }
                }
            }
        }
    }
    ExitCode::from(1)
}

fn print_defs_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} defs build --input <INSTALL-OR-DIR> --out <FILE> [--title gta5] [--target gen8]");
    eprintln!("  {program} defs show  <FILE> [SHADER]");
    eprintln!("  {program} defs find  <FILE> <NAME-OR-0xHASH>");
    eprintln!("  {program} defs check <FILE> <YDR|YDD|YFT|YPT|DIR>...");
    eprintln!(
        "  {program} defs diff  <BEFORE> <AFTER>   each a database, an install or a directory"
    );
}

fn run_inventory(args: &[String]) -> ExitCode {
    use or_shader_tool::inventory::{scan_path, Target};

    let mut input: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut target = Target::Auto;
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1);
        match args[i].as_str() {
            "--input" => {
                input = value.map(PathBuf::from);
                i += 1;
            }
            "--json" => {
                json = value.map(PathBuf::from);
                i += 1;
            }
            "--target" => {
                let Some(parsed) = value.and_then(|v| Target::parse(v)) else {
                    eprintln!("Error: --target is auto, gen8 or gen9");
                    return ExitCode::from(1);
                };
                target = parsed;
                i += 1;
            }
            other => {
                eprintln!("Error: unknown flag {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("Error: --input is required");
        return ExitCode::from(1);
    };

    let inventory = match scan_path(&input, target) {
        Ok(inventory) => inventory,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };
    print!("{}", inventory.report());
    if let Some(path) = json {
        let written = serde_json::to_string_pretty(&inventory.to_json())
            .map_err(std::io::Error::from)
            .and_then(|text| {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, text)
            });
        match written {
            Ok(()) => println!("\nwritten to {}", path.display()),
            Err(e) => {
                error!("{}: {e}", path.display());
                return ExitCode::from(1);
            }
        }
    }
    if inventory.shaders() == 0 && inventory.spirv.modules == 0 {
        eprintln!("\nNo shader container and no SPIR-V module was recognised in the input.");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// The device `--device` asks for, or the exit code that says why there is none.
#[cfg(feature = "device-check")]
fn open_device(
    wanted: bool,
) -> Result<Option<Box<dyn or_shader_tool::spirv_check::ModuleCompiler>>, ExitCode> {
    if !wanted {
        return Ok(None);
    }
    match or_shader_tool::device_check::DeviceCompiler::open() {
        Ok(device) => {
            println!("device: {}\n", device.adapter);
            Ok(Some(Box::new(device)))
        }
        Err(e) => {
            eprintln!("Error: {e}");
            Err(ExitCode::from(1))
        }
    }
}

#[cfg(not(feature = "device-check"))]
fn open_device(
    wanted: bool,
) -> Result<Option<Box<dyn or_shader_tool::spirv_check::ModuleCompiler>>, ExitCode> {
    if wanted {
        eprintln!("Error: this build has no device check. Build with `--features device-check`.");
        return Err(ExitCode::from(1));
    }
    Ok(None)
}

fn run_spv_check(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut device = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--device" => device = true,
            other if !other.starts_with("--") && input.is_none() => {
                input = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("Error: a SPIR-V file or folder is required");
        return ExitCode::from(1);
    };
    // The MSL writer panics on some modules and the check counts them, so its panic message is noise.
    std::panic::set_hook(Box::new(|_| {}));
    let compiler = match open_device(device) {
        Ok(compiler) => compiler,
        Err(code) => return code,
    };
    let tally = match or_shader_tool::spirv_check::check_path_with(&input, compiler.as_deref()) {
        Ok(tally) => tally,
        Err(e) => {
            error!("{}: {e}", input.display());
            return ExitCode::from(1);
        }
    };
    print!("{}", tally.report());
    if let Some(path) = json {
        let text = serde_json::to_string_pretty(&tally.to_json()).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            error!("{}: {e}", path.display());
            return ExitCode::from(1);
        }
    }
    if tally.modules() == 0 {
        eprintln!("\nNo file was read.");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[cfg(not(feature = "native-front"))]
fn run_translate_check(_args: &[String]) -> ExitCode {
    eprintln!("Error: this build has no DXBC translator. Build with `--features native-front`.");
    ExitCode::from(1)
}

#[cfg(feature = "native-front")]
fn run_translate_check(args: &[String]) -> ExitCode {
    use or_shader_tool::inventory::Target;

    let mut input: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut target = Target::Auto;
    let mut device = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--device" => device = true,
            "--target" => {
                let Some(parsed) = args.get(i + 1).and_then(|v| Target::parse(v)) else {
                    eprintln!("Error: --target is auto, gen8 or gen9");
                    return ExitCode::from(1);
                };
                target = parsed;
                i += 1;
            }
            other if !other.starts_with("--") && input.is_none() => {
                input = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("Error: an install, a folder or a file is required");
        return ExitCode::from(1);
    };
    // The MSL writer panics on some modules and the check counts them, so its panic message is noise.
    std::panic::set_hook(Box::new(|_| {}));
    let compiler = match open_device(device) {
        Ok(compiler) => compiler,
        Err(code) => return code,
    };
    let tally = match or_shader_tool::translate_check::translate_check_with(
        &input,
        target,
        compiler.as_deref(),
    ) {
        Ok(tally) => tally,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };
    print!("{}", tally.report());
    if let Some(path) = json {
        let text = serde_json::to_string_pretty(&tally.to_json()).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            error!("{}: {e}", path.display());
            return ExitCode::from(1);
        }
    }
    if tally.containers == 0 {
        eprintln!("\nNo shader container was found.");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[cfg(not(feature = "native-front"))]
fn run_pass_check(_args: &[String]) -> ExitCode {
    eprintln!("Error: this build has no DXBC translator. Build with `--features native-front`.");
    ExitCode::from(1)
}

#[cfg(feature = "native-front")]
fn run_pass_check(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut folder = "shaders/win32_40_final".to_string();
    let mut device = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--folder" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("Error: --folder needs a value");
                    return ExitCode::from(1);
                };
                folder = value.clone();
                i += 1;
            }
            "--device" => device = true,
            other if !other.starts_with("--") && input.is_none() => {
                input = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("Error: an install or a folder of `.fxc` files is required");
        return ExitCode::from(1);
    };
    std::panic::set_hook(Box::new(|_| {}));
    let compiler = match open_device(device) {
        Ok(compiler) => compiler,
        Err(code) => return code,
    };
    let tally = match or_shader_tool::pass_check::pass_check(&input, &folder, compiler.as_deref()) {
        Ok(tally) => tally,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(1);
        }
    };
    print!("{}", tally.report());
    if let Some(path) = json {
        let text = serde_json::to_string_pretty(&tally.to_json()).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            error!("{}: {e}", path.display());
            return ExitCode::from(1);
        }
    }
    if tally.effects == 0 {
        eprintln!("\nNo effect was read.");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// How far each of naga's writers gets with the modules of a baked pack.
fn run_backend_check(args: &[String]) -> ExitCode {
    let mut pack: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut spirv_val: Option<PathBuf> = None;
    let mut target_env: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--spirv-val" => {
                spirv_val = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--target-env" => {
                target_env = args.get(i + 1).cloned();
                i += 1;
            }
            other if !other.starts_with("--") && pack.is_none() => {
                pack = Some(PathBuf::from(other))
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let Some(pack) = pack else {
        eprintln!("Error: a pack folder is required");
        return ExitCode::from(1);
    };
    let validator = spirv_val.map(|tool| {
        let mut validator = or_shader_tool::backend_check::SpirvVal::new(tool);
        if let Some(env) = target_env {
            validator.target_env = env;
        }
        validator
    });
    // A writer that cannot take a module may panic on it, which is counted and not shown
    std::panic::set_hook(Box::new(|_| {}));
    match or_shader_tool::backend_check::check_pack_with(&pack, validator.as_ref()) {
        Ok(tally) => {
            print!("{}", tally.report());
            if let Some(path) = json {
                let text = serde_json::to_string_pretty(&tally.to_json()).unwrap_or_default();
                if let Err(e) = std::fs::write(&path, text) {
                    error!("{}: {e}", path.display());
                    return ExitCode::from(1);
                }
            }
            if tally.total() == 0 {
                eprintln!("\nNo translated program in the pack.");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

/// One digest of a baked pack's modules and manifest, to compare a pack baked elsewhere.
fn run_pack_digest(args: &[String]) -> ExitCode {
    let Some(pack) = args.first() else {
        eprintln!("Error: a pack folder is required");
        return ExitCode::from(1);
    };
    match or_shader_tool::pack_digest::pack_digest(Path::new(pack)) {
        Ok((modules, digest)) => {
            println!("{modules} modules  {digest}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "native-front"))]
fn run_pack_identity() -> ExitCode {
    eprintln!("Error: this build has no DXBC translator. Build with `--features native-front`.");
    ExitCode::from(1)
}

/// Who would write a pack now, as the one event, for a program that decides whether the pack it has is current.
#[cfg(feature = "native-front")]
fn run_pack_identity() -> ExitCode {
    let production = or_shader_tool::spirv_pack::production();
    println!(
        "{}",
        Event::Identity {
            translator: production.translator,
            legalizer: production.legalizer,
            pack_version: or_io::gta5::shaders::spirv_pack::PACK_VERSION,
        }
        .line()
    );
    ExitCode::SUCCESS
}

#[cfg(not(feature = "native-front"))]
fn run_bake_pack(_args: &[String]) -> ExitCode {
    eprintln!("Error: this build has no DXBC translator. Build with `--features native-front`.");
    ExitCode::from(1)
}

#[cfg(feature = "native-front")]
fn run_bake_pack(args: &[String]) -> ExitCode {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut folder = "shaders/win32_40_final".to_string();
    let mut build_hash = String::new();
    let mut progress = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--progress" => progress = true,
            "--out" | "--folder" | "--build-hash" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("Error: {} needs a value", args[i]);
                    return ExitCode::from(1);
                };
                match args[i].as_str() {
                    "--out" => out = Some(PathBuf::from(value)),
                    "--folder" => folder = value.clone(),
                    _ => build_hash = value.clone(),
                }
                i += 1;
            }
            other if !other.starts_with("--") && input.is_none() => {
                input = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let (Some(input), Some(out)) = (input, out) else {
        eprintln!(
            "Error: an install or a folder of `.fxc` files and --out <PACK-DIR> are required"
        );
        return ExitCode::from(1);
    };
    // The translator's checks of a module naga cannot write end in a caught panic, whose message is not for a program reading this
    std::panic::set_hook(Box::new(|_| {}));
    // With `--progress` stdout is one JSON event a line for a program that runs this, in place of the report
    let say = |event: Event| {
        use std::io::Write;
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", event.line());
        let _ = stdout.flush();
    };
    if progress {
        say(Event::Reading);
    }
    let told = throttled(say);
    let none = |_: usize, _: usize| {};
    let progress_fn: &(dyn Fn(usize, usize) + Sync) = if progress { &told } else { &none };
    match or_shader_tool::spirv_pack::bake_with(&input, &folder, &out, &build_hash, progress_fn) {
        Ok((_, report)) => {
            if progress {
                say(Event::Finished {
                    effects: report.effects,
                    programs: report.programs,
                    translated: report.translated,
                    refused: report.refused,
                    skipped: report.skipped,
                    reused: report.reused,
                    baked: report.baked,
                });
            } else {
                print!("{}", report.report());
            }
            if report.effects == 0 {
                eprintln!("\nNo effect was read.");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "render-drawable"))]
fn run_render_census(_args: &[String]) -> ExitCode {
    eprintln!("Error: this build cannot draw. Build with `--features render-drawable`.");
    ExitCode::from(1)
}

#[cfg(feature = "render-drawable")]
fn run_render_census(args: &[String]) -> ExitCode {
    let mut install: Option<PathBuf> = None;
    let mut pack: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;
    let mut sheets: Option<PathBuf> = None;
    let mut extensions = "ydr,yft".to_string();
    let mut limit = 300usize;
    let mut size = 128u32;
    let mut shaded = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--shaded" => shaded = true,
            "--pack" | "--json" | "--sheet" | "--ext" | "--limit" | "--size" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("Error: {} needs a value", args[i]);
                    return ExitCode::from(1);
                };
                let number = value.parse::<usize>();
                match args[i].as_str() {
                    "--pack" => pack = Some(PathBuf::from(value)),
                    "--json" => json = Some(PathBuf::from(value)),
                    "--sheet" => sheets = Some(PathBuf::from(value)),
                    "--ext" => extensions = value.clone(),
                    flag => match number {
                        Ok(n) if flag == "--limit" => limit = n,
                        Ok(n) => size = u32::try_from(n).unwrap_or(size),
                        Err(_) => {
                            eprintln!("Error: {flag} needs a number");
                            return ExitCode::from(1);
                        }
                    },
                }
                i += 1;
            }
            other if !other.starts_with("--") && install.is_none() => {
                install = Some(PathBuf::from(other));
            }
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let (Some(install), Some(pack)) = (install, pack) else {
        eprintln!("Error: an install and --pack <PACK-DIR> are required");
        return ExitCode::from(1);
    };
    std::panic::set_hook(Box::new(|_| {}));
    let result = (|| {
        let pack = or_renderer::game_shaders::pack::GamePack::open(&pack)
            .map_err(|e| format!("{}: {e}", pack.display()))?;
        let (device, queue, adapter) = or_shader_tool::device_check::open_device()?;
        println!("device: {adapter}");
        let install = or_shader_tool::render_drawable::Install::open(&install)?;
        let extensions: Vec<&str> = extensions.split(',').collect();
        let census = or_shader_tool::render_drawable::census::census(
            &install,
            &device,
            &queue,
            &pack,
            &extensions,
            limit,
            size,
            shaded,
            sheets.is_some(),
            &|done, of| {
                if done % 25 == 0 {
                    eprintln!("{done} of {of}");
                }
            },
        );
        print!("{}", census.report());
        if let Some(dir) = sheets {
            let sheets =
                or_shader_tool::render_drawable::write_sheets(&census.pictures, shaded, &dir)?;
            println!("{sheets} contact sheets in {}", dir.display());
        }
        if let Some(path) = json {
            let text = serde_json::to_string_pretty(&census.to_json()).unwrap_or_default();
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok::<(), String>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(feature = "render-drawable"))]
fn run_render_drawable(_args: &[String]) -> ExitCode {
    eprintln!("Error: this build cannot draw. Build with `--features render-drawable`.");
    ExitCode::from(1)
}

#[cfg(feature = "render-drawable")]
fn run_render_drawable(args: &[String]) -> ExitCode {
    let mut positional: Vec<&String> = Vec::new();
    let mut pack: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut save: Option<PathBuf> = None;
    let mut size = 512u32;
    let mut shaded = false;
    let mut options = or_shader_tool::render_drawable::ShadedOptions::default();
    // A `NAME=a,b,c,d` of a number each, the ones left out nought
    let pair = |text: Option<&String>| -> Option<(String, Vec<f64>)> {
        let (name, values) = text?.split_once('=')?;
        let numbers = values
            .split(',')
            .map(|v| v.parse::<f64>().ok())
            .collect::<Option<Vec<_>>>()?;
        Some((name.to_string(), numbers))
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--shaded" => shaded = true,
            "--ground" => options.ground = true,
            "--frames" => {
                options.frames = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--sun" => {
                let parsed = args.get(i + 1).map(|v| {
                    v.split(',')
                        .map(|n| n.parse::<f32>().ok())
                        .collect::<Option<Vec<_>>>()
                });
                let Some(Some(v)) = parsed.filter(|p| p.as_ref().is_some_and(|v| v.len() == 3))
                else {
                    eprintln!("Error: --sun needs X,Y,Z");
                    return ExitCode::from(1);
                };
                options.sun = Some([v[0], v[1], v[2]]);
                i += 1;
            }
            "--effect" => {
                options.only_effect = args.get(i + 1).cloned();
                i += 1;
            }
            "--texture" | "--vector" => {
                let Some((name, numbers)) = pair(args.get(i + 1)) else {
                    eprintln!("Error: {} needs NAME=a,b,c,d", args[i]);
                    return ExitCode::from(1);
                };
                if args[i] == "--texture" {
                    match or_shader_tool::render_drawable::texel_of(&numbers) {
                        Ok(texel) => options.textures.push((name, texel)),
                        Err(why) => {
                            eprintln!("Error: --texture {name}: {why}");
                            return ExitCode::from(1);
                        }
                    }
                } else {
                    let mut row = [0.0f64; 4];
                    for (slot, n) in row.iter_mut().zip(numbers) {
                        *slot = n;
                    }
                    options.vectors.push((name, row.map(|n| n as f32)));
                }
                i += 1;
            }
            "--pack" | "--out" | "--size" | "--save" => {
                let Some(value) = args.get(i + 1) else {
                    eprintln!("Error: {} needs a value", args[i]);
                    return ExitCode::from(1);
                };
                match args[i].as_str() {
                    "--pack" => pack = Some(PathBuf::from(value)),
                    "--out" => out = Some(PathBuf::from(value)),
                    "--save" => save = Some(PathBuf::from(value)),
                    _ => match value.parse() {
                        Ok(n) => size = n,
                        Err(_) => {
                            eprintln!("Error: --size needs a number");
                            return ExitCode::from(1);
                        }
                    },
                }
                i += 1;
            }
            other if !other.starts_with("--") => positional.push(&args[i]),
            other => {
                eprintln!("Error: unknown argument {other}");
                return ExitCode::from(1);
            }
        }
        i += 1;
    }
    let (Some(source), Some(pack), Some(out)) = (positional.first(), pack, out) else {
        eprintln!("Error: a drawable file or an install and a name, --pack <PACK-DIR> and --out <PNG> are required");
        return ExitCode::from(1);
    };
    std::panic::set_hook(Box::new(|_| {}));
    let result = (|| {
        let found = or_shader_tool::render_drawable::read_drawable(
            Path::new(source),
            positional.get(1).map(|n| n.as_str()),
        )?;
        if let Some(save) = &save {
            std::fs::write(save, &found.bytes).map_err(|e| format!("{}: {e}", save.display()))?;
        }
        let pack = or_renderer::game_shaders::pack::GamePack::open(&pack)
            .map_err(|e| format!("{}: {e}", pack.display()))?;
        let (device, queue, adapter) = or_shader_tool::device_check::open_device()?;
        println!(
            "device: {adapter}\n{}, {} textures beside it",
            found.name,
            found.beside.len()
        );
        let drawn = if shaded {
            let mut renderer = or_renderer::viewport::ViewportRenderer::new(
                &device,
                &queue,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                size,
                size,
            );
            or_shader_tool::render_drawable::render_drawable_shaded(
                &device,
                &queue,
                &pack,
                &mut renderer,
                &found,
                size,
                false,
                &options,
            )?
        } else {
            or_shader_tool::render_drawable::render_drawable(&device, &queue, &pack, &found, size)?
        };
        println!(
            "{} surfaces, {} vertices: {} drawn, {} left out{}",
            drawn.effects.len(),
            drawn.vertices,
            drawn.render.drawn,
            drawn.render.skipped.len(),
            if shaded {
                String::new()
            } else {
                format!(", {} pixels touched", drawn.render.rendered.touched())
            },
        );
        let effects: std::collections::BTreeSet<&str> =
            drawn.effects.iter().map(String::as_str).collect();
        println!(
            "  effects: {}",
            effects.into_iter().collect::<Vec<_>>().join(", ")
        );
        let mut left_out: BTreeMap<String, usize> = BTreeMap::new();
        for (effect, why) in &drawn.render.skipped {
            *left_out.entry(format!("{effect}: {why}")).or_default() += 1;
        }
        for (why, n) in left_out {
            println!("  {n:>4} left out, {why}");
        }
        if shaded {
            let rendered = drawn.render.rendered;
            or_shader_tool::render_drawable::write_encoded_png(rendered.size, rendered.bytes, &out)
        } else {
            or_shader_tool::render_drawable::write_png(drawn.render.rendered, &out)
        }
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage(program: &str) {
    eprintln!("or-shader-tool — translate GTA V .fxc shaders into a pack of SPIR-V modules");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  {program} inventory --input <INSTALL|DIR|FILE> [--json <FILE>] [--target auto|gen8|gen9]");
    eprintln!(
        "                                count the bytecode kinds, shader models, stages and"
    );
    eprintln!(
        "                                features of an install's shaders, or of loose files"
    );
    eprintln!("  {program} spv-check <FILE|DIR> [--json <FILE>] [--device]");
    eprintln!(
        "                                legalize translated SPIR-V and count how far naga gets:"
    );
    eprintln!("                                parsed, validated, written as MSL");
    eprintln!(
        "  {program} translate-check <INSTALL|DIR|FILE> [--target auto|gen8|gen9] [--json <FILE>] [--device]"
    );
    eprintln!(
        "                                translate every DXBC shader with the vendored translator,"
    );
    eprintln!("                                legalize it and count how far naga gets (needs the");
    eprintln!("                                `native-front` feature)");
    eprintln!("  {program} pass-check <INSTALL|DIR> [--folder shaders/win32_40_final] [--json <FILE>] [--device]");
    eprintln!("                                translate every effect and count the passes whose every stage builds");
    eprintln!("  {program} bake-pack <INSTALL|DIR> --out <PACK-DIR> [--folder <NAME>] [--build-hash <HASH>] [--progress]");
    eprintln!("  {program} pack-identity");
    eprintln!("  {program} backend-check <PACK-DIR> [--json <FILE>] [--spirv-val <PATH> [--target-env vulkan1.1]]");
    eprintln!("  {program} pack-digest <PACK-DIR>   one digest of a pack's modules and manifest, to compare a pack baked on another OS");
    eprintln!("  {program} render-drawable <FILE | INSTALL NAME> --pack <PACK-DIR> --out <PNG> [--size 512] [--shaded [--effect <NAME>] [--texture <NAME=r,g,b,a>] [--vector <NAME=a,b,c,d>]] [--save <FILE>]");
    eprintln!("  {program} render-census <INSTALL> --pack <PACK-DIR> [--ext ydr,yft] [--limit 300] [--size 128] [--shaded] [--json <FILE>] [--sheet <DIR>]");
    eprintln!("                                write the SPIR-V pack: one legalized module per program a pass runs,");
    eprintln!("                                and a manifest saying which pass of which technique runs which");
    eprintln!("                                (needs the `native-front` feature)");
    rdna_command::print_usage(program);
    eprintln!("  {program} defs build --input <INSTALL-OR-DIR> --out <FILE> [--title gta5] [--target gen8]");
    eprintln!("  {program} defs show  <FILE> [SHADER]");
    eprintln!("  {program} defs find  <FILE> <NAME-OR-0xHASH>");
    eprintln!("  {program} defs check <FILE> <YDR|YDD|YFT|YPT|DIR>...");
    eprintln!("  {program} defs diff  <BEFORE> <AFTER>");
}
