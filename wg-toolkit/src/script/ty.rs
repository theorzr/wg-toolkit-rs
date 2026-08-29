//! Type system for the scripts.

use std::collections::BTreeMap;
use std::fmt::{self, Debug};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use glam::{Vec2, Vec3, Vec4};
use indexmap::IndexMap;


/// Type system, containing all named types.
#[derive(Debug, Default)]
pub struct TySystem {
    types: IndexMap<Arc<str>, Ty>,
    anonymous_count: usize,
}

impl TySystem {

    pub fn register(&mut self, name: Option<String>, kind: TyKind) -> Ty {
        
        if let Some(name) = name.as_deref() {
            if let Some(ty) = self.find(name)
            && ty.kind() != &kind {
                panic!("type already exists: {name}\ncurrent: {:#?}\nnew: {:#?}", ty.kind(), kind);
            }
        }

        let name = match name {
            Some(name) => name,
            None => {
                let name = format!("ANON{}", self.anonymous_count);
                self.anonymous_count += 1;
                name
            }
        };
        let name: Arc<str> = Arc::from(name);

        let ty = Ty::new(name.clone(), kind);
        self.types.insert(name, ty.clone());
        ty

    }

    /// Find a named type into the type system, returning a cloned handle.
    pub fn find(&mut self, name: &str) -> Option<Ty> {
        match self.types.get(name) {
            Some(ty) => return Some(ty.clone()),
            None => {
                
                // If the name is a builtin that is missing then we return it.
                let new_kind = match name {
                    "INT8" =>       TyKind::Int8,
                    "INT16" =>      TyKind::Int16,
                    "INT32" =>      TyKind::Int32,
                    "INT64" =>      TyKind::Int64,
                    "UINT8" =>      TyKind::UInt8,
                    "UINT16" =>     TyKind::UInt16,
                    "UINT32" =>     TyKind::UInt32,
                    "UINT64" =>     TyKind::UInt64,
                    "FLOAT" =>      TyKind::Float32,
                    "FLOAT32" =>    TyKind::Float32,
                    "FLOAT64" =>    TyKind::Float64,
                    "VECTOR2" =>    TyKind::Vector2,
                    "VECTOR3" =>    TyKind::Vector3,
                    "VECTOR4" =>    TyKind::Vector4,
                    "STRING" =>     TyKind::String,
                    "PYTHON" =>     TyKind::Python,
                    "MAILBOX" =>    TyKind::Mailbox,
                    _ => return None
                };

                let name: Arc<str> = Arc::from(name);
                let ty = Ty::new(name.clone(), new_kind);
                self.types.insert(name, ty.clone());
                Some(ty)

            }
        }
    }

    pub fn count(&self) -> usize {
        self.types.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &'_ Ty> + '_ {
        self.types.iter().map(|(_, ty)| ty)
    }

}

#[derive(Clone, PartialEq)]
pub struct Ty {
    inner: Arc<(Arc<str>, TyKind)>,
}

impl Ty {

    #[inline]
    fn new(name: Arc<str>, kind: TyKind) -> Self {
        Self { inner: Arc::new((name, kind)) }
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.inner.0
    }

    #[inline]
    pub fn kind(&self) -> &TyKind {
        &self.inner.1
    }

}

impl Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Ty")
            .field(&self.name())
            .field(self.kind())
            .finish()
    }
}

/// Define the actual kind of a type, maybe a "meta type" containing other types.
#[derive(Debug, PartialEq)]
pub enum TyKind {
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    Vector2,
    Vector3,
    Vector4,
    String,   // This type is actually used for any string of bytes, sometimes for Python.
    Python,
    Mailbox,
    Alias(Ty),
    Dict(TyDict),
    Array(TySeq),
    Tuple(TySeq),
}

#[derive(Debug, PartialEq, Default)]
pub struct TyDict {
    pub properties: Vec<TyDictProp>,
}

#[derive(Debug, PartialEq)]
pub struct TyDictProp {
    pub name: String,
    pub ty: Ty,
    #[allow(unused)]  // Not used for generation
    pub default: Option<Value>,
}

#[derive(Debug, PartialEq)]
pub struct TySeq {
    pub ty: Ty,
    pub size: Option<u32>,
}

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

impl Value {

    /// Check if this value is a valid instance of the given type.
    pub fn is_instance(&self, ty: &Ty) -> bool {
        match (self, ty.kind()) {
            (_, TyKind::Alias(inner)) => self.is_instance(inner),
            (Value::Int8(_), TyKind::Int8) => true,
            (Value::Int16(_), TyKind::Int16) => true,
            (Value::Int32(_), TyKind::Int32) => true,
            (Value::Int64(_), TyKind::Int64) => true,
            (Value::UInt8(_), TyKind::UInt8) => true,
            (Value::UInt16(_), TyKind::UInt16) => true,
            (Value::UInt32(_), TyKind::UInt32) => true,
            (Value::UInt64(_), TyKind::UInt64) => true,
            (Value::Float32(_), TyKind::Float32) => true,
            (Value::Float64(_), TyKind::Float64) => true,
            (Value::Vector2(_), TyKind::Vector2) => true,
            (Value::Vector3(_), TyKind::Vector3) => true,
            (Value::Vector4(_), TyKind::Vector4) => true,
            (Value::String(_), TyKind::String) => true,
            (Value::Python(_), TyKind::Python) => true,
            (Value::Mailbox, TyKind::Mailbox) => true,
            (Value::Dict(map), TyKind::Dict(dict)) => {
                map.len() == dict.properties.len()
                    && dict.properties.iter().all(|prop| {
                        map.get(&prop.name).is_some_and(|value| value.is_instance(&prop.ty))
                    })
            }
            (Value::Seq(list), TyKind::Array(seq) | TyKind::Tuple(seq)) => {
                seq.size.is_none_or(|size| list.len() == size as usize)
                    && list.iter().all(|value| value.is_instance(&seq.ty))
            }
            _ => false,
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
