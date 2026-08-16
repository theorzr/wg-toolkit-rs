pub use wgtk::net::codec::{AutoString, Python, Mailbox};
pub use glam::{Vec2, Vec3, Vec4};

pub type BOOL = u8;
pub type OBJECT_ID = i32;
pub type SHOT_ID = i32;
pub type DB_ID = i64;
pub type EXTRA_ID = u8;
pub type VEH_TYPE_CD = u32;
pub type ARENA_UNIQUE_ID = u64;
pub type NETWORK_ID = u32;

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PERK_EFFECTS {
        pub equipment: Vec<AutoString>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SERVER_STATISTICS {
        pub clusterCCU: u32,
        pub regionCCU: u32,
    }
}

pub type QUEUE_INFO = Python;

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DEFAULT_QUEUE_INFO {
        pub classes: Vec<u32>,
        pub players: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct MAPS_TRAINING_QUEUE_INFO {
        pub queues: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FUN_RANDOM_QUEUE_INFO {
        pub events: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PUBLIC_ARENA_INFO {
        pub id: OBJECT_ID,
        pub typeID: i32,
        pub roundLength: i32,
        pub roundStart: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ATTACK_RESULTS {
        pub targetID: OBJECT_ID,
        pub targetVehicleIndex: u8,
        pub targetTeam: u8,
        pub targetTypeCompDescr: VEH_TYPE_CD,
        pub targetIsTeamKiller: BOOL,
        pub targetIsOnTheIgnoredBase: BOOL,
        pub targetIsOnTheCapturableBase: BOOL,
        pub targetIsNotSpotted: BOOL,
        pub targetMaxHealth: u16,
        pub targetHealthBeforeDamage: i16,
        pub enemiesNearTarget: u8,
        pub isRecoil: BOOL,
        pub reason: u8,
        pub shellCompDescr: i32,
        pub hitIndirection: u8,
        pub shotID: SHOT_ID,
        pub numVehiclesAffected: i16,
        pub hitFlags: i32,
        pub crits: i32,
        pub stunDuration: f32,
        pub allCrits: i32,
        pub anyDeviceWasDamaged: BOOL,
        pub damage: i32,
        pub repairCost: u32,
        pub critBonusFactor: f32,
        pub droppedCapturePoints: f32,
        pub trackAssistants: Vec<OBJECT_ID>,
        pub stunAssistants: Vec<OBJECT_ID>,
        pub smokeAssistants: Vec<OBJECT_ID>,
        pub distance: f32,
        pub targetInitialSpeed: f32,
        pub attackerInitialSpeed: f32,
        pub attackerHullDamage: u16,
        pub attackerKilledHimself: BOOL,
        pub attackerHealthBeforeDamage: i16,
        pub circularVisionRadius: f32,
        pub attackerWasInvisible: BOOL,
        pub equipmentID: u16,
        pub isIronShieldDamage: BOOL,
        pub attackerType: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PREBATTLE_INVITE {
        pub createTime: u32,
        pub r#type: u16,
        pub comment: AutoString,
        pub creator: AutoString,
        pub creatorBadges: Python,
        pub creatorDBID: DB_ID,
        pub creatorClanAbbrev: AutoString,
        pub extraData: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PREBATTLE_RESULTS {
        pub winner: u8,
        pub finishReason: u8,
        pub kickReason: u8,
        pub extendedResults: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PUBLIC_CHAT_CHANNEL_INFO {
        pub id: OBJECT_ID,
        pub channelName: AutoString,
        pub isReadOnly: BOOL,
        pub isSecured: BOOL,
        pub flags: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PUBLIC_USERS_ROSTER_MEMBER_INFO {
        pub id: DB_ID,
        pub nickName: AutoString,
        pub accessFlags: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CHAT_ACTION_DATA {
        pub requestID: i64,
        pub action: u8,
        pub actionResponse: u8,
        pub time: f64,
        pub sentTime: f64,
        pub channel: OBJECT_ID,
        pub originator: DB_ID,
        pub originatorNickName: AutoString,
        pub group: u8,
        pub data: Python,
        pub flags: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GENERIC_MESSENGER_ARGS_chat2 {
        pub int32Arg1: i32,
        pub int64Arg1: i64,
        pub floatArg1: f64,
        pub strArg1: AutoString,
        pub strArg2: AutoString,
        pub int8Arg1: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DISCLOSE_EVENT {
        pub vehicleID: OBJECT_ID,
        pub playerName: BOOL,
        pub vehicleType: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DOG_TAG_COMPONENT {
        pub id: i32,
        pub progress: f32,
        pub grade: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DOG_TAG {
        pub components: Vec<DOG_TAG_COMPONENT>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_DOG_TAG {
        pub dogTag: DOG_TAG,
        pub defaultDogTag: DOG_TAG,
        pub showDogTagToKiller: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_DOG_TAG {
        pub vehicleId: OBJECT_ID,
        pub dogTag: DOG_TAG,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DOG_TAG_SETTINGS {
        pub showVictimsDogTag: BOOL,
        pub showDogTagToKiller: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PUBLIC_VEHICLE_INFO {
        pub name: AutoString,
        pub compDescr: AutoString,
        pub outfit: AutoString,
        pub outfitLevel: u8,
        pub index: u8,
        pub team: u8,
        pub prebattleID: OBJECT_ID,
        pub marksOnGun: u8,
        pub crewGroups: Vec<u16>,
        pub commanderSkinID: u16,
        pub maxHealth: u16,
        pub respawnID: u32,
        pub stFrags: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ARENA_VEH_INFO {
        pub vehInvID: i32,
        pub vehCompDescr: AutoString,
        pub vehOutfit: AutoString,
        pub vehAmmo: Vec<i32>,
        pub vehSetups: Python,
        pub vehSetupsIndexes: Python,
        pub vehCrew: Vec<AutoString>,
        pub vehCrewInvIDs: Vec<i32>,
        pub vehCrewSkins: Python,
        pub marksOnGun: u8,
        pub isRent: BOOL,
        pub activeRent: u8,
        pub settings: u16,
        pub enhancements: Python,
        pub vehPerks: Python,
        pub customRoleSlotTypeId: u8,
        pub vehPostProgression: Vec<i32>,
        pub vehDisabledSetupSwitches: Vec<u8>,
        pub isSuitableForReferralBonus: BOOL,
        pub isElite: BOOL,
        pub prestigeLevel: i32,
        pub anonymizedPrestigeLevel: i32,
        pub prestigeGradeMarkID: i32,
        pub stFrags: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct USER_EXTRA_SETTINGS {
        pub hideNonHistoric: u8,
        pub dogTagsUserSettings: DOG_TAG_SETTINGS,
        pub moduleRepairTimer: BOOL,
        pub minimapMinSpottingRange: BOOL,
        pub battleNotifier: BOOL,
        pub additionalzoom: BOOL,
        pub hpinminimap: u8,
        pub hpinplayerspanels: u8,
        pub commandercam: BOOL,
        pub contour: BOOL,
        pub contourPenetrableZone: u8,
        pub contourImpenetrableZone: u8,
        pub crewPerks: BOOL,
        pub mapsInDevelopment: BOOL,
        pub postmortemMode: u8,
        pub commendationsFeedbackEnabled: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ARENA_ADDPLAYER_INFO {
        pub name: AutoString,
        pub attrs: u64,
        pub databaseID: DB_ID,
        pub clanAbbrev: AutoString,
        pub clanDBID: DB_ID,
        pub prebattle: Mailbox,
        pub isPrebattleCreator: BOOL,
        pub forbidInBattleInvitations: BOOL,
        pub arenaUniqueID: ARENA_UNIQUE_ID,
        pub team: u8,
        pub tkillRating: f32,
        pub globalRating: f32,
        pub igrType: i8,
        pub potapovQuestIDs: Vec<u16>,
        pub potapovQuestInfo: Python,
        pub vehiclesInfo: Vec<ARENA_VEH_INFO>,
        pub avatarAmmo: Vec<i32>,
        pub needCheckPenalties: BOOL,
        pub fairplayState: Python,
        pub battlesNum: u32,
        pub ranked: Vec<u16>,
        pub group: u8,
        pub recruiterId: DB_ID,
        pub premMask: i8,
        pub needsAnonymization: BOOL,
        pub wtr: i16,
        pub badges: Python,
        pub overriddenBadge: u8,
        pub dogTag: BATTLE_DOG_TAG,
        pub userExtraSettings: USER_EXTRA_SETTINGS,
        pub isSsrPlayEnabled: BOOL,
        pub componentsData: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AVATAR_VEHICLE_ROSTER {
        pub vehicleID: OBJECT_ID,
        pub prebattleID: OBJECT_ID,
        pub team: i8,
        pub observer: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ATTACKER_INFO {
        pub baseMB: Mailbox,
        pub receiveAttackResultsMB: Mailbox,
        pub team: u8,
        pub position: Vec3,
        pub circularVisionRadius: f32,
        pub health: i16,
        pub noOwner: BOOL,
        pub attackerInitialSpeed: f32,
        pub attackerWasInvisible: BOOL,
        pub attackerTypeCompactDescr: VEH_TYPE_CD,
        pub attackerVehicleIndex: u8,
        pub attackerGunBurstCount: u8,
        pub equipmentID: u16,
        pub attackerType: u8,
        pub attackerSiegeState: i8,
        pub attackerMasterVehID: OBJECT_ID,
        pub needsCount: BOOL,
        pub damageDistributionLowerBound: f32,
        pub piercingDistributionLowerBound: f32,
        pub damageDistributionUpperBound: f32,
        pub piercingDistributionUpperBound: f32,
        pub criticalHitChanceBoost: f32,
        pub attackerDualAccuracyState: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DESTRUCTIBLE_ATTACK_INFO {
        pub hitPoint: Vec3,
        pub shotID: i32,
        pub attacker: ATTACKER_INFO,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CLIENT_STATUS_STATISTICS {
        pub ping_lt_50: f32,
        pub ping_51_100: f32,
        pub ping_101_150: f32,
        pub ping_151_400: f32,
        pub ping_gt_400: f32,
        pub lag: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_SPATIAL_INFO {
        pub vehicleID: OBJECT_ID,
        pub team: u8,
        pub position: Vec3,
        pub isAlive: BOOL,
        pub vehClass: AutoString,
        pub prebattleID: OBJECT_ID,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_SYNC_ATTRS {
        pub circularVisionRadius: u16,
        pub gunPiercing: f32,
        pub gunShotsSpeed: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct IS_OBSERVED_BY_ENEMY {
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SIEGE_STATE_STATUS {
        pub status: u8,
        pub endTime: f32,
        pub timeLeft: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BURNOUT_WARNING {
        pub status: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DUAL_GUN_STATUS_TIMES {
        pub baseTime: f32,
        pub timeLeft: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DUAL_GUN_STATUS {
        pub status: u8,
        pub times: DUAL_GUN_STATUS_TIMES,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DESTROYED_DEVICE_IS_REPAIRING {
        pub extraIndex: u8,
        pub progress: u8,
        pub endTime: f32,
        pub timeLeft: f32,
        pub repairMode: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct IS_OTHER_VEHICLE_DAMAGED_DEVICES_VISIBLE {
        pub status: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BURNOUT_UNAVAILABLE {
        pub status: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct OVERTURN_LEVEL {
        pub level: u8,
        pub times: Vec<f64>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct IN_AOE_ZONE_STATUS {
        pub zoneID: OBJECT_ID,
        pub equipmentID: u16,
        pub team: u8,
        pub startTime: f64,
        pub endTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DROWN_LEVEL {
        pub level: u8,
        pub times: Vec<f64>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_EVENT {
        pub eventType: u8,
        pub targetID: OBJECT_ID,
        pub details: u64,
        pub count: u16,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_EVENTS_SUMMARY {
        pub damage: u32,
        pub trackAssist: u32,
        pub radioAssist: u32,
        pub stunAssist: u32,
        pub smokeAssist: u32,
        pub inspireAssist: u32,
        pub tankings: u32,
        pub lastKillerID: OBJECT_ID,
        pub lastDeathReasonID: u8,
        pub attackReasonExtID: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct REMOTE_CAMERA_DATA {
        pub time: f64,
        pub shotPoint: Vec3,
        pub zoom: u8,
        pub mode: u8,
    }
}

pub type STUN_INFO = f64;

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FRONT_LINE_DATA {
        pub columnWidth: f32,
        pub frontDropPerColumn: f32,
        pub outlierFraction: f32,
        pub outlierVerticalDistance: f32,
        pub intrusionVerticalTolerance: f32,
        pub intrusionCheckExtendBounds: f32,
        pub defenderTeam: u8,
        pub frontEdgeExtendColumns: u8,
        pub frontLineIds: Vec<u8>,
        pub frontLineBounds: Vec<Box<[Vec2; 2]>>,
        pub frontLineMainDirVecs: Vec<Vec2>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AVATAR_AMMO_VIEWS {
        pub vehTypeCompDescrs: Vec<i32>,
        pub compDescrs: Vec<Vec<i32>>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AVATAR_AMMO_FOR_CELL {
        pub abilitiesIDs: Python,
        pub purchasedAbilities: Vec<i8>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct POST_PROGRESSION_SETUPS {
        pub devicesSetups: Vec<Vec<u32>>,
        pub eqsSetups: Vec<Vec<u32>>,
        pub shellsSetups: Vec<Vec<u32>>,
        pub boostersSetups: Vec<Vec<u32>>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_AVAILABLE_VEHICLE {
        pub compDescr: AutoString,
        pub crewCompactDescrs: Vec<AutoString>,
        pub settings: u16,
        pub vehSetups: POST_PROGRESSION_SETUPS,
        pub vehSetupsIndexes: Vec<u8>,
        pub vehPostProgression: Vec<i32>,
        pub customRoleSlotTypeId: u8,
        pub vehDisabledSetupSwitches: Vec<u8>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_COOLDOWN_ITEM {
        pub vehTypeCompDescr: VEH_TYPE_CD,
        pub endOfCooldownPiT: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_ZONE {
        pub position: Vec3,
        pub isEnemyNear: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_INFO {
        pub compDescr: AutoString,
        pub autoRespawnPiT: f32,
        pub manualRespawnPiT: f32,
        pub respawnZones: Vec<RESPAWN_ZONE>,
        pub chosenRespawnZone: Vec3,
        pub vehSetupsIndexes: Vec<u8>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_LIMITED_VEHICLES {
        pub group: u8,
        pub vehTypeCompDescrs: Vec<VEH_TYPE_CD>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RESPAWN_INFO_VEHICLE {
        pub compDescr: AutoString,
        pub crewCompactDescrs: Vec<AutoString>,
        pub commanderSkinID: u16,
        pub marksOnGun: u8,
        pub index: u16,
        pub position: Vec3,
        pub yaw: f32,
        pub prevGroup: u8,
        pub group: u8,
        pub policyID: u8,
        pub onRespawnSettings: Python,
        pub ammo: Vec<i32>,
        pub outfit: AutoString,
        pub vehPerks: Python,
        pub vehSetups: Python,
        pub vehSetupsIndexes: Python,
        pub vehPostProgression: Vec<i32>,
        pub customRoleSlotTypeId: u8,
        pub vehDisabledSetupSwitches: Vec<u8>,
        pub prestigeLevel: i32,
        pub prestigeGradeMarkID: i32,
        pub stFrags: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BUFF_EFFECT {
        pub radius: f32,
        pub startTime: f64,
        pub endTime: f64,
        pub inactivationDelay: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DOT_EFFECT {
        pub endTime: f64,
        pub period: f32,
        pub groupId: u8,
        pub attackReasonID: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BUFF_EFFECT_INACTIVATION {
        pub senderKey: AutoString,
        pub startTime: f64,
        pub endTime: f64,
        pub inactivationStartTime: f64,
        pub inactivationEndTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct HOT_EFFECT {
        pub senderKey: AutoString,
        pub startTime: f64,
        pub endTime: f64,
        pub inactivationStartTime: f64,
        pub inactivationEndTime: f64,
        pub isInfluenceZone: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct INSPIRED_EFFECT {
        pub primary: BOOL,
        pub startTime: f64,
        pub endTime: f64,
        pub inactivationStartTime: f64,
        pub inactivationEndTime: f64,
        pub inactivationSource: BOOL,
        pub equipmentID: u16,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SMOKE_INFO {
        pub smokeID: f64,
        pub equipmentID: u16,
        pub endTime: f32,
        pub team: u8,
        pub expiring: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_DETECTOR_INFO {
        pub detectorID: u16,
        pub point: Vec3,
        pub radius: f32,
        pub aliveOnly: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COOLDOWN_INFO {
        pub id: u8,
        pub endTime: f32,
        pub baseTime: f32,
        pub leftTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CLIENT_VEHICLE_PUBLIC_INFO {
        pub vehID: OBJECT_ID,
        pub name: AutoString,
        pub realName: AutoString,
        pub compDescr: AutoString,
        pub team: i8,
        pub isAlive: i8,
        pub isAvatarReady: i8,
        pub isTeamKiller: i8,
        pub accountDBID: DB_ID,
        pub clanAbbrev: AutoString,
        pub clanDBID: DB_ID,
        pub prebattleID: OBJECT_ID,
        pub isPrebattleCreator: i8,
        pub forbidInBattleInvitations: i8,
        pub events: Python,
        pub igrType: i8,
        pub potapovQuestIDs: Vec<u16>,
        pub potapovQuestInfo: Python,
        pub ranked: Python,
        pub outfit: AutoString,
        pub sessionID: AutoString,
        pub wtr: i16,
        pub badges: Python,
        pub overriddenBadge: u8,
        pub maxHealth: u16,
        pub vehPostProgression: Vec<i32>,
        pub customRoleSlotTypeId: u8,
        pub botDisplayStatus: u8,
        pub prestigeLevel: i32,
        pub anonymizedPrestigeLevel: i32,
        pub prestigeGradeMarkID: i32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PLAY_LIMITS {
        pub curfew: i32,
        pub weeklyPlayLimit: i32,
        pub dailyPlayLimit: i32,
        pub sessionLimit: i32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_CHAT_RESTRICTION {
        pub isBattleChatDisabled: BOOL,
        pub restrictionReasonID: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_IN_DEATHZONE {
        pub vehicleId: OBJECT_ID,
        pub nextStrikeTime: f32,
        pub waveDuration: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TIME_WITH_REASON {
        pub endTime: i32,
        pub totalTime: i32,
        pub reason: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PERK_INFO_HUD {
        pub perkID: OBJECT_ID,
        pub state: u8,
        pub coolDown: f32,
        pub lifeTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PERK_INFO_RIBBON {
        pub perkID: OBJECT_ID,
        pub endTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TOURNAMEMT_PARTICIPANT {
        pub databaseID: DB_ID,
        pub name: AutoString,
        pub role: u8,
        pub teamID: u8,
        pub typeCD: i32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct STEALTH_RADAR_INFO {
        pub equipmentID: u16,
        pub isActive: BOOL,
        pub duration: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct REGENERATION_KIT_INFO {
        pub isActive: BOOL,
        pub duration: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_HEALTH_INFO {
        pub id: OBJECT_ID,
        pub health: i16,
        pub deathReasonID: i8,
        pub isCrewActive: BOOL,
        pub isRespawnActive: BOOL,
        pub deathReasonExtID: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GUN_MARKER {
        pub gunPosition: Vec3,
        pub shotVector: Vec3,
        pub dispersion: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_HIT_POINT {
        pub networkID: NETWORK_ID,
        pub segment: u64,
        pub params: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DYN_ATTACHMENTS_INFO {
        pub activeSequenceLayer: u8,
        pub sequenceTime: f32,
        pub attachmentState: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_ATTACKER_SPOTTED {
        pub position: Vec3,
        pub rotation: Vec3,
        pub gunAngles: Vec2,
        pub velocity: Vec3,
        pub angVelocity: Vec3,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_ATTACKER_UNSPOTTED {
        pub health: u16,
        pub vehicleType: AutoString,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_ATTACKER {
        pub attackerID: OBJECT_ID,
        pub spottedData: KILL_CAM_ATTACKER_SPOTTED,
        pub unspottedData: KILL_CAM_ATTACKER_UNSPOTTED,
        pub mechanicsInfo: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_VICTIM {
        pub position: Vec3,
        pub rotation: Vec3,
        pub health: u16,
        pub relativeArmor: f32,
        pub victimIsNotSpotted: BOOL,
        pub siegeState: u16,
        pub damageStickers: Vec<VEHICLE_HIT_POINT>,
        pub causeOfDeath: AutoString,
        pub mechanicsInfo: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_PROJECTILE_UNSPOTTED {
        pub shotID: SHOT_ID,
        pub trajectoryData: Vec<Box<[Vec3; 2]>>,
        pub gravity: f32,
        pub impactPoint: Vec3,
        pub isExplosion: BOOL,
        pub explosionRadius: f32,
        pub nominalArmor: f32,
        pub angleGain: f32,
        pub hitAngleCos: f32,
        pub triNormal: Vec3,
        pub hasProjectilePierced: BOOL,
        pub hasNonPiercedDamage: BOOL,
        pub ricochetCount: u16,
        pub ricochetAngleCos: f32,
        pub effectiveShellDamage: f32,
        pub damageRandomizationFactor: f32,
        pub is2CaliberRuleActive: BOOL,
        pub is3CaliberRuleActive: BOOL,
        pub impactType: u16,
        pub maxPenetrationAngle: f32,
        pub armorProtectionHE: f32,
        pub spallLinerProtectionHE: f32,
        pub gunInstallationIndex: u8,
        pub gunIndex: u8,
        pub velocity: Vec3,
        pub shellCompDescr: i32,
        pub segments: Vec<VEHICLE_HIT_POINT>,
        pub piercingPower: f32,
        pub nominalPiercingPower: f32,
        pub shellDamageBurstHE: f32,
        pub distanceLossHE: f32,
        pub averageDamageOfShell: f32,
        pub hasDistanceFalloff: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_PROJECTILE_SPOTTED {
        pub distanceOfShot: f32,
        pub damageDistanceModifier: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_PROJECTILE {
        pub unspottedData: KILL_CAM_PROJECTILE_UNSPOTTED,
        pub spottedData: KILL_CAM_PROJECTILE_SPOTTED,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct KILL_CAM_DATA {
        pub attacker: KILL_CAM_ATTACKER,
        pub victim: KILL_CAM_VICTIM,
        pub projectile: KILL_CAM_PROJECTILE,
        pub statusCode: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TRACK_STATE {
        pub isBroken: BOOL,
        pub hitPoint: Vec3,
        pub isDebris: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_HIT_DIRECTION {
        pub hitDirYaw: f32,
        pub attackerID: OBJECT_ID,
        pub damage: u16,
        pub crits: u32,
        pub isBlocked: BOOL,
        pub isShellHE: BOOL,
        pub damagedID: OBJECT_ID,
        pub attackReasonID: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_CLIP_RELOAD_TIME {
        pub endTime: f32,
        pub baseTime: f32,
        pub timeLeft: f32,
        pub firstTime: f32,
        pub stunned: BOOL,
        pub isBoostApplicable: BOOL,
        pub clipTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_GUN_RELOAD_TIME {
        pub endTime: f32,
        pub baseTime: f32,
        pub timeLeft: f32,
        pub clipTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct OWN_VEHICLE_POSITION {
        pub position: Vec3,
        pub direction: Vec3,
        pub speed: f32,
        pub rotationSpeed: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TARGET_VEHICLE_ID {
        pub targetID: OBJECT_ID,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DUAL_GUN_STATE {
        pub activeGun: u8,
        pub gunStates: Vec<u8>,
        pub cooldowns: Vec<COOLDOWN_INFO>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_AMMO {
        pub compactDescr: i32,
        pub quantity: u16,
        pub quantityInClip: u16,
        pub previousStage: u16,
        pub endTime: f32,
        pub totalTime: i16,
        pub index: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_DAMAGE_INFO {
        pub extraIndex: EXTRA_ID,
        pub damageIndex: u8,
        pub entityID: OBJECT_ID,
        pub equipmentID: u16,
        pub damageExtIndex: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_OPTIONAL_DEVICE_STATUS {
        pub deviceID: u8,
        pub isOn: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_SETTING {
        pub vehicleID: OBJECT_ID,
        pub code: u8,
        pub value: i32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TARGETING_INFO {
        pub turretYaw: f32,
        pub gunPitch: f32,
        pub maxTurretRotationSpeed: f32,
        pub maxGunRotationSpeed: f32,
        pub shotDispMultiplierFactor: f32,
        pub gunShotDispersionFactorsTurretRotation: f32,
        pub chassisShotDispersionFactorsMovement: f32,
        pub chassisShotDispersionFactorsRotation: f32,
        pub aimingTime: f32,
        pub gunShotDispersionFactorsAfterShot: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_EVENTS {
        pub events: Vec<BATTLE_EVENT>,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct WELCOME_TO_SECTOR {
        pub sectorID: u8,
        pub groupID: u8,
        pub groupState: u8,
        pub goodGroup: BOOL,
        pub actionTime: f32,
        pub actionDuration: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SECTOR_SHOOTING {
        pub sectorID: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PLANE_TRAJECTORY {
        pub equipmentID: u16,
        pub team: u8,
        pub curTime: f64,
        pub curPos: Vec3,
        pub curDir: Vec2,
        pub nextTime: f64,
        pub nextPos: Vec3,
        pub nextDir: Vec2,
        pub isEndOfFlight: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FIRE_INFO {
        pub deviceExtraIndex: EXTRA_ID,
        pub notificationIndex: u8,
        pub attackerID: OBJECT_ID,
        pub equipmentID: u16,
        pub startTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DEBUFF_INFO {
        pub attackReasonID: u16,
        pub finishTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ROCKET_ACCELERATION_STATE_STATUS {
        pub status: u8,
        pub endTime: f32,
        pub timeLeft: u16,
        pub reuseCount: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct RECHARGEABLE_NITRO_STATE_STATUS {
        pub status: u8,
        pub endTime: f32,
        pub remainingTime: f32,
        pub isCharged: BOOL,
        pub isEmpty: BOOL,
        pub isBelowThreshold: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AUTO_SHOOT_GUN_STATE_STATUS {
        pub state: u8,
        pub rateMultFactor: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct AUTO_SHOOT_GUN_DISPERSION_STATUS {
        pub dispersionFactor: f32,
        pub updateTime: f32,
        pub shotDispersionPerSec: f32,
        pub maxShotDispersion: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TEMPERATURE_GUN_STATE_STATUS {
        pub state: u8,
        pub thermalStateID: u8,
        pub updateTime: f32,
        pub directionFactor: f32,
        pub currentTemperature: f32,
        pub coolingPerSecFactor: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct POWER_MODE_STATE_STATUS {
        pub state: u8,
        pub powerProgress: f32,
        pub stateActivationTime: f32,
        pub directionFactor: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct POWER_MODE_TIME_INFO {
        pub modeThreshold: f32,
        pub modeDuration: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GOODIE_RESOURCE {
        pub r#type: u8,
        pub value: u16,
        pub isPercentage: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GOODIE_STATE_INFO {
        pub state: u8,
        pub finishTime: f64,
        pub count: u16,
        pub expirations: Python,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_GOODIE_RECORD {
        pub goodieID: u32,
        pub lifetime: u16,
        pub useby: u64,
        pub resource: GOODIE_RESOURCE,
        pub stateInfo: GOODIE_STATE_INFO,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ROLE_EQUIPMENT_STATE {
        pub level: u8,
        pub progress: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct STATUS_WITH_TIME_INTERVAL {
        pub statusID: u8,
        pub startTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TIME_INTERVAL {
        pub startTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct STATE_WITH_TIME_INTERVAL {
        pub stateID: u8,
        pub timeInterval: TIME_INTERVAL,
        pub isSourceVehicle: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VISUAL_SCRIPT_EQUIPMENT_STATE {
        pub quantity: i32,
        pub endTime: f32,
        pub totalTime: f32,
        pub prevStage: u8,
        pub stage: u8,
        pub locked: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VISUAL_SCRIPT_EQUIPMENT_PUBLIC_STATE {
        pub stage: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SPAWN_AVAILABLE_VEHICLE {
        pub compDescr: AutoString,
        pub settings: u16,
        pub isRent: BOOL,
        pub isElite: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SPAWN_POINT_INFO {
        pub vehicleID: OBJECT_ID,
        pub number: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SPAWN_KEY_POINT {
        pub guid: AutoString,
        pub position: Vec2,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TEAM_SPAWN_KEY_POINT {
        pub vehID: OBJECT_ID,
        pub guid: AutoString,
        pub placed: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct POLYGON_TRIGGER_PARAMS {
        pub step: u16,
        pub origin: Vec3,
        pub dimensions: Vec3,
        pub verts: Vec<Vec3>,
        pub segments: Vec<Vec<u16>>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SPAWNGROUP_INFO {
        pub name: AutoString,
        pub position: Vec2,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TEAM_LIVES {
        pub vehicleID: OBJECT_ID,
        pub lives: i8,
        pub usedLives: i8,
        pub lockedLives: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct DEATH_INFO {
        pub victimID: OBJECT_ID,
        pub killerID: OBJECT_ID,
        pub reasonID: i8,
        pub equipmentID: i16,
        pub numVehiclesAffected: i16,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLES_INFO {
        pub vehicleID: OBJECT_ID,
        pub isAlive: BOOL,
        pub outfitCD: AutoString,
        pub compDescr: AutoString,
        pub fakeName: AutoString,
        pub name: AutoString,
        pub team: i8,
        pub isAvatarReady: BOOL,
        pub isTeamKiller: BOOL,
        pub accountDBID: u64,
        pub clanAbbrev: AutoString,
        pub clanDBID: DB_ID,
        pub prebattleID: OBJECT_ID,
        pub isPrebattleCreator: BOOL,
        pub forbidInBattleInvitations: BOOL,
        pub igrType: i8,
        pub avatarSessionID: AutoString,
        pub overriddenBadge: i8,
        pub customRoleSlotTypeId: i8,
        pub botDisplayStatus: i8,
        pub teamPanelMode: u8,
        pub maxHealth: i16,
        pub prestigeLevel: i32,
        pub prestigeGradeMarkID: i32,
        pub vehPostProgression: Vec<i32>,
        pub personalMissionIDs: Vec<i32>,
        pub personalMissionInfo: Python,
        pub events: Python,
        pub badges: Python,
        pub ranked: Vec<u16>,
        pub deathInfo: DEATH_INFO,
        pub respawnID: u32,
        pub stFrags: u32,
        pub frags: i16,
        pub tkills: i16,
        pub fogOfWar: u8,
        pub position: Box<[i16; 2]>,
        pub __generation: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PVE_MINIMAP_DATA {
        pub minimapBorders: Box<[Vec2; 2]>,
        pub zoomLevel: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PVE_TARGET_MARKER {
        pub settingId: i16,
        pub targetId: i32,
        pub visibleStyle: AutoString,
        pub invisibleStyle: AutoString,
        pub lastVisiblePosition: Vec3,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMP7_PLAYER_STATS {
        pub damageDone: i16,
        pub damageBlocked: i16,
        pub damageAssisted: i16,
        pub spottedCount: i16,
        pub shotCount: i16,
        pub hitCount: i16,
        pub killCount: i16,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMP7_EQUIPMENT {
        pub name: AutoString,
        pub compactDescr: i32,
        pub stage: u8,
        pub endTime: f64,
        pub totalTime: f64,
        pub level: u8,
        pub progress: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GUN_RELOAD_INFO {
        pub activeGun: u8,
        pub clipSize: u16,
        pub clipLeft: u16,
        pub reloadEndTime: f64,
        pub reloadTotalTime: f64,
        pub switchEndTime: f64,
        pub switchTotalTime: f64,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMP7_VEHICLE_INFO {
        pub vehicleID: OBJECT_ID,
        pub accountDBID: u64,
        pub name: AutoString,
        pub fakeName: AutoString,
        pub team: i8,
        pub clanDBID: DB_ID,
        pub clanAbbrev: AutoString,
        pub outfitCD: AutoString,
        pub compDescr: AutoString,
        pub maxHealth: i16,
        pub curHealth: i16,
        pub isAlive: BOOL,
        pub deathInfo: DEATH_INFO,
        pub gunReloadInfo: GUN_RELOAD_INFO,
        pub criticalDevices: Vec<AutoString>,
        pub destroyedDevices: Vec<AutoString>,
        pub injuredTankmen: Vec<AutoString>,
        pub vehicleAmmoList: Vec<VEHICLE_AMMO>,
        pub consumableEquipment: Vec<COMP7_EQUIPMENT>,
        pub deviceEquipment: Vec<COMP7_EQUIPMENT>,
        pub boosterEquipment: Vec<COMP7_EQUIPMENT>,
        pub poiEquipment: Vec<COMP7_EQUIPMENT>,
        pub roleEquipment: COMP7_EQUIPMENT,
        pub isInspired: BOOL,
        pub isHealing: BOOL,
        pub isStunned: BOOL,
        pub isOnFire: BOOL,
        pub isVisible: BOOL,
        pub isAmmoBayDestroyed: BOOL,
        pub attackerSetOnFire: OBJECT_ID,
        pub attackerAmmoBayDestroyed: OBJECT_ID,
        pub playerStats: COMP7_PLAYER_STATS,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMP7_POI_INFO {
        pub id: OBJECT_ID,
        pub status: STATUS_WITH_TIME_INTERVAL,
        pub r#type: u8,
        pub invader: OBJECT_ID,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct INVADER_CAPTURE_POINTS {
        pub vehicleID: OBJECT_ID,
        pub capturePoints: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMP7_BASE_INFO {
        pub id: OBJECT_ID,
        pub baseID: u8,
        pub teamID: i8,
        pub status: i8,
        pub capturePoints: i32,
        pub captureTimeLeft: i32,
        pub invaderCapturePoints: Vec<INVADER_CAPTURE_POINTS>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct HEALTH_INFO {
        pub vehicleID: OBJECT_ID,
        pub health: u16,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct IMPACT_AREA_INFO {
        pub timeBeforeDamage: f32,
        pub isVisible: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LIVE_TAG_INFO {
        pub tag: u8,
        pub vehicles: Vec<OBJECT_ID>,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMMENDATIONS_STATE {
        pub vehID: OBJECT_ID,
        pub messageState: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct COMMENDATIONS_BATTLE_STATISTICS {
        pub vehID: OBJECT_ID,
        pub commendationsReceived: i8,
        pub commendationsSent: i8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct FALL_TANKS_EVACUATION_INFO {
        pub endTime: f32,
        pub baseTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GAME_OBJECT_STATE_INFO {
        pub prefabPath: AutoString,
        pub recreateMethod: u8,
        pub networkID: u32,
        pub parentID: u32,
        pub active: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct VEHICLE_HIERARCHY_INFO {
        pub slotName: AutoString,
        pub networkID: u32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct BATTLE_FURY_STATE {
        pub currentLevel: u8,
        pub maxLevel: u8,
        pub timeInterval: TIME_INTERVAL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CONCENTRATION_MODE_STATUS {
        pub state: u8,
        pub baseTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct WHEELED_DASH_STATUS {
        pub state: u8,
        pub baseTime: f32,
        pub endTime: f32,
        pub isReducedCooldown: BOOL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ACCURACY_STACKS_STATE {
        pub curLevel: u8,
        pub maxLevel: u8,
        pub timeElapsed: f32,
        pub timeNextGain: TIME_INTERVAL,
        pub gainMaxSpdKmh: f32,
        pub gainTime: f32,
        pub aimLevelBonus: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct SUPPORT_WEAPON_STATUS {
        pub state: u8,
        pub baseTime: f32,
        pub endTime: f32,
        pub gunInstallationIndex: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CLIENT_SHOT_RESULT_DATA {
        pub vehicleID: u32,
        pub hitFlags: u32,
        pub gunInstallationIndex: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PILLBOX_SIEGE_PUBLIC_STATUS {
        pub state: u8,
        pub nextState: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PILLBOX_SIEGE_STATUS {
        pub baseTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CHARGE_SHOT_PUBLIC_STATE {
        pub flags: u8,
        pub level: u8,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct CHARGE_SHOT_PRIVATE_STATE {
        pub flags: u16,
        pub level: u8,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LOW_CHARGE_SHOT_STATE {
        pub reloadingState: u8,
        pub baseTime: f32,
        pub timeLeft: f32,
        pub endTime: f32,
        pub lowChargeTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct LOW_CHARGE_SHOT_PUBLIC_STATE {
        pub visualState: u8,
        pub fullShotChangeTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct STANCE_DANCE_STATE {
        pub state: u8,
        pub energyFight: f32,
        pub energyTurbo: f32,
        pub timePassive: TIME_INTERVAL,
        pub timeSwitch: TIME_INTERVAL,
        pub timeActiveTurbo: TIME_INTERVAL,
        pub timeActiveFight: TIME_INTERVAL,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TARGET_DESIGNATOR_STATE {
        pub state: u8,
        pub startTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TARGET_DESIGNATOR_SPOTTED_MARKER {
        pub creatorID: OBJECT_ID,
        pub startTime: f32,
        pub endTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct TARGET_DESIGNATOR_UNSPOTTED_MARKER {
        pub creatorID: OBJECT_ID,
        pub targetID: OBJECT_ID,
        pub point: Vec4,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct GUN_ROTATOR_FIX_ANGLES {
        pub turretYaw: f32,
        pub gunPitch: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct STATIONARY_RELOAD_STATUS {
        pub state: u8,
        pub baseTime: f32,
        pub timeLeft: f32,
        pub gunLockMask: u8,
        pub sequenceEndTime: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct PROPELLANT_GUN_STATUS {
        pub state: u8,
        pub chargeStageID: u8,
        pub chargeProgress: f32,
        pub isOverchargeEnabled: BOOL,
        pub isSwitchCooldownActive: BOOL,
        pub updateTimestamp: f32,
        pub isForbiddenShell: BOOL,
        pub lastShotTimestamp: f32,
        pub lastShotCharge: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ANON155 {
        pub points: Vec<Vec3>,
        pub width: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ANON157 {
        pub position: Vec3,
        pub size: Vec3,
        pub rotation: Vec3,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ANON159 {
        pub position: Vec3,
        pub radius: Vec3,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ANON161 {
        pub position: Vec3,
        pub text: AutoString,
        pub color: Vec4,
        pub textSize: f32,
    }
}

wgtk::__struct_simple_codec! {
    #[derive(Debug)]
    pub struct ANON163 {
        pub name: AutoString,
        pub version: u32,
        pub destroyTime: f32,
        pub lines: Vec<ANON155>,
        pub boxes: Vec<ANON157>,
        pub spheres: Vec<ANON159>,
        pub texts: Vec<ANON161>,
    }
}
