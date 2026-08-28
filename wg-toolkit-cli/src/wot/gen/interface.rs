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

// ============================================== //
// ======      BattleRoyaleComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleRoyaleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   AccountBattleRoyaleComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountBattleRoyaleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== AccountBattleRoyaleTournamentComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountBattleRoyaleTournamentComponent {
    }
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

// ============================================== //
// ======       ArenaComp7Component        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaComp7Component {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      AccountComp7Component       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountComp7Component {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     PrebattleComp7Component      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PrebattleComp7Component {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    AccountComp7LightComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountComp7LightComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     ArenaComp7LightComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaComp7LightComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      KafkaLoggingComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KafkaLoggingComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleHealthBroadcastComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleHealthBroadcastComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  VehicleVisionOverrideComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleVisionOverrideComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         FLArenaComponent         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLArenaComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        FLAccountComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLAccountComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   FLAccountPrebattleComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLAccountPrebattleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  FLAccountBattleResultComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLAccountBattleResultComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   FLArenaBattleResultComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLArenaBattleResultComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        FLRespawnComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLRespawnComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    AccountFunRandomComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AccountFunRandomComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  FunRandomKafkaLoggingComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FunRandomKafkaLoggingComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     FunRandomArenaController     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FunRandomArenaController {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  InBattleAchievementsComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InBattleAchievementsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== JourneyMarathonAccountComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct JourneyMarathonAccountComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        LaPingerComponent         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LaPingerComponent {
    }
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

// ============================================== //
// ======        LastStandComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LastStandComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        LSAccountComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSAccountComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSAccountEquipmentController   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSAccountEquipmentController {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSArenaBattleResultComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaBattleResultComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSKafkaPublisherComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSKafkaPublisherComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    OpenBundleAccountComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct OpenBundleAccountComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   ResourceWellAccountComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ResourceWellAccountComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    ServerReplayArenaComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ServerReplayArenaComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        StoryModeMissions         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeMissions {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    StoryModeAccountComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeAccountComponent {
    }
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

// ============================================== //
// ======     StoryModeArenaComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeArenaComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     StoryModeAvatarComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeAvatarComponent {
        pub wrongApplicationPoint: Vec3,
        pub isPositionValid: BOOL,
    }
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

// ============================================== //
// ======        ArenaLootComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaLootComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       ArenaMinesComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaMinesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     ArenaDeathZonesComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaDeathZonesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== ArenaEntityPositionGridComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaEntityPositionGridComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   ArenaInfoDeathZonesComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaInfoDeathZonesComponent {
        pub activeZones: Vec<u8>,
        pub vehicleLifetimeInDeathZone: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      VehicleDeathZoneEffect      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleDeathZoneEffect {
        pub state: u8,
        pub damageTime: f32,
        pub timeToDamage: f32,
        pub warningStartTime: f32,
        pub warningFinishTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct VehicleDeathZoneEffect_onDamaged {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     SpawnKeyPointController      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SpawnKeyPointController {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      SpawnKeyPointTeamInfo       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SpawnKeyPointTeamInfo {
        pub availableSpawnKeyPoints: Vec<SPAWN_KEY_POINT>,
        pub teamSpawnKeyPoints: Vec<TEAM_SPAWN_KEY_POINT>,
        pub spawnKeyPointsCloseTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct SpawnKeyPointTeamInfo_chooseSpawnKeyPoint {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct SpawnKeyPointTeamInfo_placeVehicle {
    }

}

// ============================================== //
// ======             BattleXP             ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleXP {
        pub battleXP: i32,
        pub battleXpLvlData: Box<[u16; 3]>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          ConeVisibility          ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ConeVisibility {
        pub circularVisionAngle: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======              Radar               ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct Radar {
        pub radarReadinessTime: f32,
        pub radarReady: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct Radar_activatePatrickEffect {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct Radar_activateRadar {
    }

}

// ============================================== //
// ======     InBattleUpgradeReadiness     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InBattleUpgradeReadiness {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         InBattleUpgrades         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InBattleUpgrades {
        pub upgradeReadinessTime: TIME_WITH_REASON,
        pub isVehicleUpgrading: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct InBattleUpgrades_onVehicleUpgraded {
        pub a0: AutoString,
        pub a1: AutoString,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct InBattleUpgrades_upgradeVehicle {
        pub a0: i32,
    }

}

// ============================================== //
// ======           VehicleLoot            ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleLoot {
        pub lootID: OBJECT_ID,
        pub lootTypeID: u8,
        pub pickupEndTime: f32,
        pub pickupTotalTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      InBattleUpgradesAvatar      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct InBattleUpgradesAvatar {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct InBattleUpgradesAvatar_vehicleUpgradeResponse {
        pub a0: Vec<i32>,
        pub a1: Vec<AutoString>,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   AvatarBattleRoyaleComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AvatarBattleRoyaleComponent {
        pub playerPlace: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        BattleXPArenaInfo         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleXPArenaInfo {
        pub vehiclesAverageBattleLevel: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======          LootArenaInfo           ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LootArenaInfo {
        pub lootPositions: Vec<ANON185>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  VehicleCorrodingShotComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleCorrodingShotComponent {
        pub finishTime: f32,
        pub canBeStoppedRepairKit: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleCorrodingShotPreparingComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleCorrodingShotPreparingComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleAdaptationHealthRestoreComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleAdaptationHealthRestoreComponent {
        pub finishTime: f32,
        pub restoreHealth: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  VehicleThunderStrikeComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleThunderStrikeComponent {
        pub finishTime: f32,
        pub canBeStoppedRepairKit: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   VehicleShotPassionComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleShotPassionComponent {
        pub finishTime: f32,
        pub stage: u8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     VehicleSelfBuffComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleSelfBuffComponent {
        pub finishTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    VehicleBerserkerComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleBerserkerComponent {
        pub finishTime: f32,
        pub tickInterval: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       ArenaInfoBRComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaInfoBRComponent {
        pub nextDropWave: f32,
        pub defeatedTeams: Vec<u8>,
        pub isRespawnTimeFinished: BOOL,
        pub respawnPeriod: f32,
        pub timeToResurrect: f32,
        pub xpConfig: Vec<u16>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct ArenaInfoBRComponent_notifyLaunchPosition {
        pub a0: i32,
        pub a1: Vec3,
        pub a2: f32,
        pub a3: f32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    VehicleHealPointComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleHealPointComponent {
        pub endTime: f32,
        pub radius: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     VehicleHealingComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleHealingComponent {
        pub finishTime: f32,
        pub isSourceVehicle: BOOL,
        pub isInactivation: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleFireCircleEffectComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleFireCircleEffectComponent {
        pub finishTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    VehicleBRRespawnComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleBRRespawnComponent {
        pub position: Vec2,
        pub lives: i16,
        pub resurrectTime: f32,
        pub teammateResurrectTime: f32,
        pub timeBlockToResurrect: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== VehicleBRRespawnEffectComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleBRRespawnEffectComponent {
        pub initialTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== BattleRoyaleObserverInfoComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BattleRoyaleObserverInfoComponent {
        pub teamsMayRespawn: Vec<i32>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   VehicleBRStPatrickComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleBRStPatrickComponent {
        pub coinsCount: u32,
        pub teammateCoinsCount: u32,
        pub isDailyBonusAvailable: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      VehicleComp7Component       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleComp7Component {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     ArenaInfoComp7Component      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaInfoComp7Component {
        pub ranks: Python,
        pub bannedVehicles: Python,
        pub vehicleBanList: Vec<i64>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      TeamInfoComp7Component      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TeamInfoComp7Component {
        pub roleSkillLevels: Python,
        pub teamVivoxChannel: Python,
        pub endPrepickTime: f32,
        pub endVotingTime: f32,
        pub banVotingStates: Python,
        pub candidatesForBan: Python,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct TeamInfoComp7Component_setVivoxPresence {
        pub a0: BOOL,
    }

}

// ============================================== //
// ====== ArenaObserverInfoComp7Component  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaObserverInfoComp7Component {
        pub vehiclesInfo: Vec<COMP7_VEHICLE_INFO>,
        pub poiInfo: Vec<COMP7_POI_INFO>,
        pub teamBasesInfo: Vec<COMP7_BASE_INFO>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     AvatarComp7BaseComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AvatarComp7BaseComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct AvatarComp7BaseComponent_chooseVehicleForBan {
        pub a0: i64,
    }

    #[derive(Debug)]
    pub struct AvatarComp7BaseComponent_confirmBanVehicle {
    }

}

// ============================================== //
// ======       VehiclePrestigePoint       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehiclePrestigePoint {
        pub prestigePoints: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======         VehicleRoleSkill         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleRoleSkill {
        pub roleEquipmentState: ROLE_EQUIPMENT_STATE,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     VehicleInspireController     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleInspireController {
        pub isActive: BOOL,
        pub displayedState: STATE_WITH_TIME_INTERVAL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      VehicleHealController       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleHealController {
        pub displayedStates: Vec<STATE_WITH_TIME_INTERVAL>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    VehicleComp7LightComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleComp7LightComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        FLAvatarComponent         ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLAvatarComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct FLAvatarComponent_updateRespawnVehicles {
        pub a0: Vec<RESPAWN_AVAILABLE_VEHICLE>,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_updateRespawnCooldowns {
        pub a0: Vec<RESPAWN_COOLDOWN_ITEM>,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_updateRespawnInfo {
        pub a0: RESPAWN_INFO,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_updateVehicleLimits {
        pub a0: Vec<RESPAWN_LIMITED_VEHICLES>,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_onTeamLivesRestored {
        pub a0: Vec<u8>,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_updatePlayerLives {
        pub a0: u8,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct FLAvatarComponent_chooseVehicleForRespawn {
        pub a0: u32,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_chooseRespawnZone {
        pub a0: Vec3,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_switchSetup {
        pub a0: u32,
        pub a1: u8,
        pub a2: u8,
    }

    #[derive(Debug)]
    pub struct FLAvatarComponent_performRespawn {
    }

}

// ============================================== //
// ======       FLReservesComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLReservesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    FLAvatarReservesComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLAvatarReservesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     FLBattleUpgradeReserves      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLBattleUpgradeReserves {
        pub upgradeReadinessTime: TIME_WITH_REASON,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        FLVehicleComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLVehicleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    FLArenaMinefieldComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLArenaMinefieldComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== FLVehicleRegenerationKitComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLVehicleRegenerationKitComponent {
        pub regenerationKit: REGENERATION_KIT_INFO,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     FLStealthRadarComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLStealthRadarComponent {
        pub stealthRadar: STEALTH_RADAR_INFO,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== FLArenaBattleModifiersComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FLArenaBattleModifiersComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    ArenaAchievementsComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ArenaAchievementsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   VehicleAchievementsComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VehicleAchievementsComponent {
        pub achievements: Vec<u16>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSTeamInfoStatsComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSTeamInfoStatsComponent {
        pub damage: Vec<ANON201>,
        pub blocked: Vec<ANON203>,
        pub assist: Vec<ANON205>,
        pub teamHealth: Vec<ANON207>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSTeamInfoVoiceChatComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSTeamInfoVoiceChatComponent {
        pub teamVivoxChannel: Python,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct LSTeamInfoVoiceChatComponent_setVivoxPresence {
        pub a0: BOOL,
    }

}

// ============================================== //
// ======  LSVehicleBattleStatsComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleBattleStatsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       LSArenaDropComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaDropComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSArenaPhasesComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaPhasesComponent {
        pub activePhase: u8,
        pub phasesCount: u8,
        pub phaseDuration: u32,
        pub timeLeft: i32,
        pub isTimerAlarmEnabled: BOOL,
        pub isRespawnEnabled: BOOL,
        pub activeEnvironment: AutoString,
        pub isBCMarkersCleanupEnabled: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSArenaPhasesComponent_hideVehicleOnMinimap {
    }

    #[derive(Debug)]
    pub struct LSArenaPhasesComponent_cleanBCMarkers {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleSoulsContainerComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleSoulsContainerComponent {
        pub lastCollected: Box<[u32; 2]>,
        pub souls: u32,
        pub capacity: u32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSVehicleCheatsComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_spawnLootAtShotPoint {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_addSouls {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_toggleBuff {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_addModificator {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_removeFactor {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_spawnCampAtShotPoint {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_destroyCamp {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_killAllBots {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_killSelf {
    }

    #[derive(Debug)]
    pub struct LSVehicleCheatsComponent_spawnPersonalDeathZones {
    }

}

// ============================================== //
// ======       LSBuffNitroComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffNitroComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffNitroAccelerationComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffNitroAccelerationComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSBuffFactorsComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffFactorsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSBuffSequencesComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffSequencesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffWithBoosterSequencesComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffWithBoosterSequencesComponent {
        pub factors: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSArenaVehicleKillComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaVehicleKillComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSBuffRepairComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffRepairComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffPeriodicHealComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffPeriodicHealComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       LSVehicleAIComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleAIComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSBuffRadiusAoEComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffRadiusAoEComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSVehicleFireComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleFireComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffIgniteVehicleComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffIgniteVehicleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSBuffDrainHealthComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffDrainHealthComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffDamageVehicleComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffDamageVehicleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSBuffVehicleIconComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffVehicleIconComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSVehicleRepairComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleRepairComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSVehicleFeedbackComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleFeedbackComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSVehicleDissolveComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleDissolveComponent {
        pub deathTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffDamageResistsComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffDamageResistsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffSoulsDrainResistComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffSoulsDrainResistComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSArenaVehicleRemovalComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaVehicleRemovalComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSArenaWatersComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaWatersComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffInterruptibleComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffInterruptibleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffEffectsListPlayerComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffEffectsListPlayerComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleMaxHealthModifierComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleMaxHealthModifierComponent {
        pub maxHealth: i16,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffAddBuffOnShotComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffAddBuffOnShotComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffAddMaxHealthComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffAddMaxHealthComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSBuffVehicleEffectsComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffVehicleEffectsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSBuffLaserSightComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffLaserSightComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSArenaSoundComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSArenaSoundComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSArenaSoundComponent_onBotCreated {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSVehicleSoundComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleSoundComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSVehicleSoundComponent_onLootFailed {
        pub a0: AutoString,
        pub a1: Vec<OBJECT_ID>,
    }

    #[derive(Debug)]
    pub struct LSVehicleSoundComponent_onLootSucceed {
        pub a0: AutoString,
        pub a1: Vec<OBJECT_ID>,
    }

    #[derive(Debug)]
    pub struct LSVehicleSoundComponent_onDamageReceived {
        pub a0: OBJECT_ID,
        pub a1: u8,
        pub a2: i16,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       LSVehicleDeathOnFall       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleDeathOnFall {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffInvisibilityForAIComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffInvisibilityForAIComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       LSBuffSoundComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffSoundComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSKafkaVehicleLoggingComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSKafkaVehicleLoggingComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSKafkaLoggingComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSKafkaLoggingComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSBuffDissolveComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffDissolveComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSBuffAddDynGroupComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffAddDynGroupComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSLootVisualComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSLootVisualComponent {
        pub lootState: i8,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSLootVehicleWatcherComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSLootVehicleWatcherComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSNitroAccelerationControl    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSNitroAccelerationControl {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSFairplayVehicleBattleStatsComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSFairplayVehicleBattleStatsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       LSBuffShootComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffShootComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSVehicleInstantShotComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleInstantShotComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSVehicleShotChargerComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleShotChargerComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSBuffStunVehicleComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffStunVehicleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleMultiAuraHandlerComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleMultiAuraHandlerComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSVehicleShellsComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleShellsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     LSWaveProgressComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSWaveProgressComponent {
        pub enemiesInfo: ANON212,
        pub healthBreakpoints: Vec<i32>,
        pub enemiesStatus: Vec<ANON214>,
        pub convoyStatus: Vec<ANON216>,
        pub convoyDistanceIndicator: i32,
        pub convoyHealth: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  LSVehicleHitDirectionComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleHitDirectionComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSBuffOnShotComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffOnShotComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleInvulnerableMarkerComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleInvulnerableMarkerComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSVehicleInvulnerableMarkerComponent_showNoHitMarker {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSBuffAddDynGroupOnEventComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffAddDynGroupOnEventComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleBoosterFactorsComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleBoosterFactorsComponent {
        pub factors: Python,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSObeliskInfoComponent      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSObeliskInfoComponent {
        pub isPresent: BOOL,
        pub observedObeliskCD: i32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSObeliskInfoComponent_onDamageReceived {
    }

    #[derive(Debug)]
    pub struct LSObeliskInfoComponent_onDeath {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      LSBeamTargetComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBeamTargetComponent {
        pub beamParams: Vec<ANON220>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct LSBeamTargetComponent_applyEffects {
        pub a0: AutoString,
    }

    #[derive(Debug)]
    pub struct LSBeamTargetComponent_removeEffects {
        pub a0: Vec<i32>,
    }

    #[derive(Debug)]
    pub struct LSBeamTargetComponent_showDamage {
        pub a0: i32,
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   LSBuffNotificationComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSBuffNotificationComponent {
        pub startTime: f32,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== LSVehicleContinuousTurretRotator ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleContinuousTurretRotator {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    LSVehicleDrownedComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LSVehicleDrownedComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======      StoryModeAfkController      ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeAfkController {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== StoryModeDamageResistsComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeDamageResistsComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== StoryModeAccuracyOverrideComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeAccuracyOverrideComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== StoryModeModulesImmunityComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeModulesImmunityComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    StoryModeLootableComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct StoryModeLootableComponent {
        pub radius: f32,
        pub startTime: f32,
        pub captureTime: f32,
        pub markerStyle: AutoString,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  SMReconAbilityEntityComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMReconAbilityEntityComponent {
        pub spottedVehiclesIDs: Vec<u64>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  SMReconAbilityVehicleComponent  ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMReconAbilityVehicleComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== SMDistractionAbilityEntityComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMDistractionAbilityEntityComponent {
        pub spottedVehiclesIDs: Vec<u64>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== SMDistractionAbilityArenaInfoComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMDistractionAbilityArenaInfoComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== SMDetectionDelayObserverComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMDetectionDelayObserverComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== SMDetectionDelayObservableComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMDetectionDelayObservableComponent {
        pub timers: Python,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======    SMVehicleRespawnComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMVehicleRespawnComponent {
        pub explodeVehicle: BOOL,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   SMAbilitiesTrackerComponent    ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMAbilitiesTrackerComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======  SMAbilitiesInterruptComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMAbilitiesInterruptComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   SMAbilitiesRechargeComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMAbilitiesRechargeComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ====== SMArtilleryStrikeChannelComponent ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMArtilleryStrikeChannelComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======   SMRemovableDynGroupComponent   ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMRemovableDynGroupComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       SMSequencesComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMSequencesComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======        SMSound3DComponent        ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMSound3DComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======     SMSound3DObjectComponent     ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMSound3DObjectComponent {
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

// ============================================== //
// ======       BunkerLogicComponent       ====== //
// ============================================== //

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BunkerLogicComponent {
        pub vehicleIDs: Vec<i32>,
    }
}

wgtk::__struct_simple_codec! {  // Methods on client

    #[derive(Debug)]
    pub struct BunkerLogicComponent_bunkerDestroyed {
    }

}

wgtk::__struct_simple_codec! {  // Methods on base

}

wgtk::__struct_simple_codec! {  // Methods on cell

}

