//! What a bot does, by its vehicle's class, see [`BotRole`]: the posts it holds, and what
//! it likes in a spot to fire from.

use crate::game::item_defs::VehicleClass;
use crate::base::physics::Vehicle;
use crate::base::poi::PoiActivity;

use super::tactics::effective_range;
use super::CLOSE_RANGE;


/// What a bot does, by its vehicle's class: the posts it holds, see
/// [`BotPost`](super::post::BotPost), and what it likes in a spot to fire from, see
/// [`FiringSpotSearch`](super::spot::FiringSpotSearch). Heavies close in and trust their
/// armour, mediums support from behind or go round to their target's side or rear, or
/// wait while it aims at them, lights spot and keep their distance out of sight, tank
/// destroyers hide in foliage far off, artillery fires from far behind. Our choice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotRole {
    /// The activities it holds a post for, at a point where they go on, see [`PoiActivity`].
    pub posts: &'static [PoiActivity],
    /// How far from its target it likes to fire, its share of its gun's reach, see
    /// [`effective_range`], no nearer than [`CLOSE_RANGE`].
    pub range: f32,
    /// How much it likes plants round it, see `spot::COVER_RADIUS`.
    pub cover: f32,
    /// How much it likes being by its target's side or rear rather than its front.
    pub flank: f32,
    /// How much less it likes a spot each other enemy sees.
    pub exposure: f32,
    /// How much it likes its target's marker showing red on its armour, see
    /// `impact::indicate`.
    pub armor: f32,
    /// Whether it waits out of its target's sight while its target aims at it, loaded,
    /// for another to take its shot.
    pub patient: bool,
}

impl BotRole {

    pub fn of(class: VehicleClass) -> Self {
        match class {
            VehicleClass::Heavy => Self {
                posts: &[
                    PoiActivity::CloseCombat,
                    PoiActivity::SideScrape,
                    PoiActivity::HullDown,
                ],
                range: 0.3,
                cover: 0.0,
                flank: 0.0,
                exposure: 0.05,
                armor: 0.6,
                patient: false,
            },
            VehicleClass::Medium => Self {
                posts: &[
                    PoiActivity::Support,
                    PoiActivity::HullDown,
                    PoiActivity::SideScrape,
                    PoiActivity::Ambush,
                ],
                range: 0.6,
                cover: 0.3,
                flank: 0.4,
                exposure: 0.2,
                armor: 0.2,
                patient: true,
            },
            VehicleClass::Light => Self {
                posts: &[
                    PoiActivity::Spotting,
                    PoiActivity::CloseSpotting,
                ],
                range: 0.9,
                cover: 0.5,
                flank: 0.2,
                exposure: 0.3,
                armor: 0.0,
                patient: false,
            },
            VehicleClass::TankDestroyer => Self {
                posts: &[
                    PoiActivity::LongRangeFire,
                    PoiActivity::Ambush,
                    PoiActivity::Support,
                ],
                range: 1.0,
                cover: 1.0,
                flank: 0.0,
                exposure: 0.3,
                armor: 0.2,
                patient: false,
            },
            VehicleClass::Artillery => Self {
                posts: &[
                    PoiActivity::IndirectFire,
                ],
                range: 1.0,
                cover: 1.0,
                flank: 0.0,
                exposure: 0.3,
                armor: 0.2,
                patient: false,
            },
        }
    }

    /// How far from its target a bot of this role fires from in this vehicle, see
    /// [`Self::range`].
    pub fn range(&self, vehicle: &Vehicle) -> f32 {
        (effective_range(vehicle) * self.range).max(CLOSE_RANGE)
    }

}

impl Default for BotRole {
    fn default() -> Self {
        Self::of(VehicleClass::default())
    }
}
