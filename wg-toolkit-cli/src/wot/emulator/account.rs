//! The account data: everything the client knows about the account (inventory, stats,
//! quests, server settings...), sent as a resource in answer to `CMD_SYNC_DATA`, as a
//! zlib-compressed pickled dict. The client merges it in `Account._update`: a key
//! `(name, '_r')` replaces the cache entry `name` whole, a plain `name` is merged into
//! it. A full sync (the first, with revision 0) carries the `rev` it brings the client
//! to; later syncs give `prevRev` and `rev` in the `onCmdResponseExt` answer instead.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde_pickle::{HashableValue, Value};
use wgtk::util::io::{serde_pickle_de_options, serde_pickle_ser_options};

use crate::wot::item_defs::VehicleDef;
use crate::wot::items::{item_type, TankmanDescr};


/// The account data, as the pickled dict it is.
#[derive(Debug, Clone)]
pub struct AccountData {
    pub data: Value,
}

impl AccountData {

    /// Decode a sync resource's data.
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        let mut raw = Vec::new();
        ZlibDecoder::new(data).read_to_end(&mut raw)?;
        let data = serde_pickle::value_from_slice(&raw, serde_pickle_de_options())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(Self { data })
    }

    /// Encode as a sync resource: its description, then its data.
    pub fn encode(&self) -> io::Result<(Vec<u8>, Vec<u8>)> {
        let raw = serde_pickle::value_to_vec(&self.data, serde_pickle_ser_options())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        let data = encoder.finish()?;
        Ok((resource_description(&data)?, data))
    }

    /// The revision a full sync brings the client to.
    pub fn revision(&self) -> Option<i64> {
        match self.get(&[key("rev")])? {
            Value::I64(rev) => Some(*rev),
            _ => None,
        }
    }

    /// The compact descriptors of an item type's inventory entries, by inventory id:
    /// `inventory[item_type]['compDescr']`.
    pub fn compact_descrs(&self, item_type: u8) -> impl Iterator<Item = (i64, &[u8])> + '_ {
        let descrs = self.get(&[key("inventory"), HashableValue::I64(item_type.into()), key("compDescr")]);
        let descrs = match descrs {
            Some(Value::Dict(descrs)) => Some(descrs),
            _ => None,
        };
        descrs.into_iter().flatten().filter_map(|(id, cd)| match (id, cd) {
            (HashableValue::I64(id), Value::Bytes(cd)) => Some((*id, &cd[..])),
            _ => None,
        })
    }

    /// Give the account a vehicle, as bought: the default modules and ammunition, no
    /// equipment, and a new crew trained for it (full role experience, no skills).
    /// Returns its inventory id.
    pub fn add_vehicle(&mut self, def: &VehicleDef) -> io::Result<i64> {

        let next_id = |item_type| self.compact_descrs(item_type).map(|(id, _)| id).max().unwrap_or(0) + 1;
        let vehicle_id = next_id(item_type::VEHICLE);
        let first_tankman_id = next_id(item_type::TANKMAN);

        // The client checks names and portraits against its nation's tables, so take
        // valid ones from the account's other tankmen of that nation.
        let names: Vec<_> = self.compact_descrs(item_type::TANKMAN)
            .filter_map(|(_, cd)| TankmanDescr::decode(cd).ok())
            .filter(|tankman| tankman.nation == def.descr.nation)
            .map(|tankman| (tankman.first_name_id, tankman.last_name_id, tankman.icon_id))
            .take(def.crew_roles.len())
            .collect();
        if names.len() < def.crew_roles.len() {
            return Err(invalid("not enough tankmen of the vehicle's nation to name a crew after"));
        }

        let crew: Vec<i64> = (first_tankman_id..).take(def.crew_roles.len()).collect();
        let tankmen = self.section_mut(&[key("inventory"), HashableValue::I64(item_type::TANKMAN.into())])?;
        for ((&id, &role), &(first_name_id, last_name_id, icon_id)) in crew.iter().zip(&def.crew_roles).zip(&names) {
            let tankman = TankmanDescr {
                nation: def.descr.nation,
                flags: vec![0],
                vehicle_type_id: def.descr.type_id,
                role,
                skills_efficiency_xp: 100_000,
                skills: Vec::new(),
                last_skill_level: 0,
                free_skills: 0,
                bonus_skills: Vec::new(),
                first_name_id,
                last_name_id,
                icon_id,
                free_xp: 0,
                dossier: Vec::new(),
            };
            table(tankmen, "compDescr").insert(HashableValue::I64(id), Value::Bytes(tankman.encode()));
            table(tankmen, "vehicle").insert(HashableValue::I64(id), Value::I64(vehicle_id));
        }

        let shells: Vec<Value> = def.default_ammo().into_iter()
            .flat_map(|(cd, count)| [Value::I64(cd.to_int()), Value::I64(count.into())])
            .collect();
        let layout_key = HashableValue::Tuple(vec![
            HashableValue::I64(def.turret_int_descr().to_int()),
            HashableValue::I64(def.gun_int_descr().to_int()),
        ]);
        let crew = Value::List(crew.into_iter().map(Value::I64).collect());
        let empty_layout = || Value::List(vec![Value::List(Vec::new())]);

        let vehicles = self.section_mut(&[key("inventory"), HashableValue::I64(item_type::VEHICLE.into())])?;
        let id = HashableValue::I64(vehicle_id);
        let mut set = |name: &str, value: Value| table(vehicles, name).insert(id.clone(), value);
        set("compDescr", Value::Bytes(def.descr.encode()));
        set("crew", crew.clone());
        set("lastCrew", crew);
        set("shells", Value::List(shells.clone()));
        set("shellsLayout", Value::Dict(BTreeMap::from([(layout_key, Value::List(vec![Value::List(shells)]))])));
        set("eqs", Value::List(Vec::new()));
        set("eqsLayout", empty_layout());
        set("devicesLayout", empty_layout());
        set("boosters", Value::List(Vec::new()));
        set("boostersLayout", empty_layout());
        // `VEHICLE_SETTINGS_FLAG`: auto repair, auto load, auto equip.
        set("settings", Value::I64(14));
        set("extraSettings", Value::I64(0));

        Ok(vehicle_id)

    }

    /// The dict at a path of dict keys.
    fn section_mut(&mut self, path: &[HashableValue]) -> io::Result<&mut BTreeMap<HashableValue, Value>> {
        path.iter().try_fold(&mut self.data, |value, k| match value {
            Value::Dict(dict) => dict.get_mut(k),
            _ => None,
        }).and_then(|value| match value {
            Value::Dict(dict) => Some(dict),
            _ => None,
        }).ok_or_else(|| invalid("missing account data section"))
    }

    /// The value at a path of dict keys.
    pub fn get(&self, path: &[HashableValue]) -> Option<&Value> {
        path.iter().try_fold(&self.data, |value, k| match value {
            Value::Dict(dict) => dict.get(k),
            _ => None,
        })
    }

    pub fn dict(&self) -> Option<&BTreeMap<HashableValue, Value>> {
        match &self.data {
            Value::Dict(dict) => Some(dict),
            _ => None,
        }
    }

}

/// A dict key as the server writes them: Python 2 `str`, so bytes.
pub fn key(name: &str) -> HashableValue {
    HashableValue::Bytes(name.as_bytes().to_vec())
}

/// The description of a resource stream: the pickled `(len, crc32)` the client checks
/// the data against (`game.onStreamComplete`), the CRC signed as Python 2's
/// `zlib.crc32` gives it.
pub fn resource_description(data: &[u8]) -> io::Result<Vec<u8>> {
    let crc32 = crc32fast::hash(data) as i32;
    let description = Value::Tuple(vec![Value::I64(data.len() as i64), Value::I64(crc32.into())]);
    serde_pickle::value_to_vec(&description, serde_pickle_ser_options())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// A section's table by name, created empty if missing.
fn table<'a>(section: &'a mut BTreeMap<HashableValue, Value>, name: &str) -> &'a mut BTreeMap<HashableValue, Value> {
    let table = section.entry(key(name)).or_insert_with(|| Value::Dict(BTreeMap::new()));
    if !matches!(table, Value::Dict(_)) {
        *table = Value::Dict(BTreeMap::new());
    }
    match table {
        Value::Dict(dict) => dict,
        _ => unreachable!(),
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
