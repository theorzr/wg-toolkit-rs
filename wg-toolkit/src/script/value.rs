//! Represent runtime script values.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::fmt;

use glam::{Vec2, Vec3, Vec4};


#[derive(Clone, PartialEq)]
pub enum Value {
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    Vector2(Vec2),
    Vector3(Vec3),
    Vector4(Vec4),
    String(StringValue),
    Python(PythonValue),
    Mailbox,
    Dict(BTreeMap<Arc<str>, Value>),
    Seq(Vec<Value>),
    /// A `FIXED_DICT` value whose type declares `AllowNone` (see [`crate::script::TyDict`]),
    /// read off the wire as Python `None` instead of the dict's properties.
    None,
}

/// Prints each value like a regular Rust value rather than as an enum variant: numeric
/// values get a Rust-literal-style type suffix (e.g. `5i32`, `1.5f32`), dicts print as a
/// struct literal and sequences as a plain list.
impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int8(v) => write!(f, "{v:?}i8"),
            Value::Int16(v) => write!(f, "{v:?}i16"),
            Value::Int32(v) => write!(f, "{v:?}i32"),
            Value::Int64(v) => write!(f, "{v:?}i64"),
            Value::UInt8(v) => write!(f, "{v:?}u8"),
            Value::UInt16(v) => write!(f, "{v:?}u16"),
            Value::UInt32(v) => write!(f, "{v:?}u32"),
            Value::UInt64(v) => write!(f, "{v:?}u64"),
            Value::Float32(v) => write!(f, "{v:?}f32"),
            Value::Float64(v) => write!(f, "{v:?}f64"),
            Value::Vector2(v) => fmt::Debug::fmt(v, f),
            Value::Vector3(v) => fmt::Debug::fmt(v, f),
            Value::Vector4(v) => fmt::Debug::fmt(v, f),
            Value::String(v) => fmt::Debug::fmt(v, f),
            Value::Python(v) => fmt::Debug::fmt(v, f),
            Value::Mailbox => f.write_str("Mailbox"),
            Value::Dict(map) => {
                let mut d = f.debug_struct("Dict");
                for (k, v) in map {
                    d.field(k, v);
                }
                d.finish()
            }
            Value::Seq(list) => f.debug_list().entries(list).finish(),
            Value::None => f.write_str("None"),
        }
    }
}

/// The string data type used by default for all STRING types.
#[derive(Debug, Clone, PartialEq)]
pub enum StringValue {
    String(String),
    Python(serde_pickle::Value),
    Raw(Vec<u8>),
}

/// A python data value -- either successfully decoded pickle content, or, if this
/// specific blob failed to parse (e.g. an as-yet-unsupported pickle opcode, or a bundle
/// already desynced upstream), the raw bytes instead of hard-failing the whole read.
/// Mirrors how [`StringValue`] already handles an ambiguous encoding: the field's
/// declared length is always fully consumed either way (see `PythonValue`'s
/// [`crate::net::codec`] impl), so one unparseable blob doesn't have to desync
/// everything read after it.
#[derive(Debug, Clone, PartialEq)]
pub enum PythonValue {
    Decoded(serde_pickle::Value),
    Raw(Vec<u8>),
}
