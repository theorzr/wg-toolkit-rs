//! Arena definitions, read from the game's `scripts/arena_defs`: `_list_.xml` gives each
//! map's geometry id and name, `<name>.xml` its definition.

use std::io;

use glam::Vec2;

use wgtk::res::fs::ResFilesystem;

use super::item_defs::{invalid, read_xml};


/// A map, with what standard battles on it need.
#[derive(Debug, Clone)]
pub struct ArenaTypeDef {
    /// Its name, `08_ruinberg` say.
    pub name: String,
    /// The arena type id of its standard battles: its geometry id, in the low 16 bits,
    /// with the `ctf` gameplay's, 0, in the high ones.
    pub type_id: i32,
    /// Its geometry's resource path, `spaces/08_ruinberg` say.
    pub geometry: String,
    /// Its playable area, on the ground plane: its bottom left and upper right corners.
    pub bounds: (Vec2, Vec2),
    /// Where each team spawns in standard battles, on the ground plane: its spawn point
    /// if the map has one, else its base.
    pub spawns: [Spawn; 2],
}

/// Where a team spawns.
#[derive(Debug, Clone, Copy)]
pub enum Spawn {
    Point(Vec2),
    Base(Vec2),
}

impl ArenaTypeDef {

    /// Load a map by its name.
    pub fn load(fs: &ResFilesystem, name: &str) -> io::Result<Self> {

        let list = read_xml(fs, "scripts/arena_defs/_list_.xml")?;
        let geometry_id = list.iter_children("map")
            .filter_map(|map| map.as_element())
            .find(|map| map.get_child_string("name") == Some(name))
            .and_then(|map| map.get_child_integer("id"))
            .ok_or_else(|| invalid(format!("no map {name} in the arena list")))?;

        let def = read_xml(fs, &format!("scripts/arena_defs/{name}.xml"))?;
        let geometry = def.get_child_string("geometry")
            .ok_or_else(|| invalid(format!("map {name} has no geometry")))?;
        let corner = |key| def.get_child_value_at(&["boundingBox", key]).and_then(|value| value.as_vec2())
            .ok_or_else(|| invalid(format!("map {name} has no bounding box")));
        let bounds = (corner("bottomLeft")?, corner("upperRight")?);

        let spawn = |team| {
            let point = def.get_child_value_at(&["gameplayTypes", "ctf", "teamSpawnPoints", team, "position"]);
            let base = def.get_child_value_at(&["gameplayTypes", "ctf", "teamBasePositions", team, "position1"]);
            point.and_then(|value| value.as_vec2()).map(Spawn::Point)
                .or_else(|| base.and_then(|value| value.as_vec2()).map(Spawn::Base))
                .ok_or_else(|| invalid(format!("map {name} has no standard battle spawn for {team}")))
        };
        let spawns = [spawn("team1")?, spawn("team2")?];

        Ok(Self { name: name.to_string(), type_id: geometry_id as i32, geometry: geometry.to_string(), bounds, spawns })

    }

}
