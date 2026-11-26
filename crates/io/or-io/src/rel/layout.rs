//! How a typed `.rel` item outside `rel_type` 4 lays out its fields: little-endian, each
//! at its natural alignment counted from the item's start, or packed.

use super::refuse;
use crate::error::Result;

/// The class id and the 24-bit name table offset a game object opens with; the class id is its variant's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NameTableOffset(pub u32);

/// A `rage::Vector3`: x, y, z and w, 16-byte aligned.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

pub(crate) struct ItemReader<'a> {
    bytes: &'a [u8],
    at: usize,
    packed: bool,
}

pub(crate) struct ItemWriter {
    out: Vec<u8>,
    packed: bool,
    class_id: u8,
}

impl<'a> ItemReader<'a> {
    pub(crate) fn new(bytes: &'a [u8], packed: bool) -> Self {
        Self {
            bytes,
            at: 0,
            packed,
        }
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let slice = self
            .at
            .checked_add(len)
            .and_then(|end| self.bytes.get(self.at..end))
            .ok_or_else(|| {
                refuse(format!(
                    "an item needs {len} bytes at {:#x} and ends at {:#x}",
                    self.at,
                    self.bytes.len()
                ))
            })?;
        self.at += len;
        Ok(slice)
    }

    /// Skips to the next multiple of `align`, refusing padding that is not zero, which a write would lose.
    pub(crate) fn align(&mut self, align: usize) -> Result<()> {
        if self.packed {
            return Ok(());
        }
        let pad = self.at.next_multiple_of(align) - self.at;
        if self.take(pad)?.iter().any(|&b| b != 0) {
            return Err(refuse(format!(
                "an item's padding before {:#x} is not zero",
                self.at
            )));
        }
        Ok(())
    }

    pub(crate) fn at_end(&self) -> bool {
        self.at == self.bytes.len()
    }

    /// Ends the item at a multiple of `align`, refusing bytes past it.
    pub(crate) fn finish(mut self, align: usize) -> Result<()> {
        self.align(align)?;
        if self.at != self.bytes.len() {
            return Err(refuse(format!(
                "an item is {} bytes and its type takes {}",
                self.bytes.len(),
                self.at
            )));
        }
        Ok(())
    }
}

impl ItemWriter {
    pub(crate) fn new(packed: bool, class_id: u8) -> Self {
        Self {
            out: Vec::new(),
            packed,
            class_id,
        }
    }

    pub(crate) fn put(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    pub(crate) fn align(&mut self, align: usize) {
        if !self.packed {
            let len = self.out.len().next_multiple_of(align);
            self.out.resize(len, 0);
        }
    }

    pub(crate) fn class_id(&self) -> u8 {
        self.class_id
    }

    pub(crate) fn finish(mut self, align: usize) -> Vec<u8> {
        self.align(align);
        self.out
    }
}

/// A value an item holds; `read_fields` and `write_fields` leave out the value's own alignment and tail padding.
pub(crate) trait Field: Sized {
    const ALIGN: usize;

    fn read_fields(r: &mut ItemReader<'_>) -> Result<Self>;

    fn write_fields(&self, w: &mut ItemWriter) -> Result<()>;

    fn read(r: &mut ItemReader<'_>) -> Result<Self> {
        r.align(Self::ALIGN)?;
        let value = Self::read_fields(r)?;
        r.align(Self::ALIGN)?;
        Ok(value)
    }

    fn write(&self, w: &mut ItemWriter) -> Result<()> {
        w.align(Self::ALIGN);
        self.write_fields(w)?;
        w.align(Self::ALIGN);
        Ok(())
    }
}

/// A counted array's count.
pub(crate) trait Count: Field {
    fn to_len(self) -> usize;
    fn from_len(len: usize) -> Option<Self>;
}

/// An array the item stores as a count and then that many entries.
pub(crate) trait Counted: Sized {
    const ELEMENT_ALIGN: usize;

    fn read_counted<C: Count>(r: &mut ItemReader<'_>) -> Result<Self>;

    fn write_counted<C: Count>(&self, w: &mut ItemWriter) -> Result<()>;
}

impl<T: Field> Counted for Vec<T> {
    const ELEMENT_ALIGN: usize = T::ALIGN;

    fn read_counted<C: Count>(r: &mut ItemReader<'_>) -> Result<Self> {
        let count = C::read(r)?.to_len();
        let mut entries = Vec::with_capacity(count.min(r.bytes.len() - r.at));
        for _ in 0..count {
            entries.push(T::read(r)?);
        }
        Ok(entries)
    }

    fn write_counted<C: Count>(&self, w: &mut ItemWriter) -> Result<()> {
        let count = C::from_len(self.len()).ok_or_else(|| {
            refuse(format!(
                "an array of {} entries is past its count's {} bytes",
                self.len(),
                size_of::<C>()
            ))
        })?;
        count.write(w)?;
        self.iter().try_for_each(|entry| entry.write(w))
    }
}

macro_rules! scalars {
    ($($ty:ty),*) => {$(
        impl Field for $ty {
            const ALIGN: usize = size_of::<$ty>();

            fn read_fields(r: &mut ItemReader<'_>) -> Result<Self> {
                let bytes = r.take(size_of::<$ty>())?;
                Ok(<$ty>::from_le_bytes(bytes.try_into().map_err(|_| refuse("a short scalar".into()))?))
            }

            fn write_fields(&self, w: &mut ItemWriter) -> Result<()> {
                w.put(&self.to_le_bytes());
                Ok(())
            }
        }
    )*};
}

scalars!(u8, i8, u16, i16, u32, i32, f32);

macro_rules! counts {
    ($($ty:ty),*) => {$(
        impl Count for $ty {
            fn to_len(self) -> usize {
                self as usize
            }

            fn from_len(len: usize) -> Option<Self> {
                <$ty>::try_from(len).ok()
            }
        }
    )*};
}

counts!(u8, u16, u32);

impl<T: Field, const N: usize> Field for [T; N] {
    const ALIGN: usize = T::ALIGN;

    fn read_fields(r: &mut ItemReader<'_>) -> Result<Self> {
        let entries = (0..N).map(|_| T::read(r)).collect::<Result<Vec<_>>>()?;
        entries
            .try_into()
            .map_err(|_| refuse("a fixed array read short".into()))
    }

    fn write_fields(&self, w: &mut ItemWriter) -> Result<()> {
        self.iter().try_for_each(|entry| entry.write(w))
    }
}

impl Field for NameTableOffset {
    const ALIGN: usize = 4;

    fn read_fields(r: &mut ItemReader<'_>) -> Result<Self> {
        Ok(Self(u32::read_fields(r)? >> 8))
    }

    fn write_fields(&self, w: &mut ItemWriter) -> Result<()> {
        if self.0 > 0xFF_FFFF {
            return Err(refuse(format!(
                "a name table offset {:#x} is past 24 bits",
                self.0
            )));
        }
        (u32::from(w.class_id()) | self.0 << 8).write_fields(w)
    }
}

impl Field for Vector3 {
    const ALIGN: usize = 16;

    fn read_fields(r: &mut ItemReader<'_>) -> Result<Self> {
        Ok(Self {
            x: f32::read_fields(r)?,
            y: f32::read_fields(r)?,
            z: f32::read_fields(r)?,
            w: f32::read_fields(r)?,
        })
    }

    fn write_fields(&self, w: &mut ItemWriter) -> Result<()> {
        [self.x, self.y, self.z, self.w]
            .iter()
            .try_for_each(|v| v.write_fields(w))
    }
}

pub(crate) const fn max_align(aligns: &[usize]) -> usize {
    let mut max = 1;
    let mut i = 0;
    while i < aligns.len() {
        if aligns[i] > max {
            max = aligns[i];
        }
        i += 1;
    }
    max
}

macro_rules! rel_align {
    ($ty:ty) => {
        <$ty as $crate::rel::layout::Field>::ALIGN
    };
    ($ty:ty, $count:ty) => {
        $crate::rel::layout::max_align(&[
            <$count as $crate::rel::layout::Field>::ALIGN,
            <$ty as $crate::rel::layout::Counted>::ELEMENT_ALIGN,
        ])
    };
}

macro_rules! rel_read {
    ($r:ident, $ty:ty) => {
        <$ty as $crate::rel::layout::Field>::read($r)?
    };
    ($r:ident, $ty:ty, $count:ty) => {
        <$ty as $crate::rel::layout::Counted>::read_counted::<$count>($r)?
    };
}

macro_rules! rel_write {
    ($w:ident, $v:expr, $ty:ty) => {
        <$ty as $crate::rel::layout::Field>::write($v, $w)?
    };
    ($w:ident, $v:expr, $ty:ty, $count:ty) => {
        <$ty as $crate::rel::layout::Counted>::write_counted::<$count>($v, $w)?
    };
}

/// A struct of fields in file order, after its base's; `field: Vec<T> = C` is an array counted by a `C` before it.
///
/// Each field in a trailing `[newer] { .. }` is an `Option`, `None` from the
/// first one an older, shorter item stops before; only an item's own type,
/// never a base or an entry, declares them.
macro_rules! rel_struct {
    (
        $(#[$meta:meta])*
        pub struct $name:ident $(: $base:ident)? {
            $($field:ident: $ty:ty $(= $count:ty)?),* $(,)?
            $([newer] { $($newer:ident: $newer_ty:ty),* $(,)? })?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            $(pub base: $base,)?
            $(pub $field: $ty,)*
            $($(pub $newer: Option<$newer_ty>,)*)?
        }

        impl $name {
            /// The alignment of the fields this type declares, leaving out its base's.
            pub(crate) const OWN_ALIGN: usize = $crate::rel::layout::max_align(&[
                $($crate::rel::layout::rel_align!($ty $(, $count)?),)*
                $($(<$newer_ty as $crate::rel::layout::Field>::ALIGN,)*)?
            ]);
        }

        impl $crate::rel::layout::Field for $name {
            const ALIGN: usize = $crate::rel::layout::max_align(&[
                Self::OWN_ALIGN,
                $(<$base as $crate::rel::layout::Field>::ALIGN)?
            ]);

            #[allow(unused_variables)]
            fn read_fields(
                r: &mut $crate::rel::layout::ItemReader<'_>,
            ) -> $crate::error::Result<Self> {
                Ok(Self {
                    $(base: <$base as $crate::rel::layout::Field>::read_fields(r)?,)?
                    $($field: $crate::rel::layout::rel_read!(r, $ty $(, $count)?),)*
                    $($($newer: if r.at_end() {
                        None
                    } else {
                        Some(<$newer_ty as $crate::rel::layout::Field>::read(r)?)
                    },)*)?
                })
            }

            #[allow(unused_variables)]
            fn write_fields(
                &self,
                w: &mut $crate::rel::layout::ItemWriter,
            ) -> $crate::error::Result<()> {
                $(<$base as $crate::rel::layout::Field>::write_fields(&self.base, w)?;)?
                $($crate::rel::layout::rel_write!(w, &self.$field, $ty $(, $count)?);)*
                $(
                    let set = [$(self.$newer.is_some()),*];
                    if set.windows(2).any(|pair| !pair[0] && pair[1]) {
                        return Err($crate::rel::refuse(format!(
                            "{} sets a newer field after one it leaves out",
                            stringify!($name)
                        )));
                    }
                    $(if let Some(value) = &self.$newer {
                        <$newer_ty as $crate::rel::layout::Field>::write(value, w)?;
                    })*
                )?
                Ok(())
            }
        }
    };
}

/// The item enum of a `rel_type`: each variant's class id, `packed` for a type laid out with no alignment, `padded` for one that pads its end to its base's alignment too.
macro_rules! rel_items {
    (
        $(#[$meta:meta])*
        pub enum $name:ident, class byte $class_byte:literal {
            $($id:literal => $variant:ident $($layout:ident)?),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $name {
            $($variant(Box<$variant>),)*
        }

        impl $name {
            /// Reads one item by its class id, refusing one with no type here or bytes its type does not take.
            pub fn decode(body: &[u8]) -> $crate::error::Result<Self> {
                use $crate::rel::layout::{Field, ItemReader};
                let class = *body
                    .first()
                    .ok_or_else(|| $crate::rel::refuse("an item is empty".into()))?;
                match class {
                    $($id => {
                        let mut r = ItemReader::new(body, $crate::rel::layout::rel_packed!($($layout)?));
                        r.take($class_byte)?;
                        let item = <$variant as Field>::read_fields(&mut r)?;
                        r.finish($crate::rel::layout::rel_end_align!($variant $($layout)?))?;
                        Ok(Self::$variant(Box::new(item)))
                    })*
                    _ => Err($crate::rel::refuse(format!(
                        "class id {class} has no type in {}",
                        stringify!($name)
                    ))),
                }
            }

            /// The item's bytes.
            pub fn encode(&self) -> $crate::error::Result<Vec<u8>> {
                use $crate::rel::layout::{Field, ItemWriter};
                match self {
                    $(Self::$variant(item) => {
                        let mut w = ItemWriter::new($crate::rel::layout::rel_packed!($($layout)?), $id);
                        w.put(&[$id][..$class_byte]);
                        item.write_fields(&mut w)?;
                        Ok(w.finish($crate::rel::layout::rel_end_align!($variant $($layout)?)))
                    })*
                }
            }

            /// The type's name, as the engine declares it.
            pub fn type_name(&self) -> &'static str {
                match self {
                    $(Self::$variant(_) => stringify!($variant),)*
                }
            }

            /// The alignment of the fields the item's type declares itself, 1 for a packed type.
            pub fn own_align(&self) -> usize {
                match self {
                    $(Self::$variant(_) => if $crate::rel::layout::rel_packed!($($layout)?) {
                        1
                    } else {
                        $variant::OWN_ALIGN
                    },)*
                }
            }
        }
    };
}

macro_rules! rel_packed {
    () => {
        false
    };
    (packed) => {
        true
    };
    (padded) => {
        false
    };
}

macro_rules! rel_end_align {
    ($variant:ident) => {
        $variant::OWN_ALIGN
    };
    ($variant:ident packed) => {
        $variant::OWN_ALIGN
    };
    ($variant:ident padded) => {
        <$variant as $crate::rel::layout::Field>::ALIGN
    };
}

pub(crate) use {rel_align, rel_end_align, rel_items, rel_packed, rel_read, rel_struct, rel_write};
