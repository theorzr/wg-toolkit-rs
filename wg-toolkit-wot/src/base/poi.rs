//! The points of interest of a map, written by hand, see [`PoiMap`]: places of a team's
//! side and what usually goes on there, whatever the vehicle, long-range firing,
//! spotting, close combat…, and the way to face there. A file of a map, `<map>.json`,
//! saved as the dev command `/poi` adds them in battle.
//!
//! ```text
//!   {
//!     "pois": [
//!       {
//!         "activities": ["long_range_fire", "support"],   see PoiActivity
//!         "team": 1,                 1 or 2, both if left out
//!         "lane": "left",            if any
//!         "at": [12.5, 8.0, -240.0], where a tank stands, x, y, z
//!         "facing": 30.0,            the way it faces, in degrees: 0 north, 90 east
//!         "note": "behind the bush"  if any
//!       }
//!     ]
//!   }
//! ```

use std::path::Path;
use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};
use glam::{Vec3, Vec4};

use super::debug::{self, DebugChannel};


/// How far round the clients' vehicles points are drawn, in metres on the ground plane,
/// see [`PoiMap::debug_draw`], how long the line of the way each faces is, and their
/// colours: the first team's, the second's, both's.
const DEBUG_RADIUS: f32 = 300.0;
const DEBUG_FACING: f32 = 15.0;
const DEBUG_TEAM_COLORS: [Vec4; 3] = [
    Vec4::new(96.0, 255.0, 96.0, 255.0),
    Vec4::new(255.0, 96.0, 96.0, 255.0),
    Vec4::new(255.0, 255.0, 255.0, 255.0),
];


/// The points of interest of a map, see the module's documentation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PoiMap {
    pub pois: Vec<Poi>,
}

/// A point of interest of a map, see the module's documentation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Poi {
    /// What usually goes on there, see [`PoiActivity`].
    pub activities: Vec<PoiActivity>,
    /// The team it is for, 1 or 2, both if none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<u8>,
    /// The lane it is on, if named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    /// Where a tank stands.
    pub at: [f32; 3],
    /// The way it faces, in degrees: 0 north, along z, 90 east, along x.
    pub facing: f32,
    /// What it is, for who reads the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What usually goes on at a [`Poi`], whatever the vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoiActivity {
    /// Firing from afar, hidden, at enemies others spot.
    LongRangeFire,
    /// Firing from behind the front, at the enemies allies fight there.
    Support,
    /// Spotting from hiding, without firing.
    Spotting,
    /// Spotting up close, driving in sight and back out of it.
    CloseSpotting,
    /// Fighting up close, round corners, buildings and rocks.
    CloseCombat,
    /// Firing over a ridge that hides the hull.
    HullDown,
    /// Firing round a corner, the hull angled at it.
    SideScrape,
    /// Hiding by a way enemies come by, to fire once they are close.
    Ambush,
    /// Going round to the enemies' side or back.
    Flank,
    /// Firing high over cover, as artillery does.
    IndirectFire,
    /// Holding its team's base against a capture.
    DefendBase,
    /// Falling back to, once its lane is lost.
    Fallback,
}

impl PoiActivity {

    /// Every activity, as `/poi` lists them.
    pub const ALL: [Self; 12] = [
        Self::LongRangeFire,
        Self::Support,
        Self::Spotting,
        Self::CloseSpotting,
        Self::CloseCombat,
        Self::HullDown,
        Self::SideScrape,
        Self::Ambush,
        Self::Flank,
        Self::IndirectFire,
        Self::DefendBase,
        Self::Fallback,
    ];

    /// Its name, as saved and as `/poi` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::LongRangeFire => "long_range_fire",
            Self::Support => "support",
            Self::Spotting => "spotting",
            Self::CloseSpotting => "close_spotting",
            Self::CloseCombat => "close_combat",
            Self::HullDown => "hull_down",
            Self::SideScrape => "side_scrape",
            Self::Ambush => "ambush",
            Self::Flank => "flank",
            Self::IndirectFire => "indirect_fire",
            Self::DefendBase => "defend_base",
            Self::Fallback => "fallback",
        }
    }

    /// The activity of this name, see [`Self::name`].
    pub fn of_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|activity| activity.name() == name)
    }

}

impl PoiMap {

    /// The points saved at this path, none if it has no file.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Save the points at this path, as the module's documentation shows them.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(path, text + "\n")
    }

    /// The points for this team, 1 or 2.
    pub fn of_team(&self, team: u8) -> impl Iterator<Item = &Poi> {
        self.pois.iter().filter(move |poi| poi.team.is_none_or(|of| of == team))
    }

    /// The index of the point nearest this one on the ground plane, within `radius`.
    pub fn nearest(&self, at: Vec3, radius: f32) -> Option<usize> {
        self.pois.iter()
            .enumerate()
            .map(|(index, poi)| (index, poi.position().with_y(0.0).distance(at.with_y(0.0))))
            .filter(|&(_, distance)| distance <= radius)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(index, _)| index)
    }

    /// Draw its points within [`DEBUG_RADIUS`] of any of these positions, see
    /// [`DebugChannel::Poi`]: a cross where a tank stands, a line the way it faces, and
    /// what goes on there, in its team's colour.
    pub fn debug_draw(&self, near: &[Vec3]) {
        let shown = self.pois.iter().filter(|poi| {
            near.iter().any(|at| poi.position().with_y(0.0).distance(at.with_y(0.0)) <= DEBUG_RADIUS)
        });
        for poi in shown {
            let color = DEBUG_TEAM_COLORS[poi.team.map_or(2, |team| usize::from(team.clamp(1, 2) - 1))];
            let at = poi.position() + Vec3::Y;
            debug::cross(DebugChannel::Poi, at, color);
            debug::line(DebugChannel::Poi, at, at + poi.heading() * DEBUG_FACING, color);
            debug::text(DebugChannel::Poi, at + Vec3::Y * 2.0, poi.to_string(), color);
        }
    }

}

impl Poi {

    /// Where a tank stands.
    pub fn position(&self) -> Vec3 {
        Vec3::from(self.at)
    }

    /// The way it faces, on the ground plane.
    pub fn heading(&self) -> Vec3 {
        let yaw = self.facing.to_radians();
        Vec3::new(yaw.sin(), 0.0, yaw.cos())
    }

    /// Whether this activity usually goes on there.
    pub fn offers(&self, activity: PoiActivity) -> bool {
        self.activities.contains(&activity)
    }

}

impl fmt::Display for Poi {

    /// Its activities, its lane and its note, as `/poi` takes them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        let activities: Vec<_> = self.activities.iter()
            .map(|activity| activity.name())
            .collect();
        write!(f, "{}", activities.join(","))?;
        if let Some(lane) = &self.lane {
            write!(f, " {lane}")?;
        }
        if let Some(note) = &self.note {
            write!(f, ": {note}")?;
        }
        Ok(())

    }

}
