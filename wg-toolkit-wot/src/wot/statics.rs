//! A space's static models, what its `space.bin` places: each placed model of
//! `BSMI` is one of `BSMO`'s, whose collision, when it has one, is a Havok tagfile read
//! as a vehicle's [`CollisionModel`] is. Models without one are decor nothing collides
//! with, fallen branches or bricks.

use std::collections::HashMap;
use std::sync::Arc;
use std::io;

use glam::{Vec2, Vec3};
use tracing::warn;

use wgtk::space::section::{BSMI, BSMO, BWST};
use wgtk::space::CompiledSpace;
use wgtk::res::fs::ResFilesystem;

use super::collision::CollisionModel;


/// The side of the grid's square cells the triangles are indexed in, in metres: about
/// a hull's length.
const CELL: f32 = 8.0;


/// The triangles of a space's static models, in the world.
#[derive(Debug, Default)]
pub struct StaticScene {
    triangles: Vec<StaticTriangle>,
    /// The triangles whose bounds overlap each cell, by index.
    cells: HashMap<(i32, i32), Vec<u32>>,
}

/// A triangle of a placed static model.
#[derive(Debug, Clone, Copy)]
pub struct StaticTriangle {
    pub vertices: [Vec3; 3],
    /// Which placed model it is of, its index in `BSMI`.
    pub instance: u32,
    /// Its group's material kind, see [`super::collision::ArmorGroup`].
    pub material_kind: u32,
}

impl StaticScene {

    /// Load the static models of a space, `spaces/02_malinovka` say. A model whose
    /// collision fails to load is left out, and logged.
    pub fn load(fs: &ResFilesystem, space_dir: &str) -> io::Result<Self> {

        let path = format!("{space_dir}/space.bin");
        let file = fs.read(&path).map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))?;
        let mut space = CompiledSpace::new(io::BufReader::new(file))?;
        let missing = |section: &str| io::Error::new(io::ErrorKind::InvalidData, format!("{path}: no {section} section"));
        let strings: BWST = space.decode_section().ok_or_else(|| missing("BWST"))?;
        let instances: BSMI = space.decode_section().ok_or_else(|| missing("BSMI"))?;
        let models: BSMO = space.decode_section().ok_or_else(|| missing("BSMO"))?;

        // Each model's collision, loaded once for all its placements.
        let mut collisions: HashMap<u32, Option<Arc<CollisionModel>>> = HashMap::new();
        let mut scene = Self::default();
        for (index, instance) in instances.instances.iter().enumerate() {
            let Some(havok) = models.models.get(instance.model as usize).and_then(|model| model.havok_fnv) else { continue };
            let collision = collisions.entry(havok).or_insert_with(|| {
                let Some(path) = strings.get_string(havok) else {
                    warn!(space = space_dir, hash = havok, "Failed to find a static model's collision path");
                    return None;
                };
                CollisionModel::load(fs, path)
                    .inspect_err(|e| warn!(space = space_dir, path, "Failed to load a static model's collision: {e}"))
                    .ok()
                    .map(Arc::new)
            });
            let Some(collision) = collision else { continue };
            for group in &collision.groups {
                for triangle in &group.triangles {
                    scene.triangles.push(StaticTriangle {
                        vertices: triangle.map(|vertex| instance.transform.transform_point3(vertex)),
                        instance: index as u32,
                        material_kind: group.material_kind,
                    });
                }
            }
        }

        for (index, triangle) in scene.triangles.iter().enumerate() {
            let [min, max] = bounds_xz(&triangle.vertices);
            for cell in cells(min, max) {
                scene.cells.entry(cell).or_default().push(index as u32);
            }
        }

        Ok(scene)

    }

    /// How many triangles it has.
    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The triangles that may overlap a rectangle of the ground plane, its x and z
    /// least and most: those of the cells it overlaps, each once.
    pub fn triangles_near(&self, min: Vec2, max: Vec2) -> impl Iterator<Item = &StaticTriangle> {
        let mut indices: Vec<u32> = cells(min, max)
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices.into_iter().map(|index| &self.triangles[index as usize])
    }

}


/// A triangle's bounds on the ground plane, x and z.
fn bounds_xz(vertices: &[Vec3; 3]) -> [Vec2; 2] {
    let xz = vertices.map(|vertex| Vec2::new(vertex.x, vertex.z));
    [xz[0].min(xz[1]).min(xz[2]), xz[0].max(xz[1]).max(xz[2])]
}

/// The grid's cells a rectangle of the ground plane overlaps.
fn cells(min: Vec2, max: Vec2) -> impl Iterator<Item = (i32, i32)> {
    let (min, max) = ((min / CELL).floor().as_ivec2(), (max / CELL).floor().as_ivec2());
    (min.x..=max.x).flat_map(move |x| (min.y..=max.y).map(move |z| (x, z)))
}


#[cfg(test)]
mod tests {

    use super::*;

    /// With the game's `res` directory in `WGT_RES_PATH`: Malinovka's placed collision,
    /// as many triangles as the client's models give, and a hangar of Graf Zeppelin's
    /// story map where a captured drive hits it, 2.8 m ahead of the hull's centre.
    #[test]
    fn real_static_scenes() {
        let Ok(res) = std::env::var("WGT_RES_PATH") else { return };
        let fs = ResFilesystem::new(res).unwrap();
        assert_eq!(StaticScene::load(&fs, "spaces/02_malinovka").unwrap().len(), 1_246_841);

        let scene = StaticScene::load(&fs, "spaces/120_graf_zeppelin_scc").unwrap();
        // The hull's centre when it stops against the hangar, heading -z.
        let center = Vec3::new(148.4, 15.7, 480.2);
        let ahead = scene.triangles_near(Vec2::new(center.x - 4.0, center.z - 4.0), Vec2::new(center.x + 4.0, center.z + 4.0))
            .flat_map(|triangle| triangle.vertices)
            .filter(|vertex| (center.y - 0.3..center.y + 3.0).contains(&vertex.y))
            .map(|vertex| Vec2::new(vertex.x - center.x, vertex.z - center.z).length())
            .fold(f32::INFINITY, f32::min);
        assert!((2.6..3.0).contains(&ahead), "{ahead}");
    }

}
