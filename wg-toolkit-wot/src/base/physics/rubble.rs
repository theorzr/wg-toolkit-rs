//! Destructibles: what a hull rams and breaks ([`Hull::ram`]), what shells break
//! ([`Rubble::shoot`]), and what of them still stands ([`Rubble`]).
//!
//! The [`Statics`] is the map as built and never changes. What broke during the
//! battle is the world's [`Rubble`]. A [`Piece`] is what the client numbers: a
//! destructible by its chunk's id and its index there, and for a house, which module,
//! by its material kind. A house can be placed as several models, all of them the same
//! piece, so breaking one breaks the others with it.
//!
//! ```text
//!   a triangle the hull touches, of kind Destructible
//!     │ triangle.instance
//!     ▼
//!   statics.destructible(instance) ──▶ PlacedDestructible { chunk_id, index, kind, … }
//!     │                                   │ a house: its module by triangle.material_kind
//!     ▼                                   ▼
//!   Piece { chunk_id, index, module } ──▶ in rubble.broken?  yes: it is gone, ignored
//!                                                            no:  it stands
//! ```
//!
//! What a piece still standing does depends on its kind:
//!
//! - A **house's module** blocks like a wall ([`Rubble::blocks`], see the `walls`
//!   module) until it breaks.
//! - A **fence, post or barrel** never blocks. The hull drives into it, damaging it on
//!   each tick it overlaps it, until it breaks.
//! - A destructible body of a model the client doesn't number is ignored altogether.
//!   Nobody could tell the client it broke.
//!
//! # Ramming
//!
//! [`Hull::ram`] runs before its body moves, looking ahead: the pieces it rams are those
//! its box will be in by the end of the tick, as its speeds take it, grown by
//! [`RAM_REACH`], so that one it presses against counts. Its body's walls would stop
//! it against a house's module before it could see how fast it hit it. For each piece
//! it is closing on, or its controls drive it into, its damage grows by, with `m` the
//! hull's weight in kg, `w` the same in units of [`UNIT_VEHICLE_MASS`], and `v` the
//! speed of its point of contact, turning included:
//!
//! ```text
//!   damage += KINETIC_DAMAGE · ½ · m · v²     the energy of the hit, as the game's
//!                                             client reckons it
//!           + RAM_PRESS · w · dt              pressing, so a stopped hull still breaks it
//! ```
//!
//! The damage adds up from tick to tick in `rubble.damage`. Once it reaches the piece's
//! health, the piece breaks: it goes into `rubble.broken` with the modules it brings
//! down ([`Module::depends`](wgtk::space::destructibles::Module::depends)), and into
//! `rubble.fresh` for the battle to tell its clients. The hull keeps its speed, less
//! [`RAM_COST`] of the health it took. Since the broken module no longer blocks, its
//! body drives on through it in the same tick.
//!
//! # Shells
//!
//! Shells break pieces too, see `fly` in `gun.rs`. A body whose collision flags say
//! shells pass it, every light fence's, is ignored by them altogether. A piece of no more
//! than 19 health is flown through, and breaks whole when the shell gets there
//! ([`Rubble::break_shot`]), but for an HE or HESH shell, which explodes on it. A
//! stronger one stops the shell, and takes its damage ([`Rubble::shoot`]). That damage
//! adds up in `rubble.damage` with the rams', and a house's module (10 to 80 health)
//! breaks at the first shell. The breakage is marked `shot`, and the client plays its
//! hit effect rather than its ram effect.
//!
//! # Telling the clients
//!
//! The battle takes the fresh breakages ([`Rubble::take_breakages`]) at the end of the
//! tick, after the shells landed, and lists them in the `AreaDestructibles` entity of
//! their chunk, one list per kind, each sent once a tick (see `show_rubble` in
//! `battle.rs`). The explosion of a shell that broke a piece lists it as well,
//! `explodeProjectile`'s `damagedDestructibles`. The client waits up to 0.11 s for one
//! to come with the other, then breaks the piece as the shell explodes.

use std::collections::{HashMap, HashSet};
use std::vec::Drain;

use glam::{Vec2, Vec3Swizzles};
use smallvec::SmallVec;

use wgtk::space::statics::{PlacedDestructible, Statics, StaticTriangle};
use wgtk::space::Space;
use wgtk::space::destructibles::DestructibleKind;

use super::walls::wall_contact;
use super::{Footprint, Hull};


/// How much a ram damages a destructible: this share of the vehicle's kinetic energy as
/// it closes on it, `½ · m · v²` of its weight in kg and its speed in m/s, adding up
/// from a tick to the next; it breaks once that reaches its health, and keeps the
/// energy left. From the game's client, which reckons so whether a vehicle's hit may
/// break one (`Vehicle._isDestructibleMayBeBroken`): a house's module against its
/// health, anything else first scaled by the vehicle's weight against
/// [`UNIT_VEHICLE_MASS`] to the power of its `kineticDamageCorrection`, 0 unless set,
/// and no destructible sets it. A 15 t light tank breaks a house's module of 50 health
/// at 6.7 m/s. The captured drives' breaks are all well past it: the Type 64, 23 t,
/// drove through modules of 50 health at 13 m/s, 290.
const KINETIC_DAMAGE: f32 = 0.00015;
/// How much of its energy breaking a destructible costs the vehicle, as a share of the
/// health it took. Fitted: the real Type 64 drove through four house modules of 35
/// health at 15.6 m/s, all but keeping its speed.
const RAM_COST: f32 = 0.05;
/// How much a vehicle damages a destructible it presses against, its tracks driving it
/// in, per second, of its weight in [`UNIT_VEHICLE_MASS`], on top of [`KINETIC_DAMAGE`]: one
/// stopped by a house's wall breaks it in time, a module of 50 health in 2 s at 35 t.
/// Our choice, by feel.
const RAM_PRESS: f32 = 25.0;
/// How much further than its collision box a hull rams, in metres: one pressing
/// against a piece, stopped by it, still damages it. Our choice, as far as a wall
/// touches it from, `WALL_MARGIN`.
const RAM_REACH: f32 = 0.05;
/// The weight of a vehicle that destructibles' health is measured against, in kg:
/// `unitVehicleMass` in the game's `scripts/destructibles.xml`.
const UNIT_VEHICLE_MASS: f32 = 35_000.0;

impl Hull {

    /// Damage the destructibles it is about to drive into, in a tick of `dt` seconds,
    /// see [`KINETIC_DAMAGE`], and break those whose health it took: it goes on through,
    /// less the speed their health took; those still standing stop it as walls do, see
    /// the `walls` module. Its damage adds up from a tick to the next, so a crawl through a fence
    /// takes a few, and so does pressing against one, see [`RAM_PRESS`].
    pub fn ram(&mut self, dt: f32, space: &Space, rubble: &mut Rubble) {

        // 1. The pieces it will be in, each once, its first triangle telling the way into
        //    it, else, a small one it drives onto within the tick having no way out of
        //    it, the way from its middle to the triangle's.
        let footprint = self.footprint();
        let middle = footprint.middle;
        let mut hits: SmallVec<[(Piece, &PlacedDestructible, Vec2, Vec2); 8]> = SmallVec::new();
        let statics = &space.statics;
        for (triangle, against) in self.rammed(dt, RAM_REACH, space, rubble) {
            let Some((piece, destructible)) = rubble.standing(statics, triangle) else { continue };
            let center = (triangle.vertices[0] + triangle.vertices[1] + triangle.vertices[2]).xz() / 3.0;
            if !hits.iter().any(|&(hit, _, _, _)| hit == piece) {
                hits.push((piece, destructible, against, center - middle));
            }
        }

        let weight = self.mobility.weight / UNIT_VEHICLE_MASS;
        let (heading, _) = self.axes();
        let asked = heading.xz() * self.target_speed();
        for (piece, destructible, against, center) in hits {

            // 2. How fast it hits it where it touches it, turning on the spot swinging it
            //    in too: nothing if it isn't closing on it, nor driven into it, pressing.
            let velocity = self.velocity();
            let arm = ram_contact(&footprint, against, center);
            let point = self.point_velocity(arm);
            let closing = [against, center].into_iter()
                .filter_map(Vec2::try_normalize)
                .any(|toward| point.dot(toward) > 0.0 || asked.dot(toward) > 0.0);
            if !closing {
                continue;
            }

            // 3. Damaged by the energy of the hit and by its pressing. Grazing one breaks
            //    it too: the real T110E4 drove on along a fence.
            let speed = point.length();
            let energy = KINETIC_DAMAGE * self.mobility.weight * speed * speed / 2.0;
            let Some((health, done)) = rubble.damage(destructible, piece, energy + RAM_PRESS * weight * dt) else { continue };
            if done < health {
                continue;
            }

            // 4. Broken, it loses a little of its energy: see `RAM_COST`.
            let left = (health - (done - energy)).max(0.0);
            let kept = (speed * speed - 2.0 * RAM_COST * left / weight).max(0.0).sqrt();
            if speed > 0.0 {
                self.set_velocity(velocity * (kept / speed));
            }
            rubble.break_piece(destructible, piece, velocity.x.atan2(velocity.z), speed, false);

        }

    }

}

/// Where a hull's footprint touches a piece it rams, from its middle: its corner deepest
/// in it, the way `against` it, else, one it drove onto, its point nearest the
/// triangle's middle, at `center` from its middle.
fn ram_contact(footprint: &Footprint, against: Vec2, center: Vec2) -> Vec2 {
    match against.try_normalize() {
        Some(against) => wall_contact(&footprint.corners(), -against) - footprint.middle,
        None => {
            let local = Vec2::new(center.dot(footprint.right), center.dot(footprint.heading));
            let local = local.clamp(-footprint.half, footprint.half);
            footprint.right * local.x + footprint.heading * local.y
        }
    }
}

/// A destructible piece, as the client numbers it: all of a destructible, or one of a
/// structure's modules by its material kind. A destructible can be placed as several
/// models, a house's walls and roof say, each a piece of it alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Piece {
    /// Its chunk's id and its index in it, see [`PlacedDestructible`].
    pub chunk_id: u16,
    pub index: u16,
    pub module: Option<u8>,
}

/// A destructible a vehicle or a shell broke, as its clients are told of it.
#[derive(Debug, Clone, Copy)]
pub struct Breakage {
    pub piece: Piece,
    pub kind: DestructibleKind,
    /// Which way it falls, the yaw the vehicle or the shell went, in radians.
    pub yaw: f32,
    /// How fast the vehicle went, in m/s.
    pub speed: f32,
    /// Whether a shell broke it, flying through it or hitting it, not a vehicle.
    pub shot: bool,
}

/// The destructibles broken in a world, and those broken since last taken, and the
/// damage those still standing took.
#[derive(Debug, Default)]
pub struct Rubble {
    broken: HashSet<Piece>,
    fresh: Vec<Breakage>,
    damage: HashMap<Piece, f32>,
}

impl Rubble {

    /// The piece a static triangle is of, if it is destructible and still stands.
    pub fn standing<'a>(&self, statics: &'a Statics, triangle: &StaticTriangle) -> Option<(Piece, &'a PlacedDestructible)> {
        let destructible = statics.destructible(triangle.instance)?;
        let module = match destructible.kind {
            DestructibleKind::Structure => Some(destructible.module(triangle.material_kind as u8)?.material_kind),
            _ => None,
        };
        let piece = Piece {
            chunk_id: destructible.chunk_id,
            index: destructible.index,
            module,
        };
        (!self.broken.contains(&piece)).then_some((piece, destructible))
    }

    /// Whether a static triangle is of a structure's module still standing: those stop
    /// what drives into them until it breaks them, while a fence or a post is driven
    /// into as it breaks, see [`Hull::ram`].
    pub fn blocks(&self, statics: &Statics, triangle: &StaticTriangle) -> bool {
        self.standing(statics, triangle).is_some_and(|(piece, _)| piece.module.is_some())
    }

    /// Damage a piece: its health, and the damage it took in all.
    fn damage(&mut self, destructible: &PlacedDestructible, piece: Piece, damage: f32) -> Option<(f32, f32)> {
        let health = destructible.health_of(piece.module)?;
        let done = self.damage.entry(piece).or_default();
        *done += damage;
        Some((health, *done))
    }

    /// Break a piece, a module the others it brings down with it, each told of.
    fn break_piece(&mut self, destructible: &PlacedDestructible, piece: Piece, yaw: f32, speed: f32, shot: bool) {
        let depends = piece.module
            .and_then(|module| destructible.module(module))
            .map(|module| module.depends.as_slice())
            .unwrap_or_default();
        let brought = depends.iter().map(|&module| Piece {
            module: Some(module),
            ..piece
        });
        for piece in std::iter::once(piece).chain(brought) {
            if self.broken.insert(piece) {
                self.fresh.push(Breakage {
                    piece,
                    kind: destructible.kind,
                    yaw,
                    speed,
                    shot,
                });
            }
        }
    }

    /// Break a piece a shell flew through, with the modules it brings down, the shell
    /// going `yaw`: its placed model by its index in `BSMI`, of the triangle it met.
    /// Nothing if it is already broken.
    pub fn break_shot(&mut self, statics: &Statics, instance: u32, piece: Piece, yaw: f32) {
        let Some(destructible) = statics.destructible(instance) else { return };
        self.break_piece(destructible, piece, yaw, 0.0, true);
    }

    /// Damage a piece a shell hit and stopped on, a house's module say, the shell going
    /// `yaw`: it breaks once the damage of all the shells that hit it is its health,
    /// with the modules it brings down. What broke.
    pub fn shoot(&mut self, statics: &Statics, instance: u32, piece: Piece, damage: f32, yaw: f32) -> &[Breakage] {
        let fresh = self.fresh.len();
        if let Some(destructible) = statics.destructible(instance)
            && !self.broken.contains(&piece)
            && let Some((health, done)) = self.damage(destructible, piece, damage)
            && done >= health
        {
            self.break_piece(destructible, piece, yaw, 0.0, true);
        }
        &self.fresh[fresh..]
    }

    /// The destructibles broken since last taken, for the battle to tell its clients.
    pub fn take_breakages(&mut self) -> Drain<'_, Breakage> {
        self.fresh.drain(..)
    }

}


#[cfg(test)]
mod tests {

    use glam::Vec3;

    use wgtk::space::destructibles::Module;
    use wgtk::space::statics::BodyKind;
    use wgtk::space::Space;

    use super::super::fixtures::{chieftain, nose, wall};
    use super::super::movement_flag;
    use super::*;

    /// A fence across the way at z = 10, 3 m high, destructible whole with this health,
    /// the 4th of its chunk; or a house's wall, its module of this health.
    fn fence(health: f32, house: bool) -> Space {
        let mut space = wall(3.0);
        let material_kind = if house { 73 } else { 0 };
        let triangles = space.statics.triangles_in(BodyKind::Solid, Vec3::splat(-30.0), Vec3::splat(30.0))
            .map(|triangle| StaticTriangle::new(triangle.vertices, 0, material_kind, BodyKind::Destructible))
            .collect();
        space.statics = Statics::from_triangles(triangles);
        let modules = if house {
            vec![Module {
                material_kind: 73,
                health,
                depends: Vec::new(),
            }]
        } else {
            Vec::new()
        };
        space.statics.add_destructible(0, PlacedDestructible {
            chunk_id: 0x7f7f,
            index: 3,
            kind: if house { DestructibleKind::Structure } else { DestructibleKind::Fragile },
            health,
            modules,
        });
        space
    }

    #[test]
    fn breaks_through_what_it_rams_hard_enough() {
        // A fence it rams at 20 km/h, it breaks, once, and drives on through, slowed by
        // what its health took.
        let space = fence(10.0, false);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.speed = 20.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        // Its speed before the tick it broke it, and after.
        let mut broke = None;
        for _ in 0..30 {
            let before = hull.speed;
            hull.advance(0.1, &space, None, &mut rubble);
            if broke.is_none() && !rubble.fresh.is_empty() {
                broke = Some((before, hull.speed));
            }
        }
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
        // It lost a little of its energy, its engine winning a little back within the
        // tick.
        let (before, after) = broke.unwrap();
        let weight = hull.mobility.weight / UNIT_VEHICLE_MASS;
        let kept = (before * before - 2.0 * RAM_COST * 10.0 / weight).sqrt();
        assert!(after > kept && after < kept + 0.2, "{after} for {kept}");
        let broken: Vec<Breakage> = rubble.take_breakages().collect();
        assert_eq!(broken.len(), 1);
        assert_eq!((broken[0].piece.chunk_id, broken[0].piece.index, broken[0].kind), (0x7f7f, 3, DestructibleKind::Fragile));
        // Turning on the spot with its nose against one, its corner swings into it and
        // breaks it.
        let space = fence(10.0, false);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.position.z = 10.0 - hull.mobility.bounds[1].z - 0.05;
        hull.flags = movement_flag::ROTATE_LEFT;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        assert_eq!(rubble.fresh.len(), 1);
        // A house's wall too strong for it stops it as a wall would.
        let space = fence(1000.0, true);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.speed = 20.0 / 3.6;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..30 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        let front = nose(&hull);
        assert!((front - 10.0).abs() < 2e-3 && rubble.fresh.is_empty(), "front at {front}");
        // One it starts against, stopped, it presses until it breaks it.
        let space = fence(50.0, true);
        let mut rubble = Rubble::default();
        let mut hull = chieftain();
        hull.position.z = 10.0 - hull.mobility.bounds[1].z - 0.01;
        hull.flags = movement_flag::FORWARD;
        for _ in 0..40 {
            hull.advance(0.1, &space, None, &mut rubble);
        }
        assert_eq!(rubble.fresh.len(), 1);
        assert!(hull.position.z > 12.0, "{}", hull.position.z);
    }

}
