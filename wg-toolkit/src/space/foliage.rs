//! A space's foliage: its speed trees, trees and bushes alike, each a box that hides
//! what is behind it as much as its kind's `density` says, see
//! [`super::destructibles::Destructible::density`].
//!
//! A plant is a speed tree of [`SPTR`], numbered by the client as a destructible or
//! not: a young pine or a hawthorn the client doesn't number hides as much as a maple.
//! Its box is its kind's bounds, as its `.srt` file's header gives them, placed by its
//! transform: a Maple_25m's is 23 m wide and 27.5 m high, a Bush_Wild_5m's 7 m wide and
//! 4.7 m high. A plant whose kind `destructibles.xml` doesn't list, ivy on a wall say,
//! is left out, its density unknown.
//!
//! ```text
//!   SpTr  tree: transform, .srt hash ──▶ BWST: vegetation/…/Maple_25m.srt
//!                                          │                 │
//!                                          ▼                 ▼
//!                          <srt> header: its bounds    destructibles.xml: its density
//!                                          │                 │
//!                                          └──────┬──────────┘
//!                                                 ▼
//!                                   Plant { tree, transform, bounds, density }
//! ```
//!
//! What a line of sight crosses, [`Foliage::crossings`], is looked for in a grid of
//! [`CELL`] wide squares on the ground plane, each listing the plants whose box reaches
//! over it: the line walks the squares under it, and each plant they list is tested
//! once, its box against the line in the plant's own frame.

use std::collections::HashMap;
use std::io::{self, Read};

use glam::{Affine3A, BVec3, IVec2, Vec2, Vec3, Vec3Swizzles};
use tracing::warn;

use crate::res::fs::ResFilesystem;

use super::destructibles::Destructibles;
use super::bin::{SpaceBin, BWST, SPTR};


/// How a SpeedTree 6 `.srt` file starts, a version on 16 bytes, see [`srt_bounds`].
const SRT_MAGIC: &[u8] = b"SRT 06.";

/// How wide a square of the plants' grid is, in metres, see the module's documentation.
/// Our choice: about a big tree's crown.
const CELL: f32 = 25.0;


/// See the module's documentation.
#[derive(Debug, Default)]
pub struct Foliage {
    /// Its plants, in [`SPTR`]'s order.
    plants: Vec<Plant>,
    /// For each square of the grid, by its corner of least coordinates in [`CELL`]s,
    /// the indexes in [`Self::plants`] of those reaching over it.
    cells: HashMap<IVec2, Vec<u32>>,
}

/// A speed tree as foliage, see the module's documentation.
#[derive(Debug, Clone)]
pub struct Plant {
    /// Its index in [`SPTR`], which the client numbers a destructible tree by, see
    /// [`super::bin::WGDE`].
    pub tree: u32,
    /// Its placement in the world: its scaled axes, then the point it stands on.
    pub transform: Affine3A,
    /// Its kind's box, minimum then maximum, in its own frame, Y up.
    pub bounds: [Vec3; 2],
    /// How much of what is behind it it hides, of 1, see the module's documentation.
    pub density: f32,
}

/// Where a line crosses a plant, see [`Plant::crossing`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoliageCrossing {
    /// The plant's [`Plant::tree`].
    pub tree: u32,
    /// Its [`Plant::density`].
    pub density: f32,
    /// How far along the line it enters the plant's box, and leaves it, in metres from
    /// its start: 0 for a line starting in it, its length for one ending in it.
    pub enter: f32,
    pub leave: f32,
}

impl Foliage {

    /// Load the foliage of a space, `spaces/02_malinovka` say, its kinds' densities from
    /// these destructibles. A space without speed trees has none; a kind whose `.srt`
    /// fails to read is left out, and logged.
    pub fn load(fs: &ResFilesystem, space_dir: &str, destructibles: &Destructibles) -> io::Result<Self> {

        let mut bin = SpaceBin::load(fs, space_dir)?;
        let strings = bin.read_section::<BWST>()?;
        let Ok(trees) = bin.read_section::<SPTR>()
            .inspect_err(|e| warn!(space = space_dir, "Failed to read the speed trees: {e}"))
        else {
            return Ok(Self::default());
        };

        // Each kind's bounds and density, read once for all its plants.
        let mut kinds: HashMap<u32, Option<([Vec3; 2], f32)>> = HashMap::new();
        let mut plants = Vec::new();
        for (index, tree) in trees.trees.iter().enumerate() {

            let kind = kinds.entry(tree.resource).or_insert_with(|| {
                let path = strings.get_string(tree.resource)?;
                let density = destructibles.get(path)?.density;
                let bounds = srt_bounds(fs, path)
                    .inspect_err(|e| warn!(space = space_dir, path, "Failed to read a speed tree's bounds: {e}"))
                    .ok()?;
                Some((bounds, density))
            });

            let Some((bounds, density)) = *kind else { continue };
            plants.push(Plant {
                tree: index as u32,
                transform: tree.transform,
                bounds,
                density,
            });

        }

        Ok(Self::new(plants))

    }

    /// Foliage of these plants, sorted by their [`Plant::tree`], each listed in the grid
    /// squares its box reaches over.
    pub fn new(mut plants: Vec<Plant>) -> Self {

        plants.sort_by_key(|plant| plant.tree);

        let mut cells: HashMap<IVec2, Vec<u32>> = HashMap::new();
        for (index, plant) in plants.iter().enumerate() {
            let [low, high] = plant.reach().map(cell_of);
            for x in low.x..=high.x {
                for y in low.y..=high.y {
                    cells.entry(IVec2::new(x, y)).or_default().push(index as u32);
                }
            }
        }

        Self {
            plants,
            cells,
        }

    }

    /// Its plants, sorted by their [`Plant::tree`].
    pub fn plants(&self) -> &[Plant] {
        &self.plants
    }

    /// Its plant of this index in [`SPTR`], if its kind is known.
    pub fn plant(&self, tree: u32) -> Option<&Plant> {
        let index = self.plants.binary_search_by_key(&tree, |plant| plant.tree).ok()?;
        Some(&self.plants[index])
    }

    /// Where the line from `from` to `to` crosses its plants, each once, in no order,
    /// see the module's documentation.
    pub fn crossings(&self, from: Vec3, to: Vec3) -> Vec<FoliageCrossing> {

        let mut listed = cells_along(from.xz(), to.xz())
            .iter()
            .filter_map(|cell| self.cells.get(cell))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        listed.sort_unstable();
        listed.dedup();

        listed.into_iter()
            .filter_map(|index| self.plants[index as usize].crossing(from, to))
            .collect()

    }

}

impl Plant {

    /// Where the line from `from` to `to` crosses its box, if it does.
    pub fn crossing(&self, from: Vec3, to: Vec3) -> Option<FoliageCrossing> {
        self.crossing_at(&self.transform, from, to)
    }

    /// Where the line from `from` to `to` crosses its box placed by `transform`, not its
    /// own: as it lies once felled, say.
    pub fn crossing_at(&self, transform: &Affine3A, from: Vec3, to: Vec3) -> Option<FoliageCrossing> {

        // In its own frame, where its box is aligned: an affine map keeps how far along
        // the line a point is, of its length.
        let local = transform.inverse();
        let start = local.transform_point3(from);
        let offset = local.transform_point3(to) - start;

        let [min, max] = self.bounds;
        let (mut enter, mut leave) = (0.0f32, 1.0f32);
        for axis in 0..3 {
            if offset[axis].abs() <= f32::EPSILON {
                if start[axis] < min[axis] || start[axis] > max[axis] {
                    return None;
                }
                continue;
            }
            let near = (min[axis] - start[axis]) / offset[axis];
            let far = (max[axis] - start[axis]) / offset[axis];
            enter = enter.max(near.min(far));
            leave = leave.min(near.max(far));
        }
        if enter > leave {
            return None;
        }

        let length = (to - from).length();
        Some(FoliageCrossing {
            tree: self.tree,
            density: self.density,
            enter: enter * length,
            leave: leave * length,
        })

    }

    /// How far its box reaches on the ground plane, its least then greatest
    /// coordinates, X and Z.
    fn reach(&self) -> [Vec2; 2] {
        let [min, max] = self.bounds;
        let mut low = Vec2::splat(f32::INFINITY);
        let mut high = Vec2::splat(f32::NEG_INFINITY);
        for corner in 0..8 {
            let at = Vec3::select(BVec3::new(corner & 1 != 0, corner & 2 != 0, corner & 4 != 0), max, min);
            let at = self.transform.transform_point3(at).xz();
            low = low.min(at);
            high = high.max(at);
        }
        [low, high]
    }

}

/// The bounds of a speed tree's kind, minimum then maximum, as its `.srt` file's header
/// gives them: after its version, on 16 bytes, and 4 of unknown use, 0, six floats.
pub fn srt_bounds(fs: &ResFilesystem, path: &str) -> io::Result<[Vec3; 2]> {

    let mut header = [0; 44];
    fs.read(path)?.read_exact(&mut header)?;
    if !header.starts_with(SRT_MAGIC) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a SpeedTree 6 file"));
    }

    let mut floats = [0.0; 6];
    for (float, bytes) in floats.iter_mut().zip(header[20..].chunks_exact(4)) {
        *float = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    let [min_x, min_y, min_z, max_x, max_y, max_z] = floats;
    Ok([Vec3::new(min_x, min_y, min_z), Vec3::new(max_x, max_y, max_z)])

}

/// The grid square of a point of the ground plane, see [`Foliage`].
fn cell_of(at: Vec2) -> IVec2 {
    (at / CELL).floor().as_ivec2()
}

/// The grid squares under the line from `from` to `to` on the ground plane, in order:
/// each it enters, one axis at a time, as far along as it crosses their sides.
fn cells_along(from: Vec2, to: Vec2) -> Vec<IVec2> {

    let mut cell = cell_of(from);
    let end = cell_of(to);
    let offset = to - from;

    let step = IVec2::new(
        if offset.x < 0.0 { -1 } else { 1 },
        if offset.y < 0.0 { -1 } else { 1 },
    );
    let mut next = Vec2::new(
        side_crossed(from.x, offset.x, cell.x),
        side_crossed(from.y, offset.y, cell.y),
    );
    let across = Vec2::splat(CELL) / offset.abs();

    let squares = (end - cell).abs().element_sum();
    let mut cells = Vec::with_capacity(squares as usize + 1);
    cells.push(cell);
    for _ in 0..squares {
        if next.x < next.y {
            cell.x += step.x;
            next.x += across.x;
        } else {
            cell.y += step.y;
            next.y += across.y;
        }
        cells.push(cell);
    }
    cells

}

/// How far along a line, of its offset, it crosses the next side of its square, along
/// one axis: from `start`, by `offset`, in the square `cell`. Never along no offset.
fn side_crossed(start: f32, offset: f32, cell: i32) -> f32 {
    if offset > 0.0 {
        ((cell + 1) as f32 * CELL - start) / offset
    } else if offset < 0.0 {
        (cell as f32 * CELL - start) / offset
    } else {
        f32::INFINITY
    }
}


#[cfg(test)]
mod tests {

    use super::*;

    /// A bush 6 m wide and 4 m high standing at `at`, of SpTr index `tree`.
    fn bush(tree: u32, at: Vec3) -> Plant {
        Plant {
            tree,
            transform: Affine3A::from_translation(at),
            bounds: [Vec3::new(-3.0, 0.0, -3.0), Vec3::new(3.0, 4.0, 3.0)],
            density: 0.5,
        }
    }

    #[test]
    fn walks_the_squares_under_a_line() {
        let cells = cells_along(Vec2::new(1.0, 1.0), Vec2::new(60.0, 30.0));
        assert_eq!(cells, [IVec2::new(0, 0), IVec2::new(1, 0), IVec2::new(1, 1), IVec2::new(2, 1)]);
        let cells = cells_along(Vec2::new(-1.0, 10.0), Vec2::new(-1.0, -40.0));
        assert_eq!(cells, [IVec2::new(-1, 0), IVec2::new(-1, -1), IVec2::new(-1, -2)]);
    }

    #[test]
    fn crosses_each_plant_on_the_line_once() {

        // One on a square's corner, listed in four squares, one by the line, one off it.
        let foliage = Foliage::new(vec![
            bush(7, Vec3::new(25.0, 0.0, 25.0)),
            bush(3, Vec3::new(60.0, 0.0, 22.0)),
            bush(5, Vec3::new(40.0, 0.0, 40.0)),
        ]);
        assert_eq!(foliage.plant(3).map(|plant| plant.tree), Some(3));

        let mut crossings = foliage.crossings(Vec3::new(0.0, 2.0, 25.0), Vec3::new(100.0, 2.0, 25.0));
        crossings.sort_by_key(|crossing| crossing.tree);
        assert_eq!(crossings.len(), 2, "{crossings:?}");
        assert_eq!(crossings[0].tree, 3);
        assert!((crossings[0].enter - 57.0).abs() < 1e-3 && (crossings[0].leave - 63.0).abs() < 1e-3);
        assert_eq!(crossings[1].tree, 7);
        assert!((crossings[1].enter - 22.0).abs() < 1e-3 && (crossings[1].leave - 28.0).abs() < 1e-3);

        // Over its top, it doesn't hide; ending in it, it does to the end.
        assert!(foliage.crossings(Vec3::new(0.0, 5.0, 25.0), Vec3::new(100.0, 5.0, 25.0)).iter().all(|crossing| crossing.tree != 7));
        let crossings = foliage.crossings(Vec3::new(0.0, 2.0, 25.0), Vec3::new(25.0, 1.0, 25.0));
        assert_eq!(crossings.len(), 1);
        assert!((crossings[0].leave - crossings[0].enter - 3.0).abs() < 0.01);

    }

}
