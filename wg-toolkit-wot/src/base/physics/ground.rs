//! The kind of ground a hull drives on ([`ground_at`]): firm, medium or soft, as the
//! terrain is painted, which its drive's turning speed and rolling friction follow. What
//! it stands on and what stops it, the terrain and the static models alike, is the
//! space's, see "Contacts" in `wgtk::space` and the `body` module.

use glam::Vec3;

use wgtk::space::terrain::{Ground, Terrain};


/// The ground the vehicle drives on where the terrain gives none, or off it. Measured:
/// on Graf Zeppelin's unpainted cells, a third of its drive, the Pershing turned 1.18
/// times as fast as on its medium ones, its firm ground's ratio to medium.
const DEFAULT_GROUND: Ground = Ground::Firm;

/// The kind of ground at a position on the terrain, else [`DEFAULT_GROUND`]: how it
/// holds the vehicle back and how fast it turns there.
pub fn ground_at(terrain: &Terrain, position: Vec3) -> Ground {
    terrain.ground_at(position.x, position.z)
        .filter(|&ground| ground != Ground::None)
        .unwrap_or(DEFAULT_GROUND)
}
