//! The slots a title fills: what it lets a person choose at open and switch while open, and the
//! layers, rules, panel sections and streamed extras it adds. The shared code lists a slot and
//! shows its sentence, and does not know who filled it.

use crate::model::SwitchId;
use crate::sentence::Sentence;

/// An option of a world by the title's number of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionId(pub u32);

/// What an option is chosen among.
#[derive(Debug, Clone, PartialEq)]
pub enum Choices {
    Toggle,
    /// One of these, each by its number.
    OneOf(Vec<(u32, Sentence)>),
    /// A name the install decides, such as one of its packs.
    Named,
}

/// One thing chosen when a world is opened: another choice is another world, with a kept file of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionKind {
    pub id: OptionId,
    pub label: Sentence,
    pub choices: Choices,
}

/// What an option was set to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OptionValue {
    On(bool),
    Choice(u32),
    Named(String),
}

/// One switch turned while a world is open, and whether it starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchKind {
    pub id: SwitchId,
    pub label: Sentence,
    pub on: bool,
}

/// One thing a title adds to a slot, by the title's number of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub id: u32,
    pub label: Sentence,
}

/// What a title adds beyond what every world has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// Layers drawn over the world.
    pub layers: Vec<Slot>,
    /// Rules a world is checked by.
    pub rules: Vec<Slot>,
    /// Sections of the world's panel.
    pub sections: Vec<Slot>,
    /// What its bodies lay out beside entities.
    pub streamed: Vec<Slot>,
}

impl Extras {
    /// Every slot filled, with the name of the slot it fills.
    pub fn all(&self) -> impl Iterator<Item = (&'static str, &Slot)> {
        [
            ("layers", &self.layers),
            ("rules", &self.rules),
            ("sections", &self.sections),
            ("streamed", &self.streamed),
        ]
        .into_iter()
        .flat_map(|(name, slots)| slots.iter().map(move |slot| (name, slot)))
    }
}
