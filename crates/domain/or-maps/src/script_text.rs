//! A position, and a list of named objects with theirs, written as the text a script takes.
//!
//! A form is data: the templates of `script_forms.json`, which holds Lua, JSON and C#, and a file of the same shape adds another
//! with no code ([`ScriptForms::from_json`]). A template is text with `{x}`, `{y}`, `{z}` and `{heading}` where the numbers go,
//! and a list's item has `{name}` and `{place}`, the place being the form's own; any other brace is written as it is. Every
//! number is written with the decimals the caller chose, and [`crate::place::read_pasted`] reads each built-in form back.
//!
//! ⚠ **A heading is one turn about the vertical axis** ([`heading_of`]), which is all a script's heading holds: an object that
//! also leans is written with the Z of the turns the Map screen shows, and a script that sets that heading stands it upright.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::Deserialize;

use crate::edit::Pose;

/// What the `format` of a file of script forms says.
pub const FORMAT: &str = "openrage-script-forms";
pub const SCHEMA: u32 = 1;

/// The most decimals a number is written with: a 32-bit number has no digit past them.
pub const MOST_DECIMALS: usize = (f32::MANTISSA_DIGITS as i32 - f32::MIN_EXP) as usize;

const BUILT_IN: &str = include_str!("script_forms.json");

/// Why a file of script forms was not read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the script forms are not read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the script forms are not read: their format is `{0}`, not `{FORMAT}`")]
    Format(String),
    #[error(
        "the script forms were written by a newer OpenRage (schema {0}) than this one reads (schema {SCHEMA})"
    )]
    Newer(u32),
    #[error("a script form has no id, or two are called `{0}`")]
    Id(String),
    #[error(
        "the script form `{form}` cannot be used: its `{template}` has no `{{{placeholder}}}`"
    )]
    Lacks {
        form: String,
        template: &'static str,
        placeholder: &'static str,
    },
    #[error("the script form `{form}` cannot be used: its `{template}` has a `{{{placeholder}}}` nothing fills")]
    Stray {
        form: String,
        template: &'static str,
        placeholder: &'static str,
    },
    #[error("a position or a heading that is not a finite number is not written")]
    NotFinite,
    #[error("{0} decimals are not written: a 32-bit number has none past {MOST_DECIMALS}")]
    Decimals(usize),
    #[error("the name {0:?} holds a control character and is not written")]
    Name(String),
}

/// One way of writing a place and a list of objects, as templates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptForm {
    /// What a caller asks for the form by, and what a person's last choice is kept as.
    pub id: String,
    /// What a person reads: the name of the language.
    pub name: String,
    /// A position: `{x}`, `{y}` and `{z}`.
    pub place: String,
    /// A position with `{heading}`.
    pub place_heading: String,
    pub list_open: String,
    /// One object: `{name}` and `{place}`, which is `place` or `place_heading` filled.
    pub list_item: String,
    /// What stands between two items and never after the last.
    pub list_separator: String,
    pub list_close: String,
    /// What a character of a name is written as inside the quotes of `list_item`.
    #[serde(default)]
    pub escapes: BTreeMap<char, String>,
}

/// The forms a person chooses among.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptForms {
    pub format: String,
    pub schema: u32,
    pub forms: Vec<ScriptForm>,
}

/// One object of a list: what it is called and where it stands.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptObject {
    pub name: String,
    pub position: Vec3,
    /// In degrees.
    pub heading: Option<f32>,
}

/// The heading a script takes for `pose`, in degrees from 0 up to 360: the turn about Z of the turns the Map screen shows.
pub fn heading_of(pose: &Pose) -> f32 {
    let heading = pose.degrees()[2].rem_euclid(360.0);
    // ⚠ A turn a hair under nought is 360 less that hair, which a 32-bit number rounds to 360 itself.
    if heading >= 360.0 {
        0.0
    } else {
        heading
    }
}

impl ScriptForms {
    /// The forms the tree holds: Lua, JSON and C#.
    pub fn built_in() -> Result<Self, Error> {
        Self::from_json(BUILT_IN)
    }

    /// Read a file of forms, refusing one whose templates could not be filled.
    pub fn from_json(text: &str) -> Result<Self, Error> {
        let forms: Self = serde_json::from_str(text)?;
        if forms.format != FORMAT {
            return Err(Error::Format(forms.format));
        }
        if forms.schema > SCHEMA {
            return Err(Error::Newer(forms.schema));
        }
        for (at, form) in forms.forms.iter().enumerate() {
            if form.id.is_empty() || forms.forms[..at].iter().any(|other| other.id == form.id) {
                return Err(Error::Id(form.id.clone()));
            }
            form.check()?;
        }
        Ok(forms)
    }

    pub fn form(&self, id: &str) -> Option<&ScriptForm> {
        self.forms.iter().find(|form| form.id == id)
    }
}

impl ScriptForm {
    /// Whether every template holds the placeholders it is filled with and no other of them.
    fn check(&self) -> Result<(), Error> {
        let templates: [(&'static str, &str, &[&'static str]); 3] = [
            ("place", &self.place, &["x", "y", "z"]),
            (
                "place_heading",
                &self.place_heading,
                &["x", "y", "z", "heading"],
            ),
            ("list_item", &self.list_item, &["name", "place"]),
        ];
        for (template, text, filled) in templates {
            for placeholder in ["x", "y", "z", "heading", "name", "place"] {
                let held = text.contains(&format!("{{{placeholder}}}"));
                let form = || self.id.clone();
                match (held, filled.contains(&placeholder)) {
                    (false, true) => {
                        return Err(Error::Lacks {
                            form: form(),
                            template,
                            placeholder,
                        })
                    }
                    (true, false) => {
                        return Err(Error::Stray {
                            form: form(),
                            template,
                            placeholder,
                        })
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// `position`, with `heading` in degrees where there is one, as this form writes it with `decimals` decimals.
    pub fn place(
        &self,
        position: Vec3,
        heading: Option<f32>,
        decimals: usize,
    ) -> Result<String, Error> {
        if !(position.is_finite() && heading.is_none_or(f32::is_finite)) {
            return Err(Error::NotFinite);
        }
        // ⚠ Formatting panics on a precision past 16 bits, so the decimals are judged before a number is written.
        if decimals > MOST_DECIMALS {
            return Err(Error::Decimals(decimals));
        }
        let [x, y, z] = position.to_array().map(|value| number(value, decimals));
        let mut values = vec![("x", x), ("y", y), ("z", z)];
        Ok(match heading {
            Some(heading) => {
                values.push(("heading", number(heading, decimals)));
                fill(&self.place_heading, &values)
            }
            None => fill(&self.place, &values),
        })
    }

    /// `objects` as the list this form writes, each with its name and its place.
    pub fn objects(&self, objects: &[ScriptObject], decimals: usize) -> Result<String, Error> {
        // ⚠ The items of a list have one shape, which a C# array asks for: headings are written only where every object has one.
        let headed = objects.iter().all(|object| object.heading.is_some());
        let items = objects
            .iter()
            .map(|object| {
                let heading = object.heading.filter(|_| headed);
                let values = [
                    ("name", self.quoted(&object.name)?),
                    ("place", self.place(object.position, heading, decimals)?),
                ];
                Ok(fill(&self.list_item, &values))
            })
            .collect::<Result<Vec<String>, Error>>()?;
        Ok(format!(
            "{}{}{}",
            self.list_open,
            items.join(&self.list_separator),
            self.list_close
        ))
    }

    /// `name` as it stands between the quotes of an item.
    fn quoted(&self, name: &str) -> Result<String, Error> {
        if name.chars().any(char::is_control) {
            return Err(Error::Name(name.to_string()));
        }
        let mut quoted = String::with_capacity(name.len());
        for c in name.chars() {
            match self.escapes.get(&c) {
                Some(escaped) => quoted.push_str(escaped),
                None => quoted.push(c),
            }
        }
        Ok(quoted)
    }
}

/// `value` with `decimals` decimals, and no sign on a nought.
fn number(value: f32, decimals: usize) -> String {
    let written = format!("{value:.decimals$}");
    match written.strip_prefix('-') {
        Some(digits) if digits.bytes().all(|b| matches!(b, b'0' | b'.')) => digits.to_string(),
        _ => written,
    }
}

/// `template` with each `{key}` of `values` replaced by its value, in one pass: a value is never read as a template.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut filled = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        filled.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let found = values.iter().find_map(|(key, value)| {
            let closed = after.strip_prefix(key)?.strip_prefix('}')?;
            Some((value, closed))
        });
        rest = match found {
            Some((value, closed)) => {
                filled.push_str(value);
                closed
            }
            None => {
                filled.push('{');
                after
            }
        };
    }
    filled.push_str(rest);
    filled
}

#[cfg(test)]
mod tests {
    use glam::Quat;
    use or_io::decl::File;
    use or_io::meta::typed::CMapData;
    use or_world::Placement;

    use super::*;
    use crate::edit::pose_of;
    use crate::place::{read_pasted, Pasted, PastedPlace};
    use crate::sample::{map_file, Entity};

    fn forms() -> ScriptForms {
        ScriptForms::built_in().expect("the forms of the tree read")
    }

    /// Half of the last decimal written, and what a 32-bit number of that size cannot tell apart.
    fn within(read: f32, written: f32, decimals: usize) -> bool {
        let half = 0.5 * 10f64.powi(-(decimals as i32));
        let grain = f64::from(written.abs().max(1.0)) * f64::from(f32::EPSILON);
        (f64::from(read) - f64::from(written)).abs() <= half + grain
    }

    fn reads_back(text: &str, position: Vec3, heading: Option<f32>, decimals: usize) {
        let read = read_pasted(text);
        let Ok(Pasted::Place(PastedPlace {
            x,
            y,
            z: Some(z),
            heading: read_heading,
        })) = read
        else {
            panic!("{text:?} read as {read:?}, which is no place with a height");
        };
        for (read, written) in [(x, position.x), (y, position.y), (z, position.z)] {
            assert!(
                within(read, written, decimals),
                "{text:?}: {read} is not {written} to {decimals} decimals"
            );
        }
        match (read_heading, heading) {
            (Some(read), Some(written)) => assert!(
                within(read, written, decimals),
                "{text:?}: the heading {read} is not {written} to {decimals} decimals"
            ),
            (None, None) => {}
            other => panic!("{text:?}: the heading read and the one written are {other:?}"),
        }
    }

    const POSITIONS: [[f32; 3]; 6] = [
        [125.5, -340.25, 28.0],
        [-3061.4578, 7412.0063, -112.9999],
        [0.0, 0.0, 0.0],
        [-0.0004, 0.0004, -0.4996],
        [16000.125, -16000.125, 2700.5],
        [1.0e-7, -1.0e7, 123456.79],
    ];

    #[test]
    fn the_forms_of_the_tree_are_lua_json_and_c_sharp() {
        let ids: Vec<String> = forms().forms.into_iter().map(|form| form.id).collect();
        assert_eq!(ids, ["lua", "json", "csharp"]);
        assert!(forms().form("lua").is_some() && forms().form("python").is_none());
    }

    #[test]
    fn every_form_reads_back_as_the_position_it_was_written_from_to_the_decimals_written() {
        for form in &forms().forms {
            for position in POSITIONS.map(Vec3::from) {
                for heading in [None, Some(0.0), Some(271.4567), Some(-45.5)] {
                    for decimals in 0..=6 {
                        let text = form.place(position, heading, decimals).unwrap();
                        reads_back(&text, position, heading, decimals);
                    }
                }
            }
        }
    }

    #[test]
    fn a_number_is_written_with_the_decimals_asked_for_and_no_sign_on_a_nought() {
        assert_eq!(number(125.5, 0), "126");
        assert_eq!(number(125.25, 1), "125.2");
        assert_eq!(number(-340.25, 3), "-340.250");
        assert_eq!(number(-0.0004, 2), "0.00");
        assert_eq!(number(-0.0, 0), "0");
        assert_eq!(number(-0.006, 2), "-0.01");
        for form in &forms().forms {
            let text = form.place(Vec3::new(1.0, 2.0, 3.0), Some(4.0), 3).unwrap();
            let decimals = text.matches(".000").count();
            assert_eq!(decimals, 4, "{text}");
        }
    }

    #[test]
    fn a_number_of_c_sharp_is_written_as_the_32_bit_literal_a_vector_takes() {
        // Without its `f` a number with decimals is a 64-bit one, which no vector of C# is built from.
        let forms = forms();
        let form = forms.form("csharp").unwrap();
        let position = Vec3::new(125.5, -340.25, 28.0);
        for (heading, numbers) in [(None, 3), (Some(271.5), 4)] {
            let text = form.place(position, heading, 2).unwrap();
            let inside = text
                .split_once('(')
                .and_then(|(_, rest)| rest.strip_suffix(')'))
                .expect(&text);
            let written: Vec<&str> = inside.split(", ").collect();
            assert_eq!(written.len(), numbers, "{text}");
            for number in written {
                let digits = number.strip_suffix('f').expect(&text);
                assert!(digits.parse::<f32>().is_ok(), "{text}");
            }
        }
    }

    #[test]
    fn a_name_of_c_sharp_holds_no_character_that_ends_a_line_of_it() {
        // C# ends a line at these two as at a line feed, and a string does not run past the end of its line.
        let forms = forms();
        let form = forms.form("csharp").unwrap();
        for ends_a_line in ['\u{2028}', '\u{2029}'] {
            let object = ScriptObject {
                name: format!("yard{ends_a_line}gate"),
                position: Vec3::new(125.5, -340.25, 28.0),
                heading: None,
            };
            let list = form.objects(&[object], 2).unwrap();
            let escaped = format!("\"yard\\u{:04X}gate\"", u32::from(ends_a_line));
            assert!(
                !list.contains(ends_a_line) && list.contains(&escaped),
                "{list:?}"
            );
        }
    }

    #[test]
    fn more_decimals_than_a_32_bit_number_has_are_refused_and_not_written() {
        // The smallest number above nought is the one with the most decimals, and its last is the last any number has.
        let smallest = f32::from_bits(1);
        let exact = number(smallest, MOST_DECIMALS);
        assert!(!exact.ends_with('0'), "{exact}");
        assert_eq!(exact.parse::<f32>(), Ok(smallest));
        let one_more = format!("{smallest:.0$}", MOST_DECIMALS + 1);
        assert_eq!(one_more.strip_suffix('0'), Some(exact.as_str()));
        let position = Vec3::new(125.5, -340.25, 28.0);
        let object = ScriptObject {
            name: "prop_bench_01a".to_string(),
            position,
            heading: None,
        };
        for form in &forms().forms {
            let text = form.place(position, Some(90.0), MOST_DECIMALS).unwrap();
            reads_back(&text, position, Some(90.0), MOST_DECIMALS);
            for decimals in [MOST_DECIMALS + 1, usize::from(u16::MAX) + 1, usize::MAX] {
                assert!(matches!(
                    form.place(position, None, decimals),
                    Err(Error::Decimals(refused)) if refused == decimals
                ));
                assert!(matches!(
                    form.objects(std::slice::from_ref(&object), decimals),
                    Err(Error::Decimals(refused)) if refused == decimals
                ));
            }
        }
        let said = Error::Decimals(MOST_DECIMALS + 1).to_string();
        assert!(
            said.contains(&(MOST_DECIMALS + 1).to_string())
                && said.contains(&MOST_DECIMALS.to_string()),
            "{said}"
        );
    }

    #[test]
    fn a_list_of_named_objects_holds_each_name_beside_the_place_it_reads_back_as() {
        let objects: Vec<ScriptObject> = POSITIONS
            .iter()
            .enumerate()
            .map(|(at, position)| ScriptObject {
                name: format!("prop_bench_{at:02}"),
                position: Vec3::from(*position),
                heading: Some(at as f32 * 61.25),
            })
            .collect();
        let decimals = 3;
        for form in &forms().forms {
            let list = form.objects(&objects, decimals).unwrap();
            let mut rest = list.as_str();
            assert!(rest.starts_with(&form.list_open) && rest.ends_with(&form.list_close));
            for (at, object) in objects.iter().enumerate() {
                // Each object is found after the one before it, its name first and then a place the reader takes.
                let name = rest.find(&format!("\"{}\"", object.name)).expect(&list);
                assert_eq!(
                    rest[..name].matches(&form.list_separator).count(),
                    usize::from(at > 0),
                    "one separator stands between two items of {list}"
                );
                rest = &rest[name..];
                let place = form
                    .place(object.position, object.heading, decimals)
                    .unwrap();
                let found = rest.find(&place).expect(&list);
                assert!(
                    !rest[..found].contains('\n'),
                    "the name and the place are one line of {list}"
                );
                reads_back(&place, object.position, object.heading, decimals);
                rest = &rest[found + place.len()..];
            }
            let after_the_last = rest.strip_suffix(&form.list_close).expect(&list);
            assert!(
                !after_the_last.contains(&form.list_separator),
                "no separator stands after the last item of {list}"
            );
        }
        assert_eq!(
            forms().form("lua").unwrap().objects(&[], decimals).unwrap(),
            "{\n}"
        );
    }

    #[test]
    fn a_list_written_as_json_is_json_that_holds_each_name_and_number_whatever_the_name_holds() {
        let objects = [
            ScriptObject {
                name: r#"a "quoted" name with a \ in it"#.to_string(),
                position: Vec3::new(125.5, -340.25, 28.0),
                heading: Some(271.5),
            },
            ScriptObject {
                name: "{place} {x} {name}".to_string(),
                position: Vec3::new(-1.0, 2.0, -3.0),
                heading: Some(0.5),
            },
        ];
        let forms = forms();
        let list = forms.form("json").unwrap().objects(&objects, 2).unwrap();
        let read: serde_json::Value = serde_json::from_str(&list).expect(&list);
        let read = read.as_array().expect("a list");
        assert_eq!(read.len(), objects.len());
        for (read, object) in read.iter().zip(&objects) {
            assert_eq!(read["name"].as_str(), Some(object.name.as_str()));
            let number = |key: &str| read["position"][key].as_f64().expect(key) as f32;
            assert_eq!(
                [number("x"), number("y"), number("z"), number("heading")],
                [
                    object.position.x,
                    object.position.y,
                    object.position.z,
                    object.heading.unwrap()
                ]
            );
        }
        // The same names between the quotes of Lua and C#, where a quote and a backslash are written the same way.
        for id in ["lua", "csharp"] {
            let list = forms.form(id).unwrap().objects(&objects, 2).unwrap();
            assert!(
                list.contains(r#""a \"quoted\" name with a \\ in it""#)
                    && list.contains(r#""{place} {x} {name}""#),
                "{list}"
            );
        }
    }

    #[test]
    fn a_list_is_written_with_headings_only_where_every_object_has_one() {
        let object = |heading| ScriptObject {
            name: "prop_bench_01a".to_string(),
            position: Vec3::new(125.5, -340.25, 28.0),
            heading,
        };
        for form in &forms().forms {
            let headed = form.place(object(None).position, Some(90.0), 1).unwrap();
            let bare = form.place(object(None).position, None, 1).unwrap();
            let all = form
                .objects(&[object(Some(90.0)), object(Some(90.0))], 1)
                .unwrap();
            assert_eq!(
                (all.matches(&headed).count(), all.matches(&bare).count()),
                (2, 0)
            );
            let mixed = form
                .objects(&[object(Some(90.0)), object(None)], 1)
                .unwrap();
            assert_eq!(
                (mixed.matches(&headed).count(), mixed.matches(&bare).count()),
                (0, 2)
            );
        }
    }

    #[test]
    fn a_form_added_as_data_writes_a_place_and_a_list_with_no_code() {
        let python = serde_json::json!({
            "id": "python",
            "name": "Python",
            "place": "({x}, {y}, {z})",
            "place_heading": "({x}, {y}, {z}, {heading})",
            "list_open": "[",
            "list_item": "\n    {\"name\": '{name}', \"at\": {place}}",
            "list_separator": ",",
            "list_close": "\n]",
            "escapes": { "'": "\\'" }
        });
        let mut file: serde_json::Value = serde_json::from_str(BUILT_IN).unwrap();
        file["forms"].as_array_mut().unwrap().push(python);
        let forms = ScriptForms::from_json(&file.to_string()).unwrap();
        assert_eq!(forms.forms.len(), self::forms().forms.len() + 1);
        let form = forms.form("python").unwrap();
        let position = Vec3::new(125.5, -340.25, 28.0);
        let text = form.place(position, Some(271.5), 2).unwrap();
        assert_eq!(
            text,
            format!(
                "({}, {}, {}, {})",
                number(position.x, 2),
                number(position.y, 2),
                number(position.z, 2),
                number(271.5, 2)
            )
        );
        reads_back(&text, position, Some(271.5), 2);
        let list = form
            .objects(
                &[ScriptObject {
                    name: "the yard's gate".to_string(),
                    position,
                    heading: None,
                }],
                1,
            )
            .unwrap();
        assert_eq!(
            list,
            format!(
                "[\n    {{\"name\": 'the yard\\'s gate', \"at\": {}}}\n]",
                form.place(position, None, 1).unwrap()
            )
        );
    }

    #[test]
    fn a_file_of_forms_that_could_not_be_filled_is_refused_with_what_is_wrong() {
        let changed = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut file: serde_json::Value = serde_json::from_str(BUILT_IN).unwrap();
            change(&mut file);
            ScriptForms::from_json(&file.to_string())
        };
        assert!(changed(&|_| {}).is_ok());
        assert!(matches!(
            changed(&|file| file["format"] = "openrage-maps".into()),
            Err(Error::Format(format)) if format == "openrage-maps"
        ));
        assert!(matches!(
            changed(&|file| file["schema"] = (SCHEMA + 1).into()),
            Err(Error::Newer(schema)) if schema == SCHEMA + 1
        ));
        assert!(matches!(
            changed(&|file| file["forms"][1]["id"] = "lua".into()),
            Err(Error::Id(id)) if id == "lua"
        ));
        assert!(matches!(
            changed(&|file| file["forms"][0]["id"] = "".into()),
            Err(Error::Id(id)) if id.is_empty()
        ));
        assert!(matches!(
            changed(&|file| file["forms"][0]["place"] = "vector3({x}, {y})".into()),
            Err(Error::Lacks { form, template: "place", placeholder: "z" }) if form == "lua"
        ));
        assert!(matches!(
            changed(&|file| file["forms"][2]["place_heading"] = "new Vector3({x}f, {y}f, {z}f)".into()),
            Err(Error::Lacks { form, template: "place_heading", placeholder: "heading" }) if form == "csharp"
        ));
        assert!(matches!(
            changed(&|file| file["forms"][1]["list_item"] = "{name}".into()),
            Err(Error::Lacks { form, template: "list_item", placeholder: "place" }) if form == "json"
        ));
        assert!(matches!(
            changed(&|file| file["forms"][0]["place"] = "vector4({x}, {y}, {z}, {heading})".into()),
            Err(Error::Stray { form, template: "place", placeholder: "heading" }) if form == "lua"
        ));
        assert!(matches!(
            changed(&|file| file["forms"][0]["colour"] = "red".into()),
            Err(Error::Json(_))
        ));
        assert!(matches!(
            changed(&|file| file["colour"] = "red".into()),
            Err(Error::Json(_))
        ));
        // A form whose names need no escape says none.
        let bare = changed(&|file| {
            file["forms"][0]
                .as_object_mut()
                .expect("a form is an object")
                .remove("escapes")
                .expect("the form had escapes");
        })
        .expect("a form with no escapes reads");
        assert!(bare.forms[0].escapes.is_empty());
        assert!(matches!(ScriptForms::from_json("{"), Err(Error::Json(_))));
    }

    #[test]
    fn a_position_that_is_not_finite_and_a_name_with_a_control_character_are_not_written() {
        let position = Vec3::new(125.5, -340.25, 28.0);
        for form in &forms().forms {
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(matches!(
                    form.place(Vec3::new(bad, 0.0, 0.0), None, 2),
                    Err(Error::NotFinite)
                ));
                assert!(matches!(
                    form.place(position, Some(bad), 2),
                    Err(Error::NotFinite)
                ));
                let object = ScriptObject {
                    name: "prop_bench_01a".to_string(),
                    position: Vec3::new(0.0, 0.0, bad),
                    heading: None,
                };
                assert!(matches!(form.objects(&[object], 2), Err(Error::NotFinite)));
            }
            let object = ScriptObject {
                name: "prop\nbench".to_string(),
                position,
                heading: None,
            };
            assert!(matches!(
                form.objects(&[object], 2),
                Err(Error::Name(name)) if name == "prop\nbench"
            ));
        }
    }

    #[test]
    fn a_heading_written_for_a_stored_rotation_is_the_turn_about_z_of_its_quaternion() {
        // An entity stores the conjugate of the rotation that places it; the file is read back as the Map screen reads it.
        for yaw in [0.0_f32, 0.4, 1.2, 2.9, -0.7, -2.5] {
            let entity = Entity {
                yaw,
                ..Entity::at("prop_bench_01a", [125.5, -340.25, 28.0], 1)
            };
            let map = CMapData::from_bytes(&map_file("yard", &[entity], ([0.0; 3], [1.0; 3])))
                .expect("the sample map reads");
            let stored = map.entities[0].rotation;
            let placing = Quat::from_xyzw(stored.x, stored.y, stored.z, stored.w).conjugate();
            // Where the rotation that places it turns the Y axis, as an angle from Y towards -X: the turn about Z.
            let forward = placing * Vec3::Y;
            let expected = (-forward.x).atan2(forward.y).to_degrees().rem_euclid(360.0);
            let pose = pose_of(&map.entities[0]);
            let heading = heading_of(&pose);
            assert!(
                (heading - expected).abs() < 1e-3,
                "{yaw}: {heading} is not {expected}"
            );
            assert!((0.0..360.0).contains(&heading), "{heading}");
            for form in &forms().forms {
                let text = form.place(pose.position, Some(heading), 2).unwrap();
                reads_back(&text, pose.position, Some(expected), 2);
            }
        }
        // A turn a hair to either side of nought is a heading from 0 up to 360 and never 360 itself.
        for hair in [-1.0e-4_f32, -1.0e-5, -1.0e-6, -1.0e-7, -0.0, 0.0, 1.0e-6] {
            let pose = Pose {
                position: Vec3::ZERO,
                rotation: Quat::from_rotation_z(hair.to_radians()),
                scale_xy: 1.0,
                scale_z: 1.0,
            };
            let heading = heading_of(&pose);
            assert!((0.0..360.0).contains(&heading), "{hair}: {heading}");
            let from_nought = heading.min(360.0 - heading);
            assert!(from_nought < 1e-3, "{hair}: {heading}");
        }
        // The rotation of an interior is stored as it is, and the same heading comes of it.
        let turn = Quat::from_rotation_z(1.2);
        let pose = |placement: Placement| Pose {
            position: Vec3::ZERO,
            rotation: placement.rotation_of(placement.store(turn)),
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        let expected = 1.2_f32.to_degrees();
        for placement in [Placement::Entity, Placement::InteriorInstance] {
            assert!((heading_of(&pose(placement)) - expected).abs() < 1e-3);
        }
    }
}
