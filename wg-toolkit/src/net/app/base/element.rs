//! Definition of elements related to base application.
//! 
//! Such elements are sent from the client to the base application and also
//! replies to such elements if they are requests.

use std::io::{self, Read, Write};

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::net::app::common::entity::Method;
use crate::util::io::{WgReadExt, WgWriteExt};


/// Internal module containing all raw elements numerical ids.
pub mod id {

    use crate::net::element::ElementIdRange;

    // All ids below were confirmed empirically by attaching to a live game
    // process (v2.3.1.3, 2026-08-24) and reading the actual registration
    // order out of `BaseAppExtInterface`'s `Mercury::InterfaceMinder` in
    // memory (from `baseapp_ext_interface.hpp`).

    pub const LOGIN_KEY: u8                     = 0x00; // baseAppLogin
    pub const PING_DATACENTER: u8                = 0x01; // pingDatacenter
    pub const SESSION_KEY: u8                   = 0x02; // authenticate
    pub const AVATAR_UPDATE_IMPLICIT: u8        = 0x03;
    pub const AVATAR_UPDATE_EXPLICIT: u8        = 0x04;
    pub const ACK_PHYSICS_CORRECTION: u8        = 0x05;
    pub const REQUEST_ENTITY_UPDATE: u8         = 0x06;
    pub const NRL_MSG_TO_CELL: u8               = 0x07;
    pub const AVATAR_UPDATE_WARD_IMPLICIT: u8   = 0x08;
    pub const AVATAR_UPDATE_WARD_EXPLICIT: u8   = 0x09;
    pub const ACK_WARD_PHYSICS_CORRECTION: u8   = 0x0A;
    pub const ENABLE_ENTITIES: u8               = 0x0B;
    pub const RESTORE_CLIENT_ACK: u8            = 0x0C;
    pub const DISCONNECT_CLIENT: u8             = 0x0D;
    pub const CLIENT_TO_SERVER_HEARTBEAT: u8    = 0x0E;
    pub const SEND_TO_CELL: u8                  = 0x0F;

    pub const CELL_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x10, 0x87);
    pub const BASE_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x88, 0xFE);

}


crate::__struct_simple_codec! {
    /// Sent by the client to the server without encryption in order to authenticate,
    /// the server then compares with its internal login keys from past successful
    /// logins on the login app.
    /// 
    /// This element is usually a request, in such case a [`SessionKey`] must be sent as 
    /// a reply, which is the server session key (not the same as login key).
    #[derive(Debug, Clone)]
    pub struct LoginKey {
        /// The login key that was sent by the login application, part of the  element
        /// [`super::login::LoginSuccess`].
        pub login_key: u32,
        /// The current number of attempts.
        pub attempt_num: u8,
        /// Unknown 16-bits value at the end.
        pub unk: u16,
    }
}

impl SimpleElement for LoginKey {
    const ID: u8 = id::LOGIN_KEY;
    const LEN: ElementLength = ElementLength::Fixed(7);
}


crate::__struct_simple_codec! {
    /// This element can be used in two cases:
    /// - As a reply to [`LoginKey`] from the server to the client in order to give it
    ///   the initial session key.
    /// - Sent by the client on login (and apparently randomly after login) to return 
    ///   the session key that was sent by the server in the initial reply (first case).
    #[derive(Debug, Clone)]
    pub struct SessionKey {
        /// The server session key.
        pub session_key: u32,
    }
}

impl SimpleElement for SessionKey {
    const ID: u8 = id::SESSION_KEY;
    const LEN: ElementLength = ElementLength::Fixed(4);
}


crate::__struct_simple_codec! {
    /// This is sent by the client to the base application as an acknowledgment of a
    /// reset entity request sent to the client.
    #[derive(Debug, Clone)]
    pub struct EnableEntities {}
}

impl SimpleElement for EnableEntities {
    const ID: u8 = id::ENABLE_ENTITIES;
    const LEN: ElementLength = ElementLength::ZERO;
}


crate::__struct_simple_codec! {
    /// This is sent by the client to the base application as an acknowledgment of a
    /// reset entity request sent to the client.
    #[derive(Debug, Clone)]
    pub struct DisconnectClient {
        pub reason: u8,
    }
}

impl SimpleElement for DisconnectClient {
    const ID: u8 = id::DISCONNECT_CLIENT;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


// The following elements are known (id, length style, length param all confirmed
// live against v2.3.1.3) but not yet given a proper structured codec, so they are
// defined as raw/debug elements for now, following the same convention used for
// elements not yet fully reverse-engineered in `client::element`.

/// Sent by the client to report latency to various datacenters, used for
/// server/periphery selection.
pub type PingDatacenter = DebugElementFixed<{ id::PING_DATACENTER }, 10>;

pub type AvatarUpdateImplicit = DebugElementFixed<{ id::AVATAR_UPDATE_IMPLICIT }, 17>;
pub type AvatarUpdateExplicit = DebugElementFixed<{ id::AVATAR_UPDATE_EXPLICIT }, 22>;
pub type AckPhysicsCorrection = DebugElementFixed<{ id::ACK_PHYSICS_CORRECTION }, 0>;
pub type RequestEntityUpdate = DebugElementVariable16<{ id::REQUEST_ENTITY_UPDATE }>;

pub type NrlMsgToCell = DebugElementVariable16<{ id::NRL_MSG_TO_CELL }>;

pub type AvatarUpdateWardImplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_IMPLICIT }, 20>;
pub type AvatarUpdateWardExplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_EXPLICIT }, 29>;
pub type AckWardPhysicsCorrection = DebugElementFixed<{ id::ACK_WARD_PHYSICS_CORRECTION }, 4>;
pub type RestoreClientAck = DebugElementFixed<{ id::RESTORE_CLIENT_ACK }, 4>;

pub type ClientToServerHeartbeat = DebugElementFixed<{ id::CLIENT_TO_SERVER_HEARTBEAT }, 0>;
pub type SendToCell = DebugElementFixed<{ id::SEND_TO_CELL }, 0>;


/// Codec for a base entity method call.
#[derive(Debug, Clone)]
pub struct BaseEntityMethod<M: Method> {
    pub inner: BaseEntityMethodInner<M>,
}

/// The decoded (or not) payload of a [`BaseEntityMethod`].
#[derive(Debug, Clone)]
pub enum BaseEntityMethodInner<M> {
    /// A method call whose exposed id is present in `M`'s generated table.
    Known(M),
    /// An exposed id missing from `M`'s generated table (e.g. a mismatch between the
    /// generated method tables and what the live game actually sends). Unlike
    /// client-directed `EntityMethod`, this element's length is always Variable16
    /// regardless of id, so framing was never at risk here -- this variant only avoids
    /// erroring out of `M::read` and losing bundle synchronization over a method we
    /// simply don't have a decoder for yet.
    Unknown {
        exposed_id: u16,
        data: Vec<u8>,
    },
}

impl<M: Method> Element<()> for BaseEntityMethod<M> {

    fn write_length(&self, _config: &()) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<u8> {
        let exposed_id = match &self.inner {
            BaseEntityMethodInner::Known(inner) => inner.exposed_id(),
            BaseEntityMethodInner::Unknown { exposed_id, .. } => *exposed_id,
        };
        let (element_id, sub_id) = id::BASE_ENTITY_METHOD.from_exposed_id(M::count(), exposed_id);
        if let Some(sub_id) = sub_id {
            write.write_u8(sub_id)?;
        }
        match &self.inner {
            BaseEntityMethodInner::Known(inner) => { inner.write(write)?; }
            BaseEntityMethodInner::Unknown { data, .. } => write.write_all(data)?,
        }
        Ok(element_id)
    }

    fn read_length(_config: &(), _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, _config: &(), len: usize, id: u8) -> io::Result<Self> {
        if !id::BASE_ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected base entity method element id: {id:02X}")));
        }

        let mut len = len;
        let mut sub_id_err = None;
        let exposed_id = id::BASE_ENTITY_METHOD.to_exposed_id(M::count(), id, || {
            len = len.saturating_sub(1);
            match read.read_u8() {
                Ok(n) => n,
                Err(e) => {
                    sub_id_err = Some(e);
                    0 // Unused, we bail out right after via sub_id_err.
                }
            }
        });
        if let Some(e) = sub_id_err {
            return Err(e);
        }

        let inner = if M::read_length(exposed_id).is_ok() {
            BaseEntityMethodInner::Known(M::read(read, exposed_id)?)
        } else {
            let mut data = vec![0; len];
            read.read_exact(&mut data)?;
            BaseEntityMethodInner::Unknown { exposed_id, data }
        };
        Ok(Self {
            inner,
        })
    }

}
