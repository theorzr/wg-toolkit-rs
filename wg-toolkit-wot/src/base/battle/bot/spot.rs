//! Where a bot fires at its target from, see [`FiringSpotSearch`].

use std::f32::consts::TAU;

use glam::{Affine3A, Vec3};

use crate::base::physics::{VehicleId, World};
use crate::base::ballistics::gun::GunLoader;
use crate::base::ballistics::impact;

use super::tactics::{best_shell, hull_center, in_sight, INDICATION_WORTH};
use super::situation::{BotEnemy, BotSituation};
use super::knowledge::BotKnowledge;
use super::motor::way_radius;
use super::target::BotTarget;
use super::role::BotRole;


/// How many ticks the bot keeps its spot before looking round again, see
/// [`FiringSpotSearch::update`], and how many at least before it starts or stops
/// waiting, see [`BotRole::patient`]. Our choice: three seconds, and two.
const SPOT_TICKS: u32 = 30;
const WAIT_TICKS: u32 = 20;
/// How far round it the bot looks for a spot, in metres, in how many directions.
const SPOT_RADII: [f32; 2] = [15.0, 35.0];
const SPOT_DIRECTIONS: usize = 8;
/// How much more another spot must be worth for the bot to leave its own, see
/// [`FiringSpotSearch::worth`].
const SPOT_MARGIN: f32 = 0.15;
/// How much less a spot is worth per metre off its role's range, see
/// [`BotRole::range`], and per metre of the way there: a green shot is worth a yellow
/// one 50 m nearer its range, or 250 m nearer it. Our choice.
const SPOT_RANGE: f32 = 0.01;
const SPOT_TRAVEL: f32 = 0.002;
/// What a spot its target doesn't see it from is worth while the bot waits, see
/// [`BotRole::patient`]: as a yellow shot.
const HIDDEN_WORTH: f32 = 0.5;
/// How near a spot plants count as its cover, in metres, and how many make it full: a
/// bot knows where bushes are, as a player sees them, not how much they hide. Our
/// choice.
const COVER_RADIUS: f32 = 10.0;
const COVER_PLANTS: usize = 3;


/// Where a bot fires at its target from, looked for round it now and then, see
/// [`Self::update`].
#[derive(Debug, Clone, Default)]
pub struct FiringSpotSearch {
    /// Where it fires at its target from: none where it drives at it instead.
    pub spot: Option<Vec3>,
    /// How many ticks since it looked for its spot, see [`SPOT_TICKS`].
    looked: u32,
    /// Whether it waits out of its target's sight, see [`BotRole::patient`].
    waiting: bool,
}

impl FiringSpotSearch {

    /// Look for the spot of a bot of this `role` to fire at its `target` from, every
    /// [`SPOT_TICKS`], or once it starts or stops waiting, see [`BotRole::patient`], its
    /// target aiming at it with its gun loaded, as its `knowledge` reckons: of
    /// where it is, its spot and the points round it, see [`SPOT_RADII`], where it
    /// stands, see `World::stand`, the one worth the most, see [`Self::worth`], none
    /// where an ally stands, its spot kept unless another is worth [`SPOT_MARGIN`] more.
    /// None, to drive at its target, while farther from it than its role's range and the
    /// farthest of those points, or seeing it from none of them as it doesn't wait.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &mut World,
        id: VehicleId,
        role: &BotRole,
        knowledge: &BotKnowledge,
        gun: &GunLoader,
        target: Option<BotTarget>,
        situation: &BotSituation,
    ) {

        let Some(target) = target else {
            self.spot = None;
            return;
        };
        let Some(enemy) = situation.enemies.iter().find(|enemy| enemy.vehicle == target.vehicle) else { return };
        let vehicle = world.vehicle(id);
        let position = vehicle.hull.position;
        let waiting = role.patient && knowledge.enemy(enemy.vehicle)
            .is_some_and(|known| known.aims_at_me && known.loaded(situation.now));

        self.looked += 1;
        if self.looked < SPOT_TICKS && (waiting == self.waiting || self.looked < WAIT_TICKS) {
            return;
        }
        self.looked = 0;
        self.waiting = waiting;

        let far = role.range(vehicle) + SPOT_RADII[SPOT_RADII.len() - 1];
        if (enemy.center - position).with_y(0.0).length() > far {
            self.spot = None;
            return;
        }

        // Its spot first, kept unless another is worth more by a margin.
        let radius = way_radius(vehicle);
        let round = SPOT_RADII.into_iter().flat_map(|distance| {
            (0..SPOT_DIRECTIONS).map(move |direction| {
                let angle = direction as f32 / SPOT_DIRECTIONS as f32 * TAU;
                position + Vec3::new(angle.sin(), 0.0, angle.cos()) * distance
            })
        });
        let points = [self.spot.unwrap_or(position), position].into_iter().chain(round);

        let mut best: Option<(Vec3, f32)> = None;
        for (index, point) in points.enumerate() {
            let Some(point) = world.stand(point, radius) else { continue };
            let taken = situation.allies.iter()
                .any(|ally| (ally.center - point).with_y(0.0).length() < ally.radius + radius);
            if taken {
                continue;
            }
            let Some(worth) = self.worth(world, id, role, gun, point, enemy, situation) else { continue };
            let worth = worth + if index == 0 { SPOT_MARGIN } else { 0.0 };
            if best.is_none_or(|(_, best)| worth > best) {
                best = Some((point, worth));
            }
        }
        self.spot = best.map(|(point, _)| point);

    }

    /// What a spot on the ground at `point` is worth to a bot of this `role` to fire at
    /// this `enemy`, its target, from, in its `situation`, none if it doesn't see it from
    /// there as it doesn't wait: what its gun marker shows there, see
    /// [`INDICATION_WORTH`], or while it waits, whether its target doesn't see it, see
    /// [`HIDDEN_WORTH`]; less off its role's range and the farther, see [`SPOT_RANGE`]
    /// and [`SPOT_TRAVEL`]; then by what its role likes, see [`BotRole`]: plants round
    /// it, see [`COVER_RADIUS`], its target's side or rear, other enemies not seeing it,
    /// its target's marker red on its armour, facing it as it is.
    #[allow(clippy::too_many_arguments)]
    fn worth(
        &self,
        world: &World,
        id: VehicleId,
        role: &BotRole,
        gun: &GunLoader,
        point: Vec3,
        enemy: &BotEnemy,
        situation: &BotSituation,
    ) -> Option<f32> {

        // Its gun and hull's centre there, as high over the ground as here.
        let vehicle = world.vehicle(id);
        let position = vehicle.hull.position;
        let (eye, _, _) = vehicle.turret.gun_marker(&vehicle.hull);
        let moved = Affine3A::from_translation(point - position);
        let eye = moved.transform_point3(eye);
        let center = moved.transform_point3(hull_center(situation.armor));

        let shot = if self.waiting {
            if world.sees(enemy.center, center) { 0.0 } else { HIDDEN_WORTH }
        } else if in_sight(world, eye, enemy.center) {
            best_shell(gun, eye, enemy.center, &enemy.armor)
                .map_or(0.0, |(_, shown)| INDICATION_WORTH[shown as usize])
        } else {
            return None;
        };

        let distance = (enemy.center - point).with_y(0.0).length();
        let range = (distance - role.range(vehicle)).abs() * SPOT_RANGE;
        let travel = (point - position).with_y(0.0).length() * SPOT_TRAVEL;

        let plants = world.plants_near(point, COVER_RADIUS).min(COVER_PLANTS);
        let cover = plants as f32 / COVER_PLANTS as f32;
        let target = &world.vehicle(enemy.vehicle).hull;
        let heading = Vec3::new(target.yaw().sin(), 0.0, target.yaw().cos());
        let flank = (1.0 - heading.dot((point - enemy.center).with_y(0.0).normalize_or_zero())) / 2.0;
        let exposure = situation.enemies.iter()
            .filter(|other| other.vehicle != enemy.vehicle && in_sight(world, other.center, center))
            .count() as f32;

        let armor = match (&enemy.shell, role.armor > 0.0) {
            (Some(shell), true) => {
                let (from, _, _) = world.vehicle(enemy.vehicle).turret.gun_marker(target);
                let transforms = situation.armor.transforms.map(|transform| moved * transform);
                let shown = impact::indicate(shell, from, center, &situation.armor.collision, &transforms);
                1.0 - shown.map_or(0.0, |shown| INDICATION_WORTH[shown as usize])
            }
            _ => 0.0,
        };

        Some(shot - range - travel
            + role.cover * cover
            + role.flank * flank
            - role.exposure * exposure
            + role.armor * armor)

    }

}
