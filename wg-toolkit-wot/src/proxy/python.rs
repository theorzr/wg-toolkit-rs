//! The Python-level view of a session: what the account commands, tokens and resource
//! streams carried inside entity method calls mean.
//!
//! See [`crate::wot::account_commands`] for the protocol and the names used here.

use std::collections::HashMap;
use std::time::Instant;

use wgtk::script::Value;

use crate::wot::account_commands::{self as ac, token_type};


/// A request waiting for its answer.
#[derive(Debug)]
struct Pending {
    /// What was asked, e.g. `CMD_SYNC_DATA`.
    label: String,
    sent: Instant,
}

/// Follows commands, tokens and streams across a session and describes each step.
///
/// Each `on_*` method takes one decoded entity method call and returns a one-line
/// description when the call belongs to this layer, or `None` when it does not.
#[derive(Debug, Default)]
pub struct Requests {
    commands: HashMap<i64, Pending>,
    tokens: HashMap<i64, Pending>,
    /// Streams expected because a command answered `RES_STREAM`, or was sent with a
    /// predefined request id, by stream id, with the command they answer.
    streams: HashMap<i64, String>,
}

impl Requests {

    /// A base method call sent by the client.
    pub fn on_base_call(&mut self, method: &str, args: &[Value]) -> Option<String> {

        if method == "requestToken" {
            let (id, ty) = (int_arg(args, 0)?, int_arg(args, 1)?);
            let label = token_label(ty);
            self.tokens.insert(id, Pending { label: label.clone(), sent: Instant::now() });
            return Some(format!("token #{id} {label} requested"));
        }

        // Every `doCmd*` variant starts with the request id and the command code.
        if !method.starts_with("doCmd") {
            return None;
        }
        let (id, code) = (int_arg(args, 0)?, int_arg(args, 1)?);
        let label = i16::try_from(code).ok().and_then(ac::command_name)
            .map(str::to_string)
            .unwrap_or_else(|| format!("CMD {code}"));
        let shown_args = args[2..].iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(", ");

        match i16::try_from(id).ok().and_then(ac::request_id_name) {
            Some("REQUEST_ID_NO_RESPONSE") | Some("REQUEST_ID_NON_CLIENT") => {
                Some(format!("command {label}({shown_args}), no answer expected"))
            }
            // The other predefined ids are answered by a stream of the same id.
            Some(predefined) => {
                self.streams.insert(id, label.clone());
                Some(format!("command {label}({shown_args}), answered on stream {predefined}"))
            }
            None => {
                self.commands.insert(id, Pending { label: label.clone(), sent: Instant::now() });
                Some(format!("command #{id} {label}({shown_args})"))
            }
        }

    }

    /// A client method call sent by the server.
    pub fn on_client_call(&mut self, method: &str, args: &[Value]) -> Option<String> {
        match method {
            "onCmdResponse" | "onCmdResponseExt" => {

                let (id, code) = (int_arg(args, 0)?, int_arg(args, 1)?);
                let code = i16::try_from(code).ok();
                let result = code.and_then(ac::result_name).map(str::to_string)
                    .unwrap_or_else(|| format!("result {}", int_arg(args, 1).unwrap_or_default()));

                let (label, elapsed) = match self.commands.remove(&id) {
                    Some(pending) => (pending.label, format!(" after {} ms", pending.sent.elapsed().as_millis())),
                    // Sent before the proxy started, or on another connection.
                    None => ("(command not seen)".to_string(), String::new()),
                };

                if code == Some(ac::RES_STREAM) {
                    self.streams.insert(id, label.clone());
                }

                let mut line = format!("command #{id} {label}: {result}{elapsed}");
                if let Some(error) = args.get(2).map(|a| format!("{a:?}")).filter(|e| !is_empty_string(e)) {
                    line.push_str(&format!(", error {error}"));
                }
                if let Some(ext) = args.get(3) {
                    line.push_str(&format!(", ext {ext:?}"));
                }
                Some(line)

            }
            "onTokenReceived" => {
                let (id, ty) = (int_arg(args, 0)?, int_arg(args, 1)?);
                let elapsed = self.tokens.remove(&id)
                    .map(|p| format!(" after {} ms", p.sent.elapsed().as_millis()))
                    .unwrap_or_default();
                Some(format!("token #{id} {} received{elapsed}", token_label(ty)))
            }
            _ => None,
        }
    }

    /// What a completed resource stream answers, if known: the command that asked for it,
    /// or the name of a predefined stream.
    pub fn on_stream_complete(&mut self, stream_id: u16) -> Option<String> {
        if let Some(label) = self.streams.remove(&(stream_id as i64)) {
            return Some(format!("{label} (#{stream_id})"));
        }
        if let Some(name) = ac::stream_id_name(stream_id)
            .or_else(|| i16::try_from(stream_id).ok().and_then(ac::request_id_name))
        {
            return Some(name.to_string());
        }
        (ac::STREAM_ID_CHAT_MIN..=ac::STREAM_ID_CHAT_MAX).contains(&stream_id)
            .then(|| format!("chat stream {stream_id}"))
    }

    /// Forget everything outstanding, when the entities it belongs to are reset.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.tokens.clear();
        self.streams.clear();
    }

}

fn int_arg(args: &[Value], index: usize) -> Option<i64> {
    args.get(index).and_then(Value::as_i64)
}

fn token_label(ty: i64) -> String {
    u8::try_from(ty).ok().and_then(token_type::name)
        .map(|name| format!("TOKEN_TYPE.{name}"))
        .unwrap_or_else(|| format!("token type {ty}"))
}

fn is_empty_string(debug: &str) -> bool {
    matches!(debug, "\"\"" | "String(\"\")" | "Bytes(b\"\")" | "Raw([])")
}


#[cfg(test)]
mod tests {

    use super::*;

    /// A streamed command, end to end: request, `RES_STREAM` answer, then its stream.
    #[test]
    fn streamed_command_is_followed_to_its_stream() {

        let mut requests = Requests::default();

        let sent = requests.on_base_call("doCmdInt3",
            &[Value::Int16(221), Value::Int16(100), Value::Int64(0), Value::Int64(0), Value::Int64(0)]);
        assert_eq!(sent.as_deref(), Some("command #221 CMD_SYNC_DATA(0i64, 0i64, 0i64)"));

        let answered = requests.on_client_call("onCmdResponse",
            &[Value::Int16(221), Value::Int16(1)]).unwrap();
        assert!(answered.starts_with("command #221 CMD_SYNC_DATA: RES_STREAM after "), "{answered}");

        assert_eq!(requests.on_stream_complete(221).as_deref(), Some("CMD_SYNC_DATA (#221)"));
        // A stream is only claimed once.
        assert_eq!(requests.on_stream_complete(221), None);

    }

    /// Predefined request ids are not tracked as pending answers.
    #[test]
    fn predefined_request_ids() {

        let mut requests = Requests::default();

        let fire_and_forget = requests.on_base_call("doCmdInt3",
            &[Value::Int16(ac::REQUEST_ID_NO_RESPONSE), Value::Int16(ac::CMD_REQ_SERVER_STATS)]);
        assert_eq!(fire_and_forget.as_deref(), Some("command CMD_REQ_SERVER_STATS(), no answer expected"));
        assert!(requests.commands.is_empty());

        requests.on_base_call("doCmdIntArr",
            &[Value::Int16(ac::REQUEST_ID_PREBATTLES), Value::Int16(ac::CMD_REQ_PREBATTLES)]);
        assert_eq!(requests.on_stream_complete(ac::REQUEST_ID_PREBATTLES as u16).as_deref(),
            Some("CMD_REQ_PREBATTLES (#200)"));

        // Streams the server sends on its own are named without any request.
        assert_eq!(requests.on_stream_complete(ac::STREAM_ID_AVATAR_BATTLE_RESULTS).as_deref(),
            Some("STREAM_ID_AVATAR_BATTLE_RESULTS"));
        assert_eq!(requests.on_stream_complete(12).as_deref(), Some("chat stream 12"));

    }

    #[test]
    fn tokens() {
        let mut requests = Requests::default();
        assert_eq!(requests.on_base_call("requestToken", &[Value::UInt16(5), Value::UInt8(5)]).as_deref(),
            Some("token #5 TOKEN_TYPE.WGNI_JWT requested"));
        let received = requests.on_client_call("onTokenReceived", &[Value::UInt16(5), Value::UInt8(5)]).unwrap();
        assert!(received.starts_with("token #5 TOKEN_TYPE.WGNI_JWT received after "), "{received}");
    }

}
