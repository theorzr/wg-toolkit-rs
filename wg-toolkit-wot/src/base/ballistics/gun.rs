//! A vehicle's gun as the server runs it: its ammunition, the shell loaded, its magazine
//! and bursts, the reload, and each shot's flight until it hits a vehicle, the terrain,
//! a static model, or runs out of range, and on off what it ricochets off. Vehicles are
//! hit on their collision models, static models on theirs, see [`ShotObstacles`].

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use rand::Rng;

use wgtk::space::statics::BodyKind;
use wgtk::space::Space;

use crate::game::collision::{segment_triangle, VehicleCollision, VehicleIntersection};
use crate::game::item_defs::{MagazineDef, ShellKind, ShotDef};

use crate::base::physics::{DestructiblePiece, Dispersion, Rubble};


/// How many σ of its shots' normal distribution the dispersion's radius is, stopped, see
/// [`dispersed_share`]. Our choice.
const CENTERED_SIGMAS: f32 = 2.0;

/// The flight is followed in steps of this many seconds.
const FLIGHT_STEP: f32 = 0.005;

/// The ricochets a shell goes on after, at most.
const MAX_RICOCHETS: u32 = 3;
/// How far before the point it ricochets off a shell goes on from, in metres, as the
/// server shows ricochets: for it not to meet what it ricochets off again.
const RICOCHET_STANDOFF: f32 = 0.001;


#[derive(Debug, Clone)]
pub struct GunLoader {
    /// Each shell the gun fires, and how many are left of it, those in the gun included
    /// until fired.
    pub ammo: Vec<(ShotDef, u32)>,
    /// The index in `ammo` of the shell in the gun: the one it fires next, or loads
    /// while it reloads. The client knows it as `VEHICLE_SETTING.CURRENT_SHELLS`, and
    /// changes it by picking a shell twice, see [`Self::change_loaded`]. It only has none
    /// left once the gun has no shell left at all.
    loaded_shell: usize,
    /// The index in `ammo` of the shell the gun loads once its magazine is empty,
    /// `loaded_shell` itself unless the client picked another one since. The client
    /// knows it as `VEHICLE_SETTING.NEXT_SHELLS`, and changes it by picking a shell once,
    /// see [`Self::select_next`]. When there is none of it left by then, the gun loads
    /// the first shell it has some left of instead.
    next_shell: usize,
    /// How the gun's shells come beyond one per reload: its magazine and its bursts.
    magazine: MagazineDef,
    /// The shells of `loaded_shell` in the gun: those it fires before reloading whole,
    /// at most [`Self::magazine_size`], 1 for a gun without a magazine. Fired, they
    /// leave `ammo` too. Empty, the magazine fills again once its whole reload ends, as
    /// the client expects the shells to come.
    magazine_shells: u32,
    /// The burst going on: its shells left to fire, and the game time the next one fires
    /// at, see [`Self::fire_burst`].
    burst_shells: Option<(u32, f64)>,
    /// The length of a whole reload: of the magazine, for a gun with one.
    reload_time: f32,
    /// How long changing the shell of a loaded gun takes, as a share of its whole
    /// reload; it takes a whole reload unless within 0 and 1.
    quick_change_factor: f32,
    /// The reload going on, between two shots of the magazine too.
    reload: Option<Reload>,
    /// The id the next shell fired takes.
    next_shot_id: i32,
}

/// A reload going on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reload {
    /// Its length, in seconds: the magazine's interval between two of its shots. The
    /// client takes it as `baseTime`, and only takes the reload as just started, and
    /// lets the player fire again once it ends, when its time left is the same: else it
    /// holds the next shot for 2 seconds, see `__gunReloadCommandWaitEndTime`.
    pub base_time: f32,
    /// The game time it ends at.
    pub end: f64,
}

/// A shell fired, flying from `start` to `end`: from its gun, or off what it last
/// ricocheted off, see [`Self::ricochet`].
#[derive(Debug, Clone)]
pub struct Shot {
    pub id: i32,
    pub def: ShotDef,
    pub start: Vec3,
    pub velocity: Vec3,
    /// How far it flew before `start`, in metres: 0 unless it ricocheted.
    pub flown: f32,
    /// How far from `start` it flies at most: its shell's maximum distance, less what it
    /// flew.
    pub range: f32,
    /// The times it ricocheted.
    pub ricochets: u32,
    /// The share of its shell's penetration it kept, 1 until it ricochets, see
    /// [`impact::ricochet_pen_factor`](super::impact::ricochet_pen_factor), or flies
    /// through a destructible, see [`ShotPiece`].
    pub pen_factor: f32,
    pub end: Vec3,
    /// The velocity at `end`.
    pub end_velocity: Vec3,
    /// When it gets to `end`, in seconds after firing.
    pub flight_time: f32,
    pub landing: ShotLanding,
    /// The destructibles it flies through on its way to `end`, in order: each breaks as
    /// it gets there, see [`super::physics::World::break_shot`].
    pub through: Vec<ShotPiece>,
    /// The shells fired at once with it when it is the first of them, it included: the
    /// client shows them all from the first one, see `showShooting`. None for the others
    /// of a burst.
    pub burst: Option<u32>,
}

/// What a shell ends its flight on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotLanding {
    /// A vehicle, placed by `transforms` as it was then, see [`ShotTarget`].
    Vehicle { entity_id: u32, hit: VehicleIntersection, transforms: [Affine3A; 4] },
    Terrain,
    /// A static model: a wall, a rock, a house's module too strong to fly through. Its
    /// triangle faces `normal`, toward where the shell came from. `piece` is the
    /// destructible it is of, which the shell damages, see
    /// [`super::physics::World::shoot`].
    Static { normal: Vec3, piece: Option<ShotPiece> },
    /// Nothing, at its maximum distance.
    Range,
}

/// A destructible a shell meets. One of no more health than
/// [`Destructibles::shoot_through_health`] is flown through, breaking, the shell losing
/// some of its penetration, see [`Destructibles::shoot_through_loss`]; a stronger one
/// stops it, see [`ShotLanding::Static`]. An HE or HESH shell explodes on whatever it
/// meets, and flies through nothing.
///
/// [`Destructibles::shoot_through_health`]: wgtk::space::destructibles::Destructibles::shoot_through_health
/// [`Destructibles::shoot_through_loss`]: wgtk::space::destructibles::Destructibles::shoot_through_loss
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotPiece {
    /// The placed model of the triangle it met, its index in `BSMI`.
    pub instance: u32,
    pub piece: DestructiblePiece,
    /// When the shell gets there, in seconds after its start, as [`Shot::flight_time`].
    pub time: f32,
}

/// What of a map a shell can hit besides vehicles: its terrain and its static models,
/// but for what broke of them.
#[derive(Debug, Clone, Copy)]
pub struct ShotObstacles<'a> {
    pub space: &'a Space,
    pub rubble: &'a Rubble,
}

/// A vehicle shells can hit: its collision models, placed in the world.
#[derive(Debug, Clone)]
pub struct ShotTarget {
    pub entity_id: u32,
    pub collision: Arc<VehicleCollision>,
    /// Each part's transform, see [`super::physics::Vehicle::part_transforms`].
    pub transforms: [Affine3A; 4],
}

impl GunLoader {

    /// A gun with these shells and no magazine, loaded with the first one there is, then
    /// loading it again, ready to fire.
    pub fn new(ammo: Vec<(ShotDef, u32)>, reload_time: f32) -> Self {
        let loaded = first_left(&ammo).unwrap_or(0);
        let mut gun = Self {
            ammo,
            loaded_shell: loaded,
            next_shell: loaded,
            magazine: MagazineDef::default(),
            magazine_shells: 0,
            burst_shells: None,
            reload_time,
            quick_change_factor: 1.0,
            reload: None,
            next_shot_id: 1,
        };
        gun.magazine_shells = gun.full_magazine();
        gun
    }

    /// The shells of `loaded_shell` the gun holds once reloaded whole, fewer when it has fewer
    /// left.
    fn full_magazine(&self) -> u32 {
        self.ammo.get(self.loaded_shell).map_or(0, |&(_, count)| count.min(self.magazine_size()))
    }

    /// The shells the gun holds once reloaded: its magazine's, or its burst's when it
    /// fires bursts without a magazine, as the client counts them; 1 without either.
    fn magazine_size(&self) -> u32 {
        self.magazine.size.max(self.magazine.burst_size).max(1)
    }

    /// The same gun numbering its shots from `first`: the client tells shots apart by
    /// their id alone, whoever fires them.
    pub fn with_shot_ids_from(mut self, first: i32) -> Self {
        self.next_shot_id = first;
        self
    }

    /// The same gun changing the shell it is loaded with in this share of its reload,
    /// see [`Self::change_loaded`].
    pub fn with_quick_change_factor(mut self, factor: f32) -> Self {
        self.quick_change_factor = factor;
        self
    }

    /// The same gun with this magazine and these bursts, its magazine full.
    pub fn with_magazine(mut self, magazine: MagazineDef) -> Self {
        self.magazine = magazine;
        self.magazine_shells = self.full_magazine();
        self
    }

    /// Fire the loaded shell at game time `now` from `start` toward `direction`, off
    /// within the `dispersion`, see [`disperse`], if the gun is loaded and has a shell
    /// left: the first of a burst, which [`Self::fire_burst`] fires the rest of. Once
    /// they are fired, the gun waits for its magazine's interval while it has some left,
    /// else reloads whole with its next shell, or another one once there is none of it
    /// left. That reload starts now, the burst's other shells in it, for the client to
    /// know when it can fire again as soon as it fires.
    pub fn fire(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: Dispersion, obstacles: Option<ShotObstacles>, vehicles: &[ShotTarget]) -> Option<Shot> {

        self.settle(now);
        if self.burst_shells.is_some() || !self.is_loaded(now) || self.magazine_shells == 0 {
            return None;
        }

        let burst = self.magazine.burst_size.min(self.magazine_shells);
        let mut shot = self.shoot(start, direction, dispersion, obstacles, vehicles)?;
        shot.burst = Some(burst);
        if burst > 1 {
            let burst_time = (burst - 1) as f32 * self.magazine.burst_interval;
            let wait = if self.magazine_shells > burst - 1 { self.magazine.interval } else { self.reload_time };
            self.burst_shells = Some((burst - 1, now + f64::from(self.magazine.burst_interval)));
            self.start_reload(now, burst_time + wait);
        } else {
            self.after_shots(now);
        }
        Some(shot)

    }

    /// Fill the magazine again once its whole reload is over by game time `now`, before
    /// [`Self::take_loaded`] tells it, for the client to fire as soon as it is.
    fn settle(&mut self, now: f64) {
        if self.magazine_shells == 0 && self.is_loaded(now) && self.reload.is_some() {
            self.magazine_shells = self.full_magazine();
        }
    }

    /// Whether the gun is loaded by game time `now`.
    pub fn is_loaded(&self, now: f64) -> bool {
        self.reload.is_none_or(|reload| now >= reload.end)
    }

    /// Fire a shell of the magazine, without waiting or reloading.
    fn shoot(&mut self, start: Vec3, direction: Vec3, dispersion: Dispersion, obstacles: Option<ShotObstacles>, vehicles: &[ShotTarget]) -> Option<Shot> {

        if self.magazine_shells == 0 {
            return None;
        }
        let (def, count) = self.ammo.get_mut(self.loaded_shell).filter(|(_, count)| *count > 0)?;
        *count -= 1;
        self.magazine_shells -= 1;
        let def = def.clone();

        let id = self.next_shot_id;
        self.next_shot_id += 1;

        let velocity = disperse(direction.normalize(), dispersion, &mut rand::thread_rng()) * def.speed;
        let range = def.max_distance;
        let launch = Launch {
            start,
            velocity,
            flown: 0.0,
            range,
            pen_factor: 1.0,
        };
        let flight = fly(&def, launch, obstacles, vehicles);
        Some(Shot {
            id,
            def,
            start,
            velocity,
            flown: 0.0,
            range,
            ricochets: 0,
            pen_factor: flight.pen_factor,
            end: flight.end,
            end_velocity: flight.end_velocity,
            flight_time: flight.flight_time,
            landing: flight.landing,
            through: flight.through,
            burst: None,
        })

    }

    fn start_reload(&mut self, now: f64, time: f32) {
        self.reload = Some(Reload { base_time: time, end: now + f64::from(time) });
    }

    /// After a shot or a burst: wait for the magazine's interval while it has some left,
    /// else reload whole with the next shell.
    fn after_shots(&mut self, now: f64) {
        if self.magazine_shells > 0 {
            self.start_reload(now, self.magazine.interval);
        } else {
            self.load_next();
            self.start_reload(now, self.reload_time);
        }
    }

    /// Load the next shell, or another one once there is none of it left.
    fn load_next(&mut self) {
        if !self.has_left(self.next_shell) {
            self.next_shell = first_left(&self.ammo).unwrap_or(self.next_shell);
        }
        self.loaded_shell = self.next_shell;
    }

    fn has_left(&self, index: usize) -> bool {
        self.ammo.get(index).is_some_and(|&(_, count)| count > 0)
    }

    /// Fire the next shell of the burst going on, if it is due by game time `now`, as
    /// [`Self::fire`] does, whose reload goes on. Once the burst empties the magazine,
    /// the gun loads its next shell.
    pub fn fire_burst(&mut self, now: f64, start: Vec3, direction: Vec3, dispersion: Dispersion, obstacles: Option<ShotObstacles>, vehicles: &[ShotTarget]) -> Option<Shot> {

        let (left, at) = self.burst_shells.filter(|&(_, at)| now >= at)?;
        let Some(shot) = self.shoot(start, direction, dispersion, obstacles, vehicles) else {
            self.burst_shells = None;
            self.after_shots(now);
            return None;
        };

        if left > 1 {
            self.burst_shells = Some((left - 1, at + f64::from(self.magazine.burst_interval)));
        } else {
            self.burst_shells = None;
            if self.magazine_shells == 0 {
                self.load_next();
            }
        }
        Some(shot)

    }

    /// Load the shell of this integer descriptor once the gun fires, if it has some
    /// left: `VEHICLE_SETTING.NEXT_SHELLS`. Whether it does.
    pub fn select_next(&mut self, compact_descr: i64) -> bool {
        let Some(index) = self.shell_left(compact_descr) else { return false };
        self.next_shell = index;
        true
    }

    /// The index of the shell of this integer descriptor, if the gun has some left.
    fn shell_left(&self, compact_descr: i64) -> Option<usize> {
        self.ammo.iter().position(|&(ref def, count)| def.compact_descr.to_int() == compact_descr && count > 0)
    }

    /// Load the shell of this integer descriptor instead of the one loaded, and after it
    /// too, if it has some left: `VEHICLE_SETTING.CURRENT_SHELLS`. The gun then reloads,
    /// in its quick change factor of a reload if it was loaded, as captured. Whether it
    /// reloads.
    ///
    /// A magazine is emptied and reloaded whole with it, quickly only when it was full,
    /// as the client predicts. Not while firing a burst.
    pub fn change_loaded(&mut self, now: f64, compact_descr: i64) -> bool {

        self.settle(now);
        if self.burst_shells.is_some() {
            return false;
        }

        let Some(index) = self.shell_left(compact_descr) else {
            return false
        };

        self.next_shell = index;
        if index == self.loaded_shell {
            return false;
        }

        let full = self.magazine_shells > 0 && self.magazine_shells == self.full_magazine();
        let quick = full && self.is_loaded(now) && self.quick_change_factor > 0.0 && self.quick_change_factor < 1.0;
        let time = if quick { self.reload_time * self.quick_change_factor } else { self.reload_time };

        self.loaded_shell = index;
        self.magazine_shells = 0;
        self.start_reload(now, time);
        true

    }

    /// Reload a magazine that is not full or not of the next shell, whole, with the next
    /// shell: `VEHICLE_SETTING.RELOAD_PARTIAL_CLIP`. Whether it reloads: not without a
    /// magazine, while firing a burst, or already reloading whole.
    pub fn reload_magazine(&mut self, now: f64) -> bool {

        self.settle(now);
        if self.magazine_size() <= 1 || self.burst_shells.is_some() || self.magazine_shells == 0 {
            return false;
        }

        if self.magazine_shells == self.full_magazine() && self.next_shell == self.loaded_shell {
            return false;
        }

        self.magazine_shells = 0;
        self.load_next();
        self.start_reload(now, self.reload_time);
        true

    }

    /// The shell loaded, and the shell it loads after.
    pub fn shells(&self) -> (Option<&ShotDef>, Option<&ShotDef>) {
        (self.ammo.get(self.loaded_shell).map(|(def, _)| def), self.ammo.get(self.next_shell).map(|(def, _)| def))
    }

    /// Each shell, how many are left of it, and how many of them are in the magazine:
    /// none without one, as captured, and only the loaded shell's with one.
    pub fn ammo_state(&self) -> impl Iterator<Item = (&ShotDef, u32, u32)> {
        let magazine_shells = if self.magazine_size() > 1 { self.magazine_shells } else { 0 };
        self.ammo.iter().enumerate()
            .map(move |(index, (def, count))| (def, *count, if index == self.loaded_shell { magazine_shells } else { 0 }))
    }

    /// Whether the gun has a magazine, or fires bursts: shells come more than one per
    /// reload then.
    pub fn has_magazine(&self) -> bool {
        self.magazine_size() > 1
    }

    /// Whether the gun got loaded by game time `now` since last asked, its magazine
    /// filled again if it reloaded whole.
    pub fn take_loaded(&mut self, now: f64) -> bool {
        self.settle(now);
        self.reload.take_if(|reload| now >= reload.end).is_some()
    }

    /// The reload going on.
    pub fn reload(&self) -> Option<Reload> {
        self.reload
    }

    /// The length of a whole reload, in seconds.
    pub fn reload_time(&self) -> f32 {
        self.reload_time
    }

}

impl Shot {

    /// The shell going on off what it hit at `end`, ricocheting along `direction` at the
    /// same speed, as the server shows ricochets, for the rest of its range, keeping
    /// `pen_factor` of its penetration. None once it ricocheted [`MAX_RICOCHETS`] times,
    /// or out of range.
    pub fn ricochet(&self, direction: Vec3, pen_factor: f32, obstacles: Option<ShotObstacles>, vehicles: &[ShotTarget]) -> Option<Shot> {
        if self.ricochets >= MAX_RICOCHETS {
            return None;
        }
        let range = self.range - self.start.distance(self.end);
        if range <= 0.0 {
            return None;
        }
        let start = self.end - self.end_velocity.normalize_or_zero() * RICOCHET_STANDOFF;
        let velocity = direction.normalize() * self.end_velocity.length();
        let launch = Launch {
            start,
            velocity,
            flown: self.distance(),
            range,
            pen_factor: self.pen_factor * pen_factor,
        };
        let flight = fly(&self.def, launch, obstacles, vehicles);
        Some(Shot {
            id: self.id,
            def: self.def.clone(),
            start,
            velocity,
            flown: self.distance(),
            range,
            ricochets: self.ricochets + 1,
            pen_factor: flight.pen_factor,
            end: flight.end,
            end_velocity: flight.end_velocity,
            flight_time: flight.flight_time,
            landing: flight.landing,
            through: flight.through,
            burst: None,
        })
    }

    /// How far it flew from its gun to `end`, in metres.
    pub fn distance(&self) -> f32 {
        self.flown + self.start.distance(self.end)
    }

}

/// The index of the first shell there is some left of.
fn first_left(ammo: &[(ShotDef, u32)]) -> Option<usize> {
    ammo.iter().position(|&(_, count)| count > 0)
}

/// A direction off `direction` within the dispersion's angle, as far off as
/// [`dispersed_share`] says, any way around.
fn disperse(direction: Vec3, dispersion: Dispersion, rng: &mut impl Rng) -> Vec3 {
    let off = dispersion.angle * dispersed_share(dispersion.focus, rng);
    let around = rng.r#gen::<f32>() * std::f32::consts::TAU;
    let side = direction.any_orthonormal_vector();
    let up = direction.cross(side);
    (direction * off.cos() + (side * around.cos() + up * around.sin()) * off.sin()).normalize()
}

/// How far off its centre a shot strays, as a share of the dispersion's radius `R`, by
/// its `focus`: a 2D normal distribution of σ = R / k cut at the radius, whose distance
/// is a Rayleigh's, drawn through its inverse. At full focus the radius is
/// [`CENTERED_SIGMAS`] σ; with less, σ grows, to uniform over the disc at none (the
/// limit k → 0 gives √u). k goes with √focus, so that the median share goes about
/// linearly with the focus, from 0.53 (2σ) to 0.71 (uniform).
///
/// ```text
///   k     = CENTERED_SIGMAS · √focus
///   share = √( −2 · ln(1 + u · (e^(−k²/2) − 1)) ) / k,    u uniform in [0, 1]
/// ```
fn dispersed_share(focus: f32, rng: &mut impl Rng) -> f32 {
    let u = rng.r#gen::<f32>();
    let k = CENTERED_SIGMAS * focus.clamp(0.0, 1.0).sqrt();
    if k < 1e-4 {
        return u.sqrt();
    }
    let share = (-2.0 * (u * (-k * k / 2.0).exp_m1()).ln_1p()).sqrt() / k;
    share.min(1.0)
}

/// Where a shell's flight starts, see [`fly`].
#[derive(Debug, Clone, Copy)]
struct Launch {
    start: Vec3,
    velocity: Vec3,
    /// How far it flew before `start`, in metres, see [`Shot::flown`].
    flown: f32,
    /// How far from `start` it flies at most, in metres.
    range: f32,
    /// The share of its shell's penetration it has, see [`Shot::pen_factor`].
    pen_factor: f32,
}

/// A shell's flight, see [`fly`].
#[derive(Debug)]
struct Flight {
    end: Vec3,
    /// The velocity at `end`.
    end_velocity: Vec3,
    /// When it gets to `end`, in seconds after its start.
    flight_time: f32,
    landing: ShotLanding,
    /// What it flew through, in order.
    through: Vec<ShotPiece>,
    /// The share of its shell's penetration it kept, what it flew through taken.
    pen_factor: f32,
}

/// Follow a shell from its launch until the first thing that stops it: a vehicle, the
/// terrain, a static model, or its range. Each step is a segment that is tested against
/// all of them, the nearest along it winning. Destructibles weak enough are flown
/// through, see [`ShotPiece`].
fn fly(def: &ShotDef, launch: Launch, obstacles: Option<ShotObstacles>, vehicles: &[ShotTarget]) -> Flight {

    let Launch { start, velocity, flown, range, mut pen_factor } = launch;
    let mut position = start;
    let mut velocity = velocity;
    let mut time = 0.0;
    let mut through = Vec::new();
    let mut crossed = Vec::new();

    loop {

        let next = position + velocity * FLIGHT_STEP;
        let next_velocity = velocity - Vec3::Y * def.gravity * FLIGHT_STEP;

        // The first thing the step meets, as a share of it, and what it lands on.
        let mut first: Option<(f32, ShotLanding)> = vehicles.iter()
            .filter_map(|vehicle| Some((vehicle, vehicle.collision.first_hit(&vehicle.transforms, position, next)?)))
            .min_by(|a, b| a.1.fraction.total_cmp(&b.1.fraction))
            .map(|(vehicle, hit)| {
                let landing = ShotLanding::Vehicle {
                    entity_id: vehicle.entity_id,
                    hit,
                    transforms: vehicle.transforms,
                };
                (hit.fraction, landing)
            });

        crossed.clear();
        if let Some(obstacles) = obstacles {
            let statics = meet_statics(obstacles, def, position, next, time, &mut crossed);
            let terrain = cross_terrain(obstacles, position, next)
                .map(|t| (t, ShotLanding::Terrain));
            for candidate in [statics, terrain].into_iter().flatten() {
                if first.as_ref().is_none_or(|(t, _)| candidate.0 < *t) {
                    first = Some(candidate);
                }
            }
        }

        // What it flies through before that, each piece once, losing penetration.
        let stop = first.as_ref().map_or(1.0, |(t, _)| *t);
        crossed.sort_by(|a: &(f32, ShotPiece), b| a.0.total_cmp(&b.0));
        for &(t, piece) in crossed.iter().filter(|(t, _)| *t < stop) {
            if through.iter().any(|done: &ShotPiece| done.piece == piece.piece) {
                continue;
            }
            let Some(obstacles) = obstacles else { break };
            let distance = flown + start.distance(position.lerp(next, t));
            let pen = def.pen_at(distance) * pen_factor;
            let left = obstacles.space.destructibles.shoot_through_loss.apply(pen);
            pen_factor = if pen > 0.0 { pen_factor * left / pen } else { 0.0 };
            through.push(piece);
        }

        if let Some((t, landing)) = first {
            return Flight {
                end: position.lerp(next, t),
                end_velocity: velocity.lerp(next_velocity, t),
                flight_time: time + FLIGHT_STEP * t,
                landing,
                through,
                pen_factor,
            };
        }
        if (next - start).length() >= range {
            return Flight {
                end: next,
                end_velocity: next_velocity,
                flight_time: time + FLIGHT_STEP,
                landing: ShotLanding::Range,
                through,
                pen_factor,
            };
        }

        position = next;
        velocity = next_velocity;
        time += FLIGHT_STEP;

    }

}

/// The static triangles a step of a shell from `from` to `to`, starting `time` after its
/// start, meets: the first that stops it, as a share of the step, and in `crossed`, the
/// destructibles it flies through, each with the share of the step it meets them at. A
/// destructible already broken, or that the client doesn't number, isn't met, nor is a
/// triangle shells pass, see [`StaticTriangle::shells_pass`].
///
/// [`StaticTriangle::shells_pass`]: wgtk::space::statics::StaticTriangle::shells_pass
fn meet_statics(
    obstacles: ShotObstacles,
    def: &ShotDef,
    from: Vec3,
    to: Vec3,
    time: f32,
    crossed: &mut Vec<(f32, ShotPiece)>,
) -> Option<(f32, ShotLanding)> {

    let ShotObstacles { space, rubble } = obstacles;
    let statics = &space.statics;
    let min = from.min(to);
    let max = from.max(to);
    let mut stop: Option<(f32, ShotLanding)> = None;
    let explodes = matches!(def.kind(), ShellKind::HighExplosive | ShellKind::ArmorPiercingHe);

    for kind in [BodyKind::Solid, BodyKind::Ramp, BodyKind::Destructible] {
        for triangle in statics.triangles_in(kind, min, max) {
            if triangle.shells_pass {
                continue;
            }
            let Some((t, normal)) = segment_triangle(from, to - from, &triangle.vertices) else { continue };
            let mut met = None;
            if kind == BodyKind::Destructible {
                let Some((piece, destructible)) = rubble.standing(statics, triangle) else { continue };
                let piece_met = ShotPiece {
                    instance: triangle.instance,
                    piece,
                    time: time + FLIGHT_STEP * t,
                };
                let health = destructible.health_of(piece.module).unwrap_or(f32::INFINITY);
                if !explodes && health <= space.destructibles.shoot_through_health {
                    crossed.push((t, piece_met));
                    continue;
                }
                met = Some(piece_met);
            }
            if stop.as_ref().is_none_or(|(first, _)| t < *first) {
                let landing = ShotLanding::Static {
                    normal,
                    piece: met,
                };
                stop = Some((t, landing));
            }
        }
    }

    stop

}

/// Where a step from `from` to `to` goes into the terrain, as a share of it: the ground
/// taken as flat between the two ends.
fn cross_terrain(obstacles: ShotObstacles, from: Vec3, to: Vec3) -> Option<f32> {
    let terrain = &obstacles.space.terrain;
    let ground = terrain.height_at(to.x, to.z)?;
    if to.y > ground {
        return None;
    }
    let above = from.y - terrain.height_at(from.x, from.z).unwrap_or(ground);
    let below = ground - to.y;
    Some((above / (above + below)).clamp(0.0, 1.0))
}


#[cfg(test)]
mod tests {

    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use wgtk::space::destructibles::{DestructibleKind, Module, PenetrationLoss};
    use wgtk::space::statics::{PlacedDestructible, Statics, StaticTriangle};

    use crate::game::collision::{ArmorGroup, CollisionModel, VehiclePart};
    use crate::game::items::{item_type, IntCompactDescr};
    use crate::base::physics::World;

    use super::*;

    /// No dispersion: shots go where the gun points.
    const AIMED: Dispersion = Dispersion {
        angle: 0.0,
        focus: 1.0,
    };

    fn shell() -> ShotDef {
        ShotDef {
            compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id: 1 },
            kind_index: 2,
            caliber: 120.0,
            effects_index: 0,
            pierced_sticker: None,
            prefab_effects_index: None,
            speed: 800.0,
            gravity: 9.81,
            max_distance: 720.0,
            damage_near: 400.0,
            damage_far: 400.0,
            pen_near: 250.0,
            pen_far: 240.0,
            pen_randomization: 0.25,
            normalization_angle: 5f32.to_radians(),
            ricochet_angle_cos: Some(70f32.to_radians().cos()),
            jet_pen_loss: 0.0,
            explosion_radius: 120.0 * 120.0 / 5555.0,
        }
    }

    #[test]
    fn fires_then_reloads() {
        let mut gun = GunLoader::new(vec![(shell(), 2)], 10.0);
        let shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert_eq!(shot.velocity, Vec3::Z * 800.0);
        assert_eq!(shot.landing, ShotLanding::Range);
        assert!((shot.end.length() - 720.0).abs() < 5.0);
        assert!(gun.fire(5.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).is_none());
        assert!(!gun.take_loaded(5.0));
        assert!(gun.take_loaded(10.0));
        assert!(!gun.take_loaded(10.0));
        assert!(gun.fire(10.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).is_some());
        // Out of shells.
        assert!(gun.fire(20.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).is_none());
    }

    #[test]
    fn ricochets_at_the_same_speed_for_the_rest_of_its_range() {
        let mut gun = GunLoader::new(vec![(shell(), 1)], 10.0);
        let mut shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        // As if it hit something 100 m on.
        shot.end = Vec3::Z * 100.0;
        shot.end_velocity = Vec3::Z * 800.0;
        let off = shot.ricochet(Vec3::X, 0.75, None, &[]).unwrap();
        assert_eq!((off.id, off.ricochets, off.flown), (shot.id, 1, 100.0));
        assert!(off.start.distance(shot.end) < 0.01 && off.start.z < 100.0, "{}", off.start);
        assert!((off.velocity - Vec3::X * 800.0).length() < 1e-3, "{}", off.velocity);
        assert!((off.range - 620.0).abs() < 0.1, "{}", off.range);
        assert_eq!(off.pen_factor, 0.75);
        assert_eq!(off.landing, ShotLanding::Range);
        // Out of range where it ends, else at most `MAX_RICOCHETS` times.
        assert!(off.ricochet(-Vec3::X, 1.0, None, &[]).is_none());
        let mut last = off;
        loop {
            last.end = last.start + Vec3::X;
            let Some(next) = last.ricochet(-Vec3::X, 1.0, None, &[]) else { break };
            last = next;
        }
        assert_eq!(last.ricochets, MAX_RICOCHETS);
    }

    fn shell_of(id: u16) -> ShotDef {
        ShotDef { compact_descr: IntCompactDescr { item_type: item_type::SHELL, nation: 5, id }, ..shell() }
    }

    fn loaded_ids(gun: &GunLoader) -> (u16, u16) {
        let (loaded, next) = gun.shells();
        (loaded.unwrap().compact_descr.id, next.unwrap().compact_descr.id)
    }

    #[test]
    fn loads_the_next_shell_after_firing() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (1, 2));
        assert_eq!(gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap().def.compact_descr.id, 1);
        assert_eq!(loaded_ids(&gun), (2, 2));
        // None of the first left to select.
        assert!(!gun.select_next(shell_of(1).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn next_shell_falls_back_once_out() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn changes_the_loaded_shell_quickly_once_loaded() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 5), (shell_of(2), 5)], 10.0).with_quick_change_factor(0.25);
        let second = shell_of(2).compact_descr.to_int();
        assert!(gun.change_loaded(0.0, second));
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.5, end: 2.5 }));
        // Again while reloading: a whole reload.
        assert!(gun.change_loaded(1.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 11.0 }));
        assert!(!gun.is_loaded(10.0));
        assert!(gun.take_loaded(11.0));
        // Already loaded with it.
        assert!(!gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), None);
    }

    fn magazine(size: u32, burst_size: u32) -> MagazineDef {
        MagazineDef { size, interval: 2.0, burst_size, burst_interval: 0.5 }
    }

    fn fire(gun: &mut GunLoader, now: f64) -> Option<Shot> {
        gun.fire(now, Vec3::ZERO, Vec3::Z, AIMED, None, &[])
    }

    fn magazine_shells(gun: &GunLoader) -> Vec<u32> {
        gun.ammo_state().map(|(_, _, magazine_shells)| magazine_shells).collect()
    }

    #[test]
    fn empties_its_magazine_then_reloads_it_whole() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 5), (shell_of(2), 5)], 10.0).with_magazine(magazine(3, 1));
        assert_eq!(magazine_shells(&gun), [3, 0]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(1));
        // The magazine's interval, as a reload of its own for the client to fire again.
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.0, end: 2.0 }));
        assert!(fire(&mut gun, 1.0).is_none());
        assert!(gun.take_loaded(2.0));
        fire(&mut gun, 2.0).unwrap();
        // The next shell is only loaded with the magazine.
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert!(gun.take_loaded(4.0));
        assert_eq!(fire(&mut gun, 4.0).unwrap().def.compact_descr.id, 1);
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 14.0 }));
        assert!(gun.take_loaded(14.0));
        assert_eq!(magazine_shells(&gun), [0, 3]);
    }

    #[test]
    fn fills_its_magazine_with_what_is_left() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 2)], 10.0).with_magazine(magazine(3, 1));
        assert_eq!(magazine_shells(&gun), [2]);
        fire(&mut gun, 0.0).unwrap();
        fire(&mut gun, 2.0).unwrap();
        assert!(fire(&mut gun, 20.0).is_none());
    }

    #[test]
    fn fires_a_burst_then_waits_its_magazine_interval() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(6, 3));
        let first = fire(&mut gun, 0.0).unwrap();
        assert_eq!(first.burst, Some(3));
        // The burst's other shells, then the magazine's interval.
        assert_eq!(gun.reload(), Some(Reload { base_time: 3.0, end: 3.0 }));
        assert!(fire(&mut gun, 0.1).is_none());
        assert!(gun.fire_burst(0.4, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).is_none());
        let second = gun.fire_burst(0.5, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert_eq!((second.burst, second.id), (None, first.id + 1));
        gun.fire_burst(1.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert!(gun.fire_burst(1.5, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).is_none());
        assert_eq!(gun.reload(), Some(Reload { base_time: 3.0, end: 3.0 }));
        assert_eq!(magazine_shells(&gun), [3]);
        // The second burst empties the magazine.
        fire(&mut gun, 3.0).unwrap();
        gun.fire_burst(3.5, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        gun.fire_burst(4.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert_eq!(gun.reload(), Some(Reload { base_time: 11.0, end: 14.0 }));
    }

    #[test]
    fn holds_a_burst_without_a_magazine() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(1, 2));
        assert_eq!(magazine_shells(&gun), [2]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(2));
        gun.fire_burst(0.5, Vec3::ZERO, Vec3::Z, AIMED, None, &[]).unwrap();
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.5, end: 10.5 }));
    }

    #[test]
    fn reloads_a_partial_magazine() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1));
        // Full, of the next shell.
        assert!(!gun.reload_magazine(0.0));
        fire(&mut gun, 0.0).unwrap();
        assert!(gun.reload_magazine(1.0));
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 11.0 }));
        // Already reloading whole.
        assert!(!gun.reload_magazine(2.0));
        // Before the tick tells it, the client may fire as soon as it is loaded.
        assert!(fire(&mut gun, 11.0).is_some());
        assert_eq!(magazine_shells(&gun), [2, 0]);
        // A full magazine of another shell than the next.
        let mut gun = GunLoader::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1));
        gun.select_next(shell_of(2).compact_descr.to_int());
        assert!(gun.reload_magazine(0.0));
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn changes_shells_quickly_only_with_a_full_magazine() {
        let factor = 0.25;
        let mut gun = GunLoader::new(vec![(shell_of(1), 10), (shell_of(2), 10)], 10.0).with_magazine(magazine(3, 1)).with_quick_change_factor(factor);
        fire(&mut gun, 0.0).unwrap();
        assert!(gun.take_loaded(2.0));
        assert!(gun.change_loaded(2.0, shell_of(2).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 10.0, end: 12.0 }));
        assert!(gun.take_loaded(12.0));
        assert_eq!(magazine_shells(&gun), [0, 3]);
        assert!(gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), Some(Reload { base_time: 2.5, end: 14.5 }));
    }

    #[test]
    fn hits_a_vehicle_in_its_way() {
        let mut gun = GunLoader::new(vec![(shell(), 1)], 10.0);
        // A plate 3 m in front of the hull's origin, the hull 100 m ahead.
        let plate = |triangles| CollisionModel { groups: vec![ArmorGroup { name: "armor_1".into(), material_kind: 1, material: None, triangles }], radius: 5.0, bounds: [Vec3::splat(-5.0), Vec3::splat(5.0)] };
        let a = Vec3::new(-2.0, -1.0, -3.0);
        let b = Vec3::new(2.0, -1.0, -3.0);
        let c = Vec3::new(2.0, 1.0, -3.0);
        let d = Vec3::new(-2.0, 1.0, -3.0);
        let collision = VehicleCollision {
            parts: [plate(vec![]), plate(vec![[a, b, c], [a, c, d]]), plate(vec![]), plate(vec![])],
            hull_position: Vec3::ZERO,
        };
        let hull = Affine3A::from_translation(Vec3::new(0.0, 0.0, 100.0));
        let target = ShotTarget { entity_id: 7, collision: Arc::new(collision), transforms: [hull; 4] };
        let shot = gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, None, &[target]).unwrap();
        let ShotLanding::Vehicle { entity_id: 7, hit, .. } = shot.landing else { panic!("missed: {:?}", shot.landing) };
        assert_eq!(hit.part, VehiclePart::Hull);
        assert_eq!(hit.material_kind, 1);
        assert_eq!(hit.normal, -Vec3::Z);
        assert!((shot.end.z - 97.0).abs() < 0.01 && hit.local_point.z == -3.0);
    }

    /// A wall 4 m wide across the way at z = 50, of this kind, as two triangles of
    /// the placed model 0; destructible with this health when it is one, the 2nd of its
    /// chunk, a fence or a house's module; one shells pass if `shells_pass`.
    fn wall(kind: BodyKind, health: f32, house: bool, shells_pass: bool) -> Space {
        let a = Vec3::new(-2.0, -2.0, 50.0);
        let b = Vec3::new(2.0, -2.0, 50.0);
        let c = Vec3::new(2.0, 2.0, 50.0);
        let d = Vec3::new(-2.0, 2.0, 50.0);
        let material_kind = if house { 73 } else { 0 };
        let triangles = [[a, b, c], [a, c, d]]
            .map(|vertices| StaticTriangle {
                shells_pass,
                ..StaticTriangle::new(vertices, 0, material_kind, kind)
            })
            .to_vec();
        let mut statics = Statics::from_triangles(triangles);
        let modules = if house {
            vec![Module {
                material_kind: 73,
                health,
                depends: Vec::new(),
            }]
        } else {
            Vec::new()
        };
        statics.add_destructible(0, PlacedDestructible {
            chunk_id: 0x7f7f,
            index: 1,
            kind: if house { DestructibleKind::Structure } else { DestructibleKind::Fragile },
            health,
            modules,
        });
        let mut space = Space {
            statics,
            ..Space::default()
        };
        space.destructibles.shoot_through_health = 19.0;
        space.destructibles.shoot_through_loss = PenetrationLoss {
            factor: 0.0,
            min: 25.0,
        };
        space
    }

    /// Fire a shell at a space straight along +Z.
    fn fire_at(space: &Space, rubble: &Rubble, def: ShotDef) -> Shot {
        let mut gun = GunLoader::new(vec![(def, 1)], 10.0);
        let obstacles = ShotObstacles {
            space,
            rubble,
        };
        gun.fire(0.0, Vec3::ZERO, Vec3::Z, AIMED, Some(obstacles), &[]).unwrap()
    }

    #[test]
    fn static_models_stop_shells_but_what_they_fly_through() {
        let rubble = Rubble::default();
        // A wall, or a ramp, stops it where it meets it, facing it.
        for kind in [BodyKind::Solid, BodyKind::Ramp] {
            let shot = fire_at(&wall(kind, 0.0, false, false), &rubble, shell());
            let landing = ShotLanding::Static {
                normal: -Vec3::Z,
                piece: None,
            };
            assert_eq!(shot.landing, landing);
            assert!((shot.end.z - 50.0).abs() < 0.01, "{}", shot.end);
            assert!(shot.through.is_empty());
        }
        // A fence of 10 health it flies through, breaking it, 25 mm of its penetration
        // lost.
        let shot = fire_at(&wall(BodyKind::Destructible, 10.0, false, false), &rubble, shell());
        assert_eq!(shot.landing, ShotLanding::Range);
        assert_eq!(shot.through.len(), 1);
        assert_eq!((shot.through[0].instance, shot.through[0].piece.index), (0, 1));
        // When it gets there, not when it lands.
        let at = 50.0 / shell().speed;
        assert!((shot.through[0].time - at).abs() < 0.01, "{} s, not {at} s", shot.through[0].time);
        let pen = shell().pen_at(50.0);
        assert!((shot.pen_factor - (pen - 25.0) / pen).abs() < 1e-3, "{}", shot.pen_factor);
        // An HE shell explodes on that fence instead, hitting it.
        let he = ShotDef {
            kind_index: 1,
            ..shell()
        };
        let shot = fire_at(&wall(BodyKind::Destructible, 10.0, false, false), &rubble, he.clone());
        let ShotLanding::Static { piece: Some(hit), .. } = shot.landing else { panic!("{:?}", shot.landing) };
        assert_eq!(hit.piece.index, 1);
        assert!(shot.through.is_empty());
        // A light fence shells pass, neither stops nor breaks, whatever the shell.
        let light = wall(BodyKind::Destructible, 1.0, false, true);
        for def in [shell(), he] {
            let shot = fire_at(&light, &rubble, def);
            assert_eq!(shot.landing, ShotLanding::Range);
            assert!(shot.through.is_empty());
        }
        // A house's module of 50 health stops it, hit.
        let shot = fire_at(&wall(BodyKind::Destructible, 50.0, true, false), &rubble, shell());
        let ShotLanding::Static { piece: Some(hit), .. } = shot.landing else { panic!("{:?}", shot.landing) };
        assert_eq!((hit.instance, hit.piece.module), (0, Some(73)));
        assert!(shot.through.is_empty());
        // Shells' damage adds up on it: 30 then 30 of its 50 break it, once.
        let space = wall(BodyKind::Destructible, 50.0, true, false);
        let mut world = World::new(Arc::new(space), None);
        assert!(world.shoot(0, hit.piece, 30.0, 0.0).is_empty());
        assert_eq!(world.shoot(0, hit.piece, 30.0, 0.0).len(), 1);
        assert!(world.shoot(0, hit.piece, 30.0, 0.0).is_empty());
        // Once broken, it isn't there.
        let shot = fire_at(world.space(), world.rubble(), shell());
        assert_eq!(shot.landing, ShotLanding::Range);
        assert_eq!(world.take_breakages().count(), 1);
    }

    #[test]
    fn dispersion_stays_within_its_angle() {
        let mut rng = rand::thread_rng();
        for focus in [0.0, 0.5, 1.0] {
            let dispersion = Dispersion {
                angle: 0.01,
                focus,
            };
            for _ in 0..1000 {
                let direction = disperse(Vec3::Z, dispersion, &mut rng);
                assert!(direction.angle_between(Vec3::Z) <= 0.01 + 1e-5);
            }
        }
    }

    #[test]
    fn dispersion_gathers_with_its_focus() {

        let mut rng = StdRng::seed_from_u64(1);

        // Uniform over the disc: half the shots within 1/√2 of the radius.
        let uniform = median_share(0.0, &mut rng);
        assert!((uniform - 0.5f32.sqrt()).abs() < 0.01, "median {uniform}");
        // Stopped, the radius is 2σ: half within 0.53 of it.
        let centred = median_share(1.0, &mut rng);
        assert!((centred - 0.532).abs() < 0.01, "median {centred}");
        // Halfway, about halfway.
        let half = median_share(0.5, &mut rng);
        assert!((half - (uniform + centred) / 2.0).abs() < 0.01, "median {half}");

    }

    /// The median of many [`dispersed_share`]s at this focus.
    fn median_share(focus: f32, rng: &mut StdRng) -> f32 {
        let mut shares = (0..20_000).map(|_| dispersed_share(focus, rng)).collect::<Vec<_>>();
        shares.sort_by(f32::total_cmp);
        shares[shares.len() / 2]
    }

}
