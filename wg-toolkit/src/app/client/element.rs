//! Definition of the elements that can be sent from server to client
//! once connected to the base application..

use std::io::{self, Read, Write};
use std::borrow::Cow;
use std::sync::Arc;
use std::fmt;

use glam::Vec3;

use tracing::warn;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::app::bit::BitReader;
use crate::app::dispatch::{ScriptDispatch, EntityDispatch, MethodDef, PropertyDef, MethodCall};
use crate::util::io::{WgReadExt, WgWriteExt, serde_pickle_de_options};
use crate::net::codec::{Codec, SimpleCodec, WgSocketAddrV4};
use crate::script::{Ty, TyKind, TyDict, Value, PythonValue};
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
    // + dir field, in that order) -- but a live re-check of the registration table itself
    // (`re-work/frida/dump_interfaces.js`, re-run 2026-08-31 against the running client)
    // confirms all 24 of these ids (and the `entityMethod`/`entityProperty` ranges below)
    // are genuinely registered with `lengthStyle=CALLBACK`, meaning the client computes
    // each one's byte length via a dedicated per-message-handler function rather than
    // storing one constant. The `Fixed(N)` values below are NOT approximations anymore,
    // though -- static disassembly (radare2) of that length-computing function (vtable
    // slot 3 on each id's handler object; confirmed to be a pure function of `this`, no
    // other inputs) plus live calls into it via Frida (`re-work/frida/dump_avupmsg_lengths.js`)
    // recovered the *exact* ground-truth byte length for all 24 ids directly from the
    // client's own code, not by guessing from captures. The vanilla SDK formula above
    // undercounts every single one of them (this project's original constants matched
    // that undercounting formula, hence the connection-crash bug found and fixed for
    // 0x35 below, and the still-open failure on 0x3C before this fix) -- see the doc
    // comment on the `avatar_update_elements!` macro for the corrected per-id byte
    // breakdown and how the discrepancy decomposes cleanly (every position/direction mode
    // needs more bytes than the vanilla formula assumes, verified against 0x35's
    // independently live-confirmed 12-byte length as ground truth). What those extra
    // bytes actually *contain* is still unconfirmed -- would need disassembling the much
    // larger stream-parsing function (vtable slot 4) to find out, not attempted here.
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8        = 0x29;  // CALLBACK 0, confirmed exact as Fixed(15)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH: u8             = 0x2A;  // CALLBACK 0, confirmed exact as Fixed(13)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW: u8                   = 0x2B;  // CALLBACK 0, confirmed exact as Fixed(12)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR: u8                = 0x2C;  // CALLBACK 0, confirmed exact as Fixed(11)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8       = 0x2D;  // CALLBACK 0, confirmed exact as Fixed(12)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH: u8            = 0x2E;  // CALLBACK 0, confirmed exact as Fixed(10)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW: u8                  = 0x2F;  // CALLBACK 0, confirmed exact as Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR: u8               = 0x30;  // CALLBACK 0, confirmed exact as Fixed(8)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL: u8          = 0x31;  // CALLBACK 0, confirmed exact as Fixed(9)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH: u8               = 0x32;  // CALLBACK 0, confirmed exact as Fixed(7)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW: u8                     = 0x33;  // CALLBACK 0, confirmed exact as Fixed(6)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR: u8                  = 0x34;  // CALLBACK 0, confirmed exact as Fixed(5)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8           = 0x35;  // CALLBACK 0, confirmed exact as Fixed(12) (originally found via live capture, now cross-confirmed via disassembly)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH: u8                = 0x36;  // CALLBACK 0, confirmed exact as Fixed(10)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW: u8                      = 0x37;  // CALLBACK 0, confirmed exact as Fixed(9)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR: u8                   = 0x38;  // CALLBACK 0, confirmed exact as Fixed(8)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8          = 0x39;  // CALLBACK 0, confirmed exact as Fixed(9)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH: u8               = 0x3A;  // CALLBACK 0, confirmed exact as Fixed(7)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW: u8                     = 0x3B;  // CALLBACK 0, confirmed exact as Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR: u8                  = 0x3C;  // CALLBACK 0, confirmed exact as Fixed(5) -- this is the id that was previously failing live at the wrong Fixed(4)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL: u8             = 0x3D;  // CALLBACK 0, confirmed exact as Fixed(6)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH: u8                  = 0x3E;  // CALLBACK 0, confirmed exact as Fixed(4)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW: u8                        = 0x3F;  // CALLBACK 0, confirmed exact as Fixed(3)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR: u8                     = 0x40;  // CALLBACK 0, confirmed exact as Fixed(2)

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
        // TEMPORARY diagnostic: dump whatever trailing bytes follow the count so a live
        // capture with a nonzero count (previously never seen) can finally show the
        // per-component wire encoding this project doesn't understand yet -- see this
        // struct's `entity_components_count` doc comment.
        if entity_components_count != 0 {
            let mut trailer = Vec::new();
            read.read_to_end(&mut trailer)?;
            warn!("CreateBasePlayer: entity_id={entity_id} entity_components_count={entity_components_count}, trailing bytes: {trailer:?}");
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
/// actually adds it to the AoI id-alias table. Field order/types otherwise confirmed
/// against the leaked BigWorld 14.4.1 SDK's vanilla `ServerConnection::createEntity`
/// (`connection/server_connection.cpp`), but WoT inserts two extra fields
/// (`unk_u16`/`resource_name`, see their own doc comments) between `entity_type_id` and
/// `position` that don't exist in that vanilla source -- confirmed live (WoT v2.3.1.3):
/// assuming vanilla's layout made every single `position`/`direction` decode to the
/// exact same nonsense value regardless of entity (reconstructing the raw wire bytes
/// showed those "positions" were actually always the literal bytes of `resource_name`'s
/// length-prefixed string, misread 13 bytes too early), for every entity type observed
/// (`Vehicle`, `AreaDestructibles`, `NetworkEntity`, ...) -- so this extra pair is
/// unconditional, not specific to one entity type. Like [`CreateCellPlayer`]/
/// [`CreateBasePlayer`], another case of WoT diverging from this same vanilla source.
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
    /// Always `0` in every capture seen so far -- meaning unconfirmed (possibly a
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
    /// Raw pre-encoded client-visible property dict bytes (only the entity's
    /// `AllClients`-flagged properties, in exposed-id order). This project doesn't have
    /// a dispatch table for that specific subset yet (unlike [`CreateBasePlayer`]'s own
    /// `data_ty`, built for the different `AllClients | OwnClient | BaseAndClient` set
    /// exposed to an entity's *own* client), so it's carried opaque for now.
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

impl SimpleCodec for CreateEntity {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u8(0)?; // BW_COMPRESSION_NONE
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_u16(self.unk_u16)?;
        write.write_string_variable(&self.resource_name)?;
        write.write_vec3(self.position)?;
        SimpleCodec::write(&self.direction, write)?;
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
        let mut client_data = Vec::new();
        read.read_to_end(&mut client_data)?;
        Ok(Self { entity_id, entity_type_id, unk_u16, resource_name, position, direction, client_data })
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
// That vanilla layout is 1 byte short of this project's own already-confirmed sizes
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

pub type AvatarUpdateVolatileProperties = DebugElementVariable16<{ id::AVATAR_UPDATE_VOLATILE_PROPERTIES }>;
pub type ChangeVolatilePackerType = DebugElementVariable16<{ id::CHANGE_VOLATILE_PACKER_TYPE }>;

// =============================================================================
// Network Replication Layer ("NRL") -- `BW::NRL`, the subsystem that replicates
// *dynamic components* (their properties and their method calls). It is separate from
// classic `EntityProperty`/`EntityMethod` and absent from the vanilla BigWorld SDK.
//
// Everything is a node in one tree mirroring entity -> dynamic component ->
// property/method, and every message addresses a node by a 16-bit id. The shapes below
// come from the v2.4.0.0 client (`BWEndPoint`, `Hub::processMessage`,
// `ClientNodeFactory::create`) and are validated against real captures.
// =============================================================================

/// One node kind in the NRL tree, as `ClientNodeFactory::create` instantiates them
/// (the wire carries the discriminant as a single byte). The tree mirrors
/// entity -> dynamic component -> property/method.
///
/// Only the fields the client reads off the wire in `readCreate` are decoded here. The
/// value a property or method node reads next is encoded per the *component's own*
/// schema, addressed by a CGF-runtime component type id this project cannot yet resolve
/// (see [`NrlNode::DynComponent::component_type_id`]), so those bytes are left in
/// [`NrlStream::rest`] rather than guessed at.
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
        /// Index into the client's *CGF* component registry, which is not this
        /// project's `components.xml`/dispatch ordering: the wire says 113 for
        /// `EntityMarkerComponent` where `dynamic_components` has it at 67. Resolving
        /// component property schemas needs that registry dumped from the live client.
        component_type_id: u16,
        /// Often empty; otherwise an instance name such as `"EntityMarkerComponent"`,
        /// `"XrayDebuff"` or a vehicle sub-part slot.
        name: String,
        /// Node ids for the property children created alongside this component.
        children: NrlIdList,
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
    /// Type 1: create several children of this node in one go. Each id in `children`
    /// is then followed on the wire by its own node-type byte and creation payload,
    /// which land in [`NrlStream::rest`] (terminal).
    CreateChildren {
        children: NrlIdList,
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
    fn read_with(read: &mut dyn Read, implied: Option<u8>) -> io::Result<Self> {
        let mut buf = Vec::new();
        read.read_to_end(&mut buf)?;

        let mut messages = Vec::new();
        let mut pos = 0;
        let mut implied = implied;

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
                    (NrlBody::CreateChildren { children }, true)
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

        Ok(Self { messages, rest: buf[pos..].to_vec() })
    }

    fn write_with(&self, write: &mut dyn Write, implied: bool) -> io::Result<()> {
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
                NrlBody::CreateChildren { children } => write.write_all(&children.raw)?,
                NrlBody::UnlinkTree | NrlBody::Lookup
                | NrlBody::UpdateNode | NrlBody::Other { .. } => {}
            }
        }
        write.write_all(&self.rest)
    }

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
            (NrlNode::DynComponent { component_type_id, name, children }, true)
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
        NrlNode::DynComponent { component_type_id, name, children } => {
            write.write_u8(2)?;
            write.write_u16(*component_type_id)?;
            write.write_string_variable(name)?;
            write.write_all(&children.raw)
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
/// The payload's encoding belongs to the owning component's schema, which needs a
/// component registry this project cannot yet resolve (see
/// [`NrlNode::DynComponent::component_type_id`]), so it is kept raw. For the *scripted*
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

/// Drains one message-type-2 (unlink) record from the [`NrlData`] reassembly buffer;
/// it has no payload of its own.
pub type NrlUnlinkTreeFlag = DebugElementFixed<{ id::NRL_UNLINK_TREE_FLAG }, 0>;
/// Drains one message-type-6 (update) record from the [`NrlData`] reassembly buffer;
/// it has no payload of its own.
pub type NrlUpdateNodeFlag = DebugElementFixed<{ id::NRL_UPDATE_NODE_FLAG }, 0>;

/// The decoded body of an [`NrlUnreliableMsgToClient`], keyed off its `kind` byte.
#[derive(Debug, Clone)]
pub enum NrlUnreliableMsgToClientPayload {
    /// The one shape decoded so far (`kind` `0x30`): 9 constant bytes of unknown
    /// meaning, 7 little-endian `f32`s, then 2 more constant bytes. **None of the 7
    /// floats' meaning is confirmed.** `floats[3..6]` looked like a raw position at
    /// first (plausible in-map values), but its norm stays a constant `~792.0` across
    /// every sample despite each component changing continuously -- it traces a
    /// sphere, not free movement, so it's some fixed-radius/normalized direction
    /// vector instead. `floats[0..3]` only jump occasionally (a rarely-updated
    /// reference point?) and `floats[6]` stays near-zero.
    SevenFloats {
        unk_prefix: [u8; 9],
        floats: [f32; 7],
        unk_suffix: [u8; 2],
    },
    /// Any other `(kind, payload)` shape -- only the 8-byte `kind=0x11` shape seen
    /// live so far, not decoded.
    Raw(Vec<u8>),
}

/// WoT's own CGF "unreliable" node message (delivered best-effort, no
/// retransmission) -- originates from a single subject (`unk_id_a`/`unk_id_b`
/// constant for a whole battle) sent roughly every 100ms. `tick`/`prev_tick` are a
/// client-interpolation pair (this message's tick, and the last tick it has data
/// for); `prev_tick` normally trails `tick` by 1 but skipped once in one capture --
/// consistent with the channel actually being unreliable. `flags` bit `0x20` tracks
/// whether `prev_tick` holds real data (`0`, with `prev_tick` also `0`, only in the
/// first message of a capture); no other bit was ever observed set.
#[derive(Debug, Clone)]
pub struct NrlUnreliableMsgToClient {
    /// Constant for one subject across a whole battle -- likely a per-subject id
    /// pair, wider than [`NrlCreateNode::network_id`] and unconfirmed if related.
    pub unk_id_a: u32,
    pub unk_id_b: u32,
    pub tick: u32,
    pub prev_tick: u32,
    /// Correlates 1:1 with the payload shape: `0x30` for
    /// [`NrlUnreliableMsgToClientPayload::SevenFloats`], `0x11` for the 8-byte
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
            NrlUnreliableMsgToClientPayload::SevenFloats { unk_prefix, floats, unk_suffix } => {
                write.write_u8(9 + 4 * 7 + 2)?;
                write.write_all(unk_prefix)?;
                for f in floats {
                    write.write_f32(*f)?;
                }
                write.write_all(unk_suffix)?;
            }
            NrlUnreliableMsgToClientPayload::Raw(bytes) => {
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
                    (only 06 00 01 confirmed live so far)")));
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
                    (only D6 C3 C4 00 00 01 confirmed live so far)")));
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
            let mut floats = [0f32; 7];
            for f in &mut floats {
                *f = cur.read_f32()?;
            }
            let mut unk_suffix = [0u8; 2];
            cur.read_exact(&mut unk_suffix)?;
            NrlUnreliableMsgToClientPayload::SevenFloats { unk_prefix, floats, unk_suffix }
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

/// One step of a decoded [`NestedEntityProperty`]/[`SliceEntityProperty`] path: either a
/// named field of a `Dict` (or, for the first step, the entity's own top-level property
/// list) or a positional element of an `Array`/`Tuple`.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityPropertyPathStep {
    Field(Arc<str>),
    Index(u32),
}

/// Upper bound on the `Array` length this project will guess while decoding a
/// compressed property path -- see [`NestedEntityProperty`]'s doc comment.
const MAX_SEQ_LEN_GUESS: u32 = 4096;

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
enum PathOutcome {
    Single { path: Vec<EntityPropertyPathStep>, value: Value },
    Slice { path: Vec<EntityPropertyPathStep>, start: u32, end: u32, values: Vec<Value> },
}

/// One surviving interpretation from [`walk_path`], plus whether the `Array` length it
/// assumed makes the slice a pure append. Kept beside the outcome so de-duplication
/// compares only the semantic result.
struct PathCandidate {
    outcome: PathOutcome,
    append_consistent: bool,
}

/// A direct port of the real engine's `PropertyChangeReader::readCompressedPathAndApply`
/// (`entitydef/property_change_reader.cpp`): reads `[continue bit][index]` pairs,
/// descending through `container`, until a `0` continue bit is read, then performs the
/// leaf action (a single index for [`NestedEntityProperty`], an index range for
/// [`SliceEntityProperty`]) directly against whichever container was reached.
///
/// Crossing an `Array` (an unresolved element count, unlike a `Tuple`) branches over
/// every length in `0..=MAX_SEQ_LEN_GUESS`, since this project doesn't track live
/// per-entity-instance state the way the real engine does -- `out` collects every
/// length guess that leads to a fully self-consistent decode (the whole message parsing
/// cleanly to its last byte); `budget` bounds the total work across all branches taken
/// together, and the search stops early past a handful of matches (already enough to
/// call the decode ambiguous).
fn walk_path<'a>(
    container: PathContainer<'a>,
    mut reader: BitReader<'a>,
    path: Vec<EntityPropertyPathStep>,
    is_slice: bool,
    budget: &mut u32,
    out: &mut Vec<PathCandidate>,
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

        let mut child_path = path;
        child_path.push(step);

        match container_kind(child_ty) {
            ContainerKind::Resolved(child) => walk_path(child, reader, child_path, is_slice, budget, out),
            ContainerKind::UnresolvedArray(elem_ty) => {
                for len in 0..=MAX_SEQ_LEN_GUESS {
                    if *budget == 0 || out.len() >= 16 {
                        break;
                    }
                    walk_path(PathContainer::Seq(elem_ty, len), reader, child_path.clone(), is_slice, budget, out);
                }
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

        // Every `count` in one `bits_required(count + 1)` bracket reads the same bits, so
        // the guess is never pinned exactly; an append is satisfied by exactly one member
        // of the bracket, which is how `decode_compressed_path` breaks ties.
        let append_consistent = start == count && end == count;
        out.push(PathCandidate {
            outcome: PathOutcome::Slice { path, start, end, values },
            append_consistent,
        });

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
        out.push(PathCandidate {
            outcome: PathOutcome::Single { path: leaf_path, value },
            append_consistent: false,
        });

    }

}

/// Decode a compressed path, returning the outcome and whether its `Array` length had to
/// be inferred via the append rule (see below) rather than being unambiguous outright.
fn decode_compressed_path(top: &[PropertyDef], data: &[u8], is_slice: bool) -> io::Result<(PathOutcome, bool)> {

    let mut out = Vec::new();
    let mut budget = 200_000u32;
    walk_path(PathContainer::Top(top), BitReader::new(data), Vec::new(), is_slice, &mut budget, &mut out);

    // Different `Array` length guesses that land in the same `bits_required` bracket
    // read identical index/range bits and thus produce a byte-identical `PathOutcome`
    // (the guessed length itself isn't part of the outcome) -- these aren't a real
    // ambiguity, just the same answer reached via different guesses, so collapse them
    // before deciding whether more than one *distinct* decode actually matched. The
    // append flag is OR-ed across the collapsed group: at most one member of a bracket
    // can satisfy `count == start == end`, and whether *any* did is what matters.
    let mut unique: Vec<PathCandidate> = Vec::new();
    for candidate in out {
        match unique.iter_mut().find(|u| u.outcome == candidate.outcome) {
            Some(existing) => existing.append_consistent |= candidate.append_consistent,
            None => unique.push(candidate),
        }
    }

    if unique.len() == 1 {
        return Ok((unique.pop().unwrap().outcome, false));
    }

    if unique.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData,
            "entity property path: no consistent decode found (possibly an Array whose live length isn't known)"));
    }

    // Tie-break: prefer the single reading where the assumed length equals `start`/`end`,
    // i.e. the slice appends at the very end of the array. Exactly one member of a
    // `bits_required` bracket can satisfy that, so it resolves the tie with no per-entity
    // state -- which matters because a decode only ever pins the length to within a
    // bracket, leaving delta-tracking no sound starting point.
    //
    // A genuine deletion (`[0..n] = []`) has the shape this rejects, so it would be
    // mis-picked when an append-consistent reading also parses. Hence the returned flag:
    // callers must surface an inferred decode distinctly from an unambiguous one.
    if unique.iter().filter(|c| c.append_consistent).count() == 1 {
        let outcome = unique.into_iter().find(|c| c.append_consistent).unwrap().outcome;
        return Ok((outcome, true));
    }

    let outcomes: Vec<&PathOutcome> = unique.iter().map(|c| &c.outcome).collect();
    Err(io::Error::new(io::ErrorKind::InvalidData,
        format!("entity property path: ambiguous decode, {} distinct candidates matched: {outcomes:?}", unique.len())))

}

/// A client-directed update to a single field somewhere inside one of an entity's
/// properties -- e.g. one element of an array-of-dicts property, or one field within a
/// dict property -- addressed by a bit-packed "compressed path" rather than a wire id,
/// see [`crate::app::dispatch::EntityDispatch::properties`]. Decoded per the real
/// engine's `PropertyChangeReader::readCompressedPathAndApply`
/// (`entitydef/property_change_reader.cpp`, see [`walk_path`]).
///
/// This message is the standard, generic BigWorld property-sync path for updating a
/// single element of a replicated array/dict property -- e.g. WoT's own
/// `NetworkReplicationPointComponent.status` (an array of `{prefabPath, recreateMethod,
/// networkID, parentID, active}` dicts feeding `cgf_network`'s `ObjectCommand`/
/// `ReplicationState` API, see `project_wot_proxy_debug_session` memory) uses exactly
/// this message (its `setNested_status` callback) for per-element updates -- genuinely
/// unrelated to the WoT-specific `Nrl*` binary messages despite superficially serving a
/// similar purpose.
///
/// Decoding a path that crosses an `Array` (whose current length isn't known statically,
/// unlike a `Tuple`) requires guessing that length by brute-force search over the
/// remaining bytes (bounded, [`MAX_SEQ_LEN_GUESS`]) for the one length that makes the
/// rest of the message parse cleanly to the very last byte -- the same self-describing-
/// search technique already used for `NrlCreateNode::VehiclePrefab`. This is inherent to
/// this project having no per-entity-instance state tracking (the real engine instead
/// tracks each array's live length as it goes); an ambiguous or inconclusive guess
/// surfaces as a decode error rather than a silently wrong pick.
///
/// Two concrete, confirmed (via a hand-built unit test, `compressed_path_tests`) ways
/// this guessing can produce a genuine false positive, both inherent to the technique
/// rather than a decoding bug: (1) an all-zero (or otherwise plausible-looking) padding
/// byte between the bit-packed path and the byte-aligned value data can itself parse as
/// a valid short value (e.g. an empty `STRING`) under a *different* wrong length guess
/// whose own `align_to_byte` lands earlier, inside that padding; (2) for a
/// [`SliceEntityProperty`] specifically, once a guessed length is large enough that
/// `start`/`end` trivially satisfy `<= length`, a *different* `idx_bits` bracket that
/// happens to round up to the *same* byte-aligned offset can reinterpret the same bits
/// as a different, equally self-consistent `start`/`end` pair over the exact same
/// trailing value bytes. Both surface correctly as an "ambiguous decode" error rather
/// than a silent wrong pick, but reduce how often a real `Array`-crossing path decodes
/// at all -- not yet verified byte-exact against a real live capture, where these
/// collisions' actual frequency on real data is unknown.
#[derive(Debug, Clone)]
pub struct NestedEntityProperty {
    /// The path from the entity's top-level property list down to the changed field,
    /// e.g. `[Field("status"), Index(3), Field("active")]`.
    pub path: Vec<EntityPropertyPathStep>,
    pub value: Value,
}

impl NestedEntityProperty {
    pub const ID: u8 = id::NESTED_ENTITY_PROPERTY;
}

impl Element<[PropertyDef]> for NestedEntityProperty {

    fn write_length(&self, _config: &[PropertyDef]) -> io::Result<ElementLength> {
        unreachable!("NestedEntityProperty is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &[PropertyDef]) -> io::Result<u8> {
        unreachable!("NestedEntityProperty is read-only")
    }

    fn read_length(_config: &[PropertyDef], _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &[PropertyDef], len: usize, _id: u8) -> io::Result<Self> {
        let mut data = vec![0u8; len];
        read.read_exact(&mut data)?;
        match decode_compressed_path(config, &data, false)?.0 {
            PathOutcome::Single { path, value } => Ok(Self { path, value }),
            PathOutcome::Slice { .. } => unreachable!("decode_compressed_path(is_slice=false) always returns Single"),
        }
    }

}

/// A client-directed update replacing a contiguous range of a replicated `Array`
/// property's elements (Python-slice semantics: `array[start..end] = values`, an empty
/// `values` meaning pure removal) -- see [`NestedEntityProperty`]'s doc comment, which
/// this shares its path-decoding algorithm and live-length-guessing caveat with. Not yet
/// verified byte-exact against a real live capture.
#[derive(Debug, Clone)]
pub struct SliceEntityProperty {
    /// The path from the entity's top-level property list down to the changed `Array`.
    pub path: Vec<EntityPropertyPathStep>,
    pub start: u32,
    pub end: u32,
    pub values: Vec<Value>,
    /// `true` when several interpretations parsed and this one was chosen only for being
    /// the single append-consistent reading -- see [`decode_compressed_path`] for the rule
    /// and the deletion case it gets wrong. Report it distinctly from a certain decode.
    pub seq_len_inferred: bool,
}

impl SliceEntityProperty {
    pub const ID: u8 = id::SLICE_ENTITY_PROPERTY;
}

impl Element<[PropertyDef]> for SliceEntityProperty {

    fn write_length(&self, _config: &[PropertyDef]) -> io::Result<ElementLength> {
        unreachable!("SliceEntityProperty is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &[PropertyDef]) -> io::Result<u8> {
        unreachable!("SliceEntityProperty is read-only")
    }

    fn read_length(_config: &[PropertyDef], _id: u8) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn read(read: &mut dyn Read, config: &[PropertyDef], len: usize, _id: u8) -> io::Result<Self> {
        let mut data = vec![0u8; len];
        read.read_exact(&mut data)?;
        let (outcome, seq_len_inferred) = decode_compressed_path(config, &data, true)?;
        match outcome {
            PathOutcome::Slice { path, start, end, values } =>
                Ok(Self { path, start, end, values, seq_len_inferred }),
            PathOutcome::Single { .. } => unreachable!("decode_compressed_path(is_slice=true) always returns Slice"),
        }
    }

}

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

impl Element<[MethodDef]> for EntityMethod {

    fn write_length(&self, config: &[MethodDef]) -> io::Result<ElementLength> {
        
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

    fn write(&self, write: &mut dyn Write, config: &[MethodDef]) -> io::Result<u8> {
        
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

    fn read_length(config: &[MethodDef], id: u8) -> io::Result<ElementLength> {
        
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

impl Element<[PropertyDef]> for EntityProperty {

    fn write_length(&self, _config: &[PropertyDef]) -> io::Result<ElementLength> {
        unreachable!("EntityProperty is read-only")
    }

    fn write(&self, _write: &mut dyn Write, _config: &[PropertyDef]) -> io::Result<u8> {
        unreachable!("EntityProperty is read-only")
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


#[cfg(test)]
mod tests {

    use crate::app::bit::BitWriter;
    use crate::script::{TySystem, TyDictProp, TySeq, StringValue};

    use super::*;

    /// A real `forcedPosition` body captured live (WoT v2.4.0.0), used to pin the field
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
            PropertyDef { name: Arc::from("foo"), ty: foo_ty, length: ElementLength::Variable16 },
            PropertyDef { name: Arc::from("bar"), ty: bar_ty, length: ElementLength::Variable16 },
        ];
        (properties, u16_ty, string_ty, u8_ty)
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
        let result = NestedEntityProperty::read(&mut cursor, &properties, data.len(), NestedEntityProperty::ID).unwrap();

        assert!(matches!(&result.path[..],
            [EntityPropertyPathStep::Field(f), EntityPropertyPathStep::Field(b)]
            if &**f == "foo" && &**b == "b"));
        assert!(matches!(result.value, Value::UInt16(300)));

    }

    /// Exercises `walk_path` directly (rather than the full [`SliceEntityProperty`]
    /// codec) and only checks that the intended decode is *among* its candidates --
    /// not that it's the unique one. `decode_compressed_path`'s brute-force length
    /// guessing has a real, inherent blind spot this test tripped over while being
    /// written: for a guessed count large enough that `start`/`end` trivially satisfy
    /// `<= count`, a *different* `idx_bits` bracket that happens to round up to the
    /// *same* byte-aligned offset (`align_to_byte`) can reinterpret the same bits as a
    /// different, but equally self-consistent, `start`/`end` pair over the exact same
    /// trailing value bytes -- e.g. `idx_bits=3` (`start=2,end=4`) and `idx_bits=5`
    /// (`start=10,end=15`) both round up to the same byte boundary here, so both
    /// "parse cleanly to the last byte". This is a property of the technique itself
    /// (not a decoding bug), so a hand-built test for it should check the core
    /// recursive logic finds the *right* candidate, not that no wrong one exists too.
    #[test]
    fn slice_array_range() {

        let (properties, _, string_ty, _) = test_properties();

        // Path: top[1] = "bar" (an Array<STRING>, real length 5) -> leaf slice [2..4).
        let mut bits = [0u8; 2];
        let mut w = BitWriter::new(&mut bits);
        w.add(1, 1); // continue into "bar"
        w.add(bits_required(2), 1); // index 1 = "bar"
        w.add(1, 0); // stop: the leaf is "bar" itself
        let idx_bits = bits_required(5 + 1); // real (live) array length is 5
        w.add(idx_bits, 2); // start
        w.add(idx_bits, 4); // end

        let mut data = bits.to_vec();
        Value::String(StringValue::String("ab".to_string())).write(&mut data, &string_ty).unwrap();
        Value::String(StringValue::String("xyz".to_string())).write(&mut data, &string_ty).unwrap();

        let mut out = Vec::new();
        let mut budget = 200_000u32;
        walk_path(PathContainer::Top(&properties), BitReader::new(&data), Vec::new(), true, &mut budget, &mut out);

        assert!(out.iter().map(|c| &c.outcome).any(|outcome| matches!(outcome, PathOutcome::Slice { path, start: 2, end: 4, values }
            if matches!(&path[..], [EntityPropertyPathStep::Field(f)] if &**f == "bar")
            && values == &[
                Value::String(StringValue::String("ab".to_string())),
                Value::String(StringValue::String("xyz".to_string())),
            ])));


    }

}
