//! Definition of elements related to cell application.
//!
//! Such elements are sent from the client to the cell application. No fully
//! structured codecs exist yet for most of these, only raw/debug element
//! placeholders (see the `id` module doc for how these ids were obtained).

use crate::net::element::{DebugElementFixed, DebugElementVariable16};


/// Internal module containing all raw elements numerical ids.
///
/// The ids follow the registration order of `CellAppInterface`'s
/// `Mercury::InterfaceMinder` in the client. Only 8 elements are registered there,
/// unlike `BaseAppExtInterface`/`ClientInterface` (which each use the
/// full 0x00..0xFE id space, including a big exposed-method/property range), this
/// `CellAppInterface` instance never had `addRange` called on it, so `runExposedMethod`
/// (id 7) is a single anchor id, not the start of a range, at least as registered by
/// the client binary.
pub mod id {
    // No `CALLBACK` ids in this interface, same as `base::element`'s
    // `BaseAppExtInterface`.

    // --- Cell session handshake ---
    pub const CELL_APP_LOGIN: u8            = 0x00;  // VAR 2
    pub const AUTHENTICATE: u8              = 0x01;  // FIXED 8

    // --- Avatar updates, physics corrections, NRL & method dispatch (raw placeholders) ---
    pub const AVATAR_UPDATE_IMPLICIT: u8    = 0x02;  // FIXED 21
    pub const AVATAR_UPDATE_EXPLICIT: u8    = 0x03;  // FIXED 26
    pub const ACK_PHYSICS_CORRECTION: u8    = 0x04;  // FIXED 4
    pub const REQUEST_ENTITY_UPDATE: u8     = 0x05;  // VAR 2
    pub const NRL_MSG_TO_CELL: u8           = 0x06;  // VAR 2
    pub const RUN_EXPOSED_METHOD: u8        = 0x07;  // VAR 2
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
