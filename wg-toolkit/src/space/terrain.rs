//! Terrain heights, read from a space's chunks: each `XXXXZZZZo.cdata_processed` (the
//! chunk's grid coordinates as 16-bit hex) is a zip archive whose `terrain2/heights`
//! holds the chunk's height map, as BigWorld's `TerrainHeightMap2` reads it.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};

use byteorder::{LittleEndian as LE, ReadBytesExt};

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
/// The chunk size when the space's settings don't give one.
const DEFAULT_CHUNK_SIZE: f32 = 100.0;


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

    fn get(&self, x: usize, z: usize) -> f32 {
        self.heights[z.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }

    /// The height at a position relative to the chunk's corner, on the triangles the
    /// client renders, see `TerrainHeightMap2::heightAt`.
    pub fn height_at(&self, x: f32, z: f32, chunk_size: f32) -> f32 {

        let spacing_x = chunk_size / (self.width - VISIBLE_OFFSET * 2 - 1) as f32;
        let spacing_z = chunk_size / (self.height - VISIBLE_OFFSET * 2 - 1) as f32;
        let xs = (x / spacing_x + VISIBLE_OFFSET as f32).max(0.0);
        let zs = (z / spacing_z + VISIBLE_OFFSET as f32).max(0.0);
        let (xf, zf) = (xs.fract(), zs.fract());
        let (x0, z0) = (xs as usize, zs as usize);

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


/// A space's terrain: its chunks' height maps.
#[derive(Debug, Clone)]
pub struct Terrain {
    chunk_size: f32,
    chunks: HashMap<(i16, i16), HeightMap>,
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
            let heights = HeightMap::decode(archive.read_by_name("terrain2/heights")?)
                .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", entry.path())))?;
            chunks.insert((x as i16, z as i16), heights);
        }

        Ok(Self { chunk_size, chunks })

    }

    /// The terrain's height at a world position, if it has a chunk there.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let (cx, cz) = ((x / self.chunk_size).floor(), (z / self.chunk_size).floor());
        let chunk = self.chunks.get(&(cx as i16, cz as i16))?;
        Some(chunk.height_at(x - cx * self.chunk_size, z - cz * self.chunk_size, self.chunk_size))
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

}
