//! Terrain heights and grounds, read from a space's chunks: each
//! `XXXXZZZZo.cdata_processed` (the chunk's grid coordinates as 16-bit hex) is a zip
//! archive whose `terrain2/heights` holds the chunk's height map, as BigWorld's
//! `TerrainHeightMap2` reads it, and `terrain2/dominanttextures` its ground map.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};

use byteorder::{LittleEndian as LE, ReadBytesExt};
use flate2::read::ZlibDecoder;
use glam::{IVec2, Vec2, Vec3};

use crate::res::fs::ResFilesystem;
use crate::res::package::PackageReader;


/// `HeightMapHeader::MAGIC`, "hmp\0".
const HEIGHT_MAP_MAGIC: u32 = 0x00706d68;
/// `HeightMapHeader::VERSION_ABS_QFLOAT`: heights quantised to integers.
const HEIGHT_MAP_VERSION_ABS_QFLOAT: u32 = 4;
/// `QUANTISED_PNG_VERSION`, "qpng": the quantised heights as a PNG's RGBA pixels, each
/// pixel's bytes being a little-endian `int32`.
const QUANTISED_PNG_MAGIC: u32 = 0x71706e67;
/// The height a quantised unit stands for, millimetres.
const QUANTISATION_LEVEL: f32 = 0.001;
/// `TerrainHeightMap2::DEFAULT_VISIBLE_OFFSET`: samples around the chunk's own, shared
/// with its neighbours, for the first level of detail.
const VISIBLE_OFFSET: usize = 2;
/// `DominantTextureMap`'s magic, "mat\0", and the only version known.
const GROUND_MAP_MAGIC: u32 = 0x0074616d;
const GROUND_MAP_VERSION: u32 = 1;
/// The magic of a block BigWorld compresses, "\0piz" as bytes: an unknown 32-bit value
/// follows, then its size uncompressed, then its zlib stream.
const COMPRESSED_MAGIC: u32 = 0x7a697000;
/// The chunk size when the space's settings don't give one.
const DEFAULT_CHUNK_SIZE: f32 = 100.0;
/// How far either way the terrain's slope is measured for its normal, in metres: well
/// under its samples' spacing.
const NORMAL_STEP: f32 = 0.05;


/// A chunk's height map.
#[derive(Debug, Clone)]
pub struct HeightMap {
    width: usize,
    height: usize,
    /// Row-major heights, in meters.
    heights: Vec<f32>,
}

impl HeightMap {

    /// Decode a `terrain2/heights` file.
    pub fn decode<R: Read>(mut read: R) -> io::Result<Self> {

        if read.read_u32::<LE>()? != HEIGHT_MAP_MAGIC {
            return Err(invalid("not a height map"));
        }
        let width = read.read_u32::<LE>()? as usize;
        let height = read.read_u32::<LE>()? as usize;
        let _compression = read.read_u32::<LE>()?;
        let version = read.read_u32::<LE>()?;
        let _min_height = read.read_f32::<LE>()?;
        let _max_height = read.read_f32::<LE>()?;
        let _pad = read.read_u32::<LE>()?;
        if version != HEIGHT_MAP_VERSION_ABS_QFLOAT {
            return Err(invalid(format!("unsupported height map version {version}")));
        }
        if read.read_u32::<LE>()? != QUANTISED_PNG_MAGIC {
            return Err(invalid("height map not compressed as a quantised png"));
        }

        let mut decoder = png::Decoder::new(read);
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().map_err(invalid)?;
        let mut data = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut data).map_err(invalid)?;
        if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
            return Err(invalid("height map png is not 8-bit rgba"));
        }
        if (info.width as usize, info.height as usize) != (width, height) {
            return Err(invalid("height map png size differs from its header"));
        }

        let heights = data[..info.buffer_size()].chunks_exact(4)
            .map(|pixel| i32::from_le_bytes(pixel.try_into().unwrap()) as f32 * QUANTISATION_LEVEL)
            .collect();

        Ok(Self { width, height, heights })

    }

    /// The spacing of its samples in a chunk of this size, in metres.
    fn spacing(&self, chunk_size: f32) -> Vec2 {
        Vec2::new(
            chunk_size / (self.width - VISIBLE_OFFSET * 2 - 1) as f32,
            chunk_size / (self.height - VISIBLE_OFFSET * 2 - 1) as f32,
        )
    }

    /// The two triangles of a cell, `x` and `z` its first sample's, in a chunk of this
    /// size whose corner is at `corner`, as the client renders them and
    /// [`Self::height_at`] reads them: neighbouring cells have opposite diagonals. Each is
    /// wound counter-clockwise seen from above, its normal up.
    fn cell_triangles(&self, x: usize, z: usize, chunk_size: f32, corner: Vec2) -> [[Vec3; 3]; 2] {
        let spacing = self.spacing(chunk_size);
        let at = |x: usize, z: usize| {
            let flat = corner + Vec2::new(x as f32 - VISIBLE_OFFSET as f32, z as f32 - VISIBLE_OFFSET as f32) * spacing;
            Vec3::new(flat.x, self.get(x, z), flat.y)
        };
        let h00 = at(x, z);
        let h10 = at(x + 1, z);
        let h01 = at(x, z + 1);
        let h11 = at(x + 1, z + 1);
        if (x ^ z) & 1 != 0 {
            [[h00, h01, h10], [h11, h10, h01]]
        } else {
            [[h00, h11, h10], [h00, h01, h11]]
        }
    }

    fn get(&self, x: usize, z: usize) -> f32 {
        self.heights[z.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }

    /// The height at a position relative to the chunk's corner, on the triangles the
    /// client renders, see `TerrainHeightMap2::heightAt`.
    pub fn height_at(&self, x: f32, z: f32, chunk_size: f32) -> f32 {

        let spacing = self.spacing(chunk_size);
        let xs = (x / spacing.x + VISIBLE_OFFSET as f32).max(0.0);
        let zs = (z / spacing.y + VISIBLE_OFFSET as f32).max(0.0);
        let xf = xs.fract();
        let zf = zs.fract();
        let x0 = xs as usize;
        let z0 = zs as usize;

        // Neighbouring cells have opposite diagonals.
        if (x0 ^ z0) & 1 != 0 {
            let h01 = self.get(x0, z0 + 1);
            let h10 = self.get(x0 + 1, z0);
            if 1.0 - xf > zf {
                let h00 = self.get(x0, z0);
                h00 + (h10 - h00) * xf + (h01 - h00) * zf
            } else {
                let h11 = self.get(x0 + 1, z0 + 1);
                h11 + (h01 - h11) * (1.0 - xf) + (h10 - h11) * (1.0 - zf)
            }
        } else {
            let h00 = self.get(x0, z0);
            let h11 = self.get(x0 + 1, z0 + 1);
            if xf > zf {
                let h10 = self.get(x0 + 1, z0);
                h10 + (h00 - h10) * (1.0 - xf) + (h11 - h10) * zf
            } else {
                let h01 = self.get(x0, z0 + 1);
                h01 + (h11 - h01) * xf + (h00 - h01) * (1.0 - zf)
            }
        }

    }

}


/// What ground a chunk's cells are, which World of Tanks' physics drives on: its
/// dominant texture map, whose cells give in their low bits the texture covering them
/// most, and in their high 4 bits their ground. Its rows go along Z, as the height map's.
#[derive(Debug, Clone)]
pub struct GroundMap {
    width: usize,
    height: usize,
    /// Row-major grounds.
    grounds: Vec<Ground>,
}

/// A ground, as World of Tanks paints it on the terrain, `GROUND_TYPE_BY_NAME` in its
/// `constants.py`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ground {
    /// Unpainted, mostly beyond the battle's bounds.
    None,
    Firm,
    Medium,
    Soft,
}

impl GroundMap {

    /// Decode a `terrain2/dominanttextures` file.
    pub fn decode<R: Read>(mut read: R) -> io::Result<Self> {

        if read.read_u32::<LE>()? != GROUND_MAP_MAGIC {
            return Err(invalid("not a dominant texture map"));
        }
        let version = read.read_u32::<LE>()?;
        if version != GROUND_MAP_VERSION {
            return Err(invalid(format!("unsupported dominant texture map version {version}")));
        }
        let textures = read.read_u32::<LE>()? as u64;
        let name_size = read.read_u32::<LE>()? as u64;
        let width = read.read_u32::<LE>()? as usize;
        let height = read.read_u32::<LE>()? as usize;
        let _pad = read.read_u64::<LE>()?;
        // The textures' names, which only the client draws.
        io::copy(&mut (&mut read).take(textures * name_size), &mut io::sink())?;

        if read.read_u32::<LE>()? != COMPRESSED_MAGIC {
            return Err(invalid("dominant textures not compressed"));
        }
        let _unknown = read.read_u32::<LE>()?;
        let size = read.read_u32::<LE>()? as usize;
        if size != width * height {
            return Err(invalid(format!("{size} dominant textures for {width}x{height} cells")));
        }
        let mut cells = Vec::with_capacity(size);
        ZlibDecoder::new(read).read_to_end(&mut cells)?;
        if cells.len() != size {
            return Err(invalid("dominant textures shorter than their size"));
        }

        let grounds = cells.into_iter()
            .map(|cell| match cell >> 4 {
                1 => Ground::Firm,
                2 => Ground::Medium,
                3 => Ground::Soft,
                _ => Ground::None,
            })
            .collect();

        Ok(Self { width, height, grounds })

    }

    /// The ground at a position relative to the chunk's corner.
    pub fn ground_at(&self, x: f32, z: f32, chunk_size: f32) -> Ground {
        let column = ((x / chunk_size * self.width as f32).max(0.0) as usize).min(self.width - 1);
        let row = ((z / chunk_size * self.height as f32).max(0.0) as usize).min(self.height - 1);
        self.grounds[row * self.width + column]
    }

}


/// A chunk of terrain: its heights, and its grounds if it has them.
#[derive(Debug, Clone)]
struct Chunk {
    heights: HeightMap,
    grounds: Option<GroundMap>,
}

/// A space's terrain: its chunks' height and ground maps.
#[derive(Debug, Clone)]
pub struct Terrain {
    chunk_size: f32,
    chunks: HashMap<(i16, i16), Chunk>,
}

impl Default for Terrain {

    /// No terrain: no height or ground anywhere.
    fn default() -> Self {
        Self { chunk_size: DEFAULT_CHUNK_SIZE, chunks: HashMap::new() }
    }

}

impl Terrain {

    /// Load the terrain of a space, `spaces/08_ruinberg` say.
    pub fn load(fs: &ResFilesystem, space_dir: &str) -> io::Result<Self> {

        let chunk_size = crate::pxml::from_reader(fs.read(format!("{space_dir}/space.settings")).map(io::BufReader::new)?)
            .ok()
            .and_then(|settings| settings.get_child("chunkSize")?.as_float())
            .unwrap_or(DEFAULT_CHUNK_SIZE);

        let mut chunks = HashMap::new();
        for entry in fs.read_dir(space_dir)? {
            let entry = entry?;
            let Some(coords) = entry.name().strip_suffix("o.cdata_processed") else { continue };
            if coords.len() != 8 {
                continue;
            }
            let (Ok(x), Ok(z)) = (u16::from_str_radix(&coords[..4], 16), u16::from_str_radix(&coords[4..], 16)) else {
                continue;
            };

            let mut data = Vec::new();
            fs.read(entry.path())?.read_to_end(&mut data)?;
            let mut archive = PackageReader::new(Cursor::new(data))?;
            let context = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", entry.path()));
            let heights = HeightMap::decode(archive.read_by_name("terrain2/heights")?).map_err(context)?;
            let grounds = match archive.read_by_name("terrain2/dominanttextures") {
                Ok(file) => Some(GroundMap::decode(file).map_err(context)?),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(context(e)),
            };
            chunks.insert((x as i16, z as i16), Chunk { heights, grounds });
        }

        Ok(Self { chunk_size, chunks })

    }

    /// The terrain's height at a world position, if it has a chunk there.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let (chunk, x, z) = self.chunk_at(x, z)?;
        Some(chunk.heights.height_at(x, z, self.chunk_size))
    }

    /// The ground at a world position, if it has a chunk there with its grounds.
    pub fn ground_at(&self, x: f32, z: f32) -> Option<Ground> {
        let (chunk, x, z) = self.chunk_at(x, z)?;
        Some(chunk.grounds.as_ref()?.ground_at(x, z, self.chunk_size))
    }

    /// The triangles of its cells that a box of the ground plane overlaps, minimum then
    /// maximum, each once, as the client renders them: wound counter-clockwise seen from
    /// above, their normals up. None where it has no chunk. Read from its heights as
    /// they are asked for, never kept: what the space's contact queries walk, see
    /// "Contacts" in the `space` module's documentation.
    pub(crate) fn triangles_in(&self, min: Vec2, max: Vec2, mut each: impl FnMut([Vec3; 3])) {
        let first = (min / self.chunk_size).floor();
        let last = (max / self.chunk_size).floor();
        for cx in first.x as i32..=last.x as i32 {
            for cz in first.y as i32..=last.y as i32 {
                let Some(chunk) = self.chunks.get(&(cx as i16, cz as i16)) else { continue };
                let heights = &chunk.heights;
                let corner = Vec2::new(cx as f32, cz as f32) * self.chunk_size;
                let spacing = heights.spacing(self.chunk_size);
                // Its own cells only, not the border it shares with its neighbours.
                let cells = IVec2::new(
                    (heights.width - VISIBLE_OFFSET * 2 - 1) as i32,
                    (heights.height - VISIBLE_OFFSET * 2 - 1) as i32,
                );
                let low = ((min - corner) / spacing).floor().as_ivec2().max(IVec2::ZERO);
                let high = ((max - corner) / spacing).floor().as_ivec2().min(cells - 1);
                for x in low.x..=high.x {
                    for z in low.y..=high.y {
                        let x = x as usize + VISIBLE_OFFSET;
                        let z = z as usize + VISIBLE_OFFSET;
                        let cell = heights.cell_triangles(x, z, self.chunk_size, corner);
                        for triangle in cell {
                            each(triangle);
                        }
                    }
                }
            }
        }
    }

    /// A terrain of one chunk at the origin, 100 m, flat at this height: for tests.
    #[cfg(test)]
    pub(crate) fn flat(height: f32) -> Self {
        let size = 64 + VISIBLE_OFFSET * 2 + 1;
        let chunk = Chunk {
            heights: HeightMap {
                width: size,
                height: size,
                heights: vec![height; size * size],
            },
            grounds: None,
        };
        Self {
            chunk_size: 100.0,
            chunks: HashMap::from([((0, 0), chunk)]),
        }
    }

    /// The chunk at a world position, and the position relative to its corner.
    fn chunk_at(&self, x: f32, z: f32) -> Option<(&Chunk, f32, f32)> {
        let cx = (x / self.chunk_size).floor();
        let cz = (z / self.chunk_size).floor();
        let chunk = self.chunks.get(&(cx as i16, cz as i16))?;
        Some((chunk, x - cx * self.chunk_size, z - cz * self.chunk_size))
    }

    /// The terrain's normal at a world position, upward, if it has a chunk there: its
    /// slope over [`NORMAL_STEP`] either way, which is its triangle's away from their
    /// edges.
    pub fn normal_at(&self, x: f32, z: f32) -> Option<Vec3> {
        let height = |x, z| self.height_at(x, z);
        let center = height(x, z)?;
        let east = height(x + NORMAL_STEP, z).unwrap_or(center);
        let west = height(x - NORMAL_STEP, z).unwrap_or(center);
        let north = height(x, z + NORMAL_STEP).unwrap_or(center);
        let south = height(x, z - NORMAL_STEP).unwrap_or(center);
        Some(Vec3::new(west - east, 2.0 * NORMAL_STEP, south - north).normalize())
    }

}


fn invalid<E: Into<Box<dyn std::error::Error + Send + Sync>>>(error: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn height_follows_samples() {
        // A 64-block chunk with its 2-sample border, rising 1 m per sample along x.
        let size = 64 + VISIBLE_OFFSET * 2 + 1;
        let heights = (0..size * size).map(|i| (i % size) as f32).collect();
        let map = HeightMap { width: size, height: size, heights };
        let spacing = 100.0 / 64.0;
        // The chunk's corner is the first sample past the border.
        assert_eq!(map.height_at(0.0, 0.0, 100.0), VISIBLE_OFFSET as f32);
        assert!((map.height_at(spacing * 1.5, spacing * 0.25, 100.0) - (VISIBLE_OFFSET as f32 + 1.5)).abs() < 1e-4);
        assert!((map.height_at(100.0, 50.0, 100.0) - (VISIBLE_OFFSET as f32 + 64.0)).abs() < 1e-3);
    }

    #[test]
    fn ground_maps_cells_along_z() {

        use std::io::Write;
        use byteorder::WriteBytesExt;

        // Two textures, 2x2 cells: firm then soft on the first row, medium then none on
        // the second, along Z.
        let mut file = Vec::new();
        for value in [GROUND_MAP_MAGIC, GROUND_MAP_VERSION, 2, 8, 2, 2, 0, 0] {
            file.write_u32::<LE>(value).unwrap();
        }
        file.extend_from_slice(b"grass\0\0\0sand\0\0\0\0");
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[0x10, 0x31, 0x20, 0x01]).unwrap();
        for value in [COMPRESSED_MAGIC, 0, 4] {
            file.write_u32::<LE>(value).unwrap();
        }
        file.extend(encoder.finish().unwrap());

        let map = GroundMap::decode(&file[..]).unwrap();
        assert_eq!(map.ground_at(10.0, 10.0, 100.0), Ground::Firm);
        assert_eq!(map.ground_at(60.0, 10.0, 100.0), Ground::Soft);
        assert_eq!(map.ground_at(10.0, 60.0, 100.0), Ground::Medium);
        assert_eq!(map.ground_at(100.0, 100.0, 100.0), Ground::None);

    }

    #[test]
    fn cells_are_the_heights_triangles() {
        // Rising 1 m per sample along x, and 0.5 m along z.
        let size = 64 + VISIBLE_OFFSET * 2 + 1;
        let heights = (0..size * size).map(|i| (i % size) as f32 + (i / size) as f32 * 0.5).collect();
        let chunk = Chunk { heights: HeightMap { width: size, height: size, heights }, grounds: None };
        let terrain = Terrain { chunk_size: 100.0, chunks: HashMap::from([((0, 0), chunk)]) };
        // A box of 3 by 2 metres covers 3 by 2 cells of 1.5625 m, 2 triangles each, each
        // up, and on the heights: their middles are where the terrain is.
        let mut triangles = Vec::new();
        terrain.triangles_in(Vec2::new(30.0, 40.0), Vec2::new(33.0, 42.0), |triangle| triangles.push(triangle));
        assert_eq!(triangles.len(), 3 * 2 * 2);
        for [a, b, c] in triangles {
            assert!((b - a).cross(c - a).y > 0.0);
            let middle = (a + b + c) / 3.0;
            assert!((terrain.height_at(middle.x, middle.z).unwrap() - middle.y).abs() < 1e-3, "{middle}");
        }
        // None out of its chunks.
        let mut count = 0;
        terrain.triangles_in(Vec2::splat(-50.0), Vec2::splat(-10.0), |_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn normal_leans_against_the_slope() {
        // Rising 1 m per sample along x, as above: a 1 in 1.5625 slope.
        let size = 64 + VISIBLE_OFFSET * 2 + 1;
        let heights = (0..size * size).map(|i| (i % size) as f32).collect();
        let chunk = Chunk { heights: HeightMap { width: size, height: size, heights }, grounds: None };
        let terrain = Terrain { chunk_size: 100.0, chunks: HashMap::from([((0, 0), chunk)]) };
        let normal = terrain.normal_at(30.3, 40.7).unwrap();
        let expected = Vec3::new(-1.0, 100.0 / 64.0, 0.0).normalize();
        assert!(normal.distance(expected) < 1e-3, "{normal}");
    }

}
