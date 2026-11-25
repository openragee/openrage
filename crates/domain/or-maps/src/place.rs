//! What a person pastes into a search field, read as a place, a hash or text to search names with, and a hash spelled each way a
//! script and a tool write it.
//!
//! A place is read from the forms scripts and tools write: a call of two to four numbers (`vector3(…)`, `vec4(…)`,
//! `new Vector3(…)`), a table that names `x`, `y` and `z` as Lua, JSON or C# write one, with its braces or without, and bare
//! numbers, with or without brackets around them. A call or brackets are read behind what a line of a script assigns them to
//! (`coords = vector3(…)`, `"position": {…}`). Two numbers are `x` and `y` with no height; a fourth is the heading in degrees,
//! which a table names `heading`, `h` or `w`, and a line may name behind a call of three (`vector3(…), heading = 90.0`). A hash is read from hexadecimal behind `0x`, from an unsigned decimal and from the
//! signed decimal a script prints for the same 32 bits, and from `hash_` with its eight digits, as the tree shows a hash with no
//! name.
//!
//! ⚠ **One number is never a place.** A whole number is a hash, so a year is one, and any other lone number is text, as is every
//! name: a name is not a list of numbers, whatever digits it holds.
//!
//! ⚠ **A comma is a decimal comma only between semicolons.** Bare numbers a semicolon separates are a French list, where each
//! number may write its decimals after a comma (`12,5; -3,25; 7`). Everywhere else a comma separates, so `12,5 3,25` is four
//! numbers and `1,5` is two.

/// A position a person pasted, with the heading where one was given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PastedPlace {
    pub x: f32,
    pub y: f32,
    /// None where two numbers were given: the place has no height.
    pub z: Option<f32>,
    /// In degrees, as it was written.
    pub heading: Option<f32>,
}

/// What a pasted text is.
#[derive(Debug, Clone, PartialEq)]
pub enum Pasted {
    Place(PastedPlace),
    Hash(u32),
    /// The text itself, trimmed: what names are searched with.
    Text(String),
}

/// Why a pasted text that has the shape of a place or a hash is not one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("`{0}` is past what a position holds: a map stores 32-bit numbers, and one that is not finite cannot be stored")]
    NotFinite(String),
    #[error("`{0}` does not fit a hash, which is 32 bits: a whole number from {min} to {max}", min = i32::MIN, max = u32::MAX)]
    HashTooWide(String),
}

/// One hash in each spelling a person meets, with its name where one is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashSpellings {
    /// `0x` and eight digits.
    pub hexadecimal: String,
    pub unsigned: String,
    /// The same 32 bits as a script prints them.
    pub signed: String,
    pub name: Option<String>,
}

/// Spell `hash` in hexadecimal, unsigned and signed, with the name `name_of` knows it by.
pub fn spell_hash(hash: u32, name_of: impl FnOnce(u32) -> Option<String>) -> HashSpellings {
    HashSpellings {
        hexadecimal: format!("0x{hash:08X}"),
        unsigned: hash.to_string(),
        signed: (hash as i32).to_string(),
        name: name_of(hash),
    }
}

/// Read what a person pasted: a place, a hash, or the text to search names with.
pub fn read_pasted(text: &str) -> Result<Pasted, Refused> {
    let text = text.trim();
    if let Some(hash) = hash_of(text) {
        return hash.map(Pasted::Hash);
    }
    match place_of(text) {
        Some(place) => place.map(Pasted::Place),
        None => Ok(Pasted::Text(text.to_string())),
    }
}

/// The spelling the tree shows a hash with no name as, before its eight digits.
const UNNAMED: &str = "hash_";

/// The hash `text` spells, none where it is no whole number and no hexadecimal behind its prefix.
fn hash_of(text: &str) -> Option<Result<u32, Refused>> {
    let too_wide = || Refused::HashTooWide(text.to_string());
    // ⚠ A name may begin as the tree's own spelling does (`hash_face`), so that spelling is a hash only with all eight digits.
    let unnamed = || {
        text.strip_prefix(UNNAMED)
            .filter(|digits| digits.len() == 8)
    };
    let hexadecimal = ["0x", "0X"]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .or_else(unnamed);
    if let Some(digits) = hexadecimal {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return Some(u32::from_str_radix(digits, 16).map_err(|_| too_wide()));
    }
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A signed spelling is the same bits as the unsigned one, so both fit one range.
    let range = i64::from(i32::MIN)..=i64::from(u32::MAX);
    Some(match text.parse::<i64>() {
        Ok(number) if range.contains(&number) => Ok(number as u32),
        _ => Err(too_wide()),
    })
}

/// The place `text` spells, none where it has not the shape of one.
fn place_of(text: &str) -> Option<Result<PastedPlace, Refused>> {
    // A comment a script ends the line with, then the comma or semicolon that ended the statement.
    let text = ["--", "//"]
        .iter()
        .filter_map(|mark| text.find(mark))
        .min()
        .map_or(text, |at| &text[..at])
        .trim_end();
    let text = text.strip_suffix([',', ';']).map_or(text, str::trim_end);
    let text = assigned(text);
    // A heading named behind a call of three numbers: `vector3(…), heading = 90.0`.
    let (text, heading) = match text.rsplit_once(',') {
        Some((call, named)) => match (brackets(call.trim_end()), heading_of(named)) {
            (Some((Brackets::Call, _)), Some(heading)) => (call.trim_end(), Some(heading)),
            _ => (text, None),
        },
        None => (text, None),
    };
    let mut numbers = match brackets(text) {
        Some((Brackets::Call | Brackets::Plain, inside)) => list(inside, true)?,
        Some((Brackets::Braces, inside)) if inside.contains(['=', ':']) => {
            return table(inside);
        }
        // The list semicolons separate, which is read with no braces too.
        Some((Brackets::Braces, inside)) if inside.contains(';') => {
            let inside = inside.trim();
            french_list(inside.strip_suffix(';').unwrap_or(inside).trim_end())?
        }
        Some((Brackets::Braces, inside)) => list(inside, true)?,
        None if text.contains(['=', ':']) => return table(text),
        None if text.contains(';') => french_list(text)?,
        None => list(text, false)?,
    };
    if let Some(heading) = heading {
        if numbers.len() != 3 {
            return None;
        }
        numbers.push(heading);
    }
    // The count is judged before the numbers: what has not the shape of a place is text, whatever its numbers hold.
    if !(2..=4).contains(&numbers.len()) {
        return None;
    }
    let place = numbers
        .into_iter()
        .collect::<Result<Vec<f32>, Refused>>()
        .map(|numbers| PastedPlace {
            x: numbers[0],
            y: numbers[1],
            z: numbers.get(2).copied(),
            heading: numbers.get(3).copied(),
        });
    Some(place)
}

/// `text` without what a line assigns it to: of `coords = vector3(…)` and `"position": {…}`, the call and the table.
fn assigned(text: &str) -> &str {
    let Some((_, value)) = text.split_once(['=', ':']) else {
        return text;
    };
    let value = value.trim_start();
    // ⚠ A call or a bracket is assigned; a number is the first value of a table with no braces (`x = 1, y = 2, z = 3`).
    let opens = |c: char| c.is_ascii_alphabetic() || matches!(c, '(' | '[' | '{');
    if value.starts_with(opens) {
        value
    } else {
        text
    }
}

/// What stands around the numbers of a place.
enum Brackets {
    /// A name and round brackets, with `new` before it or not.
    Call,
    /// Round or square brackets alone.
    Plain,
    Braces,
}

/// The brackets that hold all of `text`, with what is inside them.
fn brackets(text: &str) -> Option<(Brackets, &str)> {
    let (kind, open, close) = if text.ends_with(')') {
        let open = text.find('(')?;
        let name = text[..open].trim_end();
        let name = name
            .strip_prefix("new")
            .filter(|rest| rest.starts_with(char::is_whitespace))
            .map_or(name, str::trim_start);
        let kind = if name.is_empty() {
            Brackets::Plain
        } else if is_name(name) {
            Brackets::Call
        } else {
            return None;
        };
        (kind, open, ')')
    } else if text.starts_with('[') {
        (Brackets::Plain, 0, ']')
    } else if text.starts_with('{') {
        (Brackets::Braces, 0, '}')
    } else {
        return None;
    };
    let inside = text[open + 1..].strip_suffix(close)?;
    Some((kind, inside))
}

/// Whether `name` is what a call is made on: `vector3`, `Vector3`, `mp.Vector3`, `vector3.new`.
fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':'))
}

/// The numbers of `text`, which commas and spaces separate; none where anything in it is no number.
fn list(text: &str, trailing_comma: bool) -> Option<Vec<Result<f32, Refused>>> {
    let text = text.trim();
    let text = match text.strip_suffix(',') {
        Some(rest) if trailing_comma => rest,
        _ => text,
    };
    let mut numbers = Vec::new();
    for between_commas in text.split(',') {
        let before = numbers.len();
        for word in between_commas.split_whitespace() {
            numbers.push(number(word)?);
        }
        if numbers.len() == before {
            return None;
        }
    }
    Some(numbers)
}

/// The numbers of `text`, which semicolons separate, each with a comma or a point before its decimals.
fn french_list(text: &str) -> Option<Vec<Result<f32, Refused>>> {
    text.split(';')
        .map(|between| {
            let word = between.trim();
            match word.split_once(',') {
                Some((whole, decimals)) => {
                    // ⚠ Only a comma between two digits is a decimal comma: `1, 5` is none, and a number takes one point, so
                    // `1.5,5` and `1,5,5` are no numbers.
                    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
                    let decimal =
                        digit(whole.chars().next_back()) && digit(decimals.chars().next());
                    decimal
                        .then(|| number(&format!("{whole}.{decimals}")))
                        .flatten()
                        .map(|read| read.map_err(|_| Refused::NotFinite(word.to_string())))
                }
                None => number(word),
            }
        })
        .collect()
}

/// The heading `text` names as a table does (`heading = 90.0`), behind a call that gave the position.
fn heading_of(text: &str) -> Option<Result<f32, Refused>> {
    let (key, value) = text.split_once(['=', ':'])?;
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "heading" | "h" | "w"
    )
    .then(|| number(value.trim()))
    .flatten()
}

/// The place a table names: `x`, `y` and `z`, and a heading as `heading`, `h` or `w`, each once and nothing else.
fn table(inside: &str) -> Option<Result<PastedPlace, Refused>> {
    let inside = inside.trim();
    let inside = inside.strip_suffix([',', ';']).unwrap_or(inside);
    let mut named: [Option<Result<f32, Refused>>; 4] = [None, None, None, None];
    for entry in inside.split([',', ';']) {
        let (key, value) = entry.split_once(['=', ':'])?;
        let key = key.trim();
        let key = key
            .strip_prefix('[')
            .and_then(|key| key.strip_suffix(']'))
            .map_or(key, str::trim);
        let key = ['"', '\'']
            .iter()
            .find_map(|quote| key.strip_prefix(*quote)?.strip_suffix(*quote))
            .unwrap_or(key);
        let slot = match key.to_ascii_lowercase().as_str() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            "heading" | "h" | "w" => 3,
            _ => return None,
        };
        if named[slot].is_some() {
            return None;
        }
        named[slot] = Some(number(value.trim())?);
    }
    let [Some(x), Some(y), Some(z), heading] = named else {
        return None;
    };
    let read = || -> Result<PastedPlace, Refused> {
        Ok(PastedPlace {
            x: x?,
            y: y?,
            z: Some(z?),
            heading: heading.transpose()?,
        })
    };
    Some(read())
}

/// The number `word` writes, none where it is none: digits with a sign, a point, an exponent and the `f` of C# or not.
fn number(word: &str) -> Option<Result<f32, Refused>> {
    let written = word.strip_suffix(['f', 'F']).unwrap_or(word);
    // ⚠ `str::parse` also reads `inf`, `infinity` and `nan`, which no script writes as a place, so a letter that is no exponent
    // makes no number.
    let of_a_number = |b: u8| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E');
    if !written.bytes().all(of_a_number) {
        return None;
    }
    let read: f32 = written.parse().ok()?;
    Some(if read.is_finite() {
        Ok(read)
    } else {
        Err(Refused::NotFinite(word.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(x: f32, y: f32, z: f32) -> Pasted {
        Pasted::Place(PastedPlace {
            x,
            y,
            z: Some(z),
            heading: None,
        })
    }

    fn facing(x: f32, y: f32, z: f32, heading: f32) -> Pasted {
        Pasted::Place(PastedPlace {
            x,
            y,
            z: Some(z),
            heading: Some(heading),
        })
    }

    fn flat(x: f32, y: f32) -> Pasted {
        Pasted::Place(PastedPlace {
            x,
            y,
            z: None,
            heading: None,
        })
    }

    /// Every row is read on its own, so one that fails names itself.
    fn each_reads(rows: &[(&str, Pasted)]) {
        let wrong: Vec<String> = rows
            .iter()
            .filter_map(|(pasted, expected)| {
                let read = read_pasted(pasted);
                (read.as_ref() != Ok(expected))
                    .then(|| format!("{pasted:?} read as {read:?}, not {expected:?}"))
            })
            .collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn a_call_of_numbers_is_a_place_under_each_spelling_a_script_writes() {
        each_reads(&[
            ("vector3(125.5, -340.25, 28.0)", place(125.5, -340.25, 28.0)),
            ("vec3(125.5, -340.25, 28.0)", place(125.5, -340.25, 28.0)),
            ("vec(125.5, -340.25, 28.0)", place(125.5, -340.25, 28.0)),
            (
                "vector4(125.5, -340.25, 28.0, 271.5)",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "vec4(125.5,-340.25,28.0,271.5)",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            ("vector2(125.5, -340.25)", flat(125.5, -340.25)),
            (
                "new Vector3(125.5f, -340.25f, 28f)",
                place(125.5, -340.25, 28.0),
            ),
            (
                "new Vector4(125.5F, -340.25F, 28F, 90F);",
                facing(125.5, -340.25, 28.0, 90.0),
            ),
            ("Vector3(125.5, -340.25, 28)", place(125.5, -340.25, 28.0)),
            (
                "new mp.Vector3(125.5, -340.25, 28)",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3.new(125.5, -340.25, 28)",
                place(125.5, -340.25, 28.0),
            ),
        ]);
    }

    #[test]
    fn signs_exponents_spaces_a_trailing_comma_and_a_comment_do_not_change_the_place() {
        each_reads(&[
            ("vector3(-1.5, -2.5, -3.5)", place(-1.5, -2.5, -3.5)),
            ("vector3(+1.5, .5, 3.)", place(1.5, 0.5, 3.0)),
            ("vector3(1.5e3, -2.5E-1, 3e+2)", place(1.5e3, -2.5e-1, 3e2)),
            (
                "  vector3 (  125.5 ,   -340.25 ,28.0  )  ",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3(125.5,\n\t-340.25,\n\t28.0)",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3(125.5, -340.25, 28.0,)",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3(125.5, -340.25, 28.0),",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3(125.5, -340.25, 28.0) -- the yard gate",
                place(125.5, -340.25, 28.0),
            ),
            (
                "vector3(125.5, -340.25, 28.0), -- the yard gate",
                place(125.5, -340.25, 28.0),
            ),
            (
                "new Vector3(125.5f, -340.25f, 28f); // the yard gate",
                place(125.5, -340.25, 28.0),
            ),
            (
                "125.5, -340.25, 28.0 -- the yard gate",
                place(125.5, -340.25, 28.0),
            ),
        ]);
    }

    #[test]
    fn a_table_that_names_x_y_and_z_is_a_place_as_lua_json_and_c_sharp_write_one() {
        each_reads(&[
            (
                "{x = 125.5, y = -340.25, z = 28.0}",
                place(125.5, -340.25, 28.0),
            ),
            (
                "{ z = 28.0, x = 125.5, y = -340.25 }",
                place(125.5, -340.25, 28.0),
            ),
            ("{x=125.5; y=-340.25; z=28.0;}", place(125.5, -340.25, 28.0)),
            (
                "{ ['x'] = 125.5, [\"y\"] = -340.25, z = 28.0, }",
                place(125.5, -340.25, 28.0),
            ),
            (
                "{ x = 125.5, y = -340.25, z = 28.0, h = 271.5 }",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "{ x = 125.5, y = -340.25, z = 28.0, w = 271.5 }",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                r#"{"x": 125.5, "y": -340.25, "z": 28.0}"#,
                place(125.5, -340.25, 28.0),
            ),
            (
                r#"{"x":125.5,"y":-340.25,"z":28.0,"heading":271.5}"#,
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "{\n  \"X\": 125.5,\n  \"Y\": -340.25,\n  \"Z\": 28.0,\n  \"Heading\": 271.5\n}",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "{ X = 125.5f, Y = -340.25f, Z = 28f }",
                place(125.5, -340.25, 28.0),
            ),
            (
                "{ x: 125.5, y: -340.25, z: 28 }",
                place(125.5, -340.25, 28.0),
            ),
            // The inside of a table, as a person selects it without its braces.
            (
                "x = 125.5, y = -340.25, z = 28.0",
                place(125.5, -340.25, 28.0),
            ),
            (
                "x = 125.5; y = -340.25; z = 28.0;",
                place(125.5, -340.25, 28.0),
            ),
            ("x = -1.5, y = 2.5, z = 3.5", place(-1.5, 2.5, 3.5)),
            ("x = +1.5, y = 2.5, z = 3.5", place(1.5, 2.5, 3.5)),
            ("x = .5, y = 2.5, z = 3.5", place(0.5, 2.5, 3.5)),
            (
                "X: 125.5, Y: -340.25, Z: 28.0, H: 271.5",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                r#""x": 125.5, "y": -340.25, "z": 28.0, -- the yard gate"#,
                place(125.5, -340.25, 28.0),
            ),
        ]);
    }

    #[test]
    fn a_place_is_read_behind_what_a_line_of_a_script_assigns_it_to() {
        each_reads(&[
            (
                "coords = vector3(125.5, -340.25, 28.0)",
                place(125.5, -340.25, 28.0),
            ),
            (
                "local gate = vector3(125.5, -340.25, 28.0) -- the yard gate",
                place(125.5, -340.25, 28.0),
            ),
            (
                "['coords'] = vector4(125.5, -340.25, 28.0, 271.5),",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "Vector3 gate = new Vector3(125.5f, -340.25f, 28f); // the yard gate",
                place(125.5, -340.25, 28.0),
            ),
            (
                "const gate = new mp.Vector3(125.5, -340.25, 28);",
                place(125.5, -340.25, 28.0),
            ),
            (
                r#""position": {"x": 125.5, "y": -340.25, "z": 28.0},"#,
                place(125.5, -340.25, 28.0),
            ),
            (
                "coords = { x = 125.5, y = -340.25, z = 28.0, h = 271.5 }",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "coords = {125.5, -340.25, 28.0}",
                place(125.5, -340.25, 28.0),
            ),
            ("position: [125.5, -340.25]", flat(125.5, -340.25)),
            ("at = (125.5, -340.25, 28.0)", place(125.5, -340.25, 28.0)),
            // What is assigned is read as strictly as it is alone, and a name is assigned to nothing.
            (
                "gate = prop_bench_01a",
                Pasted::Text("gate = prop_bench_01a".to_string()),
            ),
            (
                "gate = bench, 2, 3",
                Pasted::Text("gate = bench, 2, 3".to_string()),
            ),
            (
                "gate = bench one(1, 2, 3)",
                Pasted::Text("gate = bench one(1, 2, 3)".to_string()),
            ),
            (
                "gate = vector3(1, 2, 3) here",
                Pasted::Text("gate = vector3(1, 2, 3) here".to_string()),
            ),
            ("gate = 12.5", Pasted::Text("gate = 12.5".to_string())),
            ("gate = 2013", Pasted::Text("gate = 2013".to_string())),
            ("gate =", Pasted::Text("gate =".to_string())),
        ]);
    }

    #[test]
    fn bare_numbers_are_a_place_between_commas_semicolons_or_spaces() {
        each_reads(&[
            ("125.5, -340.25, 28.0", place(125.5, -340.25, 28.0)),
            ("125.5,-340.25,28.0", place(125.5, -340.25, 28.0)),
            ("125.5 -340.25 28.0", place(125.5, -340.25, 28.0)),
            ("125.5\t-340.25\t28.0", place(125.5, -340.25, 28.0)),
            ("125.5; -340.25; 28.0", place(125.5, -340.25, 28.0)),
            ("125.5, -340.25, 28.0,", place(125.5, -340.25, 28.0)),
            (
                "125.5, -340.25, 28.0, 271.5",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            (
                "125.5 -340.25 28.0 271.5",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            ("125.5, -340.25", flat(125.5, -340.25)),
            ("125 -340", flat(125.0, -340.0)),
            ("(125.5, -340.25, 28.0)", place(125.5, -340.25, 28.0)),
            ("[125.5, -340.25, 28.0]", place(125.5, -340.25, 28.0)),
            ("{125.5, -340.25, 28.0}", place(125.5, -340.25, 28.0)),
            ("[125.5, -340.25]", flat(125.5, -340.25)),
        ]);
    }

    #[test]
    fn a_comma_is_a_decimal_comma_between_semicolons_and_a_separator_everywhere_else() {
        each_reads(&[
            // A French list: the semicolon separates, so the comma cannot.
            ("125,5; -340,25; 28", place(125.5, -340.25, 28.0)),
            (
                "125,5;-340,25;28,0;271,5",
                facing(125.5, -340.25, 28.0, 271.5),
            ),
            ("125,5; -340,25", flat(125.5, -340.25)),
            ("1,5e2; 2; 3", place(1.5e2, 2.0, 3.0)),
            // No semicolon: the comma separates, whatever stands around it.
            ("12,5 3,25", facing(12.0, 5.0, 3.0, 25.0)),
            ("1,5", flat(1.0, 5.0)),
            ("vector3(1,5, 2,5)", facing(1.0, 5.0, 2.0, 5.0)),
            // Six numbers are no place, and a comma is no decimal comma to make three of them.
            ("12,5 3,25 7,5", Pasted::Text("12,5 3,25 7,5".to_string())),
            (
                "12,5, 3,25, 7,5",
                Pasted::Text("12,5, 3,25, 7,5".to_string()),
            ),
            // Between semicolons a comma stands between two digits of one number and nowhere else.
            ("1, 5; 2; 3", Pasted::Text("1, 5; 2; 3".to_string())),
            ("1,5,5; 2; 3", Pasted::Text("1,5,5; 2; 3".to_string())),
            ("1.5,5; 2; 3", Pasted::Text("1.5,5; 2; 3".to_string())),
            (",5; 2; 3", Pasted::Text(",5; 2; 3".to_string())),
            ("-,5; 2; 3", Pasted::Text("-,5; 2; 3".to_string())),
            ("1,; 2; 3", Pasted::Text("1,; 2; 3".to_string())),
            ("1,e5; 2; 3", Pasted::Text("1,e5; 2; 3".to_string())),
            // The names of a table do not make a comma a decimal comma.
            (
                "x = 12,5; y = 3,25; z = 7",
                Pasted::Text("x = 12,5; y = 3,25; z = 7".to_string()),
            ),
        ]);
    }

    #[test]
    fn braces_around_a_list_of_semicolons_and_a_heading_named_behind_a_call_are_read() {
        each_reads(&[
            ("{1.0; 2.0; 3.0}", place(1.0, 2.0, 3.0)),
            ("{12,5; -3,25; 7;}", place(12.5, -3.25, 7.0)),
            (
                "vector3(-1234.56, 789.01, 30.5), heading = 90.0",
                facing(-1234.56, 789.01, 30.5, 90.0),
            ),
            (
                "coords = vector3(-1234.56, 789.01, 30.5), h = 90.0,",
                facing(-1234.56, 789.01, 30.5, 90.0),
            ),
        ]);
        // A name that is no heading, and a call that states a heading of its own, stay text.
        for text in [
            "vector3(1, 2, 3), speed = 4",
            "vector4(1, 2, 3, 4), heading = 5",
            "(1, 2, 3), heading = 5",
        ] {
            assert_eq!(read_pasted(text), Ok(Pasted::Text(text.to_string())));
        }
    }

    #[test]
    fn a_name_with_digits_a_lone_number_and_nothing_are_text_and_not_a_place() {
        fn text(text: &str) -> (&str, Pasted) {
            (text, Pasted::Text(text.to_string()))
        }
        each_reads(&[
            text(""),
            text("prop_bench_01a"),
            text("v_ilev_door_2"),
            text("hei_3_4_5"),
            text("3d_marker"),
            text("door 12 5 8"),
            text("12 5 8 door"),
            text("bench 01"),
            text("12.5"),
            text("-12.5"),
            text("1e5"),
            text("12.5f"),
            text("1.2.3"),
            text("12:30"),
            text("2013-10-01"),
            text("vector3"),
            text("vector3()"),
            text("vector3(1)"),
            text("vector3(1, 2, 3, 4, 5)"),
            text("vector3(1, two, 3)"),
            text("vector3(1,, 2, 3)"),
            text("vector3(, 1, 2, 3)"),
            text("vector3((1, 2, 3))"),
            text("vector3(1, 2, 3) here"),
            text("bench one(1, 2, 3)"),
            text("12(1, 2, 3)"),
            text("(1, 2, 3"),
            text("[1, 2, 3)"),
            text("{x = 1, y = 2, z = 3"),
            text("x = 1, y = 2"),
            text("x = 1"),
            text("x = 1 y = 2 z = 3"),
            text("x = 1, y = 2, z = 3, name = 4"),
            text("bench = 1, 2, 3"),
            text("caf\u{e9} 12 5"),
            text("\u{ff11}, \u{ff12}, \u{ff13}"),
            text("\u{2212}1, 2, 3"),
            text("1, 2, 3,,"),
            text("1 2 3 4 5"),
            text("{x = 1, y = 2}"),
            text("{x = 1, y = 2, z = 3, x = 4}"),
            text("{x = 1, y = 2, z = 3, name = 4}"),
            text("{x = 1, y = 2, z = prop}"),
            text("inf, nan, 3"),
            text("nan, 2, 3"),
            text("NaN 2 3"),
            text("infinity, 2, 3"),
            text("-infinity; 2; 3"),
            text("{x = nan, y = 2, z = 3}"),
            text("., 2, 3"),
            text("e5, 2, 3"),
            text("1e, 2, 3"),
            text("-, 2, 3"),
            text("f, 2, 3"),
            text("(1, 2, 3))"),
            text("[1, [2], 3]"),
            text("0x"),
            text("0xZZ"),
            text("0x12 0x13 0x14"),
            text("hash_face"),
            text("hash_0000000g"),
            text("--"),
        ]);
        assert_eq!(
            read_pasted("   prop_bench_01a \n"),
            Ok(Pasted::Text("prop_bench_01a".to_string())),
            "the text is trimmed"
        );
        // A whole number alone is a hash and no place: a year is one.
        assert_eq!(read_pasted("2013"), Ok(Pasted::Hash(2013)));
    }

    #[test]
    fn the_three_spellings_of_a_hash_read_to_the_same_32_bits() {
        // A name whose hash a script prints with a minus sign, and one it prints without.
        let hash_of = |negative: bool| {
            (0..64)
                .map(|n| or_io::string_hash(format!("prop_bench_{n:02}").as_bytes()))
                .find(|hash| (*hash > i32::MAX as u32) == negative)
                .expect("one name in 64 has the sign looked for")
        };
        let (high, low) = (hash_of(true), hash_of(false));
        for hash in [high, low, 0, 1, u32::MAX, i32::MAX as u32, i32::MIN as u32] {
            let spelled = spell_hash(hash, |_| None);
            for spelling in [&spelled.hexadecimal, &spelled.unsigned, &spelled.signed] {
                assert_eq!(read_pasted(spelling), Ok(Pasted::Hash(hash)), "{spelling}");
            }
            assert_eq!(spelled.unsigned, hash.to_string());
            assert_eq!(spelled.signed, (hash as i32).to_string());
            assert_eq!(
                u32::from_str_radix(spelled.hexadecimal.trim_start_matches("0x"), 16),
                Ok(hash)
            );
            // The spelling the tree shows a hash with no name as, whose eight digits are the ones behind `0x`, and
            // hexadecimal in either case with fewer digits.
            let unnamed = or_io::NameIndex::new("test").display(hash);
            assert_eq!(read_pasted(&unnamed), Ok(Pasted::Hash(hash)));
            assert_eq!(
                spelled.hexadecimal.strip_prefix("0x"),
                unnamed.strip_prefix(UNNAMED),
                "{hash}"
            );
            assert_eq!(read_pasted(&format!("0X{hash:x}")), Ok(Pasted::Hash(hash)));
            assert_eq!(
                read_pasted(&format!(" 0x{hash:X} ")),
                Ok(Pasted::Hash(hash))
            );
        }
    }

    #[test]
    fn a_number_that_fits_neither_spelling_of_32_bits_is_refused() {
        let above = (i64::from(u32::MAX) + 1).to_string();
        let below = (i64::from(i32::MIN) - 1).to_string();
        let long = "9".repeat(60);
        let wide = format!("0x1{:08X}", 0);
        for pasted in [&above, &below, &long, &wide] {
            assert_eq!(
                read_pasted(pasted),
                Err(Refused::HashTooWide(pasted.to_string())),
                "{pasted}"
            );
        }
        // The edges fit, and noughts in front add no bits.
        assert_eq!(
            read_pasted(&u32::MAX.to_string()),
            Ok(Pasted::Hash(u32::MAX))
        );
        assert_eq!(
            read_pasted(&i32::MIN.to_string()),
            Ok(Pasted::Hash(i32::MIN as u32))
        );
        assert_eq!(
            read_pasted(&format!("0x000{:08X}", u32::MAX)),
            Ok(Pasted::Hash(u32::MAX))
        );
        let said = Refused::HashTooWide(above.clone()).to_string();
        assert!(
            said.contains(&above)
                && said.contains(&i32::MIN.to_string())
                && said.contains(&u32::MAX.to_string()),
            "{said}"
        );
    }

    #[test]
    fn a_name_is_spelled_with_its_hash_where_the_lookup_knows_it() {
        let hash = or_io::string_hash(b"prop_bench_01a");
        let mut names = or_io::NameIndex::new("test");
        names.insert("prop_bench_01a");
        let lookup = |hash: u32| names.resolve(hash).map(|found| found.name.to_string());
        assert_eq!(
            spell_hash(hash, lookup).name.as_deref(),
            Some("prop_bench_01a")
        );
        assert_eq!(spell_hash(hash ^ 1, lookup).name, None);
    }

    #[test]
    fn a_position_past_what_a_map_stores_is_refused_with_the_reason() {
        // The Map screen stores 32-bit numbers and refuses one that is not finite; the first power of ten past them is not.
        let past = format!("1e{}", f32::MAX_10_EXP + 1);
        assert!(past.parse::<f32>().is_ok_and(f32::is_infinite));
        for pasted in [
            format!("vector3({past}, 2, 3)"),
            format!("1, -{past}, 3"),
            format!("{{x = 1, y = 2, z = {past}}}"),
            format!("1; 2; 3; {past}"),
        ] {
            let refused = read_pasted(&pasted);
            assert!(
                matches!(&refused, Err(Refused::NotFinite(number)) if number.trim_start_matches('-') == past),
                "{pasted}: {refused:?}"
            );
        }
        // A number of a French list is named as it was written, with its comma.
        let french = past.replacen('e', ",5e", 1);
        assert_eq!(
            read_pasted(&format!("1; 2; -{french}")),
            Err(Refused::NotFinite(format!("-{french}")))
        );
        let said = Refused::NotFinite(past.clone()).to_string();
        assert!(
            said.contains(&past) && said.contains("not finite"),
            "{said}"
        );
        // The largest number a map stores is a place, and a lone number past it is text: it has no shape of a place.
        let most = format!("{:e}", f32::MAX);
        assert_eq!(
            read_pasted(&format!("{most}, 2, 3")),
            Ok(place(f32::MAX, 2.0, 3.0))
        );
        assert_eq!(read_pasted(&past), Ok(Pasted::Text(past)));
    }
}
