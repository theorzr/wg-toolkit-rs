//! A battle's chat, as the client's `bw_chat2` messenger has it: actions, by
//! `MESSENGER_ACTION_IDS`, the client's through its avatar's base, the server's to its
//! client, each with its arguments, see [`ChatArgs`].
//!
//! ```text
//!   client                              server
//!   REINIT_BATTLE_CHAT            ->
//!                                 <-    INIT_BATTLE_CHAT         its history, none
//!   BROADCAST_BATTLE_MESSAGE      ->                             strArg1   its text
//!     (request id)                                               int32Arg1 to all, not
//!                                                                          only its team
//!                                 <-    RESPONSE_SUCCESS         (that request id)
//!                                 <-    ON_BATTLE_MESSAGE_BROADCAST, to each it is for:
//!                                         floatArg1 when, in seconds since the epoch
//!                                         int32Arg1 to all
//!                                         int64Arg1 the vehicle of who sent it
//!                                         strArg1   its text
//! ```
//!
//! The client only sends once its battle chat is set up, given its history, empty here:
//! once it is ready, and again whenever it asks. It awaits an answer to each of its
//! messages, 10 at most. A line starting with `/` is a command to the server, answered
//! to it alone, see `Battle::chat`, as a message of the server, see [`server_message`].
//!
//! A message of the server is from no vehicle, 0: the client finds no sender in its
//! arena, and so filters none of its text, which may hold HTML, where it escapes its
//! players' messages. It shows it after an empty name, in the colour its text says.

use std::time::{SystemTime, UNIX_EPOCH};
use std::io;

use flate2::write::ZlibEncoder;
use flate2::Compression;

use wgtk::app::entity::{Bytes, EntityId, EntityKey};
use wgtk::pickle::{self, Value as PickleValue};
use wgtk::app::base;

use crate::game::schema::{Avatar, ChatArgs};


/// The colours of a message of the server, see [`server_message`]: gold, the client's
/// own warnings', or red for an error.
const SERVER_COLOR: &str = "#FFC364";
const SERVER_ERROR_COLOR: &str = "#FF5A4A";

/// The chat actions, `MESSENGER_ACTION_IDS` in the client's `messenger_common_chat2.py`.
pub const RESPONSE_SUCCESS: i16 = 0;
pub const INIT_BATTLE_CHAT: i16 = 19;
pub const REINIT_BATTLE_CHAT: i16 = 20;
pub const BROADCAST_BATTLE_MESSAGE: i16 = 22;
pub const ON_BATTLE_MESSAGE_BROADCAST: i16 = 23;


/// Set up a client's battle chat, as its avatar is told: its history, none.
pub fn init(app: &mut base::App, avatar: EntityKey<Avatar>) -> io::Result<()> {

    // `transport.z_dumps([])`: the client loads it, and fails on anything else.
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    pickle::to_writer(&mut encoder, &PickleValue::List(Vec::new()))?;

    app.entity(avatar)?.messenger_onActionByServer_chat2(INIT_BATTLE_CHAT, 0, ChatArgs {
        strArg1: Bytes(encoder.finish()?),
        ..ChatArgs::default()
    })

}

/// Answer a client's request of this id, done.
pub fn answer(app: &mut base::App, avatar: EntityKey<Avatar>, request_id: u16, action_id: i16) -> io::Result<()> {
    app.entity(avatar)?.messenger_onActionByServer_chat2(RESPONSE_SUCCESS, request_id, ChatArgs {
        int32Arg1: action_id.into(),
        ..ChatArgs::default()
    })
}

/// Show a client a message from the vehicle of this entity id, sent now, to all if
/// `common`, else to its team.
pub fn message(
    app: &mut base::App,
    avatar: EntityKey<Avatar>,
    from: EntityId,
    common: bool,
    text: &str,
) -> io::Result<()> {

    let sent = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64());

    app.entity(avatar)?.messenger_onActionByServer_chat2(ON_BATTLE_MESSAGE_BROADCAST, 0, ChatArgs {
        int32Arg1: common.into(),
        int64Arg1: from.0.into(),
        floatArg1: sent,
        strArg1: Bytes(text.as_bytes().to_vec()),
        ..ChatArgs::default()
    })

}

/// Show a client a message of the server, to its team, in its colour, see the module's
/// documentation: an error's red, else gold.
pub fn server_message(app: &mut base::App, avatar: EntityKey<Avatar>, text: &str, error: bool) -> io::Result<()> {
    let color = if error { SERVER_ERROR_COLOR } else { SERVER_COLOR };
    let text = format!("<font color='{color}'>{}</font>", escape(text));
    message(app, avatar, EntityId(0), false, &text)
}

/// A text escaped for HTML, as the client escapes its players' messages.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
