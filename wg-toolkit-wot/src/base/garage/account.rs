//! The account data: everything the client knows about the account (inventory, stats,
//! quests...), sent as a resource in answer to `CMD_SYNC_DATA`, as a zlib-compressed
//! pickled dict. The client merges it in `Account._update`: a key `(name, '_r')`
//! replaces the cache entry `name` whole, a plain `name` is merged into it. A full sync
//! (the first, with revision 0) carries the `rev` it brings the client to; later syncs
//! give `prevRev` and `rev` in the `onCmdResponseExt` answer instead.
//!
//! Only the sections the client needs are given: the inventory, the stats with the
//! money, the personal missions and the cache allowing to spend it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;

use flate2::write::ZlibEncoder;
use flate2::Compression;

use wgtk::pickle::{self, Value};
use wgtk::res::fs::ResFilesystem;
use wgtk::pxml::Element;

use crate::game::items::{item_type, IntCompactDescr, TankmanDescr, VehicleDescr};
use crate::game::item_defs::{self, VehicleDef, NATIONS};


/// The money of every account, until accounts are stored.
const CREDITS: i64 = 100_000_000;
const GOLD: i64 = 1_000_000;
const FREE_XP: i64 = 1_000_000;
/// The garage slots and barracks berths of every account, plenty.
const SLOTS: i64 = 100;
const BERTHS: i64 = 100;
/// `VEHICLE_SETTINGS_FLAG`: auto repair, auto load, auto equip.
const VEHICLE_SETTINGS: i64 = 14;

/// A player's account, standing in for its stored one.
#[derive(Debug, Clone)]
pub struct AccountData {
    /// Its vehicles, whose inventory ids are their indices from 1.
    pub vehicles: Vec<Vehicle>,
}

/// A vehicle of the account, as bought: the default modules and ammunition, no
/// equipment, and a new crew trained for it (full role experience, no skills).
#[derive(Debug, Clone)]
pub struct Vehicle {
    pub descr: VehicleDescr,
    /// Its crew, in crew order, whose inventory ids follow those of the crews of the
    /// vehicles before.
    pub crew: Vec<TankmanDescr>,
    /// Its shells, by integer descriptor, and how many of each.
    pub shells: Vec<(IntCompactDescr, u32)>,
}

impl AccountData {

    /// An account with the given vehicles.
    pub fn new(fs: &ResFilesystem, defs: &[VehicleDef]) -> io::Result<Self> {
        let mut passports = HashMap::new();
        let mut vehicles = Vec::new();
        for def in defs {
            let nation = def.descr.nation;
            if !passports.contains_key(&nation) {
                passports.insert(nation, Passports::load(fs, nation)?);
            }
            let passports = &passports[&nation];
            let crew = def.crew_roles.iter().enumerate().map(|(i, &role)| TankmanDescr {
                nation,
                flags: vec![0],
                vehicle_type_id: def.descr.type_id,
                role,
                skills_efficiency_xp: 100_000,
                skills: Vec::new(),
                last_skill_level: 0,
                free_skills: 0,
                bonus_skills: Vec::new(),
                first_name_id: passports.first_names[i % passports.first_names.len()],
                last_name_id: passports.last_names[i % passports.last_names.len()],
                icon_id: passports.icons[i % passports.icons.len()],
                free_xp: 0,
                dossier: Vec::new(),
            }).collect();
            vehicles.push(Vehicle { descr: def.descr.clone(), crew, shells: def.default_ammo() });
        }
        Ok(Self { vehicles })
    }

    /// A vehicle, by inventory id.
    pub fn vehicle(&self, vehicle_id: i64) -> Option<&Vehicle> {
        self.vehicles.get(usize::try_from(vehicle_id).ok()?.checked_sub(1)?)
    }

    /// Each vehicle with its inventory id and its crew's.
    fn vehicles(&self) -> impl Iterator<Item = (i64, Vec<i64>, &Vehicle)> {
        let mut next_tankman_id = 1;
        self.vehicles.iter().zip(1..).map(move |(vehicle, id)| {
            let crew = (next_tankman_id..).take(vehicle.crew.len()).collect();
            next_tankman_id += vehicle.crew.len() as i64;
            (id, crew, vehicle)
        })
    }

    /// The data of a full sync, bringing the client to the given revision.
    pub fn data(&self, revision: i64) -> Value {
        wgtk::pickle!({
            "rev": revision,
            "inventory": (self.inventory()),
            "stats": (self.stats()),
            "potapovQuests": (potapov_quests()),
            "cache": (cache()),
        })
    }

    /// The items the account has, by item type: the vehicles' tables of their state by
    /// inventory id, the tankmen's likewise, the crew skins' tables, and the other
    /// items' counts by integer descriptor, of which there are none.
    fn inventory(&self) -> Value {

        let mut vehicles = BTreeMap::<&str, BTreeMap<Value, Value>>::new();
        let mut tankmen = BTreeMap::<&str, BTreeMap<Value, Value>>::new();
        // The client reads these tables of every vehicle, which have no entry when empty.
        for table in ["customRoleSlots", "customizationExpiryTime", "enhancements", "igrCustomizationLayout",
            "layoutIndexes", "postProgression", "rent", "repair"] {
            vehicles.insert(table, BTreeMap::new());
        }

        for (id, crew, vehicle) in self.vehicles() {
            let shells = vehicle.shells_value();
            let layout_key = wgtk::pickle!(((vehicle.descr.turret_int_descr().to_int()), (vehicle.descr.gun_int_descr().to_int())));
            let crew_ids = Value::List(crew.iter().copied().map(Value::from).collect());
            let empty_layout = || wgtk::pickle!([[]]);
            for (table, value) in [
                ("compDescr", Value::Bytes(vehicle.descr.encode())),
                ("crew", crew_ids.clone()),
                ("lastCrew", crew_ids),
                ("shells", shells.clone()),
                ("shellsLayout", Value::Dict(BTreeMap::from([(layout_key, Value::List(vec![shells]))]))),
                ("eqs", wgtk::pickle!([])),
                ("eqsLayout", empty_layout()),
                ("devicesLayout", empty_layout()),
                ("boosters", wgtk::pickle!([])),
                ("boostersLayout", empty_layout()),
                ("settings", Value::from(VEHICLE_SETTINGS)),
                ("extraSettings", Value::from(0)),
            ] {
                vehicles.entry(table).or_default().insert(Value::from(id), value);
            }
            for (&tankman_id, tankman) in crew.iter().zip(&vehicle.crew) {
                tankmen.entry("compDescr").or_default().insert(Value::from(tankman_id), Value::Bytes(tankman.encode()));
                tankmen.entry("vehicle").or_default().insert(Value::from(tankman_id), Value::from(id));
            }
        }

        let tables = |tables: BTreeMap<&str, BTreeMap<Value, Value>>| {
            Value::dict(tables.into_iter().map(|(name, table)| (name, Value::Dict(table))))
        };
        let mut inventory = BTreeMap::from([
            (Value::from(item_type::VEHICLE as i64), tables(vehicles)),
            (Value::from(item_type::TANKMAN as i64), tables(tankmen)),
            // `SkinInvData`: the skins by id, and those worn by tankman, which the
            // client reads without a default.
            (Value::from(item_type::CREW_SKIN as i64), wgtk::pickle!({1: {}, 2: {}})),
            (Value::from("enhancements"), wgtk::pickle!({})),
            (Value::from("telecomOrders"), wgtk::pickle!({})),
        ]);
        for item_type in item_type::CHASSIS..=item_type::CREW_BOOK {
            inventory.entry(Value::from(item_type as i64)).or_insert_with(|| wgtk::pickle!({}));
        }
        Value::Dict(inventory)

    }

    /// The account's money, its research (the vehicles and their modules) and its
    /// limits, a new account's.
    fn stats(&self) -> Value {
        let unlocks = self.vehicles.iter()
            .flat_map(|vehicle| [vehicle.descr.int_descr()].into_iter().chain(vehicle.descr.module_int_descrs()))
            .map(|cd| Value::from(cd.to_int()));
        let set = |items: BTreeSet<Value>| Value::Set(items);
        wgtk::pickle!({
            "credits": CREDITS,
            "gold": GOLD,
            "crystal": 0,
            "freeXP": FREE_XP,
            "equipCoin": 0,
            "eventCoin": 0,
            "bpcoin": 0,
            "slots": SLOTS,
            "berths": BERTHS,
            "unlocks": (set(unlocks.collect())),
            "eliteVehicles": (set(BTreeSet::new())),
            "multipliedXPVehs": (set(BTreeSet::new())),
            "multipliedRankedBattlesVehs": (set(BTreeSet::new())),
            "vehTypeXP": {},
            "vehTypeLocks": {},
            "globalVehicleLocks": {},
            "maxResearchedLevelByNation": {},
            "prestigeMilestonesAchieved": {},
            "weeklyVehicleCrystals": {},
            "restrictions": {},
            "dossier": (Value::Bytes(Vec::new())),
            "clanInfo": None,
            "language": "en",
            "XPpp": 0.0,
            "accOnline": 0,
            "accOffline": 0,
            "applyAdditionalXPCount": 0,
            "applyAdditionalWoTPlusXPCount": 0,
            "dailyPlayHours": [0],
            "playLimits": ((172800, ""), (1209600, "")),
            "dummySessionStats": {
                "base": {"credits": 0, "xp": 0},
                "premium": {"credits": 0, "xp": 0},
                "totalDailyAppliedAdditionalXP": 0,
            },
            "denunciationsLeft": 10,
            "finPswdAttemptsLeft": 5,
            "hasFinPassword": false,
            "freeTMenLeft": 0,
            "freeVehiclesLeft": 0,
            "vehicleSellsLeft": 5,
            "fortResource": 0,
            "tkillIsSuspected": false,
        })
    }

}

impl Vehicle {

    /// The vehicle's setups, as the battle gives them to its vehicle: its loadouts of
    /// shells, equipment, optional devices and boosters, only of shells for now.
    pub fn setups(&self) -> Value {
        wgtk::pickle!({
            "boostersSetups": [[]],
            "devicesSetups": [[]],
            "eqsSetups": [[]],
            "shellsSetups": [(self.shells_value())],
        })
    }

    /// Its shells as the account lists them, `[shell, count, ...]`.
    fn shells_value(&self) -> Value {
        Value::List(self.shells.iter()
            .flat_map(|&(cd, count)| [Value::from(cd.to_int()), Value::from(count)])
            .collect())
    }

}

/// The personal missions of an account that has none.
fn potapov_quests() -> Value {
    let campaign = || wgtk::pickle!({
        "lastIDs": {},
        "rewards": ({}, {}),
        "selected": (Value::Set(BTreeSet::new())),
        "slots": 3,
    });
    wgtk::pickle!({
        "activeCampaigns": [],
        // The progress of no quest.
        "compDescr": (Value::Bytes(Vec::new())),
        "lastActiveOperationID": None,
        "regular": (campaign()),
        "pm2": (campaign()),
        "pm3": (campaign()),
        "pm4": (campaign()),
        "tiles": (Value::Set(BTreeSet::new())),
    })
}

/// What the client caches of the account's state on the server, notably that it may
/// spend its money.
fn cache() -> Value {
    wgtk::pickle!({
        "isResourcesConsumptionAllowed": true,
        "isEmergencyModeEnabled": false,
        "isFinPswdVerified": false,
        "isSsrPlayEnabled": false,
        "SPA": {},
        "activeOrders": {},
        "clanFortState": 0,
        "clanMembers": {},
        "comp7": {"isOnline": false, "qualification": {"battles": [], "state": "in_progress"}},
        "dynamicCurrencies": {},
        "entitlements": {},
        "gameRestrictions": {"payment": {}, "privateChat": {}, "publicChat": {}, "session": {}, "settings": {}},
        "giftsData": {"isReady": false},
        "potapovQuestIDs": {},
        "vehsLock": {},
    })
}

/// The ids of the names and portraits a nation's tankmen are given: those of its first
/// normal group of men, which the client checks theirs against.
#[derive(Debug)]
struct Passports {
    first_names: Vec<u16>,
    last_names: Vec<u16>,
    icons: Vec<u16>,
}

impl Passports {

    fn load(fs: &ResFilesystem, nation: u8) -> io::Result<Self> {
        let nation_name = NATIONS.get(nation as usize)
            .ok_or_else(|| item_defs::invalid(format!("no nation {nation}")))?;
        let config = item_defs::read_xml(fs, &format!("scripts/item_defs/tankmen/{nation_name}.xml"))?;
        let group = config.get_child_element("normalGroups").into_iter()
            .flat_map(Element::iter_child_elements)
            .map(|(_, group)| group)
            .find(|group| group.get_child_string("sex").map(str::trim) == Some("male")
                && group.get_child_boolean("notInShop") != Some(true))
            .ok_or_else(|| item_defs::invalid(format!("no group of men among the {nation_name} tankmen")))?;
        // Ids are named `_<id>`.
        let ids = |name: &str| -> io::Result<Vec<u16>> {
            let ids = group.get_child_element(name).into_iter()
                .flat_map(Element::iter_children_all)
                .filter_map(|(key, _)| key.strip_prefix('_')?.parse().ok())
                .collect::<Vec<_>>();
            if ids.is_empty() {
                return Err(item_defs::invalid(format!("no {name} for the {nation_name} tankmen")));
            }
            Ok(ids)
        };
        Ok(Self { first_names: ids("firstNames")?, last_names: ids("lastNames")?, icons: ids("icons")? })
    }

}

/// A value as streamed: pickled, then zlib-compressed.
pub fn compress(value: &Value) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    pickle::to_writer(&mut encoder, value)?;
    encoder.finish()
}

/// The description of a resource stream: the pickled `(len, crc32)` the client checks
/// the data against (`game.onStreamComplete`), the CRC signed as Python 2's
/// `zlib.crc32` gives it.
pub fn resource_description(data: &[u8]) -> Vec<u8> {
    let crc32 = crc32fast::hash(data) as i32;
    let description = Value::Tuple(vec![Value::I64(data.len() as i64), Value::I64(crc32.into())]);
    pickle::to_bytes(&description)
}
