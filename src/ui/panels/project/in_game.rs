//! What the Build tab says of the acceptance sheet under a build that went through: the kinds of file it wrote that nobody recorded
//! seeing the game load, or the day all were seen, and the link to the steps that check it.
//!
//! ⚠ **What was seen, never what loads.** A reading is `or_project::acceptance::Seen`, which `openrage-cli project build` prints in
//! English; `the_tab_and_the_command_say_a_reading_in_the_same_english` holds the two to one text.

use or_project::acceptance::{Seen, Sheet};
use or_project::{Output, OutputKind, Target};

use super::build_tab::said;
use crate::state::workbench::BuiltPart;
use crate::ui::theme;

/// The headings of `docs/guides/In_Game_Acceptance.md` that say how a build is loaded, which the link leads to.
pub(crate) const ON_A_SERVER: &str = "Load it on a FiveM server";
pub(crate) const IN_STORY_MODE: &str = "Load it in story mode";

/// The link's name, which a screen reader speaks.
pub(crate) const LINK: &str = tr_key!("How to check it in the game");

/// The section of the acceptance guide that says how a build of `kind` is loaded.
pub(crate) fn section_of(kind: &OutputKind) -> &'static str {
    match kind {
        OutputKind::FivemResource | OutputKind::Unknown(_) => ON_A_SERVER,
        OutputKind::SingleplayerDlc | OutputKind::OivPackage => IN_STORY_MODE,
    }
}

/// What a person calls the files of `kind`, several of them, in the language in use: by their extension where the kind has no words yet.
fn kind_words(kind: &str) -> String {
    match kind {
        "ymap" => tr!("maps").to_string(),
        "ytyp" => tr!("type files").to_string(),
        "rpf" => tr!("singleplayer DLC packs").to_string(),
        other => tr!(".{extension} files", extension = other),
    }
}

/// One reading of the sheet as the sentence the tab says, in the language in use.
pub(crate) fn sentence(seen: &Seen, edition: Target) -> String {
    let edition = edition.generation().edition_label();
    match seen {
        Seen::Not(kinds) => {
            let named: Vec<String> = kinds.iter().map(|kind| kind_words(kind)).collect();
            tr_n!(
                "{count} kind of file this build wrote was not seen loading in GTA V {edition}: {kinds}",
                "{count} kinds of file this build wrote were not seen loading in GTA V {edition}: {kinds}",
                named.len(),
                edition = edition,
                kinds = named.join(", "),
            )
        }
        Seen::All(date) => tr!(
            "Every kind of file this build wrote that the acceptance sheet has a line about was seen loading in GTA V {edition}; the newest result is dated {date}",
            edition = edition,
            date = date,
        ),
    }
}

/// Says what `sheet` holds of the kinds of file `part` wrote, for the edition it was written for or for both when the build stated none; the guide section its link asked for.
pub(crate) fn seen_in_game(
    ui: &mut egui::Ui,
    sheet: &Sheet,
    output: &Output,
    part: &BuiltPart,
) -> Option<&'static str> {
    let seen = sheet.seen(&part.kinds, part.target);
    for (edition, reading) in &seen {
        said(ui, theme::TEXT_PRIMARY, sentence(reading, *edition));
    }
    // ⚠ The steps are offered where something is left to check: a build whose kinds were all seen is sent nowhere.
    if !seen
        .iter()
        .any(|(_, reading)| matches!(reading, Seen::Not(_)))
    {
        return None;
    }
    let section = section_of(&output.kind);
    let link = ui.link(crate::i18n::tr(LINK)).on_hover_text(tr!(
        "Read \"{section}\" in the guide to checking a build in the game, which nothing in OpenRage runs",
        section = section
    ));
    link.clicked().then_some(section)
}

#[cfg(test)]
mod tests {
    use or_project::acceptance::kind_words as said_by_the_command;

    use super::*;
    use crate::state::front_door::GuideDocument;

    #[test]
    fn the_tab_and_the_command_say_a_reading_in_the_same_english() {
        let _english = crate::i18n::pin("en");
        let sheet = Sheet::shipped().expect("the sheet the tree ships");
        // Every kind a line is about, and one no line is about yet, which both say by its extension.
        let mut kinds: Vec<String> = sheet.lines.iter().map(|line| line.kind.clone()).collect();
        kinds.push("ybn".to_string());
        kinds.dedup();
        for kind in &kinds {
            assert_eq!(kind_words(kind), said_by_the_command(kind), "{kind}");
        }
        let mut readings: Vec<Seen> = (kinds.iter())
            .map(|kind| Seen::Not(vec![kind.clone()]))
            .collect();
        readings.push(Seen::Not(kinds.clone()));
        readings.push(Seen::All("2026-12-01".to_string()));
        for reading in &readings {
            for edition in [Target::Legacy, Target::Enhanced] {
                assert_eq!(sentence(reading, edition), reading.sentence(edition));
            }
        }
    }

    #[test]
    fn every_kind_of_build_is_sent_to_a_heading_of_the_acceptance_guide() {
        let headings = crate::ui::panels::guide::headings_of(GuideDocument::Acceptance);
        for kind in [
            OutputKind::FivemResource,
            OutputKind::SingleplayerDlc,
            OutputKind::OivPackage,
        ] {
            let section = section_of(&kind);
            assert!(
                headings.contains(&section),
                "{kind:?}: `{section}` of {headings:?}"
            );
        }
        assert_eq!(section_of(&OutputKind::FivemResource), ON_A_SERVER);
        assert_eq!(section_of(&OutputKind::SingleplayerDlc), IN_STORY_MODE);
    }
}
