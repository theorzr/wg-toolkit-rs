//! A battle's shells in flight: when each lands, and what it flies through on its way,
//! which breaks as it gets there; and their tracers, as the clients are shown them.

use std::io;

use wgtk::app::entity::{EntityId, EntityKey};
use wgtk::app::base;

use crate::base::ballistics::gun::{Shot, ShotPiece};
use crate::game::schema::{self, Avatar};
use crate::base::NO_PREFAB_EFFECTS;


/// The shells in flight of a battle, see the module's documentation.
#[derive(Debug, Default)]
pub struct Shells {
    /// Each shell, with the game time it lands at and who fired it.
    flying: Vec<(f64, EntityId, Shot)>,
    /// What they fly through, with the game time they get there and the yaw they go:
    /// it breaks then, see [`Shot::through`].
    through: Vec<(f64, ShotPiece, f32)>,
}

impl Shells {

    /// A shell fired by `shooter` at game time `fired`, or ricocheting then: it flies
    /// until it lands.
    pub fn fly(&mut self, fired: f64, shooter: EntityId, shot: Shot) {
        let yaw = shot.velocity.x.atan2(shot.velocity.z);
        for &through in &shot.through {
            self.through.push((fired + f64::from(through.time), through, yaw));
        }
        self.flying.push((fired + f64::from(shot.flight_time), shooter, shot));
    }

    /// What the shells flew through by game time `now`, since last asked, with the yaw
    /// they went.
    pub fn take_through(&mut self, now: f64) -> Vec<(ShotPiece, f32)> {
        let (reached, ahead): (Vec<_>, _) = std::mem::take(&mut self.through)
            .into_iter()
            .partition(|&(at, _, _)| at <= now);
        self.through = ahead;
        reached.into_iter().map(|(_, through, yaw)| (through, yaw)).collect()
    }

    /// The shells landed by game time `now`, since last asked, with the game time each
    /// landed at and who fired it.
    pub fn take_landed(&mut self, now: f64) -> Vec<(f64, EntityId, Shot)> {
        let (landed, flying): (Vec<_>, _) = std::mem::take(&mut self.flying)
            .into_iter()
            .partition(|&(at, _, _)| at <= now);
        self.flying = flying;
        landed
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
            shot.velocity,
            def.gravity,
            shot.range,
            0,
            0,
        )?;
    }
    Ok(())
}
