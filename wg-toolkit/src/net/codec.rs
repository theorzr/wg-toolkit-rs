//! This module contains the network codec trait and builtin implementations for trivial
//! types that are commonly used, such as ints, floats and various common blobs.


use std::io::{self, Read, Write};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use glam::{Vec2, Vec3, Vec4};

use crate::util::io::{WgReadExt, WgWriteExt, serde_pickle_de_options, serde_pickle_ser_options};
use crate::util::AsciiFmt;


/// Represent a codec for some data that can be both encoded and decoded, with a 
/// configuration value that can alter how the data is actually encoded and decoded.
pub trait Codec<C>: Sized {

    /// Write the data onto the given writer and configuration.
    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<()>;

    /// Read the data from the given reader and configuration.
    fn read(read: &mut dyn Read, config: &C) -> io::Result<Self>;

}

/// Alternate trait to [`Codec`] without config value, automatically implementing the
/// [`Codec`] trait for any implementor, therefore it's not possible to impl both.
pub trait SimpleCodec: Sized {
    
    /// Write the data onto the given writer.
    fn write(&self, write: &mut dyn Write) -> io::Result<()>;

    /// Read the data from the given reader.
    fn read(read: &mut dyn Read) -> io::Result<Self>;
    
}

impl<C: SimpleCodec> Codec<()> for C {

    #[inline(always)]
    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<()> {
        SimpleCodec::write(self, write)
    }

    #[inline(always)]
    fn read(read: &mut dyn Read, _config: &()) -> io::Result<Self> {
        SimpleCodec::read(read)
    }

}

impl SimpleCodec for () {

    #[inline(always)]
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }

    #[inline(always)]
    fn read(_read: &mut dyn Read) -> io::Result<Self> {
        Ok(())
    }

}

impl SimpleCodec for String {

    #[inline(always)]
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_string_variable(self)
    }

    #[inline(always)]
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        read.read_string_variable()
    }

}

impl<const LEN: usize, C, D: Codec<C>> Codec<C> for Box<[D; LEN]> {

    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<()> {
        for comp in &**self {
            comp.write(&mut *write, config)?;
        }
        Ok(())
    }

    fn read(read: &mut dyn Read, config: &C) -> io::Result<Self> {
        
        let mut tmp = Vec::with_capacity(LEN);
        for _ in 0..LEN {
            tmp.push(D::read(&mut *read, config)?);
        }

        let Ok(ret) = tmp.into_boxed_slice().try_into() else {
            unreachable!();
        };

        Ok(ret)

    }
    
}

impl<C, D: Codec<C>> Codec<C> for Vec<D> {

    fn write(&self, write: &mut dyn Write, config: &C) -> io::Result<()> {
        write.write_packed_u24(self.len() as u32)?;
        for comp in &**self {
            comp.write(&mut *write, config)?;
        }
        Ok(())
    }

    fn read(read: &mut dyn Read, config: &C) -> io::Result<Self> {
        let len = read.read_packed_u24()? as usize;
        let mut tmp = Vec::with_capacity(len);
        for _ in 0..len {
            tmp.push(D::read(&mut *read, config)?);
        }
        Ok(tmp)
    }

}

macro_rules! impl_builtin_copy {
    ($ty:ty, $write_method:ident, $read_method:ident) => {
        impl SimpleCodec for $ty {

            #[inline(always)]
            fn write(&self, write: &mut dyn Write) -> io::Result<()> {
                write.$write_method(*self)
            }
        
            #[inline(always)]
            fn read(read: &mut dyn Read) -> io::Result<Self> {
                read.$read_method()
            }
        
        }
    };
}

impl_builtin_copy!(bool, write_bool, read_bool);
impl_builtin_copy!(u8, write_u8, read_u8);
impl_builtin_copy!(i8, write_i8, read_i8);
impl_builtin_copy!(u16, write_u16, read_u16);
impl_builtin_copy!(i16, write_i16, read_i16);
impl_builtin_copy!(u32, write_u32, read_u32);
impl_builtin_copy!(i32, write_i32, read_i32);
impl_builtin_copy!(u64, write_u64, read_u64);
impl_builtin_copy!(i64, write_i64, read_i64);
impl_builtin_copy!(f32, write_f32, read_f32);
impl_builtin_copy!(f64, write_f64, read_f64);
impl_builtin_copy!(Vec2, write_vec2, read_vec2);
impl_builtin_copy!(Vec3, write_vec3, read_vec3);
impl_builtin_copy!(Vec4, write_vec4, read_vec4);


/// A `Mercury::Address`: an IPv4 socket address paired with a 16-bit "salt".
///
/// Confirmed against the leaked BigWorld source (`server/baseapp/baseapp.cpp`, e.g.
/// `address.salt = pChannel->isTCP() ? 1 : 0`) that this salt is just a TCP/UDP
/// discriminator bit for an address-of-an-app value like this -- not a checksum or
/// anything derived from the ip/port -- so it's always `0` for this project's UDP-only
/// client-facing protocol. It's still modeled explicitly (rather than silently discarded
/// like the plain `SocketAddrV4` codec used to do) so that code rewriting an
/// already-encoded address in place (e.g. a proxy patching a `SwitchBaseApp`'s embedded
/// address) can preserve or set it deliberately instead of by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WgSocketAddrV4 {
    pub addr: SocketAddrV4,
    pub salt: u16,
}

impl WgSocketAddrV4 {

    pub fn new(addr: SocketAddrV4, salt: u16) -> Self {
        Self { addr, salt }
    }

    /// Encode into the exact 8 on-wire bytes (ip, then port big-endian, then salt
    /// little-endian) -- e.g. for locating/rewriting an in-place occurrence of this
    /// value within an already-encoded packet.
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out[..4].copy_from_slice(&self.addr.ip().octets());
        out[4..6].copy_from_slice(&self.addr.port().to_be_bytes());
        out[6..8].copy_from_slice(&self.salt.to_le_bytes());
        out
    }

    /// Decode from the exact 8 on-wire bytes produced by [`Self::to_bytes`].
    pub fn from_bytes(bytes: [u8; 8]) -> Self {
        let ip = Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]);
        let port = u16::from_be_bytes([bytes[4], bytes[5]]);
        let salt = u16::from_le_bytes([bytes[6], bytes[7]]);
        Self { addr: SocketAddrV4::new(ip, port), salt }
    }

}

impl Deref for WgSocketAddrV4 {
    type Target = SocketAddrV4;
    fn deref(&self) -> &SocketAddrV4 {
        &self.addr
    }
}

impl fmt::Display for WgSocketAddrV4 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.addr, f)
    }
}

impl From<SocketAddrV4> for WgSocketAddrV4 {
    fn from(addr: SocketAddrV4) -> Self {
        Self { addr, salt: 0 }
    }
}

impl From<WgSocketAddrV4> for SocketAddrV4 {
    fn from(value: WgSocketAddrV4) -> Self {
        value.addr
    }
}

impl From<WgSocketAddrV4> for SocketAddr {
    fn from(value: WgSocketAddrV4) -> Self {
        SocketAddr::V4(value.addr)
    }
}

impl SimpleCodec for WgSocketAddrV4 {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.to_bytes())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut bytes = [0u8; 8];
        read.read_exact(&mut bytes)?;
        Ok(Self::from_bytes(bytes))
    }

}


/// The string data type used by default for all STRING types, it will try to 
#[derive(Clone)]
pub enum AutoString {
    String(String),
    Python(serde_pickle::Value),
    Raw(Vec<u8>),
}

impl fmt::Debug for AutoString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(string) => f.debug_tuple("Utf8").field(string).finish(),
            Self::Python(value) => f.debug_tuple("Python").field(&format_args!("{value}")).finish(),
            Self::Raw(bytes) => f.debug_tuple("Raw").field(&AsciiFmt(&bytes)).finish(),
        }
    }
}

impl SimpleCodec for AutoString {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_blob_variable(&*(match self {
            AutoString::String(v) => Cow::Borrowed(v.as_bytes()),
            AutoString::Python(v) => Cow::Owned(serde_pickle::value_to_vec(v, serde_pickle_ser_options()).unwrap()),
            AutoString::Raw(v) => Cow::Borrowed(&v[..]),
        }))
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        
        let raw = read.read_blob_variable()?;

        if let Ok(v) = serde_pickle::value_from_reader(&raw[..], serde_pickle_de_options()) {
            return Ok(Self::Python(v));
        }

        match String::from_utf8(raw) {
            Ok(s) => Ok(Self::String(s)),
            Err(e) => Ok(Self::Raw(e.into_bytes())),
        }

    }

}


/// The Python builtin data type.
pub struct Python {
    /// Internal pickle value.
    pub value: serde_pickle::Value,
}

impl SimpleCodec for Python {

    #[inline(always)]
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_python_pickle(&self.value)
    }

    #[inline(always)]
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        read.read_python_pickle().map(|value| Self { value })
    }

}

impl fmt::Debug for Python {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Python")
            .field(&format_args!("{}", self.value))
            .finish()
    }
}


/// The mailbox type used sparingly in method calls.
#[derive(Debug)]
pub struct Mailbox {
    pub entity_id: u32,
    pub address: (), // TODO: 
}

impl SimpleCodec for Mailbox {

    #[inline]
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::InvalidData, "mailbox write not yet supported"))
    }

    #[inline]
    fn read(_read: &mut dyn Read) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::InvalidData, "mailbox write not yet supported"))
    }

}


/// This macro can be used to create simple aggregation of structures with all fields of
/// type [`Codec<()>`], the structure is both defined and trait is implemented.
#[macro_export]
macro_rules! __struct_simple_codec {
    (
        $(
            $(#[$attr:meta])* 
            $struct_vis:vis struct $struct_name:ident {
                $( $(#[$field_attr:meta])* $field_vis:vis $field_name:ident : $field_ty:ty ),*
                $(,)?
            }
        )*
    ) => {
        $(
            $(#[$attr])* 
            $struct_vis struct $struct_name {
                $( $(#[$field_attr])* $field_vis $field_name : $field_ty,)*
            }

            #[allow(unused_imports, unused_variables)]
            impl $crate::net::codec::SimpleCodec for $struct_name {
                fn write(&self, write: &mut dyn std::io::Write) -> std::io::Result<()> {
                    use $crate::net::codec::Codec;
                    $( Codec::<()>::write(&self.$field_name, &mut *write, &())?; )*
                    Ok(())
                }
                fn read(read: &mut dyn std::io::Read) -> std::io::Result<Self> {
                    use $crate::net::codec::Codec;
                    Ok(Self {
                        $( $field_name: Codec::<()>::read(&mut *read, &())?, )*
                    })
                }
            }
        )*
    };
}
