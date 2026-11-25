//! Rows moved or copied into a folder together, as a drag drops the selection: the edit gate asked
//! once, and the moves or the copies one undo step.

use super::document::{DocumentId, RpfNodeId};
use super::{AppState, DocPayload, GateOutcome, PendingRetailEdit, PendingRetailEditPrompt};

impl AppState {
    /// Move `rows` of `doc` into the row `folder`, asking the edit gate once for every archive or folder the moves write.
    ///
    /// ⚠ As a paste does: a row that would go into itself or a folder inside it, or a gate that
    /// refuses, moves nothing, and is said before anything is asked; a row refused on its own, by a
    /// name taken, is named in the `Err` and the rest move. Rows already in `folder` stay, and a
    /// move of only those writes nothing and asks nothing.
    pub fn rpf_move_nodes(
        &mut self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        folder: RpfNodeId,
    ) -> Result<(), String> {
        let rows = self.rows_to_place(doc, rows, folder, "move")?;
        if rows.is_empty() {
            return Ok(());
        }
        // ⚠ Each archive or folder asked once, so a refusal is said once and one Yes moves every row.
        let mut targets = vec![self.affected_disk_path(doc, folder)];
        targets.extend(rows.iter().map(|&row| self.affected_disk_path(doc, row)));
        targets.sort();
        targets.dedup();
        let mut ask = None;
        for target in targets {
            match self.gate_permits(target.as_deref()) {
                GateOutcome::Refused => return Err(super::retail_pending_message()),
                GateOutcome::Confirm(path) => ask = ask.or(Some(path)),
                GateOutcome::Proceed => {}
            }
        }
        if let Some(target_path) = ask {
            self.pending_retail_edit = Some(PendingRetailEditPrompt {
                action: PendingRetailEdit::MoveNode {
                    doc_id: doc,
                    node_ids: rows,
                    new_parent: folder,
                },
                target_path,
                verb: "move",
            });
            return Err(super::retail_pending_message());
        }
        self.rpf_move_nodes_inner(doc, &rows, folder)
    }

    /// Copy `rows` of `doc` into the row `folder`, as a drop with Ctrl held does: the clipboard is left alone, the edit gate is asked once and the copies are one undo step.
    pub fn rpf_copy_nodes(
        &mut self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        folder: RpfNodeId,
    ) -> Result<(), String> {
        let rows = self.rows_to_place(doc, rows, folder, "copy")?;
        if rows.is_empty() {
            return Ok(());
        }
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Err(tr!("The archive is no longer open").into());
        };
        // ⚠ Held before anything is written into it, so a copy that fails on the way leaves nothing
        let staged = super::pending_prompts::Staged::new(crate::ui::os_drop::staging_dir(
            "openrage-clipboard",
        ));
        let files = super::clipboard::files_of_rows(&rpf.session, &rows, staged.folder())?;
        self.copy_paths_into_folder(doc, folder, files, staged);
        Ok(())
    }

    /// `rows` of `doc` that a move or a copy into `folder` changes: none inside another, the root or gone, and none already in `folder`.
    ///
    /// ⚠ An `Err` for a row that would go into itself or a folder inside it, and for no row at all;
    /// `Ok` and empty when each of them is in `folder` already, which asks and writes nothing.
    pub(super) fn rows_to_place(
        &self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        folder: RpfNodeId,
        verb: &str,
    ) -> Result<Vec<RpfNodeId>, String> {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Err(tr!("The archive is no longer open").into());
        };
        self.require_writable_rpf_row(doc, folder)?;
        for &row in rows {
            self.require_writable_rpf_row(doc, row)?;
        }
        let rows = super::clipboard::top_rows(&rpf.session, rows);
        if rows.is_empty() {
            return Err(tr!("Select a file or folder to {verb}", verb = verb));
        }
        if let Some(&row) = rows
            .iter()
            .find(|&&row| rpf.session.is_descendant(folder, row))
        {
            let name = rpf.session.node(row).map_or("", |n| n.name.as_str());
            return Err(tr!(
                "`{name}` cannot go into itself or a folder inside it",
                name = name
            ));
        }
        Ok(rows
            .into_iter()
            .filter(|&row| rpf.session.node(row).and_then(|n| n.parent) != Some(folder))
            .collect())
    }

    /// The moves, once the gate is satisfied: one row as a move of it alone, several as one undo step.
    pub(super) fn rpf_move_nodes_inner(
        &mut self,
        doc: DocumentId,
        rows: &[RpfNodeId],
        folder: RpfNodeId,
    ) -> Result<(), String> {
        if let [row] = rows[..] {
            return self.rpf_move_node_inner(doc, row, folder);
        }
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return Err(tr!("The archive is no longer open").into());
        };
        let names: Vec<String> = rows
            .iter()
            .map(|&row| {
                rpf.session
                    .node(row)
                    .map_or_else(String::new, |n| n.name.clone())
            })
            .collect();
        if let Some(d) = self.document_mut(doc) {
            d.history.begin_batch();
        }
        let mut failures = Vec::new();
        for (&row, name) in rows.iter().zip(names) {
            if let Err(e) = self.rpf_move_node_inner(doc, row, folder) {
                failures.push(format!("{name}: {e}"));
            }
        }
        let moved = rows.len() - failures.len();
        if let Some(d) = self.document_mut(doc) {
            d.history
                .end_batch(tr!("Move {moved} item(s)", moved = moved));
        }
        match failures.len() {
            0 => Ok(()),
            failed => Err(tr!(
                "Moved {moved} item(s); {failed} failed: {failures}",
                moved = moved,
                failed = failed,
                failures = failures.join("; ")
            )),
        }
    }
}
