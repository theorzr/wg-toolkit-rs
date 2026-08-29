//! Definition of the elements that can be sent from server to client
//! once connected to the base application..

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;

use glam::Vec3;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::net::codec::{Codec, SimpleCodec, WgSocketAddrV4};
use crate::util::io::{WgReadExt, WgWriteExt};
use crate::app::script::{MethodDef, PropertyDef, MethodCall};
use crate::script::{Ty, Value};
use crate::util::AsciiFmt;


/// Internal module containing all raw elements numerical ids.
pub mod id {

    use crate::net::element::ElementIdRange;

    pub const AUTHENTICATE: u8                                          = 0x00;  // FIXED 4 (1.26.1.1 handler: 143326C40)
    pub const BANDWIDTH_NOTIFICATION: u8                                = 0x01;  // FIXED 4 (1.26.1.1 handler: 143326C58)
    pub const UPDATE_FREQUENCY_NOTIFICATION: u8                         = 0x02;  // FIXED 7 (1.26.1.1 handler: 143326C70)
    pub const SET_GAME_TIME: u8                                         = 0x03;  // FIXED 4 (1.26.1.1 handler: 143326C88)
    pub const RESET_ENTITIES: u8                                        = 0x04;  // FIXED 1 (1.26.1.1 handler: 143326CA0)
    pub const CREATE_BASE_PLAYER: u8                                    = 0x05;  // VAR 2 (1.26.1.1 handler: 143326CC0)
    pub const CREATE_CELL_PLAYER: u8                                    = 0x06;  // VAR 2 (1.26.1.1 handler: 143326D10)
    pub const DUMMY_PACKET: u8                                          = 0x07;  // VAR 2 (1.26.1.1 handler: 143326D60)
    pub const SPACE_PROPERTY: u8                                        = 0x08;  // VAR 2 (1.26.1.1 handler: 143326DB0)
    pub const ADD_SPACE_GEOMETRY_MAPPING: u8                            = 0x09;  // VAR 2 (1.26.1.1 handler: 143326E00)
    pub const REMOVE_SPACE_GEOMETRY_MAPPING: u8                         = 0x0A;  // VAR 2 (1.26.1.1 handler: 143326E50)
    pub const CREATE_ENTITY: u8                                         = 0x0B;  // VAR 2 (1.26.1.1 handler: 143326EA0)
    pub const CREATE_ENTITY_DETAILED: u8                                = 0x0C;  // VAR 2 (1.26.1.1 handler: 143326EF0)
    pub const CELL_APP_SUSPENDED: u8                                    = 0x0D;  // FIXED 0 (1.26.1.1 handler: 143326F38)
    pub const CELL_APP_RESUMED: u8                                      = 0x0E;  // FIXED 0 (1.26.1.1 handler: 143326F50)
    pub const CLIENT_SUSPENSION_DETECTION_ENABLED: u8                   = 0x0F;  // FIXED 4 (1.26.1.1 handler: 143326F68)
    pub const ENTER_AOI: u8                                             = 0x10;  // FIXED 5 (1.26.1.1 handler: 143326F80)
    pub const ENTER_AOI_ON_VEHICLE: u8                                  = 0x11;  // FIXED 9 (1.26.1.1 handler: 143326F98)
    pub const LEAVE_AOI: u8                                             = 0x12;  // VAR 2 (1.26.1.1 handler: 143326FB0)
    pub const TICK_SYNC: u8                                             = 0x13;  // FIXED 1 (1.26.1.1 handler: 143326FF8)
    pub const TICK_SYNC_PERIODIC: u8                                    = 0x14;  // FIXED 2 (1.26.1.1 handler: 143327010)
    pub const RELATIVE_POSITION_REFERENCE: u8                           = 0x15;  // FIXED 1 (1.26.1.1 handler: 143327028)
    pub const RELATIVE_POSITION: u8                                     = 0x16;  // FIXED 12 (1.26.1.1 handler: 143327040)
    pub const SET_VEHICLE: u8                                           = 0x17;  // FIXED 8 (1.26.1.1 handler: 143327058)
    pub const SELECT_ALIASED_ENTITY: u8                                 = 0x18;  // FIXED 1 (1.26.1.1 handler: 143327070)
    pub const SELECT_ENTITY: u8                                         = 0x19;  // FIXED 4 (1.26.1.1 handler: 143327088)
    pub const SELECT_PLAYER_ENTITY: u8                                  = 0x1A;  // FIXED 0 (1.26.1.1 handler: 1433270A0)
    pub const FORCED_POSITION: u8                                       = 0x1B;  // FIXED 38 (1.26.1.1 handler: 1433270B8)
    pub const AVATAR_UPDATE_NO_ALIAS_DETAILED: u8                       = 0x1C;  // FIXED 29 (1.26.1.1 handler: 1433270D0)
    pub const AVATAR_UPDATE_ALIAS_DETAILED: u8                          = 0x1D;  // FIXED 26 (1.26.1.1 handler: 1433270E8)
    pub const AVATAR_UPDATE_PLAYER_DETAILED: u8                         = 0x1E;  // FIXED 25 (1.26.1.1 handler: 143327100)
    pub const AVATAR_UPDATE_VOLATILE_PROPERTIES: u8                     = 0x1F;  // VAR 2 (1.26.1.1 handler: 143327120)
    pub const CHANGE_VOLATILE_PACKER_TYPE: u8                           = 0x20;  // VAR 2 (1.26.1.1 handler: 143327170)
    pub const NRL_CREATE_NODE: u8                                       = 0x21;  // VAR 2 (1.26.1.1 handler: 1433271C0)
    pub const NRL_UNLINK_TREE: u8                                       = 0x22;  // VAR 2 (1.26.1.1 handler: 143327210)
    pub const NRL_UPDATE_NODE: u8                                       = 0x23;  // VAR 2 (1.26.1.1 handler: 143327260)
    pub const NRL_UNLINK_TREE_FLAG: u8                                  = 0x24;  // FIXED 0 (1.26.1.1 handler: 1433272A8)
    pub const NRL_UPDATE_NODE_FLAG: u8                                  = 0x25;  // FIXED 0 (1.26.1.1 handler: 1433272C0)
    pub const NRL_DATA: u8                                              = 0x26;  // VAR 2 (1.26.1.1 handler: 1433272E0)
    pub const NRL_MSG_TO_CLIENT: u8                                     = 0x27;  // VAR 2 (1.26.1.1 handler: 143327330)
    pub const NRL_UNRELIABLE_MSG_TO_CLIENT: u8                          = 0x28;  // VAR 2 (1.26.1.1 handler: 143327380)
    // The 24 AVUPMSG combinations (see `common_client_interface.hpp` in the leaked
    // BigWorld 14.4.1 SDK, `re-work/bigworld-src-14.4.1/`): each combination of
    // {NoAlias 4-byte EntityID, Alias 1-byte IDAlias} x {FullPos 5-byte PackedXYZ,
    // OnGround 3-byte PackedXZ, NoPos none} x {YawPitchRoll 3 bytes, YawPitch 2 bytes,
    // Yaw 1 byte, NoDir none} is registered as its own fixed-size message id (id field
    // + pos field + dir field, in that order) -- all bit-widths are compile-time
    // `#define`s in `msgtypes.hpp`, not runtime/connection state, so these sizes are
    // constant for this client build. Previously mislabeled "CALLBACK 0" (a length-style
    // placeholder, not an actual byte count) -- that mislabeling made a bundle reader
    // misparse these as some other framing, corrupting the read position for anything
    // bundled alongside them.
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8        = 0x29;  // FIXED 12 (1.26.1.1 handler: 1433273D0)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH: u8             = 0x2A;  // FIXED 11 (1.26.1.1 handler: 143327430)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW: u8                   = 0x2B;  // FIXED 10 (1.26.1.1 handler: 143327490)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR: u8                = 0x2C;  // FIXED 9 (1.26.1.1 handler: 1433274F0)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8       = 0x2D;  // FIXED 10 (1.26.1.1 handler: 143327550)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH: u8            = 0x2E;  // FIXED 9 (1.26.1.1 handler: 1433275B0)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW: u8                  = 0x2F;  // FIXED 8 (1.26.1.1 handler: 143327610)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR: u8               = 0x30;  // FIXED 7 (1.26.1.1 handler: 143327670)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL: u8          = 0x31;  // FIXED 7 (1.26.1.1 handler: 1433276D0)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH: u8               = 0x32;  // FIXED 6 (1.26.1.1 handler: 143327730)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW: u8                     = 0x33;  // FIXED 5 (1.26.1.1 handler: 143327790)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR: u8                  = 0x34;  // FIXED 4 (1.26.1.1 handler: 1433277F0)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8           = 0x35;  // FIXED 9 (1.26.1.1 handler: 143327850)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH: u8                = 0x36;  // FIXED 8 (1.26.1.1 handler: 1433278B0)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW: u8                      = 0x37;  // FIXED 7 (1.26.1.1 handler: 143327910)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR: u8                   = 0x38;  // FIXED 6 (1.26.1.1 handler: 143327970)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8          = 0x39;  // FIXED 7 (1.26.1.1 handler: 1433279D0)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH: u8               = 0x3A;  // FIXED 6 (1.26.1.1 handler: 143327A30)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW: u8                     = 0x3B;  // FIXED 5 (1.26.1.1 handler: 143327A90)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR: u8                  = 0x3C;  // FIXED 4 (1.26.1.1 handler: 143327AF0)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL: u8             = 0x3D;  // FIXED 4 (1.26.1.1 handler: 143327B50)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH: u8                  = 0x3E;  // FIXED 3 (1.26.1.1 handler: 143327BB0)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW: u8                        = 0x3F;  // FIXED 2 (1.26.1.1 handler: 143327C10)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR: u8                     = 0x40;  // FIXED 1 (1.26.1.1 handler: 143327C70)
    pub const CONTROL_ENTITY: u8                                        = 0x41;  // FIXED 5 (1.26.1.1 handler: 143327CC8)
    pub const VOICE_DATA: u8                                            = 0x42;  // VAR 2 (1.26.1.1 handler: 143327CE0)
    pub const RESTORE_CLIENT: u8                                        = 0x43;  // VAR 2 (1.26.1.1 handler: 143327D00)
    pub const SWITCH_BASE_APP: u8                                       = 0x44;  // FIXED 9 (1.26.1.1 handler: 143327D48)
    pub const RESOURCE_HEADER: u8                                       = 0x45;  // VAR 2 (1.26.1.1 handler: 143327D60)
    pub const RESOURCE_FRAGMENT: u8                                     = 0x46;  // VAR 2 (1.26.1.1 handler: 143327DB0)
    pub const LOGGED_OFF: u8                                            = 0x47;  // FIXED 1 (1.26.1.1 handler: 143327DF8)
    pub const DETAILED_POSITION: u8                                     = 0x48;  // FIXED 24 (1.26.1.1 handler: 143327E10)
    pub const NESTED_ENTITY_PROPERTY: u8                                = 0x49;  // VAR 2 (1.26.1.1 handler: 143327E30)
    pub const SLICE_ENTITY_PROPERTY: u8                                 = 0x4A;  // VAR 2 (1.26.1.1 handler: 143327E80)
    pub const UPDATE_ENTITY: u8                                         = 0x4B;  // VAR 2 (1.26.1.1 handler: 143327ED0)
    pub const SET_CELL_APP_EXT_ADDRESS: u8                              = 0x4C;  // VAR 2 (1.26.1.1 handler: 143327F20)
    pub const LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION: u8   = 0x4D;  // FIXED 0 (1.26.1.1 handler: 143327F68)
    
    pub const ENTITY_METHOD: ElementIdRange     = ElementIdRange::new(0x4E, 0xA6);  // CALLBACK 0 (1.26.1.1 handler: 143327F80)
    pub const ENTITY_PROPERTY: ElementIdRange   = ElementIdRange::new(0xA7, 0xFE);  // CALLBACK 0 (1.26.1.1 handler: 143327FA8)

}


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


/// The header for the non-generic [`CreateBasePlayer`] element, that can be used to read
/// the header once before 
#[derive(Debug, Clone)]
pub struct CreateBasePlayerHeader {
    /// The unique identifier of the entity being created.
    pub entity_id: u32,
    /// The entity type id.
    pub entity_type_id: u16,
}

impl SimpleCodec for CreateBasePlayerHeader {

    fn write(&self, _write: &mut dyn Write) -> io::Result<()> {
        panic!("this header element should not be used for encoding");
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        Ok(Self {
            entity_id: read.read_u32()?,
            entity_type_id: read.read_u16()?,
        })
    }

}

impl SimpleElement for CreateBasePlayerHeader {
    const ID: u8 = id::CREATE_BASE_PLAYER;
    const LEN: ElementLength = ElementLength::Variable16;
}


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
/// the account UID) -- encoded as a runtime [`Value`] against a runtime-computed [`Ty`]
/// (see [`crate::app::script::EntityDispatch::data_ty`]), resolved dynamically from the
/// loaded script model rather than a statically generated `Entity` struct. Write-only:
/// [`super::super::base::App`] only ever sends these, never decodes them.
#[derive(Debug, Clone)]
pub struct CreateBasePlayer<'a> {
    /// The unique identifier of the entity being created.
    pub entity_id: u32,
    /// The entity type id.
    pub entity_type_id: u16,
    /// The actual data to be sent for creating the player's entity.
    pub entity_data: &'a Value,
    /// The type describing `entity_data`'s layout, see [`crate::app::script::EntityDispatch::data_ty`].
    pub entity_data_ty: &'a Ty,
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

impl Element<()> for CreateBasePlayer<'_> {

    fn write_length(&self, _config: &()) -> io::Result<ElementLength> {
        Ok(ElementLength::Variable16)
    }

    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<u8> {
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_blob_variable(&[])?;  // Unknown blob or string?
        Codec::write(self.entity_data, write, self.entity_data_ty)?;
        write.write_u8(self.entity_components_count)?;
        Ok(id::CREATE_BASE_PLAYER)
    }

    fn read_length(_config: &(), _id: u8) -> io::Result<ElementLength> {
        unreachable!("CreateBasePlayer is write-only")
    }

    fn read(_read: &mut dyn Read, _config: &(), _len: usize, _id: u8) -> io::Result<Self> {
        unreachable!("CreateBasePlayer is write-only")
    }

}


pub type DummyPacket = DebugElementVariable16<{ id::DUMMY_PACKET }>;
pub type SpaceProperty = DebugElementVariable16<{ id::SPACE_PROPERTY }>;
pub type AddSpaceGeometryMapping = DebugElementVariable16<{ id::ADD_SPACE_GEOMETRY_MAPPING }>;
pub type RemoveSpaceGeometryMapping = DebugElementVariable16<{ id::REMOVE_SPACE_GEOMETRY_MAPPING }>;

pub type CreateEntity = DebugElementVariable16<{ id::CREATE_ENTITY }>;
pub type CreateEntityDetailed = DebugElementVariable16<{ id::CREATE_ENTITY_DETAILED }>;

pub type CellAppSuspended = DebugElementFixed<{ id::CELL_APP_SUSPENDED }, 0>;
pub type CellAppResumed = DebugElementFixed<{ id::CELL_APP_RESUMED }, 0>;

pub type ClientSuspensionDetectionEnabled = DebugElementFixed<{ id::CLIENT_SUSPENSION_DETECTION_ENABLED }, 4>;
pub type EnterAoi = DebugElementFixed<{ id::ENTER_AOI }, 5>;
pub type EnterAoiOnVehicle = DebugElementFixed<{ id::ENTER_AOI_ON_VEHICLE }, 9>;
pub type LeaveAoi = DebugElementVariable16<{ id::LEAVE_AOI }>;


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


pub type AvatarUpdateNoAliasDetailed = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_DETAILED }, 29>;
pub type AvatarUpdateAliasDetailed = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_DETAILED }, 26>;
pub type AvatarUpdatePlayerDetailed = DebugElementFixed<{ id::AVATAR_UPDATE_PLAYER_DETAILED }, 25>;

// The 24 AVUPMSG combinations -- see the doc comment on `id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL`.
pub type AvatarUpdateNoAliasFullPosYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL }, 12>;
pub type AvatarUpdateNoAliasFullPosYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH }, 11>;
pub type AvatarUpdateNoAliasFullPosYaw = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW }, 10>;
pub type AvatarUpdateNoAliasFullPosNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR }, 9>;
pub type AvatarUpdateNoAliasOnGroundYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL }, 10>;
pub type AvatarUpdateNoAliasOnGroundYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH }, 9>;
pub type AvatarUpdateNoAliasOnGroundYaw = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW }, 8>;
pub type AvatarUpdateNoAliasOnGroundNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR }, 7>;
pub type AvatarUpdateNoAliasNoPosYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL }, 7>;
pub type AvatarUpdateNoAliasNoPosYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH }, 6>;
pub type AvatarUpdateNoAliasNoPosYaw = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW }, 5>;
pub type AvatarUpdateNoAliasNoPosNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR }, 4>;
pub type AvatarUpdateAliasFullPosYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL }, 9>;
pub type AvatarUpdateAliasFullPosYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH }, 8>;
pub type AvatarUpdateAliasFullPosYaw = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_FULL_POS_YAW }, 7>;
pub type AvatarUpdateAliasFullPosNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR }, 6>;
pub type AvatarUpdateAliasOnGroundYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL }, 7>;
pub type AvatarUpdateAliasOnGroundYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH }, 6>;
pub type AvatarUpdateAliasOnGroundYaw = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_ON_GROUND_YAW }, 5>;
pub type AvatarUpdateAliasOnGroundNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR }, 4>;
pub type AvatarUpdateAliasNoPosYawPitchRoll = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL }, 4>;
pub type AvatarUpdateAliasNoPosYawPitch = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH }, 3>;
pub type AvatarUpdateAliasNoPosYaw = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_NO_POS_YAW }, 2>;
pub type AvatarUpdateAliasNoPosNoDir = DebugElementFixed<{ id::AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR }, 1>;

pub type AvatarUpdateVolatileProperties = DebugElementVariable16<{ id::AVATAR_UPDATE_VOLATILE_PROPERTIES }>;
pub type ChangeVolatilePackerType = DebugElementVariable16<{ id::CHANGE_VOLATILE_PACKER_TYPE }>;

pub type NrlCreateNode = DebugElementVariable16<{ id::NRL_CREATE_NODE }>;
pub type NrlUnlinkTree = DebugElementVariable16<{ id::NRL_UNLINK_TREE }>;
pub type NrlUpdateNode = DebugElementVariable16<{ id::NRL_UPDATE_NODE }>;
pub type NrlUnlinkTreeFlag = DebugElementFixed<{ id::NRL_UNLINK_TREE_FLAG }, 0>;
pub type NrlUpdateNodeFlag = DebugElementFixed<{ id::NRL_UPDATE_NODE_FLAG }, 0>;
pub type NrlData = DebugElementVariable16<{ id::NRL_DATA }>;
pub type NrlMsgToClient = DebugElementVariable16<{ id::NRL_MSG_TO_CLIENT }>;
pub type NrlUnreliableMsgToClient = DebugElementVariable16<{ id::NRL_UNRELIABLE_MSG_TO_CLIENT }>;

// TODO: Avatar update

pub type ControlEntity = DebugElementFixed<{ id::CONTROL_ENTITY }, 5>;
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


pub type DetailedPosition = DebugElementFixed<{ id::DETAILED_POSITION }, 24>;

pub type NestedEntityProperty = DebugElementVariable16<{ id::NESTED_ENTITY_PROPERTY }>;
pub type SliceEntityProperty = DebugElementVariable16<{ id::SLICE_ENTITY_PROPERTY }>;
pub type UpdateEntity = DebugElementVariable16<{ id::UPDATE_ENTITY }>;
pub type SetCellAppExtAddress = DebugElementVariable16<{ id::SET_CELL_APP_EXT_ADDRESS }>;
pub type LastProxyMessageAfterDirectCellAppConnection = DebugElementVariable16<{ id::LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION }>;


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

        Ok(Self { name: def.name.clone(), value: Value::read(read, &def.ty)? })
    }

}
