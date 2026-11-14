//! What broke of a battle's destructibles, as the clients are told: each chunk of the map
//! that has some has an [`AreaDestructibles`] entity, listing those broken in it, by kind.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::PI;
use std::io;

use tracing::{info, warn};
use glam::Vec3;

use wgtk::app::base::{self, EntityInit, SpaceKey, Visibility};
use wgtk::space::destructibles::DestructibleKind;
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


/// The destructibles' entities of a battle's chunks, see the module's documentation.
#[derive(Debug)]
pub struct DestructibleAreas {
    /// Each chunk's, by its id.
    areas: HashMap<u16, Area>,
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
}

impl DestructibleAreas {

    /// The destructibles' entity of each chunk of a battle's space that has some, created
    /// right away, as the real server does.
    pub fn new(app: &mut base::App, space: SpaceKey, statics: &Statics) -> io::Result<Self> {
        let chunk_ids: BTreeSet<u16> = statics.destructibles()
            .map(|(_, destructible)| destructible.chunk_id)
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
            });
        }
        Ok(Self {
            areas,
        })
    }

    /// How many chunks have destructibles.
    pub fn len(&self) -> usize {
        self.areas.len()
    }

    /// Tell the clients of this battle what broke, rammed or shot: each list once,
    /// however much of it broke, a house bringing down a dozen modules at once.
    pub fn show(&mut self, app: &mut base::App, battle: u32, breakages: impl IntoIterator<Item = Breakage>) -> io::Result<()> {
        let mut broken = HashSet::new();
        for breakage in breakages {
            if let Some(list) = self.record(battle, breakage) {
                broken.insert(list);
            }
        }
        for (chunk_id, kind) in broken {
            self.areas[&chunk_id].show(app, kind)?;
        }
        Ok(())
    }

    /// List a destructible broken in its chunk's entity, encoded as its kind is, see
    /// [`AreaDestructibles`]: the list it changed, by its chunk's id and its kind.
    fn record(&mut self, battle: u32, breakage: Breakage) -> Option<(u16, DestructibleKind)> {

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
                area.columns.push([high, low, pack_angle(yaw) << 2 | fall]);
            }
            DestructibleKind::Structure => {
                let module = piece.module.unwrap_or_default();
                area.modules.push([high, low, (module & 0x7f) << 1 | u8::from(shot)]);
            }
            // Not numbered: they are speed trees, see `Rubble`.
            DestructibleKind::Tree => return None,
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
            DestructibleKind::Tree => Ok(()),
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

/// An angle on 6 bits, as the client's native `packAngleToUint` does: from -π to π
/// over 0 to 63. Fitted: the two lamp posts the captured Pershing knocked down fell its
/// way, within 0.25 rad, read so.
fn pack_angle(angle: f32) -> u8 {
    ((wrap_angle(angle) + PI) / (2.0 * PI) * 63.0).round() as u8
}
