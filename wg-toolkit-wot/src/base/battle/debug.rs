//! What a battle draws in its clients' world to be debugged, with `--debug-draw`: a
//! [`DebugDrawEntity`], every client seeing it. Under each vehicle alive, how its
//! enemies best look at it, see [`Look`]:
//!
//! ```text
//!   Bot 3                          its name
//!   camo 0.31 -> 0.66, 2 plants    its camouflage, then hidden by the foliage
//!   312 m of 230 m: clear, top     how far, of how far it is spotted, what is in the way,
//!                                  of which of its checkpoints, or of its hull if
//!                                  near, see the `spotting` module
//! ```
//!
//! magenta if spotted, cyan if not, white if no enemy looks at it, none a team's colour.
//! It is where the vehicle stands, our client mod hanging it under it, never over its
//! marker. A vehicle's text is drawn anew each tick it changed, or the vehicle moved
//! [`TEXT_STEP`]: its group is named `entity:<id>:<height>`, which our client mod moves
//! with the vehicle, as the client draws it, while the client has it, see its
//! `debug_draw`.

use std::io;

use glam::{Vec3, Vec4};

use wgtk::app::base::{self, EntityInit, SpaceKey, Visibility};
use wgtk::app::entity::EntityKey;

use crate::game::schema::{DebugDrawEntity, DebugDrawObject, DebugText};
use crate::base::physics::World;

use super::spotting::{Look, Point, Sight, Spotting};
use super::vehicle::BattleVehicle;


/// How high over a vehicle its text is, in metres from where it stands: where it stands,
/// our client mod hanging it under it, see its `debug_draw`.
const TEXT_HEIGHT: f32 = 0.0;
/// How big a vehicle's text is, of the client's default size, 1: twice as big, to be
/// read from behind its own vehicle. Our client mod ignores it, drawing its texts on the
/// screen as big as their font however far, see its `debug_draw`.
const TEXT_SIZE: f32 = 2.0;
/// How far a vehicle moves before its text is moved, in metres: our client mod moves it
/// with the vehicle, as smoothly as it draws the vehicle, while the client has it.
const TEXT_STEP: f32 = 2.0;
/// The colours of a vehicle's text, of 255, pure for contrast, none a team's, neither
/// red, green, gold nor purple: spotted, magenta, not, cyan, and with no enemy looking,
/// white.
const SPOTTED: Vec4 = Vec4::new(255.0, 64.0, 255.0, 255.0);
const HIDDEN: Vec4 = Vec4::new(64.0, 224.0, 255.0, 255.0);
const UNSEEN: Vec4 = Vec4::new(255.0, 255.0, 255.0, 255.0);


/// See the module's documentation.
#[derive(Debug)]
pub struct DebugDraw {
    /// Its entity, at the middle of the map.
    entity: EntityKey<DebugDrawEntity>,
    /// What it drew last, to tell what changes: a group changed gets a version more.
    drawn: Vec<DebugDrawObject>,
}

impl DebugDraw {

    /// Its entity, in a battle's space, at `center`, seen by every client.
    pub fn new(app: &mut base::App, space: SpaceKey, center: Vec3) -> io::Result<Self> {
        
        let entity = app.create_entity::<DebugDrawEntity>(space, EntityInit {
            position: center,
            visibility: Visibility::Always,
            ..EntityInit::default()
        })?;

        Ok(Self {
            entity,
            drawn: Vec::new(),
        })
        
    }

    /// Draw, over each of the battle's vehicles alive, how its enemies look at it this
    /// tick, see the module's documentation, if something of it changed: only the
    /// groups that did are drawn again.
    pub fn show(&mut self, app: &mut base::App, vehicles: &[BattleVehicle], world: &World, spotting: &Spotting) -> io::Result<()> {

        let mut changed = false;
        let mut objects = Vec::new();
        for (index, vehicle) in vehicles.iter().enumerate().filter(|(_, vehicle)| vehicle.alive()) {

            let mut object = DebugDrawObject {
                name: format!("entity:{}:{TEXT_HEIGHT}", vehicle.entity.id().0),
                version: 0,
                destroyTime: -1.0,
                texts: vec![text(vehicle, world, spotting.look(index))],
                ..DebugDrawObject::default()
            };

            match self.drawn.iter().find(|drawn| drawn.name == object.name) {
                Some(drawn) if same_texts(&object, drawn) => object = drawn.clone(),
                Some(drawn) => {
                    object.version = drawn.version.wrapping_add(1);
                    changed = true;
                }
                None => changed = true,
            }
            objects.push(object);

        }

        if !changed && objects.len() == self.drawn.len() {
            return Ok(());
        }
        app.entity(self.entity)?.set_drawObjects(objects.clone())?;
        self.drawn = objects;
        Ok(())

    }

}

/// A vehicle's text, over it, of how its enemies best look at it, if any do.
fn text(vehicle: &BattleVehicle, world: &World, look: Option<&Look>) -> DebugText {

    let position = world.vehicle(vehicle.id).hull.position + Vec3::Y * TEXT_HEIGHT;
    let Some(look) = look else {
        return DebugText {
            position,
            text: vehicle.info.name.clone(),
            color: UNSEEN,
            textSize: TEXT_SIZE,
        };
    };

    let sight = match look.sight {
        Sight::Near => "near",
        Sight::Far => "far",
        Sight::Blocked => "blocked",
        Sight::Clear => "clear",
    };
    let point = match look.point {
        None => "hull",
        Some(Point::Top) => "top",
        Some(Point::Gun) => "gun",
        Some(Point::Front) => "front",
        Some(Point::Rear) => "rear",
        Some(Point::Left) => "left",
        Some(Point::Right) => "right",
    };
    let text = format!(
        "{}\ncamo {:.2} -> {:.2}, {} plants\n{:.0} m of {:.0} m: {sight}, {point}",
        vehicle.info.name,
        look.camouflage,
        look.hidden,
        look.plants,
        look.distance,
        look.range,
    );
    DebugText {
        position,
        text,
        color: if look.spots() { SPOTTED } else { HIDDEN },
        textSize: TEXT_SIZE,
    }

}

/// Whether two groups draw the same texts, about at the same places, see [`TEXT_STEP`].
fn same_texts(a: &DebugDrawObject, b: &DebugDrawObject) -> bool {
    a.texts.len() == b.texts.len()
        && a.texts.iter().zip(&b.texts).all(|(a, b)| {
            a.text == b.text && a.color == b.color && (a.position - b.position).length() < TEXT_STEP
        })
}
