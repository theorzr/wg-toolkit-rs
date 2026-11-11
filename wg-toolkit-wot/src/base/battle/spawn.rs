//! Where a battle's vehicles start: their team's start, on the map's spawn points or
//! behind its bases, each team in rows from there.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use wgtk::space::terrain::Terrain;

use crate::game::arena_defs::{ArenaTypeDef, Spawn};


/// Where a vehicle starts.
#[derive(Debug, Clone, Copy)]
pub struct TeamStart {
    pub position: Vec3,
    /// Roll, pitch and yaw.
    pub direction: Vec3,
}

impl TeamStart {

    /// Where a team's vehicle starts on a map: at its spawn point, else
    /// [`SPAWN_BEHIND_BASE`] behind its base, facing the enemy's spawn, inside the map and
    /// on the ground.
    pub fn team(map: &ArenaTypeDef, terrain: &Terrain, team: u8) -> Self {
        let own = map.spawns[team as usize - 1];
        let enemy = map.spawns[2 - team as usize];
        let forward = (spawn_point(enemy) - spawn_point(own)).normalize_or_zero();
        let at = match own {
            Spawn::Point(at) => at,
            Spawn::Base(at) => at - forward * SPAWN_BEHIND_BASE,
        };
        let (min, max) = map.bounds;
        let at = at.clamp(min + SPAWN_MARGIN, max - SPAWN_MARGIN);
        let height = terrain.height_at(at.x, at.y).unwrap_or(0.0);
        Self {
            position: Vec3::new(at.x, height, at.y),
            direction: Vec3::new(0.0, 0.0, forward.x.atan2(forward.y)),
        }
    }

    /// A start at the given point and yaw, on the ground.
    pub fn at(at: Vec2, yaw: f32, terrain: &Terrain) -> Self {
        let height = terrain.height_at(at.x, at.y).unwrap_or(0.0);
        Self {
            position: Vec3::new(at.x, height, at.y),
            direction: Vec3::new(0.0, 0.0, packed_yaw(yaw)),
        }
    }

    /// Where a team's vehicle of this slot starts, from this start, its team's: the first
    /// here, the next ones beside it, either side in turn, in rows of [`SLOT_ROW`], each
    /// behind the last, on the ground.
    pub fn slot(self, slot: u32, terrain: &Terrain) -> Self {
        let row = slot / SLOT_ROW;
        let column = slot % SLOT_ROW;
        let side = (column + 1) / 2;
        let side = if column % 2 == 1 { side as f32 } else { -(side as f32) };
        let yaw = self.direction.z;
        let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        let mut position = self.position + right * side * SLOT_SPACING - forward * row as f32 * ROW_SPACING;
        position.y = terrain.height_at(position.x, position.z).unwrap_or(position.y);
        Self {
            position,
            direction: self.direction,
        }
    }

    /// A start `distance` metres in front of this one, facing it, on the ground.
    pub fn facing(self, distance: f32, terrain: &Terrain) -> Self {
        let yaw = self.direction.z;
        let mut position = self.position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * distance;
        position.y = terrain.height_at(position.x, position.z).unwrap_or(position.y);
        Self {
            position,
            direction: Vec3::new(0.0, 0.0, packed_yaw(yaw + PI)),
        }
    }

}

/// Where a spawn is, a point or a base.
fn spawn_point(spawn: Spawn) -> Vec2 {
    match spawn {
        Spawn::Point(at) | Spawn::Base(at) => at,
    }
}

/// A yaw in [-π, π), as angles are packed.
fn packed_yaw(yaw: f32) -> f32 {
    (yaw + PI).rem_euclid(TAU) - PI
}


/// How far behind its base a team starts, in metres, on maps without spawn points: about
/// as captured.
const SPAWN_BEHIND_BASE: f32 = 40.0;
/// How far inside the map's bounds a team starts at least, in metres.
const SPAWN_MARGIN: f32 = 20.0;

/// How far apart a team's vehicles start side by side, in metres, and its rows of
/// [`SLOT_ROW`] one behind the other.
const SLOT_SPACING: f32 = 12.0;
const ROW_SPACING: f32 = 15.0;
const SLOT_ROW: u32 = 5;
