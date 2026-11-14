//! A client in a battle: its avatar, its vehicle, its own arena info, what it sees of the
//! others' vehicles, and whether it is done loading.

use std::io;

use tracing::info;

use wgtk::app::entity::{ComponentKey, EntityKey};
use wgtk::app::base::{self, ClientKey};

use crate::game::schema::{Avatar, AvatarVehiclesInfo};
use crate::base::{VehicleLoadout, TICK};
use crate::base::physics::World;

use super::vehicle::BattleVehicle;
use super::period::ArenaPeriod;
use super::VIEW_RANGE;


/// See the module's documentation.
#[derive(Debug)]
pub struct Participant {
    pub client: ClientKey,
    pub avatar: EntityKey<Avatar>,
    /// Its vehicle's configuration, and its vehicle, by its index among the battle's.
    pub loadout: VehicleLoadout,
    pub vehicle: usize,
    /// The arena's vehicles, on its own info entity.
    pub vehicles_info: ComponentKey<AvatarVehiclesInfo>,
    /// Whether it sees each of the battle's vehicles, by index, see [`Self::spot`].
    spotted: Vec<bool>,
    /// Whether its client is done loading, ready for the countdown.
    pub ready: bool,
}

impl Participant {

    /// A client in a battle of `vehicles` vehicles, with its avatar and its vehicle's.
    pub fn new(
        client: ClientKey,
        avatar: EntityKey<Avatar>,
        loadout: VehicleLoadout,
        vehicle: usize,
        vehicles_info: ComponentKey<AvatarVehiclesInfo>,
        vehicles: usize,
    ) -> Self {
        Self {
            client,
            avatar,
            loadout,
            vehicle,
            vehicles_info,
            spotted: vec![false; vehicles],
            ready: false,
        }
    }

    /// Show it the battle's vehicles, or hide them, as its own vehicle spots them: its
    /// allies always, its enemies within [`VIEW_RANGE`]. Its own it always sees.
    pub fn spot(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World) -> io::Result<()> {
        let own = &vehicles[self.vehicle];
        let position = world.vehicle(own.id).hull.position;
        for (index, vehicle) in vehicles.iter().enumerate() {
            if index == self.vehicle {
                continue;
            }
            let offset = world.vehicle(vehicle.id).hull.position - position;
            let spotted = vehicle.team == own.team || offset.x.hypot(offset.z) <= VIEW_RANGE;
            if spotted == self.spotted[index] {
                continue;
            }
            self.spotted[index] = spotted;
            if spotted {
                app.show(self.client, vehicle.entity)?;
            } else {
                app.hide(self.client, vehicle.entity)?;
            }
            info!(client = %self.client, entity_id = %vehicle.entity.id(), team = vehicle.team, "{} a vehicle", if spotted { "Spotted" } else { "Lost" });
        }
        Ok(())
    }

    /// Tell its client the battle's period, as long as is left of it: its game time is
    /// the ticks it got. Once over, it is first told who won, which it shows.
    pub fn show_period(&self, app: &mut base::App, period: ArenaPeriod) -> io::Result<()> {

        if let ArenaPeriod::AfterBattle { winner, reason, .. } = period {
            app.entity(self.avatar)?.onRoundFinished(winner as i8, reason)?;
            info!(client = %self.client, winner, reason, "Told the battle's end");
        }

        let game_time = f64::from(app.ticks(self.client).unwrap_or(0)) * TICK.as_secs_f64();
        let Some((kind, data)) = period.arena_update(game_time)? else { return Ok(()) };
        app.entity(self.avatar)?.updateArena(kind, data)?;
        info!(client = %self.client, ?period, "Changed the arena period");
        Ok(())

    }

}
