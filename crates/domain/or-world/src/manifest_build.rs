//! A `_manifest.ymf` for the maps a pack ships, made from manifests the game ships.
//!
//! ⚠ **A manifest is not made from nothing.** No schema in the tree declares
//! its classes, and an entry's `manifestFlags` is a statement no derivation
//! produces. So the document is typed by the type tables of the manifests it
//! is handed, and each entry is a copy of the one they state for a map of
//! that name, with the type files stated or derived. A map none of them
//! states gets no entry, and the caller is told its name.
//!
//! ⚠ The flags of an entry are a set of bits whose enum a shipped type table
//! names. The table written here names none, as for every PSO document the
//! synthesiser lays out, and the bits are kept.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::binary::hash_index::with_public_names_only;
use or_io::meta::binary::{parse_meta_binary_full, type_tables, MetaLayout};
use or_io::meta::{
    canonical, write_meta_binary, BinaryFormat, Diagnostics, MetaError, MetaInstance,
    SchemaLibrary, Value,
};

use crate::field;
use crate::keys;
use crate::manifest::{hashes, Manifest, StatedDependencies};

/// One map a manifest is asked to state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    /// The map's own name, which an entry states it by.
    pub name: u32,
    /// Whether the entry the game states for it is kept whole, as for a map taken from the game, whose derived list may differ from the stated one.
    pub keep_stated: bool,
    /// The type files the derivation says it needs, the resident ones left out.
    pub needs: BTreeSet<u32>,
    /// Whether the derivation left an archetype the map places unresolved, so `needs` is added to the type files the game states and never replaces them.
    pub incomplete: bool,
}

/// A manifest that was written, and what it states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub bytes: Vec<u8>,
    /// The type files it states for each map it states, as it reads back.
    pub stated: BTreeMap<u32, BTreeSet<u32>>,
    /// The maps whose entry is the one the game states, kept whole.
    pub kept: BTreeSet<u32>,
    /// The maps it states nothing for, since no manifest handed in states a map of that name.
    pub unstated: BTreeSet<u32>,
}

/// Why no manifest was written.
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("no manifest of the game was handed in, and a manifest is typed by one and takes the flags of each entry from one")]
    NoDonor,
    /// The first number of this and the next two is which of the manifests handed in it is, from one.
    #[error("it does not read as a metadata document: {1}")]
    Unreadable(usize, String),
    #[error("it is a {1} document, and a manifest is written from a PSO one, the container the game ships its manifests in")]
    NotPso(usize, &'static str),
    #[error("it is no manifest: its root is `{1}`")]
    NotAManifest(usize, String),
    #[error("no manifest handed in states any of the {0} map(s), so there is no entry to write")]
    NoEntry(usize),
    #[error("the first manifest handed in has no list of the kind `{0}` to put an entry in")]
    NoList(&'static str),
    #[error(
        "the manifest that was written reads back stating other than what it was made to state"
    )]
    ReadsBackDifferent,
    #[error(transparent)]
    Meta(#[from] MetaError),
}

/// Write a manifest stating `maps`, typed by `kept` and taking each entry from them.
pub fn build(kept: &[&[u8]], maps: &[Wanted]) -> Result<Built, BuildError> {
    // ⚠ Named from the public table alone, so the bytes are a function of `kept` and `maps` whatever schema the host installed.
    with_public_names_only(|| build_named(kept, maps))
}

fn build_named(kept: &[&[u8]], maps: &[Wanted]) -> Result<Built, BuildError> {
    if kept.is_empty() {
        return Err(BuildError::NoDonor);
    }
    let empty = SchemaLibrary::default();
    let mut donors: Vec<(MetaInstance, MetaLayout)> = Vec::with_capacity(kept.len());
    for (at, bytes) in kept.iter().enumerate() {
        donors.push(donor(bytes, at + 1)?);
    }
    let roots: Vec<&MetaInstance> = donors.iter().map(|(root, _)| root).collect();
    let tree = tree_of(&roots, maps)?;

    let mut tables = SchemaLibrary::default();
    for (_, layout) in &donors {
        type_tables::fill(&mut tables, &type_tables::of_layout(layout));
    }
    let typed = type_tables::extend(&empty, &tables);
    let layout = canonical::layout_for(&typed, &tree.root, BinaryFormat::Pso, None)?;
    let bytes = write_meta_binary(BinaryFormat::Pso, Some(&layout), &tree.root)?;

    // ⚠ A file the readers refuse, or read as stating something else, would ship as a manifest the game misreads.
    let (back, _) = parse_meta_binary_full(&empty, &bytes, &mut Diagnostics::strict())?;
    let stated = stated_by(&back);
    if stated != stated_by(&tree.root) {
        return Err(BuildError::ReadsBackDifferent);
    }
    Ok(Built {
        bytes,
        stated,
        kept: tree.kept,
        unstated: tree.unstated,
    })
}

/// The type files the manifest in `bytes` states for each map, refused when it is not one a manifest can be written from.
pub fn stated_in(bytes: &[u8]) -> Result<BTreeMap<u32, BTreeSet<u32>>, BuildError> {
    with_public_names_only(|| Ok(stated_by(&donor(bytes, 1)?.0)))
}

/// A manifest handed in as the `at`th, read with no schema: its tree, and the layout whose type table types a copy of its entries.
fn donor(bytes: &[u8], at: usize) -> Result<(MetaInstance, MetaLayout), BuildError> {
    let mut diagnostics = Diagnostics::lenient();
    let (root, layout) = parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut diagnostics)
        .map_err(|e| BuildError::Unreadable(at, e.to_string()))?;
    if layout.format != BinaryFormat::Pso {
        return Err(BuildError::NotPso(at, layout.format.label()));
    }
    if Manifest::new(&root).is_none() {
        return Err(BuildError::NotAManifest(at, root.type_name.clone()));
    }
    // ⚠ A reader told to go on reads past a member it cannot decode and leaves it empty, and an entry copied from that would state less than the game's.
    if let Some(gap) = diagnostics.gaps().first() {
        return Err(BuildError::Unreadable(at, gap.to_string()));
    }
    Ok((root, layout))
}

/// The type files a manifest's two pairings state for each map.
fn stated_by(root: &MetaInstance) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut stated = StatedDependencies::new();
    if let Some(manifest) = Manifest::new(root) {
        stated.add(manifest);
    }
    stated
        .names()
        .map(|map| (map, stated.of(map).cloned().unwrap_or_default()))
        .collect()
}

/// The value tree of a manifest, before it is typed and encoded.
#[derive(Debug, Clone)]
pub struct Tree {
    pub root: MetaInstance,
    pub kept: BTreeSet<u32>,
    pub unstated: BTreeSet<u32>,
}

/// The tree of a manifest stating `maps`: the first of `kept` with every list emptied, then an entry for each map that one of `kept` states.
pub fn tree_of(kept: &[&MetaInstance], maps: &[Wanted]) -> Result<Tree, BuildError> {
    let Some(first) = kept.first() else {
        return Err(BuildError::NoDonor);
    };
    // ⚠ Several maps of one name are one map, whose needs are joined: a map is spread over more than one file and an entry is for the map.
    let mut wanted: BTreeMap<u32, Asked> = BTreeMap::new();
    for map in maps {
        let slot = wanted.entry(map.name).or_default();
        slot.keep_stated |= map.keep_stated;
        slot.incomplete |= map.incomplete;
        slot.needs.extend(map.needs.iter().copied());
    }
    // ⚠ One pass over what is kept, so the work is the entries and the maps and never their product. An entry a later manifest
    // states as an earlier one does is one entry: an update of the game restates the entries of the pack it patches, and both are
    // kept of one install. What one manifest states twice is kept twice, as it states it.
    let stating = |key: u32| {
        let mut by_map: BTreeMap<u32, Vec<(usize, &MetaInstance)>> = BTreeMap::new();
        for (at, root) in kept.iter().enumerate() {
            for entry in field::structs_at(root, key) {
                let Some(name) = field::hash_at(entry, keys::IMAP_NAME) else {
                    continue;
                };
                if !wanted.contains_key(&name) {
                    continue;
                }
                let held = by_map.entry(name).or_default();
                if !(held.iter()).any(|(from, earlier)| *from != at && *earlier == entry) {
                    held.push((at, entry));
                }
            }
        }
        by_map
            .into_iter()
            .map(|(name, held)| {
                let entries: Vec<&MetaInstance> = held.into_iter().map(|(_, e)| e).collect();
                (name, entries)
            })
            .collect::<BTreeMap<u32, Vec<&MetaInstance>>>()
    };
    let (groups, pairs) = (
        stating(keys::IMAP_DEPENDENCIES_2),
        stating(keys::IMAP_DEPENDENCIES),
    );

    let (mut grouped, mut flat) = (Vec::new(), Vec::new());
    let (mut kept_whole, mut unstated) = (BTreeSet::new(), BTreeSet::new());
    for (&name, asked) in &wanted {
        let in_groups = groups.get(&name).map_or(&[][..], Vec::as_slice);
        let in_pairs = pairs.get(&name).map_or(&[][..], Vec::as_slice);
        if in_groups.is_empty() && in_pairs.is_empty() {
            unstated.insert(name);
            continue;
        }
        if asked.keep_stated {
            kept_whole.insert(name);
            grouped.extend(in_groups.iter().map(|entry| (*entry).clone()));
            flat.extend(in_pairs.iter().map(|entry| (*entry).clone()));
            continue;
        }
        let mut types = asked.needs.clone();
        if asked.incomplete {
            // ⚠ A type file the streamer is not told to load is a model that does not appear, so a list known to be short adds to the stated one.
            types.extend(
                in_groups
                    .iter()
                    .flat_map(|entry| hashes(entry, keys::ITYP_DEP_ARRAY)),
            );
            types.extend(
                in_pairs
                    .iter()
                    .filter_map(|entry| field::hash_at(entry, keys::ITYP_NAME)),
            );
        }
        if let Some(like) = in_groups.first() {
            let mut entry = (*like).clone();
            if let Some(Value::Array(stated)) = field::get_mut(&mut entry, keys::ITYP_DEP_ARRAY) {
                let spelled = stated.first().cloned();
                *stated = (types.iter())
                    .map(|hash| hash_like(spelled.as_ref(), *hash))
                    .collect();
            }
            grouped.push(entry);
        } else if let Some(like) = in_pairs.first() {
            for hash in &types {
                let mut pair = (*like).clone();
                set_hash(&mut pair, keys::ITYP_NAME, *hash);
                flat.push(pair);
            }
        }
    }
    if grouped.is_empty() && flat.is_empty() {
        return Err(BuildError::NoEntry(wanted.len()));
    }

    let mut root = (*first).clone();
    for (_, value) in &mut root.fields {
        if let Value::Array(items) = value {
            items.clear();
        }
    }
    for (key, name, items) in [
        (keys::IMAP_DEPENDENCIES_2, "imapDependencies_2", grouped),
        (keys::IMAP_DEPENDENCIES, "imapDependencies", flat),
    ] {
        if items.is_empty() {
            continue;
        }
        match field::get_mut(&mut root, key) {
            Some(Value::Array(slot)) => {
                *slot = items
                    .into_iter()
                    .map(|entry| Value::Struct(Box::new(entry)))
                    .collect();
            }
            _ => return Err(BuildError::NoList(name)),
        }
    }
    Ok(Tree {
        root,
        kept: kept_whole,
        unstated,
    })
}

/// What the maps of one name ask of their entry, joined.
#[derive(Default)]
struct Asked {
    keep_stated: bool,
    incomplete: bool,
    needs: BTreeSet<u32>,
}

/// A hash spelled the way `like` spells one, so an entry's new member is the kind of value its type table declares.
fn hash_like(like: Option<&Value>, hash: u32) -> Value {
    match like {
        Some(Value::UInt(_)) => Value::UInt(u64::from(hash)),
        _ => Value::Hash(or_io::meta::instance::value::HashedString::from_hash(hash)),
    }
}

fn set_hash(inst: &mut MetaInstance, key: u32, hash: u32) {
    match field::get_mut(inst, key) {
        Some(Value::Hash(held)) => {
            held.hash = hash;
            held.raw.clear();
        }
        Some(Value::UInt(held)) => *held = u64::from(hash),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::build_library;
    use or_io::meta::instance::value::HashedString;

    /// The classes of a manifest with the members this crate reads, for a document made here: no file of the game is in it.
    const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CImapDependency">
    <string name="imapName" type="atHashString"/>
    <string name="itypName" type="atHashString"/>
  </structdef>
  <structdef type="CImapDependencies">
    <string name="imapName" type="atHashString"/>
    <u32 name="manifestFlags"/>
    <array name="itypDepArray" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CPackFileMetaData">
    <array name="imapDependencies" type="atArray"><struct type="CImapDependency"/></array>
    <array name="imapDependencies_2" type="atArray"><struct type="CImapDependencies"/></array>
  </structdef>
</ParserSchema>"#;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn library() -> SchemaLibrary {
        build_library([("inline.psc", PSC.as_bytes())]).expect("the schema parses")
    }

    /// The flags member as a document spells it: the public table of names lacks it, so it is its hash.
    fn flags_member() -> String {
        format!("hash_{:08X}", field::name_hash("manifestFlags"))
    }

    fn grouped(map: &str, flags: u32, types: &[&str]) -> Value {
        let mut entry = MetaInstance::new("CImapDependencies");
        entry.set("imapName", Value::Hash(HashedString::new(map)));
        entry.set(flags_member(), Value::UInt(u64::from(flags)));
        entry.set(
            "itypDepArray",
            Value::Array(
                (types.iter())
                    .map(|t| Value::Hash(HashedString::new(*t)))
                    .collect(),
            ),
        );
        Value::Struct(Box::new(entry))
    }

    fn pair(map: &str, ityp: &str) -> Value {
        let mut entry = MetaInstance::new("CImapDependency");
        entry.set("imapName", Value::Hash(HashedString::new(map)));
        entry.set("itypName", Value::Hash(HashedString::new(ityp)));
        Value::Struct(Box::new(entry))
    }

    fn manifest(pairs: Vec<Value>, groups: Vec<Value>) -> MetaInstance {
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set("imapDependencies", Value::Array(pairs));
        root.set("imapDependencies_2", Value::Array(groups));
        root
    }

    /// ⚠ Named from the public table alone, as `flags_member` is: a schema another test of the process installed would name the member, and the writer would look for it under that name.
    fn encoded(root: &MetaInstance) -> Vec<u8> {
        with_public_names_only(|| {
            let layout = canonical::layout_for(&library(), root, BinaryFormat::Pso, None)
                .expect("the manifest lays out");
            write_meta_binary(BinaryFormat::Pso, Some(&layout), root).expect("the manifest encodes")
        })
    }

    fn wanted(name: &str, keep_stated: bool, needs: &[&str]) -> Wanted {
        Wanted {
            name: hash(name),
            keep_stated,
            needs: needs.iter().map(|t| hash(t)).collect(),
            incomplete: false,
        }
    }

    /// How many entries of the flat pairing the manifest in `bytes` holds for `map`.
    fn pairs_of(bytes: &[u8], map: &str) -> usize {
        let (root, _) = parse_meta_binary_full(
            &SchemaLibrary::default(),
            bytes,
            &mut Diagnostics::lenient(),
        )
        .expect("the manifest reads");
        field::structs_at(&root, keys::IMAP_DEPENDENCIES)
            .filter(|e| field::hash_at(e, keys::IMAP_NAME) == Some(hash(map)))
            .count()
    }

    fn flags_of(bytes: &[u8], map: &str) -> Vec<u32> {
        let (root, _) = parse_meta_binary_full(
            &SchemaLibrary::default(),
            bytes,
            &mut Diagnostics::lenient(),
        )
        .expect("the manifest reads");
        field::structs_at(&root, keys::IMAP_DEPENDENCIES_2)
            .filter(|e| field::hash_at(e, keys::IMAP_NAME) == Some(hash(map)))
            .filter_map(|e| field::u32_at(e, field::name_hash("manifestFlags")))
            .collect()
    }

    #[test]
    fn a_manifest_states_the_derived_type_files_under_the_flags_the_game_states_for_that_map() {
        let donor = encoded(&manifest(
            Vec::new(),
            vec![
                grouped("harbour", 5, &["t_old"]),
                grouped("elsewhere", 9, &["t_far"]),
            ],
        ));
        let built = build(&[&donor], &[wanted("harbour", false, &["t_new", "t_more"])])
            .expect("the manifest is written");
        assert_eq!(
            built.stated,
            BTreeMap::from([(
                hash("harbour"),
                BTreeSet::from([hash("t_new"), hash("t_more")])
            )]),
            "the map the pack ships and no other, with what the derivation says"
        );
        assert_eq!(flags_of(&built.bytes, "harbour"), [5]);
        assert!(built.kept.is_empty() && built.unstated.is_empty());
        // The same pack builds the same manifest.
        let again = build(&[&donor], &[wanted("harbour", false, &["t_more", "t_new"])]).unwrap();
        assert_eq!(built.bytes, again.bytes);
    }

    #[test]
    fn a_map_taken_from_the_game_keeps_the_entry_the_game_states_where_the_derivation_differs() {
        let donor = encoded(&manifest(
            vec![pair("harbour", "t_paired")],
            vec![grouped("harbour", 3, &["t_game_a", "t_game_b"])],
        ));
        let built = build(&[&donor], &[wanted("harbour", true, &["t_derived"])]).unwrap();
        assert_eq!(
            built.stated[&hash("harbour")],
            BTreeSet::from([hash("t_game_a"), hash("t_game_b"), hash("t_paired")]),
            "what the game states in both pairings, and not the derived list"
        );
        assert_eq!(flags_of(&built.bytes, "harbour"), [3]);
        assert_eq!(built.kept, BTreeSet::from([hash("harbour")]));
    }

    #[test]
    fn a_derivation_that_left_an_archetype_unresolved_adds_to_what_the_game_states_and_takes_nothing_away(
    ) {
        let donor = encoded(&manifest(
            vec![pair("quay", "t_quay_game")],
            vec![grouped("harbour", 5, &["t_game"])],
        ));
        let short = |name: &str, needs: &[&str]| Wanted {
            incomplete: true,
            ..wanted(name, false, needs)
        };
        let built = build(
            &[&donor],
            &[short("harbour", &["t_new"]), short("quay", &["t_quay_new"])],
        )
        .unwrap();
        assert_eq!(
            built.stated,
            BTreeMap::from([
                (
                    hash("harbour"),
                    BTreeSet::from([hash("t_game"), hash("t_new")])
                ),
                (
                    hash("quay"),
                    BTreeSet::from([hash("t_quay_game"), hash("t_quay_new")])
                ),
            ]),
            "a list known to be short is never written as the whole"
        );
        assert_eq!(flags_of(&built.bytes, "harbour"), [5]);
        assert!(built.kept.is_empty(), "the entry is rewritten, not kept");
    }

    #[test]
    fn an_entry_two_kept_manifests_state_alike_is_written_once() {
        let stating = |flags| {
            encoded(&manifest(
                vec![pair("harbour", "t_paired")],
                vec![grouped("harbour", flags, &["t_game"])],
            ))
        };
        // A pack of the game and the update that restates it, kept of one install.
        let (pack, update) = (stating(3), stating(3));
        let built = build(&[&pack, &update], &[wanted("harbour", true, &[])]).unwrap();
        assert_eq!(flags_of(&built.bytes, "harbour"), [3], "one entry");
        assert_eq!(pairs_of(&built.bytes, "harbour"), 1, "one pair");

        // What two manifests state differently is two statements, and neither is chosen for the game.
        let other = stating(4);
        let built = build(&[&pack, &update, &other], &[wanted("harbour", true, &[])]).unwrap();
        assert_eq!(flags_of(&built.bytes, "harbour"), [3, 4]);
        assert_eq!(pairs_of(&built.bytes, "harbour"), 1);

        // What one manifest states twice is its own statement, kept as it is.
        let twice = encoded(&manifest(
            Vec::new(),
            vec![
                grouped("harbour", 3, &["t_game"]),
                grouped("harbour", 3, &["t_game"]),
            ],
        ));
        let built = build(&[&twice, &pack], &[wanted("harbour", true, &[])]).unwrap();
        assert_eq!(flags_of(&built.bytes, "harbour"), [3, 3]);
    }

    #[test]
    fn nothing_the_manifest_it_is_typed_by_states_of_another_map_is_left_in_it() {
        // The donor pairs another map the flat way, in a list the map asked for puts nothing in.
        let donor = encoded(&manifest(
            vec![pair("elsewhere", "t_far")],
            vec![grouped("harbour", 1, &["t_old"])],
        ));
        let built = build(&[&donor], &[wanted("harbour", false, &["t_new"])]).unwrap();
        assert_eq!(
            built.stated,
            BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("t_new")]))])
        );
        assert_eq!(pairs_of(&built.bytes, "elsewhere"), 0);
    }

    #[test]
    fn the_files_of_one_map_are_one_entry_with_their_needs_joined() {
        let donor = encoded(&manifest(Vec::new(), vec![grouped("harbour", 2, &[])]));
        let built = build(
            &[&donor],
            &[
                wanted("harbour", false, &["t_a"]),
                wanted("harbour", false, &["t_b"]),
            ],
        )
        .unwrap();
        assert_eq!(
            built.stated[&hash("harbour")],
            BTreeSet::from([hash("t_a"), hash("t_b")])
        );
        assert_eq!(flags_of(&built.bytes, "harbour"), [2], "one entry");
    }

    #[test]
    fn a_map_no_manifest_states_gets_no_entry_and_is_named() {
        let donor = encoded(&manifest(Vec::new(), vec![grouped("harbour", 0, &[])]));
        let built = build(
            &[&donor],
            &[
                wanted("harbour", false, &["t1"]),
                wanted("made_here", false, &["t1"]),
            ],
        )
        .unwrap();
        assert_eq!(built.unstated, BTreeSet::from([hash("made_here")]));
        assert!(!built.stated.contains_key(&hash("made_here")));

        let none = build(&[&donor], &[wanted("made_here", false, &["t1"])]).unwrap_err();
        assert!(matches!(none, BuildError::NoEntry(1)), "{none}");
    }

    /// ⚠ That the entry is typed by the table of the manifest it came from is the corpus suite's to hold
    /// (`every_staged_manifest_of_the_game_is_written_back_stating_what_it_states`): a table made here declares every class its
    /// schema reaches, where a shipped one leaves out the class of an empty list.
    #[test]
    fn an_entry_is_taken_from_a_later_manifest_when_the_first_states_nothing() {
        let bare = encoded(&manifest(Vec::new(), Vec::new()));
        let stating = encoded(&manifest(
            Vec::new(),
            vec![grouped("harbour", 1, &["t_old"])],
        ));
        let built = build(&[&bare, &stating], &[wanted("harbour", false, &["t_new"])]).unwrap();
        assert_eq!(
            built.stated[&hash("harbour")],
            BTreeSet::from([hash("t_new")])
        );
    }

    #[test]
    fn a_map_stated_only_by_the_flat_pairing_is_stated_that_way() {
        let donor = encoded(&manifest(vec![pair("harbour", "t_old")], Vec::new()));
        let built = build(&[&donor], &[wanted("harbour", false, &["t_a", "t_b"])]).unwrap();
        assert_eq!(
            built.stated[&hash("harbour")],
            BTreeSet::from([hash("t_a"), hash("t_b")])
        );
    }

    #[test]
    fn a_kept_manifest_with_a_member_that_does_not_decode_is_refused_and_not_read_short() {
        let whole = encoded(&manifest(
            Vec::new(),
            vec![grouped("harbour", 1, &["t_game"])],
        ));
        // One byte changed so that a reader told to go on reads the file past a member it could not decode.
        let damaged = (0..whole.len())
            .find_map(|at| {
                let mut bytes = whole.clone();
                bytes[at] ^= 0xFF;
                let mut diagnostics = Diagnostics::lenient();
                let (root, layout) = with_public_names_only(|| {
                    parse_meta_binary_full(&SchemaLibrary::default(), &bytes, &mut diagnostics)
                })
                .ok()?;
                (layout.format == BinaryFormat::Pso
                    && Manifest::new(&root).is_some()
                    && !diagnostics.is_clean())
                .then_some(bytes)
            })
            .expect("a byte whose change the reader reads past");
        let refused = build(&[&damaged], &[wanted("harbour", true, &[])]).unwrap_err();
        assert!(matches!(refused, BuildError::Unreadable(1, _)), "{refused}");
        assert!(matches!(
            stated_in(&damaged),
            Err(BuildError::Unreadable(1, _))
        ));
    }

    #[test]
    fn what_is_handed_in_is_refused_by_what_it_is() {
        assert!(matches!(build(&[], &[]), Err(BuildError::NoDonor)));
        let junk: &[u8] = b"not a metadata document";
        let refused = build(&[junk], &[wanted("harbour", false, &[])]).unwrap_err();
        assert!(matches!(refused, BuildError::Unreadable(1, _)), "{refused}");

        let mut other = MetaInstance::new("CImapDependency");
        other.set("imapName", Value::Hash(HashedString::new("x")));
        other.set("itypName", Value::Hash(HashedString::new("y")));
        let refused = build(&[&encoded(&other)], &[wanted("harbour", false, &[])]).unwrap_err();
        assert!(
            matches!(refused, BuildError::NotAManifest(1, _)),
            "{refused}"
        );

        // A manifest in another container, which the game ships a few of, is one no type table of a PSO document comes from.
        let root = manifest(Vec::new(), vec![grouped("harbour", 0, &[])]);
        let rbf = write_meta_binary(BinaryFormat::Rbf, None, &root).expect("the manifest encodes");
        let refused = build(&[&rbf], &[wanted("harbour", false, &[])]).unwrap_err();
        assert!(matches!(refused, BuildError::NotPso(1, "RBF")), "{refused}");
        assert!(matches!(stated_in(&rbf), Err(BuildError::NotPso(1, "RBF"))));
    }
}
