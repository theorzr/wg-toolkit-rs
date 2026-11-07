//! Vehicle definitions, read from the game's `scripts/item_defs/vehicles`: per nation,
//! `list.xml` gives each vehicle's type id, `<vehicle>.xml` its modules by name, and
//! `components/<kind>.xml` the modules' ids (in an `ids` section) and the shared
//! modules' definitions (in a `shared` section), guns with their shells. `components/shells.xml` gives each
//! shell's id.

use std::collections::HashMap;
use std::sync::Arc;
use std::io;

use glam::Vec3;

use wgtk::pxml::{self, Element};
use wgtk::res::fs::ResFilesystem;

use super::collision::{CollisionModel, Material, VehicleCollision};
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
    /// The default modules' collision models.
    pub collision: Arc<VehicleCollision>,
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

/// `PIERCING_POWER_INTERPOLATION_DIST_FIRST` and `_LAST`, the same as for damage.
const INTERPOLATION_DISTANCES: [f32; 2] = [50.0, 500.0];

/// A value at a distance between its values at [`INTERPOLATION_DISTANCES`], held before
/// the first, carried on past the last, never below zero.
fn interpolate_by_distance([near, far]: [f32; 2], distance: f32) -> f32 {
    let [first, last] = INTERPOLATION_DISTANCES;
    let t = (distance.max(first) - first) / (last - first);
    (near + (far - near) * t).max(0.0)
}

/// `DEFAULT_PIERCING_POWER_RANDOMIZATION`.
const DEFAULT_PEN_RANDOMIZATION: f32 = 0.25;

/// `SHELL_TYPES_LIST`: a shell kind's index is its position.
const SHELL_KINDS: &[&str] = &[
    "HOLLOW_CHARGE", "HIGH_EXPLOSIVE", "ARMOR_PIERCING", "ARMOR_PIERCING_HE", "ARMOR_PIERCING_CR", "SMOKE",
];

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
        let list = read_xml(fs, &format!("scripts/item_defs/vehicles/{nation}/list.xml"))?;
        let name = list.iter_children_all()
            .find(|(_, vehicle)| vehicle.as_element().and_then(|v| v.get_child_integer("id")) == Some(type_id.into()))
            .map(|(name, _)| name.clone())
            .ok_or_else(|| invalid(format!("no vehicle {type_id} in {nation}")))?;
        Self::load(fs, nation, &name)
    }

    /// Load a vehicle by nation and name, `GB98_T95_FV4201_Chieftain` say, with the
    /// first module of each kind as the default one, as the game does.
    pub fn load(fs: &ResFilesystem, nation: &str, name: &str) -> io::Result<Self> {

        let nation_id = NATIONS.iter().position(|n| *n == nation)
            .ok_or_else(|| invalid(format!("unknown nation {nation}")))? as u8;
        let dir = format!("scripts/item_defs/vehicles/{nation}");

        let list = read_xml(fs, &format!("{dir}/list.xml"))?;
        let type_id = list.get_child_element(name)
            .and_then(|vehicle| vehicle.get_child_integer("id"))
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

        let speed_limit = |direction| vehicle.get_child_element("speedLimits")
            .and_then(|limits| limits.get_child_float(direction))
            .map(|kmh| kmh / 3.6)
            .ok_or_else(|| invalid(format!("{name} has no {direction} speed")));
        let mobility = mobility(fs, &dir, name, &vehicle, turret, guns.get_child_element(&gun_name), gun_def, speed_limit("forward")?, speed_limit("backward")?)?;

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

        Ok(Self { name: name.to_string(), descr, crew_roles, shells, shots, max_ammo, max_health, mobility, aiming, magazine, collision })

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
fn read_armor(materials: &Element, armor: &Element) -> HashMap<String, Material> {
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
            let material = Material {
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
    Ok(MobilityDef {
        speed_forward,
        speed_backward,
        rotation_speed: terrain_resistance.to_array().map(|resistance| rotation_speed * terrain_resistance.x / resistance),
        engine_power: engine_power * HP_TO_WATTS,
        weight,
        rolling_friction: [firm, medium, soft],
    })

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
                    Err(e) => assert!(!e.to_string().contains("rolling friction") && !e.to_string().contains("terrainResistance"), "{nation}:{name}: {e}"),
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
