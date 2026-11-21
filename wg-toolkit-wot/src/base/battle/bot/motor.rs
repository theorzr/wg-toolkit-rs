//! How a bot drives its vehicle where it is ordered, see [`BotMotor`]: its way over the
//! map round its allies, backing up to points behind it, getting unstuck, and once
//! there, its hull angled to what it faces.

use std::f32::consts::PI;

use glam::{Affine3A, Vec3};

use crate::base::physics::{movement_flag, wrap_angle, Vehicle, VehicleId, World};
use crate::base::ballistics::impact::{self, Indication};
use crate::base::ballistics::shot::ShotTarget;
use crate::base::TICK;

use super::tactics::{hull_center, turn_limit};
use super::situation::{BotAlly, BotEnemy};


/// How far off what it faces its hull turns once stopped, in radians, when its target's
/// marker shows no worse on it otherwise: its front armour thicker along a shell's way,
/// its side not yet bare. Our choice, see [`BotMotor::stance`].
pub const ARMOR_ANGLE: f32 = 25.0f32.to_radians();
/// The angles off what it faces the bot tries its hull at once stopped, positive to the
/// right, see [`BotMotor::stance`].
const HULL_ANGLES: [f32; 7] = [
    -35.0f32.to_radians(),
    -25.0f32.to_radians(),
    -15.0f32.to_radians(),
    0.0,
    15.0f32.to_radians(),
    25.0f32.to_radians(),
    35.0f32.to_radians(),
];
/// How far off its heading its way may be for the bot to drive while turning, in
/// radians.
const TURN_WHILE_DRIVING: f32 = 0.6;
/// How far off its heading its way may be before the bot turns at all.
pub const HEADING_TOLERANCE: f32 = 0.1;
/// How near ahead an ally must be for the bot to give way to it, in metres between their
/// centres on the ground plane, and how far off its heading, in radians, see
/// [`gives_way`]. Our choice: a hull's length and some.
const GIVE_WAY_DISTANCE: f32 = 12.0;
const GIVE_WAY_ANGLE: f32 = 35.0f32.to_radians();
/// How fast an ally must drive the bot's way, in m/s, for it to give way: one standing
/// or coming at it is driven round instead.
const GIVE_WAY_SPEED: f32 = 1.0;
/// How close to a point of its way the bot gets before heading for the next, in metres.
/// Our choice: as close as it can turn short of it.
pub const ARRIVE: f32 = 4.0;
/// How far behind it a point of its way may be, off its heading in radians and in
/// metres, for the bot to back up to it, its front kept to what it faces.
const REVERSE_ANGLE: f32 = 110.0f32.to_radians();
const REVERSE_DISTANCE: f32 = 40.0;
/// How far its goal may move off its way's end before the bot plans again, in metres.
const REPLAN_DISTANCE: f32 = 25.0;
/// How many ticks the bot keeps a way before planning again anyway: what moved, a
/// vehicle in its way say, is planned around.
const REPLAN_TICKS: u32 = 50;
/// How much clearance the bot keeps past its hull's half width, in metres: a cell's is
/// measured from the middle of the nearest one it can't enter, half a cell off the wall.
const WIDTH_MARGIN: f32 = 1.0;
/// How slow the bot must move, in m/s, driving, for how many ticks, to be stuck: its
/// tracks may turn as it pushes against something.
const STUCK_SPEED: f32 = 0.5;
const STUCK_TICKS: u32 = 20;
/// How many ticks the bot backs off for once stuck.
const BACK_OFF_TICKS: u32 = 15;


/// Where a bot's motor drives its vehicle, see [`BotMotor::controls`].
#[derive(Debug, Clone, Copy)]
pub struct DriveOrder {
    /// Where it drives.
    pub goal: Vec3,
    /// How close to its goal it stops, in metres on the ground plane.
    pub range: f32,
    /// What it faces once stopped, its hull angled off it, see [`BotMotor::stance`].
    pub face: Vec3,
    /// Whether it backs up to a point of its way near behind it, its front kept to what
    /// it faces.
    pub reverse: bool,
}

impl DriveOrder {

    /// Whether a vehicle at `position` is there: within its range of its goal.
    fn reached(&self, position: Vec3) -> bool {
        (self.goal - position).with_y(0.0).length() <= self.range
    }

}

/// How a bot drives its vehicle, as its [`DriveOrder`] says, see [`Self::controls`].
#[derive(Debug, Clone, Default)]
pub struct BotMotor {
    /// The points of its way it hasn't reached, the next first.
    way: Vec<Vec3>,
    /// Where its goal was when it planned its way.
    goal: Option<Vec3>,
    /// How many ticks since it planned its way.
    planned: u32,
    /// Where its hull was the tick before, which says whether it moves.
    last: Option<Vec3>,
    /// How many ticks it has driven without moving.
    stuck: u32,
    /// How many ticks it still backs off for, and the way it does: back if it was
    /// driving forward, else forward.
    backing: u32,
    escape: u8,
    /// The angle off what it faces its hull turns to once stopped, see [`Self::stance`].
    angle: f32,
    /// The side it angles its hull to once stopped, see [`ARMOR_ANGLE`]: 1 to the right
    /// of what it faces, -1 to the left, 0 while it drives.
    side: f32,
}

impl BotMotor {

    /// Choose how its hull stands once its `order` is reached, `against` its target, the
    /// enemy it faces, if any, of its own `armor`: when it just stopped, or to `rethink`;
    /// see [`Self::hull_angle`]. Facing no enemy, straight at what it faces.
    pub fn stance(
        &mut self,
        world: &World,
        id: VehicleId,
        order: &DriveOrder,
        against: Option<&BotEnemy>,
        armor: &ShotTarget,
        rethink: bool,
    ) {
        if !order.reached(world.vehicle(id).hull.position) {
            self.side = 0.0;
            return;
        }
        match against {
            None => self.angle = 0.0,
            Some(enemy) if self.side == 0.0 || rethink => {
                self.angle = self.hull_angle(world.vehicle(id), world, order.face, enemy, armor);
            }
            Some(_) => {}
        }
    }

    /// The angle off what it `face`s, positive to the right, its hull turns to once
    /// stopped: of [`HULL_ANGLES`] its turret's yaw limits keep its target within, see
    /// [`turn_limit`], to the side it first stopped on, the one this `enemy`'s gun marker
    /// shows worst on its `armor`, see [`impact::indicate`], the nearest its angle among
    /// equals, or [`ARMOR_ANGLE`] just stopped; that one if it doesn't know the enemy's
    /// shell.
    fn hull_angle(
        &mut self,
        vehicle: &Vehicle,
        world: &World,
        face: Vec3,
        enemy: &BotEnemy,
        armor: &ShotTarget,
    ) -> f32 {

        let (position, yaw) = (vehicle.hull.position, vehicle.hull.yaw());
        let to = (face - position).with_y(0.0);
        let toward = to.x.atan2(to.z);
        let limit = turn_limit(vehicle);
        let preferred = if self.side == 0.0 {
            self.side = if wrap_angle(yaw - toward) < 0.0 { -1.0 } else { 1.0 };
            self.side * ARMOR_ANGLE.min(limit)
        } else {
            self.angle
        };

        let Some(shell) = enemy.shell.as_ref() else { return preferred };
        let target = world.vehicle(enemy.vehicle);
        let (from, _, _) = target.turret.gun_marker(&target.hull);
        let center = hull_center(armor);

        HULL_ANGLES.into_iter()
            .filter(|angle| angle.abs() <= limit && angle * self.side >= 0.0)
            .map(|angle| {
                let turn = Affine3A::from_translation(position)
                    * Affine3A::from_rotation_y(wrap_angle(toward + angle - yaw))
                    * Affine3A::from_translation(-position);
                let transforms = armor.transforms.map(|transform| turn * transform);
                let shown = impact::indicate(shell, from, center, &armor.collision, &transforms)
                    .unwrap_or(Indication::NotPierced);
                (angle, shown)
            })
            .min_by(|(a, a_shown), (b, b_shown)| {
                a_shown.cmp(b_shown)
                    .then((a - preferred).abs().total_cmp(&(b - preferred).abs()))
            })
            .map_or(preferred, |(angle, _)| angle)

    }

    /// The controls driving a vehicle as its `order` says, round its `allies`: on its
    /// way to its goal until within its range of it, then turning its angle off what it
    /// faces, see [`Self::stance`].
    pub fn controls(&mut self, world: &mut World, id: VehicleId, order: &DriveOrder, allies: &[BotAlly]) -> u8 {

        let hull = &world.vehicle(id).hull;
        let (position, yaw) = (hull.position, hull.yaw());
        let goal = order.goal;

        // 4. Stuck, it backs off first, the other way it drove.
        let moved = self.last.replace(position)
            .map_or(f32::INFINITY, |last| last.distance(position) / TICK.as_secs_f32());
        if self.backing > 0 {
            self.backing -= 1;
            return self.escape;
        }
        let driving = hull.flags & (movement_flag::FORWARD | movement_flag::BACKWARD);
        if driving != 0 && moved < STUCK_SPEED {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_TICKS {
            self.stuck = 0;
            self.backing = BACK_OFF_TICKS;
            self.escape = if driving & movement_flag::FORWARD != 0 {
                movement_flag::BACKWARD
            } else {
                movement_flag::FORWARD
            };
            self.way.clear();
            return self.escape;
        }

        // 1. Close, it stops, turned its angle off what it faces.
        if order.reached(position) {
            self.way.clear();
            let to = (order.face - position).with_y(0.0);
            return steer(wrap_angle(to.x.atan2(to.z) + self.angle - yaw), false);
        }

        // 2. Its way, round its allies, planned again as needed, the points it got close
        //    to dropped.
        self.planned += 1;
        let moved = self.goal.is_none_or(|planned| planned.distance(goal) > REPLAN_DISTANCE);
        if self.way.is_empty() || moved || self.planned >= REPLAN_TICKS {
            let radius = way_radius(world.vehicle(id));
            let around: Vec<_> = allies.iter()
                .map(|ally| (ally.center, ally.radius))
                .collect();
            self.way = world.path(position, goal, radius, &around).unwrap_or_default();
            self.goal = Some(goal);
            self.planned = 0;
        }
        while self.way.first().is_some_and(|point| (*point - position).with_y(0.0).length() < ARRIVE) {
            self.way.remove(0);
        }

        // 3. It drives to the next point, its goal if none, but behind an ally its way;
        //    near behind it, as ordered, it backs up to it, its keys the other way round,
        //    see `physics::drive`.
        let next = self.way.first().copied().unwrap_or(goal);
        let to = (next - position).with_y(0.0);
        let off = wrap_angle(to.x.atan2(to.z) - yaw);
        if order.reverse && off.abs() > REVERSE_ANGLE && to.length() < REVERSE_DISTANCE {
            let back = wrap_angle(off - PI);
            if back.abs() < TURN_WHILE_DRIVING {
                return steer(-back, false) | movement_flag::BACKWARD;
            }
            return steer(back, false);
        }
        steer(off, off.abs() < TURN_WHILE_DRIVING && !gives_way(position, yaw, allies))

    }

}

/// The controls turning toward a way `off` the heading, in radians, positive to the
/// right, driving forward if `forward`.
fn steer(off: f32, forward: bool) -> u8 {
    let mut flags = 0;
    if off > HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_RIGHT;
    } else if off < -HEADING_TOLERANCE {
        flags |= movement_flag::ROTATE_LEFT;
    }
    if forward {
        flags |= movement_flag::FORWARD;
    }
    flags
}

/// How wide a way a vehicle needs, in metres from its middle: its hull's half width and
/// [`WIDTH_MARGIN`].
pub fn way_radius(vehicle: &Vehicle) -> f32 {
    let [min, max] = vehicle.hull.mobility.bounds;
    max.x.max(-min.x) + WIDTH_MARGIN
}

/// Whether a bot at `position`, heading `yaw`, gives way to one of its `allies`: one
/// driving its way, faster than [`GIVE_WAY_SPEED`], within [`GIVE_WAY_DISTANCE`] ahead of
/// it and [`GIVE_WAY_ANGLE`] of its heading.
fn gives_way(position: Vec3, yaw: f32, allies: &[BotAlly]) -> bool {
    let heading = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    allies.iter().any(|ally| {
        let to = (ally.center - position).with_y(0.0);
        ally.velocity.dot(heading) > GIVE_WAY_SPEED
            && to.length() < GIVE_WAY_DISTANCE
            && wrap_angle(to.x.atan2(to.z) - yaw).abs() < GIVE_WAY_ANGLE
    })
}


#[cfg(test)]
mod tests {

    use std::f32::consts::FRAC_PI_2;
    use std::sync::Arc;

    use glam::Vec2;

    use crate::base::physics::fixtures::{blocks, chieftain_aiming, chieftain_mobility};
    use super::super::fixtures::HULL_CENTER;
    use super::super::SPOT_ARRIVE;
    use super::*;

    #[test]
    fn gives_way_to_an_ally_driving_its_way() {

        let ally = BotAlly {
            center: Vec3::new(1.0, HULL_CENTER, 8.0),
            radius: 4.0,
            velocity: Vec3::Z * 5.0,
        };

        // Just ahead, driving its way: it waits.
        assert!(gives_way(Vec3::ZERO, 0.0, &[ally]));
        // Not standing, coming at it, far, or aside.
        let standing = BotAlly {
            velocity: Vec3::ZERO,
            ..ally
        };
        let coming = BotAlly {
            velocity: -Vec3::Z * 5.0,
            ..ally
        };
        let far = BotAlly {
            center: Vec3::new(0.0, HULL_CENTER, 20.0),
            ..ally
        };
        assert!(!gives_way(Vec3::ZERO, 0.0, &[standing, coming, far]));
        assert!(!gives_way(Vec3::ZERO, FRAC_PI_2, &[ally]));

    }

    #[test]
    fn backs_up_to_a_spot_behind_it() {

        // Facing its enemy 200 m ahead, its spot 25 m behind and a little aside.
        let mut world = World::new(Arc::new(blocks(&[])), Some((Vec2::splat(-150.0), Vec2::splat(150.0))));
        let id = world.add(Vehicle::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility(), chieftain_aiming()));
        let order = DriveOrder {
            goal: Vec3::new(5.0, 0.0, -25.0),
            range: SPOT_ARRIVE,
            face: Vec3::new(0.0, HULL_CENTER, 200.0),
            reverse: true,
        };
        let mut motor = BotMotor::default();

        let mut backed = false;
        for _ in 0..150 {
            let flags = motor.controls(&mut world, id, &order, &[]);
            backed |= flags & movement_flag::BACKWARD != 0;
            assert!(flags & movement_flag::FORWARD == 0, "drove forward");
            world.vehicle_mut(id).hull.flags = flags;
            world.step(TICK.as_secs_f32());
        }

        // There, its front still to its enemy.
        let hull = &world.vehicle(id).hull;
        assert!(backed);
        assert!((order.goal - hull.position).with_y(0.0).length() <= SPOT_ARRIVE + 1.0, "stopped at {:?}", hull.position);
        assert!(hull.yaw().abs() < 45.0f32.to_radians(), "turned {}°", hull.yaw().to_degrees());

    }

}
