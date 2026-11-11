//! The members of WoT's entities that the server uses, bound against the loaded
//! script when it starts, see [`wgtk::app::entity`].

use glam::Vec3;

use wgtk::{component_schema, entity_schema, script_dict};
use wgtk::app::entity::{BindError, Bytes, EntityId, Pickle};
use wgtk::script::Value;
use wgtk::app::base;


/// Register every schema of this module with the given app.
pub fn register(app: &mut base::App) -> Result<(), BindError> {
    app.register::<Account>()?;
    app.register::<Avatar>()?;
    app.register::<Vehicle>()?;
    app.register::<AvatarInfo>()?;
    app.register::<AreaDestructibles>()?;
    app.register_component::<AvatarVehiclesInfo>()?;
    app.register_component::<OwnVehicle>()?;
    app.register_component::<GunMarkerComponent>()?;
    app.register_component::<Fire>()?;
    Ok(())
}

entity_schema! {
    pub Account {
        client {
            fn showGUI(ctx: Pickle);
            fn onArenaCreated();
            fn onCmdResponse(request_id: i16, result_id: i16, error: String);
            fn onCmdResponseExt(request_id: i16, result_id: i16, error: String, ext: Pickle);
            fn update(diff: Pickle);
            fn receiveServerStats(stats: ServerStats);
            fn receiveQueueInfo(info: Pickle);
            fn onEnqueued(queue_type: u8);
            fn onDequeued(queue_type: u8);
            fn onTokenReceived(request_id: u16, token_type: u8, data: Pickle);
            fn onKickedFromServer(reason: String, kick_reason_type: u8, expiry_time: u32);
        }
        base {
            fn doCmdStr(request_id: i16, cmd: i16, arg: String);
            fn doCmdInt(request_id: i16, cmd: i16, arg: i64);
            fn doCmdInt2(request_id: i16, cmd: i16, arg0: i64, arg1: i64);
            fn doCmdInt3(request_id: i16, cmd: i16, arg0: i64, arg1: i64, arg2: i64);
            fn doCmdIntArr(request_id: i16, cmd: i16, args: Vec<i32>);
            fn doCmdIntArrStrArr(request_id: i16, cmd: i16, ints: Vec<i64>, strings: Vec<String>);
            fn requestToken(request_id: u16, token_type: u8);
            fn chatCommandFromClient(
                request_id: i64,
                action: u8,
                channel: EntityId,
                int_arg: i64,
                short_arg: i16,
                str_arg: String,
                str_arg2: String,
            );
        }
    }
}

script_dict! {
    /// How many players are online, on the player's cluster and the whole region.
    #[derive(Debug, Clone)]
    pub struct ServerStats {
        pub clusterCCU: u32,
        pub regionCCU: u32,
    }
}

entity_schema! {
    pub Avatar {
        client {
            fn onKickedFromServer(reason: String, kick_reason_type: u8, expiry_time: u32);
            fn updateArena(update_type: u8, data: Bytes);
            fn showTracer(
                shooter: EntityId,
                shot_id: i32,
                is_ricochet: u8,
                effects_index: u8,
                prefab_effects_index: u8,
                shell_kind: u8,
                caliber: f32,
                start: Vec3,
                velocity: Vec3,
                gravity: f32,
                max_distance: f32,
                gun_index: u8,
                unknown: u8,
            );
            fn stopTracer(shot_id: i32, end: Vec3);
            fn explodeProjectile(
                shot_id: i32,
                effects_index: u8,
                prefab_effects_index: u8,
                effect_material: u8,
                shell_kind: u8,
                caliber: f32,
                position: Vec3,
                // The shell's last step, along which the client projects the crater.
                step: Vec3,
                speed: f32,
                // What it broke, each `chunk id << 16 | index << 8 | material kind`, so
                // that the client breaks it as it explodes, see `unpackChunkObstacles`.
                damaged_destructibles: Vec<u32>,
            );
            fn showShotResults(results: Vec<ShotResult>);
        }
        base {
            fn setClientReady();
            fn leaveArena();
        }
        cell {
            fn vehicle_shoot();
            fn vehicle_moveWith(flags: u8);
            fn vehicle_stopTrackingWithGun(yaw: f32, pitch: f32);
            /// Change one of the vehicle's settings, by `VEHICLE_SETTING` code.
            fn vehicle_changeSetting(code: u8, value: i32);
            // What the client aims from, so it must turn with the hull, and the engine's
            // RPM.
            ownVehicleAuxPhysicsData: u64,
            ownVehicleGear: u8,
        }
    }
}

entity_schema! {
    pub Vehicle {
        client {
            fn showShooting(burst_count: u8, gun_index: i8, shell_kind: u8);
            fn showDamageFromShot(
                attacker: EntityId,
                points: Vec<HitPoint>,
                effects_index: u8,
                prefab_effects_index: u8,
                damage: i32,
                // The damage as a fraction of the maximum health, out of 255.
                damage_factor: u8,
                last_material_is_shield: u8,
                speed: f32,
                unknown: u8,
            );
            // An explosion beside the vehicle, its damage as `showDamageFromShot`'s.
            fn showDamageFromExplosion(attacker: EntityId, center: Vec3, effects_index: u8, damage: i32, damage_factor: u8, gun_index: u8);
            fn onHealthChanged(new_health: i16, old_health: i16, attacker: EntityId, attack_reason: u8, extended_reason: i8);
        }
        cell {
            fn trackRelativePointWithGun(point: Vec3);
            fn stopTrackingWithGun(yaw: f32, pitch: f32);
            health: i16,
            damageStickers: Vec<HitPoint>,
            gunAnglesPacked: u16,
            engineMode: [u8; 2],
            avatarID: EntityId,
            vehPostProgression: Vec<i32>,
            publicInfo: PublicVehicleInfo,
            isStrafing: bool,
            isCrewActive: bool,
            publicStateModifiers: Vec<u8>,
            dogTag: BattleDogTag,
            // Its `MY_VEHICLE` level of detail, which only its own client gets.
            isMyVehicle: bool,
            quickShellChangerFactor: f32,
            onRespawnReloadTimeFactor: f32,
            perkEffects: PerkEffects,
            perks: Vec<Value>,
            perksRibbonNotify: Vec<Value>,
            crewCompactDescrs: Vec<Bytes>,
            enhancements: Pickle,
            setups: Pickle,
            setupsIndexes: Pickle,
            vehPerks: Pickle,
            disabledSwitches: Vec<i32>,
            ownVehiclePosition: Option<OwnVehiclePosition>,
        }
    }
}

entity_schema! {
    /// The arena's info entity, which the arena's vehicles are told of with.
    pub AvatarInfo {
        cell {
            avatarID: EntityId,
        }
    }
}

entity_schema! {
    /// A map chunk's destructibles, those destroyed in it, each encoded as the client's
    /// `DestructiblesCache.py` does: its index in the chunk, high byte first, then what
    /// its kind tells, see the client's `AreaDestructibles.py`.
    pub AreaDestructibles {
        cell {
            destroyedModules: Vec<[u8; 3]>,
            destroyedFragiles: Vec<[u8; 3]>,
            fallenColumns: Vec<[u8; 3]>,
            fallenTrees: Vec<[u8; 5]>,
        }
    }
}

component_schema! {
    /// The arena's vehicles, on its info entity.
    pub AvatarVehiclesInfo {
        cell {
            vehiclesInfo: Vec<VehicleInfo>,
        }
    }
}

component_schema! {
    /// What the client knows of its own vehicle, on that vehicle.
    pub OwnVehicle {
        cell {
            syncVehicleAttrs: VehicleSyncAttrs,
            /// How its turret and gun aim.
            targetingInfo: Option<TargetingInfo>,
            vehicleAmmoList: Vec<VehicleAmmo>,
            /// The shell loaded, then the next one.
            vehicleSettings: Vec<VehicleSetting>,
            vehicleGunReloadTime: GunReloadTime,
            vehicleHealthInfo: VehicleHealthInfo,
        }
    }
}

component_schema! {
    /// The server's reticle of the client's own vehicle, on that vehicle.
    pub GunMarkerComponent {
        cell {
            gunMarker: Option<GunMarker>,
        }
    }
}

component_schema! {
    /// A fire on a vehicle, burning while the vehicle has it.
    pub Fire {
        cell {
            /// A `FIRE_INFO` dict, or none as captured.
            fireInfo: Option<FireInfo>,
        }
    }
}

script_dict! {
    /// `PUBLIC_VEHICLE_INFO`: what every client knows of a vehicle.
    #[derive(Debug, Clone)]
    pub struct PublicVehicleInfo {
        pub name: String,
        pub compDescr: Bytes,
        pub outfit: Bytes,
        pub outfitLevel: u8,
        pub index: u16,
        pub team: u8,
        pub prebattleID: EntityId,
        pub marksOnGun: u8,
        pub crewGroups: Vec<u16>,
        pub commanderSkinID: u16,
        pub maxHealth: u16,
        pub respawnID: u32,
        pub stFrags: u32,
    }
}

script_dict! {
    /// `DEATH_INFO`: who destroyed a vehicle, and how.
    #[derive(Debug, Clone)]
    pub struct DeathInfo {
        pub victimID: EntityId,
        pub killerID: EntityId,
        /// The `ATTACK_REASON` index.
        pub reasonID: i8,
        pub equipmentID: i16,
        pub numVehiclesAffected: i16,
    }
}

script_dict! {
    /// `FIRE_INFO`: what set a vehicle on fire, and when.
    #[derive(Debug, Clone)]
    pub struct FireInfo {
        /// The vehicle's fire, among its extras.
        pub deviceExtraIndex: u8,
        pub notificationIndex: u8,
        pub attackerID: EntityId,
        pub equipmentID: u16,
        pub startTime: f64,
    }
}

script_dict! {
    /// `VEHICLE_SYNC_ATTRS`: the vehicle's view range, and its shells' piercing and
    /// speed factors.
    #[derive(Debug, Clone)]
    pub struct VehicleSyncAttrs {
        pub circularVisionRadius: u16,
        pub gunPiercing: f32,
        pub gunShotsSpeed: f32,
    }
}

script_dict! {
    /// `TARGETING_INFO`: where the turret and gun point, how fast they turn, and how
    /// the gun's dispersion grows and shrinks.
    #[derive(Debug, Clone)]
    pub struct TargetingInfo {
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

script_dict! {
    /// `VEHICLES_INFO`: a vehicle of the arena, as every client lists it.
    #[derive(Debug, Clone)]
    pub struct VehicleInfo {
        pub vehicleID: EntityId,
        pub isAlive: bool,
        pub outfitCD: Bytes,
        pub compDescr: Bytes,
        pub fakeName: String,
        pub name: String,
        pub team: i8,
        pub isAvatarReady: bool,
        pub isTeamKiller: bool,
        pub accountDBID: u64,
        pub clanAbbrev: String,
        pub clanDBID: i64,
        pub prebattleID: EntityId,
        pub isPrebattleCreator: bool,
        pub forbidInBattleInvitations: bool,
        pub igrType: i8,
        pub avatarSessionID: String,
        pub overriddenBadge: i8,
        pub customRoleSlotTypeId: i8,
        pub botDisplayStatus: i8,
        pub teamPanelMode: u8,
        pub maxHealth: i16,
        pub prestigeLevel: i32,
        pub prestigeGradeMarkID: i32,
        pub vehPostProgression: Vec<i32>,
        pub personalMissionIDs: Vec<i32>,
        pub personalMissionInfo: Pickle,
        pub events: Pickle,
        pub badges: Pickle,
        pub ranked: Vec<u16>,
        pub deathInfo: Option<DeathInfo>,
        pub respawnID: u32,
        pub stFrags: u32,
        pub frags: i16,
        pub activePetID: i32,
        pub tkills: i16,
        pub fogOfWar: u8,
        pub position: [i16; 2],
        pub __generation: u32,
    }
}

script_dict! {
    /// `BATTLE_DOG_TAG`: the dog tag a vehicle's player shows, and its default one.
    #[derive(Debug, Clone)]
    pub struct BattleDogTag {
        pub dogTag: DogTag,
        pub defaultDogTag: DogTag,
        pub showDogTagToKiller: bool,
    }
}

script_dict! {
    /// `DOG_TAG`: a dog tag, as its components.
    #[derive(Debug, Clone)]
    pub struct DogTag {
        pub components: Vec<DogTagComponent>,
    }
}

script_dict! {
    /// `DOG_TAG_COMPONENT`: a dog tag's background, engraving or badge.
    #[derive(Debug, Clone)]
    pub struct DogTagComponent {
        pub id: i32,
        pub progress: f32,
        pub grade: i8,
    }
}

script_dict! {
    /// `PERK_EFFECTS`: the equipment the vehicle's perks affect.
    #[derive(Debug, Clone, Default)]
    pub struct PerkEffects {
        pub equipment: Vec<Bytes>,
    }
}

script_dict! {
    /// `OWN_VEHICLE_POSITION`: where the server last placed the vehicle, for its client.
    #[derive(Debug, Clone)]
    pub struct OwnVehiclePosition {
        pub position: Vec3,
        pub direction: Vec3,
        pub speed: f32,
        pub rotationSpeed: f32,
    }
}

script_dict! {
    /// `VEHICLE_HIT_POINT`: where a shell hit a vehicle.
    #[derive(Debug, Clone)]
    pub struct HitPoint {
        pub networkID: u32,
        /// Encoded over the hit part's bounds, see `VehicleHit::encode_segment`.
        pub segment: u64,
        pub params: u32,
    }
}

script_dict! {
    /// What a shell did to a vehicle, as its shooter sees it.
    #[derive(Debug, Clone)]
    pub struct ShotResult {
        pub gunInstallationIndex: u8,
        pub hitFlags: u32,
        pub vehicleID: EntityId,
    }
}

script_dict! {
    /// A shell or a consumable of a vehicle.
    #[derive(Debug, Clone)]
    pub struct VehicleAmmo {
        pub compactDescr: i32,
        pub endTime: f32,
        pub index: u8,
        pub previousStage: u16,
        pub quantity: u16,
        pub quantityInClip: u16,
        pub totalTime: i16,
    }
}

script_dict! {
    /// `VEHICLE_SETTING`: one of a vehicle's settings, by code, the shell loaded say.
    #[derive(Debug, Clone)]
    pub struct VehicleSetting {
        pub code: u8,
        pub value: i32,
        pub vehicleID: EntityId,
    }
}

script_dict! {
    /// How long a gun takes to reload, and when it is loaded: both 0 once it is, -1
    /// before the battle.
    #[derive(Debug, Clone)]
    pub struct GunReloadTime {
        pub baseTime: f32,
        pub clipTime: f32,
        pub endTime: f32,
        pub timeLeft: f32,
    }
}

script_dict! {
    #[derive(Debug, Clone)]
    pub struct VehicleHealthInfo {
        pub deathReasonExtID: i8,
        pub deathReasonID: i8,
        pub health: i16,
        pub id: EntityId,
        pub isCrewActive: bool,
        pub isRespawnActive: bool,
    }
}

script_dict! {
    /// Where the gun fires from, the shell's velocity and the dispersion angle.
    #[derive(Debug, Clone)]
    pub struct GunMarker {
        pub gunPosition: Vec3,
        pub shotVector: Vec3,
        pub dispersion: f32,
    }
}

script_dict! {
    /// The `Account`'s base data, which its client gets as it is created: its player,
    /// the client version it requires and the server's settings.
    #[derive(Debug, Clone)]
    pub struct AccountData {
        pub name: String,
        /// How many sessions the player had, the client telling new players by it.
        pub incarnationID: u64,
        /// The client version the server requires, refusing others.
        pub requiredVersion_2410: String,
        /// A dict of the server's settings, of every feature the client reads at login.
        pub initialServerSettings: Pickle,
    }
}

script_dict! {
    /// The `Avatar`'s base data, which its client gets as it is created: its battle,
    /// what it may do in it, and its player.
    #[derive(Debug, Clone)]
    pub struct AvatarData {
        pub name: String,
        /// Unused by the client.
        pub sessionID: String,
        pub arenaUniqueID: u64,
        /// The map and gameplay, an index in `arena_defs/_list_.xml`.
        pub arenaTypeID: i32,
        /// The `ARENA_BONUS_TYPE`.
        pub arenaBonusType: u8,
        /// The `ARENA_GUI_TYPE`.
        pub arenaGuiType: u8,
        /// A dict the client reads with defaults: `battleLevel`, `bonusCapsOverrides`,
        /// `battleModifiersDescr`, `postmortemSettings`...
        pub arenaExtraData: Pickle,
        pub weatherPresetID: u8,
        /// How many players it may still report.
        pub denunciationsLeft: i16,
        pub clientCtx: String,
        pub tkillIsSuspected: bool,
        pub playLimits: PlayLimits,
        pub battleChatRestriction: BattleChatRestriction,
        /// `BATTLE_GOODIES_LIST`, its active boosters.
        pub goodiesSnapshot: Vec<Value>,
    }
}

script_dict! {
    /// `PLAY_LIMITS`: the seconds left to play, -1 for none.
    #[derive(Debug, Clone)]
    pub struct PlayLimits {
        pub curfew: i32,
        pub weeklyPlayLimit: i32,
        pub dailyPlayLimit: i32,
        pub sessionLimit: i32,
    }
}

script_dict! {
    /// `BATTLE_CHAT_RESTRICTION`.
    #[derive(Debug, Clone)]
    pub struct BattleChatRestriction {
        pub isBattleChatDisabled: bool,
        pub restrictionReasonID: i8,
    }
}

script_dict! {
    /// The `Avatar`'s cell data, which its client gets with its cell part: its team and
    /// vehicle, and its vehicle's state that the client predicts.
    #[derive(Debug, Clone)]
    pub struct AvatarCellData {
        pub team: u8,
        pub playerVehicleID: EntityId,
        pub isGunLocked: bool,
        pub ownVehicleGear: u8,
        pub ownVehicleAuxPhysicsData: u64,
        pub ownVehicleHullAimingPitchPacked: u16,
        pub ammoViews: AmmoViews,
        pub customizationDisplayType: u8,
        pub isObserverBothTeams: bool,
        pub isObserverFPV: bool,
        pub numOfObservers: u8,
        pub observableTeamID: u8,
        pub remoteCamera: RemoteCamera,
        pub shouldSendKillcamSimulationData: bool,
    }
}

script_dict! {
    /// `AVATAR_AMMO_VIEWS`.
    #[derive(Debug, Clone, Default)]
    pub struct AmmoViews {
        pub vehTypeCompDescrs: Vec<i32>,
        pub compDescrs: Vec<Vec<i32>>,
    }
}

script_dict! {
    /// `REMOTE_CAMERA_DATA`: where an observed player aims.
    #[derive(Debug, Clone, Default)]
    pub struct RemoteCamera {
        pub time: f64,
        pub shotPoint: Vec3,
        pub zoom: u8,
        pub mode: u8,
    }
}

script_dict! {
    /// The cell data of the `Avatar`'s static components, as one dict: the in-battle
    /// vehicle switch's spawns, and an application point check, all unused in a random
    /// battle.
    #[derive(Debug, Clone, Default)]
    pub struct AvatarComponentsData {
        pub isPositionValid: bool,
        pub wrongApplicationPoint: Vec3,
        pub isVehicleConfirmed: bool,
        /// `RESPAWN_INFO_VEHICLE`, none.
        pub spawnInfoForVehicle: Option<Value>,
        /// `SPAWN_POINT_INFO`s.
        pub spawnPoints: Vec<Value>,
        /// `SPAWN_AVAILABLE_VEHICLE`s.
        pub vehicleSpawnList: Vec<Value>,
    }
}
