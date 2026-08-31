//! Definition of the elements that can be sent from server to client
//! once connected to the base application..

use std::io::{self, Read, Write};
use std::borrow::Cow;
use std::sync::Arc;
use std::fmt;

use glam::Vec3;

use tracing::warn;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::app::dispatch::{ScriptDispatch, EntityDispatch, MethodDef, PropertyDef, MethodCall};
use crate::util::io::{WgReadExt, WgWriteExt, serde_pickle_de_options};
use crate::net::codec::{Codec, SimpleCodec, WgSocketAddrV4};
use crate::script::{Ty, TyKind, Value, PythonValue};
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

    // --- Cell suspend/resume & client suspension detection (not in vanilla BigWorld,
    // confirmed live via `re-work/frida/dump_interfaces.js`) ---
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
    // + dir field, in that order) -- but a fresh live re-check of the registration table
    // itself (`re-work/frida/dump_interfaces.js`, re-run 2026-08-31 against the running
    // client) CONFIRMS all 24 of these ids (and the `entityMethod`/`entityProperty`
    // ranges below) are genuinely registered with `lengthStyle=CALLBACK`, not `FIXED` --
    // an earlier pass through this file had concluded the opposite (that CALLBACK was
    // some kind of stale/placeholder mislabeling and these were "really" fixed-size),
    // which was wrong: CALLBACK means the client asks a per-message callback for the
    // byte count of each individual instance rather than using one constant, so the
    // *true* wire length of any of these ids can vary message-to-message depending on
    // runtime state this project hasn't investigated (a good candidate given the
    // existence of `CHANGE_VOLATILE_PACKER_TYPE` nearby, but not confirmed -- not
    // pursued further here per explicit decision to not dig into what CALLBACK-style
    // elements compute). The `Fixed(N)` declarations below are this project's own
    // approximation, good enough for the common case observed live, but NOT a wire
    // guarantee -- `AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL` (0x35) needed a
    // different constant than the vanilla-SDK-derived formula in a live capture (see its
    // hand-written impl below), and `AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR` (0x3C) has
    // since been observed failing to decode at its declared `Fixed(4)` in another live
    // capture too -- both consistent with this being a systemic "declared length is only
    // an approximation" issue across the whole CALLBACK family, not one-off bugs in
    // isolated ids.
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8        = 0x29;  // CALLBACK 0, approximated here as Fixed(12)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH: u8             = 0x2A;  // CALLBACK 0, approximated here as Fixed(11)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW: u8                   = 0x2B;  // CALLBACK 0, approximated here as Fixed(10)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR: u8                = 0x2C;  // CALLBACK 0, approximated here as Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8       = 0x2D;  // CALLBACK 0, approximated here as Fixed(10)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH: u8            = 0x2E;  // CALLBACK 0, approximated here as Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW: u8                  = 0x2F;  // CALLBACK 0, approximated here as Fixed(8)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR: u8               = 0x30;  // CALLBACK 0, approximated here as Fixed(7)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL: u8          = 0x31;  // CALLBACK 0, approximated here as Fixed(7)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH: u8               = 0x32;  // CALLBACK 0, approximated here as Fixed(6)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW: u8                     = 0x33;  // CALLBACK 0, approximated here as Fixed(5)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR: u8                  = 0x34;  // CALLBACK 0, approximated here as Fixed(4)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8           = 0x35;  // CALLBACK 0, approximated here as Fixed(12) -- see hand-written impl below
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH: u8                = 0x36;  // CALLBACK 0, approximated here as Fixed(8)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW: u8                      = 0x37;  // CALLBACK 0, approximated here as Fixed(7)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR: u8                   = 0x38;  // CALLBACK 0, approximated here as Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8          = 0x39;  // CALLBACK 0, approximated here as Fixed(7)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH: u8               = 0x3A;  // CALLBACK 0, approximated here as Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW: u8                     = 0x3B;  // CALLBACK 0, approximated here as Fixed(5)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR: u8                  = 0x3C;  // CALLBACK 0, approximated here as Fixed(4) -- known to sometimes need more, unresolved
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL: u8             = 0x3D;  // CALLBACK 0, approximated here as Fixed(4)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH: u8                  = 0x3E;  // CALLBACK 0, approximated here as Fixed(3)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW: u8                        = 0x3F;  // CALLBACK 0, approximated here as Fixed(2)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR: u8                     = 0x40;  // CALLBACK 0, approximated here as Fixed(1)

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
/// CONFIRMED against a live capture by disassembling this project's actual target (WoT
/// v2.3.1.3)'s own `ServerConnection::createCellPlayer` handler (found via the live
/// `ClientInterface` message table, `re-work/frida/dump_cellplayer_handler*.js`): this
/// fork's wire layout is NOT a plain byte-for-byte match for vanilla 14.4.1's `stream >>
/// spaceID_ >> vehicleID >> pos >> packedXZScale_ >> dir` (same as [`CreateBasePlayer`]
/// needing extra fields vanilla doesn't have) -- it inserts a leading byte and widens the
/// space between `space_id` and `vehicle_id` by a 2-byte field, confirmed against real
/// capture bytes: `packed_xz_scale` came out byte-for-byte identical across two different
/// battles (a per-server-config constant, as expected) and `position`/`direction` came out
/// as plausible in-map meter-scale values, both of which failed completely under the old
/// (wrong) offsets. `direction` is still assumed to be 3 raw `f32`s (yaw/pitch/roll,
/// BigWorld's conventional `Direction3D` layout) -- unconfirmed beyond "values are in a
/// plausible radian range". There's also no generated per-entity "cell properties" struct
/// yet (unlike `entity_data: Codec<()>` on [`CreateBasePlayer`], entities only have
/// base-exposed properties modelled today), so the trailing property-dict stream is
/// carried as a raw pre-encoded blob (`cell_data`) the caller must build.
#[derive(Debug, Clone)]
pub struct CreateCellPlayer {
    /// Always `0` in every capture seen so far -- meaning unconfirmed.
    pub unk_flag: u8,
    /// The id of the space this entity now lives in.
    pub space_id: u32,
    /// Always `0` in every capture seen so far -- meaning unconfirmed.
    pub unk_short: u16,
    /// CONFIRMED live (WoT v2.3.1.3, actual battle capture): the id of a distinct
    /// `Vehicle` entity, not the player's own base/`Account` entity -- a later
    /// `SelectEntity` targets this same id for the vehicle's own property/method
    /// updates (see `wg-toolkit-cli`'s proxy `CreateCellPlayer` handling, which
    /// registers it). Empirically the exact same value also appears again inside
    /// `cell_data` right after a constant marker byte.
    pub vehicle_id: u32,
    pub position: Vec3,
    /// The server's packed-XZ compression scale, needed by the client to decode any
    /// later packed-XZ position updates -- this project doesn't send those (no compressed
    /// `AvatarUpdate*` elements are implemented), so this value has no effect today.
    pub packed_xz_scale: f32,
    /// Yaw/pitch/roll -- see the struct doc comment for why this encoding is unconfirmed.
    pub direction: Vec3,
    /// Raw pre-encoded cell-exposed property dict bytes -- see the struct doc comment.
    pub cell_data: Vec<u8>,
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
    /// The number of *dynamic* components attached to this specific entity instance --
    /// confirmed against `wg-toolkit-cli/src/bootstrap/mod.rs`'s own extension-parsing
    /// comments: WoT's "StaticComponents" (declared per `extension.xml`) fold their
    /// methods/properties into every instance's own method table at codegen time, so
    /// they need no runtime handling at all and aren't counted here. "DynamicComponents"
    /// are instead attached to individual entity instances at runtime (e.g. only for the
    /// duration of a particular battle mode), don't claim a fixed exposed id, and are
    /// what this count refers to.
    ///
    /// TODO: no live capture with a nonzero count has been analyzed yet, so the actual
    /// per-component wire encoding that would follow (id/name + its own data) isn't
    /// confirmed -- this field is read/written verbatim (round-trips correctly even at
    /// 0, the only value seen so far) but the components themselves aren't decoded.
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
        unreachable!("CreateBasePlayer<ScriptDispatch> is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &ScriptDispatch) -> io::Result<u8> {
        unreachable!("CreateBasePlayer<ScriptDispatch> is read-only")
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

pub type DummyPacket = DebugElementVariable16<{ id::DUMMY_PACKET }>;
pub type SpaceProperty = DebugElementVariable16<{ id::SPACE_PROPERTY }>;
pub type AddSpaceGeometryMapping = DebugElementVariable16<{ id::ADD_SPACE_GEOMETRY_MAPPING }>;
pub type RemoveSpaceGeometryMapping = DebugElementVariable16<{ id::REMOVE_SPACE_GEOMETRY_MAPPING }>;

/// Sent from the cell when another entity (not the client's own player) enters its Area
/// of Interest -- see [`EnterAoi`]/[`EnterAoiOnVehicle`] for the companion message that
/// actually adds it to the AoI id-alias table. Layout confirmed against the leaked
/// BigWorld 14.4.1 SDK's vanilla `ServerConnection::createEntity`
/// (`connection/server_connection.cpp`) -- NOT yet confirmed against a live WoT capture,
/// unlike [`CreateCellPlayer`]/[`CreateBasePlayer`] (both found to diverge from this same
/// vanilla source once checked).
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
    pub position: Vec3,
    /// Yaw/pitch/roll, packed the same way as the `AVATAR_UPDATE_*_YAW_PITCH_ROLL`
    /// family (see [`PackedYawPitchRoll`]) but with `HALFPITCH` forced to `false` here
    /// (confirmed: `ServerConnection::createEntity` explicitly instantiates
    /// `PackedYawPitchRoll</* HALFPITCH */ false>`, unlike the avatar-update messages'
    /// default `HALFPITCH = true`) -- decode with [`PackedYawPitchRoll::unpack`]
    /// passing `half_pitch = false`.
    pub direction: PackedYawPitchRoll,
    /// Raw pre-encoded client-visible property dict bytes (only the entity's
    /// `AllClients`-flagged properties, in exposed-id order). This project doesn't have
    /// a dispatch table for that specific subset yet (unlike [`CreateBasePlayer`]'s own
    /// `data_ty`, built for the different `AllClients | OwnClient | BaseAndClient` set
    /// exposed to an entity's *own* client), so it's carried opaque for now.
    pub client_data: Vec<u8>,
}

impl SimpleCodec for CreateEntity {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(0)?; // BW_COMPRESSION_NONE
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_vec3(self.position)?;
        SimpleCodec::write(&self.direction, write)?;
        write.write_all(&self.client_data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let compression_type = read.read_u8()?;
        if compression_type != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported,
                format!("CreateEntity: compressed payload (type {compression_type}) not supported")));
        }
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let position = read.read_vec3()?;
        let direction = SimpleCodec::read(read)?;
        let mut client_data = Vec::new();
        read.read_to_end(&mut client_data)?;
        Ok(Self { entity_id, entity_type_id, position, direction, client_data })
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
/// not confirmed against a live WoT capture, and the whole payload (this struct's fields
/// included) is `CompressionIStream`-wrapped, with only `BW_COMPRESSION_NONE` handled.
#[derive(Debug, Clone)]
pub struct CreateEntityDetailed {
    pub entity_id: u32,
    pub entity_type_id: u16,
    pub position: Vec3,
    /// Yaw/pitch/roll as three raw, uncompressed `f32`s (unlike [`CreateEntity::direction`]).
    pub direction: Vec3,
    /// See [`CreateEntity::client_data`].
    pub client_data: Vec<u8>,
}

impl SimpleCodec for CreateEntityDetailed {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(0)?; // BW_COMPRESSION_NONE
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_vec3(self.position)?;
        write.write_vec3(self.direction)?;
        write.write_all(&self.client_data)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let compression_type = read.read_u8()?;
        if compression_type != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported,
                format!("CreateEntityDetailed: compressed payload (type {compression_type}) not supported")));
        }
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let position = read.read_vec3()?;
        let direction = read.read_vec3()?;
        let mut client_data = Vec::new();
        read.read_to_end(&mut client_data)?;
        Ok(Self { entity_id, entity_type_id, position, direction, client_data })
    }

}

impl SimpleElement for CreateEntityDetailed {
    const ID: u8 = id::CREATE_ENTITY_DETAILED;
    const LEN: ElementLength = ElementLength::Variable16;
}

// `cellAppSuspended`/`cellAppResumed`/`clientSuspensionDetectionEnabled` don't exist in
// the leaked BigWorld 14.4.1 SDK at all (like the `Nrl*` elements above, apparently a
// later-engine or WoT-specific addition) -- their names and fixed lengths below are
// instead confirmed live by dumping the real client's own registered `ClientInterface`
// message table (`re-work/frida/dump_interfaces.js`, ids `0x0D`-`0x0F`), which is where
// this project's names for them (and the `id` module's byte counts) originally came
// from; there's no header to cross-check field-level semantics against, though, so
// [`ClientSuspensionDetectionEnabled`]'s single field is a plausible guess, not confirmed.

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
    /// length is confirmed live (`re-work/frida/dump_interfaces.js`); there's no header
    /// to confirm the field's exact type or unit against, so `threshold` is a plausible
    /// reading (a timeout/period, `f32` seconds or `u32` milliseconds) rather than a
    /// confirmed one -- kept as a raw `u32` here to avoid asserting a specific meaning.
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
    /// message: `EntityID id; IDAlias idAlias;`), matching this project's own
    /// already-confirmed 5-byte length for `ENTER_AOI`.
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
    /// EntityID vehicleID; IDAlias idAlias;`), matching this project's own
    /// already-confirmed 9-byte length for `ENTER_AOI_ON_VEHICLE`.
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


pub type TickSyncPeriodic = DebugElementFixed<{ id::TICK_SYNC_PERIODIC }, 2>;
pub type RelativePositionReference = DebugElementFixed<{ id::RELATIVE_POSITION_REFERENCE }, 1>;
pub type RelativePosition = DebugElementFixed<{ id::RELATIVE_POSITION }, 12>;
pub type SetVehicle = DebugElementFixed<{ id::SET_VEHICLE }, 8>;

crate::__struct_simple_codec! {
    /// Sent by the server to inform that subsequent elements will target another entity's
    /// property/method updates, referenced directly by its full id -- confirmed live
    /// against WoT v2.3.1.3: the controlled `Vehicle` entity created by
    /// [`CreateCellPlayer`] gets its own `OwnClient`-flagged property updates selected
    /// this way, distinct from [`SelectPlayerEntity`] (the base/`Account` entity) and
    /// from the more compact byte-alias form used for broadcast (`AllClients`)
    /// properties of *other* nearby entities, [`SelectAliasedEntity`] (not decoded here
    /// -- its alias table would need [`CreateEntity`]/[`EnterAoi`] decoded first, neither
    /// confirmed live yet).
    #[derive(Debug, Clone, Copy)]
    pub struct SelectEntity {
        pub entity_id: u32,
    }
}

impl SimpleElement for SelectEntity {
    const ID: u8 = id::SELECT_ENTITY;
    const LEN: ElementLength = ElementLength::Fixed(4);
}

pub type SelectAliasedEntity = DebugElementFixed<{ id::SELECT_ALIASED_ENTITY }, 1>;


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
// That vanilla layout is 1 byte short of this project's own already-confirmed sizes
// (28/25/24 vs the confirmed-live 29/26/25 -- see the `id` module), in all three cases by
// exactly the same amount. The same SDK's `BaseAppExtInterface::avatarUpdateImplicit`
// (`connection/baseapp_ext_interface.hpp`, the base-directed sibling of these) has a
// directly analogous trailing `uint8 refNum` appended after `pos`/`dir` specifically
// under this build's `VOLATILE_POSITIONS_ARE_ABSOLUTE == 0` configuration (confirmed
// still in effect, see `msgtypes.hpp`) -- "refNum is used to refer to this position later
// as the base for relative positions", matching this doc comment's own "detailed enough
// to be reference positions" wording almost verbatim. A trailing `ref_num: u8` here
// accounts for the missing byte exactly in all three messages independently, which is
// strong circumstantial support, but this hasn't been confirmed against a live WoT
// capture (unlike e.g. [`CreateCellPlayer`]'s own confirmed deviations from vanilla).
crate::__struct_simple_codec! {
    #[derive(Debug, Clone, Copy)]
    pub struct AvatarUpdateNoAliasDetailed {
        pub entity_id: u32,
        pub position: Vec3,
        /// Yaw/pitch/roll -- see [`ForcedPosition::direction`] for why the exact float
        /// encoding is unconfirmed beyond "plausible radian values".
        pub direction: Vec3,
        /// See this type's doc comment: likely a reference-position sequence number, not
        /// confirmed live.
        pub ref_num: u8,
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
        pub id_alias: u8,
        pub position: Vec3,
        pub direction: Vec3,
        pub ref_num: u8,
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
        pub position: Vec3,
        pub direction: Vec3,
        pub ref_num: u8,
    }
}

impl SimpleElement for AvatarUpdatePlayerDetailed {
    const ID: u8 = id::AVATAR_UPDATE_PLAYER_DETAILED;
    const LEN: ElementLength = ElementLength::Fixed(25);
}

/// Generates one [`SimpleElement`] struct per `AVUPMSG` combination: a `NoAlias`
/// (`entity_id: u32`) or `Alias` (`id_alias: u8`) target, followed by its position and
/// direction fields -- see the doc comment on
/// `id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL` for how these three independent
/// axes combine into the 24 distinct element ids, and for why every `Fixed($len)` below
/// is only this project's approximation of a wire length the live client actually
/// registers as `CALLBACK` (dynamic, computed per-instance) -- not a guaranteed constant.
macro_rules! avatar_update_elements {
    ($( $name:ident { $id_field:ident: $id_ty:ty, position: $pos_ty:ty, direction: $dir_ty:ty } = $id_const:ident, $len:literal; )*) => {
        $(
            crate::__struct_simple_codec! {
                #[derive(Debug, Clone, Copy)]
                pub struct $name {
                    pub $id_field: $id_ty,
                    pub position: $pos_ty,
                    pub direction: $dir_ty,
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
    AvatarUpdateNoAliasFullPosYawPitchRoll  { entity_id: u32, position: PackedXyz, direction: PackedYawPitchRoll } = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL, 12;
    AvatarUpdateNoAliasFullPosYawPitch      { entity_id: u32, position: PackedXyz, direction: PackedYawPitch }     = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH, 11;
    AvatarUpdateNoAliasFullPosYaw           { entity_id: u32, position: PackedXyz, direction: PackedYaw }          = AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW, 10;
    AvatarUpdateNoAliasFullPosNoDir         { entity_id: u32, position: PackedXyz, direction: () }                 = AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR, 9;
    AvatarUpdateNoAliasOnGroundYawPitchRoll { entity_id: u32, position: PackedXz, direction: PackedYawPitchRoll }  = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL, 10;
    AvatarUpdateNoAliasOnGroundYawPitch     { entity_id: u32, position: PackedXz, direction: PackedYawPitch }      = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH, 9;
    AvatarUpdateNoAliasOnGroundYaw          { entity_id: u32, position: PackedXz, direction: PackedYaw }           = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW, 8;
    AvatarUpdateNoAliasOnGroundNoDir        { entity_id: u32, position: PackedXz, direction: () }                  = AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR, 7;
    AvatarUpdateNoAliasNoPosYawPitchRoll    { entity_id: u32, position: (), direction: PackedYawPitchRoll }        = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL, 7;
    AvatarUpdateNoAliasNoPosYawPitch        { entity_id: u32, position: (), direction: PackedYawPitch }            = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH, 6;
    AvatarUpdateNoAliasNoPosYaw             { entity_id: u32, position: (), direction: PackedYaw }                 = AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW, 5;
    AvatarUpdateNoAliasNoPosNoDir           { entity_id: u32, position: (), direction: () }                        = AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR, 4;
    AvatarUpdateAliasFullPosYawPitch        { id_alias: u8, position: PackedXyz, direction: PackedYawPitch }       = AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH, 8;
    AvatarUpdateAliasFullPosYaw             { id_alias: u8, position: PackedXyz, direction: PackedYaw }            = AVATAR_UPDATE_ALIAS_FULL_POS_YAW, 7;
    AvatarUpdateAliasFullPosNoDir           { id_alias: u8, position: PackedXyz, direction: () }                   = AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR, 6;
    AvatarUpdateAliasOnGroundYawPitchRoll   { id_alias: u8, position: PackedXz, direction: PackedYawPitchRoll }    = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL, 7;
    AvatarUpdateAliasOnGroundYawPitch       { id_alias: u8, position: PackedXz, direction: PackedYawPitch }        = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH, 6;
    AvatarUpdateAliasOnGroundYaw            { id_alias: u8, position: PackedXz, direction: PackedYaw }             = AVATAR_UPDATE_ALIAS_ON_GROUND_YAW, 5;
    AvatarUpdateAliasOnGroundNoDir          { id_alias: u8, position: PackedXz, direction: () }                    = AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR, 4;
    AvatarUpdateAliasNoPosYawPitchRoll      { id_alias: u8, position: (), direction: PackedYawPitchRoll }          = AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL, 4;
    AvatarUpdateAliasNoPosYawPitch          { id_alias: u8, position: (), direction: PackedYawPitch }              = AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH, 3;
    AvatarUpdateAliasNoPosYaw               { id_alias: u8, position: (), direction: PackedYaw }                   = AVATAR_UPDATE_ALIAS_NO_POS_YAW, 2;
    AvatarUpdateAliasNoPosNoDir             { id_alias: u8, position: (), direction: () }                          = AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR, 1;
}

/// `AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL` (id `0x35`) -- kept out of the
/// `avatar_update_elements!` family above because, unlike the rest of it, its observed
/// size does NOT match the vanilla BigWorld SDK formula (`id_alias` 1 + `position` 5 +
/// `direction` 3 = 9): confirmed live (WoT 2.3.1.3, two independent battle captures,
/// each replayed byte-exact through to a clean bundle end with zero further decode
/// errors via `wg-toolkit-cli/examples/replay_bundle.rs`) that this element is actually
/// 12 bytes, with 3 extra trailing bytes this project doesn't yet know the meaning of
/// (values vary a lot between instances, so not padding/a constant marker) -- likely a
/// WoT-specific extension of the vanilla AVUPMSG format, same idea as the WoT-only NRL
/// messages nearby. Previously declared as `Fixed(9)` (matching the vanilla formula), which
/// desynced the bundle reader for everything following it -- this was a real,
/// reproducible cause of the client dropping its connection mid-battle.
///
/// Now understood as one instance of a wider pattern, not an isolated one-off: a fresh
/// live re-check of `ClientInterface`'s own registration table
/// (`re-work/frida/dump_interfaces.js`, re-run 2026-08-31) confirms this id -- and all 23
/// others in the family above, plus `entityMethod`/`entityProperty` -- are registered
/// with `lengthStyle=CALLBACK`, meaning the *true* wire length is computed per-instance
/// by the client itself and is not a fixed constant at all. This project's `Fixed(N)`
/// declarations (this hand-written one included) are only approximations of the common
/// case, not a protocol guarantee -- `AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR` (`0x3C`) has
/// since also been observed failing to decode at its declared `Fixed(4)` in a later live
/// capture, consistent with the same systemic issue rather than a second unrelated bug.
/// What actually drives the callback's length choice (`ChangeVolatilePackerType`-related
/// state is a plausible candidate, given the name) has not been investigated -- out of
/// scope here by design.
#[derive(Debug, Clone, Copy)]
pub struct AvatarUpdateAliasFullPosYawPitchRoll {
    pub id_alias: u8,
    pub position: PackedXyz,
    pub direction: PackedYawPitchRoll,
    /// Unconfirmed meaning -- see this struct's doc comment.
    pub unk: [u8; 3],
}

impl SimpleCodec for AvatarUpdateAliasFullPosYawPitchRoll {
    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(self.id_alias)?;
        SimpleCodec::write(&self.position, write)?;
        SimpleCodec::write(&self.direction, write)?;
        write.write_all(&self.unk)
    }
    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let id_alias = read.read_u8()?;
        let position = <PackedXyz as SimpleCodec>::read(read)?;
        let direction = <PackedYawPitchRoll as SimpleCodec>::read(read)?;
        let mut unk = [0; 3];
        read.read_exact(&mut unk)?;
        Ok(Self { id_alias, position, direction, unk })
    }
}

impl SimpleElement for AvatarUpdateAliasFullPosYawPitchRoll {
    const ID: u8 = id::AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL;
    const LEN: ElementLength = ElementLength::Fixed(12);
}

pub type AvatarUpdateVolatileProperties = DebugElementVariable16<{ id::AVATAR_UPDATE_VOLATILE_PROPERTIES }>;
pub type ChangeVolatilePackerType = DebugElementVariable16<{ id::CHANGE_VOLATILE_PACKER_TYPE }>;

// =============================================================================
// Network Replication Layer ("NRL") -- WoT's own CGF node-replication
// messages (`NetworkReplicationPointComponent.py`), not present in vanilla BigWorld.
// =============================================================================

pub type NrlCreateNode = DebugElementVariable16<{ id::NRL_CREATE_NODE }>;
pub type NrlUnlinkTree = DebugElementVariable16<{ id::NRL_UNLINK_TREE }>;
pub type NrlUpdateNode = DebugElementVariable16<{ id::NRL_UPDATE_NODE }>;
pub type NrlUnlinkTreeFlag = DebugElementFixed<{ id::NRL_UNLINK_TREE_FLAG }, 0>;
pub type NrlUpdateNodeFlag = DebugElementFixed<{ id::NRL_UPDATE_NODE_FLAG }, 0>;
pub type NrlData = DebugElementVariable16<{ id::NRL_DATA }>;
pub type NrlMsgToClient = DebugElementVariable16<{ id::NRL_MSG_TO_CLIENT }>;
pub type NrlUnreliableMsgToClient = DebugElementVariable16<{ id::NRL_UNRELIABLE_MSG_TO_CLIENT }>;

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
    /// `EntityID id; bool on;`), matching this project's own already-confirmed 5-byte
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

pub type VoiceData = DebugElementVariable16<{ id::VOICE_DATA }>;
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
    /// position; Direction3D direction;`), matching this project's own already-confirmed
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

pub type NestedEntityProperty = DebugElementVariable16<{ id::NESTED_ENTITY_PROPERTY }>;
pub type SliceEntityProperty = DebugElementVariable16<{ id::SLICE_ENTITY_PROPERTY }>;
pub type UpdateEntity = DebugElementVariable16<{ id::UPDATE_ENTITY }>;
pub type SetCellAppExtAddress = DebugElementVariable16<{ id::SET_CELL_APP_EXT_ADDRESS }>;
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

/// Find a method by name in an exposed-id-ordered table, for [`EntityMethod`]'s write
/// side (the read side instead looks up by exposed id, see [`EntityMethod::read`]).
fn find_method<'m>(config: &'m [MethodDef], name: &str) -> io::Result<(u16, &'m MethodDef)> {
    config.iter().enumerate()
        .find(|(_, def)| &*def.name == name)
        .map(|(exposed_id, def)| (exposed_id as u16, def))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unknown method: {name}")))
}

impl Element<Vec<MethodDef>> for EntityMethod {

    fn write_length(&self, config: &Vec<MethodDef>) -> io::Result<ElementLength> {
        
        let (exposed_id, preferred_len) = match &self.call {
            MethodCall::Known { name, .. } => {
                let (exposed_id, def) = find_method(config, name)?;
                (exposed_id, def.length)
            }
            MethodCall::Unknown { exposed_id, .. } => (*exposed_id, ElementLength::Variable8),
        };

        // A sub-id is written as an extra byte ahead of the method's own payload (see
        // `write` below), so the preferred length only applies to full-slot ids; ids
        // requiring a sub-id always frame as Variable16, matching `read_length` below.
        let (_, sub_id) = id::ENTITY_METHOD.from_exposed_id(config.len() as u16, exposed_id);
        Ok(if sub_id.is_some() { ElementLength::Variable16 } else { preferred_len })
    
    }

    fn write(&self, write: &mut dyn Write, config: &Vec<MethodDef>) -> io::Result<u8> {
        
        let exposed_id = match &self.call {
            MethodCall::Known { name, .. } => find_method(config, name)?.0,
            MethodCall::Unknown { exposed_id, .. } => *exposed_id,
        };
        
        let (element_id, sub_id) = id::ENTITY_METHOD.from_exposed_id(config.len() as u16, exposed_id);
        if let Some(sub_id) = sub_id {
            write.write_u8(sub_id)?;
        }
        
        match &self.call {
            MethodCall::Known { args, .. } => config[exposed_id as usize].write_args(write, args)?,
            MethodCall::Unknown { data, .. } => write.write_all(data)?,
        }

        Ok(element_id)

    }

    fn read_length(config: &Vec<MethodDef>, id: u8) -> io::Result<ElementLength> {
        
        if !id::ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity method element id: {id:02X}")));
        }

        Ok(match id::ENTITY_METHOD.to_exposed_id_checked(config.len() as u16, id) {
            // An unrecognized exposed id falls back to Variable8 here instead of
            // erroring -- confirmed live by hooking `getEntityMethodStreamSize` on a
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

    fn read(read: &mut dyn Read, config: &Vec<MethodDef>, len: usize, id: u8) -> io::Result<Self> {
        
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
/// "assume Variable8" fallback was confirmed live by hooking the real client's own
/// `getEntityMethodStreamSize`, specifically for methods -- that confirmation was never
/// done for properties, and guessing wrong here isn't just a missed decode: it silently
/// misframes the element, desyncing every following element in the bundle, which was
/// confirmed live to cascade into misinterpreting unrelated garbage bytes as other
/// message ids (observed: bogus `SwitchBaseApp` triggers, whose `patch_raw` handling
/// then corrupted and forwarded real, unrelated packet data to the live game client and
/// crashed it). An unrecognized exposed id must therefore surface as a read error here,
/// so the caller stops decoding this bundle rather than guessing further.
#[derive(Debug, Clone)]
pub struct EntityProperty {
    pub name: Arc<str>,
    pub value: Value,
}

impl Element<Vec<PropertyDef>> for EntityProperty {

    fn write_length(&self, _config: &Vec<PropertyDef>) -> io::Result<ElementLength> {
        unreachable!("EntityProperty is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &Vec<PropertyDef>) -> io::Result<u8> {
        unreachable!("EntityProperty is read-only")
    }

    fn read_length(config: &Vec<PropertyDef>, id: u8) -> io::Result<ElementLength> {
        
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

    fn read(read: &mut dyn Read, config: &Vec<PropertyDef>, _len: usize, id: u8) -> io::Result<Self> {
        
        if !id::ENTITY_PROPERTY.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity property element id: {id:02X}")));
        }

        // No sub-id handling: `read_length` above already rejected any id outside the
        // known full-slot range before `read` is ever reached (see this type's doc
        // comment), so overflow sub-ids -- which would only ever be used for an id count
        // this project doesn't have confirmed anyway -- can't occur here.
        let exposed_id = id::ENTITY_PROPERTY.to_exposed_id(config.len() as u16, id, || unreachable!(
            "read_length already rejected any id requiring a sub-id"));

        let def = config.get(exposed_id as usize)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("unrecognized entity property exposed id: 0x{exposed_id:02X}")))?;

        // A top-level `PYTHON` property is read directly from the *whole* remaining
        // bytes of this already element-length-bounded reader, bypassing `Value::read`'s
        // usual `PythonValue` codec (which additionally expects its own embedded packed
        // length, correct for a `PYTHON` field nested inside a larger concatenated `Dict`
        // -- e.g. `CreateBasePlayer`'s `initialServerSettings`, still working correctly
        // as of this comment -- but confirmed live to systematically overrun here: this
        // project has no way to know a standalone property's declared length ahead of
        // time (see `property_length`'s own doc comment on its `Variable8` guess), but
        // *this* element's own length is already known once we're inside `read` at all,
        // and stacking a second, redundant inner length on top of it consistently asked
        // for more bytes than remained). `EntityProperty`'s framing already fully
        // delimits this property on its own, so no extra inner length is needed here.
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
