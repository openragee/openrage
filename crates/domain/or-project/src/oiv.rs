//! The `.oiv` output: the plan of a package, the write of it and the read back.
//!
//! The plan lists the files at the paths they install to, and the package's manifest as the one
//! document. The zip keeps each file under `content/` at that path, so the plan's paths and the
//! package's entries differ by that prefix and by nothing else.
//!
//! ⚠ **A build checks that the package reads back, not that OpenIV installs it.** The manifest is
//! written from the 2.2 shape as `or_title::oiv` knows it, and no OpenIV was available to try it.

use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter, Cursor};
use std::path::Path;

use or_io::gta5::shaders::shader_defs::ParamRemap;
use or_title::oiv::{Add, Package, PackageReader, Writer, ASSEMBLY, CONTENT};

use crate::build::{check_file, moved, uncooked, BuildError, Loss, Verification};
use crate::container::Output;
use crate::contribution::ManifestEntry;
use crate::dlc;
use crate::plan::{Origin, Plan, PlannedFile};
use crate::watch::Watch;

/// The extension of the file a build writes, which is `<resource>.oiv` in the resource's folder.
pub const EXTENSION: &str = "oiv";

/// Where the game looks for the packs it is told of in `dlclist.xml`, which a package installs one into.
pub(crate) const PACKS: &str = "update/x64/dlcpacks";

/// Where a package installs the pack it adds, as a path in the game's tree.
pub(crate) fn pack_root(resource: &str) -> String {
    format!("{PACKS}/{resource}/{}", dlc::PACK)
}

/// The files of the contents in `packs` are the files of a pack, so the package installs them inside the `dlc.rpf` it adds, with the documents that make it a pack.
///
/// ⚠ The package edits no `dlclist.xml`: OpenIV's add command is all this authors, so the plan says which line to add.
fn install_packs<'a>(
    owned: &mut Vec<(&'a str, PlannedFile)>,
    (entries, packs): (&[(&'a str, &ManifestEntry)], &BTreeSet<&str>),
    plan: &mut Plan,
) {
    let (packed, rest): (Vec<_>, Vec<_>) = std::mem::take(owned)
        .into_iter()
        .partition(|(module, _)| packs.contains(module));
    *owned = rest;
    let theirs: Vec<(&str, &ManifestEntry)> = (entries.iter())
        .filter(|(module, _)| packs.contains(module))
        .copied()
        .collect();
    let name = plan.resource.clone();
    let registrations = dlc::register(&packed, &theirs, &[], &name, &mut plan.problems);
    let documents = dlc::documents_of(&name, &registrations);
    let root = pack_root(&name);
    for (module, file) in packed {
        owned.push((
            module,
            PlannedFile {
                path: format!("{root}/{}", file.path),
                origin: file.origin,
            },
        ));
    }
    for (path, text) in [
        ("setup2.xml", documents.setup2),
        ("content.xml", documents.content),
    ] {
        owned.push((
            "",
            PlannedFile {
                path: format!("{root}/{path}"),
                origin: Origin::Document(text),
            },
        ));
    }
    plan.notes.push(dlclist_note(&documents.dlclist_entry));
    plan.registrations = registrations;
    plan.dlclist_entry = Some(documents.dlclist_entry);
}

/// The note that says which line of `dlclist.xml` a package that installs a pack leaves for the person to add.
fn dlclist_note(entry: &str) -> String {
    format!("the package adds the pack and does not edit dlclist.xml: add the line {entry} to it yourself")
}

/// The package of `plan` once the files at `gone` are not in it: the pack it installs registers only what remains, and the manifest adds only what remains.
///
/// ⚠ A pack with nothing left is not installed at all: its two documents, its registrations and the note about its line of `dlclist.xml` go,
/// and the rest of the package is shipped. Refused, with a sentence, only when the package would add nothing.
pub(crate) fn without(plan: &mut Plan, gone: &BTreeSet<String>) -> Result<(), String> {
    let mut dropped = gone.clone();
    if !plan.registrations.is_empty() {
        let prefix = format!("{}/", pack_root(&plan.resource));
        if !dlc::without(plan, gone, &prefix) {
            dropped.extend(dlc::DOCUMENTS.iter().map(|path| format!("{prefix}{path}")));
            plan.files.retain(|f| !dropped.contains(&f.path));
            if let Some(entry) = plan.dlclist_entry.take() {
                plan.notes.retain(|note| *note != dlclist_note(&entry));
            }
        }
    }
    let Some(file) = plan.files.iter_mut().find(|f| f.path == ASSEMBLY) else {
        return Err(format!("the plan holds no {ASSEMBLY}"));
    };
    let Origin::Document(text) = &file.origin else {
        return Err(format!("{ASSEMBLY} is not a document"));
    };
    let (manifest, empty) = crate::left_out::manifest_without(text, &dropped)?;
    if empty {
        return Err(crate::left_out::nothing_left());
    }
    file.origin = Origin::Document(manifest);
    Ok(())
}

/// The package's author, version and description are keys on the output's entry, which the container carries.
pub(crate) fn finish<'a>(
    owned: &mut Vec<(&'a str, PlannedFile)>,
    packed: (&[(&'a str, &ManifestEntry)], &BTreeSet<&str>),
    declared: &Output,
    plan: &mut Plan,
) {
    if !packed.1.is_empty() {
        install_packs(owned, packed, plan);
    }
    let mut package = Package::new(&plan.resource);
    for (key, field) in [
        ("author", &mut package.author),
        ("description", &mut package.description),
    ] {
        match declared.extra.get(key) {
            None => {}
            Some(serde_json::Value::String(text)) => field.clone_from(text),
            Some(v) => plan.problems.push(format!(
                "build `{}` has `{key}` {v}, which is not text",
                declared.id
            )),
        }
    }
    match declared.extra.get("version") {
        None => {}
        Some(v) => match v.as_str().and_then(version) {
            Some((major, minor)) => (package.major, package.minor) = (major, minor),
            None => plan.problems.push(format!(
                "build `{}` has `version` {v}, which is not `major.minor`",
                declared.id
            )),
        },
    }
    if owned
        .iter()
        .any(|(_, file)| file.path.eq_ignore_ascii_case(ASSEMBLY))
    {
        plan.problems.push(format!(
            "`{ASSEMBLY}` is the package's own manifest, so no file of the project can have that name"
        ));
    }
    let mut paths: Vec<&str> = owned.iter().map(|(_, file)| file.path.as_str()).collect();
    paths.sort_unstable();
    package.adds = paths.into_iter().map(Add::of).collect();
    owned.push((
        "",
        PlannedFile {
            path: ASSEMBLY.to_string(),
            origin: Origin::Document(package.to_xml()),
        },
    ));
}

fn version(text: &str) -> Option<(u16, u16)> {
    let (major, minor) = text.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Write the plan as `staging/<resource>.oiv`.
pub(crate) fn write(
    plan: &Plan,
    staging: &Path,
    remap: &ParamRemap,
    watch: &Watch,
) -> Result<Vec<Loss>, BuildError> {
    let io = |what: String| move |source: std::io::Error| BuildError::Io { what, source };
    let refused = |e: or_title::oiv::OivError| BuildError::Refused(vec![e.to_string()]);
    let manifest = plan
        .files
        .iter()
        .find_map(|file| match &file.origin {
            Origin::Document(text) if file.path == ASSEMBLY => Some(text.as_str()),
            _ => None,
        })
        .ok_or_else(|| BuildError::Refused(vec![format!("the plan holds no {ASSEMBLY}")]))?;
    let at = staging.join(format!("{}.{EXTENSION}", plan.resource));
    let out = std::fs::File::create(&at).map_err(io(format!("creating {}", at.display())))?;
    let mut writer = Writer::new(BufWriter::new(out), manifest).map_err(refused)?;
    let mut losses = Vec::new();
    let mut done = 0;
    for file in plan.files.iter().filter(|file| file.path != ASSEMBLY) {
        watch.go_on()?;
        watch.at(&file.path, done, plan.totals.bytes);
        done += file.bytes();
        match &file.origin {
            Origin::Copy { from, bytes } => {
                let source =
                    std::fs::File::open(from).map_err(io(format!("opening {}", from.display())))?;
                let len = source
                    .metadata()
                    .map_err(io(format!("reading {}", from.display())))?
                    .len();
                if len != *bytes {
                    return Err(BuildError::SourceChanged(from.clone()));
                }
                writer
                    .add(&file.path, len, &mut BufReader::new(source))
                    .map_err(refused)?;
            }
            Origin::Retarget { from, bytes } => {
                let (data, loss) = moved(plan, &file.path, from, *bytes, remap)?;
                writer
                    .add(&file.path, data.len() as u64, &mut Cursor::new(data))
                    .map_err(refused)?;
                losses.extend(loss);
            }
            Origin::Document(text) => {
                writer
                    .add(&file.path, text.len() as u64, &mut text.as_bytes())
                    .map_err(refused)?;
            }
            Origin::Generated(bytes) => {
                writer
                    .add(&file.path, bytes.len() as u64, &mut bytes.as_slice())
                    .map_err(refused)?;
            }
            Origin::Cook { .. } => return Err(uncooked(&file.path)),
        }
    }
    let mut out = writer.finish().map_err(refused)?;
    std::io::Write::flush(&mut out).map_err(io(format!("writing {}", at.display())))?;
    watch.all(plan.totals.bytes);
    Ok(losses)
}

/// Read `folder/<resource>.oiv` back against the plan: the manifest, then each file under `content/`.
pub(crate) fn verify(plan: &Plan, folder: &Path) -> Verification {
    let mut found = Verification::default();
    let name = format!("{}.{EXTENSION}", plan.resource);
    let on_disk = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0);
    let opened = std::fs::File::open(folder.join(&name))
        .map_err(|e| e.to_string())
        .and_then(|file| PackageReader::new(BufReader::new(file)).map_err(|e| e.to_string()));
    let mut reader = match opened {
        Ok(reader) => reader,
        Err(why) => {
            found
                .problems
                .push(format!("`{name}` does not open: {why}"));
            return found;
        }
    };
    if on_disk != 1 {
        found.problems.push(format!(
            "the folder holds {on_disk} entries, and a package is `{name}` alone"
        ));
    }

    let mut expected = vec![ASSEMBLY.to_string()];
    for file in &plan.files {
        found.checked += 1;
        if file.path == ASSEMBLY {
            if let Origin::Document(text) = &file.origin {
                if reader.manifest != text.as_bytes() {
                    found
                        .problems
                        .push(format!("`{ASSEMBLY}` differs from what the plan names"));
                }
            }
            continue;
        }
        expected.push(format!("{CONTENT}/{}", file.path));
        match reader.read(&file.path) {
            Ok(bytes) => check_file(&mut found, plan, file, &bytes),
            Err(e) => found.problems.push(format!(
                "`{}` cannot be read from the package: {e}",
                file.path
            )),
        }
    }
    let entries = reader.entries();
    for entry in entries.iter().filter(|entry| !expected.contains(entry)) {
        found
            .problems
            .push(format!("`{entry}` is in the package and not in the plan"));
    }
    for add in &reader.package.adds {
        if !entries.contains(&format!("{CONTENT}/{}", add.source)) {
            found.problems.push(format!(
                "the manifest adds `{}`, which the package does not hold",
                add.source
            ));
        }
    }
    found
}
