//! Content-selected previews for complete RPF9 entry bodies.

use std::path::Path;

use or_io::container::rpf::Gta6EntryType;
use or_io::gta6::other_bodies::OtherBody;

use crate::state::{DocPayload, MetaDocument};

pub(super) fn payload(path: &Path, bytes: &[u8]) -> Result<Option<DocPayload>, String> {
    let payload = match Gta6EntryType::from_header(bytes) {
        Some(Gta6EntryType::Awc) => crate::audio::AwcDocument::open(bytes).map(DocPayload::Awc),
        Some(Gta6EntryType::Pso | Gta6EntryType::Rbf) => {
            let document = MetaDocument::from_bytes(bytes.to_vec());
            match &document.parse_error {
                Some(error) => Err(error.clone()),
                None => Ok(DocPayload::Meta(document)),
            }
        }
        Some(Gta6EntryType::File("rel")) => crate::state::gta6_text::Gta6AudioDocument::open(bytes)
            .map(|document| DocPayload::Gta6AudioData(Box::new(document))),
        Some(Gta6EntryType::File("bk2")) => {
            let document = crate::video::BikDocument::new(bytes);
            if document.header.is_none() {
                return Err(super::preview_error("the movie header is incomplete"));
            }
            Ok(DocPayload::Bik(document))
        }
        Some(Gta6EntryType::File("xml")) => Ok(crate::state::document_io::text_payload(
            path,
            bytes.to_vec(),
        )),
        _ => {
            if let Some(body) = OtherBody::of(bytes, bytes.len() as u64) {
                if body == OtherBody::Edup {
                    return Ok(None);
                }
                crate::state::gta6_other_bodies::payload(path, body, bytes)
            } else {
                return Ok(None);
            }
        }
    };
    payload.map(Some)
}
