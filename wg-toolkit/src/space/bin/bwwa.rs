use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use glam::{Vec2, Vec3};

use crate::util::io::WgReadExt;

use super::{skip_vector, SpaceSection};


/// `BWWa`: the space's water, each body of it a flat surface, see [`WaterBody`], its
/// triangles a mesh of [`WaterVertex`] over a grid.
///
/// It holds five vectors: the bodies, 340 bytes each in its version 3 (BigWorld
/// 14.4.1's `StaticSceneWaterTypes::Water`, version 1, is a `Water::WaterState` and
/// four texture names), then, for all bodies one after the other, the tiles of their
/// grids they cover, their vertices, their triangles' indices, and bytes of unknown
/// use, which this doesn't read. A body gives where its own are in each, as ranges.
#[derive(Debug)]
pub struct BWWA {
    pub bodies: Vec<WaterBody>,
    /// The boxes of the tiles each body covers, by their least and greatest corners, see
    /// [`WaterBody::tiles`].
    pub tiles: Vec<[Vec3; 2]>,
    pub vertices: Vec<WaterVertex>,
    /// Each triangle's three vertices, by their index among their own body's, see
    /// [`WaterBody::indices`].
    pub indices: Vec<u32>,
}

/// A body of water, see [`BWWA`]: a flat surface at its height.
#[derive(Debug, Clone)]
pub struct WaterBody {
    /// The least and greatest corners of its surface's box, both at its height.
    pub bounds: [Vec3; 2],
    /// The centre of its surface as placed, `WaterState::position_`, at its height.
    pub position: Vec3,
    /// How wide and long its surface is as placed, `WaterState::size_`, in metres.
    pub size: Vec2,
    /// Its tiles in [`BWWA::tiles`].
    pub tiles: Range<usize>,
    /// Its vertices in [`BWWA::vertices`].
    pub vertices: Range<usize>,
    /// Its triangles' indices in [`BWWA::indices`].
    pub indices: Range<usize>,
}

/// A vertex of a body of water's mesh, see [`BWWA`].
#[derive(Debug, Clone, Copy)]
pub struct WaterVertex {
    /// Where it is in the world, at its body's height.
    pub position: Vec3,
    /// Its column and row in its body's grid.
    pub cell: [u32; 2],
    /// How deep the water is there, from the surface down to the terrain, 0 on the
    /// shore.
    pub depth: f32,
}

impl SpaceSection for BWWA {

    const ID: [u8; 4] = *b"BWWa";

    fn decode<R: Read + Seek>(read: &mut R) -> io::Result<Self> {

        // Its box, then its `WaterState` from `position_` on, then the ranges, each a
        // start and an end, after the hash of its depth texture, as its other textures'
        // are, see `BWST`; the rest is how it is drawn.
        let bodies = read.read_vector(|buf| {

            let bounds = [buf.read_vec3()?, buf.read_vec3()?];
            buf.seek(SeekFrom::Start(156))?;
            let position = buf.read_vec3()?;
            let size = buf.read_vec2()?;

            buf.seek(SeekFrom::Start(308))?;
            let mut ranges = [0; 6];
            for value in &mut ranges {
                *value = buf.read_u32()? as usize;
            }

            Ok(WaterBody {
                bounds,
                position,
                size,
                tiles: ranges[0]..ranges[1],
                vertices: ranges[2]..ranges[3],
                indices: ranges[4]..ranges[5],
            })

        })?;

        let tiles = read.read_vector(|buf| Ok([buf.read_vec3()?, buf.read_vec3()?]))?;
        let vertices = read.read_vector(|buf| {
            Ok(WaterVertex {
                position: buf.read_vec3()?,
                cell: [buf.read_u32()?, buf.read_u32()?],
                depth: buf.read_f32()?,
            })
        })?;
        let indices = read.read_vector(|buf| buf.read_u32())?;
        skip_vector(read)?;

        Ok(Self {
            bodies,
            tiles,
            vertices,
            indices,
        })

    }

}
