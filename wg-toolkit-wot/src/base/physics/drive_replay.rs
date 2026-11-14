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
//! - `# space,<name>`: the map it drove on, whose grounds the hull is given, firm
//!   without;
//! - `# setup,<equipment kg>,<driver>,<medium ground>,<top speed m/s>`: how the vehicle
//!   was set up, see [`CapturedSetup`], stock without;
//! - `# max_median_error,<1 s>,<3 s>`: the median errors over 1 s and 3 s windows the
//!   hull must stay under, in metres, a guard against regressions, none without;
//! - `# other,<vehicle>,<bytes>`: another vehicle near it, by its entity id, and its
//!   compact descriptor, whose positions are `other,<vehicle>,x,y,z,yaw` lines: the hull
//!   collides with it where the real server had it, see [`Other`].

use std::f32::consts::PI;
use std::sync::Arc;
use std::fs;

use glam::{EulerRot, Quat, Vec2, Vec3};

use wgtk::res::fs::ResFilesystem;
use wgtk::space::Space;

use crate::game::item_defs::{self, AimingDef, MobilityDef, VehicleDef};
use crate::game::arena_defs::{self, STANDARD_GAMEPLAY};
use crate::game::items::VehicleDescr;

use crate::base::TICK;

use super::{wrap_angle, Hull, Rubble, Vehicle, World};


/// How long after the client sends its controls the server's positions follow them, in
/// seconds, from the position a tick starts from: negative, a tick already moves as the
/// controls sent up to this long after its first position say. The capture has both as
/// the client got and sent them, so a tick's position reaches it a round trip after
/// controls the server moved it by. Fitted: the delay the sweep in the test's report
/// finds best on every drive, −0.06 s; at +0.05 s, the turns started and stopped a tick
/// late, and the median errors over 1 s were half as large again.
const CONTROL_DELAY: f64 = -0.06;

/// How far the real vehicle must move over a window for it to count, in metres. Our
/// choice: standing windows would count as perfect and hide the errors.
const MOVED: f32 = 0.5;

/// How long [`tilt_errors`] lets a hull rest, in seconds.
const REST_TIME: f32 = 3.0;


/// The seconds between two positions: the server sends one each tick, which reach the
/// capture irregularly, bunched up after a stall, so always one tick. Measured: counted
/// on the drives' 100 ms clock, their gaps are one tick but for as many of 0 as of 2.
fn ticks_between(_from: f64, _to: f64) -> f32 {
    TICK.as_secs_f32()
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

/// A server position of the vehicle.
#[derive(Debug, Clone, Copy)]
struct Update {
    t: f64,
    position: Vec3,
    /// Roll, pitch and yaw.
    direction: Vec3,
}

/// Another vehicle of a drive: its descriptor and its positions.
struct OtherTrack {
    id: u32,
    descr: Vec<u8>,
    updates: Vec<Update>,
}

/// A drive, see the module's documentation.
struct CapturedDrive {
    descr: Vec<u8>,
    space: Option<String>,
    setup: CapturedSetup,
    max_median_error: Vec<(f64, f32)>,
    controls: Vec<(f64, u8)>,
    updates: Vec<Update>,
    others: Vec<OtherTrack>,
}

/// Another vehicle the hull collides with where the real server had it, as heavy as
/// [`OTHER_WEIGHT`]: its track already holds how the real hull pushed it, so the hull
/// gets all of the push, and its speed into it.
struct Other<'a> {
    mobility: MobilityDef,
    aiming: AimingDef,
    updates: &'a [Update],
}

/// The weight of another vehicle of a drive, far over any vehicle's, see [`Other`].
const OTHER_WEIGHT: f32 = 1e9;

/// How long after its last position another vehicle of a drive is still there, in
/// seconds: past it, it left the area the server tells the client about.
const OTHER_TIMEOUT: f64 = 0.5;

impl Other<'_> {

    /// The vehicle as it was at `t`, moving as it went then, if it was known.
    fn at(&self, t: f64) -> Option<Vehicle> {
        let index = self.updates.partition_point(|update| update.t <= t);
        let (before, at) = index.checked_sub(2).map(|before| (self.updates[before], self.updates[index - 1]))?;
        if t - at.t > OTHER_TIMEOUT {
            return None;
        }
        let mut vehicle = Vehicle::new(at.position, at.direction, self.mobility, self.aiming.clone());
        let velocity = (at.position - before.position).with_y(0.0) / (at.t - before.t).max(1e-3) as f32;
        // Faster than it drives, it jumped where the capture lost it: still.
        let velocity = if velocity.length() > 2.0 * self.mobility.speed_forward { Vec3::ZERO } else { velocity };
        let heading = Vec3::new(at.direction.z.sin(), 0.0, at.direction.z.cos());
        let right = Vec3::new(at.direction.z.cos(), 0.0, -at.direction.z.sin());
        vehicle.hull.speed = velocity.dot(heading);
        vehicle.hull.slide = velocity.dot(right);
        Some(vehicle)
    }

}

fn parse(csv: &str) -> CapturedDrive {
    let mut drive = CapturedDrive {
        descr: Vec::new(),
        space: None,
        setup: CapturedSetup::STOCK,
        max_median_error: Vec::new(),
        controls: Vec::new(),
        updates: Vec::new(),
        others: Vec::new(),
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
        if let Some(other) = line.strip_prefix("# other,") {
            let (id, descr) = other.split_once(',').unwrap();
            let descr = descr.split(',').map(|byte| byte.parse().unwrap()).collect();
            drive.others.push(OtherTrack {
                id: id.parse().unwrap(),
                descr,
                updates: Vec::new(),
            });
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
            "other" => {
                let id = fields[2].parse::<u32>().unwrap();
                let other = drive.others.iter_mut().find(|other| other.id == id).unwrap();
                other.updates.push(Update {
                    t,
                    position: Vec3::new(float(3), float(4), float(5)),
                    direction: Vec3::new(0.0, 0.0, float(6)),
                });
            }
            kind => panic!("unknown line kind {kind}"),
        }
    }
    drive
}

/// The errors at the end of windows, in metres on the ground plane and radians of yaw,
/// and when each window starts.
#[derive(Debug, Default)]
struct Errors {
    distance: Vec<f32>,
    /// How far above or below the real vehicle it ends, in metres.
    height: Vec<f32>,
    yaw: Vec<f32>,
    starts: Vec<f64>,
}

impl Errors {

    fn median_distance(&self) -> f32 {
        Self::stats(&self.distance).0
    }

    /// The median, the 90th percentile and the maximum.
    fn stats(values: &[f32]) -> (f32, f32, f32) {
        let mut sorted = values.to_vec();
        sorted.sort_by(f32::total_cmp);
        let at = |share: f32| sorted[((sorted.len() - 1) as f32 * share) as usize];
        (at(0.5), at(0.9), at(1.0))
    }

    fn report(&self, label: &str) {
        let (d50, d90, dmax) = Self::stats(&self.distance);
        let (h50, h90, hmax) = Self::stats(&self.height);
        let (y50, y90, ymax) = Self::stats(&self.yaw);
        println!(
            "{label}: {} windows, distance {d50:.2} / {d90:.2} / {dmax:.2} m, height {h50:.2} / {h90:.2} / {hmax:.2} m, yaw {:.1} / {:.1} / {:.1}° (median / 90% / max)",
            self.distance.len(),
            y50 * 180.0 / PI,
            y90 * 180.0 / PI,
            ymax * 180.0 / PI,
        );
    }

    /// The median distance error in metres and yaw error in degrees of the windows
    /// starting between two times, and how many.
    fn median_between(&self, from: f64, to: f64) -> (f32, f32, usize) {
        let within = (0..self.starts.len()).filter(|&index| (from..to).contains(&self.starts[index])).collect::<Vec<_>>();
        if within.is_empty() {
            return (0.0, 0.0, 0);
        }
        let distances = within.iter().map(|&index| self.distance[index]).collect::<Vec<_>>();
        let yaws = within.iter().map(|&index| self.yaw[index]).collect::<Vec<_>>();
        (Self::stats(&distances).0, Self::stats(&yaws).0 * 180.0 / PI, within.len())
    }

    /// The windows ending furthest from the real server, to look into.
    fn report_worst(&self, count: usize) {
        let mut worst = (0..self.distance.len()).collect::<Vec<_>>();
        worst.sort_by(|&a, &b| self.distance[b].total_cmp(&self.distance[a]));
        let worst = worst.iter().take(count)
            .map(|&i| format!("{:.1} s ({:.1} m, {:.0}°)", self.starts[i], self.distance[i], self.yaw[i] * 180.0 / PI))
            .collect::<Vec<_>>();
        println!("    worst from: {}", worst.join(", "));
        let mut highest = (0..self.height.len()).collect::<Vec<_>>();
        highest.sort_by(|&a, &b| self.height[b].total_cmp(&self.height[a]));
        let highest = highest.iter().take(count)
            .map(|&i| format!("{:.1} s ({:.2} m)", self.starts[i], self.height[i]))
            .collect::<Vec<_>>();
        println!("    worst in height from: {}", highest.join(", "));
    }

}

/// Replay the drive in windows of `window` seconds, each from the real state, through
/// the hull of this mobility, in its space and within its map's bounds, colliding with
/// the other vehicles.
fn replay(drive: &CapturedDrive, mobility: MobilityDef, aiming: &AimingDef, others: &[Other], space: &Arc<Space>, bounds: Option<(Vec2, Vec2)>, window: f64, delay: f64) -> Errors {

    let updates = &drive.updates;
    let mut errors = Errors::default();
    // What it broke, kept from a window to the next, in order: a window that starts as
    // the real vehicle drives through a house finds the walls it already broke down.
    let mut rubble = Rubble::default();
    let mut start = 1;
    while start < updates.len() {

        let before = updates[start - 1];
        let from = updates[start];
        let end = updates[start..].iter().position(|update| update.t >= from.t + window).map(|offset| start + offset);
        let Some(end) = end else { break };
        let next = end;

        // The real state: where it is, and how fast it moves and turns.
        let dt = ticks_between(before.t, from.t);
        let mut world = World::new(space.clone(), bounds);
        let me = world.add(Vehicle::new(from.position, from.direction, mobility, aiming.clone()));
        let hull = &mut world.vehicle_mut(me).hull;
        let heading = Vec3::new(from.direction.z.sin(), 0.0, from.direction.z.cos());
        let right = Vec3::new(from.direction.z.cos(), 0.0, -from.direction.z.sin());
        // Its velocity is its last tick's chord's, turned as it turned over half of it:
        // the chord lags its heading so, which would make it slide.
        hull.yaw_rate = wrap_angle(from.direction.z - before.direction.z) / dt;
        let chord = (from.position - before.position) / dt;
        let velocity = Quat::from_rotation_y(hull.yaw_rate * dt / 2.0) * chord;
        hull.speed = velocity.dot(heading);
        hull.slide = velocity.dot(right);
        hull.linear_velocity = velocity;
        hull.match_tracks();
        // It rolls and pitches as it did over its last tick: without, a hull starting as
        // it rears up onto a rise comes up late, and its belly meets it.
        hull.angular_velocity = (rotation_of(from.direction) * rotation_of(before.direction).inverse()).to_scaled_axis() / dt;
        let controls_at = |t: f64| drive.controls.iter().rev()
            .find(|&&(at, _)| at + delay <= t)
            .map_or(0, |&(_, flags)| flags);
        hull.flags = controls_at(from.t);

        // The other vehicles that are there, by their place in `others`.
        let mut present = vec![None; others.len()];
        // What it broke, the world's for the window.
        std::mem::swap(world.rubble_mut(), &mut rubble);
        for pair in updates[start..=end].windows(2) {
            let dt = ticks_between(pair[0].t, pair[1].t);
            world.vehicle_mut(me).hull.flags = controls_at(pair[0].t);
            for (other, id) in others.iter().zip(&mut present) {
                match (other.at(pair[0].t), *id) {
                    (Some(vehicle), Some(known)) => *world.vehicle_mut(known) = vehicle,
                    (Some(vehicle), None) => *id = Some(world.add(vehicle)),
                    (None, Some(known)) => {
                        world.remove(known);
                        *id = None;
                    }
                    (None, None) => {}
                }
            }
            world.step(dt);
        }
        std::mem::swap(world.rubble_mut(), &mut rubble);
        let hull = &world.vehicle(me).hull;

        let real = updates[end];
        let moved = (real.position - from.position).with_y(0.0).length();
        if moved >= MOVED {
            errors.distance.push((hull.position - real.position).with_y(0.0).length());
            errors.height.push((hull.position.y - real.position.y).abs());
            errors.yaw.push(wrap_angle(hull.yaw() - real.direction.z).abs());
            errors.starts.push(from.t);
        }
        start = next;

    }
    errors

}

/// The orientation of a roll, pitch and yaw, as the server gives them: yaw about Y, then
/// pitch about X, then roll about Z.
fn rotation_of(direction: Vec3) -> Quat {
    Quat::from_euler(EulerRot::YXZ, direction.z, direction.y, direction.x)
}

/// How far the hull, left to rest on its suspension for [`REST_TIME`] where the real
/// vehicle was, tilts from it: the median and 90th percentile pitch and roll errors, in
/// degrees, over every tenth position, those on the ground.
fn tilt_errors(drive: &CapturedDrive, mobility: MobilityDef, space: &Space) -> [(f32, f32); 2] {
    let mut pitch = Vec::new();
    let mut roll = Vec::new();
    for update in drive.updates.iter().step_by(10) {
        let mut hull = Hull::new(update.position, update.direction, mobility);
        let mut rubble = Rubble::default();
        for _ in 0..(REST_TIME / 0.1) as usize {
            hull.advance(0.1, space, None, &mut rubble);
        }
        if hull.airborne {
            continue;
        }
        let direction = hull.direction();
        pitch.push(wrap_angle(direction.y - update.direction.y).abs());
        roll.push(wrap_angle(direction.x - update.direction.x).abs());
    }
    [pitch, roll].map(|errors| {
        let (median, p90, _) = Errors::stats(&errors);
        (median * 180.0 / PI, p90 * 180.0 / PI)
    })
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
        let space = drive.space.as_ref()
            .map(|space| Space::load(&res, &format!("spaces/{space}"), STANDARD_GAMEPLAY).unwrap())
            .unwrap_or_default();
        let space = Arc::new(space);
        let bounds = drive.space.as_ref()
            .map(|space| arena_defs::read_bounds(&*item_defs::read_xml(&res, &format!("scripts/arena_defs/{space}.xml")).unwrap(), space).unwrap());
        if drive.space.is_some() {
            let [(pitch, pitch_90), (roll, roll_90)] = tilt_errors(&drive, mobility, &space);
            println!("  tilt settled: pitch {pitch:.2} / {pitch_90:.2}°, roll {roll:.2} / {roll_90:.2}° (median / 90%)");
        }
        let others = drive.others.iter()
            .map(|other| {
                let decoded = VehicleDescr::decode(&other.descr).unwrap();
                let def = VehicleDef::load_by_id(&res, decoded.nation, decoded.type_id).unwrap();
                Other {
                    mobility: MobilityDef {
                        weight: OTHER_WEIGHT,
                        ..def.mobility
                    },
                    aiming: def.aiming,
                    updates: &other.updates,
                }
            })
            .collect::<Vec<_>>();
        for delay in [-0.1, -0.08, -0.06, -0.04, -0.02, 0.0] {
            replay(&drive, mobility, &def.aiming, &others, &space, bounds, 1.0, delay).report(&format!("  delay {delay}"));
        }
        if !others.is_empty() {
            for window in [1.0, 3.0] {
                let (alone, with) = (replay(&drive, mobility, &def.aiming, &[], &space, bounds, window, CONTROL_DELAY),
                    replay(&drive, mobility, &def.aiming, &others, &space, bounds, window, CONTROL_DELAY));
                for (from, to) in [(254.0, 258.0), (267.0, 271.0), (272.0, 282.0)] {
                    let ((alone, alone_yaw, count), (with, with_yaw, _)) = (alone.median_between(from, to), with.median_between(from, to));
                    println!("  {window} s windows from {from} to {to} s: {alone:.2} m {alone_yaw:.1}° alone, {with:.2} m {with_yaw:.1}° with the others ({count})");
                }
            }
        }
        for &(window, max) in &drive.max_median_error {
            let errors = replay(&drive, mobility, &def.aiming, &others, &space, bounds, window, CONTROL_DELAY);
            errors.report(&format!("  {window} s windows"));
            errors.report_worst(6);
            assert!(errors.median_distance() <= max, "{name}: median error over {window} s windows: {} m", errors.median_distance());
        }

    }

}

