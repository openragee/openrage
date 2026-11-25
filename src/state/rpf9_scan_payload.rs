//! The payload and view of a clear body found through an explicit GTA VI archive scan.

use super::super::{
    document_io, gta6_dialogue, gta6_effect_facts, gta6_entry_skeletons, gta6_face, gta6_head,
    gta6_name_pool, gta6_other_bodies, gta6_particles, gta6_resource, gta6_text, gta6_weapon_parts,
    ttpf_document,
};
use super::*;

/// What `bytes`, the body `find` of `archive`, open as under `path`: its view's read, else its bytes.
pub(super) fn body_payload(
    path: &Path,
    archive: &Path,
    find: Rpf9ScanFind,
    bytes: Vec<u8>,
) -> Result<DocPayload, String> {
    let resource = match find.kind {
        Rpf9BodyKind::Zlib(Rpf9ZlibRun { resource, .. }) => resource,
        _ => None,
    };
    let view = match view_of(&find) {
        // No reader fits it: its bytes, under the reason.
        Err(banner) if opens_as_bytes(&find) => {
            if let Some(names) = gta6_name_pool::payload(path, &find, &bytes) {
                return Ok(names);
            }
            return Ok(DocPayload::Hex(HexDocument { bytes, banner }));
        }
        view => view?,
    };
    let resource = || resource.ok_or_else(not_a_start);
    let read = match view {
        View::Xml | View::Text => return Ok(document_io::text_payload(path, bytes)),
        View::Pso => {
            gta6_dialogue::opened(path, &find, &bytes);
            return Ok(DocPayload::Meta(MetaDocument::from_bytes(bytes)));
        }
        View::Headerless => headerless(path, &resource()?, &bytes),
        View::TextTable => gta6_text::Gta6TextDocument::open(resource()?.sections(&bytes).0)
            .inspect(|text| gta6_dialogue::note_table(&text.table))
            .map(|text| DocPayload::Gta6Text(Box::new(text))),
        View::AudioData => gta6_text::Gta6AudioDocument::open(&bytes)
            .map(|audio| DocPayload::Gta6AudioData(Box::new(audio))),
        View::Identified(body) => gta6_other_bodies::payload(path, body, &bytes),
        View::Effects => gta6_effect_facts::payload(path, &bytes),
        View::Movie => Ok(DocPayload::Bik(crate::video::BikDocument::new(&bytes))),
        View::Audio => crate::audio::AwcDocument::open(&bytes).map(DocPayload::Awc),
        View::Animation => animation(&find, &bytes),
        // Views only the list gives: a nested archive's head is read above, and never as one
        // body, and a body no reader fits is returned above.
        View::Archive | View::Bytes => Err(Rpf9ScanError::ExtentNotKnown {
            offset: find.offset,
        }
        .to_string()),
        View::Facts(signature) => {
            let resource = resource()?;
            let (virtual_section, physical) = resource.sections(&bytes);
            match gta6_face::learn(path, signature, virtual_section) {
                Some(gta6_face::Learned::Skeleton(skeleton)) => {
                    skeletons().insert(path.to_path_buf(), Ok(skeleton));
                }
                Some(gta6_face::Learned::Expressions(set)) => {
                    return Ok(DocPayload::Gta6Expressions(Arc::from(set)))
                }
                Some(gta6_face::Learned::Facts(text)) => {
                    return Ok(document_io::text_payload(path, text.into_bytes()))
                }
                None => {}
            }
            if signature.role() == Some(or_io::container::rpf::Rpf9Role::Dictionary) {
                use or_io::gta6::headerless::{read_dictionary, Entry, Resource};
                // ponytail: a dictionary is read again for its facts when none of its textures reads.
                if let Ok(dict) = read_dictionary(virtual_section, Some(physical)) {
                    // A frame filter or animation record opens here too, where its values edit.
                    let edits = |e: &Entry| {
                        matches!(
                            e,
                            Entry::Drawable(_)
                                | Entry::FrameFilter { .. }
                                | Entry::AnimationRecord { .. }
                        )
                    };
                    if (dict.entries.iter()).any(|(_, e)| e.as_ref().is_ok_and(edits)) {
                        ttpf_document::learn_names(&dict);
                        gta6_entry_skeletons::keep(path, virtual_section);
                        return Ok(DocPayload::Gta6Drawable(Arc::new(Resource::Dictionary(
                            dict,
                        ))));
                    }
                    if let Some(textures) = ttpf_document::scanned_textures(path, dict) {
                        return Ok(textures);
                    }
                }
            }
            if let Some(c) = crate::render::gta6_cloth::opened(signature, &resource, &bytes) {
                return Ok(c);
            }
            if let Some(mesh) = gta6_resource::as_ynv(path, &resource, signature, &bytes) {
                return Ok(DocPayload::Resource(or_io::Asset::NavMesh(mesh)));
            }
            if let Some(p) = gta6_particles::opened(path, signature, &resource, &bytes) {
                return Ok(p);
            }
            gta6_resource::facts(&resource, signature, &bytes)
                .map(|text| document_io::text_payload(path, text.into_bytes()))
        }
        View::Drawable => {
            let (virtual_section, physical) = resource()?.sections(&bytes);
            let signature = match find.kind {
                Rpf9BodyKind::Zlib(run) => run.signature,
                _ => None,
            };
            let signature = signature.ok_or_else(not_a_start)?;
            let fragment = match signature.role() {
                Some(or_io::container::rpf::Rpf9Role::Drawable) => Some(false),
                Some(or_io::container::rpf::Rpf9Role::DrawableWithBound) => Some(true),
                _ => None,
            };
            if let Some(fragment) = fragment {
                let skeleton = skeleton_of(virtual_section, fragment);
                skeletons().insert(path.to_path_buf(), skeleton);
            }
            let read = or_io::gta6::headerless::read(signature, virtual_section, Some(physical));
            read.map(|read| {
                let read = Arc::new(read);
                if let Some(fragment) = fragment {
                    gta6_head::keep(path, &read, virtual_section, physical, fragment);
                    gta6_weapon_parts::keep(path, archive, &read);
                }
                DocPayload::Gta6Drawable(read)
            })
            .map_err(|e| {
                tr!(
                    "GTA VI's headerless resource reader does not read this body: {e}",
                    e = e
                )
            })
        }
    };
    // A body its reader refuses is shown as its bytes, under the refusal.
    read.or_else(|banner| Ok(DocPayload::Hex(HexDocument { bytes, banner })))
}
