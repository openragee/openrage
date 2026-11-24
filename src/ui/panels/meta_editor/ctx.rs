use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use or_io::meta::{SchemaLibrary, Value};

use crate::state::name_index::NameIndexSnapshot;
use crate::state::presets::MetaStep;
use crate::state::reference_names::ReferenceNames;
use crate::state::MetaEdit;

use super::presets::RecordUi;

#[derive(Clone)]
pub struct EditorCtx {
    pub library: &'static SchemaLibrary,
    pub name_index: Arc<NameIndexSnapshot>,
    pub references: ReferenceNames,
    /// Where the value being drawn sits, from the document's root.
    pub path: Vec<MetaStep>,
    pub records: Rc<RecordUi>,
    /// What the form changed this frame, oldest first, as undo holds it.
    pub edits: Rc<RefCell<Vec<MetaEdit>>>,
    /// Whether a hash value no table names shows the vocabulary's name: a GTA VI form's only.
    pub text_names: bool,
    /// The names the document's own string sections give its hash values.
    pub strings: Arc<crate::state::DocumentStrings>,
}

impl EditorCtx {
    /// This context inside the field `name` of the struct being drawn that `nth` others of that name come before.
    pub fn field(&self, name: &str, nth: usize) -> Self {
        self.at(match nth {
            0 => MetaStep::Field(name.to_string()),
            nth => MetaStep::Nth(name.to_string(), nth),
        })
    }

    /// This context inside element `index` of the array being drawn.
    pub fn index(&self, index: usize) -> Self {
        self.at(MetaStep::Index(index))
    }

    fn at(&self, step: MetaStep) -> Self {
        let mut ctx = self.clone();
        ctx.path.push(step);
        ctx
    }

    /// The value at this context's path held `before` until now.
    pub fn changed(&self, before: Value) {
        self.record(MetaEdit::Set {
            path: self.path.clone(),
            other: before,
        });
    }

    pub fn record(&self, edit: MetaEdit) {
        self.edits.borrow_mut().push(edit);
    }

    /// ⚠ The installed library, which with no snapshot found knows
    /// nothing — which is the state the derived field list exists for and so
    /// the right one to test against.
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self {
            library: crate::state::meta_schema(),
            name_index: Arc::new(NameIndexSnapshot::default()),
            references: ReferenceNames::default(),
            path: Vec::new(),
            records: Rc::new(RecordUi::default()),
            edits: Rc::default(),
            text_names: false,
            strings: Default::default(),
        }
    }
}
