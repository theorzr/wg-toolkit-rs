//! Item prices, read from the game's definitions as the server reads them: the release
//! client doesn't, and takes them from the shop. Every vehicle, module, shell, optional
//! device and equipment has a price, in a single currency, and may not be in the shop.
//!
//! Vehicles are priced in their nation's `list.xml`, shared modules in the
//! `components/<kind>.xml` defining them, the others in the vehicle defining them, and
//! shells in `components/shells.xml`. See `_readPriceForItem` in `items/vehicles.py`.

use std::collections::HashMap;
use std::io;

use wgtk::pxml::{self, Element};
use wgtk::res::fs::ResFilesystem;

use super::item_defs::{self, NATIONS};
use super::items::{item_type, IntCompactDescr};


/// An item's price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price {
    /// `credits`, `gold`, `crystal`, `xp`, `freeXP` or `equipCoin`.
    pub currency: &'static str,
    pub amount: i64,
}

/// The prices of every item, and the items not in the shop.
#[derive(Debug, Default)]
pub struct ItemPrices {
    pub prices: Vec<(IntCompactDescr, Price)>,
    pub not_in_shop: Vec<IntCompactDescr>,
}

/// The currencies a price can be in other than credits, as `_xml.readPrice` reads them.
const CURRENCIES: &[&str] = &["gold", "crystal", "xp", "freeXP", "equipCoin"];

/// The module sections of a vehicle, with the components defining them and their type.
/// Guns are in their turret.
const MODULES: &[(&str, &str, u8)] = &[
    ("chassis", "chassis", item_type::CHASSIS),
    ("engines", "engines", item_type::ENGINE),
    ("fuelTanks", "fuelTanks", item_type::FUEL_TANK),
    ("radios", "radios", item_type::RADIO),
    ("turrets0", "turrets", item_type::TURRET),
];

/// The nation of optional devices and equipment, `nations.NONE_INDEX`.
const NO_NATION: u8 = 15;

impl ItemPrices {

    pub fn load(fs: &ResFilesystem) -> io::Result<Self> {

        let mut prices = Self::default();

        for (nation, nation_name) in NATIONS.iter().enumerate() {

            let nation = nation as u8;
            let dir = format!("scripts/item_defs/vehicles/{nation_name}");

            // Each kind's ids by module name, guns included, and its shared modules.
            let mut ids = HashMap::new();
            for kind in MODULES.iter().map(|&(_, kind, item_type)| (kind, item_type)).chain([("guns", item_type::GUN)]) {
                let components = item_defs::read_xml(fs, &format!("{dir}/components/{}.xml", kind.0))?;
                let kind_ids = components.get_child_element("ids").into_iter()
                    .flat_map(|ids| ids.iter_children_all())
                    .filter_map(|(name, id)| Some((name.clone(), id.as_integer()? as u16)))
                    .collect::<HashMap<_, _>>();
                for (name, module) in elements(components.get_child_element("shared")) {
                    if let Some(&id) = kind_ids.get(name) {
                        prices.add(IntCompactDescr { item_type: kind.1, nation, id }, module);
                    }
                }
                ids.insert(kind.0, kind_ids);
            }

            let module = |kind: &str, name: &str, item_type| -> io::Result<IntCompactDescr> {
                let id = ids[kind].get(name).copied()
                    .ok_or_else(|| item_defs::invalid(format!("no id for {kind} {name} in {nation_name}")))?;
                Ok(IntCompactDescr { item_type, nation, id })
            };

            // Vehicles, and the modules they define themselves rather than share.
            let list = item_defs::read_xml(fs, &format!("{dir}/list.xml"))?;
            for (name, vehicle) in elements(Some(&list)) {
                let id = vehicle.get_child_integer("id")
                    .ok_or_else(|| item_defs::invalid(format!("no id for vehicle {name} in {nation_name}")))?;
                prices.add(IntCompactDescr { item_type: item_type::VEHICLE, nation, id: id as u16 }, vehicle);
                let vehicle = item_defs::read_xml(fs, &format!("{dir}/{name}.xml"))?;
                for &(section, kind, item_type) in MODULES {
                    for (module_name, def) in local(vehicle.get_child_element(section)) {
                        prices.add(module(kind, module_name, item_type)?, def);
                    }
                }
                for (_, turret) in elements(vehicle.get_child_element("turrets0")) {
                    for (gun_name, gun) in local(turret.get_child_element("guns")) {
                        prices.add(module("guns", gun_name, item_type::GUN)?, gun);
                    }
                }
            }

            let shells = item_defs::read_xml(fs, &format!("{dir}/components/shells.xml"))?;
            for (_, shell) in elements(Some(&shells)) {
                if let Some(id) = shell.get_child_integer("id") {
                    prices.add(IntCompactDescr { item_type: item_type::SHELL, nation, id: id as u16 }, shell);
                }
            }

        }

        for (file, item_type) in [("optional_devices", item_type::OPTIONAL_DEVICE), ("equipments", item_type::EQUIPMENT)] {
            let artefacts = item_defs::read_xml(fs, &format!("scripts/item_defs/vehicles/common/{file}.xml"))?;
            for (_, artefact) in elements(Some(&artefacts)) {
                if let Some(id) = artefact.get_child_integer("id") {
                    prices.add(IntCompactDescr { item_type, nation: NO_NATION, id: id as u16 }, artefact);
                }
            }
        }

        Ok(prices)

    }

    /// Add an item from its definition, which prices it in credits unless it says
    /// otherwise, free if it says nothing.
    fn add(&mut self, cd: IntCompactDescr, def: &Element) {
        // The price is an element only to hold its currency's marker.
        let (amount, currency) = match def.get_child("price") {
            Some(pxml::Value::Element(price)) => (price.value.as_integer(), CURRENCIES.iter().copied().find(|&currency| price.get_child(currency).is_some())),
            Some(price) => (price.as_integer(), None),
            None => (None, None),
        };
        self.prices.push((cd, Price { currency: currency.unwrap_or("credits"), amount: amount.unwrap_or(0) }));
        if def.get_child_boolean("notInShop") == Some(true) {
            self.not_in_shop.push(cd);
        }
    }

}

/// The child elements of a section, if any.
fn elements(section: Option<&Element>) -> impl Iterator<Item = (&str, &Element)> {
    section.into_iter().flat_map(Element::iter_child_elements)
}

/// The modules a vehicle's section defines, without those it shares: these are only
/// named `shared`, with what the vehicle changes of them if anything.
fn local(section: Option<&Element>) -> impl Iterator<Item = (&str, &Element)> {
    elements(section).filter(|(_, module)| module.value.as_string().map(str::trim) != Some("shared"))
}
