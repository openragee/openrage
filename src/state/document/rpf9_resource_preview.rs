//! GTA VI previews from table-validated resource sections.

use std::path::Path;

use crate::state::{document::MetaDocument, DocPayload};

use super::preview_error;

pub(super) fn navmesh(
    path: &Path,
    virtual_section: &[u8],
    _physical: &[u8],
) -> Result<DocPayload, String> {
    let cell = or_io::gta6::headerless::read_navmesh(virtual_section).map_err(preview_error)?;
    let mesh = crate::state::gta6_resource::cell_view(&cell).ok_or_else(|| {
        preview_error("the navigation cell cannot be represented by the navigation preview")
    })?;
    crate::state::gta6_navmesh::remember(path, cell);
    Ok(DocPayload::Resource(or_io::Asset::NavMesh(mesh)))
}

pub(super) fn metadata(
    path: &Path,
    virtual_section: &[u8],
    physical: &[u8],
) -> Result<DocPayload, String> {
    let map = crate::state::gta6_map::keep(path, virtual_section, physical);
    crate::state::gta6_world::keep(path, virtual_section, physical);
    let document = match MetaDocument::from_gta_vi_headerless(virtual_section, physical) {
        Ok(document) => document,
        Err(error) if map => {
            let mut document = MetaDocument::empty();
            document.parsed = None;
            document.parse_error = Some(preview_error(error));
            document.source_format = or_io::meta::SourceFormat::Rbm;
            document
        }
        Err(error) => return Err(preview_error(error)),
    };
    Ok(DocPayload::Meta(document))
}

pub(super) fn animation(
    _path: &Path,
    virtual_section: &[u8],
    _physical: &[u8],
) -> Result<DocPayload, String> {
    let resource = or_io::gta6::anim::read(virtual_section).map_err(preview_error)?;
    Ok(DocPayload::Gta6Anim(std::sync::Arc::new(resource)))
}

pub(super) fn expressions(
    _path: &Path,
    virtual_section: &[u8],
    _physical: &[u8],
) -> Result<DocPayload, String> {
    let set = or_io::gta6::anim::read_expressions(virtual_section).map_err(preview_error)?;
    Ok(DocPayload::Gta6Expressions(std::sync::Arc::new(set)))
}

pub(super) fn particles(
    path: &Path,
    virtual_section: &[u8],
    physical: &[u8],
) -> Result<DocPayload, String> {
    crate::state::gta6_particles::from_sections(path, virtual_section, physical)
        .map_err(preview_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::opening_an_rpf9_archive::headerless_rbm;
    use crate::state::{Document, DocumentKind};
    use or_io::container::rpf::{Rpf9Role, Rpf9Signature};

    fn empty_cell() -> Vec<u8> {
        let mut bytes = vec![0; 0x1C0];
        let pointer = |at: usize| 0x5000_0000u64 + at as u64;
        for (at, target) in [
            (0x08, 0xF0),
            (0x70, 0x100),
            (0x80, 0x100),
            (0x88, 0x100),
            (0x98, 0x100),
            (0xA0, 0x140),
            (0x110, 0x130),
            (0x118, 0x130),
            (0x16C, 0x1A0),
        ] {
            bytes[at..at + 8].copy_from_slice(&pointer(target).to_le_bytes());
        }
        bytes[0xF8] = 1;
        bytes[0x10..0x14].copy_from_slice(&9u32.to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&0x0001_0015u32.to_le_bytes());
        for row in 0..3 {
            let at = 0x20 + row * 20;
            bytes[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
        }
        let cell = 23u32 + 512 * 17;
        bytes[0xC8..0xCC].copy_from_slice(&cell.to_le_bytes());
        bytes
    }

    #[test]
    fn a_table_navigation_preview_keeps_the_cell_and_uses_the_navmesh_view() {
        let bytes = empty_cell();
        assert_eq!(
            Rpf9Signature::of(&bytes, bytes.len() as u64)
                .unwrap()
                .role(),
            Some(Rpf9Role::NavMesh),
        );
        let (_dir, table) = super::super::preview_tests::preview_archive(&bytes, 79);
        let source = table.entry(0, 0).unwrap();
        let path = &source.path;
        let payload = source
            .payload()
            .expect("a bounded synthetic cell from its table");
        let document = Document::new(1, Some(path.to_path_buf()), payload);
        assert_eq!(document.kind, DocumentKind::Ynv);
        assert_eq!(document.read_only, Some(super::super::READ_ONLY));
        let DocPayload::Resource(or_io::Asset::NavMesh(view)) = document.payload else {
            panic!("a navigation cell must use the existing navigation view")
        };
        assert_eq!(view.index_of_mesh, 23 + 512 * 17);
        let cell = crate::state::gta6_navmesh::scanned_cell(path).expect("cell facts kept");
        assert_eq!(cell.cell, view.index_of_mesh);
        assert_eq!(cell.nodes, 1);
        assert_eq!(cell.leaves, 1);
        assert!(!path.exists());
    }

    #[test]
    fn a_navigation_preview_refuses_missing_array_bytes() {
        let bytes = empty_cell();
        let path = Path::new("navigation.rpf/.rpf9-table/0/broken_nav.ynv");
        assert!(navmesh(path, &bytes[..0x100], &[]).is_err());
        assert!(crate::state::gta6_navmesh::scanned_cell(path).is_none());
    }

    #[test]
    fn a_table_ymt_reads_headerless_metadata_from_each_stated_section() {
        for (block, physical, expected) in [
            (0x5000_00E0, Vec::new(), 77u64),
            (
                0x6000_0010,
                [vec![0; 0x10], 91u32.to_le_bytes().to_vec()].concat(),
                91,
            ),
        ] {
            let mut bytes = headerless_rbm(block);
            bytes[0xF9] = u8::from(!physical.is_empty());
            let path = Path::new("metadata.rpf/.rpf9-table/0/hash_metadata.ymt");
            let payload = metadata(path, &bytes, &physical).expect("synthetic metadata");
            let document = Document::new(1, Some(path.to_path_buf()), payload);
            assert_eq!(document.kind, DocumentKind::Meta);
            assert_eq!(document.read_only, Some(super::super::READ_ONLY));
            let DocPayload::Meta(meta) = document.payload else {
                panic!("headerless YMT must use the metadata view")
            };
            let fields = &meta.parsed.expect("parsed tree").instance.fields;
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].1, or_io::meta::Value::UInt(expected));
            assert_eq!(meta.source_format, or_io::meta::SourceFormat::Rbm);
            assert!(!path.exists());
        }
    }

    #[test]
    fn a_ymt_preview_refuses_a_missing_physical_block() {
        let mut bytes = headerless_rbm(0x6000_0010);
        bytes[0xF9] = 1;
        let path = Path::new("metadata.rpf/.rpf9-table/0/broken_metadata.ymt");
        assert!(metadata(path, &bytes, &[0; 0x10]).is_err());
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod table_tests {
    use super::*;
    use crate::state::{Document, DocumentKind};
    #[test]
    fn a_ymt_entry_selects_the_metadata_reader_without_extracting() {
        let bytes = crate::state::opening_an_rpf9_archive::headerless_rbm(0x5000_00E0);
        let (dir, table) = super::super::preview_tests::preview_archive(&bytes, 11);
        let source = table.entry(0, 0).unwrap();
        let DocPayload::Meta(meta) = source.payload().unwrap() else {
            panic!("YMT metadata viewer")
        };
        assert_eq!(
            meta.parsed.unwrap().instance.fields[0].1,
            or_io::meta::Value::UInt(77)
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(!source.path.exists());
    }

    #[test]
    fn a_table_animation_keeps_its_clips_and_tracks_without_writable_sections() {
        let bytes = crate::state::gta6_anim_edit::tests::dictionary();
        let path = Path::new("animation.rpf/.rpf9-table/0/hash_animation.ycd");
        let payload = animation(path, &bytes, &[]).expect("synthetic dictionary");
        let document = Document::new(1, Some(path.to_path_buf()), payload);
        assert_eq!(document.kind, DocumentKind::Gta6Anim);
        assert!(document.is_from_rpf9_table());
        let DocPayload::Gta6Anim(resource) = document.payload else {
            panic!("a table animation must use the animation view")
        };
        assert_eq!(resource.name, "dict_a");
        assert_eq!(resource.clips.len(), 1);
        assert_eq!(resource.sets.len(), 1);
        assert_eq!(resource.sets[0].animations.len(), 1);
        assert_eq!(resource.sets[0].animations[0].frames, 2);
        assert!(resource.sections.is_none());
        assert!(!resource.edited);
        assert!(!path.exists());
    }

    #[test]
    fn a_table_animation_refuses_a_missing_track_section() {
        let bytes = crate::state::gta6_anim_edit::tests::dictionary();
        let path = Path::new("animation.rpf/.rpf9-table/0/broken_animation.ycd");
        assert!(animation(path, &bytes[..0xB0], &[]).is_err());
        assert!(!path.exists());
    }

    fn empty_particles() -> Vec<u8> {
        let registry = [
            ("ptxu_Acceleration", 2usize),
            ("ptxu_Rotation", 4),
            ("ptxu_Dampening", 2),
            ("ptxu_MatrixWeight", 1),
            ("ptxu_Size", 4),
            ("ptxu_Colour", 6),
            ("ptxu_AnimateTexture", 1),
            ("ptxu_Attractor", 1),
            ("ptxu_Noise", 1),
            ("ptxu_Collision", 2),
            ("ptxu_ZCull", 2),
            ("ptxu_Wind", 1),
            ("ptxu_Velocity", 1),
            ("ptxu_CameraEvolution", 1),
        ];
        let count: usize = registry.iter().map(|(_, count)| count).sum();
        let dictionaries = 0xB0 + count * 0x10;
        let mut bytes = vec![0; dictionaries + 3 * 0x40];
        let pointer = |at: usize| (0x5000_0000u64 + at as u64).to_le_bytes();
        for (at, to) in [(0x08, 0x70), (0x10, 0x80), (0x20, 0x90), (0x98, 0xB0)] {
            bytes[at..at + 8].copy_from_slice(&pointer(to));
        }
        bytes[0x78] = 1;
        bytes[0x80..0x8D].copy_from_slice(b"table_effects");
        bytes[0xA0..0xA2].copy_from_slice(&(count as u16).to_le_bytes());
        bytes[0xA2..0xA4].copy_from_slice(&(count as u16).to_le_bytes());
        let mut at = 0xB0;
        for (class, count) in registry {
            let hash = or_io::gta6::hash::name_hash32(class.as_bytes());
            for _ in 0..count {
                bytes[at + 0xC..at + 0x10].copy_from_slice(&hash.to_le_bytes());
                at += 0x10;
            }
        }
        for (index, slot) in [0x28, 0x30, 0x38].into_iter().enumerate() {
            let at = dictionaries + index * 0x40;
            bytes[slot..slot + 8].copy_from_slice(&pointer(at));
            bytes[at + 0x18..at + 0x20].copy_from_slice(&1u64.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_table_particle_preview_preserves_all_preview_caches() {
        let bytes = empty_particles();
        let path = Path::new("particles.rpf/.rpf9-table/0/hash_particles.ypt");
        let payload = particles(path, &bytes, &[]).expect("synthetic particle list");
        let document = Document::new(1, Some(path.to_path_buf()), payload);
        assert_eq!(document.kind, DocumentKind::Ypt);
        assert!(document.is_from_rpf9_table());
        let DocPayload::Resource(or_io::Asset::Particles(list)) = document.payload else {
            panic!("a table particle list must use the particle view")
        };
        assert_eq!(list.name, "table_effects");
        assert!(crate::state::gta6_particles::scanned_lines(path).is_some());
        assert!(crate::state::gta6_particles::sprites::of(path).is_some());
        assert!(crate::state::gta6_particles::spawners::of(path).is_some());
        assert!(crate::state::gta6_particle_models::kept(path).is_some());
        assert!(!path.exists());
    }

    #[test]
    fn a_table_particle_preview_refuses_missing_bytes_without_populating_caches() {
        let bytes = empty_particles();
        let path = Path::new("particles.rpf/.rpf9-table/0/broken_particles.ypt");
        assert!(particles(path, &bytes[..bytes.len() - 1], &[]).is_err());
        assert!(crate::state::gta6_particles::scanned_lines(path).is_none());
        assert!(crate::state::gta6_particles::sprites::of(path).is_none());
        assert!(crate::state::gta6_particles::spawners::of(path).is_none());
        assert!(crate::state::gta6_particle_models::kept(path).is_none());
    }

    #[test]
    fn a_table_expression_set_uses_its_existing_inspector() {
        let mut bytes = vec![0; 0x1C0];
        let key = or_io::gta6::hash::name_hash32(b"table_expression");
        bytes[0x08..0x10].copy_from_slice(&0x5000_01B0u64.to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&key.to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&key.to_le_bytes());
        let path = Path::new("expressions.rpf/.rpf9-table/0/hash_expression.yed");
        let payload = expressions(path, &bytes, &[]).expect("an empty expression set");
        let document = Document::new(1, Some(path.to_path_buf()), payload);
        assert_eq!(document.kind, DocumentKind::Gta6Expressions);
        assert!(document.is_from_rpf9_table());
        let DocPayload::Gta6Expressions(set) = document.payload else {
            panic!("an expression set must use the expressions view")
        };
        assert_eq!(set.key, key);
        assert!(set.inputs.tracks.is_empty());
        assert!(set.outputs.is_none());
        assert!(expressions(path, &bytes[..0x80], &[]).is_err());
        assert!(!path.exists());
    }
}
