//! The image the grid draws, built again from the payload where an edit of the payload took it.

use std::sync::atomic::{AtomicBool, Ordering};

use super::Document;

/// A view's ask for an image the document no longer holds, and the payload last built into one.
#[derive(Debug, Default)]
pub(super) struct Rebuild {
    asked: AtomicBool,
    /// The payload epoch of the last build, which a refusal is not retried at.
    tried: Option<u64>,
    /// The version of the image the file opened with, which a rebuilt image may not keep, until a save rewrites the file.
    opened: Option<u32>,
}

impl Document {
    /// The version the file's own image had when it opened.
    pub fn opened_version(&self) -> Option<u32> {
        self.rebuild.opened
    }

    /// Note the version of `image`, an image the file was read as; `None` once a save has written the file again.
    pub(super) fn opened_as(&mut self, image: Option<&super::ResourceImage>) {
        self.rebuild.opened = image.map(|image| image.seal.version());
    }

    /// Ask for the image an edit of the payload took, which `AppState::seal_edits` builds from it.
    pub(in crate::state) fn ask_for_image(&self) {
        if self.read_only == Some(super::rpf9::entry::READ_ONLY) {
            return;
        }
        self.rebuild.asked.store(true, Ordering::Relaxed);
    }

    /// Build the image a view asked for from the payload, as a save writes it.
    ///
    /// ⚠ Once per payload epoch: a payload its writer refuses would otherwise be written every frame.
    pub(in crate::state) fn build_the_asked_image(&mut self) {
        let asked = self.rebuild.asked.swap(false, Ordering::Relaxed);
        if !asked
            || self.read_only == Some(super::rpf9::entry::READ_ONLY)
            || self.resource.is_some()
            || self.rebuild.tried == Some(self.payload_epoch)
        {
            return;
        }
        self.rebuild.tried = Some(self.payload_epoch);
        let built = crate::state::payload_handlers::handler_for_payload(&self.payload)
            .ok_or_else(|| "no writer is registered for it".to_string())
            .and_then(|handler| handler.save(&self.payload))
            .and_then(|bytes| {
                crate::state::document_io::resource_image_for(self.kind, &bytes)
                    .ok_or_else(|| "what its writer wrote is not a resource".to_string())
            });
        match built {
            Ok(image) => self.resource = Some(image),
            Err(why) => {
                tracing::warn!(document = %self.name, %why, "the payload builds no image for the grid");
            }
        }
    }
}
