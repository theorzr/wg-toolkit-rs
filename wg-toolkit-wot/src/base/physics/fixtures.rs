//! What the physics' tests drive on: a Chieftain-like vehicle and simple maps.

use glam::{BVec3, Vec2, Vec3};

use wgtk::space::statics::{BodyKind, Statics, StaticTriangle};
use wgtk::space::terrain::Ground;
use wgtk::space::Space;

use crate::wot::item_defs::{AimingDef, MobilityDef, PitchCurve, WheelDef, Wheels};

use super::contacts::Shape;
use super::{Hull, Rubble};


/// How high the test gun is above the vehicle's origin.
pub const GUN_HEIGHT: f32 = 1.7;

/// About a Chieftain's: 750 hp for 55 t, with its physics section's rolling friction,
/// its terrain resistance 1, 1.3 and 2.5.
pub fn chieftain_mobility() -> MobilityDef {
    MobilityDef {
        speed_forward: 40.0 / 3.6,
        speed_backward: 15.0 / 3.6,
        rotation_speed: [28f32.to_radians(), 28f32.to_radians() / 1.3, 28f32.to_radians() / 2.5],
        engine_power: 750.0 * 735.5,
        weight: 55_000.0,
        rolling_friction: [0.0805, 0.10465, 0.20125],
        carrying_point: Vec2::new(1.6, 2.4),
        bounds: [Vec3::new(-1.8, 0.0, -3.7), Vec3::new(1.8, 2.6, 3.8)],
        turret_bounds: [Vec3::new(-1.3, 2.0, -1.5), Vec3::new(1.3, 2.9, 1.5)],
        wheels: Some(chieftain_wheels()),
    }
}

/// Wheels laid out as the T110E4's model lays them, over the Chieftain's tracks: 6
/// road wheels a side, 0.33 m, on tracks 9 cm thick, an idler in front, 0.92 m up,
/// and a sprocket behind, 1 m up.
fn chieftain_wheels() -> Wheels {
    let mut road = Vec::new();
    for side in [1.6, -1.6] {
        for index in 0..6 {
            road.push(WheelDef {
                center: Vec3::new(side, 0.42, -2.4 + 0.96 * index as f32),
                radius: 0.33,
            });
        }
    }
    let ends = [1.6, -1.6].into_iter()
        .flat_map(|side| [
            WheelDef {
                center: Vec3::new(side, 0.92, 3.2),
                radius: 0.3,
            },
            WheelDef {
                center: Vec3::new(side, 1.0, -3.15),
                radius: 0.34,
            },
        ])
        .collect::<Vec<_>>();
    Wheels::new(&road, &ends, 0.09)
}

/// About a Chieftain's gun, its turret straight above the origin to keep the tests'
/// geometry simple.
pub fn chieftain_aiming() -> AimingDef {
    AimingDef {
        turret_rotation_speed: 30f32.to_radians(),
        gun_rotation_speed: 26f32.to_radians(),
        aiming_time: 2.1,
        dispersion_turret_rotation: 0.11f32.to_degrees(),
        dispersion_after_shot: 4.0,
        dispersion_movement: 0.14 * 3.6,
        dispersion_rotation: 0.14f32.to_degrees(),
        yaw_limits: None,
        min_pitch: pitch_curve(&[(0.0, -20.0), (1.0, -20.0)]),
        // 10° of depression at the front, -1° over the engine deck at the back.
        max_pitch: pitch_curve(&[(0.0, 10.0), (0.4, 10.0), (0.45, -1.0), (0.55, -1.0), (0.6, 10.0), (1.0, 10.0)]),
        dispersion_angle: 0.0035,
        shot_speed: 716.0,
        shot_gravity: 6.28,
        reload_time: 16.0,
        turret_position: Vec3::new(0.0, 1.4, 0.0),
        gun_position: Vec3::new(0.0, GUN_HEIGHT - 1.4, 0.0),
    }
}

/// A pitch limit over the turret's yaw, by shares of a turn and degrees.
fn pitch_curve(points: &[(f32, f32)]) -> PitchCurve {
    PitchCurve(points.iter().map(|&(t, a)| (t * std::f32::consts::TAU, a.to_radians())).collect())
}

/// A Chieftain at the origin, facing +Z.
pub fn chieftain() -> Hull {
    Hull::new(Vec3::ZERO, Vec3::ZERO, chieftain_mobility())
}

/// A tick of `dt` seconds on this ground, in a space without any, see
/// [`Hull::advance_on`].
pub fn tick(hull: &mut Hull, dt: f32, ground: Ground) {
    hull.advance_on(dt, &Space::default(), None, &mut Rubble::default(), Some(ground));
}

/// A wall across the way at z = 10, `height` high, as two triangles, on flat ground from
/// z = −50 to 100.
pub fn wall(height: f32) -> Space {
    let low_left = Vec3::new(-20.0, -1.0, 10.0);
    let low_right = Vec3::new(20.0, -1.0, 10.0);
    let high_left = Vec3::new(-20.0, height, 10.0);
    let high_right = Vec3::new(20.0, height, 10.0);
    let mut triangles = vec![
        StaticTriangle::new([low_left, low_right, high_right], 0, 0, BodyKind::Solid),
        StaticTriangle::new([low_left, high_right, high_left], 0, 0, BodyKind::Solid),
    ];
    quad(&mut triangles, -50.0, 100.0, 0.0, 0.0);
    Space {
        statics: Statics::from_triangles(triangles),
        ..Space::default()
    }
}

/// Flat ground from z = −50 to 100, as two triangles, under `deck`: one from z = 0 to
/// 20, `height` high, with its face down to the ground at its far end, or one that falls
/// `drop` along z from 0 to 40, a slope.
pub fn ground_under(deck: Option<f32>, drop: f32) -> Space {
    let mut triangles = Vec::new();
    match deck {
        Some(height) => {
            quad(&mut triangles, -50.0, 100.0, 0.0, 0.0);
            quad(&mut triangles, 0.0, 20.0, height, height);
            // Its face, down to the ground.
            let top_left = Vec3::new(-5.0, height, 20.0);
            let top_right = Vec3::new(5.0, height, 20.0);
            let low_left = Vec3::new(-5.0, 0.0, 20.0);
            let low_right = Vec3::new(5.0, 0.0, 20.0);
            triangles.push(StaticTriangle::new([low_left, low_right, top_right], 0, 0, BodyKind::Solid));
            triangles.push(StaticTriangle::new([low_left, top_right, top_left], 0, 0, BodyKind::Solid));
        }
        None => {
            quad(&mut triangles, -50.0, 0.0, 0.0, 0.0);
            quad(&mut triangles, 0.0, 40.0, 0.0, -drop);
            quad(&mut triangles, 40.0, 100.0, -drop, -drop);
        }
    }
    Space {
        statics: Statics::from_triangles(triangles),
        ..Space::default()
    }
}

/// Ground 300 m square around the origin, sloping down along z by `degrees`, through
/// the origin: a hull at the origin facing +Z on it is pitched nose down by as much.
pub fn slope(degrees: f32) -> Space {
    let rise = 150.0 * degrees.to_radians().tan();
    let near_left = Vec3::new(-150.0, rise, -150.0);
    let near_right = Vec3::new(150.0, rise, -150.0);
    let far_left = Vec3::new(-150.0, -rise, 150.0);
    let far_right = Vec3::new(150.0, -rise, 150.0);
    let triangles = vec![
        StaticTriangle::new([near_left, far_right, near_right], 0, 0, BodyKind::Solid),
        StaticTriangle::new([near_left, far_left, far_right], 0, 0, BodyKind::Solid),
    ];
    Space {
        statics: Statics::from_triangles(triangles),
        ..Space::default()
    }
}

/// Flat ground 300 m square around the origin, and blocks on it, each from its least
/// corner to its greatest, as triangles: their tops and sides, each a model of its own.
pub fn blocks(blocks: &[(Vec3, Vec3)]) -> Space {
    let mut triangles = vec![
        StaticTriangle::new([Vec3::new(-150.0, 0.0, -150.0), Vec3::new(150.0, 0.0, 150.0), Vec3::new(150.0, 0.0, -150.0)], 0, 0, BodyKind::Solid),
        StaticTriangle::new([Vec3::new(-150.0, 0.0, -150.0), Vec3::new(-150.0, 0.0, 150.0), Vec3::new(150.0, 0.0, 150.0)], 0, 0, BodyKind::Solid),
    ];
    // Each face's corners, by whether they are at the greatest along each axis.
    let faces = [
        [(false, true, false), (false, true, true), (true, true, true), (true, true, false)],
        [(false, false, false), (true, false, false), (true, true, false), (false, true, false)],
        [(false, false, true), (false, true, true), (true, true, true), (true, false, true)],
        [(false, false, false), (false, true, false), (false, true, true), (false, false, true)],
        [(true, false, false), (true, false, true), (true, true, true), (true, true, false)],
    ];
    for (instance, &(min, max)) in blocks.iter().enumerate() {
        for face in faces {
            let [a, b, c, d] = face.map(|(x, y, z)| Vec3::select(BVec3::new(x, y, z), max, min));
            triangles.push(StaticTriangle::new([a, b, c], instance as u32 + 1, 0, BodyKind::Solid));
            triangles.push(StaticTriangle::new([a, c, d], instance as u32 + 1, 0, BodyKind::Solid));
        }
    }
    Space {
        statics: Statics::from_triangles(triangles),
        ..Space::default()
    }
}

/// Two triangles 10 m wide from z = `near` to `far`, at heights `near_y` to `far_y`.
pub fn quad(triangles: &mut Vec<StaticTriangle>, near: f32, far: f32, near_y: f32, far_y: f32) {
    let near_left = Vec3::new(-5.0, near_y, near);
    let near_right = Vec3::new(5.0, near_y, near);
    let far_left = Vec3::new(-5.0, far_y, far);
    let far_right = Vec3::new(5.0, far_y, far);
    triangles.push(StaticTriangle::new([near_left, far_right, near_right], 0, 0, BodyKind::Solid));
    triangles.push(StaticTriangle::new([near_left, far_left, far_right], 0, 0, BodyKind::Solid));
}

/// How far forward along z a hull's boxes reach, as it is turned: its nose, wherever
/// it leans.
pub fn nose(hull: &Hull) -> f32 {
    hull.contacts.iter()
        .filter_map(|contact| match contact.shape {
            Shape::Box { center, half } => Some((center, half)),
            _ => None,
        })
        .take(2)
        .flat_map(|(center, half)| {
            [-1.0, 1.0].into_iter().flat_map(move |x| {
                [-1.0, 1.0].into_iter().flat_map(move |y| [-1.0, 1.0].map(|z| center + half * Vec3::new(x, y, z)))
            })
        })
        .map(|corner| (hull.position + hull.orientation * corner).z)
        .fold(f32::NEG_INFINITY, f32::max)
}
