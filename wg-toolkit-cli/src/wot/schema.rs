//! The members of WoT's entities that the emulator uses, bound against the loaded
//! script when it starts, see [`wgtk::app::entity`].

use glam::Vec3;

use wgtk::app::base;
use wgtk::app::entity::{BindError, Bytes, EntityId};
use wgtk::script::Value;
use wgtk::{entity_schema, script_dict};


/// Register every schema of this module with the given app.
pub fn register(app: &mut base::App) -> Result<(), BindError> {
    app.register::<Account>()?;
    app.register::<Avatar>()?;
    app.register::<Vehicle>()?;
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
        }
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
