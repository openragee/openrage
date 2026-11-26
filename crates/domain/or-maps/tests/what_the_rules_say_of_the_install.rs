//! What the files of a mounted install state of entities, archetypes, rooms and portals, and what the rules of a maps module say of them.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! Two questions, both asked of every `.ymap` and `.ytyp` the mount lists. First, every member the typed world family declares
//! (`or_io::meta::typed`) is one the install states, and the members the install states of an interior's rooms, portals, entity
//! sets and time cycle modifiers are listed, so that what those records type stays a measurement. Second, the rules of
//! [`or_maps::RULES`] are run over the files as one set, and how many maps each fires on is printed and held to the figures the
//! documents quote.
//!
//! ⚠ The install is opened read-only and nothing here writes into one.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use or_io::meta::typed::{
    CBaseArchetypeDef, CEntityDef, CMapData, CMapTypes, CMloArchetypeDef, CMloInstanceDef,
    CTimeArchetypeDef,
};
use or_io::meta::{MetaDocument, MetaInstance, Value};
use or_maps::facts::{inside_of, Fact, Facts};
use or_maps::model::{MapSet, Maps};
use or_maps::{check, SourceFact};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

const WHAT: &str = "what_the_rules_say_of_the_install";

/// The members each type of the world family states in the install, with how many instances state it.
type Census = BTreeMap<String, BTreeMap<String, usize>>;

fn walk(inst: &MetaInstance, census: &mut Census) {
    let entry = census.entry(inst.type_name.clone()).or_default();
    for (name, _) in &inst.fields {
        *entry.entry(name.clone()).or_default() += 1;
    }
    for (_, value) in &inst.fields {
        visit(value, census);
    }
}

fn visit(value: &Value, census: &mut Census) {
    match value {
        Value::Struct(i) | Value::Pointer(Some(i)) => walk(i, census),
        Value::Array(items) => items.iter().for_each(|v| visit(v, census)),
        _ => {}
    }
}

fn source(name: &str, extension: &str, bytes: &[u8]) -> Option<Fact> {
    let inside = inside_of(bytes, extension, None).ok()?;
    Some(Fact::Source(Box::new(SourceFact {
        kind: "metadata".to_string(),
        name: name.to_string(),
        extension: extension.to_string(),
        bytes: bytes.len() as u64,
        game: Vec::new(),
        digest: None,
        inside: Some(Ok(inside)),
    })))
}

#[test]
fn the_typed_world_family_names_only_what_the_install_states_and_the_rules_fire_on_few_of_its_maps()
{
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let game = common::mount(&root).expect("the install mounts");
    let (mut ymaps, mut ytyps) = (Vec::new(), Vec::new());
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".ymap") {
            ymaps.push(logical.to_string());
        } else if leaf.ends_with(".ytyp") {
            ytyps.push(logical.to_string());
        }
    });
    // ⚠ Each logical path once: a mount lists some paths twice, and a file listed twice is one file read twice.
    for listed in [&mut ymaps, &mut ytyps] {
        listed.sort();
        listed.dedup();
    }
    let census = Mutex::new(Census::new());
    let unread = Mutex::new(Vec::<String>::new());
    let types_facts = Mutex::new(Vec::<(String, Fact)>::new());
    let maps_facts = Mutex::new(Vec::<(String, Fact)>::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let all: Vec<&String> = ytyps.iter().chain(ymaps.iter()).collect();
    std::thread::scope(|scope| {
        for _ in 0..12 {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(logical) = all.get(at) else { break };
                let Some(bytes) = game.vfs.open(logical) else {
                    continue;
                };
                let extension = if logical.to_ascii_lowercase().ends_with(".ymap") {
                    "ymap"
                } else {
                    "ytyp"
                };
                let stem = logical
                    .rsplit('/')
                    .next()
                    .unwrap_or(logical)
                    .rsplit_once('.')
                    .map_or("", |(s, _)| s);
                // The typed root reads it.
                let typed = match extension {
                    "ymap" => or_io::decl::File::from_bytes(&bytes).map(|_: CMapData| ()),
                    _ => or_io::decl::File::from_bytes(&bytes).map(|_: CMapTypes| ()),
                };
                if let Err(e) = typed {
                    unread.lock().unwrap().push(format!("{logical}: {e}"));
                    continue;
                }
                let Ok(doc) = MetaDocument::parse(or_io::meta::default_library(), bytes.clone())
                else {
                    unread
                        .lock()
                        .unwrap()
                        .push(format!("{logical}: no document"));
                    continue;
                };
                let mut local = Census::new();
                walk(&doc.instance, &mut local);
                let mut shared = census.lock().unwrap();
                for (t, fields) in local {
                    let e = shared.entry(t).or_default();
                    for (f, n) in fields {
                        *e.entry(f).or_default() += n;
                    }
                }
                drop(shared);
                if let Some(fact) = source(stem, extension, &bytes) {
                    let list = if extension == "ymap" {
                        &maps_facts
                    } else {
                        &types_facts
                    };
                    list.lock().unwrap().push((logical.to_string(), fact));
                }
            });
        }
    });
    let unread = unread.into_inner().unwrap();
    println!(
        "{} .ymap and {} .ytyp files; {} do not open as their typed root",
        ymaps.len(),
        ytyps.len(),
        unread.len()
    );
    assert!(
        unread.is_empty(),
        "files the typed root cannot open: {:?}",
        &unread[..unread.len().min(5)]
    );

    // 1. Every member a typed record declares is stated by the install.
    let census = census.into_inner().unwrap();
    let hash_spelling = |name: &str| format!("hash_{:08X}", or_io::string_hash(name.as_bytes()));
    let mut undeclared_by_the_install: Vec<String> = Vec::new();
    for (type_name, members) in [
        ("CEntityDef", CEntityDef::MEMBERS),
        ("CMloInstanceDef", CMloInstanceDef::MEMBERS),
        ("CBaseArchetypeDef", CBaseArchetypeDef::MEMBERS),
        ("CTimeArchetypeDef", CTimeArchetypeDef::MEMBERS),
        ("CMloArchetypeDef", CMloArchetypeDef::MEMBERS),
        ("CMapData", CMapData::MEMBERS),
        ("CMapTypes", CMapTypes::MEMBERS),
    ] {
        let stated = census.get(type_name);
        for member in members {
            let held = stated
                .is_some_and(|s| s.contains_key(*member) || s.contains_key(&hash_spelling(member)));
            // A derived record's own members are the ones not on its base; the base's are checked under the base.
            if !held && !derived_shares(type_name, member) {
                undeclared_by_the_install.push(format!("{type_name}::{member}"));
            }
        }
    }
    assert!(
        undeclared_by_the_install.is_empty(),
        "members the install never states: {undeclared_by_the_install:?}"
    );

    // 2. What the install states for the four records of an interior, which `or_io::meta::typed::interior` types.
    println!("interiors (what the install states for each record):");
    for type_name in [
        "CMloRoomDef",
        "CMloPortalDef",
        "CMloEntitySet",
        "CMloTimeCycleModifier",
    ] {
        let members: Vec<String> = census
            .get(type_name)
            .map(|m| m.iter().map(|(name, n)| format!("{name}x{n}")).collect())
            .unwrap_or_default();
        println!("  {type_name}: {}", members.join(", "));
        assert!(
            !members.is_empty(),
            "{type_name} is not stated by the install"
        );
    }

    // 3. The rules over the files as sets of up to 2,000 maps and every type file.
    let types: Vec<(String, Fact)> = types_facts.into_inner().unwrap();
    let mut maps: Vec<(String, Fact)> = maps_facts.into_inner().unwrap();
    maps.sort_by(|a, b| a.0.cmp(&b.0));
    let mut fired: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let mut checked = 0usize;
    for chunk in maps.chunks(2000) {
        let mut facts = Facts::default();
        let mut set = MapSet::new("install");
        for (n, (_, fact)) in types.iter().chain(chunk.iter()).enumerate() {
            let path = format!("f{n}.metadata.src");
            set.sources.push(path.clone());
            facts.insert(&path, fact.clone());
        }
        let model = Maps {
            maps: vec![set],
            ..Maps::new()
        };
        let findings = check(&model, &facts, None);
        let names: BTreeMap<String, String> = chunk
            .iter()
            .enumerate()
            .map(|(i, (logical, _))| {
                (
                    format!("f{}.metadata.src", types.len() + i),
                    logical.clone(),
                )
            })
            .collect();
        for finding in findings {
            let who = finding
                .place
                .source
                .as_ref()
                .and_then(|s| names.get(s))
                .cloned()
                .unwrap_or_default();
            fired.entry(finding.code).or_default().insert(who);
        }
        checked += chunk.len();
    }
    println!("rules over {checked} maps (of which each fires on):");
    for (code, who) in &fired {
        println!(
            "  {code:28} {}",
            who.iter().filter(|w| !w.is_empty()).count()
        );
    }
    let maps_with = |code: &str| {
        fired
            .get(code)
            .map_or(0, |who| who.iter().filter(|w| !w.is_empty()).count())
    };
    // ⚠ A map is the file called by its name: no file of an install that states another name holds an entity, and with a map
    // found by its file no link into a parent map is past its end. Found by the name a file states, that rule fired on 1,432
    // of the 19,392 maps of the Legacy install (2026-11-25).
    for code in [
        "map-named-otherwise",
        "parent-link-broken",
        "records-broken",
    ] {
        assert_eq!(maps_with(code), 0, "{code} fires on the game's own maps");
    }
    // The figures the documents quote: a tenth of the game's maps is too many for a rule that only warns.
    for code in [
        "entity-twice",
        "guid-twice",
        "hierarchy-broken",
        "archetype-undeclared",
    ] {
        assert!(
            maps_with(code) * 100 <= checked,
            "{code} fires on {} of {checked} maps",
            maps_with(code)
        );
    }
}

/// Whether `member` of the derived record `type_name` is stated by its base's instances in the census, which the base's own check holds.
fn derived_shares(type_name: &str, member: &str) -> bool {
    match type_name {
        "CMloInstanceDef" => CEntityDef::MEMBERS.contains(&member),
        "CTimeArchetypeDef" | "CMloArchetypeDef" => CBaseArchetypeDef::MEMBERS.contains(&member),
        _ => false,
    }
}
