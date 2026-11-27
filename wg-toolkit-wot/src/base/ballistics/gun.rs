//! A vehicle's gun as the server runs it: its ammunition, the shell loaded and the one
//! it loads next, its magazine and bursts, and its reload, as the client expects them,
//! see [`GunLoader`]; and where its shells go, off its aim within its dispersion, see
//! [`disperse`]. A shell fired then flies on its own, see [`Shot`].
//!
//! ```text
//!   loaded           fire: a shell, the first of its burst, the reload starting then
//!   in a burst       fire_burst: the next shell, every burst interval
//!   after them       the magazine's interval, while it has shells left
//!   magazine empty   a whole reload, of the next shell, the magazine full again
//! ```

use std::f32::consts::TAU;

use glam::Vec3;
use rand::Rng;

use crate::game::item_defs::{MagazineDef, ShotDef};
use crate::base::physics::Dispersion;

use super::shot::Shot;


/// How many σ of its shots' normal distribution the dispersion's radius is, stopped, see
/// [`dispersed_share`]. Our choice.
const CENTERED_SIGMAS: f32 = 2.0;


/// A vehicle's gun, see the module's documentation.
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
    /// The length of a whole reload: of the magazine, for a gun with one, see
    /// [`Self::reload_time`].
    reload_time: f32,
    /// What its whole reloads take as long as, see [`Self::set_reload_factor`].
    reload_factor: f32,
    /// How long its reloads and its magazine's intervals take instead of their own, in
    /// seconds, its shells fired not counted then, see [`Self::set_forced_reload`].
    forced_reload: Option<f32>,
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

impl GunLoader {

    // -- Made --

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
            reload_factor: 1.0,
            forced_reload: None,
            quick_change_factor: 1.0,
            reload: None,
            next_shot_id: 1,
        };

        gun.magazine_shells = gun.full_magazine();
        gun

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

    // -- Firing --

    /// Fire the loaded shell at game time `now` from `start` toward `direction`, off
    /// within the `dispersion`, see [`disperse`], if the gun is loaded and has a shell
    /// left: the first of a burst, which [`Self::fire_burst`] fires the rest of. Once
    /// they are fired, the gun waits for its magazine's interval while it has some left,
    /// else reloads whole with its next shell, or another one once there is none of it
    /// left. That reload starts now, the burst's other shells in it, for the client to
    /// know when it can fire again as soon as it fires.
    pub fn fire(
        &mut self,
        now: f64,
        start: Vec3,
        direction: Vec3,
        dispersion: Dispersion,
    ) -> Option<Shot> {

        self.settle(now);
        if self.burst_shells.is_some() || !self.is_loaded(now) || self.magazine_shells == 0 {
            return None;
        }

        let burst = self.magazine.burst_size.min(self.magazine_shells);
        let mut shot = self.shoot(start, direction, dispersion)?;
        shot.burst = Some(burst);

        // The rest of its burst to come, then the magazine's interval, or a whole reload
        // once the burst empties it.
        if burst > 1 {
            let burst_time = (burst - 1) as f32 * self.magazine.burst_interval;
            let wait = if self.magazine_shells > burst - 1 { self.interval() } else { self.reload_time() };
            self.burst_shells = Some((burst - 1, now + f64::from(self.magazine.burst_interval)));
            self.start_reload(now, burst_time + wait);
        } else {
            self.after_shots(now);
        }

        Some(shot)

    }

    /// Fire the next shell of the burst going on, if it is due by game time `now`, as
    /// [`Self::fire`] does, whose reload goes on. Once the burst empties the magazine,
    /// the gun loads its next shell.
    pub fn fire_burst(
        &mut self,
        now: f64,
        start: Vec3,
        direction: Vec3,
        dispersion: Dispersion,
    ) -> Option<Shot> {

        let (left, at) = self.burst_shells.filter(|&(_, at)| now >= at)?;
        let Some(shot) = self.shoot(start, direction, dispersion) else {
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

    /// Fire a shell of the magazine, without waiting or reloading: it starts flying.
    fn shoot(&mut self, start: Vec3, direction: Vec3, dispersion: Dispersion) -> Option<Shot> {

        if self.magazine_shells == 0 {
            return None;
        }

        // One shell fewer, of the magazine and of the ammunition, unless its reload is
        // forced.
        let (def, count) = self.ammo.get_mut(self.loaded_shell).filter(|(_, count)| *count > 0)?;
        if self.forced_reload.is_none() {
            *count -= 1;
        }
        self.magazine_shells -= 1;
        let def = def.clone();

        let id = self.next_shot_id;
        self.next_shot_id += 1;

        let velocity = disperse(direction.normalize(), dispersion, &mut rand::thread_rng()) * def.speed;
        Some(Shot::new(id, def, start, velocity))

    }

    /// After a shot or a burst: wait for the magazine's interval while it has some left,
    /// else reload whole with the next shell.
    fn after_shots(&mut self, now: f64) {
        if self.magazine_shells > 0 {
            self.start_reload(now, self.interval());
        } else {
            self.load_next();
            self.start_reload(now, self.reload_time());
        }
    }

    // -- Its shells --

    /// Load the shell of this integer descriptor once the gun fires, if it has some
    /// left: `VEHICLE_SETTING.NEXT_SHELLS`. Whether it does.
    pub fn select_next(&mut self, compact_descr: i64) -> bool {
        let Some(index) = self.shell_left(compact_descr) else { return false };
        self.next_shell = index;
        true
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

        // Quickly, only from a full magazine, loaded, with a quick change factor.
        let full = self.magazine_shells > 0 && self.magazine_shells == self.full_magazine();
        let factor = self.quick_change_factor;
        let quick = full && self.is_loaded(now) && factor > 0.0 && factor < 1.0;
        let time = if quick { self.reload_time() * factor } else { self.reload_time() };

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
        self.start_reload(now, self.reload_time());
        true

    }

    /// Load the next shell, or another one once there is none of it left.
    fn load_next(&mut self) {
        if !self.has_left(self.next_shell) {
            self.next_shell = first_left(&self.ammo).unwrap_or(self.next_shell);
        }
        self.loaded_shell = self.next_shell;
    }

    /// The shell loaded, and the shell it loads after.
    pub fn shells(&self) -> (Option<&ShotDef>, Option<&ShotDef>) {
        let loaded = self.ammo.get(self.loaded_shell).map(|(def, _)| def);
        let next = self.ammo.get(self.next_shell).map(|(def, _)| def);
        (loaded, next)
    }

    /// Each shell, how many are left of it, and how many of them are in the magazine:
    /// none without one, as captured, and only the loaded shell's with one.
    pub fn ammo_state(&self) -> impl Iterator<Item = (&ShotDef, u32, u32)> {
        let magazine_shells = if self.magazine_size() > 1 { self.magazine_shells } else { 0 };
        self.ammo.iter()
            .enumerate()
            .map(move |(index, (def, count))| {
                let in_magazine = if index == self.loaded_shell { magazine_shells } else { 0 };
                (def, *count, in_magazine)
            })
    }

    /// Whether the gun has a magazine, or fires bursts: shells come more than one per
    /// reload then.
    pub fn has_magazine(&self) -> bool {
        self.magazine_size() > 1
    }

    /// The index of the shell of this integer descriptor, if the gun has some left.
    fn shell_left(&self, compact_descr: i64) -> Option<usize> {
        self.ammo.iter()
            .position(|&(ref def, count)| def.compact_descr.to_int() == compact_descr && count > 0)
    }

    /// Whether the gun has some left of its shell of this index.
    fn has_left(&self, index: usize) -> bool {
        self.ammo.get(index).is_some_and(|&(_, count)| count > 0)
    }

    // -- Its reload --

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

    /// The length of a whole reload, in seconds, as long as its factor has it, unless
    /// forced.
    pub fn reload_time(&self) -> f32 {
        self.forced_reload.unwrap_or(self.reload_time * self.reload_factor)
    }

    /// The wait between two shots of its magazine, in seconds, unless forced as its
    /// reloads are.
    fn interval(&self) -> f32 {
        self.forced_reload.unwrap_or(self.magazine.interval)
    }

    /// Have its whole reloads from now on take as long as this factor of their own, a
    /// stunned crew's longer, the one going on as it is.
    pub fn set_reload_factor(&mut self, factor: f32) {
        self.reload_factor = factor;
    }

    /// How long its reloads and its magazine's intervals take instead of their own, in
    /// seconds, see [`Self::set_forced_reload`].
    pub fn forced_reload(&self) -> Option<f32> {
        self.forced_reload
    }

    /// Have its reloads and its magazine's intervals take this many seconds from game
    /// time `now` on, the one going on ending now, and its shells fired not counted, or
    /// as long as their own again. A burst's shells keep their interval.
    pub fn set_forced_reload(&mut self, now: f64, time: Option<f32>) {
        self.forced_reload = time;
        if let Some(reload) = self.reload.as_mut().filter(|_| time.is_some()) {
            reload.end = reload.end.min(now);
        }
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

    /// Start a reload of this many seconds at game time `now`.
    fn start_reload(&mut self, now: f64, time: f32) {
        self.reload = Some(Reload {
            base_time: time,
            end: now + f64::from(time),
        });
    }

    /// The shells of `loaded_shell` the gun holds once reloaded whole, fewer when it has
    /// fewer left.
    fn full_magazine(&self) -> u32 {
        self.ammo.get(self.loaded_shell).map_or(0, |&(_, count)| count.min(self.magazine_size()))
    }

    /// The shells the gun holds once reloaded: its magazine's, or its burst's when it
    /// fires bursts without a magazine, as the client counts them; 1 without either.
    fn magazine_size(&self) -> u32 {
        self.magazine.size.max(self.magazine.burst_size).max(1)
    }

}

/// The index of the first shell of `ammo` some are left of.
fn first_left(ammo: &[(ShotDef, u32)]) -> Option<usize> {
    ammo.iter().position(|&(_, count)| count > 0)
}

/// A direction off `direction` within the dispersion's angle, as far off as
/// [`dispersed_share`] says, any way around.
fn disperse(direction: Vec3, dispersion: Dispersion, rng: &mut impl Rng) -> Vec3 {

    // How far off, and which way around.
    let off = dispersion.angle * dispersed_share(dispersion.focus, rng);
    let around = rng.r#gen::<f32>() * TAU;

    // Turned off `direction` that far, toward that way.
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


#[cfg(test)]
mod tests {

    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use crate::game::items::{item_type, IntCompactDescr};
    use crate::game::item_defs::ShellKind;

    use super::super::fixtures::shot;
    use super::*;

    /// No dispersion: shots go where the gun points.
    const AIMED: Dispersion = Dispersion {
        angle: 0.0,
        focus: 1.0,
    };

    /// An AP shell of this id.
    fn shell_of(id: u16) -> ShotDef {
        ShotDef {
            compact_descr: IntCompactDescr {
                item_type: item_type::SHELL,
                nation: 5,
                id,
            },
            ..shot(ShellKind::ArmorPiercing, 120.0, 250.0)
        }
    }

    /// A magazine of this size, of shots 2 s apart, of bursts of this size, of shots
    /// 0.5 s apart.
    fn magazine(size: u32, burst_size: u32) -> MagazineDef {
        MagazineDef {
            size,
            interval: 2.0,
            burst_size,
            burst_interval: 0.5,
        }
    }

    /// A reload of this length, ending at this game time.
    fn reloading(base_time: f32, end: f64) -> Option<Reload> {
        Some(Reload {
            base_time,
            end,
        })
    }

    /// Fire it at game time `now` from the origin along +Z.
    fn fire(gun: &mut GunLoader, now: f64) -> Option<Shot> {
        gun.fire(now, Vec3::ZERO, Vec3::Z, AIMED)
    }

    /// Fire its burst on at game time `now` from the origin along +Z.
    fn fire_burst(gun: &mut GunLoader, now: f64) -> Option<Shot> {
        gun.fire_burst(now, Vec3::ZERO, Vec3::Z, AIMED)
    }

    /// The ids of the shell loaded and of the shell it loads after.
    fn loaded_ids(gun: &GunLoader) -> (u16, u16) {
        let (loaded, next) = gun.shells();
        (loaded.unwrap().compact_descr.id, next.unwrap().compact_descr.id)
    }

    /// How many of each shell are in the magazine.
    fn magazine_shells(gun: &GunLoader) -> Vec<u32> {
        gun.ammo_state()
            .map(|(_, _, magazine_shells)| magazine_shells)
            .collect()
    }

    #[test]
    fn fires_then_reloads() {

        let mut gun = GunLoader::new(vec![(shell_of(1), 2)], 10.0);
        let shot = fire(&mut gun, 0.0).unwrap();
        assert_eq!(shot.start_velocity, Vec3::Z * 800.0);

        assert!(fire(&mut gun, 5.0).is_none());
        assert!(!gun.take_loaded(5.0));
        assert!(gun.take_loaded(10.0));
        assert!(!gun.take_loaded(10.0));
        assert!(fire(&mut gun, 10.0).is_some());

        // Out of shells.
        assert!(fire(&mut gun, 20.0).is_none());

    }

    #[test]
    fn loads_the_next_shell_after_firing() {

        let mut gun = GunLoader::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (1, 2));
        assert_eq!(fire(&mut gun, 0.0).unwrap().def.compact_descr.id, 1);
        assert_eq!(loaded_ids(&gun), (2, 2));

        // None of the first left to select.
        assert!(!gun.select_next(shell_of(1).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (2, 2));

    }

    #[test]
    fn next_shell_falls_back_once_out() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 1), (shell_of(2), 5)], 10.0);
        fire(&mut gun, 0.0).unwrap();
        assert_eq!(loaded_ids(&gun), (2, 2));
    }

    #[test]
    fn changes_the_loaded_shell_quickly_once_loaded() {

        let ammo = vec![(shell_of(1), 5), (shell_of(2), 5)];
        let mut gun = GunLoader::new(ammo, 10.0).with_quick_change_factor(0.25);
        assert!(gun.change_loaded(0.0, shell_of(2).compact_descr.to_int()));
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), reloading(2.5, 2.5));

        // Again while reloading: a whole reload.
        assert!(gun.change_loaded(1.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), reloading(10.0, 11.0));
        assert!(!gun.is_loaded(10.0));
        assert!(gun.take_loaded(11.0));

        // Already loaded with it.
        assert!(!gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), None);

    }

    #[test]
    fn empties_its_magazine_then_reloads_it_whole() {

        let ammo = vec![(shell_of(1), 5), (shell_of(2), 5)];
        let mut gun = GunLoader::new(ammo, 10.0).with_magazine(magazine(3, 1));
        assert_eq!(magazine_shells(&gun), [3, 0]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(1));

        // The magazine's interval, as a reload of its own for the client to fire again.
        assert_eq!(gun.reload(), reloading(2.0, 2.0));
        assert!(fire(&mut gun, 1.0).is_none());
        assert!(gun.take_loaded(2.0));
        fire(&mut gun, 2.0).unwrap();

        // The next shell is only loaded with the magazine.
        assert!(gun.select_next(shell_of(2).compact_descr.to_int()));
        assert!(gun.take_loaded(4.0));
        assert_eq!(fire(&mut gun, 4.0).unwrap().def.compact_descr.id, 1);
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(loaded_ids(&gun), (2, 2));
        assert_eq!(gun.reload(), reloading(10.0, 14.0));
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
        assert_eq!(gun.reload(), reloading(3.0, 3.0));
        assert!(fire(&mut gun, 0.1).is_none());
        assert!(fire_burst(&mut gun, 0.4).is_none());
        let second = fire_burst(&mut gun, 0.5).unwrap();
        assert_eq!((second.burst, second.id), (None, first.id + 1));
        fire_burst(&mut gun, 1.0).unwrap();
        assert!(fire_burst(&mut gun, 1.5).is_none());
        assert_eq!(gun.reload(), reloading(3.0, 3.0));
        assert_eq!(magazine_shells(&gun), [3]);

        // The second burst empties the magazine.
        fire(&mut gun, 3.0).unwrap();
        fire_burst(&mut gun, 3.5).unwrap();
        fire_burst(&mut gun, 4.0).unwrap();
        assert_eq!(gun.reload(), reloading(11.0, 14.0));

    }

    #[test]
    fn holds_a_burst_without_a_magazine() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(1, 2));
        assert_eq!(magazine_shells(&gun), [2]);
        assert_eq!(fire(&mut gun, 0.0).unwrap().burst, Some(2));
        fire_burst(&mut gun, 0.5).unwrap();
        assert_eq!(gun.reload(), reloading(10.5, 10.5));
    }

    #[test]
    fn reloads_a_partial_magazine() {

        // Full, of the next shell.
        let ammo = vec![(shell_of(1), 10), (shell_of(2), 10)];
        let mut gun = GunLoader::new(ammo.clone(), 10.0).with_magazine(magazine(3, 1));
        assert!(!gun.reload_magazine(0.0));
        fire(&mut gun, 0.0).unwrap();
        assert!(gun.reload_magazine(1.0));
        assert_eq!(magazine_shells(&gun), [0, 0]);
        assert_eq!(gun.reload(), reloading(10.0, 11.0));

        // Already reloading whole.
        assert!(!gun.reload_magazine(2.0));

        // Before the tick tells it, the client may fire as soon as it is loaded.
        assert!(fire(&mut gun, 11.0).is_some());
        assert_eq!(magazine_shells(&gun), [2, 0]);

        // A full magazine of another shell than the next.
        let mut gun = GunLoader::new(ammo, 10.0).with_magazine(magazine(3, 1));
        gun.select_next(shell_of(2).compact_descr.to_int());
        assert!(gun.reload_magazine(0.0));
        assert_eq!(loaded_ids(&gun), (2, 2));

    }

    #[test]
    fn changes_shells_quickly_only_with_a_full_magazine() {

        let ammo = vec![(shell_of(1), 10), (shell_of(2), 10)];
        let mut gun = GunLoader::new(ammo, 10.0)
            .with_magazine(magazine(3, 1))
            .with_quick_change_factor(0.25);

        fire(&mut gun, 0.0).unwrap();
        assert!(gun.take_loaded(2.0));
        assert!(gun.change_loaded(2.0, shell_of(2).compact_descr.to_int()));
        assert_eq!(gun.reload(), reloading(10.0, 12.0));

        assert!(gun.take_loaded(12.0));
        assert_eq!(magazine_shells(&gun), [0, 3]);
        assert!(gun.change_loaded(12.0, shell_of(1).compact_descr.to_int()));
        assert_eq!(gun.reload(), reloading(2.5, 14.5));

    }

    #[test]
    fn reloads_as_forced_once_set() {

        let mut gun = GunLoader::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(2, 1));
        fire(&mut gun, 0.0).unwrap();
        fire(&mut gun, 2.0).unwrap();
        assert_eq!(gun.reload(), reloading(10.0, 12.0));

        // The reload going on ends now, the next ones take the forced time, the shells
        // fired not counted.
        gun.set_forced_reload(5.0, Some(0.5));
        assert!(gun.take_loaded(5.0));
        fire(&mut gun, 5.0).unwrap();
        assert_eq!(gun.reload(), reloading(0.5, 5.5));
        assert!(fire(&mut gun, 5.4).is_none());
        fire(&mut gun, 5.5).unwrap();
        assert_eq!(gun.reload(), reloading(0.5, 6.0));
        assert!(gun.take_loaded(6.0));
        fire(&mut gun, 6.0).unwrap();
        assert_eq!(gun.ammo[0].1, 18);

        // As long as they should again, once its magazine is empty.
        gun.set_forced_reload(6.0, None);
        fire(&mut gun, 6.5).unwrap();
        assert_eq!(gun.reload(), reloading(10.0, 16.5));
        assert!(fire(&mut gun, 7.0).is_none());

    }

    #[test]
    fn forced_reloads_keep_the_burst_interval() {
        let mut gun = GunLoader::new(vec![(shell_of(1), 20)], 10.0).with_magazine(magazine(6, 3));
        gun.set_forced_reload(0.0, Some(0.5));
        fire(&mut gun, 0.0).unwrap();
        assert_eq!(gun.reload(), reloading(1.5, 1.5));
        assert!(fire_burst(&mut gun, 0.4).is_none());
        fire_burst(&mut gun, 0.5).unwrap();
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
        let mut shares = (0..20_000)
            .map(|_| dispersed_share(focus, rng))
            .collect::<Vec<_>>();
        shares.sort_by(f32::total_cmp);
        shares[shares.len() / 2]
    }

}
