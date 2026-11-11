//! Where a tank can drive on a map, and its way there ([`NavGrid`]): a grid over the
//! battle's area, each cell asked of the space the first time a way looks at it, which
//! bots find their way by, see [`World::path`](super::World::path).
//!
//! A cell's height is the top of what covers it, seen from the sky: the terrain, or a
//! static model over it, a house's roof say, of what stops a tank, as `body::blocking`
//! says: what the game lets vehicles through, and destructibles that break or fall as
//! they are driven into, a fence or a lamp post, aren't; a house's walls standing are. A
//! tank can't enter a cell too steep, or a step too high from a neighbour: a house stands
//! out of the ground as a block of such steps, a wall as a ridge of them. Nor can it
//! leave the grid.
//!
//! ```text
//!   seen from the side, one height per cell:
//!
//!               ┌──────┐              a house: its roof, steps up and down at its walls
//!     ____      │      │       ___/   a slope: steps small enough to climb
//!         \_____│      │______/
//!     cells   x  x  x  # # # #  x  x  x   x passable, # not: a step or too steep
//! ```
//!
//! Each cell's clearance is how far it is from the nearest cell a tank can't enter, up to
//! [`MAX_CLEARANCE`]: a tank fits where its clearance is over its half width, so a way
//! keeps it off walls and out of gaps narrower than it.
//!
//! What it learnt holds until something breaks: the world has it forget all, and it asks
//! the space again as ways look, see [`NavGrid::forget`].

use std::collections::BinaryHeap;
use std::f32::consts::PI;
use std::cmp::Ordering;

use glam::{Vec2, Vec3};

use wgtk::space::statics::StaticTriangle;
use wgtk::space::Space;


/// A cell's side, in metres. Our choice: fine enough that a wall's thickness, from about
/// half a metre, isn't missed between two cells.
pub const CELL: f32 = 1.0;
/// How steep a cell's surface may be for a tank to drive on it, in radians. Our choice,
/// below where its tracks' grip falls, from 27.5°, see `body::SLOPE_GRIP_ALONG`.
const MAX_SLOPE: f32 = 30.0 * PI / 180.0;
/// How much higher or lower a neighbouring cell may be for a tank to drive onto it, in
/// metres: as steep as [`MAX_SLOPE`] allows, over a cell, and a curb's height more.
const MAX_STEP: f32 = 0.6;
/// How high above any surface the grid's rays start, in metres.
const SKY: f32 = 1000.0;
/// The most clearance a cell is given, in metres: past it, it makes no difference to a
/// way, see [`WALL_MARGIN`].
const MAX_CLEARANCE: f32 = 8.0;
/// How far off walls a way keeps, past a tank's half width, where it can, in metres. Our
/// choice: a hull drifting a little off its way doesn't scrape along them.
const WALL_MARGIN: f32 = 3.0;
/// How much more a step costs at a tank's half width from a wall than [`WALL_MARGIN`]
/// further off. Our choice.
const WALL_COST: f32 = 2.0;


/// See the module's documentation.
#[derive(Debug, Clone)]
pub struct NavGrid {
    /// The corner of its first cell, the least x and z.
    origin: Vec2,
    /// How many cells it has along x, then along z.
    size: (usize, usize),
    /// Each cell's height, NaN where nothing is under it, row by row along z; none where
    /// not asked yet.
    heights: Vec<Option<f32>>,
    /// Whether each cell is too steep to drive on, likewise.
    steep: Vec<bool>,
    /// Whether a tank can't enter each cell, none where not known yet.
    blocked: Vec<Option<bool>>,
    /// Each cell's clearance, in metres, none where not known yet.
    clearance: Vec<Option<f32>>,
    /// The cells around a cell within [`MAX_CLEARANCE`], by their offset and distance,
    /// nearest first: where its clearance looks.
    around: Vec<(isize, isize, f32)>,
}

impl NavGrid {

    /// A grid over the battle's bounds, minimum then maximum, nothing asked yet.
    pub fn new((min, max): (Vec2, Vec2)) -> Self {
        let size = (((max.x - min.x) / CELL).ceil() as usize, ((max.y - min.y) / CELL).ceil() as usize);
        let reach = (MAX_CLEARANCE / CELL).ceil() as isize;
        let mut around = (-reach..=reach)
            .flat_map(|dz| (-reach..=reach).map(move |dx| (dx, dz, (dx as f32).hypot(dz as f32) * CELL)))
            .filter(|&(_, _, distance)| distance <= MAX_CLEARANCE)
            .collect::<Vec<_>>();
        around.sort_by(|a, b| a.2.total_cmp(&b.2));
        let cells = size.0 * size.1;
        Self {
            origin: min,
            size,
            heights: vec![None; cells],
            steep: vec![false; cells],
            blocked: vec![None; cells],
            clearance: vec![None; cells],
            around,
        }
    }

    /// Forget all it asked the space: something broke.
    pub fn forget(&mut self) {
        self.heights.fill(None);
        self.blocked.fill(None);
        self.clearance.fill(None);
    }

    /// A way for a tank `radius` wide from one point to another, or to as near it as it
    /// can get, over `space` of what `keep` says stops it: the points it turns at, then
    /// its end, see the module's documentation. None if it starts off the grid. The way
    /// keeps [`WALL_MARGIN`] off walls where it can, and leaves a cell too narrow for it,
    /// where it starts, the shortest way.
    ///
    /// The shortest way over the grid, by A* from cell to cell and to their diagonals,
    /// each step costing its length, more near walls; then pulled straight: from each
    /// point, on along the way as far as the line there keeps as far off walls as the way
    /// did, see [`Self::clear`].
    pub fn path(&mut self, from: Vec3, to: Vec3, radius: f32, space: &Space, keep: &impl Fn(&StaticTriangle) -> bool) -> Option<Vec<Vec3>> {

        let start = self.cell_of(from)?;
        let goal = self.nearest_cell(to);
        let (columns, rows) = self.size;
        let index = |(column, row): (usize, usize)| row * columns + column;

        // 1. A*: each cell's cost from the start, and the cell it was reached from; the
        //    cell nearest the goal reached so far, where the way ends if it can't get
        //    there. Leaving a cell too narrow, where it starts, it may cross others.
        let mut cost = vec![f32::INFINITY; columns * rows];
        let mut from_cell = vec![u32::MAX; columns * rows];
        let mut open = BinaryHeap::new();
        cost[index(start)] = 0.0;
        open.push(Open {
            estimate: self.distance(start, goal),
            cell: start,
        });
        let mut nearest = (self.distance(start, goal), start);
        let squeezed = self.clearance(start, space, keep) < radius;
        while let Some(Open { estimate, cell }) = open.pop() {
            if cell == goal {
                nearest = (0.0, cell);
                break;
            }
            let here = cost[index(cell)];
            if estimate > here + self.distance(cell, goal) + 1e-3 {
                continue;
            }
            for (dx, dz) in NEIGHBOURS {
                let (Some(column), Some(row)) = (cell.0.checked_add_signed(dx), cell.1.checked_add_signed(dz)) else { continue };
                if column >= columns || row >= rows {
                    continue;
                }
                let next = (column, row);
                let clearance = self.clearance(next, space, keep);
                let escaping = squeezed && clearance > 0.0 && self.distance(start, next) < radius;
                if clearance < radius && !escaping {
                    continue;
                }
                let near_wall = (radius + WALL_MARGIN - clearance).max(0.0) / WALL_MARGIN;
                let through = here + self.distance(cell, next) * (1.0 + WALL_COST * near_wall);
                if through >= cost[index(next)] {
                    continue;
                }
                cost[index(next)] = through;
                from_cell[index(next)] = index(cell) as u32;
                let left = self.distance(next, goal);
                if left < nearest.0 {
                    nearest = (left, next);
                }
                open.push(Open {
                    estimate: through + left,
                    cell: next,
                });
            }
        }

        // 2. Its cells, from its end back to its start.
        let mut cells = vec![nearest.1];
        while let Some(&last) = cells.last() {
            let previous = from_cell[index(last)];
            if previous == u32::MAX {
                break;
            }
            cells.push((previous as usize % columns, previous as usize / columns));
        }
        cells.reverse();

        // 3. Pulled straight: from each point, on along the way as long as the line
        //    there keeps as far off walls as the way did, up to [`WALL_MARGIN`].
        let mut points = Vec::new();
        let mut at = 0;
        while at + 1 < cells.len() {
            let mut furthest = at + 1;
            let mut kept = (radius + WALL_MARGIN).min(self.clearance(cells[at], space, keep));
            for next in at + 1..cells.len() {
                kept = kept.min(self.clearance(cells[next], space, keep));
                if !self.clear(cells[at], cells[next], kept.max(radius), space, keep) {
                    break;
                }
                furthest = next;
            }
            points.push(self.center_of(cells[furthest]));
            at = furthest;
        }
        Some(points)

    }

    /// Whether a tank goes straight from one cell to another without leaving those of
    /// `clearance` at least: every cell on the line between them, at half a cell's steps.
    fn clear(&mut self, from: (usize, usize), to: (usize, usize), clearance: f32, space: &Space, keep: &impl Fn(&StaticTriangle) -> bool) -> bool {
        let (from, to) = (cell_vec(from), cell_vec(to));
        let steps = (from.distance(to) * 2.0).ceil().max(1.0) as usize;
        (0..=steps).all(|step| {
            let point = from.lerp(to, step as f32 / steps as f32).round();
            self.clearance((point.x as usize, point.y as usize), space, keep) >= clearance
        })
    }

    /// A cell's clearance, in metres, see the module's documentation: the distance to the
    /// nearest cell around it a tank can't enter, else [`MAX_CLEARANCE`].
    fn clearance(&mut self, cell: (usize, usize), space: &Space, keep: &impl Fn(&StaticTriangle) -> bool) -> f32 {
        let index = cell.1 * self.size.0 + cell.0;
        if let Some(clearance) = self.clearance[index] {
            return clearance;
        }
        let mut clearance = MAX_CLEARANCE;
        for at in 0..self.around.len() {
            let (dx, dz, distance) = self.around[at];
            let near = match (cell.0.checked_add_signed(dx), cell.1.checked_add_signed(dz)) {
                (Some(column), Some(row)) if column < self.size.0 && row < self.size.1 => Some((column, row)),
                _ => None,
            };
            if near.is_none_or(|near| self.blocked(near, space, keep)) {
                clearance = distance;
                break;
            }
        }
        self.clearance[index] = Some(clearance);
        clearance
    }

    /// Whether a tank can't enter a cell: nothing under it, too steep, or a step too high
    /// from a neighbour, either side of it; or on the grid's edge.
    fn blocked(&mut self, cell: (usize, usize), space: &Space, keep: &impl Fn(&StaticTriangle) -> bool) -> bool {
        let index = cell.1 * self.size.0 + cell.0;
        if let Some(blocked) = self.blocked[index] {
            return blocked;
        }
        let (columns, rows) = self.size;
        let height = self.height(cell, space, keep);
        let blocked = height.is_nan()
            || self.steep[index]
            || cell.0 == 0 || cell.1 == 0 || cell.0 + 1 == columns || cell.1 + 1 == rows
            || [(cell.0 + 1, cell.1), (cell.0 - 1, cell.1), (cell.0, cell.1 + 1), (cell.0, cell.1 - 1)].into_iter()
                .any(|next| !((self.height(next, space, keep) - height).abs() <= MAX_STEP));
        self.blocked[index] = Some(blocked);
        blocked
    }

    /// A cell's height, NaN where nothing is under it, asked of the space the first
    /// time: what is on top of it, seen from the sky, and whether it is too steep.
    fn height(&mut self, (column, row): (usize, usize), space: &Space, keep: &impl Fn(&StaticTriangle) -> bool) -> f32 {
        let index = row * self.size.0 + column;
        if let Some(height) = self.heights[index] {
            return height;
        }
        let center = self.origin + (cell_vec((column, row)) + 0.5) * CELL;
        let hit = space.ray(Vec3::new(center.x, SKY, center.y), Vec3::NEG_Y, 2.0 * SKY, keep);
        let height = hit.as_ref().map_or(f32::NAN, |hit| hit.point.y);
        self.steep[index] = hit.is_some_and(|hit| hit.normal.y < MAX_SLOPE.cos());
        self.heights[index] = Some(height);
        height
    }

    /// The cell a point is over, its column and row, if within the grid.
    fn cell_of(&self, point: Vec3) -> Option<(usize, usize)> {
        let cell = (Vec2::new(point.x, point.z) - self.origin) / CELL;
        if cell.x < 0.0 || cell.y < 0.0 || cell.x >= self.size.0 as f32 || cell.y >= self.size.1 as f32 {
            return None;
        }
        Some((cell.x as usize, cell.y as usize))
    }

    /// The cell of the grid nearest a point, within it.
    fn nearest_cell(&self, point: Vec3) -> (usize, usize) {
        let cell = (Vec2::new(point.x, point.z) - self.origin) / CELL;
        (
            (cell.x.max(0.0) as usize).min(self.size.0 - 1),
            (cell.y.max(0.0) as usize).min(self.size.1 - 1),
        )
    }

    /// The middle of a cell, on its surface, as last asked.
    fn center_of(&self, (column, row): (usize, usize)) -> Vec3 {
        let center = self.origin + (cell_vec((column, row)) + 0.5) * CELL;
        let height = self.heights[row * self.size.0 + column].unwrap_or(f32::NAN);
        Vec3::new(center.x, height, center.y)
    }

    /// The distance between two cells' middles, in metres.
    fn distance(&self, a: (usize, usize), b: (usize, usize)) -> f32 {
        cell_vec(a).distance(cell_vec(b)) * CELL
    }

}

/// A cell, as a point of the grid's plane, in cells.
fn cell_vec((column, row): (usize, usize)) -> Vec2 {
    Vec2::new(column as f32, row as f32)
}

/// The cells next to a cell, by their offsets: its sides and its corners.
const NEIGHBOURS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// A cell to look from in [`NavGrid::path`], by the cost of the way through it to the goal
/// as estimated, least first.
#[derive(Debug, Clone, Copy)]
struct Open {
    estimate: f32,
    cell: (usize, usize),
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.estimate == other.estimate
    }
}

impl Eq for Open {}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.estimate.total_cmp(&self.estimate)
    }
}


#[cfg(test)]
mod tests {

    use std::time::Instant;

    use wgtk::res::fs::ResFilesystem;

    use crate::game::arena_defs::{self, ArenaTypeDef, Spawn, STANDARD_GAMEPLAY};
    use crate::game::item_defs;

    use super::super::body::blocking;
    use super::super::Rubble;
    use super::*;

    #[test]
    fn finds_its_way_across_malinovka() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let res = ResFilesystem::new(res).unwrap();
        let space = Space::load(&res, "spaces/02_malinovka", STANDARD_GAMEPLAY).unwrap();
        let bounds = arena_defs::read_bounds(&*item_defs::read_xml(&res, "scripts/arena_defs/02_malinovka.xml").unwrap(), "02_malinovka").unwrap();
        let arena = ArenaTypeDef::load(&res, "02_malinovka").unwrap();
        let [from, to] = arena.spawns.map(|spawn| match spawn {
            Spawn::Point(point) | Spawn::Base(point) => Vec3::new(point.x, 0.0, point.y),
        });
        let rubble = Rubble::default();
        let keep = blocking(&space, &rubble);
        let mut grid = NavGrid::new(bounds);
        let start = Instant::now();
        let way = grid.path(from, to, 2.0, &space, &keep).unwrap();
        println!("Found a way of {} points in {:?}", way.len(), start.elapsed());
        // It ends at the other spawn, and keeps its width off walls all the way.
        assert!((*way.last().unwrap() - to).with_y(0.0).length() < 2.0);
        let mut previous = from;
        for &point in &way {
            for step in 0..=100 {
                let cell = grid.cell_of(previous.lerp(point, step as f32 / 100.0)).unwrap();
                assert!(grid.clearance(cell, &space, &keep) >= 2.0);
            }
            previous = point;
        }
    }

}
