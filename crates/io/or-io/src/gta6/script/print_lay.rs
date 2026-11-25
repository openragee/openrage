//! Breaking a statement that passes the width into lines.
//!
//! ⚠ Only a space becomes a line end, never one inside a string or a comment, so the text reads
//! the same broken or not.

/// The widest line written, counted in characters with its indentation.
pub(super) const WIDTH: usize = 120;

/// The cuts nested in one another that one statement is given; past them a line stays long.
const DEPTH: usize = 16;

/// A place a line may break, by how loosely the two sides of it bind: the loosest is cut first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    /// After the comma of a list.
    List,
    /// Before the `?` and before the `:` of a choice.
    Choice,
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    /// Before a `+` or a `-` between two operands.
    Sum,
}

impl Place {
    /// Whether every place of the kind breaks once one does; the others break only where a line is full.
    fn all(self) -> bool {
        matches!(self, Place::Choice | Place::Or | Place::And)
    }
}

/// The operators a line may break before, each as it stands after the space in front of it.
const OPERATORS: [(&str, Place); 9] = [
    ("|| ", Place::Or),
    ("&& ", Place::And),
    ("? ", Place::Choice),
    (": ", Place::Choice),
    ("| ", Place::BitOr),
    ("^ ", Place::BitXor),
    ("& ", Place::BitAnd),
    ("+ ", Place::Sum),
    ("- ", Place::Sum),
];

/// `text` as the lines of a statement `indent` levels in: one line where it fits, else cut at its loosest places.
// The first line's indentation is the caller's; each line started here stands one level further in.
pub(super) fn lay(text: &str, indent: usize) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    put(&mut out, text, indent, 4 * indent, 0);
    out
}

/// Writes `text` from column `col` on and returns the column it ends at.
fn put(out: &mut String, text: &str, indent: usize, col: usize, nested: usize) -> usize {
    let len = text.chars().count();
    let pieces = match col + len > WIDTH && nested < DEPTH {
        true => cut(text),
        false => None,
    };
    let Some((kind, parts)) = pieces else {
        out.push_str(text);
        return col + len;
    };
    let mut col = col;
    for (n, part) in parts.into_iter().enumerate() {
        let len = part.chars().count();
        if n == 0 {
            col = put(out, part, indent, col, nested + 1);
        } else if !kind.all() && col + 1 + len <= WIDTH {
            out.push(' ');
            out.push_str(part);
            col += 1 + len;
        } else {
            out.push('\n');
            for _ in 0..=indent {
                out.push_str("    ");
            }
            col = put(out, part, indent + 1, 4 * (indent + 1), nested + 1);
        }
    }
    col
}

/// `text` cut at the loosest places of the shallowest brackets that hold a place, and the kind of those places.
// A part keeps the comma it ends with and the operator it starts with; the space between two parts is dropped.
fn cut(text: &str) -> Option<(Place, Vec<&str>)> {
    let bytes = text.as_bytes();
    // Where the part before a place ends, where the part after it starts, how deep in brackets it is, and its kind.
    let mut places: Vec<(usize, usize, usize, Place)> = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => break,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if bytes.get(i + 1) == Some(&b' ') => {
                places.push((i + 1, i + 2, depth, Place::List));
            }
            b' ' => {
                let rest = &text[i + 1..];
                let found = OPERATORS.iter().find(|(op, _)| rest.starts_with(*op));
                if let Some((_, kind)) = found {
                    places.push((i, i + 1, depth, *kind));
                }
            }
            _ => {}
        }
        i += 1;
    }
    let shallowest = places.iter().map(|place| place.2).min()?;
    let here = places.iter().filter(|place| place.2 == shallowest);
    let kind = here.map(|place| place.3).min()?;
    let mut parts = Vec::new();
    let mut from = 0;
    for (end, next, depth, of) in places {
        if depth == shallowest && of == kind {
            parts.push(&text[from..end]);
            from = next;
        }
    }
    parts.push(&text[from..]);
    Some((kind, parts))
}

#[cfg(test)]
#[path = "print_lay_tests.rs"]
mod tests;
