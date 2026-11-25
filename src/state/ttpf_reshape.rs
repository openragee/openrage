//! A pack's texture given another size, format or mip count, which moves every texture after it,
//! and undone by putting the whole pack back.

use or_io::authoring::{FormatChoice, TextureBuildOptions};
use or_io::codec::texture::RgbaImage;
use or_io::gta6::{Entry, Format, TtpfError};

use super::{edited, Replaced, TtpfDocument};
use crate::state::document::DocPayload;
use crate::state::edit_history::Change;
use crate::state::texture_edit::TextureEdit;

/// Texture `name` of the pack `payload` is, from `image`: replaced in its own shape when the image has its size and `choice` its format, else given the image's size, that format and `options`' mips; `None` for any other document.
pub(in crate::state) fn import(
    payload: &mut DocPayload,
    name: &str,
    image: &RgbaImage,
    choice: FormatChoice,
    options: &TextureBuildOptions,
) -> Option<Result<Replaced, String>> {
    let DocPayload::Ttpf(ttpf) = &*payload else {
        return None;
    };
    let Some((index, entry)) = texture_of(ttpf, name) else {
        return Some(Err(tr!("{name} is not in this pack", name = name)));
    };
    let target = match target(&entry, index, choice, image) {
        Ok(target) => target,
        Err(why) => return Some(Err(why)),
    };
    let levels = ttpf.texture(index).ok().map(|t| u32::from(t.mip_count));
    let same_mips = |mips: u32| levels.is_none_or(|held| held == mips);
    let same = (image.width, image.height) == (entry.width, entry.height)
        && entry.format() == Some(target)
        && options.mip_levels.is_none_or(same_mips);
    if same {
        super::replace(payload, name, image, options)
    } else {
        reshape(payload, name, Some(image), choice, options)
    }
}

/// Texture `name` of the pack `payload` is, encoded from `image`, or from its own top mip when there is none, into `choice`'s format with `options`' mips; `None` for any other document.
pub(in crate::state) fn reshape(
    payload: &mut DocPayload,
    name: &str,
    image: Option<&RgbaImage>,
    choice: FormatChoice,
    options: &TextureBuildOptions,
) -> Option<Result<Replaced, String>> {
    // The shown slice takes `image`; an edit forgets which it is.
    let shown = match &*payload {
        DocPayload::Ttpf(ttpf) => ttpf.index_of(name).map(|index| ttpf.shown_slice(index)),
        _ => None,
    };
    let ttpf = edited(payload)?;
    let Some((index, entry)) = texture_of(ttpf, name) else {
        return Some(Err(tr!("{name} is not in this pack", name = name)));
    };
    if let Some(refused) = super::bcpack_refusal(&ttpf.pack, index) {
        return Some(Err(refused));
    }
    let image = (image, shown.unwrap_or(0));
    Some(reshaped(ttpf, index, &entry, name, image, choice, options))
}

fn reshaped(
    ttpf: &mut TtpfDocument,
    index: usize,
    entry: &Entry,
    name: &str,
    (image, shown): (Option<&RgbaImage>, u32),
    choice: FormatChoice,
    options: &TextureBuildOptions,
) -> Result<Replaced, String> {
    // A cube's, an array's or a volume's every slice takes the new shape: the shown one from
    // `image`, each other from its own top mip resampled to the image's size.
    let sources = (ttpf.pack.slice_sources(index, image, shown, options.filter))
        .map_err(|e| e.to_string())?;
    let source = match image {
        Some(image) => image,
        None => (sources.first()).ok_or_else(|| tr!("{name} is not in this pack", name = name))?,
    };
    let format = target(entry, index, choice, source)?;
    let before = ttpf.pack.clone();
    let mips = ttpf.pack.reshape_slices(index, &sources, format, options);
    let mips = mips.map_err(|e| e.to_string())?;
    // ⚠ Its own key: a replacement's step, which puts back only the replaced rows, must not take this one in.
    let change = Change::part(format!("texture {name} shape"), before, |payload| {
        Some(&mut edited(payload)?.pack)
    });
    let (name, format) = (name.to_string(), format.format());
    let edit = match image {
        Some(_) => TextureEdit::Imported {
            name: name.clone(),
            format,
            mips,
            replaced: true,
        },
        None => TextureEdit::Reencoded {
            name: name.clone(),
            format,
            mips,
        },
    };
    Ok(((tr!("Replace {name}", name = name), change), edit))
}

/// The texture `name` names in `ttpf`, and its entry.
fn texture_of(ttpf: &TtpfDocument, name: &str) -> Option<(usize, Entry)> {
    let index = ttpf.index_of(name)?;
    Some((index, *ttpf.pack.entries.get(index)?))
}

/// The pack format `choice` makes of `image`, sRGB where the texture is; an automatic choice, or the format the texture reads as, keeps its own.
fn target(
    entry: &Entry,
    index: usize,
    choice: FormatChoice,
    image: &RgbaImage,
) -> Result<Format, String> {
    let own = entry.format();
    let kept = match choice {
        FormatChoice::Automatic => own,
        FormatChoice::Fixed(format) => own.filter(|own| own.format() == format),
    };
    if let Some(own) = kept {
        return Ok(own);
    }
    let srgb = own.is_some_and(|f| f.is_srgb());
    let format = choice.resolve(image);
    Format::of_grc(format, srgb).ok_or_else(|| {
        let format = format!("{format:?}");
        TtpfError::ReshapeFormat { index, format }.to_string()
    })
}

#[cfg(test)]
mod tests {
    use or_io::authoring::{FormatChoice, TextureBuildOptions};
    use or_io::codec::texture::{decode_level, GrcTextureFormat, RgbaImage};

    use crate::state::opening_a_ttpf::{cube, open, pack, save_as, written, SIDE};
    use crate::state::texture_edit::TextureEdit;
    use crate::state::textures::{self, TextureSource};
    use crate::state::{AppState, DocPayload, DocumentId};

    fn pack_of(state: &AppState, doc: DocumentId) -> or_io::gta6::TexturePack {
        let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
            panic!("a .ttpf payload");
        };
        ttpf.pack.clone()
    }

    #[test]
    fn a_pack_texture_re_encoded_and_resized_moves_the_one_after_it_undoes_and_says_so_at_save() {
        const BLUE: [u8; 4] = [0, 0, 0xFF, 0xFF];
        let dir = tempfile::tempdir().expect("a scratch directory");
        let source = written(dir.path(), &pack(0xF800));
        let mut state = AppState::new();
        let doc = open(&mut state, source.clone());
        let opened = pack_of(&state, doc);
        let start = state.document(doc).expect("open").history.at();
        let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
            panic!("a .ttpf payload");
        };
        let name = ttpf.name(0);
        let one = TextureBuildOptions {
            mip_levels: Some(1),
            ..TextureBuildOptions::default()
        };
        let at = TextureSource::Document;

        let dxt5 = FormatChoice::Fixed(GrcTextureFormat::DXT5);
        let edit = (state.reencode_texture_in_place(doc, at, &name, dxt5, &one))
            .expect("a pack's texture takes another format");
        assert!(
            matches!(edit, TextureEdit::Reencoded { mips: 1, .. }),
            "{edit:?}"
        );
        let moved = pack_of(&state, doc);
        assert!(moved.is_relaid());
        assert_eq!(moved.entries[0].format(), or_io::gta6::Format::of_dxgi(77));
        let end = moved.rows[moved.rows_of(0).expect("rows").end].virtual_address;
        assert_eq!(
            u64::from(moved.entries[1].page) << 12,
            end,
            "the next texture follows it"
        );
        assert!(moved.entries[1].page > opened.entries[1].page);

        let wide = RgbaImage {
            width: 2 * SIDE.0,
            height: SIDE.1,
            data: BLUE.repeat((2 * SIDE.0 * SIDE.1) as usize),
            is_srgb: false,
        };
        let dxt1 = FormatChoice::Fixed(GrcTextureFormat::DXT1);
        (state.import_texture_over(doc, at, &name, &wide, dxt1, &one))
            .expect("an image of another size is taken");
        let payload = &state.document(doc).expect("open").payload;
        let shown = textures::texture_at(payload, at, &name).expect("shown");
        let decoded = decode_level(&shown, 0).expect("decodes");
        assert_eq!((decoded.width, decoded.height), (wide.width, wide.height));
        assert!(decoded.data.chunks(4).all(|pixel| pixel == BLUE));
        let DocPayload::Ttpf(ttpf) = payload else {
            panic!("a .ttpf payload");
        };
        assert!(
            ttpf.refusal(1).is_some(),
            "the texture after it still says why it does not read"
        );

        let edited = pack_of(&state, doc);
        let done = state.document(doc).expect("open").history.at();
        state.go_to_step(doc, start);
        assert_eq!(pack_of(&state, doc).entries, opened.entries, "undone whole");
        assert!(!pack_of(&state, doc).is_relaid());
        state.go_to_step(doc, done);
        assert_eq!(pack_of(&state, doc).entries, edited.entries, "redone");

        let saved = dir.path().join("saved.ttpf");
        assert!(
            save_as(&mut state, doc, &saved),
            "{}",
            state.error_dialog_message
        );
        assert!(
            (state.toasts.iter()).any(|t| t.message.contains("no game has been seen to load")),
            "the save says the game has not loaded such a pack"
        );
        let reopened = or_io::gta6::TexturePack::open(&saved).expect("the saved pack opens");
        assert_eq!(reopened.entries, edited.entries);
        let texture = reopened
            .texture(0)
            .expect("the new texture reads from the new file");
        let decoded = decode_level(&texture, 0).expect("decodes");
        assert_eq!((decoded.width, decoded.height), (wide.width, wide.height));
        assert!(decoded.data.chunks(4).all(|pixel| pixel == BLUE));
        assert!(
            std::fs::read(&source).expect("read") == pack(0xF800),
            "the source is untouched"
        );
    }

    #[test]
    fn a_pack_cube_re_encoded_takes_the_format_in_every_face_and_undoes_whole() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut state = AppState::new();
        let doc = open(&mut state, written(dir.path(), &cube()));
        let opened = pack_of(&state, doc);
        let start = state.document(doc).expect("open").history.at();
        let Some(DocPayload::Ttpf(ttpf)) = state.document(doc).map(|d| &d.payload) else {
            panic!("a .ttpf payload");
        };
        let name = ttpf.name(0);
        let one = TextureBuildOptions {
            mip_levels: Some(1),
            ..TextureBuildOptions::default()
        };
        let at = TextureSource::Document;
        let dxt1 = FormatChoice::Fixed(GrcTextureFormat::DXT1);
        let edit = (state.reencode_texture_in_place(doc, at, &name, dxt1, &one))
            .expect("a cube takes another format");
        assert!(
            matches!(edit, TextureEdit::Reencoded { mips: 1, .. }),
            "{edit:?}"
        );
        let moved = pack_of(&state, doc);
        assert!(moved.is_relaid());
        assert_eq!(moved.entries[0].format(), or_io::gta6::Format::of_dxgi(71));
        let read = moved.texture(0).expect("the cube reads");
        assert_eq!((read.faces, read.levels), (6, 1));

        // One face's image of another size: the other five take their own top mips at its size.
        let wide = RgbaImage {
            width: 32,
            height: 32,
            data: [0, 0, 0xFF, 0xFF].repeat(32 * 32),
            is_srgb: false,
        };
        (state.import_texture_over(doc, at, &name, &wide, dxt1, &one))
            .expect("the cube takes the image's size");
        let read = pack_of(&state, doc).texture(0).expect("the cube reads");
        assert_eq!((read.width, read.height, read.faces), (32, 32, 6));
        let shown = decode_level(&read.face(0).expect("face 0"), 0).expect("decodes");
        assert!(shown
            .data
            .chunks(4)
            .all(|pixel| pixel == [0, 0, 0xFF, 0xFF]));
        state.go_to_step(doc, start);
        assert_eq!(pack_of(&state, doc).entries, opened.entries, "undone whole");
    }
}
