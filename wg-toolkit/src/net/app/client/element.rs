//! Definition of the elements that can be sent from server to client
//! once connected to the base application..

use std::fmt;
use std::io::{self, Read, Write};
use std::net::SocketAddrV4;

use glam::Vec3;

use tracing::warn;

use crate::net::element::{DebugElementFixed, DebugElementVariable16, ElementLength, Element, SimpleElement};
use crate::util::io::{WgReadExt, WgWriteExt};
use crate::net::codec::SimpleCodec;
use crate::util::AsciiFmt;

use crate::net::app::common::entity::{Entity, Method};


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
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8        = 0x29;  // CALLBACK 0 (1.26.1.1 handler: 1433273D0)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH: u8             = 0x2A;  // CALLBACK 0 (1.26.1.1 handler: 143327430)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW: u8                   = 0x2B;  // CALLBACK 0 (1.26.1.1 handler: 143327490)
    pub const AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR: u8                = 0x2C;  // CALLBACK 0 (1.26.1.1 handler: 1433274F0)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8       = 0x2D;  // CALLBACK 0 (1.26.1.1 handler: 143327550)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH: u8            = 0x2E;  // CALLBACK 0 (1.26.1.1 handler: 1433275B0)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW: u8                  = 0x2F;  // CALLBACK 0 (1.26.1.1 handler: 143327610)
    pub const AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR: u8               = 0x30;  // CALLBACK 0 (1.26.1.1 handler: 143327670)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL: u8          = 0x31;  // CALLBACK 0 (1.26.1.1 handler: 1433276D0)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH: u8               = 0x32;  // CALLBACK 0 (1.26.1.1 handler: 143327730)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW: u8                     = 0x33;  // CALLBACK 0 (1.26.1.1 handler: 143327790)
    pub const AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR: u8                  = 0x34;  // CALLBACK 0 (1.26.1.1 handler: 1433277F0)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL: u8           = 0x35;  // CALLBACK 0 (1.26.1.1 handler: 143327850)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH: u8                = 0x36;  // CALLBACK 0 (1.26.1.1 handler: 1433278B0)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_YAW: u8                      = 0x37;  // CALLBACK 0 (1.26.1.1 handler: 143327910)
    pub const AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR: u8                   = 0x38;  // CALLBACK 0 (1.26.1.1 handler: 143327970)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL: u8          = 0x39;  // CALLBACK 0 (1.26.1.1 handler: 1433279D0)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH: u8               = 0x3A;  // CALLBACK 0 (1.26.1.1 handler: 143327A30)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_YAW: u8                     = 0x3B;  // CALLBACK 0 (1.26.1.1 handler: 143327A90)
    pub const AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR: u8                  = 0x3C;  // CALLBACK 0 (1.26.1.1 handler: 143327AF0)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL: u8             = 0x3D;  // CALLBACK 0 (1.26.1.1 handler: 143327B50)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH: u8                  = 0x3E;  // CALLBACK 0 (1.26.1.1 handler: 143327BB0)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_YAW: u8                        = 0x3F;  // CALLBACK 0 (1.26.1.1 handler: 143327C10)
    pub const AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR: u8                     = 0x40;  // CALLBACK 0 (1.26.1.1 handler: 143327C70)
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


/// Sent from the base when a player should be created, the entity id
/// is given with its type.
/// 
/// The remaining data will later be decoded properly depending on the
/// entity type, it's used for initializing its properties (TODO).
/// For example the `Login` entity receive the account UID.
#[derive(Debug, Clone)]
pub struct CreateBasePlayer<E: Entity> {
    /// The unique identifier of the entity being created.
    pub entity_id: u32,
    /// The entity type id.
    pub entity_type_id: u16,
    /// The actual data to be sent for creating the player's entity.
    pub entity_data: Box<E>,
    /// This integer describe the number of entity components composing
    /// the entity, this value must be strictly equal to the same value
    /// as the client.
    /// 
    /// TODO: This number is used to know how much entity components
    /// must be parsed after this number. Components can be seen as
    /// regular components. **It's not currently implemented.**
    pub entity_components_count: u8,
}

impl<E: Entity> SimpleCodec for CreateBasePlayer<E> {

    fn write(&self, write: &mut dyn Write) -> io::Result<()> {
        write.write_u32(self.entity_id)?;
        write.write_u16(self.entity_type_id)?;
        write.write_blob_variable(&[])?;  // Unknown blob or string?
        self.entity_data.write(&mut *write)?;
        write.write_u8(self.entity_components_count)
    }

    fn read(read: &mut dyn Read) -> io::Result<Self> {
        let entity_id = read.read_u32()?;
        let entity_type_id = read.read_u16()?;
        let unk = read.read_blob_variable()?;
        if !unk.is_empty() {
            warn!("Non empty unknown blob when decoding CreateBasePlayer: {unk:?}");
        }
        Ok(Self {
            entity_id,
            entity_type_id,
            entity_data: Box::new(E::read(&mut *read)?),
            entity_components_count: read.read_u8()?,
        })
    }

}

impl<E: Entity> SimpleElement for CreateBasePlayer<E> {
    const ID: u8 = id::CREATE_BASE_PLAYER;
    const LEN: ElementLength = ElementLength::Variable16;
}


pub type CreateCellPlayer = DebugElementVariable16<{ id::CREATE_CELL_PLAYER }>;
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
pub type SelectAliasedEntity = DebugElementFixed<{ id::SELECT_ALIASED_ENTITY }, 1>;
pub type SelectEntity = DebugElementFixed<{ id::SELECT_ENTITY }, 4>;


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
        pub base_addr: SocketAddrV4,
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


/// Codec for a method call on an entity, the given method type should be the one of
/// the entity being called.
#[derive(Debug, Clone)]
pub struct EntityMethod<M: Method> {
    pub inner: EntityMethodInner<M>,
}

/// The decoded (or not) payload of an [`EntityMethod`].
#[derive(Debug, Clone)]
pub enum EntityMethodInner<M> {
    /// A method call whose exposed id is present in `M`'s generated table.
    Known(M),
    /// An exposed id missing from `M`'s generated table (e.g. a mismatch between the
    /// generated method tables and what the live game actually sends). The real client
    /// falls back to treating such a call as Variable8-framed instead of erroring --
    /// confirmed by hooking `getEntityMethodStreamSize` on a live client instance, which
    /// returns Mercury's `DEFAULT_VARIABLE_LENGTH_HEADER_SIZE` sentinel (-1, i.e. "read
    /// 1 more header byte") for an id it doesn't recognize either. We mirror that framing
    /// here and keep the raw, undecoded body instead of losing bundle synchronization.
    Unknown {
        exposed_id: u16,
        data: Vec<u8>,
    },
}

impl<M: Method> Element<()> for EntityMethod<M> {

    fn write_length(&self, _config: &()) -> io::Result<ElementLength> {
        let (exposed_id, preferred_len) = match &self.inner {
            EntityMethodInner::Known(inner) => (inner.exposed_id(), inner.write_length()),
            EntityMethodInner::Unknown { exposed_id, .. } => (*exposed_id, ElementLength::Variable8),
        };
        // A sub-id is written as an extra byte ahead of the method's own payload (see
        // `write` below), so the preferred length only applies to full-slot ids; ids
        // requiring a sub-id always frame as Variable16, matching `read_length` below.
        let (_, sub_id) = id::ENTITY_METHOD.from_exposed_id(M::count(), exposed_id);
        Ok(if sub_id.is_some() { ElementLength::Variable16 } else { preferred_len })
    }

    fn write(&self, write: &mut dyn Write, _config: &()) -> io::Result<u8> {
        let exposed_id = match &self.inner {
            EntityMethodInner::Known(inner) => inner.exposed_id(),
            EntityMethodInner::Unknown { exposed_id, .. } => *exposed_id,
        };
        let (element_id, sub_id) = id::ENTITY_METHOD.from_exposed_id(M::count(), exposed_id);
        if let Some(sub_id) = sub_id {
            write.write_u8(sub_id)?;
        }
        match &self.inner {
            EntityMethodInner::Known(inner) => { inner.write(write)?; }
            EntityMethodInner::Unknown { data, .. } => write.write_all(data)?,
        }
        Ok(element_id)
    }

    fn read_length(_config: &(), id: u8) -> io::Result<ElementLength> {
        if !id::ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity method element id: {id:02X}")));
        }
        Ok(match id::ENTITY_METHOD.to_exposed_id_checked(M::count(), id) {
            // See `EntityMethodInner::Unknown` for why an unrecognized exposed id falls
            // back to Variable8 here instead of erroring.
            Some(exposed_id) => M::read_length(exposed_id).unwrap_or(ElementLength::Variable8),
            // A sub-id slot: the actual exposed id (and so its preferred length) can only
            // be known once the sub-id byte prefixing the payload has been read, so the
            // whole payload is always Variable16-framed instead.
            None => ElementLength::Variable16,
        })
    }

    fn read(read: &mut dyn Read, _config: &(), len: usize, id: u8) -> io::Result<Self> {
        if !id::ENTITY_METHOD.contains(id) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected entity method element id: {id:02X}")));
        }

        let mut len = len;
        let mut sub_id_err = None;
        let exposed_id = id::ENTITY_METHOD.to_exposed_id(M::count(), id, || {
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
            EntityMethodInner::Known(M::read(read, exposed_id)?)
        } else {
            let mut data = vec![0; len];
            read.read_exact(&mut data)?;
            EntityMethodInner::Unknown { exposed_id, data }
        };
        Ok(Self {
            inner,
        })
    }

}
