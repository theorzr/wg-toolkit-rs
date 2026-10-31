//! What the real server answered in a captured session, extracted to be served again:
//! the account's creation data and, for each request the client made, everything the
//! server sent back for it.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::io;

use wgtk::app::client::element::{
    self as client_el, CreateBasePlayer, EntityMethod, ResourceFragment, ResourceHeader,
    SelectPlayerEntity, SwitchBaseApp, TickSync, UpdateFrequencyNotification,
};
use wgtk::app::base::element::{self as base_el, BaseEntityMethod, EnableEntities, SessionKey};
use wgtk::app::proxy::{self, capture::CaptureReader, PacketChannel, PacketDirection, Peer};
use wgtk::net::bundle::{Bundle, NextElementReader};
use wgtk::net::element::{DebugElementVariable32, SimpleElement};
use wgtk::app::dispatch::{MethodCall, ScriptDispatch};
use wgtk::script::Value;


/// Everything served from a capture, see the module documentation.
#[derive(Debug, Default)]
pub struct CapturedSession {
    /// The `UpdateFrequencyNotification` sent after login: frequency and unknown field.
    pub update_frequency: Option<(u8, u16)>,
    /// The `Account` entity's creation data and dynamic components count.
    pub account: Option<(Value, u8)>,
    /// The arguments of the `showGUI` call that follows the account's creation.
    pub show_gui: Option<Vec<Value>>,
    /// Each base method call the client made, in order, with the server's answers.
    pub requests: Vec<CapturedRequest>,
}

/// A base method call the client made, and the answers to it.
#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub method: Arc<str>,
    pub args: Vec<Value>,
    pub answers: Vec<Answer>,
}

/// Something the server sent back for a request: a client method call with the
/// request's id as first argument, or a resource with it as id. A call carrying no such
/// id (`receiveServerStats`, say, which answers the server stats command) counts as an
/// answer to the latest request if it comes before the client's next one.
#[derive(Debug, Clone)]
pub enum Answer {
    Call { method: Arc<str>, args: Vec<Value> },
    Resource { description: Vec<u8>, data: Vec<u8> },
}

impl CapturedSession {

    /// Extract a session from a capture, decoded against the given script model.
    pub fn load(path: &Path, dispatch: &ScriptDispatch) -> io::Result<Self> {
        let capture = CaptureReader::open(path)?;
        let mut extractor = Extractor {
            dispatch,
            player_type: None,
            session: CapturedSession::default(),
            pending: HashMap::new(),
            latest: None,
            resources: HashMap::new(),
        };
        proxy::replay(capture, &mut extractor)?;
        Ok(extractor.session)
    }

    /// The captured request closest to a new one: of the same method, the one with the
    /// same second argument (the command, for `doCmdInt3`-style calls) and then the most
    /// other arguments equal, the request id aside. Some arguments differ between
    /// sessions, so an exact match can't be required: a sync request carries the
    /// client's revision but also a session-specific value. Requests that got answers
    /// come first, since the client reuses some request ids and a reused id's answers
    /// can land on another request; ties go to the earliest.
    pub fn find_request(&self, method: &str, args: &[Value]) -> Option<&CapturedRequest> {
        let score = |r: &CapturedRequest| {
            let same_command = r.args.get(1) == args.get(1);
            let same_args = r.args.iter().zip(args).skip(1).filter(|(a, b)| a == b).count();
            (!r.answers.is_empty(), same_command, same_args)
        };
        self.requests.iter()
            .filter(|r| &*r.method == method)
            .rev()
            .max_by_key(|r| score(r))
    }

}

/// Replays a capture, collecting a [`CapturedSession`].
struct Extractor<'d> {
    dispatch: &'d ScriptDispatch,
    /// The type of the client's current player entity, which its method calls decode
    /// against in both directions.
    player_type: Option<u16>,
    session: CapturedSession,
    /// Requests by id, as indices into `session.requests`, until answered.
    pending: HashMap<i64, usize>,
    /// The client's latest request, which a call carrying no request id answers.
    latest: Option<usize>,
    /// Resources being received: description, fragments by sequence number, and the
    /// last sequence number once seen.
    resources: HashMap<u16, (Vec<u8>, BTreeMap<u8, Vec<u8>>, Option<u8>)>,
}

impl proxy::Handler for Extractor<'_> {

    type Error = io::Error;

    fn receive_bundle(&mut self,
        _peer: Peer,
        bundle: Bundle,
        direction: PacketDirection,
        _channel: Option<PacketChannel>,
    ) -> Result<(), Self::Error> {
        // A bundle that does not decode only loses what it carries; the rest of the
        // capture still extracts.
        let _ = match direction {
            PacketDirection::Out => self.read_out(&bundle),
            PacketDirection::In => self.read_in(&bundle),
        };
        Ok(())
    }

}

impl Extractor<'_> {

    /// Elements from the client: only its base method calls matter.
    fn read_out(&mut self, bundle: &Bundle) -> io::Result<()> {
        let mut reader = bundle.element_reader();
        while let Some(NextElementReader::Element(elt)) = reader.next() {
            match elt.id() {
                SessionKey::ID => { elt.read_simple::<SessionKey>()?; }
                EnableEntities::ID => { elt.read_simple::<EnableEntities>()?; }
                id if base_el::id::BASE_ENTITY_METHOD.contains(id) => {
                    let Some(dispatch) = self.player_type.and_then(|t| self.dispatch.entity_from_id(t)) else { break };
                    let MethodCall::Known { name, args } = elt.read::<BaseEntityMethod, _>(&dispatch.base_methods)?.element.call else {
                        continue;
                    };
                    if let Some(request_id) = args.first().and_then(Value::as_i64) {
                        self.pending.insert(request_id, self.session.requests.len());
                    }
                    self.latest = Some(self.session.requests.len());
                    self.session.requests.push(CapturedRequest { method: name, args, answers: Vec::new() });
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// Elements from the server.
    fn read_in(&mut self, bundle: &Bundle) -> io::Result<()> {
        let mut reader = bundle.element_reader();
        while let Some(next) = reader.next() {
            let elt = match next {
                NextElementReader::Element(elt) => elt,
                NextElementReader::Reply(reply) => {
                    reply.read_simple::<DebugElementVariable32<{ wgtk::net::element::REPLY_ID }>>()?;
                    continue;
                }
            };
            match elt.id() {
                UpdateFrequencyNotification::ID => {
                    let e = elt.read_simple::<UpdateFrequencyNotification>()?.element;
                    self.session.update_frequency.get_or_insert((e.frequency, e.unknown));
                }
                client_el::id::CREATE_BASE_PLAYER => {
                    let e = elt.read::<CreateBasePlayer, _>(self.dispatch)?.element;
                    self.player_type = Some(e.entity_type_id);
                    let is_account = self.dispatch.entity_from_name("Account").map(|(t, _)| t) == Some(e.entity_type_id);
                    if is_account {
                        self.session.account = Some((e.entity_data.into_owned(), e.entity_components_count));
                    }
                }
                SelectPlayerEntity::ID => { elt.read_simple::<SelectPlayerEntity>()?; }
                TickSync::ID => { elt.read_simple::<TickSync>()?; }
                SwitchBaseApp::ID => { elt.read_simple::<SwitchBaseApp>()?; }
                id if client_el::id::ENTITY_METHOD.contains(id) => {
                    let Some(dispatch) = self.player_type.and_then(|t| self.dispatch.entity_from_id(t)) else { break };
                    let MethodCall::Known { name, args } = elt.read::<EntityMethod, _>(&dispatch.client_methods)?.element.call else {
                        continue;
                    };
                    if &*name == "showGUI" {
                        self.session.show_gui.get_or_insert(args);
                    } else if let Some(&index) = args.first().and_then(Value::as_i64).and_then(|id| self.pending.get(&id)) {
                        self.session.requests[index].answers.push(Answer::Call { method: name, args });
                    } else if let Some(index) = self.latest.take() {
                        self.session.requests[index].answers.push(Answer::Call { method: name, args });
                    }
                }
                ResourceHeader::ID => {
                    let e = elt.read_simple::<ResourceHeader>()?.element;
                    self.resources.insert(e.id, (e.description, BTreeMap::new(), None));
                }
                ResourceFragment::ID => {
                    let e = elt.read_simple::<ResourceFragment>()?.element;
                    let Some((_, fragments, last)) = self.resources.get_mut(&e.id) else { continue };
                    fragments.insert(e.sequence_num, e.data);
                    if e.last {
                        *last = Some(e.sequence_num);
                    }
                    let complete = last.is_some_and(|last| fragments.len() == usize::from(last) + 1);
                    if complete {
                        let (description, fragments, _) = self.resources.remove(&e.id).unwrap();
                        let data = fragments.into_values().flatten().collect();
                        if let Some(&index) = self.pending.get(&i64::from(e.id)) {
                            self.session.requests[index].answers.push(Answer::Resource { description, data });
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

}
