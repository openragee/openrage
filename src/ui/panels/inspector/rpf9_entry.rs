//! What the table's row states of a GTA VI archive's file selected in the Files tree.

use or_io::container::rpf::gta6_rpf::row::Rpf9Row;

use super::*;
use crate::state::{RpfEditSession, RpfNodeBacking};

/// The row of the file `backing` names, where it is one of a GTA VI archive; nothing for any other.
pub(super) fn draw(ui: &mut egui::Ui, session: &RpfEditSession, backing: &RpfNodeBacking) {
    let RpfNodeBacking::ArchiveFile {
        archive_id,
        entry_index,
        ..
    } = backing
    else {
        return;
    };
    let facts = (session.archive(*archive_id)).and_then(|slot| slot.archive.rpf9_row(*entry_index));
    if let Some(facts) = facts {
        draw_row(ui, &facts);
    }
}

fn draw_row(ui: &mut egui::Ui, facts: &Rpf9Row) {
    ui.add_space(theme::SPACING_DEFAULT);
    grid(ui, "rpf9_entry", |ui| {
        row(ui, tr!("Kind"), &facts.kind);
        row(ui, tr!("Stored"), &or_job::human_bytes(facts.stored));
        row(ui, tr!("Real size"), &or_job::human_bytes(facts.real));
        let ciphered = if facts.ciphered {
            tr!("yes")
        } else {
            tr!("no")
        };
        row(ui, tr!("Enciphered"), ciphered);
        let shape = match facts.row_bytes {
            Some(24) => tr!("24 bytes"),
            Some(_) => tr!("20 bytes, a headerless resource's"),
            None => tr!("none"),
        };
        row(ui, tr!("Row"), shape);
        if let Some(hash) = facts.hash {
            row(ui, tr!("Name hash"), &format!("0x{hash:016x}"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(facts: &Rpf9Row) -> String {
        let _serial = crate::ui::theme::serial();
        let output = egui::Context::default().run_ui(Default::default(), |ui| draw_row(ui, facts));
        format!("{:?}", output.shapes)
    }

    #[test]
    fn a_selected_gta_vi_row_shows_its_kind_sizes_cipher_row_and_unnamed_hash() {
        let shown = drawn(&Rpf9Row {
            kind: "drawable".into(),
            stored: 0x200,
            real: 0x5000,
            ciphered: true,
            row_bytes: Some(20),
            hash: Some(0x0123_4567_89AB_CDEF),
        });
        for want in [
            "drawable",
            &or_job::human_bytes(0x200),
            &or_job::human_bytes(0x5000),
            "yes",
            "20 bytes, a headerless resource",
            "0x0123456789abcdef",
        ] {
            assert!(shown.contains(want), "{want} is not shown: {shown}");
        }
    }

    #[test]
    fn a_named_row_shows_no_hash() {
        let shown = drawn(&Rpf9Row {
            kind: "media".into(),
            stored: 16,
            real: 16,
            ciphered: false,
            row_bytes: Some(24),
            hash: None,
        });
        assert!(
            shown.contains("24 bytes") && shown.contains("\"no\""),
            "{shown}"
        );
        assert!(!shown.contains("Name hash"), "{shown}");
    }
}
