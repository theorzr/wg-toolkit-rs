//! Represent runtime script values.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
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

/// A python data value.
#[derive(Debug, Clone, PartialEq)]
pub struct PythonValue {
    inner: serde_pickle::Value,
}

impl PythonValue {

    pub fn new(inner: serde_pickle::Value) -> Self {
        Self { inner }
    }

}

impl Deref for PythonValue {
    type Target = serde_pickle::Value;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for PythonValue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
