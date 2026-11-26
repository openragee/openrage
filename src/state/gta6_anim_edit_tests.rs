use std::io::Cursor;
use std::time::{Duration, Instant};

use or_io::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form, Rpf9Sections};
use or_io::gta6::anim::{AnimAt, AnimEdit, AnimResource, ClipPlay, Dof};
use or_io::gta6::hash::name_hash32;

use crate::state::opening_an_rpf9_archive::{said, settle};
use crate::state::{AppState, Change, DocPayload, DocumentId, DocumentKind, ToastType};

const SCALE: f32 = 0.5;
const OFFSET: f32 = -2.0;
const DURATION: f32 = 1.0 / 30.0;

fn hash(name: &str) -> u32 {
    name_hash32(name.as_bytes())
}

/// A virtual section being laid out, every object on 16 bytes.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn alloc(&mut self, len: usize) -> usize {
        let at = self.0.len().next_multiple_of(16);
        self.0.resize(at + len, 0);
        at
    }
    fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
    fn ptr(&mut self, at: usize, to: usize) {
        self.put(at, &(0x5000_0000 + to as u64).to_le_bytes());
    }
    /// An array `{pointer, u16 count}` of `count` elements from `first`.
    fn array(&mut self, at: usize, first: usize, count: u16) {
        self.ptr(at, first);
        self.put(at + 8, &count.to_le_bytes());
    }
    /// A map of one entry: one bucket, its key at 0, its value at 8.
    fn map(&mut self, at: usize, key: &[u8], value: Option<usize>) {
        let entry = self.alloc(0x18);
        self.put(entry, key);
        if let Some(value) = value {
            self.ptr(entry + 8, value);
        }
        let bucket = self.alloc(8);
        self.ptr(bucket, entry);
        self.ptr(at, bucket);
        self.u32(at + 8, 1);
        self.u32(at + 0x10, 1);
        // The flag byte every measured map holds, which a scan takes a root's by.
        self.0[at + 0x17] = 1;
    }
    fn inline_name(&mut self, at: usize, name: &str) {
        self.put(at, name.as_bytes());
        self.0[at + 15] = 0x80 | name.len() as u8;
    }
}

/// The section-10 channel of the steps `[0, 1]`: one segment, one group of width 2 whose one
/// code is 2, its unary one at bit 130, its lanes one word each.
fn channel() -> Vec<u32> {
    let mut words = vec![13, 1 << 16, SCALE.to_bits(), OFFSET.to_bits(), 1 << 2];
    words.extend([2, 0, 0, 0, 0, 0, 0, 0]);
    words
}

/// The section-10 channel of the steps `[0, 65535]`: its one code 131,070 is 17 bits wide, its
/// unary one at bit 145, so an edit to `[65535, 0]` adds 16 initial bits and a header word.
fn wide_channel() -> Vec<u32> {
    let mut words = vec![13, 1 << 16, SCALE.to_bits(), OFFSET.to_bits(), 1 << 17];
    words.extend([131_070, 0, 0, 0, 0, 0, 0, 0]);
    words
}

/// A clip dictionary `dict_a` of one clip of one animation of 2 frames, whose one DOF's channel
/// is in section 10.
pub(crate) fn dictionary() -> Vec<u8> {
    dictionary_with(channel())
}

/// [`dictionary`] with its channel's words `channel`, 13 of them.
fn dictionary_with(channel: Vec<u32>) -> Vec<u8> {
    let mut b = Bytes::default();
    let root = b.alloc(0xB0);
    let block_map = b.alloc(0x18);
    b.0[block_map + 8] = 1;
    b.ptr(root + 0x08, block_map);
    b.inline_name(root + 0x38, "dict_a");
    b.u32(root + 0x68, hash("dict_a"));
    b.u32(root + 0x6C, 2);

    let set = b.alloc(0x80);
    b.inline_name(set + 0x38, "dict_a-0");
    b.u32(set + 0x68, hash("dict_a-0"));
    let list = b.alloc(16);
    b.ptr(list, set);
    b.u32(list + 8, hash("dict_a-0"));
    b.u32(list + 12, 1);
    b.array(root + 0x70, list, 1);

    let animation = b.alloc(0x70);
    b.u32(animation + 0x10, 2);
    b.ptr(animation + 0x18, set);
    b.put(animation + 0x20, &0x140u16.to_le_bytes());
    b.put(animation + 0x24, &2u16.to_le_bytes());
    b.put(animation + 0x26, &15u16.to_le_bytes());
    b.put(animation + 0x28, &DURATION.to_le_bytes());
    let name = b.alloc(16);
    b.put(name, b"pack:/walk");
    b.ptr(animation + 0x30, name);
    let dof = b.alloc(12);
    b.u32(dof, hash("FACIAL_C_Jaw"));
    b.u32(dof + 4, hash("facialTranslation"));
    b.u32(dof + 8, 4);
    b.array(animation + 0x60, dof, 1);
    let mut data: Vec<u8> = channel.iter().flat_map(|w| w.to_le_bytes()).collect();
    let constant_size = data.len() as u32;
    let mut directory = vec![0u16; 11];
    directory[10] = 1;
    directory.extend([0, 4, 4, 4]);
    data.extend(directory.iter().flat_map(|w| w.to_le_bytes()));
    let block = b.alloc(0x20 + data.len());
    b.u32(block + 4, data.len() as u32);
    b.u32(block + 0xC, constant_size);
    b.u32(block + 0x10, 0x20 + data.len() as u32);
    b.put(block + 0x14, &2u16.to_le_bytes());
    b.put(block + 0x16, &2u16.to_le_bytes());
    b.0[block + 0x1E] = 11;
    b.put(block + 0x20, &data);
    let blocks = b.alloc(8);
    b.ptr(blocks, block);
    b.array(animation + 0x50, blocks, 1);
    b.u32(animation + 0x48, 0x20 + data.len() as u32);
    b.map(set + 0x48, &hash("walk").to_le_bytes(), Some(animation));
    b.map(root + 0x80, &hash("walk").to_le_bytes(), None);

    let clip = b.alloc(0xA0);
    let key = u64::from(hash("dict_a")) << 32 | u64::from(hash("clip_a"));
    b.put(clip + 0x10, &1u64.to_le_bytes());
    b.ptr(clip + 0x18, root);
    b.u32(clip + 0x20, 1);
    b.inline_name(clip + 0x28, "clip_a.clip");
    b.put(clip + 0x58, &key.to_le_bytes());
    b.ptr(clip + 0x68, animation);
    b.u32(clip + 0x70, hash("walk"));
    b.put(clip + 0x94, &DURATION.to_le_bytes());
    b.put(clip + 0x98, &1.0f32.to_le_bytes());
    b.map(root + 0x48, &key.to_le_bytes(), Some(clip));
    let properties = b.alloc(0x28);
    b.put(properties + 0x20, &1u64.to_le_bytes());
    b.ptr(clip + 0x48, properties);
    b.ptr(root + 0xA0, properties);
    b.u32(root + 0xA8, 0x28);
    b.0
}

fn held(state: &AppState, doc: DocumentId) -> std::sync::Arc<AnimResource> {
    let Some(DocPayload::Gta6Anim(resource)) = state.document(doc).map(|d| &d.payload) else {
        panic!("not a GTA VI animation document");
    };
    resource.clone()
}

fn key_at(state: &AppState, doc: DocumentId, frame: u16) -> Option<f32> {
    held(state, doc).sets[0].animations[0].delta_value(0, 0, frame)
}

fn key(frame: u16, value: f32) -> AnimEdit {
    AnimEdit::Key {
        animation: AnimAt {
            set: 0,
            animation: 0,
        },
        dof: 0,
        component: 0,
        frame,
        value,
    }
}

fn read_back(path: &std::path::Path) -> Rpf9Sections {
    let bytes = std::fs::read(path).expect("the file");
    let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
    Rpf9Sections::of(&find, &read).expect("a resource")
}

#[test]
fn a_gta_vi_animation_s_key_and_clip_window_edit_undo_and_save_as_a_loose_resource() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("dict_a.rsc");
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: dictionary(),
        physical: Vec::new(),
    };
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path.clone());
    settle(&mut state);
    let doc = state
        .active_document
        .unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));
    assert_eq!(
        state.document(doc).map(|d| d.kind),
        Some(DocumentKind::Gta6Anim)
    );
    assert!(held(&state, doc).sections.is_some() && !held(&state, doc).edited);
    let (first, second) = (OFFSET, SCALE + OFFSET);
    assert_eq!(key_at(&state, doc, 1), Some(second));

    // A key: made, undone and made again through the document's history.
    assert!(state.edit_gta6_key(doc, key(1, 7.0)));
    assert_eq!(key_at(&state, doc, 1), Some(7.0));
    assert_eq!(key_at(&state, doc, 0), Some(first));
    assert!(state.undo().is_some());
    assert_eq!(key_at(&state, doc, 1), Some(second));
    assert!(!held(&state, doc).edited);
    assert!(state.redo().is_some());
    assert_eq!(key_at(&state, doc, 1), Some(7.0));
    state.toasts.clear();
    assert!(!state.edit_gta6_key(doc, key(2, 1.0)));
    assert!(said(&state, ToastType::Error, &["frame"]));

    // The clip's window, through the clip fields GTA V's clips edit.
    let before = state
        .clip_field_snapshot(doc, 0)
        .expect("a clip of one animation");
    let mut after = before.clone();
    after.start_time = 0.01;
    after.rate = 2.0;
    assert!(state.apply_clip_fields(doc, 0, &after));
    state.record(
        doc,
        "Edit clip fields",
        Change::ClipFields {
            clip_key: 0,
            other: before.clone(),
        },
    );
    assert_eq!(state.clip_field_snapshot(doc, 0), Some(after.clone()));
    assert!(state.undo().is_some());
    assert_eq!(state.clip_field_snapshot(doc, 0), Some(before));
    assert!(state.redo().is_some());

    // Save writes the edited sections over the loose file, which reads back as edited.
    assert!(state.save_active_document());
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let saved = read_back(&path);
    let edited = held(&state, doc);
    let mut expected = edited
        .sections
        .as_deref()
        .expect("sections")
        .virtual_section
        .clone();
    expected.resize(expected.len().next_multiple_of(16), 0);
    assert_eq!(saved.virtual_section, expected);
    let read = or_io::gta6::anim::read(&saved.virtual_section).expect("reads");
    assert_eq!(read.sets[0].animations[0].delta_value(0, 0, 1), Some(7.0));
    assert!(
        matches!(read.clips[0].play, ClipPlay::One { start, rate, .. } if start == 0.01 && rate == 2.0)
    );
}

#[test]
fn a_key_edit_that_grows_its_block_says_at_once_that_its_archive_will_be_rebuilt() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("dict_a.rsc");
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: dictionary_with(wide_channel()),
        physical: Vec::new(),
    };
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path);
    settle(&mut state);
    let doc = state
        .active_document
        .unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));
    let length = |state: &AppState| {
        let resource = held(state, doc);
        resource
            .sections
            .as_deref()
            .expect("sections")
            .virtual_section
            .len()
    };
    let told = |state: &AppState| {
        (state.toasts.iter())
            .filter(|t| t.toast_type == ToastType::Warning && t.message.contains("rebuilt"))
            .count()
    };
    let read = length(&state);
    state.toasts.clear();

    // Frame 1 halfway up the range: the steps span 0 to 65,535 again, in the same words.
    assert!(state.edit_gta6_key(doc, key(1, OFFSET + SCALE * 30_000.0)));
    assert_eq!(length(&state), read);
    assert_eq!(told(&state), 0);

    // Frame 0 above the range: the steps become [65535, 0], and the block outgrows its extent.
    assert!(state.edit_gta6_key(doc, key(0, 40_000.0)));
    assert!(length(&state) > read, "the block grew");
    assert_eq!(told(&state), 1);
    assert!(state.undo().is_some());
    assert!(state.edit_gta6_key(doc, key(0, 40_000.0)));
    assert_eq!(told(&state), 1, "the sentence stands once");
}

#[test]
fn a_track_added_held_over_the_clip_and_removed_is_each_a_step_that_undoes() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("dict_a.rsc");
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: dictionary(),
        physical: Vec::new(),
    };
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path);
    settle(&mut state);
    let doc = state
        .active_document
        .unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));
    let at = AnimAt {
        set: 0,
        animation: 0,
    };
    let dofs = |state: &AppState| held(state, doc).sets[0].animations[0].dofs.clone();
    let read = dofs(&state);
    let new = Dof {
        id: hash("SKEL_Head"),
        track: hash("genericControl"),
        kind: 4,
    };
    let add = AnimEdit::AddDof {
        animation: at,
        dof: new,
        value: [0.25, 0.0, 0.0, 0.0],
    };
    assert!(state.relay_gta6_anim(doc, add));
    let added = dofs(&state);
    assert_eq!(added.len(), 2);
    let place = (added.iter().position(|d| *d == new)).expect("added");
    assert!(held(&state, doc).sets[0].animations[0].signature_holds());
    let pose = AnimEdit::Pose {
        animation: at,
        dof: place,
        value: [0.5, 0.0, 0.0, 0.0],
    };
    assert!(state.relay_gta6_anim(doc, pose));
    let posed = held(&state, doc);
    assert_eq!(
        posed.sets[0].animations[0].key_value(place, 0, 1),
        Some(0.5)
    );
    let remove = AnimEdit::RemoveDof {
        animation: at,
        dof: 1 - place,
    };
    assert!(state.relay_gta6_anim(doc, remove));
    assert_eq!(dofs(&state), vec![new]);
    state.toasts.clear();
    let past = AnimEdit::RemoveDof {
        animation: at,
        dof: 4,
    };
    assert!(!state.relay_gta6_anim(doc, past));
    assert!(said(&state, ToastType::Error, &["DOF"]));

    for _ in 0..3 {
        assert!(state.undo().is_some());
    }
    assert_eq!(dofs(&state), read);
    assert!(!held(&state, doc).edited);
}
