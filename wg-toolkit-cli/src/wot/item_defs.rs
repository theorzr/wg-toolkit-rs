//! Vehicle definitions, read from the game's `scripts/item_defs/vehicles`: per nation,
//! `list.xml` gives each vehicle's type id, `<vehicle>.xml` its modules by name, and
//! `components/<kind>.xml` the modules' ids (in an `ids` section) and the shared
//! modules' definitions (in a `shared` section), guns with their shells. `components/shells.xml` gives each
//! shell's id.

use glam::Vec3;
use std::io;
use std::sync::Arc;

use wgtk::pxml::{self, Element};
use wgtk::res::fs::ResFilesystem;

use super::collision::{CollisionModel, VehicleCollision};
use super::items::{item_type, IntCompactDescr, VehicleDescr};


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
    /// Top speeds forward and backward, in m/s.
    pub speed_forward: f32,
    pub speed_backward: f32,
    /// How fast the default chassis turns the vehicle, in rad/s.
    pub rotation_speed: f32,
    pub aiming: AimingDef,
    /// The default modules' collision models.
    pub collision: Arc<VehicleCollision>,
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
    /// In m/s and m/s², scaled by `projectileSpeedFactor` as the client's item loader
    /// does.
    pub speed: f32,
    pub gravity: f32,
    /// In metres.
    pub max_distance: f32,
    /// The damage it deals to a vehicle it pierces, on average.
    pub damage: f32,
}

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
            .find(|(_, vehicle)| vehicle.as_element().and_then(|v| child_int(v, "id")) == Some(type_id.into()))
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
        let type_id = child_element(&list, name)
            .and_then(|vehicle| child_int(vehicle, "id"))
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
            child_element(components, "ids")
                .and_then(|ids| child_int(ids, module))
                .map(|id| id as u16)
                .ok_or_else(|| invalid(format!("no id for {module}")))
        };
        let default_module = |section: &str, kind: &str| -> io::Result<u16> {
            let section = child_element(&vehicle, section)
                .ok_or_else(|| invalid(format!("{name} has no {section}")))?;
            module_id(&*component_ids(kind)?, &first(section, kind)?)
        };

        let turrets = child_element(&vehicle, "turrets0")
            .ok_or_else(|| invalid(format!("{name} has no turret")))?;
        let turret_name = first(turrets, "turret")?;
        let turret = child_element(turrets, &turret_name).unwrap();
        let guns = child_element(turret, "guns")
            .ok_or_else(|| invalid(format!("{name} has no guns")))?;
        let gun_name = first(guns, "gun")?;

        let guns_components = component_ids("guns")?;
        let gun_def = child_element(&guns_components, "shared")
            .and_then(|shared| child_element(shared, &gun_name))
            .ok_or_else(|| invalid(format!("no definition for gun {gun_name}")))?;
        // The vehicle may set its own capacity for a shared gun.
        let max_ammo = child_element(guns, &gun_name).and_then(|gun| child_int(gun, "maxAmmo"))
            .or_else(|| child_int(gun_def, "maxAmmo"))
            .ok_or_else(|| invalid(format!("no ammunition capacity for {gun_name}")))? as u32;

        let shells_components = component_ids("shells")?;
        // Shells are scaled as `gun_readers.readShot` does.
        let projectile_speed_factor = read_xml(fs, "scripts/item_defs/vehicles/common/vehicle.xml")?
            .get_child("miscParams").and_then(|params| child_float(params.as_element()?, "projectileSpeedFactor"))
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
        let pierced_sticker = |effects: &str| child_element(&shot_effects, effects)
            .and_then(|effects| child_string(child_element(effects, "targetStickers")?, "armorPierced"))
            .and_then(|sticker| sticker_index(sticker))
            .map(|index| index as u8);
        let mut shells = Vec::new();
        let mut shots = Vec::new();
        for (shell_name, shot) in child_element(gun_def, "shots").into_iter().flat_map(|shots| shots.iter_children_all()) {
            let shell = child_element(&shells_components, shell_name)
                .ok_or_else(|| invalid(format!("no definition for shell {shell_name}")))?;
            let shell_id = child_int(shell, "id")
                .ok_or_else(|| invalid(format!("no id for shell {shell_name}")))?;
            let shot = shot.as_element().ok_or_else(|| invalid(format!("no shot for shell {shell_name}")))?;
            let portion = child_float(shot, "defaultPortion").unwrap_or(0.0);
            let cd = IntCompactDescr { item_type: item_type::SHELL, nation: nation_id, id: shell_id as u16 };
            shells.push((cd, portion));
            let shot_float = |name: &str| child_float(shot, name)
                .ok_or_else(|| invalid(format!("shell {shell_name} has no {name}")));
            shots.push(ShotDef {
                compact_descr: cd,
                kind_index: child_string(shell, "kind").and_then(|kind| SHELL_KINDS.iter().position(|&k| k == kind))
                    .ok_or_else(|| invalid(format!("shell {shell_name} has no known kind")))? as u8,
                caliber: child_float(shell, "caliber")
                    .ok_or_else(|| invalid(format!("shell {shell_name} has no caliber")))?,
                effects_index: child_string(shell, "effects").and_then(|effects| effects_index(&effects))
                    .ok_or_else(|| invalid(format!("shell {shell_name} has no known effects")))? as u8,
                pierced_sticker: child_string(shell, "effects").and_then(|effects| pierced_sticker(effects)),
                speed: shot_float("speed")? * projectile_speed_factor,
                gravity: shot_float("gravity")? * projectile_speed_factor * projectile_speed_factor,
                max_distance: shot_float("maxDistance")?,
                damage: child_float_at(shell, &["damage", "armor"])
                    .ok_or_else(|| invalid(format!("shell {shell_name} has no damage")))?,
            });
        }

        let max_health = child_element(&vehicle, "hull").and_then(|hull| child_int(hull, "maxHealth"))
            .zip(child_int(turret, "maxHealth"))
            .map(|(hull, turret)| (hull + turret) as u32)
            .ok_or_else(|| invalid(format!("{name} has no health")))?;

        let speed_limit = |direction| child_element(&vehicle, "speedLimits")
            .and_then(|limits| child_float(limits, direction))
            .map(|kmh| kmh / 3.6)
            .ok_or_else(|| invalid(format!("{name} has no {direction} speed")));
        let speed_forward = speed_limit("forward")?;
        let speed_backward = speed_limit("backward")?;
        let rotation_speed = child_element(&vehicle, "chassis")
            .and_then(|chassis| chassis.iter_children_all().next())
            .and_then(|(_, chassis)| child_float(chassis.as_element()?, "rotationSpeed"))
            .map(f32::to_radians)
            .ok_or_else(|| invalid(format!("{name} has no chassis rotation speed")))?;

        // The vehicle may set its own values for a shared gun.
        let gun_float = |path: &[&str]| [child_element(guns, &gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| child_float_at(gun, path))
            .ok_or_else(|| invalid(format!("no {} for gun {gun_name}", path.join("/"))));
        let chassis_float = |path: &[&str]| child_element(&vehicle, "chassis")
            .and_then(|chassis| chassis.iter_children_all().next())
            .and_then(|(_, chassis)| child_float_at(chassis.as_element()?, path))
            .ok_or_else(|| invalid(format!("{name} has no chassis {}", path.join("/"))));
        let pitch_curve = |limit: &str| [child_element(guns, &gun_name), Some(gun_def)].into_iter().flatten()
            .find_map(|gun| child_string(child_element(gun, "pitchLimits")?, limit))
            .and_then(PitchCurve::parse)
            .ok_or_else(|| invalid(format!("no valid {limit} for gun {gun_name}")));
        // The first shell's flight, scaled as every shell is, see `gun_readers.readShot`.
        let first_shot = child_element(gun_def, "shots")
            .and_then(|shots| shots.iter_children_all().next())
            .and_then(|(_, shot)| shot.as_element())
            .ok_or_else(|| invalid(format!("gun {gun_name} has no shot")))?;
        let shot_float = |name: &str| child_float(first_shot, name)
            .ok_or_else(|| invalid(format!("gun {gun_name}'s first shot has no {name}")));

        let aiming = AimingDef {
            turret_rotation_speed: child_float(turret, "rotationSpeed")
                .ok_or_else(|| invalid(format!("{name} has no turret rotation speed")))?
                .to_radians(),
            gun_rotation_speed: gun_float(&["rotationSpeed"])?.to_radians(),
            aiming_time: gun_float(&["aimingTime"])?,
            dispersion_turret_rotation: gun_float(&["shotDispersionFactors", "turretRotation"])?.to_degrees(),
            dispersion_after_shot: gun_float(&["shotDispersionFactors", "afterShot"])?,
            dispersion_movement: chassis_float(&["shotDispersionFactors", "vehicleMovement"])? * 3.6,
            dispersion_rotation: chassis_float(&["shotDispersionFactors", "vehicleRotation"])?.to_degrees(),
            min_pitch: pitch_curve("minPitch")?,
            max_pitch: pitch_curve("maxPitch")?,
            dispersion_angle: (gun_float(&["shotDispersionRadius"])? / 100.0).atan(),
            shot_speed: shot_float("speed")? * projectile_speed_factor,
            shot_gravity: shot_float("gravity")? * projectile_speed_factor * projectile_speed_factor,
            reload_time: gun_float(&["reloadTime"])?,
            turret_position: child_element(&vehicle, "chassis")
                .and_then(|chassis| chassis.iter_children_all().next())
                .and_then(|(_, chassis)| child_vec3(chassis.as_element()?, "hullPosition"))
                .ok_or_else(|| invalid(format!("{name} has no hull position")))?
                + child_element(&vehicle, "hull")
                .and_then(|hull| child_element(hull, "turretPositions"))
                .and_then(|positions| child_vec3(positions, "turret"))
                .ok_or_else(|| invalid(format!("{name} has no turret position")))?,
            gun_position: child_vec3(turret, "gunPosition")
                .ok_or_else(|| invalid(format!("{name} has no gun position")))?,
        };

        // The gun's hit tester is the vehicle's own for it, or the shared gun's.
        let chassis = child_element(&vehicle, "chassis")
            .and_then(|chassis| chassis.iter_children_all().next())
            .and_then(|(_, chassis)| chassis.as_element());
        let gun = [child_element(guns, &gun_name), Some(gun_def)].into_iter().flatten()
            .find(|gun| child_element(gun, "hitTester").is_some());
        let parts = [("chassis", chassis), ("hull", child_element(&vehicle, "hull")), ("turret", Some(turret)), ("gun", gun)];
        let mut models = Vec::new();
        for (part, module) in parts {
            let path = module.and_then(|module| child_string(child_element(module, "hitTester")?, "collisionModelClient"))
                .ok_or_else(|| invalid(format!("{name} has no {part} collision model")))?;
            // The definitions name a `.model`, the game ships it as a `.havok`.
            let path = path.strip_suffix(".model").map_or_else(|| path.to_string(), |stem| format!("{stem}.havok"));
            models.push(CollisionModel::load(fs, &path)?);
        }
        let collision = Arc::new(VehicleCollision {
            parts: models.try_into().unwrap(),
            hull_position: chassis.and_then(|chassis| child_vec3(chassis, "hullPosition"))
                .ok_or_else(|| invalid(format!("{name} has no hull position")))?,
        });

        let crew_roles = child_element(&vehicle, "crew")
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

        Ok(Self { name: name.to_string(), descr, crew_roles, shells, shots, max_ammo, max_health, speed_forward, speed_backward, rotation_speed, aiming, collision })

    }

    pub fn turret_int_descr(&self) -> IntCompactDescr {
        IntCompactDescr { item_type: item_type::TURRET, nation: self.descr.nation, id: self.descr.turret }
    }

    pub fn gun_int_descr(&self) -> IntCompactDescr {
        IntCompactDescr { item_type: item_type::GUN, nation: self.descr.nation, id: self.descr.gun }
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

fn read_xml(fs: &ResFilesystem, path: &str) -> io::Result<Box<Element>> {
    let file = fs.read(path).map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
    pxml::from_reader(io::BufReader::new(file)).map_err(|e| invalid(format!("{path}: {e}")))
}

fn child_element<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.get_child(name)?.as_element()
}

/// A child's integer value, the child holding it directly or as its own value.
fn child_int(element: &Element, name: &str) -> Option<i64> {
    match element.get_child(name)? {
        pxml::Value::Element(child) => child.value.as_integer(),
        value => value.as_integer().or_else(|| value.as_string()?.trim().parse().ok()),
    }
}

fn child_string<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    match element.get_child(name)? {
        pxml::Value::Element(child) => child.value.as_string(),
        value => value.as_string(),
    }
}

fn child_float(element: &Element, name: &str) -> Option<f32> {
    element.get_child(name)?.as_float()
}

/// A child holding three floats, as a packed vector or as space-separated text.
fn child_vec3(element: &Element, name: &str) -> Option<Vec3> {
    let value = match element.get_child(name)? {
        pxml::Value::Element(child) => &child.value,
        value => value,
    };
    let floats: Vec<f32> = match value {
        pxml::Value::Vector(vector) => (0..vector.len()).filter_map(|i| vector.get(i)).collect(),
        pxml::Value::String(text) => text.split_whitespace().filter_map(|n| n.parse().ok()).collect(),
        _ => return None,
    };
    match floats[..] {
        [x, y, z] => Some(Vec3::new(x, y, z)),
        _ => None,
    }
}

/// A float down a path of children.
fn child_float_at(element: &Element, path: &[&str]) -> Option<f32> {
    let (last, parents) = path.split_last()?;
    let parent = parents.iter().try_fold(element, |element, name| child_element(element, name))?;
    child_float(parent, last)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
