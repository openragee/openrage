use super::*;
use crate::gta6::hash::name_hash32;

const D: ExpressionGroup = ExpressionGroup::FacialOutputs;

fn translation() -> u32 {
    name_hash32(b"facialTranslation")
}

fn group(tracks: &[(u32, u32)], slots: &[u16], components: &[u16]) -> ExpressionTracks {
    ExpressionTracks {
        tracks: tracks.to_vec(),
        slots: slots.to_vec(),
        components: components.to_vec(),
    }
}

/// A set reading facial controls A1 (1 component), A2 (2) and A3 (1), writing `written` of them
/// as group D, then driving one bone of three slots as group O.
fn set(written: &[(u32, u16)]) -> Expressions {
    let t = translation();
    let d_tracks: Vec<(u32, u32)> = written.iter().map(|(id, _)| (*id, t)).collect();
    let d_components: Vec<u16> = written.iter().map(|(_, c)| *c).collect();
    let m: u16 = d_components.iter().sum();
    Expressions {
        key: 0x1122_3344,
        inputs: group(
            &[(0xA1, t), (0xA2, t), (0xA3, t)],
            &[0, 1, 2, 3],
            &[1, 2, 1],
        ),
        bones: group(&[], &[], &[]),
        bone_masks: Vec::new(),
        controls: group(&[], &[], &[]),
        control_slots: Vec::new(),
        control_words: Vec::new(),
        facial_outputs: group(&d_tracks, &(0..m).collect::<Vec<u16>>(), &d_components),
        outputs: Some(group(
            &[(0xE1, name_hash32(b"boneTranslation"))],
            &[m, m + 1, m + 2],
            &[0],
        )),
        output_masks: vec![0x87],
    }
}

/// `set` without every track of group D, last first.
fn emptied(set: &Expressions) -> Expressions {
    let mut out = set.clone();
    while !out.facial_outputs.tracks.is_empty() {
        let last = out.facial_outputs.tracks.len() - 1;
        out = remove_expression_track(&out, D, last).expect("removed");
    }
    out
}

#[test]
fn a_set_emptied_of_its_d_tracks_regains_its_first_d_track_as_stored() {
    for (id, c) in [(0xA1, 1u16), (0xA2, 2)] {
        let one = set(&[(id, c)]);
        let stored = lay_out_expressions(&one, 0).expect("laid out");
        let none = emptied(&one);
        let mut bytes = stored.clone();
        write_edited_expressions(&mut bytes, 0, &none).expect("written");
        assert_eq!(read_expressions(&bytes).expect("reads"), none);

        let again = add_expression_track(&none, D, 0, id).expect("the first track added");
        assert_eq!(again, one);
        write_edited_expressions(&mut bytes, 0, &again).expect("written");
        assert_eq!(
            bytes, stored,
            "the first track of {id:#X} gives the stored bytes"
        );
    }
    // Of two, the first comes back alone, in its own slots, and group O follows it.
    let two = set(&[(0xA2, 2), (0xA3, 1)]);
    let first = add_expression_track(&emptied(&two), D, 0, 0xA2).expect("added");
    assert_eq!(first, set(&[(0xA2, 2)]));
}

#[test]
fn the_first_d_track_is_only_a_facial_translation_group_a_reads_from_track_0() {
    let none = set(&[]);
    for (from, id) in [(1, 0xA1), (0, 0xD9)] {
        let r = add_expression_track(&none, D, from, id);
        assert!(
            matches!(r, Err(AnimError::Unwritten(_))),
            "{from}, {id:#X}: {r:?}"
        );
    }
    // A control group A reads on another word is not written as group D.
    let mut rotation = none.clone();
    rotation.inputs.tracks[0].1 = name_hash32(b"facialRotation");
    assert!(add_expression_track(&rotation, D, 0, 0xA1).is_err());
}

#[test]
fn a_net_zero_shape_change_saves_and_reads_back_equal_in_values() {
    let d = set(&[(0xA1, 1), (0xA3, 1)]);
    let stored = lay_out_expressions(&d, 0).expect("laid out");
    // D keeps two tracks: a 1-component one gives way to a 2-component one.
    let edited = remove_expression_track(&d, D, 0).expect("removed");
    let edited = add_expression_track(&edited, D, 0, 0xA2).expect("added");
    assert_eq!(edited.facial_outputs.tracks.len(), 2);
    let mut bytes = stored.clone();
    write_edited_expressions(&mut bytes, 0, &edited).expect("written");
    assert_eq!(read_expressions(&bytes).expect("reads"), edited);
    assert_ne!(bytes, stored);

    // A keeps three tracks: a 1-component control gives way to a copy of a 2-component one.
    let plain = set(&[]);
    let plain_stored = lay_out_expressions(&plain, 0).expect("laid out");
    let edited = remove_expression_track(&plain, ExpressionGroup::Inputs, 2).expect("removed");
    let edited = add_expression_track(&edited, ExpressionGroup::Inputs, 1, 0xA9).expect("added");
    assert_eq!(edited.inputs.tracks.len(), 3);
    let mut bytes = plain_stored.clone();
    write_edited_expressions(&mut bytes, 0, &edited).expect("written");
    assert_eq!(read_expressions(&bytes).expect("reads"), edited);

    // An edit of ids alone is still written where they lie.
    let mut renamed = d.clone();
    renamed.inputs.tracks[1].0 = 0x0BAD_F00D;
    let mut bytes = stored.clone();
    write_edited_expressions(&mut bytes, 0, &renamed).expect("written");
    assert_eq!(bytes.len(), stored.len());
    assert_eq!(read_expressions(&bytes).expect("reads"), renamed);
}
