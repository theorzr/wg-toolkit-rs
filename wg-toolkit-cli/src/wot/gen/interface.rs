use super::alias::*;

// ============================================== //
// ======              Wheels              ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Wheels {
        pub steeringAngles: Vec<u8>,
        pub wheelsScroll: Vec<u8>,
        pub wheelsState: u64,
        pub burnoutLevel: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehiclesSpawnListStorage_Avatar  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehiclesSpawnListStorage_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct VehiclesSpawnListStorage_Avatar_updateSpawnList {
        pub a0: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleRemovalController_Avatar  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleRemovalController_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct VehicleRemovalController_Avatar_removeVehicle {
        pub a0: OBJECT_ID,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         VehicleObserver          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleObserver {
        pub remoteCamera: REMOTE_CAMERA_DATA,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct VehicleObserver_setRemoteCamera {
        pub a0: REMOTE_CAMERA_DATA,
    }

}

// ============================================== //
// ====== VehicleHealthBroadcastListenerComponent_Avatar ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleHealthBroadcastListenerComponent_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct VehicleHealthBroadcastListenerComponent_Avatar_onVehicleHealthChanged {
        pub a0: OBJECT_ID,
        pub a1: i16,
        pub a2: OBJECT_ID,
        pub a3: u8,
        pub a4: i8,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          VehicleAIProxy          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleAIProxy {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    TriggersController_Avatar     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TriggersController_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct TriggersController_Avatar_externalTrigger {
        pub a0: AutoString,
        pub a1: Python,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         TransactionUser          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TransactionUser {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         ThrottledMethods         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ThrottledMethods {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       TeamHealthBar_Avatar       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TeamHealthBar_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct TeamHealthBar_Avatar_updateTeamsHealthPercentage {
        pub a0: Vec<u8>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         TeamBase_Vehicle         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TeamBase_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     StepRepairPoint_Vehicle      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StepRepairPoint_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     SmokeController_Vehicle      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SmokeController_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          SessionTracker          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SessionTracker {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        ServerSideReplays         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ServerSideReplays {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          Sector_Vehicle          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Sector_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        SectorBase_Vehicle        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SectorBase_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        RepairBase_Vehicle        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RepairBase_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     RecoveryMechanic_Vehicle     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RecoveryMechanic_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct RecoveryMechanic_Vehicle_recoveryMechanic_startRecovering {
    }

    #[derive(Debug)]
    pub struct RecoveryMechanic_Vehicle_recoveryMechanic_stopRecovering {
    }

}

// ============================================== //
// ======     RecoveryMechanic_Avatar      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RecoveryMechanic_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct RecoveryMechanic_Avatar_notifyCannotStartRecovering {
    }

    #[derive(Debug)]
    pub struct RecoveryMechanic_Avatar_notifyCancelled {
    }

    #[derive(Debug)]
    pub struct RecoveryMechanic_Avatar_updateState {
        pub activated: BOOL,
        pub state: i32,
        pub timer_duration: i32,
        pub end_of_timer: f32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          QuestProcessor          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct QuestProcessor {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      ProtectionZone_Vehicle      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ProtectionZone_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== ProtectionZoneController_Avatar  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ProtectionZoneController_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      PlayerMessenger_chat2       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PlayerMessenger_chat2 {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct PlayerMessenger_chat2_messenger_onActionByServer_chat2 {
        pub action_id: i16,
        pub request_id: u16,
        pub args: GENERIC_MESSENGER_ARGS_chat2,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct PlayerMessenger_chat2_messenger_onActionByClient_chat2 {
        pub action_id: i16,
        pub request_id: u16,
        pub args: GENERIC_MESSENGER_ARGS_chat2,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======            PlayLimits            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PlayLimits {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     PlaneTrajectoryArenaInfo     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PlaneTrajectoryArenaInfo {
        pub planeTrajectory: PLANE_TRAJECTORY,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          Perks_Vehicle           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Perks_Vehicle {
        pub perkEffects: PERK_EFFECTS,
        pub perks: Vec<PERK_INFO_HUD>,
        pub perksRibbonNotify: Vec<PERK_INFO_RIBBON>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======            Invoicing             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Invoicing {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        InvitationsClient         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InvitationsClient {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct InvitationsClient_processInvitations {
        pub a0: Python,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           Invitations            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Invitations {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======               Harm               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Harm {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======            EntityTrap            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct EntityTrap {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    DestructibleEntity_Vehicle    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DestructibleEntity_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    DestructibleEntity_Avatar     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DestructibleEntity_Avatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           Destructible           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Destructible {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== DefenderBonusController_Vehicle  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DefenderBonusController_Vehicle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           ControlPoint           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ControlPoint {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        ClientCommandsPort        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ClientCommandsPort {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct ClientCommandsPort_onCmdResponse {
        pub request_id: i16,
        pub result_id: i16,
        pub error: AutoString,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_onCmdResponseExt {
        pub request_id: i16,
        pub result_id: i16,
        pub error: AutoString,
        pub ext: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdNoArgs {
        pub request_id: i16,
        pub command_id: i16,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdStr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: AutoString,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt2 {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: i64,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt3 {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: i64,
        pub arg2: i64,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt4 {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: i64,
        pub arg2: i64,
        pub arg3: i64,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt2Str {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: i64,
        pub arg2: AutoString,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdInt3Str {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: i64,
        pub arg2: i64,
        pub arg3: AutoString,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdIntArr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: Vec<i32>,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdIntStr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: AutoString,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdIntStrArr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: i64,
        pub arg1: Vec<AutoString>,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdIntArrStrArr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: Vec<i64>,
        pub arg1: Vec<AutoString>,
    }

    #[derive(Debug)]
    pub struct ClientCommandsPort_doCmdStrArr {
        pub request_id: i16,
        pub command_id: i16,
        pub arg0: Vec<AutoString>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======               Chat               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Chat {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Chat_onChatAction {
        pub a0: CHAT_ACTION_DATA,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct Chat_chatCommandFromClient {
        pub request_id: i64,
        pub command_id: u8,
        pub channel_id: OBJECT_ID,
        pub i64_arg: i64,
        pub i16_arg: i16,
        pub str_arg0: AutoString,
        pub str_arg1: AutoString,
    }

    #[derive(Debug)]
    pub struct Chat_inviteCommand {
        pub request_id: i64,
        pub command_id: u8,
        pub invalid_type: i8,
        pub receiver_name: AutoString,
        pub i64_arg: i64,
        pub i16_arg: i16,
        pub str_arg0: AutoString,
        pub str_arg1: AutoString,
    }

    #[derive(Debug)]
    pub struct Chat_ackCommand {
        pub request_id: i64,
        pub command_id: u8,
        pub time: f64,
        pub invite_id: i64,
        pub a4: i64,
    }

    #[derive(Debug)]
    pub struct Chat_onStreamComplete {
        pub a0: i16,
        pub a1: BOOL,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      BattleResultProcessor       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleResultProcessor {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          BattleFeedback          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleFeedback {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          AvatarObserver          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AvatarObserver {
        pub remoteCamera: REMOTE_CAMERA_DATA,
        pub isObserverFPV: BOOL,
        pub numOfObservers: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct AvatarObserver_switchObserverFPV {
        pub a0: BOOL,
    }

}

// ============================================== //
// ======            AvatarEpic            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AvatarEpic {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct AvatarEpic_welcomeToSector {
        pub sector_id: u8,
        pub group_id: u8,
        pub group_state: u8,
        pub good_group: BOOL,
        pub action_time: f32,
        pub action_duration: f32,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onStepRepairPointAction {
        pub repair_point_index: OBJECT_ID,
        pub action: u8,
        pub next_action_time: f32,
        pub points_healed: u16,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onSectorBaseAction {
        pub sector_base_id: u8,
        pub action: u8,
        pub next_action_time: f32,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_enteringProtectionZone {
        pub zone_id: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_leavingProtectionZone {
        pub zone_id: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_protectionZoneShooting {
        pub zone_id: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onSectorShooting {
        pub sector_id: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onXPUpdated {
        pub xp: i16,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onCrewRoleFactorAndRankUpdate {
        pub new_factor: f32,
        pub ally_vehicle_id: i64,
        pub ally_new_rank: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_syncPurchasedAbilities {
        pub abilities: Vec<i64>,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onRandomReserveOffer {
        pub offer: Vec<i32>,
        pub level: Vec<u8>,
        pub slot_index: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onRankUpdate {
        pub new_rank: u8,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_showDestructibleShotResults {
        pub destructible_entity_id: u8,
        pub hit_flags: Vec<u32>,
    }

    #[derive(Debug)]
    pub struct AvatarEpic_onDestructibleDestroyed {
        pub destructible_entity_id: u8,
        pub shooter_id: OBJECT_ID,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AvatarEpic_enableFrontLineDevInfo {
        pub a0: BOOL,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          AccountVersion          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountVersion {
        pub requiredVersion_2310: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        AccountUnitRemote         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountUnitRemote {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        AccountUnitClient         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountUnitClient {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountUnitClient_accountUnitClient_create {
        pub a0: i32,
        pub a1: i32,
        pub a2: i32,
        pub a3: AutoString,
        pub a4: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountUnitClient_accountUnitClient_join {
        pub a0: i32,
        pub a1: u64,
        pub a2: i32,
    }

    #[derive(Debug)]
    pub struct AccountUnitClient_accountUnitClient_doCmd {
        pub a0: i32,
        pub a1: OBJECT_ID,
        pub a2: i32,
        pub a3: u64,
        pub a4: i32,
        pub a5: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountUnitClient_accountUnitClient_sendInvites {
        pub a0: i32,
        pub a1: u64,
        pub a2: Vec<DB_ID>,
        pub a3: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountUnitClient_accountUnitClient_setRosterSlots {
        pub a0: i32,
        pub a1: u64,
        pub a2: Vec<i32>,
        pub a3: Vec<AutoString>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        AccountUnitBrowser        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountUnitBrowser {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountUnitBrowser_accountUnitBrowser_subscribe {
        pub unit_type_flags: i16,
        pub show_other_locations: BOOL,
    }

    #[derive(Debug)]
    pub struct AccountUnitBrowser_accountUnitBrowser_unsubscribe {
    }

    #[derive(Debug)]
    pub struct AccountUnitBrowser_accountUnitBrowser_recenter {
        pub target_rating: i32,
        pub unit_type_flags: i16,
        pub show_other_locations: BOOL,
    }

    #[derive(Debug)]
    pub struct AccountUnitBrowser_accountUnitBrowser_doCmd {
        pub cmd: i32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       AccountUnitAssembler       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountUnitAssembler {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           AccountUnit            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountUnit {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       AccountSysMessenger        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountSysMessenger {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       AccountSpaProcessor        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountSpaProcessor {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         AccountPrebattle         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountPrebattle {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountPrebattle_accountPrebattle_createTraining {
        pub arena_type_id: i32,
        pub round_length: i32,
        pub is_opened: BOOL,
        pub comment: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountPrebattle_accountPrebattle_createDevPrebattle {
        pub bonus_type: u8,
        pub arena_gui_type: u8,
        pub arena_type_id: i32,
        pub round_length: i32,
        pub comment: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountPrebattle_accountPrebattle_sendPrebattleInvites {
        pub accounts: Vec<i64>,
        pub comment: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       AccountIGRProcessing       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountIGRProcessing {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    AccountGlobalMapConnector     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountGlobalMapConnector {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountGlobalMapConnector_accountGlobalMapConnector_callGlobalMapMethod {
        pub request_id: u64,
        pub method: i32,
        pub i64_arg: i64,
        pub str_arg: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          AccountEditor           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountEditor {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         AccountDebugger          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountDebugger {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountDebugger_accountDebugger_registerDebugTaskResult {
        pub a0: i64,
        pub a1: i32,
        pub a2: i64,
    }

    #[derive(Debug)]
    pub struct AccountDebugger_accountDebugger_sendDebugTaskResultChunk {
        pub a0: i64,
        pub a1: i64,
        pub a2: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           AccountClan            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountClan {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          AccountAvatar           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountAvatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountAvatar_accountAvatar_sendAccountStats {
        pub a0: u32,
        pub a1: Vec<AutoString>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  AccountAuthTokenProviderClient  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountAuthTokenProviderClient {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct AccountAuthTokenProviderClient_onTokenReceived {
        pub request_id: u16,
        pub token_type: u8,
        pub data: Python,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     AccountAuthTokenProvider     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountAuthTokenProvider {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct AccountAuthTokenProvider_requestToken {
        pub request_id: u16,
        pub token_type: u8,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======           AccountAdmin           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountAdmin {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct AccountBattleRoyaleTournamentComponent_setTournamentToken {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct AccountBattleRoyaleTournamentComponent_setParticipants {
        pub a0: Vec<TOURNAMEMT_PARTICIPANT>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LaPingerComponent_pingMeAndThenJustTouchMe {
        pub a0: AutoString,
        pub a1: u16,
        pub a2: DB_ID,
        pub a3: u16,
        pub a4: u32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct StoryModeAccountComponent_setDevelopmentFeature {
        pub a0: AutoString,
        pub a1: i32,
        pub a2: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

    #[derive(Debug)]
    pub struct StoryModeAvatarComponent_setDevelopmentFeature {
        pub a0: AutoString,
        pub a1: i32,
        pub a2: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct StoryModeAvatarComponent_checkPositionForEquipment {
        pub a0: i32,
        pub a1: Vec3,
    }

}

