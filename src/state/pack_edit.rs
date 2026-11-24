use std::sync::Arc;

use or_editor::{Edit, EditKind};
use or_io::desc::{ImageMut, Scalar, ScalarValue};

use super::{AppState, Change, DocumentId};

pub(super) fn scalar(kind: EditKind) -> Scalar {
    match kind {
        EditKind::Bool => Scalar::Bool,
        EditKind::F32 => Scalar::F32,
        EditKind::F64 => Scalar::F64,
        EditKind::Int(width) | EditKind::Enum(width) => Scalar::Int(width),
    }
}

/// Write `value` at `at` as `kind`; false, with nothing written, where the image refuses it.
pub fn apply(image: &mut ImageMut<'_>, at: usize, kind: EditKind, value: ScalarValue) -> bool {
    image.put_scalar(at, scalar(kind), value).is_ok()
}

impl AppState {
    /// Write `value` where `edit` points in `doc`'s resource image, as the step `Set {field}`.
    pub fn patch_scalar(
        &mut self,
        doc: DocumentId,
        field: &str,
        edit: Edit,
        value: ScalarValue,
    ) -> bool {
        self.patch_scalars(doc, format!("Set {field}"), &[(edit, value)])
    }

    /// Write each value where its edit points in `doc`'s resource image, all or none, as the one step `label`.
    pub fn patch_scalars(
        &mut self,
        doc: DocumentId,
        label: String,
        writes: &[(Edit, ScalarValue)],
    ) -> bool {
        let Some(image) = self
            .document_mut(doc)
            .filter(|d| d.read_only != Some(super::document::rpf9::entry::READ_ONLY))
            .and_then(|d| d.resource.as_mut())
        else {
            return false;
        };
        let span = |edit: &Edit| edit.offset..edit.offset + scalar(edit.kind).size();
        let at = writes.iter().map(|(edit, _)| span(edit).start).min();
        let end = writes.iter().map(|(edit, _)| span(edit).end).max();
        let Some((at, before)) = at
            .zip(end)
            .and_then(|(at, end)| Some((at, image.data.get(at..end)?.to_vec())))
        else {
            return false;
        };
        let image = Arc::make_mut(image);
        let written = {
            let mut view = image.view_mut();
            writes
                .iter()
                .all(|(edit, value)| apply(&mut view, edit.offset, edit.kind, value.clone()))
        };
        if !written {
            if let Some(bytes) = image.data.get_mut(at..at + before.len()) {
                bytes.copy_from_slice(&before);
            }
            return false;
        }
        let change = Change::Bytes { at, other: before };
        self.record(doc, label, change);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::desc::{Image, IntWidth, Pack};
    use or_io::gta5::{File as _, GtaDrawable};

    #[test]
    fn a_float_written_at_an_offset_reads_back() {
        let mut bytes = vec![0u8; 0x40];
        {
            let mut image = ImageMut::new(&mut bytes, 0x20);
            assert!(apply(
                &mut image,
                0x10,
                EditKind::F32,
                ScalarValue::F32(42.5)
            ));
        }
        assert_eq!(
            f32::from_le_bytes(bytes[0x10..0x14].try_into().unwrap()),
            42.5
        );
    }

    #[test]
    fn an_integer_is_written_at_its_declared_width_and_no_wider() {
        let mut bytes = vec![0xFFu8; 0x20];
        {
            let mut image = ImageMut::new(&mut bytes, 0x10);
            assert!(apply(
                &mut image,
                4,
                EditKind::Int(IntWidth::U16),
                ScalarValue::Int(0x1234)
            ));
        }
        assert_eq!(&bytes[4..6], &0x1234u16.to_le_bytes());
        assert_eq!(bytes[3], 0xFF);
        assert_eq!(bytes[6], 0xFF);
    }

    #[test]
    fn a_value_too_wide_for_its_field_is_refused_and_writes_nothing() {
        let mut bytes = vec![0u8; 0x20];
        let mut image = ImageMut::new(&mut bytes, 0x10);
        assert!(!apply(
            &mut image,
            4,
            EditKind::Int(IntWidth::U16),
            ScalarValue::Int(70_000)
        ));
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn a_bool_is_written_as_one_rather_than_any_non_zero() {
        let mut bytes = vec![0u8; 0x10];
        {
            let mut image = ImageMut::new(&mut bytes, 0x8);
            assert!(apply(
                &mut image,
                2,
                EditKind::Bool,
                ScalarValue::Bool(true)
            ));
        }
        assert_eq!(bytes[2], 1);
    }

    #[test]
    fn a_write_past_the_end_is_refused_rather_than_panicking() {
        let mut bytes = vec![0u8; 0x10];
        let mut image = ImageMut::new(&mut bytes, 0x8);
        assert!(!apply(
            &mut image,
            0x20,
            EditKind::F32,
            ScalarValue::F32(1.0)
        ));
    }

    fn staged_drawable() -> Option<std::path::PathBuf> {
        or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydr")
            .find()
            .into_iter()
            .min_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX))
    }

    /// The built-in drawable description for `target`, and its root type.
    fn described_drawable(target: or_io::Generation) -> (std::sync::Arc<Pack>, String) {
        let format = or_io::gta5::formats::format("ydr").expect(".ydr is declared");
        (
            format.description(target).expect("a description"),
            format.root_name(target).expect("a root"),
        )
    }

    fn target_of(image: &crate::state::ResourceImage) -> or_io::Generation {
        or_io::gta5::formats::format("ydr")
            .and_then(|f| f.target_of_version(image.seal.version()).ok())
            .expect("a declared .ydr version")
    }

    fn first_lod_distance(pack: &Pack, root: &str, image: Image<'_>) -> Option<or_editor::Edit> {
        or_editor::Summary::of(pack, root, image, 0)?
            .rows
            .iter()
            .find(|r| r.label.ends_with("Distances [0]"))?
            .edit
    }

    #[test]
    fn a_patched_distance_survives_a_save_and_reopen() {
        let Some(path) = staged_drawable() else {
            or_test_support::skip_corpus(
                "a_patched_distance_survives_a_save_and_reopen",
                "no staged .ydr",
            );
            return;
        };
        let original = std::fs::read(&path).expect("read");
        let mut image = crate::state::document_io::resource_image_for(
            crate::state::DocumentKind::Ydr,
            &original,
        )
        .map(|arc| (*arc).clone())
        .expect("a .ydr keeps its bytes");
        let (pack, root) = described_drawable(target_of(&image));

        let edit = first_lod_distance(&pack, &root, image.view())
            .expect("the summary reaches a drawable's LOD distances and offers a handle");
        let before = image
            .view()
            .at(edit.offset)
            .bytes()
            .get(edit.offset..edit.offset + 4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .expect("readable");

        let before_bytes = image.data.clone();
        let patched_value = before + 137.5;
        assert!(apply(
            &mut image.view_mut(),
            edit.offset,
            edit.kind,
            ScalarValue::F32(patched_value)
        ));

        let differing = before_bytes
            .iter()
            .zip(&image.data)
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing <= 4, "a scalar patch touched {differing} bytes");

        let written = image.to_resource_bytes().expect("re-envelope");
        let reopened = crate::state::document_io::resource_image_for(
            crate::state::DocumentKind::Ydr,
            &written,
        )
        .map(|arc| (*arc).clone())
        .expect("what we wrote is still a resource");
        assert_eq!(
            reopened.data, image.data,
            "re-enveloping changed the payload; it should carry it verbatim"
        );

        let parsed = <GtaDrawable>::from_bytes(&written).expect("reopens as a drawable");
        assert!(
            (parsed.base.lod_group.lod_distances[0] - patched_value).abs() < 0.001,
            "the reader sees {} where the patch wrote {patched_value}",
            parsed.base.lod_group.lod_distances[0]
        );
    }

    #[test]
    fn patching_preserves_what_re_serializing_would_rebuild() {
        let Some(path) = staged_drawable() else {
            or_test_support::skip_corpus(
                "patching_preserves_what_re_serializing_would_rebuild",
                "no staged .ydr",
            );
            return;
        };
        let original = std::fs::read(&path).expect("read");
        let mut image = crate::state::document_io::resource_image_for(
            crate::state::DocumentKind::Ydr,
            &original,
        )
        .map(|arc| (*arc).clone())
        .expect("keeps its bytes");

        let (pack, root) = described_drawable(target_of(&image));
        let edit = first_lod_distance(&pack, &root, image.view()).expect("a handle");
        assert!(apply(
            &mut image.view_mut(),
            edit.offset,
            edit.kind,
            ScalarValue::F32(123.0)
        ));

        let patched = image.to_resource_bytes().expect("re-envelope");
        let rebuilt = <GtaDrawable>::from_bytes(&original)
            .and_then(|parsed| parsed.to_bytes())
            .expect("re-serialize");

        let decompress = |bytes: &[u8]| {
            <or_io::container::resource::Rsc7 as or_io::container::resource::ResourceEnvelope>::open(bytes)
                .expect("resource")
                .data
        };
        let from_disk = decompress(&original);
        let differs = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();

        let patch_delta = differs(&from_disk, &decompress(&patched));
        let rebuild_delta = differs(&from_disk, &decompress(&rebuilt));
        assert!(
            patch_delta <= 4,
            "patching moved {patch_delta} bytes; it should move four"
        );
        assert!(
            patch_delta <= rebuild_delta,
            "patching moved {patch_delta} bytes and re-serializing {rebuild_delta}; \
             the patch path exists because it should never be the larger"
        );
    }

    #[test]
    fn no_count_in_a_real_drawable_is_offered_for_editing() {
        let format = or_io::gta5::formats::format("ydr").expect(".ydr is declared");
        for target in format.targets() {
            let (pack, root) = described_drawable(target);
            let bytes = vec![0u8; 0x2000];
            let summary = or_editor::Summary::of(&pack, &root, Image::new(&bytes, 0x1000), 0)
                .expect("the description describes a drawable");

            for row in summary.editable() {
                let label = row.label.to_ascii_lowercase();
                assert!(
                    !label.contains("count") && !label.contains("capacity"),
                    "{} is offered as editable for {} and looks structural",
                    row.label,
                    target.slug()
                );
            }
            assert!(
                summary.editable().count() > 0,
                "nothing at all is editable for {}, which means the walk found no scalars",
                target.slug()
            );
        }
    }

    #[test]
    fn the_grid_reaches_a_drawables_lod_distances() {
        let format = or_io::gta5::formats::format("ydr").expect(".ydr is declared");
        for target in format.targets() {
            let (pack, root) = described_drawable(target);
            let bytes = vec![0u8; 0x2000];
            let edit =
                first_lod_distance(&pack, &root, Image::new(&bytes, 0x1000)).unwrap_or_else(|| {
                    panic!(
                        "a drawable's first LOD distance should be reachable and editable for {}",
                        target.slug()
                    )
                });
            assert!(
                matches!(edit.kind, or_editor::EditKind::F32),
                "a switch distance is an f32, got {:?}",
                edit.kind
            );
            assert!(edit.offset > 0, "it is embedded, so not at the root");
        }
    }
}
