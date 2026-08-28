use wgtk::net::app::entity::Entity;

use super::alias::*;
use super::interface::*;

// ============================================== //
// ======             Account              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Account {
        pub i_AccountVersion: AccountVersion,
        pub name: AutoString,
        pub incarnationID: u64,
        pub initialServerSettings: Python,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Account_onKickedFromServer {
        pub reason: AutoString,
        pub kick_reason_type: u8,
        pub expiry_time: u32,
    }

    #[derive(Debug)]
    pub struct Account_onEnqueued {
        pub queue_type: u8,
    }

    #[derive(Debug)]
    pub struct Account_onEnqueueFailure {
        pub queue_type: u8,
        pub error_code: u8,
        pub error_str: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onDequeued {
        pub queue_type: u8,
    }

    #[derive(Debug)]
    pub struct Account_onKickedFromQueue {
        pub queue_type: u8,
    }

    #[derive(Debug)]
    pub struct Account_onArenaCreated {
    }

    #[derive(Debug)]
    pub struct Account_onIGRTypeChanged {
        pub data: Python,
    }

    #[derive(Debug)]
    pub struct Account_onArenaJoinFailure {
        pub error_code: u8,
        pub error_str: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onPrebattleJoined {
        pub prebattle_id: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct Account_onPrebattleJoinFailure {
        pub error_code: u8,
    }

    #[derive(Debug)]
    pub struct Account_onPrebattleLeft {
    }

    #[derive(Debug)]
    pub struct Account_onKickedFromArena {
        pub reason_code: u8,
    }

    #[derive(Debug)]
    pub struct Account_onKickedFromPrebattle {
        pub reason_code: u8,
    }

    #[derive(Debug)]
    pub struct Account_onCenterIsLongDisconnected {
        pub is_long_disconnected: BOOL,
    }

    #[derive(Debug)]
    pub struct Account_showGUI {
        pub data: Python,
    }

    #[derive(Debug)]
    pub struct Account_receiveActiveArenas {
        pub arenas: Vec<PUBLIC_ARENA_INFO>,
    }

    #[derive(Debug)]
    pub struct Account_receiveServerStats {
        pub stats: SERVER_STATISTICS,
    }

    #[derive(Debug)]
    pub struct Account_receiveQueueInfo {
        pub info: QUEUE_INFO,
    }

    #[derive(Debug)]
    pub struct Account_updatePrebattle {
        pub update_type: u8,
        pub str_arg: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_update {
        pub diff: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_resyncDossiers {
        pub is_full_resync: BOOL,
    }

    #[derive(Debug)]
    pub struct Account_reloadShop {
    }

    #[derive(Debug)]
    pub struct Account_onUnitUpdate {
        pub unit_manager_id: u64,
        pub packed_unit: AutoString,
        pub packed_ops: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onUnitCallOk {
        pub request_id: i32,
    }

    #[derive(Debug)]
    pub struct Account_onUnitNotify {
        pub unit_manager_id: u64,
        pub notify_code: i32,
        pub notify_str: AutoString,
        pub args: Python,
    }

    #[derive(Debug)]
    pub struct Account_onUnitError {
        pub request_id: i32,
        pub unit_manager_id: u64,
        pub error_code: i32,
        pub error_str: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onUnitBrowserError {
        pub error_code: i32,
        pub error_str: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onUnitBrowserResultsSet {
        pub browser_results: Python,
    }

    #[derive(Debug)]
    pub struct Account_onUnitBrowserResultsUpdate {
        pub browser_updates: Python,
    }

    #[derive(Debug)]
    pub struct Account_onGlobalMapUpdate {
        pub packed_ops: AutoString,
        pub packed_update: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onGlobalMapReply {
        pub request_id: u64,
        pub result_code: i32,
        pub result_str: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_onSendPrebattleInvites {
        pub id: DB_ID,
        pub name: AutoString,
        pub clan_id: DB_ID,
        pub clan_abbrev: AutoString,
        pub prebattle_id: u64,
        pub status: u8,
    }

    #[derive(Debug)]
    pub struct Account_onClanInfoReceived {
        pub id: DB_ID,
        pub name: AutoString,
        pub abbrev: AutoString,
        pub motto: AutoString,
        pub description: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_receiveNotification {
        pub notification: AutoString,
    }

    #[derive(Debug)]
    pub struct Account_receiveConversionResults {
        pub a0: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct Account_makeDenunciation {
        pub a0: DB_ID,
        pub a1: i32,
        pub a2: i8,
    }

    #[derive(Debug)]
    pub struct Account_banUnbanUser {
        pub a0: DB_ID,
        pub a1: u8,
        pub a2: u32,
        pub a3: AutoString,
        pub a4: i8,
    }

    #[derive(Debug)]
    pub struct Account_requestToken {
        pub request_id: u16,
        pub token_type: u8,
    }

    #[derive(Debug)]
    pub struct Account_logStreamCorruption {
        pub stream_id: i16,
        pub original_packet_len: i32,
        pub packet_len: i32,
        pub original_crc32: i32,
        pub crc32: i32,
    }

    #[derive(Debug)]
    pub struct Account_setKickAtTime {
        pub a0: i64,
        pub a1: AutoString,
        pub a2: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Account_Client {
        Account_onArenaCreated(0x00, 0),
        Account_onPrebattleLeft(0x01, 0),
        Account_reloadShop(0x02, 0),
        Account_onEnqueued(0x03, 1),
        Account_onDequeued(0x04, 1),
        Account_onKickedFromQueue(0x05, 1),
        Account_onPrebattleJoinFailure(0x06, 1),
        Account_onKickedFromArena(0x07, 1),
        Account_onKickedFromPrebattle(0x08, 1),
        Account_onCenterIsLongDisconnected(0x09, 1),
        Account_resyncDossiers(0x0A, 1),
        Account_onPrebattleJoined(0x0B, 4),
        Account_onUnitCallOk(0x0C, 4),
        Account_receiveServerStats(0x0D, 8),
        Chat_onChatAction(0x0E, var8),
        PlayerMessenger_chat2_messenger_onActionByServer_chat2(0x0F, var8),
        ClientCommandsPort_onCmdResponse(0x10, var8),
        ClientCommandsPort_onCmdResponseExt(0x11, var8),
        AccountAuthTokenProviderClient_onTokenReceived(0x12, var8),
        InvitationsClient_processInvitations(0x13, var8),
        Account_onKickedFromServer(0x14, var8),
        Account_onEnqueueFailure(0x15, var8),
        Account_onIGRTypeChanged(0x16, var8),
        Account_onArenaJoinFailure(0x17, var8),
        Account_receiveActiveArenas(0x18, var8),
        Account_receiveQueueInfo(0x19, var8),
        Account_updatePrebattle(0x1A, var8),
        Account_update(0x1B, var8),
        Account_onUnitUpdate(0x1C, var8),
        Account_onUnitNotify(0x1D, var8),
        Account_onUnitError(0x1E, var8),
        Account_onUnitBrowserError(0x1F, var8),
        Account_onUnitBrowserResultsSet(0x20, var8),
        Account_onUnitBrowserResultsUpdate(0x21, var8),
        Account_onGlobalMapUpdate(0x22, var8),
        Account_onGlobalMapReply(0x23, var8),
        Account_onSendPrebattleInvites(0x24, var8),
        Account_onClanInfoReceived(0x25, var8),
        Account_receiveNotification(0x26, var8),
        Account_receiveConversionResults(0x27, var8),
        Account_showGUI(0x28, var16),
        AccountBattleRoyaleTournamentComponent_setTournamentToken(0x29, var8),
        AccountBattleRoyaleTournamentComponent_setParticipants(0x2A, var16),
        LaPingerComponent_pingMeAndThenJustTouchMe(0x2B, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Account_Base {
        AccountUnitBrowser_accountUnitBrowser_unsubscribe(0x00, 0),
        Chat_onStreamComplete(0x01, 3),
        AccountAuthTokenProvider_requestToken(0x02, 3),
        AccountUnitBrowser_accountUnitBrowser_subscribe(0x03, 3),
        Account_requestToken(0x04, 3),
        ClientCommandsPort_doCmdNoArgs(0x05, 4),
        AccountUnitBrowser_accountUnitBrowser_doCmd(0x06, 4),
        AccountUnitBrowser_accountUnitBrowser_recenter(0x07, 7),
        ClientCommandsPort_doCmdInt(0x08, 12),
        Account_makeDenunciation(0x09, 13),
        AccountUnitClient_accountUnitClient_join(0x0A, 16),
        Account_logStreamCorruption(0x0B, 18),
        ClientCommandsPort_doCmdInt2(0x0C, 20),
        AccountDebugger_accountDebugger_registerDebugTaskResult(0x0D, 20),
        ClientCommandsPort_doCmdInt3(0x0E, 28),
        Chat_ackCommand(0x0F, 33),
        ClientCommandsPort_doCmdInt4(0x10, 36),
        Chat_chatCommandFromClient(0x11, var8),
        Chat_inviteCommand(0x12, var8),
        PlayerMessenger_chat2_messenger_onActionByClient_chat2(0x13, var8),
        ClientCommandsPort_doCmdStr(0x14, var8),
        ClientCommandsPort_doCmdInt2Str(0x15, var8),
        ClientCommandsPort_doCmdInt3Str(0x16, var8),
        ClientCommandsPort_doCmdIntArr(0x17, var8),
        ClientCommandsPort_doCmdIntStr(0x18, var8),
        ClientCommandsPort_doCmdIntStrArr(0x19, var8),
        ClientCommandsPort_doCmdIntArrStrArr(0x1A, var8),
        ClientCommandsPort_doCmdStrArr(0x1B, var8),
        AccountAvatar_accountAvatar_sendAccountStats(0x1C, var8),
        AccountPrebattle_accountPrebattle_createTraining(0x1D, var8),
        AccountPrebattle_accountPrebattle_createDevPrebattle(0x1E, var8),
        AccountPrebattle_accountPrebattle_sendPrebattleInvites(0x1F, var8),
        AccountGlobalMapConnector_accountGlobalMapConnector_callGlobalMapMethod(0x20, var8),
        AccountUnitClient_accountUnitClient_create(0x21, var8),
        AccountUnitClient_accountUnitClient_doCmd(0x22, var8),
        AccountUnitClient_accountUnitClient_sendInvites(0x23, var8),
        AccountUnitClient_accountUnitClient_setRosterSlots(0x24, var8),
        AccountDebugger_accountDebugger_sendDebugTaskResultChunk(0x25, var8),
        Account_banUnbanUser(0x26, var8),
        Account_setKickAtTime(0x27, var8),
        StoryModeAccountComponent_setDevelopmentFeature(0x28, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Account_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Account_Property {
        incarnationID(0x00, 8, u64),
        requiredVersion_2310(0x01, var16, AutoString),
        name(0x02, var16, AutoString),
        initialServerSettings(0x03, var16, Python),
    }
}

impl Entity for Account {
    const TYPE_ID: u16 = 0x01;
    type ClientMethod = Account_Client;
    type BaseMethod = Account_Base;
    type CellMethod = Account_Cell;
    type Property = Account_Property;
}

// ============================================== //
// ======              Avatar              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Avatar {
        pub i_AvatarObserver: AvatarObserver,
        pub name: AutoString,
        pub sessionID: AutoString,
        pub arenaUniqueID: ARENA_UNIQUE_ID,
        pub arenaTypeID: i32,
        pub arenaBonusType: u8,
        pub arenaGuiType: u8,
        pub arenaExtraData: Python,
        pub weatherPresetID: u8,
        pub denunciationsLeft: i16,
        pub clientCtx: AutoString,
        pub tkillIsSuspected: BOOL,
        pub team: u8,
        pub playerVehicleID: OBJECT_ID,
        pub isObserverBothTeams: BOOL,
        pub observableTeamID: u8,
        pub isGunLocked: BOOL,
        pub ownVehicleGear: u8,
        pub ownVehicleAuxPhysicsData: u64,
        pub ownVehicleHullAimingPitchPacked: u16,
        pub ammoViews: AVATAR_AMMO_VIEWS,
        pub customizationDisplayType: u8,
        pub playLimits: PLAY_LIMITS,
        pub battleChatRestriction: BATTLE_CHAT_RESTRICTION,
        pub goodiesSnapshot: Vec<BATTLE_GOODIE_RECORD>,
        pub shouldSendKillcamSimulationData: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Avatar_update {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_onKickedFromServer {
        pub a0: AutoString,
        pub a1: u8,
        pub a2: u32,
    }

    #[derive(Debug)]
    pub struct Avatar_onIGRTypeChanged {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_onAutoAimVehicleLost {
        pub a0: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_receiveAccountStats {
        pub a0: u32,
        pub a1: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_updateVehicleHealth {
        pub a0: OBJECT_ID,
        pub a1: i16,
        pub a2: i8,
        pub a3: BOOL,
        pub a4: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_updateVehicleAmmo {
        pub a0: OBJECT_ID,
        pub a1: i32,
        pub a2: u16,
        pub a3: u8,
        pub a4: u8,
        pub a5: i16,
        pub a6: i16,
        pub a7: i16,
    }

    #[derive(Debug)]
    pub struct Avatar_onSwitchViewpoint {
        pub a0: OBJECT_ID,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_updateVehicleSetting {
        pub a0: OBJECT_ID,
        pub a1: u8,
        pub a2: i32,
    }

    #[derive(Debug)]
    pub struct Avatar_updateTargetingInfo {
        pub a0: f32,
        pub a1: f32,
        pub a2: f32,
        pub a3: f32,
        pub a4: f32,
        pub a5: f32,
        pub a6: f32,
        pub a7: f32,
        pub a8: f32,
    }

    #[derive(Debug)]
    pub struct Avatar_updateTargetVehicleID {
        pub a0: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct Avatar_showOwnVehicleHitDirection {
        pub a0: f32,
        pub a1: OBJECT_ID,
        pub a2: u16,
        pub a3: u32,
        pub a4: BOOL,
        pub a5: BOOL,
        pub a6: OBJECT_ID,
        pub a7: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_showOtherVehicleDamagedDevices {
        pub a0: OBJECT_ID,
        pub a1: Vec<EXTRA_ID>,
        pub a2: Vec<EXTRA_ID>,
    }

    #[derive(Debug)]
    pub struct Avatar_showShotResults {
        pub a0: Vec<CLIENT_SHOT_RESULT_DATA>,
    }

    #[derive(Debug)]
    pub struct Avatar_showDevelopmentInfo {
        pub a0: u8,
        pub a1: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_showHittingArea {
        pub a0: u16,
        pub a1: Vec3,
        pub a2: Vec3,
        pub a3: f64,
    }

    #[derive(Debug)]
    pub struct Avatar_showCarpetBombing {
        pub a0: u16,
        pub a1: Vec3,
        pub a2: Vec3,
        pub a3: f64,
    }

    #[derive(Debug)]
    pub struct Avatar_showTracer {
        pub a0: OBJECT_ID,
        pub a1: SHOT_ID,
        pub a2: BOOL,
        pub a3: u8,
        pub a4: u8,
        pub a5: u8,
        pub a6: f32,
        pub a7: Vec3,
        pub a8: Vec3,
        pub a9: f32,
        pub a10: f32,
        pub a11: u8,
        pub a12: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_stopTracer {
        pub a0: SHOT_ID,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_explodeProjectile {
        pub a0: SHOT_ID,
        pub a1: u8,
        pub a2: u8,
        pub a3: u8,
        pub a4: u8,
        pub a5: f32,
        pub a6: Vec3,
        pub a7: Vec3,
        pub a8: f32,
        pub a9: Vec<u32>,
    }

    #[derive(Debug)]
    pub struct Avatar_onRoundFinished {
        pub a0: i8,
        pub a1: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_onKickedFromArena {
        pub a0: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_onBattleEvents {
        pub a0: Vec<BATTLE_EVENT>,
    }

    #[derive(Debug)]
    pub struct Avatar_battleEventsSummary {
        pub a0: BATTLE_EVENTS_SUMMARY,
    }

    #[derive(Debug)]
    pub struct Avatar_updateArena {
        pub a0: u8,
        pub a1: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_receivePhysicsDebugInfo {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_updateCarriedFlagPositions {
        pub a0: Vec<u8>,
        pub a1: Vec<i16>,
    }

    #[derive(Debug)]
    pub struct Avatar_receiveNotification {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_onRepairPointAction {
        pub a0: u8,
        pub a1: u8,
        pub a2: f32,
    }

    #[derive(Debug)]
    pub struct Avatar_updateAvatarPrivateStats {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_updateResourceAmount {
        pub a0: u8,
        pub a1: u32,
    }

    #[derive(Debug)]
    pub struct Avatar_onFrictionWithVehicle {
        pub a0: OBJECT_ID,
        pub a1: Vec3,
        pub a2: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_onCollisionWithVehicle {
        pub a0: Vec3,
        pub a1: f32,
    }

    #[derive(Debug)]
    pub struct Avatar_onSmoke {
        pub a0: SMOKE_INFO,
    }

    #[derive(Debug)]
    pub struct Avatar_onCombatEquipmentShotLaunched {
        pub a0: u16,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_updateQuestProgress {
        pub a0: AutoString,
        pub a1: Python,
    }

    #[derive(Debug)]
    pub struct Avatar_updateVehicleQuickShellChanger {
        pub a0: OBJECT_ID,
        pub a1: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_enemySPGHit {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_enemySPGShotSound {
        pub a0: Vec3,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_handleScriptEventFromServer {
        pub a0: AutoString,
        pub a1: AutoString,
        pub a2: AutoString,
        pub a3: AutoString,
        pub a4: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_setUpdatedGoodiesSnapshot {
        pub a0: Vec<BATTLE_GOODIE_RECORD>,
    }

    #[derive(Debug)]
    pub struct Avatar_onRandomEvent {
        pub a0: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct Avatar_logLag {
    }

    #[derive(Debug)]
    pub struct Avatar_setClientReady {
    }

    #[derive(Debug)]
    pub struct Avatar_leaveArena {
    }

    #[derive(Debug)]
    pub struct Avatar_onLoginToCellFailed {
    }

    #[derive(Debug)]
    pub struct Avatar_confirmBattleResultsReceiving {
    }

    #[derive(Debug)]
    pub struct Avatar_makeDenunciation {
        pub a0: OBJECT_ID,
        pub a1: i32,
        pub a2: i8,
    }

    #[derive(Debug)]
    pub struct Avatar_banUnbanUser {
        pub a0: DB_ID,
        pub a1: u8,
        pub a2: u32,
        pub a3: AutoString,
        pub a4: i8,
    }

    #[derive(Debug)]
    pub struct Avatar_requestToken {
        pub a0: u16,
        pub a1: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_sendAccountStats {
        pub a0: u32,
        pub a1: Vec<AutoString>,
    }

    #[derive(Debug)]
    pub struct Avatar_setClientCtx {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_teleport {
        pub a0: Vec3,
        pub a1: f32,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_replenishAmmo {
    }

    #[derive(Debug)]
    pub struct Avatar_setDevelopmentFeature {
        pub a0: OBJECT_ID,
        pub a1: AutoString,
        pub a2: i32,
        pub a3: AutoString,
    }

    #[derive(Debug)]
    pub struct Avatar_addBotToArena {
        pub a0: AutoString,
        pub a1: u8,
        pub a2: AutoString,
        pub a3: Vec3,
        pub a4: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_receiveFakeShot {
        pub a0: i32,
        pub a1: f32,
        pub a2: Vec3,
        pub a3: Vec3,
        pub a4: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_logStreamCorruption {
        pub a0: i16,
        pub a1: i32,
        pub a2: i32,
        pub a3: i32,
        pub a4: i32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct Avatar_autoAim {
        pub a0: OBJECT_ID,
        pub a1: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_moveTo {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_bindToVehicle {
        pub a0: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct Avatar_monitorVehicleDamagedDevices {
        pub a0: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct Avatar_activateEquipment {
        pub a0: u16,
        pub a1: i16,
    }

    #[derive(Debug)]
    pub struct Avatar_setEquipmentApplicationPoint {
        pub a0: u16,
        pub a1: Vec3,
        pub a2: Vec2,
    }

    #[derive(Debug)]
    pub struct Avatar_switchViewPointOrBindToVehicle {
        pub a0: BOOL,
        pub a1: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct Avatar_setDualGunCharger {
        pub a0: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_reportClientStats {
        pub a0: CLIENT_STATUS_STATISTICS,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_moveWith {
        pub a0: u8,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_shoot {
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_trackWorldPointWithGun {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_trackRelativePointWithGun {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_stopTrackingWithGun {
        pub a0: f32,
        pub a1: f32,
    }

    #[derive(Debug)]
    pub struct Avatar_setupAmmo {
        pub a0: i64,
    }

    #[derive(Debug)]
    pub struct Avatar_vehicle_changeSetting {
        pub a0: u8,
        pub a1: i32,
    }

    #[derive(Debug)]
    pub struct Avatar_setServerMarker {
        pub a0: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_setSendKillCamSimulationData {
        pub a0: BOOL,
    }

    #[derive(Debug)]
    pub struct Avatar_submitPlayerSatisfactionRating {
        pub a0: i8,
    }

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Avatar_Client {
        RecoveryMechanic_Avatar_notifyCannotStartRecovering(0x00, 0),
        RecoveryMechanic_Avatar_notifyCancelled(0x01, 0),
        AvatarEpic_enteringProtectionZone(0x02, 1),
        AvatarEpic_leavingProtectionZone(0x03, 1),
        AvatarEpic_protectionZoneShooting(0x04, 1),
        AvatarEpic_onSectorShooting(0x05, 1),
        AvatarEpic_onRankUpdate(0x06, 1),
        Avatar_onAutoAimVehicleLost(0x07, 1),
        Avatar_onKickedFromArena(0x08, 1),
        AvatarEpic_onXPUpdated(0x09, 2),
        Avatar_onRoundFinished(0x0A, 2),
        VehicleRemovalController_Avatar_removeVehicle(0x0B, 4),
        Avatar_updateTargetVehicleID(0x0C, 4),
        AvatarEpic_onDestructibleDestroyed(0x0D, 5),
        Avatar_updateResourceAmount(0x0E, 5),
        Avatar_updateVehicleQuickShellChanger(0x0F, 5),
        AvatarEpic_onSectorBaseAction(0x10, 6),
        Avatar_onRepairPointAction(0x11, 6),
        Avatar_updateVehicleHealth(0x12, 9),
        Avatar_updateVehicleSetting(0x13, 9),
        AvatarEpic_onStepRepairPointAction(0x14, 11),
        VehicleHealthBroadcastListenerComponent_Avatar_onVehicleHealthChanged(0x15, 12),
        AvatarEpic_welcomeToSector(0x16, 12),
        Avatar_enemySPGHit(0x17, 12),
        RecoveryMechanic_Avatar_updateState(0x18, 13),
        AvatarEpic_onCrewRoleFactorAndRankUpdate(0x19, 13),
        Avatar_onCombatEquipmentShotLaunched(0x1A, 14),
        Avatar_onSwitchViewpoint(0x1B, 16),
        Avatar_stopTracer(0x1C, 16),
        Avatar_onCollisionWithVehicle(0x1D, 16),
        Avatar_onSmoke(0x1E, 16),
        Avatar_onFrictionWithVehicle(0x1F, 17),
        Avatar_updateVehicleAmmo(0x20, 18),
        Avatar_showOwnVehicleHitDirection(0x21, 21),
        Avatar_enemySPGShotSound(0x22, 24),
        Avatar_showHittingArea(0x23, 34),
        Avatar_showCarpetBombing(0x24, 34),
        Avatar_battleEventsSummary(0x25, 34),
        Avatar_updateTargetingInfo(0x26, 36),
        Avatar_showTracer(0x27, 50),
        Chat_onChatAction(0x28, var8),
        PlayerMessenger_chat2_messenger_onActionByServer_chat2(0x29, var8),
        ClientCommandsPort_onCmdResponse(0x2A, var8),
        ClientCommandsPort_onCmdResponseExt(0x2B, var8),
        InvitationsClient_processInvitations(0x2C, var8),
        AccountAuthTokenProviderClient_onTokenReceived(0x2D, var8),
        TeamHealthBar_Avatar_updateTeamsHealthPercentage(0x2E, var8),
        TriggersController_Avatar_externalTrigger(0x2F, var8),
        AvatarEpic_syncPurchasedAbilities(0x30, var8),
        AvatarEpic_onRandomReserveOffer(0x31, var8),
        AvatarEpic_showDestructibleShotResults(0x32, var8),
        Avatar_update(0x33, var8),
        Avatar_onKickedFromServer(0x34, var8),
        Avatar_onIGRTypeChanged(0x35, var8),
        Avatar_receiveAccountStats(0x36, var8),
        Avatar_showOtherVehicleDamagedDevices(0x37, var8),
        Avatar_showShotResults(0x38, var8),
        Avatar_showDevelopmentInfo(0x39, var8),
        Avatar_explodeProjectile(0x3A, var8),
        Avatar_onBattleEvents(0x3B, var8),
        Avatar_updateArena(0x3C, var8),
        Avatar_receivePhysicsDebugInfo(0x3D, var8),
        Avatar_updateCarriedFlagPositions(0x3E, var8),
        Avatar_receiveNotification(0x3F, var8),
        Avatar_updateAvatarPrivateStats(0x40, var8),
        Avatar_updateQuestProgress(0x41, var8),
        Avatar_handleScriptEventFromServer(0x42, var8),
        Avatar_setUpdatedGoodiesSnapshot(0x43, var8),
        Avatar_onRandomEvent(0x44, var8),
        VehiclesSpawnListStorage_Avatar_updateSpawnList(0x45, var16),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Avatar_Base {
        Avatar_logLag(0x00, 0),
        Avatar_setClientReady(0x01, 0),
        Avatar_leaveArena(0x02, 0),
        Avatar_onLoginToCellFailed(0x03, 0),
        Avatar_confirmBattleResultsReceiving(0x04, 0),
        Avatar_vehicle_replenishAmmo(0x05, 0),
        AvatarEpic_enableFrontLineDevInfo(0x06, 1),
        Chat_onStreamComplete(0x07, 3),
        Avatar_requestToken(0x08, 3),
        ClientCommandsPort_doCmdNoArgs(0x09, 4),
        Avatar_makeDenunciation(0x0A, 9),
        ClientCommandsPort_doCmdInt(0x0B, 12),
        Avatar_vehicle_teleport(0x0C, 16),
        Avatar_logStreamCorruption(0x0D, 18),
        ClientCommandsPort_doCmdInt2(0x0E, 20),
        ClientCommandsPort_doCmdInt3(0x0F, 28),
        Chat_ackCommand(0x10, 33),
        Avatar_receiveFakeShot(0x11, 33),
        ClientCommandsPort_doCmdInt4(0x12, 36),
        Chat_chatCommandFromClient(0x13, var8),
        Chat_inviteCommand(0x14, var8),
        PlayerMessenger_chat2_messenger_onActionByClient_chat2(0x15, var8),
        ClientCommandsPort_doCmdStr(0x16, var8),
        ClientCommandsPort_doCmdInt2Str(0x17, var8),
        ClientCommandsPort_doCmdInt3Str(0x18, var8),
        ClientCommandsPort_doCmdIntArr(0x19, var8),
        ClientCommandsPort_doCmdIntStr(0x1A, var8),
        ClientCommandsPort_doCmdIntStrArr(0x1B, var8),
        ClientCommandsPort_doCmdIntArrStrArr(0x1C, var8),
        ClientCommandsPort_doCmdStrArr(0x1D, var8),
        Avatar_banUnbanUser(0x1E, var8),
        Avatar_sendAccountStats(0x1F, var8),
        Avatar_setClientCtx(0x20, var8),
        Avatar_setDevelopmentFeature(0x21, var8),
        Avatar_addBotToArena(0x22, var8),
        StoryModeAvatarComponent_setDevelopmentFeature(0x23, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Avatar_Cell {
        Avatar_vehicle_shoot(0x00, 0),
        AvatarObserver_switchObserverFPV(0x01, 1),
        Avatar_setDualGunCharger(0x02, 1),
        Avatar_vehicle_moveWith(0x03, 1),
        Avatar_setServerMarker(0x04, 1),
        Avatar_setSendKillCamSimulationData(0x05, 1),
        Avatar_submitPlayerSatisfactionRating(0x06, 1),
        Avatar_bindToVehicle(0x07, 4),
        Avatar_monitorVehicleDamagedDevices(0x08, 4),
        Avatar_activateEquipment(0x09, 4),
        Avatar_autoAim(0x0A, 5),
        Avatar_switchViewPointOrBindToVehicle(0x0B, 5),
        Avatar_vehicle_changeSetting(0x0C, 5),
        Avatar_vehicle_stopTrackingWithGun(0x0D, 8),
        Avatar_setupAmmo(0x0E, 8),
        Avatar_moveTo(0x0F, 12),
        Avatar_vehicle_trackWorldPointWithGun(0x10, 12),
        Avatar_vehicle_trackRelativePointWithGun(0x11, 12),
        Avatar_setEquipmentApplicationPoint(0x12, 22),
        Avatar_reportClientStats(0x13, 24),
        StoryModeAvatarComponent_checkPositionForEquipment(0x14, 16),
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Avatar_Property {
        isObserverFPV(0x00, 1, BOOL),
        numOfObservers(0x01, 1, u8),
        arenaBonusType(0x02, 1, u8),
        arenaGuiType(0x03, 1, u8),
        weatherPresetID(0x04, 1, u8),
        tkillIsSuspected(0x05, 1, BOOL),
        team(0x06, 1, u8),
        isObserverBothTeams(0x07, 1, BOOL),
        observableTeamID(0x08, 1, u8),
        isGunLocked(0x09, 1, BOOL),
        ownVehicleGear(0x0A, 1, u8),
        customizationDisplayType(0x0B, 1, u8),
        shouldSendKillcamSimulationData(0x0C, 1, BOOL),
        denunciationsLeft(0x0D, 2, i16),
        ownVehicleHullAimingPitchPacked(0x0E, 2, u16),
        battleChatRestriction(0x0F, 2, BATTLE_CHAT_RESTRICTION),
        arenaTypeID(0x10, 4, i32),
        playerVehicleID(0x11, 4, OBJECT_ID),
        arenaUniqueID(0x12, 8, ARENA_UNIQUE_ID),
        ownVehicleAuxPhysicsData(0x13, 8, u64),
        playLimits(0x14, 16, PLAY_LIMITS),
        remoteCamera(0x15, 22, REMOTE_CAMERA_DATA),
        name(0x16, var16, AutoString),
        sessionID(0x17, var16, AutoString),
        arenaExtraData(0x18, var16, Python),
        clientCtx(0x19, var16, AutoString),
        ammoViews(0x1A, var16, AVATAR_AMMO_VIEWS),
        goodiesSnapshot(0x1B, var16, Vec<BATTLE_GOODIE_RECORD>),
        wrongApplicationPoint(0x1C, 12, Vec3),
        isPositionValid(0x1D, 1, BOOL),
    }
}

impl Entity for Avatar {
    const TYPE_ID: u16 = 0x02;
    type ClientMethod = Avatar_Client;
    type BaseMethod = Avatar_Base;
    type CellMethod = Avatar_Cell;
    type Property = Avatar_Property;
}

// ============================================== //
// ======            ArenaInfo             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaInfo {
        pub i_PlaneTrajectoryArenaInfo: PlaneTrajectoryArenaInfo,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct ArenaInfo_showCarpetBombing {
        pub a0: u16,
        pub a1: Vec3,
        pub a2: Vec3,
        pub a3: f32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ArenaInfo_Client {
        ArenaInfo_showCarpetBombing(0x00, 30),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ArenaInfo_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ArenaInfo_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ArenaInfo_Property {
        planeTrajectory(0x00, 60, PLANE_TRAJECTORY),
    }
}

impl Entity for ArenaInfo {
    const TYPE_ID: u16 = 0x03;
    type ClientMethod = ArenaInfo_Client;
    type BaseMethod = ArenaInfo_Base;
    type CellMethod = ArenaInfo_Cell;
    type Property = ArenaInfo_Property;
}

// ============================================== //
// ======      ClientSelectableObject      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableObject {
        pub modelName: AutoString,
        pub selectionId: AutoString,
        pub mouseOverSoundName: AutoString,
        pub isOver3DSound: BOOL,
        pub clickSoundName: AutoString,
        pub isClick3DSound: BOOL,
        pub edgeMode: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableObject_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableObject_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableObject_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableObject_Property {
        isOver3DSound(0x00, 1, BOOL),
        isClick3DSound(0x01, 1, BOOL),
        edgeMode(0x02, 1, u8),
        modelName(0x03, var16, AutoString),
        selectionId(0x04, var16, AutoString),
        mouseOverSoundName(0x05, var16, AutoString),
        clickSoundName(0x06, var16, AutoString),
    }
}

impl Entity for ClientSelectableObject {
    const TYPE_ID: u16 = 0x04;
    type ClientMethod = ClientSelectableObject_Client;
    type BaseMethod = ClientSelectableObject_Base;
    type CellMethod = ClientSelectableObject_Cell;
    type Property = ClientSelectableObject_Property;
}

// ============================================== //
// ======          HangarVehicle           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct HangarVehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum HangarVehicle_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum HangarVehicle_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum HangarVehicle_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum HangarVehicle_Property {
    }
}

impl Entity for HangarVehicle {
    const TYPE_ID: u16 = 0x05;
    type ClientMethod = HangarVehicle_Client;
    type BaseMethod = HangarVehicle_Base;
    type CellMethod = HangarVehicle_Cell;
    type Property = HangarVehicle_Property;
}

// ============================================== //
// ======             Vehicle              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Vehicle {
        pub i_VehicleObserver: VehicleObserver,
        pub i_Wheels: Wheels,
        pub i_Perks_Vehicle: Perks_Vehicle,
        pub isStrafing: BOOL,
        pub postmortemViewPointName: AutoString,
        pub isHidden: BOOL,
        pub physicsMode: u8,
        pub siegeState: u8,
        pub gunAnglesPacked: u16,
        pub publicInfo: PUBLIC_VEHICLE_INFO,
        pub health: i16,
        pub isCrewActive: BOOL,
        pub engineMode: Box<[u8; 2]>,
        pub damageStickers: Vec<VEHICLE_HIT_POINT>,
        pub publicStateModifiers: Vec<EXTRA_ID>,
        pub stunInfo: STUN_INFO,
        pub crewCompactDescrs: Vec<AutoString>,
        pub enhancements: Python,
        pub setups: Python,
        pub setupsIndexes: Python,
        pub customRoleSlotTypeId: u8,
        pub vehPerks: Python,
        pub vehPostProgression: Vec<i32>,
        pub disabledSwitches: Vec<i32>,
        pub avatarID: OBJECT_ID,
        pub masterVehID: u32,
        pub arenaTypeID: i32,
        pub arenaBonusType: u8,
        pub arenaUniqueID: ARENA_UNIQUE_ID,
        pub inspiringEffect: BUFF_EFFECT,
        pub healingEffect: BUFF_EFFECT,
        pub dotEffect: DOT_EFFECT,
        pub inspired: INSPIRED_EFFECT,
        pub healing: BUFF_EFFECT_INACTIVATION,
        pub healOverTime: HOT_EFFECT,
        pub debuff: i32,
        pub isSpeedCapturing: BOOL,
        pub isBlockingCapture: BOOL,
        pub dogTag: BATTLE_DOG_TAG,
        pub isMyVehicle: BOOL,
        pub quickShellChangerFactor: f32,
        pub onRespawnReloadTimeFactor: f32,
        pub ownVehiclePosition: OWN_VEHICLE_POSITION,
        pub enableExternalRespawn: BOOL,
        pub botDisplayStatus: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Vehicle_onVehiclePickup {
    }

    #[derive(Debug)]
    pub struct Vehicle_onExtraHitted {
        pub a0: i16,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Vehicle_onHealthChanged {
        pub a0: i16,
        pub a1: i16,
        pub a2: OBJECT_ID,
        pub a3: u8,
        pub a4: i8,
    }

    #[derive(Debug)]
    pub struct Vehicle_showShooting {
        pub a0: u8,
        pub a1: i8,
        pub a2: u8,
    }

    #[derive(Debug)]
    pub struct Vehicle_updateLaserSight {
        pub a0: OBJECT_ID,
        pub a1: BOOL,
        pub a2: AutoString,
    }

    #[derive(Debug)]
    pub struct Vehicle_showDamageFromShot {
        pub a0: OBJECT_ID,
        pub a1: Vec<VEHICLE_HIT_POINT>,
        pub a2: u8,
        pub a3: u8,
        pub a4: i32,
        pub a5: u8,
        pub a6: BOOL,
        pub a7: f32,
        pub a8: u8,
    }

    #[derive(Debug)]
    pub struct Vehicle_showDamageFromExplosion {
        pub a0: OBJECT_ID,
        pub a1: Vec3,
        pub a2: u8,
        pub a3: i32,
        pub a4: u8,
        pub a5: u8,
    }

    #[derive(Debug)]
    pub struct Vehicle_showAmmoBayEffect {
        pub a0: u8,
        pub a1: f32,
        pub a2: f32,
    }

    #[derive(Debug)]
    pub struct Vehicle_onPushed {
        pub a0: f32,
        pub a1: f32,
    }

    #[derive(Debug)]
    pub struct Vehicle_onStaticCollision {
        pub a0: f32,
        pub a1: Vec3,
        pub a2: Vec3,
        pub a3: u8,
        pub a4: f32,
        pub a5: i8,
        pub a6: u16,
    }

    #[derive(Debug)]
    pub struct Vehicle_showRammingEffect {
        pub a0: f32,
        pub a1: f32,
        pub a2: Vec3,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct Vehicle_moveWith {
        pub a0: u8,
    }

    #[derive(Debug)]
    pub struct Vehicle_trackWorldPointWithGun {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Vehicle_trackRelativePointWithGun {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct Vehicle_stopTrackingWithGun {
        pub a0: f32,
        pub a1: f32,
    }

    #[derive(Debug)]
    pub struct Vehicle_changeSetting {
        pub a0: u8,
        pub a1: i32,
    }

    #[derive(Debug)]
    pub struct Vehicle_sendVisibilityDevelopmentInfo {
        pub a0: OBJECT_ID,
        pub a1: Vec3,
    }

    #[derive(Debug)]
    pub struct Vehicle_sendStateToOwnClient {
    }

    #[derive(Debug)]
    pub struct Vehicle_switchSetup {
        pub a0: u8,
        pub a1: u8,
    }

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Vehicle_Client {
        Vehicle_onVehiclePickup(0x00, 0),
        Vehicle_showShooting(0x01, 3),
        Vehicle_onPushed(0x02, 8),
        Vehicle_showAmmoBayEffect(0x03, 9),
        Vehicle_onHealthChanged(0x04, 10),
        Vehicle_onExtraHitted(0x05, 14),
        Vehicle_showRammingEffect(0x06, 20),
        Vehicle_showDamageFromExplosion(0x07, 23),
        Vehicle_onStaticCollision(0x08, 36),
        Vehicle_updateLaserSight(0x09, var8),
        Vehicle_showDamageFromShot(0x0A, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Vehicle_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Vehicle_Cell {
        RecoveryMechanic_Vehicle_recoveryMechanic_startRecovering(0x00, 0),
        RecoveryMechanic_Vehicle_recoveryMechanic_stopRecovering(0x01, 0),
        Vehicle_sendStateToOwnClient(0x02, 0),
        Vehicle_moveWith(0x03, 1),
        Vehicle_switchSetup(0x04, 2),
        Vehicle_changeSetting(0x05, 5),
        Vehicle_stopTrackingWithGun(0x06, 8),
        Vehicle_trackWorldPointWithGun(0x07, 12),
        Vehicle_trackRelativePointWithGun(0x08, 12),
        Vehicle_sendVisibilityDevelopmentInfo(0x09, 16),
        VehicleObserver_setRemoteCamera(0x0A, 22),
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Vehicle_Property {
        burnoutLevel(0x00, 1, u8),
        isStrafing(0x01, 1, BOOL),
        isHidden(0x02, 1, BOOL),
        physicsMode(0x03, 1, u8),
        siegeState(0x04, 1, u8),
        isCrewActive(0x05, 1, BOOL),
        customRoleSlotTypeId(0x06, 1, u8),
        arenaBonusType(0x07, 1, u8),
        isSpeedCapturing(0x08, 1, BOOL),
        isBlockingCapture(0x09, 1, BOOL),
        isMyVehicle(0x0A, 1, BOOL),
        enableExternalRespawn(0x0B, 1, BOOL),
        botDisplayStatus(0x0C, 1, u8),
        gunAnglesPacked(0x0D, 2, u16),
        health(0x0E, 2, i16),
        engineMode(0x0F, 2, Box<[u8; 2]>),
        avatarID(0x10, 4, OBJECT_ID),
        masterVehID(0x11, 4, u32),
        arenaTypeID(0x12, 4, i32),
        debuff(0x13, 4, i32),
        quickShellChangerFactor(0x14, 4, f32),
        onRespawnReloadTimeFactor(0x15, 4, f32),
        wheelsState(0x16, 8, u64),
        stunInfo(0x17, 8, STUN_INFO),
        arenaUniqueID(0x18, 8, ARENA_UNIQUE_ID),
        dotEffect(0x19, 14, DOT_EFFECT),
        remoteCamera(0x1A, 22, REMOTE_CAMERA_DATA),
        inspiringEffect(0x1B, 24, BUFF_EFFECT),
        healingEffect(0x1C, 24, BUFF_EFFECT),
        ownVehiclePosition(0x1D, 32, OWN_VEHICLE_POSITION),
        inspired(0x1E, 36, INSPIRED_EFFECT),
        steeringAngles(0x1F, var16, Vec<u8>),
        wheelsScroll(0x20, var16, Vec<u8>),
        perkEffects(0x21, var16, PERK_EFFECTS),
        perks(0x22, var16, Vec<PERK_INFO_HUD>),
        perksRibbonNotify(0x23, var16, Vec<PERK_INFO_RIBBON>),
        postmortemViewPointName(0x24, var16, AutoString),
        publicInfo(0x25, var16, PUBLIC_VEHICLE_INFO),
        damageStickers(0x26, var16, Vec<VEHICLE_HIT_POINT>),
        publicStateModifiers(0x27, var16, Vec<EXTRA_ID>),
        crewCompactDescrs(0x28, var16, Vec<AutoString>),
        enhancements(0x29, var16, Python),
        setups(0x2A, var16, Python),
        setupsIndexes(0x2B, var16, Python),
        vehPerks(0x2C, var16, Python),
        vehPostProgression(0x2D, var16, Vec<i32>),
        disabledSwitches(0x2E, var16, Vec<i32>),
        healing(0x2F, var16, BUFF_EFFECT_INACTIVATION),
        healOverTime(0x30, var16, HOT_EFFECT),
        dogTag(0x31, var16, BATTLE_DOG_TAG),
    }
}

impl Entity for Vehicle {
    const TYPE_ID: u16 = 0x06;
    type ClientMethod = Vehicle_Client;
    type BaseMethod = Vehicle_Base;
    type CellMethod = Vehicle_Cell;
    type Property = Vehicle_Property;
}

// ============================================== //
// ======        AreaDestructibles         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AreaDestructibles {
        pub destroyedModules: Vec<Box<[u8; 3]>>,
        pub destroyedFragiles: Vec<Box<[u8; 3]>>,
        pub fallenColumns: Vec<Box<[u8; 3]>>,
        pub fallenTrees: Vec<Box<[u8; 5]>>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum AreaDestructibles_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum AreaDestructibles_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum AreaDestructibles_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum AreaDestructibles_Property {
        destroyedModules(0x00, var16, Vec<Box<[u8; 3]>>),
        destroyedFragiles(0x01, var16, Vec<Box<[u8; 3]>>),
        fallenColumns(0x02, var16, Vec<Box<[u8; 3]>>),
        fallenTrees(0x03, var16, Vec<Box<[u8; 5]>>),
    }
}

impl Entity for AreaDestructibles {
    const TYPE_ID: u16 = 0x07;
    type ClientMethod = AreaDestructibles_Client;
    type BaseMethod = AreaDestructibles_Base;
    type CellMethod = AreaDestructibles_Cell;
    type Property = AreaDestructibles_Property;
}

// ============================================== //
// ======          OfflineEntity           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct OfflineEntity {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum OfflineEntity_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum OfflineEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum OfflineEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum OfflineEntity_Property {
    }
}

impl Entity for OfflineEntity {
    const TYPE_ID: u16 = 0x08;
    type ClientMethod = OfflineEntity_Client;
    type BaseMethod = OfflineEntity_Base;
    type CellMethod = OfflineEntity_Cell;
    type Property = OfflineEntity_Property;
}

// ============================================== //
// ======              Flock               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Flock {
        pub modelName: AutoString,
        pub modelName2: AutoString,
        pub modelCount: u8,
        pub yawSpeed: f32,
        pub pitchSpeed: f32,
        pub rollSpeed: f32,
        pub animSpeedMin: f32,
        pub animSpeedMax: f32,
        pub height: f32,
        pub radius: f32,
        pub deadZoneRadius: f32,
        pub speedAtBottom: f32,
        pub speedAtTop: f32,
        pub decisionTime: f32,
        pub flyAroundCenter: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Flock_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Flock_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Flock_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Flock_Property {
        modelCount(0x00, 1, u8),
        flyAroundCenter(0x01, 1, BOOL),
        yawSpeed(0x02, 4, f32),
        pitchSpeed(0x03, 4, f32),
        rollSpeed(0x04, 4, f32),
        animSpeedMin(0x05, 4, f32),
        animSpeedMax(0x06, 4, f32),
        height(0x07, 4, f32),
        radius(0x08, 4, f32),
        deadZoneRadius(0x09, 4, f32),
        speedAtBottom(0x0A, 4, f32),
        speedAtTop(0x0B, 4, f32),
        decisionTime(0x0C, 4, f32),
        modelName(0x0D, var16, AutoString),
        modelName2(0x0E, var16, AutoString),
    }
}

impl Entity for Flock {
    const TYPE_ID: u16 = 0x09;
    type ClientMethod = Flock_Client;
    type BaseMethod = Flock_Base;
    type CellMethod = Flock_Cell;
    type Property = Flock_Property;
}

// ============================================== //
// ======           FlockExotic            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FlockExotic {
        pub animSpeedMax: f32,
        pub animSpeedMin: f32,
        pub modelCount: u8,
        pub modelName: AutoString,
        pub modelName2: AutoString,
        pub speed: f32,
        pub initSpeedRandom: Vec2,
        pub speedRandom: Vec2,
        pub accelerationTime: f32,
        pub triggerRadius: f32,
        pub explosionRadius: Vec2,
        pub spawnRadius: f32,
        pub spawnHeight: f32,
        pub flightRadius: f32,
        pub flightHeight: f32,
        pub flightAngleMin: f32,
        pub flightAngleMax: f32,
        pub flightOffsetFromOrigin: f32,
        pub lifeTime: f32,
        pub respawnTime: f32,
        pub flightSound: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum FlockExotic_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum FlockExotic_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum FlockExotic_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum FlockExotic_Property {
        modelCount(0x00, 1, u8),
        animSpeedMax(0x01, 4, f32),
        animSpeedMin(0x02, 4, f32),
        speed(0x03, 4, f32),
        accelerationTime(0x04, 4, f32),
        triggerRadius(0x05, 4, f32),
        spawnRadius(0x06, 4, f32),
        spawnHeight(0x07, 4, f32),
        flightRadius(0x08, 4, f32),
        flightHeight(0x09, 4, f32),
        flightAngleMin(0x0A, 4, f32),
        flightAngleMax(0x0B, 4, f32),
        flightOffsetFromOrigin(0x0C, 4, f32),
        lifeTime(0x0D, 4, f32),
        respawnTime(0x0E, 4, f32),
        initSpeedRandom(0x0F, 8, Vec2),
        speedRandom(0x10, 8, Vec2),
        explosionRadius(0x11, 8, Vec2),
        modelName(0x12, var16, AutoString),
        modelName2(0x13, var16, AutoString),
        flightSound(0x14, var16, AutoString),
    }
}

impl Entity for FlockExotic {
    const TYPE_ID: u16 = 0x0A;
    type ClientMethod = FlockExotic_Client;
    type BaseMethod = FlockExotic_Base;
    type CellMethod = FlockExotic_Cell;
    type Property = FlockExotic_Property;
}

// ============================================== //
// ======              Login               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Login {
        pub accountDBID_s: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Login_onKickedFromServer {
        pub a0: i32,
    }

    #[derive(Debug)]
    pub struct Login_receiveLoginQueueNumber {
        pub a0: u64,
    }

    #[derive(Debug)]
    pub struct Login_setPeripheryRoutingGroup {
        pub a0: AutoString,
        pub a1: Python,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Login_Client {
        Login_onKickedFromServer(0x00, 4),
        Login_receiveLoginQueueNumber(0x01, 8),
        Login_setPeripheryRoutingGroup(0x02, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Login_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Login_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Login_Property {
        accountDBID_s(0x00, var16, AutoString),
    }
}

impl Entity for Login {
    const TYPE_ID: u16 = 0x0B;
    type ClientMethod = Login_Client;
    type BaseMethod = Login_Base;
    type CellMethod = Login_Cell;
    type Property = Login_Property;
}

// ============================================== //
// ======          DetachedTurret          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DetachedTurret {
        pub vehicleCompDescr: AutoString,
        pub outfitCD: AutoString,
        pub isUnderWater: BOOL,
        pub isCollidingWithWorld: BOOL,
        pub vehicleID: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct DetachedTurret_onStaticCollision {
        pub a0: f32,
        pub a1: Vec3,
        pub a2: Vec3,
    }

    #[derive(Debug)]
    pub struct DetachedTurret_showDamageFromShot {
        pub a0: Vec<VEHICLE_HIT_POINT>,
        pub a1: u8,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum DetachedTurret_Client {
        DetachedTurret_onStaticCollision(0x00, 28),
        DetachedTurret_showDamageFromShot(0x01, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum DetachedTurret_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum DetachedTurret_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum DetachedTurret_Property {
        isUnderWater(0x00, 1, BOOL),
        isCollidingWithWorld(0x01, 1, BOOL),
        vehicleID(0x02, 4, i32),
        vehicleCompDescr(0x03, var16, AutoString),
        outfitCD(0x04, var16, AutoString),
    }
}

impl Entity for DetachedTurret {
    const TYPE_ID: u16 = 0x0C;
    type ClientMethod = DetachedTurret_Client;
    type BaseMethod = DetachedTurret_Base;
    type CellMethod = DetachedTurret_Cell;
    type Property = DetachedTurret_Property;
}

// ============================================== //
// ======         DebugDrawEntity          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DebugDrawEntity {
        pub drawObjects: Vec<ANON163>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum DebugDrawEntity_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum DebugDrawEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum DebugDrawEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum DebugDrawEntity_Property {
        drawObjects(0x00, var16, Vec<ANON163>),
    }
}

impl Entity for DebugDrawEntity {
    const TYPE_ID: u16 = 0x0D;
    type ClientMethod = DebugDrawEntity_Client;
    type BaseMethod = DebugDrawEntity_Base;
    type CellMethod = DebugDrawEntity_Cell;
    type Property = DebugDrawEntity_Property;
}

// ============================================== //
// ======   ClientSelectableCameraObject   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableCameraObject {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableCameraObject_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableCameraObject_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableCameraObject_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableCameraObject_Property {
    }
}

impl Entity for ClientSelectableCameraObject {
    const TYPE_ID: u16 = 0x0E;
    type ClientMethod = ClientSelectableCameraObject_Client;
    type BaseMethod = ClientSelectableCameraObject_Base;
    type CellMethod = ClientSelectableCameraObject_Cell;
    type Property = ClientSelectableCameraObject_Property;
}

// ============================================== //
// ======  ClientSelectableCameraVehicle   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableCameraVehicle {
        pub modelName: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableCameraVehicle_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableCameraVehicle_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableCameraVehicle_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableCameraVehicle_Property {
        modelName(0x00, var16, AutoString),
    }
}

impl Entity for ClientSelectableCameraVehicle {
    const TYPE_ID: u16 = 0x0F;
    type ClientMethod = ClientSelectableCameraVehicle_Client;
    type BaseMethod = ClientSelectableCameraVehicle_Base;
    type CellMethod = ClientSelectableCameraVehicle_Cell;
    type Property = ClientSelectableCameraVehicle_Property;
}

// ============================================== //
// ======  ClientSelectableWebLinksOpener  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableWebLinksOpener {
        pub url: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableWebLinksOpener_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableWebLinksOpener_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableWebLinksOpener_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableWebLinksOpener_Property {
        url(0x00, var16, AutoString),
    }
}

impl Entity for ClientSelectableWebLinksOpener {
    const TYPE_ID: u16 = 0x10;
    type ClientMethod = ClientSelectableWebLinksOpener_Client;
    type BaseMethod = ClientSelectableWebLinksOpener_Base;
    type CellMethod = ClientSelectableWebLinksOpener_Cell;
    type Property = ClientSelectableWebLinksOpener_Property;
}

// ============================================== //
// ======    ClientSelectableEasterEgg     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableEasterEgg {
        pub imageName: AutoString,
        pub multiLanguageSupport: BOOL,
        pub outlineModelName: AutoString,
        pub animationSequence: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableEasterEgg_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableEasterEgg_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableEasterEgg_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableEasterEgg_Property {
        multiLanguageSupport(0x00, 1, BOOL),
        imageName(0x01, var16, AutoString),
        outlineModelName(0x02, var16, AutoString),
        animationSequence(0x03, var16, AutoString),
    }
}

impl Entity for ClientSelectableEasterEgg {
    const TYPE_ID: u16 = 0x11;
    type ClientMethod = ClientSelectableEasterEgg_Client;
    type BaseMethod = ClientSelectableEasterEgg_Base;
    type CellMethod = ClientSelectableEasterEgg_Cell;
    type Property = ClientSelectableEasterEgg_Property;
}

// ============================================== //
// ======           EmptyEntity            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct EmptyEntity {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum EmptyEntity_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum EmptyEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum EmptyEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum EmptyEntity_Property {
    }
}

impl Entity for EmptyEntity {
    const TYPE_ID: u16 = 0x12;
    type ClientMethod = EmptyEntity_Client;
    type BaseMethod = EmptyEntity_Base;
    type CellMethod = EmptyEntity_Cell;
    type Property = EmptyEntity_Property;
}

// ============================================== //
// ======     LimitedVisibilityEntity      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LimitedVisibilityEntity {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum LimitedVisibilityEntity_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum LimitedVisibilityEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum LimitedVisibilityEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum LimitedVisibilityEntity_Property {
    }
}

impl Entity for LimitedVisibilityEntity {
    const TYPE_ID: u16 = 0x13;
    type ClientMethod = LimitedVisibilityEntity_Client;
    type BaseMethod = LimitedVisibilityEntity_Base;
    type CellMethod = LimitedVisibilityEntity_Cell;
    type Property = LimitedVisibilityEntity_Property;
}

// ============================================== //
// ======             HeroTank             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct HeroTank {
        pub markerHeightFactor: f32,
        pub vehicleTurretYaw: f32,
        pub vehicleGunPitch: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum HeroTank_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum HeroTank_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum HeroTank_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum HeroTank_Property {
        markerHeightFactor(0x00, 4, f32),
        vehicleTurretYaw(0x01, 4, f32),
        vehicleGunPitch(0x02, 4, f32),
    }
}

impl Entity for HeroTank {
    const TYPE_ID: u16 = 0x14;
    type ClientMethod = HeroTank_Client;
    type BaseMethod = HeroTank_Base;
    type CellMethod = HeroTank_Cell;
    type Property = HeroTank_Property;
}

// ============================================== //
// ======           PlatoonTank            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PlatoonTank {
        pub markerHeightFactor: f32,
        pub vehicleTurretYaw: f32,
        pub vehicleGunPitch: f32,
        pub slotIndex: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum PlatoonTank_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum PlatoonTank_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum PlatoonTank_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum PlatoonTank_Property {
        markerHeightFactor(0x00, 4, f32),
        vehicleTurretYaw(0x01, 4, f32),
        vehicleGunPitch(0x02, 4, f32),
        slotIndex(0x03, 4, i32),
    }
}

impl Entity for PlatoonTank {
    const TYPE_ID: u16 = 0x15;
    type ClientMethod = PlatoonTank_Client;
    type BaseMethod = PlatoonTank_Base;
    type CellMethod = PlatoonTank_Cell;
    type Property = PlatoonTank_Property;
}

// ============================================== //
// ======         PlatoonLighting          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PlatoonLighting {
        pub animationStateMachine: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum PlatoonLighting_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum PlatoonLighting_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum PlatoonLighting_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum PlatoonLighting_Property {
        animationStateMachine(0x00, var16, AutoString),
    }
}

impl Entity for PlatoonLighting {
    const TYPE_ID: u16 = 0x16;
    type ClientMethod = PlatoonLighting_Client;
    type BaseMethod = PlatoonLighting_Base;
    type CellMethod = PlatoonLighting_Cell;
    type Property = PlatoonLighting_Property;
}

// ============================================== //
// ======            SectorBase            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SectorBase {
        pub isActive: BOOL,
        pub team: u8,
        pub baseID: u8,
        pub sectorID: u8,
        pub maxPoints: f32,
        pub pointsPercentage: u8,
        pub capturingStopped: BOOL,
        pub onDamageCooldownTime: f32,
        pub radius: f32,
        pub isCaptured: BOOL,
        pub invadersCount: u8,
        pub expectedCaptureTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum SectorBase_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum SectorBase_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum SectorBase_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum SectorBase_Property {
        isActive(0x00, 1, BOOL),
        team(0x01, 1, u8),
        baseID(0x02, 1, u8),
        sectorID(0x03, 1, u8),
        pointsPercentage(0x04, 1, u8),
        capturingStopped(0x05, 1, BOOL),
        isCaptured(0x06, 1, BOOL),
        invadersCount(0x07, 1, u8),
        maxPoints(0x08, 4, f32),
        onDamageCooldownTime(0x09, 4, f32),
        radius(0x0A, 4, f32),
        expectedCaptureTime(0x0B, 4, f32),
    }
}

impl Entity for SectorBase {
    const TYPE_ID: u16 = 0x17;
    type ClientMethod = SectorBase_Client;
    type BaseMethod = SectorBase_Base;
    type CellMethod = SectorBase_Cell;
    type Property = SectorBase_Property;
}

// ============================================== //
// ======              Sector              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Sector {
        pub groupID: u8,
        pub sectorID: u8,
        pub playerGroup: u8,
        pub IDInPlayerGroup: u8,
        pub lengthX: f32,
        pub lengthZ: f32,
        pub team: u8,
        pub state: u8,
        pub transitionTime: f32,
        pub endOfTransitionPeriod: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Sector_showBomb {
        pub a0: Vec3,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Sector_Client {
        Sector_showBomb(0x00, 12),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Sector_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Sector_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Sector_Property {
        groupID(0x00, 1, u8),
        sectorID(0x01, 1, u8),
        playerGroup(0x02, 1, u8),
        IDInPlayerGroup(0x03, 1, u8),
        team(0x04, 1, u8),
        state(0x05, 1, u8),
        lengthX(0x06, 4, f32),
        lengthZ(0x07, 4, f32),
        transitionTime(0x08, 4, f32),
        endOfTransitionPeriod(0x09, 4, f32),
    }
}

impl Entity for Sector {
    const TYPE_ID: u16 = 0x18;
    type ClientMethod = Sector_Client;
    type BaseMethod = Sector_Base;
    type CellMethod = Sector_Cell;
    type Property = Sector_Property;
}

// ============================================== //
// ======        DestructibleEntity        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DestructibleEntity {
        pub isActive: BOOL,
        pub team: u8,
        pub destructibleEntityID: u8,
        pub health: f32,
        pub maxHealth: f32,
        pub isDestructibleDestroyed: BOOL,
        pub typeID: u8,
        pub linkedMapActivities: AutoString,
        pub damageStickers: Vec<VEHICLE_HIT_POINT>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct DestructibleEntity_onHealthChanged {
        pub a0: i16,
        pub a1: OBJECT_ID,
        pub a2: u8,
        pub a3: i32,
    }

    #[derive(Debug)]
    pub struct DestructibleEntity_showDamageFromShot {
        pub a0: OBJECT_ID,
        pub a1: u8,
        pub a2: i32,
        pub a3: u8,
    }

    #[derive(Debug)]
    pub struct DestructibleEntity_showDamageFromExplosion {
        pub a0: OBJECT_ID,
        pub a1: i32,
        pub a2: u8,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum DestructibleEntity_Client {
        DestructibleEntity_showDamageFromExplosion(0x00, 9),
        DestructibleEntity_showDamageFromShot(0x01, 10),
        DestructibleEntity_onHealthChanged(0x02, 11),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum DestructibleEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum DestructibleEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum DestructibleEntity_Property {
        isActive(0x00, 1, BOOL),
        team(0x01, 1, u8),
        destructibleEntityID(0x02, 1, u8),
        isDestructibleDestroyed(0x03, 1, BOOL),
        typeID(0x04, 1, u8),
        health(0x05, 4, f32),
        maxHealth(0x06, 4, f32),
        linkedMapActivities(0x07, var16, AutoString),
        damageStickers(0x08, var16, Vec<VEHICLE_HIT_POINT>),
    }
}

impl Entity for DestructibleEntity {
    const TYPE_ID: u16 = 0x19;
    type ClientMethod = DestructibleEntity_Client;
    type BaseMethod = DestructibleEntity_Base;
    type CellMethod = DestructibleEntity_Cell;
    type Property = DestructibleEntity_Property;
}

// ============================================== //
// ======         StepRepairPoint          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StepRepairPoint {
        pub team: u8,
        pub radius: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum StepRepairPoint_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum StepRepairPoint_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum StepRepairPoint_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum StepRepairPoint_Property {
        team(0x00, 1, u8),
        radius(0x01, 4, f32),
    }
}

impl Entity for StepRepairPoint {
    const TYPE_ID: u16 = 0x1A;
    type ClientMethod = StepRepairPoint_Client;
    type BaseMethod = StepRepairPoint_Base;
    type CellMethod = StepRepairPoint_Cell;
    type Property = StepRepairPoint_Property;
}

// ============================================== //
// ======          ProtectionZone          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ProtectionZone {
        pub zoneID: u8,
        pub lengthX: f32,
        pub lengthZ: f32,
        pub team: u8,
        pub isActive: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ProtectionZone_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ProtectionZone_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ProtectionZone_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ProtectionZone_Property {
        zoneID(0x00, 1, u8),
        team(0x01, 1, u8),
        isActive(0x02, 1, BOOL),
        lengthX(0x03, 4, f32),
        lengthZ(0x04, 4, f32),
    }
}

impl Entity for ProtectionZone {
    const TYPE_ID: u16 = 0x1B;
    type ClientMethod = ProtectionZone_Client;
    type BaseMethod = ProtectionZone_Base;
    type CellMethod = ProtectionZone_Cell;
    type Property = ProtectionZone_Property;
}

// ============================================== //
// ======             TeamInfo             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TeamInfo {
        pub teamID: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct TeamInfo_onCombatEquipmentUsed {
        pub a0: OBJECT_ID,
        pub a1: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct TeamInfo_showHittingArea {
        pub a0: u16,
        pub a1: Vec3,
        pub a2: Vec3,
        pub a3: f64,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum TeamInfo_Client {
        TeamInfo_onCombatEquipmentUsed(0x00, 8),
        TeamInfo_showHittingArea(0x01, 34),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum TeamInfo_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum TeamInfo_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum TeamInfo_Property {
        teamID(0x00, 4, i32),
    }
}

impl Entity for TeamInfo {
    const TYPE_ID: u16 = 0x1C;
    type ClientMethod = TeamInfo_Client;
    type BaseMethod = TeamInfo_Base;
    type CellMethod = TeamInfo_Cell;
    type Property = TeamInfo_Property;
}

// ============================================== //
// ======            AvatarInfo            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AvatarInfo {
        pub avatarID: OBJECT_ID,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum AvatarInfo_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum AvatarInfo_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum AvatarInfo_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum AvatarInfo_Property {
        avatarID(0x00, 4, OBJECT_ID),
    }
}

impl Entity for AvatarInfo {
    const TYPE_ID: u16 = 0x1D;
    type ClientMethod = AvatarInfo_Client;
    type BaseMethod = AvatarInfo_Base;
    type CellMethod = AvatarInfo_Cell;
    type Property = AvatarInfo_Property;
}

// ============================================== //
// ======        ArenaObserverInfo         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaObserverInfo {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ArenaObserverInfo_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ArenaObserverInfo_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ArenaObserverInfo_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ArenaObserverInfo_Property {
    }
}

impl Entity for ArenaObserverInfo {
    const TYPE_ID: u16 = 0x1E;
    type ClientMethod = ArenaObserverInfo_Client;
    type BaseMethod = ArenaObserverInfo_Base;
    type CellMethod = ArenaObserverInfo_Cell;
    type Property = ArenaObserverInfo_Property;
}

// ============================================== //
// ======           AreaOfEffect           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AreaOfEffect {
        pub vehicleID: i32,
        pub equipmentID: i32,
        pub launchTime: f64,
        pub strikeTime: f64,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct AreaOfEffect_playEffect {
        pub a0: AutoString,
        pub a1: Vec3,
        pub a2: f32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum AreaOfEffect_Client {
        AreaOfEffect_playEffect(0x00, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum AreaOfEffect_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum AreaOfEffect_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum AreaOfEffect_Property {
        vehicleID(0x00, 4, i32),
        equipmentID(0x01, 4, i32),
        launchTime(0x02, 8, f64),
        strikeTime(0x03, 8, f64),
    }
}

impl Entity for AreaOfEffect {
    const TYPE_ID: u16 = 0x1F;
    type ClientMethod = AreaOfEffect_Client;
    type BaseMethod = AreaOfEffect_Base;
    type CellMethod = AreaOfEffect_Cell;
    type Property = AreaOfEffect_Property;
}

// ============================================== //
// ======           AttackBomber           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AttackBomber {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum AttackBomber_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum AttackBomber_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum AttackBomber_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum AttackBomber_Property {
    }
}

impl Entity for AttackBomber {
    const TYPE_ID: u16 = 0x20;
    type ClientMethod = AttackBomber_Client;
    type BaseMethod = AttackBomber_Base;
    type CellMethod = AttackBomber_Cell;
    type Property = AttackBomber_Property;
}

// ============================================== //
// ======       AttackArtilleryFort        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AttackArtilleryFort {
        pub team: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum AttackArtilleryFort_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum AttackArtilleryFort_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum AttackArtilleryFort_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum AttackArtilleryFort_Property {
        team(0x00, 4, i32),
    }
}

impl Entity for AttackArtilleryFort {
    const TYPE_ID: u16 = 0x21;
    type ClientMethod = AttackArtilleryFort_Client;
    type BaseMethod = AttackArtilleryFort_Base;
    type CellMethod = AttackArtilleryFort_Cell;
    type Property = AttackArtilleryFort_Property;
}

// ============================================== //
// ======        PersonalDeathZone         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PersonalDeathZone {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum PersonalDeathZone_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum PersonalDeathZone_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum PersonalDeathZone_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum PersonalDeathZone_Property {
    }
}

impl Entity for PersonalDeathZone {
    const TYPE_ID: u16 = 0x22;
    type ClientMethod = PersonalDeathZone_Client;
    type BaseMethod = PersonalDeathZone_Base;
    type CellMethod = PersonalDeathZone_Cell;
    type Property = PersonalDeathZone_Property;
}

// ============================================== //
// ======   ClientSelectableRankedObject   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableRankedObject {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableRankedObject_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableRankedObject_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableRankedObject_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableRankedObject_Property {
    }
}

impl Entity for ClientSelectableRankedObject {
    const TYPE_ID: u16 = 0x23;
    type ClientMethod = ClientSelectableRankedObject_Client;
    type BaseMethod = ClientSelectableRankedObject_Base;
    type CellMethod = ClientSelectableRankedObject_Cell;
    type Property = ClientSelectableRankedObject_Property;
}

// ============================================== //
// ======         SimulatedVehicle         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SimulatedVehicle {
        pub publicInfo: PUBLIC_VEHICLE_INFO,
        pub isPlayerVehicle: BOOL,
        pub realVehicleID: OBJECT_ID,
        pub simulationData_position: Vec3,
        pub simulationData_dynAttachmentsInfo: DYN_ATTACHMENTS_INFO,
        pub simulationData_rotation: Vec3,
        pub simulationData_velocity: Vec3,
        pub simulationData_angVelocity: Vec3,
        pub simulationData_simulationType: AutoString,
        pub simulationData_health: i16,
        pub simulationData_engineMode: Box<[u8; 2]>,
        pub simulationData_gunAngles: Vec2,
        pub simulationData_turretAndGunSpeed: Vec2,
        pub simulationData_damageStickers: Vec<VEHICLE_HIT_POINT>,
        pub simulationData_brokenTracks: Vec<TRACK_STATE>,
        pub simulationData_siegeState: BOOL,
        pub simulationData_wheelsState: u16,
        pub simulationData_wheelsSteering: Vec<f32>,
        pub simulationData_tracksInAir: Box<[BOOL; 2]>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum SimulatedVehicle_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum SimulatedVehicle_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum SimulatedVehicle_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum SimulatedVehicle_Property {
        isPlayerVehicle(0x00, 1, BOOL),
        simulationData_siegeState(0x01, 1, BOOL),
        simulationData_health(0x02, 2, i16),
        simulationData_engineMode(0x03, 2, Box<[u8; 2]>),
        simulationData_wheelsState(0x04, 2, u16),
        simulationData_tracksInAir(0x05, 2, Box<[BOOL; 2]>),
        realVehicleID(0x06, 4, OBJECT_ID),
        simulationData_dynAttachmentsInfo(0x07, 6, DYN_ATTACHMENTS_INFO),
        simulationData_gunAngles(0x08, 8, Vec2),
        simulationData_turretAndGunSpeed(0x09, 8, Vec2),
        simulationData_position(0x0A, 12, Vec3),
        simulationData_rotation(0x0B, 12, Vec3),
        simulationData_velocity(0x0C, 12, Vec3),
        simulationData_angVelocity(0x0D, 12, Vec3),
        publicInfo(0x0E, var16, PUBLIC_VEHICLE_INFO),
        simulationData_simulationType(0x0F, var16, AutoString),
        simulationData_damageStickers(0x10, var16, Vec<VEHICLE_HIT_POINT>),
        simulationData_brokenTracks(0x11, var16, Vec<TRACK_STATE>),
        simulationData_wheelsSteering(0x12, var16, Vec<f32>),
    }
}

impl Entity for SimulatedVehicle {
    const TYPE_ID: u16 = 0x24;
    type ClientMethod = SimulatedVehicle_Client;
    type BaseMethod = SimulatedVehicle_Base;
    type CellMethod = SimulatedVehicle_Cell;
    type Property = SimulatedVehicle_Property;
}

// ============================================== //
// ====== ClientSelectableHangarsSwitcher  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientSelectableHangarsSwitcher {
        pub destHangar: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ClientSelectableHangarsSwitcher_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ClientSelectableHangarsSwitcher_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ClientSelectableHangarsSwitcher_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ClientSelectableHangarsSwitcher_Property {
        destHangar(0x00, var16, AutoString),
    }
}

impl Entity for ClientSelectableHangarsSwitcher {
    const TYPE_ID: u16 = 0x25;
    type ClientMethod = ClientSelectableHangarsSwitcher_Client;
    type BaseMethod = ClientSelectableHangarsSwitcher_Base;
    type CellMethod = ClientSelectableHangarsSwitcher_Cell;
    type Property = ClientSelectableHangarsSwitcher_Property;
}

// ============================================== //
// ======         StaticDeathZone          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StaticDeathZone {
        pub zoneId: AutoString,
        pub isActive: BOOL,
        pub vehiclesUnderFire: Vec<VEHICLE_IN_DEATHZONE>,
        pub maskingPolygonsCount: u8,
        pub proximityMarkerStyle: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct StaticDeathZone_onDeathZoneDamage {
        pub a0: OBJECT_ID,
        pub a1: AutoString,
    }

    #[derive(Debug)]
    pub struct StaticDeathZone_onDeathZoneNotification {
        pub a0: BOOL,
        pub a1: OBJECT_ID,
        pub a2: f32,
        pub a3: f32,
    }

    #[derive(Debug)]
    pub struct StaticDeathZone_onEntityEnteredInZone {
        pub a0: OBJECT_ID,
    }

    #[derive(Debug)]
    pub struct StaticDeathZone_onEntityLeftZone {
        pub a0: OBJECT_ID,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum StaticDeathZone_Client {
        StaticDeathZone_onEntityEnteredInZone(0x00, 4),
        StaticDeathZone_onEntityLeftZone(0x01, 4),
        StaticDeathZone_onDeathZoneNotification(0x02, 13),
        StaticDeathZone_onDeathZoneDamage(0x03, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum StaticDeathZone_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum StaticDeathZone_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum StaticDeathZone_Property {
        isActive(0x00, 1, BOOL),
        maskingPolygonsCount(0x01, 1, u8),
        zoneId(0x02, var16, AutoString),
        vehiclesUnderFire(0x03, var16, Vec<VEHICLE_IN_DEATHZONE>),
        proximityMarkerStyle(0x04, var16, AutoString),
    }
}

impl Entity for StaticDeathZone {
    const TYPE_ID: u16 = 0x26;
    type ClientMethod = StaticDeathZone_Client;
    type BaseMethod = StaticDeathZone_Base;
    type CellMethod = StaticDeathZone_Cell;
    type Property = StaticDeathZone_Property;
}

// ============================================== //
// ======            BasicMine             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BasicMine {
        pub equipmentID: u32,
        pub ownerVehicleID: u32,
        pub isDetonated: BOOL,
        pub isActivated: BOOL,
        pub activationTimeDelay: u32,
        pub mineNumber: u8,
        pub isMarkerEnabled: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum BasicMine_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum BasicMine_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum BasicMine_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum BasicMine_Property {
        isDetonated(0x00, 1, BOOL),
        isActivated(0x01, 1, BOOL),
        mineNumber(0x02, 1, u8),
        isMarkerEnabled(0x03, 1, BOOL),
        equipmentID(0x04, 4, u32),
        ownerVehicleID(0x05, 4, u32),
        activationTimeDelay(0x06, 4, u32),
    }
}

impl Entity for BasicMine {
    const TYPE_ID: u16 = 0x27;
    type ClientMethod = BasicMine_Client;
    type BaseMethod = BasicMine_Base;
    type CellMethod = BasicMine_Cell;
    type Property = BasicMine_Property;
}

// ============================================== //
// ======          NetworkEntity           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct NetworkEntity {
        pub unique_id: AutoString,
        pub prefab_path: AutoString,
        pub scale: Vec3,
        pub name: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum NetworkEntity_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum NetworkEntity_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum NetworkEntity_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum NetworkEntity_Property {
        scale(0x00, 12, Vec3),
        unique_id(0x01, var16, AutoString),
        prefab_path(0x02, var16, AutoString),
        name(0x03, var16, AutoString),
    }
}

impl Entity for NetworkEntity {
    const TYPE_ID: u16 = 0x28;
    type ClientMethod = NetworkEntity_Client;
    type BaseMethod = NetworkEntity_Base;
    type CellMethod = NetworkEntity_Cell;
    type Property = NetworkEntity_Property;
}

// ============================================== //
// ======               Mine               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Mine {
        pub equipmentID: u32,
        pub ownerVehicleID: u32,
        pub isDetonated: BOOL,
        pub deployTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Mine_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Mine_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Mine_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Mine_Property {
        isDetonated(0x00, 1, BOOL),
        equipmentID(0x01, 4, u32),
        ownerVehicleID(0x02, 4, u32),
        deployTime(0x03, 4, f32),
    }
}

// UNCONFIRMED: `Mine` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for Mine {
    const TYPE_ID: u16 = 0x29;
    type ClientMethod = Mine_Client;
    type BaseMethod = Mine_Base;
    type CellMethod = Mine_Cell;
    type Property = Mine_Property;
}

// ============================================== //
// ======               Loot               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Loot {
        pub pickupRange: f32,
        pub pickupTime: f32,
        pub pickedUpBy: OBJECT_ID,
        pub typeID: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Loot_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Loot_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Loot_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Loot_Property {
        typeID(0x00, 1, u8),
        pickupRange(0x01, 4, f32),
        pickupTime(0x02, 4, f32),
        pickedUpBy(0x03, 4, OBJECT_ID),
    }
}

// UNCONFIRMED: `Loot` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for Loot {
    const TYPE_ID: u16 = 0x2A;
    type ClientMethod = Loot_Client;
    type BaseMethod = Loot_Base;
    type CellMethod = Loot_Cell;
    type Property = Loot_Property;
}

// ============================================== //
// ======            Placement             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Placement {
        pub typeID: i32,
        pub dropTime: f32,
        pub teamID: i32,
        pub yawAxis: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Placement_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Placement_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Placement_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Placement_Property {
        typeID(0x00, 4, i32),
        dropTime(0x01, 4, f32),
        teamID(0x02, 4, i32),
        yawAxis(0x03, 4, f32),
    }
}

// UNCONFIRMED: `Placement` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for Placement {
    const TYPE_ID: u16 = 0x2B;
    type ClientMethod = Placement_Client;
    type BaseMethod = Placement_Base;
    type CellMethod = Placement_Cell;
    type Property = Placement_Property;
}

// ============================================== //
// ======          InfluenceZone           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InfluenceZone {
        pub equipmentID: u32,
        pub team: u8,
        pub zonesPosition: Vec<Vec3>,
        pub dotCreatorId: u32,
        pub dropOffset: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum InfluenceZone_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum InfluenceZone_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum InfluenceZone_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum InfluenceZone_Property {
        team(0x00, 1, u8),
        equipmentID(0x01, 4, u32),
        dotCreatorId(0x02, 4, u32),
        dropOffset(0x03, 4, f32),
        zonesPosition(0x04, var16, Vec<Vec3>),
    }
}

// UNCONFIRMED: `InfluenceZone` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for InfluenceZone {
    const TYPE_ID: u16 = 0x2C;
    type ClientMethod = InfluenceZone_Client;
    type BaseMethod = InfluenceZone_Base;
    type CellMethod = InfluenceZone_Cell;
    type Property = InfluenceZone_Property;
}

// ============================================== //
// ======        BattleRoyaleRadio         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleRoyaleRadio {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum BattleRoyaleRadio_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum BattleRoyaleRadio_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum BattleRoyaleRadio_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum BattleRoyaleRadio_Property {
    }
}

// UNCONFIRMED: `BattleRoyaleRadio` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for BattleRoyaleRadio {
    const TYPE_ID: u16 = 0x2D;
    type ClientMethod = BattleRoyaleRadio_Client;
    type BaseMethod = BattleRoyaleRadio_Base;
    type CellMethod = BattleRoyaleRadio_Cell;
    type Property = BattleRoyaleRadio_Property;
}

// ============================================== //
// ======          ThunderStrike           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ThunderStrike {
        pub equipmentID: u16,
        pub attackerID: OBJECT_ID,
        pub delayEndTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct ThunderStrike_hitThunderStrike {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ThunderStrike_Client {
        ThunderStrike_hitThunderStrike(0x00, 0),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ThunderStrike_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ThunderStrike_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ThunderStrike_Property {
        equipmentID(0x00, 2, u16),
        attackerID(0x01, 4, OBJECT_ID),
        delayEndTime(0x02, 4, f32),
    }
}

// UNCONFIRMED: `ThunderStrike` is declared by the `battle_royale` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for ThunderStrike {
    const TYPE_ID: u16 = 0x2E;
    type ClientMethod = ThunderStrike_Client;
    type BaseMethod = ThunderStrike_Base;
    type CellMethod = ThunderStrike_Cell;
    type Property = ThunderStrike_Property;
}

// ============================================== //
// ======          Comp7Lighting           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Comp7Lighting {
        pub animationStateMachine: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum Comp7Lighting_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum Comp7Lighting_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum Comp7Lighting_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum Comp7Lighting_Property {
        animationStateMachine(0x00, var16, AutoString),
    }
}

// UNCONFIRMED: `Comp7Lighting` is declared by the `comp7` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for Comp7Lighting {
    const TYPE_ID: u16 = 0x2F;
    type ClientMethod = Comp7Lighting_Client;
    type BaseMethod = Comp7Lighting_Base;
    type CellMethod = Comp7Lighting_Cell;
    type Property = Comp7Lighting_Property;
}

// ============================================== //
// ======         ApplicationPoint         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ApplicationPoint {
        pub vehicleID: i32,
        pub equipmentID: i32,
        pub launchTime: f32,
        pub level: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ApplicationPoint_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ApplicationPoint_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ApplicationPoint_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ApplicationPoint_Property {
        vehicleID(0x00, 4, i32),
        equipmentID(0x01, 4, i32),
        launchTime(0x02, 4, f32),
        level(0x03, 4, i32),
    }
}

// UNCONFIRMED: `ApplicationPoint` is declared by the `comp7_core` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for ApplicationPoint {
    const TYPE_ID: u16 = 0x30;
    type ClientMethod = ApplicationPoint_Client;
    type BaseMethod = ApplicationPoint_Base;
    type CellMethod = ApplicationPoint_Cell;
    type Property = ApplicationPoint_Property;
}

// ============================================== //
// ======          ReplayAccount           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ReplayAccount {
        pub filename: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct ReplayAccount_onKickedFromServer {
        pub a0: AutoString,
        pub a1: u8,
        pub a2: u32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct ReplayAccount_stopReplay {
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum ReplayAccount_Client {
        ReplayAccount_onKickedFromServer(0x00, var8),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum ReplayAccount_Base {
        ReplayAccount_stopReplay(0x00, 0),
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum ReplayAccount_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum ReplayAccount_Property {
        filename(0x00, var16, AutoString),
    }
}

// UNCONFIRMED: `ReplayAccount` is declared by the `server_side_replay` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for ReplayAccount {
    const TYPE_ID: u16 = 0x31;
    type ClientMethod = ReplayAccount_Client;
    type BaseMethod = ReplayAccount_Base;
    type CellMethod = ReplayAccount_Cell;
    type Property = ReplayAccount_Property;
}

// ============================================== //
// ======             SPGZone              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SPGZone {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__enum_entity_methods! {  // Entity methods on client
    pub enum SPGZone_Client {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on base
    pub enum SPGZone_Base {
    }
}

wgtk::__enum_entity_methods! {  // Entity methods on cell
    pub enum SPGZone_Cell {
    }
}

wgtk::__enum_entity_properties! {  // Client-visible properties
    pub enum SPGZone_Property {
    }
}

// UNCONFIRMED: `SPGZone` is declared by the `story_mode` extension, not the main
// `scripts/entities.xml`. Its TYPE_ID below only continues that list's own
// numbering (extension-alphabetical, then declaration order) by analogy with
// the already-confirmed static-component method-folding rule -- it has NOT
// itself been checked against a live capture.
impl Entity for SPGZone {
    const TYPE_ID: u16 = 0x32;
    type ClientMethod = SPGZone_Client;
    type BaseMethod = SPGZone_Base;
    type CellMethod = SPGZone_Cell;
    type Property = SPGZone_Property;
}

// ============================================== //
// ======           [COLLECTION]           ====== //
// ============================================== //

wgtk::__enum_entities! {
    /// Generic entity type enumeration allowing decoding of any entity.
    pub enum Entities {
        Account(0x01),
        Avatar(0x02),
        ArenaInfo(0x03),
        ClientSelectableObject(0x04),
        HangarVehicle(0x05),
        Vehicle(0x06),
        AreaDestructibles(0x07),
        OfflineEntity(0x08),
        Flock(0x09),
        FlockExotic(0x0A),
        Login(0x0B),
        DetachedTurret(0x0C),
        DebugDrawEntity(0x0D),
        ClientSelectableCameraObject(0x0E),
        ClientSelectableCameraVehicle(0x0F),
        ClientSelectableWebLinksOpener(0x10),
        ClientSelectableEasterEgg(0x11),
        EmptyEntity(0x12),
        LimitedVisibilityEntity(0x13),
        HeroTank(0x14),
        PlatoonTank(0x15),
        PlatoonLighting(0x16),
        SectorBase(0x17),
        Sector(0x18),
        DestructibleEntity(0x19),
        StepRepairPoint(0x1A),
        ProtectionZone(0x1B),
        TeamInfo(0x1C),
        AvatarInfo(0x1D),
        ArenaObserverInfo(0x1E),
        AreaOfEffect(0x1F),
        AttackBomber(0x20),
        AttackArtilleryFort(0x21),
        PersonalDeathZone(0x22),
        ClientSelectableRankedObject(0x23),
        SimulatedVehicle(0x24),
        ClientSelectableHangarsSwitcher(0x25),
        StaticDeathZone(0x26),
        BasicMine(0x27),
        NetworkEntity(0x28),
        Mine(0x29),
        Loot(0x2A),
        Placement(0x2B),
        InfluenceZone(0x2C),
        BattleRoyaleRadio(0x2D),
        ThunderStrike(0x2E),
        Comp7Lighting(0x2F),
        ApplicationPoint(0x30),
        ReplayAccount(0x31),
        SPGZone(0x32),
    }
}

