//! A battle's shells in flight, flown on each tick against its vehicles where they are
//! then, what they fly through breaking as they get there; and their tracers, as the
//! clients are shown them.

use std::io;

use wgtk::app::entity::{EntityId, EntityKey};
use wgtk::app::base;

use crate::base::ballistics::shot::{Shot, ShotLanding, ShotObstacles, ShotPiece, ShotTarget};
use crate::game::schema::{self, Avatar};
use crate::base::NO_PREFAB_EFFECTS;


/// The shells in flight of a battle, see the module's documentation.
#[derive(Debug, Default)]
pub struct Shells {
    /// Each shell, with the game time it started flying at and who fired it.
    flying: Vec<(f64, EntityId, Shot)>,
}

impl Shells {

    /// A shell fired by `shooter` at game time `fired`, or ricocheting then: it flies
    /// until it lands, see [`Self::step`].
    pub fn fly(&mut self, fired: f64, shooter: EntityId, shot: Shot) {
        self.flying.push((fired, shooter, shot));
    }

    /// Fly the shells on until game time `now`, see [`Shot::fly`], against these
    /// vehicles, where they are now: what they flew through since, each with the yaw its
    /// shell went, and those that landed, each with the game time it landed at, who fired
    /// it, and what it landed on.
    pub fn step(
        &mut self,
        now: f64,
        obstacles: ShotObstacles,
        vehicles: &[ShotTarget],
    ) -> (Vec<(ShotPiece, f32)>, Vec<(f64, EntityId, Shot, ShotLanding)>) {

        let mut through = Vec::new();
        let mut landed = Vec::new();
        let mut index = 0;

        while index < self.flying.len() {

            let (fired, shooter, shot) = &mut self.flying[index];
            let crossed = shot.through.len();
            let landing = shot.fly((now - *fired) as f32, Some(obstacles), vehicles, shooter.0);

            let yaw = shot.start_velocity.x.atan2(shot.start_velocity.z);
            through.extend(shot.through[crossed..].iter().map(|&piece| (piece, yaw)));

            if let Some(landing) = landing {
                let (fired, shooter, shot) = self.flying.swap_remove(index);
                landed.push((fired + f64::from(shot.flight_time), shooter, shot, landing));
            } else {
                index += 1;
            }

        }

        (through, landed)

    }

}

/// Show a shot to the players of these avatars: its tracer, and the gun firing on the
/// shooter.
pub fn show_shot(app: &mut base::App, avatars: &[EntityKey<Avatar>], shooter: EntityKey<schema::Vehicle>, shot: &Shot) -> io::Result<()> {
    show_tracer(app, avatars, shooter.id(), shot, false)?;
    // Its whole burst from the only gun, on its first shell.
    match shot.burst {
        Some(burst) => app.entity(shooter)?.showShooting(burst.min(u8::MAX.into()) as u8, 0, shot.def.kind_index),
        None => Ok(()),
    }
}

/// Show a shell's tracer to the players of these avatars, flying from its start, off
/// what it ricocheted off for a ricochet: the client then shows it from there, not from
/// the gun.
pub fn show_tracer(app: &mut base::App, avatars: &[EntityKey<Avatar>], shooter: EntityId, shot: &Shot, ricochet: bool) -> io::Result<()> {
    let def = &shot.def;
    for &avatar in avatars {
        app.entity(avatar)?.showTracer(
            shooter,
            shot.id,
            ricochet.into(),
            def.effects_index,
            def.prefab_effects_index.unwrap_or(NO_PREFAB_EFFECTS),
            def.kind_index,
            def.caliber,
            shot.start,
            shot.start_velocity,
            def.gravity,
            shot.range,
            0,
            0,
        )?;
    }
    Ok(())
}
