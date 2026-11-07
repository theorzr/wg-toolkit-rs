//! Spaces, as the game ships them in `spaces/<name>`: their compiled space, see
//! [`bin`], and what moves in a space needs of it, see [`Space`].

pub mod bin;
pub mod terrain;
pub mod statics;

use std::io;

use crate::res::fs::ResFilesystem;

use statics::StaticScene;
use terrain::Terrain;


/// A space as what moves in it needs it: its terrain and its static models' collision.
/// The default one is empty, with neither.
#[derive(Debug, Default)]
pub struct Space {
    pub terrain: Terrain,
    pub statics: StaticScene,
}

impl Space {

    /// Load a space, `spaces/08_ruinberg` say.
    pub fn load(fs: &ResFilesystem, space_dir: &str) -> io::Result<Self> {
        Ok(Self { terrain: Terrain::load(fs, space_dir)?, statics: StaticScene::load(fs, space_dir)? })
    }

}

