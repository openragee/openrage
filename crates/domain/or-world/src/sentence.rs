//! The key of a sentence a title shows a person: the English text, which the app's catalogs hold
//! and turn into the language in use where it is drawn.
//!
//! ⚠ A key is made by [`sentence!`](crate::sentence) alone: the language gate reads the source for
//! that macro's literals, so a key built any other way is a text no catalog was asked for.

/// The English text of a sentence, as the key the catalogs hold it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sentence(&'static str);

impl Sentence {
    /// The key of `text`; called by [`sentence!`](crate::sentence), which the language gate reads.
    // ⚠ Under a name nothing else bears: the gate finds a key made by hand by this name, under
    // whatever name the type was imported and however the call is laid out.
    #[doc(hidden)]
    pub const fn keyed_by_the_macro(text: &'static str) -> Self {
        Self(text)
    }

    /// The English text, which is the key.
    pub const fn key(self) -> &'static str {
        self.0
    }
}

/// The key of an English sentence a person is shown later, in the language in use.
#[macro_export]
macro_rules! sentence {
    ($text:literal) => {
        $crate::sentence::Sentence::keyed_by_the_macro($text)
    };
}
