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

    /// A settings record laid out by hand: the header, then `payload` where the header says.
    fn record(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; save::ASAV_HEADER];
        bytes[..4].copy_from_slice(b"ASAV");
        bytes[4..8].copy_from_slice(&save::ASAV_VERSION.to_le_bytes());
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&(save::ASAV_HEADER as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
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
