//! A space's static models, what its `space.bin` places: each placed model of
//! [`BSMI`] is one of [`BSMO`]'s, whose [`Collision`], when it has one, is a `.havok`
//! file. Models without one are decor nothing collides with, fallen branches or bricks.
//! See the [`super`] module for how this fits with the other sources.
//!
//! # What the scene holds
//!
//! A [`StaticScene`] is the map as it was built, the same for the whole battle:
//!
//! - **Triangles**: every triangle of every body of every placed model visible in the
//!   battle, moved into the world by its placement's transform. Each [`StaticTriangle`]
//!   keeps its bounds and normal, computed once, and where it came from: its placed
//!   model (`instance`, its index in [`BSMI`]), its body's material kind and its
//!   [`BodyKind`]. Malinovka has about 1.2 million.
//! - **Grids**: one per [`BodyKind`], so a test asks only for the kind it cares about.
//!   Each maps a square cell of the ground plane, `CELL` (8 m) wide, to the triangles
//!   whose bounds overlap it. A triangle across several cells is in each of them.
//! - **Destructibles**: a [`PlacedDestructible`] for each placed model that is
//!   destructible, keyed by its `BSMI` index, the same as a triangle's `instance`.
//!   From a triangle a vehicle touches, it gives the client's number for what the
//!   triangle is a part of, its kind and its health at the scale it is placed at.
//!
//! ```text
//!   StaticTriangle { vertices, bounds, normal, instance, material_kind, kind }
//!          │                                       │          │           │
//!          │ found through grids[kind]             │          │           └─ BodyKind
//!          │ by the cells a box covers             ▼          │
//!          │                      destructibles[instance] ────┤
//!          │                      PlacedDestructible          │ a house: which
//!          │                      { chunk_id, index, kind,    │ module, by its
//!          │                        health, modules } ────────┘ material kind
//! ```
//!
//! # How bodies are sorted
//!
//! A `.havok` model has several bodies; [`BodyKind::of`] gives each a kind, by its name
//! and whether [`Destructibles`] lists its model:
//!
//! | Body                                              | Kind           | For a vehicle            |
//! |---------------------------------------------------|----------------|--------------------------|
//! | `s_ramp…`, of a model part that isn't destructible | `Ramp`         | ground it drives up      |
//! | any body of a fence, post or barrel (destructible whole) | `Destructible` | breaks as it drives in |
//! | a house's module, `n_wood0_1` say, and its solid bodies of the module's material kind, `s_wall_1` say | `Destructible` | a wall until it breaks |
//! | `n_…` of a model not listed                        | `Destructible` | nothing: no number to break it by |
//! | `c_…`, what a destroyed module leaves               | `Remains`      | nothing, not used yet    |
//! | anything else                                      | `Solid`        | a wall, or ground if flat |
//!
//! A house's module and its walls share a material kind: the game gives each module
//! one, see [`super::destructibles::FIRST_MODULE_MATERIAL_KIND`], and its collision's
//! bodies carry it. A gun emplacement's `s_wall_1` is the solid side of its wooden
//! `n_wood0_1`, both 73, and breaks with it.
//!
//! # Finding triangles
//!
//! [`StaticScene::triangles_in`] lists a kind's triangles whose bounds overlap a box,
//! walking the cells the box covers. A triangle in several of those cells is reported
//! only by the first of them, the cell where its overlap with the box starts, so each
//! comes once without a set to remember them. [`StaticScene::triangles_at`] is the same
//! for a vertical line, with the triangle's height there: it's how the ground under a
//! vehicle is found on bridges and decks.

use std::io::{self, Read, Seek};
use std::collections::HashMap;
use std::sync::Arc;

use glam::{IVec2, Vec2, Vec3, Vec3Swizzles};
use tracing::warn;

use crate::havok::collision::Collision;
use crate::res::fs::ResFilesystem;

use super::destructibles::{scaled_health, Destructibles, DestructibleKind, Module};
use super::bin::{SpaceBin, BSMI, BSMO, BWST, WGDE};


/// The side of the grid's square cells the triangles are indexed in, in metres: about
/// a hull's length.
const CELL: f32 = 8.0;


/// The triangles of a space's static models, in the world.
#[derive(Debug, Default)]
pub struct StaticScene {
    triangles: Vec<StaticTriangle>,
    /// For each [`BodyKind`], the triangles whose bounds overlap each cell, by index.
    grids: [HashMap<IVec2, Vec<u32>>; 4],
    /// The placed models that are destructible, by their index in [`BSMI`].
    destructibles: HashMap<u32, PlacedDestructible>,
}

/// A destructible placed model, as the client numbers it and as it is destroyed, see
/// [`WGDE`] and [`Destructibles`].
#[derive(Debug, Clone)]
pub struct PlacedDestructible {
    /// The id of the 100 m chunk it is in, see [`WGDE`].
    pub chunk_id: u16,
    /// Its index in its chunk.
    pub index: u16,
    pub kind: DestructibleKind,
    /// Its health, whole, scaled with it, see [`scaled_health`].
    pub health: f32,
    /// A structure's modules, their health scaled with it.
    pub modules: Vec<Module>,
}

impl PlacedDestructible {

    /// One of a structure's modules, by its material kind.
    pub fn module(&self, material_kind: u8) -> Option<&Module> {
        self.modules.iter().find(|module| module.material_kind == material_kind)
    }

}

/// A triangle of a placed static model.
#[derive(Debug, Clone, Copy)]
pub struct StaticTriangle {
    pub vertices: [Vec3; 3],
    /// Its bounding box, minimum then maximum.
    pub bounds: [Vec3; 2],
    /// Its unit normal, its vertices' winding deciding its side; zero if degenerate.
    pub normal: Vec3,
    /// Which placed model it is of, its index in [`BSMI`].
    pub instance: u32,
    /// Its body's material kind, see [`crate::havok::collision::CollisionBody`].
    pub material_kind: u32,
    pub kind: BodyKind,
}

/// What a static model's body is to what drives into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// It stops what drives into it.
    Solid = 0,
    /// It is driven up, a truck's `s_ramp_0` say.
    Ramp = 1,
    /// It breaks or falls when driven into, as [`Destructibles`] lists it, or as its
    /// `n_` name says.
    Destructible = 2,
    /// What a destructible module leaves once destroyed, `c_c_stone0_1` of
    /// `n_stone0_1` say: nothing until then.
    Remains = 3,
}

impl BodyKind {

    /// The kind of a body of a model, by the model's path and the body's name and
    /// material kind, see [`crate::havok::collision::CollisionBody`].
    pub fn of(destructibles: &Destructibles, model: &str, body: &str, material_kind: u32) -> Self {
        if body.starts_with("s_ramp") && !destructibles.is_destructible(model, body, material_kind) {
            Self::Ramp
        } else if destructibles.is_destructible(model, body, material_kind) || body.starts_with("n_") {
            Self::Destructible
        } else if body.starts_with("c_") {
            Self::Remains
        } else {
            Self::Solid
        }
    }

}

impl StaticTriangle {

    pub fn new(vertices: [Vec3; 3], instance: u32, material_kind: u32, kind: BodyKind) -> Self {
        let [a, b, c] = vertices;
        Self {
            vertices,
            bounds: [a.min(b).min(c), a.max(b).max(c)],
            normal: (b - a).cross(c - a).normalize_or_zero(),
            instance,
            material_kind,
            kind,
        }
    }

    /// Its height above a point of the ground plane, if it covers it: where a vertical
    /// line there crosses it. None for a vertical triangle.
    pub fn height_at(&self, point: Vec2) -> Option<f32> {
        let [a, b, c] = self.vertices.map(Vec3Swizzles::xz);
        let area = (b - a).perp_dot(c - a);
        if area.abs() < 1e-9 {
            return None;
        }
        // Barycentric weights of `b` and `c`, all three within 0 and 1 inside.
        let u = (point - a).perp_dot(c - a) / area;
        let v = (b - a).perp_dot(point - a) / area;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            return None;
        }
        let [a, b, c] = self.vertices.map(|vertex| vertex.y);
        Some(a + (b - a) * u + (c - a) * v)
    }

}

impl StaticScene {

    /// Load the static models of a space, `spaces/02_malinovka` say, those visible in
    /// a visibility mask, see [`super::visibility_mask`], their bodies' kinds from these
    /// destructibles. A model whose collision fails to load is left out, and logged.
    pub fn load(fs: &ResFilesystem, space_dir: &str, definitions: &Destructibles, visibility_mask: u32) -> io::Result<Self> {

        let mut bin = SpaceBin::load(fs, space_dir)?;
        let (strings, instances, models) = read_sections(&mut bin)
            .map_err(|e| io::Error::new(e.kind(), format!("{space_dir}/space.bin: {e}")))?;
        let numbered = bin.read_section::<WGDE>()
            .inspect_err(|e| warn!(space = space_dir, "Failed to read the destructibles' numbering: {e}"))
            .map(|section| section.destructibles)
            .unwrap_or_default();

        // Each model's collision, loaded once for all its placements, with its bodies'
        // kinds.
        let mut collisions: HashMap<u32, Option<(Arc<Collision>, Vec<BodyKind>)>> = HashMap::new();
        let mut triangles = Vec::new();
        for (index, instance) in instances.instances.iter().enumerate() {
            if instance.visibility_mask & visibility_mask == 0 {
                continue;
            }
            let Some(havok) = models.models.get(instance.model as usize).and_then(|model| model.havok_fnv) else { continue };
            let collision = collisions.entry(havok).or_insert_with(|| {
                let Some(path) = strings.get_string(havok) else {
                    warn!(space = space_dir, hash = havok, "Failed to find a static model's collision path");
                    return None;
                };
                let collision = Collision::load(fs, path)
                    .inspect_err(|e| warn!(space = space_dir, path, "Failed to load a static model's collision: {e}"))
                    .ok()?;
                let kinds = collision.bodies.iter().map(|body| BodyKind::of(definitions, path, &body.name, body.material_kind)).collect();
                Some((Arc::new(collision), kinds))
            });
            let Some((collision, kinds)) = collision else { continue };
            for (body, &kind) in collision.bodies.iter().zip(kinds.iter()) {
                for triangle in &body.triangles {
                    let vertices = triangle.map(|vertex| instance.transform.transform_point3(vertex));
                    triangles.push(StaticTriangle::new(vertices, index as u32, body.material_kind, kind));
                }
            }
        }

        // Those that the client numbers and the game lists as destructible.
        let mut destructibles = HashMap::new();
        for numbered in numbered {
            for &index in &numbered.instances {
                let Some(instance) = instances.instances.get(index as usize) else { continue };
                if instance.visibility_mask & visibility_mask == 0 {
                    continue;
                }
                let Some(havok) = models.models.get(instance.model as usize).and_then(|model| model.havok_fnv) else { continue };
                let Some(destructible) = strings.get_string(havok).and_then(|path| definitions.get(path)) else { continue };
                let scale = instance.transform.matrix3.x_axis.length();
                let modules = destructible.modules.values()
                    .map(|module| Module {
                        health: scaled_health(scale, module.health),
                        ..module.clone()
                    })
                    .collect();
                destructibles.insert(index, PlacedDestructible {
                    chunk_id: numbered.chunk_id,
                    index: numbered.index,
                    kind: destructible.kind,
                    health: scaled_health(scale, destructible.health),
                    modules,
                });
            }
        }

        let mut scene = Self::from_triangles(triangles);
        scene.destructibles = destructibles;
        Ok(scene)

    }

    /// Make a placed model destructible, by its index in [`BSMI`]: for scenes built of
    /// triangles, see [`Self::from_triangles`].
    pub fn add_destructible(&mut self, instance: u32, destructible: PlacedDestructible) {
        self.destructibles.insert(instance, destructible);
    }

    /// A placed model's destructible, by its index in [`BSMI`], if it is one.
    pub fn destructible(&self, instance: u32) -> Option<&PlacedDestructible> {
        self.destructibles.get(&instance)
    }

    /// Every destructible placed model, by its index in [`BSMI`].
    pub fn destructibles(&self) -> impl Iterator<Item = (u32, &PlacedDestructible)> {
        self.destructibles.iter().map(|(&instance, destructible)| (instance, destructible))
    }

    /// A scene of these triangles, already in the world.
    pub fn from_triangles(triangles: Vec<StaticTriangle>) -> Self {
        let mut grids: [HashMap<IVec2, Vec<u32>>; 4] = Default::default();
        for (index, triangle) in triangles.iter().enumerate() {
            let [min, max] = triangle.bounds.map(|bound| cell_of(bound.xz()));
            for x in min.x..=max.x {
                for z in min.y..=max.y {
                    grids[triangle.kind as usize].entry(IVec2::new(x, z)).or_default().push(index as u32);
                }
            }
        }
        Self {
            triangles,
            grids,
            destructibles: HashMap::new(),
        }
    }

    /// How many triangles it has.
    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The triangles of a body kind whose bounds overlap a box, minimum then maximum,
    /// each once: each cell reports those whose overlap with the box starts in it.
    pub fn triangles_in(&self, kind: BodyKind, min: Vec3, max: Vec3) -> impl Iterator<Item = &StaticTriangle> {
        let grid = &self.grids[kind as usize];
        let first = cell_of(min.xz());
        let last = cell_of(max.xz());
        (first.x..=last.x)
            .flat_map(move |x| (first.y..=last.y).map(move |z| IVec2::new(x, z)))
            .flat_map(move |cell| grid.get(&cell).into_iter().flatten().map(move |&index| (cell, index)))
            .map(|(cell, index)| (cell, &self.triangles[index as usize]))
            .filter(move |(cell, triangle)| {
                let [low, high] = triangle.bounds;
                low.cmple(max).all() && high.cmpge(min).all() && *cell == cell_of(low.xz()).max(first)
            })
            .map(|(_, triangle)| triangle)
    }

    /// The triangles of a body kind above or below a point of the ground plane, each
    /// with its height there.
    pub fn triangles_at(&self, kind: BodyKind, point: Vec2) -> impl Iterator<Item = (&StaticTriangle, f32)> {
        let min = Vec3::new(point.x, f32::MIN, point.y);
        let max = Vec3::new(point.x, f32::MAX, point.y);
        self.triangles_in(kind, min, max)
            .filter_map(move |triangle| Some((triangle, triangle.height_at(point)?)))
    }

}


/// The sections of a compiled space its static models need: its strings, its placed
/// models and its models.
fn read_sections<R: Read + Seek>(bin: &mut SpaceBin<R>) -> io::Result<(BWST, BSMI, BSMO)> {
    Ok((bin.read_section()?, bin.read_section()?, bin.read_section()?))
}

/// The grid's cell of a point of the ground plane.
fn cell_of(point: Vec2) -> IVec2 {
    (point / CELL).floor().as_ivec2()
}


#[cfg(test)]
mod tests {

    use super::*;

    /// A triangle across several cells is found once, in a box over them all, and not
    /// in a box beside it or above it.
    #[test]
    fn finds_each_triangle_once() {
        let triangle = StaticTriangle::new([Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0)], 0, 0, BodyKind::Solid);
        let scene = StaticScene::from_triangles(vec![triangle]);
        assert_eq!(scene.triangles_in(BodyKind::Solid, Vec3::splat(-20.0), Vec3::splat(20.0)).count(), 1);
        assert_eq!(scene.triangles_in(BodyKind::Solid, Vec3::new(5.0, 1.0, 5.0), Vec3::new(30.0, 2.0, 30.0)).count(), 1);
        assert_eq!(scene.triangles_in(BodyKind::Ramp, Vec3::splat(-20.0), Vec3::splat(20.0)).count(), 0);
        assert_eq!(scene.triangles_in(BodyKind::Solid, Vec3::new(11.0, 0.0, -20.0), Vec3::new(30.0, 5.0, 20.0)).count(), 0);
        assert_eq!(scene.triangles_in(BodyKind::Solid, Vec3::new(-20.0, 6.0, -20.0), Vec3::new(20.0, 9.0, 20.0)).count(), 0);
        assert!((scene.triangles[0].normal.length() - 1.0).abs() < 1e-5);
        // Its height above points of the ground plane, inside it, and none outside.
        let heights: Vec<f32> = scene.triangles_at(BodyKind::Solid, Vec2::new(5.0, 0.0)).map(|(_, height)| height).collect();
        assert_eq!(heights.len(), 1);
        assert!((heights[0] - 2.5).abs() < 1e-5, "{heights:?}");
        assert_eq!(scene.triangles_at(BodyKind::Solid, Vec2::new(-5.0, 5.0)).count(), 0);
    }

    /// With the game's `res` directory in `WGT_RES_PATH`: Malinovka's placed collision,
    /// as many triangles as the client's models give, and a hangar of Graf Zeppelin's
    /// story map where a captured drive hits it, 2.8 m ahead of the hull's centre, its
    /// lamp post destructible.
    #[test]
    fn real_static_scenes() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let destructibles = Destructibles::load(&fs).unwrap();
        assert_eq!(StaticScene::load(&fs, "spaces/02_malinovka", &destructibles, u32::MAX).unwrap().len(), 1_246_841);

        let scene = StaticScene::load(&fs, "spaces/120_graf_zeppelin_scc", &destructibles, u32::MAX).unwrap();
        // The hull's centre when it stops against the hangar, heading -z.
        let center = Vec3::new(148.4, 15.7, 480.2);
        let ahead = scene.triangles_in(BodyKind::Solid, center - Vec3::new(4.0, 0.3, 4.0), center + Vec3::new(4.0, 3.0, 4.0))
            .flat_map(|triangle| triangle.vertices)
            .filter(|vertex| (center.y - 0.3..center.y + 3.0).contains(&vertex.y))
            .map(|vertex| (vertex - center).xz().length())
            .fold(f32::INFINITY, f32::min);
        assert!((2.6..3.0).contains(&ahead), "{ahead}");
        // The lamp post it hits next, 0.6 m from its centre then.
        let lamp_post = Vec3::new(156.2, 15.6, 479.8);
        let near = scene.triangles_in(BodyKind::Destructible, lamp_post - Vec3::splat(2.0), lamp_post + Vec3::splat(2.0)).count();
        assert!(near > 0);
        // What the real server named as the captured Pershing knocked them down: the
        // lamp post it fell, then the box it broke, in the chunk it drove in.
        let post = scene.destructible(5387).unwrap();
        assert_eq!((post.chunk_id, post.index, post.kind, post.health), (0x8083, 21, DestructibleKind::FallingAtom, 10.0));
        let crate_box = scene.destructible(5396).unwrap();
        assert_eq!((crate_box.chunk_id, crate_box.index, crate_box.kind), (0x8083, 9, DestructibleKind::Fragile));
    }


}
