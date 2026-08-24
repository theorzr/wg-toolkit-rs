use std::io::{self, Read, Write};

use crate::net::element::ElementLength;
use crate::net::codec::Codec;


/// Abstract type representing an entity type.
pub trait Entity: Sized {

    /// The client method enum type associated to this entity.
    type ClientMethod: Method;
    /// The base method enum type associated to this entity.
    type BaseMethod: Method;
    /// The cell method enum type associated to this entity.
    type CellMethod: Method;

    fn write(&self, write: &mut dyn Write) -> io::Result<()>;

    fn read(read: &mut dyn Read) -> io::Result<Self>;

}

/// An alternative to implementing the [`Entity`] trait that only requires the type to
/// already implement the [`Codec`] trait.
pub trait SimpleEntity: Codec<()> {

    /// The client method enum type associated to this entity.
    type ClientMethod: Method;
    /// The base method enum type associated to this entity.
    type BaseMethod: Method;
    /// The cell method enum type associated to this entity.
    type CellMethod: Method;
    
}

impl<E: SimpleEntity> Entity for E {

    type ClientMethod = <E as SimpleEntity>::ClientMethod;
    type BaseMethod = <E as SimpleEntity>::BaseMethod;
    type CellMethod = <E as SimpleEntity>::CellMethod;

    #[inline]
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        Codec::write(self, write, &())
    }

    #[inline]
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        Codec::read(read, &())
    }

}

/// Abstract type representing a method for an entity.
pub trait Method: Sized {

    /// Return the preferred encoding length of this method, when sub message id is used
    /// this is just ignored.
    fn write_length(&self) -> ElementLength;

    /// Encode the method call into the given writer.
    fn write(&self, write: &mut dyn Write) -> io::Result<u16>;

    /// Return the decode length for the given exposed method id, or an error if this
    /// exposed id isn't known (e.g. a mismatch between the generated method tables and
    /// what the live game actually sends, rather than something to ever crash on).
    fn read_length(exposed_id: u16) -> io::Result<ElementLength>;

    /// Decode the given method from the given reader and its exposed id.
    fn read(read: &mut dyn Read, exposed_id: u16) -> io::Result<Self>;

}

/// This macro can be used to generate an enumeration capable of encoding and decoding
/// an arbitrary number of methods, the enumeration implements the [`Method`] trait, and
/// all methods should 
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

            impl $crate::net::app::common::entity::Method for $enum_name {
                fn write_length(&self) -> $crate::net::element::ElementLength {
                    match self {
                        $( Self::$method_name (_) => $crate::__enum_entity_methods!(__length; $method_length), )*
                        _ => unreachable!()
                    }
                }
                fn write(&self, write: &mut dyn std::io::Write) -> std::io::Result<u16> {
                    use $crate::net::codec::Codec;
                    match self {
                        $( Self::$method_name (m) => Codec::<()>::write(m, write, &()).map(|()| $method_exposed_id), )*
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
        )*
    };
}
