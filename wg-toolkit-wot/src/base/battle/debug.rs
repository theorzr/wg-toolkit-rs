//! What a battle draws in its clients' world to be debugged, by channel, those chosen
//! with `--debug-draw`: a [`DebugDrawEntity`], every client seeing it.
//!
//! Each tick, what has something to debug draws it in a [`DebugFrame`], channel by
//! channel, see [`DebugChannel`], in groups: spheres and texts, either in the world,
//! or following a vehicle, see [`DebugFrame::on`]. A group following a
//! vehicle is drawn in its frame, x right, y up, z ahead, from where it stands: our
//! client mod moves it with the vehicle, turning with it, as the client draws it, while
//! the client has it. What is drawn is given in the world as the server has it this
//! tick, and put in the vehicle's frame here.
//!
//! ```text
//!   group name                 drawn
//!   <channel>                  in the world
//!   entity:<id>:<channel>      in the frame of the vehicle of that entity id
//! ```
//!
//! A line has a colour, red, green and blue, packed in its width as `0xRRGGBB`, see
//! [`DebugGroup::line`]: our client mod draws it that colour, seen through the map,
//! where the client draws its lines as white boxes that wide. A line has two points, as
//! the client joins more with spheres as big.
//!
//! The groups that changed since the last tick are drawn anew, their version one more,
//! see [`DebugDraw::show`], told by a hash of each: what is drawn is rounded to
//! [`SNAP`], so that a vehicle's own shapes, still in its frame, don't change. The
//! client draws spheres as models, see our client mod's `debug_draw`, white, hidden by
//! the map; lines and texts of a colour, seen through it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;

use clap::ValueEnum;
use glam::{Affine3A, Quat, Vec3, Vec4};

use wgtk::app::base::{self, EntityInit, SpaceKey, Visibility};
use wgtk::app::entity::EntityKey;

use crate::game::schema::{DebugDrawEntity, DebugDrawObject, DebugLine, DebugSphere, DebugText};


/// How finely what is drawn is placed, in metres, see the module's documentation.
const SNAP: f32 = 0.01;
/// How big a text is, of the client's default size, 1: twice as big, to be read from
/// behind its own vehicle. Our client mod ignores it, drawing its texts on the screen as
/// big as their font however far, see its `debug_draw`.
const TEXT_SIZE: f32 = 2.0;


/// What a battle may draw to be debugged, each chosen with `--debug-draw`, see the
/// module's documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DebugChannel {
    /// Under each vehicle, how its enemies best look at it, see `Spotting::debug_draw`.
    Spotting,
    /// Each vehicle's checkpoints, and its enemies' best line of sight to one of them.
    SpotPoints,
    /// Where each vehicle touches the space and the others, see `debug_contacts`.
    Contacts,
}

/// A set of [`DebugChannel`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugChannels(u32);

/// See the module's documentation.
#[derive(Debug)]
pub struct DebugDraw {
    /// Its entity, at the middle of the map.
    entity: EntityKey<DebugDrawEntity>,
    /// The channels it draws.
    channels: DebugChannels,
    /// What it drew last, to tell what changes: a group changed gets a version more.
    drawn: Vec<Drawn>,
}

/// A group a [`DebugDraw`] drew: hashes of its name and of what it draws, and its
/// version.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    name: u64,
    shapes: u64,
    version: u32,
}

/// What a battle draws to be debugged this tick, in groups, see the module's
/// documentation: nothing of a channel it doesn't draw.
#[derive(Debug)]
pub struct DebugFrame {
    channels: DebugChannels,
    groups: Vec<DebugDrawObject>,
}

/// A group of a [`DebugFrame`], drawing what it is given in the world in its own frame.
#[derive(Debug)]
pub struct DebugGroup<'a> {
    object: &'a mut DebugDrawObject,
    /// From the world to its frame.
    local: Affine3A,
}

impl DebugChannel {

    /// Its name, as `--debug-draw` takes it.
    fn name(self) -> &'static str {
        match self {
            DebugChannel::Spotting => "spotting",
            DebugChannel::SpotPoints => "spot-points",
            DebugChannel::Contacts => "contacts",
        }
    }

}

impl DebugChannels {

    /// The set of these channels.
    pub fn of(channels: &[DebugChannel]) -> Self {
        Self(channels.iter().fold(0, |set, &channel| set | 1 << channel as u32))
    }

    /// Whether it has no channel.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether it has this channel.
    fn contains(self, channel: DebugChannel) -> bool {
        self.0 & 1 << channel as u32 != 0
    }

}

impl DebugDraw {

    /// Its entity, in a battle's space, at `center`, seen by every client, drawing these
    /// channels.
    pub fn new(app: &mut base::App, space: SpaceKey, center: Vec3, channels: DebugChannels) -> io::Result<Self> {

        let entity = app.create_entity::<DebugDrawEntity>(space, EntityInit {
            position: center,
            visibility: Visibility::Always,
            ..EntityInit::default()
        })?;

        Ok(Self {
            entity,
            channels,
            drawn: Vec::new(),
        })

    }

    /// An empty frame to draw this tick in, see [`Self::show`].
    pub fn frame(&self) -> DebugFrame {
        DebugFrame {
            channels: self.channels,
            groups: Vec::new(),
        }
    }

    /// Draw a frame in the clients' world: the groups that changed since the last, or
    /// came, drawn anew, a version more, those gone taken away. Nothing is sent if none
    /// did.
    pub fn show(&mut self, app: &mut base::App, frame: DebugFrame) -> io::Result<()> {

        let mut changed = frame.groups.len() != self.drawn.len();
        let mut objects = frame.groups;
        let mut drawn = Vec::with_capacity(objects.len());
        for object in &mut objects {

            let name = hash_of(&object.name);
            let shapes = shapes_hash(object);
            match self.drawn.iter().find(|before| before.name == name) {
                Some(before) if before.shapes == shapes => object.version = before.version,
                Some(before) => {
                    object.version = before.version.wrapping_add(1);
                    changed = true;
                }
                None => changed = true,
            }

            drawn.push(Drawn {
                name,
                shapes,
                version: object.version,
            });

        }

        if !changed {
            return Ok(());
        }
        app.entity(self.entity)?.set_drawObjects(objects)?;
        self.drawn = drawn;
        Ok(())

    }

}

impl DebugFrame {

    /// The group of a channel in the world, if it draws it.
    pub fn world(&mut self, channel: DebugChannel) -> Option<DebugGroup<'_>> {
        self.group(channel, channel.name().to_string(), Affine3A::IDENTITY)
    }

    /// The group of a channel following a vehicle, if it draws it: its entity's id, and
    /// where the vehicle stands and how it is turned on the server this tick, as its
    /// entity is told, see the module's documentation.
    pub fn on(&mut self, channel: DebugChannel, entity_id: u32, position: Vec3, orientation: Quat) -> Option<DebugGroup<'_>> {
        let local = Affine3A::from_rotation_translation(orientation, position).inverse();
        self.group(channel, format!("entity:{entity_id}:{}", channel.name()), local)
    }

    /// Whether it draws a channel.
    pub fn draws(&self, channel: DebugChannel) -> bool {
        self.channels.contains(channel)
    }

    /// The group of this name, of a channel it draws, made if new.
    fn group(&mut self, channel: DebugChannel, name: String, local: Affine3A) -> Option<DebugGroup<'_>> {

        if !self.draws(channel) {
            return None;
        }

        let index = match self.groups.iter()
            .position(|group| group.name == name) {
                Some(index) => index,
                None => {
                    self.groups.push(DebugDrawObject {
                        name,
                        version: 0,
                        destroyTime: -1.0,
                        ..DebugDrawObject::default()
                    });
                    self.groups.len() - 1
                }
            };

        Some(DebugGroup {
            object: &mut self.groups[index],
            local,
        })

    }

}

impl DebugGroup<'_> {

    /// A sphere at `center`, in the world, of this radius, in metres.
    pub fn sphere(&mut self, center: Vec3, radius: f32) {
        self.object.spheres.push(DebugSphere {
            position: snap(self.local.transform_point3(center)),
            radius: Vec3::splat(radius),
        });
    }

    /// A line from `from` to `to`, in the world, in this colour, red, green and blue, of
    /// 255, see the module's documentation.
    pub fn line(&mut self, from: Vec3, to: Vec3, color: Vec4) {
        let [red, green, blue, _] = color.to_array()
            .map(|channel| channel.clamp(0.0, 255.0) as u32);
        self.object.lines.push(DebugLine {
            points: vec![
                snap(self.local.transform_point3(from)),
                snap(self.local.transform_point3(to)),
            ],
            width: (red << 16 | green << 8 | blue) as f32,
        });
    }

    /// A text at `at`, in the world, of several lines, in this colour, red, green, blue
    /// and alpha, of 255.
    pub fn text(&mut self, at: Vec3, text: String, color: Vec4) {
        self.object.texts.push(DebugText {
            position: snap(self.local.transform_point3(at)),
            text,
            color,
            textSize: TEXT_SIZE,
        });
    }

}

/// A point rounded to [`SNAP`].
fn snap(point: Vec3) -> Vec3 {
    (point / SNAP).round() * SNAP
}

/// A hash of a value.
fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A hash of what a group draws, whatever its version: the same shapes, the same hash.
fn shapes_hash(object: &DebugDrawObject) -> u64 {

    let mut hasher = DefaultHasher::new();
    for sphere in &object.spheres {
        hash_floats(&mut hasher, &sphere.position.to_array());
        hash_floats(&mut hasher, &sphere.radius.to_array());
    }
    for line in &object.lines {
        line.points.len().hash(&mut hasher);
        for point in &line.points {
            hash_floats(&mut hasher, &point.to_array());
        }
        hash_floats(&mut hasher, &[line.width]);
    }
    for cuboid in &object.boxes {
        hash_floats(&mut hasher, &cuboid.position.to_array());
        hash_floats(&mut hasher, &cuboid.size.to_array());
        hash_floats(&mut hasher, &cuboid.rotation.to_array());
    }
    for text in &object.texts {
        hash_floats(&mut hasher, &text.position.to_array());
        text.text.hash(&mut hasher);
        hash_floats(&mut hasher, &text.color.to_array());
        hash_floats(&mut hasher, &[text.textSize]);
    }
    hasher.finish()

}

/// Hash these numbers, by their bits.
fn hash_floats(hasher: &mut DefaultHasher, values: &[f32]) {
    for value in values {
        value.to_bits().hash(hasher);
    }
}
