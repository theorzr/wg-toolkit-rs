//! Arena definitions, read from the game's `scripts/arena_defs`: `_list_.xml` gives each
//! map's geometry id and name, `<name>.xml` its definition.

use std::io;

use wgtk::res::fs::ResFilesystem;

use super::item_defs::{invalid, read_xml};


/// A map, as an arena type id names it.
#[derive(Debug, Clone)]
pub struct ArenaTypeDef {
    /// Its name, `08_ruinberg` say.
    pub name: String,
    /// Its geometry's resource path, `spaces/08_ruinberg` say.
    pub geometry: String,
}

impl ArenaTypeDef {

    /// Load the map of an arena type id, its geometry id in the low 16 bits and its
    /// gameplay's in the high ones.
    pub fn load_by_type_id(fs: &ResFilesystem, arena_type_id: i32) -> io::Result<Self> {
        let geometry_id = i64::from(arena_type_id & 0xFFFF);
        let list = read_xml(fs, "scripts/arena_defs/_list_.xml")?;
        let name = list.iter_children("map")
            .filter_map(|map| map.as_element())
            .find(|map| map.get_child_integer("id") == Some(geometry_id))
            .and_then(|map| map.get_child_string("name"))
            .ok_or_else(|| invalid(format!("no map {geometry_id} in the arena list")))?;
        let def = read_xml(fs, &format!("scripts/arena_defs/{name}.xml"))?;
        let geometry = def.get_child_string("geometry")
            .ok_or_else(|| invalid(format!("map {name} has no geometry")))?;
        Ok(Self { name: name.to_string(), geometry: geometry.to_string() })
    }

}
