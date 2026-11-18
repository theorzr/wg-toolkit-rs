//! Vehicle definitions, read from the game's `scripts/item_defs/vehicles`: per nation,
//! `list.xml` gives each vehicle's type id, `<vehicle>.xml` its modules by name, and
//! `components/<kind>.xml` the modules' ids (in an `ids` section) and the shared
//! modules' definitions (in a `shared` section), guns with their shells. `components/shells.xml` gives each
//! shell's id.

use std::collections::HashMap;
use std::sync::Arc;
use std::io;

use glam::{Affine3A, Vec2, Vec3};

use tracing::warn;

use wgtk::pxml::{self, Element};
use wgtk::res::fs::ResFilesystem;

use super::collision::{ArmorMaterial, CollisionModel, VehicleCollision};
use super::items::{item_type, IntCompactDescr, VehicleDescr};


/// `HP_TO_WATTS`: an engine's power is given in horsepower.
const HP_TO_WATTS: f32 = 735.5;

/// The nations, `nations.NAMES` order: a nation's id is its index.
pub const NATIONS: &[&str] = &[
    "ussr", "germany", "usa", "china", "france", "uk", "japan", "czech", "sweden", "poland", "italy",
];

/// The tankman roles, their index in `SKILL_NAMES`.
pub const ROLES: &[(&str, u8)] = &[
    ("commander", 1), ("radioman", 2), ("driver", 3), ("gunner", 4), ("loader", 5),
];

/// `DEFAULT_PIERCING_POWER_RANDOMIZATION`.
const DEFAULT_PEN_RANDOMIZATION: f32 = 0.25;

/// `SHELL_TYPES_LIST`: a shell kind's index is its position.
const SHELL_KINDS: &[&str] = &[
    "HOLLOW_CHARGE", "HIGH_EXPLOSIVE", "ARMOR_PIERCING", "ARMOR_PIERCING_HE", "ARMOR_PIERCING_CR", "SMOKE",
];

/// The most road wheels a chassis has, both sides, and wheels at its tracks' ends.
pub const MAX_ROAD_WHEELS: usize = 32;
pub const MAX_TRACK_ENDS: usize = 4;

/// The most wheels a chassis' general wheels have.
pub const MAX_GENERAL_WHEELS: usize = 16;

/// `PIERCING_POWER_INTERPOLATION_DIST_FIRST` and `_LAST`, the same as for damage.
const INTERPOLATION_DISTANCES: [f32; 2] = [50.0, 500.0];

/// How far above the lowest road wheel's bottom a wheel's bottom may be, in metres, for
/// it to be a road wheel: higher, it is a return roller, on top of its track, a metre
/// up. Our choice.
const ROAD_WHEEL_RISE: f32 = 0.25;

/// How far apart along a chassis two of its general wheels may be, in metres, to be on
/// the same axle. Our choice.
const AXLE_SPREAD: f32 = 0.1;

/// How long a vehicle takes to switch to its siege mode, or back, in seconds, when its
/// `siege_mode` section doesn't say: `_readSiegeModeParams`' default.
const SIEGE_SWITCH_TIME: f32 = 2.0;


/// What it takes to give a vehicle to an account: its descriptor with the default
/// modules, its crew and its default ammunition.
#[derive(Debug, Clone)]
pub struct VehicleDef {
    pub name: String,
    pub descr: VehicleDescr,
    /// Each crew member's main role, as a role index, in the vehicle's crew order.
    pub crew_roles: Vec<u8>,
    /// The default gun's shells, by integer descriptor, with their default share of
    /// the ammunition.
    pub shells: Vec<(IntCompactDescr, f32)>,
    /// How the default gun fires each of its shells, in the same order.
    pub shots: Vec<ShotDef>,
    pub max_ammo: u32,
    /// Hull plus default turret, before any battle modifier.
    pub max_health: u32,
    pub mobility: MobilityDef,
    pub aiming: AimingDef,
    pub magazine: MagazineDef,
    pub vision: VisionDef,
    /// The default modules' collision models.
    pub collision: Arc<VehicleCollision>,
    /// Its siege mode, if it has one: a wheeled vehicle's fast one.
    pub siege: Option<SiegeDef>,
}

/// A vehicle's siege mode, which vehicles tagged `siegeMode` switch to and back: how it
/// moves in it, from its own definition, `<name>_siege_mode.xml`, and how long it takes
/// to switch to it and back, in seconds, `siege_mode/switchOnTime` and `switchOffTime`.
/// A wheeled vehicle's is its fast one: faster, steering less.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SiegeDef {
    pub mobility: MobilityDef,
    pub switch_on: f32,
    pub switch_off: f32,
}

/// How a vehicle moves with its default modules: the values the game derives its
/// physics from, see `VehicleDescriptor.physics`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobilityDef {
    /// Top speeds forward and backward, in m/s.
    pub speed_forward: f32,
    pub speed_backward: f32,
    /// How fast the vehicle turns on the spot on firm, medium and soft ground, in rad/s:
    /// its chassis' `rotationSpeed` on firm ground, slower on the others as their
    /// resistance is higher, `rotationSpeedLimit` in `vehicles.py`, which its engine's
    /// power over its weight would also scale, both the default ones here. Measured on
    /// the real server within 1% on all three grounds.
    pub rotation_speed: [f32; 3],
    /// The engine's power, in W: the server's physics' own, `smplEnginePower` in the
    /// vehicle's `physics/detailed` section, else the engine's.
    pub engine_power: f32,
    /// The vehicle's weight, in kg: its hull's, chassis', turret's, gun's, engine's,
    /// fuel tank's and radio's.
    pub weight: f32,
    /// The chassis' rolling friction on firm, medium and soft ground, from the same
    /// physics section: the share of its weight the ground holds it back with.
    pub rolling_friction: [f32; 3],
    /// Where its front right track rests on the ground, from its centre, in metres: across
    /// to the track's centre, and along. Its chassis' `topRightCarryingPoint`, which its
    /// `carryingTriangles` rest on.
    pub carrying_point: Vec2,
    /// The box of what collides of it, its chassis and its hull, in the vehicle's frame
    /// (x right, y up, z forward): minimum then maximum, from their collision models.
    pub bounds: [Vec3; 2],
    /// Its turret's box, from its collision model, in the vehicle's frame as it faces
    /// ahead: what else of it hits the ground when it rolls over.
    pub turret_bounds: [Vec3; 2],
    /// Its chassis' wheels, where its model places them, if it does.
    pub wheels: Option<Wheels>,
    /// Its chassis' wheels that the client steers and rolls by what the server tells,
    /// a wheeled vehicle's, if it has any.
    pub general_wheels: Option<GeneralWheels>,
}

/// A wheel of a chassis: its centre, in the vehicle's frame, and its radius, in metres.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WheelDef {
    pub center: Vec3,
    pub radius: f32,
}

/// A chassis' wheels, as its definition names them (`chassis/wheels`) and its model's
/// nodes place them (`<chassis>.visual_processed`): those its tracks rest on, the road
/// wheels, and those its tracks turn around at their ends, the leading ones, its idlers
/// and sprockets, `isLeading`. The return rollers, on top of its tracks, aren't kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wheels {
    road: [WheelDef; MAX_ROAD_WHEELS],
    road_count: u8,
    ends: [WheelDef; MAX_TRACK_ENDS],
    end_count: u8,
    /// How thick its tracks are, in metres: how far above the ground its road wheels'
    /// bottoms are, on average, its origin on the ground.
    pub track_thickness: f32,
}

impl Wheels {

    /// Wheels of these road wheels and track ends, as many as fit, see
    /// [`MAX_ROAD_WHEELS`] and [`MAX_TRACK_ENDS`].
    pub fn new(road: &[WheelDef], ends: &[WheelDef], track_thickness: f32) -> Self {
        let road = &road[..road.len().min(MAX_ROAD_WHEELS)];
        let ends = &ends[..ends.len().min(MAX_TRACK_ENDS)];
        let mut wheels = Self {
            road: [WheelDef::default(); MAX_ROAD_WHEELS],
            road_count: road.len() as u8,
            ends: [WheelDef::default(); MAX_TRACK_ENDS],
            end_count: ends.len() as u8,
            track_thickness,
        };
        wheels.road[..road.len()].copy_from_slice(road);
        wheels.ends[..ends.len()].copy_from_slice(ends);
        wheels
    }

    /// Its road wheels.
    pub fn road(&self) -> &[WheelDef] {
        &self.road[..self.road_count as usize]
    }

    /// The wheels at its tracks' ends, the frontmost and the rearmost of each side.
    pub fn ends(&self) -> &[WheelDef] {
        &self.ends[..self.end_count as usize]
    }

}

/// A wheel of a chassis' general wheels, see [`GeneralWheels`]: its centre, in the
/// vehicle's frame, its radius, in metres, and whether it steers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GeneralWheel {
    pub center: Vec3,
    pub radius: f32,
    pub steers: bool,
    /// How far its axle steers it at most, in radians, positive to the right, the
    /// vehicle turning right, the inner wheel's: turning with no way asked, its axle's
    /// `axleSteeringLockAngles`, and driving, its `axleSteeringAngles`. Negative for an
    /// axle steering against the front's, none for one that doesn't steer.
    pub lock: f32,
    pub on_move: f32,
}

impl GeneralWheel {

    /// The radius of the circle it turns the vehicle on at its lock, the centre `level`
    /// metres ahead, in metres, see [`GeneralWheels`].
    fn radius_at_lock(&self, level: f32) -> f32 {
        (self.center.z - level).abs() / self.lock.abs().tan() + self.center.x.abs()
    }

}

/// A chassis' general wheels, as its definition names them (`chassis/wheels`, with
/// `generalWheels`): those not under a track, `nonTrack`, by their `index`, which the
/// client turns and rolls by its vehicle's `steeringAngles`, one for each that steers,
/// `steeringProcessor`, and `wheelsScroll`, one for each. How far their axles steer
/// comes from its physics section, and whether it turns on the spot, as tracks do,
/// `isWheeledOnSpotRotation`.
///
/// They steer as a car's: each pointing where it goes as the vehicle turns about a
/// centre level with its axles that don't steer, between them, else with its middle,
/// those on the inside of the turn more than those outside.
///
/// ```text
///   seen from above, front up, turning right about the centre ·, R from the middle:
///
///     / /     front axle, z ahead of the centre's level
///     | |  ·  axles that don't steer, level with it
///     \ \     rear axle, steering against the front
///
///   a wheel at (x, z) points at atan((z − level) · c / (1 − x · c)), c = 1 / R, the
///   curvature, signed as the turn: the inner one, nearer the centre, the most
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneralWheels {
    wheels: [GeneralWheel; MAX_GENERAL_WHEELS],
    count: u8,
    pub turns_on_spot: bool,
}

impl GeneralWheels {

    /// General wheels of these, by their index, as many as fit, see
    /// [`MAX_GENERAL_WHEELS`].
    pub fn new(wheels: &[GeneralWheel], turns_on_spot: bool) -> Self {
        let wheels = &wheels[..wheels.len().min(MAX_GENERAL_WHEELS)];
        let mut general = Self {
            wheels: [GeneralWheel::default(); MAX_GENERAL_WHEELS],
            count: wheels.len() as u8,
            turns_on_spot,
        };
        general.wheels[..wheels.len()].copy_from_slice(wheels);
        general
    }

    /// The tightest they turn the vehicle, its curvature, in rad/s for each m/s it
    /// goes, on their locks, else as they steer driving, see [`Self::curvature`].
    pub fn turning(&self, locked: bool) -> f32 {
        self.curvature(self.steering(locked))
    }

    /// How far their keys steer them, on their locks, else driving, by the angle of the
    /// wheel that turns the vehicle the tightest, which [`Self::curvature`] takes, in
    /// radians.
    pub fn steering(&self, locked: bool) -> f32 {
        self.tightest().map_or(0.0, |wheel| if locked { wheel.lock.abs() } else { wheel.on_move.abs() })
    }

    /// The vehicle's curvature, in rad/s for each m/s it goes, positive to the right,
    /// its tightest turning wheel pointing at this angle, positive to the right: one
    /// over the radius of the circle it puts it on, see [`GeneralWheels`].
    ///
    /// ```text
    ///   R = |z − level| / tan |δ| + |x|
    /// ```
    pub fn curvature(&self, angle: f32) -> f32 {
        let Some(wheel) = self.tightest().filter(|_| angle != 0.0) else { return 0.0 };
        angle.signum() / ((wheel.center.z - self.level()).abs() / angle.abs().tan() + wheel.center.x.abs())
    }

    /// How far its tightest turning wheel steers for the vehicle to turn with this
    /// curvature, in rad/s for each m/s, positive to the right, see [`Self::curvature`],
    /// in radians, no further than its lock.
    #[cfg(test)]
    pub fn steering_for(&self, curvature: f32) -> f32 {
        let Some(wheel) = self.tightest().filter(|_| curvature != 0.0) else { return 0.0 };
        let radius = (1.0 / curvature.abs() - wheel.center.x.abs()).max(1e-3);
        curvature.signum() * ((wheel.center.z - self.level()).abs() / radius).atan().min(wheel.lock.abs())
    }

    /// Where a wheel points, in radians, positive to the right, the vehicle turning
    /// with this curvature, in rad/s for each m/s, positive to the right, see
    /// [`GeneralWheels`]. Without steering, straight ahead.
    pub fn angle(&self, wheel: &GeneralWheel, curvature: f32) -> f32 {
        if !wheel.steers {
            return 0.0;
        }
        ((wheel.center.z - self.level()) * curvature).atan2(1.0 - wheel.center.x * curvature)
    }

    /// The wheel that turns the vehicle the tightest at its lock, the inner one of its
    /// axles that steer, if any steers.
    fn tightest(&self) -> Option<&GeneralWheel> {
        let level = self.level();
        self.all().iter()
            .filter(|wheel| wheel.steers && wheel.lock != 0.0)
            .min_by(|a, b| a.radius_at_lock(level).total_cmp(&b.radius_at_lock(level)))
    }

    /// How far ahead the centre they turn about is level with, in metres: between its
    /// axles that don't steer, else its middle.
    fn level(&self) -> f32 {
        let (mut sum, mut count) = (0.0, 0);
        for wheel in self.all().iter().filter(|wheel| wheel.lock == 0.0 && wheel.on_move == 0.0) {
            sum += wheel.center.z;
            count += 1;
        }
        if count == 0 { 0.0 } else { sum / count as f32 }
    }

    /// Its wheels, by their index.
    pub fn all(&self) -> &[GeneralWheel] {
        &self.wheels[..self.count as usize]
    }

}

/// A shell as the default gun fires it: what the client needs to show its tracer.
#[derive(Debug, Clone)]
pub struct ShotDef {
    pub compact_descr: IntCompactDescr,
    /// Its kind's index in `SHELL_TYPES_LIST`.
    pub kind_index: u8,
    /// In millimetres.
    pub caliber: f32,
    /// Its effects' index in `common/shot_effects.xml`.
    pub effects_index: u8,
    /// The damage sticker a pierce leaves, as its effects give it: its index in
    /// `common/damage_stickers.xml`.
    pub pierced_sticker: Option<u8>,
    /// Its prefab effects' index in `common/prefab_effects/shot_effects.xml`: its own
    /// `prefabEffects`, else its kind's default in `common/vehicle.xml`.
    pub prefab_effects_index: Option<u8>,
    /// In m/s and m/s², scaled by `projectileSpeedFactor` as the client's item loader
    /// does.
    pub speed: f32,
    pub gravity: f32,
    /// In metres.
    pub max_distance: f32,
    /// The damage it deals to a vehicle it pierces, on average, within 50 m.
    pub damage_near: f32,
    /// The same at 500 m and beyond, see [`Self::damage_at`].
    pub damage_far: f32,
    /// The armour it pierces within 50 m, in millimetres, as the gun fires it.
    pub pen_near: f32,
    /// The same at 500 m and beyond, see [`Self::pen_at`].
    pub pen_far: f32,
    /// How much its penetration varies, either way, as a share of it.
    pub pen_randomization: f32,
    /// How much it turns toward an armour's normal as it hits it, in radians: AP kinds
    /// only.
    pub normalization_angle: f32,
    /// The cosine of the angle from an armour's normal past which it ricochets off it:
    /// none for the kinds that never do.
    pub ricochet_angle_cos: Option<f32>,
    /// The share of its penetration a HEAT jet loses per metre once through a screen.
    pub jet_pen_loss: f32,
    /// How far an HE shell's explosion damages, in metres, as the legacy HE mechanics
    /// had it: `caliber² / 5555`.
    pub explosion_radius: f32,
}

/// `SHELL_TYPES`, by their index in `SHELL_TYPES_LIST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellKind {
    HollowCharge = 0,
    HighExplosive = 1,
    ArmorPiercing = 2,
    ArmorPiercingHe = 3,
    ArmorPiercingCr = 4,
    Smoke = 5,
}

impl ShotDef {

    pub fn kind(&self) -> ShellKind {
        match self.kind_index {
            0 => ShellKind::HollowCharge,
            1 => ShellKind::HighExplosive,
            2 => ShellKind::ArmorPiercing,
            3 => ShellKind::ArmorPiercingHe,
            4 => ShellKind::ArmorPiercingCr,
            _ => ShellKind::Smoke,
        }
    }

    /// Its penetration after flying this many metres: linear from 50 m to 500 m, as
    /// `computePiercingPowerAtDist` interpolates it.
    pub fn pen_at(&self, distance: f32) -> f32 {
        interpolate_by_distance([self.pen_near, self.pen_far], distance)
    }

    /// Its average damage after flying this many metres, as `computeDamageAtDist`
    /// interpolates it.
    pub fn damage_at(&self, distance: f32) -> f32 {
        interpolate_by_distance([self.damage_near, self.damage_far], distance)
    }

}

/// A value at a distance between its values at [`INTERPOLATION_DISTANCES`], held before
/// the first, carried on past the last, never below zero.
fn interpolate_by_distance([near, far]: [f32; 2], distance: f32) -> f32 {
    let [first, last] = INTERPOLATION_DISTANCES;
    let t = (distance.max(first) - first) / (last - first);
    (near + (far - near) * t).max(0.0)
}

/// How the default turret and gun aim, in the units the client's own item loader
/// converts them to: speeds in rad/s, dispersion factors per m/s and per rad/s.
#[derive(Debug, Clone)]
pub struct AimingDef {
    pub turret_rotation_speed: f32,
    pub gun_rotation_speed: f32,
    /// Seconds for the dispersion to shrink by e.
    pub aiming_time: f32,
    pub dispersion_turret_rotation: f32,
    pub dispersion_after_shot: f32,
    pub dispersion_movement: f32,
    pub dispersion_rotation: f32,
    /// The turret's yaw limits relative to the hull, in radians, positive rightward:
    /// None when it turns all around, as `turretYawLimits` has it within ±179°.
    pub yaw_limits: Option<(f32, f32)>,
    /// The gun's pitch limits, in radians, positive downward: the highest elevation
    /// and the deepest depression for each turret yaw.
    pub min_pitch: PitchCurve,
    pub max_pitch: PitchCurve,
    /// The base dispersion angle, in radians: `atan(shotDispersionRadius / 100)`.
    pub dispersion_angle: f32,
    /// The first shell's speed in m/s and gravity in m/s², scaled by the game's
    /// `projectileSpeedFactor` as the client's item loader does.
    pub shot_speed: f32,
    pub shot_gravity: f32,
    /// Seconds between two shots, before crew and equipment.
    pub reload_time: f32,
    /// Where the turret turns, from the vehicle's origin: the chassis' `hullPosition`
    /// plus the hull's turret position.
    pub turret_position: Vec3,
    /// Where the gun pitches, from the turret's pivot, turning with it.
    pub gun_position: Vec3,
}

/// How far the vehicle sees with its default turret, and how hard it is to see, as the
/// client's item loader reads them (`vehicles.py`): before crew, equipment and the
/// camouflage paint's bonus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisionDef {
    /// How far its turret sees, in metres: its `circularVisionRadius`.
    pub view_range: f32,
    /// Its camouflage standing still and moving, as a share of an observer's view range
    /// beyond the closest it is always seen at: its `invisibility` `still` and `moving`.
    pub camouflage_still: f32,
    pub camouflage_moving: f32,
    /// The share of its camouflage it keeps right after firing: its default gun's
    /// `invisibilityFactorAtShot`, the vehicle's own for a shared gun if it sets one.
    pub camouflage_at_shot: f32,
}

/// How the default gun's shells come, beyond one per reload: from a magazine (`clip` to
/// the game), in bursts, as `_readGunClip` and `_readGunBurst` read them, its own section for the vehicle if
/// it has one, else the shared gun's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MagazineDef {
    /// The shells in a full magazine, 1 without one: the gun reloads it whole once
    /// empty, in its reload time.
    pub size: u32,
    /// Seconds between two shots of the magazine, from its `rate` per minute.
    pub interval: f32,
    /// The shells the gun fires each time it is fired, 1 without bursts.
    pub burst_size: u32,
    /// Seconds between two shells of a burst, from its `rate` per minute.
    pub burst_interval: f32,
}

impl Default for MagazineDef {
    fn default() -> Self {
        Self { size: 1, interval: 0.0, burst_size: 1, burst_interval: 0.0 }
    }
}

impl AimingDef {

    /// The lowest and highest pitch over all turret yaws, `pitchLimits['absolute']`.
    pub fn absolute_pitch_limits(&self) -> (f32, f32) {
        (self.min_pitch.lowest(), self.max_pitch.highest())
    }

}

/// A gun pitch limit over the turret's yaw, as `(yaw, pitch)` points in radians, the yaw
/// going from 0 (forward) to 2π, both ends having the same pitch.
#[derive(Debug, Clone)]
pub struct PitchCurve(pub Vec<(f32, f32)>);

impl PitchCurve {

    /// Parse a curve as the game's definitions write it, `position angle` pairs with the
    /// position a fraction of a turn and the angle in degrees.
    fn parse(curve: &str) -> Option<Self> {
        let numbers = curve.split_whitespace().map(str::parse::<f32>).collect::<Result<Vec<_>, _>>().ok()?;
        let points: Vec<_> = numbers.chunks_exact(2)
            .map(|pair| (pair[0] * std::f32::consts::TAU, pair[1].to_radians()))
            .collect();
        (points.len() >= 2 && numbers.len() % 2 == 0).then_some(Self(points))
    }

    /// The limit at a turret yaw, interpolated between the points around it.
    pub fn at(&self, yaw: f32) -> f32 {
        let yaw = yaw.rem_euclid(std::f32::consts::TAU);
        self.0.windows(2)
            .find(|pair| yaw <= pair[1].0)
            .map(|pair| {
                let ((from_yaw, from), (to_yaw, to)) = (pair[0], pair[1]);
                let t = if to_yaw > from_yaw { (yaw - from_yaw) / (to_yaw - from_yaw) } else { 0.0 };
                from + (to - from) * t
            })
            .unwrap_or(self.0[0].1)
    }

    fn lowest(&self) -> f32 {
        self.0.iter().map(|&(_, pitch)| pitch).fold(f32::INFINITY, f32::min)
    }

    fn highest(&self) -> f32 {
        self.0.iter().map(|&(_, pitch)| pitch).fold(f32::NEG_INFINITY, f32::max)
    }

}

impl VehicleDef {

    /// Load a vehicle by nation id and type id, as its descriptors give them.
    pub fn load_by_id(fs: &ResFilesystem, nation_id: u8, type_id: u16) -> io::Result<Self> {
        let nation = NATIONS.get(usize::from(nation_id))
            .ok_or_else(|| invalid(format!("unknown nation id {nation_id}")))?;
        for dir in vehicle_dirs(fs, nation) {
            let list = read_xml(fs, &format!("{dir}/list.xml"))?;
            let name = list.iter_children_all()
                .find(|(_, vehicle)| vehicle.as_element().and_then(|v| v.get_child_integer("id")) == Some(type_id.into()))
                .map(|(name, _)| name.clone());
            if let Some(name) = name {
                return Self::load(fs, nation, &name);
            }
        }
        Err(invalid(format!("no vehicle {type_id} in {nation}")))
    }

    /// Load a vehicle by nation and name, `GB98_T95_FV4201_Chieftain` say, with the
    /// first module of each kind as the default one, as the game does. A vehicle of an
    /// extension, story mode's `A1035_Pershing_SM_SCC` say, is read from the
    /// extension's own definitions, see [`vehicle_dirs`].
    pub fn load(fs: &ResFilesystem, nation: &str, name: &str) -> io::Result<Self> {

        let nation_id = NATIONS.iter().position(|n| *n == nation)
            .ok_or_else(|| invalid(format!("unknown nation {nation}")))? as u8;

        let (dir, type_id, siege_mode) = vehicle_dirs(fs, nation).into_iter()
            .find_map(|dir| {
                let list = read_xml(fs, &format!("{dir}/list.xml")).ok()?;
                let entry = list.get_child_element(name)?;
                let siege_mode = entry.get_child_string("tags")
                    .is_some_and(|tags| tags.split_whitespace().any(|tag| tag == "siegeMode"));
                Some((dir, entry.get_child_integer("id")?, siege_mode))
            })
            .ok_or_else(|| invalid(format!("no vehicle {name} in {nation}")))?;

        let vehicle = read_xml(fs, &format!("{dir}/{name}.xml"))?;
        let component_ids = |kind: &str| -> io::Result<Box<Element>> {
            read_xml(fs, &format!("{dir}/components/{kind}.xml"))
        };

        // The first module of a section, and its id from the components' ids.
        let first = |section: &Element, what: &str| -> io::Result<String> {
            section.iter_children_all().next()
                .map(|(module, _)| module.clone())
                .ok_or_else(|| invalid(format!("{name} has no {what}")))
        };
        let module_id = |components: &Element, module: &str| -> io::Result<u16> {
            components.get_child_element("ids")
                .and_then(|ids| ids.get_child_integer(module))
                .map(|id| id as u16)
                .ok_or_else(|| invalid(format!("no id for {module}")))
        };
        let default_module = |section: &str, kind: &str| -> io::Result<u16> {
            let section = vehicle.get_child_element(section)
                .ok_or_else(|| invalid(format!("{name} has no {section}")))?;
            module_id(&*component_ids(kind)?, &first(section, kind)?)
        };

        let turrets = vehicle.get_child_element("turrets0")
            .ok_or_else(|| invalid(format!("{name} has no turret")))?;
        let turret_name = first(turrets, "turret")?;
        let turret = turrets.get_child_element(&turret_name).unwrap();
        let guns = turret.get_child_element("guns")
            .ok_or_else(|| invalid(format!("{name} has no guns")))?;
        let gun_name = first(guns, "gun")?;

        let guns_components = component_ids("guns")?;
        let gun_def = guns_components.get_child_element("shared")
            .and_then(|shared| shared.get_child_element(&gun_name))
            .ok_or_else(|| invalid(format!("no definition for gun {gun_name}")))?;
        // The vehicle may set its own capacity for a shared gun.
        let max_ammo = guns.get_child_element(&gun_name).and_then(|gun| gun.get_child_integer("maxAmmo"))
            .or_else(|| gun_def.get_child_integer("maxAmmo"))
            .ok_or_else(|| invalid(format!("no ammunition capacity for {gun_name}")))? as u32;

        let shells_components = component_ids("shells")?;
        // Shells are scaled as `gun_readers.readShot` does.
        let common = read_xml(fs, "scripts/item_defs/vehicles/common/vehicle.xml")?;
        let materials = common.get_child_element("materials")
            .ok_or_else(|| invalid("no common materials".to_string()))?;
        let projectile_speed_factor = common
            .get_child_value_at(&["miscParams", "projectileSpeedFactor"]).and_then(pxml::Value::as_float)
            .ok_or_else(|| invalid("no projectile speed factor".to_string()))?;
        let shot_effects = read_xml(fs, "scripts/item_defs/vehicles/common/shot_effects.xml")?;
        let effects_index = |effects: &str| shot_effects.iter_children_all()
            .map(|(name, _)| name)
            .filter(|name| !name.starts_with("xmlns:"))
            .position(|name| name == effects);
        // As the server reads stickers, their id is their index, see `_readDamageStickers`.
        let damage_stickers = read_xml(fs, "scripts/item_defs/vehicles/common/damage_stickers.xml")?;
        let sticker_index = |sticker: &str| damage_stickers.iter_children_all()
            .map(|(name, _)| name)
            .filter(|name| !name.starts_with("xmlns:") && *name != "texture")
            .position(|name| name == sticker);
        let pierced_sticker = |effects: &str| shot_effects.get_child_element(effects)
            .and_then(|effects| effects.get_child_element("targetStickers")?.get_child_string("armorPierced"))
            .and_then(|sticker| sticker_index(sticker))
            .map(|index| index as u8);
        let prefab_shot_effects = read_xml(fs, "scripts/item_defs/vehicles/common/prefab_effects/shot_effects.xml")?;
        let default_prefab_effects = common.get_child_element("defaultPrefabEffects").and_then(|defaults| defaults.get_child_element("shot"));
        let prefab_effects_index = |shell: &Element, kind: &str| shell.get_child_string("prefabEffects")
            .or_else(|| default_prefab_effects?.get_child_string(kind))
            .and_then(|effects| prefab_shot_effects.iter_children_all().position(|(name, _)| name == effects))
            .map(|index| index as u8);
        let mut shells = Vec::new();
        let mut shots = Vec::new();
        for (shell_name, shot) in gun_def.get_child_element("shots").into_iter().flat_map(|shots| shots.iter_children_all()) {
            let shell = shells_components.get_child_element(shell_name)
                .ok_or_else(|| invalid(format!("no definition for shell {shell_name}")))?;
            let shell_id = shell.get_child_integer("id")
                .ok_or_else(|| invalid(format!("no id for shell {shell_name}")))?;
            let shot = shot.as_element().ok_or_else(|| invalid(format!("no shot for shell {shell_name}")))?;
            let portion = shot.get_child_float("defaultPortion").unwrap_or(0.0);
            let cd = IntCompactDescr { item_type: item_type::SHELL, nation: nation_id, id: shell_id as u16 };
            shells.push((cd, portion));
            let shot_float = |name: &str| shot.get_child_float(name)
                .ok_or_else(|| invalid(format!("shell {shell_name} has no {name}")));
            let kind = shell.get_child_string("kind").and_then(|kind| SHELL_KINDS.iter().position(|&k| k == kind))
                .ok_or_else(|| invalid(format!("shell {shell_name} has no known kind")))? as u8;
            let caliber = shell.get_child_float("caliber")
                .ok_or_else(|| invalid(format!("shell {shell_name} has no caliber")))?;
            let damage = shell.get_child_value_at(&["damage", "armor"]).and_then(float_pair)
                .ok_or_else(|| invalid(format!("shell {shell_name} has no damage")))?;
            let pen = shot.get_child_value("piercingPower").and_then(float_pair)
                .ok_or_else(|| invalid(format!("shell {shell_name} has no penetration")))?;
            // As `_readShell` reads each kind's own parameters, `_SHELL_EXTRA_DATA` the
            // ones the kind uses.
            let ricochet_angle_cos = shell.get_child_float("ricochetAngle")
                .filter(|_| matches!(SHELL_KINDS[kind as usize], "ARMOR_PIERCING" | "ARMOR_PIERCING_CR" | "HOLLOW_CHARGE"))
                .map(|angle| angle.to_radians().cos());
            let normalization_angle = shell.get_child_float("normalizationAngle")
                .filter(|_| SHELL_KINDS[kind as usize].starts_with("ARMOR_PIERCING"))
                .unwrap_or(0.0)
                .to_radians();
            shots.push(ShotDef {
                compact_descr: cd,
                kind_index: kind,
                caliber,
                effects_index: shell.get_child_string("effects").and_then(|effects| effects_index(&effects))
                    .ok_or_else(|| invalid(format!("shell {shell_name} has no known effects")))? as u8,
                pierced_sticker: shell.get_child_string("effects").and_then(|effects| pierced_sticker(effects)),
                prefab_effects_index: shell.get_child_string("kind").and_then(|kind| prefab_effects_index(shell, kind)),
                speed: shot_float("speed")? * projectile_speed_factor,
                gravity: shot_float("gravity")? * projectile_speed_factor * projectile_speed_factor,
                max_distance: shot_float("maxDistance")?,
                damage_near: damage[0],
                damage_far: damage[1],
                pen_near: pen[0],
                pen_far: pen[1],
                pen_randomization: shell.get_child_float("piercingPowerRandomization")
                    .unwrap_or(DEFAULT_PEN_RANDOMIZATION),
                normalization_angle,
                ricochet_angle_cos,
                jet_pen_loss: 10.0 * shell.get_child_float("piercingPowerLossFactorByDistance").unwrap_or(0.0),
                explosion_radius: caliber * caliber / 5555.0,
            });
        }

        let max_health = vehicle.get_child_element("hull").and_then(|hull| hull.get_child_integer("maxHealth"))
            .zip(turret.get_child_integer("maxHealth"))
            .map(|(hull, turret)| (hull + turret) as u32)
            .ok_or_else(|| invalid(format!("{name} has no health")))?;

        // The vehicle may set its own values for a shared gun.
        let gun_float = |path: &[&str]| [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| gun.get_child_value_at(path).and_then(pxml::Value::as_float))
            .ok_or_else(|| invalid(format!("no {} for gun {gun_name}", path.join("/"))));
        let chassis_float = |path: &[&str]| vehicle.get_child_element("chassis")
            .and_then(|chassis| chassis.iter_children_all().next())
            .and_then(|(_, chassis)| chassis.as_element()?.get_child_value_at(path).and_then(pxml::Value::as_float))
            .ok_or_else(|| invalid(format!("{name} has no chassis {}", path.join("/"))));
        let pitch_curve = |limit: &str| [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| gun.get_child_element("pitchLimits")?.get_child_string(limit))
            .and_then(PitchCurve::parse)
            .ok_or_else(|| invalid(format!("no valid {limit} for gun {gun_name}")));
        // As `_readGunLocals` reads them, in degrees.
        let yaw_limits = [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| gun.get_child_string("turretYawLimits"))
            .map(|limits| match limits.split_whitespace().map(str::parse::<f32>).collect::<Result<Vec<_>, _>>().as_deref() {
                Ok(&[min, max]) if min <= max => Ok((min > -179.0 || max < 179.0).then(|| (min.to_radians(), max.to_radians()))),
                _ => Err(invalid(format!("invalid turretYawLimits for gun {gun_name}: {limits}"))),
            })
            .transpose()?
            .flatten();
        // The first shell's flight, scaled as every shell is, see `gun_readers.readShot`.
        let first_shot = gun_def.get_child_element("shots")
            .and_then(|shots| shots.iter_children_all().next())
            .and_then(|(_, shot)| shot.as_element())
            .ok_or_else(|| invalid(format!("gun {gun_name} has no shot")))?;
        let shot_float = |name: &str| first_shot.get_child_float(name)
            .ok_or_else(|| invalid(format!("gun {gun_name}'s first shot has no {name}")));

        let aiming = AimingDef {
            turret_rotation_speed: turret.get_child_float("rotationSpeed")
                .ok_or_else(|| invalid(format!("{name} has no turret rotation speed")))?
                .to_radians(),
            gun_rotation_speed: gun_float(&["rotationSpeed"])?.to_radians(),
            aiming_time: gun_float(&["aimingTime"])?,
            dispersion_turret_rotation: gun_float(&["shotDispersionFactors", "turretRotation"])?.to_degrees(),
            dispersion_after_shot: gun_float(&["shotDispersionFactors", "afterShot"])?,
            dispersion_movement: chassis_float(&["shotDispersionFactors", "vehicleMovement"])? * 3.6,
            dispersion_rotation: chassis_float(&["shotDispersionFactors", "vehicleRotation"])?.to_degrees(),
            yaw_limits,
            min_pitch: pitch_curve("minPitch")?,
            max_pitch: pitch_curve("maxPitch")?,
            dispersion_angle: (gun_float(&["shotDispersionRadius"])? / 100.0).atan(),
            shot_speed: shot_float("speed")? * projectile_speed_factor,
            shot_gravity: shot_float("gravity")? * projectile_speed_factor * projectile_speed_factor,
            reload_time: gun_float(&["reloadTime"])?,
            turret_position: vehicle.get_child_element("chassis")
                .and_then(|chassis| chassis.iter_children_all().next())
                .and_then(|(_, chassis)| chassis.as_element()?.get_child_vec3("hullPosition"))
                .ok_or_else(|| invalid(format!("{name} has no hull position")))?
                + vehicle.get_child_element("hull")
                .and_then(|hull| hull.get_child_element("turretPositions"))
                .and_then(|positions| positions.get_child_vec3("turret"))
                .ok_or_else(|| invalid(format!("{name} has no turret position")))?,
            gun_position: turret.get_child_vec3("gunPosition")
                .ok_or_else(|| invalid(format!("{name} has no gun position")))?,
        };

        // The gun's hit tester is the vehicle's own for it, or the shared gun's.
        let chassis = vehicle.get_child_element("chassis")
            .and_then(|chassis| chassis.iter_children_all().next())
            .and_then(|(_, chassis)| chassis.as_element());
        let gun = [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find(|gun| gun.get_child_element("hitTester").is_some());
        let parts = [("chassis", chassis), ("hull", vehicle.get_child_element("hull")), ("turret", Some(turret)), ("gun", gun)];
        // Each part's armour: the gun's is the vehicle's own for it, or the shared gun's.
        let gun_armor = [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| gun.get_child_element("armor"));
        let armors = [chassis.and_then(|chassis| chassis.get_child_element("armor")), vehicle.get_child_element("hull")
            .and_then(|hull| hull.get_child_element("armor")), turret.get_child_element("armor"), gun_armor];
        let mut models = Vec::new();
        for ((part, module), armor) in parts.into_iter().zip(armors) {
            let path = module.and_then(|module| module.get_child_element("hitTester")?.get_child_string("collisionModelClient"))
                .ok_or_else(|| invalid(format!("{name} has no {part} collision model")))?;
            // The definitions name a `.model`, the game ships it as a `.havok`.
            let path = path.strip_suffix(".model").map_or_else(|| path.to_string(), |stem| format!("{stem}.havok"));
            let mut model = CollisionModel::load(fs, &path)?;
            let armor = armor.map(|armor| read_armor(materials, armor)).unwrap_or_default();
            for group in &mut model.groups {
                group.material = armor.get(&group.name).copied();
            }
            models.push(model);
        }
        let collision = Arc::new(VehicleCollision {
            parts: models.try_into().unwrap(),
            hull_position: chassis.and_then(|chassis| chassis.get_child_vec3("hullPosition"))
                .ok_or_else(|| invalid(format!("{name} has no hull position")))?,
        });

        // What collides of it: its chassis and its hull, in the vehicle's frame.
        let [chassis_bounds, hull_bounds] = [collision.parts[0].bounds, collision.parts[1].bounds];
        let bounds = [
            chassis_bounds[0].min(hull_bounds[0] + collision.hull_position),
            chassis_bounds[1].max(hull_bounds[1] + collision.hull_position),
        ];
        let turret_bounds = collision.parts[2].bounds.map(|corner| corner + aiming.turret_position);
        // Its siege mode's definition differs in how it moves, its modules the same.
        let siege = if siege_mode {
            let siege_vehicle = read_xml(fs, &format!("{dir}/{name}_siege_mode.xml"))?;
            let params = vehicle.get_child_element("siege_mode");
            Some(SiegeDef {
                mobility: mobility(fs, &dir, name, &siege_vehicle, turret, guns.get_child_element(&gun_name), gun_def,
                    speed_limit(&siege_vehicle, name, "forward")?, speed_limit(&siege_vehicle, name, "backward")?, [bounds, turret_bounds])?,
                switch_on: siege_switch_time(params, "switchOnTime"),
                switch_off: siege_switch_time(params, "switchOffTime"),
            })
        } else {
            None
        };

        let mobility = mobility(fs, &dir, name, &vehicle, turret, guns.get_child_element(&gun_name), gun_def,
            speed_limit(&vehicle, name, "forward")?, speed_limit(&vehicle, name, "backward")?, [bounds, turret_bounds])?;

        let crew_roles = vehicle.get_child_element("crew")
            .into_iter()
            .flat_map(|crew| crew.iter_children_all())
            .filter_map(|(role, _)| ROLES.iter().find(|(name, _)| name == role).map(|&(_, index)| index))
            .collect();

        let descr = VehicleDescr {
            nation: nation_id,
            type_id: type_id as u16,
            chassis: default_module("chassis", "chassis")?,
            engine: default_module("engines", "engines")?,
            fuel_tank: default_module("fuelTanks", "fuelTanks")?,
            radio: default_module("radios", "radios")?,
            turret: module_id(&*component_ids("turrets")?, &turret_name)?,
            gun: module_id(&guns_components, &gun_name)?,
            optional_devices: Vec::new(),
            extra_flags: 0,
            extra: Vec::new(),
        };

        // A count of 1, the default, has no interval.
        let section = |name: &str| [guns.get_child_element(&gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| gun.get_child_element(name))
            .map(|section| -> io::Result<(u32, f32)> {
                let count = section.get_child_integer("count").unwrap_or(1).max(1) as u32;
                let rate = section.get_child_float("rate").filter(|&rate| rate > 0.0)
                    .ok_or_else(|| invalid(format!("gun {gun_name} has no positive {name}/rate")))?;
                Ok((count, if count > 1 { 60.0 / rate } else { 0.0 }))
            })
            .transpose();
        let (size, interval) = section("clip")?.unwrap_or((1, 0.0));
        let (burst_size, burst_interval) = section("burst")?.unwrap_or((1, 0.0));
        let magazine = MagazineDef { size, interval, burst_size, burst_interval };

        let vision = VisionDef {
            view_range: turret.get_child_float("circularVisionRadius")
                .ok_or_else(|| invalid(format!("{name} has no turret circularVisionRadius")))?,
            camouflage_still: vehicle.get_child_value_at(&["invisibility", "still"])
                .and_then(pxml::Value::as_float)
                .ok_or_else(|| invalid(format!("{name} has no invisibility/still")))?,
            camouflage_moving: vehicle.get_child_value_at(&["invisibility", "moving"])
                .and_then(pxml::Value::as_float)
                .ok_or_else(|| invalid(format!("{name} has no invisibility/moving")))?,
            camouflage_at_shot: gun_float(&["invisibilityFactorAtShot"])?,
        };

        Ok(Self {
            name: name.to_string(),
            descr,
            crew_roles,
            shells,
            shots,
            max_ammo,
            max_health,
            mobility,
            aiming,
            magazine,
            vision,
            collision,
            siege,
        })

    }

    /// The default ammunition, by shell: the capacity split by the shells' default
    /// shares, the rounding remainder to the first shell.
    pub fn default_ammo(&self) -> Vec<(IntCompactDescr, u32)> {
        let mut ammo: Vec<_> = self.shells.iter()
            .map(|&(cd, portion)| (cd, (portion * self.max_ammo as f32).round() as u32))
            .collect();
        let total: u32 = ammo.iter().map(|&(_, count)| count).sum();
        if let Some((_, first)) = ammo.first_mut() {
            *first = (*first + self.max_ammo).saturating_sub(total);
        }
        ammo
    }

}

/// A part's armour section, by material name, each over the game's default for it, as
/// `_readArmor` reads it: a thickness, with the material's parameters it overrides as
/// children. A material without a default isn't one a vehicle uses.
fn read_armor(materials: &Element, armor: &Element) -> HashMap<String, ArmorMaterial> {
    armor.iter_children_all()
        .filter_map(|(name, value)| {
            let default = materials.get_child_element(name)?;
            let (thickness, overrides) = match value {
                pxml::Value::Element(element) => (element.value.as_float()?, Some(&**element)),
                value => (value.as_float()?, None),
            };
            let flag = |param: &str| overrides.and_then(|o| o.get_child_boolean(param))
                .or_else(|| default.get_child_boolean(param))
                .unwrap_or(false);
            let material = ArmorMaterial {
                armor: thickness,
                vehicle_damage_factor: overrides.and_then(|o| o.get_child_float("vehicleDamageFactor"))
                    .or_else(|| default.get_child_float("vehicleDamageFactor"))
                    .unwrap_or(0.0),
                use_hit_angle: flag("useHitAngle"),
                may_ricochet: flag("mayRicochet"),
                check_caliber_for_ricochet: flag("checkCaliberForRicochet"),
                check_caliber_for_hit_angle_norm: flag("checkCaliberForHitAngleNorm"),
                collide_once_only: flag("collideOnceOnly"),
            };
            Some((name.clone(), material))
        })
        .collect()
}

/// A vehicle's top speed one way, `forward` or `backward`, in m/s, from its
/// `speedLimits` in km/h.
fn speed_limit(vehicle: &Element, name: &str, direction: &str) -> io::Result<f32> {
    vehicle.get_child_element("speedLimits")
        .and_then(|limits| limits.get_child_float(direction))
        .map(|kmh| kmh / 3.6)
        .ok_or_else(|| invalid(format!("{name} has no {direction} speed")))
}

/// How long switching its siege mode takes one way, by its `siege_mode` section's `key`,
/// in seconds, else [`SIEGE_SWITCH_TIME`].
fn siege_switch_time(params: Option<&Element>, key: &str) -> f32 {
    params.and_then(|params| params.get_child_float(key)).unwrap_or(SIEGE_SWITCH_TIME)
}

/// A pair of floats, `readFloatPair`'s: two space-separated, or one for both.
fn float_pair(value: &pxml::Value) -> Option<[f32; 2]> {
    value.as_vec2().map(|pair| pair.to_array()).or_else(|| value.as_float().map(|value| [value; 2]))
}

/// A vehicle's mobility with its default modules, its top speeds given, as
/// `VehicleDescriptor.physics` has it. The engine, fuel tank and radio are shared
/// modules, which the vehicle may set its own values for, as it may for its gun.
#[allow(clippy::too_many_arguments)]
fn mobility(
    fs: &ResFilesystem,
    dir: &str,
    name: &str,
    vehicle: &Element,
    turret: &Element,
    own_gun: Option<&Element>,
    gun: &Element,
    speed_forward: f32,
    speed_backward: f32,
    [bounds, turret_bounds]: [[Vec3; 2]; 2],
) -> io::Result<MobilityDef> {

    let missing = |what: &str| invalid(format!("{name} has no {what}"));
    let first = |section: &str| vehicle.get_child_element(section)
        .and_then(|section| section.iter_children_all().next())
        .ok_or_else(|| missing(section));
    // A module's own value, else its shared definition's.
    let module_float = |section: &str, kind: &str, key: &str| -> io::Result<f32> {
        let (module, own) = first(section)?;
        if let Some(value) = own.as_element().and_then(|own| own.get_child_float(key)) {
            return Ok(value);
        }
        read_xml(fs, &format!("{dir}/components/{kind}.xml"))?
            .get_child_element("shared")
            .and_then(|shared| shared.get_child_element(module))
            .and_then(|shared| shared.get_child_float(key))
            .ok_or_else(|| missing(&format!("{key} for {module}")))
    };

    let (_, chassis) = first("chassis")?;
    let chassis = chassis.as_element().ok_or_else(|| missing("chassis"))?;
    let chassis_float = |key: &str| chassis.get_child_float(key).ok_or_else(|| missing(&format!("chassis {key}")));
    let terrain_resistance = chassis.get_child("terrainResistance").and_then(pxml::Value::as_vec3)
        .ok_or_else(|| missing("chassis terrainResistance"))?;
    let carrying_point = chassis.get_child("topRightCarryingPoint").and_then(pxml::Value::as_vec2)
        .ok_or_else(|| missing("chassis topRightCarryingPoint"))?;

    // What the client keeps of the server's physics, by module name.
    let physics = vehicle.get_child_element("physics").and_then(|physics| physics.get_child_element("detailed"));
    let (engine_name, _) = first("engines")?;
    let (chassis_name, _) = first("chassis")?;
    let engine_power = match physics.and_then(|physics| physics.get_child_value_at(&["engines", engine_name.as_str(), "smplEnginePower"])).and_then(pxml::Value::as_float) {
        Some(power) => power,
        None => module_float("engines", "engines", "power")?,
    };
    let rolling_friction = ["firm", "medium", "soft"].map(|ground| physics
        .and_then(|physics| physics.get_child_value_at(&["chassis", chassis_name.as_str(), "grounds", ground, "rollingFriction"]))
        .and_then(pxml::Value::as_float));
    let [Some(firm), Some(medium), Some(soft)] = rolling_friction else {
        return Err(missing("rolling friction in its physics section"));
    };

    let hull_weight = vehicle.get_child_element("hull").and_then(|hull| hull.get_child_float("weight"))
        .ok_or_else(|| missing("hull weight"))?;
    let turret_weight = turret.get_child_float("weight").ok_or_else(|| missing("turret weight"))?;
    let gun_weight = own_gun.and_then(|gun| gun.get_child_float("weight")).or_else(|| gun.get_child_float("weight"))
        .ok_or_else(|| missing("gun weight"))?;
    let weight = hull_weight + chassis_float("weight")? + turret_weight + gun_weight
        + module_float("engines", "engines", "weight")?
        + module_float("fuelTanks", "fuelTanks", "weight")?
        + module_float("radios", "radios", "weight")?;

    let rotation_speed = chassis_float("rotationSpeed")?.to_radians();
    let wheels = read_wheels(fs, chassis)
        .inspect_err(|e| warn!(vehicle = name, "Failed to read a chassis' wheels: {e}"))
        .ok()
        .flatten();
    let chassis_physics = physics.and_then(|physics| physics.get_child_element("chassis"))
        .and_then(|chassis| chassis.get_child_element(chassis_name));
    let general_wheels = read_general_wheels(chassis, chassis_physics);
    Ok(MobilityDef {
        speed_forward,
        speed_backward,
        rotation_speed: terrain_resistance.to_array().map(|resistance| rotation_speed * terrain_resistance.x / resistance),
        engine_power: engine_power * HP_TO_WATTS,
        weight,
        rolling_friction: [firm, medium, soft],
        carrying_point,
        bounds,
        turret_bounds,
        wheels,
        general_wheels,
    })

}

/// A chassis' general wheels, see [`GeneralWheels`], how far their axles steer from
/// its section of the vehicle's physics, `physics`: none if its definition has none.
/// Its axles are its wheels' places along it, from the rearmost, as the physics section
/// lists them; without lock angles, an axle's lock is its angle driving.
fn read_general_wheels(chassis: &Element, physics: Option<&Element>) -> Option<GeneralWheels> {

    let wheels = chassis.get_child_element("wheels")?;
    if !wheels.get_child("generalWheels").and_then(pxml::Value::as_boolean).unwrap_or(false) {
        return None;
    }

    let mut indexed = Vec::new();
    for (key, value) in wheels.iter_children_all() {
        let Some(wheel) = value.as_element().filter(|_| key.as_str() == "wheel") else { continue };
        if !wheel.get_child("nonTrack").and_then(pxml::Value::as_boolean).unwrap_or(false) {
            continue;
        }
        let (Some(index), Some(center)) = (wheel.get_child_integer("index"), wheel.get_child_vec3("wheelPos")) else { continue };
        let radius = wheel.get_child_element("geometry")
            .and_then(|geometry| geometry.get_child_float("radius"))
            .or_else(|| wheel.get_child_float("radius"));
        let Some(radius) = radius else { continue };
        let steers = wheel.get_child("steeringProcessor").is_some();
        indexed.push((index, GeneralWheel {
            center,
            radius,
            steers,
            lock: 0.0,
            on_move: 0.0,
        }));
    }
    indexed.sort_by_key(|&(index, _)| index);

    // Its axles, from the rearmost, and how far each steers.
    let mut axles: Vec<f32> = Vec::new();
    for (_, wheel) in &indexed {
        if !axles.iter().any(|&z| (z - wheel.center.z).abs() < AXLE_SPREAD) {
            axles.push(wheel.center.z);
        }
    }
    axles.sort_by(f32::total_cmp);
    let on_move = physics.and_then(|physics| physics.get_child_value("axleSteeringAngles")).and_then(floats).unwrap_or_default();
    let lock = physics.and_then(|physics| physics.get_child_value("axleSteeringLockAngles")).and_then(floats).unwrap_or_else(|| on_move.clone());
    for (_, wheel) in &mut indexed {
        let Some(axle) = axles.iter().position(|&z| (z - wheel.center.z).abs() < AXLE_SPREAD) else { continue };
        wheel.lock = lock.get(axle).copied().unwrap_or(0.0).to_radians();
        wheel.on_move = on_move.get(axle).copied().unwrap_or(0.0).to_radians();
    }

    let turns_on_spot = physics.and_then(|physics| physics.get_child_value("isWheeledOnSpotRotation"))
        .is_some_and(|value| value.as_boolean().unwrap_or(false) || value.as_string().is_some_and(|value| value.trim().eq_ignore_ascii_case("true")));
    let wheels: Vec<GeneralWheel> = indexed.into_iter().map(|(_, wheel)| wheel).collect();
    (!wheels.is_empty()).then(|| GeneralWheels::new(&wheels, turns_on_spot))

}

/// The floats of a value: a vector's, or a string's, space-separated, or a single one.
fn floats(value: &pxml::Value) -> Option<Vec<f32>> {
    if let Some(vector) = value.as_vector() {
        return Some(vector.iter().copied().collect());
    }
    if let Some(text) = value.as_string() {
        return text.split_whitespace().map(|float| float.parse().ok()).collect();
    }
    value.as_float().map(|float| vec![float])
}

/// A chassis' wheels, see [`Wheels`]: none if its definition names none, or its model
/// places none of them.
fn read_wheels(fs: &ResFilesystem, chassis: &Element) -> io::Result<Option<Wheels>> {

    // Each wheel's name, its radius, and whether it is a leading one.
    let mut named: Vec<(String, f32, bool)> = Vec::new();
    let Some(wheels) = chassis.get_child_element("wheels") else { return Ok(None) };
    for (key, value) in wheels.iter_children_all() {
        let Some(wheel) = value.as_element() else { continue };
        let Some(radius) = wheel.get_child_float("radius") else { continue };
        match key.as_str() {
            "wheel" => {
                let Some(name) = wheel.get_child_string("name") else { continue };
                let leading = wheel.get_child("isLeading").and_then(pxml::Value::as_boolean).unwrap_or(false);
                named.push((name.to_string(), radius, leading));
            }
            "group" => {
                let Some(template) = wheel.get_child_string("template") else { continue };
                let start = wheel.get_child_integer("startIndex").unwrap_or(0);
                let count = wheel.get_child_integer("count").unwrap_or(0);
                for index in start..start + count {
                    named.push((format!("{template}{index}"), radius, false));
                }
            }
            _ => {}
        }
    }

    let Some(model) = chassis.get_child_element("models").and_then(|models| models.get_child_string("undamaged")) else {
        return Ok(None);
    };
    let path = model.strip_suffix(".model").map_or_else(|| model.to_string(), |stem| format!("{stem}.visual_processed"));
    let visual = read_xml(fs, &path)?;
    let mut placed = HashMap::new();
    if let Some(root) = visual.get_child_element("node") {
        place_nodes(root, Affine3A::IDENTITY, &mut placed);
    }

    let found: Vec<(WheelDef, bool)> = named.iter()
        .filter_map(|(name, radius, leading)| {
            let wheel = WheelDef {
                center: *placed.get(name.as_str())?,
                radius: *radius,
            };
            Some((wheel, *leading))
        })
        .collect();
    let lowest = found.iter()
        .filter(|(_, leading)| !leading)
        .map(|(wheel, _)| wheel.center.y - wheel.radius)
        .reduce(f32::min);
    let Some(lowest) = lowest else { return Ok(None) };

    let road: Vec<WheelDef> = found.iter()
        .filter(|(wheel, leading)| !leading && wheel.center.y - wheel.radius <= lowest + ROAD_WHEEL_RISE)
        .map(|(wheel, _)| *wheel)
        .collect();
    let bottoms = road.iter().map(|wheel| wheel.center.y - wheel.radius).sum::<f32>();
    let track_thickness = (bottoms / road.len() as f32).max(0.0);

    // The frontmost and the rearmost leading wheel of each side.
    let mut ends: Vec<WheelDef> = Vec::new();
    for right in [true, false] {
        let side: Vec<WheelDef> = found.iter()
            .filter(|(wheel, leading)| *leading && (wheel.center.x > 0.0) == right)
            .map(|(wheel, _)| *wheel)
            .collect();
        let front = side.iter().copied().max_by(|a, b| a.center.z.total_cmp(&b.center.z));
        let back = side.iter().copied().min_by(|a, b| a.center.z.total_cmp(&b.center.z));
        for wheel in [front, back].into_iter().flatten() {
            if !ends.contains(&wheel) {
                ends.push(wheel);
            }
        }
    }

    Ok(Some(Wheels::new(&road, &ends, track_thickness)))

}

/// Where each node of a visual's tree is, by its identifier: its transform's
/// translation once composed with its parents', from `parent`.
fn place_nodes<'a>(node: &'a Element, parent: Affine3A, placed: &mut HashMap<&'a str, Vec3>) {
    let transform = parent * node.get_child("transform").and_then(pxml::Value::as_affine3).unwrap_or(Affine3A::IDENTITY);
    if let Some(identifier) = node.get_child_string("identifier") {
        placed.insert(identifier, transform.translation.into());
    }
    for child in node.iter_children("node") {
        if let Some(child) = child.as_element() {
            place_nodes(child, transform, placed);
        }
    }
}

/// Where a nation's vehicles are defined: the core's directory, then each enabled
/// extension's that has some, in name order, as `ExtensionsManager` finds them: a
/// top-level directory with an `extension.xml`. An extension lists its vehicles in its
/// own `list.xml`, and its components define their modules: story mode's do.
fn vehicle_dirs(fs: &ResFilesystem, nation: &str) -> Vec<String> {
    let mut extensions = fs.read_dir("").into_iter().flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.stat().is_dir())
        .map(|entry| entry.name().to_string())
        .filter(|extension| read_xml(fs, &format!("{extension}/extension.xml"))
            .is_ok_and(|extension| extension.get_child_boolean("IsEnabled") == Some(true)))
        .map(|extension| format!("{extension}/"))
        .collect::<Vec<_>>();
    extensions.sort();
    std::iter::once(String::new()).chain(extensions)
        .map(|root| format!("{root}scripts/item_defs/vehicles/{nation}"))
        .filter(|dir| fs.stat(format!("{dir}/list.xml")).is_ok())
        .collect()
}

pub(crate) fn read_xml(fs: &ResFilesystem, path: &str) -> io::Result<Box<Element>> {
    let file = fs.read(path).map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
    pxml::from_reader(io::BufReader::new(file)).map_err(|e| invalid(format!("{path}: {e}")))
}

pub(crate) fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}


#[cfg(test)]
mod tests {

    use super::*;

    /// With the game's `res` directory in `WGT_RES_PATH`: the T110E4's mobility, as its
    /// definitions, its physics section and the shared modules give it.
    #[test]
    fn real_mobility() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let mobility = VehicleDef::load(&fs, "usa", "A83_T110E4").unwrap().mobility;
        assert_eq!(mobility.engine_power, 950.0 * HP_TO_WATTS);
        assert_eq!(mobility.rolling_friction, [0.1127, 0.1288, 0.21735]);
        // 26°/s on firm ground, its terrain resistance 1.4, 1.6 and 2.7.
        let degrees = mobility.rotation_speed.map(f32::to_degrees);
        assert!(degrees.iter().zip([26.0, 22.75, 13.481]).all(|(real, expected)| (real - expected).abs() < 1e-3), "{degrees:?}");
        // Hull, chassis, turret, engine, fuel tank and radio, and the gun's few tonnes.
        assert!((63_000.0..70_000.0).contains(&mobility.weight), "{}", mobility.weight);
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: the EBR's six wheels, by their
    /// index, the rear and front pairs steering, locked 15° and 4.6° driving, the rear
    /// against the front, not on the spot; the Concept No. 5's front pair alone, 6°
    /// either way, and it turns on the spot.
    #[test]
    fn real_general_wheels() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let mobility = VehicleDef::load(&fs, "france", "F107_Hotchkiss_EBR").unwrap().mobility;
        let wheels = mobility.general_wheels.unwrap();
        let steers: Vec<bool> = wheels.all().iter().map(|wheel| wheel.steers).collect();
        assert_eq!(steers, [true, true, false, false, true, true]);
        assert!(wheels.all().iter().all(|wheel| wheel.radius == 0.608 && wheel.center.y == 0.618));
        assert_eq!(wheels.all()[4].center, Vec3::new(-1.119, 0.618, 2.005));
        let locks: Vec<[f32; 2]> = wheels.all().iter().map(|wheel| [wheel.lock, wheel.on_move].map(|lock| lock.to_degrees().round())).collect();
        assert_eq!(locks, [[-15.0, -5.0], [-15.0, -5.0], [0.0, 0.0], [0.0, 0.0], [15.0, 5.0], [15.0, 5.0]]);
        assert!(!wheels.turns_on_spot);

        let mobility = VehicleDef::load(&fs, "uk", "GB120_Concept_No_5").unwrap().mobility;
        let wheels = mobility.general_wheels.unwrap();
        let locks: Vec<f32> = wheels.all().iter().filter(|wheel| wheel.steers).map(|wheel| wheel.lock.to_degrees().round()).collect();
        assert_eq!(locks, [6.0, 6.0]);
        assert!(wheels.turns_on_spot);
        assert!(VehicleDef::load(&fs, "france", "F107_Hotchkiss_EBR").unwrap().siege.is_none());
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: the EBR 105's siege mode, its
    /// fast one, at 91 and 65 km/h where it goes 70 and 50 out of it, its wheels locked
    /// 15° where they lock 33°, switched at once.
    #[test]
    fn real_siege_mode() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let def = VehicleDef::load(&fs, "france", "F108_Panhard_EBR_105").unwrap();
        let siege = def.siege.unwrap();
        assert_eq!(top_speeds_kmh(&def.mobility), [70.0, 50.0]);
        assert_eq!(top_speeds_kmh(&siege.mobility), [91.0, 65.0]);
        assert_eq!(front_lock_degrees(&def.mobility), 33.0);
        assert_eq!(front_lock_degrees(&siege.mobility), 15.0);
        assert_eq!([siege.switch_on, siege.switch_off], [0.0; 2]);
    }

    /// Its top speeds forward and backward, in km/h, rounded.
    fn top_speeds_kmh(mobility: &MobilityDef) -> [f32; 2] {
        [mobility.speed_forward, mobility.speed_backward].map(|speed| (speed * 3.6).round())
    }

    /// The most its general wheels steer at a standstill, to the right, in degrees,
    /// rounded.
    fn front_lock_degrees(mobility: &MobilityDef) -> f32 {
        let wheels = mobility.general_wheels.unwrap();
        wheels.all().iter().map(|wheel| wheel.lock).fold(0.0, f32::max).to_degrees().round()
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: story mode's Pershing, from the
    /// extension's own definitions, by name and by its descriptor's id, with its own
    /// chassis turning at 38°/s where the core Pershing's turn at 47 and 50.
    #[test]
    fn extension_vehicle() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let def = VehicleDef::load(&fs, "usa", "A1035_Pershing_SM_SCC").unwrap();
        assert_eq!(def.descr.type_id, 409);
        assert!((def.mobility.rotation_speed[0].to_degrees() - 38.0).abs() < 1e-3);
        assert_eq!(VehicleDef::load_by_id(&fs, def.descr.nation, 409).unwrap().name, "A1035_Pershing_SM_SCC");
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: every vehicle's chassis and
    /// physics section have what its mobility needs. Slow, loading every vehicle: run with `--ignored`.
    #[test]
    #[ignore]
    fn every_vehicle_has_rolling_friction() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let mut loaded = 0;
        for nation in NATIONS {
            let list = read_xml(&fs, &format!("scripts/item_defs/vehicles/{nation}/list.xml")).unwrap();
            for (name, _) in list.iter_children_all().filter(|(name, _)| !name.starts_with("xmlns")) {
                // Some listed vehicles miss other parts, which this test isn't about.
                match VehicleDef::load(&fs, nation, name) {
                    Ok(def) => {
                        assert!(def.mobility.rolling_friction.iter().all(|&friction| friction > 0.0), "{nation}:{name}");
                        loaded += 1;
                    }
                    Err(e) => assert!(!["rolling friction", "terrainResistance", "topRightCarryingPoint"].iter().any(|part| e.to_string().contains(part)), "{nation}:{name}: {e}"),
                }
            }
        }
        assert!(loaded > 1000, "{loaded}");
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: AP and APCR shells ricochet
    /// past 70° from the armour's normal, HEAT past 85°, HE never.
    #[test]
    fn real_ricochet_angles() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let mut kinds = std::collections::HashSet::new();
        for (nation, name) in [("uk", "GB98_T95_FV4201_Chieftain"), ("uk", "GB83_FV4005"), ("germany", "G44_JagdTiger"), ("france", "F75_Char_de_25t")] {
            for shot in VehicleDef::load(&fs, nation, name).unwrap().shots {
                let angle = shot.ricochet_angle_cos.map(|cos| cos.acos().to_degrees().round());
                let expected = match shot.kind() {
                    ShellKind::ArmorPiercing | ShellKind::ArmorPiercingCr => Some(70.0),
                    ShellKind::HollowCharge => Some(85.0),
                    _ => None,
                };
                assert_eq!(angle, expected, "{name}: {:?}", shot.kind());
                kinds.insert(shot.kind());
            }
        }
        assert!(kinds.contains(&ShellKind::HollowCharge) && kinds.contains(&ShellKind::ArmorPiercingCr), "{kinds:?}");
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: a turret turning all around,
    /// and a casemate's few degrees either way.
    #[test]
    fn real_turret_yaw_limits() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let chieftain = VehicleDef::load(&fs, "uk", "GB98_T95_FV4201_Chieftain").unwrap();
        assert_eq!(chieftain.aiming.yaw_limits, None);
        let jagdtiger = VehicleDef::load(&fs, "germany", "G44_JagdTiger").unwrap();
        let (min, max) = jagdtiger.aiming.yaw_limits.unwrap();
        assert!(min < 0.0 && max > 0.0 && max - min < 30f32.to_radians(), "{:?}", (min.to_degrees(), max.to_degrees()));
        // A turret, but not all around.
        let fv4005 = VehicleDef::load(&fs, "uk", "GB83_FV4005").unwrap();
        let (min, max) = fv4005.aiming.yaw_limits.unwrap();
        assert!((min.to_degrees() + 90.0).abs() < 1e-3 && (max.to_degrees() - 90.0).abs() < 1e-3, "{:?}", (min, max));
    }

}
