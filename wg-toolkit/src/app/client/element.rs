//! Definition of the elements that can be sent from server to client
//! once connected to the base application..

use std::io::{self, Read, Write};
use std::collections::HashMap;
use std::borrow::Cow;
use std::sync::Arc;
use std::fmt;

use tracing::warn;
use glam::Vec3;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::util::io::{WgReadExt, WgWriteExt, serde_pickle_de_options, serde_pickle_ser_options};
use crate::app::dispatch::{ScriptDispatch, EntityDispatch, MethodDef, PropertyDef, MethodCall};
use crate::net::codec::{Codec, SimpleCodec, WgSocketAddrV4};
use crate::script::{Ty, TyKind, TyDict, Value, PythonValue};
use crate::app::bit::{BitReader, BitWriter};
use crate::util::AsciiFmt;

pub use crate::app::math::{PackedXyz, PackedXz, PackedYawPitch, PackedYawPitchRoll, PackedYaw};


/// Internal module containing all raw elements numerical ids.
pub mod id {

    use crate::net::element::ElementIdRange;

    // --- Connection handshake & session bookkeeping ---
    pub const AUTHENTICATE: u8                                          = 0x00;  // FIXED 4
    pub const BANDWIDTH_NOTIFICATION: u8                                = 0x01;  // FIXED 4
    pub const UPDATE_FREQUENCY_NOTIFICATION: u8                         = 0x02;  // FIXED 7
    pub const SET_GAME_TIME: u8                                         = 0x03;  // FIXED 4
    pub const RESET_ENTITIES: u8                                        = 0x04;  // FIXED 1
    // --- Player entity creation (base & cell) ---
    pub const CREATE_BASE_PLAYER: u8                                    = 0x05;  // VAR 2
    pub const CREATE_CELL_PLAYER: u8                                    = 0x06;  // VAR 2

    // --- Spaces & general entity lifecycle (creation, AoI enter/leave) ---
    pub const DUMMY_PACKET: u8                                          = 0x07;  // VAR 2
    pub const SPACE_PROPERTY: u8                                        = 0x08;  // VAR 2
    pub const ADD_SPACE_GEOMETRY_MAPPING: u8                            = 0x09;  // VAR 2
    pub const REMOVE_SPACE_GEOMETRY_MAPPING: u8                         = 0x0A;  // VAR 2
    pub const CREATE_ENTITY: u8                                         = 0x0B;  // VAR 2
    pub const CREATE_ENTITY_DETAILED: u8                                = 0x0C;  // VAR 2

    // --- Cell suspend/resume & client suspension detection (WoT additions, not in
    // vanilla BigWorld) ---
    pub const CELL_APP_SUSPENDED: u8                                    = 0x0D;  // FIXED 0
    pub const CELL_APP_RESUMED: u8                                      = 0x0E;  // FIXED 0
    pub const CLIENT_SUSPENSION_DETECTION_ENABLED: u8                   = 0x0F;  // FIXED 4

    // --- Area of Interest enter/leave ---
    pub const ENTER_AOI: u8                                             = 0x10;  // FIXED 5
    pub const ENTER_AOI_ON_VEHICLE: u8                                  = 0x11;  // FIXED 9
    pub const LEAVE_AOI: u8                                             = 0x12;  // VAR 2

    // --- Timing, positioning references & entity selection ---
    pub const TICK_SYNC: u8                                             = 0x13;  // FIXED 1
    pub const TICK_SYNC_PERIODIC: u8                                    = 0x14;  // FIXED 2
    pub const RELATIVE_POSITION_REFERENCE: u8                           = 0x15;  // FIXED 1
    pub const RELATIVE_POSITION: u8                                     = 0x16;  // FIXED 12
    pub const SET_VEHICLE: u8                                           = 0x17;  // FIXED 8
    pub const SELECT_ALIASED_ENTITY: u8                                 = 0x18;  // FIXED 1
    pub const SELECT_ENTITY: u8                                         = 0x19;  // FIXED 4
    pub const SELECT_PLAYER_ENTITY: u8                                  = 0x1A;  // FIXED 0
    pub const FORCED_POSITION: u8                                       = 0x1B;  // FIXED 38

    // --- Avatar detailed updates & volatile properties (see also 0x29-0x40 below) ---
    pub const AVATAR_UPDATE_NO_ALIAS_DETAILED: u8                       = 0x1C;  // FIXED 29
    pub const AVATAR_UPDATE_ALIAS_DETAILED: u8                          = 0x1D;  // FIXED 26
    pub const AVATAR_UPDATE_PLAYER_DETAILED: u8                         = 0x1E;  // FIXED 25
    pub const AVATAR_UPDATE_VOLATILE_PROPERTIES: u8                     = 0x1F;  // VAR 2
    pub const CHANGE_VOLATILE_PACKER_TYPE: u8                           = 0x20;  // VAR 2

    // --- Network Replication Layer ("NRL"): WoT's own CGF node-replication messages
    // (`NetworkReplicationPointComponent.py`), not present in vanilla BigWorld. ---
    pub const NRL_CREATE_NODE: u8                                       = 0x21;  // VAR 2
    pub const NRL_UNLINK_TREE: u8                                       = 0x22;  // VAR 2
    pub const NRL_UPDATE_NODE: u8                                       = 0x23;  // VAR 2
    pub const NRL_UNLINK_TREE_FLAG: u8                                  = 0x24;  // FIXED 0
    pub const NRL_UPDATE_NODE_FLAG: u8                                  = 0x25;  // FIXED 0
    pub const NRL_DATA: u8                                              = 0x26;  // VAR 2
    pub const NRL_MSG_TO_CLIENT: u8                                     = 0x27;  // VAR 2
    pub const NRL_UNRELIABLE_MSG_TO_CLIENT: u8                          = 0x28;  // VAR 2

    // --- Avatar movement updates (`AVUPMSG` combinatorial family), continued from 0x1C ---
    // The 24 AVUPMSG combinations (see `common_client_interface.hpp` in the leaked
    // BigWorld 14.4.1 SDK, `re-work/bigworld-src-14.4.1/`): each combination of
    // {NoAlias 4-byte EntityID, Alias 1-byte IDAlias} x {FullPos 5-byte PackedXYZ,
    // OnGround 3-byte PackedXZ, NoPos none} x {YawPitchRoll 3 bytes, YawPitch 2 bytes,
    // Yaw 1 byte, NoDir none} suggests a fixed-size message per id (id field + pos field
    // + dir field, in that order) -- but all 24 of these ids (and the
    // `entityMethod`/`entityProperty` ranges below) are registered with
    // `lengthStyle=CALLBACK`, meaning the client computes each one's byte length through
    // a dedicated per-message-handler function rather than storing one constant.
    //
    // The `Fixed(N)` values below are that function's ground truth, read out of the
    // client's own code (vtable slot 3 on each id's handler object, a pure function of
    // `this`), not estimated from captures. They do not match the vanilla SDK formula
    // above, which undercounts every one of them. See the doc comment on the
    // `avatar_update_elements!` macro for the per-id byte layouts.
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8        = 0x29;  // CALLBACK, Fixed(15)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH: u8             = 0x2A;  // CALLBACK, Fixed(13)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW: u8                   = 0x2B;  // CALLBACK, Fixed(12)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR: u8                = 0x2C;  // CALLBACK, Fixed(11)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8       = 0x2D;  // CALLBACK, Fixed(12)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH: u8            = 0x2E;  // CALLBACK, Fixed(10)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW: u8                  = 0x2F;  // CALLBACK, Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR: u8               = 0x30;  // CALLBACK, Fixed(8)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL: u8          = 0x31;  // CALLBACK, Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH: u8               = 0x32;  // CALLBACK, Fixed(7)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW: u8                     = 0x33;  // CALLBACK, Fixed(6)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR: u8                  = 0x34;  // CALLBACK, Fixed(5)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8           = 0x35;  // CALLBACK, Fixed(12)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH: u8                = 0x36;  // CALLBACK, Fixed(10)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW: u8                      = 0x37;  // CALLBACK, Fixed(9)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR: u8                   = 0x38;  // CALLBACK, Fixed(8)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8          = 0x39;  // CALLBACK, Fixed(9)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH: u8               = 0x3A;  // CALLBACK, Fixed(7)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW: u8                     = 0x3B;  // CALLBACK, Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR: u8                  = 0x3C;  // CALLBACK, Fixed(5) -- not the Fixed(4) the vanilla formula gives
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL: u8             = 0x3D;  // CALLBACK, Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH: u8                  = 0x3E;  // CALLBACK, Fixed(4)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW: u8                        = 0x3F;  // CALLBACK, Fixed(3)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR: u8                     = 0x40;  // CALLBACK, Fixed(2)

    // --- Entity control, voice & session hand-off ---
    pub const CONTROL_ENTITY: u8                                        = 0x41;  // FIXED 5
    pub const VOICE_DATA: u8                                            = 0x42;  // VAR 2
    pub const RESTORE_CLIENT: u8                                        = 0x43;  // VAR 2
    pub const SWITCH_BASE_APP: u8                                       = 0x44;  // FIXED 9

    // --- Resource download (fonts, sounds, etc. streamed on demand) ---
    pub const RESOURCE_HEADER: u8                                       = 0x45;  // VAR 2
    pub const RESOURCE_FRAGMENT: u8                                     = 0x46;  // VAR 2

    // --- Session teardown & raw entity property/position streaming ---
    pub const LOGGED_OFF: u8                                            = 0x47;  // FIXED 1
    pub const DETAILED_POSITION: u8                                     = 0x48;  // FIXED 24
    pub const NESTED_ENTITY_PROPERTY: u8                                = 0x49;  // VAR 2
    pub const SLICE_ENTITY_PROPERTY: u8                                 = 0x4A;  // VAR 2
    pub const UPDATE_ENTITY: u8                                         = 0x4B;  // VAR 2
    pub const SET_CELL_APP_EXT_ADDRESS: u8                              = 0x4C;  // VAR 2
    pub const LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION: u8   = 0x4D;  // FIXED 0

    // --- Dynamic entity method/property dispatch (script-model-driven) ---
    pub const ENTITY_METHOD: ElementIdRange     = ElementIdRange::new(0x4E, 0xA6);  // CALLBACK 0
    pub const ENTITY_PROPERTY: ElementIdRange   = ElementIdRange::new(0xA7, 0xFE);  // CALLBACK 0

}

/// The wire length of a client element by id, from the table above, for reading past
/// elements one doesn't decode. `None` for the entity method and property ranges,
/// whose lengths depend on the selected entity's script model.
pub fn known_length(id: u8) -> Option<ElementLength> {
    use id::*;
    Some(match id {
        CELL_APP_SUSPENDED | CELL_APP_RESUMED | SELECT_PLAYER_ENTITY | NRL_UNLINK_TREE_FLAG | NRL_UPDATE_NODE_FLAG | LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION => ElementLength::Fixed(0),
        RESET_ENTITIES | TICK_SYNC | RELATIVE_POSITION_REFERENCE | SELECT_ALIASED_ENTITY | LOGGED_OFF => ElementLength::Fixed(1),
        AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH | AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH => ElementLength::Fixed(10),
        AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR => ElementLength::Fixed(11),
        RELATIVE_POSITION | AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW | AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL | AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL => ElementLength::Fixed(12),
        AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH => ElementLength::Fixed(13),
        AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL => ElementLength::Fixed(15),
        TICK_SYNC_PERIODIC | AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR => ElementLength::Fixed(2),
        DETAILED_POSITION => ElementLength::Fixed(24),
        AVATAR_UPDATE_PLAYER_DETAILED => ElementLength::Fixed(25),
        AVATAR_UPDATE_ALIAS_DETAILED => ElementLength::Fixed(26),
        AVATAR_UPDATE_NO_ALIAS_DETAILED => ElementLength::Fixed(29),
        AVATAR_UPDATE_ALIAS_NO_POS_YAW => ElementLength::Fixed(3),
        FORCED_POSITION => ElementLength::Fixed(38),
        AUTHENTICATE | BANDWIDTH_NOTIFICATION | SET_GAME_TIME | CLIENT_SUSPENSION_DETECTION_ENABLED | SELECT_ENTITY | AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH => ElementLength::Fixed(4),
        ENTER_AOI | AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR | AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR | CONTROL_ENTITY => ElementLength::Fixed(5),
        AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW | AVATAR_UPDATE_ALIAS_ON_GROUND_YAW | AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL => ElementLength::Fixed(6),
        UPDATE_FREQUENCY_NOTIFICATION | AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH | AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH => ElementLength::Fixed(7),
        SET_VEHICLE | AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR | AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR => ElementLength::Fixed(8),
        ENTER_AOI_ON_VEHICLE | AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW | AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL | AVATAR_UPDATE_ALIAS_FULL_POS_YAW | AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL | SWITCH_BASE_APP => ElementLength::Fixed(9),
        CREATE_BASE_PLAYER | CREATE_CELL_PLAYER | DUMMY_PACKET | SPACE_PROPERTY | ADD_SPACE_GEOMETRY_MAPPING | REMOVE_SPACE_GEOMETRY_MAPPING | CREATE_ENTITY | CREATE_ENTITY_DETAILED | LEAVE_AOI | AVATAR_UPDATE_VOLATILE_PROPERTIES | CHANGE_VOLATILE_PACKER_TYPE | NRL_CREATE_NODE | NRL_UNLINK_TREE | NRL_UPDATE_NODE | NRL_DATA | NRL_MSG_TO_CLIENT | NRL_UNRELIABLE_MSG_TO_CLIENT | VOICE_DATA | RESTORE_CLIENT | RESOURCE_HEADER | RESOURCE_FRAGMENT | NESTED_ENTITY_PROPERTY | SLICE_ENTITY_PROPERTY | UPDATE_ENTITY | SET_CELL_APP_EXT_ADDRESS => ElementLength::Variable16,
        _ => return None,
    })
}


// =============================================================================
// Connection handshake & session bookkeeping
// =============================================================================

crate::__struct_simple_codec! {
    #[derive(Debug, Clone)]
    pub struct Authenticate {
        pub key: u32,
    }
}

impl SimpleElement for Authenticate {
    const ID: u8 = id::AUTHENTICATE;
    const LEN: ElementLength = ElementLength::Fixed(4);
}


crate::__struct_simple_codec! {
    #[derive(Debug, Clone)]
    pub struct BandwidthNotification {
        pub bps: u32,
    }
}

impl SimpleElement for BandwidthNotification {
    const ID: u8 = id::BANDWIDTH_NOTIFICATION;
    const LEN: ElementLength = ElementLength::Fixed(4);
}


crate::__struct_simple_codec! {
    /// The server informs us how frequently it is going to send update
    /// the the client, and also give the server game time (exactly the
    /// same as [`SetGameTime`] element, but inlined here).
    #[derive(Debug, Clone)]
    pub struct UpdateFrequencyNotification {
        /// The frequency in hertz.
        pub frequency: u8,
        /// Unknown value!
        pub unknown: u16,
        /// The server game time.
        pub game_time: u32,
    }
}

impl SimpleElement for UpdateFrequencyNotification {
    const ID: u8 = id::UPDATE_FREQUENCY_NOTIFICATION;
    const LEN: ElementLength = ElementLength::Fixed(7);
}


crate::__struct_simple_codec! {
    /// The server informs us of the current (server) game time.
    #[derive(Debug, Clone)]
    pub struct SetGameTime {
        /// The server game time.
        pub game_time: u32,
    }
}

impl SimpleElement for SetGameTime {
    const ID: u8 = id::SET_GAME_TIME;
    const LEN: ElementLength = ElementLength::Fixed(4);
}


crate::__struct_simple_codec! {
    /// The server wants to resets the entities in the Area of Interest (AoI).
    #[derive(Debug, Clone)]
    pub struct ResetEntities {
        pub keep_player_on_base: bool,
    }
}

impl SimpleElement for ResetEntities {
    const ID: u8 = id::RESET_ENTITIES;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


// =============================================================================
// Player entity creation (base & cell)
// =============================================================================

/// Sent from the base to give the client's already-created base-player entity (see
/// [`CreateBasePlayer`]) a cell-side presence too, e.g. it has entered a space/battle.
///
/// Field order confirmed against vanilla BigWorld's `Witness::Witness`
/// (`cellapp/witness.cpp`, `CREATE_REAL_FROM_INIT` case) building this exact message,
/// which the base app then forwards to the client byte-for-byte
/// (`Proxy::createCellPlayer` in `baseapp/proxy.cpp` is a raw passthrough, not a
/// re-encode). No entity id is written -- like [`CreateBasePlayer`], this message only
/// ever targets the one player entity the client already has (base and cell slices of an
/// entity share one id, `lib/network/basictypes.hpp`), so `base::App::create_cell_player`
/// takes the existing base [`Handle`](super::super::base::Handle) instead of minting one.
///
/// The layout follows the client's own `ServerConnection::createCellPlayer` handler, and
/// is not a byte-for-byte match for vanilla 14.4.1's `stream >> spaceID_ >> vehicleID >>
/// pos >> packedXZScale_ >> dir`: like [`CreateBasePlayer`], WoT adds fields. It inserts
/// a leading byte and a 2-byte field between `space_id` and `vehicle_id`. `packed_xz_scale` is
/// constant per server configuration, identical across battles.
///
/// `direction` is BigWorld's `Direction3D`, streamed as roll, pitch then yaw
/// (`network/basictypes.cpp`); in a live battle only the last one, the heading, is set.
///
/// The trailing `cell_data` is the player entity's cell-hosted, client-visible properties
/// (`Witness::Witness` streams `FROM_CELL_TO_CLIENT_DATA`), which only the entity's type
/// can decode, so it is kept raw here and decoded on demand with
/// [`Self::read_cell_data`].
#[derive(Debug, Clone)]
pub struct CreateCellPlayer {
    /// Always `0` in observed traffic; meaning unknown.
    pub unk_flag: u8,
    /// The id of the space this entity now lives in.
    pub space_id: u32,
    /// Always `0` in observed traffic; meaning unknown.
    pub unk_short: u16,
    /// The id of a distinct `Vehicle` entity, not the player's own base entity. A later
    /// `SelectEntity` targets this id for the vehicle's own property and method updates
    /// (see `wg-toolkit-cli`'s proxy `CreateCellPlayer` handling, which registers it).
    /// For an `Avatar`, the same value is its `playerVehicleID` property in `cell_data`.
    pub vehicle_id: u32,
    pub position: Vec3,
    /// The server's packed-XZ compression scale, needed by the client to decode any
    /// later packed-XZ position updates -- this project doesn't send those (no compressed
    /// `AvatarUpdate*` elements are implemented), so this value has no effect today.
    pub packed_xz_scale: f32,
    /// Roll, pitch and yaw, in this order.
    pub direction: Vec3,
    /// The player entity's cell properties, encoded -- see [`Self::read_cell_data`].
    pub cell_data: Vec<u8>,
}

impl CreateCellPlayer {

    /// Decode [`Self::cell_data`] against the player entity's own type: its cell-hosted,
    /// client-visible properties, then a count byte, then those of its static
    /// components. Bytes left over after that are an error, the layout did not fit.
    pub fn read_cell_data(&self, dispatch: &EntityDispatch) -> io::Result<CellPlayerData> {
        let mut read = &self.cell_data[..];
        let properties = Value::read(&mut read, &dispatch.cell_data_ty)?;
        let components_count = read.read_u8()?;
        let components_properties = Value::read(&mut read, &dispatch.cell_components_data_ty)?;
        if !read.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("{} bytes left after the cell data", read.len())));
        }
        Ok(CellPlayerData { properties, components_count, components_properties })
    }

}

/// The decoded [`CreateCellPlayer::cell_data`].
#[derive(Debug, Clone)]
pub struct CellPlayerData {
    /// The entity's own cell-hosted, client-visible properties, as a dict.
    pub properties: Value,
    /// Matches the number of the entity's static components in the one live sample (2,
    /// for `Avatar`), which is also `CreateBasePlayer`'s `entity_components_count` for
    /// that same entity, so which components it counts is not settled.
    pub components_count: u8,
    /// The static components' cell-hosted, client-visible properties, as one dict.
    pub components_properties: Value,
}

impl SimpleCodec for CreateCellPlayer {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(self.unk_flag)?;
        write.write_u32(self.space_id)?;
        write.write_u16(self.unk_short)?;
        write.write_u32(self.vehicle_id)?;
        write.write_vec3(self.position)?;
        write.write_f32(self.packed_xz_scale)?;
        write.write_vec3(self.direction)?;
        write.write_all(&self.cell_data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let unk_flag = read.read_u8()?;
        let space_id = read.read_u32()?;
        let unk_short = read.read_u16()?;
        let vehicle_id = read.read_u32()?;
        let position = read.read_vec3()?;
        let packed_xz_scale = read.read_f32()?;
        let direction = read.read_vec3()?;
        let mut cell_data = Vec::new();
        read.read_to_end(&mut cell_data)?;
        Ok(Self { unk_flag, space_id, unk_short, vehicle_id, position, packed_xz_scale, direction, cell_data })
    }

}

impl SimpleElement for CreateCellPlayer {
    const ID: u8 = id::CREATE_CELL_PLAYER;
    const LEN: ElementLength = ElementLength::Variable16;
}


/// Sent from the base when a player should be created, the entity id is given with its
/// type. The remaining data initializes its properties (e.g. the `Login` entity receives
/// the account UID) -- encoded/decoded as a runtime [`Value`] against a runtime-computed
/// [`Ty`] (see [`crate::app::script::EntityDispatch::data_ty`]), resolved dynamically from
/// the loaded script model rather than a statically generated `Entity` struct.
///
/// Generic over how that `Ty` gets resolved, via two separate [`Element`] impls: against a
/// full [`ScriptDispatch`] by reading `entity_type_id` off the wire first (for a generic
/// wire observer that doesn't know the entity type ahead of time, e.g. this project's
/// debugging proxy -- read-only, since such an observer never constructs one of these
/// itself), or directly against an already-resolved [`EntityDispatch`] (for
/// [`super::super::base::App`], which always knows exactly which entity it's creating --
/// write-only, since it never decodes one of these itself).
#[derive(Debug, Clone)]
pub struct CreateBasePlayer<'a> {
    /// The unique identifier of the entity being created.
    pub entity_id: u32,
    /// The entity type id.
    pub entity_type_id: u16,
    /// The actual data sent for creating the player's entity -- borrowed when writing
    /// (the caller already owns it), owned when read off the wire.
    pub entity_data: Cow<'a, Value>,
    /// The number of *dynamic* components attached to this specific entity instance.
    /// WoT's static components fold their methods and properties into the entity's own
    /// tables, so they need no runtime handling and are not counted here. Dynamic
    /// components are attached to individual instances at runtime (e.g. for one battle
    /// mode) and claim no fixed exposed id.
    ///
    /// Nonzero counts are normal (an `Account` carries 19), yet no component data follows
    /// the count: the element ends right after it. The components themselves are
    /// replicated separately, over the NRL protocol (see the `Nrl*` elements).
    pub entity_components_count: u8,
}

impl CreateBasePlayer<'_> {

    /// Read everything but `entity_data` (which needs a resolved [`Ty`] the two
    /// [`Element`] impls below each get differently), returning it alongside the
    /// remaining trailer so each impl only has to plug in its own `data_ty` resolution.
    fn read_prefix(read: &mut dyn Read) -> io::Result<(u32, u16)> {
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let unk = read.read_blob_variable()?;
        if !unk.is_empty() {
            warn!("non-empty unknown blob when decoding CreateBasePlayer: {unk:?}");
        }
        Ok((entity_id, entity_type_id))
    }

    fn read_suffix(read: &mut dyn Read, entity_id: u32, entity_type_id: u16, data_ty: &Ty) -> io::Result<Self> {
        let entity_data = Value::read(read, data_ty)?;
        let entity_components_count = read.read_u8()?;
        // Nothing follows a nonzero count (see `entity_components_count`), so leftover
        // bytes would mean the layout above does not fit this payload.
        if entity_components_count != 0 {
            let mut trailer = Vec::new();
            read.read_to_end(&mut trailer)?;
            if !trailer.is_empty() {
                warn!("CreateBasePlayer: entity_id={entity_id} entity_components_count={entity_components_count}, \
                    {} unexpected trailing bytes: {trailer:?}", trailer.len());
            }
            return Ok(Self {
                entity_id,
                entity_type_id,
                entity_data: Cow::Owned(entity_data),
                entity_components_count,
            });
        }
        Ok(Self {
            entity_id,
            entity_type_id,
            entity_data: Cow::Owned(entity_data),
            entity_components_count,
        })
    }

    fn write_to(&self, write: &mut dyn Write, data_ty: &Ty) -> io::Result<u8> {
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_blob_variable(&[])?;  // Unknown blob or string?
        Codec::write(&*self.entity_data, write, data_ty)?;
        write.write_u8(self.entity_components_count)?;
        Ok(id::CREATE_BASE_PLAYER)
    }

}

impl Element<ScriptDispatch> for CreateBasePlayer<'_> {

    fn write_length(&self, _config: &ScriptDispatch) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, config: &ScriptDispatch) -> io::Result<u8> {
        let dispatch = config.entity_from_id(self.entity_type_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unknown entity type id: 0x{:02X}", self.entity_type_id)))?;
        self.write_to(write, &dispatch.data_ty)
    }

    fn read_length(_config: &ScriptDispatch, _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &ScriptDispatch, _len: usize, _id: u8) -> io::Result<Self> {
        let (entity_id, entity_type_id) = Self::read_prefix(read)?;
        let dispatch = config.entity_from_id(entity_type_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unknown entity type id: 0x{entity_type_id:02X}")))?;
        Self::read_suffix(read, entity_id, entity_type_id, &dispatch.data_ty)
    }

}

impl Element<EntityDispatch> for CreateBasePlayer<'_> {

    fn write_length(&self, _config: &EntityDispatch) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, config: &EntityDispatch) -> io::Result<u8> {
        self.write_to(write, &config.data_ty)
    }

    fn read_length(_config: &EntityDispatch, _id: u8) -> io::Result<ElementLength> {
        unreachable!("CreateBasePlayer<EntityDispatch> is write-only")
    }

    fn read(_read: &mut dyn Read, _config: &EntityDispatch, _len: usize, _id: u8) -> io::Result<Self> {
        unreachable!("CreateBasePlayer<EntityDispatch> is write-only")
    }

}

// =============================================================================
// Spaces & general entity lifecycle (creation, AoI enter/leave)
// =============================================================================

/// Filler the server can put in a bundle to pad it out; carries no game state.
///
/// Body not decoded, and never seen on the wire, so even its length style is only
/// inferred from the id table.
pub type DummyPacket = DebugElementVariable16<{ id::DUMMY_PACKET }>;
/// One space-data record: a property of the space the player is in.
///
/// Three arrive together on entering a space, each beginning with the space id followed
/// by a field taking the values 0, 1 and 2. Those are the values vanilla BigWorld uses
/// for `SPACE_DATA_TOD_KEY`, `SPACE_DATA_MAPPING_KEY_CLIENT_SERVER` and
/// `SPACE_DATA_MAPPING_KEY_CLIENT_ONLY` (`common/space_data_types.hpp`), but the
/// payloads are far too short for those keys' structs, so the correspondence is unproven
/// and the field's width is unknown -- `u32`, or `u16` plus two unknown bytes, fit the
/// bytes equally.
///
/// Every payload observed is zero apart from the space id, and identical between
/// different maps, so the records appear to be fixed stubs in this client. Body not
/// decoded. Geometry mappings do *not* arrive here despite the key values above; they
/// have their own element, [`AddSpaceGeometryMapping`].
pub type SpaceProperty = DebugElementVariable16<{ id::SPACE_PROPERTY }>;
/// Maps a geometry (terrain/scene) resource into a space, with the transform to place it.
///
/// Vanilla BigWorld carries this inside the single `spaceData` message, keyed by a
/// `uint16`; WoT split `spaceData` into one element id per key, so there is no key field
/// here -- the id is the key.
///
/// Laid out as `[space_id u32][entry_id 8][packed-len path][transform 16 x f32][unk u8]`.
/// `entry_id` is a `SpaceEntryID`, which derives from `Mercury::Address`
/// (`network/basictypes.hpp`) and so reads through the same [`WgSocketAddrV4`] codec as
/// [`SwitchBaseApp`].
#[derive(Debug, Clone)]
pub struct AddSpaceGeometryMapping {
    pub space_id: u32,
    /// Identifies this entry so a later removal can name it.
    pub entry_id: WgSocketAddrV4,
    /// Resource path, e.g. `"spaces/251_br_battle_city3_hw25"`.
    pub path: String,
    /// The placement transform, 16 floats in wire order.
    ///
    /// Not a `Mat4`, because the row- versus column-major order is unknown: every
    /// observed transform is the identity, which cannot distinguish the two.
    pub transform: [f32; 16],
    /// One trailing byte, meaning unknown; `0x01` in all observed traffic.
    pub unk: u8,
    /// Anything beyond the layout above; empty for a well-formed message. Reported here
    /// rather than raised as an error so a mismatch does not stop the rest of the
    /// bundle from decoding.
    pub rest: Vec<u8>,
}

impl SimpleCodec for AddSpaceGeometryMapping {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u32(self.space_id)?;
        SimpleCodec::write(&self.entry_id, write)?;
        write.write_string_variable(&self.path)?;
        for f in &self.transform {
            write.write_all(&f.to_le_bytes())?;
        }
        write.write_u8(self.unk)?;
        write.write_all(&self.rest)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let space_id = read.read_u32()?;
        let entry_id = SimpleCodec::read(read)?;
        let path = read.read_string_variable_lossy()?;
        let mut buf = [0u8; 64];
        read.read_exact(&mut buf)?;
        let mut transform = [0f32; 16];
        for (slot, chunk) in transform.iter_mut().zip(buf.chunks_exact(4)) {
            *slot = f32::from_le_bytes(chunk.try_into().unwrap());
        }
        let unk = read.read_u8()?;
        let mut rest = Vec::new();
        read.read_to_end(&mut rest)?;
        Ok(Self { space_id, entry_id, path, transform, unk, rest })
    }

}

impl SimpleElement for AddSpaceGeometryMapping {
    const ID: u8 = id::ADD_SPACE_GEOMETRY_MAPPING;
    const LEN: ElementLength = ElementLength::Variable16;
}
/// Unmaps a geometry resource previously attached by [`AddSpaceGeometryMapping`],
/// presumably naming the entry by its `SpaceEntryID`.
///
/// Body not decoded, and never seen on the wire -- unsurprising, since a battle tears
/// its space down by disconnecting rather than by unmapping geometry.
pub type RemoveSpaceGeometryMapping = DebugElementVariable16<{ id::REMOVE_SPACE_GEOMETRY_MAPPING }>;

/// Sent from the cell when another entity (not the client's own player) enters its Area
/// of Interest -- see [`EnterAoi`]/[`EnterAoiOnVehicle`] for the companion message that
/// actually adds it to the AoI id-alias table.
///
/// Field order and types otherwise follow the vanilla
/// `ServerConnection::createEntity` (`connection/server_connection.cpp`), except that
/// WoT inserts two extra fields between `entity_type_id` and `position`:
/// `unk_u16` and `resource_name`, each documented on its own field. The pair is
/// unconditional, present for every entity type, so reading this with the vanilla
/// layout shifts `position`/`direction` 13 bytes early and decodes them from the bytes
/// of `resource_name` instead. [`CreateCellPlayer`] and [`CreateBasePlayer`] diverge
/// from the same vanilla source in the same way.
///
/// The whole payload (`entity_id`/`entity_type_id`/`position`/`direction` included, not
/// just the trailing property dict) is wrapped server-side in a `CompressionIStream`: a
/// 1-byte compression-type tag optionally followed by a zlib-compressed body. This crate
/// has no zlib dependency, so a non-`NONE` tag surfaces as an error here rather than
/// silently misreading compressed bytes as plain fields.
#[derive(Debug, Clone)]
pub struct CreateEntity {
    pub entity_id: u32,
    pub entity_type_id: u16,
    /// Always `0` in observed traffic; meaning unknown (possibly a
    /// component/variant index, always 0 for a plain entity with no dynamic component
    /// attached at creation time). See [`Self::resource_name`] and this struct's own
    /// doc comment for how this was found.
    pub unk_u16: u16,
    /// A packed-length-prefixed string (see [`crate::util::io::WgReadExt::read_packed_u24`])
    /// sent right before `position` -- always `"bw.default"` in every capture seen so
    /// far, for every entity type observed, so most likely a default prefab/resource tag
    /// rather than a per-entity value. Meaning otherwise unconfirmed.
    pub resource_name: String,
    pub position: Vec3,
    /// Yaw/pitch/roll, packed the same way as the `AVATAR_UPDATE_*_YAW_PITCH_ROLL`
    /// family (see [`PackedYawPitchRoll`]) but with `HALFPITCH` forced to `false` here
    /// (confirmed: `ServerConnection::createEntity` explicitly instantiates
    /// `PackedYawPitchRoll</* HALFPITCH */ false>`, unlike the avatar-update messages'
    /// default `HALFPITCH = true`) -- decode with [`PackedYawPitchRoll::unpack`]
    /// passing `half_pitch = false`.
    pub direction: PackedYawPitchRoll,
    /// The byte WoT sends immediately after a packed yaw/pitch/roll, meaning unknown.
    ///
    /// Not part of [`PackedYawPitchRoll`], whose three bytes are confirmed by the fixed
    /// element lengths of the whole `AVATAR_UPDATE_*` family: every `*_YAW_PITCH_ROLL`
    /// variant there is exactly one byte longer than its three packed bytes account for,
    /// and those lengths decode live. `CreateEntity` simply never had the extra byte
    /// modelled, which shifted [`Self::client_data`] one byte early and made every
    /// `CreateEntity` payload undecodable -- `CreateEntityDetailed`, whose direction is
    /// three raw floats with no such byte, was unaffected and decoded fine all along.
    pub direction_unk: u8,
    /// The entity's initial client-visible property values, still encoded -- decode with
    /// [`Self::client_properties`] against this entity type's own property table.
    ///
    /// Carried raw because this element's codec is reached without a dispatch table (the
    /// entity's type id is only read *here*, from the same payload). Not a positional dict
    /// Not a positional dict of every `AllClients` property: `addChangedProperties`
    /// writes a count followed by `(exposed id, value)` pairs, covering only the
    /// properties at the outer detail level whose event stamp is newer than the LoD's.
    pub client_data: Vec<u8>,
}

/// Read a `CompressionIStream`-wrapped payload (BigWorld
/// `lib/network/compression_stream.cpp`): a one-byte `BWCompressionType` tag, then either
/// the plain body (`BW_COMPRESSION_NONE` = 0) or a zlib stream (`BW_COMPRESSION_ZIP_1..9`
/// = 1..=9, the tag being the zlib *level*, which doesn't affect decoding).
///
/// Any other tag value is not a compression type at all -- the real client treats it as
/// fatal (`CRITICAL_MSG "Invalid compression type"`), so in this proxy it almost always
/// means the read position is already desynced rather than that some exotic codec is in
/// use. The error says so, to stop that being misread as a missing feature.
fn read_compressed(read: &mut dyn Read) -> io::Result<Vec<u8>> {
    let compression_type = read.read_u8()?;
    let mut body = Vec::new();
    match compression_type {
        0 => { read.read_to_end(&mut body)?; }
        1..=9 => {
            let mut decoder = flate2::read::ZlibDecoder::new(read);
            decoder.read_to_end(&mut body)?;
        }
        other => return Err(io::Error::new(io::ErrorKind::InvalidData, format!(
            "CompressionIStream: invalid compression type {other} (valid: 0, or 1..=9 for \
             zlib) -- the stream is most likely desynced, not compressed with something new"))),
    }
    Ok(body)
}

impl CreateEntity {

    /// Decode [`Self::client_data`] against this entity type's property table, giving the
    /// entity's initial property values -- the same `(exposed id, value)` run that
    /// [`UpdateEntity`] carries later.
    ///
    /// The table is a parameter rather than a field because this element's codec is
    /// reached before any table can be chosen: [`Self::entity_type_id`] is read *here*.
    pub fn client_properties(&self, properties: &[PropertyDef]) -> ChangedProperties {
        read_changed_properties(&self.client_data, properties)
    }

}

impl SimpleCodec for CreateEntity {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(0)?; // BW_COMPRESSION_NONE
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_u16(self.unk_u16)?;
        write.write_string_variable(&self.resource_name)?;
        write.write_vec3(self.position)?;
        SimpleCodec::write(&self.direction, write)?;
        write.write_u8(self.direction_unk)?;
        write.write_all(&self.client_data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let body = read_compressed(read)?;
        let read = &mut &body[..];
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let unk_u16 = read.read_u16()?;
        let resource_name = read.read_string_variable_lossy()?;
        let position = read.read_vec3()?;
        let direction = SimpleCodec::read(read)?;
        let direction_unk = read.read_u8()?;
        let mut client_data = Vec::new();
        read.read_to_end(&mut client_data)?;
        Ok(Self { entity_id, entity_type_id, unk_u16, resource_name, position, direction, direction_unk, client_data })
    }

}

impl SimpleElement for CreateEntity {
    const ID: u8 = id::CREATE_ENTITY;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// Same as [`CreateEntity`], but with an uncompressed direction instead of a packed
/// [`PackedYawPitchRoll`] -- confirmed against the leaked BigWorld 14.4.1 SDK's vanilla
/// `ServerConnection::createEntityDetailed` (`connection/server_connection.cpp`), which
/// is identical to `createEntity` except `stream >> pos >> yaw >> pitch >> roll` reads
/// three raw `f32`s instead of a `PackedYawPitchRoll`. Same caveats as [`CreateEntity`]:
/// WoT inserts the same extra `unk_u16`/`resource_name` pair between `entity_type_id`
/// and `position` (see [`CreateEntity`]'s doc comment for how this was found), and the
/// whole payload (this struct's fields included) is `CompressionIStream`-wrapped, with
/// only `BW_COMPRESSION_NONE` handled.
#[derive(Debug, Clone)]
pub struct CreateEntityDetailed {
    pub entity_id: u32,
    pub entity_type_id: u16,
    /// See [`CreateEntity::unk_u16`].
    pub unk_u16: u16,
    /// See [`CreateEntity::resource_name`].
    pub resource_name: String,
    pub position: Vec3,
    /// Yaw/pitch/roll as three raw, uncompressed `f32`s (unlike [`CreateEntity::direction`]).
    pub direction: Vec3,
    /// See [`CreateEntity::client_data`].
    pub client_data: Vec<u8>,
}

impl CreateEntityDetailed {

    /// Decode [`Self::client_data`] against this entity type's property table, giving the
    /// entity's initial property values -- the same `(exposed id, value)` run that
    /// [`UpdateEntity`] carries later.
    ///
    /// The table is a parameter rather than a field because this element's codec is
    /// reached before any table can be chosen: [`Self::entity_type_id`] is read *here*.
    pub fn client_properties(&self, properties: &[PropertyDef]) -> ChangedProperties {
        read_changed_properties(&self.client_data, properties)
    }

}

impl SimpleCodec for CreateEntityDetailed {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(0)?; // BW_COMPRESSION_NONE
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_u16(self.unk_u16)?;
        write.write_string_variable(&self.resource_name)?;
        write.write_vec3(self.position)?;
        write.write_vec3(self.direction)?;
        write.write_all(&self.client_data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let body = read_compressed(read)?;
        let read = &mut &body[..];
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let unk_u16 = read.read_u16()?;
        let resource_name = read.read_string_variable_lossy()?;
        let position = read.read_vec3()?;
        let direction = read.read_vec3()?;
        let mut client_data = Vec::new();
        read.read_to_end(&mut client_data)?;
        Ok(Self { entity_id, entity_type_id, unk_u16, resource_name, position, direction, client_data })
    }

}

impl SimpleElement for CreateEntityDetailed {
    const ID: u8 = id::CREATE_ENTITY_DETAILED;
    const LEN: ElementLength = ElementLength::Variable16;
}

// `cellAppSuspended`/`cellAppResumed`/`clientSuspensionDetectionEnabled` (ids
// `0x0D`-`0x0F`) do not exist in the vanilla BigWorld 14.4.1 SDK; like the `Nrl*`
// elements above, they are a later-engine or WoT-specific addition. Their names and
// fixed lengths come from the client's own registered `ClientInterface` message table.
//
// No header describes their field-level semantics, so
// [`ClientSuspensionDetectionEnabled`]'s single field is a plausible reading rather than
// an established one.

crate::__struct_simple_codec! {
    /// Sent by the cell when it is about to be suspended (e.g. for a space/cell
    /// hand-off) -- the client is presumably expected to stop predicting/simulating
    /// entity movement until a matching [`CellAppResumed`] arrives. No fields: this is
    /// purely a state transition signal.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct CellAppSuspended {}
}

impl SimpleElement for CellAppSuspended {
    const ID: u8 = id::CELL_APP_SUSPENDED;
    const LEN: ElementLength = ElementLength::ZERO;
}

crate::__struct_simple_codec! {
    /// Counterpart to [`CellAppSuspended`]: sent when the cell resumes normal operation.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct CellAppResumed {}
}

impl SimpleElement for CellAppResumed {
    const ID: u8 = id::CELL_APP_RESUMED;
    const LEN: ElementLength = ElementLength::ZERO;
}

crate::__struct_simple_codec! {
    /// Enables or configures the client's "suspension detection" -- likely a mechanism
    /// for the client to notice when *itself* has been stalled (OS-level pause, a
    /// debugger breakpoint, a long GC/loading hitch, ...) for long enough that the
    /// resulting time gap shouldn't be treated as a network issue. Only the 4-byte
    /// length is known; the field's type and unit are not, so `threshold` is a plausible
    /// reading (a timeout or period, `f32` seconds or `u32` milliseconds) rather than an
    /// established one. Kept as a raw `u32` so no specific meaning is asserted.
    #[derive(Debug, Clone, Copy)]
    pub struct ClientSuspensionDetectionEnabled {
        pub threshold: u32,
    }
}

impl SimpleElement for ClientSuspensionDetectionEnabled {
    const ID: u8 = id::CLIENT_SUSPENSION_DETECTION_ENABLED;
    const LEN: ElementLength = ElementLength::Fixed(4);
}

crate::__struct_simple_codec! {
    /// Sent when an entity enters the client's Area of Interest -- see [`CreateEntity`]
    /// for the companion message carrying its initial snapshot. Layout confirmed against
    /// the leaked BigWorld 14.4.1 SDK (`connection/client_interface.hpp`'s `enterAoI`
    /// message: `EntityID id; IDAlias idAlias;`), matching the 5-byte length of
    /// `ENTER_AOI`.
    #[derive(Debug, Clone, Copy)]
    pub struct EnterAoi {
        pub entity_id: u32,
        pub id_alias: u8,
    }
}

impl SimpleElement for EnterAoi {
    const ID: u8 = id::ENTER_AOI;
    const LEN: ElementLength = ElementLength::Fixed(5);
}

crate::__struct_simple_codec! {
    /// Like [`EnterAoi`], but for an entity that enters while riding a vehicle (a
    /// passenger). Layout confirmed against the leaked BigWorld 14.4.1 SDK
    /// (`connection/client_interface.hpp`'s `enterAoIOnVehicle` message: `EntityID id;
    /// EntityID vehicleID; IDAlias idAlias;`), matching the 9-byte length of
    /// `ENTER_AOI_ON_VEHICLE`.
    #[derive(Debug, Clone, Copy)]
    pub struct EnterAoiOnVehicle {
        pub entity_id: u32,
        pub vehicle_id: u32,
        pub id_alias: u8,
    }
}

impl SimpleElement for EnterAoiOnVehicle {
    const ID: u8 = id::ENTER_AOI_ON_VEHICLE;
    const LEN: ElementLength = ElementLength::Fixed(9);
}

/// Sent when an entity leaves the client's Area of Interest. Layout confirmed against
/// the leaked BigWorld 14.4.1 SDK (`ServerConnection::leaveAoI`,
/// `connection/server_connection.cpp`): an `EntityID` followed by zero or more
/// `EventNumber` (`i32`) values filling the rest of the element -- there's no explicit
/// count, the reader instead divides the remaining byte count by 4 (`sizeof(EventNumber)`
/// in the source), which this project's framing already gives for free since the
/// underlying reader is bounded to this element's declared `Variable16` length before
/// [`SimpleCodec::read`] is ever called.
#[derive(Debug, Clone)]
pub struct LeaveAoi {
    pub entity_id: u32,
    pub last_event_numbers: Vec<i32>,
}

impl SimpleCodec for LeaveAoi {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u32(self.entity_id)?;
        for &event_number in &self.last_event_numbers {
            write.write_i32(event_number)?;
        }
        Ok(())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let entity_id = read.read_u32()?;
        let mut data = Vec::new();
        read.read_to_end(&mut data)?;
        let last_event_numbers = data.chunks_exact(4)
            .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        Ok(Self { entity_id, last_event_numbers })
    }

}

impl SimpleElement for LeaveAoi {
    const ID: u8 = id::LEAVE_AOI;
    const LEN: ElementLength = ElementLength::Variable16;
}

// =============================================================================
// Timing, positioning references & entity selection
// =============================================================================

crate::__struct_simple_codec! {
    /// It is used as a timestamp for the elements in a bundle.
    #[derive(Debug, Clone)]
    pub struct TickSync {
        pub tick: u8,
    }
}

impl SimpleElement for TickSync {
    const ID: u8 = id::TICK_SYNC;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


/// A periodic form of [`TickSync`], carrying two bytes rather than one.
///
/// Body not decoded, and never seen on the wire: this client sends the plain
/// [`TickSync`] instead.
pub type TickSyncPeriodic = DebugElementFixed<{ id::TICK_SYNC_PERIODIC }, 2>;

crate::__struct_simple_codec! {
    /// Sets the base position that subsequent *relative* volatile updates
    /// (`AVATAR_UPDATE_*`) are measured from, naming it indirectly: the client is told to
    /// reuse a position **it previously sent to the server itself**, identified by the
    /// sequence number it stamped on that outgoing update.
    ///
    /// So resolving this requires having tracked the client->server stream:
    /// `reference = calculate_reference_position(sent_positions[sequence_number])`, where
    /// `sent_positions` is fed by [`crate::app::base::element::AvatarUpdateImplicit`] and
    /// [`crate::app::base::element::AvatarUpdateExplicit`] (both carry the matching
    /// `ref_num`). Confirmed in `ServerConnection::relativePositionReference` and, for
    /// the sending half, `server_connection.cpp:757`
    /// (`sentPositions_[ sendingSequenceNumber_ ] = globalPos; ++sendingSequenceNumber_`).
    ///
    /// Note the rounding: this path rounds, [`RelativePosition`] does not.
    #[derive(Debug, Clone, Copy)]
    pub struct RelativePositionReference {
        pub sequence_number: u8,
    }
}

impl SimpleElement for RelativePositionReference {
    const ID: u8 = id::RELATIVE_POSITION_REFERENCE;
    const LEN: ElementLength = ElementLength::Fixed(1);
}

crate::__struct_simple_codec! {
    /// Sets the base position for subsequent relative volatile updates directly, rather
    /// than by reference to something the client sent (see
    /// [`RelativePositionReference`]).
    ///
    /// Assigned **verbatim, without rounding** -- `ServerConnection::relativePosition` is
    /// just `referencePosition_ = args.position;`, unlike every other writer of that
    /// field. Preserved here rather than normalised, because rounding it would silently
    /// shift every position decoded until the next reference change.
    #[derive(Debug, Clone, Copy)]
    pub struct RelativePosition {
        pub position: Vec3,
    }
}

impl SimpleElement for RelativePosition {
    const ID: u8 = id::RELATIVE_POSITION;
    const LEN: ElementLength = ElementLength::Fixed(12);
}

crate::__struct_simple_codec! {
    /// Announces which vehicle an entity is riding, which decides how that entity's
    /// volatile positions are interpreted: BigWorld's `AVATAR_UPDATE_GET_POS_ORIGIN` uses
    /// the connection's reference position as the origin only when the entity is *not* on
    /// a vehicle, and the zero vector when it is (positions are then vehicle-relative).
    ///
    /// `vehicle_id == 0` (`NULL_ENTITY_ID`) means "no vehicle", i.e. it clears the
    /// mapping. Per the SDK's own comment this names the vehicle for the *next* position
    /// update, which "may not be the one currently associated with that entity".
    #[derive(Debug, Clone, Copy)]
    pub struct SetVehicle {
        pub passenger_id: u32,
        pub vehicle_id: u32,
    }
}

impl SimpleElement for SetVehicle {
    const ID: u8 = id::SET_VEHICLE;
    const LEN: ElementLength = ElementLength::Fixed(8);
}

crate::__struct_simple_codec! {
    /// Sent by the server to inform that subsequent elements will target another entity's
    /// property/method updates, referenced directly by its full id.
    ///
    /// The controlled `Vehicle` entity created by [`CreateCellPlayer`] gets its own
    /// `OwnClient`-flagged property updates selected this way. Distinct from
    /// [`SelectPlayerEntity`], which targets the base/`Account` entity, and from
    /// [`SelectAliasedEntity`], the compact byte-alias form used for the broadcast
    /// (`AllClients`) properties of *other* nearby entities.
    #[derive(Debug, Clone, Copy)]
    pub struct SelectEntity {
        pub entity_id: u32,
    }
}

impl SimpleElement for SelectEntity {
    const ID: u8 = id::SELECT_ENTITY;
    const LEN: ElementLength = ElementLength::Fixed(4);
}

crate::__struct_simple_codec! {
    /// Sent by the server to re-target subsequent property/method elements at a nearby
    /// entity, naming it by its one-byte Area-of-Interest alias rather than its full id.
    ///
    /// The compact counterpart to [`SelectEntity`]: `ServerConnection::selectAliasedEntity`
    /// does `selectedEntityID_ = idAlias_[args.idAlias]`. Aliases are handed out by
    /// [`EnterAoi`]/[`EnterAoiOnVehicle`] and are only meaningful against that table, so a
    /// reader must track it to resolve one. An alias with no entry leaves the selection
    /// undefined, and anything decoded against it afterwards would target the wrong entity.
    #[derive(Debug, Clone, Copy)]
    pub struct SelectAliasedEntity {
        pub id_alias: u8,
    }
}

impl SimpleElement for SelectAliasedEntity {
    const ID: u8 = id::SELECT_ALIASED_ENTITY;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


crate::__struct_simple_codec! {
    /// Sent by the server to inform that subsequent elements will target
    /// the player entity.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct SelectPlayerEntity {}
}

impl SimpleElement for SelectPlayerEntity {
    const ID: u8 = id::SELECT_PLAYER_ENTITY;
    const LEN: ElementLength = ElementLength::Fixed(0);
}


crate::__struct_simple_codec! {
    /// This is when an update is being forced back for an (ordinarily)
    /// client controlled entity, including for the player. Usually this is
    /// due to a physics correction from the server, but it could be for any
    /// reason decided by the server (e.g. server-initiated teleport).
    #[derive(Debug, Clone)]
    pub struct ForcedPosition {
        pub entity_id: u32,
        pub space_id: u32,
        pub vehicle_entity_id: u32,
        /// Two bytes WoT adds that the BigWorld 14.4.1 `forcedPosition` layout
        /// (`x.id >> x.spaceID >> x.vehicleID >> x.position >> x.direction`, 36 bytes)
        /// does not have; they make up the declared 38. Observed zero in all 27 samples
        /// of a battle, so the meaning is unknown -- but the *position* is not: placing
        /// them here rather than at the end is what makes the following `Vec3` read as
        /// clean, smoothly drifting world coordinates (e.g. `(271.13, 60.88, 320.84)`)
        /// instead of the denormal garbage a 2-byte-early read produced.
        pub unk: u16,
        pub position: Vec3,
        pub direction: Vec3,
    }
}

impl SimpleElement for ForcedPosition {
    const ID: u8 = id::FORCED_POSITION;
    const LEN: ElementLength = ElementLength::Fixed(38);
}


// =============================================================================
// Avatar movement updates (`AVUPMSG` family) & their packed field types
// =============================================================================

// These three messages carry a full-detail, uncompressed position/direction, for cases
// where the entity is too far from the reference position for the packed `AVUPMSG`
// forms above, or more precision is wanted, at the cost of more bandwidth -- confirmed
// against the leaked BigWorld 14.4.1 SDK (`connection/common_client_interface.hpp`'s
// `avatarUpdateNoAliasDetailed`/`avatarUpdateAliasDetailed`/`avatarUpdatePlayerDetailed`:
// `{NoAlias EntityID | Alias IDAlias | Player nothing} + Position3D + Direction3D`, same
// `EntityID`/`IDAlias` split as the `AVUPMSG` family above, `PlayerDetailed` omitting the
// id entirely since it only ever targets the client's own player).
//
// That vanilla layout is 1 byte short of the sizes used here
// (28/25/24 vs the confirmed-live 29/26/25 -- see the `id` module), in all three cases by
// exactly the same amount. The same SDK's `BaseAppExtInterface::avatarUpdateImplicit`
// (`connection/baseapp_ext_interface.hpp`, the base-directed sibling of these) has a
// directly analogous trailing `uint8 refNum` appended after `pos`/`dir` specifically
// under this build's `VOLATILE_POSITIONS_ARE_ABSOLUTE == 0` configuration (confirmed
// still in effect, see `msgtypes.hpp`) -- "refNum is used to refer to this position later
// as the base for relative positions", matching this doc comment's own "detailed enough
// to be reference positions" wording almost verbatim. That extra byte is indeed a
// `ref_num` -- but it *leads* rather than trails, now confirmed against a live WoT battle
// capture. Raw `AvatarUpdateAliasDetailed` bytes
// `00 fa | 0d10b943 d9f33d41 6b638bc3 | 00000000 267158bc 2727cbbf` decode under the
// leading layout as ref_num=0, id_alias=250 (an alias `EnterAoi` had actually assigned),
// position (370.13, 11.87, -278.77) and direction (0.0, -0.013, -1.59) rad -- all
// plausible battle values. Under the old trailing layout the same bytes gave
// `position: Vec3(-0.00014, 0.119, -4.4e-32)` and made *every* id_alias read as 0, which
// is what stranded ~2000 property updates per battle on the "no entity selected" path.
crate::__struct_simple_codec! {
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdateNoAliasDetailed {
        /// Reference-position sequence number, and the *first* field on the wire.
        /// Confirmed live: raw `00 fa 0d10b943 d9f33d41 6b638bc3 ...` decodes as
        /// ref_num=0, id_alias=250 (an alias `EnterAoi` really did assign),
        /// position (370.13, 11.87, -278.77) and direction (0.0, -0.013, -1.59)
        /// rad -- all plausible. The previous trailing layout gave garbage
        /// (`Vec3(-0.00014, 0.119, -4.4e-32)`) and made every alias read as 0.
        pub ref_num: u8,
        pub entity_id: u32,
        pub position: Vec3,
        /// Yaw/pitch/roll -- see [`ForcedPosition::direction`] for why the exact float
        /// encoding is unconfirmed beyond "plausible radian values".
        pub direction: Vec3,
    }
}

impl SimpleElement for AvatarUpdateNoAliasDetailed {
    const ID: u8 = id::AVATAR_UPDATE_NO_ALIAS_DETAILED;
    const LEN: ElementLength = ElementLength::Fixed(29);
}

crate::__struct_simple_codec! {
    /// See [`AvatarUpdateNoAliasDetailed`].
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdateAliasDetailed {
        /// Reference-position sequence number, and the *first* field on the wire.
        /// Confirmed live: raw `00 fa 0d10b943 d9f33d41 6b638bc3 ...` decodes as
        /// ref_num=0, id_alias=250 (an alias `EnterAoi` really did assign),
        /// position (370.13, 11.87, -278.77) and direction (0.0, -0.013, -1.59)
        /// rad -- all plausible. The previous trailing layout gave garbage
        /// (`Vec3(-0.00014, 0.119, -4.4e-32)`) and made every alias read as 0.
        pub ref_num: u8,
        pub id_alias: u8,
        pub position: Vec3,
        pub direction: Vec3,
    }
}

impl SimpleElement for AvatarUpdateAliasDetailed {
    const ID: u8 = id::AVATAR_UPDATE_ALIAS_DETAILED;
    const LEN: ElementLength = ElementLength::Fixed(26);
}

crate::__struct_simple_codec! {
    /// See [`AvatarUpdateNoAliasDetailed`]. Always targets the client's own player
    /// entity, so there's no id field at all -- confirmed by this being the only one of
    /// the three actually used as a reference position (per the same doc comment on
    /// `avatarUpdatePlayerDetailed` in the leaked SDK).
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdatePlayerDetailed {
        /// Reference-position sequence number, and the *first* field on the wire.
        /// Confirmed live: raw `00 fa 0d10b943 d9f33d41 6b638bc3 ...` decodes as
        /// ref_num=0, id_alias=250 (an alias `EnterAoi` really did assign),
        /// position (370.13, 11.87, -278.77) and direction (0.0, -0.013, -1.59)
        /// rad -- all plausible. The previous trailing layout gave garbage
        /// (`Vec3(-0.00014, 0.119, -4.4e-32)`) and made every alias read as 0.
        pub ref_num: u8,
        pub position: Vec3,
        pub direction: Vec3,
    }
}

impl SimpleElement for AvatarUpdatePlayerDetailed {
    const ID: u8 = id::AVATAR_UPDATE_PLAYER_DETAILED;
    const LEN: ElementLength = ElementLength::Fixed(25);
}

/// Generates one [`SimpleElement`] struct per `AVUPMSG` combination: a `NoAlias`
/// (`entity_id: u32`) or `Alias` (`id_alias: u8`) target, followed by its position and
/// direction fields, followed by `unk_len` bytes of unconfirmed-purpose trailing data --
/// see the doc comment on `id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL` for how
/// these ids were confirmed to be `CALLBACK`-length, not `FIXED`.
///
/// Every one of the 24 ids needs a nonzero `unk_len`, confirmed via static disassembly
/// (radare2, `re-work/bin-2.3.1.3/WorldOfTanks.exe`) of the live client's own
/// length-computing function for these ids (a pure function of the handler object's own
/// fields, called directly via Frida for ground truth -- `re-work/frida/dump_avupmsg_lengths.js`)
/// and cross-checked against `AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL` (`0x35`)'s
/// independently live-confirmed 12-byte length (two real battle captures, see git
/// history). The discrepancy from the vanilla-SDK formula decomposes cleanly and
/// uniformly once measured this way: every position mode needs more bytes than assumed
/// (`FullPos` 7 not 5, `OnGround` 4 not 3, `NoPos` 1 not 0) and `YawPitchRoll` needs one
/// more (4 not 3) while `YawPitch`/`Yaw`/`NoDir` match the vanilla formula exactly --
/// `unk_len` below is just `$len` minus the vanilla-formula total, kept as an opaque
/// trailing field per id rather than folded into wider `Packed*` types, since what these
/// bytes actually *contain* is still unconfirmed (would need disassembling the much
/// larger stream-parsing function, vtable slot 4, not attempted here).
macro_rules! avatar_update_elements {
    ($( $name:ident { $id_field:ident: $id_ty:ty, position: $pos_ty:ty, direction: $dir_ty:ty, unk: $unk_len:literal } = $id_const:ident, $len:literal; )*) => {
        $(
            #[derive(Debug, Clone, Copy)]
            pub struct $name {
                /// Reference-position sequence number, and the *first* field on the wire --
                /// see the doc comment on the `avatar_update_elements!` invocation below
                /// for the live evidence that it leads rather than trails.
                pub ref_num: u8,
                pub $id_field: $id_ty,
                pub position: $pos_ty,
                pub direction: $dir_ty,
                /// Unconfirmed-purpose trailing bytes -- see the doc comment on the
                /// `avatar_update_elements!` macro invocation.
                pub unk: [u8; $unk_len],
            }

            impl SimpleCodec for $name {
                fn write(&self, write: &mut dyn Write) -> io::Result<()> {
                    write.write_u8(self.ref_num)?;
                    <$id_ty as SimpleCodec>::write(&self.$id_field, write)?;
                    <$pos_ty as SimpleCodec>::write(&self.position, write)?;
                    <$dir_ty as SimpleCodec>::write(&self.direction, write)?;
                    write.write_all(&self.unk)
                }
                fn read(read: &mut dyn Read) -> io::Result<Self> {
                    let ref_num = read.read_u8()?;
                    let $id_field = <$id_ty as SimpleCodec>::read(read)?;
                    let position = <$pos_ty as SimpleCodec>::read(read)?;
                    let direction = <$dir_ty as SimpleCodec>::read(read)?;
                    let mut unk = [0; $unk_len];
                    read.read_exact(&mut unk)?;
                    Ok(Self { ref_num, $id_field, position, direction, unk })
                }
            }

            impl SimpleElement for $name {
                const ID: u8 = id::$id_const;
                const LEN: ElementLength = ElementLength::Fixed($len);
            }
        )*
    };
}

avatar_update_elements! {
    AvatarUpdateNoAliasFullPosYawPitchRoll  { entity_id: u32, position: PackedXyz, direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL, 15;
    AvatarUpdateNoAliasFullPosYawPitch      { entity_id: u32, position: PackedXyz, direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH, 13;
    AvatarUpdateNoAliasFullPosYaw           { entity_id: u32, position: PackedXyz, direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW, 12;
    AvatarUpdateNoAliasFullPosNoDir         { entity_id: u32, position: PackedXyz, direction: (),                 unk: 0 } = AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR, 11;
    AvatarUpdateNoAliasOnGroundYawPitchRoll { entity_id: u32, position: PackedXz,  direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL, 12;
    AvatarUpdateNoAliasOnGroundYawPitch     { entity_id: u32, position: PackedXz,  direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH, 10;
    AvatarUpdateNoAliasOnGroundYaw          { entity_id: u32, position: PackedXz,  direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW, 9;
    AvatarUpdateNoAliasOnGroundNoDir        { entity_id: u32, position: PackedXz,  direction: (),                 unk: 0 } = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR, 8;
    AvatarUpdateNoAliasNoPosYawPitchRoll    { entity_id: u32, position: (),        direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL, 9;
    AvatarUpdateNoAliasNoPosYawPitch        { entity_id: u32, position: (),        direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH, 7;
    AvatarUpdateNoAliasNoPosYaw             { entity_id: u32, position: (),        direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW, 6;
    AvatarUpdateNoAliasNoPosNoDir           { entity_id: u32, position: (),        direction: (),                 unk: 0 } = AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR, 5;
    AvatarUpdateAliasFullPosYawPitchRoll    { id_alias: u8,   position: PackedXyz, direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL, 12;
    AvatarUpdateAliasFullPosYawPitch        { id_alias: u8,   position: PackedXyz, direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH, 10;
    AvatarUpdateAliasFullPosYaw             { id_alias: u8,   position: PackedXyz, direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_ALIAS_FULL_POS_YAW, 9;
    AvatarUpdateAliasFullPosNoDir           { id_alias: u8,   position: PackedXyz, direction: (),                 unk: 0 } = AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR, 8;
    AvatarUpdateAliasOnGroundYawPitchRoll   { id_alias: u8,   position: PackedXz,  direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL, 9;
    AvatarUpdateAliasOnGroundYawPitch       { id_alias: u8,   position: PackedXz,  direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH, 7;
    AvatarUpdateAliasOnGroundYaw            { id_alias: u8,   position: PackedXz,  direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW, 6;
    AvatarUpdateAliasOnGroundNoDir          { id_alias: u8,   position: PackedXz,  direction: (),                 unk: 0 } = AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR, 5;
    AvatarUpdateAliasNoPosYawPitchRoll      { id_alias: u8,   position: (),        direction: PackedYawPitchRoll, unk: 1 } = AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL, 6;
    AvatarUpdateAliasNoPosYawPitch          { id_alias: u8,   position: (),        direction: PackedYawPitch,     unk: 0 } = AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH, 4;
    AvatarUpdateAliasNoPosYaw               { id_alias: u8,   position: (),        direction: PackedYaw,          unk: 0 } = AVATAR_UPDATE_ALIAS_NO_POS_YAW, 3;
    AvatarUpdateAliasNoPosNoDir             { id_alias: u8,   position: (),        direction: (),                 unk: 0 } = AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR, 2;
}

/// Updates an entity's volatile properties -- the frequently-changing position and
/// direction fields that the `AVATAR_UPDATE_*` family otherwise carries.
///
/// Body not decoded, and never seen on the wire; this client receives the
/// `AVATAR_UPDATE_*` elements instead. Related to [`ChangeVolatilePackerType`], which
/// selects how such fields are packed.
pub type AvatarUpdateVolatileProperties = DebugElementVariable16<{ id::AVATAR_UPDATE_VOLATILE_PROPERTIES }>;
/// Switches the packing scheme used for volatile position/direction fields.
///
/// Relevant because the `AVATAR_UPDATE_*` family's packed layouts
/// ([`crate::app::math::PackedXyz`], [`crate::app::math::PackedYawPitchRoll`]) assume a
/// fixed bit allocation; a packer change would invalidate that assumption mid-stream.
/// Body not decoded, and never seen on the wire, so no such change has had to be handled.
pub type ChangeVolatilePackerType = DebugElementVariable16<{ id::CHANGE_VOLATILE_PACKER_TYPE }>;

// =============================================================================
// Network Replication Layer ("NRL") -- `BW::NRL`, the subsystem that replicates
// *dynamic components* (their properties and their method calls). It is separate from
// classic `EntityProperty`/`EntityMethod` and absent from the vanilla BigWorld SDK.
//
// Everything is a node in one tree mirroring entity -> dynamic component ->
// property/method, and every message addresses a node by a 16-bit id. The shapes below
// follow the client's `BWEndPoint`, `Hub::processMessage` and
// `ClientNodeFactory::create`.
// =============================================================================

/// One node kind in the NRL tree, as `ClientNodeFactory::create` instantiates them
/// (the wire carries the discriminant as a single byte). The tree mirrors
/// entity -> dynamic component -> property/method.
///
/// Only the fields the client reads off the wire in `readCreate` are decoded here. What
/// a component node's children carry next is encoded per the *component's own* schema,
/// so framing stops there, leaving those bytes in [`NrlStream::rest`] until
/// [`NrlStream::decode_components`] decodes them against the script model.
#[derive(Debug, Clone)]
pub enum NrlNode {
    /// `BWRootNode`, the tree root. Carries nothing.
    Root,
    /// `BWEntityNode`: one per replicated entity, the parent of that entity's
    /// dynamic-component nodes.
    Entity {
        entity_id: u32,
    },
    /// `ClientDynComponentNode`: attaches one dynamic component to the entity node
    /// above it. On creation the client also creates one child property node per
    /// component property -- their ids come from `children`, and each then reads its
    /// initial value, which is why a `DynComponent` record ends this project's framing.
    DynComponent {
        /// The component's type, resolved with `ScriptDispatch::component_from_nrl_id`:
        /// numbered like entity type ids but counting only the components the client
        /// has a script class for.
        component_type_id: u16,
        /// Often empty; otherwise an instance name such as `"EntityMarkerComponent"`,
        /// `"XrayDebuff"` or a vehicle sub-part slot.
        name: String,
        /// Node ids for the children created alongside this component.
        children: NrlIdList,
        /// The initial state of those children, once decoded against the component with
        /// [`NrlStream::decode_components`]. Until then those bytes are in
        /// [`NrlStream::rest`].
        data: Option<NrlComponentData>,
    },
    /// `ClientPrimitivePropertyNode`: one replicated scalar property of a component.
    /// Which property is decided locally by the parent component, not by the wire.
    PrimitiveProperty,
    /// `ClientCompositePropertyNode`: as above for a structured property.
    CompositeProperty,
    /// `ClientMethodCalleeNode`: a component method the server can call on the client.
    MethodCallee {
        method_index: u32,
    },
    /// `ServerMethodCallerNode`: the client->server direction, so not expected inbound.
    ServerMethodCaller,
}

/// The initial state a [`NrlNode::DynComponent`] creates its children with.
///
/// The children are the component's properties in property table order, then one child per
/// client method, then one more child whose role is unknown. Only the children marked
/// present in [`NrlIdList::present`] carry anything: a property its value, a method child
/// the index of its method in the method table. Checked against every component created
/// in a live battle, each consuming its bytes exactly.
#[derive(Debug, Clone)]
pub struct NrlComponentData {
    /// The component's own type id, see `ScriptDispatch::component_from_nrl_id`.
    pub type_id: u16,
    /// Each present child, by node id.
    pub children: Vec<(u32, NrlChild)>,
    /// The exact bytes this was decoded from, written back as is.
    raw: Vec<u8>,
}

impl NrlComponentData {

    /// The initial state of a component, of the given NRL type id, from its present
    /// children in order, encoded against its schema.
    pub fn encode(dispatch: &ScriptDispatch, nrl_type_id: u16, children: Vec<(u32, NrlChild)>) -> io::Result<Self> {
        let (type_id, component) = dispatch.component_from_nrl_id(nrl_type_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
                format!("unknown nrl component type id: {nrl_type_id}")))?;
        let mut raw = Vec::new();
        for (_, child) in &children {
            match child {
                NrlChild::Property { name, value } => {
                    let property = component.properties.iter().find(|p| p.name == *name)
                        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
                            format!("nrl component type id {nrl_type_id} has no property {name}")))?;
                    value.write(&mut raw, &property.ty)?;
                }
                NrlChild::Method { method_index, .. } => raw.write_packed_u24(*method_index)?,
                NrlChild::Trailing => {}
            }
        }
        Ok(Self { type_id, children, raw })
    }

}

/// One child of a [`NrlNode::DynComponent`], see [`NrlComponentData`].
#[derive(Debug, Clone, PartialEq)]
pub enum NrlChild {
    /// A property node, with its initial value.
    Property {
        name: Arc<str>,
        value: Value,
    },
    /// A method node: the index of its method in the component's client method table,
    /// in exposed order. The method children are not in that order themselves: on a live
    /// `OwnVehicle` each later call only decoded against the method this index names.
    Method {
        name: Arc<str>,
        method_index: u32,
    },
    /// The trailing child every component has, which carries nothing.
    Trailing,
}

/// A node-id list in the encoding `readIdList` uses: a sequence of signed bytes read
/// relative to a running id that starts at a base (the owning node's id), so sibling
/// ids -- which are usually consecutive -- cost one byte for a whole run.
///
/// | byte (as `i8`) | meaning |
/// |---|---|
/// | `1..=127` | advance the running id by that much, emit it |
/// | `-128` (`0x80`) | the next two bytes are an absolute id, big-endian |
/// | `-127` (`0x81`) | emit the explicit null id, `0` |
/// | `-126` (`0x82`) | end of list, followed by a `ceil(count / 8)`-byte presence bitmask |
/// | `-125..=-1` | a run of `byte + 126` consecutive ids |
/// | `0` | end of list, with no bitmask |
#[derive(Clone)]
pub struct NrlIdList {
    /// The decoded ids in order; `0` is the encoding's explicit null-id marker.
    pub ids: Vec<u32>,
    /// The presence bitmask that terminated the list, one bit per id (LSB first), or
    /// `None` when the list ended with the no-bitmask terminator instead.
    pub present: Option<Vec<u8>>,
    /// The exact bytes this list was read from, kept so an enclosing message re-encodes
    /// byte-for-byte: one id sequence has several legal encodings (runs, deltas,
    /// absolutes) and re-deriving one would not reproduce the client's choice.
    raw: Vec<u8>,
}

impl fmt::Debug for NrlIdList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `raw` is redundant with `ids`/`present` and only adds noise to trace logs.
        f.debug_struct("NrlIdList")
            .field("ids", &self.ids)
            .field("present", &self.present)
            .finish()
    }
}

impl NrlIdList {

    /// A list of the given ids, relative to the owning node's id `base`, ending with
    /// the given presence bitmask (one bit per id, LSB first) or none. Consecutive ids
    /// are written as runs, small steps as deltas, anything else absolute.
    pub fn new(base: u32, ids: Vec<u32>, present: Option<Vec<u8>>) -> Self {
        let mut raw = Vec::new();
        let mut cur = base;
        let mut index = 0;
        while index < ids.len() {
            let id = ids[index];
            // A run: consecutive ids from the running one, up to 125 at once.
            let run = ids[index..].iter().zip(1..)
                .take_while(|&(&id, step)| id != 0 && id == cur.wrapping_add(step))
                .count()
                .min(125);
            if run >= 2 {
                raw.push((run as i32 - 126) as i8 as u8);
                cur = cur.wrapping_add(run as u32);
                index += run;
                continue;
            }
            if id == 0 {
                raw.push(0x81);
            } else if id > cur && id - cur <= 127 {
                raw.push((id - cur) as u8);
                cur = id;
            } else {
                raw.push(0x80);
                raw.extend_from_slice(&(id as u16).to_be_bytes());
                cur = id;
            }
            index += 1;
        }
        match &present {
            Some(mask) => {
                raw.push(0x82);
                raw.extend_from_slice(mask);
            }
            None => raw.push(0),
        }
        Self { ids, present, raw }
    }

    /// Decode a list starting at `*pos`, advancing it past the list. `base` is the id
    /// the running counter starts from. `None` if the bytes run out mid-list, in which
    /// case `pos` is left untouched.
    fn read_from(buf: &[u8], pos: &mut usize, base: u32) -> Option<Self> {
        let start = *pos;
        let mut at = *pos;
        let mut ids = Vec::new();
        let mut cur = base;
        let present = loop {
            let byte = *buf.get(at)? as i8;
            at += 1;
            match byte {
                0 => break None,
                -126 => {
                    let len = (ids.len() + 7) / 8;
                    let mask = buf.get(at..at + len)?;
                    at += len;
                    break Some(mask.to_vec());
                }
                -127 => ids.push(0),
                -128 => {
                    let bytes = buf.get(at..at + 2)?;
                    at += 2;
                    cur = u16::from_be_bytes([bytes[0], bytes[1]]) as u32;
                    ids.push(cur);
                }
                1.. => {
                    cur = cur.wrapping_add(byte as u32);
                    ids.push(cur);
                }
                // -125..=-1: a run of consecutive ids.
                _ => {
                    for _ in 0..(byte as i32 + 126) {
                        cur = cur.wrapping_add(1);
                        ids.push(cur);
                    }
                }
            }
        };
        *pos = at;
        Some(Self { ids, present, raw: buf[start..at].to_vec() })
    }

}

/// The body of one NRL message, keyed off its message-type byte.
///
/// A variant is *terminal* when the client would go on to read schema-dependent bytes
/// this project cannot frame; the enclosing [`NrlStream`] then stops and keeps the
/// remainder in [`NrlStream::rest`].
#[derive(Debug, Clone)]
pub enum NrlBody {
    /// Type 0: create one node under `parent_id`.
    CreateNode {
        parent_id: u16,
        node: NrlNode,
    },
    /// Type 1: create several children of this node in one go. Each present id in
    /// `children` is then followed on the wire by its own node-type byte and creation
    /// payload, read into `nodes` until one needs its component's schema, as a
    /// [`Self::CreateNode`] would stop.
    CreateChildren {
        children: NrlIdList,
        nodes: Vec<(u32, NrlNode)>,
    },
    /// Type 2: detach this node and its subtree.
    UnlinkTree,
    /// Type 5: a lookup with no payload.
    Lookup,
    /// Type 6: apply an update to this node -- a property value or a method call,
    /// encoded per the component's schema (terminal).
    UpdateNode,
    /// Types 3 and 4, whose payloads are likewise schema-dependent (terminal).
    Other {
        msg_type: u8,
    },
}

/// One NRL message: a target node plus a body.
#[derive(Debug, Clone)]
pub struct NrlMessage {
    pub node_id: u16,
    pub body: NrlBody,
}

/// A stream of NRL messages, which is what every `Nrl*` element except [`NrlData`]
/// carries.
///
/// On the wire a message is `[msg_type: u8][node_id: u16 big-endian][body]`. The
/// single-purpose elements ([`NrlCreateNode`], [`NrlUnlinkTree`]) leave the type byte
/// out of the *first* message, since their element id already implies it, and then fall
/// through to the same generic loop for anything that follows -- so one
/// [`NrlCreateNode`] element routinely creates a whole entity-plus-components subtree.
#[derive(Debug, Clone)]
pub struct NrlStream {
    pub messages: Vec<NrlMessage>,
    /// Everything after the last message that could be framed. Empty when the element
    /// decoded completely; otherwise it starts at a property/method value whose length
    /// only the component's own schema gives.
    pub rest: Vec<u8>,
}

impl NrlStream {

    /// Decode a stream. `implied` is the message type of the leading message when the
    /// element id already fixes it (its type byte is then absent from the wire).
    pub(crate) fn read_with(read: &mut dyn Read, implied: Option<u8>) -> io::Result<Self> {
        let mut buf = Vec::new();
        read.read_to_end(&mut buf)?;
        let mut messages = Vec::new();
        let pos = Self::frame(&buf, implied, &mut messages);
        Ok(Self { messages, rest: buf[pos..].to_vec() })
    }

    /// Decode the initial state of each component this stream creates (see
    /// [`NrlComponentData`]), which the framing alone stops at, framing on after each
    /// one. On error the stream is left as it was decoded up to that component.
    pub fn decode_components(&mut self, dispatch: &ScriptDispatch) -> io::Result<()> {
        loop {

            let pending = match self.messages.last_mut().map(|message| &mut message.body) {
                Some(NrlBody::CreateNode { node, .. }) => node,
                Some(NrlBody::CreateChildren { nodes, .. }) => match nodes.last_mut() {
                    Some((_, node)) => node,
                    None => return Ok(()),
                },
                _ => return Ok(()),
            };
            let NrlNode::DynComponent { component_type_id, children, data: data @ None, .. } = pending else {
                return Ok(());
            };

            let mut read = &self.rest[..];
            let (type_id, decoded) = read_component_children(&mut read, dispatch, *component_type_id, children)?;
            let consumed = self.rest.len() - read.len();
            let raw = self.rest[..consumed].to_vec();
            // Checks the encoder against every component decoded: a server builds them.
            match NrlComponentData::encode(dispatch, *component_type_id, decoded.clone()) {
                Ok(encoded) if encoded.raw == raw => {}
                // Python values pickle differently but equivalently.
                Ok(encoded) if read_component_children(&mut &encoded.raw[..], dispatch, *component_type_id, children)
                    .is_ok_and(|(_, reread)| reread == decoded) => {}
                Ok(encoded) => warn!("nrl component type id {component_type_id} encodes differently: read {} bytes, encoded {}",
                    raw.len(), encoded.raw.len()),
                Err(e) => warn!("nrl component type id {component_type_id} does not encode: {e}"),
            }
            *data = Some(NrlComponentData { type_id, children: decoded, raw });

            let tail = self.rest.split_off(consumed);
            let pos = Self::frame(&tail, None, &mut self.messages);
            self.rest = tail[pos..].to_vec();

        }
    }

    /// Frame messages from `buf` into `messages`, up to the first terminal one, returning
    /// the position framing stopped at. A `CreateChildren` left unfinished at the end of
    /// `messages` is resumed first.
    fn frame(buf: &[u8], implied: Option<u8>, messages: &mut Vec<NrlMessage>) -> usize {

        let mut pos = 0;
        let mut implied = implied;

        if let Some(NrlMessage { body: NrlBody::CreateChildren { children, nodes }, .. }) = messages.last_mut() {
            match read_child_nodes(buf, &mut pos, children, nodes) {
                Some(false) => {}
                _ => return pos,
            }
        }

        loop {
            let mut at = pos;
            let msg_type = match implied.take() {
                Some(msg_type) => msg_type,
                None => {
                    let Some(&byte) = buf.get(at) else { break };
                    at += 1;
                    byte
                }
            };
            let Some(id_bytes) = buf.get(at..at + 2) else { break };
            at += 2;
            let node_id = u16::from_be_bytes([id_bytes[0], id_bytes[1]]);

            // `terminal` messages are kept, but nothing can be framed after them.
            let (body, terminal) = match msg_type {
                0 => {
                    let Some(parent_bytes) = buf.get(at..at + 2) else { break };
                    at += 2;
                    let parent_id = u16::from_be_bytes([parent_bytes[0], parent_bytes[1]]);
                    let Some(&node_type) = buf.get(at) else { break };
                    at += 1;
                    let Some((node, terminal)) = read_node(&buf, &mut at, node_type, node_id) else { break };
                    (NrlBody::CreateNode { parent_id, node }, terminal)
                }
                1 => {
                    let Some(children) = NrlIdList::read_from(&buf, &mut at, node_id as u32) else { break };
                    let mut nodes = Vec::new();
                    // Out of bytes mid-list: keep what was read, the rest stays raw.
                    let terminal = read_child_nodes(buf, &mut at, &children, &mut nodes).unwrap_or(true);
                    (NrlBody::CreateChildren { children, nodes }, terminal)
                }
                2 => (NrlBody::UnlinkTree, false),
                5 => (NrlBody::Lookup, false),
                6 => (NrlBody::UpdateNode, true),
                3 | 4 => (NrlBody::Other { msg_type }, true),
                _ => break,
            };

            messages.push(NrlMessage { node_id, body });
            pos = at;
            if terminal {
                break;
            }
        }

        pos

    }

    pub(crate) fn write_with(&self, write: &mut dyn Write, implied: bool) -> io::Result<()> {
        for (index, message) in self.messages.iter().enumerate() {
            let msg_type = match &message.body {
                NrlBody::CreateNode { .. } => 0,
                NrlBody::CreateChildren { .. } => 1,
                NrlBody::UnlinkTree => 2,
                NrlBody::Lookup => 5,
                NrlBody::UpdateNode => 6,
                NrlBody::Other { msg_type } => *msg_type,
            };
            if index > 0 || !implied {
                write.write_u8(msg_type)?;
            }
            write.write_all(&message.node_id.to_be_bytes())?;
            match &message.body {
                NrlBody::CreateNode { parent_id, node } => {
                    write.write_all(&parent_id.to_be_bytes())?;
                    write_node(node, write)?;
                }
                NrlBody::CreateChildren { children, nodes } => {
                    write.write_all(&children.raw)?;
                    for (_, node) in nodes {
                        write_node(node, write)?;
                    }
                }
                NrlBody::UnlinkTree | NrlBody::Lookup
                | NrlBody::UpdateNode | NrlBody::Other { .. } => {}
            }
        }
        write.write_all(&self.rest)
    }

}

impl NrlIdList {

    /// Whether the child at this index is present, every child being so without a mask.
    fn is_present(&self, index: usize) -> bool {
        match &self.present {
            Some(mask) => mask.get(index / 8).is_some_and(|byte| byte & (1 << (index % 8)) != 0),
            None => true,
        }
    }

}

/// Read the node of each present child of a `CreateChildren` not yet in `nodes`,
/// advancing `at` past each one read. Returns whether reading stopped on a terminal node,
/// or `None` if the bytes run out.
fn read_child_nodes(buf: &[u8], at: &mut usize, children: &NrlIdList, nodes: &mut Vec<(u32, NrlNode)>) -> Option<bool> {
    let present = children.ids.iter().enumerate()
        .filter(|&(index, _)| children.is_present(index))
        .map(|(_, &id)| id);
    for id in present.skip(nodes.len()) {
        let mut next = *at;
        let &node_type = buf.get(next)?;
        next += 1;
        let (node, terminal) = read_node(buf, &mut next, node_type, id as u16)?;
        *at = next;
        nodes.push((id, node));
        if terminal {
            return Some(true);
        }
    }
    Some(false)
}

/// Read the initial state of a component's children, see [`NrlComponentData`].
fn read_component_children(
    read: &mut &[u8],
    dispatch: &ScriptDispatch,
    component_type_id: u16,
    children: &NrlIdList,
) -> io::Result<(u16, Vec<(u32, NrlChild)>)> {

    let (type_id, component) = dispatch.component_from_nrl_id(component_type_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
            format!("unknown nrl component type id: {component_type_id}")))?;

    let properties = component.properties.len();
    let methods = component.client_methods.len();
    if children.ids.len() != properties + methods + 1 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!(
            "nrl component type id {component_type_id} has {} children, expected {}",
            children.ids.len(), properties + methods + 1)));
    }

    let mut decoded = Vec::new();
    for (index, &node_id) in children.ids.iter().enumerate() {
        if !children.is_present(index) {
            continue;
        }
        let child = if let Some(property) = component.properties.get(index) {
            NrlChild::Property { name: Arc::clone(&property.name), value: Value::read(read, &property.ty)? }
        } else if index < properties + methods {
            let method_index = read.read_packed_u24()?;
            let name = usize::try_from(method_index).ok()
                .and_then(|index| component.client_methods.get(index))
                .map(|method| Arc::clone(&method.name))
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!(
                    "nrl component type id {component_type_id} has no client method {method_index}")))?;
            NrlChild::Method { name, method_index }
        } else {
            NrlChild::Trailing
        };
        decoded.push((node_id, child));
    }

    Ok((type_id, decoded))

}

/// Read one node's creation payload, advancing `at`. Returns the node and whether it is
/// terminal (the client reads schema-dependent bytes next). `None` if the bytes run out.
fn read_node(buf: &[u8], at: &mut usize, node_type: u8, node_id: u16) -> Option<(NrlNode, bool)> {
    Some(match node_type {
        0 => (NrlNode::Root, false),
        1 => {
            let bytes = buf.get(*at..*at + 4)?;
            *at += 4;
            (NrlNode::Entity { entity_id: u32::from_le_bytes(bytes.try_into().unwrap()) }, false)
        }
        2 => {
            let type_bytes = buf.get(*at..*at + 2)?;
            *at += 2;
            let component_type_id = u16::from_le_bytes([type_bytes[0], type_bytes[1]]);
            let name = read_packed_str(buf, at)?;
            let children = NrlIdList::read_from(buf, at, node_id as u32)?;
            (NrlNode::DynComponent { component_type_id, name, children, data: None }, true)
        }
        3 => (NrlNode::PrimitiveProperty, true),
        4 => (NrlNode::CompositeProperty, true),
        5 => {
            let method_index = read_packed_len(buf, at)?;
            (NrlNode::MethodCallee { method_index }, true)
        }
        6 => (NrlNode::ServerMethodCaller, true),
        _ => return None,
    })
}

fn write_node(node: &NrlNode, write: &mut dyn Write) -> io::Result<()> {
    match node {
        NrlNode::Root => write.write_u8(0),
        NrlNode::Entity { entity_id } => {
            write.write_u8(1)?;
            write.write_u32(*entity_id)
        }
        NrlNode::DynComponent { component_type_id, name, children, data } => {
            write.write_u8(2)?;
            write.write_u16(*component_type_id)?;
            write.write_string_variable(name)?;
            write.write_all(&children.raw)?;
            match data {
                Some(data) => write.write_all(&data.raw),
                None => Ok(()),
            }
        }
        NrlNode::PrimitiveProperty => write.write_u8(3),
        NrlNode::CompositeProperty => write.write_u8(4),
        NrlNode::MethodCallee { method_index } => {
            write.write_u8(5)?;
            write.write_packed_u24(*method_index)
        }
        NrlNode::ServerMethodCaller => write.write_u8(6),
    }
}

/// BigWorld's packed length: one byte, or `0xFF` followed by a 24-bit little-endian one.
fn read_packed_len(buf: &[u8], at: &mut usize) -> Option<u32> {
    match *buf.get(*at)? {
        0xFF => {
            let bytes = buf.get(*at + 1..*at + 4)?;
            *at += 4;
            Some(bytes[0] as u32 | (bytes[1] as u32) << 8 | (bytes[2] as u32) << 16)
        }
        len => {
            *at += 1;
            Some(len as u32)
        }
    }
}

fn read_packed_str(buf: &[u8], at: &mut usize) -> Option<String> {
    let len = read_packed_len(buf, at)? as usize;
    let bytes = buf.get(*at..*at + len)?;
    *at += len;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Creates a node (and, via the trailing messages the client's generic loop reads,
/// usually its whole subtree): this is how a dynamic component is attached to an entity,
/// which is the identity [`CreateBasePlayer`]'s always-empty `entity_components_count`
/// trailer implies must arrive separately.
///
/// The leading message's type is implied to be `create node`, so it starts directly at
/// its `node_id`. A typical 9-byte element is one entity node
/// (`[node_id][parent_id][type 1][entity_id]`), and longer ones chain component nodes
/// under it.
#[derive(Debug, Clone)]
pub struct NrlCreateNode(pub NrlStream);

impl SimpleCodec for NrlCreateNode {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        self.0.write_with(write, true)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        NrlStream::read_with(read, Some(0)).map(Self)
    }
}

impl SimpleElement for NrlCreateNode {
    const ID: u8 = id::NRL_CREATE_NODE;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// Detaches a node and its subtree; leading message type implied, as for
/// [`NrlCreateNode`].
#[derive(Debug, Clone)]
pub struct NrlUnlinkTree(pub NrlStream);

impl SimpleCodec for NrlUnlinkTree {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        self.0.write_with(write, true)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        NrlStream::read_with(read, Some(2)).map(Self)
    }
}

impl SimpleElement for NrlUnlinkTree {
    const ID: u8 = id::NRL_UNLINK_TREE;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// A batch of NRL messages, each carrying its own type byte.
#[derive(Debug, Clone)]
pub struct NrlMsgToClient(pub NrlStream);

impl SimpleCodec for NrlMsgToClient {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        self.0.write_with(write, false)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        NrlStream::read_with(read, None).map(Self)
    }
}

impl SimpleElement for NrlMsgToClient {
    const ID: u8 = id::NRL_MSG_TO_CLIENT;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// A *fragment* of the NRL message stream, not a message.
///
/// `BWEndPoint::onReceivedData` appends the whole payload verbatim to a per-connection
/// reassembly buffer and parses nothing. Messages therefore straddle element
/// boundaries, so a single `NrlData` element is not decodable on its own -- earlier
/// attempts to model it as a list of self-describing records were fitting noise.
///
/// The buffer is drained by [`NrlUnlinkTreeFlag`] and [`NrlUpdateNodeFlag`], which have
/// no payload of their own: each reads `[node_id: u16 big-endian]` plus that node's
/// payload *out of this buffer* and applies message type 2 or 6 respectively, resetting
/// the buffer once it is empty. So reconstructing these updates means concatenating
/// every `NrlData` payload per connection and consuming one record per `*Flag` message.
#[derive(Debug, Clone)]
pub struct NrlData {
    pub fragment: Vec<u8>,
}

impl SimpleCodec for NrlData {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.fragment)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut fragment = Vec::new();
        read.read_to_end(&mut fragment)?;
        Ok(Self { fragment })
    }

}

impl SimpleElement for NrlData {
    const ID: u8 = id::NRL_DATA;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// Applies an update to one node: its message type is implied, so the element is
/// `[node_id: u16 big-endian][payload]`.
///
/// The payload's encoding belongs to the owning component's schema, so it is kept raw
/// here and decoded against the node tree by `wgtk::app::nrl::NrlTree::decode_update`. For the *scripted*
/// (`PyDynamicComponent`-style) nodes that dominate real traffic it is a packed-length
/// pickle, which [`Self::python_value`] carries whenever that reading accounts for the
/// payload exactly: one such node was a Ruinberg mission/objective tracker
/// (`finishTime`/`state`/`params`/`type`/`id`/`timer`) progressing over time. Nodes
/// where it does not add up (observed: a node whose value is variable-length but not a
/// pickle) get `None` rather than a mis-framed guess.
#[derive(Debug, Clone)]
pub struct NrlUpdateNode {
    pub node_id: u16,
    /// Every byte after `node_id`, verbatim -- this is what re-encodes, so the element
    /// always round-trips byte-for-byte.
    pub payload: Vec<u8>,
    /// `payload` read as a packed-length pickle, and only when that consumes it exactly.
    pub python_value: Option<PythonValue>,
}

impl SimpleCodec for NrlUpdateNode {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&self.node_id.to_be_bytes())?;
        write.write_all(&self.payload)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut id_bytes = [0; 2];
        read.read_exact(&mut id_bytes)?;
        let mut payload = Vec::new();
        read.read_to_end(&mut payload)?;

        // Only accept the pickle reading when it accounts for the whole payload; a
        // partial read would mean the node's schema is something else entirely.
        let mut cursor = io::Cursor::new(&payload[..]);
        let python_value = match SimpleCodec::read(&mut cursor) {
            Ok(value) if cursor.position() == payload.len() as u64 => Some(value),
            _ => None,
        };

        Ok(Self { node_id: u16::from_be_bytes(id_bytes), payload, python_value })
    }

}

impl SimpleElement for NrlUpdateNode {
    const ID: u8 = id::NRL_UPDATE_NODE;
    const LEN: ElementLength = ElementLength::Variable16;
}

/// Drains one message-type-2 (unlink) record from the [`NrlData`] reassembly buffer.
///
/// Carries no bytes at all, so there is nothing left to decode: the element's *presence*
/// in the bundle is the whole message. Spelled out as its own zero-length type rather
/// than a `DebugElement*` so it stops reading as an undecoded gap -- what it still lacks
/// is the [`NrlData`] reassembly state it refers to, not a codec.
#[derive(Debug, Clone, Copy, Default)]
pub struct NrlUnlinkTreeFlag;

impl SimpleCodec for NrlUnlinkTreeFlag {
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> { Ok(()) }
    fn read(_read: &mut dyn Read) -> io::Result<Self> { Ok(Self) }
}

impl SimpleElement for NrlUnlinkTreeFlag {
    const ID: u8 = id::NRL_UNLINK_TREE_FLAG;
    const LEN: ElementLength = ElementLength::ZERO;
}

/// Drains one message-type-6 (update) record from the [`NrlData`] reassembly buffer.
/// Payload-less, exactly as [`NrlUnlinkTreeFlag`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NrlUpdateNodeFlag;

impl SimpleCodec for NrlUpdateNodeFlag {
    fn write(&self, _write: &mut dyn Write) -> io::Result<()> { Ok(()) }
    fn read(_read: &mut dyn Read) -> io::Result<Self> { Ok(Self) }
}

impl SimpleElement for NrlUpdateNodeFlag {
    const ID: u8 = id::NRL_UPDATE_NODE_FLAG;
    const LEN: ElementLength = ElementLength::ZERO;
}

/// The decoded body of an [`NrlUnreliableMsgToClient`], keyed off its `kind` byte.
#[derive(Debug, Clone)]
pub enum NrlUnreliableMsgToClientPayload {
    /// `kind` `0x30`: the server-side gun marker of the player's vehicle, what BigWorld
    /// WoT once sent as `updateGunMarker(shotPos, shotVec, dispersionAngle)`. Identified
    /// on a 2.4.1.1 battle against the vehicle's own position updates: `shot_position`
    /// stays within 2.5 m of the vehicle, about 2 m up; `shot_velocity`'s length takes
    /// only the values 870.4 and 696.0, the T110E4's APCR and AP shell speeds (1.25
    /// ratio), switching exactly when the player switched shells, and its heading
    /// follows the vehicle's yaw; `dispersion_angle` grows from
    /// 0.003 at rest to 0.025 on the move. The bytes around them never changed.
    GunMarker {
        unk_prefix: [u8; 9],
        shot_position: Vec3,
        shot_velocity: Vec3,
        dispersion_angle: f32,
        unk_suffix: [u8; 2],
    },
    /// `kind` `0x11`: no new gun marker this tick. Sent for whole stretches where it
    /// cannot move (the pre-battle countdown, after the vehicle is out), and for single
    /// ticks in between where the marker barely moved: across one, the full messages on
    /// either side differ by 0 to 1.2, against a median 1.8 between consecutive full
    /// ones, which always differ. Its 8 bytes never changed.
    Unchanged(Vec<u8>),
    /// Any other `(kind, payload)` shape.
    Raw(Vec<u8>),
}

/// WoT's own CGF "unreliable" node message (delivered best-effort, no
/// retransmission): an update of the NRL root node (the `06 00 01` header is message
/// type 6 on node 1, as in [`NrlMsgToCell`]) carrying the server-side gun marker, see
/// [`NrlUnreliableMsgToClientPayload::GunMarker`], every tick (roughly 100ms).
/// `unk_id_a`/`unk_id_b` were constant for a whole battle, and look like hashes rather
/// than network ids, which are small integers; one battle cannot say whether they change
/// between battles. `tick`/`prev_tick` are a
/// client-interpolation pair (this message's tick, and the last tick it has data
/// for); `prev_tick` normally trails `tick` by 1 but skipped once in one capture --
/// consistent with the channel actually being unreliable. `flags` bit `0x20` tracks
/// whether `prev_tick` holds real data (`0`, with `prev_tick` also `0`, only in the
/// first message of a capture); no other bit was ever observed set.
#[derive(Debug, Clone)]
pub struct NrlUnreliableMsgToClient {
    /// Constant across a whole battle, see the struct documentation.
    pub unk_id_a: u32,
    pub unk_id_b: u32,
    pub tick: u32,
    pub prev_tick: u32,
    /// Correlates 1:1 with the payload shape: `0x30` for
    /// [`NrlUnreliableMsgToClientPayload::GunMarker`], `0x11` for the 8-byte
    /// not-yet-decoded shape.
    pub kind: u8,
    /// Bit `0x20`: whether `prev_tick` holds real data. See the struct-level doc.
    pub flags: u8,
    pub payload: NrlUnreliableMsgToClientPayload,
}

impl SimpleCodec for NrlUnreliableMsgToClient {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_all(&[0x06, 0x00, 0x01])?;
        write.write_u32(self.unk_id_a)?;
        write.write_u32(self.unk_id_b)?;
        write.write_u32(self.tick)?;
        write.write_u32(self.prev_tick)?;
        write.write_u8(self.kind)?;
        write.write_all(&[0xD6, 0xC3, 0xC4, 0x00, 0x00, 0x01])?;
        write.write_u8(self.flags)?;
        write.write_u8(0x00)?;
        match &self.payload {
            NrlUnreliableMsgToClientPayload::GunMarker { unk_prefix, shot_position, shot_velocity, dispersion_angle, unk_suffix } => {
                write.write_u8(9 + 4 * 7 + 2)?;
                write.write_all(unk_prefix)?;
                write.write_vec3(*shot_position)?;
                write.write_vec3(*shot_velocity)?;
                write.write_f32(*dispersion_angle)?;
                write.write_all(unk_suffix)?;
            }
            NrlUnreliableMsgToClientPayload::Unchanged(bytes)
            | NrlUnreliableMsgToClientPayload::Raw(bytes) => {
                write.write_u8(bytes.len().try_into().map_err(|_| io::Error::new(
                    io::ErrorKind::InvalidData, "NrlUnreliableMsgToClient: payload too long for its 1-byte length prefix"))?)?;
                write.write_all(bytes)?;
            }
        }
        Ok(())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let mut header = [0u8; 3];
        read.read_exact(&mut header)?;
        if header != [0x06, 0x00, 0x01] {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("NrlUnreliableMsgToClient: unrecognized header {header:02X?} \
                    (only 06 00 01 is known)")));
        }
        let unk_id_a = read.read_u32()?;
        let unk_id_b = read.read_u32()?;
        let tick = read.read_u32()?;
        let prev_tick = read.read_u32()?;
        let kind = read.read_u8()?;
        let mut class_tag = [0u8; 6];
        read.read_exact(&mut class_tag)?;
        if class_tag != [0xD6, 0xC3, 0xC4, 0x00, 0x00, 0x01] {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("NrlUnreliableMsgToClient: unrecognized class tag {class_tag:02X?} \
                    (only D6 C3 C4 00 00 01 is known)")));
        }
        let flags = read.read_u8()?;
        let zero = read.read_u8()?;
        if zero != 0x00 {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("NrlUnreliableMsgToClient: expected a constant 0x00 byte before \
                    the payload length, got {zero:#04X}")));
        }
        let payload_len = read.read_u8()?;
        let mut payload_bytes = vec![0u8; payload_len as usize];
        read.read_exact(&mut payload_bytes)?;
        let payload = if kind == 0x30 && payload_len as usize == 9 + 4 * 7 + 2 {
            let mut cur = &payload_bytes[..];
            let mut unk_prefix = [0u8; 9];
            cur.read_exact(&mut unk_prefix)?;
            let shot_position = cur.read_vec3()?;
            let shot_velocity = cur.read_vec3()?;
            let dispersion_angle = cur.read_f32()?;
            let mut unk_suffix = [0u8; 2];
            cur.read_exact(&mut unk_suffix)?;
            NrlUnreliableMsgToClientPayload::GunMarker { unk_prefix, shot_position, shot_velocity, dispersion_angle, unk_suffix }
        } else if kind == 0x11 {
            NrlUnreliableMsgToClientPayload::Unchanged(payload_bytes)
        } else {
            NrlUnreliableMsgToClientPayload::Raw(payload_bytes)
        };
        Ok(Self { unk_id_a, unk_id_b, tick, prev_tick, kind, flags, payload })
    }

}

impl SimpleElement for NrlUnreliableMsgToClient {
    const ID: u8 = id::NRL_UNRELIABLE_MSG_TO_CLIENT;
    const LEN: ElementLength = ElementLength::Variable16;
}

// =============================================================================
// Entity control, voice & session hand-off
// =============================================================================

crate::__struct_simple_codec! {
    /// Sent by the server to tell the client whether it now has (`on = true`) or has
    /// lost (`on = false`) control authority over an entity -- while controlled, the
    /// client is expected to locally predict/simulate the entity's movement and report
    /// it back itself (the base-directed `AvatarUpdateImplicit`/`AvatarUpdateExplicit`
    /// elements in [`super::super::base::element`]) rather than only passively receiving
    /// server-driven `AVATAR_UPDATE_*` elements for it. Layout confirmed against the
    /// leaked BigWorld 14.4.1 SDK (`connection/client_interface.hpp`'s `controlEntity`:
    /// `EntityID id; bool on;`), matching the 5-byte
    /// length for `CONTROL_ENTITY`.
    #[derive(Debug, Clone, Copy)]
    pub struct ControlEntity {
        pub entity_id: u32,
        pub on: bool,
    }
}

impl SimpleElement for ControlEntity {
    const ID: u8 = id::CONTROL_ENTITY;
    const LEN: ElementLength = ElementLength::Fixed(5);
}

/// Carries voice-chat audio from another player.
///
/// Body not decoded, and never seen on the wire -- it requires voice chat to be both
/// enabled and in use by someone in the battle.
pub type VoiceData = DebugElementVariable16<{ id::VOICE_DATA }>;
/// Tells the client to restore its state after the server has moved or recovered it,
/// for instance when a cell fails over.
///
/// The client answers with [`crate::app::base::element::RestoreClientAck`]. Body not
/// decoded, and never seen on the wire: it needs a server-side failure to trigger it.
pub type RestoreClient = DebugElementVariable16<{ id::RESTORE_CLIENT }>;


crate::__struct_simple_codec! {
    /// This is used to tell the client to switch control to a new base app address.
    #[derive(Debug, Clone)]
    pub struct SwitchBaseApp {
        pub base_addr: WgSocketAddrV4,
        pub reset_entities: bool,
    }
}

impl SimpleElement for SwitchBaseApp {
    const ID: u8 = id::SWITCH_BASE_APP;
    const LEN: ElementLength = ElementLength::Fixed(9);
}

// =============================================================================
// Resource download (fonts, sounds, etc. streamed on demand)
// =============================================================================

/// Header describing a resource that will be downloaded in possibly many fragments.
#[derive(Clone)]
pub struct ResourceHeader {
    pub id: u16,
    pub description: Vec<u8>,
}

impl SimpleCodec for ResourceHeader {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u16(self.id)?;
        write.write_blob_variable(&self.description)?;
        Ok(())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        Ok(Self {
            id: read.read_u16()?,
            description: read.read_blob_variable()?,
        })
    }

}

impl SimpleElement for ResourceHeader {
    const ID: u8 = id::RESOURCE_HEADER;
    const LEN: ElementLength = ElementLength::Variable16;
}

impl fmt::Debug for ResourceHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceHeader")
            .field("id", &self.id)
            .field("description", &AsciiFmt(&self.description))
            .finish()
    }
}


/// Header describing a resource that will be downloaded in possibly many fragments.
#[derive(Clone)]
pub struct ResourceFragment {
    pub id: u16,
    pub sequence_num: u8,
    pub last: bool,
    pub data: Vec<u8>,
}

impl SimpleCodec for ResourceFragment {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u16(self.id)?;
        write.write_u8(self.sequence_num)?;
        write.write_bool(self.last)?;
        write.write_blob(&self.data)?;
        Ok(())
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        Ok(Self {
            id: read.read_u16()?,
            sequence_num: read.read_u8()?,
            last: read.read_bool()?,
            data: read.read_blob_to_end()?,
        })
    }

}

impl SimpleElement for ResourceFragment {
    const ID: u8 = id::RESOURCE_FRAGMENT;
    const LEN: ElementLength = ElementLength::Variable16;
}

impl fmt::Debug for ResourceFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceFragment")
            .field("id", &self.id)
            .field("sequence_num", &self.sequence_num)
            .field("last", &self.last)
            .field("data", &AsciiFmt(&self.data))
            .finish()
    }
}


// =============================================================================
// Session teardown & raw entity property/position streaming
// =============================================================================

crate::__struct_simple_codec! {
    /// Sent by the server to inform that subsequent elements will target
    /// the player entity.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct LoggedOff {
        pub reason: u8,
    }
}

impl SimpleElement for LoggedOff {
    const ID: u8 = id::LOGGED_OFF;
    const LEN: ElementLength = ElementLength::Fixed(1);
}


crate::__struct_simple_codec! {
    /// Sent for the currently-selected entity (see [`SelectEntity`]/[`SelectPlayerEntity`]/
    /// [`SelectAliasedEntity`] -- no id field of its own) when its volatile position
    /// becomes "less volatile" or it teleports, i.e. an accurate, uncompressed correction
    /// on top of the regular `AVATAR_UPDATE_*` stream. Ignored client-side for an entity
    /// under local control (confirmed by `ServerConnection::detailedPosition`'s own
    /// `isControlledLocally` early-return). Layout confirmed against the leaked BigWorld
    /// 14.4.1 SDK (`connection/client_interface.hpp`'s `detailedPosition`: `Position3D
    /// position; Direction3D direction;`), matching the
    /// 24-byte length for `DETAILED_POSITION` exactly (unlike the `*Detailed` avatar
    /// update messages, no extra byte here).
    #[derive(Debug, Clone, Copy)]
    pub struct DetailedPosition {
        pub position: Vec3,
        /// Yaw/pitch/roll -- see [`ForcedPosition::direction`] for why the exact float
        /// encoding is unconfirmed beyond "plausible radian values".
        pub direction: Vec3,
    }
}

impl SimpleElement for DetailedPosition {
    const ID: u8 = id::DETAILED_POSITION;
    const LEN: ElementLength = ElementLength::Fixed(24);
}

/// One step of a decoded [`NestedEntityProperty`]/[`SliceEntityProperty`] path: either a
/// named field of a `Dict` (or, for the first step, the entity's own top-level property
/// list) or a positional element of an `Array`/`Tuple`.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityPropertyPathStep {
    Field(Arc<str>),
    Index(u32),
}

/// `BitReader::bitsRequired` from the real engine (`cstdmf/bit_reader.cpp`): the number
/// of bits needed to represent any value in `0..num_values`, `0` for `num_values <= 1`.
fn bits_required(num_values: u32) -> u32 {
    if num_values <= 1 { 0 } else { u32::BITS - (num_values - 1).leading_zeros() }
}

/// The container a compressed-path decode is currently positioned in: the entity's
/// top-level property list, a `Dict`, or a resolved `Array`/`Tuple` (a definite element
/// count -- known statically for a `Tuple`, otherwise a guess being tried, see
/// [`ContainerKind::UnresolvedArray`]).
enum PathContainer<'a> {
    Top(&'a [PropertyDef]),
    Dict(&'a TyDict),
    Seq(&'a Ty, u32),
}

impl<'a> PathContainer<'a> {

    fn count(&self) -> u32 {
        match self {
            Self::Top(props) => props.len() as u32,
            Self::Dict(dict) => dict.properties.len() as u32,
            Self::Seq(_, count) => *count,
        }
    }

    /// The path step and type of the child at `index`, if in range.
    fn child(&self, index: u32) -> Option<(EntityPropertyPathStep, &'a Ty)> {
        match self {
            Self::Top(props) => props.get(index as usize)
                .map(|p| (EntityPropertyPathStep::Field(p.name.clone()), &p.ty)),
            Self::Dict(dict) => dict.properties.get(index as usize)
                .map(|p| (EntityPropertyPathStep::Field(p.name.clone()), &p.ty)),
            Self::Seq(ty, count) => (index < *count).then(|| (EntityPropertyPathStep::Index(index), *ty)),
        }
    }

}

enum ContainerKind<'a> {
    NotAContainer,
    Resolved(PathContainer<'a>),
    /// An `Array` element type whose current length isn't known statically (unlike a
    /// `Tuple`) and so must be guessed -- see [`MAX_SEQ_LEN_GUESS`].
    UnresolvedArray(&'a Ty),
}

/// Classify `ty` for compressed-path descent, transparently unwrapping `Alias`.
fn container_kind(ty: &Ty) -> ContainerKind<'_> {
    match ty.kind() {
        TyKind::Alias(inner) => container_kind(inner),
        TyKind::Dict(dict) => ContainerKind::Resolved(PathContainer::Dict(dict)),
        TyKind::Tuple(seq) => ContainerKind::Resolved(PathContainer::Seq(&seq.ty, seq.size.unwrap_or(0))),
        TyKind::Array(seq) => ContainerKind::UnresolvedArray(&seq.ty),
        _ => ContainerKind::NotAContainer,
    }
}

/// A fully decoded compressed path, as produced by [`walk_path`].
#[derive(Debug, PartialEq)]
pub enum PathOutcome {
    Single { path: Vec<EntityPropertyPathStep>, value: Value },
    Slice { path: Vec<EntityPropertyPathStep>, start: u32, end: u32, values: Vec<Value> },
}

/// A direct port of the real engine's `PropertyChangeReader::readCompressedPathAndApply`
/// (`entitydef/property_change_reader.cpp`): reads `[continue bit][index]` pairs,
/// descending through `container`, until a `0` continue bit is read, then performs the
/// leaf action (a single index for [`NestedEntityProperty`], an index range for
/// [`SliceEntityProperty`]) directly against whichever container was reached.
///
/// Crossing an `Array` needs its *live* element count, which -- unlike a `Tuple`'s -- is
/// not in the type, so it comes from `values`: the caller's tracked property values (see
/// [`decode_compressed_path`]). Every container count is therefore known, every field
/// width with it, and the descent follows exactly one branch -- `out` ends up with one
/// outcome or none.
///
/// An `Array` with no tracked value ends the descent, and the decode is reported as
/// undecodable rather than guessed at.
///
/// `live` is the tracked value of `container` itself, or `None` at the top level (where
/// `values` is keyed by property name instead) and wherever tracking has no entry.
/// `budget` bounds total work against malformed data.
fn walk_path<'a>(
    container: PathContainer<'a>,
    values: Option<&'a HashMap<Arc<str>, Value>>,
    live: Option<&'a Value>,
    mut reader: BitReader<'a>,
    path: Vec<EntityPropertyPathStep>,
    is_slice: bool,
    budget: &mut u32,
    out: &mut Vec<PathOutcome>,
) {

    if *budget == 0 || out.len() >= 16 {
        return;
    }
    *budget -= 1;

    let Some(cont) = reader.try_get(1) else { return };

    if cont != 0 {

        let count = container.count();
        if count == 0 {
            return;
        }
        let Some(index) = reader.try_get(bits_required(count)) else { return };
        let Some((step, child_ty)) = container.child(index) else { return };

        // The tracked value of the child we are about to descend into. At the top level
        // the path step names a property, so it resolves against `values`; below that it
        // indexes whatever `live` holds.
        let live_child = match (&container, &step) {
            (PathContainer::Top(_), EntityPropertyPathStep::Field(name)) =>
                values.and_then(|values| values.get(name)),
            (_, EntityPropertyPathStep::Field(name)) => match live {
                Some(Value::Dict(dict)) => dict.get(name),
                _ => None,
            },
            (_, EntityPropertyPathStep::Index(index)) => match live {
                Some(Value::Seq(items)) => items.get(*index as usize),
                _ => None,
            },
        };

        let mut child_path = path;
        child_path.push(step);

        match container_kind(child_ty) {
            ContainerKind::Resolved(child) =>
                walk_path(child, values, live_child, reader, child_path, is_slice, budget, out),
            ContainerKind::UnresolvedArray(elem_ty) => {
                // An `Array`'s length is not in the type, so it has to come from the
                // caller's tracked value. Without one the descent stops here and the
                // decode fails, rather than branching over candidate lengths.
                let Some(Value::Seq(items)) = live_child else { return };
                walk_path(PathContainer::Seq(elem_ty, items.len() as u32), values, live_child,
                    reader, child_path, is_slice, budget, out);
            }
            ContainerKind::NotAContainer => {}
        }

        return;

    }

    // `cont == 0`: `container` is the final container, read the leaf action against it.
    if is_slice {

        // An empty container is valid here, unlike a single-field leaf below: appending to
        // an empty array encodes `start == end == 0` in zero bits, so rejecting
        // `count == 0` would make a first-element insertion undecodable.
        let PathContainer::Seq(elem_ty, count) = container else { return };
        let idx_bits = bits_required(count + 1);
        let Some(start) = reader.try_get(idx_bits) else { return };
        let Some(end) = reader.try_get(idx_bits) else { return };
        if start > count || end > count || start > end {
            return;
        }

        reader.align_to_byte();
        let Some(rest) = reader.remaining_bytes() else { return };

        let mut cursor = io::Cursor::new(rest);
        let mut values = Vec::new();
        while (cursor.position() as usize) < rest.len() {
            let Ok(value) = Value::read(&mut cursor, elem_ty) else { return };
            values.push(value);
        }

        out.push(PathOutcome::Slice { path, start, end, values });

    } else {

        if container.count() == 0 {
            return;
        }

        let Some(index) = reader.try_get(bits_required(container.count())) else { return };
        let Some((step, leaf_ty)) = container.child(index) else { return };

        reader.align_to_byte();
        let Some(rest) = reader.remaining_bytes() else { return };

        let mut cursor = io::Cursor::new(rest);
        let Ok(value) = Value::read(&mut cursor, leaf_ty) else { return };
        if cursor.position() as usize != rest.len() {
            return;
        }

        let mut leaf_path = path;
        leaf_path.push(step);
        out.push(PathOutcome::Single { path: leaf_path, value });

    }

}

/// Decode a compressed path to the single interpretation consistent with the data, or
/// fail. Nothing is guessed between candidates: several surviving readings means the
/// `Array` length is undetermined and the caller is told so.
pub(crate) fn decode_compressed_path(
    top: &[PropertyDef],
    values: Option<&HashMap<Arc<str>, Value>>,
    data: &[u8],
    is_slice: bool,
) -> io::Result<PathOutcome> {

    let mut out = Vec::new();
    let mut budget = 200_000u32;
    walk_path(PathContainer::Top(top), values, None, BitReader::new(data), Vec::new(), is_slice, &mut budget, &mut out);

    match out.len() {
        1 => Ok(out.pop().unwrap()),
        // Either the path crossed an `Array` this entity has no tracked value for, or the
        // bytes genuinely don't fit the table. The two are worth telling apart by the
        // caller's state, not here.
        0 => Err(io::Error::new(io::ErrorKind::InvalidData,
            "entity property path: no consistent decode (an Array whose live length isn't \
             tracked for this entity, or a table that doesn't match these bytes)")),
        // Every container count is known, so the descent cannot fork.
        n => unreachable!("compressed path produced {n} outcomes from a single branch"),
    }

}

/// The leaf of a path to encode, see [`encode_compressed_path`].
enum PathLeaf<'v> {
    Single(&'v Value),
    Slice { start: u32, end: u32, values: &'v [Value] },
}

impl<'a> PathContainer<'a> {

    /// The index of `step` in this container, the inverse of [`Self::child`].
    fn index_of(&self, step: &EntityPropertyPathStep) -> Option<u32> {
        match (self, step) {
            (Self::Top(props), EntityPropertyPathStep::Field(name)) =>
                props.iter().position(|p| p.name == *name).map(|index| index as u32),
            (Self::Dict(dict), EntityPropertyPathStep::Field(name)) =>
                dict.properties.iter().position(|p| p.name == *name).map(|index| index as u32),
            (Self::Seq(_, count), EntityPropertyPathStep::Index(index)) => (index < count).then_some(*index),
            _ => None,
        }
    }

}

/// The inverse of [`decode_compressed_path`]: write `path` as `[continue bit][index]`
/// pairs, then the leaf, against the same table and tracked values the decode would use.
/// For a single value the path's last step is the leaf's index; for a slice the path
/// leads to the `Array` itself.
fn encode_compressed_path(
    top: &[PropertyDef],
    values: Option<&HashMap<Arc<str>, Value>>,
    path: &[EntityPropertyPathStep],
    leaf: PathLeaf<'_>,
    write: &mut dyn Write,
) -> io::Result<()> {

    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, format!("entity property path {path:?}: {message}"));

    let descend = match leaf {
        PathLeaf::Single(_) => &path[..path.len().checked_sub(1).ok_or_else(|| invalid("empty"))?],
        PathLeaf::Slice { .. } => path,
    };

    let mut bits = Vec::new();
    let mut container = PathContainer::Top(top);
    let mut live: Option<&Value> = None;

    for step in descend {
        let index = container.index_of(step).ok_or_else(|| invalid("a step not in its container"))?;
        bits.push((1, 1));
        bits.push((bits_required(container.count()), index));
        let (_, child_ty) = container.child(index).ok_or_else(|| invalid("a step not in its container"))?;
        // Same tracked value lookup as `walk_path`, for the `Array` lengths below.
        let live_child = match (&container, step) {
            (PathContainer::Top(_), EntityPropertyPathStep::Field(name)) => values.and_then(|values| values.get(name)),
            (_, EntityPropertyPathStep::Field(name)) => match live {
                Some(Value::Dict(dict)) => dict.get(name),
                _ => None,
            },
            (_, EntityPropertyPathStep::Index(index)) => match live {
                Some(Value::Seq(items)) => items.get(*index as usize),
                _ => None,
            },
        };
        container = match container_kind(child_ty) {
            ContainerKind::Resolved(child) => child,
            ContainerKind::UnresolvedArray(elem_ty) => match live_child {
                Some(Value::Seq(items)) => PathContainer::Seq(elem_ty, items.len() as u32),
                _ => return Err(invalid("an Array whose live length isn't tracked")),
            },
            ContainerKind::NotAContainer => return Err(invalid("a step into a non-container")),
        };
        live = live_child;
    }

    bits.push((1, 0));
    let leaf_values: Vec<(&Value, &Ty)> = match leaf {
        PathLeaf::Single(value) => {
            let step = path.last().unwrap();
            let index = container.index_of(step).ok_or_else(|| invalid("a leaf not in its container"))?;
            bits.push((bits_required(container.count()), index));
            vec![(value, container.child(index).unwrap().1)]
        }
        PathLeaf::Slice { start, end, values } => {
            let PathContainer::Seq(elem_ty, count) = container else {
                return Err(invalid("a slice of something else than an Array"));
            };
            if start > end || end > count {
                return Err(invalid("a slice outside the Array"));
            }
            let idx_bits = bits_required(count + 1);
            bits.push((idx_bits, start));
            bits.push((idx_bits, end));
            values.iter().map(|value| (value, elem_ty)).collect()
        }
    };

    let total: u32 = bits.iter().map(|&(n, _)| n).sum();
    let mut buf = vec![0; total.div_ceil(8) as usize];
    let mut writer = BitWriter::new(&mut buf);
    for (n, value) in bits {
        writer.add(n, value);
    }
    write.write_all(&buf)?;

    for (value, ty) in leaf_values {
        Codec::write(value, write, ty)?;
    }

    Ok(())

}

/// What a run of client-visible property values decoded to, whole or not.
///
/// Decoding stops at the first exposed id the property table doesn't cover, or the first
/// value that doesn't read. Stopping is forced: an unknown id means an unknown *size*, so
/// every byte after it is unreadable. But the properties *before* that point are real, so
/// they are kept rather than discarded with the rest -- a `Vehicle` whose table is missing
/// its dynamic components still seeds the properties it does have, which is what later
/// gives a compressed path a live `Array` length to resolve against.
#[derive(Debug, Clone, Default)]
pub struct ChangedProperties {
    /// Everything that decoded, in wire order, as `(exposed id, name, value)`. Possibly
    /// empty: a payload with nothing to send carries no bytes at all.
    pub values: Vec<(u8, Arc<str>, Value)>,
    /// Set when the run did *not* decode cleanly to its declared count.
    pub stopped: Option<ChangedPropertiesStop>,
}

impl ChangedProperties {

    /// Whether the whole run decoded and consumed the payload exactly.
    pub fn is_complete(&self) -> bool {
        self.stopped.is_none()
    }

    /// Encode as the entity creation messages carry them, against the entity type's
    /// property table: a count, then `(exposed id, value)` pairs.
    pub fn encode(&self, properties: &[PropertyDef]) -> io::Result<Vec<u8>> {
        let mut data = Vec::new();
        write_changed_properties(self, properties, &mut data)?;
        Ok(data)
    }

}

/// Why a [`ChangedProperties`] run didn't decode cleanly, and what was left over.
#[derive(Debug, Clone)]
pub struct ChangedPropertiesStop {
    pub reason: String,
    /// What the count byte promised; compare against [`ChangedProperties::values`]'s
    /// length for how many were actually recovered.
    pub expected: u8,
    /// From the stopping point to the end of the payload, kept for logging -- these bytes
    /// are the evidence for whatever the property table is missing.
    pub rest: Vec<u8>,
}

/// Decode a run of client-visible property values written by the real engine's
/// `EntityCache::addChangedProperties` (`cellapp/entity_cache.cpp`): a `uint8` count,
/// then that many `(uint8 exposed id, value)` pairs. An empty payload is valid and
/// decodes to no values -- see the `is_empty` case below.
///
/// The ids index `EntityDescription::clientServerProperty(i)` -- the very table
/// [`crate::app::dispatch::EntityDispatch::properties`] reproduces -- so a decode here is
/// also a check on that table. This is the body of both [`CreateEntity::client_data`]
/// (the entity's initial values, via `addOuterDetailLevel`) and [`UpdateEntity`] (a later
/// detail level's, or a refresh); reach it through [`CreateEntity::client_properties`] or
/// by reading the element, rather than calling it directly.
///
/// Confirmed byte-exact against a live capture: a `SPGZone` create carried
/// `04 00 <vehicleID> 01 <equipmentID> 02 <launchTime> 03 <strikeTime>`, consuming the
/// payload to its last byte, with `strikeTime` exactly 6.0s after `launchTime`.
///
/// Never fails: anything it can't read becomes a [`ChangedPropertiesStop`] alongside
/// whatever did read, because a partially understood entity is still worth seeding and a
/// malformed run must not abort the bundle around it.
/// The inverse of [`read_changed_properties`]. A run that stopped early writes back the
/// count it announced and the bytes it could not read, so it encodes as it was read.
fn write_changed_properties(changed: &ChangedProperties, properties: &[PropertyDef], write: &mut dyn Write) -> io::Result<()> {

    if changed.values.is_empty() && changed.stopped.is_none() {
        return Ok(());
    }

    let count = match &changed.stopped {
        Some(stop) => stop.expected,
        None => u8::try_from(changed.values.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData,
            format!("{} changed properties, at most 255 fit", changed.values.len())))?,
    };
    write.write_u8(count)?;

    for (exposed_id, name, value) in &changed.values {
        let property = properties.get(*exposed_id as usize)
            .filter(|property| property.name == *name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
                format!("property {name} is not exposed id {exposed_id}")))?;
        write.write_u8(*exposed_id)?;
        Codec::write(value, write, &property.ty)?;
    }

    if let Some(stop) = &changed.stopped {
        write.write_all(&stop.rest)?;
    }

    Ok(())

}

fn read_changed_properties(data: &[u8], properties: &[PropertyDef]) -> ChangedProperties {

    // An empty payload is valid and means "nothing changed": `addChangedProperties`
    // returns *before* writing the count when it finds nothing to send, and the client
    // matches that with `if (!data.remainingLength()) return;` in `BWEntity::onProperties`.
    // So the count byte is absent, not zero.
    let Some(&count) = data.first() else {
        return ChangedProperties::default();
    };

    let mut cursor = io::Cursor::new(data);
    cursor.set_position(1);

    let mut values = Vec::with_capacity(count as usize);
    // Where the offending property *starts*, so `rest` is cut from there rather than from
    // wherever a half-read value happened to leave the cursor.
    let mut stop: Option<(usize, String)> = None;
    let mut last_id: Option<u8> = None;

    for _ in 0..count {

        let at = cursor.position() as usize;

        let exposed_id = match cursor.read_u8() {
            Ok(id) => id,
            Err(e) => { stop = Some((at, format!("reading the next exposed id: {e}"))); break }
        };

        // `addChangedProperties` walks `i` from 0 to `clientServerPropertyCount()` and
        // writes `uint8(i)` for each property it sends, so the ids in a run are strictly
        // ascending. A repeat or a step backwards therefore isn't an id at all -- it is a
        // value byte being read at the wrong offset, i.e. an earlier property whose type
        // we size differently from the server.
        //
        // Worth checking separately from the bounds test below: a desync that happens to
        // land on an *in-range* id would otherwise decode "successfully" and seed garbage
        // into the caller's tracked state, which is far worse than reporting nothing.
        if last_id.is_some_and(|last| exposed_id <= last) {
            stop = Some((at, format!(
                "exposed id {exposed_id} after {} -- ids are written in ascending order, so \
                 this is a desync from an earlier property's length, not a new property",
                last_id.unwrap())));
            break;
        }

        let Some(property) = properties.get(exposed_id as usize) else {
            stop = Some((at, format!(
                "exposed id {exposed_id} past the end of a {}-entry property table",
                properties.len())));
            break;
        };

        match Value::read(&mut cursor, &property.ty) {
            Ok(value) => {
                last_id = Some(exposed_id);
                values.push((exposed_id, Arc::clone(&property.name), value));
            }
            Err(e) => { stop = Some((at, format!("property {} (exposed id {exposed_id}): {e}", property.name))); break }
        }

    }

    // A clean run consumes the payload exactly, so leftovers mean the table drifted from
    // the server's even though every id in it happened to resolve. Worth the same report.
    let at = cursor.position() as usize;
    if stop.is_none() && at < data.len() {
        stop = Some((at, format!(
            "{} trailing bytes after the {count} declared properties", data.len() - at)));
    }

    ChangedProperties {
        values,
        stopped: stop.map(|(at, reason)| ChangedPropertiesStop {
            reason,
            expected: count,
            rest: data[at.min(data.len())..].to_vec(),
        }),
    }

}

/// What a compressed-path element is decoded against: the entity's exposed-id property
/// table, and -- when the caller tracks them -- that entity's current property values.
///
/// The values matter because a path's bit layout depends on the *live* length of every
/// `Array` it crosses; without them [`walk_path`] has to try each length and keep every
/// self-consistent parse. See [`crate::app::dispatch::EntityDispatch::properties`] for the
/// table itself.
#[derive(Debug, Clone, Copy)]
pub struct PropertyPathConfig<'a> {
    pub table: &'a [PropertyDef],
    pub values: Option<&'a HashMap<Arc<str>, Value>>,
}

impl<'a> PropertyPathConfig<'a> {

    /// Decode against the table alone, guessing every `Array` length.
    pub fn new(table: &'a [PropertyDef]) -> Self {
        Self { table, values: None }
    }

    /// Decode against the table and the entity's tracked values, which pin those lengths.
    pub fn with_values(table: &'a [PropertyDef], values: Option<&'a HashMap<Arc<str>, Value>>) -> Self {
        Self { table, values }
    }

}

/// A client-directed update to a single field somewhere inside one of an entity's
/// properties -- e.g. one element of an array-of-dicts property, or one field within a
/// dict property -- addressed by a bit-packed "compressed path" rather than a wire id,
/// see [`crate::app::dispatch::EntityDispatch::properties`]. Decoded per the real
/// engine's `PropertyChangeReader::readCompressedPathAndApply`
/// (`entitydef/property_change_reader.cpp`, see [`walk_path`]).
#[derive(Debug, Clone)]
pub enum NestedEntityProperty {
    /// The path resolved to exactly one field.
    Known {
        /// The path from the entity's top-level property list down to the changed field,
        /// e.g. `[Field("status"), Index(3), Field("active")]`.
        path: Vec<EntityPropertyPathStep>,
        value: Value,
    },
    /// The path could not be resolved -- see [`SliceEntityProperty::Unknown`], which this
    /// shares its reasoning with.
    Unknown { data: Vec<u8>, reason: String },
}

impl NestedEntityProperty {
    pub const ID: u8 = id::NESTED_ENTITY_PROPERTY;
}

impl Element<PropertyPathConfig<'_>> for NestedEntityProperty {

    fn write_length(&self, _config: &PropertyPathConfig<'_>) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, config: &PropertyPathConfig<'_>) -> io::Result<u8> {
        match self {
            Self::Known { path, value } =>
                encode_compressed_path(config.table, config.values, path, PathLeaf::Single(value), write)?,
            Self::Unknown { data, .. } => write.write_all(data)?,
        }
        Ok(id::NESTED_ENTITY_PROPERTY)
    }

    fn read_length(_config: &PropertyPathConfig<'_>, _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &PropertyPathConfig<'_>, len: usize, _id: u8) -> io::Result<Self> {
        let mut data = vec![0u8; len];
        read.read_exact(&mut data)?;
        match decode_compressed_path(config.table, config.values, &data, false) {
            Ok(PathOutcome::Single { path, value }) => Ok(Self::Known { path, value }),
            Ok(PathOutcome::Slice { .. }) =>
                unreachable!("decode_compressed_path(is_slice=false) always returns Single"),
            Err(e) => Ok(Self::Unknown { data, reason: e.to_string() }),
        }
    }

}

/// A client-directed update replacing a contiguous range of a replicated `Array`
/// property's elements (Python-slice semantics: `array[start..end] = values`, an empty
/// `values` meaning pure removal) -- see [`NestedEntityProperty`]'s doc comment, which
/// this shares its path-decoding algorithm with.
#[derive(Debug, Clone)]
pub enum SliceEntityProperty {
    /// The path resolved to exactly one `Array` range.
    Known {
        /// The path from the entity's top-level property list down to the changed `Array`.
        path: Vec<EntityPropertyPathStep>,
        start: u32,
        end: u32,
        values: Vec<Value>,
    },
    /// The path could not be resolved: no length guess parsed cleanly, or several did and
    /// the `Array` length is therefore undetermined (which tracking the entity's property
    /// values prevents -- see [`PropertyPathConfig`]).
    ///
    /// Not an `Err`, deliberately, and for the same reason as [`MethodCall::Unknown`]: the
    /// element is `Variable16`, so it is fully framed regardless, and the bundle reader
    /// *rolls back* on a read error -- returning one would leave the element unconsumed
    /// and force the caller to abandon every element after it. Carries the raw payload so
    /// the failure stays diagnosable.
    Unknown { data: Vec<u8>, reason: String },
}

impl SliceEntityProperty {
    pub const ID: u8 = id::SLICE_ENTITY_PROPERTY;
}

impl Element<PropertyPathConfig<'_>> for SliceEntityProperty {

    fn write_length(&self, _config: &PropertyPathConfig<'_>) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, config: &PropertyPathConfig<'_>) -> io::Result<u8> {
        match self {
            Self::Known { path, start, end, values } => encode_compressed_path(config.table, config.values, path,
                PathLeaf::Slice { start: *start, end: *end, values }, write)?,
            Self::Unknown { data, .. } => write.write_all(data)?,
        }
        Ok(id::SLICE_ENTITY_PROPERTY)
    }

    fn read_length(_config: &PropertyPathConfig<'_>, _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &PropertyPathConfig<'_>, len: usize, _id: u8) -> io::Result<Self> {
        let mut data = vec![0u8; len];
        read.read_exact(&mut data)?;
        match decode_compressed_path(config.table, config.values, &data, true) {
            Ok(PathOutcome::Slice { path, start, end, values }) =>
                Ok(Self::Known { path, start, end, values }),
            Ok(PathOutcome::Single { .. }) =>
                unreachable!("decode_compressed_path(is_slice=true) always returns Slice"),
            Err(e) => Ok(Self::Unknown { data, reason: e.to_string() }),
        }
    }

}

/// A client-directed update carrying whole values for one or more of the *selected*
/// entity's client-visible properties -- the same `(exposed id, value)` run that an
/// entity's creation payload carries, see [`ChangedProperties`].
///
/// Written by the same `EntityCache::addChangedProperties` as [`CreateEntity::client_data`],
/// just with a bundle header instead of inside the create: the engine sends it when an
/// entity crosses into a nearer level of detail, so these are the properties that were
/// *not* in the create (`addOuterDetailLevel` only sends the outermost level's).
/// `ServerConnection::updateEntity` applies it to `selectedEntityID_`, so it is targeted
/// by the preceding `SelectEntity`/`SelectPlayerEntity`, exactly like `ENTITY_PROPERTY`.
#[derive(Debug, Clone)]
pub struct UpdateEntity(pub ChangedProperties);

impl UpdateEntity {
    pub const ID: u8 = id::UPDATE_ENTITY;
}

impl Element<[PropertyDef]> for UpdateEntity {

    fn write_length(&self, _config: &[PropertyDef]) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, config: &[PropertyDef]) -> io::Result<u8> {
        write_changed_properties(&self.0, config, write)?;
        Ok(id::UPDATE_ENTITY)
    }

    fn read_length(_config: &[PropertyDef], _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &[PropertyDef], len: usize, _id: u8) -> io::Result<Self> {
        let mut data = vec![0u8; len];
        read.read_exact(&mut data)?;
        Ok(Self(read_changed_properties(&data, config)))
    }

}
/// Tells the client the CellApp's externally reachable address.
///
/// Eight bytes, read through the same [`WgSocketAddrV4`] codec as [`SwitchBaseApp`].
/// The address sits in the same subnet as the BaseApps, with a zero salt.
#[derive(Debug, Clone)]
pub struct SetCellAppExtAddress(pub WgSocketAddrV4);

impl SimpleCodec for SetCellAppExtAddress {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        SimpleCodec::write(&self.0, write)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        SimpleCodec::read(read).map(Self)
    }
}

impl SimpleElement for SetCellAppExtAddress {
    const ID: u8 = id::SET_CELL_APP_EXT_ADDRESS;
    const LEN: ElementLength = ElementLength::Variable16;
}
/// Marks the final message relayed through the BaseApp proxy before the client begins
/// talking to a CellApp directly.
///
/// Body not decoded, and never seen on the wire: this client reaches the cell through the
/// BaseApp for the whole session and never takes up a direct connection. See
/// [`SetCellAppExtAddress`], which carries the address such a connection would use.
pub type LastProxyMessageAfterDirectCellAppConnection = DebugElementVariable16<{ id::LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION }>;

// =============================================================================
// Dynamic entity method/property dispatch (script-model-driven)
// =============================================================================

/// A client-directed entity method call, encoded/decoded dynamically against a
/// runtime-computed [`MethodDef`] table (see [`crate::app::script::EntityDispatch`])
/// resolved from the loaded script model, rather than a statically generated `AnyMethod`
/// enum -- lets [`super::super::base::App`] call a method purely by name, and lets a
/// generic wire observer (e.g. a debugging proxy) decode a live capture without knowing
/// any concrete entity type statically.
#[derive(Debug, Clone)]
pub struct EntityMethod {
    pub call: MethodCall,
}

/// The method a known call's exposed id names in the table it is written against.
fn known_method(config: &[MethodDef], exposed_id: u16) -> io::Result<&MethodDef> {
    config.get(exposed_id as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("no method of exposed id {exposed_id}")))
}

impl Element<[MethodDef]> for EntityMethod {

    fn write_length(&self, config: &[MethodDef]) -> io::Result<ElementLength> {
        
        let (exposed_id, preferred_len) = match &self.call {
            MethodCall::Known { exposed_id, .. } => (*exposed_id, known_method(config, *exposed_id)?.length),
            MethodCall::Unknown { exposed_id, .. } => (*exposed_id, ElementLength::Variable8),
        };

        // A sub-id is written as an extra byte ahead of the method's own payload (see
        // `write` below), so the preferred length only applies to full-slot ids; ids
        // requiring a sub-id always frame as Variable16, matching `read_length` below.
        let (_, sub_id) = id::ENTITY_METHOD.from_exposed_id(config.len() as u16, exposed_id);
        Ok(if sub_id.is_some() { ElementLength::Variable16 } else { preferred_len })
    
    }

    fn write(&self, write: &mut dyn Write, config: &[MethodDef]) -> io::Result<u8> {
        
        let exposed_id = match &self.call {
            MethodCall::Known { exposed_id, .. } | MethodCall::Unknown { exposed_id, .. } => *exposed_id,
        };
        
        let (element_id, sub_id) = id::ENTITY_METHOD.from_exposed_id(config.len() as u16, exposed_id);
        if let Some(sub_id) = sub_id {
            write.write_u8(sub_id)?;
        }
        
        match &self.call {
            MethodCall::Known { args, .. } => known_method(config, exposed_id)?.write_args(write, args)?,
            MethodCall::Unknown { data, .. } => write.write_all(data)?,
        }

        Ok(element_id)

    }

    fn read_length(config: &[MethodDef], id: u8) -> io::Result<ElementLength> {
        
        if !id::ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity method element id: {id:02X}")));
        }

        Ok(match id::ENTITY_METHOD.to_exposed_id_checked(config.len() as u16, id) {
            // An unrecognized exposed id falls back to Variable8 here instead of
            // erroring -- matching `getEntityMethodStreamSize` on a
            // live client instance, which returns Mercury's
            // `DEFAULT_VARIABLE_LENGTH_HEADER_SIZE` sentinel (-1, i.e. "read 1 more
            // header byte") for an id it doesn't recognize either.
            Some(exposed_id) => config.get(exposed_id as usize).map(|def| def.length).unwrap_or(ElementLength::Variable8),
            // A sub-id slot: the actual exposed id (and so its preferred length) can only
            // be known once the sub-id byte prefixing the payload has been read, so the
            // whole payload is always Variable16-framed instead.
            None => ElementLength::Variable16,
        })

    }

    fn read(read: &mut dyn Read, config: &[MethodDef], len: usize, id: u8) -> io::Result<Self> {
        
        if !id::ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity method element id: {id:02X}")));
        }

        let mut len = len;
        let mut sub_id_err = None;
        let exposed_id = id::ENTITY_METHOD.to_exposed_id(config.len() as u16, id, || {
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


/// A client-directed property update on an entity (either its base or cell slice, both
/// share one flat client-visible property list, see [`crate::app::script::EntityDispatch::properties`]),
/// decoded dynamically against a runtime-computed [`PropertyDef`] table resolved from the
/// loaded script model. Read-only: for a generic wire observer (e.g. a debugging proxy)
/// decoding a live capture without knowing any concrete entity type statically --
/// `base::App` never sends property updates itself.
///
/// Unlike [`EntityMethod`], there is no `Unknown` fallback here: an unrecognized exposed
/// id (e.g. one belonging to a *dynamic* component, whose properties this project's model
/// can't predict at all) has no confirmed wire framing to fall back to. `EntityMethod`'s
/// "assume Variable8" fallback holds for methods but has no equivalent for properties,
/// and guessing wrong here is not just a missed decode: it silently misframes the
/// element and desyncs every following element in the bundle. A desync cascades into
/// reading unrelated bytes as other message ids, up to and including bogus
/// [`SwitchBaseApp`] triggers whose `patch_raw` handling then forwards corrupted data to
/// the game client. An unrecognized exposed id must therefore surface as a read error,
/// so the caller stops decoding this bundle rather than guessing further.
#[derive(Debug, Clone)]
pub struct EntityProperty {
    pub name: Arc<str>,
    pub value: Value,
}

/// Find a property's exposed id in a property table, refusing one past the full slots of
/// the `ENTITY_PROPERTY` range: its sub-id byte would not be read back, see
/// [`EntityProperty::read_length`].
fn find_property<'p>(config: &'p [PropertyDef], name: &str) -> io::Result<(u16, &'p PropertyDef)> {
    let exposed_id = config.iter().position(|def| &*def.name == name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unknown property: {name}")))?;
    Ok((exposed_id as u16, property_by_id(config, exposed_id as u16)?))
}

/// The property of the given exposed id, which must not need a sub-id, see
/// [`find_property`].
fn property_by_id(config: &[PropertyDef], exposed_id: u16) -> io::Result<&PropertyDef> {
    let def = config.get(exposed_id as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("no property of exposed id {exposed_id}")))?;
    match id::ENTITY_PROPERTY.from_exposed_id(config.len() as u16, exposed_id) {
        (_, None) => Ok(def),
        (_, Some(_)) => Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("property {} needs a sub-id, which is not supported", def.name))),
    }
}

/// Write a property's value as an [`EntityProperty`] element carries it, returning the
/// element id.
fn write_property(write: &mut dyn Write, config: &[PropertyDef], exposed_id: u16, def: &PropertyDef, value: &Value) -> io::Result<u8> {
    // A top-level `PYTHON` value is written without its own length, the element's
    // being enough, exactly as `read` takes it.
    match (def.ty.kind(), value) {
        (TyKind::Python, Value::Python(PythonValue::Raw(raw))) => write.write_all(raw)?,
        (TyKind::Python, Value::Python(PythonValue::Decoded(value))) =>
            write.write_all(&serde_pickle::value_to_vec(value, serde_pickle_ser_options())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?)?,
        _ => Codec::write(value, write, &def.ty)?,
    }
    Ok(id::ENTITY_PROPERTY.from_exposed_id(config.len() as u16, exposed_id).0)
}

impl Element<[PropertyDef]> for EntityProperty {

    fn write_length(&self, config: &[PropertyDef]) -> io::Result<ElementLength> {
        Ok(find_property(config, &self.name)?.1.length)
    }

    fn write(&self, write: &mut dyn Write, config: &[PropertyDef]) -> io::Result<u8> {
        let (exposed_id, def) = find_property(config, &self.name)?;
        write_property(write, config, exposed_id, def, &self.value)
    }

    fn read_length(config: &[PropertyDef], id: u8) -> io::Result<ElementLength> {
        
        if !id::ENTITY_PROPERTY.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity property element id: {id:02X}")));
        }

        match id::ENTITY_PROPERTY.to_exposed_id_checked(config.len() as u16, id) {
            // See this type's doc comment for why an unrecognized exposed id must error
            // here instead of guessing a fallback length like `EntityMethod` does.
            Some(exposed_id) => config.get(exposed_id as usize).map(|def| def.length)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unrecognized entity property element id: {id:02X}"))),
            None => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unrecognized entity property element id: {id:02X}"))),
        }

    }

    fn read(read: &mut dyn Read, config: &[PropertyDef], _len: usize, id: u8) -> io::Result<Self> {
        
        if !id::ENTITY_PROPERTY.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity property element id: {id:02X}")));
        }

        // No sub-id handling: `read_length` above already rejected any id outside the
        // known full-slot range before `read` is ever reached (see this type's doc
        // comment), so overflow sub-ids -- which would only ever be used for an id count
        // which is not modelled here anyway -- can't occur here.
        let exposed_id = id::ENTITY_PROPERTY.to_exposed_id(config.len() as u16, id, || unreachable!(
            "read_length already rejected any id requiring a sub-id"));

        let def = config.get(exposed_id as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unrecognized entity property exposed id: 0x{exposed_id:02X}")))?;

        // A top-level `PYTHON` property is read from the *whole* remaining bytes of this
        // already element-length-bounded reader, bypassing `Value::read`'s usual
        // `PythonValue` codec.
        //
        // That codec expects its own embedded packed length, which is right for a
        // `PYTHON` field nested inside a larger concatenated `Dict` (such as
        // `CreateBasePlayer`'s `initialServerSettings`) but wrong here: this element's
        // length already delimits the property exactly, so a second inner length on top
        // of it asks for more bytes than remain.
        let value = if matches!(def.ty.kind(), TyKind::Python) {
            let mut raw = Vec::new();
            read.read_to_end(&mut raw)?;
            Value::Python(match serde_pickle::value_from_reader(&raw[..], serde_pickle_de_options()) {
                Ok(v) => PythonValue::Decoded(v),
                Err(_) => PythonValue::Raw(raw),
            })
        } else {
            Value::read(read, &def.ty)?
        };

        Ok(Self { name: def.name.clone(), value })

    }

}

/// An [`EntityProperty`] given by its exposed id rather than its name, as bound schemas
/// write them (see [`crate::app::entity`]). Write-only: what is read is an
/// [`EntityProperty`].
#[derive(Debug, Clone)]
pub struct IndexedEntityProperty {
    pub exposed_id: u16,
    pub value: Value,
}

impl Element<[PropertyDef]> for IndexedEntityProperty {

    fn write_length(&self, config: &[PropertyDef]) -> io::Result<ElementLength> {
        Ok(property_by_id(config, self.exposed_id)?.length)
    }

    fn write(&self, write: &mut dyn Write, config: &[PropertyDef]) -> io::Result<u8> {
        let def = property_by_id(config, self.exposed_id)?;
        write_property(write, config, self.exposed_id, def, &self.value)
    }

    fn read_length(_config: &[PropertyDef], _id: u8) -> io::Result<ElementLength> {
        unreachable!("IndexedEntityProperty is write-only")
    }

    fn read(_read: &mut dyn Read, _config: &[PropertyDef], _len: usize, _id: u8) -> io::Result<Self> {
        unreachable!("IndexedEntityProperty is write-only")
    }

}


#[cfg(test)]
mod tests {

    use crate::app::bit::BitWriter;
    use crate::script::{TySystem, TyDictProp, TySeq, StringValue, PropertyFlags};

    use super::*;

    /// A real `forcedPosition` body, pinning the field
    /// split: BigWorld's own layout accounts for only 36 of the declared 38 bytes, and
    /// the two extra sit *between* `vehicle_entity_id` and `position`, not at the end.
    /// Reading them at the end instead shifts `position` two bytes early and turns clean
    /// world coordinates into denormal garbage, which is how the bug showed up.
    #[test]
    fn forced_position_field_split() {

        let body = [
            0x7c, 0x3c, 0x48, 0x20, // entity_id  = 541604988
            0xe1, 0x0a, 0x00, 0x00, // space_id   = 2785
            0x00, 0x00, 0x00, 0x00, // vehicle_id = 0
            0x00, 0x00,             // the two unaccounted bytes
            0x6a, 0x90, 0x87, 0x43, 0x30, 0x86, 0x73, 0x42, 0x69, 0x6b, 0xa0, 0x43, // position
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // direction
        ];

        // The declared element length must account for every field, or the bundle reader
        // leaves a tail and desyncs the elements after it.
        assert_eq!(ForcedPosition::LEN, ElementLength::Fixed(body.len() as u32));

        let mut read = &body[..];
        let elt = <ForcedPosition as SimpleCodec>::read(&mut read).unwrap();
        assert!(read.is_empty(), "the body must be consumed exactly");

        assert_eq!(elt.entity_id, 541604988);
        assert_eq!(elt.space_id, 2785);
        assert_eq!(elt.vehicle_entity_id, 0);
        assert_eq!(elt.unk, 0);
        // Plausible world coordinates rather than denormals -- this is the actual check.
        assert_eq!(elt.position, Vec3::new(271.12823, 60.881042, 320.83914));
        assert_eq!(elt.direction, Vec3::new(0.0, 0.0, 0.0));

        let mut written = Vec::new();
        SimpleCodec::write(&elt, &mut written).unwrap();
        assert_eq!(written, body, "codec must round-trip");

    }

    /// A tiny synthetic schema: `foo: { a: UINT8, b: UINT16 }`, `bar: Array<STRING>`
    /// (real/live length 5, unknown to the decoder -- must be guessed). `bar`'s element
    /// is deliberately a length-prefixed `STRING` rather than a bare scalar: a scalar
    /// like `UINT8` has no self-describing structure at all, so *every* length guess
    /// "successfully" parses the trailing bytes as some number of raw scalars (just
    /// with different leftover byte accounting), which is a real, inherent limitation
    /// of this brute-force approach for weakly-typed array elements, not a bug -- this
    /// test picks a type that actually exercises the disambiguating power the technique
    /// relies on for realistic (mostly `Dict`-of-typed-fields) array elements.
    fn const_hex(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    /// [`AddSpaceGeometryMapping`] against bytes taken verbatim off the wire.
    ///
    /// The whole case for this layout is that it accounts for all 109 bytes exactly --
    /// `4 + 8 + 1 + 31 + 64 + 1` -- so the test asserts an empty `rest` as much as it
    /// asserts the fields. The `entry_id` decoding to a sane internal address, rather
    /// than noise, is the other half of the evidence.
    #[test]
    fn add_space_geometry_mapping_from_a_live_capture() {

        let data = const_hex(concat!(
            "de100000",                       // space 4318
            "0a930831401f0900",               // entry id: 10.147.8.49:16415 salt 9
            "1f",                             // path length, 31
            "7370616365732f3235315f62725f626174746c655f63697479335f68773235",
            // transform: identity, 16 little-endian f32
            "0000803f000000000000000000000000",
            "000000000000803f0000000000000000",
            "00000000000000000000803f00000000",
            "0000000000000000000000000000803f",
            "01",                             // trailing byte
        ));
        assert_eq!(data.len(), 109, "the capture was 109 bytes");

        let m = <AddSpaceGeometryMapping as SimpleCodec>::read(&mut &data[..]).unwrap();

        assert_eq!(m.space_id, 4318);
        assert_eq!(m.entry_id.to_string(), "10.147.8.49:16415");
        assert_eq!(m.path, "spaces/251_br_battle_city3_hw25");
        assert_eq!(m.transform, [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ]);
        assert_eq!(m.unk, 1);
        assert!(m.rest.is_empty(), "the layout must consume the payload exactly: {:?}", m.rest);

    }

    /// `CreateEntity` must consume *four* bytes of direction, not three, or `client_data`
    /// starts one byte early and the whole property run is garbage.
    ///
    /// The run here is the head of a real `Vehicle` create from a live battle. Aligned
    /// correctly it starts with its count (`0x0a`, ten properties) and then ascending
    /// exposed ids; one byte early, that count reads as the trailing direction byte
    /// instead and everything after it is nonsense. See [`CreateEntity::direction_unk`].
    #[test]
    fn create_entity_direction_carries_a_trailing_byte() {

        let run = "0a010105010b010d2a800e2c010f0100";
        let body = const_hex(&format!(
            "00\
             00000000\
             0600\
             0000\
             0a62772e64656661756c74\
             000000000000000000000000\
             010203\
             07\
             {run}"));

        let create = <CreateEntity as SimpleCodec>::read(&mut &body[..]).unwrap();

        assert_eq!(create.entity_type_id, 6);
        assert_eq!(create.resource_name, "bw.default");
        assert_eq!(create.direction.0, [0x01, 0x02, 0x03]);
        assert_eq!(create.direction_unk, 0x07);
        // The payload must begin at the run's count byte, not one byte before it.
        assert_eq!(create.client_data, const_hex(run));
        assert_eq!(create.client_data[0], 0x0a, "the real count: ten properties");

    }

    fn test_properties() -> (Vec<PropertyDef>, Ty, Ty, Ty) {
        let mut sys = TySystem::default();
        let u8_ty = sys.find("UINT8").unwrap();
        let u16_ty = sys.find("UINT16").unwrap();
        let string_ty = sys.find("STRING").unwrap();
        let foo_ty = sys.register(Some("Foo".to_string()), TyKind::Dict(TyDict {
            properties: vec![
                TyDictProp { name: Arc::from("a"), ty: u8_ty.clone(), default: None },
                TyDictProp { name: Arc::from("b"), ty: u16_ty.clone(), default: None },
            ],
            allow_none: false,
        }));
        let bar_ty = sys.register(None, TyKind::Array(TySeq { ty: string_ty.clone(), size: None }));
        let properties = vec![
            PropertyDef { name: Arc::from("foo"), ty: foo_ty, length: ElementLength::Variable16, flags: PropertyFlags::AllClients, volatile: false },
            PropertyDef { name: Arc::from("bar"), ty: bar_ty, length: ElementLength::Variable16, flags: PropertyFlags::AllClients, volatile: false },
        ];
        (properties, u16_ty, string_ty, u8_ty)
    }

    /// `read_changed_properties` against bytes taken verbatim off the wire.
    ///
    /// Captured `CreateEntityDetailed` for a `SPGZone` (an artillery strike marker, whose
    /// whole def is `<Parent>AreaOfEffect</Parent>`), so this pins three things at once:
    /// the `count + (exposed id, value)` framing, the inherited property set, and the
    /// size-sorted exposed order `vehicleID, equipmentID, launchTime, strikeTime`.
    #[test]
    fn changed_properties_from_a_live_create() {

        let mut sys = TySystem::default();
        let i32_ty = sys.find("INT32").unwrap();
        let f64_ty = sys.find("FLOAT64").unwrap();

        // AreaOfEffect's client-visible properties, in the order `build_property_table`
        // produces (two 4-byte, then two 8-byte).
        let properties = vec![
            PropertyDef { name: Arc::from("vehicleID"), ty: i32_ty.clone(), length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
            PropertyDef { name: Arc::from("equipmentID"), ty: i32_ty, length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
            PropertyDef { name: Arc::from("launchTime"), ty: f64_ty.clone(), length: ElementLength::Fixed(8), flags: PropertyFlags::AllClients, volatile: false },
            PropertyDef { name: Arc::from("strikeTime"), ty: f64_ty, length: ElementLength::Fixed(8), flags: PropertyFlags::AllClients, volatile: false },
        ];

        let data = const_hex("0400e729890001e53600000266666666b66601410366666666e6660141");
        let decoded = read_changed_properties(&data, &properties);

        assert!(decoded.is_complete(), "must consume the payload exactly: {:?}", decoded.stopped);
        let values = decoded.values;

        let named: Vec<(u8, &str)> = values.iter().map(|(id, name, _)| (*id, &**name)).collect();
        assert_eq!(named, [
            (0, "vehicleID"), (1, "equipmentID"), (2, "launchTime"), (3, "strikeTime"),
        ]);

        assert_eq!(values[0].2, Value::Int32(8989159), "a live vehicle id from that battle");
        assert_eq!(values[1].2, Value::Int32(14053));

        let (Value::Float64(launch), Value::Float64(strike)) = (&values[2].2, &values[3].2) else {
            panic!("launch/strike should be FLOAT64");
        };
        assert!((strike - launch - 6.0).abs() < 1e-6,
            "the strike lands 6s after launch, got {launch} -> {strike}");

        // An empty payload is "nothing changed", not a malformed count:
        // `addChangedProperties` returns before writing the count when it finds nothing to
        // send, and `BWEntity::onProperties` matches that with an up-front
        // `if (!data.remainingLength()) return;`.
        let empty = read_changed_properties(&[], &properties);
        assert!(empty.values.is_empty() && empty.is_complete());

    }

    /// An exposed id the table doesn't cover must keep the properties decoded *before* it.
    ///
    /// This is the live `Vehicle` case: its table is short by the dynamic components, so
    /// a create carries ids past the end. Discarding the whole run there left vehicles
    /// with no tracked values at all, and so no `Array` lengths for compressed paths.
    #[test]
    fn changed_properties_keep_the_prefix_past_an_unknown_id() {

        let mut sys = TySystem::default();
        let i32_ty = sys.find("INT32").unwrap();
        let properties = vec![
            PropertyDef { name: Arc::from("vehicleID"), ty: i32_ty, length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
        ];

        // Three properties promised; the second id (0x62 = 98, as seen live) is past the
        // end of a one-entry table, so everything from there on is unreadable.
        let data = const_hex("0300e729890062deadbeef");
        let decoded = read_changed_properties(&data, &properties);

        assert_eq!(decoded.values.len(), 1, "the readable prefix survives");
        assert_eq!(decoded.values[0].1.as_ref(), "vehicleID");
        assert_eq!(decoded.values[0].2, Value::Int32(8989159));

        let stop = decoded.stopped.expect("must report why it stopped");
        assert_eq!(stop.expected, 3);
        assert!(stop.reason.contains("exposed id 98"), "{}", stop.reason);
        // Cut from the start of the offending property, not mid-value.
        assert_eq!(stop.rest, const_hex("62deadbeef"));

    }

    /// A desync that lands on an *in-range* id must still be caught, by the ascending-id
    /// rule -- otherwise it seeds a real property name with a garbage value.
    #[test]
    fn changed_properties_catch_a_backwards_id() {

        let mut sys = TySystem::default();
        let i32_ty = sys.find("INT32").unwrap();
        let properties = vec![
            PropertyDef { name: Arc::from("a"), ty: i32_ty.clone(), length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
            PropertyDef { name: Arc::from("b"), ty: i32_ty, length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
        ];

        // Two properties, but the second id repeats the first. `addChangedProperties`
        // walks `i` upwards, so this cannot be a real run.
        let data = const_hex("0200e72989000000000001");
        let decoded = read_changed_properties(&data, &properties);

        assert_eq!(decoded.values.len(), 1, "only the property before the desync");
        let stop = decoded.stopped.expect("a backwards id must be reported");
        assert!(stop.reason.contains("desync"), "{}", stop.reason);

    }

    /// Every id resolved, yet bytes are left: the table drifted from the server's.
    #[test]
    fn changed_properties_report_trailing_bytes() {

        let mut sys = TySystem::default();
        let i32_ty = sys.find("INT32").unwrap();
        let properties = vec![
            PropertyDef { name: Arc::from("vehicleID"), ty: i32_ty, length: ElementLength::Fixed(4), flags: PropertyFlags::AllClients, volatile: false },
        ];

        let data = const_hex("0100e7298900ff");
        let decoded = read_changed_properties(&data, &properties);

        assert_eq!(decoded.values.len(), 1);
        let stop = decoded.stopped.expect("trailing bytes must be reported");
        assert!(stop.reason.contains("trailing"), "{}", stop.reason);
        assert_eq!(stop.rest, const_hex("ff"));

    }

    #[test]
    fn nested_dict_field() {

        let (properties, u16_ty, ..) = test_properties();

        // Path: top[0] = "foo" (a Dict) -> leaf index 1 ("b") = 300u16.
        let mut bits = [0u8; 1];
        let mut w = BitWriter::new(&mut bits);
        w.add(1, 1); // continue into "foo"
        w.add(bits_required(2), 0); // index 0 = "foo" (of 2 top-level properties)
        w.add(1, 0); // stop: the leaf is directly within "foo"
        w.add(bits_required(2), 1); // index 1 = "b" (of foo's 2 fields)

        let mut data = bits.to_vec();
        Value::UInt16(300).write(&mut data, &u16_ty).unwrap();

        let mut cursor = io::Cursor::new(&data[..]);
        let config = PropertyPathConfig::new(&properties);
        let result = NestedEntityProperty::read(&mut cursor, &config, data.len(), NestedEntityProperty::ID).unwrap();

        let NestedEntityProperty::Known { path, value } = result else {
            panic!("expected a resolved path, got {result:?}");
        };
        assert!(matches!(&path[..],
            [EntityPropertyPathStep::Field(f), EntityPropertyPathStep::Field(b)]
            if &**f == "foo" && &**b == "b"));
        assert!(matches!(value, Value::UInt16(300)));

    }

    /// Without a tracked value for the `Array` it crosses, a slice path is undecodable.
    ///
    /// The bit width of the index and range fields is a function of the array's live
    /// length, which is not in the type, so without a tracked value there is nothing to
    /// derive it from. Not knowing must be reported as not knowing.
    #[test]
    fn slice_array_range_without_tracked_length_is_undecodable() {

        let (properties, _, string_ty, _) = test_properties();

        // Exactly the payload the tracked-length test below decodes, byte for byte.
        let mut bits = [0u8; 2];
        let mut w = BitWriter::new(&mut bits);
        w.add(1, 1);
        w.add(bits_required(2), 1); // index 1 = "bar", an Array<STRING>
        w.add(1, 0);
        let idx_bits = bits_required(5 + 1);
        w.add(idx_bits, 2);
        w.add(idx_bits, 4);

        let mut data = bits.to_vec();
        Value::String(StringValue::String("ab".to_string())).write(&mut data, &string_ty).unwrap();
        Value::String(StringValue::String("xyz".to_string())).write(&mut data, &string_ty).unwrap();

        let mut out = Vec::new();
        let mut budget = 200_000u32;
        walk_path(PathContainer::Top(&properties), None, None,
            BitReader::new(&data), Vec::new(), true, &mut budget, &mut out);

        assert!(out.is_empty(), "an untracked Array length must yield no reading, got {out:?}");

        // And the element surfaces that as `Unknown` rather than an error, so the bundle
        // keeps going -- see `SliceEntityProperty::Unknown`.
        let mut cursor = io::Cursor::new(&data[..]);
        let config = PropertyPathConfig::new(&properties);
        let decoded = SliceEntityProperty::read(&mut cursor, &config, data.len(), SliceEntityProperty::ID).unwrap();
        assert!(matches!(decoded, SliceEntityProperty::Unknown { .. }), "got {decoded:?}");

    }

    /// The same slice, decoded against the entity's *tracked* value.
    ///
    /// An `Array`'s live length decides how many bits its index and range fields occupy.
    /// Given the real length there is exactly one reading, and nothing is inferred.
    #[test]
    fn slice_array_range_with_tracked_length() {

        let (properties, _, string_ty, _) = test_properties();

        let mut bits = [0u8; 2];
        let mut w = BitWriter::new(&mut bits);
        w.add(1, 1);
        w.add(bits_required(2), 1); // index 1 = "bar"
        w.add(1, 0);
        let idx_bits = bits_required(5 + 1); // live length 5, as tracked below
        w.add(idx_bits, 2);
        w.add(idx_bits, 4);

        let mut data = bits.to_vec();
        Value::String(StringValue::String("ab".to_string())).write(&mut data, &string_ty).unwrap();
        Value::String(StringValue::String("xyz".to_string())).write(&mut data, &string_ty).unwrap();

        // What the proxy tracks for this entity: "bar" currently holds 5 elements.
        let tracked = HashMap::from([(
            Arc::from("bar"),
            Value::Seq((0..5).map(|i| Value::String(StringValue::String(format!("e{i}")))).collect()),
        )]);

        let mut out = Vec::new();
        let mut budget = 200_000u32;
        walk_path(PathContainer::Top(&properties), Some(&tracked), None,
            BitReader::new(&data), Vec::new(), true, &mut budget, &mut out);

        assert_eq!(out.len(), 1, "a tracked length must leave exactly one reading, got {:?}",
            out);
        assert!(matches!(&out[0], PathOutcome::Slice { path, start: 2, end: 4, values }
            if matches!(&path[..], [EntityPropertyPathStep::Field(f)] if &**f == "bar")
            && values == &[
                Value::String(StringValue::String("ab".to_string())),
                Value::String(StringValue::String("xyz".to_string())),
            ]));

        // And the element-level decode agrees, with nothing left undetermined.
        let outcome = decode_compressed_path(&properties, Some(&tracked), &data, true).unwrap();
        assert!(matches!(outcome, PathOutcome::Slice { start: 2, end: 4, .. }));

    }

    /// An entity created with its components in one `CreateChildren`, as a live battle
    /// sends it: the children's nodes are read, framing stops at the first component
    /// (whose initial state needs its schema), and the stream writes back unchanged.
    /// A built id list reads back as the same ids and mask, whatever mix of runs,
    /// deltas, absolute and null ids it takes.
    #[test]
    fn nrl_id_list_encodes() {
        let cases: [(u32, Vec<u32>, Option<Vec<u8>>); 4] = [
            (2173, vec![2174, 2175], Some(vec![2])),
            (10, vec![11, 12, 13, 20, 0, 5, 300, 301], None),
            (100, (101..400).collect(), Some(vec![0xFF; 38])),
            (7, vec![], None),
        ];
        for (base, ids, present) in cases {
            let list = NrlIdList::new(base, ids.clone(), present.clone());
            let mut pos = 0;
            let read = NrlIdList::read_from(&list.raw, &mut pos, base).unwrap();
            assert_eq!(pos, list.raw.len());
            assert_eq!(read.ids, ids);
            assert_eq!(read.present, present);
        }
        // Consecutive ids from the base cost one byte: a run of 2, `2 - 126`.
        assert_eq!(NrlIdList::new(2173, vec![2174, 2175], None).raw, [0x84, 0x00]);
    }

    #[test]
    fn nrl_create_children_reads_nodes() {

        let raw = [
            0x00, 0x05, 0x00, 0x01, 0x01, 0xC5, 0x08, 0x3F, 0x00,  // node 5: entity 4131013
            0x01, 0x00, 0x05, 0x03, 0x02, 0x03, 0x00,              // its children: 8, 10, 13
            0x02, 0x4C, 0x00, 0x00, 0x83, 0x82, 0x01,              // 8: component 76
            0x02, 0x4D, 0x00, 0x00, 0x84, 0x82, 0x02,              // 10: component 77
        ];

        let NrlCreateNode(stream) = <NrlCreateNode as SimpleCodec>::read(&mut &raw[..]).unwrap();
        assert_eq!(stream.messages.len(), 2);
        let NrlBody::CreateChildren { children, nodes } = &stream.messages[1].body else {
            panic!("expected children: {:?}", stream.messages[1].body);
        };
        assert_eq!(children.ids, [8, 10, 13]);
        assert_eq!(nodes.len(), 1);
        let (8, NrlNode::DynComponent { component_type_id: 76, data: None, .. }) = &nodes[0] else {
            panic!("expected component 76: {nodes:?}");
        };
        assert_eq!(stream.rest, &raw[23..]);

        let mut written = Vec::new();
        SimpleCodec::write(&NrlCreateNode(stream), &mut written).unwrap();
        assert_eq!(written, raw);

    }

}
