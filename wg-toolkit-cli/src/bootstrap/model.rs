use std::fmt::Debug;
use std::sync::Arc;

use indexmap::IndexMap;


/// Type system, containing all named types.
#[derive(Debug, Default)]
pub struct TySystem {
    types: IndexMap<Box<str>, Ty>,
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
        }.into_boxed_str();

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

                let name = name.to_string().into_boxed_str();
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
    inner: Arc<(Box<str>, TyKind)>,
}

impl Ty {

    #[inline]
    fn new(name: Box<str>, kind: TyKind) -> Self {
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    pub default: Option<TyDefault>,
}

#[derive(Debug, PartialEq)]
pub struct TySeq {
    pub ty: Ty,
    pub size: Option<u32>,
}

/// Defines the default value for every type category.
#[derive(Debug, PartialEq)]
#[allow(unused)]  // Not used for generation
pub enum TyDefault {
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
    Unsupported,
}

/// Represent the a full model of resources.
#[derive(Debug, Default)]
pub struct Model {
    /// The game version of the parsed model.
    pub version: String,
    /// The type system in this model where all types are defined.
    pub tys: TySystem,
    /// The list of all interfaces available.
    pub interfaces: Vec<Interface>,
    /// The list of all entities available.
    pub entities: Vec<Entity>,
}

/// Ref: https://github.com/v2v3v4/BigWorld-Engine-14.4.1/blob/main/programming/bigworld/lib/entitydef/entity_description.cpp
#[derive(Debug)]
pub struct Entity {
    /// The actual storage for the entity, this has the same properties as an interface.
    pub interface: Interface,
    /// An optional parent entity to import all properties from.
    #[allow(unused)]  // Not used for generation
    pub parent: Option<String>,
    /// The index for network protocol.
    pub id: usize,
}

/// Ref: https://github.com/v2v3v4/BigWorld-Engine-14.4.1/blob/main/programming/bigworld/lib/entitydef/entity_description.cpp
#[derive(Debug)]
pub struct Interface {
    pub name: String,
    pub implements: Vec<String>,
    pub properties: Vec<Property>,
    pub temp_properties: Vec<String>,
    pub client_methods: Vec<Method>,
    pub base_methods: Vec<Method>,
    pub cell_methods: Vec<Method>,
}

#[derive(Debug)]
pub struct Method {
    pub name: String,
    /// True if this method is exposed to all clients, note that client methods have this
    /// force enabled. This cannot be parsed for base methods, and is possible for cell
    /// methods.
    pub exposed_to_all_clients: bool,
    /// True if the method is exposed to own client, this is available for base and cell
    /// methods only.
    pub exposed_to_own_client: bool,
    pub variable_header_size: VariableHeaderSize,
    pub args: Vec<Arg>,
}

#[derive(Debug)]
pub struct Arg {
    pub ty: Ty,
}

/// Ref: https://github.com/v2v3v4/BigWorld-Engine-14.4.1/blob/main/programming/bigworld/lib/entitydef/data_description.cpp
#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub ty: Ty,
    #[allow(unused)]  // Not used for generation
    pub persistent: bool,
    #[allow(unused)]  // Not used for generation
    pub identifier: bool,
    #[allow(unused)]  // Not used for generation
    pub indexed: bool,
    #[allow(unused)]  // Not used for generation
    pub database_len: Option<u32>,
    #[allow(unused)]  // Not used for generation
    pub default: Option<String>,
    pub flags: PropertyFlags,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PropertyFlags {
    None,
    Base,
    BaseAndClient,
    OwnClient,
    CellPrivate,
    CellPublic,
    AllClients,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VariableHeaderSize {
    Variable8,
    Variable16,
    Variable24,
    Variable32,
}
