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
    /// The list of all entities available: first every entity declared in the main
    /// `scripts/entities.xml`, then every extension's own `Entities/ClientServerEntities`
    /// appended after (extension directories in alphabetical order, declaration order
    /// within each extension), continuing the same id counter -- confirmed live, see
    /// [`Entity::from_extension`].
    pub entities: Vec<Entity>,
    /// The list of all "static" extension components available, in a stable order
    /// (extension directories in alphabetical order, then declaration order within
    /// each extension's `StaticComponents`). See [`Component`] for why order matters.
    pub static_components: Vec<Component>,
    /// The list of all "dynamic" extension components available (`DynamicComponents`),
    /// same stable order as [`Model::static_components`]. Unlike static components,
    /// these are NOT folded into any entity's method table (see [`Component`]'s doc
    /// comment) -- they still get their own generated struct/codec (some carry real
    /// properties/methods, e.g. `battle_royale`'s `Radar`), just not composed anywhere.
    pub dynamic_components: Vec<Component>,
}

/// A component from a WoT extension package (`res/<ext>/extension.xml`'s `Components`).
/// "Static" components (`StaticComponents`) have their methods/properties folded into
/// the method table of every entity they target (its `<ofEntity>` list) -- e.g.
/// `la_pinger`'s `LaPingerComponent` folds into `Account`. Confirmed live (see
/// `re-work/HANGAR_LOADING.md`) that these are appended *after* the entity's own
/// interface-derived, size-sorted method table, keeping their own relative order
/// (component order, then declared method order) rather than being merged back
/// into that sort -- this is why existing exposed ids never shift when an
/// unrelated extension is added or removed.
///
/// "Dynamic" components (`DynamicComponents`, [`Model::dynamic_components`]) are
/// attached to specific entity *instances* at runtime (e.g. only during a specific
/// battle mode), not baked into every instance's static method table -- and, unlike
/// static components, have no confirmed stable exposed-id assignment at all (which
/// instances get which dynamic components, and in what order, isn't known), so their
/// methods/properties are deliberately never folded into any entity's method table --
/// only their own standalone struct/codec is generated.
#[derive(Debug)]
pub struct Component {
    /// The component's name (e.g. `LaPingerComponent`), also used as its
    /// `Interface::name`.
    pub name: String,
    /// The entities this component's methods/properties fold into (`<ofEntity>`).
    pub of_entities: Vec<String>,
    /// The component's own properties/methods, parsed the same way as a regular
    /// interface.
    pub interface: Interface,
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
    /// `None` for an entity declared in the main `scripts/entities.xml`. `Some(ext_name)`
    /// for one declared in an extension's own `Entities/ClientServerEntities` (e.g.
    /// `story_mode`'s `SPGZone`) -- for these, `id` continues the main list's numbering
    /// (extension-alphabetical, then declaration order). CONFIRMED live (2026-08-29): a
    /// Frida-based scan of a running client's `BW::EntityDescriptionMap` vector
    /// (`re-work/frida/dump_entity_types.js`, see doc/ENTITY.md) read every entity's true
    /// index directly out of process memory, and every one of the 10 extension entities
    /// that existed at the time matched the id this rule already assigned it.
    pub from_extension: Option<String>,
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
