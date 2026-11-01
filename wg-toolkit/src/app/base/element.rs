//! Definition of elements related to base application.
//! 
//! Such elements are sent from the client to the base application and also
//! replies to such elements if they are requests.

use std::io::{self, Read, Write};

use glam::Vec3;

use crate::net::element::{DebugElementFixed, ElementLength, Element, SimpleElement};
use crate::app::dispatch::{MethodDef, MethodCall};
use crate::app::client::element::NrlStream;
use crate::app::math::PackedYawPitchRoll;
use crate::net::codec::SimpleCodec;
use crate::util::io::WgReadExt;


/// Internal module containing all raw elements numerical ids.
pub mod id {

    use crate::net::element::ElementIdRange;

    // The ids below follow the registration order of `BaseAppExtInterface`'s
    // `Mercury::InterfaceMinder` (`baseapp_ext_interface.hpp`). Unlike
    // `client::element`'s `ClientInterface`, nothing in this interface uses
    // `CALLBACK`: every id here is either a fixed byte count or a
    // `Variable16`-style dynamic size.

    // --- Login/session handshake & datacenter ping ---
    pub const LOGIN_KEY: u8                     = 0x00; // baseAppLogin, FIXED 7
    pub const PING_DATACENTER: u8               = 0x01; // pingDatacenter, FIXED 10
    pub const SESSION_KEY: u8                   = 0x02; // authenticate, FIXED 4

    // --- Avatar updates & physics corrections (raw placeholders) ---
    pub const AVATAR_UPDATE_IMPLICIT: u8        = 0x03;  // FIXED 17
    pub const AVATAR_UPDATE_EXPLICIT: u8        = 0x04;  // FIXED 22
    pub const ACK_PHYSICS_CORRECTION: u8        = 0x05;  // FIXED 0
    pub const REQUEST_ENTITY_UPDATE: u8         = 0x06;  // VAR 2

    // --- Network Replication Layer ("NRL") ---
    pub const NRL_MSG_TO_CELL: u8               = 0x07;  // VAR 2

    // --- Ward avatar updates & physics corrections ---
    pub const AVATAR_UPDATE_WARD_IMPLICIT: u8   = 0x08;  // FIXED 20
    pub const AVATAR_UPDATE_WARD_EXPLICIT: u8   = 0x09;  // FIXED 29
    pub const ACK_WARD_PHYSICS_CORRECTION: u8   = 0x0A;  // FIXED 4

    // --- Session control & acks ---
    pub const ENABLE_ENTITIES: u8               = 0x0B;  // FIXED 0
    pub const RESTORE_CLIENT_ACK: u8            = 0x0C;  // FIXED 4
    pub const DISCONNECT_CLIENT: u8             = 0x0D;  // FIXED 1
    pub const CLIENT_TO_SERVER_HEARTBEAT: u8    = 0x0E;  // FIXED 0
    pub const SEND_TO_CELL: u8                  = 0x0F;  // FIXED 0

    // --- Dynamic entity method dispatch (base & cell) ---
    pub const CELL_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x10, 0x87);  // VAR 2 (all entries, not CALLBACK)
    pub const BASE_ENTITY_METHOD: ElementIdRange = ElementIdRange::new(0x88, 0xFE);  // VAR 2 (all entries, not CALLBACK)

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


// The following elements have a known id and framing but no structured codec, so they
// are defined as placeholder elements, the same convention as in `client::element`.

// =============================================================================
// Avatar updates & physics corrections
// =============================================================================

/// Sent by the client to report latency to various datacenters, used for
/// server/periphery selection.
pub type PingDatacenter = DebugElementFixed<{ id::PING_DATACENTER }, 10>;

// Both messages below follow `BaseAppExtInterface::avatarUpdateImplicit`/
// `avatarUpdateExplicit` (`connection/baseapp_ext_interface.hpp`), in their
// `!VOLATILE_POSITIONS_ARE_ABSOLUTE` variant (`network/msgtypes.hpp`), but each is one
// byte longer than that layout: 17 bytes rather than 16, and 22 rather than 21. The
// client-directed `*Detailed` messages in `client::element` carry the same extra byte.
//
// Its position and meaning are unknown, so it is kept as a trailing `unk_byte: u8`;
// trailing is an assumption. The same extra byte follows the packed yaw/pitch/roll in
// `CreateEntity` (`direction_unk`) and in the `*_YAW_PITCH_ROLL` avatar updates, so it
// may belong to the direction rather than to the end of the message.
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
        /// Identifies this update so a later relative position can reference it as its
        /// base.
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
        /// Bit flags. Bit `0x1` is on-ground
        /// (`AVATAR_UPDATE_EXPLICT_FLAG_ONGROUND`); read it through [`Self::on_ground`]
        /// rather than testing the bit directly. The other bits have no known meaning.
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
    /// ackPhysicsCorrection )`), matching the 0-byte
    /// length exactly.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct AckPhysicsCorrection {}
}

impl SimpleElement for AckPhysicsCorrection {
    const ID: u8 = id::ACK_PHYSICS_CORRECTION;
    const LEN: ElementLength = ElementLength::ZERO;
}

/// The reply to `BaseAppExtInterface::baseAppLogin`: this connection's `SessionKey`.
///
/// Not an element -- it has no id of its own, since a reply is addressed by the request
/// id it answers -- so it is a bare [`SimpleCodec`] read straight off the reply body.
///
/// The client echoes this value as [`SessionKey`] at the head of every subsequent
/// outbound bundle, which is what authenticates the connection.
///
/// The request that asks for it, `baseAppLogin`, is the one packet on a BaseApp
/// connection that is not encrypted: `BaseAppLoginRequestProtocol::sendRequest` sends it
/// off-channel and only then enables the channel's block cipher, to receive this reply.
/// A reader that expects every packet to be encrypted fails on that request, so it has no
/// request id to match the reply against and must identify the reply by position: the
/// first reply on the connection.
#[derive(Debug, Clone, Copy)]
pub struct BaseAppLoginReply {
    pub session_key: u32,
}

impl SimpleCodec for BaseAppLoginReply {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.session_key.to_le_bytes())
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut buf = [0u8; 4];
        read.read_exact(&mut buf)?;
        Ok(Self { session_key: u32::from_le_bytes(buf) })
    }
}

/// Asks the server to re-send an entity's properties, naming the cache event stamps the
/// client already holds so the server can send only what changed.
///
/// `ServerConnection::requestEntityUpdate` writes the id and then streams the stamps with
/// no count and no terminator -- the element's own length is the only bound, so the stamp
/// count is `(len - 4) / 4`. `CacheStamps` is `vector<EventNumber>` and `EventNumber` is
/// `int32` (`network/basictypes.hpp`), hence signed.
///
/// An empty `stamps` is normal and meaningful: `ServerConnection` calls this with no
/// stamps when it holds no cached state for the entity, which asks for everything.
#[derive(Debug, Clone)]
pub struct RequestEntityUpdate {
    pub entity_id: u32,
    pub stamps: Vec<i32>,
    /// Bytes after the last whole stamp; empty for a well-formed message.
    ///
    /// A ragged tail is reported here rather than raised as an error, so a payload that
    /// does not match this layout does not stop the rest of the bundle from decoding.
    pub rest: Vec<u8>,
}

impl SimpleCodec for RequestEntityUpdate {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.entity_id.to_le_bytes())?;
        for stamp in &self.stamps {
            write.write_all(&stamp.to_le_bytes())?;
        }
        write.write_all(&self.rest)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut body = Vec::new();
        read.read_to_end(&mut body)?;
        if body.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("requestEntityUpdate: {} bytes, need at least the entity id", body.len())));
        }
        let entity_id = u32::from_le_bytes(body[..4].try_into().unwrap());
        let tail = &body[4..];
        let whole = tail.len() / 4 * 4;
        Ok(Self {
            entity_id,
            stamps: tail[..whole].chunks_exact(4)
                .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                .collect(),
            rest: tail[whole..].to_vec(),
        })
    }

}

impl SimpleElement for RequestEntityUpdate {
    const ID: u8 = id::REQUEST_ENTITY_UPDATE;
    const LEN: ElementLength = ElementLength::Variable16;
}

// =============================================================================
// Network Replication Layer ("NRL"), WoT-specific -- see `client::element`'s own
// `Nrl*` section banner for what this is.
// =============================================================================

/// The client -> cell direction of the NRL message stream: the mirror of
/// [`crate::app::client::element::NrlMsgToClient`], and framed identically.
///
/// Same `[msg_type: u8][node_id: u16 big-endian][body]` records, with no implied leading
/// type, because both directions are the same `BWEndPoint` stream -- only the transport
/// element differs. What travels this way is the `ServerMethodCallerNode` side of the
/// tree (the client invoking a dynamic component's server method) plus node lookups.
///
/// A non-empty [`NrlStream::rest`] is normal here, not a framing failure: a type-6
/// (update node) record is terminal because its body is the node's new value, encoded
/// per the owning component's schema, which nothing on the wire describes.
///
/// In practice almost all traffic on this element is a single type-6 record on node 1,
/// whose value is a little-endian `u32` **server tick**, sent once per tick at
/// BigWorld's default 10 Hz `updateHertz` -- the client acknowledging the server clock
/// rather than replicating component data. The counter tracks server uptime, not the
/// local session, so a discontinuity in it means the server side restarted.
#[derive(Debug, Clone)]
pub struct NrlMsgToCell(pub NrlStream);

impl SimpleCodec for NrlMsgToCell {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        self.0.write_with(write, false)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        NrlStream::read_with(read, None).map(Self)
    }
}

impl SimpleElement for NrlMsgToCell {
    const ID: u8 = id::NRL_MSG_TO_CELL;
    const LEN: ElementLength = ElementLength::Variable16;
}

// =============================================================================
// Ward avatar updates & session control
// =============================================================================

/// The [`AvatarUpdateImplicit`] form for a *ward*: an entity the client drives besides
/// its own avatar, such as a towed or remotely controlled vehicle.
///
/// Twenty bytes, four more than [`AvatarUpdateImplicit`]'s payload, which is consistent
/// with an extra entity id naming the ward. Body not decoded, and never seen on the
/// wire -- no observed battle has given the client a second entity to drive.
pub type AvatarUpdateWardImplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_IMPLICIT }, 20>;
/// The [`AvatarUpdateExplicit`] form for a ward, stating the vehicle it is mounted on.
/// Relates to [`AvatarUpdateWardImplicit`] exactly as [`AvatarUpdateExplicit`] does to
/// [`AvatarUpdateImplicit`]. Body not decoded, and never seen on the wire.
pub type AvatarUpdateWardExplicit = DebugElementFixed<{ id::AVATAR_UPDATE_WARD_EXPLICIT }, 29>;
/// Acknowledges a server physics correction applied to a ward, the ward counterpart of
/// [`AckPhysicsCorrection`].
///
/// Four bytes, presumably the ward's entity id. Body not decoded, and never seen on the
/// wire.
pub type AckWardPhysicsCorrection = DebugElementFixed<{ id::ACK_WARD_PHYSICS_CORRECTION }, 4>;
/// Confirms the client has restored its state in response to
/// [`crate::app::client::element::RestoreClient`].
///
/// Body not decoded, and never seen on the wire: it is only sent in answer to a restore,
/// which needs a server-side failure to trigger.
pub type RestoreClientAck = DebugElementFixed<{ id::RESTORE_CLIENT_ACK }, 4>;

/// Keeps the client -> BaseApp connection alive. Carries no bytes: its presence in the
/// bundle is the whole message, so there is nothing left to decode.
///
/// Zero-length, like [`crate::app::client::element::NrlUpdateNodeFlag`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientToServerHeartbeat;

impl SimpleCodec for ClientToServerHeartbeat {
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> { Ok(()) }
    fn read(_read: &mut dyn Read) -> io::Result<Self> { Ok(Self) }
}

impl SimpleElement for ClientToServerHeartbeat {
    const ID: u8 = id::CLIENT_TO_SERVER_HEARTBEAT;
    const LEN: ElementLength = ElementLength::ZERO;
}

/// Marks the bundle as destined for the cell rather than the base. Payload-less, exactly
/// as [`ClientToServerHeartbeat`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SendToCell;

impl SimpleCodec for SendToCell {
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> { Ok(()) }
    fn read(_read: &mut dyn Read) -> io::Result<Self> { Ok(Self) }
}

impl SimpleElement for SendToCell {
    const ID: u8 = id::SEND_TO_CELL;
    const LEN: ElementLength = ElementLength::ZERO;
}

// =============================================================================
// Dynamic entity method dispatch (base & cell)
// =============================================================================

/// A base-directed entity method call, decoded dynamically against a runtime-computed
/// [`MethodDef`] table (see [`crate::app::script::EntityDispatch`]) resolved from the
/// loaded script model -- lets [`super::App`] dispatch by name against whatever game
/// version's script was loaded. Read-only: this is only ever received, from the client.
/// 
/// Base method is always done against the current player entity id, and so we know in
/// advance the method dispatch table. This is not the case for cell methods below!
#[derive(Debug, Clone)]
pub struct BaseEntityMethod {
    pub call: MethodCall,
}

impl Element<[MethodDef]> for BaseEntityMethod {

    fn write_length(&self, _config: &[MethodDef]) -> io::Result<ElementLength> {
        unreachable!("BaseEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &[MethodDef]) -> io::Result<u8> {
        unreachable!("BaseEntityMethod is read-only")
    }

    fn read_length(_config: &[MethodDef], _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &[MethodDef], len: usize, id: u8) -> io::Result<Self> {

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
            Some(def) => MethodCall::Known { exposed_id, name: def.name.clone(), args: def.read_args(read)? },
            None => {
                let mut data = vec![0; len];
                read.read_exact(&mut data)?;
                MethodCall::Unknown { exposed_id, data }
            }
        };

        Ok(Self { call })

    }

}


/// Resolves the exposed cell-method table a client-to-cell call should be decoded
/// against.
///
/// A cell call names its target entity on the wire, and different entity *types* have
/// different exposed tables, so the element id alone does not identify the method: the
/// same id means `Avatar::activateEquipment` for one target and
/// `Vehicle::trackRelativePointWithGun` for another. The target must therefore be
/// resolved before the call can be decoded; in a battle most calls are aimed at the
/// player's vehicle rather than the player entity, since gun aiming is sent every tick.
pub trait CellMethodTables {
    /// Exposed cell methods for `entity_id`, or `None` if the target is unknown (the
    /// call is then reported raw rather than guessed at). `0` is not a real id: it means
    /// "the client's own entity", which the base app substitutes.
    fn cell_methods(&self, entity_id: u32) -> Option<&[MethodDef]>;
}

/// Same as [`BaseEntityMethod`], but for a cell-directed call. The client sends these to
/// the base app (there's no direct client-to-cell connection), which forwards them to the
/// cell app in real BigWorld; this project has no `cell::App` yet, so decoding one only
/// recovers the call itself, not anything about forwarding it.
#[derive(Debug, Clone)]
pub struct CellEntityMethod {
    /// Target entity, streamed *before* the method's sub-id and arguments. `0` is the
    /// common case and is not a real id: it means "the client's own entity", which the
    /// base app substitutes (`Proxy::cellEntityMethod`: `if (entityID == 0) entityID =
    /// id_;`). Kept verbatim rather than resolved, since the connection's own entity id
    /// is not this element's to know.
    pub entity_id: u32,
    pub call: MethodCall,
}

impl<'t> Element<dyn CellMethodTables + 't> for CellEntityMethod {

    fn write_length(&self, _config: &(dyn CellMethodTables + 't)) -> io::Result<ElementLength> {
        unreachable!("CellEntityMethod is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &(dyn CellMethodTables + 't)) -> io::Result<u8> {
        unreachable!("CellEntityMethod is read-only")
    }

    fn read_length(_config: &(dyn CellMethodTables + 't), _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &(dyn CellMethodTables + 't), len: usize, id: u8) -> io::Result<Self> {

        if !id::CELL_ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected cell entity method element id: {id:02X}")));
        }

        // The target entity id leads the body, before the sub-id and the arguments. Only
        // the *cell* form carries it: `Proxy::cellEntityMethod` opens with `data >>
        // entityID;` and forwards the rest, whereas `Proxy::baseEntityMethod` goes
        // straight to `exposedMethodFromMsgID` -- so this must not be copied to
        // [`BaseEntityMethod`]. The sub-id is read after it, on the cell app, by
        // `ExposedMethodMessageRange::exposedIDFromMsgID`, which is why the order here is
        // entity id first and sub-id second.
        //
        // Missing this consumed the (almost always zero) entity id *as the arguments* and
        // left the real ones unread as trailing bytes -- every client-to-cell call was
        // silently decoded with shifted, wrong arguments.
        let entity_id = read.read_u32()?;
        let mut len = len.saturating_sub(4);

        // Pick the table by target, not by the connection's own entity (see
        // [`CellMethodTables`]). An unknown target is reported raw: guessing with some
        // other entity's table is exactly the failure this replaced.
        let Some(config) = config.cell_methods(entity_id) else {
            let mut data = vec![0; len];
            read.read_exact(&mut data)?;
            return Ok(Self {
                entity_id,
                call: MethodCall::Unknown { exposed_id: (id - id::CELL_ENTITY_METHOD.first) as u16, data },
            });
        };

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
            Some(def) => MethodCall::Known { exposed_id, name: def.name.clone(), args: def.read_args(read)? },
            None => {
                let mut data = vec![0; len];
                read.read_exact(&mut data)?;
                MethodCall::Unknown { exposed_id, data }
            }
        };

        Ok(Self { entity_id, call })

    }

}


#[cfg(test)]
mod tests {

    use super::*;
    use crate::app::client::element::NrlBody;

    /// `requestEntityUpdate` streams its stamps with no count: the element length is the
    /// only bound, so the stamp count is `(len - 4) / 4`.
    #[test]
    fn request_entity_update_reads_stamps_to_the_end() {

        // entity 0x01020304, then three int32 stamps (the last negative, since
        // `EventNumber` is signed).
        let data = [
            0x04, 0x03, 0x02, 0x01,
            0x0a, 0x00, 0x00, 0x00,
            0xff, 0xff, 0xff, 0x7f,
            0xff, 0xff, 0xff, 0xff,
        ];
        let r = <RequestEntityUpdate as SimpleCodec>::read(&mut &data[..]).unwrap();

        assert_eq!(r.entity_id, 0x01020304);
        assert_eq!(r.stamps, [10, i32::MAX, -1]);
        assert!(r.rest.is_empty(), "rest: {:?}", r.rest);

        // No stamps at all is legal: it asks for the entity's whole state.
        let bare = <RequestEntityUpdate as SimpleCodec>::read(&mut &data[..4]).unwrap();
        assert_eq!(bare.entity_id, 0x01020304);
        assert!(bare.stamps.is_empty() && bare.rest.is_empty());

        // A partial trailing stamp is reported, not silently dropped and not an error.
        let ragged = <RequestEntityUpdate as SimpleCodec>::read(&mut &data[..10]).unwrap();
        assert_eq!(ragged.stamps, [10]);
        assert_eq!(ragged.rest, [0xff, 0xff]);

    }

    /// [`NrlMsgToCell`] frames the generic `[type][node id][body]` stream, with no
    /// implied leading type -- the one assumption this direction rests on.
    #[test]
    fn nrl_msg_to_cell_frames_the_generic_stream() {

        // type 5 (lookup) on node 0x002a, then type 2 (unlink) on node 0x002b. Neither
        // is terminal, so both must frame and nothing may be left over.
        let data = [0x05, 0x00, 0x2a, 0x02, 0x00, 0x2b];
        let NrlMsgToCell(stream) = <NrlMsgToCell as SimpleCodec>::read(&mut &data[..]).unwrap();

        assert_eq!(stream.messages.len(), 2);
        assert_eq!(stream.messages[0].node_id, 0x2a);
        assert!(matches!(stream.messages[0].body, NrlBody::Lookup));
        assert_eq!(stream.messages[1].node_id, 0x2b);
        assert!(matches!(stream.messages[1].body, NrlBody::UnlinkTree));
        assert!(stream.rest.is_empty(), "rest: {:?}", stream.rest);

    }

}
