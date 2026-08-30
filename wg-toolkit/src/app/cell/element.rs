//! Definition of elements related to cell application.
//!
//! Such elements are sent from the client to the cell application. No fully
//! structured codecs exist yet for most of these, only raw/debug element
//! placeholders (see the `id` module doc for how these ids were obtained).

use crate::net::element::{DebugElementFixed, DebugElementVariable16};


/// Internal module containing all raw elements numerical ids.
///
/// All ids below were confirmed empirically by attaching to a live game process
/// (v2.3.1.3, 2026-08-24) and reading the actual registration order out of
/// `CellAppInterface`'s `Mercury::InterfaceMinder` in memory, directly from
/// `WorldOfTanks.exe` (the client binary). Only 8 elements were registered on the
/// client build — unlike `BaseAppExtInterface`/`ClientInterface` (which each use the
/// full 0x00..0xFE id space, including a big exposed-method/property range), this
/// `CellAppInterface` instance never had `addRange` called on it, so `runExposedMethod`
/// (id 7) is a single anchor id, not the start of a range, at least as registered by
/// the client binary.
pub mod id {
    // --- Cell session handshake ---
    pub const CELL_APP_LOGIN: u8           = 0x00;
    pub const AUTHENTICATE: u8              = 0x01;

    // --- Avatar updates, physics corrections, NRL & method dispatch (raw placeholders) ---
    pub const AVATAR_UPDATE_IMPLICIT: u8    = 0x02;
    pub const AVATAR_UPDATE_EXPLICIT: u8    = 0x03;
    pub const ACK_PHYSICS_CORRECTION: u8    = 0x04;
    pub const REQUEST_ENTITY_UPDATE: u8     = 0x05;
    pub const NRL_MSG_TO_CELL: u8           = 0x06;
    pub const RUN_EXPOSED_METHOD: u8        = 0x07;
}


// =============================================================================
// Cell session handshake
// =============================================================================

pub type CellAppLogin = DebugElementVariable16<{ id::CELL_APP_LOGIN }>;
pub type Authenticate = DebugElementFixed<{ id::AUTHENTICATE }, 8>;

// =============================================================================
// Avatar updates, physics corrections, NRL & method dispatch (raw placeholders)
// =============================================================================

pub type AvatarUpdateImplicit = DebugElementFixed<{ id::AVATAR_UPDATE_IMPLICIT }, 21>;
pub type AvatarUpdateExplicit = DebugElementFixed<{ id::AVATAR_UPDATE_EXPLICIT }, 26>;
pub type AckPhysicsCorrection = DebugElementFixed<{ id::ACK_PHYSICS_CORRECTION }, 4>;
pub type RequestEntityUpdate = DebugElementVariable16<{ id::REQUEST_ENTITY_UPDATE }>;
pub type NrlMsgToCell = DebugElementVariable16<{ id::NRL_MSG_TO_CELL }>;
pub type RunExposedMethod = DebugElementVariable16<{ id::RUN_EXPOSED_METHOD }>;
