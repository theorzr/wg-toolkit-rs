//! Type system for the scripts.

use std::fmt::{self, Debug};
use std::sync::Arc;

use indexmap::IndexMap;

use super::value::Value;


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

    /// Check if this value is a valid instance of the given type.
    pub fn is_instance(&self, value: &Value) -> bool {
        match (value, self.kind()) {
            (_, TyKind::Alias(inner)) => inner.is_instance(value),
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
            (Value::None, TyKind::Dict(dict)) => dict.allow_none,
            (Value::Dict(map), TyKind::Dict(dict)) => {
                map.len() == dict.properties.len()
                    && dict.properties.iter().all(|prop| {
                        map.get(&prop.name).is_some_and(|value| prop.ty.is_instance(value))
                    })
            }
            (Value::Seq(list), TyKind::Array(seq) | TyKind::Tuple(seq)) => {
                seq.size.is_none_or(|size| list.len() == size as usize)
                    && list.iter().all(|value| seq.ty.is_instance(value))
            }
            _ => false,
        }
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
    /// BigWorld's `FIXED_DICT` `AllowNone` flag: the value is preceded on the wire by a
    /// single discriminator byte (`0` = the whole dict is Python `None`, no property
    /// bytes follow; `1` = present, properties follow as normal) -- confirmed live
    /// (WoT v2.3.1.3): `BATTLE_GOODIE_RECORD`/`GOODIE_RESOURCE`/`GOODIE_STATE_INFO`
    /// (used by `Avatar::goodiesSnapshot`) all declare this, and decoding them without
    /// consuming that leading byte desyncs every nested dict after the first one,
    /// eventually overrunning the enclosing element's declared length. See
    /// `FixedDictDataType`'s `fromSourceToStream`/`fromStreamToSink` in the real engine
    /// source (`entitydef/data_types/fixed_dict_data_type.cpp`).
    pub allow_none: bool,
}

#[derive(Debug, PartialEq)]
pub struct TyDictProp {
    pub name: Arc<str>,
    pub ty: Ty,
    #[allow(unused)]  // Not used for generation
    pub default: Option<Value>,
}

#[derive(Debug, PartialEq)]
pub struct TySeq {
    pub ty: Ty,
    pub size: Option<u32>,
}
