//! Sources that are still the game's own file and so are not shipped, and the documents of a pack or a package that must stop naming them.
//!
//! A FiveM resource lists nothing of `stream/`, so leaving a file out of it changes no document.
//! A singleplayer DLC registers each file in `content.xml` and an `.oiv` names each in its manifest,
//! so both are written again from what remains, and the result is what the same project would have
//! planned had it never held the source.

use std::collections::BTreeSet;
use std::io::BufReader;
use std::path::Path;

use or_io::container::rpf::RpfArchive;
use or_title::oiv::{Package, PackageReader, ASSEMBLY, CONTENT};

use crate::container::OutputKind;
use crate::plan::{Origin, Plan, PlannedFile, Totals};
use crate::{dlc, oiv};

/// The sentence every build refuses with when nothing is left to ship.
const NOTHING_LEFT: &str = "every source of this build is still the game's own file, so it would ship nothing: change one, or turn on Ship a source that is still the game's own file";

/// Whether `file` is a source taken from the game, which a build leaves out while it is still the game's file.
pub(crate) fn is_the_games(file: &PlannedFile) -> bool {
    matches!(&file.origin, Origin::Cook { game, .. } if !game.is_empty())
}

/// What the plan tells a person about the sources it may leave out, none when it names none or the build ships them all.
pub(crate) fn note(plan: &Plan) -> Option<String> {
    if plan.ship_unchanged {
        return None;
    }
    let count = plan.files.iter().filter(|f| is_the_games(f)).count();
    match count {
        0 => None,
        1 => Some("1 source taken from the game is left out of the build if it is still the game's own file: turn on Ship a source that is still the game's own file to ship it".to_string()),
        n => Some(format!("{n} sources taken from the game are left out of the build if they are still the game's own file: turn on Ship a source that is still the game's own file to ship them")),
    }
}

/// `plan` once the files at `gone` are not in it: the registrations of a pack and the manifest of a package name only what remains.
///
/// ⚠ A FiveM resource is returned as it is, since its manifest does not list the files of `stream/`.
pub(crate) fn rewrite(plan: &mut Plan, gone: &BTreeSet<String>) -> Result<(), String> {
    match plan.kind {
        OutputKind::SingleplayerDlc => {
            if !dlc::without(plan, gone, "") {
                return Err(nothing_left());
            }
        }
        OutputKind::OivPackage => oiv::without(plan, gone)?,
        _ => {}
    }
    Ok(())
}

/// `plan` without the files at `gone`, its documents written again and its totals counted again.
pub(crate) fn without(plan: &Plan, gone: &BTreeSet<String>) -> Result<Plan, String> {
    let mut kept = plan.clone();
    kept.files.retain(|f| !gone.contains(&f.path));
    rewrite(&mut kept, gone)?;
    count(&mut kept);
    Ok(kept)
}

/// The totals of the files the plan holds.
pub(crate) fn count(plan: &mut Plan) {
    plan.totals = Totals {
        files: plan.files.len() as u64,
        bytes: plan.files.iter().map(PlannedFile::bytes).sum(),
    };
}

/// The error for a pack or a package every file of which was left out.
pub(crate) fn nothing_left() -> String {
    NOTHING_LEFT.to_string()
}

/// The plan as a build left it in `folder`: without the sources of the game that the build did not write.
///
/// `plan` itself when the build ships them all or every one is there, and also when the folder cannot be read,
/// since verifying it reports that.
pub(crate) fn as_built<'a>(plan: &'a Plan, folder: &Path) -> std::borrow::Cow<'a, Plan> {
    use std::borrow::Cow;

    if plan.ship_unchanged || !plan.files.iter().any(is_the_games) {
        return Cow::Borrowed(plan);
    }
    let Some(present) = present_in(plan, folder) else {
        return Cow::Borrowed(plan);
    };
    let gone: BTreeSet<String> = plan
        .files
        .iter()
        .filter(|f| is_the_games(f) && !present.contains(&f.path.to_lowercase()))
        .map(|f| f.path.clone())
        .collect();
    if gone.is_empty() {
        return Cow::Borrowed(plan);
    }
    match without(plan, &gone) {
        Ok(kept) => Cow::Owned(kept),
        Err(_) => Cow::Borrowed(plan),
    }
}

/// The path of every file a pack or a package holds, in lower case, nested archives included; none for a kind this does not read.
fn present_in(plan: &Plan, folder: &Path) -> Option<BTreeSet<String>> {
    match plan.kind {
        OutputKind::SingleplayerDlc => {
            let archive = RpfArchive::open(folder.join(dlc::PACK)).ok()?;
            let mut found = BTreeSet::new();
            files_of(&archive, "", &mut found);
            Some(found)
        }
        OutputKind::OivPackage => {
            let name = format!("{}.{}", plan.resource, oiv::EXTENSION);
            let file = std::fs::File::open(folder.join(name)).ok()?;
            let reader = PackageReader::new(BufReader::new(file)).ok()?;
            let prefix = format!("{CONTENT}/");
            Some(
                reader
                    .entries()
                    .iter()
                    .filter_map(|entry| entry.strip_prefix(&prefix))
                    .map(str::to_lowercase)
                    .collect(),
            )
        }
        _ => None,
    }
}

/// Every file of `archive` under the path `held`, lower-cased, going into the archives it holds.
fn files_of(archive: &RpfArchive, held: &str, found: &mut BTreeSet<String>) {
    for node in archive.nodes.iter().filter(|node| !node.is_dir()) {
        let path = node
            .path()
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_lowercase();
        let whole = if held.is_empty() {
            path
        } else {
            format!("{held}/{path}")
        };
        if whole.ends_with(".rpf") {
            let inner = archive
                .read_entry(node.entry_index())
                .ok()
                .and_then(|bytes| RpfArchive::from_bytes(bytes).ok());
            if let Some(inner) = inner {
                files_of(&inner, &whole, found);
            }
        }
        found.insert(whole);
    }
}

/// The manifest of a package without the adds of the files at `gone`.
pub(crate) fn manifest_without(
    text: &str,
    gone: &BTreeSet<String>,
) -> Result<(String, bool), String> {
    let mut package =
        Package::parse(text.as_bytes()).map_err(|e| format!("{ASSEMBLY} does not read: {e}"))?;
    package.adds.retain(|add| !gone.contains(&add.source));
    Ok((package.to_xml(), package.adds.is_empty()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{plan_with, Container, Contribution, ManifestEntry, Module, Output};

    const SHOP: &str = "common/data/u_shop.meta";

    fn document(path: &str, text: &str) -> PlannedFile {
        PlannedFile {
            path: path.to_string(),
            origin: Origin::Document(text.to_string()),
        }
    }

    /// A package that adds a readme and installs a pack whose only file is `SHOP`.
    fn package_with_a_pack() -> Plan {
        let mut files = Contribution::of("files");
        files.files = vec![document("readme.txt", "read me")];
        let mut pack = Contribution::of("wardrobe");
        pack.pack = true;
        pack.files = vec![document(SHOP, "<shop/>")];
        pack.manifest = vec![ManifestEntry {
            key: "data_file".into(),
            values: vec!["SHOP_PED_APPAREL_META_FILE".into(), SHOP.into()],
        }];
        let container = Container {
            modules: vec![
                Module::files("files"),
                Module::clothing("wardrobe", "wardrobe"),
            ],
            outputs: vec![Output::of(
                OutputKind::OivPackage,
                "out",
                "mypack",
                &["files", "wardrobe"],
            )],
        };
        let plan = plan_with(&container, "out", Path::new("p"), &[files, pack]).unwrap();
        assert!(plan.is_buildable(), "{:?}", plan.problems);
        plan
    }

    fn adds_of(plan: &Plan) -> Vec<String> {
        let manifest = plan.files.iter().find(|f| f.path == ASSEMBLY).unwrap();
        let Origin::Document(text) = &manifest.origin else {
            panic!("the manifest is a document");
        };
        let package = Package::parse(text.as_bytes()).expect("the manifest reads");
        package.adds.into_iter().map(|add| add.source).collect()
    }

    #[test]
    fn a_package_whose_pack_has_nothing_left_drops_the_pack_and_keeps_the_rest() {
        let plan = package_with_a_pack();
        assert!(plan.dlclist_entry.is_some() && !plan.registrations.is_empty());
        let in_pack = format!("{}/{SHOP}", oiv::pack_root("mypack"));

        let kept = without(&plan, &BTreeSet::from([in_pack])).expect("the readme is still added");

        assert!(kept.registrations.is_empty());
        assert_eq!(kept.dlclist_entry, None);
        assert!(
            kept.notes.iter().all(|note| !note.contains("dlclist.xml")),
            "the note about the pack's line of dlclist.xml is still there: {:?}",
            kept.notes
        );
        let mut paths: Vec<&str> = kept.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, [ASSEMBLY, "readme.txt"]);
        assert_eq!(adds_of(&kept), ["readme.txt"]);
        assert_eq!(kept.totals.files, 2);
    }

    #[test]
    fn a_name_a_pack_asks_a_type_file_by_goes_with_that_type_file_and_stays_with_another() {
        let in_archive = |name: &str| format!("x64/levels/props.rpf/{name}.ytyp");
        let mut props = Contribution::of("props");
        for name in ["a", "b"] {
            props.files.push(PlannedFile {
                path: in_archive(name),
                origin: Origin::Generated(name.as_bytes().to_vec()),
            });
            props.manifest.push(ManifestEntry {
                key: "data_file".into(),
                values: vec!["DLC_ITYP_REQUEST".into(), format!("x64/levels/{name}.ityp")],
            });
        }
        let container = Container {
            modules: vec![Module::files("props")],
            outputs: vec![Output::of(
                OutputKind::SingleplayerDlc,
                "out",
                "mypack",
                &["props"],
            )],
        };
        let plan = plan_with(&container, "out", Path::new("p"), &[props]).unwrap();
        let registered = |plan: &Plan| -> Vec<String> {
            plan.registrations.iter().map(|r| r.path.clone()).collect()
        };
        assert_eq!(
            registered(&plan),
            [
                "x64/levels/a.ityp",
                "x64/levels/b.ityp",
                "x64/levels/props.rpf"
            ]
        );

        let kept = without(&plan, &BTreeSet::from([in_archive("a")])).expect("b is still shipped");
        assert_eq!(
            registered(&kept),
            ["x64/levels/b.ityp", "x64/levels/props.rpf"],
            "the name of the type file left out goes, and the other's stays"
        );
    }

    #[test]
    fn a_package_that_would_add_nothing_is_refused() {
        let plan = package_with_a_pack();
        let gone: BTreeSet<String> = plan
            .files
            .iter()
            .map(|f| f.path.clone())
            .filter(|path| path != ASSEMBLY)
            .collect();

        assert_eq!(without(&plan, &gone).unwrap_err(), nothing_left());
    }
}
