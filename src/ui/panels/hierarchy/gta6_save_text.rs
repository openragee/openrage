//! A GTA VI save record's payload as the JSON text it holds, read only, under the save's facts.

use or_io::gta6::save::Save;

/// The text a save record's payload holds; `None` for a story save or a payload that is no text.
pub(super) fn record_json(save: &Save) -> Option<&str> {
    match save {
        Save::Record(record) => record.text().filter(|text| !text.is_empty()),
        Save::Story(_) => None,
    }
}

/// The record's payload as text that can be selected and copied but not edited.
pub(super) fn draw(ui: &mut egui::Ui, save: &Save) {
    let Some(mut text) = record_json(save) else {
        return;
    };
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("gta6_save_record_json")
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
        });
}

#[cfg(test)]
mod tests {
    use or_io::gta6::save;

    use super::*;

    /// A settings record carrying `payload`, written with a valid checksum so the reader accepts it.
    fn record(payload: &[u8]) -> Vec<u8> {
        let header = save::AsavHeader {
            kind: 2,
            word_0c: 1,
            word_10: 0,
            payload_offset: save::ASAV_HEADER as u32,
            stored_size: 0,
            size: 0,
            time: 0,
            title: String::new(),
            slot: String::new(),
            checksum: 0,
            word_164: 0,
            size_again: 0,
        };
        save::Asav {
            header,
            payload: payload.to_vec(),
        }
        .save()
        .expect("the record writes")
    }

    #[test]
    fn a_settings_record_shows_the_json_its_payload_holds() {
        let json = "{\"zq_volume\":0.5,\"zq_subtitles\":true}";
        let mut payload = json.as_bytes().to_vec();
        payload.push(0);
        let opened = save::open(&record(&payload)).expect("the record reads");
        assert_eq!(record_json(&opened), Some(json));
        // A record whose payload is empty, or not text, shows none.
        let empty = save::open(&record(&[])).expect("the record reads");
        assert_eq!(record_json(&empty), None);
        let binary = save::open(&record(&[0xFF, 0xFE, 0x00])).expect("the record reads");
        assert_eq!(record_json(&binary), None);
    }
}
