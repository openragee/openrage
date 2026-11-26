//! What a build wrote and read back, part by part: the one place the Build tab says a built folder.

use or_project::acceptance::Sheet;
use or_project::{Container, Output};

use super::build_tab::{against, file_row, said, LISTED, LIST_MARK, ROW};
use super::{ago, severity_colour, size};
use crate::state::front_door::counted;
use crate::state::workbench::{content_name, BuiltPart, Severity};
use crate::ui::theme;

/// What kind of file `path` is, as the Assets table says it; the reader's own name is for the hover.
fn kind_of(path: &str) -> String {
    crate::state::DocumentKind::from_path(std::path::Path::new(path)).map_or_else(
        || tr!("File").to_string(),
        |kind| kind.info().noun.to_string(),
    )
}

/// What a metadata document is, in words: the engine's name for its type is the hover's.
fn metadata_words(reader: &str) -> &'static str {
    if reader.contains("CPedVariationInfo") {
        tr!("clothing variation metadata")
    } else {
        tr!("metadata")
    }
}

/// Says what the build wrote and read back of `part`; the section of the acceptance guide its link asked for.
pub(super) fn built_part(
    ui: &mut egui::Ui,
    (container, output, sheet): (&Container, &Output, &Sheet),
    at: std::time::SystemTime,
    part: &BuiltPart,
) -> Option<&'static str> {
    let plain = theme::TEXT_PRIMARY;
    let dim = theme::TEXT_SECONDARY;
    let warning = severity_colour(Severity::Warning);
    let error = severity_colour(Severity::Error);

    // ⚠ Where it is in the project, and the whole path on hover: a path of the person's disk runs to a line of its own.
    let leaf = part
        .folder
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    said(
        ui,
        plain,
        tr!(
            "Built {ago}: {files}, {size}, in build/{leaf}",
            ago = ago(at),
            files = counted(part.files as usize, "file"),
            size = size(part.bytes),
            leaf = leaf,
        ),
    )
    .on_hover_text(part.folder.display().to_string());
    said(
        ui,
        plain,
        tr_n!(
            "{count} file was read back and equals the plan",
            "{count} files were read back and equal the plan",
            part.checked
        ),
    );
    let without = part.checked.saturating_sub(part.reloaded.len());
    said(
        ui,
        plain,
        match without {
            0 => tr_n!(
                "{count} file was opened again to check them",
                "{count} files were opened again to check them",
                part.reloaded.len()
            ),
            n => tr_n!(
                "{reloaded}, and {count} is a kind of file OpenRage cannot open",
                "{reloaded}, and {count} are a kind of file OpenRage cannot open",
                n,
                reloaded = tr_n!(
                    "{count} file was opened again to check them",
                    "{count} files were opened again to check them",
                    part.reloaded.len()
                ),
            ),
        },
    );
    for count in &part.metadata {
        said(
            ui,
            plain,
            tr!(
                "{documents} of {kind} with {entries}",
                documents = counted(count.documents, "document"),
                kind = metadata_words(&count.reader),
                entries = tr_n!("{count} entry", "{count} entries", count.entries),
            ),
        )
        .on_hover_text(&count.reader);
    }
    if !part.names.held.is_empty() && part.names.clashes.is_empty() {
        let kinds: Vec<String> = (part.names.held.iter())
            .map(|(kind, n)| counted(*n, kind))
            .collect();
        said(
            ui,
            plain,
            tr!("No name is held twice ({kinds})", kinds = kinds.join(", ")),
        );
    }
    for clash in &part.names.clashes {
        said(ui, error, format!("{LIST_MARK} {clash}"));
    }
    if part.memory.files > 0 {
        let over = part
            .memory
            .ceiling
            .is_some_and(|max| part.memory.bytes > max);
        said(
            ui,
            if over { error } else { plain },
            tr!(
                "{memory} ({files} counted)",
                memory = against(
                    tr!("Streaming memory"),
                    size(part.memory.bytes),
                    part.memory.ceiling.map(size)
                ),
                files = counted(part.memory.files, "file"),
            ),
        );
    }
    match &part.reproducible {
        or_project::Reproducible::NotRun => {
            said(
                ui,
                dim,
                tr!("A second build to compare byte for byte was not run: Verify with a second build asks for it"),
            );
        }
        or_project::Reproducible::Yes { files } => {
            said(
                ui,
                plain,
                tr!(
                    "A second build wrote the same bytes in {files}",
                    files = counted(*files, "file")
                ),
            );
        }
        or_project::Reproducible::No { differences } => {
            said(
                ui,
                error,
                tr!(
                    "A second build wrote different bytes in {files}",
                    files = counted(differences.len(), "file")
                ),
            );
            for difference in differences.iter().take(LISTED) {
                said(ui, error, format!("{LIST_MARK} {difference}"));
            }
        }
    }
    let asked = super::in_game::seen_in_game(ui, sheet, output, part);
    if !part.cooked.is_empty() {
        said(
            ui,
            plain,
            tr_n!(
                "{count} source was cooked into the file they stand for",
                "{count} sources were cooked into the files they stand for",
                part.cooked.len()
            ),
        );
    }
    for path in part.unchanged.iter().take(LISTED) {
        said(
            ui,
            plain,
            tr!(
                "{path}: unchanged from the game, so not shipped",
                path = path
            ),
        );
    }
    if let Some(entry) = &part.dlclist_entry {
        said(
            ui,
            plain,
            tr!(
                "dlclist.xml needs <Item>{entry}</Item> inside <Paths>",
                entry = entry
            ),
        );
    }
    for loss in part.losses.iter().take(LISTED) {
        said(
            ui,
            warning,
            tr!(
                "{path} dropped {values} the edition has no field for: {list}",
                path = loss.path,
                values = counted(loss.dropped.len(), "value"),
                list = (loss.dropped.iter().take(3).cloned().collect::<Vec<_>>()).join("; "),
            ),
        );
    }
    for (content, checks) in &part.pack_checks {
        // ⚠ A mark and a colour, never the words ok and FAILED in capitals: the mark is the verdict and the colour only agrees.
        said(
            ui,
            dim,
            tr!(
                "{content} was read back against its garments:",
                content = content_name(container, content)
            ),
        );
        for check in checks {
            let (mark, colour) = match check.passed {
                true => (egui_phosphor::regular::CHECK, theme::SUCCESS),
                false => (egui_phosphor::regular::X, error),
            };
            said(
                ui,
                colour,
                format!("{mark} {}: {}", check.name, check.detail),
            );
        }
    }
    if part.kept_previous {
        said(
            ui,
            dim,
            tr!("The build it replaced is kept as the previous one"),
        );
    }
    if !part.reloaded.is_empty() {
        ui.add_space(theme::SPACING_DEFAULT);
        said(
            ui,
            dim,
            tr!(
                "Every file opened again to check it ({count}):",
                count = part.reloaded.len()
            ),
        );
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("build_reloaded_files", &part.resource))
                .max_height(180.0)
                .show_rows(ui, ROW, part.reloaded.len(), |ui, range| {
                    for (path, reader) in &part.reloaded[range] {
                        file_row(ui, path, &kind_of(path), &format!("{path}\n{reader}"));
                    }
                });
        });
    }
    asked
}
