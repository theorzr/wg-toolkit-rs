//! Vehicle definitions, read from the game's `scripts/item_defs/vehicles`: per nation,
//! `list.xml` gives each vehicle's type id, `<vehicle>.xml` its modules by name, and
//! `components/<kind>.xml` the modules' ids (in an `ids` section) and the shared
//! modules' definitions (in a `shared` section), guns with their shells. `components/shells.xml` gives each
//! shell's id.

use std::io;

use wgtk::pxml::{self, Element};
use wgtk::res::fs::ResFilesystem;

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
    pub max_ammo: u32,
    /// Hull plus default turret, before any battle modifier.
    pub max_health: u32,
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
        let mut shells = Vec::new();
        for (shell_name, shot) in child_element(gun_def, "shots").into_iter().flat_map(|shots| shots.iter_children_all()) {
            let shell_id = child_element(&shells_components, shell_name)
                .and_then(|shell| child_int(shell, "id"))
                .ok_or_else(|| invalid(format!("no id for shell {shell_name}")))?;
            let portion = shot.as_element().and_then(|shot| child_float(shot, "defaultPortion")).unwrap_or(0.0);
            let cd = IntCompactDescr { item_type: item_type::SHELL, nation: nation_id, id: shell_id as u16 };
            shells.push((cd, portion));
        }

        let max_health = child_element(&vehicle, "hull").and_then(|hull| child_int(hull, "maxHealth"))
            .zip(child_int(turret, "maxHealth"))
            .map(|(hull, turret)| (hull + turret) as u32)
            .ok_or_else(|| invalid(format!("{name} has no health")))?;

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

        Ok(Self { name: name.to_string(), descr, crew_roles, shells, max_ammo, max_health })

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

fn child_float(element: &Element, name: &str) -> Option<f32> {
    element.get_child(name)?.as_float()
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
