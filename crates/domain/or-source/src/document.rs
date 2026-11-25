//! The members of a JSON sidecar read by name, each fault with its place in the document.

use serde_json::{Map, Value};

pub(crate) use crate::particles::text::{hash_from, hash_text, Fault, Json, Trail};

/// The member `name` of `o`, or the fault that the object states none.
pub(crate) fn stated<'v>(
    o: &'v Map<String, Value>,
    name: &str,
    at: &Trail,
) -> Result<&'v Value, Fault> {
    o.get(name)
        .ok_or_else(|| at.fault(format!("states no `{name}`")))
}

/// The member `name` of `o` read as a `T`.
pub(crate) fn member<T: Json>(
    o: &Map<String, Value>,
    name: &str,
    at: &mut Trail,
) -> Result<T, Fault> {
    let v = stated(o, name, at)?;
    let mut out = T::blank();
    at.field(name, |at| out.read(v, at))?;
    Ok(out)
}

/// The member `name` of `o` as a 32-bit word, written `"0x…"`.
pub(crate) fn word(o: &Map<String, Value>, name: &str, at: &mut Trail) -> Result<u32, Fault> {
    let v = stated(o, name, at)?;
    at.field(name, |at| {
        hash_from(v).ok_or_else(|| at.fault("is not a 32-bit word"))
    })
}

/// The member `name` of `o` as a list.
pub(crate) fn list<'v>(
    o: &'v Map<String, Value>,
    name: &str,
    at: &mut Trail,
) -> Result<&'v Vec<Value>, Fault> {
    let v = stated(o, name, at)?;
    at.field(name, |at| {
        v.as_array().ok_or_else(|| at.fault("is not a list"))
    })
}

/// The document `bytes` hold, as the object at its root.
pub(crate) fn root(bytes: &[u8]) -> Result<Map<String, Value>, Fault> {
    let at = Trail::default();
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(o)) => Ok(o),
        Ok(_) => Err(at.fault("is not an object")),
        Err(e) => Err(at.fault(format!("is not JSON: {e}"))),
    }
}
