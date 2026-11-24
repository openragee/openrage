use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::meta::binary::layout::ContainerLayout;
use or_io::meta::binary::{parse_meta_binary_full, write_meta_binary};
use or_io::meta::{BinaryFormat, Diagnostics, ErrorMode, MetaInstance, Value};

pub fn run(args: &[String]) -> ExitCode {
    let Some(sub) = args.first() else {
        print_usage();
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    let result = match sub.as_str() {
        "info" => cmd_info(rest),
        "to-xml" => cmd_to_xml(rest),
        "from-xml" => cmd_from_xml(rest),
        "rewrite" => cmd_rewrite(rest),
        "roundtrip" => cmd_roundtrip(rest),
        "types" => cmd_types(rest),
        "snapshot" => cmd_snapshot(rest),
        "-h" | "--help" | "help" => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown subcommand '{other}'")),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn format_label(bytes: &[u8]) -> &'static str {
    match or_io::meta::detect_binary_format(bytes) {
        Some(BinaryFormat::Pso) => "PSO",
        Some(BinaryFormat::Rbm) => "RBM",
        Some(BinaryFormat::Rbf) => "RBF",
        None => "XML",
    }
}

fn cmd_info(args: &[String]) -> Result<ExitCode, String> {
    let opts = Options::parse(args)?;
    let path = opts.require_input()?;
    let bytes = read(path)?;
    if !opts.json {
        print_info(&path.display().to_string(), &bytes, opts.error_mode())?;
        return Ok(ExitCode::SUCCESS);
    }
    let (instance, _, diags) = read_instance(&bytes, opts.error_mode())?;
    let stats = Stats::of(&instance);
    println!(
        r#"{{"path":{:?},"format":"{}","bytes":{},"rootType":{:?},"fields":{},"values":{},"structs":{},"arrays":{},"gaps":{}}}"#,
        path.display().to_string(),
        format_label(&bytes),
        bytes.len(),
        instance.type_name,
        instance.fields.len(),
        stats.values,
        stats.structs,
        stats.arrays,
        diags.gaps().len(),
    );
    Ok(ExitCode::SUCCESS)
}

pub(crate) type Document = (
    MetaInstance,
    Option<or_io::meta::binary::MetaLayout>,
    Diagnostics,
);

/// A metadata document through the reader its leading bytes pick.
pub(crate) fn read_instance(bytes: &[u8], mode: ErrorMode) -> Result<Document, String> {
    let mut diags = Diagnostics::new(mode);
    let (instance, layout) = match or_io::meta::detect_binary_format(bytes) {
        Some(_) => {
            let (i, l) =
                parse_meta_binary_full(schema(), bytes, &mut diags).map_err(|e| e.to_string())?;
            (i, Some(l))
        }
        None => (
            or_io::meta::instance::parse_meta(schema(), bytes).map_err(|e| e.to_string())?,
            None,
        ),
    };
    Ok((instance, layout, diags))
}

/// What `meta info` prints of a document, under the name `name`.
pub(crate) fn print_info(name: &str, bytes: &[u8], mode: ErrorMode) -> Result<(), String> {
    let document = read_instance(bytes, mode)?;
    print_document(name, format_label(bytes), bytes.len(), &document);
    if let (instance, Some(layout), _) = &document {
        print_gta6_save(layout, instance, bytes);
    }
    Ok(())
}

/// The sentence a GTA VI PSO save carries: its checksum is the one it was read with.
pub(crate) const CHECKSUM_KEPT: &str =
    "the CHKS checksum is not recomputed, so the game may refuse an edited file";

/// Whether `layout` is a GTA VI PSO, which carries a `CHKS` checksum no save recomputes.
pub(crate) fn keeps_a_checksum(layout: &or_io::meta::binary::MetaLayout) -> bool {
    matches!(&layout.container, ContainerLayout::Pso(p) if p.is_table_form() || p.is_gta6())
}

/// For a GTA VI form, whether a save of the unedited tree gives back `stored`, the bytes it was
/// read from; nothing for any other.
pub(crate) fn print_gta6_save(
    layout: &or_io::meta::binary::MetaLayout,
    instance: &MetaInstance,
    stored: &[u8],
) {
    let written = match &layout.container {
        ContainerLayout::Rbm(r) if r.seal.is_none() => {
            or_io::meta::binary::rbm::gta6::write_sections(r, instance)
                .map(|(v, p)| [v, p].concat())
        }
        ContainerLayout::Rbf(r) if r.wide_values => {
            write_meta_binary(layout.format, Some(layout), instance)
        }
        ContainerLayout::Pso(_) if keeps_a_checksum(layout) => {
            write_meta_binary(layout.format, Some(layout), instance)
        }
        _ => return,
    };
    let said = save_verdict(written, stored);
    // `CHKS` ends the body: `{u32 length, u32 checksum, tag}`, big-endian.
    let checksum = (stored.len().checked_sub(8)).and_then(|at| stored.get(at..at + 4));
    match (keeps_a_checksum(layout), checksum) {
        (true, Some(c)) => println!(
            "  save         {said}; checksum {:#010X} kept: {CHECKSUM_KEPT}",
            u32::from_be_bytes([c[0], c[1], c[2], c[3]])
        ),
        _ => println!("  save         {said}"),
    }
}

/// Whether `written`, a save of the unedited tree, gives back `stored`.
fn save_verdict(written: or_io::meta::Result<Vec<u8>>, stored: &[u8]) -> String {
    match written {
        Ok(bytes) if bytes == stored => "writes back byte-identical".to_string(),
        Ok(bytes) => {
            let at = (0..bytes.len().max(stored.len())).find(|&i| bytes.get(i) != stored.get(i));
            format!("writes back other bytes, from {:#X}", at.unwrap_or(0))
        }
        Err(e) => format!("refused: {e}"),
    }
}

/// What `rpf scan-open` prints of an RBF body: `meta info`'s lines, and a save line whichever form
/// it has.
pub(crate) fn print_scanned_rbf(name: &str, bytes: &[u8]) -> Result<(), String> {
    let document = read_instance(bytes, ErrorMode::Lenient)?;
    print_document(name, format_label(bytes), bytes.len(), &document);
    let (instance, Some(layout), _) = &document else {
        return Ok(());
    };
    match &layout.container {
        ContainerLayout::Rbf(r) if !r.wide_values => {
            let written = write_meta_binary(layout.format, Some(layout), instance);
            println!("  save         {}", save_verdict(written, bytes));
        }
        _ => print_gta6_save(layout, instance, bytes),
    }
    Ok(())
}

/// What `meta info` prints of a document read from `len` bytes of the container `label`.
pub(crate) fn print_document(name: &str, label: &str, len: usize, document: &Document) {
    let (instance, layout, diags) = document;
    let stats = Stats::of(instance);
    println!("{name}");
    println!("  container    {label}  ({len} bytes)");
    println!("  root type    {}", named_root(&instance.type_name));
    println!("  root fields  {}", instance.fields.len());
    println!(
        "  tree         {} values, {} structs, {} arrays, {} pointers",
        stats.values, stats.structs, stats.arrays, stats.pointers
    );
    if let Some(layout) = layout {
        match &layout.container {
            ContainerLayout::Pso(p) => println!(
                "  layout       {} schema entries, {} blocks, {} opaque sections",
                p.schema.items.len(),
                p.pmap.entries.len(),
                p.opaque_sections.len()
            ),
            ContainerLayout::Rbm(r) => println!(
                "  layout       {} classes, {} enums, {} blocks",
                r.model.classes.len(),
                r.model.enums.len(),
                r.model.data_blocks.len()
            ),
            ContainerLayout::Rbf(r) => println!(
                "  layout       {} descriptors, {} shapes",
                r.descriptors.len(),
                r.shapes.len()
            ),
        }
    }
    if !diags.is_clean() {
        println!("  reader gaps  {}", diags.gaps().len());
        for ((kind, detail), count) in diags.census().iter().take(10) {
            println!("      {count:>6}  {}  {detail}", kind.as_str());
        }
    }
}

/// Whether the XML this file would produce comes back.
///
/// ⚠ **Not the same check as `roundtrip`**, which asks whether binary survives
/// binary: this goes binary → XML → binary.
///
/// ⚠ **Coming back means reaching the canonical form, not reproducing the
/// original.** The first cycle may move bytes and the second must not. A
/// document whose first cycle also changed the XML lost a value the export
/// could not carry, and is withheld as well.
fn xml_comes_back(bytes: &[u8]) -> Result<(), String> {
    if or_io::meta::detect_binary_format(bytes).is_none() {
        return Ok(());
    }
    match or_io::meta::canonical::converge(schema(), bytes) {
        Ok(c) if c.values_moved => Err(
            "re-importing its XML loses a value: the second export differs from the first".into(),
        ),
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// The XML a file exports as: canonical for a binary container, rewritten
/// for text.
fn export_xml(bytes: &[u8], mode: ErrorMode) -> Result<String, String> {
    let mut diags = Diagnostics::new(mode);
    if or_io::meta::detect_binary_format(bytes).is_some() {
        let (instance, layout) =
            parse_meta_binary_full(schema(), bytes, &mut diags).map_err(|e| e.to_string())?;
        return or_io::meta::canonical::export(&instance, &layout).map_err(|e| e.to_string());
    }
    let instance = or_io::meta::instance::parse_meta(schema(), bytes).map_err(|e| e.to_string())?;
    or_io::meta::write_meta(&instance).map_err(|e| e.to_string())
}

fn cmd_to_xml(args: &[String]) -> Result<ExitCode, String> {
    let opts = Options::parse(args)?;
    if opts.batch.is_some() {
        return cmd_to_xml_batch(&opts);
    }
    let input = opts.require_input()?;
    let bytes = read(input)?;

    if !opts.anyway {
        if let Err(why) = xml_comes_back(&bytes) {
            return Err(format!(
                "{}: {why}.\n  Nothing was written. Exporting this would produce XML that \
                 cannot be imported, and a file that silently lost a field is worse than \
                 no file. Pass --anyway to export it regardless, or `meta info` to see \
                 what the reader could not account for.",
                input.display()
            ));
        }
    }

    let xml = export_xml(&bytes, opts.error_mode())?;
    match opts.output.as_ref() {
        Some(out) => {
            fs::write(out, xml.as_bytes()).map_err(|e| format!("{}: {e}", out.display()))?;
            eprintln!("Wrote {} ({} bytes)", out.display(), xml.len());
        }
        None => print!("{xml}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_to_xml_batch(opts: &Options) -> Result<ExitCode, String> {
    let dir = opts.batch.as_ref().expect("checked by the caller");
    let out_dir = opts
        .output
        .as_ref()
        .or(opts.input.as_ref())
        .ok_or("to-xml --batch needs an output directory")?;
    let files = collect_files(dir)?;
    if files.is_empty() {
        return Err(format!("no metadata files under {}", dir.display()));
    }
    fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;

    let mut written = 0usize;
    let mut withheld: Vec<(PathBuf, String)> = Vec::new();
    let mut failed = 0usize;
    for path in &files {
        let Ok(bytes) = fs::read(path) else {
            failed += 1;
            continue;
        };
        if !opts.anyway {
            if let Err(why) = xml_comes_back(&bytes) {
                withheld.push((path.clone(), why));
                continue;
            }
        }
        let Ok(xml) = export_xml(&bytes, opts.error_mode()) else {
            failed += 1;
            continue;
        };
        let leaf = path.file_name().unwrap_or_default();
        let dest = out_dir.join(leaf).with_extension("xml");
        if fs::write(&dest, xml.as_bytes()).is_err() {
            failed += 1;
            continue;
        }
        written += 1;
    }

    for (path, why) in &withheld {
        eprintln!("withheld {}: {why}", path.display());
    }
    println!(
        "{written} exported, {} withheld, {failed} unreadable, of {} file(s)",
        withheld.len(),
        files.len()
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn cmd_from_xml(args: &[String]) -> Result<ExitCode, String> {
    let opts = Options::parse(args)?;
    let input = opts.require_input()?;
    let output = opts
        .output
        .as_ref()
        .ok_or("from-xml needs an output path")?;

    let xml = read(input)?;

    let bytes = match (opts.like.as_ref(), opts.target) {
        (Some(like), _) => {
            let donor = read(like)?;
            let format = or_io::meta::detect_binary_format(&donor)
                .ok_or_else(|| format!("{} is not a binary metadata file", like.display()))?;
            if format == BinaryFormat::Rbf {
                // RBF carries its layout in its own XML, so a donor has nothing to add.
                or_io::meta::canonical::import(schema(), &xml, format, None)
                    .map_err(|e| e.to_string())?
            } else {
                let mut diags = Diagnostics::lenient();
                let (_, layout) = parse_meta_binary_full(schema(), &donor, &mut diags)
                    .map_err(|e| format!("{}: {e}", like.display()))?;
                let typed = or_io::meta::canonical::typed_by(schema(), Some(&layout));
                // GTA VI's table form names types no schema has, so its XML is read without one.
                let instance = match &layout.container {
                    ContainerLayout::Pso(p) if p.is_table_form() => {
                        or_io::meta::instance::parse_schemaless(&xml).map_err(|e| e.to_string())?
                    }
                    _ => {
                        or_io::meta::canonical::parse(&typed, &xml)
                            .map_err(|e| e.to_string())?
                            .1
                    }
                };
                if keeps_a_checksum(&layout) {
                    eprintln!("openrage-cli: {CHECKSUM_KEPT}");
                }
                write_meta_binary(format, Some(&layout), &instance).map_err(|e| e.to_string())?
            }
        }
        (None, Some(format)) => {
            match or_io::meta::canonical::import(schema(), &xml, format, None) {
                Ok(bytes) => bytes,
                Err(e)
                    if format != BinaryFormat::Rbf
                        && (schema().structs.is_empty()
                            || matches!(e, or_io::meta::MetaError::UnknownType(_))) =>
                {
                    let game_root =
                        crate::search::resolve_game_root(opts.game.clone()).map_err(|why| {
                            format!("{e}; no install to read type tables from: {why}")
                        })?;
                    install_type_tables(&game_root);
                    or_io::meta::canonical::import(schema(), &xml, format, None)
                        .map_err(|e| e.to_string())?
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        (None, None) => {
            return Err(
                "from-xml needs a container: --as pso|rbm|rbf lays one out from the struct \
                 definitions, or --like <file> takes the type table and allocation from an \
                 existing file."
                    .to_string(),
            )
        }
    };
    let format = or_io::meta::detect_binary_format(&bytes).unwrap_or(BinaryFormat::Pso);
    fs::write(output, &bytes).map_err(|e| format!("{}: {e}", output.display()))?;
    eprintln!(
        "Wrote {} ({} bytes, {:?})",
        output.display(),
        bytes.len(),
        format
    );
    Ok(ExitCode::SUCCESS)
}

/// Install what the type tables of every metadata file in `game_root` state,
/// for an import nothing else can type.
///
/// ⚠ **A crawl of the whole install**, minutes on a retail one, so it runs
/// only once an import has failed for want of a definition. Where two files
/// state one type differently, the first read stands.
fn install_type_tables(game_root: &Path) {
    use or_io::meta::binary::type_tables;
    use or_title::install::crawl::{self, ControlFlow};

    let resolved = crate::keys::resolve_crypto(game_root, None);
    let mut warnings = resolved.notes;
    let mut library = or_io::meta::SchemaLibrary::default();
    eprintln!(
        "openrage-cli: reading the type tables in {} ...",
        game_root.display()
    );
    let _ = crawl::crawl(game_root, resolved.provider, &mut warnings, &mut |found| {
        let ext = found
            .leaf
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if crate::verify::is_known(found.leaf, &ext) {
            return ControlFlow::Continue(());
        }
        let Ok(bytes) = found.read() else {
            return ControlFlow::Continue(());
        };
        if !matches!(
            or_io::meta::binary::classify(&bytes),
            or_io::meta::binary::Container::Meta(_)
        ) {
            return ControlFlow::Continue(());
        }
        let mut diags = Diagnostics::lenient();
        if let Ok((_, layout)) = parse_meta_binary_full(schema(), &bytes, &mut diags) {
            type_tables::fill(&mut library, &type_tables::of_layout(&layout));
        }
        ControlFlow::Continue(())
    });
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    type_tables::install(library);
}

fn cmd_rewrite(args: &[String]) -> Result<ExitCode, String> {
    let opts = Options::parse(args)?;
    let input = opts.require_input()?;
    let output = opts.output.as_ref().ok_or("rewrite needs an output path")?;

    let bytes = read(input)?;
    let format = or_io::meta::detect_binary_format(&bytes)
        .ok_or_else(|| format!("{} is not a binary metadata file", input.display()))?;
    let mut diags = Diagnostics::new(opts.error_mode());
    let (instance, layout) = parse_meta_binary_full(schema(), &bytes, &mut diags)
        .map_err(|e| format!("{}: {e}", input.display()))?;
    let written = write_meta_binary(format, Some(&layout), &instance).map_err(|e| e.to_string())?;
    if keeps_a_checksum(&layout) {
        eprintln!("openrage-cli: {CHECKSUM_KEPT}");
    }
    fs::write(output, &written).map_err(|e| format!("{}: {e}", output.display()))?;
    eprintln!(
        "Wrote {} ({} bytes, {} gaps)",
        output.display(),
        written.len(),
        diags.gaps().len()
    );
    Ok(ExitCode::SUCCESS)
}

struct Outcome {
    path: PathBuf,
    format: &'static str,
    passed: bool,
    detail: String,
    gaps: usize,
}

fn cmd_roundtrip(args: &[String]) -> Result<ExitCode, String> {
    let opts = Options::parse(args)?;

    let files: Vec<PathBuf> = if let Some(dir) = opts.batch.as_ref() {
        collect_files(dir)?
    } else {
        vec![opts.require_input()?.clone()]
    };
    if files.is_empty() {
        return Err("no metadata files found".to_string());
    }

    let mut outcomes = Vec::with_capacity(files.len());
    for path in &files {
        outcomes.push(roundtrip_one(path, opts.error_mode()));
    }

    let failed = outcomes.iter().filter(|o| !o.passed).count();
    if opts.json {
        println!(
            "{{\"files\":{},\"failed\":{},\"results\":[",
            outcomes.len(),
            failed
        );
        for (i, o) in outcomes.iter().enumerate() {
            println!(
                "  {{\"path\":{:?},\"format\":\"{}\",\"ok\":{},\"gaps\":{},\"detail\":{:?}}}{}",
                o.path.display().to_string(),
                o.format,
                o.passed,
                o.gaps,
                o.detail,
                if i + 1 == outcomes.len() { "" } else { "," }
            );
        }
        println!("]}}");
    } else {
        for o in &outcomes {
            if o.passed && !opts.verbose {
                continue;
            }
            let mark = if o.passed { "ok  " } else { "FAIL" };
            println!("{mark} [{}] {}  {}", o.format, o.path.display(), o.detail);
        }
        println!();
        println!(
            "{} of {} round-tripped; {failed} failed",
            outcomes.len() - failed,
            outcomes.len()
        );
    }

    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// One row of the published list: a root type OpenRage can read.
struct RootType {
    hash: u32,
    name: Option<String>,
    /// How the name was arrived at.
    named_by: &'static str,
    files: usize,
    containers: std::collections::BTreeMap<&'static str, usize>,
    extensions: std::collections::BTreeSet<String>,
}

/// `openrage-cli meta types`: every metadata root type an install contains.
///
/// ⚠ **The names come from the game, not from a schema snapshot.** RBF0 is
/// self-describing and PSO and RBM are not, and the game ships overlapping
/// types in all three — so a type named in an RBF file names the same type
/// wherever it appears as a bare hash. Measured on a retail Legacy install:
/// the 3 RBF `.ymf` files name `CPackFileMetaData`, whose literal hash is
/// `0x93A68A2F`, which is the root of all 1,792 PSO `.ymf`.
fn cmd_types(args: &[String]) -> Result<ExitCode, String> {
    use or_io::names::{HashFlavour, NameIndex};
    use or_title::install::crawl::{self, ControlFlow};

    let opts = Options::parse(args)?;
    let game_root = crate::search::resolve_game_root(opts.game.clone())?;
    if !game_root.is_dir() {
        return Err(format!(
            "game root is not a directory: {}",
            game_root.display()
        ));
    }
    let resolved = crate::keys::resolve_crypto(&game_root, None);
    let mut warnings = resolved.notes;

    // Pass one: read every metadata file, recording the root hash and
    // harvesting any name the container spelled out.
    let mut seen: std::collections::BTreeMap<u32, RootType> = Default::default();
    let mut harvested = NameIndex::new("gta5");
    let mut unreadable = 0usize;
    eprintln!(
        "openrage-cli: reading the metadata in {} ...",
        game_root.display()
    );
    let _ = crawl::crawl(&game_root, resolved.provider, &mut warnings, &mut |found| {
        let ext = found
            .leaf
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        // ⚠ A format `verify` knows is a paged asset, not metadata, and
        // classifying one costs a full decompression. Measured over a retail
        // Legacy install: 895 s without this line and 371 s with it, same 46
        // root types.
        if crate::verify::is_known(found.leaf, &ext) {
            return ControlFlow::Continue(());
        }
        let Ok(bytes) = found.read() else {
            return ControlFlow::Continue(());
        };
        let container = match or_io::meta::binary::classify(&bytes) {
            or_io::meta::binary::Container::Meta(format) => format,
            _ => return ControlFlow::Continue(()),
        };
        let mut diags = Diagnostics::lenient();
        let Ok(instance) =
            or_io::meta::binary::parse_meta_binary_with(schema(), &bytes, &mut diags)
        else {
            unreadable += 1;
            return ControlFlow::Continue(());
        };

        let spelled = instance.type_name.clone();
        let hash = match hash_of_root(&spelled) {
            Some(hash) => hash,
            None => {
                // The container named it, so the game has told us what this
                // hash means.
                harvested.insert(&spelled);
                HashFlavour::Literal.of(&spelled)
            }
        };

        let row = seen.entry(hash).or_insert_with(|| RootType {
            hash,
            name: None,
            named_by: "nothing",
            files: 0,
            containers: Default::default(),
            extensions: Default::default(),
        });
        row.files += 1;
        *row.containers.entry(container.label()).or_default() += 1;
        row.extensions.insert(ext.clone());
        ControlFlow::Continue(())
    });

    // Pass two: resolve. A name the game spelled out beats one the local
    // snapshot supplies, because the first is evidence and the second is a
    // build artefact somebody may not have.
    for row in seen.values_mut() {
        if let Some(name) = harvested.resolve_with(HashFlavour::Literal, row.hash) {
            row.name = Some(name.to_string());
            row.named_by = "game";
        } else if let Some(name) = or_io::meta::binary::root_types::name_for(row.hash) {
            row.name = Some(name.to_string());
            row.named_by = "public";
        } else if let Some(name) =
            or_io::meta::binary::hash_index::default_hash_index().lookup(row.hash)
        {
            row.name = Some(name.to_string());
            row.named_by = "snapshot";
        }
    }

    // ⚠ A table entry that matches nothing is the way a hand-written list
    // rots, so it is reported rather than left to be discovered.
    let unused: Vec<&str> = or_io::meta::binary::root_types::claims()
        .filter(|(hash, _)| !seen.contains_key(hash))
        .map(|(_, name)| name)
        .collect();

    report_types(&seen, unreadable, &unused, &warnings, &opts);
    Ok(ExitCode::SUCCESS)
}

/// A root type as the reader spelled it, with the public class name beside
/// it where the table has one.
///
/// ⚠ **The hash stays** beside the name: a PSO file stores the number and
/// nothing else, and printing the name alone would hide which of the two the
/// file contains.
fn named_root(type_name: &str) -> String {
    match hash_of_root(type_name).and_then(or_io::meta::binary::root_types::name_for) {
        Some(name) => format!("{type_name}  ({name})"),
        None => type_name.to_string(),
    }
}

/// `hash_D3593FA6` when the reader could not name the type, `None` when it
/// could — in which case the name itself is the answer.
fn hash_of_root(type_name: &str) -> Option<u32> {
    or_io::meta::binary::hash_index::spelled_hash(type_name)
}

fn report_types(
    seen: &std::collections::BTreeMap<u32, RootType>,
    unreadable: usize,
    unused: &[&str],
    warnings: &[String],
    opts: &Options,
) {
    let named = seen.values().filter(|r| r.name.is_some()).count();
    let files: usize = seen.values().map(|r| r.files).sum();

    if opts.json {
        let rows: Vec<serde_json::Value> = seen
            .values()
            .map(|r| {
                serde_json::json!({
                    "hash": format!("0x{:08X}", r.hash),
                    "name": r.name,
                    "namedBy": r.named_by,
                    "files": r.files,
                    "containers": r.containers,
                    "extensions": r.extensions,
                })
            })
            .collect();
        let doc = serde_json::json!({
            "rootTypes": rows,
            "named": named,
            "total": seen.len(),
            "files": files,
            "unreadable": unreadable,
            "unusedKnownNames": unused,
            "schemaSnapshot": has_schema(),
            "warnings": warnings,
        });
        match serde_json::to_string_pretty(&doc) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("Error: serialize: {e}"),
        }
        return;
    }

    for w in warnings {
        eprintln!("note: {w}");
    }
    println!(
        "{:<12} {:<34} {:<10} {:>8}  containers",
        "hash", "name", "named by", "files"
    );
    let mut rows: Vec<&RootType> = seen.values().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.files));
    for r in rows {
        let containers: Vec<String> = r
            .containers
            .iter()
            .map(|(c, n)| format!("{c} x{n}"))
            .collect();
        println!(
            "0x{:08X}   {:<34} {:<10} {:>8}  {}",
            r.hash,
            r.name.as_deref().unwrap_or("—"),
            r.named_by,
            r.files,
            containers.join(", ")
        );
    }
    println!(
        "\n{} root type(s), {named} named, over {files} file(s){}",
        seen.len(),
        if unreadable > 0 {
            format!("; {unreadable} unreadable")
        } else {
            String::new()
        }
    );
    if !unused.is_empty() {
        println!(
            "\n⚠ {} known name(s) matched no root type in this install: {}. \
             Either the install does not ship them or the entry is wrong; a name that \
             matches nothing is never printed as a result.",
            unused.len(),
            unused.join(", ")
        );
    }
    if !has_schema() {
        println!(
            "\n⚠ No schema snapshot is staged. Root types named `game` or `public` do \
             not need one — the container spelled them out, or the name is public and \
             its hash matched. Field names do need one."
        );
    }
}

fn roundtrip_one(path: &Path, mode: ErrorMode) -> Outcome {
    let mut outcome = Outcome {
        path: path.to_path_buf(),
        format: "?",
        passed: false,
        detail: String::new(),
        gaps: 0,
    };
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            outcome.detail = format!("read failed: {e}");
            return outcome;
        }
    };
    outcome.format = format_label(&bytes);
    let Some(format) = or_io::meta::detect_binary_format(&bytes) else {
        outcome.detail = "not a binary metadata file".into();
        return outcome;
    };

    let mut diags = Diagnostics::new(mode);
    let (instance, layout) = match parse_meta_binary_full(schema(), &bytes, &mut diags) {
        Ok(v) => v,
        Err(e) => {
            outcome.detail = format!("parse failed: {e}");
            return outcome;
        }
    };
    outcome.gaps = diags.gaps().len();

    let written = match write_meta_binary(format, Some(&layout), &instance) {
        Ok(b) => b,
        Err(e) => {
            outcome.detail = format!("write failed: {e}");
            return outcome;
        }
    };

    if format == BinaryFormat::Rbm {
        let before =
            <or_io::container::resource::Rsc7 as or_io::container::resource::ResourceEnvelope>::open(
                &bytes,
            )
            .map(|r| r.data);
        let after =
            <or_io::container::resource::Rsc7 as or_io::container::resource::ResourceEnvelope>::open(
                &written,
            )
            .map(|r| r.data);
        match (before, after) {
            (Ok(a), Ok(b)) if a == b => {
                outcome.passed = true;
                outcome.detail = format!("payload identical ({} bytes)", a.len());
            }
            (Ok(a), Ok(b)) => {
                outcome.detail = format!(
                    "payload differs at {:#X} ({} vs {} bytes)",
                    a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(0),
                    b.len(),
                    a.len()
                );
            }
            _ => outcome.detail = "could not decompress for comparison".into(),
        }
        return outcome;
    }

    if written == bytes {
        outcome.passed = true;
        outcome.detail = format!("byte-identical ({} bytes)", bytes.len());
    } else {
        let at = written
            .iter()
            .zip(&bytes)
            .position(|(a, b)| a != b)
            .unwrap_or(written.len().min(bytes.len()));
        outcome.detail = format!(
            "differs at {at:#X} ({} vs {} bytes)",
            written.len(),
            bytes.len()
        );
    }
    outcome
}

fn collect_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = fs::read_dir(&current).map_err(|e| format!("{}: {e}", current.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(head) = read_head(&path) else { continue };
            if or_io::meta::detect_binary_format(&head).is_some() {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut buf = [0u8; 4];
    let n = f.read(&mut buf)?;
    Ok(buf[..n].to_vec())
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Default)]
struct Stats {
    values: usize,
    structs: usize,
    arrays: usize,
    pointers: usize,
}

impl Stats {
    fn of(inst: &MetaInstance) -> Self {
        let mut s = Stats::default();
        s.walk_instance(inst);
        s
    }

    fn walk_instance(&mut self, inst: &MetaInstance) {
        for (_, v) in &inst.fields {
            self.walk(v);
        }
    }

    fn walk(&mut self, value: &Value) {
        self.values += 1;
        match value {
            Value::Struct(inner) => {
                self.structs += 1;
                self.walk_instance(inner);
            }
            Value::Pointer(Some(inner)) => {
                self.pointers += 1;
                self.walk_instance(inner);
            }
            Value::Array(items) => {
                self.arrays += 1;
                for i in items {
                    self.walk(i);
                }
            }
            Value::Map(pairs) => {
                for (k, v) in pairs {
                    self.walk(k);
                    self.walk(v);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct Options {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    like: Option<PathBuf>,
    batch: Option<PathBuf>,
    game: Option<PathBuf>,
    json: bool,
    verbose: bool,
    strict: bool,
    /// Export even when the XML will not come back.
    anyway: bool,
    /// The container `from-xml` lays out, when no donor supplies one.
    target: Option<BinaryFormat>,
}

fn parse_container(name: &str) -> Result<BinaryFormat, String> {
    match name.to_ascii_lowercase().as_str() {
        "pso" => Ok(BinaryFormat::Pso),
        "rbm" => Ok(BinaryFormat::Rbm),
        "rbf" => Ok(BinaryFormat::Rbf),
        other => Err(format!("--as takes pso, rbm or rbf, not '{other}'")),
    }
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Options::default();
        let mut positional = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--json" => o.json = true,
                "--verbose" | "-v" => o.verbose = true,
                "--strict" => o.strict = true,
                "--anyway" => o.anyway = true,
                "--as" => {
                    i += 1;
                    o.target = Some(parse_container(
                        args.get(i).ok_or("--as needs pso, rbm or rbf")?,
                    )?);
                }
                other if other.starts_with("--as=") => {
                    o.target = Some(parse_container(other.trim_start_matches("--as="))?);
                }
                "--game" => {
                    i += 1;
                    o.game = Some(PathBuf::from(
                        args.get(i).ok_or("--game needs a directory")?.clone(),
                    ));
                }
                other if other.starts_with("--game=") => {
                    o.game = Some(PathBuf::from(other.trim_start_matches("--game=")));
                }
                "--like" => {
                    i += 1;
                    o.like = Some(PathBuf::from(
                        args.get(i).ok_or("--like needs a path")?.clone(),
                    ));
                }
                "--batch" => {
                    i += 1;
                    o.batch = Some(PathBuf::from(
                        args.get(i).ok_or("--batch needs a directory")?.clone(),
                    ));
                }
                other if other.starts_with("--like=") => {
                    o.like = Some(PathBuf::from(other.trim_start_matches("--like=")));
                }
                other if other.starts_with("--batch=") => {
                    o.batch = Some(PathBuf::from(other.trim_start_matches("--batch=")));
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option '{other}'"));
                }
                other => positional.push(PathBuf::from(other)),
            }
            i += 1;
        }
        let mut it = positional.into_iter();
        o.input = it.next();
        o.output = it.next();
        Ok(o)
    }

    fn require_input(&self) -> Result<&PathBuf, String> {
        self.input
            .as_ref()
            .ok_or_else(|| "no input file".to_string())
    }

    fn error_mode(&self) -> ErrorMode {
        if self.strict {
            ErrorMode::Strict
        } else {
            ErrorMode::Lenient
        }
    }
}

/// The metadata schema, installed on first use from where `or_title::schema` finds it.
pub(crate) fn schema() -> &'static or_io::meta::SchemaLibrary {
    static FOUND: std::sync::Once = std::sync::Once::new();
    FOUND.call_once(|| {
        if let Some(path) = or_title::schema::find_gta6_snapshot(config_dir().as_deref()) {
            if let Err(error) = or_io::meta::install_names_snapshot(&path) {
                tracing::warn!(path = %path.display(), %error, "GTA VI's schema snapshot is unusable");
            }
        }
        let Some(path) = or_title::schema::find_snapshot(config_dir().as_deref()) else {
            return;
        };
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "the schema snapshot is unusable; field names read as hashes"
            );
        }
    });
    or_io::meta::default_library()
}

pub(crate) fn has_schema() -> bool {
    schema();
    or_io::meta::has_default_library()
}

/// The user's configuration directory, where the keys cache lives.
fn config_dir() -> Option<PathBuf> {
    or_title::config::config_dir()
}

fn cmd_snapshot(args: &[String]) -> Result<ExitCode, String> {
    let mut psc_root = None;
    let mut out = None;
    let mut status = false;
    let (mut par_tree, mut store) = (None, None);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{arg} expects a value"))
        };
        match arg.as_str() {
            "--psc-root" => psc_root = Some(value()?),
            "--out" => out = Some(value()?),
            "--par-tree" => par_tree = Some(value()?),
            "--store" => store = Some(value()?),
            "--status" => status = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    if status {
        snapshot_status();
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(dump) = par_tree {
        return snapshot_of_par_tree(&dump, out, store);
    }
    let out = out.or(store.map(|corpus| or_title::schema::store_file(&corpus)));

    let psc_root = psc_root
        .or_else(|| std::env::var_os("OPENRAGE_PSC_ROOT").map(PathBuf::from))
        .ok_or("--psc-root not provided and OPENRAGE_PSC_ROOT not set")?;
    if !psc_root.is_dir() {
        return Err(format!("PSC root not found: {}", psc_root.display()));
    }
    let out = match out {
        Some(out) => out,
        None => or_title::schema::source_tree()
            .and_then(or_title::schema::corpus_above)
            .map(|corpus| or_title::schema::store_file(&corpus))
            .ok_or("no openrage-corpus beside or above this source tree; pass --out or --store")?,
    };

    let started = std::time::Instant::now();
    let mut psc_files = Vec::new();
    walk_psc(&psc_root, &mut psc_files)
        .map_err(|e| format!("walking {}: {e}", psc_root.display()))?;
    psc_files.sort();
    println!(
        "Scanning {} ... found {} PSC file(s)",
        psc_root.display(),
        psc_files.len()
    );
    let mut sources: Vec<(String, Vec<u8>)> = Vec::with_capacity(psc_files.len());
    for path in &psc_files {
        let rel = path
            .strip_prefix(&psc_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        match fs::read(path) {
            Ok(bytes) => sources.push((rel, bytes)),
            Err(e) => eprintln!("warning: failed to read {}: {e}", path.display()),
        }
    }
    let lib = or_io::meta::build_library(sources.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
        .map_err(|e| format!("build_library failed: {e}"))?;
    let bytes = or_io::meta::encode_snapshot(&lib).map_err(|e| format!("encode failed: {e}"))?;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(&out, &bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "Wrote {} bytes to {} ({} struct(s), {} enum(s)) in {:.2?}",
        bytes.len(),
        out.display(),
        lib.structs.len(),
        lib.enums.len(),
        started.elapsed()
    );

    // ⚠ The stamp names the `build_library` that wrote the snapshot. Without
    // it a snapshot outlives a change to `src/schema` and every worktree on
    // the machine keeps measuring the old answer.
    let stamp_at = or_io::meta::snapshot_discovery::stamp_path(&out);
    match or_io::meta::builder_stamp() {
        Some(stamp) => {
            fs::write(&stamp_at, format!("{stamp}\n"))
                .map_err(|e| format!("write {}: {e}", stamp_at.display()))?;
            println!("Stamped {} with schema/ {stamp}", stamp_at.display());
        }
        None => eprintln!(
            "warning: not running from a source tree, so {} carries no generator stamp",
            out.display()
        ),
    }
    Ok(ExitCode::SUCCESS)
}

/// Writes GTA VI's snapshot from the parser dump at `dump`: to `out`, else into the store of the
/// corpus directory `store`, else into the store above this source tree.
fn snapshot_of_par_tree(
    dump: &Path,
    out: Option<PathBuf>,
    store: Option<PathBuf>,
) -> Result<ExitCode, String> {
    let corpus = || {
        store.or_else(|| or_title::schema::source_tree().and_then(or_title::schema::corpus_above))
    };
    let out = out
        .or_else(|| Some(corpus()?.join(or_title::schema::GTA6_STORE_FILE)))
        .ok_or("no openrage-corpus beside or above this source tree; pass --out or --store")?;
    let json = fs::read(dump).map_err(|e| format!("{}: {e}", dump.display()))?;
    let lib = or_io::meta::par_tree::library_from_par_tree(&json)
        .map_err(|e| format!("{}: {e}", dump.display()))?;
    let bytes = or_io::meta::encode_snapshot(&lib).map_err(|e| format!("encode failed: {e}"))?;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    fs::write(&out, &bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "Wrote {} bytes to {} ({} struct(s), {} enum(s))",
        bytes.len(),
        out.display(),
        lib.structs.len(),
        lib.enums.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// Which snapshot this run finds, what is in it, and which generator wrote it.
fn snapshot_status() {
    match or_title::schema::find_gta6_snapshot(config_dir().as_deref()) {
        Some(path) => match or_io::meta::load_snapshot(&path) {
            Ok(lib) => println!(
                "GTA VI snapshot, read for its names: {} ({} struct(s), {} enum(s))",
                path.display(),
                lib.structs.len(),
                lib.enums.len()
            ),
            Err(e) => println!("GTA VI snapshot: {} is unusable: {e}", path.display()),
        },
        None => println!("GTA VI snapshot: none found."),
    }
    let Some(path) = or_title::schema::find_snapshot(config_dir().as_deref()) else {
        println!("snapshot: none found, so the schema library is empty.");
        println!(
            "Looked at {}, the configuration directory and, from a source tree, the \
             store above it. Field names outside binary::public_names read as hashes and \
             XML re-import fails at the root type. Run `just snapshot <psc-root>`.",
            or_title::schema::SCHEMA_ENV
        );
        return;
    };
    println!("snapshot: {}", path.display());
    match or_io::meta::load_snapshot(&path) {
        Ok(lib) => println!(
            "contents: {} struct(s), {} enum(s)",
            lib.structs.len(),
            lib.enums.len()
        ),
        Err(e) => println!("contents: unusable: {e}"),
    }
    let age = or_io::meta::snapshot_age(&path);
    println!(
        "generator: {}",
        match &age {
            or_io::meta::SnapshotAge::Current => format!(
                "current, schema/ {}",
                or_io::meta::builder_stamp().unwrap_or("?")
            ),
            or_io::meta::SnapshotAge::Unstamped => "UNSTAMPED".to_string(),
            or_io::meta::SnapshotAge::Stale { recorded, .. } =>
                format!("STALE, built by {recorded}"),
            or_io::meta::SnapshotAge::Unjudged => "not judged outside a source tree".to_string(),
        }
    );
    if let Some(concern) = age.concern() {
        println!();
        println!("{concern}");
    }
}

fn walk_psc(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk_psc(&path, out)?;
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("psc"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("Inspect and convert GTA V metadata (PSO / RBM / RBF0 / XML)");
    eprintln!();
    eprintln!("Usage: openrage-cli meta <subcommand> [options]");
    eprintln!();
    eprintln!("Subcommands:");
    eprintln!("  info <file>                    Container, root type and tree summary.");
    eprintln!("  to-xml <file> [out.xml]        Decode to XML. Prints to stdout without [out].");
    eprintln!("  from-xml <in.xml> <out> --as pso|rbm|rbf [--game <dir>]");
    eprintln!("                                 Encode XML, typed by the schema snapshot and");
    eprintln!("                                 laid out from its values. Where no snapshot");
    eprintln!("                                 declares the root, PSO and RBM take their");
    eprintln!("                                 types from the install's own type tables.");
    eprintln!("  from-xml <in.xml> <out> --like <donor>");
    eprintln!("                                 Encode XML into a binary container, taking the");
    eprintln!("                                 type table and allocation from <donor>.");
    eprintln!(
        "  rewrite <in> <out>             Read and re-encode unchanged (used by cw-compare)."
    );
    eprintln!("  roundtrip <file>               Read, re-write, and compare.");
    eprintln!("  roundtrip --batch <dir>        Same across a directory tree.");
    eprintln!("  to-xml --batch <dir> <outdir>  Export a whole tree, withholding what");
    eprintln!("                                 will not come back.");
    eprintln!("  types --game <dir>             Every metadata root type in an install,");
    eprintln!("                                 named where the game names it.");
    eprintln!("  snapshot --psc-root <dir> [--out <file> | --store <corpus-dir>]");
    eprintln!("                                 Build the schema snapshot from a PSC tree;");
    eprintln!("                                 by default into the store above this tree.");
    eprintln!("  snapshot --par-tree <dump.json> [--out <file> | --store <corpus-dir>]");
    eprintln!("                                 Build GTA VI's snapshot from a parser dump, into");
    eprintln!("                                 the shared store beside the first; it is read for");
    eprintln!(
        "                                 its names ({}).",
        or_title::schema::GTA6_SCHEMA_ENV
    );
    eprintln!("  snapshot --status             Which snapshot this run finds, and its age.");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --strict     Treat anything the reader cannot decode as an error.");
    eprintln!("  --json       Machine-readable output.");
    eprintln!("  --verbose    In batch roundtrip, list passing files too.");
    eprintln!("  --anyway     Export even when the XML will not re-import. Off by default:");
    eprintln!("               a file that silently lost a field is worse than no file.");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  PSO and RBF round-trip byte-identically. RBM cannot: its payload is raw");
    eprintln!("  DEFLATE and the original compressor's output is not reproducible, so the");
    eprintln!("  comparison is on the decompressed payload instead.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  openrage-cli meta info common/data/peds.ymt");
    eprintln!("  openrage-cli meta to-xml peds.ymt peds.xml");
    eprintln!("  openrage-cli meta from-xml peds.xml new.ymt --like peds.ymt");
    eprintln!("  openrage-cli meta roundtrip --batch corpus/ --json");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ Three crates name the store, and none can depend on the others: the
    /// snapshot writer and the editor read `or_title`'s, a suite reads
    /// `or_test_support`'s, and the shape directory is `or_io::meta`'s.
    #[test]
    fn every_crate_names_the_same_schema_store() {
        assert_eq!(or_title::schema::SCHEMA_ENV, or_test_support::SCHEMA_ENV);
        assert_eq!(or_title::schema::STORE_FILE, or_test_support::SCHEMA_STORE);
        assert_eq!(
            Path::new(or_title::schema::STORE_FILE)
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str()),
            Some(or_io::meta::snapshot_discovery::SNAPSHOT_FORMAT)
        );
    }

    /// The reader spells an unnamed root `hash_HHHHHHHH` and a named one with
    /// its name, and `meta types` has to tell them apart to know whether it
    /// has learnt something.
    #[test]
    fn a_root_type_is_a_hash_only_when_the_reader_could_not_name_it() {
        assert_eq!(hash_of_root("hash_D3593FA6"), Some(0xD359_3FA6));
        assert_eq!(hash_of_root("hash_00000000"), Some(0));
        assert_eq!(hash_of_root("hash_table"), None);
        assert_eq!(hash_of_root("CMapData"), None);
        assert_eq!(hash_of_root("CPackFileMetaData"), None);
        // Not a hash the reader would have written, so not one to parse.
        assert_eq!(hash_of_root("0xZZZZ"), None);
        assert_eq!(hash_of_root(""), None);
    }

    /// ⚠ A name the game spelled out and the literal hash of that name are
    /// the same fact, which is what lets an RBF file name a PSO one.
    #[test]
    fn a_harvested_name_names_the_hash_the_other_containers_store() {
        use or_io::names::{HashFlavour, NameIndex};

        let mut harvested = NameIndex::new("gta5");
        harvested.insert("CPackFileMetaData");
        assert_eq!(
            harvested.resolve_with(HashFlavour::Literal, 0x93A6_8A2F),
            Some("CPackFileMetaData"),
            "the 3 RBF .ymf name the root of the 1792 PSO .ymf"
        );
        assert_eq!(
            harvested.resolve_with(HashFlavour::Literal, 0xD359_3FA6),
            None,
            "and it must not name anything else"
        );
    }

    #[test]
    fn text_input_has_nothing_to_withhold() {
        assert!(xml_comes_back(b"<CMapData/>").is_ok());
    }

    #[test]
    fn a_container_that_cannot_be_read_is_refused_rather_than_exported() {
        let err = xml_comes_back(b"PSIN\0\0\0\0garbage")
            .expect_err("a truncated PSO must not pass the gate");
        assert!(err.starts_with("the original does not read:"), "{err}");
    }
}
