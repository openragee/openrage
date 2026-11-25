//! A GTA VI text table's facts, and GTA VI audio game data's items as the `.rel` view lists them.

use or_io::gta6::audio_data::{AudioDataKind, AudioFields, WaveReference};
use or_io::gta6::audio_links::{BankLink, ItemLayout};
use or_io::gta6::text::TextItem;

use crate::state::gta6_audio_links::{pool_names, Linked};

use super::*;

#[path = "gta6_audio_slots.rs"]
mod slots;

/// The audio game data row of the capability table: read-only, its items listed by class.
pub(crate) const AUDIO_DATA_CAPABILITIES: capabilities::FormatCapabilities =
    capabilities::FormatCapabilities {
        inspector: Some(rows::gta6_audio_data),
        ..capabilities::FormatCapabilities::none(DocumentKind::Gta6AudioData)
    };

/// Rows a class's list shows before it scrolls.
const GROUP_ROWS: f32 = 12.0;

pub(super) fn draw_text_table(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let language = (state.document(doc_id).and_then(|d| d.path.as_deref()))
        .and_then(|path| crate::state::gta6_dialogue::table_language(state, path));
    let Some(DocPayload::Gta6Text(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    ui.heading(tr!("GTA VI Text Table"));
    ui.separator();
    let sets = (doc.table.entries.iter())
        .filter(|e| matches!(e.item, TextItem::Variants(_)))
        .count();
    grid(ui, "gta6_text_summary", |ui| {
        row(ui, tr!("Groups"), &doc.table.groups.to_string());
        row(ui, tr!("Keys"), &doc.table.entries.len().to_string());
        row(ui, tr!("Texts"), &doc.rows.len().to_string());
        row(ui, tr!("Keys with grammatical variants"), &sets.to_string());
        if let Some(language) = &language {
            row(ui, tr!("Language"), language);
        }
    });
    if language.is_some() {
        ui.label(
            egui::RichText::new(tr!("A table states no language: it is the one its archive is registered for, the n-th language archive of its file being the n-th a change set registers. A language shown as # and a number is that place alone: index the open scans, with the scan of the archive whose change set registers the languages open, to name it."))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    }
    ui.add_space(crate::ui::theme::SPACING_SNUG);
    ui.label(
        egui::RichText::new(tr!("A key is the hash of a lower-cased label, shown by name where a known name or the search hashes to it. Texts edit in the centre panel; Save As writes the table as a loose resource, laid out as OpenRage lays it out and not as the game's, or as GTA V's text table under a .gxt2 name."))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
}

/// What a click in the audio game data view asks for.
enum Asked {
    Select(usize),
    Link,
    Open(std::path::PathBuf, u64, u32),
}

pub(super) fn draw_audio_data(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    if !state
        .document(doc_id)
        .is_some_and(crate::state::Document::is_from_rpf9_table)
    {
        super::gta6_dialogue::draw_radio(ui, state, doc_id);
    }
    let Some(asked) = draw_audio_items(ui, state, doc_id) else {
        return;
    };
    match asked {
        Asked::Select(i) => {
            if let Some(DocPayload::Gta6AudioData(doc)) =
                state.document_mut(doc_id).map(|d| &mut d.payload)
            {
                doc.selected = Some(i);
            }
        }
        Asked::Link => {
            if let Err(why) = state.link_gta6_audio(doc_id) {
                state.show_error_toast(why);
            }
        }
        Asked::Open(archive, container, stream) => {
            state.open_gta6_stream(&archive, container, stream);
        }
    }
}

/// What the scan holds for the body, as the last look for it found.
fn draw_links(ui: &mut egui::Ui, linked: Option<&Linked>, running: bool) -> bool {
    match linked {
        _ if running => {
            // Drawn again until the look ends, so its result shows without a click.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
            ui.label(tr!(
                "Looking in the scan for what this body links to; the job list shows how far."
            ));
        }
        None => {}
        Some(Linked::Waves {
            banks,
            containers,
            linked,
            fields,
            ..
        }) => {
            let absent = (banks.values()).filter(|b| **b == BankLink::Absent).count();
            ui.label(tr!(
                "{linked} of {fields} wave fields lie in the audio container their bank stands for, among the {containers} the scan found; no container holds a wave of {absent} of {banks} banks, whose containers are not among the bodies the scan found.",
                linked = linked,
                fields = fields,
                containers = containers,
                absent = absent,
                banks = banks.len()
            ));
        }
        Some(Linked::Names { pool, count }) => {
            ui.label(tr_n!(
                "{count} name read from the name pool at {at}.",
                "{count} names read from the name pool at {at}.",
                *count,
                at = format!("{pool:#X}")
            ));
        }
        Some(Linked::NoPool) => {
            ui.label(tr!("No body after it in the scan is its name pool: none holds a name for each of its items that hashes to the item's key."));
        }
        Some(Linked::Stopped) => {
            ui.label(tr!("Stopped before it finished"));
        }
        Some(Linked::Failed(why)) => {
            ui.colored_label(
                theme::WARNING,
                tr!("What it links to was not looked for: {why}", why = why),
            );
        }
    }
    !running
        && ui
            .button(tr!("Find what it links to in the scan"))
            .on_hover_text(tr!("Reads the stream tables of the audio containers the open scan of its archive found, or the bodies after an audio configuration body for its name pool, as a job."))
            .clicked()
}

/// A wave field's link: the stream to open, or why there is none.
fn draw_wave_link(
    ui: &mut egui::Ui,
    linked: Option<&Linked>,
    wave: &WaveReference,
) -> Option<(u64, u32)> {
    let Some(Linked::Waves { banks, found, .. }) = linked else {
        return None;
    };
    let stream = wave.stream_id();
    match banks.get(&wave.bank) {
        Some(BankLink::Container(at)) if found.contains(&stream) => ui
            .link(tr!(
                "Open its stream in the audio container at {at}",
                at = format!("{at:#X}")
            ))
            .clicked()
            .then_some((*at, stream)),
        Some(BankLink::Container(_)) => {
            ui.label(tr!("No audio container the scan found holds its stream, though one holds its bank's other waves."));
            None
        }
        Some(BankLink::Tied(at)) => {
            let at: Vec<String> = at.iter().map(|a| format!("{a:#X}")).collect();
            ui.label(tr!(
                "The audio containers at {at} each hold every wave of its bank, differ and have as few streams, so which one the bank names is not known.",
                at = at.join(", ")
            ));
            None
        }
        Some(BankLink::Split) => {
            ui.label(tr!(
                "No one audio container the scan found holds every wave of its bank."
            ));
            None
        }
        Some(BankLink::Absent) | None => {
            ui.label(tr!("No audio container the scan found holds a wave of its bank: its container is not among the bodies the scan found."));
            None
        }
    }
}

fn draw_audio_items(ui: &mut egui::Ui, state: &AppState, doc_id: DocumentId) -> Option<Asked> {
    let Some(DocPayload::Gta6AudioData(doc)) = state.document(doc_id).map(|d| &d.payload) else {
        return None;
    };
    ui.heading(tr!("GTA VI Audio Game Data"));
    ui.separator();
    ui.label(tr!(
        "Schema {schema} ({kind}): {count} items",
        schema = doc.data.schema,
        kind = doc.data.kind.label(),
        count = doc.items.len()
    ));
    ui.label(
        egui::RichText::new(tr!("Its names are in an encrypted table, so an item is listed by its key; an item whose class's layout is not known is listed by its class and size."))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    let mut asked = None;
    let guard = doc.linking.as_ref().map(|l| l.ended());
    let linked = guard.as_deref().and_then(Option::as_ref);
    let linkable = matches!(
        doc.data.kind,
        AudioDataKind::Sounds | AudioDataKind::AudioConfig
    );
    if linkable
        && !state
            .document(doc_id)
            .is_some_and(crate::state::Document::is_from_rpf9_table)
        && draw_links(ui, linked, doc.linking.is_some() && linked.is_none())
    {
        asked = Some(Asked::Link);
    }
    let pool = pool_names();
    let name = |key: u32| {
        let pooled = pool.get(&key).map(String::as_str);
        match pooled.or_else(|| state.name_index.name_of(key)) {
            Some(known) => hash_text(key, Some(known)),
            None => hash_text(key, crate::state::hash_text_name(key).as_deref()),
        }
    };
    let row_height = ui.spacing().interact_size.y;
    for (class, items) in &doc.groups {
        let title = match class {
            Some(class) => tr!(
                "class {class} ({count} items, {laid} laid out)",
                class = class,
                count = items.len(),
                laid = doc.laid_out(items)
            ),
            None => tr!("items ({count})", count = items.len()),
        };
        egui::CollapsingHeader::new(title)
            .id_salt(("gta6_audio_class", *class))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("gta6_audio_rows", *class))
                    .max_height(row_height * GROUP_ROWS)
                    .auto_shrink([false, true])
                    .show_rows(ui, row_height, items.len(), |ui, range| {
                        for &i in &items[range] {
                            let item = &doc.items[i];
                            let label = format!("{}  {} B", name(item.key), item.span);
                            let label = egui::RichText::new(label).monospace();
                            if ui
                                .selectable_label(doc.selected == Some(i), label)
                                .clicked()
                            {
                                asked = Some(Asked::Select(i));
                            }
                        }
                    });
            });
    }

    let selected = doc.selected.and_then(|i| Some((i, doc.items.get(i)?)));
    if let Some((i, item)) = selected {
        ui.separator();
        ui.strong(name(item.key));
        ui.label(tr!(
            "At {offset} of the data block, {span} bytes to the next item",
            offset = format!("{:#X}", item.offset),
            span = item.span
        ));
        let shown = |ui: &mut egui::Ui, text: String| {
            egui::ScrollArea::both()
                .id_salt("gta6_audio_fields")
                .max_height(row_height * GROUP_ROWS)
                .auto_shrink([false, true])
                .show(ui, |ui| ui.monospace(text));
        };
        match (&item.fields, doc.layouts.get(i).and_then(Option::as_ref)) {
            (AudioFields::Opaque, None) => {
                ui.label(tr!("Its class's layout is not known."));
            }
            (AudioFields::Opaque, Some(ItemLayout::Table(table))) => {
                let bytes = doc
                    .data
                    .data
                    .get(item.offset as usize..)
                    .unwrap_or_default();
                slots::draw(ui, table, bytes, name, row_height * GROUP_ROWS);
            }
            (AudioFields::Opaque, Some(layout)) => shown(ui, format!("{layout:#?}")),
            (fields, _) => shown(ui, format!("{fields:#?}")),
        }
        for r in &item.references {
            let kinds = AudioDataKind::of_mask(r.mask);
            let text = tr!(
                "+{at}: refers to {key} ({kinds})",
                at = format!("{:#X}", r.at),
                key = name(r.key),
                kinds = (kinds.iter().map(|k| k.label()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            // A key of this body's kind is one of its own items, which the click selects.
            let own = (doc.by_key.get(&r.key)).filter(|_| kinds.contains(&doc.data.kind));
            match own {
                Some(&j) => {
                    if ui.link(text).clicked() {
                        asked = Some(Asked::Select(j));
                    }
                }
                None => {
                    ui.label(text);
                }
            }
        }
        // A category's array holds its sub-categories' index keys, so the categories are a tree.
        if let Some(ItemLayout::Table(table)) = doc.layouts.get(i).and_then(Option::as_ref) {
            for child in &table.children {
                let text = tr!("Sub-category {key}", key = name(*child));
                match doc.by_key.get(child) {
                    Some(&j) => {
                        if ui.link(text).clicked() {
                            asked = Some(Asked::Select(j));
                        }
                    }
                    None => {
                        ui.label(text);
                    }
                }
            }
        }
        for w in &item.waves {
            ui.label(tr!(
                "+{at}: wave {wave} of the audio container {bank}, its stream {stream}",
                at = format!("{:#X}", w.at),
                wave = format!("{:#010X}", w.wave),
                bank = format!("{:#010X}", w.bank),
                stream = format!("{:#X}", w.stream_id())
            ));
            let open = draw_wave_link(ui, linked, w);
            if let (Some((at, stream)), Some(linking)) = (open, &doc.linking) {
                asked = Some(Asked::Open(linking.archive.clone(), at, stream));
            }
        }
    }
    asked
}
