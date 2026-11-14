//! What broke of a battle's destructibles, as the clients are told: each chunk of the map
//! that has some has an [`AreaDestructibles`] entity, listing those broken in it, by kind.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, PI};
use std::io;

use tracing::{info, warn};
use glam::Vec3;

use wgtk::app::base::{self, EntityInit, SpaceKey, Visibility};
use wgtk::space::destructibles::DestructibleKind;
use wgtk::space::terrain::Terrain;
use wgtk::space::statics::Statics;
use wgtk::app::entity::EntityKey;

use crate::base::physics::{wrap_angle, Breakage, DestructiblePiece};
use crate::game::schema::AreaDestructibles;


/// How far from the middle of its chunk an [`AreaDestructibles`] entity is, in metres:
/// `RADIUS_FOR_LOCATION_AD` in the client's `DestructiblesCache.py`.
const AREA_OFFSET: f32 = 4.5;
/// How fast a vehicle goes for each step of a column's fall speed, in m/s, of 3 at
/// most. Fitted: the captured Pershing knocked down lamp posts at 5.5 and 12 m/s, which
/// fell at 2 and 3.
const FALL_SPEED_STEP: f32 = 2.5;
/// How far along its fall a tree's tip is reckoned from the ground, in metres at a
/// scale of 1, see [`fall_pitch`]. Our choice, about where a tree's crown lands: the
/// captured trees tip from 69° to 103°, 86° in the middle, as the ground falls away or
/// rises along their way.
const FALL_REACH: f32 = 8.0;


/// The destructibles' entities of a battle's chunks, see the module's documentation.
#[derive(Debug)]
pub struct DestructibleAreas {
    /// Each chunk's, by its id.
    areas: HashMap<u16, Area>,
    /// Where each destructible tree stands and its scale, by its chunk's id and its
    /// index there: how far it tips as it falls depends on the ground around it.
    trees: HashMap<(u16, u16), (Vec3, f32)>,
}

/// A chunk's destructibles entity, and what it lists as broken.
#[derive(Debug)]
struct Area {
    /// Its `AreaDestructibles` entity, near the chunk's middle, see [`AREA_OFFSET`].
    entity: EntityKey<AreaDestructibles>,
    /// Its `destroyedModules`: each structure's module broken, by its index in the
    /// chunk, big-endian, then the module and whether a shot broke it.
    modules: Vec<[u8; 3]>,
    /// Its `destroyedFragiles`: each fragile broken, by its index, then whether a shot
    /// broke it.
    fragiles: Vec<[u8; 3]>,
    /// Its `fallenColumns`: each column knocked down, by its index, then the way it
    /// fell and how fast, see [`FALL_SPEED_STEP`].
    columns: Vec<[u8; 3]>,
    /// Its `fallenTrees`: each tree knocked down, by its index, then how far it tips,
    /// see [`fall_pitch`], on 16 bits, then the way it fell and how fast, as columns
    /// do: `encodeFallenTree` in the client's `DestructiblesCache.py`.
    trees: Vec<[u8; 5]>,
}

impl DestructibleAreas {

    /// The destructibles' entity of each chunk of a battle's space that has some, created
    /// right away, as the real server does.
    pub fn new(app: &mut base::App, space: SpaceKey, statics: &Statics) -> io::Result<Self> {
        
        let chunk_ids: BTreeSet<u16> = statics.destructibles()
            .map(|(_, destructible)| destructible.chunk_id)
            .collect();
        
        let trees = statics.trees().iter()
            .map(|tree| ((tree.chunk_id, tree.index), (tree.position, tree.scale)))
            .collect();
        
        let mut areas = HashMap::new();
        for chunk_id in chunk_ids {
            
            let entity = app.create_entity::<AreaDestructibles>(space, EntityInit {
                position: area_position(chunk_id),
                visibility: Visibility::Always,
                ..EntityInit::default()
            })?;

            areas.insert(chunk_id, Area {
                entity,
                modules: Vec::new(),
                fragiles: Vec::new(),
                columns: Vec::new(),
                trees: Vec::new(),
            });
            
        }
        
        Ok(Self {
            areas,
            trees,
        })

    }

    /// How many chunks have destructibles.
    pub fn len(&self) -> usize {
        self.areas.len()
    }

    /// Tell the clients of this battle what broke, rammed or shot, on this terrain: each
    /// list once, however much of it broke, a house bringing down a dozen modules at
    /// once.
    pub fn show(&mut self, app: &mut base::App, battle: u32, breakages: impl IntoIterator<Item = Breakage>, terrain: &Terrain) -> io::Result<()> {
        let mut broken = HashSet::new();
        for breakage in breakages {
            if let Some(list) = self.record(battle, breakage, terrain) {
                broken.insert(list);
            }
        }
        for (chunk_id, kind) in broken {
            self.areas[&chunk_id].show(app, kind)?;
        }
        Ok(())
    }

    /// List a destructible broken in its chunk's entity, encoded as its kind is, see
    /// [`AreaDestructibles`], a tree tipping as `terrain` lets it: the list it changed,
    /// by its chunk's id and its kind.
    fn record(&mut self, battle: u32, breakage: Breakage, terrain: &Terrain) -> Option<(u16, DestructibleKind)> {

        let Breakage { piece, kind, yaw, speed, shot } = breakage;
        let Some(area) = self.areas.get_mut(&piece.chunk_id) else {
            warn!(battle, chunk_id = piece.chunk_id, index = piece.index, "Failed to tell of a destructible broken out of the areas");
            return None;
        };

        let [high, low] = piece.index.to_be_bytes();
        match kind {
            DestructibleKind::Fragile => {
                area.fragiles.push([high, low, u8::from(shot)]);
            }
            DestructibleKind::FallingAtom => {
                let fall = ((speed / FALL_SPEED_STEP) as u8).min(3);
                area.columns.push([high, low, (pack_angle(yaw, 6) as u8) << 2 | fall]);
            }
            DestructibleKind::Tree => {
                let fall = ((speed / FALL_SPEED_STEP) as u8).min(3);
                let pitch = match self.trees.get(&(piece.chunk_id, piece.index)) {
                    Some(&(position, scale)) => fall_pitch(terrain, position, scale, yaw),
                    None => FRAC_PI_2,
                };
                let [pitch_high, pitch_low] = pack_angle(pitch, 16).to_be_bytes();
                area.trees.push([high, low, pitch_high, pitch_low, (pack_angle(yaw, 6) as u8) << 2 | fall]);
            }
            DestructibleKind::Structure => {
                let module = piece.module.unwrap_or_default();
                area.modules.push([high, low, (module & 0x7f) << 1 | u8::from(shot)]);
            }
        }

        info!(battle, chunk_id = piece.chunk_id, index = piece.index, ?kind, module = ?piece.module, shot, "Broke a destructible");
        Some((piece.chunk_id, kind))

    }

}

impl Area {

    /// Tell the clients its list of destructibles of a kind broken.
    fn show(&self, app: &mut base::App, kind: DestructibleKind) -> io::Result<()> {
        let mut entity = app.entity(self.entity)?;
        match kind {
            DestructibleKind::Fragile => entity.set_destroyedFragiles(self.fragiles.clone()),
            DestructibleKind::FallingAtom => entity.set_fallenColumns(self.columns.clone()),
            DestructibleKind::Structure => entity.set_destroyedModules(self.modules.clone()),
            DestructibleKind::Tree => entity.set_fallenTrees(self.trees.clone()),
        }
    }

}

/// A piece as an explosion lists it, see `explodeProjectile`: none of an index the code
/// has no room for.
pub fn explosion_obstacle(piece: DestructiblePiece) -> Option<u32> {
    let index = u8::try_from(piece.index).ok()?;
    Some(u32::from(piece.chunk_id) << 16 | u32::from(index) << 8 | u32::from(piece.module.unwrap_or(0)))
}

/// The position of a chunk's [`AreaDestructibles`] entity: near the middle of the chunk,
/// `areaDestructiblesPositionFromChunkID` in the client's `DestructiblesCache.py`.
fn area_position(chunk_id: u16) -> Vec3 {
    let x = i32::from(chunk_id >> 8) - 127;
    let z = i32::from(chunk_id & 0xff) - 127;
    let angle = PI / 10.0 * (x + z) as f32;
    Vec3::new(
        x as f32 * 100.0 + 50.0 + AREA_OFFSET * angle.cos(),
        0.0,
        z as f32 * 100.0 + 50.0 + AREA_OFFSET * angle.sin(),
    )
}

/// How far a tree standing at `position`, placed at `scale`, tips as it falls toward
/// `yaw`, in radians from upright: until it lies on the ground, a quarter turn on
/// flat ground, less where the ground rises along its way and more where it falls
/// away, the ground read [`FALL_REACH`] off, terrain only.
fn fall_pitch(terrain: &Terrain, position: Vec3, scale: f32, yaw: f32) -> f32 {
    let reach = FALL_REACH * scale;
    let tip = position + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * reach;
    let base = terrain.height_at(position.x, position.z).unwrap_or(position.y);
    let ground = terrain.height_at(tip.x, tip.z).unwrap_or(base);
    FRAC_PI_2 + ((base - ground) / reach).atan()
}

/// An angle on `bits` bits, as the client's native `packAngleToUint` does: from -π to
/// π over 0 to its most. Fitted on 6 bits: the two lamp posts the captured Pershing
/// knocked down fell its way, within 0.25 rad, read so; on 16, the captured trees tip
/// 69° to 103°, read so, big-endian.
fn pack_angle(angle: f32, bits: u32) -> u16 {
    let most = ((1u32 << bits) - 1) as f32;
    ((wrap_angle(angle) + PI) / (2.0 * PI) * most).round() as u16
}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn packs_a_fallen_tree_as_the_real_server() {

        // The first tree the captured battle's server felled tipped 87°, packed so.
        assert_eq!(pack_angle(1.519_67, 16).to_be_bytes(), [189, 234]);

        // On flat ground, a tree falls flat, whichever way.
        let terrain = Terrain::default();
        let position = Vec3::new(10.0, 3.0, -20.0);
        assert_eq!(fall_pitch(&terrain, position, 1.3, 2.0), FRAC_PI_2);

    }

}
