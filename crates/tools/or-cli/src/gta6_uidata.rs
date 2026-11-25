//! GTA VI's interface trees, radio and dialogue from a scan: `rpf scan-open` of an RBF0 body as
//! its strict element tree, `rpf scan-radio` of an archive's stations with their tracks' audio
//! streams, `rpf scan-dialogue` of archives' dialogue lines with their subtitles, and the
//! comments `awc export --dialogue` writes from that listing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;

use or_io::container::rpf::{
    read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Scan, Rpf9ScanFind, Rpf9ZlibRun,
};
use or_io::gta6::audio_data::{AudioData, AudioDataKind};
use or_io::gta6::audio_radio::{
    station_lists, stations, streamed_sounds, track_texts, Station, StationList, StreamedSound,
    TrackText,
};
use or_io::gta6::dialogue::{plain, subtitles_in, Dialogue};
use or_io::gta6::text_language::{registered, LanguageArchives, Scanned};
use or_io::gta6::ui_tree::{self, Element, Node, Text, UiTree};

pub(crate) const RADIO_USAGE: &str =
    "usage: rpf scan-radio <archive.rpf>... [--tracks] [--language <name>] [--limit <bytes>]";
pub(crate) const DIALOGUE_USAGE: &str =
    "usage: rpf scan-dialogue <archive.rpf>... [--out <file.tsv>] [--language <name>] [--limit <bytes>]";

/// A change set that registers languages is a few kilobytes: a longer XML body is not read for one.
const CHANGE_SET_LIMIT: u64 = 1 << 20;

/// The strict tree of an RBF0 body, or the note said where the strict reader refuses it.
fn strict(bytes: &[u8]) -> Result<UiTree, String> {
    ui_tree::read(bytes).map_err(|e| {
        format!("the strict RBF0 reader does not read this body ({e}): what follows is the lenient reader's tree alone")
    })
}

/// An RBF0 body as its strict tree: its counts, then each element, attribute, value and text. A
/// body the strict reader refuses is said, and never hidden: the lenient reader prints it next.
pub(crate) fn print_tree(bytes: &[u8]) -> Result<(), String> {
    let tree = match strict(bytes) {
        Ok(tree) => tree,
        Err(note) => {
            println!("  element tree  {note}");
            return Ok(());
        }
    };
    let counts = tree.counts();
    println!(
        "  element tree  root {}, {} elements to depth {}, {} attributes, {} values, {} texts; \
         integers and floats of {} bytes",
        tree.root.name,
        counts.elements,
        counts.depth,
        counts.attributes,
        counts.values,
        counts.texts,
        if tree.wide { 8 } else { 4 }
    );
    let mut out = String::new();
    element_lines(&tree.root, 2, &mut out);
    print!("{out}");
    Ok(())
}

/// `element` and what it holds, a line each, indented by `depth`.
// The reader bounds an element's depth, so this recursion is bounded too.
fn element_lines(element: &Element, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&format!("{pad}{}", element.name));
    for (name, value) in &element.attributes {
        out.push_str(&format!(" {name}={value}"));
    }
    out.push('\n');
    for node in &element.content {
        match node {
            Node::Element(child) => element_lines(child, depth + 1, out),
            Node::Value(name, value) => {
                out.push_str(&format!("{pad}  {name}: {} {value}\n", value.kind()))
            }
            Node::Text(Text::Utf8(text)) => out.push_str(&format!("{pad}  text {text:?}\n")),
            Node::Text(Text::Bytes(bytes)) => out.push_str(&format!(
                "{pad}  text of {} bytes, not UTF-8 before one NUL\n",
                bytes.len()
            )),
        }
    }
}

/// Scanned bodies, each with its content.
type Found = Vec<(Rpf9ScanFind, Rpf9Content)>;
/// The texts read of each key.
type Read = BTreeMap<u32, Vec<Subtitle>>;

/// The zlib bodies of `wanted` content the scan of `file` finds, and the file's language archives.
fn finds(
    file: &fs::File,
    wanted: &[Rpf9Content],
    cancelled: &dyn Fn() -> bool,
) -> Result<(Found, LanguageArchives), String> {
    let mut out = Vec::new();
    let mut scanned = Vec::new();
    for find in Rpf9Scan::new(file, 0..u64::MAX).map_err(|e| e.to_string())? {
        if cancelled() {
            return Err("cancelled".to_string());
        }
        let find = find.map_err(|e| e.to_string())?;
        scanned.push((find.offset, Scanned::of(&find)));
        match find.kind {
            Rpf9BodyKind::Zlib(run) if wanted.contains(&run.content) => {
                out.push((find, run.content))
            }
            _ => {}
        }
    }
    Ok((out, LanguageArchives::of(scanned)))
}

/// The body at `offset` of `file`; a body the reader refuses is said and stepped over, since
/// the scan found its start and not its end.
fn body_at(
    file: &fs::File,
    path: &str,
    offset: u64,
    limit: u64,
) -> Option<(Rpf9ScanFind, Vec<u8>)> {
    match read_rpf9_body(file, offset, limit) {
        Ok(read) => Some(read),
        Err(e) => {
            eprintln!("  {path} at {offset:#X}: {e}");
            None
        }
    }
}

fn open(path: &str) -> Result<fs::File, String> {
    fs::File::open(path).map_err(|e| format!("open({path}): {e}"))
}

/// The words of `args` that are no `flag`, and whether `flag` was among them.
fn without(args: &[String], flag: &str) -> (bool, Vec<String>) {
    let rest: Vec<String> = args.iter().filter(|a| *a != flag).cloned().collect();
    (rest.len() < args.len(), rest)
}

/// A language archive's place among its file's, as a table with no registered name is told.
fn place(ordinal: usize) -> String {
    format!("#{}", ordinal + 1)
}

/// The text tables of the archives read, each with its language, and the languages a change
/// set among the archives registers.
#[derive(Default)]
struct Texts {
    /// Each table's archive, offset, and its language archive's place among its file's.
    tables: Vec<(String, u64, Option<usize>, usize)>,
    registered: Vec<String>,
}

impl Texts {
    /// Notes the text tables and reads the change sets among `found`, the bodies of `path`.
    fn note(
        &mut self,
        file: &fs::File,
        path: &str,
        found: &[(Rpf9ScanFind, Rpf9Content)],
        languages: &LanguageArchives,
    ) {
        for (find, content) in found {
            match content {
                Rpf9Content::TextTable => self.tables.push((
                    path.to_string(),
                    find.offset,
                    languages.ordinal(find.offset),
                    languages.len(),
                )),
                Rpf9Content::Xml
                    if self.registered.is_empty()
                        && find.inflated_len.is_none_or(|len| len <= CHANGE_SET_LIMIT) =>
                {
                    if let Ok((_, bytes)) = read_rpf9_body(file, find.offset, CHANGE_SET_LIMIT) {
                        self.registered = registered(&String::from_utf8_lossy(&bytes));
                    }
                }
                _ => {}
            }
        }
    }

    /// The language of a table: its registered name where its file holds as many language
    /// archives as are registered, else its archive's place, else nothing.
    fn language(&self, ordinal: Option<usize>, of: usize) -> String {
        match ordinal {
            Some(n) if of == self.registered.len() => self.registered[n].clone(),
            Some(n) => place(n),
            None => String::new(),
        }
    }

    /// The language read where none is asked for: the first registered, else the first archive's.
    fn first_language(&self) -> String {
        (self.registered.first().cloned()).unwrap_or_else(|| place(0))
    }

    /// The texts of `wanted` keys in the tables of `language`, or of every table where it is
    /// `None`: each with its table and that table's language. Returns how many tables were read.
    fn read(
        &self,
        wanted: &BTreeSet<u32>,
        language: Option<&str>,
        limit: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(Read, usize), String> {
        let mut texts = Read::new();
        let mut read = 0;
        for (path, offset, ordinal, of) in &self.tables {
            if cancelled() {
                return Err("cancelled".to_string());
            }
            let spoken = self.language(*ordinal, *of);
            if language.is_some_and(|l| !l.eq_ignore_ascii_case(&spoken)) {
                continue;
            }
            let Some((find, bytes)) = body_at(&open(path)?, path, *offset, limit) else {
                continue;
            };
            let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(resource),
                ..
            }) = find.kind
            else {
                continue;
            };
            let table = match or_io::gta6::text::read(resource.sections(&bytes).0) {
                Ok((table, _)) => table,
                Err(e) => {
                    eprintln!("  {path} at {offset:#X}: {e}");
                    continue;
                }
            };
            read += 1;
            let name = std::path::Path::new(path.as_str())
                .file_name()
                .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
            for (hash, text) in subtitles_in(&table, wanted) {
                texts.entry(hash).or_default().push(Subtitle {
                    table: format!("{name}@{offset:#X}"),
                    language: spoken.clone(),
                    text: text.to_string(),
                });
            }
        }
        Ok((texts, read))
    }

    /// How the languages were told, as a listing says it.
    fn said(&self) -> String {
        match self.registered.len() {
            0 => "no change set among the archives registers the text archives' languages, so a \
                  table's language is its archive's place among its file's (#1, #2, ...)"
                .to_string(),
            n => format!(
                "a change set registers {n} languages ({}); the n-th language archive of a file \
                 that holds {n} is the n-th registered",
                self.registered.join(", ")
            ),
        }
    }
}

/// A text a table holds for a key.
struct Subtitle {
    table: String,
    language: String,
    text: String,
}

/// What a music body states of the radio.
struct Music {
    path: String,
    offset: u64,
    stations: Vec<Station>,
    lists: Vec<StationList>,
    texts: BTreeMap<u32, TrackText>,
}

/// `key` as a listing names it: its text where one was read, else its hash.
fn named(texts: &BTreeMap<u32, Vec<Subtitle>>, key: u32) -> String {
    match texts.get(&key).and_then(|found| found.first()) {
        Some(found) => format!("{:?} (hash_{key:08X})", plain(&found.text)),
        None => format!("hash_{key:08X}"),
    }
}

pub(crate) fn cmd_scan_radio(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let (language, args) = super::rpf::split_value(&args, "--language")?;
    let (tracks, paths) = without(&args, "--tracks");
    if paths.is_empty() || paths.iter().any(|p| p.starts_with("--")) {
        return Err(RADIO_USAGE.to_string());
    }
    let mut found: Vec<Music> = Vec::new();
    let mut sounds: BTreeMap<u32, StreamedSound> = BTreeMap::new();
    let mut unread = 0;
    let mut texts = Texts::default();
    let cancelled = super::jobs::on_interrupt();
    for path in &paths {
        let file = open(path)?;
        let wanted = [
            Rpf9Content::AudioData,
            Rpf9Content::TextTable,
            Rpf9Content::Xml,
        ];
        let (bodies, languages) = finds(&file, &wanted, &|| cancelled.is_set())?;
        texts.note(&file, path, &bodies, &languages);
        for (find, content) in &bodies {
            if *content != Rpf9Content::AudioData {
                continue;
            }
            let offset = find.offset;
            let Some((_, bytes)) = body_at(&file, path, offset, limit) else {
                continue;
            };
            let read = AudioData::read(&bytes).map_err(|e| e.to_string());
            let read = read.and_then(|data| match data.kind {
                AudioDataKind::Music => {
                    let music = Music {
                        path: path.clone(),
                        offset,
                        stations: stations(&data).map_err(|e| e.to_string())?,
                        lists: station_lists(&data).map_err(|e| e.to_string())?,
                        texts: track_texts(&data).map_err(|e| e.to_string())?,
                    };
                    if !music.stations.is_empty() {
                        found.push(music);
                    }
                    Ok(())
                }
                AudioDataKind::Sounds => {
                    let (read, skipped) = streamed_sounds(&data).map_err(|e| e.to_string())?;
                    sounds.extend(read);
                    unread += skipped;
                    Ok(())
                }
                _ => Ok(()),
            });
            if let Err(e) = read {
                eprintln!("  {path} at {offset:#X}: {e}");
            }
        }
    }
    // A station's key is the key of its name in the text tables.
    let keys: BTreeSet<u32> = (found.iter())
        .flat_map(|m| m.stations.iter().map(|s| s.key))
        .collect();
    let language = language.unwrap_or_else(|| texts.first_language());
    let (names, tables) = texts.read(&keys, Some(language.as_str()), limit, &|| {
        cancelled.is_set()
    })?;
    let count: usize = found.iter().map(|m| m.stations.len()).sum();
    println!(
        "  radio         {count} station(s), {} with a name in the {tables} text table(s) read of \
         the language {language}; {} streamed sound(s), {unread} more whose layout is not read",
        names.len(),
        sounds.len()
    );
    println!("  languages     {}", texts.said());
    println!(
        "  not read      here, a track's shown title and artist, keyed by the track id a marker \
         of its audio stream states (the app's index reads them): a track's text names them as \
         identifiers; what a list's flags say of its play order; a track's start time, which is \
         not stored"
    );
    for music in &found {
        for list in &music.lists {
            println!(
                "  station list  hash_{:08X}  {} station(s), {}   ({} at {:#X})",
                list.key,
                list.stations.len(),
                match list.ordered {
                    true => "every station in the order it states",
                    false => "a map's, in its keys' order",
                },
                music.path,
                music.offset
            );
            if tracks {
                for station in &list.stations {
                    println!("    station     {}", named(&names, *station));
                }
            }
        }
        for station in &music.stations {
            print_station(station, music, &sounds, &names, tracks);
        }
    }
    Ok(())
}

/// A station's line, and with `tracks` each of its lists and tracks.
fn print_station(
    station: &Station,
    music: &Music,
    sounds: &BTreeMap<u32, StreamedSound>,
    names: &BTreeMap<u32, Vec<Subtitle>>,
    tracks: bool,
) {
    let joined: Vec<_> = station.tracks(sounds).collect();
    let played: Vec<&StreamedSound> = joined.iter().filter_map(|(_, s)| *s).collect();
    let ms: u64 = played.iter().map(|s| u64::from(s.duration_ms)).sum();
    let streams: BTreeSet<u32> = (played.iter().copied()).flat_map(|s| s.streams()).collect();
    let titled = (joined.iter())
        .filter(|(t, _)| t.text.is_some_and(|key| music.texts.contains_key(&key)))
        .count();
    println!(
        "  station       {}  band {}, {} list(s), {} track(s): {} with a streamed sound, {titled} \
         with a title, {ms} ms in {} stream(s)   ({} at {:#X})",
        named(names, station.key),
        station.band,
        station.lists.len(),
        joined.len(),
        played.len(),
        streams.len(),
        music.path,
        music.offset
    );
    if !tracks {
        return;
    }
    for list in &station.lists {
        let Some(listed) = &list.tracks else {
            println!("    list        hash_{:08X}  not in this body", list.key);
            continue;
        };
        println!(
            "    list        hash_{:08X}  {} track(s), category {}, flags {}",
            list.key,
            listed.len(),
            list.category
                .map_or_else(|| "none".to_string(), |key| format!("hash_{key:08X}")),
            list.flags
                .map_or_else(|| "none".to_string(), |flags| format!("{flags:#010X}")),
        );
        for track in listed {
            let sound = track.sound.and_then(|key| sounds.get(&key));
            let ids: Vec<String> = (sound.into_iter())
                .flat_map(|s| s.streams())
                .map(|id| format!("{id:08X}"))
                .collect();
            let said = match (track.sound, sound) {
                (None, _) => "not in this body".to_string(),
                (Some(key), None) => format!("sound hash_{key:08X}, no streamed sound read"),
                (Some(key), Some(s)) => format!(
                    "sound hash_{key:08X}  {} ms  streams {}",
                    s.duration_ms,
                    if ids.is_empty() {
                        "none named".to_string()
                    } else {
                        ids.join(" ")
                    }
                ),
            };
            let title = match track.text.and_then(|key| music.texts.get(&key)) {
                Some(text) if text.artists.is_empty() => format!("  {}", text.title),
                Some(text) => format!("  {} by {}", text.title, text.artists.join(", ")),
                None => String::new(),
            };
            println!("      track     hash_{:08X}{title}  {said}", track.key);
        }
    }
}

/// A field of a tab-separated line: its backslashes, tabs and line ends escaped.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// What [`escaped`] made of a text, undone.
fn unescaped(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

const HEADER: &str = "wave\tstream\tkey\tlabel\tspeaker\ttable\tlanguage\ttext";

/// The listing's lines: for each line of `dialogue`, one per table that holds its text, in the
/// order the tables were read, or one with no table where none does.
fn dialogue_rows(dialogue: &Dialogue, texts: &BTreeMap<u32, Vec<Subtitle>>) -> Vec<String> {
    let mut rows = vec![HEADER.to_string()];
    for line in dialogue.lines() {
        let head = format!(
            "{}\t{:08X}\t{}\t{}\t{}",
            line.wave(),
            line.stream_id(),
            line.key,
            line.label,
            line.speaker().unwrap_or("")
        );
        match texts.get(&line.label_hash()) {
            Some(found) => rows.extend(found.iter().map(|s| {
                let text = escaped(&s.text);
                format!("{head}\t{}\t{}\t{text}", s.table, s.language)
            })),
            None => rows.push(format!("{head}\t\t\t")),
        }
    }
    rows
}

pub(crate) fn cmd_scan_dialogue(args: &[String]) -> Result<(), String> {
    let (limit, args) = super::rpf::split_limit(args)?;
    let (out, args) = super::rpf::split_value(&args, "--out")?;
    let (language, paths) = super::rpf::split_value(&args, "--language")?;
    if paths.is_empty() || paths.iter().any(|p| p.starts_with("--")) {
        return Err(DIALOGUE_USAGE.to_string());
    }
    // The keys and their texts lie in different archives: every key is read before any table.
    let mut dialogue = Dialogue::default();
    let mut texts = Texts::default();
    let mut bodies = 0;
    let cancelled = super::jobs::on_interrupt();
    for path in &paths {
        let file = open(path)?;
        let wanted = [Rpf9Content::Pso, Rpf9Content::TextTable, Rpf9Content::Xml];
        let (found, languages) = finds(&file, &wanted, &|| cancelled.is_set())?;
        texts.note(&file, path, &found, &languages);
        for (find, content) in &found {
            if *content != Rpf9Content::Pso {
                continue;
            }
            if let Some((_, bytes)) = body_at(&file, path, find.offset, limit) {
                bodies += usize::from(dialogue.add(&bytes) > 0);
            }
        }
    }
    let wanted: BTreeSet<u32> = dialogue.lines().map(|l| l.label_hash()).collect();
    let (texts_of, read) =
        texts.read(&wanted, language.as_deref(), limit, &|| cancelled.is_set())?;
    let rows = dialogue_rows(&dialogue, &texts_of);
    let lines = dialogue.lines().count();
    let with_text = (dialogue.lines())
        .filter(|l| texts_of.contains_key(&l.label_hash()))
        .count();
    let said = format!(
        "  dialogue      {lines} line(s) in {bodies} conversation bodies; {with_text} with a text \
         in some of {read} text table(s) read{}\n  languages     {}",
        match &language {
            Some(language) => format!(" of the language {language}"),
            None => String::new(),
        },
        texts.said()
    );
    match out {
        Some(out) => {
            fs::write(&out, rows.join("\n") + "\n").map_err(|e| format!("write({out}): {e}"))?;
            println!("{said}");
            println!(
                "  wrote {out}; `awc export --dialogue {out}` writes it into Ogg Opus comments"
            );
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            for row in &rows {
                writeln!(stdout, "{row}").map_err(|e| format!("write: {e}"))?;
            }
            eprintln!("{said}");
        }
    }
    Ok(())
}

/// The comments of each audio stream the listing `scan-dialogue` wrote to `path` names: the
/// line's name, label and speaker, and its text in the first table listed that has one.
pub(crate) fn dialogue_comments(
    path: &str,
) -> Result<BTreeMap<u32, Vec<(String, String)>>, String> {
    let listing = fs::read_to_string(path).map_err(|e| format!("read({path}): {e}"))?;
    let mut lines = listing.lines();
    if lines.next() != Some(HEADER) {
        return Err(format!(
            "{path} is no listing `rpf scan-dialogue --out` wrote: its first line is not the \
             column names"
        ));
    }
    let mut out: BTreeMap<u32, Vec<(String, String)>> = BTreeMap::new();
    for (n, row) in lines.enumerate() {
        let fields: Vec<&str> = row.split('\t').collect();
        let (&[wave, _, _, label, speaker, _, _, text], Ok(id)) = (
            fields.as_slice(),
            u32::from_str_radix(fields.get(1).copied().unwrap_or(""), 16),
        ) else {
            return Err(format!(
                "{path}: line {} is not eight columns with a stream id",
                n + 2
            ));
        };
        let comments = out.entry(id).or_insert_with(|| {
            let mut named = vec![("TITLE".to_string(), wave.to_string())];
            named.push(("LABEL".to_string(), label.to_string()));
            if !speaker.is_empty() {
                named.push(("SPEAKER".to_string(), speaker.to_string()));
            }
            named
        });
        if !text.is_empty() && !comments.iter().any(|(field, _)| field == "SUBTITLE") {
            comments.push(("SUBTITLE".to_string(), plain(&unescaped(text))));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::dialogue::DialogueLine;

    fn body(stated: &[(&str, &str)]) -> Vec<u8> {
        let mut out = vec![0];
        for (key, label) in stated {
            out.extend(key.as_bytes());
            out.push(0);
            out.extend(label.as_bytes());
            out.extend([0, 0]);
        }
        out
    }

    #[test]
    fn the_listing_states_each_line_per_table_and_gives_back_its_comments() {
        let mut dialogue = Dialogue::default();
        dialogue.add(&body(&[
            ("MIS1AU_AB12", "MIS1_JD_1"),
            ("MIS1AU_AB12", "MIS1_90s_2"),
        ]));
        let lines: Vec<DialogueLine> = dialogue.lines().collect();
        let mut texts = BTreeMap::new();
        let tabled = |table: &str, language: &str, text: &str| Subtitle {
            table: table.to_string(),
            language: language.to_string(),
            text: text.to_string(),
        };
        texts.insert(
            lines[0].label_hash(),
            vec![
                tabled("a.rpf@0x1000", "northern", "~z~Hello,\tyou.~n~Back\\slash"),
                tabled("a.rpf@0x2000", "#2", "Bonjour."),
            ],
        );
        let rows = dialogue_rows(&dialogue, &texts);
        let id = |i: usize| format!("{:08X}", lines[i].stream_id());
        assert_eq!(
            rows,
            [
                HEADER.to_string(),
                format!(
                    "MIS1AU_AB12_01\t{}\tMIS1AU_AB12\tMIS1_JD_1\tJD\ta.rpf@0x1000\tnorthern\t~z~Hello,\\tyou.~n~Back\\\\slash",
                    id(0)
                ),
                format!(
                    "MIS1AU_AB12_01\t{}\tMIS1AU_AB12\tMIS1_JD_1\tJD\ta.rpf@0x2000\t#2\tBonjour.",
                    id(0)
                ),
                format!("MIS1AU_AB12_02\t{}\tMIS1AU_AB12\tMIS1_90s_2\t\t\t\t", id(1)),
            ]
        );

        let scratch = tempfile::tempdir().expect("a folder");
        let dir = scratch.path().to_path_buf();
        fs::create_dir_all(&dir).expect("a folder");
        let path = dir.join("dialogue.tsv");
        fs::write(&path, rows.join("\n") + "\n").expect("written");
        let comments = dialogue_comments(&path.to_string_lossy()).expect("read back");
        let comment = |f: &str, v: &str| (f.to_string(), v.to_string());
        assert_eq!(
            comments[&lines[0].stream_id()],
            [
                comment("TITLE", "MIS1AU_AB12_01"),
                comment("LABEL", "MIS1_JD_1"),
                comment("SPEAKER", "JD"),
                comment("SUBTITLE", "Hello,\tyou. Back\\slash"),
            ]
        );
        assert_eq!(
            comments[&lines[1].stream_id()],
            [
                comment("TITLE", "MIS1AU_AB12_02"),
                comment("LABEL", "MIS1_90s_2"),
            ]
        );
        assert_eq!(comments.len(), 2);

        fs::write(&path, "not a listing\n").expect("written");
        assert!(dialogue_comments(&path.to_string_lossy()).is_err());
        fs::write(&path, format!("{HEADER}\nonly\ttwo\n")).expect("written");
        let refused = dialogue_comments(&path.to_string_lossy()).expect_err("refused");
        assert!(refused.contains("line 2"), "{refused}");
        fs::remove_dir_all(&dir).expect("removed");
    }

    #[test]
    fn a_table_s_language_is_its_registered_name_or_its_archive_s_place() {
        let mut texts = Texts::default();
        assert_eq!(texts.language(Some(1), 3), "#2");
        assert_eq!(texts.language(None, 3), "");
        assert_eq!(texts.first_language(), "#1");
        assert!(texts.said().contains("no change set"));
        texts.registered = ["northern", "eastern", "southern"]
            .map(String::from)
            .to_vec();
        assert_eq!(texts.language(Some(1), 3), "eastern");
        // A file that holds another count of language archives is not given the names.
        assert_eq!(texts.language(Some(1), 2), "#2");
        assert_eq!(texts.first_language(), "northern");
        assert!(texts.said().contains("northern, eastern, southern"));
        let none = BTreeMap::new();
        assert_eq!(named(&none, 0x1234), "hash_00001234");
        let found = BTreeMap::from([(
            0x1234,
            vec![Subtitle {
                table: String::new(),
                language: String::new(),
                text: "~z~A Station".into(),
            }],
        )]);
        assert_eq!(named(&found, 0x1234), "\"A Station\" (hash_00001234)");
    }

    #[test]
    fn an_rbf0_body_prints_as_its_tree_and_a_broken_one_is_said_and_not_refused() {
        // Root, one attribute, a text, a child with a value: wide values.
        let mut doc = b"RBF0\xFB\xFF\x01\x00".to_vec();
        let named = |doc: &mut Vec<u8>, index: u8, kind: u8, name: &str| {
            doc.extend([index, kind]);
            doc.extend((name.len() as u16).to_le_bytes());
            doc.extend(name.as_bytes());
        };
        named(&mut doc, 0, 0x00, "Screen");
        doc.extend([0, 0, 0, 0, 1, 0]);
        named(&mut doc, 1, 0x60, "Id");
        doc.extend([4, 0]);
        doc.extend(b"main");
        doc.extend([0xFD, 0xFF, 3, 0, 0, 0]);
        doc.extend(b"hi\0");
        named(&mut doc, 2, 0x00, "Child");
        doc.extend([0; 6]);
        named(&mut doc, 3, 0x10, "Order");
        doc.extend(7u64.to_le_bytes());
        doc.extend([0xFF, 0xFF, 0xFF, 0xFF]);

        let tree = strict(&doc).expect("read");
        let mut out = String::new();
        element_lines(&tree.root, 0, &mut out);
        assert_eq!(
            out,
            "Screen Id=\"main\"\n  text \"hi\"\n  Child\n    Order: integer 7\n"
        );
        assert!(print_tree(&doc).is_ok());
        // A body the strict reader refuses is left to the lenient reader, under a note.
        let note = strict(&doc[..doc.len() - 2]).expect_err("refused");
        assert!(note.contains("strict RBF0 reader"), "{note}");
        assert!(note.contains("lenient reader's tree"), "{note}");
        assert!(print_tree(&doc[..doc.len() - 2]).is_ok());
    }

    #[test]
    fn a_field_escapes_and_reads_back() {
        for text in ["plain", "a\tb\nc\rd", "back\\slash\\t", "", "é ~n~ 語"] {
            let field = escaped(text);
            assert!(!field.contains(['\t', '\n', '\r']), "{field:?}");
            assert_eq!(unescaped(&field), text);
        }
    }
}
