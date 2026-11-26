//! The interiors and rooms of the game data `.rel` files an install ships: what holds them, how
//! their files are laid out, what names them, and that each such file takes one more interior by
//! copy, reads it back with everything it held, and saves to its fixed point.
//!
//! ⚠ Install-gated: `OPENRAGE_GAME_ROOT` names the installs, `;`-separated. The files sit inside
//! archives, so each install's own executable supplies the keys and the bundled snapshot the NG
//! ones (D18). The game data siblings a DLC ships beside its `.rel` files, `.dat149` to `.dat151`,
//! are walked and counted apart, so a figure of the `.rel` files counts no file twice.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5Keys, Gta5RetailCryptoProvider};
use or_io::container::rpf::{RpfArchive, RpfCryptoProvider};
use or_io::hash::string_hash_str;
use or_io::rel::game_object::{GameObjectItem, InteriorRoom, InteriorSettings};
use or_io::rel::{RelFile, RelIndex, RelIndexHash};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

/// How deep a `.rpf` inside a `.rpf` is followed, as the install crawl does.
const MAX_NEST_DEPTH: u32 = 8;

fn provider(keys: &Arc<Gta5Keys>) -> Arc<dyn RpfCryptoProvider> {
    Arc::new(Gta5RetailCryptoProvider::new(Arc::clone(keys)))
}

fn walk_archive(
    archive: &Arc<RpfArchive>,
    label: &str,
    depth: u32,
    visit: &mut dyn FnMut(String, Result<Vec<u8>, String>),
) {
    for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
        let name = node.name().to_ascii_lowercase();
        let path = format!("{label}/{}", node.path());
        let i = node.entry_index();
        if name.ends_with(".rpf") && depth < MAX_NEST_DEPTH {
            match archive.open_nested(i) {
                Ok(nested) => walk_archive(&Arc::new(nested), &path, depth + 1, visit),
                Err(e) => visit(path, Err(e.to_string())),
            }
        } else if is_rel(&name) {
            visit(path, archive.read_entry(i).map_err(|e| e.to_string()));
        }
    }
}

/// Whether a lower-case file name is a `.rel`, or a game data sibling a DLC ships beside one.
fn is_rel(name: &str) -> bool {
    [".rel", ".dat149", ".dat150", ".dat151"]
        .iter()
        .any(|extension| name.ends_with(extension))
}

/// Whether a file of the walk is a game data sibling and not a `.rel`.
fn is_sibling(label: &str) -> bool {
    !label.to_ascii_lowercase().ends_with(".rel")
}

/// Every `.rel` and game data sibling under `root`, loose or in an archive at any depth.
fn every_rel(root: &Path, visit: &mut dyn FnMut(String, Result<Vec<u8>, String>)) {
    let exe = ["GTA5_Enhanced.exe", "GTA5.exe"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("no game executable under {}", root.display()));
    let keys = Arc::new(extract_keys(&exe, &|_| {}).expect("keys from the install's executable"));
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let label = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let lower = label.to_ascii_lowercase();
            if is_rel(&lower) {
                visit(label, std::fs::read(&path).map_err(|e| e.to_string()));
            } else if lower.ends_with(".rpf") {
                match RpfArchive::open_with(&path, provider(&keys)) {
                    Ok(archive) => walk_archive(&Arc::new(archive), &label, 0, visit),
                    Err(e) => visit(label, Err(e.to_string())),
                }
            }
        }
    }
}

/// A `.rel` as an install ships it: where it is, its bytes, and the file they open as.
type Shipped = (String, Vec<u8>, RelFile);

/// Every `.rel` and game data sibling of `root` whose `rel_type` is game data's, 151, or older game data's, 149 and 150, opened, with the ones that would not open by name.
fn game_data(root: &Path) -> (Vec<Shipped>, Vec<String>) {
    let (mut files, mut failed) = (Vec::new(), Vec::new());
    every_rel(root, &mut |label, bytes| {
        // A nested archive that will not open is a `.rel` this run did not
        // see, so it fails rather than shrinking the census.
        match bytes.and_then(|bytes| {
            let file = RelFile::open(&bytes).map_err(|e| e.to_string())?;
            Ok((bytes, file))
        }) {
            Ok((bytes, file)) if (149..=151).contains(&file.rel_type) => {
                files.push((label, bytes, file))
            }
            Ok(_) => {}
            Err(e) => failed.push(format!("{label}: {e}")),
        }
    });
    (files, failed)
}

fn hashed(file: &RelFile) -> &[RelIndexHash] {
    match &file.index {
        RelIndex::Hashed(index) => index,
        RelIndex::Named(_) => &[],
    }
}

/// Every item of a file with its key and its start, in data block order, or the first that reads as no type.
fn typed(file: &RelFile) -> Result<Vec<(RelIndexHash, GameObjectItem)>, String> {
    let mut spans = hashed(file).to_vec();
    spans.sort_by_key(|entry| entry.offset);
    spans
        .into_iter()
        .map(|entry| {
            let start = entry.offset as usize;
            GameObjectItem::decode(&file.data[start..start + entry.length as usize])
                .map(|item| (entry, item))
                .map_err(|e| format!("{:#010x}: {e}", entry.hash))
        })
        .collect()
}

/// An item's bytes and the words of it the hash table and the pack table list, counted from its start.
type Held = (Vec<u8>, Vec<u32>, Vec<u32>);

/// Every item of the file by its key.
fn held(file: &RelFile) -> BTreeMap<u32, Held> {
    let mut spans = hashed(file).to_vec();
    spans.sort_by_key(|entry| entry.offset);
    let mut held: BTreeMap<u32, Held> = spans
        .iter()
        .map(|entry| {
            let start = entry.offset as usize;
            let body = file.data[start..start + entry.length as usize].to_vec();
            (entry.hash, (body, Vec::new(), Vec::new()))
        })
        .collect();
    for (offsets, pack) in [(&file.hash_offsets, false), (&file.pack_offsets, true)] {
        for &offset in offsets {
            let at = spans.partition_point(|entry| entry.offset <= offset);
            let entry = spans[at.checked_sub(1).expect("a listed word before every item")];
            let (_, hash_words, pack_words) = held.get_mut(&entry.hash).unwrap();
            let words = if pack { pack_words } else { hash_words };
            words.push(offset - entry.offset);
        }
    }
    held
}

/// Where `body` states `value`, when it states it once.
fn word_at(body: &[u8], value: u32) -> Option<u32> {
    let mut at = body
        .windows(4)
        .enumerate()
        .filter(|(_, word)| *word == value.to_le_bytes())
        .map(|(at, _)| at as u32);
    at.next().filter(|_| at.next().is_none())
}

/// A typed member's value as bits, by the member's name.
fn interior_members(interior: &InteriorSettings) -> Vec<(&'static str, u32)> {
    vec![
        (
            "InteriorSettings.name_table_offset",
            interior.name_table_offset.0,
        ),
        ("InteriorSettings.flags", interior.flags),
        (
            "InteriorSettings.interior_walla_sound_set",
            interior.interior_walla_sound_set,
        ),
        (
            "InteriorSettings.interior_reflections",
            interior.interior_reflections,
        ),
        (
            "InteriorSettings.room, its count",
            interior.room.len() as u32,
        ),
    ]
}

fn room_members(room: &InteriorRoom) -> Vec<(&'static str, u32)> {
    vec![
        ("InteriorRoom.name_table_offset", room.name_table_offset.0),
        ("InteriorRoom.flags", room.flags),
        ("InteriorRoom.room_name", room.room_name),
        ("InteriorRoom.ambient_zone", room.ambient_zone),
        ("InteriorRoom.interior_type", room.interior_type.into()),
        ("InteriorRoom.reverb_small", room.reverb_small.to_bits()),
        ("InteriorRoom.reverb_medium", room.reverb_medium.to_bits()),
        ("InteriorRoom.reverb_large", room.reverb_large.to_bits()),
        ("InteriorRoom.room_tone_sound", room.room_tone_sound),
        ("InteriorRoom.rain_type", room.rain_type.into()),
        (
            "InteriorRoom.exterior_audibility",
            room.exterior_audibility.to_bits(),
        ),
        (
            "InteriorRoom.room_occlusion_damping",
            room.room_occlusion_damping.to_bits(),
        ),
        (
            "InteriorRoom.non_marked_portal_occlusion",
            room.non_marked_portal_occlusion.to_bits(),
        ),
        (
            "InteriorRoom.distance_from_portal_for_occlusion",
            room.distance_from_portal_for_occlusion.to_bits(),
        ),
        (
            "InteriorRoom.distance_from_portal_fade_distance",
            room.distance_from_portal_fade_distance.to_bits(),
        ),
        ("InteriorRoom.weapon_metrics", room.weapon_metrics),
        (
            "InteriorRoom.interior_walla_sound_set",
            room.interior_walla_sound_set,
        ),
    ]
}

/// A count out of a population, printed as both.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Of(usize, usize);

impl Of {
    fn tally(&mut self, held: bool) {
        self.0 += usize::from(held);
        self.1 += 1;
    }
}

impl std::fmt::Display for Of {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} of {}", self.0, self.1)
    }
}

#[test]
fn the_interiors_of_an_install_lay_out_as_an_edit_writes_them() {
    let what = "the interiors and rooms of an install's game data, measured";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("no install root; set {GAME_ROOT_ENV}"));
        return;
    }
    for root in roots {
        let (files, mut failed) = game_data(&root);
        let (mut current, mut older, mut older_with) = (0usize, 0usize, 0usize);
        let (mut with, mut with_interior) = (0usize, 0usize);
        let (mut interiors, mut rooms) = (0usize, 0usize);
        let (mut named, mut packed) = (0usize, 0usize);
        let (mut typed_back, mut untouched, mut null_edit) =
            (Of::default(), Of::default(), Of::default());
        let (mut aligned, mut ordered, mut ascending) =
            (Of::default(), Of::default(), Of::default());
        let (mut room_refs_listed, mut zone_listed) = (Of::default(), Of::default());
        let (mut refs_held, mut shared, mut unlisted) = (Of::default(), 0usize, 0usize);
        let mut namers: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut strays: BTreeMap<(&'static str, usize), usize> = BTreeMap::new();
        let mut members: BTreeMap<&'static str, BTreeSet<u32>> = BTreeMap::new();
        let shipped_as_rel: HashSet<&[u8]> = files
            .iter()
            .filter(|(label, _, _)| !is_sibling(label))
            .map(|(_, bytes, _)| &bytes[..])
            .collect();
        let (mut siblings, mut siblings_with) = (Of::default(), 0usize);
        for (label, bytes, file) in &files {
            let items = match typed(file) {
                Ok(items) => items,
                Err(e) => {
                    failed.push(format!("{label}: {e}"));
                    continue;
                }
            };
            if is_sibling(label) {
                siblings.tally(shipped_as_rel.contains(&bytes[..]));
                siblings_with += usize::from(
                    items
                        .iter()
                        .any(|(_, item)| matches!(item, GameObjectItem::InteriorSettings(_))),
                );
                continue;
            }
            if file.rel_type != 151 {
                older += 1;
                older_with += usize::from(items.iter().any(|(_, item)| {
                    matches!(
                        item,
                        GameObjectItem::InteriorSettings(_) | GameObjectItem::InteriorRoom(_)
                    )
                }));
                continue;
            }
            current += 1;
            // Every game data file, whether or not it holds an interior: each item at the next
            // multiple of its own alignment after the four leading bytes, the item table by each
            // key's low byte and then the key, the hash table ascending.
            let mut end = 4usize;
            let mut laid = true;
            for (entry, item) in &items {
                let start = entry.offset as usize;
                laid &= start == end.next_multiple_of(item.own_align())
                    && file.data[end..start].iter().all(|&b| b == 0);
                end = start + entry.length as usize;
            }
            aligned.tally(laid && (items.is_empty() || end == file.data.len()));
            let order: Vec<(u32, u32)> = hashed(file)
                .iter()
                .map(|e| (e.hash & 0xFF, e.hash))
                .collect();
            ordered.tally(order.windows(2).all(|pair| pair[0] < pair[1]));
            ascending.tally(file.hash_offsets.windows(2).all(|pair| pair[0] < pair[1]));

            let interior_keys: BTreeSet<u32> = items
                .iter()
                .filter(|(_, item)| matches!(item, GameObjectItem::InteriorSettings(_)))
                .map(|(entry, _)| entry.hash)
                .collect();
            let room_keys: BTreeSet<u32> = items
                .iter()
                .filter(|(_, item)| matches!(item, GameObjectItem::InteriorRoom(_)))
                .map(|(entry, _)| entry.hash)
                .collect();
            if interior_keys.is_empty() && room_keys.is_empty() {
                continue;
            }
            with += 1;
            with_interior += usize::from(!interior_keys.is_empty());
            interiors += interior_keys.len();
            rooms += room_keys.len();
            named += usize::from(!file.names.is_empty());
            packed += usize::from(!file.pack_offsets.is_empty());
            untouched.tally(file.save().ok().as_deref() == Some(&bytes[..]));

            let held = held(file);
            let mut uses: BTreeMap<u32, usize> = BTreeMap::new();
            // One interior and one room of each file are written again as they read.
            let mut null = file.clone();
            let (mut null_ok, mut interior_set, mut room_set) = (true, false, false);
            for (entry, item) in &items {
                let (body, hash_words, _) = &held[&entry.hash];
                match item {
                    GameObjectItem::InteriorSettings(interior) => {
                        typed_back.tally(item.encode().ok().as_ref() == Some(body));
                        let refs: Vec<Option<u32>> = (0..interior.room.len())
                            .map(|i| u32::try_from(body.len() - 4 * (interior.room.len() - i)).ok())
                            .collect();
                        let listed: Vec<Option<u32>> =
                            hash_words.iter().copied().map(Some).collect();
                        room_refs_listed.tally(listed == refs);
                        for room in &interior.room {
                            refs_held.tally(room_keys.contains(&room.r#ref));
                            *uses.entry(room.r#ref).or_default() += 1;
                        }
                        for (member, value) in interior_members(interior) {
                            members.entry(member).or_default().insert(value);
                        }
                        if !std::mem::replace(&mut interior_set, true) {
                            null_ok &= null.set_interior(entry.hash, interior).is_ok();
                        }
                    }
                    GameObjectItem::InteriorRoom(room) => {
                        typed_back.tally(item.encode().ok().as_ref() == Some(body));
                        // The one listed word holds the ambient zone, found by its value where it is stated once.
                        let zone = hash_words.len() == 1
                            && body[hash_words[0] as usize..][..4]
                                == room.ambient_zone.to_le_bytes()
                            && word_at(body, room.ambient_zone)
                                .is_none_or(|at| at == hash_words[0]);
                        zone_listed.tally(zone);
                        for (member, value) in room_members(room) {
                            members.entry(member).or_default().insert(value);
                        }
                        if !std::mem::replace(&mut room_set, true) {
                            null_ok &= null.set_interior_room(entry.hash, room).is_ok();
                        }
                    }
                    _ => {}
                }
                // Every word of every item that equals an interior's or a room's key, by who states it.
                for (at, word) in body.windows(4).enumerate().skip(4) {
                    let value = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
                    let (is_interior, is_room) =
                        (interior_keys.contains(&value), room_keys.contains(&value));
                    if !is_interior && !is_room {
                        continue;
                    }
                    let who = match item {
                        GameObjectItem::InteriorSettings(interior)
                            if is_room && interior.room.iter().any(|room| room.r#ref == value) =>
                        {
                            "InteriorSettings.room"
                        }
                        GameObjectItem::StaticEmitter(emitter)
                            if is_interior && emitter.interior_settings == value =>
                        {
                            "StaticEmitter.interior_settings"
                        }
                        GameObjectItem::StaticEmitter(emitter)
                            if is_room && emitter.interior_room == value =>
                        {
                            "StaticEmitter.interior_room"
                        }
                        GameObjectItem::AlarmSettings(alarm)
                            if is_interior && alarm.interior_settings == value =>
                        {
                            "AlarmSettings.interior_settings"
                        }
                        // A room's name in the MLO is the hash of a name and names no item: it
                        // is its own item's key where the item is named as the room is.
                        GameObjectItem::InteriorRoom(room) if room.room_name == value => {
                            if value == entry.hash {
                                "InteriorRoom.room_name, its own key"
                            } else if is_room {
                                "InteriorRoom.room_name, another room's key"
                            } else {
                                "InteriorRoom.room_name, an interior's key"
                            }
                        }
                        other => {
                            *strays.entry((other.type_name(), at)).or_default() += 1;
                            continue;
                        }
                    };
                    *namers.entry(who).or_default() += 1;
                }
            }
            null_edit.tally(null_ok && null == *file);
            shared += uses.values().filter(|&&n| n > 1).count();
            unlisted += room_keys
                .iter()
                .filter(|key| !uses.contains_key(key))
                .count();
        }
        println!("{}:", root.display());
        println!("  {current} game data .rel (rel_type 151); {with} hold an interior or a room, {with_interior} an interior");
        println!("  {older} older game data .rel (rel_type 149 and 150); {older_with} hold an interior or a room");
        println!("  {interiors} InteriorSettings and {rooms} InteriorRoom");
        println!("  read typed and written back to their bytes: {typed_back} items");
        println!("  saved untouched to the bytes read: {untouched} files");
        println!("  an edit that changes nothing leaves the file as read: {null_edit} files");
        println!("  carry a name table: {named}; a pack table entry: {packed}");
        println!("  of every game data .rel: items at their own alignment after four bytes {aligned}, item table by low byte then key {ordered}, hash table ascending {ascending}");
        println!("  the hash table lists each room an interior names and no other word of it: {room_refs_listed} interiors");
        println!("  the hash table lists a room's ambient zone and no other word of it: {zone_listed} rooms");
        println!("  rooms an interior lists that its file holds as an InteriorRoom: {refs_held}");
        println!(
            "  rooms more than one interior lists: {shared}; rooms no interior lists: {unlisted}"
        );
        println!("  words equal to an interior's or a room's key, by member: {namers:?}");
        println!("  the same in no member that names one: {strays:?}");
        println!("  game data siblings (.dat149, .dat150, .dat151) that are byte for byte a .rel of the walk: {siblings}; {siblings_with} hold an interior");
        println!("  distinct values a member takes:");
        for (member, values) in &members {
            println!("    {member}: {}", values.len());
        }
        assert!(failed.is_empty(), "{}: {failed:#?}", root.display());
        assert!(
            interiors > 0 && rooms > 0,
            "{}: no interior found",
            root.display()
        );
        for (what, of) in [
            ("typed", typed_back),
            ("untouched", untouched),
            ("null edit", null_edit),
            ("aligned", aligned),
            ("ordered", ordered),
            ("ascending", ascending),
            ("room references listed", room_refs_listed),
            ("ambient zone listed", zone_listed),
            ("room references held", refs_held),
            ("siblings that are a .rel", siblings),
        ] {
            assert_eq!(of.0, of.1, "{}: {what}", root.display());
        }
        assert_eq!((named, packed, older_with), (0, 0, 0), "{}", root.display());
        assert!(strays.is_empty(), "{}: {strays:?}", root.display());
    }
}

/// A key no item of the file is under, from a name.
fn fresh(file: &RelFile, name: &str, taken: &[u32]) -> u32 {
    (0u32..)
        .map(|n| string_hash_str(&format!("{name}_{n}")))
        .find(|key| !taken.contains(key) && hashed(file).iter().all(|entry| entry.hash != *key))
        .expect("a free key")
}

/// The interiors among `items`, in item table order.
fn interiors<'a>(
    file: &RelFile,
    items: &'a [(RelIndexHash, GameObjectItem)],
) -> Vec<(u32, &'a InteriorSettings)> {
    hashed(file)
        .iter()
        .filter_map(|entry| {
            items.iter().find_map(|(at, item)| match item {
                GameObjectItem::InteriorSettings(interior) if at.hash == entry.hash => {
                    Some((entry.hash, &**interior))
                }
                _ => None,
            })
        })
        .collect()
}

/// Copies the interior of `file` that lists the fewest rooms, at least one, and holds the result to what the copy must be; the rooms copied.
fn one_more_by_copy(bytes: &[u8], file: &RelFile) -> Result<usize, String> {
    let items = typed(file)?;
    let mut sources = interiors(file, &items);
    sources.retain(|(_, interior)| !interior.room.is_empty());
    sources.sort_by_key(|(_, interior)| interior.room.len());
    let &(from, source) = sources.first().ok_or("no interior lists a room")?;
    copy_and_remove(bytes, file, items.len(), &held(file), from, source)
}

/// Copies the interior `source` of `file`, under `from`, holds the result to what the copy must be, and removes it; the rooms copied.
fn copy_and_remove(
    bytes: &[u8],
    file: &RelFile,
    items: usize,
    was: &BTreeMap<u32, Held>,
    from: u32,
    source: &InteriorSettings,
) -> Result<usize, String> {
    let to = fresh(file, "openrage_copied_interior", &[]);
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for room in &source.room {
        if pairs.iter().all(|(old, _)| *old != room.r#ref) {
            let taken: Vec<u32> = pairs.iter().map(|(_, new)| *new).chain([to]).collect();
            pairs.push((room.r#ref, fresh(file, "openrage_copied_room", &taken)));
        }
    }
    let new_key = |old: u32| {
        pairs
            .iter()
            .find(|(key, _)| *key == old)
            .map(|(_, new)| *new)
    };

    let mut edited = file.clone();
    let text = |e: or_io::IoError| e.to_string();
    edited.copy_interior(from, to, &pairs).map_err(text)?;
    let saved = edited.save().map_err(text)?;
    let read = RelFile::open(&saved).map_err(text)?;
    if read != edited {
        return Err("the saved file reads back as another".into());
    }
    if read.save().map_err(text)? != saved {
        return Err("the second save differs from the first".into());
    }
    let mut is = held(&read);
    let mut copy = source.clone();
    for room in &mut copy.room {
        room.r#ref = new_key(room.r#ref).ok_or("a room with no new key")?;
    }
    if read.interior(to).map_err(text)?.as_ref() != Some(&copy) {
        return Err(
            "the copied interior does not read back as its source under the new keys".into(),
        );
    }
    let (body, hash_words, pack_words) = is
        .remove(&to)
        .ok_or("the copied interior is not in the item table")?;
    let refs: Option<Vec<u32>> = copy
        .room
        .iter()
        .map(|room| word_at(&body, room.r#ref))
        .collect();
    let distinct = pairs.len() == copy.room.len();
    if (distinct && Some(&hash_words) != refs.as_ref())
        || hash_words.len() != copy.room.len()
        || !pack_words.is_empty()
    {
        return Err("the tables list other words of the copied interior than its rooms".into());
    }
    for &(old, new) in &pairs {
        let room = file.interior_room(old).map_err(text)?;
        if room.is_none() || read.interior_room(new).map_err(text)? != room {
            return Err("a copied room does not read back as its source".into());
        }
        if is.remove(&new) != Some(was[&old].clone()) {
            return Err("a copied room's bytes or listed words are not its source's".into());
        }
    }
    if is != *was {
        return Err("an item the file held changed".into());
    }
    if read.typed_items().map_err(text)?.len() != items + 1 + pairs.len() {
        return Err("the file does not hold the old items and the new".into());
    }
    let mut back = read;
    back.remove_interior(to).map_err(text)?;
    if back.save().map_err(text)? != bytes {
        return Err("removing the copy does not give the shipped bytes back".into());
    }
    Ok(pairs.len())
}

#[test]
fn every_install_rel_with_an_interior_takes_one_more_by_copy() {
    let what = "one more interior by copy in every .rel of an install that holds one";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("no install root; set {GAME_ROOT_ENV}"));
        return;
    }
    for root in roots {
        let (files, mut failed) = game_data(&root);
        let (mut seen, mut took, mut one_room, mut rooms) = (0usize, 0usize, 0usize, 0usize);
        let mut siblings = Of::default();
        for (label, bytes, file) in files.iter().filter(|(_, _, file)| file.rel_type == 151) {
            let holds = typed(file).is_ok_and(|items| {
                items
                    .iter()
                    .any(|(_, item)| matches!(item, GameObjectItem::InteriorSettings(_)))
            });
            if !holds {
                continue;
            }
            let copied = one_more_by_copy(bytes, file);
            if let Err(e) = &copied {
                failed.push(format!("{label}: {e}"));
            }
            if is_sibling(label) {
                siblings.tally(copied.is_ok());
                continue;
            }
            seen += 1;
            if let Ok(copied) = copied {
                took += 1;
                one_room += usize::from(copied == 1);
                rooms += copied;
            }
        }
        println!(
            "{}: {took} of {seen} .rel that hold an interior took one more by copy, read it back with \
             every item they held, saved to a fixed point, and gave the shipped bytes back once the \
             copy was removed; {one_room} copies took one room, and {rooms} rooms were copied in all; \
             so did {siblings} .dat151 siblings that hold one",
            root.display()
        );
        assert!(failed.is_empty(), "{}: {failed:#?}", root.display());
        assert!(seen > 0, "{}: no .rel holds an interior", root.display());
        assert_eq!(took, seen, "{}", root.display());
        assert_eq!(siblings.0, siblings.1, "{}", root.display());
    }
}

#[test]
fn every_interior_of_an_install_is_copied_and_the_copy_removed() {
    let what = "a copy of every interior of an install, each removed again";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("no install root; set {GAME_ROOT_ENV}"));
        return;
    }
    for root in roots {
        let (files, mut failed) = game_data(&root);
        // One file of each content: an update ships a file its base ships too.
        let mut walked: HashSet<&[u8]> = HashSet::new();
        let (mut copied, mut distinct_files) = (Of::default(), 0usize);
        let (mut roomless, mut shared, mut twice, mut most) = (0usize, 0usize, 0usize, 0usize);
        for (label, bytes, file) in files.iter().filter(|(_, _, file)| file.rel_type == 151) {
            if !walked.insert(&bytes[..]) {
                continue;
            }
            let items = match typed(file) {
                Ok(items) => items,
                Err(e) => {
                    failed.push(format!("{label}: {e}"));
                    continue;
                }
            };
            let sources = interiors(file, &items);
            if sources.is_empty() {
                continue;
            }
            distinct_files += 1;
            let was = held(file);
            for &(from, source) in &sources {
                let listed: BTreeSet<u32> = source.room.iter().map(|room| room.r#ref).collect();
                roomless += usize::from(listed.is_empty());
                twice += usize::from(listed.len() != source.room.len());
                shared += usize::from(sources.iter().any(|(other, interior)| {
                    *other != from
                        && interior
                            .room
                            .iter()
                            .any(|room| listed.contains(&room.r#ref))
                }));
                most = most.max(listed.len());
                let took = copy_and_remove(bytes, file, items.len(), &was, from, source);
                copied.tally(took.is_ok());
                if let Err(e) = took {
                    failed.push(format!("{label}: {from:#010x}: {e}"));
                }
            }
        }
        println!(
            "{}: {copied} interiors of {distinct_files} distinct game data files were copied, read back \
             with every item their file held, saved to a fixed point, and gave the shipped bytes back \
             once the copy was removed; {roomless} list no room, {shared} list a room another interior \
             lists too, {twice} list a room twice, and the most rooms one lists is {most}",
            root.display()
        );
        assert!(failed.is_empty(), "{}: {failed:#?}", root.display());
        assert!(copied.1 > 0, "{}: no interior found", root.display());
        assert_eq!(copied.0, copied.1, "{}", root.display());
    }
}
