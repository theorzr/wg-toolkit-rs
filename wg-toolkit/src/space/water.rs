//! A space's water: each body of it a flat surface, as its triangles cover the map, see
//! [`BWWA`]. The triangles reach over the land around it, where the client draws them
//! hidden under the terrain: there is water only where the ground is under the surface.
//! Where a body's surface lies over the ground plane, [`Water::level_at`], is looked for
//! in a grid of [`CELL`] wide squares, each listing the triangles over it.
//!
//! ```text
//!   BWWa  body: box, ranges ──▶ its vertices, on a grid of its own, at its height
//!                          └──▶ its triangles, by their vertices' indices
//!                                          │
//!                                          ▼
//!                       Water { triangles, a grid of them, shoreline }
//! ```
//!
//! Its shoreline, [`Water::shoreline`], is where its surface meets the ground, traced by
//! marching squares [`SHORE_STEP`] wide over the squares of its grid, those under its
//! triangles at all four corners: at each corner, how high the surface is over the
//! ground, its sign changing across the shore, which crosses a square's side where that
//! height, from one corner to the next, is 0.

use std::collections::HashMap;
use std::io;

use glam::{IVec2, Vec2, Vec3, Vec3Swizzles};

use crate::res::fs::ResFilesystem;

use super::bin::{SpaceBin, SpaceSection, BWWA};


/// How wide a square of the triangles' grid is, in metres, see the module's
/// documentation. Our choice: the game's water meshes are on a grid of about 20 m.
const CELL: f32 = 20.0;
/// How wide a square of the shoreline's marching squares is, in metres, see the module's
/// documentation: a fifth of [`CELL`]. Our choice.
const SHORE_STEP: f32 = 4.0;


/// See the module's documentation. The default one has no water.
#[derive(Debug, Default)]
pub struct Water {
    /// How many bodies of water it has.
    bodies: usize,
    /// Its triangles, of all bodies, each a flat one at its body's height.
    triangles: Vec<[Vec3; 3]>,
    /// For each square of the grid, by its corner of least coordinates in [`CELL`]s,
    /// the indexes in [`Self::triangles`] of those over it.
    cells: HashMap<IVec2, Vec<u32>>,
    /// Where its surface meets the ground, see the module's documentation.
    shoreline: Vec<[Vec3; 2]>,
}

impl Water {

    /// Load the water of a space, `spaces/08_ruinberg` say, none if its compiled space
    /// has none, its shoreline traced on the ground, its height at a point of the ground
    /// plane, its terrain's say.
    pub fn load(fs: &ResFilesystem, space_dir: &str, ground: impl Fn(f32, f32) -> Option<f32>) -> io::Result<Self> {

        let mut bin = SpaceBin::load(fs, space_dir)?;
        if bin.section(BWWA::ID).is_none() {
            return Ok(Self::default());
        }

        Ok(Self::new(&bin.read_section::<BWWA>()?, ground))

    }

    /// The water of a compiled space's section, its triangles in their grid, its
    /// shoreline traced on the ground, as in [`Self::load`].
    pub fn new(section: &BWWA, ground: impl Fn(f32, f32) -> Option<f32>) -> Self {

        let mut triangles = Vec::new();
        for body in &section.bodies {

            let vertices = &section.vertices[body.vertices.clone()];
            triangles.extend(section.indices[body.indices.clone()].chunks_exact(3)
                .filter_map(|triangle| {

                    let [a, b, c] = 
                        [triangle[0], triangle[1], triangle[2]]
                            .map(|index| vertices.get(index as usize)
                                .map(|vertex| vertex.position));
                            
                    Some([a?, b?, c?])

                }));
            
        }

        let mut cells = HashMap::<IVec2, Vec<u32>>::new();
        for (index, triangle) in triangles.iter().enumerate() {
            let min = triangle.iter().fold(Vec2::INFINITY, |min, corner| min.min(corner.xz()));
            let max = triangle.iter().fold(Vec2::NEG_INFINITY, |max, corner| max.max(corner.xz()));
            let min = (min / CELL).floor().as_ivec2();
            let max = (max / CELL).floor().as_ivec2();
            for x in min.x..=max.x {
                for y in min.y..=max.y {
                    cells.entry(IVec2::new(x, y)).or_default().push(index as u32);
                }
            }
        }

        let mut water = Self {
            bodies: section.bodies.len(),
            triangles,
            cells,
            shoreline: Vec::new(),
        };
        water.shoreline = water.trace_shoreline(&ground);
        water

    }

    /// Where its surface meets the ground, each line on a square of the marching squares,
    /// see the module's documentation.
    fn trace_shoreline(&self, ground: &impl Fn(f32, f32) -> Option<f32>) -> Vec<[Vec3; 2]> {

        let per_cell = (CELL / SHORE_STEP).round() as i32;
        let mut shoreline = Vec::new();
        for cell in self.cells.keys() {
            for i in 0..per_cell {
                for j in 0..per_cell {

                    let min = cell.as_vec2() * CELL + Vec2::new(i as f32, j as f32) * SHORE_STEP;
                    let corners = [
                        min,
                        min + Vec2::new(SHORE_STEP, 0.0),
                        min + Vec2::splat(SHORE_STEP),
                        min + Vec2::new(0.0, SHORE_STEP),
                    ];
                    let [Some(a), Some(b), Some(c), Some(d)] = corners.map(|corner| self.level_at(corner.x, corner.y)) else {
                        continue;
                    };
                    let levels = [a, b, c, d];
                    let level = a.max(b).max(c).max(d);

                    // Its height over the ground, at each corner: below 0 off the water.
                    let heights = [0, 1, 2, 3].map(|at| match ground(corners[at].x, corners[at].y) {
                        Some(ground) => levels[at] - ground,
                        None => -1.0,
                    });

                    let crossings: Vec<Vec3> = (0..4)
                        .filter_map(|from| {
                            let to = (from + 1) % 4;
                            if (heights[from] > 0.0) == (heights[to] > 0.0) {
                                return None;
                            }
                            let along = heights[from] / (heights[from] - heights[to]);
                            let point = corners[from].lerp(corners[to], along);
                            Some(Vec3::new(point.x, level, point.y))
                        })
                        .collect();

                    // A saddle, its opposite corners wet, crosses twice: two lines.
                    shoreline.extend(crossings.chunks_exact(2).map(|line| [line[0], line[1]]));

                }
            }
        }

        shoreline

    }

    /// How many bodies of water it has.
    pub fn bodies(&self) -> usize {
        self.bodies
    }

    /// Where its surface meets the ground, see the module's documentation.
    pub fn shoreline(&self) -> &[[Vec3; 2]] {
        &self.shoreline
    }

    /// The height of its surface over this point of the ground plane, if one of its
    /// triangles is there, the ground under it or not: the highest, should two bodies
    /// overlap.
    pub fn level_at(&self, x: f32, z: f32) -> Option<f32> {

        let point = Vec2::new(x, z);
        let cell = (point / CELL).floor().as_ivec2();

        self.cells.get(&cell)?.iter()
            .map(|&index| &self.triangles[index as usize])
            .filter(|triangle| covers(triangle, point))
            .map(|triangle| triangle[0].y)
            .reduce(f32::max)

    }

}

/// Whether a triangle covers a point of the ground plane, its edges included, either
/// way it winds.
fn covers(triangle: &[Vec3; 3], point: Vec2) -> bool {

    let [a, b, c] = triangle.map(|corner| corner.xz());
    let sides = [
        (b - a).perp_dot(point - a),
        (c - b).perp_dot(point - b),
        (a - c).perp_dot(point - c),
    ];

    sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0)

}


#[cfg(test)]
mod tests {

    use super::*;

    use crate::space::bin::{WaterBody, WaterVertex};
    use crate::space::terrain::Terrain;

    /// A vertex of water at 4 m high.
    fn vertex(x: f32, z: f32) -> WaterVertex {
        WaterVertex {
            position: Vec3::new(x, 4.0, z),
            cell: [0, 0],
            depth: 0.0,
        }
    }

    /// A square of water of two triangles, 20 m wide at 4 m high, over a ground stepping
    /// from 0 m to 8 m high at x 5 m: its surface over it inside, its edges included,
    /// not outside, and its shoreline across the step, at its height, 20 m long.
    #[test]
    fn square() {

        let section = BWWA {
            bodies: vec![WaterBody {
                bounds: [Vec3::new(0.0, 4.0, 0.0), Vec3::new(20.0, 4.0, 20.0)],
                position: Vec3::new(10.0, 4.0, 10.0),
                size: Vec2::splat(20.0),
                tiles: 0..0,
                vertices: 0..4,
                indices: 0..6,
            }],
            tiles: Vec::new(),
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(20.0, 0.0),
                vertex(20.0, 20.0),
                vertex(0.0, 20.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        };

        let water = Water::new(&section, |x, _| Some(if x < 5.0 { 0.0 } else { 8.0 }));
        assert_eq!(water.level_at(2.0, 7.0), Some(4.0));
        assert_eq!(water.level_at(20.0, 5.0), Some(4.0));
        assert_eq!(water.level_at(-0.1, 5.0), None);
        assert_eq!(water.level_at(25.0, 5.0), None);

        let shoreline = water.shoreline();
        let length: f32 = shoreline.iter().map(|[from, to]| from.distance(*to)).sum();
        assert!((length - 20.0).abs() < 1e-3, "{length}");
        assert!(shoreline.iter().flatten().all(|point| point.y == 4.0 && (4.0..=8.0).contains(&point.x)));

    }

    /// With the game's `res` directory in `WGT_RES_PATH`: Malinovka's water, one body at
    /// 4.06 m high, over its deepest vertex, and its shoreline traced.
    #[test]
    fn real_water() {

        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        let terrain = Terrain::load(&fs, "spaces/02_malinovka").unwrap();
        let water = Water::load(&fs, "spaces/02_malinovka", |x, z| terrain.height_at(x, z)).unwrap();

        assert_eq!(water.bodies(), 1);
        assert_eq!(water.level_at(-662.766, 455.56), Some(4.059));
        assert!(water.shoreline().len() > 1000, "{}", water.shoreline().len());

    }

}
