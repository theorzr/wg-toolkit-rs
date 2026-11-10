//! What a hull stands on, and what stops it: the ground under a point, its kind, and
//! which static triangles are ground and which are walls.
//!
//! A static triangle within [`WALL_ANGLE`] of vertical is a *wall*: it stops a hull's
//! box, see the `walls` module. Flatter, it is *ground*, as the terrain is, or a ramp
//! whatever its slope. The ground under a contact of a hull is the *highest* of the
//! terrain and of the ground-like triangles *no higher than [`STEP_HEIGHT`] above the
//! contact*, see [`ground_height`]. The contact is where it is on the tilted hull, so a
//! nose raised by a rock's face, a step or a ramp finds the ground further up, and the
//! hull climbs onto it. That bound is also what makes a bridge work from both sides:
//!
//! ```text
//!    height                                         the deck is above the contacts by
//!      ▲                      ═════════════════     more than 0.8 m, so it isn't ground:
//!      │         ┌──────┐                           the tank drives under
//!      │ y+0.8 ┈ │ tank │ ┈┈┈┈┈┈┈┈┈┈┈ top ┈┈┈┈     ground = highest surface below this
//!      │ y     ──┴──────┴────────── terrain
//!      ┴───────────────────────────────────▶
//!
//!      on the ramp, the front contacts find it first and lift the nose, and once the
//!      tank is on the deck, the deck is below `top` and holds it, not the terrain under:
//!
//!      │                          ╱═════════════════ deck
//!      │       ┌──────┐        ╱   (ramp)
//!      │ ──────┴──────┴─────╱
//! ```

use glam::{Vec2, Vec3};

use wgtk::space::statics::{BodyKind, StaticTriangle};
use wgtk::space::terrain::{Ground, Terrain};
use wgtk::space::Space;


/// How high above a contact the ground under it may be found, and above the bottom of
/// its box the walls start, in metres: what is lower it drives over. Fitted roughly: the
/// drives' real hulls went over debris and porches up to 0.76 m high.
pub const STEP_HEIGHT: f32 = 0.8;
/// How far from vertical a solid static triangle stops the hull, in radians: flatter,
/// it is ground the hull would climb. Our choice, before fitting.
const WALL_ANGLE: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// The ground the vehicle drives on where the terrain gives none, or off it. Our choice:
/// unpainted cells are mostly beyond the battle's bounds, and medium is most of the
/// painted ones.
const DEFAULT_GROUND: Ground = Ground::Medium;

/// The height of the ground under a point of the ground plane, no higher than `top`:
/// the terrain's, or a static model's above it, its solid triangles that aren't walls or
/// its ramps, the deck of a bridge it drives onto say, not the one it drives under. None
/// with neither.
pub fn ground_height(space: &Space, point: Vec2, top: f32) -> Option<f32> {

    let terrain = space.terrain.height_at(point.x, point.y);
    let statics = [BodyKind::Solid, BodyKind::Ramp].into_iter()
        .flat_map(|kind| space.statics.triangles_at(kind, point))
        .filter(|&(triangle, height)| height <= top && (triangle.kind == BodyKind::Ramp || !is_wall(triangle)))
        .map(|(_, height)| height)
        .reduce(f32::max);

    match (terrain, statics) {
        (Some(terrain), Some(statics)) => Some(terrain.max(statics)),
        (terrain, statics) => terrain.or(statics),
    }

}

/// Whether a static triangle is a wall, within [`WALL_ANGLE`] of vertical, rather than
/// ground.
pub fn is_wall(triangle: &StaticTriangle) -> bool {
    triangle.normal.y.abs() < WALL_ANGLE.sin()
}

/// The kind of ground at a position on the terrain, else [`DEFAULT_GROUND`]: how it
/// holds the vehicle back and how fast it turns there.
pub fn ground_at(terrain: &Terrain, position: Vec3) -> Ground {
    terrain.ground_at(position.x, position.z)
        .filter(|&ground| ground != Ground::None)
        .unwrap_or(DEFAULT_GROUND)
}
