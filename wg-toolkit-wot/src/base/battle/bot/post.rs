//! The point of interest a bot holds, see [`BotPost`]: one of its map's, for its team,
//! where what its role does goes on, chosen once at its battle's start, held until it
//! has long had nothing to do there.

use rand::seq::SliceRandom;
use glam::Vec3;

use crate::base::poi::{Poi, PoiMap};

use super::role::BotRole;


/// How long a bot holds its post with no enemy spotted before it leaves it to search, in
/// seconds. Our choice.
const POST_PATIENCE: f64 = 60.0;
/// How far ahead of its post a bot looks, the way the post faces, in metres.
pub const POST_LOOK: f32 = 100.0;


/// The point of interest a bot holds, see the module's documentation.
#[derive(Debug, Clone, Default)]
pub struct BotPost {
    /// Its point, none once it left it, or without any.
    pub poi: Option<Poi>,
    /// Whether it chose its point yet.
    chosen: bool,
    /// The game time it last had something to do at its post: an enemy, or getting there.
    busy: Option<f64>,
}

impl BotPost {

    /// Choose its point, the first time only, of this `poi_map` for its `team`: one where
    /// an activity its `role` holds a post for goes on, see [`BotRole::posts`], at random.
    pub fn choose(&mut self, role: &BotRole, poi_map: &PoiMap, team: u8) {

        if self.chosen {
            return;
        }
        self.chosen = true;

        let suited: Vec<_> = poi_map.of_team(team)
            .filter(|poi| role.posts.iter().any(|&activity| poi.offers(activity)))
            .collect();
        self.poi = suited.choose(&mut rand::thread_rng()).map(|&poi| poi.clone());

    }

    /// Keep its point at game time `now` while it has something to do there: leave it
    /// once [`POST_PATIENCE`] at it, within `arrive` of it at `position`, with no target.
    pub fn update(&mut self, now: f64, position: Vec3, arrive: f32, target: bool) {
        let Some(poi) = &self.poi else { return };
        let there = (poi.position() - position).with_y(0.0).length() <= arrive;
        if target || !there {
            self.busy = Some(now);
        } else if self.busy.is_some_and(|busy| now - busy > POST_PATIENCE) {
            self.poi = None;
        }
    }

}
