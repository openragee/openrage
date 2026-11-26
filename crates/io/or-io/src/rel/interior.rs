//! The interiors of a game data file and their rooms, added, edited and removed: `rel_type` 151's
//! `InteriorSettings` and `InteriorRoom` items, which no other `rel_type` holds in either install.
//!
//! An edit lays the whole data block out again as every shipped game data file is laid out, and a
//! file that layout does not write back byte for byte is refused before anything changes.

use super::game_object::{GameObjectItem, InteriorRoom, InteriorSettings, Room};
use super::{refuse, RelFile, RelIndex, RelIndexHash};
use crate::error::Result;

/// The `rel_type` of a game data file.
const GAME_DATA: u32 = 151;

/// The bytes a game data file's data block opens with, before its first item.
const LEAD: usize = 4;

/// Where an `InteriorRoom` states its ambient zone: after its name table offset, flags and room name.
const AMBIENT_ZONE_AT: usize = 3 * size_of::<u32>();

/// The most rooms an interior lists: its count is one byte.
const MAX_ROOMS: usize = u8::MAX as usize;

/// One item out of the data block, with the words of it the two offset tables list.
struct Entry {
    key: u32,
    item: GameObjectItem,
    body: Vec<u8>,
    /// Counted from the item's start, as the hash table and the pack table list them.
    hash_fields: Vec<usize>,
    pack_fields: Vec<usize>,
}

/// A game data file's items in data block order.
struct Laid {
    lead: [u8; LEAD],
    entries: Vec<Entry>,
}

/// What [`Laid`] writes: the four members of a [`RelFile`] an edit derives.
#[derive(PartialEq)]
struct Packed {
    data: Vec<u8>,
    index: Vec<RelIndexHash>,
    hash_offsets: Vec<u32>,
    pack_offsets: Vec<u32>,
}

fn key_hex(key: u32) -> String {
    format!("{key:#010x}")
}

fn word(len: usize, what: &str) -> Result<u32> {
    u32::try_from(len).map_err(|_| refuse(format!("{what} is at {len:#x}, past a u32")))
}

impl Entry {
    fn interior(key: u32, interior: &InteriorSettings) -> Result<Self> {
        if interior.room.len() > MAX_ROOMS {
            return Err(refuse(format!(
                "the interior {} lists {} rooms, and its count states at most {MAX_ROOMS}",
                key_hex(key),
                interior.room.len()
            )));
        }
        let item = GameObjectItem::InteriorSettings(Box::new(interior.clone()));
        let body = encode(key, &item)?;
        // The rooms are the item's last member, one word each.
        let rooms = body.len() - interior.room.len() * size_of::<u32>();
        let hash_fields = (0..interior.room.len())
            .map(|i| rooms + i * size_of::<u32>())
            .collect();
        Ok(Self {
            key,
            item,
            body,
            hash_fields,
            pack_fields: Vec::new(),
        })
    }

    fn room(key: u32, room: &InteriorRoom) -> Result<Self> {
        let item = GameObjectItem::InteriorRoom(Box::new(room.clone()));
        let body = encode(key, &item)?;
        Ok(Self {
            key,
            item,
            body,
            hash_fields: vec![AMBIENT_ZONE_AT],
            pack_fields: Vec::new(),
        })
    }
}

fn encode(key: u32, item: &GameObjectItem) -> Result<Vec<u8>> {
    item.encode()
        .map_err(|e| refuse(format!("the {} {}: {e}", item.type_name(), key_hex(key))))
}

impl Laid {
    /// Takes a game data file apart, refusing one whose bytes [`Laid::pack`] would not write back.
    fn unpack(file: &RelFile) -> Result<Self> {
        let index = game_data_index(file)?;
        if !file.names.is_empty() {
            return Err(refuse(format!(
                "this game data file carries a name table of {} names, and how an item's name table \
                 offset counts into one is not known: no shipped game data file with an interior carries one",
                file.names.len()
            )));
        }
        let lead: [u8; LEAD] = file
            .data
            .get(..LEAD)
            .and_then(|lead| lead.try_into().ok())
            .ok_or_else(|| {
                refuse(format!(
                    "the data block is {} bytes, short of the {LEAD} it opens with",
                    file.data.len()
                ))
            })?;
        let mut spans: Vec<&RelIndexHash> = index.iter().collect();
        spans.sort_by_key(|e| e.offset);
        let mut entries = Vec::with_capacity(spans.len());
        for span in &spans {
            let start = span.offset as usize;
            let body = start
                .checked_add(span.length as usize)
                .and_then(|end| file.data.get(start..end))
                .ok_or_else(|| {
                    refuse(format!(
                        "the item {} lies outside the data block",
                        key_hex(span.hash)
                    ))
                })?;
            let item = GameObjectItem::decode(body)
                .map_err(|e| refuse(format!("the item {}: {e}", key_hex(span.hash))))?;
            entries.push(Entry {
                key: span.hash,
                item,
                body: body.to_vec(),
                hash_fields: Vec::new(),
                pack_fields: Vec::new(),
            });
        }
        let holder = |offset: u32, table: &str| -> Result<(usize, usize)> {
            let at = spans
                .partition_point(|span| span.offset <= offset)
                .checked_sub(1)
                .filter(|&i| {
                    u64::from(offset) + size_of::<u32>() as u64
                        <= u64::from(spans[i].offset) + u64::from(spans[i].length)
                })
                .ok_or_else(|| {
                    refuse(format!(
                        "the {table} table lists {offset:#x}, a word no item holds"
                    ))
                })?;
            Ok((at, (offset - spans[at].offset) as usize))
        };
        for &offset in &file.hash_offsets {
            let (at, field) = holder(offset, "hash")?;
            entries[at].hash_fields.push(field);
        }
        for &offset in &file.pack_offsets {
            let (at, field) = holder(offset, "pack")?;
            entries[at].pack_fields.push(field);
        }
        let laid = Self { lead, entries };
        let packed = laid.pack()?;
        let differs = if packed.data != file.data {
            Some("data block")
        } else if packed.index != *index {
            Some("item table")
        } else if packed.hash_offsets != file.hash_offsets {
            Some("hash table")
        } else if packed.pack_offsets != file.pack_offsets {
            Some("pack table")
        } else {
            None
        };
        if let Some(what) = differs {
            return Err(refuse(format!(
                "this file's {what} is not the one a save lays out from its items, so an edit would \
                 change more than it was asked to"
            )));
        }
        Ok(laid)
    }

    /// Lays the items out: each at the next multiple of its own alignment, the item table by each key's low byte and then the key.
    fn pack(&self) -> Result<Packed> {
        let mut data = self.lead.to_vec();
        let mut index = Vec::with_capacity(self.entries.len());
        let (mut hash_offsets, mut pack_offsets) = (Vec::new(), Vec::new());
        for entry in &self.entries {
            data.resize(data.len().next_multiple_of(entry.item.own_align()), 0);
            let offset = data.len();
            index.push(RelIndexHash {
                hash: entry.key,
                offset: word(offset, "an item")?,
                length: word(entry.body.len(), "an item's end")?,
            });
            for (fields, table) in [
                (&entry.hash_fields, &mut hash_offsets),
                (&entry.pack_fields, &mut pack_offsets),
            ] {
                for field in fields {
                    table.push(word(offset + field, "a listed word")?);
                }
            }
            data.extend_from_slice(&entry.body);
        }
        word(data.len(), "the data block's end")?;
        index.sort_by_key(|e| (e.hash & 0xFF, e.hash));
        if let Some(twice) = index.windows(2).find(|pair| pair[0].hash == pair[1].hash) {
            return Err(refuse(format!(
                "the file holds two items under {}",
                key_hex(twice[0].hash)
            )));
        }
        Ok(Packed {
            data,
            index,
            hash_offsets,
            pack_offsets,
        })
    }

    fn position(&self, key: u32) -> Option<usize> {
        self.entries.iter().position(|entry| entry.key == key)
    }

    fn interior_at(&self, key: u32) -> Result<(usize, &InteriorSettings)> {
        match self.position(key).map(|i| (i, &self.entries[i].item)) {
            Some((i, GameObjectItem::InteriorSettings(interior))) => Ok((i, interior)),
            Some((_, other)) => Err(not_a("an InteriorSettings", key, other)),
            None => Err(refuse(format!(
                "the file holds no interior under {}",
                key_hex(key)
            ))),
        }
    }

    fn room_at(&self, key: u32) -> Result<(usize, &InteriorRoom)> {
        match self.position(key).map(|i| (i, &self.entries[i].item)) {
            Some((i, GameObjectItem::InteriorRoom(room))) => Ok((i, room)),
            Some((_, other)) => Err(not_a("an InteriorRoom", key, other)),
            None => Err(refuse(format!(
                "the file holds no room under {}",
                key_hex(key)
            ))),
        }
    }

    fn refuse_held(&self, key: u32) -> Result<()> {
        match self.position(key) {
            Some(i) => Err(refuse(format!(
                "the file already holds an item under {}, {} {}",
                key_hex(key),
                article(self.entries[i].item.type_name()),
                self.entries[i].item.type_name()
            ))),
            None => Ok(()),
        }
    }

    /// Refuses an interior that lists a room the file does not hold as an `InteriorRoom`.
    fn refuse_absent_rooms(&self, key: u32, interior: &InteriorSettings) -> Result<()> {
        for room in &interior.room {
            let why = match self.position(room.r#ref).map(|i| &self.entries[i].item) {
                Some(GameObjectItem::InteriorRoom(_)) => continue,
                Some(other) => format!(
                    "which is {} {} and not an InteriorRoom",
                    article(other.type_name()),
                    other.type_name()
                ),
                None => "which the file does not hold".into(),
            };
            return Err(refuse(format!(
                "the interior {} lists the room {}, {why}",
                key_hex(key),
                key_hex(room.r#ref)
            )));
        }
        Ok(())
    }

    /// Writes an item of the file again from its typed form, refusing one whose listed words a save would not derive.
    fn replace(&mut self, at: usize, new: Entry) -> Result<()> {
        let old = &self.entries[at];
        let derived = match &old.item {
            GameObjectItem::InteriorSettings(interior) => Entry::interior(old.key, interior)?,
            GameObjectItem::InteriorRoom(room) => Entry::room(old.key, room)?,
            other => return Err(not_a("an interior or a room", old.key, other)),
        };
        if derived.hash_fields != old.hash_fields || !old.pack_fields.is_empty() {
            return Err(refuse(format!(
                "the hash and pack tables list other words of the {} {} than a save derives",
                old.item.type_name(),
                key_hex(old.key)
            )));
        }
        self.entries[at] = new;
        Ok(())
    }

    /// Refuses a removal while an emitter or an alarm of the file names the interior.
    fn refuse_named_interior(&self, key: u32) -> Result<()> {
        let namer = self.entries.iter().find(|entry| match &entry.item {
            GameObjectItem::StaticEmitter(emitter) => emitter.interior_settings == key,
            GameObjectItem::AlarmSettings(alarm) => alarm.interior_settings == key,
            _ => false,
        });
        refuse_named("interior", key, namer)
    }

    /// Refuses a removal while an emitter of the file names the room.
    fn refuse_named_room(&self, key: u32) -> Result<()> {
        let namer = self.entries.iter().find(
            |entry| matches!(&entry.item, GameObjectItem::StaticEmitter(emitter) if emitter.interior_room == key),
        );
        refuse_named("room", key, namer)
    }

    /// The keys of the interiors that list `room`, in data block order.
    fn listing(&self, room: u32) -> Vec<u32> {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(&entry.item, GameObjectItem::InteriorSettings(interior)
                    if interior.room.iter().any(|listed| listed.r#ref == room))
            })
            .map(|entry| entry.key)
            .collect()
    }

    fn remove_room(&mut self, key: u32) -> Result<()> {
        let (at, _) = self.room_at(key)?;
        self.refuse_named_room(key)?;
        for interior_key in self.listing(key) {
            let (i, interior) = self.interior_at(interior_key)?;
            let mut interior = interior.clone();
            interior.room.retain(|listed| listed.r#ref != key);
            self.replace(i, Entry::interior(interior_key, &interior)?)?;
        }
        self.entries.remove(at);
        Ok(())
    }
}

fn article(type_name: &str) -> &'static str {
    if type_name.starts_with(['A', 'E', 'I', 'O', 'U']) {
        "an"
    } else {
        "a"
    }
}

fn not_a(wanted: &str, key: u32, found: &GameObjectItem) -> crate::error::IoError {
    refuse(format!(
        "the item {} is {} {}, not {wanted}",
        key_hex(key),
        article(found.type_name()),
        found.type_name()
    ))
}

fn refuse_named(what: &str, key: u32, namer: Option<&Entry>) -> Result<()> {
    match namer {
        Some(namer) => Err(refuse(format!(
            "the {} {} names the {what} {}, which cannot go while it does",
            namer.item.type_name(),
            key_hex(namer.key),
            key_hex(key)
        ))),
        None => Ok(()),
    }
}

/// The item table of a game data file, refusing any other `rel_type` or a table keyed by name.
fn game_data_index(file: &RelFile) -> Result<&Vec<RelIndexHash>> {
    if file.rel_type != GAME_DATA {
        return Err(refuse(format!(
            "an interior is an item of a game data file, rel_type {GAME_DATA}, and this file is rel_type {}",
            file.rel_type
        )));
    }
    match &file.index {
        RelIndex::Hashed(index) => Ok(index),
        RelIndex::Named(_) => Err(refuse(
            "a game data file keys its items by hash, and this one keys them by name".into(),
        )),
    }
}

impl RelFile {
    /// The item of a game data file under `key`, typed; `None` when the file holds none there.
    fn game_object(&self, key: u32) -> Result<Option<GameObjectItem>> {
        let Some(entry) = game_data_index(self)?.iter().find(|e| e.hash == key) else {
            return Ok(None);
        };
        let start = entry.offset as usize;
        let body = start
            .checked_add(entry.length as usize)
            .and_then(|end| self.data.get(start..end))
            .ok_or_else(|| {
                refuse(format!(
                    "the item {} lies outside the data block",
                    key_hex(key)
                ))
            })?;
        GameObjectItem::decode(body)
            .map(Some)
            .map_err(|e| refuse(format!("the item {}: {e}", key_hex(key))))
    }

    /// The interior under `key`, or `None` when the file holds no item there; refuses an item of another type.
    pub fn interior(&self, key: u32) -> Result<Option<InteriorSettings>> {
        match self.game_object(key)? {
            Some(GameObjectItem::InteriorSettings(interior)) => Ok(Some(*interior)),
            Some(other) => Err(not_a("an InteriorSettings", key, &other)),
            None => Ok(None),
        }
    }

    /// The room under `key`, or `None` when the file holds no item there; refuses an item of another type.
    pub fn interior_room(&self, key: u32) -> Result<Option<InteriorRoom>> {
        match self.game_object(key)? {
            Some(GameObjectItem::InteriorRoom(room)) => Ok(Some(*room)),
            Some(other) => Err(not_a("an InteriorRoom", key, &other)),
            None => Ok(None),
        }
    }

    fn commit(&mut self, laid: &Laid) -> Result<()> {
        let packed = laid.pack()?;
        self.data = packed.data;
        self.index = RelIndex::Hashed(packed.index);
        self.hash_offsets = packed.hash_offsets;
        self.pack_offsets = packed.pack_offsets;
        Ok(())
    }

    /// Adds an interior under `key`; refuses a key the file holds, and a listed room it does not.
    pub fn add_interior(&mut self, key: u32, interior: &InteriorSettings) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        laid.refuse_held(key)?;
        laid.refuse_absent_rooms(key, interior)?;
        laid.entries.push(Entry::interior(key, interior)?);
        self.commit(&laid)
    }

    /// Adds a room under `key` and lists it last in `interior`; refuses a key the file holds, an interior it does not, and a 256th room.
    pub fn add_interior_room(
        &mut self,
        interior: u32,
        key: u32,
        room: &InteriorRoom,
    ) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        if laid.position(interior).is_none() {
            return Err(refuse(format!(
                "the room {} names the interior {}, which the file does not hold",
                key_hex(key),
                key_hex(interior)
            )));
        }
        let (at, listed) = laid.interior_at(interior)?;
        laid.refuse_held(key)?;
        let mut listed = listed.clone();
        listed.room.push(Room { r#ref: key });
        laid.replace(at, Entry::interior(interior, &listed)?)?;
        laid.entries.push(Entry::room(key, room)?);
        self.commit(&laid)
    }

    /// Copies the interior `from` under `to`, and each room it lists under the key `rooms` pairs with it; refuses a room left unpaired or paired twice, a pair it does not list, and a new key the file holds or two items of the copy share.
    pub fn copy_interior(&mut self, from: u32, to: u32, rooms: &[(u32, u32)]) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        let (_, source) = laid.interior_at(from)?;
        let mut copy = source.clone();
        if let Some((stray, _)) = rooms
            .iter()
            .find(|(old, _)| copy.room.iter().all(|listed| listed.r#ref != *old))
        {
            return Err(refuse(format!(
                "the interior {} does not list the room {}, so no copy of it is made",
                key_hex(from),
                key_hex(*stray)
            )));
        }
        if let Some((twice, _)) = rooms
            .iter()
            .enumerate()
            .find(|(i, (old, _))| rooms[..*i].iter().any(|(earlier, _)| earlier == old))
            .map(|(_, pair)| pair)
        {
            return Err(refuse(format!(
                "the copy was given two keys for the room {}",
                key_hex(*twice)
            )));
        }
        let new_keys: Vec<u32> = std::iter::once(to)
            .chain(rooms.iter().map(|(_, new)| *new))
            .collect();
        if let Some(twice) = new_keys
            .iter()
            .enumerate()
            .find(|(i, key)| new_keys[..*i].contains(key))
            .map(|(_, key)| key)
        {
            return Err(refuse(format!(
                "the copy was given the key {} for two of its items",
                key_hex(*twice)
            )));
        }
        let mut added: Vec<Entry> = Vec::new();
        let mut copied: Vec<u32> = Vec::new();
        for listed in &mut copy.room {
            let old = listed.r#ref;
            let Some(&(_, new)) = rooms.iter().find(|(key, _)| *key == old) else {
                return Err(refuse(format!(
                    "the interior {} lists the room {}, and the copy was given no key for it",
                    key_hex(from),
                    key_hex(old)
                )));
            };
            listed.r#ref = new;
            // An interior that lists a room twice lists its copy twice, and the room is copied once.
            if !copied.contains(&old) {
                copied.push(old);
                added.push(Entry::room(new, laid.room_at(old)?.1)?);
            }
        }
        added.insert(0, Entry::interior(to, &copy)?);
        for entry in added {
            laid.refuse_held(entry.key)?;
            laid.entries.push(entry);
        }
        self.commit(&laid)
    }

    /// Writes the interior under `key` again from `interior`; refuses a listed room the file does not hold.
    pub fn set_interior(&mut self, key: u32, interior: &InteriorSettings) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        let (at, _) = laid.interior_at(key)?;
        laid.refuse_absent_rooms(key, interior)?;
        laid.replace(at, Entry::interior(key, interior)?)?;
        self.commit(&laid)
    }

    /// Writes the room under `key` again from `room`.
    pub fn set_interior_room(&mut self, key: u32, room: &InteriorRoom) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        let (at, _) = laid.room_at(key)?;
        laid.replace(at, Entry::room(key, room)?)?;
        self.commit(&laid)
    }

    /// Removes the room under `key` and its place in every interior that lists it; refuses while a static emitter names it.
    pub fn remove_interior_room(&mut self, key: u32) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        laid.remove_room(key)?;
        self.commit(&laid)
    }

    /// Removes the interior under `key` and each room only it lists; refuses while a static emitter or an alarm names one of them.
    pub fn remove_interior(&mut self, key: u32) -> Result<()> {
        let mut laid = Laid::unpack(self)?;
        let (_, interior) = laid.interior_at(key)?;
        laid.refuse_named_interior(key)?;
        let mut own: Vec<u32> = Vec::new();
        for listed in &interior.room {
            let room = listed.r#ref;
            // A listed key the file holds as another type is not this interior's to take out.
            let held = laid.room_at(room).is_ok();
            if held && !own.contains(&room) && laid.listing(room) == [key] {
                own.push(room);
            }
        }
        for room in own {
            laid.remove_room(room)?;
        }
        let (at, _) = laid.interior_at(key)?;
        laid.entries.remove(at);
        self.commit(&laid)
    }
}
