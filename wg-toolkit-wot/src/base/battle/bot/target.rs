//! The enemy a bot aims at, and where it aims, see [`BotTarget`].

use glam::Vec3;

use crate::base::physics::{VehicleId, World};
use crate::base::TICK;


/// How many times the bot works out where its target will be, from where the last time
/// said, see [`BotTarget::lead`].
const LEAD_PASSES: u32 = 3;


/// An enemy a bot aims at, see [`Bot::choose_target`](super::Bot::choose_target): its
/// vehicle, where its hull's centre is, where the bot aims, and whether the bot sees it
/// itself.
#[derive(Debug, Clone, Copy)]
pub struct BotTarget {
    /// Its vehicle, in the world.
    pub vehicle: VehicleId,
    /// Where its hull's centre is, as the bot's team spots it.
    pub at: Vec3,
    /// Where the bot aims: where it will be once a shell fired gets there while the bot
    /// sees it, see [`Self::lead`], else where it is.
    pub aim: Vec3,
    /// Whether the bot sees it itself, which it fires at only then.
    pub seen: bool,
}

impl BotTarget {

    /// Aim where it will be once a shell the bot's vehicle of this id fires gets there,
    /// while the bot sees it: its hull's centre moved on at its velocity for a tick, the
    /// battle firing after its step, then for the shell's flight to there, worked out
    /// again from there [`LEAD_PASSES`] times. The flight is the distance at the shell's
    /// speed, as near as its low arc is to a line.
    pub fn lead(&mut self, world: &World, id: VehicleId) {

        if !self.seen {
            return;
        }
        let vehicle = world.vehicle(id);
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);
        let speed = vehicle.turret.aiming.shot_speed.max(f32::EPSILON);
        let velocity = world.vehicle(self.vehicle).hull.velocity();

        for _ in 0..LEAD_PASSES {
            let flight = TICK.as_secs_f32() + self.aim.distance(eye) / speed;
            self.aim = self.at + velocity * flight;
        }

    }

}
