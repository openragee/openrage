//! `openrage-cli dlc`: build a DLC pack, and read one back.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::sync::Arc;

use or_io::container::rpf::{NoOpCryptoProvider, RpfEncryption};
use or_title::dlc::{
    build_pack_with, ContentChangeSetDef, ContentDef, ContentXml, DlcPackSpec, PackDocuments,
    PackFile, SetupContentType, SetupData,
};

pub fn run(args: &[String]) -> ExitCode {
    let result = match args.first().map(String::as_str) {
        Some("create") => cmd_create(&args[1..]),
        Some("inspect") => cmd_inspect(&args[1..]),
        Some("create-gta6") => crate::gta6_dlc::run(&args[1..]),
        Some("-h") | Some("--help") | Some("help") => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(format!("unknown command: {other}")),
        None => {
            print_usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         openrage-cli dlc create <src_dir> <out_dir> --name <name> [--type compat|level|tu|paid]\n                          \
         [--order N] [--minor-order N] [--encryption open|aes|ng]\n  \
         openrage-cli dlc inspect <dlc.rpf | document>\n\n\
         inspect also reads one setup, content or change-set document, GTA V's or GTA VI's.\n\n\
         create builds <out_dir>/<name>/dlc.rpf from everything under <src_dir>,\n\
         preserving the tree, and prints the dlclist.xml line the pack needs.\n\n\
         It never writes into a game install: install the pack by copying the\n\
         folder it produced and adding that line yourself.\n"
    );
}

/// What `dlc create` was asked for.
struct CreateArgs {
    src: PathBuf,
    out_root: PathBuf,
    name: String,
    content_type: SetupContentType,
    order: i32,
    minor_order: i32,
    encryption: RpfEncryption,
}

/// The arguments of `dlc create` as it reads them; an `Err` is what it refuses.
fn create_args(args: &[String]) -> Result<CreateArgs, String> {
    let mut positional = Vec::new();
    let mut name = String::new();
    let mut content_type = SetupContentType::CompatPack;
    let mut order = 255i32;
    let mut minor_order = 0i32;
    let mut encryption = RpfEncryption::Open;

    let mut i = 0;
    while i < args.len() {
        let next = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{what} needs a value"))
        };
        match args[i].as_str() {
            "--name" => {
                name = next(i, "--name")?;
                i += 2;
            }
            "--type" => {
                let v = next(i, "--type")?;
                content_type = match v.to_ascii_lowercase().as_str() {
                    "compat" => SetupContentType::CompatPack,
                    "level" => SetupContentType::LevelPack,
                    "tu" => SetupContentType::TitleUpdate,
                    "paid" => SetupContentType::PaidPack,
                    other => return Err(format!("unknown --type {other}")),
                };
                i += 2;
            }
            "--order" => {
                order = next(i, "--order")?
                    .parse()
                    .map_err(|_| "--order wants a number")?;
                i += 2;
            }
            "--minor-order" => {
                minor_order = next(i, "--minor-order")?
                    .parse()
                    .map_err(|_| "--minor-order wants a number")?;
                i += 2;
            }
            "--encryption" => {
                let v = next(i, "--encryption")?;
                encryption = match v.to_ascii_lowercase().as_str() {
                    "open" => RpfEncryption::Open,
                    "aes" => RpfEncryption::Aes,
                    "ng" => RpfEncryption::Ng,
                    other => return Err(format!("unknown --encryption {other}")),
                };
                i += 2;
            }
            other => {
                positional.push(other.to_string());
                i += 1;
            }
        }
    }

    let [src, out_root] = positional.as_slice() else {
        return Err("expected <src_dir> and <out_dir>".into());
    };
    if name.is_empty() {
        return Err("--name is required; it becomes the folder, the device and the hash".into());
    }
    Ok(CreateArgs {
        src: PathBuf::from(src),
        out_root: PathBuf::from(out_root),
        name,
        content_type,
        order,
        minor_order,
        encryption,
    })
}

/// Whether `args` are ones `dlc create` takes, read as it reads them and without building a pack.
pub(crate) fn accepts_create(args: &[String]) -> Result<(), String> {
    create_args(args).map(drop)
}

fn cmd_create(args: &[String]) -> Result<(), String> {
    let CreateArgs {
        src,
        out_root,
        name,
        content_type,
        order,
        minor_order,
        encryption,
    } = create_args(args)?;

    if !src.is_dir() {
        return Err(format!("{} is not a directory", src.display()));
    }

    let mut spec = dlc_pack_of_folder(&src, &name)?;
    spec.content_type = content_type;
    spec.order = order;
    spec.minor_order = minor_order;
    let (out, docs) = create_dlc_pack(&spec, &out_root, encryption, &mut |_| true)?;

    println!("wrote {}", out.display());
    println!(
        "  {} file(s), device {}",
        spec.files.len(),
        spec.device_name()
    );
    println!("  registered against {}", spec.content_device());
    println!();
    println!("Add this to dlclist.xml, inside <Paths>:");
    println!("  <Item>{}</Item>", docs.dlclist_entry);
    Ok(())
}

/// The pack `dlc create` builds of `src`: every file under it, at its path there.
pub fn dlc_pack_of_folder(src: &Path, name: &str) -> Result<DlcPackSpec, String> {
    let mut spec = DlcPackSpec::new(name);
    for path in walk(src) {
        let rel = path
            .strip_prefix(src)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        spec.files.push(PackFile::new(rel, path));
    }
    Ok(spec)
}

/// Why `dlc create` refuses to build `spec` with `encryption`, in its words; empty when it builds it.
pub fn dlc_refusals(spec: &DlcPackSpec, encryption: RpfEncryption) -> Vec<String> {
    let mut refused = spec.problems();
    if encryption != RpfEncryption::Open {
        refused.push(ONLY_OPEN.to_string());
    }
    refused
}

/// Build `spec` as `<out_root>/<name>/dlc.rpf`, as `dlc create` does, telling `go_on` the bytes written; a `false` stops it.
///
/// ⚠ The app's Create DLC pack… builds through this too, so a folder packs
/// the same from either.
pub fn create_dlc_pack(
    spec: &DlcPackSpec,
    out_root: &Path,
    encryption: RpfEncryption,
    go_on: &mut dyn FnMut(u64) -> bool,
) -> Result<(PathBuf, PackDocuments), String> {
    // ⚠ Encrypting needs keys this command does not take, so anything but
    // `open` is refused here rather than written wrong. A pack in a `mods`
    // folder does not need encryption.
    if encryption != RpfEncryption::Open {
        return Err(ONLY_OPEN.into());
    }
    let out = out_root.join(&spec.name).join("dlc.rpf");
    let docs = build_pack_with(
        spec,
        &out,
        encryption,
        Arc::new(NoOpCryptoProvider::new()),
        go_on,
    )
    .map_err(|e| e.to_string())?;
    Ok((out, docs))
}

const ONLY_OPEN: &str = "only an open, unencrypted archive is built; a pack the game loads from \
                         dlcpacks does not need an encrypted one";

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn cmd_inspect(args: &[String]) -> Result<(), String> {
    let (keys_override, args) = crate::rpf::split_keys_arg(args)?;
    let Some(path) = args.first() else {
        return Err("expected <dlc.rpf>".into());
    };
    let path = PathBuf::from(path);
    if is_xml_document(&path)? {
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return inspect_document(&bytes);
    }
    // ⚠ A shipped `dlc.rpf` is NG-encrypted, so the common case for this
    // command needs keys. They come from the install the archive sits in, so
    // reading a pack in place asks for nothing.
    let crypto = crate::rpf::resolve_read_provider(&path, keys_override.as_deref())?;
    let archive = or_io::container::rpf::RpfArchive::open_with(&path, crypto)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let read = |want: &str| archive.read(want).ok();

    let setup_bytes = read("setup2.xml").ok_or("no setup2.xml; this is not a DLC pack")?;
    let setup = SetupData::parse(&setup_bytes).map_err(|e| format!("setup2.xml: {e}"))?;
    print_setup(&setup);
    println!("registers against {}", setup.content_device());
    print_groups(&setup);

    let dat = if setup.dat_file.is_empty() {
        "content.xml".to_string()
    } else {
        setup.dat_file.clone()
    };
    let Some(content_bytes) = read(&dat) else {
        println!("\n{dat} is missing, so the pack registers nothing");
        return Ok(());
    };
    let content = ContentXml::parse(&content_bytes).map_err(|e| format!("{dat}: {e}"))?;
    print_content(&content);
    Ok(())
}

fn print_setup(setup: &SetupData) {
    println!("name      {}", setup.name_hash);
    println!("device    {}", setup.device_name);
    println!("type      {}", setup.type_name());
    println!("order     {} (minor {})", setup.order, setup.minor_order);
    if let Some(under_root) = setup.platform_data_under_root {
        println!("platform data under root {under_root}");
    }
}

fn print_groups(setup: &SetupData) {
    for group in &setup.content_change_set_groups {
        let names: Vec<&str> = group
            .content_change_sets
            .iter()
            .map(|c| c.value.as_str())
            .collect();
        println!("group     {} -> {}", group.name_hash, names.join(", "));
    }
}

/// Whether `path` holds an XML document rather than an archive, judged by its first bytes.
fn is_xml_document(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let head = &head[..n];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    Ok(head
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|&b| b == b'<'))
}

/// A setup, content or change-set document given on its own: GTA V's `setup2.xml` and `content.xml`, or GTA VI's.
///
/// ⚠ Paths are printed as written: what GTA VI's `$(root)`, `$(target)` and
/// `$(common)` expand to is not known.
pub(crate) fn inspect_document(bytes: &[u8]) -> Result<(), String> {
    let setup = match SetupData::parse(bytes) {
        Ok(setup) => {
            print_setup(&setup);
            print_groups(&setup);
            return Ok(());
        }
        Err(e) => e,
    };
    let content = match ContentXml::parse(bytes) {
        Ok(content) => {
            print_content(&content);
            return Ok(());
        }
        Err(e) => e,
    };
    let def = match ContentDef::parse(bytes) {
        Ok(def) => {
            print_content_def(&def);
            return Ok(());
        }
        Err(e) => e,
    };
    match ContentChangeSetDef::parse(bytes) {
        Ok(set) => {
            print_change_set("change set", &set);
            Ok(())
        }
        Err(set) => Err(format!("{setup}; {content}; {def}; {set}")),
    }
}

fn print_content_def(def: &ContentDef) {
    println!(
        "GTA VI content definition: {} content set(s), {} included set(s), {} included change set(s)",
        def.content_sets.len(),
        def.included_sets.len(),
        def.included_changesets.len()
    );
    for inc in &def.included_changesets {
        let mut line = format!("includes  {}", inc.load_path);
        if !inc.changeset_include_flags.is_empty() {
            line.push_str(&format!(" [{}]", inc.changeset_include_flags.join(" ")));
        }
        if !inc.platform.is_empty() || !inc.subplatform.is_empty() {
            line.push_str(&format!(" (platform {} {})", inc.platform, inc.subplatform));
        }
        println!("{}", line.trim_end());
    }
    for set in &def.included_sets {
        println!("set file  {}", set.value);
    }
    for set in &def.content_sets {
        print_change_set("content set", set);
    }
    print_unread("", &def.unread);
}

fn print_change_set(what: &str, set: &ContentChangeSetDef) {
    println!(
        "{what} {} [{}]: {} file(s)",
        set.id,
        set.flags.join(" "),
        set.file_list.len()
    );
    let include_flags = set.changeset_include_flags.join(" ");
    for (label, value) in [
        ("registration group", &set.registration_group),
        ("associated map", &set.associated_map),
        ("include flags", &include_flags),
    ] {
        if !value.is_empty() {
            println!("  {label} {value}");
        }
    }
    if let Some(conditions) = &set.execution_conditions {
        println!(
            "  conditions active `{}`, generic `{}`",
            conditions.active_changeset_conditions, conditions.generic_conditions
        );
    }
    let mut types = std::collections::BTreeMap::<&str, usize>::new();
    for file in &set.file_list {
        *types.entry(file.file_type.as_str()).or_default() += 1;
    }
    for (file_type, n) in types {
        let name = if file_type.is_empty() {
            "(no fileType)"
        } else {
            file_type
        };
        println!("  {n:>5} {name}");
    }
    let mut flags = std::collections::BTreeMap::<&str, usize>::new();
    for flag in set.file_list.iter().flat_map(|f| &f.flags) {
        *flags.entry(flag.as_str()).or_default() += 1;
    }
    for (flag, n) in flags {
        println!("  {n:>5} file(s) flagged {flag}");
    }
    for (path_macro, n) in set.unexpanded() {
        println!(
            "  {n:>5} file(s) under {path_macro}, as written: no document states what it expands to"
        );
    }
    print_unread("  ", &set.unread);
}

fn print_unread(indent: &str, unread: &[String]) {
    for path in unread {
        println!("{indent}not read  {path}");
    }
}

fn print_content(content: &ContentXml) {
    let enabled = content.files_to_enable();
    println!(
        "\n{} data file(s), {} change set(s)",
        content.data_files.len(),
        content.content_change_sets.len()
    );

    // ⚠ The one thing worth saying out loud about a pack: a registered file
    // no change set enables never loads, and the pack looks like it works.
    let orphans: Vec<&str> = content
        .data_files
        .iter()
        .filter(|d| {
            d.disabled
                && !enabled
                    .iter()
                    .any(|e| e.value.eq_ignore_ascii_case(&d.filename))
        })
        .map(|d| d.filename.as_str())
        .collect();
    if orphans.is_empty() {
        println!("every disabled file is enabled by a change set");
    } else {
        println!(
            "{} registered file(s) are disabled and no change set enables them:",
            orphans.len()
        );
        for path in orphans.iter().take(10) {
            println!("  {path}");
        }
        if orphans.len() > 10 {
            println!("  … and {} more", orphans.len() - 10);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_reads_each_kind_of_document_and_refuses_another() {
        for doc in [
            &b"<SSetupData><nameHash>a</nameHash></SSetupData>"[..],
            b"<CDataFileMgr__ContentsOfDataFileXml />",
            b"<CDataFileMgr__ContentDef><includedSets /></CDataFileMgr__ContentDef>",
            b"<CDataFileMgr__ContentChangeSetDef><id>a</id></CDataFileMgr__ContentChangeSetDef>",
        ] {
            assert!(
                inspect_document(doc).is_ok(),
                "{}",
                String::from_utf8_lossy(doc)
            );
        }
        let err = inspect_document(b"<SMandatoryPacksData />").unwrap_err();
        assert!(err.contains("SMandatoryPacksData"), "{err}");
    }
}
