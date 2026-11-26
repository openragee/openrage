//! `openrage-cli project check-server` and `project check-build`: the resources of a server read together, and what of them a build is about to write again.
//!
//! ⚠ Both write nothing and answer with their exit code: 0 where no rule found a warning or an error, 1 where one did. A note never
//! fails them: the same bytes in two resources and a file of the game's name are said, and are no fault.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::ExitCode;

use or_project::resource::manifest;
use or_project::server::config::Config;
use or_project::server::{
    concerning, placement, scan, Declares, Disk, Finding, GameFiles, Report, Scan,
};
use or_title::vfs::EntryFacts;
use serde_json::json;

use crate::project::{parse, planned, root_of, Failure, BUILD};
use crate::wardrobe_fix::without;
use Failure::{Run, Usage};

/// The name of every file an install holds, in lower case.
#[derive(Default)]
struct Leaves(HashSet<String>);

impl Leaves {
    fn keep(&mut self, facts: &EntryFacts) {
        self.0.insert(facts.leaf.to_lowercase());
    }
}

impl GameFiles for Leaves {
    fn holds_file(&self, name: &str) -> bool {
        self.0.contains(name)
    }
}

/// The resources under `folder`, or why it is not the folder of a server's resources.
///
/// ⚠ A folder with no resource under it is refused: read, it would report nothing and the check would read as passed.
fn scanned(folder: &Path) -> Result<Scan, String> {
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    if let Some(manifest) = manifest::NAMES
        .iter()
        .find(|name| folder.join(name).is_file())
    {
        return Err(format!(
            "{} holds a {manifest}, so it is one resource and not the folder of a server's resources: check-resource reads one",
            folder.display()
        ));
    }
    let read = scan(folder);
    if read.resources.is_empty() && read.unlisted.is_empty() {
        let inner = match folder.join("resources").is_dir() {
            true => "; its folder `resources` may be the one a server reads",
            false => "",
        };
        return Err(format!(
            "{} holds no resource: no folder in it, through its categories in brackets, has a manifest{inner}",
            folder.display()
        ));
    }
    Ok(read)
}

/// What the rules find in the server's resources `folder`, read with the configuration `cfg` and the install at `game` where one is given.
pub fn check_server_folder(
    folder: &Path,
    cfg: Option<&Path>,
    game: Option<&Path>,
    keys: Option<&str>,
) -> Result<Report, String> {
    if let Some(file) = cfg.filter(|file| !file.is_file()) {
        return Err(format!("{} is not a file", file.display()));
    }
    let read = scanned(folder)?;
    let config = match cfg {
        Some(file) => Some(Config::read_beside(file, &Disk, &read).map_err(|e| e.to_string())?),
        None => None,
    };
    let leaves = match game {
        Some(root) => {
            let mount = crate::keys::mount_all(root, keys.map(Path::new))?;
            let mut leaves = Leaves::default();
            mount.for_each_entry(|facts| leaves.keep(facts));
            Some(leaves)
        }
        None => None,
    };
    let game = leaves.as_ref().map(|held| held as &dyn GameFiles);
    Ok(Report::of(&read, config.as_ref(), game))
}

fn code_of(failed: bool) -> ExitCode {
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// `project check-server`: what the rules find across a server's resources, as text or as JSON.
pub fn check_server(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &["--json"]);
    let (positional, flags) = parse(&rest, &["--cfg", "--game", "--keys"])?;
    let [folder] = positional.as_slice() else {
        return Err(Usage("expected one <resources folder>".into()));
    };
    let report = check_server_folder(
        Path::new(folder),
        flags.get("--cfg").map(Path::new),
        flags.get("--game").map(Path::new),
        flags.get("--keys").map(String::as_str),
    )
    .map_err(Run)?;
    if bare[0] {
        let document = serde_json::to_value(&report).map_err(|e| Run(e.to_string()))?;
        crate::wardrobe_fix::print_json(vec![document]);
    } else {
        for line in report.lines() {
            println!("{line}");
        }
    }
    Ok(code_of(report.fails()))
}

/// The flag that names the folder builds are written into.
const OUT: &str = "--out";

/// What one build of a project is about to write that the server already holds.
pub struct BuildConcerns {
    /// The build's id.
    pub build: String,
    /// The resource it writes.
    pub resource: String,
    pub findings: Vec<Finding>,
    /// Why the plan cannot be built, which is why the findings are not all there are.
    pub problems: Vec<String>,
    /// The type files it ships whose archetypes are not known, each with why: no resource was asked about them.
    pub type_files_unread: Vec<(String, String)>,
}

impl BuildConcerns {
    /// Whether the check answers with a failing exit code: a finding, or a plan that says what it cannot build.
    pub fn fails(&self) -> bool {
        !self.findings.is_empty() || !self.problems.is_empty()
    }
}

/// The bytes the metadata source folder `source` cooks to.
fn cooked(source: &Path) -> Result<Vec<u8>, String> {
    let said = |found: Vec<or_source::Problem>| {
        let said: Vec<String> = found.iter().map(ToString::to_string).collect();
        said.join("; ")
    };
    let opened = or_source::asset::Source::open(source).map_err(|p| p.to_string())?;
    Ok(or_source::metadata::compile(&opened).map_err(said)?.bytes)
}

/// What a check of the builds of a project against a server's resources found.
pub struct BuildCheck {
    /// The resources read of the server.
    pub resources: usize,
    /// Whether every resource was read whole; where not, the findings are not all there are.
    pub whole: bool,
    /// One for each resource the build writes: several where it divides.
    pub builds: Vec<BuildConcerns>,
}

/// Ask of each resource the build `build` of the project at `project` writes, into `out` or the project's build folder, what the server's resources `server` already hold of it.
pub fn check_build_against(
    project: &Path,
    build: Option<&str>,
    out: Option<&Path>,
    server: &Path,
) -> Result<BuildCheck, String> {
    let positional = [project.to_string_lossy().into_owned()];
    let out = out.map(|folder| (OUT.to_string(), folder.to_string_lossy().into_owned()));
    let flags = (build
        .map(|id| (BUILD.to_string(), id.to_string()))
        .into_iter())
    .chain(out)
    .collect();
    concerns(&positional, &flags, server).map_err(|failure| match failure {
        Usage(why) | Run(why) => why,
    })
}

fn concerns(
    positional: &[String],
    flags: &HashMap<String, String>,
    server: &Path,
) -> Result<BuildCheck, Failure> {
    if !server.is_dir() {
        return Err(Run(format!("{} is not a folder", server.display())));
    }
    let (_, plans) = planned(positional, flags)?;
    let root = root_of(&positional[0], flags);
    let read = scanned(server).map_err(Run)?;
    let asked = |plan: &or_project::Plan| {
        let declares = Declares::of(plan, &cooked);
        let mut findings = concerning(plan, &read, &declares);
        findings.extend(placement(plan, &root.join(&plan.resource), &read));
        BuildConcerns {
            build: plan.output.clone(),
            resource: plan.resource.clone(),
            findings,
            problems: plan.problems.clone(),
            type_files_unread: declares.unread,
        }
    };
    Ok(BuildCheck {
        resources: read.resources.len(),
        whole: read.is_whole(),
        builds: plans.iter().map(asked).collect(),
    })
}

/// `project check-build`: what a build is about to write that another resource of a server already streams or declares.
pub fn check_build(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &["--json"]);
    let (positional, flags) = parse(&rest, &[BUILD, OUT, "--server"])?;
    let Some(server) = flags.get("--server") else {
        return Err(Usage(
            "expected --server <resources folder>: the folder of the server's resources".into(),
        ));
    };
    let check = concerns(&positional, &flags, Path::new(server))?;
    let failed = check.builds.iter().any(BuildConcerns::fails);
    if bare[0] {
        let documents = (check.builds.iter())
            .map(|b| {
                json!({
                    "build": b.build,
                    "resource": b.resource,
                    "server": server,
                    "resources": check.resources,
                    "whole": check.whole,
                    "problems": b.problems,
                    "type_files_unread": (b.type_files_unread.iter())
                        .map(|(path, why)| json!({ "path": path, "why": why }))
                        .collect::<Vec<_>>(),
                    "findings": b.findings,
                })
            })
            .collect();
        crate::wardrobe_fix::print_json(documents);
        return Ok(code_of(failed));
    }
    for b in &check.builds {
        println!(
            "build {}: the resource {}, against the {} resource(s) of {server}",
            b.build, b.resource, check.resources
        );
        for finding in &b.findings {
            println!("{}", finding.line());
        }
        for problem in &b.problems {
            println!("  problem: {problem}");
        }
        for (path, why) in &b.type_files_unread {
            println!(
                "  the archetypes of `{path}` are not known, so no resource was asked whether it declares them: {why}"
            );
        }
        // ⚠ The all-clear is said only of a plan read whole: what a plan with a problem writes is not all known.
        if !b.problems.is_empty() {
            println!(
                "  the plan cannot be built as it stands, so what it writes and declares is not all known: `project plan` says the same"
            );
        } else if b.findings.is_empty() && b.type_files_unread.is_empty() {
            println!(
                "  no other resource streams a file it writes or declares an archetype it declares, and it is written where a server looks or outside the server's folder"
            );
        } else if b.findings.is_empty() {
            println!(
                "  no other resource streams a file it writes or declares an archetype of the type files that were read, and it is written where a server looks or outside the server's folder"
            );
        }
    }
    if !check.whole {
        println!("A folder, a type file or a map of the server was not read, so this is not all it holds: check-server names them.");
    }
    println!("This reads the files as a server finds them. It cannot see which copy of a file a server loads.");
    Ok(code_of(failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::RpfEntryKind;

    #[test]
    fn an_install_is_asked_by_the_name_of_a_file_whatever_its_type_and_its_case() {
        let mut leaves = Leaves::default();
        for leaf in ["HEI_Dock.ymap", "nodes12.ynd", "bench.ydr"] {
            leaves.keep(&EntryFacts {
                logical: format!("x64a.rpf/{leaf}"),
                container: "x64a.rpf".into(),
                leaf: leaf.into(),
                kind: RpfEntryKind::Resource,
                stored_size: 32,
                size: 32,
                compressed: false,
                paging: Some((0x8000_0000, 0x1000_0000)),
            });
        }
        for held in ["hei_dock.ymap", "nodes12.ynd", "bench.ydr"] {
            assert!(leaves.holds_file(held), "{held}");
        }
        for absent in ["bench.yft", "bench", "hei_dock"] {
            assert!(!leaves.holds_file(absent), "{absent}");
        }
    }
}
