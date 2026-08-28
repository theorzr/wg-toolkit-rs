use std::io::{self, Read, Write};
use std::any::Any;
use std::fmt;

use crate::net::element::ElementLength;
use crate::net::bundle::ElementReader;
use crate::net::codec::Codec;


/// Abstract type representing an entity type.
pub trait Entity: Codec<()> {
    /// This entity type's numeric type id, as declared in `entities.xml`.
    const TYPE_ID: u16;
    /// The client method enum type associated to this entity.
    type ClientMethod: AnyMethod;
    /// The base method enum type associated to this entity.
    type BaseMethod: AnyMethod;
    /// The cell method enum type associated to this entity.
    type CellMethod: AnyMethod;
    /// The client-visible property enum type associated to this entity: one variant per
    /// property exposed to the client (base-and-client, own-client or all-clients --
    /// same filter BigWorld's `DataDescription::isClientServerData()` uses), whether the
    /// update actually originates from the entity's base or cell slice. Exposed ids are
    /// assigned the exact same way as [`AnyMethod`]'s (stable sort: fixed-size first
    /// ascending, then variable-size ascending, static components appended afterward
    /// keeping their own order) -- confirmed against the same BigWorld engine source
    /// (`entitydef/entity_description.cpp`'s `allocateClientServerFullIndexes` /
    /// `ClientServerPropertiesSortHelper`) that already justified the method ordering.
    type Property: AnyProperty;
}

/// A runtime-typed entity value: one variant per entity type declared in
/// `entities.xml`, tagged by its wire `type_id`. Implemented exactly once per game
/// (unlike [`AnyMethod`], implemented once per entity per direction) by one generated
/// sum type covering every entity type -- this is what lets code that only knows an
/// `entity_type_id` at runtime (e.g. from `CreateBasePlayerHeader`) recover a
/// concretely-typed entity to decode/dispatch on.
pub trait AnyEntity: Sized {

    /// Return this entity's type id, without writing anything.
    fn type_id(&self) -> u16;

    /// Return the total number of known entity types.
    fn count() -> u16;

    /// Encode this entity's data into the given writer.
    fn write(&self, write: &mut dyn Write) -> io::Result<()>;

    /// Decode an entity of the given type id from the given reader.
    fn read(read: &mut dyn Read, type_id: u16) -> io::Result<Self>;

    /// Decode a base-method call targeting this entity directly from the bundle's
    /// element reader. This dispatches on `self`'s own variant to recover the concrete
    /// entity type, then decodes via that type's `BaseMethod` straight off the reader
    /// (no intermediate raw-byte copy). The concrete decoded type varies per entity, so
    /// it's erased into an [`AnyMethodValue`].
    ///
    /// Returns `Ok(None)` (not an error) when the framing decoded fine but the exposed
    /// id itself isn't in `BaseMethod`'s generated table (e.g. a mismatch between the
    /// generated tables and what's actually sent) -- the bundle reader has still safely
    /// advanced past this element in that case. An `Err` means the read itself failed
    /// (e.g. truncated data for a *recognized* id) and the bundle reader was rolled back
    /// to before this element -- callers must not keep reading past it, or they'll loop
    /// on the same unconsumed element forever.
    fn read_base_method(&self, reader: ElementReader<'_, '_>) -> io::Result<Option<AnyMethodValue>>;

    /// Same as [`AnyEntity::read_base_method`], but for a `CellMethod` call. The same
    /// entity id (and so the same stored `Self` variant) is shared across an entity's
    /// base and cell slices, so this dispatches the same way, just against `CellMethod`
    /// instead of `BaseMethod`.
    fn read_cell_method(&self, reader: ElementReader<'_, '_>) -> io::Result<Option<AnyMethodValue>>;

    /// Same as [`AnyEntity::read_base_method`], but for a `ClientMethod` call (sent
    /// server -> client). `base::App` never needs this (it only ever sends client
    /// methods, never decodes them), but a generic wire observer -- e.g. a debugging
    /// proxy sniffing both directions of real traffic without knowing any concrete
    /// entity type statically -- does.
    fn read_client_method(&self, reader: ElementReader<'_, '_>) -> io::Result<Option<AnyMethodValue>>;

    /// Decode a property update targeting this entity from the bundle's element reader
    /// (server -> client, either the entity's base or cell slice, both share one flat
    /// client-visible property list -- see [`Entity::Property`]). Unlike
    /// [`AnyEntity::read_base_method`], there's no `Ok(None)` case: an unrecognized
    /// exposed id (e.g. one belonging to a *dynamic* component -- see
    /// [`EntityPropertyInner`](crate::net::app::client::element::EntityPropertyInner)'s
    /// doc comment for why) always surfaces as `Err` here, since this project has no
    /// confirmed safe framing to fall back to for it. Callers must not keep reading past
    /// an `Err`, same as for a method read failure.
    fn read_client_property(&self, reader: ElementReader<'_, '_>) -> io::Result<AnyPropertyValue>;

}

/// A type-erased, decoded method call value returned by [`AnyEntity::read_base_method`]
/// (and siblings). Carries both the concrete value, so a caller who knows the concrete
/// `M` can recover it (see `base::App`'s `BaseMethodEvent::extract`), and a `Debug`
/// rendering of it captured while `M` was still known -- so a caller with no static type
/// info at all (e.g. a generic wire-trace logger walking every entity type) can still log
/// it meaningfully, unlike a bare `Box<dyn Any>` whose `Debug` impl is just a placeholder.
/// `Send` (via `AnyMethod`'s supertrait) so events carrying one can still cross a thread
/// boundary, e.g. `base::App::poll()`'s events being handled on a dedicated thread.
pub struct AnyMethodValue {
    value: Box<dyn Any + Send>,
    debug: String,
}

impl AnyMethodValue {

    /// Erase a concrete method value. Public only so that the generated `__enum_entities!`
    /// impls (invoked from downstream crates) can call it -- not meant to be constructed
    /// directly by users of this library.
    pub fn new<M: AnyMethod + 'static>(value: M) -> Self {
        let debug = format!("{value:?}");
        Self { value: Box::new(value), debug }
    }

    /// Recover the concrete method value, if `M` matches the type that was actually
    /// decoded.
    pub fn downcast<M: AnyMethod + 'static>(&self) -> Option<&M> {
        self.value.downcast_ref()
    }

}

impl fmt::Debug for AnyMethodValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.debug)
    }
}

/// A type-erased, decoded property value returned by [`AnyEntity::read_client_property`].
/// Same rationale as [`AnyMethodValue`] -- see its doc comment.
pub struct AnyPropertyValue {
    value: Box<dyn Any + Send>,
    debug: String,
}

impl AnyPropertyValue {

    /// Erase a concrete property value. Public only so that the generated
    /// `__enum_entities!` impls (invoked from downstream crates) can call it -- not
    /// meant to be constructed directly by users of this library.
    pub fn new<P: AnyProperty + 'static>(value: P) -> Self {
        let debug = format!("{value:?}");
        Self { value: Box::new(value), debug }
    }

    /// Recover the concrete property value, if `P` matches the type that was actually
    /// decoded.
    pub fn downcast<P: AnyProperty + 'static>(&self) -> Option<&P> {
        self.value.downcast_ref()
    }

}

impl fmt::Debug for AnyPropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.debug)
    }
}

/// Abstract type representing a method for an entity.
pub trait AnyMethod: Sized + fmt::Debug + Send {

    /// Return the exposed id of this specific method value, without writing anything.
    /// Callers that need to know the id ahead of writing (e.g. to decide whether a
    /// sub-id byte must be written before the method itself, see [`ElementIdRange`])
    /// can use this instead of calling [`AnyMethod::write`] and computing it themselves.
    ///
    /// [`ElementIdRange`]: crate::net::element::ElementIdRange
    fn exposed_id(&self) -> u16;

    /// Return the total number of exposed methods for this type, used together with
    /// [`ElementIdRange`] to translate between exposed ids and wire ids (and sub-ids).
    ///
    /// [`ElementIdRange`]: crate::net::element::ElementIdRange
    fn count() -> u16;

    /// Return the preferred encoding length of this method, when sub message id is used
    /// this is just ignored.
    fn write_length(&self) -> ElementLength;

    /// Encode the method call into the given writer.
    fn write(&self, write: &mut dyn Write) -> io::Result<()>;

    /// Return the decode length for the given exposed method id, or an error if this
    /// exposed id isn't known (e.g. a mismatch between the generated method tables and
    /// what the live game actually sends, rather than something to ever crash on).
    fn read_length(exposed_id: u16) -> io::Result<ElementLength>;

    /// Decode the given method from the given reader and its exposed id.
    fn read(read: &mut dyn Read, exposed_id: u16) -> io::Result<Self>;

}

/// Abstract type representing a client-visible property value for an entity. Same shape
/// as [`AnyMethod`] (a property update is framed and dispatched by exposed id exactly
/// the same way a method call is, just carrying one bare value instead of an args
/// struct) -- see [`Entity::Property`] for why the id assignment is the same too.
pub trait AnyProperty: Sized + fmt::Debug + Send {

    /// Return the exposed id of this specific property value, without writing anything.
    fn exposed_id(&self) -> u16;

    /// Return the total number of exposed client-visible properties for this type.
    fn count() -> u16;

    /// Return the preferred encoding length of this property, when a sub message id is
    /// used this is just ignored.
    fn write_length(&self) -> ElementLength;

    /// Encode the property value into the given writer.
    fn write(&self, write: &mut dyn Write) -> io::Result<()>;

    /// Return the decode length for the given exposed property id, or an error if this
    /// exposed id isn't known.
    fn read_length(exposed_id: u16) -> io::Result<ElementLength>;

    /// Decode the given property from the given reader and its exposed id.
    fn read(read: &mut dyn Read, exposed_id: u16) -> io::Result<Self>;

}

/// This macro can be used to generate an enumeration capable of encoding and decoding
/// an arbitrary number of methods, the enumeration implements the [`AnyMethod`] trait.
#[macro_export]
macro_rules! __enum_entity_methods {
    (__length; $length:literal) => { $crate::net::element::ElementLength::Fixed($length) };
    (__length; var8 ) => { $crate::net::element::ElementLength::Variable8 };
    (__length; var16 ) => { $crate::net::element::ElementLength::Variable16 };
    (__length; var24 ) => { $crate::net::element::ElementLength::Variable24 };
    (__length; var32 ) => { $crate::net::element::ElementLength::Variable32 };
    (
        $(
            $(#[$attr:meta])* 
            $enum_vis:vis enum $enum_name:ident {
                $( $method_name:ident ( $method_exposed_id:literal, $method_length:tt ) ),*
                $(,)?
            }
        )*
    ) => {
        $(
            $(#[$attr])* 
            $enum_vis enum $enum_name {
                $( $method_name ( $method_name ),)*
            }

            impl $crate::net::app::entity::AnyMethod for $enum_name {
                fn exposed_id(&self) -> u16 {
                    match self {
                        $( Self::$method_name (_) => $method_exposed_id, )*
                        _ => unreachable!()
                    }
                }
                fn count() -> u16 {
                    // Counts repetitions without relying on array element type inference,
                    // which breaks on an empty (zero-method) enum with `[$(...),*].len()`.
                    0u16 $(+ { $method_exposed_id; 1u16 })*
                }
                fn write_length(&self) -> $crate::net::element::ElementLength {
                    match self {
                        $( Self::$method_name (_) => $crate::__enum_entity_methods!(__length; $method_length), )*
                        _ => unreachable!()
                    }
                }
                fn write(&self, write: &mut dyn std::io::Write) -> std::io::Result<()> {
                    use $crate::net::codec::Codec;
                    match self {
                        $( Self::$method_name (m) => Codec::<()>::write(m, write, &()), )*
                        _ => unreachable!()
                    }
                }
                fn read_length(exposed_id: u16) -> std::io::Result<$crate::net::element::ElementLength> {
                    Ok(match exposed_id {
                        $( $method_exposed_id => $crate::__enum_entity_methods!(__length; $method_length), )*
                        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid method exposed id: 0x{exposed_id:02X}"))),
                    })
                }
                fn read(read: &mut dyn std::io::Read, exposed_id: u16) -> std::io::Result<Self> {
                    use $crate::net::codec::Codec;
                    Ok(match exposed_id {
                        $( $method_exposed_id => Self::$method_name(Codec::<()>::read(read, &())?), )*
                        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid method exposed id: 0x{exposed_id:02X}")))
                    })
                }
            }

            impl std::fmt::Debug for $enum_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                    match self {
                        $( Self::$method_name (m) => std::fmt::Debug::fmt(m, f), )*
                        _ => unreachable!()
                    }
                }
            }

            // Lets callers pass a concrete method value where the enum is expected
            // (e.g. `App::call_method`) without needing a match at the call site.
            $(
                impl std::convert::From<$method_name> for $enum_name {
                    fn from(method: $method_name) -> Self {
                        Self::$method_name(method)
                    }
                }
            )*
        )*
    };
}

/// This macro can be used to generate an enumeration capable of encoding and decoding
/// an arbitrary number of client-visible properties, the enumeration implements the
/// [`AnyProperty`] trait. Unlike [`__enum_entity_methods!`], each variant wraps the
/// property's own value type directly (a property update carries one bare value, not a
/// named-args struct), so the macro takes a Rust type per property instead of expecting
/// a separately pre-generated struct name.
#[macro_export]
macro_rules! __enum_entity_properties {
    (__length; $length:literal) => { $crate::net::element::ElementLength::Fixed($length) };
    (__length; var8 ) => { $crate::net::element::ElementLength::Variable8 };
    (__length; var16 ) => { $crate::net::element::ElementLength::Variable16 };
    (__length; var24 ) => { $crate::net::element::ElementLength::Variable24 };
    (__length; var32 ) => { $crate::net::element::ElementLength::Variable32 };
    (
        $(
            $(#[$attr:meta])*
            $enum_vis:vis enum $enum_name:ident {
                $( $prop_name:ident ( $prop_exposed_id:literal, $prop_length:tt, $prop_ty:ty ) ),*
                $(,)?
            }
        )*
    ) => {
        $(
            $(#[$attr])*
            $enum_vis enum $enum_name {
                $( $prop_name ( $prop_ty ),)*
            }

            impl $crate::net::app::entity::AnyProperty for $enum_name {
                fn exposed_id(&self) -> u16 {
                    match self {
                        $( Self::$prop_name (_) => $prop_exposed_id, )*
                        _ => unreachable!()
                    }
                }
                fn count() -> u16 {
                    // Counts repetitions without relying on array element type inference,
                    // which breaks on an empty (zero-property) enum with `[$(...),*].len()`.
                    0u16 $(+ { $prop_exposed_id; 1u16 })*
                }
                fn write_length(&self) -> $crate::net::element::ElementLength {
                    match self {
                        $( Self::$prop_name (_) => $crate::__enum_entity_properties!(__length; $prop_length), )*
                        _ => unreachable!()
                    }
                }
                fn write(&self, write: &mut dyn std::io::Write) -> std::io::Result<()> {
                    use $crate::net::codec::Codec;
                    match self {
                        $( Self::$prop_name (p) => Codec::<()>::write(p, write, &()), )*
                        _ => unreachable!()
                    }
                }
                fn read_length(exposed_id: u16) -> std::io::Result<$crate::net::element::ElementLength> {
                    Ok(match exposed_id {
                        $( $prop_exposed_id => $crate::__enum_entity_properties!(__length; $prop_length), )*
                        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid property exposed id: 0x{exposed_id:02X}"))),
                    })
                }
                fn read(read: &mut dyn std::io::Read, exposed_id: u16) -> std::io::Result<Self> {
                    use $crate::net::codec::Codec;
                    Ok(match exposed_id {
                        $( $prop_exposed_id => Self::$prop_name(Codec::<()>::read(read, &())?), )*
                        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid property exposed id: 0x{exposed_id:02X}")))
                    })
                }
            }

            impl std::fmt::Debug for $enum_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                    match self {
                        $( Self::$prop_name (p) => f.debug_tuple(stringify!($prop_name)).field(p).finish(), )*
                        _ => unreachable!()
                    }
                }
            }
        )*
    };
}

/// This macro can be used to generate a sum type covering every entity type in the
/// game, the generated enum implements [`AnyEntity`], one variant per entity (each
/// wrapping that entity's own struct, of the same name), tagged by that entity's
/// [`Entity::TYPE_ID`]. Unlike [`__enum_entity_methods!`], this is meant to be invoked
/// exactly once per game (there's only one such sum type), covering every entity
/// declared, not once per entity.
#[macro_export]
macro_rules! __enum_entities {
    (
        $(
            $(#[$attr:meta])*
            $enum_vis:vis enum $enum_name:ident {
                $( $entity_name:ident ( $entity_type_id:literal ) ),*
                $(,)?
            }
        )*
    ) => {
        $(
            $(#[$attr])*
            $enum_vis enum $enum_name {
                $( $entity_name ( $entity_name ), )*
            }

            impl $crate::net::app::entity::AnyEntity for $enum_name {

                fn type_id(&self) -> u16 {
                    match self {
                        $( Self::$entity_name (_) => $entity_type_id, )*
                        _ => unreachable!()
                    }
                }

                fn count() -> u16 {
                    // Counts repetitions without relying on array element type inference,
                    // which breaks on an empty (zero-entity) enum with `[$(...),*].len()`.
                    0u16 $(+ { $entity_type_id; 1u16 })*
                }

                fn write(&self, write: &mut dyn std::io::Write) -> std::io::Result<()> {
                    use $crate::net::codec::Codec;
                    match self {
                        $( Self::$entity_name (e) => Codec::<()>::write(e, write, &()), )*
                        _ => unreachable!()
                    }
                }

                fn read(read: &mut dyn std::io::Read, type_id: u16) -> std::io::Result<Self> {
                    use $crate::net::codec::Codec;
                    Ok(match type_id {
                        $( $entity_type_id => Self::$entity_name(Codec::<()>::read(read, &())?), )*
                        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid entity type id: 0x{type_id:02X}"))),
                    })
                }

                fn read_base_method(&self, reader: $crate::net::bundle::ElementReader<'_, '_>) -> std::io::Result<Option<$crate::net::app::entity::AnyMethodValue>> {
                    use $crate::net::app::base::element::{BaseEntityMethod, BaseEntityMethodInner};
                    use $crate::net::app::entity::AnyMethodValue;
                    match self {
                        $(
                            Self::$entity_name (_) => {
                                type M = <$entity_name as $crate::net::app::entity::Entity>::BaseMethod;
                                Ok(match reader.read::<BaseEntityMethod<M>, ()>(&())?.element.inner {
                                    BaseEntityMethodInner::Known(m) => Some(AnyMethodValue::new(m)),
                                    BaseEntityMethodInner::Unknown { .. } => None,
                                })
                            }
                        )*
                        _ => unreachable!()
                    }
                }

                fn read_cell_method(&self, reader: $crate::net::bundle::ElementReader<'_, '_>) -> std::io::Result<Option<$crate::net::app::entity::AnyMethodValue>> {
                    use $crate::net::app::base::element::{CellEntityMethod, CellEntityMethodInner};
                    use $crate::net::app::entity::AnyMethodValue;
                    match self {
                        $(
                            Self::$entity_name (_) => {
                                type M = <$entity_name as $crate::net::app::entity::Entity>::CellMethod;
                                Ok(match reader.read::<CellEntityMethod<M>, ()>(&())?.element.inner {
                                    CellEntityMethodInner::Known(m) => Some(AnyMethodValue::new(m)),
                                    CellEntityMethodInner::Unknown { .. } => None,
                                })
                            }
                        )*
                        _ => unreachable!()
                    }
                }

                fn read_client_method(&self, reader: $crate::net::bundle::ElementReader<'_, '_>) -> std::io::Result<Option<$crate::net::app::entity::AnyMethodValue>> {
                    use $crate::net::app::client::element::{EntityMethod, EntityMethodInner};
                    use $crate::net::app::entity::AnyMethodValue;
                    match self {
                        $(
                            Self::$entity_name (_) => {
                                type M = <$entity_name as $crate::net::app::entity::Entity>::ClientMethod;
                                Ok(match reader.read::<EntityMethod<M>, ()>(&())?.element.inner {
                                    EntityMethodInner::Known(m) => Some(AnyMethodValue::new(m)),
                                    EntityMethodInner::Unknown { .. } => None,
                                })
                            }
                        )*
                        _ => unreachable!()
                    }
                }

                fn read_client_property(&self, reader: $crate::net::bundle::ElementReader<'_, '_>) -> std::io::Result<$crate::net::app::entity::AnyPropertyValue> {
                    use $crate::net::app::client::element::{EntityProperty, EntityPropertyInner};
                    use $crate::net::app::entity::AnyPropertyValue;
                    match self {
                        $(
                            Self::$entity_name (_) => {
                                type P = <$entity_name as $crate::net::app::entity::Entity>::Property;
                                let EntityPropertyInner::Known(p) = reader.read::<EntityProperty<P>, ()>(&())?.element.inner;
                                Ok(AnyPropertyValue::new(p))
                            }
                        )*
                        _ => unreachable!()
                    }
                }

            }

            impl std::fmt::Debug for $enum_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                    match self {
                        $( Self::$entity_name (e) => std::fmt::Debug::fmt(e, f), )*
                        _ => unreachable!()
                    }
                }
            }

            // Lets `base::App::create_base_player` accept a concrete entity and store
            // it as this sum type via `.into()`, without needing a match at the call
            // site.
            $(
                impl std::convert::From<$entity_name> for $enum_name {
                    fn from(entity: $entity_name) -> Self {
                        Self::$entity_name(entity)
                    }
                }
            )*
        )*
    };
}
