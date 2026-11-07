//! How closely the hull follows the real server: drives captured from it, the controls
//! the client sent and the positions the server gave the vehicle, replayed through
//! [`Hull`] in windows, each starting from the real state. The drives aren't in the
//! repository: run with every `*.csv` drive in `WGT_DRIVES_PATH`, the game's `res`
//! directory in `WGT_RES_PATH`, and `--nocapture` for the errors.
//!
//! A drive is extracted from a capture's trace by `wot replay`, whose trace gives each
//! record's capture time. Its lines are its capture time in seconds, then either
//! `flags,<vehicle_moveWith flags>` the client sent, or `pos,x,y,z,roll,pitch,yaw` the
//! server gave the vehicle. Its header, lines starting with `#`, gives:
//! - `# descr,<bytes>`: the vehicle's compact descriptor;
//! - `# space,<name>`: the map it drove on, whose grounds the hull is given, medium
//!   without;
//! - `# setup,<equipment kg>,<driver>,<medium ground>,<top speed m/s>`: how the vehicle
//!   was set up, see [`CapturedSetup`], stock without;
//! - `# max_median_error,<1 s>,<3 s>`: the median errors over 1 s and 3 s windows the
//!   hull must stay under, in metres, a guard against regressions, none without.

use std::f32::consts::PI;
use std::fs;

use glam::Vec3;

use wgtk::space::terrain::Terrain;
use wgtk::res::fs::ResFilesystem;

use crate::wot::item_defs::{MobilityDef, VehicleDef};
use crate::wot::items::VehicleDescr;

use super::physics::{ground_at, wrap_angle, Hull};
use super::TICK;


/// How long after the client sends its controls the server's positions follow them, in
/// seconds. Fitted: the delay the sweep in the test's report finds best, 0 to 0.05 s
/// within a few centimetres, below the capture's tick.
const CONTROL_DELAY: f64 = 0.05;

/// The seconds between two positions: the server sends one per tick, which reach the
/// capture irregularly, so as many whole ticks as is closest.
fn ticks_between(from: f64, to: f64) -> f32 {
    let tick = TICK.as_secs_f64();
    (((to - from) / tick).round().max(1.0) * tick) as f32
}

/// How a drive's vehicle was set up, apart from a stock one, from what the capture's
/// `setups` and `crewCompactDescrs` hold: the hull is given the same, to compare alike.
#[derive(Debug, Clone, Copy)]
struct CapturedSetup {
    /// The weight of its equipment, in kg, from `optional_devices.xml`.
    equipment_weight: f32,
    /// The factor its driver's skill divides every terrain resistance by, so multiplies
    /// its turning speed by, `0.57 + 0.43 · efficiency` in `VehicleDescrCrew.py`.
    driver: f32,
    /// What off-road driving divides medium ground's friction by, its
    /// `mediumGroundFactor` in `tankmen.xml`.
    medium_ground: f32,
    /// The top speed its driver's `driver_motorExpert` adds, in m/s.
    top_speed: f32,
}

impl CapturedSetup {

    /// No equipment, field modifications or perks, and a crew whose skills don't change
    /// its mobility.
    const STOCK: Self = Self { equipment_weight: 0.0, driver: 1.0, medium_ground: 1.0, top_speed: 0.0 };

    /// A stock vehicle's mobility, set up as the drive's was.
    fn apply(&self, mobility: MobilityDef) -> MobilityDef {
        let [firm, medium, soft] = mobility.rolling_friction;
        MobilityDef {
            speed_forward: mobility.speed_forward + self.top_speed,
            speed_backward: mobility.speed_backward + self.top_speed,
            // The driver divides every resistance, and the weight slows the turn.
            rotation_speed: {
                let factor = self.driver * mobility.weight / (mobility.weight + self.equipment_weight);
                let [firm, medium, soft] = mobility.rotation_speed.map(|speed| speed * factor);
                [firm, medium * self.medium_ground, soft]
            },
            weight: mobility.weight + self.equipment_weight,
            rolling_friction: [firm / self.driver, medium / (self.driver * self.medium_ground), soft / self.driver],
            ..mobility
        }
    }

}

/// How far the real vehicle must move over a window for it to count, in metres. Our
/// choice: standing windows would count as perfect and hide the errors.
const MOVED: f32 = 0.5;


/// A server position of the vehicle.
#[derive(Debug, Clone, Copy)]
struct Update {
    t: f64,
    position: Vec3,
    /// Roll, pitch and yaw.
    direction: Vec3,
}

/// A drive, see the module's documentation.
struct Drive {
    descr: Vec<u8>,
    space: Option<String>,
    setup: CapturedSetup,
    max_median_error: Vec<(f64, f32)>,
    controls: Vec<(f64, u8)>,
    updates: Vec<Update>,
}

fn parse(csv: &str) -> Drive {
    let mut drive = Drive {
        descr: Vec::new(),
        space: None,
        setup: CapturedSetup::STOCK,
        max_median_error: Vec::new(),
        controls: Vec::new(),
        updates: Vec::new(),
    };
    let floats = |values: &str| values.split(',').map(|value| value.parse::<f32>().unwrap()).collect::<Vec<_>>();
    for line in csv.lines() {
        if let Some(descr) = line.strip_prefix("# descr,") {
            drive.descr = descr.split(',').map(|byte| byte.parse().unwrap()).collect();
            continue;
        }
        if let Some(space) = line.strip_prefix("# space,") {
            drive.space = Some(space.to_string());
            continue;
        }
        if let Some(setup) = line.strip_prefix("# setup,") {
            let [equipment_weight, driver, medium_ground, top_speed] = floats(setup)[..] else { panic!("bad setup: {line}") };
            drive.setup = CapturedSetup { equipment_weight, driver, medium_ground, top_speed };
            continue;
        }
        if let Some(max) = line.strip_prefix("# max_median_error,") {
            drive.max_median_error = [1.0, 3.0].into_iter().zip(floats(max)).collect();
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let t = fields[0].parse().unwrap();
        let float = |index: usize| fields[index].parse::<f32>().unwrap();
        match fields[1] {
            "flags" => drive.controls.push((t, fields[2].parse().unwrap())),
            "pos" => drive.updates.push(Update {
                t,
                position: Vec3::new(float(2), float(3), float(4)),
                direction: Vec3::new(float(5), float(6), float(7)),
            }),
            kind => panic!("unknown line kind {kind}"),
        }
    }
    drive
}

/// The errors at the end of windows, in metres on the ground plane and radians of yaw.
#[derive(Debug, Default)]
struct Errors {
    distance: Vec<f32>,
    yaw: Vec<f32>,
}

impl Errors {

    /// The median, the 90th percentile and the maximum.
    fn stats(values: &[f32]) -> (f32, f32, f32) {
        let mut sorted = values.to_vec();
        sorted.sort_by(f32::total_cmp);
        let at = |share: f32| sorted[((sorted.len() - 1) as f32 * share) as usize];
        (at(0.5), at(0.9), at(1.0))
    }

    fn median_distance(&self) -> f32 {
        Self::stats(&self.distance).0
    }

    fn report(&self, label: &str) {
        let (d50, d90, dmax) = Self::stats(&self.distance);
        let (y50, y90, ymax) = Self::stats(&self.yaw);
        println!("{label}: {} windows, distance {d50:.2} / {d90:.2} / {dmax:.2} m, yaw {:.1} / {:.1} / {:.1}° (median / 90% / max)",
            self.distance.len(), y50 * 180.0 / PI, y90 * 180.0 / PI, ymax * 180.0 / PI);
    }

}

/// Replay the drive in windows of `window` seconds, each from the real state, through
/// the hull of this mobility, on its terrain's grounds if any.
fn replay(drive: &Drive, mobility: MobilityDef, terrain: Option<&Terrain>, window: f64, delay: f64) -> Errors {

    let updates = &drive.updates;
    let mut errors = Errors::default();
    let mut start = 1;
    while start < updates.len() {

        let (before, from) = (updates[start - 1], updates[start]);
        let end = updates[start..].iter().position(|update| update.t >= from.t + window).map(|offset| start + offset);
        let Some(end) = end else { break };
        let next = end;

        // The real state: where it is, and how fast it moves and turns.
        let dt = ticks_between(before.t, from.t);
        let mut hull = Hull::new(from.position, from.direction, mobility);
        let heading = Vec3::new(from.direction.z.sin(), 0.0, from.direction.z.cos());
        hull.speed = (from.position - before.position).dot(heading) / dt;
        hull.yaw_rate = wrap_angle(from.direction.z - before.direction.z) / dt;
        let controls_at = |t: f64| drive.controls.iter().rev()
            .find(|&&(at, _)| at + delay <= t)
            .map_or(0, |&(_, flags)| flags);
        hull.flags = controls_at(from.t);

        for pair in updates[start..=end].windows(2) {
            hull.flags = controls_at(pair[0].t);
            hull.step(ticks_between(pair[0].t, pair[1].t), ground_at(terrain, hull.position));
        }

        let real = updates[end];
        let moved = (real.position - from.position).with_y(0.0).length();
        if moved >= MOVED {
            errors.distance.push((hull.position - real.position).with_y(0.0).length());
            errors.yaw.push(wrap_angle(hull.direction.z - real.direction.z).abs());
        }
        start = next;

    }
    errors

}

#[test]
fn hull_follows_the_real_server() {

    let (Ok(res), Ok(drives)) = (std::env::var("WGT_RES_PATH"), std::env::var("WGT_DRIVES_PATH")) else { return };
    let res = ResFilesystem::new(res).unwrap();
    let mut paths = fs::read_dir(&drives).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "csv"))
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {

        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let drive = parse(&fs::read_to_string(&path).unwrap());
        let decoded = VehicleDescr::decode(&drive.descr).unwrap();
        let def = VehicleDef::load_by_id(&res, decoded.nation, decoded.type_id).unwrap();
        println!("{name} ({}): {} controls, {} positions over {:.0} s, {:?}", def.name, drive.controls.len(),
            drive.updates.len(), drive.updates.last().unwrap().t - drive.updates[0].t, def.mobility);

        let mobility = drive.setup.apply(def.mobility);
        let terrain = drive.space.as_ref().map(|space| Terrain::load(&res, &format!("spaces/{space}")).unwrap());
        for delay in [0.0, 0.05, 0.1, 0.15, 0.2, 0.3] {
            replay(&drive, mobility, terrain.as_ref(), 1.0, delay).report(&format!("  delay {delay}"));
        }
        for &(window, max) in &drive.max_median_error {
            let errors = replay(&drive, mobility, terrain.as_ref(), window, CONTROL_DELAY);
            errors.report(&format!("  {window} s windows"));
            assert!(errors.median_distance() <= max, "{name}: median error over {window} s windows: {} m", errors.median_distance());
        }

    }

}
