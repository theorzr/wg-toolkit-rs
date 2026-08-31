//! Replay a single raw packet (as captured by the proxy's `raw_packets` error field)
//! through the real bundle/element decoding logic, with verbose per-element tracing,
//! to find exactly where a live decode desync started -- without needing a fresh game
//! session.

use std::env;
use std::fs;
use std::path::Path;

use wgtk::res::fs::ResFilesystem;
use wgtk::script;
use wgtk::app::dispatch::ScriptDispatch;
use wgtk::app::client::element as client_el;
use wgtk::net::bundle::{Bundle, NextElementReader};
use wgtk::net::packet::Packet;

fn main() {
    let dir = env::args().nth(1).expect("usage: <game-dir> <entity-name> <hex-packet>");
    let dir = Path::new(&dir);
    let entity_name = env::args().nth(2).expect("need entity name (e.g. Avatar)");
    let hex_str = env::args().nth(3).expect("need hex packet bytes");

    let version_file = fs::read_to_string(dir.join("version.xml")).unwrap();
    let version = version_file
        .split_once("<version>").unwrap().1
        .split_once("</version>").unwrap().0
        .trim().to_string();

    let fs = ResFilesystem::new(dir.join("res")).unwrap();
    let script = script::load(&fs, version).unwrap();
    let dispatch = ScriptDispatch::new(script);
    let (_type_id, ed) = dispatch.entity_from_name(&entity_name).expect("entity not found");

    let raw: Vec<u8> = (0..hex_str.len()).step_by(2)
        .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
        .collect();

    println!("raw packet: {} bytes", raw.len());

    let mut packet = Packet::new();
    packet.buf_mut()[..raw.len()].copy_from_slice(&raw);
    packet.set_len(raw.len());

    let locked = match packet.read_config_locked() {
        Ok(locked) => locked,
        Err((e, _)) => {
            println!("FAILED to read packet config: {e:?}");
            return;
        }
    };

    println!("packet config: {:?}", locked.config());

    let bundle = Bundle::new_with_single(locked);
    let mut reader = bundle.element_reader();

    loop {
        match reader.next() {
            None => {
                println!("-- no more elements --");
                break;
            }
            Some(NextElementReader::Reply(reply)) => {
                let request_id = reply.request_id();
                match reply.read_simple::<()>() {
                    Ok(_) => println!("Reply request_id={request_id}"),
                    Err(e) => { println!("Reply request_id={request_id} FAILED: {e}"); break; }
                }
            }
            Some(NextElementReader::Element(elt)) => {
                let id = elt.id();
                use client_el::id::*;

                macro_rules! h {
                    ($ty:ty) => {{
                        match elt.read_simple::<$ty>() {
                            Ok(e) => { println!("[id=0x{id:02X}] {}: {:?}", stringify!($ty), e.element); true }
                            Err(e) => { println!("[id=0x{id:02X}] {} FAILED: {e}", stringify!($ty)); false }
                        }
                    }};
                }

                let ok = if id == AVATAR_UPDATE_NO_ALIAS_DETAILED { h!(client_el::AvatarUpdateNoAliasDetailed) }
                else if id == AVATAR_UPDATE_ALIAS_DETAILED { h!(client_el::AvatarUpdateAliasDetailed) }
                else if id == AVATAR_UPDATE_PLAYER_DETAILED { h!(client_el::AvatarUpdatePlayerDetailed) }
                else if id == RELATIVE_POSITION { h!(client_el::RelativePosition) }
                else if id == RELATIVE_POSITION_REFERENCE { h!(client_el::RelativePositionReference) }
                else if id == TICK_SYNC { h!(client_el::TickSync) }
                else if id == UPDATE_FREQUENCY_NOTIFICATION { h!(client_el::UpdateFrequencyNotification) }
                else if id == RESET_ENTITIES { h!(client_el::ResetEntities) }
                else if id == SELECT_PLAYER_ENTITY { h!(client_el::SelectPlayerEntity) }
                else if id == SELECT_ENTITY { h!(client_el::SelectEntity) }
                else if id == AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateNoAliasFullPosYawPitchRoll) }
                else if id == AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW_PITCH { h!(client_el::AvatarUpdateNoAliasFullPosYawPitch) }
                else if id == AVATAR_UPDATE_NO_ALIAS_FULL_POS_YAW { h!(client_el::AvatarUpdateNoAliasFullPosYaw) }
                else if id == AVATAR_UPDATE_NO_ALIAS_FULL_POS_NO_DIR { h!(client_el::AvatarUpdateNoAliasFullPosNoDir) }
                else if id == AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateNoAliasOnGroundYawPitchRoll) }
                else if id == AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW_PITCH { h!(client_el::AvatarUpdateNoAliasOnGroundYawPitch) }
                else if id == AVATAR_UPDATE_NO_ALIAS_ON_GROUND_YAW { h!(client_el::AvatarUpdateNoAliasOnGroundYaw) }
                else if id == AVATAR_UPDATE_NO_ALIAS_ON_GROUND_NO_DIR { h!(client_el::AvatarUpdateNoAliasOnGroundNoDir) }
                else if id == AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateNoAliasNoPosYawPitchRoll) }
                else if id == AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW_PITCH { h!(client_el::AvatarUpdateNoAliasNoPosYawPitch) }
                else if id == AVATAR_UPDATE_NO_ALIAS_NO_POS_YAW { h!(client_el::AvatarUpdateNoAliasNoPosYaw) }
                else if id == AVATAR_UPDATE_NO_ALIAS_NO_POS_NO_DIR { h!(client_el::AvatarUpdateNoAliasNoPosNoDir) }
                else if id == AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateAliasFullPosYawPitchRoll) }
                else if id == AVATAR_UPDATE_ALIAS_FULL_POS_YAW_PITCH { h!(client_el::AvatarUpdateAliasFullPosYawPitch) }
                else if id == AVATAR_UPDATE_ALIAS_FULL_POS_YAW { h!(client_el::AvatarUpdateAliasFullPosYaw) }
                else if id == AVATAR_UPDATE_ALIAS_FULL_POS_NO_DIR { h!(client_el::AvatarUpdateAliasFullPosNoDir) }
                else if id == AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateAliasOnGroundYawPitchRoll) }
                else if id == AVATAR_UPDATE_ALIAS_ON_GROUND_YAW_PITCH { h!(client_el::AvatarUpdateAliasOnGroundYawPitch) }
                else if id == AVATAR_UPDATE_ALIAS_ON_GROUND_YAW { h!(client_el::AvatarUpdateAliasOnGroundYaw) }
                else if id == AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR {
                    // HYPOTHESIS UNDER TEST: same +3 trailing bytes as
                    // AvatarUpdateAliasFullPosYawPitchRoll (id 0x35) -- declared Fixed(4),
                    // testing Fixed(7).
                    match elt.read_simple::<wgtk::net::element::DebugElementFixed<{ AVATAR_UPDATE_ALIAS_ON_GROUND_NO_DIR }, 7>>() {
                        Ok(e) => { println!("[id=0x{id:02X}] AvatarUpdateAliasOnGroundNoDir (as Fixed(7)): {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] AvatarUpdateAliasOnGroundNoDir (as Fixed(7)) FAILED: {e}"); false }
                    }
                }
                else if id == AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH_ROLL { h!(client_el::AvatarUpdateAliasNoPosYawPitchRoll) }
                else if id == AVATAR_UPDATE_ALIAS_NO_POS_YAW_PITCH { h!(client_el::AvatarUpdateAliasNoPosYawPitch) }
                else if id == AVATAR_UPDATE_ALIAS_NO_POS_YAW { h!(client_el::AvatarUpdateAliasNoPosYaw) }
                else if id == AVATAR_UPDATE_ALIAS_NO_POS_NO_DIR { h!(client_el::AvatarUpdateAliasNoPosNoDir) }
                else if id == CHANGE_VOLATILE_PACKER_TYPE {
                    match elt.read::<client_el::ChangeVolatilePackerType, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] ChangeVolatilePackerType: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] ChangeVolatilePackerType FAILED: {e}"); false }
                    }
                }
                else if id == NRL_UPDATE_NODE_FLAG { h!(client_el::NrlUpdateNodeFlag) }
                else if id == NRL_MSG_TO_CLIENT {
                    match elt.read::<client_el::NrlMsgToClient, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] NrlMsgToClient: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] NrlMsgToClient FAILED: {e}"); false }
                    }
                }
                else if id == NRL_DATA {
                    match elt.read::<client_el::NrlData, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] NrlData: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] NrlData FAILED: {e}"); false }
                    }
                }
                else if id == NRL_UNRELIABLE_MSG_TO_CLIENT {
                    match elt.read::<client_el::NrlUnreliableMsgToClient, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] NrlUnreliableMsgToClient: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] NrlUnreliableMsgToClient FAILED: {e}"); false }
                    }
                }
                else if id == NESTED_ENTITY_PROPERTY {
                    match elt.read::<client_el::NestedEntityProperty, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] NestedEntityProperty: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] NestedEntityProperty FAILED: {e}"); false }
                    }
                } else if id == SLICE_ENTITY_PROPERTY {
                    match elt.read::<client_el::SliceEntityProperty, _>(&()) {
                        Ok(e) => { println!("[id=0x{id:02X}] SliceEntityProperty: {:?}", e.element); true }
                        Err(e) => { println!("[id=0x{id:02X}] SliceEntityProperty FAILED: {e}"); false }
                    }
                } else if ENTITY_PROPERTY.contains(id) {
                    match elt.read::<client_el::EntityProperty, _>(&ed.properties) {
                        Ok(e) => { println!("[id=0x{id:02X}] EntityProperty: {}={:?}", e.element.name, e.element.value); true }
                        Err(e) => { println!("[id=0x{id:02X}] EntityProperty FAILED: {e}"); false }
                    }
                } else if ENTITY_METHOD.contains(id) {
                    match elt.read::<client_el::EntityMethod, _>(&ed.client_methods) {
                        Ok(e) => { println!("[id=0x{id:02X}] EntityMethod: {:?}", e.element.call); true }
                        Err(e) => { println!("[id=0x{id:02X}] EntityMethod FAILED: {e}"); false }
                    }
                } else {
                    println!("[id=0x{id:02X}] UNHANDLED in this replay tool -- stopping");
                    false
                };

                if !ok {
                    break;
                }
            }
        }
    }
}
