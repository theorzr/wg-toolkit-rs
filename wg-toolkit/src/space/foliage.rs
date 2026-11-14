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

use std::collections::HashMap;
use std::io::{self, Read};

use glam::{Affine3A, Vec3};
use tracing::warn;

use crate::res::fs::ResFilesystem;

use super::destructibles::Destructibles;
use super::bin::{SpaceBin, BWST, SPTR};


/// How a SpeedTree 6 `.srt` file starts, a version on 16 bytes, see [`srt_bounds`].
const SRT_MAGIC: &[u8] = b"SRT 06.";


/// See the module's documentation.
#[derive(Debug, Default)]
pub struct Foliage {
    /// Its plants, in [`SPTR`]'s order.
    pub plants: Vec<Plant>,
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

        Ok(Self { plants })

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
