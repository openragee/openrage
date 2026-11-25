//! The GTA VI bodies a scan names by their head, opened in the view the tree has for them or as
//! the facts their reader reads.

use std::collections::BTreeSet;
use std::path::Path;

use or_io::gta6::move_network::{
    condition_name, kind_name, Binding, MachineEntry, MoveBody, MoveCondition, MoveOutline,
    MoveTransition, StopReason,
};
use or_io::gta6::other_bodies::{EdupHead, Gta6HeightGrid, Gta6HeightMap, OtherBody, SpanGrid};
use or_io::mrf::{Operation, Operator};

use super::document::DocPayload;
use super::rpf9_scan::View;

#[path = "gta6_move_nodes.rs"]
pub(crate) mod nodes;

/// What the scan's list calls the body.
pub(crate) fn label(body: OtherBody) -> &'static str {
    match body {
        OtherBody::HeightMap => tr!("GTA VI height map"),
        OtherBody::HeightGrid => tr!("GTA VI height grid, one array"),
        OtherBody::SpanGrid => tr!("GTA VI span grid"),
        OtherBody::MoveNetwork => tr!("GTA VI MoVE network"),
        OtherBody::Gfx => tr!("Scaleform movie"),
        OtherBody::Edup => tr!("EDUP, high entropy"),
        OtherBody::ObjectTrees => tr!("GTA VI object trees"),
    }
}

/// The view the body opens in; an `EDUP` body's payload is refused there, under its head.
pub(super) fn view(body: OtherBody) -> Result<View, String> {
    Ok(View::Identified(body))
}

/// An `EDUP` body's head as read, then why its payload is not.
fn edup_lines(head: &EdupHead) -> Vec<String> {
    let mut lines = vec![tr!(
        "An EDUP body: its count word at 0x04 is {count}, and {length} payload bytes follow its 8-byte head.",
        count = head.count,
        length = head.payload_len,
    )];
    if let Some(stated) = head.stated_len {
        lines.push(tr!(
            "With a count of zero, the word at 0x08 states {stated} bytes, and that many follow it.",
            stated = stated,
        ));
    }
    lines.push(tr!("Its payload is not read: no record width divides it by its count, and its bytes are of high entropy with no cipher record, so what it holds is not known and no view shows it.").to_string());
    lines
}

/// The extension of the path the body opens under.
pub(super) fn ext(body: OtherBody) -> &'static str {
    match body {
        OtherBody::HeightMap | OtherBody::HeightGrid => "hmap",
        OtherBody::Gfx => "gfx",
        _ => "txt",
    }
}

/// The body's payload: GTA V's height map and Scaleform views, the lines of its facts, or an `EDUP` head's lines as the refusal its bytes show under.
pub(super) fn payload(path: &Path, body: OtherBody, bytes: &[u8]) -> Result<DocPayload, String> {
    let refused = |e: or_io::IoError| {
        tr!(
            "The reader of this {what} does not read it: {e}",
            what = label(body),
            e = e
        )
    };
    let facts = match body {
        OtherBody::HeightMap => {
            return Gta6HeightMap::open(bytes)
                .map(|map| DocPayload::HeightMap(map.grid))
                .map_err(refused)
        }
        OtherBody::Gfx => {
            return or_io::gfx::GfxMovie::open(bytes)
                .map(DocPayload::Gfx)
                .map_err(refused)
        }
        OtherBody::Edup => {
            let head = EdupHead::open(bytes).map_err(refused)?;
            return Err(edup_lines(&head).join("\n"));
        }
        OtherBody::HeightGrid => {
            let grid = Gta6HeightGrid::open(bytes).map_err(refused)?;
            return grid
                .height_map()
                .map(DocPayload::HeightMap)
                .map_err(refused);
        }
        OtherBody::SpanGrid => span_lines(&SpanGrid::open(bytes).map_err(refused)?),
        OtherBody::MoveNetwork => {
            let outline = MoveOutline::open(bytes).map_err(refused)?;
            let lines = move_lines(&outline);
            nodes::keep(path, outline);
            lines
        }
        OtherBody::ObjectTrees => super::gta6_object_trees::lines(bytes)?,
    };
    Ok(super::document_io::text_payload(
        path,
        facts.join("\n").into_bytes(),
    ))
}

fn span_lines(grid: &SpanGrid) -> Vec<String> {
    let word = if grid.word() == 0 {
        tr!("Its kind is 0 and both its words are zero: the empty form, which covers no cell.")
            .to_string()
    } else {
        tr!(
            "Its kind is {kind} and its word at {at} is {word}: its low half is its high half's low byte with bit 7 set, as in every grid. The two planes hold data over land and do not follow its tile's heights; what they and the word's high half hold is not known.",
            kind = grid.kind,
            at = if grid.kind >= 2 { "0x14" } else { "0x18" },
            word = format!("{:#010X}", grid.word()),
        )
    };
    vec![
        tr!(
            "A GTA VI span grid of 128 by 128 cells: rows {first} to {end} hold two spans each, which cover {cells} cells of two byte planes.",
            first = grid.first_row,
            end = usize::from(grid.first_row) + grid.spans.len(),
            cells = grid.planes[0].len(),
        ),
        word,
        tr!("It lies on the height map tile of the same entry number in the nested archive beside its own; with no archive table, the scan's order of the two archives' bodies is their entry order.").to_string(),
    ]
}

/// The network's head, how far its walk read, then its nodes as an outline from the root.
fn move_lines(outline: &MoveOutline) -> Vec<String> {
    let head = &outline.head;
    let walk = if outline.stops.is_empty() {
        tr!(
            "Its {nodes} nodes tile the body from the root to its end, with {tails} tails no offset names between them. A kind's name is GTA V's at the same place in GTA V's list, which is not confirmed.",
            nodes = outline.nodes.len(),
            tails = outline.loose_tails,
        )
    } else {
        tr!(
            "The walk read {nodes} nodes and reached {stops} whose layout is not known, which it does not read past: the rest of the body is kept as stored. A kind's name is GTA V's at the same place in GTA V's list, which is not confirmed.",
            nodes = outline.nodes.len(),
            stops = outline.stops.len(),
        )
    };
    let mut lines = vec![
        tr!(
            "A GTA VI MoVE network at version 0x13 whose name hash is {hash}: lists of {a}, {b} and {d} entries, then a root node of type {kind}, index {index} and id {id}.",
            hash = super::gta6_names::said32(head.name_hash),
            a = head.list_a.len(),
            b = head.list_b.len(),
            d = head.list_d.len(),
            kind = head.root_type,
            index = head.root_index,
            id = format!("{:#010X}", head.root_id),
        ),
        walk,
        String::new(),
    ];
    if let Some(name) = text_name(head.name_hash) {
        lines.insert(1, tr!("Network name: {name}", name = name));
    }
    outline_lines(outline, &mut lines);
    lines
}

/// Machine to states, state to output and node to inputs, depth first; a node met again is named.
fn outline_lines(outline: &MoveOutline, lines: &mut Vec<String>) {
    let mut shown = BTreeSet::new();
    let mut stack = vec![(outline.root, 0usize)];
    while let Some((at, depth)) = stack.pop() {
        let indent = "  ".repeat(depth);
        let inner = "  ".repeat(depth + 1);
        let node = node_label(outline, at);
        let Some(read) = outline.node(at) else {
            let line = match outline.stop(at) {
                Some(stop) => tr!(
                    "{node} at {at}, child {index}, word {word}: not read, as {why}",
                    node = node,
                    at = format!("{at:#X}"),
                    index = stop.index,
                    word = format!("{:#010X}", stop.word),
                    why = why(stop.reason),
                ),
                None => tr!("{node}: not read", node = node),
            };
            lines.push(format!("{indent}{line}"));
            continue;
        };
        if !shown.insert(at) {
            lines.push(format!(
                "{indent}{}",
                tr!("{node}, shown above", node = node)
            ));
            continue;
        }
        let mut next = Vec::new();
        match &read.body {
            MoveBody::Machine(m) => {
                let line = tr!(
                    "{node}: states {states}, transitions {transitions}, entries {entries}, inputs {inputs}",
                    node = node,
                    states = m.states.len(),
                    transitions = m.transitions.len(),
                    entries = m.entries.len(),
                    inputs = m.inputs.len(),
                );
                lines.push(format!("{indent}{line}"));
                transition_lines(outline, &m.transitions, &inner, lines);
                for entry in &m.entries {
                    lines.push(format!("{inner}{}", entry_line(outline, entry)));
                    for c in &entry.conditions {
                        lines.push(format!("{inner}  {}", condition_line(c)));
                    }
                }
                next.extend(&m.inputs);
                next.extend(m.states.iter().map(|s| s.1));
            }
            MoveBody::State(s) => {
                let line = tr!(
                    "{node}, child {index}: children {children}, transitions {transitions}, bindings {bindings}",
                    node = node,
                    index = read.index,
                    children = s.children,
                    transitions = s.transitions.len(),
                    bindings = s.bindings.len(),
                );
                lines.push(format!("{indent}{line}"));
                transition_lines(outline, &s.transitions, &inner, lines);
                for binding in &s.bindings {
                    lines.push(format!("{inner}{}", binding_line(binding)));
                }
                next.push(s.output);
            }
            MoveBody::Leaf { inputs, .. } => {
                let line = tr!(
                    "{node}, child {index}: {size} bytes",
                    node = node,
                    index = read.index,
                    size = read.size,
                );
                lines.push(format!("{indent}{line}"));
                next.extend(inputs);
            }
        }
        stack.extend(next.into_iter().rev().map(|at| (at, depth + 1)));
    }
}

fn transition_lines(
    outline: &MoveOutline,
    transitions: &[MoveTransition],
    indent: &str,
    lines: &mut Vec<String>,
) {
    for t in transitions {
        let line = tr!(
            "transition to {target}, {size} bytes",
            target = node_label(outline, t.target),
            size = t.size,
        );
        lines.push(format!("{indent}{line}"));
        lines.push(format!("{indent}  {}", transition_words(t)));
        for c in &t.conditions {
            lines.push(format!("{indent}  {}", condition_line(c)));
        }
    }
}

/// A transition's fixed words as stored, of which only its size, flags and target are read.
fn transition_words(t: &MoveTransition) -> String {
    let words: Vec<String> = t.words.iter().map(|w| format!("{w:08X}")).collect();
    tr!(
        "words from +0x00 to +0x48 as stored: {words}",
        words = words.join(" ")
    )
}

/// A machine entry's state, size and flag.
fn entry_line(outline: &MoveOutline, entry: &MachineEntry) -> String {
    tr!(
        "entry to {state}, {size} bytes, flag {flag}",
        state = node_label(outline, entry.state),
        size = entry.size,
        flag = entry.flag,
    )
}

/// A condition as GTA V's network view writes one, its hashes as `named` names them and a flag
/// half of 1 named.
fn condition_line(c: &MoveCondition) -> String {
    condition_line_with(c, named)
}

fn condition_line_with(c: &MoveCondition, hash: impl Fn(u32) -> String) -> String {
    let float = f32::from_bits;
    let test = match (c.kind, c.words.as_slice()) {
        (0, [s, upper, lower, _]) => tr!(
            "{signal} in [{lower}, {upper}]",
            signal = hash(*s),
            lower = float(*lower),
            upper = float(*upper),
        ),
        (1, [s, upper, lower, _]) => tr!(
            "{signal} outside [{lower}, {upper}]",
            signal = hash(*s),
            lower = float(*lower),
            upper = float(*upper),
        ),
        (5..=8, [s, value]) => {
            let op = match c.kind {
                5 => ">",
                6 => ">=",
                7 => "<",
                _ => "<=",
            };
            format!("{} {op} {}", hash(*s), float(*value))
        }
        (9, [t]) => tr!("time in state > {seconds} s", seconds = float(*t)),
        (10, [t]) => tr!("time in state < {seconds} s", seconds = float(*t)),
        (12, [s, value]) => format!("{} == {}", hash(*s), *value != 0),
        _ => (c.words.iter().map(|w| hash(*w)))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let kind = match condition_name(c.kind) {
        Some(name) => format!("{} ({name})", c.kind),
        None => c.kind.to_string(),
    };
    match c.flag {
        1 => tr!("{kind}: {test}, flag 1", kind = kind, test = test),
        _ => format!("{kind}: {test}"),
    }
}

/// The records a state binds to one child, inputs from groups A and B, and its operations.
fn binding_line(binding: &Binding) -> String {
    let inputs: Vec<[u32; 4]> = (binding.inputs_a.iter())
        .chain(&binding.inputs_b)
        .copied()
        .collect();
    tr!(
        "child {child} binds inputs {inputs}, outputs {outputs} and events {events}, with operations {operations}",
        child = binding.child,
        inputs = hashes(&inputs),
        outputs = hashes(&binding.outputs),
        events = hashes(&binding.events),
        operations = operations_line(&binding.operations),
    )
}

/// Each record's first word as `named` names a hash and its further words as stored, or a dash.
fn hashes<const N: usize>(records: &[[u32; N]]) -> String {
    hashes_with(records, named)
}

fn hashes_with<const N: usize>(records: &[[u32; N]], hash: impl Fn(u32) -> String) -> String {
    if records.is_empty() {
        return "-".to_string();
    }
    (records.iter())
        .map(|r| {
            let rest: Vec<String> = r[1..].iter().map(|w| format!("{w:08X}")).collect();
            format!("{} [{}]", hash(r[0]), rest.join(" "))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Each operation in brackets, its words as stored and a curve as read, or a dash for none.
fn operations_line(operations: &[Operation]) -> String {
    if operations.is_empty() {
        return "-".to_string();
    }
    (operations.iter())
        .map(|op| format!("[{}]", operation_line(op)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An operation's size, its three other halves and its operators; no half's role is known.
fn operation_line(op: &Operation) -> String {
    let operators: Vec<String> = op.operators.iter().map(operator_line).collect();
    tr!(
        "{size} bytes, halves at +0x00, +0x02 and +0x06 {first} {second} {fourth}, operators {operators}",
        size = op.size,
        first = format!("{:04X}", op.node_index),
        second = format!("{:04X}", op.type_index),
        fourth = format!("{:04X}", op.index),
        operators = operators.join(", "),
    )
}

/// An operator's type and packet words, then a curve's range and keys as read.
fn operator_line(o: &Operator) -> String {
    let words = format!("{:08X} {:08X}", o.operator_type, o.packet);
    let Some(curve) = &o.fcurve else {
        return words;
    };
    let keys: Vec<String> = (curve.keyframes.iter())
        .map(|k| {
            let [c0, c1] = k.constants;
            format!("({:08X} {} {c0} {c1})", k.keyframe_type, k.key)
        })
        .collect();
    tr!(
        "{words}, a curve over [{minimum}, {maximum}] with keys {keys}",
        words = words,
        minimum = curve.minimum,
        maximum = curve.maximum,
        keys = keys.join(" "),
    )
}

/// A node's kind and id, for a node read or reached; its offset for any other.
fn node_label(outline: &MoveOutline, at: usize) -> String {
    node_label_with(outline, at, named)
}

fn node_label_with(outline: &MoveOutline, at: usize, hash: impl Fn(u32) -> String) -> String {
    let (kind, id) = match (outline.node(at), outline.stop(at)) {
        (Some(node), _) => (node.kind, node.id),
        (None, Some(stop)) => (stop.kind, stop.id),
        (None, None) => return tr!("the node at {at}", at = format!("{at:#X}")),
    };
    match kind_name(kind) {
        Some(name) => format!("{name} {}", hash(id)),
        None => format!("{} {}", tr!("type {kind}", kind = kind), hash(id)),
    }
}

/// A 32-bit hash by the one name the harvested text holds for it, which may be a collision and
/// is marked so, else as `hash_`.
fn named(hash: u32) -> String {
    text_name(hash).unwrap_or_else(|| format!("hash_{hash:08X}"))
}

/// The name the harvested text holds for `hash`, marked as from the text.
fn text_name(hash: u32) -> Option<String> {
    super::gta6_names::name32(hash).map(|name| from_the_text(&name))
}

fn from_the_text(name: &str) -> String {
    tr!("{name} (from the text)", name = name)
}

fn why(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Keyed => tr!("its word at +0x08 picks a field schema the bytes do not state"),
        StopReason::Unmeasured => tr!("no layout of this form of its type is measured"),
        StopReason::WideCount => {
            tr!("its count word has bytes 9 to 11 set, a form no layout is measured for")
        }
        StopReason::ClipContext => {
            tr!("it is a clip of context 3, a form no body was seen to hold")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edup_body_shows_its_head_over_its_refused_payload_and_the_others_open_in_a_view() {
        assert_eq!(view(OtherBody::Edup), Ok(View::Identified(OtherBody::Edup)));
        let path = Path::new("0x10.txt");
        let mut zero = b"EDUP".to_vec();
        zero.extend_from_slice(&0u32.to_le_bytes());
        zero.extend_from_slice(&16u32.to_le_bytes());
        zero.extend_from_slice(&[0x5A; 16]);
        let Err(banner) = payload(path, OtherBody::Edup, &zero) else {
            panic!("the payload is refused");
        };
        let lines: Vec<&str> = banner.lines().collect();
        assert_eq!(lines.len(), 3, "{banner}");
        assert!(lines[0].contains("is 0, and 20 payload bytes"), "{banner}");
        assert!(lines[1].contains("states 16 bytes"), "{banner}");
        assert!(lines[2].starts_with("Its payload is not read"), "{banner}");
        let mut counted = b"EDUP".to_vec();
        counted.extend_from_slice(&4u32.to_le_bytes());
        counted.extend_from_slice(&[0; 228]);
        let Err(banner) = payload(path, OtherBody::Edup, &counted) else {
            panic!("the payload is refused");
        };
        assert_eq!(banner.lines().count(), 2, "{banner}");
        assert!(banner.contains("is 4, and 228 payload bytes"), "{banner}");
        // A head its reader refuses shows the reader's reason, not the head's lines.
        zero.pop();
        let Err(why) = payload(path, OtherBody::Edup, &zero) else {
            panic!("the head is refused");
        };
        assert!(why.contains("states 16 bytes where 15 follow it"), "{why}");
        assert!(!why.contains("Its payload is not read"), "{why}");
        assert_eq!(
            view(OtherBody::SpanGrid),
            Ok(View::Identified(OtherBody::SpanGrid))
        );
        assert_eq!(ext(OtherBody::HeightMap), "hmap");
        assert_eq!(ext(OtherBody::HeightGrid), "hmap");
    }

    /// A network with every list empty whose root node is of `kind`, with `word` at its +0x08.
    fn network(kind: u16, word: u32) -> Vec<u8> {
        let mut bytes = b"MoVE\x13".to_vec();
        bytes.extend_from_slice(&[0; 15]);
        bytes.extend_from_slice(&0xCAFE_F00Du32.to_le_bytes());
        // The length word, the zero word, then empty lists A and B, the word after B and list D.
        bytes.extend_from_slice(&[0; 4 * 6]);
        bytes.extend_from_slice(&[0; 10]);
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&word.to_le_bytes());
        let length = (bytes.len() - 0x2E) as u32;
        bytes[0x18..0x1C].copy_from_slice(&length.to_le_bytes());
        bytes
    }

    #[test]
    fn a_move_network_opens_as_its_outline() {
        let outline = MoveOutline::open(&network(2, 0)).expect("opens");
        let lines = move_lines(&outline);
        assert!(lines[0].contains("hash_CAFEF00D"), "{}", lines[0]);
        assert!(lines[0].contains("0x00000007"), "{}", lines[0]);
        assert!(lines[1].contains("Its 1 nodes tile"), "{}", lines[1]);
        assert_eq!(lines[3], "Tail hash_00000007, child 0: 12 bytes");
    }

    #[test]
    fn a_condition_reads_as_gta_v_s_network_view_writes_one() {
        let line = |kind: u16, flag: u16, words: &[u32]| {
            let words = words.to_vec();
            condition_line(&MoveCondition { kind, flag, words })
        };
        let range = [7, 2.0f32.to_bits(), 0.5f32.to_bits(), 0x1_0001];
        assert_eq!(line(0, 0, &range), "0 (InRange): hash_00000007 in [0.5, 2]");
        assert_eq!(
            line(6, 0, &[7, 0.25f32.to_bits()]),
            "6 (GreaterThanEqual): hash_00000007 >= 0.25"
        );
        assert_eq!(line(3, 1, &[0xAB]), "3 (OnFlag): hash_000000AB, flag 1");
        assert_eq!(
            line(12, 0, &[7, 1]),
            "12 (BoolEquals): hash_00000007 == true"
        );
        assert_eq!(
            line(14, 0, &[0xAB, 0xCD]),
            "14: hash_000000AB, hash_000000CD"
        );
    }

    #[test]
    fn an_id_and_a_signal_the_text_names_are_shown_by_that_name_marked_as_from_the_text() {
        let text = |w: u32| match w {
            7 => from_the_text("direction"),
            w => format!("hash_{w:08X}"),
        };
        let line = |kind: u16, words: &[u32]| {
            let words = words.to_vec();
            condition_line_with(
                &MoveCondition {
                    kind,
                    flag: 0,
                    words,
                },
                text,
            )
        };
        assert_eq!(
            line(6, &[7, 0.25f32.to_bits()]),
            "6 (GreaterThanEqual): direction (from the text) >= 0.25"
        );
        assert_eq!(
            line(14, &[0xCD, 7]),
            "14: hash_000000CD, direction (from the text)"
        );
        let outline = MoveOutline::open(&network(2, 0)).expect("opens");
        assert_eq!(
            node_label_with(&outline, outline.root, text),
            "Tail direction (from the text)"
        );
        assert_eq!(
            node_label_with(&outline, outline.root, |w| format!("hash_{w:08X}")),
            "Tail hash_00000007"
        );
    }

    #[test]
    fn a_binding_names_its_hashes_and_a_transition_shows_its_stored_words() {
        let text = |w: u32| match w {
            7 => from_the_text("speed"),
            w => format!("hash_{w:08X}"),
        };
        assert_eq!(
            hashes_with(&[[7u32, 1, 2, 3], [9, 0, 0, 0]], text),
            "speed (from the text) [00000001 00000002 00000003], hash_00000009 [00000000 00000000 00000000]"
        );
        assert_eq!(
            hashes_with(&[[7u32, 1]], text),
            "speed (from the text) [00000001]"
        );
        let none: [[u32; 2]; 0] = [];
        assert_eq!(hashes_with(&none, text), "-");

        let mut words = [0u32; 19];
        words[4] = 0x4C;
        words[18] = 0xFFFF_FFB8;
        let transition = MoveTransition {
            at: 0x48,
            size: 0x4C,
            target: 0,
            words,
            conditions: Vec::new(),
        };
        let shown: Vec<String> = words.iter().map(|w| format!("{w:08X}")).collect();
        assert_eq!(
            transition_words(&transition),
            format!("words from +0x00 to +0x48 as stored: {}", shown.join(" "))
        );
        assert!(shown[4] == "0000004C" && shown[18] == "FFFFFFB8");
    }

    #[test]
    fn a_binding_shows_its_further_words_and_its_operations_as_stored() {
        let curve = or_io::mrf::FCurve {
            minimum: -1.0,
            maximum: 1.0,
            keyframes_offset: 4,
            keyframes: vec![or_io::mrf::Keyframe {
                keyframe_type: 2,
                key: 0.5,
                constants: [1.0, 0.25],
            }],
        };
        let operators = vec![
            Operator {
                operator_type: Operator::FCURVE,
                packet: 4,
                fcurve: Some(curve),
            },
            Operator {
                operator_type: Operator::END,
                packet: 0,
                fcurve: None,
            },
        ];
        let operation = Operation {
            node_index: 1,
            type_index: 0x2A,
            size: 0x30,
            index: 3,
            operators,
        };
        let binding = Binding {
            child: 2,
            inputs_a: vec![[7, 1, 0, 0]],
            inputs_b: Vec::new(),
            outputs: Vec::new(),
            events: vec![[7, 1]],
            operations: vec![operation],
        };
        let line = binding_line(&binding);
        assert!(
            line.contains("inputs hash_00000007 [00000001 00000000 00000000], outputs -"),
            "{line}"
        );
        assert!(line.contains("events hash_00000007 [00000001]"), "{line}");
        let op = format!(
            "[{} bytes, halves at +0x00, +0x02 and +0x06 0001 002A 0003, operators \
             00000005 00000004, a curve over [-1, 1] with keys (00000002 0.5 1 0.25), \
             00000000 00000000]",
            0x30
        );
        assert!(line.ends_with(&format!("with operations {op}")), "{line}");
        let none = Binding {
            operations: Vec::new(),
            ..binding
        };
        assert!(binding_line(&none).ends_with("with operations -"));
    }

    #[test]
    fn a_keyed_root_is_named_and_not_read() {
        let outline = MoveOutline::open(&network(28, 0x0A)).expect("opens");
        let lines = move_lines(&outline);
        assert!(lines[1].contains("reached 1"), "{}", lines[1]);
        assert!(
            lines[3]
                .starts_with("type 28 hash_00000007 at 0x3A, child 0, word 0x0000000A: not read"),
            "{}",
            lines[3]
        );
    }

    #[test]
    fn a_height_grid_opens_in_the_height_map_view_with_byte_0x28_where_no_span_is() {
        let mut bytes = b"MHIU\x01\x00".to_vec();
        for v in [2u16, 1, 0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0.0f32, 0.0, -50.0, 512.0, 512.0, 500.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&(8u32 + 1).to_le_bytes());
        bytes.extend_from_slice(&[4, 0x11, 0x22, 0x33]);
        // One row: a cell at column 1, packed first.
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes.push(9);
        let read = payload(Path::new("0x10.hmap"), OtherBody::HeightGrid, &bytes);
        let Ok(DocPayload::HeightMap(map)) = read else {
            panic!("a height map");
        };
        assert_eq!((map.num_cols, map.num_rows), (2, 1));
        assert_eq!((map.max, map.min), (vec![4, 9], vec![4, 9]));
    }
}
