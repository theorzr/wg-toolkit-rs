//! What a battle draws in its clients' world to be debugged, by channel, those chosen
//! with `--debug-draw`: a [`DebugDrawEntity`], every client seeing it.
//!
//! A battle has a [`DebugFrame`], kept from a tick to the next. Each tick, it opens it
//! on its thread, cleared, see [`DebugFrame::begin`], then whatever has something to
//! debug draws in it as it computes, however deep, channel by channel, see
//! [`DebugChannel`]: a [`sphere`], a [`line`] or a [`text`], nothing if the channel
//! isn't drawn, no frame is open, or what draws is [`muted`]. Then the battle takes it
//! back and shows it, see [`DebugFrame::end`] and [`DebugFrame::show`]. The frame keeps
//! its groups and their room from a tick to the next: drawing the same as the last tick
//! takes no memory, only what changed and is sent does.
//!
//! What is drawn is given in the world, where it is as it is drawn, and goes where
//! [`at`] says: in the world, by default, or in a vehicle's frame, see
//! [`DebugPlace::on`], x right, y up, z ahead, from where it stands as it is drawn. Our
//! client mod moves a vehicle's frame with it, turning with it, as the client draws it,
//! while the client has it. The battle names each vehicle's group by its entity's id as
//! it shows the frame.
//!
//! ```text
//!   group name                 drawn
//!   <channel>                  in the world
//!   entity:<id>:<channel>      in the frame of the vehicle of that entity id
//! ```
//!
//! A line has a colour, red, green and blue, packed in its width as `0xRRGGBB`, see
//! [`line`]: our client mod draws it that colour, seen through the map, where the client
//! draws its lines as white boxes that wide.
//!
//! The groups that changed since the last tick are drawn anew, their version one more,
//! see [`DebugFrame::show`], told by a hash of each: what is drawn is rounded to
//! [`SNAP`], so that a vehicle's own shapes, still in its frame, don't change. The
//! client draws spheres as models, see our client mod's `debug_draw`, white, hidden by
//! the map; lines and texts of a colour, seen through it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::cell::RefCell;
use std::io;

use clap::ValueEnum;
use glam::{Affine3A, Quat, Vec3, Vec4};

use wgtk::app::base::{self, EntityInit, SpaceKey, Visibility};
use wgtk::app::entity::EntityKey;

use crate::game::schema::{DebugDrawEntity, DebugDrawObject, DebugLine, DebugSphere, DebugText};
use crate::base::physics::VehicleId;


/// How finely what is drawn is placed, in metres, see the module's documentation.
const SNAP: f32 = 0.01;
/// How big a text is, of the client's default size, 1: twice as big, to be read from
/// behind its own vehicle. Our client mod ignores it, drawing its texts on the screen as
/// big as their font however far, see its `debug_draw`.
const TEXT_SIZE: f32 = 2.0;

thread_local! {
    /// The frame open on this thread, see [`DebugFrame::begin`].
    static FRAME: RefCell<Option<DebugFrame>> = const { RefCell::new(None) };
}


/// What a battle may draw to be debugged, each chosen with `--debug-draw`, see the
/// module's documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DebugChannel {
    /// Under each vehicle, how its enemies best look at it, see `Spotting::debug_draw`.
    Spotting,
    /// Each vehicle's checkpoints, and its enemies' best line of sight to one of them.
    SpotPoints,
    /// Where each vehicle touches the space and the others, as its body steps, see the
    /// physics' `body` and `collision` modules.
    Contacts,
}

/// A set of [`DebugChannel`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugChannels(u32);

/// Where what is drawn goes, see [`at`].
#[derive(Debug, Clone, Copy)]
pub enum DebugPlace {
    /// In the world.
    World,
    /// In a vehicle's frame, from the world to it, see [`Self::on`].
    On(VehicleId, Affine3A),
}

/// What a battle draws to be debugged, see the module's documentation.
#[derive(Debug)]
pub struct DebugFrame {
    /// Its entity, at the middle of the map.
    entity: EntityKey<DebugDrawEntity>,
    /// The channels it draws.
    channels: DebugChannels,
    /// Where what is drawn goes now, see [`at`].
    place: DebugPlace,
    /// Whether what is drawn now is dropped, see [`muted`].
    muted: bool,
    /// Its groups, those drawn in this tick and those kept from the last, cleared, for
    /// their room.
    groups: Vec<Group>,
    /// What it drew last, to tell what changes: a group changed gets a version more.
    drawn: Vec<Drawn>,
    /// What it draws this tick, see [`Self::show`], then swapped with `drawn`.
    drawing: Vec<Drawn>,
}

/// A group of a [`DebugFrame`]: its channel, the vehicle it follows, if any, what it
/// draws, unnamed, and whether anything was drawn in it this tick.
#[derive(Debug)]
struct Group {
    channel: DebugChannel,
    vehicle: Option<VehicleId>,
    object: DebugDrawObject,
    used: bool,
}

/// A group a [`DebugFrame`] draws: which of its groups, the id of the entity it follows,
/// if any, hashes of its name and of what it draws, and its version.
#[derive(Debug, Clone, Copy)]
struct Drawn {
    group: usize,
    entity_id: Option<u32>,
    name: u64,
    shapes: u64,
    version: u32,
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

impl DebugPlace {

    /// In the frame of a vehicle, standing at `position`, turned by `orientation`, as it
    /// is when drawn.
    pub fn on(vehicle: VehicleId, position: Vec3, orientation: Quat) -> Self {
        Self::On(vehicle, Affine3A::from_rotation_translation(orientation, position).inverse())
    }

}

impl DebugFrame {

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
            place: DebugPlace::World,
            muted: false,
            groups: Vec::new(),
            drawn: Vec::new(),
            drawing: Vec::new(),
        })

    }

    /// Open it on this thread, for a tick to draw in, cleared: the groups drawn in the
    /// last kept, empty, the others gone. See [`Self::end`].
    pub fn begin(mut self) {
        self.groups.retain(|group| group.used);
        for group in &mut self.groups {
            let DebugDrawObject { lines, boxes, spheres, texts, .. } = &mut group.object;
            lines.clear();
            boxes.clear();
            spheres.clear();
            texts.clear();
            group.used = false;
        }
        self.place = DebugPlace::World;
        self.muted = false;
        FRAME.set(Some(self));
    }

    /// Close the frame open on this thread, if any, see [`Self::begin`].
    pub fn end() -> Option<Self> {
        FRAME.take()
    }

    /// Draw it in the clients' world, each vehicle's groups by its entity's id, as
    /// `entity_id` gives it, those of a vehicle without dropped: the groups that changed
    /// since the last tick, or came, drawn anew, a version more, those gone taken away.
    /// Nothing is sent if none did; what is sent is moved out of its groups.
    pub fn show(&mut self, app: &mut base::App, entity_id: impl Fn(VehicleId) -> Option<u32>) -> io::Result<()> {

        self.drawing.clear();
        let mut changed = false;
        for (index, group) in self.groups.iter().enumerate().filter(|(_, group)| group.used) {

            let entity_id = match group.vehicle {
                None => None,
                Some(vehicle) => match entity_id(vehicle) {
                    Some(entity_id) => Some(entity_id),
                    None => continue,
                },
            };
            let name = hash_of(&(group.channel as u32, entity_id));
            let shapes = shapes_hash(&group.object);
            let version = match self.drawn.iter().find(|before| before.name == name) {
                Some(before) if before.shapes == shapes => before.version,
                Some(before) => {
                    changed = true;
                    before.version.wrapping_add(1)
                }
                None => {
                    changed = true;
                    0
                }
            };

            self.drawing.push(Drawn {
                group: index,
                entity_id,
                name,
                shapes,
                version,
            });

        }

        changed |= self.drawing.len() != self.drawn.len();
        std::mem::swap(&mut self.drawn, &mut self.drawing);
        if !changed {
            return Ok(());
        }

        let objects = self.drawn.iter()
            .map(|drawn| {
                let group = &mut self.groups[drawn.group];
                let name = match drawn.entity_id {
                    None => group.channel.name().to_string(),
                    Some(entity_id) => format!("entity:{entity_id}:{}", group.channel.name()),
                };
                DebugDrawObject {
                    name,
                    version: drawn.version,
                    destroyTime: -1.0,
                    ..std::mem::take(&mut group.object)
                }
            })
            .collect();
        app.entity(self.entity)?.set_drawObjects(objects)

    }

    /// The group of a channel, following a vehicle, if any, made if new.
    fn group(&mut self, channel: DebugChannel, vehicle: Option<VehicleId>) -> &mut DebugDrawObject {

        let index = match self.groups.iter()
            .position(|group| group.channel == channel && group.vehicle == vehicle) {
                Some(index) => index,
                None => {
                    self.groups.push(Group {
                        channel,
                        vehicle,
                        object: DebugDrawObject::default(),
                        used: false,
                    });
                    self.groups.len() - 1
                }
            };

        let group = &mut self.groups[index];
        group.used = true;
        &mut group.object

    }

}

/// Whether what is drawn now in a channel is kept: a frame is open, drawing it, not
/// muted. To make no text that would be dropped.
pub fn draws(channel: DebugChannel) -> bool {
    FRAME.with_borrow(|frame| frame.as_ref()
        .is_some_and(|frame| !frame.muted && frame.channels.contains(channel)))
}

/// Draw in this place while `draw` runs, see [`DebugPlace`], then where it was.
pub fn at<R>(place: DebugPlace, draw: impl FnOnce() -> R) -> R {
    let before = FRAME.with_borrow_mut(|frame| frame.as_mut()
        .map(|frame| std::mem::replace(&mut frame.place, place)));
    let result = draw();
    if let Some(before) = before {
        FRAME.with_borrow_mut(|frame| if let Some(frame) = frame {
            frame.place = before;
        });
    }
    result
}

/// Drop what is drawn while `draw` runs, wherever.
pub fn muted<R>(draw: impl FnOnce() -> R) -> R {
    let before = FRAME.with_borrow_mut(|frame| frame.as_mut()
        .map(|frame| std::mem::replace(&mut frame.muted, true)));
    let result = draw();
    if let Some(before) = before {
        FRAME.with_borrow_mut(|frame| if let Some(frame) = frame {
            frame.muted = before;
        });
    }
    result
}

/// A sphere at `center`, in the world, of this radius, in metres.
pub fn sphere(channel: DebugChannel, center: Vec3, radius: f32) {
    draw(channel, |object, local| object.spheres.push(DebugSphere {
        position: snap(local.transform_point3(center)),
        radius: Vec3::splat(radius),
    }));
}

/// A line from `from` to `to`, in the world, in this colour, red, green and blue, of
/// 255, see the module's documentation.
pub fn line(channel: DebugChannel, from: Vec3, to: Vec3, color: Vec4) {
    let [red, green, blue, _] = color.to_array()
        .map(|channel| channel.clamp(0.0, 255.0) as u32);
    draw(channel, |object, local| object.lines.push(DebugLine {
        points: [
            snap(local.transform_point3(from)),
            snap(local.transform_point3(to)),
        ],
        width: (red << 16 | green << 8 | blue) as f32,
    }));
}

/// A text at `at`, in the world, of several lines, in this colour, red, green, blue and
/// alpha, of 255. Made only if [`draws`] says it is kept.
pub fn text(channel: DebugChannel, at: Vec3, text: String, color: Vec4) {
    draw(channel, |object, local| object.texts.push(DebugText {
        position: snap(local.transform_point3(at)),
        text,
        color,
        textSize: TEXT_SIZE,
    }));
}

/// Add to the group of a channel where the frame open draws now, if kept, see
/// [`draws`], given it and how to put what is in the world in its frame.
fn draw(channel: DebugChannel, add: impl FnOnce(&mut DebugDrawObject, &Affine3A)) {
    FRAME.with_borrow_mut(|frame| {
        let Some(frame) = frame else { return };
        if frame.muted || !frame.channels.contains(channel) {
            return;
        }
        let (vehicle, local) = match frame.place {
            DebugPlace::World => (None, Affine3A::IDENTITY),
            DebugPlace::On(vehicle, local) => (Some(vehicle), local),
        };
        add(frame.group(channel, vehicle), &local);
    });
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

/// A hash of what a group draws, whatever its name and version: the same shapes, the
/// same hash.
fn shapes_hash(object: &DebugDrawObject) -> u64 {

    let mut hasher = DefaultHasher::new();
    for sphere in &object.spheres {
        hash_floats(&mut hasher, &sphere.position.to_array());
        hash_floats(&mut hasher, &sphere.radius.to_array());
    }
    for line in &object.lines {
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
