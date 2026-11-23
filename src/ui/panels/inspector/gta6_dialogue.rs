//! GTA VI's radio stations of a music body and the dialogue lines of the open scans, each with
//! the texts of the language picked and a button that plays its audio stream.

use or_io::gta6::audio_data::AudioDataKind;
use or_io::gta6::audio_radio::{track_text_keys, Station, Track};
use or_io::gta6::dialogue::plain;
use or_io::gta6::hash::name_hash32;

use crate::state::gta6_dialogue::{self as store, lines, Lines, Radio};

use super::*;

/// Rows a list of tracks or of lines shows before it scrolls.
const LIST_ROWS: f32 = 16.0;

/// How the index of the open scans stands, and the button that starts it.
fn draw_index(ui: &mut egui::Ui, state: &mut AppState) {
    if store::running() {
        // Drawn again until the index ends, so what it kept shows without a click.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tr!("Indexing the open scans; the job list shows how far."));
        });
        return;
    }
    let (said, language) = {
        let lines = lines();
        (lines.said.clone(), lines.filter.language.clone())
    };
    match said {
        Some(Ok(kept)) => {
            ui.label(kept);
        }
        Some(Err(why)) => {
            ui.colored_label(theme::WARNING, why);
        }
        None => {}
    }
    let index = ui
        .button(tr!("Index the open scans"))
        .on_hover_text(tr!("Reads, as a job, every open scan of a whole GTA VI archive: the conversation bodies for their lines, the text tables of the language picked, the sounds bodies for their streamed sounds, and the audio containers' stream tables for the container of each stream."));
    if index.clicked() {
        let language = (!language.is_empty()).then_some(language);
        if let Err(why) = state.index_gta6_dialogue(language) {
            state.show_error_toast(why);
        }
    }
}

/// The language of the texts shown, picked among those registered or found.
fn draw_language(ui: &mut egui::Ui) {
    let mut guard = lines();
    let lines: &mut Lines = &mut guard;
    let offered = lines.languages();
    if offered.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(tr!("Language"));
        let shown = match lines.filter.language.is_empty() {
            true => lines.first_language(),
            false => lines.filter.language.clone(),
        };
        egui::ComboBox::from_id_salt("gta6_dialogue_language")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for language in &offered {
                    let picked = &mut lines.filter.language;
                    ui.selectable_value(picked, language.clone(), language.as_str());
                }
            });
        if !lines.filter.language.is_empty() && !lines.holds(&lines.filter.language) {
            ui.label(
                egui::RichText::new(tr!(
                    "Its texts are not read yet: index the open scans again."
                ))
                .small()
                .color(theme::TEXT_SECONDARY),
            );
        }
    });
}

/// A length in milliseconds as minutes and seconds.
fn length(ms: u32) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1_000 % 60)
}

/// The title and the artist the texts of `language` show for the track of id `id`.
fn shown_title(lines: &Lines, language: &str, id: u32) -> Option<String> {
    let (title, artist) = track_text_keys(id);
    let title = plain(lines.text(title, language)?);
    Some(match lines.text(artist, language) {
        Some(artist) => format!("{title}  ({})", plain(artist)),
        None => title,
    })
}

/// A track's row: the button that plays it where its stream's container is known, the title and
/// artist the game shows where its stream states its id and the texts are read, else those its
/// text names as identifiers, and its length. Returns the stream to play.
fn draw_track(
    ui: &mut egui::Ui,
    lines: &Lines,
    radio: &Radio,
    language: &str,
    track: &Track,
) -> Option<u32> {
    let sound = track.sound.and_then(|key| lines.sound(key));
    let stream = sound.and_then(|s| s.streams().find(|id| lines.container(*id).is_some()));
    let mut play = None;
    let button = egui::Button::new(ph::PLAY);
    let hover = match stream {
        Some(_) => tr!("Opens the audio container that holds its stream and plays it"),
        None => tr!("No audio container of the indexed scans holds its stream: index the open scans with the scan of the audio archive open"),
    };
    if ui
        .add_enabled(stream.is_some(), button)
        .on_hover_text(hover)
        .on_disabled_hover_text(hover)
        .clicked()
    {
        play = stream;
    }
    let ids = sound.map_or(&[][..], |s| lines.track_ids(s));
    let shown: Vec<String> = (ids.iter())
        .filter_map(|id| shown_title(lines, language, *id))
        .collect();
    match shown.as_slice() {
        [] => {
            let title = match track.text.and_then(|key| radio.texts.get(&key)) {
                Some(text) if text.artists.is_empty() => text.title.clone(),
                Some(text) => format!("{}  ({})", text.title, text.artists.join(", ")),
                None => format!("0x{:08X}", track.key),
            };
            ui.monospace(title);
        }
        [title] => {
            ui.label(title);
        }
        // A mix states the id of each track it plays, in order.
        mix => {
            ui.label(tr_n!(
                "A mix of {count} track",
                "A mix of {count} tracks",
                mix.len()
            ))
            .on_hover_text(mix.join("\n"));
        }
    }
    ui.label(sound.map_or_else(String::new, |s| length(s.duration_ms)));
    ui.end_row();
    play
}

/// A station under its name: its tracks, list after list in the order stated.
fn draw_station(
    ui: &mut egui::Ui,
    lines: &Lines,
    radio: &Radio,
    language: &str,
    station: &Station,
    name: String,
) -> Option<u32> {
    let tracks: Vec<&Track> = (station.lists.iter())
        .flat_map(|list| list.tracks.iter().flatten())
        .collect();
    let title = tr_n!(
        "{name}: {count} track",
        "{name}: {count} tracks",
        tracks.len(),
        name = name
    );
    let mut play = None;
    egui::CollapsingHeader::new(title)
        .id_salt(("gta6_station", station.key))
        .show(ui, |ui| {
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical()
                .id_salt(("gta6_station_tracks", station.key))
                .max_height(row_height * LIST_ROWS)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, tracks.len(), |ui, range| {
                    egui::Grid::new(("gta6_station_grid", station.key))
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for track in &tracks[range] {
                                play = draw_track(ui, lines, radio, language, track).or(play);
                            }
                        });
                });
        });
    play
}

/// The name a station kept off the wheel is listed under: the name found of its item, after the
/// shown name of the station of the wheel it is the twin of where it is one; else its key.
fn hidden_name(state: &AppState, lines: &Lines, language: &str, station: &Station) -> String {
    let Some(name) = lines.station_name(station.key) else {
        return hash_text(station.key, state.name_index.name_of(station.key));
    };
    let twin = name
        .strip_prefix("hidden_")
        .map(|n| name_hash32(n.as_bytes()));
    match twin.and_then(|key| lines.text(key, language)) {
        Some(text) => format!("{}  ({name})", plain(text)),
        None => name.to_string(),
    }
}

/// The radio of a music body: the stations a text names first, in the order their bands give, each
/// with its tracks; then the others, by the name found of each.
pub(super) fn draw_radio(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    let (Some(path), DocPayload::Gta6AudioData(audio)) = (doc.path.as_deref(), &doc.payload) else {
        return;
    };
    if audio.data.kind != AudioDataKind::Music {
        return;
    }
    let read = store::radio_of(path, &audio.data);
    let radio = match &*read {
        Ok(radio) if radio.stations.is_empty() => return,
        Ok(radio) => radio,
        Err(why) => {
            ui.colored_label(
                theme::WARNING,
                tr!("Its radio stations do not read: {why}", why = why),
            );
            return;
        }
    };
    ui.heading(tr!("GTA VI Radio"));
    ui.separator();
    ui.label(
        egui::RichText::new(tr!("A station's key is the key of its name in the text tables, and a track plays the streamed sound of a sounds body: index the open scans with the scans of the text and audio archives open to name the stations, show their tracks' titles and artists and play them. A title the game shows is keyed by the track id a marker of the track's stream states; a track with none is listed by the identifiers its text states. A station kept off the wheel is named from the names harvested."))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    draw_index(ui, state);
    draw_language(ui);

    // A station's band orders the stations a text names.
    let mut order: Vec<&Station> = radio.stations.iter().collect();
    order.sort_by_key(|station| station.band);
    let mut play = None;
    {
        let lines = lines();
        let language = match lines.filter.language.is_empty() {
            true => lines.first_language(),
            false => lines.filter.language.clone(),
        };
        let name_of = |station: &Station| lines.text(station.key, &language).map(plain);
        for station in order.iter().filter(|s| name_of(s).is_some()) {
            let name = name_of(station).unwrap_or_default();
            play = draw_station(ui, &lines, radio, &language, station, name).or(play);
        }
        let mut others: Vec<(String, &Station)> = (order.iter())
            .filter(|s| name_of(s).is_none())
            .map(|s| (hidden_name(state, &lines, &language, s), *s))
            .collect();
        others.sort_by(|a, b| a.0.cmp(&b.0));
        let title = tr_n!(
            "{count} station no text names",
            "{count} stations no text names",
            others.len()
        );
        egui::CollapsingHeader::new(title)
            .id_salt("gta6_stations_unnamed")
            .show(ui, |ui| {
                for (name, station) in others {
                    play = draw_station(ui, &lines, radio, &language, station, name).or(play);
                }
            });
    }
    if let Some(stream) = play {
        state.play_gta6_stream(stream);
    }
}

/// The dialogue lines of the scans indexed: each line's speaker, audio stream, label and its
/// text in the language picked, kept by the speaker and the text typed, with a button that
/// plays its stream.
pub(super) fn draw_dialogue(ui: &mut egui::Ui, state: &mut AppState) {
    egui::CollapsingHeader::new(tr!("GTA VI Dialogue"))
        .id_salt("gta6_dialogue")
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(tr!("The lines the conversation bodies of the open scans state, with no body opened first. A line's stream is named by its key and its number among the key's labels of its body; its text is its label's in the text tables of the language picked; its speaker is the one or two capitals before its label's number, where a label has them."))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            draw_index(ui, state);
            draw_language(ui);
            if let Some(stream) = draw_lines(ui) {
                state.play_gta6_stream(stream);
            }
        });
}

/// The filter and the lines it keeps; returns the stream of the line whose button was clicked.
fn draw_lines(ui: &mut egui::Ui) -> Option<u32> {
    let mut guard = lines();
    let lines: &mut Lines = &mut guard;
    if lines.count() == 0 {
        return None;
    }
    ui.horizontal(|ui| {
        ui.label(tr!("Speaker"));
        ui.add(egui::TextEdit::singleline(&mut lines.filter.speaker).desired_width(40.0));
        ui.label(tr!("Text"));
        ui.add(egui::TextEdit::singleline(&mut lines.filter.text).desired_width(200.0));
    });
    lines.filter_lines();
    let lines: &Lines = lines;
    let shown = lines.shown();
    ui.label(tr!(
        "{shown} of {count} lines",
        shown = shown.len(),
        count = lines.count()
    ));
    let language = lines.filter.language.clone();
    let mut play = None;
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::both()
        .id_salt("gta6_dialogue_lines")
        .max_height(row_height * LIST_ROWS)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, shown.len(), |ui, range| {
            egui::Grid::new("gta6_dialogue_grid")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for label in &shown[range] {
                        let Some(line) = lines.line(*label) else {
                            continue;
                        };
                        let stream = line.stream_id();
                        let button = egui::Button::new(ph::PLAY);
                        let held = lines.container(stream).is_some();
                        let hover = match held {
                            true => tr!("Opens the audio container that holds its stream and plays it"),
                            false => tr!("No audio container of the indexed scans holds its stream: index the open scans with the scan of the audio archive open"),
                        };
                        if ui
                            .add_enabled(held, button)
                            .on_hover_text(hover)
                            .on_disabled_hover_text(hover)
                            .clicked()
                        {
                            play = Some(stream);
                        }
                        ui.label(line.speaker().unwrap_or(""));
                        ui.monospace(line.wave());
                        ui.monospace(line.label.as_str());
                        ui.label(lines.text(*label, &language).map_or_else(String::new, plain));
                        ui.end_row();
                    }
                });
        });
    play
}
