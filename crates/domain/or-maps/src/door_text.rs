//! A selection of doors written as the text a door script takes: each door's model, hash, position and heading, and which two are
//! one double door.
//!
//! A form is data: the templates of `door_forms.json`, one form for each way of writing, and a file of the same shape adds another
//! with no code ([`DoorForms::from_json`]). A form's `single` is a door alone and its `pair` a double door. In `single`, `{name}`,
//! `{hash}`, `{hash_signed}`, `{hash_hex}`, `{x}`, `{y}`, `{z}` and `{heading}` are the door's; in `pair` the same behind `a_` and
//! `b_` are the two leaves', and `{x}`, `{y}` and `{z}` the point midway between them. `{index}` counts the items of the list from
//! one. Any other brace is written as it is.
//!
//! ⚠ **The tree vouches for two forms, `lua` and `json`, which are its own.** A form named after another project's door script
//! states `unverified`, and the text it writes begins with a comment that says so: that script's format is its project's to
//! change, and nothing here can check it.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::Deserialize;

use crate::doors::SetDoors;
use crate::script_text::{fill, number, MOST_DECIMALS};

/// What the `format` of a file of door forms says.
pub const FORMAT: &str = "openrage-door-forms";
/// The newest schema of a file of door forms this build reads.
pub const SCHEMA: u32 = 1;

const BUILT_IN: &str = include_str!("door_forms.json");

/// What a template writes of one door, before any prefix.
const OF_A_DOOR: [&str; 8] = [
    "name",
    "hash",
    "hash_signed",
    "hash_hex",
    "x",
    "y",
    "z",
    "heading",
];

/// Why a file of door forms was not read, or a door not written through a form.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the door forms are not read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the door forms are not read: their format is `{0}`, not `{FORMAT}`")]
    Format(String),
    #[error(
        "the door forms were written by a newer OpenRage (schema {0}) than this one reads (schema {SCHEMA})"
    )]
    Newer(u32),
    #[error("a door form has no id, or two are called `{0}`")]
    Id(String),
    #[error("the door form `{form}` cannot be used: its `{template}` has no `{{{placeholder}}}`")]
    Lacks {
        form: String,
        template: &'static str,
        placeholder: String,
    },
    #[error("the door form `{form}` cannot be used: its `{template}` has a `{{{placeholder}}}` nothing fills")]
    Stray {
        form: String,
        template: &'static str,
        placeholder: String,
    },
    #[error("the door form `{0}` is not vouched for and has no `comment` to say so with in the text it writes")]
    Unmarked(String),
    #[error(
        "the door form `{0}` says why it is not vouched for in more than one line, or in none"
    )]
    Mark(String),
    #[error("a position or a heading that is not a finite number is not written")]
    NotFinite,
    #[error("{0} decimals are not written: a 32-bit number has none past {MOST_DECIMALS}")]
    Decimals(usize),
    #[error("the name {0:?} holds a control character and is not written")]
    Name(String),
}

/// One way of writing a list of doors, as templates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoorForm {
    /// What a caller asks for the form by.
    pub id: String,
    /// What a person reads.
    pub name: String,
    /// Why the tree does not vouch for the form, in one line: stated by every form another project's script defines.
    #[serde(default)]
    pub unverified: Option<String>,
    /// What begins a comment in the text the form writes.
    #[serde(default)]
    pub comment: Option<String>,
    pub open: String,
    /// A door alone.
    pub single: String,
    /// A double door.
    pub pair: String,
    /// What stands between two items and never after the last.
    pub separator: String,
    pub close: String,
    /// What a character of a name is written as inside the quotes of an item.
    #[serde(default)]
    pub escapes: BTreeMap<char, String>,
}

/// The forms a person chooses among.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoorForms {
    pub format: String,
    pub schema: u32,
    pub forms: Vec<DoorForm>,
}

/// One door as a script is told of it.
#[derive(Debug, Clone, PartialEq)]
pub struct DoorCopy {
    /// The model's name, or the spelling the tree gives a hash with none.
    pub name: String,
    pub hash: u32,
    pub position: Vec3,
    /// In degrees.
    pub heading: f32,
}

/// One item of a list: a door alone, or the two leaves of a double door.
#[derive(Debug, Clone, PartialEq)]
pub enum Copied {
    Single(DoorCopy),
    Pair(DoorCopy, DoorCopy),
}

/// The doors of `doors` at the places `selection` names, in its order, each double door once and whole.
pub fn copies(doors: &SetDoors, selection: &[usize]) -> Vec<Copied> {
    // ⚠ One leaf of a double door selects both: a script that locks one leaf of two leaves the door open.
    let copy = |at: usize| {
        let door = &doors.doors[at];
        DoorCopy {
            name: door.model.clone(),
            hash: door.hash,
            position: Vec3::from(door.position),
            heading: door.heading,
        }
    };
    let mut written = vec![false; doors.doors.len()];
    let mut out = Vec::new();
    for &at in selection {
        if at >= written.len() || written[at] {
            continue;
        }
        written[at] = true;
        match doors.doors[at].pair.filter(|&other| other < written.len()) {
            Some(other) => {
                written[other] = true;
                out.push(Copied::Pair(copy(at.min(other)), copy(at.max(other))));
            }
            None => out.push(Copied::Single(copy(at))),
        }
    }
    out
}

impl DoorForms {
    /// The forms the tree holds.
    pub fn built_in() -> Result<Self, Error> {
        Self::from_json(BUILT_IN)
    }

    /// Read a file of forms, refusing one whose templates could not be filled or that could not say it is unverified.
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

    /// The form `id` names.
    pub fn form(&self, id: &str) -> Option<&DoorForm> {
        self.forms.iter().find(|form| form.id == id)
    }
}

fn holds(text: &str, placeholder: &str) -> bool {
    text.contains(&format!("{{{placeholder}}}"))
}

impl DoorForm {
    /// Whether each template holds the positions it must and no placeholder only the other fills, and an unverified form can say what it is.
    fn check(&self) -> Result<(), Error> {
        let form = || self.id.clone();
        if let Some(why) = &self.unverified {
            if self.comment.as_ref().is_none_or(String::is_empty) {
                return Err(Error::Unmarked(form()));
            }
            if why.trim().is_empty() || why.chars().any(char::is_control) {
                return Err(Error::Mark(form()));
            }
        }
        for axis in ["x", "y", "z"] {
            if !holds(&self.single, axis) {
                return Err(Error::Lacks {
                    form: form(),
                    template: "single",
                    placeholder: axis.to_string(),
                });
            }
            for leaf in ["a_", "b_"] {
                let placeholder = format!("{leaf}{axis}");
                if !holds(&self.pair, &placeholder) {
                    return Err(Error::Lacks {
                        form: form(),
                        template: "pair",
                        placeholder,
                    });
                }
            }
        }
        for of_a_door in OF_A_DOOR {
            for leaf in ["a_", "b_"] {
                let placeholder = format!("{leaf}{of_a_door}");
                if holds(&self.single, &placeholder) {
                    return Err(Error::Stray {
                        form: form(),
                        template: "single",
                        placeholder,
                    });
                }
            }
            // In `pair` a bare `{x}`, `{y}` or `{z}` is the point midway; the rest of a door alone is neither leaf's.
            if !["x", "y", "z"].contains(&of_a_door) && holds(&self.pair, of_a_door) {
                return Err(Error::Stray {
                    form: form(),
                    template: "pair",
                    placeholder: of_a_door.to_string(),
                });
            }
        }
        Ok(())
    }

    /// `doors` as the list this form writes, every position and heading with `decimals` decimals.
    pub fn write(&self, doors: &[Copied], decimals: usize) -> Result<String, Error> {
        // ⚠ Formatting panics on a precision past 16 bits, so the decimals are judged before a number is written.
        if decimals > MOST_DECIMALS {
            return Err(Error::Decimals(decimals));
        }
        let mut items = Vec::with_capacity(doors.len());
        for (at, copied) in doors.iter().enumerate() {
            let mut values = vec![("index".to_string(), (at + 1).to_string())];
            let template = match copied {
                Copied::Single(door) => {
                    self.of_a_door("", door, decimals, &mut values)?;
                    &self.single
                }
                Copied::Pair(a, b) => {
                    self.of_a_door("a_", a, decimals, &mut values)?;
                    self.of_a_door("b_", b, decimals, &mut values)?;
                    let middle = (a.position + b.position) * 0.5;
                    for (axis, value) in ["x", "y", "z"].into_iter().zip(middle.to_array()) {
                        values.push((axis.to_string(), number(value, decimals)));
                    }
                    &self.pair
                }
            };
            let values: Vec<(&str, String)> = (values.iter())
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect();
            items.push(fill(template, &values));
        }
        let mut text = String::new();
        if let (Some(why), Some(comment)) = (&self.unverified, &self.comment) {
            text.push_str(comment);
            text.push_str(why);
            text.push('\n');
        }
        text.push_str(&self.open);
        text.push_str(&items.join(&self.separator));
        text.push_str(&self.close);
        Ok(text)
    }

    /// What the templates write of `door`, each behind `prefix`.
    fn of_a_door(
        &self,
        prefix: &str,
        door: &DoorCopy,
        decimals: usize,
        values: &mut Vec<(String, String)>,
    ) -> Result<(), Error> {
        if !(door.position.is_finite() && door.heading.is_finite()) {
            return Err(Error::NotFinite);
        }
        if door.name.chars().any(char::is_control) {
            return Err(Error::Name(door.name.clone()));
        }
        let mut name = String::with_capacity(door.name.len());
        for c in door.name.chars() {
            match self.escapes.get(&c) {
                Some(escaped) => name.push_str(escaped),
                None => name.push(c),
            }
        }
        let [x, y, z] = door.position.to_array().map(|v| number(v, decimals));
        let written = [
            name,
            door.hash.to_string(),
            // The same 32 bits as a script prints them.
            (door.hash as i32).to_string(),
            format!("0x{:08X}", door.hash),
            x,
            y,
            z,
            number(door.heading, decimals),
        ];
        for (key, value) in OF_A_DOOR.into_iter().zip(written) {
            values.push((format!("{prefix}{key}"), value));
        }
        Ok(())
    }
}
