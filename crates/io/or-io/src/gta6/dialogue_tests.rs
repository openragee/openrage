use super::*;
use crate::gta6::text::{Variant, Variants};

const KEY: &str = "MIS1_INTAU_AB12";

/// A body that states each `(key, label)` as two NUL-terminated names, bytes of no name before
/// each key and zeros after each label.
fn body(stated: &[(&str, &str)]) -> Vec<u8> {
    let mut out = b"PSIN\x01\x02".to_vec();
    for (key, label) in stated {
        out.extend([0xFF, 0x80, 0x00]);
        out.extend(key.as_bytes());
        out.push(0);
        out.extend(label.as_bytes());
        out.extend([0, 0, 0, 0]);
    }
    out
}

fn line(number: usize, label: &str) -> DialogueLine {
    DialogueLine {
        key: KEY.to_string(),
        number,
        label: label.to_string(),
    }
}

/// A one-group table of `texts`, each under its label's hash.
fn table(texts: &[(&str, TextItem)]) -> TextTable {
    let entries = (texts.iter().enumerate())
        .map(|(slot, (label, item))| TextEntry {
            key: name_hash32(label.as_bytes()),
            group: 0,
            slot: slot as u8,
            item: item.clone(),
        })
        .collect();
    TextTable { groups: 1, entries }
}

#[test]
fn a_key_s_distinct_labels_number_its_lines_in_the_order_found() {
    let stated = [
        (KEY, "MIS1_INT_JD_1"),
        ("OTHR_AAAAU_0001", "OTHR_90s_1"),
        (KEY, "MIS1_INT_LUC_2"),
        (KEY, "MIS1_INT_JD_1"),
    ];
    let bytes = body(&stated);
    assert_eq!(pairs(&bytes), stated);

    let mut dialogue = Dialogue::default();
    assert!(dialogue.is_empty());
    assert_eq!(dialogue.add(&bytes), 4);
    // A later body's labels number from 1 again, and a label found before keeps its number.
    let later = [(KEY, "MIS1_INT_JD_3"), (KEY, "MIS1_INT_LUC_2")];
    assert_eq!(dialogue.add(&body(&later)), 2);
    let lines: Vec<DialogueLine> = dialogue.lines().collect();
    let want = [
        line(1, "MIS1_INT_JD_1"),
        line(2, "MIS1_INT_LUC_2"),
        line(1, "MIS1_INT_JD_3"),
        DialogueLine {
            key: "OTHR_AAAAU_0001".into(),
            number: 1,
            label: "OTHR_90s_1".into(),
        },
    ];
    assert_eq!(lines, want);
    assert_eq!(lines[1].wave(), "MIS1_INTAU_AB12_02");
    assert_eq!(
        lines[1].stream_id(),
        name_hash32(b"mis1_intau_ab12_02") & 0x1FFF_FFFF
    );
    let speakers: Vec<_> = lines.iter().map(|l| l.speaker()).collect();
    // Three capitals before the number are no speaker's initials.
    assert_eq!(speakers, [Some("JD"), None, Some("JD"), None]);
}

#[test]
fn only_a_key_shaped_name_before_a_terminated_label_is_a_pair() {
    let none: [&[u8]; 7] = [
        // No root before `AU_`, a lower-case code, a code of three.
        b"\0AU_AB12\0LABEL_1\0",
        b"\0MIS1AU_ab12\0LABEL_1\0",
        b"\0MIS1AU_AB1\0LABEL_1\0",
        // An empty label, a label that is no name, a label no NUL ends, a label that is a key.
        b"\0MIS1AU_AB12\0\0",
        b"\0MIS1AU_AB12\0a label\0",
        b"\0MIS1AU_AB12\0LABEL_1",
        b"\0MIS1AU_AB12\0OTHRAU_0001\0\0",
    ];
    for bytes in none {
        assert!(
            pairs(bytes).is_empty(),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
    // The key is the name that ends its run, whatever comes before it.
    assert_eq!(
        pairs(b"\x07 x.MIS1AU_AB12\0LABEL_1\0"),
        [("MIS1AU_AB12", "LABEL_1")]
    );
    assert!(pairs(b"").is_empty());
}

#[test]
fn a_line_joins_its_text_in_each_table_that_holds_its_label() {
    let forms = TextItem::Variants(Variants {
        gender: 0,
        plurality: 0,
        hash_27d7357f: 0,
        hash_e0237fcf: 0,
        forms: vec![Variant {
            text: "eine Form".into(),
            gender: 0,
            plurality: 0,
            case: 0,
            hash_34a643c5: [[0; 2]; 9],
        }],
    });
    let text = |t: &str| TextItem::Text(t.to_string());
    let english = table(&[
        ("MIS1_INT_JD_1", text("~z~Hello,~n~you.")),
        ("UNRELATED", text("x")),
    ]);
    let french = table(&[("mis1_int_jd_1", text("Bonjour."))]);
    let german = table(&[("MIS1_INT_JD_1", forms)]);
    let other = table(&[("UNRELATED", text("x"))]);
    let spoken = line(1, "MIS1_INT_JD_1");

    assert_eq!(spoken.subtitle(&english), Some("~z~Hello,~n~you."));
    assert_eq!(spoken.subtitle(&french), Some("Bonjour."));
    assert_eq!(spoken.subtitle(&german), Some("eine Form"));
    assert_eq!(spoken.subtitle(&other), None);

    let wanted = BTreeSet::from([spoken.label_hash()]);
    let found: Vec<_> = subtitles_in(&english, &wanted).collect();
    assert_eq!(found, [(spoken.label_hash(), "~z~Hello,~n~you.")]);
    assert_eq!(subtitles_in(&other, &wanted).count(), 0);
}

#[test]
fn a_line_s_comments_name_it_and_carry_its_subtitle_without_markup() {
    let comment = |f: &str, v: &str| (f.to_string(), v.to_string());
    let spoken = line(4, "MIS1_INT_JD_1");
    assert_eq!(
        spoken.comments(Some("~z~Hello,~n~you.")),
        [
            comment("TITLE", "MIS1_INTAU_AB12_04"),
            comment("LABEL", "MIS1_INT_JD_1"),
            comment("SPEAKER", "JD"),
            comment("SUBTITLE", "Hello, you."),
        ]
    );
    let unnamed = line(1, "MIS1_INT_90s_1");
    assert_eq!(
        unnamed.comments(None),
        [
            comment("TITLE", "MIS1_INTAU_AB12_01"),
            comment("LABEL", "MIS1_INT_90s_1"),
        ]
    );
    assert_eq!(plain("a ~sl:0.05:0.9~b~n~c"), "a b c");
    assert_eq!(plain("a tilde ~ alone"), "a tilde ~ alone");
    assert_eq!(plain(""), "");
}

#[test]
fn a_key_that_keeps_one_take_is_numbered_by_the_stream_that_exists() {
    const OTHER: &str = "OTHR_AAAAU_0001";
    let mut dialogue = Dialogue::default();
    dialogue.add(&body(&[
        (KEY, "MIS1_INT_EXT_15"),
        (OTHER, "OTHR_A_1"),
        (OTHER, "OTHR_B_2"),
    ]));
    let id = |name: &str| name_hash32(name.as_bytes()) & 0x1FFF_FFFF;
    // The first key's one stream is its sixteenth take; the other has one stream for two labels.
    let held = [id("MIS1_INTAU_AB12_16"), id("OTHR_AAAAU_0001_01")];
    assert_eq!(dialogue.settle(|stream| held.contains(&stream)), 1);
    let lines: Vec<DialogueLine> = dialogue.lines().collect();
    assert_eq!(lines[0], line(16, "MIS1_INT_EXT_15"));
    assert_eq!(lines[0].stream_id(), held[0]);
    assert_eq!((lines[1].number, lines[2].number), (1, 2));
    assert_eq!(dialogue.settle(|stream| held.contains(&stream)), 0);
}
