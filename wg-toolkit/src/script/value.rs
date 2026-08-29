//! Represent runtime script values.

use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

use glam::{Vec2, Vec3, Vec4};


#[derive(Debug, Clone, PartialEq)]
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
    Dict(BTreeMap<String, Value>),
    Seq(Vec<Value>),
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
