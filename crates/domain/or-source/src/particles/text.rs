//! The text a particle list's values are written as and read from: whole numbers as numbers, a float as the shortest decimal
//! that reads back as the same float, a vector as one string of its four floats, a hash as `"0x…"`.
//!
//! ⚠ Nothing is guessed on the way back: a value of the wrong shape, out of range or named by no member is a fault that names its
//! place in the document, and a float that no decimal holds exactly (a NaN with a payload, an infinity) is written as its bits.

use std::fmt;

use or_io::decl::{Bool8, Hash32, Keyed, Member};
use or_io::gta5::{Vec3V, Vec4V};
use serde_json::{Map, Value};

/// Where in the document a value is, as `effect_rules[2].timeline.events[0].start_ratio`.
#[derive(Debug, Default)]
pub struct Trail(Vec<String>);

impl Trail {
    /// `f` with the field `name` added to the place.
    pub fn field<R>(&mut self, name: &str, f: impl FnOnce(&mut Self) -> R) -> R {
        self.0.push(name.trim_start_matches("r#").to_string());
        let out = f(self);
        self.0.pop();
        out
    }

    /// `f` with the element `index` added to the place.
    pub fn index<R>(&mut self, index: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        self.0.push(format!("[{index}]"));
        let out = f(self);
        self.0.pop();
        out
    }

    pub fn fault(&self, why: impl Into<String>) -> Fault {
        let mut at = String::new();
        for part in &self.0 {
            if !at.is_empty() && !part.starts_with('[') {
                at.push('.');
            }
            at.push_str(part);
        }
        Fault {
            at,
            why: why.into(),
        }
    }

    /// The members of an object, or the fault that a value is not one.
    pub fn object<'v>(&self, v: &'v Value) -> Result<&'v Map<String, Value>, Fault> {
        v.as_object().ok_or_else(|| self.fault("is not an object"))
    }

    /// The fault that the object `o` holds a member named in none of `known`.
    pub fn only(&self, o: &Map<String, Value>, known: &[&str]) -> Result<(), Fault> {
        match o.keys().find(|k| !known.contains(&k.as_str())) {
            Some(k) => Err(self.fault(format!("has no member `{k}`"))),
            None => Ok(()),
        }
    }
}

/// A fault in the document: where, and what is wrong there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub at: String,
    pub why: String,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.at.is_empty() {
            f.write_str(&self.why)
        } else {
            write!(f, "{} {}", self.at, self.why)
        }
    }
}

/// A value of a particle list that has a text.
pub trait Json: Sized {
    /// A value to read a document into: the declaration's own default.
    fn blank() -> Self;

    fn write(&self) -> Value;
    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault>;
}

macro_rules! whole_numbers {
    ($($t:ty),*) => {$(
        impl Json for $t {
            fn blank() -> Self {
                0
            }

            fn write(&self) -> Value {
                Value::from(*self)
            }

            fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
                let n = v
                    .as_i64()
                    .map(i128::from)
                    .or_else(|| v.as_u64().map(i128::from))
                    .ok_or_else(|| at.fault("is not a whole number"))?;
                *self = <$t>::try_from(n)
                    .map_err(|_| at.fault(format!("is {n}, which a {} cannot hold", stringify!($t))))?;
                Ok(())
            }
        }
    )*};
}

whole_numbers!(i8, u8, i16, u16, i32, u32);

/// A float as the text that reads back as exactly it: the shortest decimal, or its bits where none does.
fn float_text(x: f32) -> String {
    let short = format!("{x:?}");
    match short.parse::<f32>() {
        Ok(back) if x.is_finite() && back.to_bits() == x.to_bits() => short,
        _ => format!("{:#010x}", x.to_bits()),
    }
}

fn float_from(text: &str) -> Option<f32> {
    match text.strip_prefix("0x") {
        Some(bits) => u32::from_str_radix(bits, 16).ok().map(f32::from_bits),
        None => text.parse::<f32>().ok().filter(|x| x.is_finite()),
    }
}

impl Json for f32 {
    fn blank() -> Self {
        0.0
    }

    fn write(&self) -> Value {
        // The decimal is parsed as a double and read back through a float: a number in the document is the short one.
        let number = float_text(*self)
            .parse::<f64>()
            .ok()
            .filter(|wide| (*wide as f32).to_bits() == self.to_bits())
            .and_then(serde_json::Number::from_f64);
        number.map_or_else(
            || Value::String(format!("{:#010x}", self.to_bits())),
            Value::Number,
        )
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        let x = match v {
            // ⚠ A number past what a float holds would be read as an infinity; one is stated by its bits.
            Value::Number(n) => n.as_f64().map(|wide| wide as f32).filter(|x| x.is_finite()),
            Value::String(text) => float_from(text),
            _ => None,
        };
        *self = x.ok_or_else(|| at.fault("is not a number"))?;
        Ok(())
    }
}

impl Json for Bool8 {
    fn blank() -> Self {
        Bool8::default()
    }

    fn write(&self) -> Value {
        match self.raw() {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            other => Value::from(other),
        }
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        *self = match v {
            Value::Bool(b) => Bool8::new(*b),
            Value::Number(n) if n.as_u64() == Some(0) => Bool8::new(false),
            Value::Number(n) if n.as_u64() == Some(1) => Bool8::new(true),
            Value::Number(_) => {
                return Err(
                    at.fault("is a flag byte other than 0 and 1, which this build cannot hold")
                )
            }
            _ => return Err(at.fault("is not true or false")),
        };
        Ok(())
    }
}

impl Json for Hash32 {
    fn blank() -> Self {
        Hash32::default()
    }

    fn write(&self) -> Value {
        hash_text(self.get())
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        *self = Hash32::from_raw(hash_from(v).ok_or_else(|| at.fault("is not a hash"))?);
        Ok(())
    }
}

/// A hash as `"0x…"` or a whole number.
pub fn hash_from(v: &Value) -> Option<u32> {
    match v {
        Value::String(text) => u32::from_str_radix(text.strip_prefix("0x")?, 16).ok(),
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

/// A hash as the text a document holds it as.
pub fn hash_text(hash: u32) -> Value {
    Value::String(format!("{hash:#010x}"))
}

impl Json for String {
    fn blank() -> Self {
        String::new()
    }

    fn write(&self) -> Value {
        Value::String(self.clone())
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        *self = v
            .as_str()
            .ok_or_else(|| at.fault("is not text"))?
            .to_string();
        Ok(())
    }
}

fn lanes_text(lanes: [f32; 4]) -> Value {
    Value::String(
        lanes
            .iter()
            .map(|x| float_text(*x))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn lanes_from(v: &Value, at: &Trail) -> Result<[f32; 4], Fault> {
    let text = v
        .as_str()
        .ok_or_else(|| at.fault("is not four numbers in one text"))?;
    let lanes: Option<Vec<f32>> = text.split_whitespace().map(float_from).collect();
    match lanes.as_deref() {
        Some([x, y, z, w]) => Ok([*x, *y, *z, *w]),
        _ => Err(at.fault("is not four numbers in one text")),
    }
}

impl Json for Vec4V {
    fn blank() -> Self {
        Vec4V::default()
    }

    fn write(&self) -> Value {
        lanes_text([self.x, self.y, self.z, self.w])
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        [self.x, self.y, self.z, self.w] = lanes_from(v, at)?;
        Ok(())
    }
}

impl Json for Vec3V {
    fn blank() -> Self {
        Vec3V::default()
    }

    fn write(&self) -> Value {
        lanes_text([self.x, self.y, self.z, self.w])
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        [self.x, self.y, self.z, self.w] = lanes_from(v, at)?;
        Ok(())
    }
}

/// A name a record holds in sixty-four bytes, NUL-padded.
impl Json for [u8; 64] {
    fn blank() -> Self {
        [0; 64]
    }

    fn write(&self) -> Value {
        let end = self.iter().position(|b| *b == 0).unwrap_or(self.len());
        Value::String(String::from_utf8_lossy(&self[..end]).into_owned())
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        let text = v.as_str().ok_or_else(|| at.fault("is not text"))?;
        if text.len() > 63 {
            return Err(at.fault(format!(
                "is {} bytes, and a name here holds at most 63",
                text.len()
            )));
        }
        *self = [0; 64];
        self[..text.len()].copy_from_slice(text.as_bytes());
        Ok(())
    }
}

/// Two whole numbers, as a list of two.
impl Json for [i32; 2] {
    fn blank() -> Self {
        [0; 2]
    }

    fn write(&self) -> Value {
        Value::Array(self.iter().map(|n| Value::from(*n)).collect())
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        let items = v
            .as_array()
            .filter(|items| items.len() == 2)
            .ok_or_else(|| at.fault("is not a list of two numbers"))?;
        for (i, (slot, item)) in self.iter_mut().zip(items).enumerate() {
            at.index(i, |at| slot.read(item, at))?;
        }
        Ok(())
    }
}

impl<T: Json> Json for Vec<T> {
    fn blank() -> Self {
        Vec::new()
    }

    fn write(&self) -> Value {
        Value::Array(self.iter().map(Json::write).collect())
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        let items = v.as_array().ok_or_else(|| at.fault("is not a list"))?;
        self.clear();
        for (i, item) in items.iter().enumerate() {
            let mut one = T::blank();
            at.index(i, |at| one.read(item, at))?;
            self.push(one);
        }
        Ok(())
    }
}

impl<T: Json> Json for Option<Box<T>> {
    fn blank() -> Self {
        None
    }

    fn write(&self) -> Value {
        self.as_deref().map_or(Value::Null, Json::write)
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        if v.is_null() {
            *self = None;
            return Ok(());
        }
        let mut one = T::blank();
        one.read(v, at)?;
        *self = Some(Box::new(one));
        Ok(())
    }
}

/// A pointer at a member of the record holding it, as `field` or `field[index]`.
impl<T> Json for Member<T> {
    fn blank() -> Self {
        Member::default()
    }

    fn write(&self) -> Value {
        Value::String(match self.index {
            Some(i) => format!("{}[{i}]", self.field),
            None => self.field.clone(),
        })
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        let text = v.as_str().ok_or_else(|| at.fault("is not a member name"))?;
        let (field, index) =
            match text.strip_suffix(']').and_then(|t| t.split_once('[')) {
                Some((field, index)) => (
                    field,
                    Some(index.parse::<usize>().map_err(|_| {
                        at.fault("names a member by an index that is not a number")
                    })?),
                ),
                None => (text, None),
            };
        *self = Member::to(field);
        self.index = index;
        Ok(())
    }
}

/// A pointer at the entry a dictionary keys by this hash, or null.
impl<T> Json for Keyed<T> {
    fn blank() -> Self {
        Keyed::default()
    }

    fn write(&self) -> Value {
        self.key.map_or(Value::Null, hash_text)
    }

    fn read(&mut self, v: &Value, at: &mut Trail) -> Result<(), Fault> {
        *self = if v.is_null() {
            Keyed::default()
        } else {
            Keyed::to(hash_from(v).ok_or_else(|| at.fault("is not a hash or null"))?)
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn through<T: Json>(value: &T) -> T {
        let mut back = T::blank();
        back.read(&value.write(), &mut Trail::default())
            .expect("what is written reads back");
        back
    }

    #[test]
    fn a_float_is_written_as_the_shortest_decimal_and_one_no_decimal_holds_is_written_as_its_bits()
    {
        for x in [
            0.1f32,
            1.0,
            -0.0,
            3.4028235e38,
            1e-7,
            123456.79,
            f32::MIN_POSITIVE,
        ] {
            assert_eq!(through(&x).to_bits(), x.to_bits(), "{x:?}");
        }
        assert_eq!(0.1f32.write().to_string(), "0.1");
        let nan = f32::from_bits(0x7fc0_1234);
        assert_eq!(through(&nan).to_bits(), nan.to_bits());
        assert_eq!(nan.write(), Value::String("0x7fc01234".into()));
        assert_eq!(through(&f32::INFINITY).to_bits(), f32::INFINITY.to_bits());
    }

    #[test]
    fn a_vector_is_one_text_and_reads_back_lane_for_lane() {
        let v = Vec4V {
            x: 1.0,
            y: -0.5,
            z: 0.1,
            w: f32::from_bits(0x3f80_0001),
            ..Default::default()
        };
        assert_eq!(v.write().as_str().unwrap().split(' ').count(), 4);
        assert_eq!(through(&v), v);
        let mut bad = Vec4V::default();
        assert!(bad
            .read(&Value::String("1 2 3".into()), &mut Trail::default())
            .is_err());
    }

    #[test]
    fn a_number_out_of_range_or_of_the_wrong_shape_is_a_fault_that_names_its_place() {
        let mut trail = Trail::default();
        let mut n = 0u8;
        let fault = trail
            .field("effect_rules", |t| {
                t.index(2, |t| {
                    t.field("draw_list_id", |t| n.read(&Value::from(300), t))
                })
            })
            .unwrap_err();
        assert_eq!(fault.at, "effect_rules[2].draw_list_id");
        assert!(fault.why.contains("300"), "{fault}");
        let mut s = String::new();
        assert!(s.read(&Value::from(1), &mut Trail::default()).is_err());
    }

    #[test]
    fn a_hash_a_flag_a_name_and_a_member_read_back_as_they_were_written() {
        let h = Hash32::from_raw(0xdead_beef);
        assert_eq!(h.write(), Value::String("0xdeadbeef".into()));
        assert_eq!(through(&h), h);
        assert_eq!(through(&Bool8::new(true)), Bool8::new(true));
        let mut name = [0u8; 64];
        name[..5].copy_from_slice(b"alpha");
        assert_eq!(through(&name), name);
        let mut m = Member::<u8>::to("position_kfp");
        m.index = Some(3);
        assert_eq!(m.write(), Value::String("position_kfp[3]".into()));
        assert_eq!(through(&m), m);
    }
}
