//! Which parts of a map see which others, see [`SightGrid`]: the map's bounds cut in
//! cells of [`CELL`] metres, each with where a tank stands in it, how much of it a tank
//! can stand on, how many plants grow in it, and which cells within view range see it,
//! from a tank's gun's height to another's.
//!
//! ```text
//!   each cell:
//!     its point: of SAMPLES × SAMPLES points over it, where a tank stands, see
//!       World::stand, the one nearest its middle; none where it stands on none
//!     its openness: the share of those points it stands on
//!     its plants: the plants of the map's foliage within half a cell of its middle,
//!       see World::plants_near
//!   each two cells with points within the view range on the ground plane:
//!     whether they see each other, see World::sees, from EYE_HEIGHT over one's point to
//!       as high over the other's
//! ```
//!
//! Building it asks the map a few hundred thousand lines of sight, a few seconds, so it
//! is saved once built, see [`SightGrid::load_or_build`], under a name of its map, and
//! read again as long as what it was built from is the same.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::fs::File;

use glam::{Vec2, Vec3, Vec4};

use crate::base::debug::{self, DebugChannel};

use super::World;


/// How wide a cell is, in metres. Our choice: about where a tank and its neighbours
/// stand.
pub const CELL: f32 = 30.0;
/// How many points along each side of a cell are tried for a tank to stand on, and how
/// wide a tank standing there is, from its middle, see `World::stand`: a medium's half
/// width and a margin.
const SAMPLES: usize = 5;
const STAND_RADIUS: f32 = 3.0;
/// How high over the ground a tank sees from and is seen at, in metres: about a gun's
/// height. Our choice.
const EYE_HEIGHT: f32 = 2.0;
/// What a saved grid starts with, its version.
const MAGIC: &[u8; 10] = b"WGTKSIGHT1";

/// How far round a vehicle its cells are drawn, in metres, see [`SightGrid::debug_draw`],
/// and their colours: one that sees the vehicle's cell, one that doesn't, one no tank
/// stands in.
const DEBUG_RADIUS: f32 = 150.0;
const DEBUG_SEES: Vec4 = Vec4::new(255.0, 64.0, 64.0, 255.0);
const DEBUG_BLIND: Vec4 = Vec4::new(64.0, 255.0, 96.0, 255.0);
const DEBUG_CLOSED: Vec4 = Vec4::new(128.0, 128.0, 128.0, 255.0);
/// How high over a vehicle the text of its cell is drawn, in metres.
const DEBUG_TEXT_HEIGHT: f32 = 6.0;


/// Which cells of a map see which others, see the module's documentation.
#[derive(Debug, Clone)]
pub struct SightGrid {
    /// What it was built for: the corner of its first cell, the least x and z, how many
    /// cells it has along x then z, how far it sees, and its map's fingerprint, see
    /// [`SightKey`].
    key: SightKey,
    /// Each cell, row by row along z.
    cells: Vec<SightCell>,
    /// For each cell, the bits of the cells it sees, [`Self::words`] words of them.
    sees: Vec<u64>,
    /// How many words a cell's bits take.
    words: usize,
}

/// What a [`SightGrid`] was built for, which a saved one must match to be read again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SightKey {
    /// The corner of its first cell, the least x and z.
    pub origin: Vec2,
    /// How many cells it has along x, then along z.
    pub size: (usize, usize),
    /// How far its cells see, in metres on the ground plane.
    pub range: f32,
    /// How many static triangles and plants its map has: a map changed by the game
    /// makes another grid.
    pub triangles: usize,
    pub plants: usize,
}

/// A cell of a [`SightGrid`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SightCell {
    /// Where a tank stands in it, nearest its middle, none where none stands.
    pub point: Option<Vec3>,
    /// The share of it a tank stands on.
    pub open: f32,
    /// How many plants grow in it.
    pub plants: u16,
}

impl SightGrid {

    /// The grid of this map's `world` over its `bounds`, minimum then maximum, seeing
    /// `range` far: read from under `dir`, as `name`, if saved there for the same, else
    /// built, see [`Self::build`], and saved there. A grid that fails to
    /// be read or saved is built or kept all the same, the error said.
    pub fn load_or_build(
        world: &mut World,
        bounds: (Vec2, Vec2),
        range: f32,
        dir: &Path,
        name: &str,
    ) -> (Self, Result<bool, io::Error>) {

        let key = SightKey::of(world, bounds, range);
        let path = dir.join(format!("{name}.sight"));
        match Self::read(&path, key) {
            Ok(Some(grid)) => return (grid, Ok(true)),
            Ok(None) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return (Self::build(world, key), Err(e)),
        }

        let grid = Self::build(world, key);
        let saved = std::fs::create_dir_all(dir).and_then(|()| grid.write(&path));
        (grid, saved.map(|()| false))

    }

    /// Build the grid of this `key` over this map's `world`, see the module's documentation: the cells first, then the lines of sight between
    /// them, each two once, the rows dealt in turn to as many threads as there are cores.
    pub fn build(world: &mut World, key: SightKey) -> Self {

        let (columns, rows) = key.size;
        let cells: Vec<_> = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|cell| Self::build_cell(world, key.origin, cell))
            .collect();

        let count = cells.len();
        let words = count.div_ceil(64);
        let mut sees = vec![0u64; count * words];
        let world = &*world;
        let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
        std::thread::scope(|scope| {
            let built: Vec<_> = (0..threads)
                .map(|thread| {
                    let cells = &cells;
                    scope.spawn(move || {
                        (thread..count).step_by(threads)
                            .map(|from| (from, Self::build_row(world, cells, from, key.range)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for rows in built {
                for (from, seen) in rows.join().expect("a sight grid thread panicked") {
                    for to in seen {
                        sees[from * words + to / 64] |= 1 << (to % 64);
                        sees[to * words + from / 64] |= 1 << (from % 64);
                    }
                }
            }
        });

        Self {
            key,
            cells,
            sees,
            words,
        }

    }

    /// A cell of the grid, at its column and row, see the module's documentation.
    fn build_cell(world: &mut World, origin: Vec2, (column, row): (usize, usize)) -> SightCell {

        let corner = origin + Vec2::new(column as f32, row as f32) * CELL;
        let middle = corner + Vec2::splat(CELL / 2.0);
        let step = CELL / SAMPLES as f32;

        let mut stood = 0;
        let mut point: Option<Vec3> = None;
        for x in 0..SAMPLES {
            for z in 0..SAMPLES {
                let sample = corner + (Vec2::new(x as f32, z as f32) + 0.5) * step;
                let Some(at) = world.stand(Vec3::new(sample.x, 0.0, sample.y), STAND_RADIUS) else { continue };
                stood += 1;
                let nearer = point.is_none_or(|point| {
                    Vec2::new(at.x, at.z).distance(middle) < Vec2::new(point.x, point.z).distance(middle)
                });
                if nearer {
                    point = Some(at);
                }
            }
        }

        let plants = world.plants_near(Vec3::new(middle.x, 0.0, middle.y), CELL / 2.0);
        SightCell {
            point,
            open: stood as f32 / (SAMPLES * SAMPLES) as f32,
            plants: plants.min(u16::MAX as usize) as u16,
        }

    }

    /// The cells after `from` that see it, within `range`, see the module's
    /// documentation; those before it saw it as theirs were built.
    fn build_row(world: &World, cells: &[SightCell], from: usize, range: f32) -> Vec<usize> {
        let Some(eye) = cells[from].point else { return Vec::new() };
        let eye = eye + Vec3::Y * EYE_HEIGHT;
        cells.iter()
            .enumerate()
            .skip(from + 1)
            .filter_map(|(to, cell)| Some((to, cell.point? + Vec3::Y * EYE_HEIGHT)))
            .filter(|&(_, other)| (other - eye).with_y(0.0).length() <= range && world.sees(eye, other))
            .map(|(to, _)| to)
            .collect()
    }

    /// The cell a point is over, if within the grid.
    pub fn cell_of(&self, point: Vec3) -> Option<usize> {
        let cell = (Vec2::new(point.x, point.z) - self.key.origin) / CELL;
        let (columns, rows) = self.key.size;
        if cell.x < 0.0 || cell.y < 0.0 || cell.x >= columns as f32 || cell.y >= rows as f32 {
            return None;
        }
        Some(cell.y as usize * columns + cell.x as usize)
    }

    /// The cell of this index, see [`Self::cell_of`].
    pub fn cell(&self, index: usize) -> &SightCell {
        &self.cells[index]
    }

    /// Whether a tank in one cell sees one in another, as built, see the module's
    /// documentation.
    pub fn sees(&self, from: usize, to: usize) -> bool {
        self.sees[from * self.words + to / 64] & 1 << (to % 64) != 0
    }

    /// How many cells see this one.
    pub fn exposure(&self, cell: usize) -> u32 {
        self.sees[cell * self.words..(cell + 1) * self.words].iter()
            .map(|bits| bits.count_ones())
            .sum()
    }

    /// Draw the cells within [`DEBUG_RADIUS`] of a point, see [`DebugChannel::Sight`]: a
    /// cross at each one's point, red if it sees the point's cell, green if not, grey
    /// where no tank stands; and over the point, how many cells see its cell, how many
    /// plants grow in it, and how open it is.
    pub fn debug_draw(&self, at: Vec3) {

        let Some(from) = self.cell_of(at) else { return };
        let cell = self.cell(from);
        let text = format!(
            "seen by {} cells, {} plants, {:.0}% open",
            self.exposure(from),
            cell.plants,
            cell.open * 100.0,
        );
        debug::text(DebugChannel::Sight, at + Vec3::Y * DEBUG_TEXT_HEIGHT, text, DEBUG_SEES);
        let (columns, _) = self.key.size;
        let reach = (DEBUG_RADIUS / CELL).ceil() as isize;
        let (column, row) = ((from % columns) as isize, (from / columns) as isize);

        for z in row - reach..=row + reach {
            for x in column - reach..=column + reach {
                let middle = self.key.origin + (Vec2::new(x as f32, z as f32) + 0.5) * CELL;
                let Some(cell) = self.cell_of(Vec3::new(middle.x, 0.0, middle.y)) else { continue };
                let (point, color) = match self.cells[cell].point {
                    Some(point) if self.sees(cell, from) => (point, DEBUG_SEES),
                    Some(point) => (point, DEBUG_BLIND),
                    None => (Vec3::new(middle.x, at.y, middle.y), DEBUG_CLOSED),
                };
                debug::cross(DebugChannel::Sight, point + Vec3::Y * EYE_HEIGHT, color);
            }
        }

    }

    // -- Its file --

    /// Read the grid saved at this path, if saved for this `key`.
    fn read(path: &Path, key: SightKey) -> io::Result<Option<Self>> {

        let mut read = BufReader::new(File::open(path)?);
        let mut magic = [0; MAGIC.len()];
        read.read_exact(&mut magic)?;
        if &magic != MAGIC || SightKey::read(&mut read)? != key {
            return Ok(None);
        }

        let (columns, rows) = key.size;
        let count = columns * rows;
        let mut cells = Vec::with_capacity(count);
        for _ in 0..count {
            let stands = read_u8(&mut read)? != 0;
            let point = Vec3::new(read_f32(&mut read)?, read_f32(&mut read)?, read_f32(&mut read)?);
            cells.push(SightCell {
                point: stands.then_some(point),
                open: read_f32(&mut read)?,
                plants: read_u16(&mut read)?,
            });
        }

        let words = count.div_ceil(64);
        let mut sees = vec![0u64; count * words];
        for bits in &mut sees {
            let mut bytes = [0; 8];
            read.read_exact(&mut bytes)?;
            *bits = u64::from_le_bytes(bytes);
        }

        Ok(Some(Self {
            key,
            cells,
            sees,
            words,
        }))

    }

    /// Save the grid at this path.
    fn write(&self, path: &Path) -> io::Result<()> {

        let mut write = BufWriter::new(File::create(path)?);
        write.write_all(MAGIC)?;
        self.key.write(&mut write)?;

        for cell in &self.cells {
            let point = cell.point.unwrap_or(Vec3::ZERO);
            write.write_all(&[u8::from(cell.point.is_some())])?;
            for value in [point.x, point.y, point.z, cell.open] {
                write.write_all(&value.to_le_bytes())?;
            }
            write.write_all(&cell.plants.to_le_bytes())?;
        }
        for bits in &self.sees {
            write.write_all(&bits.to_le_bytes())?;
        }
        write.flush()

    }

}

impl SightKey {

    /// What a grid of this map's `world` over its `bounds`, minimum then maximum, seeing
    /// `range` far, is built for: cells covering the bounds from their least corner.
    pub fn of(world: &World, (min, max): (Vec2, Vec2), range: f32) -> Self {
        let size = ((max - min) / CELL).ceil();
        Self {
            origin: min,
            size: (size.x as usize, size.y as usize),
            range,
            triangles: world.space().statics.len(),
            plants: world.space().foliage.plants().len(),
        }
    }

    /// Read a key saved by [`Self::write`].
    fn read(read: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            origin: Vec2::new(read_f32(read)?, read_f32(read)?),
            size: (read_u32(read)? as usize, read_u32(read)? as usize),
            range: read_f32(read)?,
            triangles: read_u32(read)? as usize,
            plants: read_u32(read)? as usize,
        })
    }

    /// Save the key.
    fn write(&self, write: &mut impl Write) -> io::Result<()> {
        for value in [self.origin.x, self.origin.y] {
            write.write_all(&value.to_le_bytes())?;
        }
        for value in [self.size.0, self.size.1] {
            write.write_all(&(value as u32).to_le_bytes())?;
        }
        write.write_all(&self.range.to_le_bytes())?;
        for value in [self.triangles, self.plants] {
            write.write_all(&(value as u32).to_le_bytes())?;
        }
        Ok(())
    }

}

/// Read a little-endian value of a saved grid.
fn read_u8(read: &mut impl Read) -> io::Result<u8> {
    let mut bytes = [0; 1];
    read.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u16(read: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0; 2];
    read.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(read: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    read.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_f32(read: &mut impl Read) -> io::Result<f32> {
    let mut bytes = [0; 4];
    read.read_exact(&mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}
