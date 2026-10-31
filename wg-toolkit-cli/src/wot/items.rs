//! The compact descriptors the game identifies items with, in the account data and in
//! commands: an integer for any item type (`makeIntCompactDescrByID`), and byte strings
//! for the items with a state of their own, vehicles and tankmen. See
//! `common/items/__init__.py`, `vehicles.py` and `tankmen.py` in the client scripts.

// Complete tables on purpose, like the account commands: not every item type or flag is
// used yet.
#![allow(dead_code)]

use std::fmt;


/// The item type ids, `ITEM_TYPE_NAMES` order.
pub mod item_type {
    pub const VEHICLE: u8 = 1;
    pub const CHASSIS: u8 = 2;
    pub const TURRET: u8 = 3;
    pub const GUN: u8 = 4;
    pub const ENGINE: u8 = 5;
    pub const FUEL_TANK: u8 = 6;
    pub const RADIO: u8 = 7;
    pub const TANKMAN: u8 = 8;
    pub const OPTIONAL_DEVICE: u8 = 9;
    pub const SHELL: u8 = 10;
    pub const EQUIPMENT: u8 = 11;
    pub const CUSTOMIZATION: u8 = 12;
    pub const CREW_SKIN: u8 = 13;
    pub const CREW_BOOK: u8 = 14;
    pub const PET: u8 = 15;
}

/// An item's integer compact descriptor: its type, nation and id within both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntCompactDescr {
    pub item_type: u8,
    pub nation: u8,
    pub id: u16,
}

impl IntCompactDescr {

    /// Types above 15 don't fit the low nibble and move to the high byte, leaving 0
    /// in the nibble.
    pub fn from_int(cd: i64) -> Self {
        let mut item_type = (cd & 15) as u8;
        if item_type == 0 {
            item_type = (cd >> 24 & 255) as u8;
        }
        Self { item_type, nation: (cd >> 4 & 15) as u8, id: (cd >> 8 & 0xFFFF) as u16 }
    }

    pub fn to_int(self) -> i64 {
        let base = (i64::from(self.id) << 8) + (i64::from(self.nation) << 4);
        if self.item_type <= 15 {
            base + i64::from(self.item_type)
        } else {
            base + (i64::from(self.item_type) << 24)
        }
    }

}

/// A malformed compact descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescrError(&'static str);

impl fmt::Display for DescrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for DescrError {}

/// Reads a descriptor front to back.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {

    fn take(&mut self, len: usize) -> Result<&'a [u8], DescrError> {
        if self.0.len() < len {
            return Err(DescrError("descriptor too short"));
        }
        let (head, tail) = self.0.split_at(len);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, DescrError> {
        Ok(self.take(1)?[0])
    }

    fn u16_le(&mut self) -> Result<u16, DescrError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u16_be(&mut self) -> Result<u16, DescrError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

}

/// The vehicle header flag telling that the type id takes two bytes.
const VEHICLE_EXTENDED_TYPE_ID: u8 = 2;

/// A vehicle's compact descriptor (`VehicleDescr.makeCompactDescr`): its type and the
/// modules and optional devices mounted.
///
/// The number of turrets is a property of the vehicle type, which the descriptor
/// doesn't carry; every vehicle of the current game has one (turretless ones included),
/// so one is assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleDescr {
    pub nation: u8,
    pub type_id: u16,
    pub chassis: u16,
    pub engine: u16,
    pub fuel_tank: u16,
    pub radio: u16,
    pub turret: u16,
    pub gun: u16,
    /// Optional devices by slot, counted from the vehicle type's last slot: the
    /// descriptor numbers slots that way and the slot count belongs to the type, so
    /// slot `i` here is slot `count - 1 - i` of the type. Trailing `None`s aren't kept.
    pub optional_devices: Vec<Option<u16>>,
    /// The flags above the optional device slots and what they announce, kept as read:
    /// enhancements (16), emblems and inscriptions (32), a byte of unknown use (64),
    /// camouflages (128).
    pub extra_flags: u8,
    pub extra: Vec<u8>,
}

/// The optional device slots a vehicle descriptor's flags have room for.
const MAX_OPTIONAL_DEVICES_SLOTS: usize = 4;

impl VehicleDescr {

    pub fn int_descr(&self) -> IntCompactDescr {
        IntCompactDescr { item_type: item_type::VEHICLE, nation: self.nation, id: self.type_id }
    }

    pub fn decode(cd: &[u8]) -> Result<Self, DescrError> {

        let mut cur = Cursor(cd);
        let header = cur.u8()?;
        if header & 15 & !VEHICLE_EXTENDED_TYPE_ID != item_type::VEHICLE {
            return Err(DescrError("not a vehicle descriptor"));
        }

        let mut type_id = u16::from(cur.u8()?);
        if header & VEHICLE_EXTENDED_TYPE_ID != 0 {
            type_id += u16::from(cur.u8()?) << 8;
        }

        let chassis = cur.u16_le()?;
        let engine = cur.u16_le()?;
        let fuel_tank = cur.u16_le()?;
        let radio = cur.u16_le()?;
        let turret = cur.u16_le()?;
        let gun = cur.u16_le()?;

        // The writer shifts the slot bits left for each slot and prepends each device's
        // id, so the last slot is bit 0 and its device comes first.
        let flags = cur.u8()?;
        let slots = flags & 0xF;
        let mut optional_devices = Vec::new();
        for slot in 0..MAX_OPTIONAL_DEVICES_SLOTS {
            if slots >> slot == 0 {
                break;
            }
            optional_devices.push(if slots & (1 << slot) != 0 { Some(cur.u16_le()?) } else { None });
        }

        Ok(Self {
            nation: header >> 4 & 15,
            type_id,
            chassis,
            engine,
            fuel_tank,
            radio,
            turret,
            gun,
            optional_devices,
            extra_flags: flags & 0xF0,
            extra: cur.rest().to_vec(),
        })

    }

    pub fn encode(&self) -> Vec<u8> {

        let extended = self.type_id > 255;
        let mut cd = vec![(self.nation << 4) | item_type::VEHICLE | if extended { VEHICLE_EXTENDED_TYPE_ID } else { 0 }];
        cd.extend_from_slice(&self.type_id.to_le_bytes()[..if extended { 2 } else { 1 }]);
        for id in [self.chassis, self.engine, self.fuel_tank, self.radio, self.turret, self.gun] {
            cd.extend_from_slice(&id.to_le_bytes());
        }

        let mut slots = 0u8;
        let mut devices = Vec::new();
        for (slot, device) in self.optional_devices.iter().enumerate().take(MAX_OPTIONAL_DEVICES_SLOTS) {
            if let Some(id) = device {
                devices.extend_from_slice(&id.to_le_bytes());
                slots |= 1 << slot;
            }
        }

        cd.push(slots | self.extra_flags);
        cd.extend_from_slice(&devices);
        cd.extend_from_slice(&self.extra);
        cd

    }

}

/// The tankman flags (`TankmanFlags`), in the first flags byte.
pub mod tankman_flag {
    pub const EXTENDED_VEHICLE_TYPE_ID: u8 = 128;
    pub const IS_PREMIUM: u8 = 64;
    pub const IS_FEMALE: u8 = 32;
    pub const HAS_FREE_SKILLS: u8 = 16;
    pub const HAS_BONUS_SKILLS: u8 = 8;
    pub const DISABLE_FIRST_SKILL_RESET: u8 = 4;
    /// Another flags byte follows; none is defined yet.
    pub const MORE_FLAGS: u8 = 1;
}

/// A tankman's compact descriptor (`TankmanDescr.makeCompactDescr`). Skills and roles
/// are indices into `SKILL_NAMES` (`common/items/tankmen.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TankmanDescr {
    pub nation: u8,
    /// The flags bytes, see [`tankman_flag`]; the extended vehicle type id, free skills
    /// and bonus skills flags are recomputed when encoding.
    pub flags: Vec<u8>,
    /// The vehicle type the tankman is trained for.
    pub vehicle_type_id: u16,
    pub role: u8,
    /// The experience toward the role, on 17 bits.
    pub skills_efficiency_xp: u32,
    pub skills: Vec<u8>,
    pub last_skill_level: u8,
    pub free_skills: u8,
    pub bonus_skills: Vec<u8>,
    pub first_name_id: u16,
    pub last_name_id: u16,
    pub icon_id: u16,
    pub free_xp: u32,
    /// The tankman's dossier, its own compact format.
    pub dossier: Vec<u8>,
}

impl TankmanDescr {

    pub fn vehicle_int_descr(&self) -> IntCompactDescr {
        IntCompactDescr { item_type: item_type::VEHICLE, nation: self.nation, id: self.vehicle_type_id }
    }

    pub fn decode(cd: &[u8]) -> Result<Self, DescrError> {

        use tankman_flag::*;

        let mut cur = Cursor(cd);
        let header = cur.u8()?;
        if header & 15 != item_type::TANKMAN {
            return Err(DescrError("not a tankman descriptor"));
        }

        let mut flags = vec![cur.u8()?];
        while flags.last().unwrap() & MORE_FLAGS != 0 {
            flags.push(cur.u8()?);
        }

        let vehicle_type_id = if flags[0] & EXTENDED_VEHICLE_TYPE_ID != 0 {
            cur.u16_be()?
        } else {
            u16::from(cur.u8()?)
        };

        let rs = cur.take(3)?;
        let role = rs[0] >> 1;
        let skills_efficiency_xp = (u32::from(rs[0] & 1) << 16) | (u32::from(rs[1]) << 8) | u32::from(rs[2]);

        let num_skills = cur.u8()?;
        let skills = cur.take(num_skills.into())?.to_vec();
        let last_skill_level = cur.u8()?;
        let free_skills = if flags[0] & HAS_FREE_SKILLS != 0 { cur.u8()? } else { 0 };
        let bonus_skills = if flags[0] & HAS_BONUS_SKILLS != 0 {
            let num = cur.u8()?;
            cur.take(num.into())?.to_vec()
        } else {
            Vec::new()
        };

        let first_name_id = cur.u16_be()?;
        let last_name_id = cur.u16_be()?;
        let icon_id = cur.u16_be()?;
        let free_xp = u32::from_be_bytes(cur.take(4)?.try_into().unwrap());

        Ok(Self {
            nation: header >> 4 & 15,
            flags,
            vehicle_type_id,
            role,
            skills_efficiency_xp,
            skills,
            last_skill_level,
            free_skills,
            bonus_skills,
            first_name_id,
            last_name_id,
            icon_id,
            free_xp,
            dossier: cur.rest().to_vec(),
        })

    }

    pub fn encode(&self) -> Vec<u8> {

        use tankman_flag::*;

        let mut flags = self.flags.clone();
        if flags.is_empty() {
            flags.push(0);
        }
        let derived = EXTENDED_VEHICLE_TYPE_ID | HAS_FREE_SKILLS | HAS_BONUS_SKILLS;
        flags[0] &= !derived;
        if self.vehicle_type_id > 255 { flags[0] |= EXTENDED_VEHICLE_TYPE_ID; }
        if self.free_skills > 0 { flags[0] |= HAS_FREE_SKILLS; }
        if !self.bonus_skills.is_empty() { flags[0] |= HAS_BONUS_SKILLS; }

        let mut cd = vec![(self.nation << 4) | item_type::TANKMAN];
        cd.extend_from_slice(&flags);
        if self.vehicle_type_id > 255 {
            cd.extend_from_slice(&self.vehicle_type_id.to_be_bytes());
        } else {
            cd.push(self.vehicle_type_id as u8);
        }

        let rs = (u32::from(self.role) << 17) + (self.skills_efficiency_xp & 0x1FFFF);
        cd.extend_from_slice(&rs.to_be_bytes()[1..]);
        cd.push(self.skills.len() as u8);
        cd.extend_from_slice(&self.skills);
        cd.push(if self.skills.is_empty() { 0 } else { self.last_skill_level });
        if self.free_skills > 0 {
            cd.push(self.free_skills);
        }
        if !self.bonus_skills.is_empty() {
            cd.push(self.bonus_skills.len() as u8);
            cd.extend_from_slice(&self.bonus_skills);
        }
        for id in [self.first_name_id, self.last_name_id, self.icon_id] {
            cd.extend_from_slice(&id.to_be_bytes());
        }
        cd.extend_from_slice(&self.free_xp.to_be_bytes());
        cd.extend_from_slice(&self.dossier);
        cd

    }

}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn int_descr() {
        // The T110E4 sent with the vehicle commands.
        let cd = IntCompactDescr::from_int(13089);
        assert_eq!(cd, IntCompactDescr { item_type: item_type::VEHICLE, nation: 2, id: 51 });
        assert_eq!(cd.to_int(), 13089);
        let big = IntCompactDescr { item_type: 20, nation: 3, id: 7 };
        assert_eq!(IntCompactDescr::from_int(big.to_int()), big);
    }

    #[test]
    fn vehicle_descr() {
        let raw = b"!3g\x00\xa3\x00\xcd\x00\x0b\x00X\x004\x01\x07f\x00R\x00M\x00";
        let descr = VehicleDescr::decode(raw).unwrap();
        assert_eq!((descr.nation, descr.type_id), (2, 51));
        assert_eq!((descr.chassis, descr.engine, descr.fuel_tank, descr.radio), (103, 163, 205, 11));
        assert_eq!((descr.turret, descr.gun), (88, 308));
        assert_eq!(descr.optional_devices, [Some(102), Some(82), Some(77)]);
        assert_eq!(descr.encode(), raw);
    }

    #[test]
    fn vehicle_descr_gap() {
        // A device in the first of three slots only.
        let descr = VehicleDescr {
            nation: 0, type_id: 300, chassis: 1, engine: 2, fuel_tank: 3, radio: 4,
            turret: 5, gun: 6, optional_devices: vec![None, None, Some(9)], extra_flags: 0, extra: vec![],
        };
        let raw = descr.encode();
        assert_eq!(raw[0], 0x03);
        assert_eq!(raw[raw.len() - 3], 0b100);
        assert_eq!(VehicleDescr::decode(&raw).unwrap(), descr);
    }

    #[test]
    fn tankman_descr() {
        let raw = b"(\x043\t\x86\xa0\x05\t\x06&') \x01]\x02\xce\x00\x15\x00\x00\x17\xcfB\x00\x04\x00";
        let descr = TankmanDescr::decode(raw).unwrap();
        assert_eq!((descr.nation, descr.vehicle_type_id, descr.role), (2, 51, 4));
        assert_eq!(descr.skills, [9, 6, 38, 39, 41]);
        assert_eq!(descr.last_skill_level, 32);
        assert_eq!((descr.first_name_id, descr.last_name_id, descr.icon_id, descr.free_xp), (349, 718, 21, 6095));
        assert_eq!(descr.dossier, b"B\x00\x04\x00");
        assert_eq!(descr.encode(), raw);
    }

}
