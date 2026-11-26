//! `openrage-cli maps roads`: the path regions a folder holds as sources, with their counts and what the rules say of them
//! (`or_maps::roads`), and a list of operations applied to them.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use or_maps::roads::operation::{apply_all, Error, Operations};
use or_maps::roads::rules::{check, Severity};
use or_maps::roads::{read, Listed};
use serde_json::{json, Value};

use crate::project::{parse, Failure};
use Failure::{Run, Usage};

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let (said, clean) = report(args)?;
    print!("{said}");
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What `maps roads` prints for `args`, and whether no finding is an error.
pub(crate) fn report(args: &[String]) -> Result<(String, bool), Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let rest: Vec<String> = (args.iter())
        .filter(|a| *a != "--json" && *a != "--dry-run")
        .cloned()
        .collect();
    let (positional, flags) = parse(&rest, &["--apply"])?;
    let [folder] = positional.as_slice() else {
        return Err(Usage(
            "expected one <folder> that holds path region sources".into(),
        ));
    };
    match flags.get("--apply") {
        Some(_) if json_out => Err(Usage(
            "--json is the form of the listing, and --apply prints what each operation changed"
                .into(),
        )),
        Some(list) => applied(Path::new(folder), list, dry_run).map(|said| (said, true)),
        None if dry_run => Err(Usage("--dry-run goes with --apply".into())),
        None => listed(folder, json_out),
    }
}

/// One region of the listing as data: its counts, or why its document does not read.
fn region_json(listed: &Listed) -> Value {
    match &listed.stated {
        Ok(stated) => json!({
            "source": listed.source,
            "region": stated.region,
            "nodes": stated.nodes.len(),
            "car_nodes": stated.num_nodes_car_nodes,
            "ped_nodes": stated.num_nodes_ped_nodes,
            "links": stated.nodes.iter().map(|node| node.links.len()).sum::<usize>(),
            "junctions": stated.virtual_junctions,
        }),
        Err(why) => json!({ "source": listed.source, "error": why }),
    }
}

/// The line of the listing for one region, from its data.
fn region_said(region: &Value) -> String {
    let source = region["source"].as_str().unwrap_or_default();
    if let Some(why) = region["error"].as_str() {
        return format!("  {source}: does not read: {why}");
    }
    let which = match region["region"].as_u64() {
        Some(region) => format!("region {region}"),
        None => "no node, so no region stated".to_string(),
    };
    format!(
        "  {source}: {which}, {} node(s) ({} vehicle, {} pedestrian), {} link(s), {} junction(s)",
        region["nodes"],
        region["car_nodes"],
        region["ped_nodes"],
        region["links"],
        region["junctions"]
    )
}

/// The regions the sources of `folder` state, with their counts and what the rules say.
fn listed(folder: &str, json_out: bool) -> Result<(String, bool), Failure> {
    let set = read(Path::new(folder)).map_err(|e| Run(e.to_string()))?;
    // No ground is at hand here: the rules that need it are listed as not checked.
    let found = check(&set, None);
    let regions: Vec<Value> = set.iter().map(region_json).collect();
    let clean = !found.blocks();
    if json_out {
        let whole = json!({
            "folder": folder,
            "regions": regions,
            "findings": found.findings,
            "skipped": found.skipped,
        });
        let mut text = serde_json::to_string_pretty(&whole).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
        return Ok((text, clean));
    }
    let count = |severity| {
        (found.findings.iter())
            .filter(|f| f.severity == severity)
            .count()
    };
    let mut text = String::new();
    let _ = writeln!(
        text,
        "folder {folder}: {} region(s), {} error(s), {} warning(s), {} note(s)",
        set.len(),
        count(Severity::Error),
        count(Severity::Warning),
        count(Severity::Info),
    );
    for region in &regions {
        let _ = writeln!(text, "{}", region_said(region));
    }
    for f in &found.findings {
        let word = match f.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "note",
        };
        let _ = writeln!(text, "  {word:<7} {:<20} {}", f.code, f.message);
    }
    for skipped in &found.skipped {
        let _ = writeln!(text, "  not checked {:<16} {}", skipped.code, skipped.why);
    }
    Ok((text, clean))
}

/// Do the operations of the list `list` to the sources of `folder`, all of them or none, and say what each changed.
fn applied(folder: &Path, list: &str, dry_run: bool) -> Result<String, Failure> {
    let bytes = std::fs::read(list).map_err(|e| Run(format!("{list}: {e}")))?;
    let operations = Operations::from_json(&bytes, list)
        .map_err(|e| Run(e.to_string()))?
        .operations;
    let applied = apply_all(folder, &operations, dry_run).map_err(|e| match e {
        // The one failure after which a source may hold what the list wrote: it says which.
        Error::Write(_) => Run(e.to_string()),
        refused => Run(format!("{refused}; no source was changed")),
    })?;

    let mut text = String::new();
    // Each document with what it held before the list and what it holds after: an operation taken back later in the list changes nothing.
    let mut files: Vec<(String, &[u8], &[u8])> = Vec::new();
    for (at, (operation, done)) in operations.iter().zip(&applied).enumerate() {
        let said = match (done.edited.iter().any(|e| e.changed()), done.added) {
            (false, _) => ": no change".to_string(),
            (true, Some(node)) => format!(": added node {node}"),
            (true, None) => String::new(),
        };
        let _ = writeln!(
            text,
            "  {}. {} of {}{said}",
            at + 1,
            operation.name(),
            operation.target()
        );
        for note in &done.notes {
            let _ = writeln!(text, "     note: {note}");
        }
        for edited in &done.edited {
            let file = (edited.file.strip_prefix(folder))
                .unwrap_or(&edited.file)
                .to_string_lossy()
                .replace('\\', "/");
            match files.iter_mut().find(|(held, ..)| *held == file) {
                Some((.., after)) => *after = &edited.after,
                None => files.push((file, &edited.before, &edited.after)),
            }
        }
    }
    files.sort();
    let changed: Vec<&str> = (files.iter())
        .filter(|(_, before, after)| before != after)
        .map(|(file, ..)| file.as_str())
        .collect();
    let verb = if dry_run { "would change" } else { "changed" };
    for file in &changed {
        let _ = writeln!(text, "{verb} {file}");
    }
    let _ = writeln!(
        text,
        "{} operation(s) {verb} {} source(s){}",
        operations.len(),
        changed.len(),
        if dry_run { "; nothing was written" } else { "" }
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::decl::File;
    use or_io::gta5::{CNodeAddress, CPathNode, CPathNodeLink, CPathRegion};
    use or_maps::roads::operation::Operation;
    use or_maps::roads::{Address, Word};
    use or_source::textures::Extraction;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Region `id` built by hand as the source `nodes<id>` of `folder`: vehicle nodes a hundred apart, each with the links given.
    fn region(folder: &Path, id: u16, links: &[&[(u16, u16)]]) {
        let mut built = CPathRegion {
            num_nodes_car_nodes: links.len() as i32,
            ..Default::default()
        };
        for (index, own) in links.iter().enumerate() {
            let mut node = CPathNode {
                address: CNodeAddress {
                    region: id,
                    index: index as u16,
                    ..Default::default()
                },
                coors_x: 100 * index as i16,
                coors_y: id as i16,
                ..Default::default()
            };
            node.set_num_links(own.len()).unwrap();
            built.nodes.push(node);
            for (region, index) in *own {
                built.links.push(CPathNodeLink {
                    other_node: CNodeAddress {
                        region: *region,
                        index: *index,
                        ..Default::default()
                    },
                    as_integer1: 0x11,
                    ..Default::default()
                });
            }
        }
        let how = Extraction {
            from_game: false,
            virtual_path: None,
        };
        let file = built.to_bytes().unwrap();
        or_source::paths::extract(&file, &format!("nodes{id}"), folder, &how).unwrap();
    }

    /// A folder of two regions: 7, whose node 2 nothing reaches and whose node 0 links to a region that is not there, and 8.
    fn folder(dir: &Path) -> PathBuf {
        let folder = dir.join("roads");
        std::fs::create_dir_all(&folder).unwrap();
        region(&folder, 7, &[&[(7, 1), (9, 3)], &[(7, 0), (8, 0)], &[]]);
        region(&folder, 8, &[&[(8, 1)], &[(8, 0)]]);
        folder
    }

    fn said(args: &[&str]) -> Result<(String, bool), Failure> {
        report(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut found = BTreeMap::new();
        for (_, dir) in or_maps::roads::sources(folder).unwrap() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                found.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
        found
    }

    #[test]
    fn the_listing_as_json_lists_what_its_text_lists() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder(dir.path());
        let path = folder.to_str().unwrap();
        let (text, clean) = said(&[path]).unwrap_or_else(|_| panic!("the folder lists"));
        let (data, clean_as_data) =
            said(&[path, "--json"]).unwrap_or_else(|_| panic!("the folder lists as data"));
        assert!(clean && clean_as_data, "{text}");
        let json: Value = serde_json::from_str(&data).unwrap();

        let regions = json["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 2);
        assert_eq!(
            regions[0],
            json!({
                "source": "nodes7.paths.src", "region": 7, "nodes": 3, "car_nodes": 3, "ped_nodes": 0,
                "links": 4, "junctions": 0,
            })
        );
        assert!(
            text.contains(
                "  nodes7.paths.src: region 7, 3 node(s) (3 vehicle, 0 pedestrian), 4 link(s), 0 junction(s)\n"
            ),
            "{text}"
        );
        let findings = json["findings"].as_array().unwrap();
        let codes: Vec<&str> = findings
            .iter()
            .map(|f| f["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            codes,
            ["link-region-absent", "link-one-way", "node-unreached"],
            "{text}"
        );
        let skipped = json["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 3, "{text}");

        // Each line of the text is a line of the data, and the text holds no line the data does not.
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            format!("folder {path}: 2 region(s), 0 error(s), 2 warning(s), 1 note(s)")
        );
        let mut expected = vec![lines[0].to_string()];
        expected.extend(regions.iter().map(region_said));
        for f in findings {
            let word = match f["severity"].as_str().unwrap() {
                "error" => "error",
                "warning" => "warning",
                _ => "note",
            };
            let line = format!(
                "  {word:<7} {:<20} {}",
                f["code"].as_str().unwrap(),
                f["message"].as_str().unwrap()
            );
            // The place of a finding is in its message.
            if let Some(node) = f["place"]["node"].as_str() {
                assert!(line.contains(node), "{line}");
            }
            expected.push(line);
        }
        for s in skipped {
            expected.push(format!(
                "  not checked {:<16} {}",
                s["code"].as_str().unwrap(),
                s["why"].as_str().unwrap()
            ));
        }
        assert_eq!(lines, expected);
    }

    #[test]
    fn a_finding_that_is_an_error_fails_the_command_and_a_document_that_does_not_read_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder(dir.path());
        std::fs::write(folder.join("nodes8.paths.src/paths.json"), "[]").unwrap();
        let path = folder.to_str().unwrap();
        let (text, clean) = said(&[path]).unwrap_or_else(|_| panic!("the folder lists"));
        assert!(!clean, "{text}");
        assert!(
            text.contains("  nodes8.paths.src: does not read: paths.json: "),
            "{text}"
        );
        assert!(text.contains("  error   region-unreadable"), "{text}");
        let (data, clean) = said(&[path, "--json"]).unwrap_or_else(|_| panic!("lists as data"));
        assert!(!clean);
        // The verb of the family reaches it, and exits as it reports.
        let run = |args: &[&str]| {
            let mut all = vec!["maps".to_string(), "roads".to_string()];
            all.extend(args.iter().map(|a| a.to_string()));
            crate::run(&all)
        };
        assert_eq!(run(&[path]), ExitCode::FAILURE);
        std::fs::remove_dir_all(folder.join("nodes8.paths.src")).unwrap();
        assert_eq!(run(&[path]), ExitCode::SUCCESS);
        assert_eq!(
            run(&[]),
            ExitCode::from(2),
            "no folder named is a usage error"
        );
        let json: Value = serde_json::from_str(&data).unwrap();
        assert!(json["regions"][1]["error"].is_string(), "{data}");
        assert_eq!(json["findings"][0]["code"], "region-unreadable");
    }

    #[test]
    fn a_list_of_operations_is_applied_whole_and_a_dry_run_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder(dir.path());
        let path = folder.to_str().unwrap();
        let before = files(&folder);
        let list = dir.path().join("operations.json");
        let write = |operations: Vec<Operation>| {
            std::fs::write(&list, Operations::new(operations).to_json().unwrap()).unwrap();
        };
        let linked = Operation::AddLink {
            node: Address::new(7, 2),
            other_node: Address::new(8, 1),
            as_integer1: Word(0x21),
            link: None,
            back_as_integer1: Some(Word(0x22)),
            back_link: None,
        };
        let added = Operation::AddNode {
            region: 8,
            ped: true,
            coors_x: 5,
            coors_y: 6,
            coors_z: 7,
            linked_to: Address::new(8, 0),
            as_integer1: Word(0x31),
            back_as_integer1: Word(0x32),
        };
        write(vec![linked.clone(), added.clone()]);

        let (dry, _) = said(&[path, "--apply", list.to_str().unwrap(), "--dry-run"])
            .unwrap_or_else(|_| panic!("the dry run runs"));
        assert!(files(&folder) == before, "a dry run writes nothing");
        assert_eq!(
            dry,
            "  1. add_link of node 7:2\n  2. add_node of region 8: added node 8:2\n\
             would change nodes7.paths.src/paths.json\nwould change nodes8.paths.src/paths.json\n\
             2 operation(s) would change 2 source(s); nothing was written\n"
        );
        let (wet, _) = said(&[path, "--apply", list.to_str().unwrap()])
            .unwrap_or_else(|_| panic!("the list applies"));
        assert_eq!(
            wet,
            dry.replace("would change", "changed")
                .replace("; nothing was written", "")
        );
        let expected = tempfile::tempdir().unwrap();
        let again = self::folder(expected.path());
        apply_all(&again, &[linked, added], false).unwrap();
        // The documents, since each source's `asset.json` states an id of its own.
        let documents = |folder: &Path| -> Vec<Vec<u8>> {
            (files(folder).into_iter())
                .filter(|(path, _)| path.ends_with("paths.json"))
                .map(|(_, bytes)| bytes)
                .collect()
        };
        assert_eq!(documents(&folder).len(), 2);
        assert!(documents(&folder) == documents(&again));
        // The node the list linked is reached now.
        let (text, _) = said(&[path]).unwrap_or_else(|_| panic!("the folder lists"));
        assert!(!text.contains("node-unreached"), "{text}");

        // A list with a refused operation changes nothing and names its place.
        let after = files(&folder);
        write(vec![
            Operation::SetLink {
                node: Address::new(7, 0),
                link: 0,
                as_integer1: Word(0x99),
            },
            Operation::DisableNode {
                node: Address::new(7, 0),
            },
        ]);
        let Err(Run(why)) = said(&[path, "--apply", list.to_str().unwrap()]) else {
            panic!("the list is refused");
        };
        assert!(
            why.starts_with(
                "operation 2 of 2, disable_node of node 7:0: the file states no such thing"
            ),
            "{why}"
        );
        assert!(why.ends_with("; no source was changed"), "{why}");
        assert!(files(&folder) == after);

        // The flags of one form are refused with the other.
        let usage = |args: &[&str]| matches!(said(args), Err(Usage(_)));
        assert!(usage(&[path, "--dry-run"]));
        assert!(usage(&[path, "--apply", list.to_str().unwrap(), "--json"]));
        assert!(usage(&[]));
    }

    #[test]
    fn an_operation_that_changes_nothing_and_a_note_are_said_and_a_region_with_no_node_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder(dir.path());
        let path = folder.to_str().unwrap();
        let list = dir.path().join("operations.json");
        let write = |operations: Vec<Operation>| {
            std::fs::write(&list, Operations::new(operations).to_json().unwrap()).unwrap();
        };
        write(vec![
            // The word the link holds, stated again.
            Operation::SetLink {
                node: Address::new(7, 0),
                link: 0,
                as_integer1: Word(0x11),
            },
            Operation::AddLink {
                node: Address::new(7, 2),
                other_node: Address::new(7, 0),
                as_integer1: Word(0x41),
                link: None,
                back_as_integer1: None,
                back_link: None,
            },
        ]);
        let (dry, _) = said(&[path, "--apply", list.to_str().unwrap(), "--dry-run"])
            .unwrap_or_else(|_| panic!("the dry run runs"));
        assert_eq!(
            dry,
            "  1. set_link of node 7:0: no change\n  2. add_link of node 7:2\n     \
             note: the link is stated one way: node 7:0 states no link back to 7:2\n\
             would change nodes7.paths.src/paths.json\n\
             2 operation(s) would change 1 source(s); nothing was written\n"
        );

        // A region whose one node is taken out states no address, so no region.
        region(&folder, 9, &[&[]]);
        write(vec![Operation::RemoveNode {
            node: Address::new(9, 0),
        }]);
        said(&[path, "--apply", list.to_str().unwrap()])
            .unwrap_or_else(|_| panic!("the node is taken out"));
        let (text, _) = said(&[path]).unwrap_or_else(|_| panic!("the folder lists"));
        assert!(
            text.contains(
                "  nodes9.paths.src: no node, so no region stated, 0 node(s) (0 vehicle, 0 pedestrian), 0 link(s), 0 junction(s)\n"
            ),
            "{text}"
        );
        let (data, _) = said(&[path, "--json"]).unwrap_or_else(|_| panic!("lists as data"));
        let json: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(json["regions"][2]["source"], "nodes9.paths.src");
        assert!(json["regions"][2]["region"].is_null(), "{data}");
    }
}
