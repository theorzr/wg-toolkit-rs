//! The members of WoT's entities that the emulator uses, bound against the loaded
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
    app.register_component::<AvatarVehiclesInfo>()?;
    app.register_component::<OwnVehicle>()?;
    app.register_component::<GunMarkerComponent>()?;
    app.register_component::<Fire>()?;
    Ok(())
}

entity_schema! {
    pub Account {
        client {
            fn onArenaCreated();
        }
    }
}

entity_schema! {
    pub Avatar {
        client {
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
                damaged_destructibles: Vec<Value>,
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
    pub AvatarInfo {}
}

component_schema! {
    /// The arena's vehicles, on its info entity.
    pub AvatarVehiclesInfo {
        cell {
            vehiclesInfo: Vec<Value>,
        }
    }
}

component_schema! {
    /// What the client knows of its own vehicle, on that vehicle.
    pub OwnVehicle {
        cell {
            syncVehicleAttrs: Value,
            /// How its turret and gun aim.
            targetingInfo: Value,
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
            fireInfo: Value,
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
