//! Definition of elements related to base application.
//! 
//! Such elements are sent from the client to the base application and also
//! replies to such elements if they are requests.

use std::io::{self, Read, Write};

use glam::Vec3;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::util::io::WgReadExt;
use crate::app::dispatch::{MethodDef, MethodCall};
use crate::app::math::PackedYawPitchRoll;


/// Internal module containing all raw elements numerical ids.
pub mod id {

    use crate::net::element::ElementIdRange;

    // All ids below were confirmed empirically by attaching to a live game
    // process (v2.3.1.3, 2026-08-24) and reading the actual registration
    // order out of `BaseAppExtInterface`'s `Mercury::InterfaceMinder` in
    // memory (from `baseapp_ext_interface.hpp`).

    // --- Login/session handshake & datacenter ping ---
    pub const LOGIN_KEY: u8                     = 0x00; // baseAppLogin
    pub const PING_DATACENTER: u8               = 0x01; // pingDatacenter
    pub const SESSION_KEY: u8                   = 0x02; // authenticate

    // --- Avatar updates & physics corrections (raw placeholders) ---
    pub const AVATAR_UPDATE_IMPLICIT: u8        = 0x03;
    pub const AVATAR_UPDATE_EXPLICIT: u8        = 0x04;
    pub const ACK_PHYSICS_CORRECTION: u8        = 0x05;
    pub const REQUEST_ENTITY_UPDATE: u8         = 0x06;

    // --- Network Replication Layer ("NRL") ---
    pub const NRL_MSG_TO_CELL: u8               = 0x07;

    // --- Ward avatar updates & physics corrections ---
    pub const AVATAR_UPDATE_WARD_IMPLICIT: u8   = 0x08;
    pub const AVATAR_UPDATE_WARD_EXPLICIT: u8   = 0x09;
    pub const ACK_WARD_PHYSICS_CORRECTION: u8   = 0x0A;

    // --- Session control & acks ---
    pub const ENABLE_ENTITIES: u8               = 0x0B;
    pub const RESTORE_CLIENT_ACK: u8            = 0x0C;
    pub const DISCONNECT_CLIENT: u8             = 0x0D;
    pub const CLIENT_TO_SERVER_HEARTBEAT: u8    = 0x0E;
    pub const SEND_TO_CELL: u8                  = 0x0F;

    // --- Dynamic entity method dispatch (base & cell) ---
    pub const CELL_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x10, 0x87);
    pub const BASE_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x88, 0xFE);

}


// =============================================================================
// Login/session handshake
// =============================================================================

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

// =============================================================================
// Avatar updates & physics corrections
// =============================================================================

/// Sent by the client to report latency to various datacenters, used for
/// server/periphery selection.
pub type PingDatacenter = DebugElementFixed<{ id::PING_DATACENTER }, 10>;

// Both messages below are confirmed against the leaked BigWorld 14.4.1 SDK
// (`connection/baseapp_ext_interface.hpp`'s `BaseAppExtInterface::avatarUpdateImplicit`/
// `avatarUpdateExplicit`, specifically their `!VOLATILE_POSITIONS_ARE_ABSOLUTE` variant
// -- confirmed still in effect for this build, see `network/msgtypes.hpp`), but each
// comes out exactly 1 byte short of this project's own already-confirmed length (16 vs
// 17, 21 vs 22 -- both from live `BaseAppExtInterface` registration data, see the `id`
// module). The same +1 discrepancy shows up independently in every other
// `avatarUpdate*`-family message checked so far (all three client-directed `*Detailed`
// messages in `client::element`), which is consistent enough to suspect a single,
// uniform addition across the whole family rather than a one-off -- but its exact
// position and meaning aren't confirmed here, so it's kept as a raw trailing
// `unk_byte: u8` (trailing is a guess, not confirmed placement) rather than asserted as
// a specific field.
crate::__struct_simple_codec! {
    /// Sent by the client to report an update to its own position/direction, without
    /// stating which vehicle it's on -- see [`AvatarUpdateExplicit`] for the version that
    /// does, and this type's enclosing doc comment for the `+1` byte caveat.
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdateImplicit {
        pub position: Vec3,
        /// Yaw/pitch/roll, packed like [`crate::app::math::PackedYawPitchRoll`] with
        /// `half_pitch = true` (`YawPitchRoll`'s default in the SDK).
        pub direction: PackedYawPitchRoll,
        /// Confirmed in vanilla (`// refNum is used to refer to this position later as
        /// the base for relative positions`): identifies this update so a later relative
        /// position can reference it as a base.
        pub ref_num: u8,
        pub unk_byte: u8,
    }
}

impl SimpleElement for AvatarUpdateImplicit {
    const ID: u8 = id::AVATAR_UPDATE_IMPLICIT;
    const LEN: ElementLength = ElementLength::Fixed(17);
}

crate::__struct_simple_codec! {
    /// Sent by the client to report an update to its own position/direction while
    /// explicitly stating which vehicle it's mounted on/in -- unlike
    /// [`AvatarUpdateImplicit`], which omits vehicle context entirely (implicitly
    /// unchanged or not applicable). See [`AvatarUpdateImplicit`]'s enclosing doc comment
    /// for the `+1` byte caveat shared by both.
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdateExplicit {
        pub vehicle_id: u32,
        pub position: Vec3,
        pub direction: PackedYawPitchRoll,
        /// Bit flags; confirmed live bit `0x1` = on-ground (the vanilla source's own
        /// `AVATAR_UPDATE_EXPLICT_FLAG_ONGROUND`, "must match `cellapp_interface.hpp`").
        /// Use [`Self::on_ground`] rather than reading the bit directly.
        pub flags: u8,
        pub ref_num: u8,
        pub unk_byte: u8,
    }
}

impl AvatarUpdateExplicit {
    /// Decode the confirmed `AVATAR_UPDATE_EXPLICT_FLAG_ONGROUND` bit from [`Self::flags`].
    pub fn on_ground(&self) -> bool {
        self.flags & 0x1 != 0
    }
}

impl SimpleElement for AvatarUpdateExplicit {
    const ID: u8 = id::AVATAR_UPDATE_EXPLICIT;
    const LEN: ElementLength = ElementLength::Fixed(22);
}

crate::__struct_simple_codec! {
    /// Acknowledges a physics correction applied to the client's controlled entity (see
    /// [`super::super::client::element::ForcedPosition`], the server-directed message
    /// this replies to). No fields: confirmed against the leaked BigWorld 14.4.1 SDK
    /// (`connection/baseapp_ext_interface.hpp`'s `MF_EMPTY_BLOCKABLE_PROXY_MSG(
    /// ackPhysicsCorrection )`), matching this project's own already-confirmed 0-byte
    /// length exactly.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct AckPhysicsCorrection {}
}

impl SimpleElement for AckPhysicsCorrection {
    const ID: u8 = id::ACK_PHYSICS_CORRECTION;
    const LEN: ElementLength = ElementLength::ZERO;
}

pub type RequestEntityUpdate = DebugElementVariable16<{ id::REQUEST_ENTITY_UPDATE }>;

// =============================================================================
// Network Replication Layer ("NRL"), WoT-specific -- see `client::element`'s own
// `Nrl*` section banner for what this is.
// =============================================================================

pub type NrlMsgToCell = DebugElementVariable16<{ id::NRL_MSG_TO_CELL }>;

// =============================================================================
// Ward avatar updates & session control
// =============================================================================

pub type AvatarUpdateWardImplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_IMPLICIT }, 20>;
pub type AvatarUpdateWardExplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_EXPLICIT }, 29>;
pub type AckWardPhysicsCorrection = DebugElementFixed<{ id::ACK_WARD_PHYSICS_CORRECTION }, 4>;
pub type RestoreClientAck = DebugElementFixed<{ id::RESTORE_CLIENT_ACK }, 4>;

pub type ClientToServerHeartbeat = DebugElementFixed<{ id::CLIENT_TO_SERVER_HEARTBEAT }, 0>;
pub type SendToCell = DebugElementFixed<{ id::SEND_TO_CELL }, 0>;

// =============================================================================
// Dynamic entity method dispatch (base & cell)
// =============================================================================

/// A base-directed entity method call, decoded dynamically against a runtime-computed
/// [`MethodDef`] table (see [`crate::app::script::EntityDispatch`]) resolved from the
/// loaded script model -- lets [`super::App`] dispatch by name against whatever game
/// version's script was loaded. Read-only: this is only ever received, from the client.
#[derive(Debug, Clone)]
pub struct BaseEntityMethod {
    pub call: MethodCall,
}

impl Element<Vec<MethodDef>> for BaseEntityMethod {

    fn write_length(&self, _config: &Vec<MethodDef>) -> io::Result<ElementLength> {
        unreachable!("BaseEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &Vec<MethodDef>) -> io::Result<u8> {
        unreachable!("BaseEntityMethod is read-only")
    }

    fn read_length(_config: &Vec<MethodDef>, _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &Vec<MethodDef>, len: usize, id: u8) -> io::Result<Self> {

        if !id::BASE_ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected base entity method element id: {id:02X}")));
        }

        let mut len = len;
        let mut sub_id_err = None;
        let exposed_id = id::BASE_ENTITY_METHOD.to_exposed_id(config.len() as u16, id, || {
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

        let call = match config.get(exposed_id as usize) {
            Some(def) => MethodCall::Known { name: def.name.clone(), args: def.read_args(read)? },
            None => {
                let mut data = vec![0; len];
                read.read_exact(&mut data)?;
                MethodCall::Unknown { exposed_id, data }
            }
        };

        Ok(Self { call })

    }

}


/// Same as [`BaseEntityMethod`], but for a cell-directed call. The client sends these to
/// the base app (there's no direct client-to-cell connection), which forwards them to the
/// cell app in real BigWorld; this project has no `cell::App` yet, so decoding one only
/// recovers the call itself, not anything about forwarding it.
#[derive(Debug, Clone)]
pub struct CellEntityMethod {
    pub call: MethodCall,
}

impl Element<Vec<MethodDef>> for CellEntityMethod {

    fn write_length(&self, _config: &Vec<MethodDef>) -> io::Result<ElementLength> {
        unreachable!("CellEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &Vec<MethodDef>) -> io::Result<u8> {
        unreachable!("CellEntityMethod is read-only")
    }

    fn read_length(_config: &Vec<MethodDef>, _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &Vec<MethodDef>, len: usize, id: u8) -> io::Result<Self> {

        if !id::CELL_ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected cell entity method element id: {id:02X}")));
        }

        let mut len = len;
        let mut sub_id_err = None;
        let exposed_id = id::CELL_ENTITY_METHOD.to_exposed_id(config.len() as u16, id, || {
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

        let call = match config.get(exposed_id as usize) {
            Some(def) => MethodCall::Known { name: def.name.clone(), args: def.read_args(read)? },
            None => {
                let mut data = vec![0; len];
                read.read_exact(&mut data)?;
                MethodCall::Unknown { exposed_id, data }
            }
        };

        Ok(Self { call })

    }

}
