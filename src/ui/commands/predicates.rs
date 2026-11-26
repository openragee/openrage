//! The checks a command is enabled by, shared by the table and the files of commands beside it.

use crate::state::{AppState, DocumentKind, LoadingState};
use crate::ui::panels::registry::ids;

pub(super) fn always(_: &AppState) -> Result<(), &'static str> {
    Ok(())
}

pub(super) fn none_if(cannot: bool, why: &'static str) -> Result<(), &'static str> {
    if cannot {
        Err(why)
    } else {
        Ok(())
    }
}

pub(super) fn idle(state: &AppState) -> Result<(), &'static str> {
    let opening = matches!(state.loading, LoadingState::Loading);
    none_if(opening, tr!("A file is still opening"))
}

pub(super) fn not_busy(state: &AppState) -> Result<(), &'static str> {
    none_if(
        state.is_busy(),
        tr!("Wait for the file being opened or saved"),
    )
}

pub(super) fn a_document(state: &AppState) -> Result<(), &'static str> {
    none_if(state.active_document.is_none(), tr!("No document is open"))?;
    let compared = (state.active_document()).is_some_and(|d| d.kind == DocumentKind::Compare);
    none_if(compared, tr!("A comparison is shown, never saved"))?;
    not_busy(state)
}

/// A document in front of a kind OpenRage writes: what Save as… needs.
pub(super) fn a_document_that_is_written(state: &AppState) -> Result<(), &'static str> {
    a_document(state)?;
    let never = (state.active_document()).and_then(|d| crate::state::never_written(d.kind));
    never.map_or(Ok(()), Err)
}

pub(super) fn a_project(state: &AppState) -> Result<(), &'static str> {
    none_if(state.project_path.is_none(), tr!("No project is open"))
}

pub(super) fn a_viewport(state: &AppState) -> Result<(), &'static str> {
    let shown = state
        .workspace_docks
        .shows(state.active_workspace, ids::VIEWPORT_3D);
    none_if(!shown, tr!("This workspace shows no viewport"))
}

pub(super) fn a_text_document(state: &AppState) -> Result<(), &'static str> {
    let any = (state.documents.iter()).any(|d| d.payload.found_in().is_some());
    none_if(!any, tr!("No text document is open"))
}
