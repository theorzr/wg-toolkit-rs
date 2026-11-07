//! Spaces, as the game ships them in `spaces/<name>`: their compiled space, see
//! [`bin`], and what moves in a space needs of it, see [`Space`].

pub mod bin;
pub mod terrain;
pub mod statics;
pub mod destructibles;

use std::io;

use crate::res::fs::ResFilesystem;
use crate::pxml;

use destructibles::Destructibles;
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

    /// Load a space, `spaces/08_ruinberg` say, as a gameplay type has it, `ctf` say:
    /// only its static models visible in it, see [`visibility_mask`], what of them is
    /// destructible as the game lists it.
    pub fn load(fs: &ResFilesystem, space_dir: &str, gameplay: &str) -> io::Result<Self> {
        let destructibles = Destructibles::load(fs)?;
        let mask = visibility_mask(fs, space_dir, gameplay)?;
        Ok(Self { terrain: Terrain::load(fs, space_dir)?, statics: StaticScene::load(fs, space_dir, &destructibles, mask)? })
    }

}


/// The gameplay types, `ARENA_GAMEPLAY_NAMES` in the game's `constants.py`: a type's id
/// is its index.
pub const GAMEPLAY_NAMES: &[&str] = &[
    "ctf", "domination", "assault", "nations", "ctf2", "domination2", "assault2", "fallout",
    "fallout2", "fallout3", "fallout4", "ctf30x30", "domination30x30", "sandbox", "bootcamp",
    "epic", "maps_training", "rts", "rts_1x1", "rts_bootcamp", "comp7",
];

/// How many of a visibility mask's low bits are the server's, the gameplay types', see
/// `SpaceVisibilityFlags._SERVER_FLAGS_NUMBER`.
const SERVER_VISIBILITY_BITS: u32 = 20;

/// The visibility mask of a gameplay type in a space, which the static models in it
/// have, see [`bin::StaticModelInstance::visibility_mask`]: as the client's
/// `SpaceVisibilityFlags.py` reads it, its bit as the space's
/// `space.settings/spaceVisibilityFlags` maps it, else its id.
pub fn visibility_mask(fs: &ResFilesystem, space_dir: &str, gameplay: &str) -> io::Result<u32> {

    let path = format!("{space_dir}/space.settings");
    let settings = pxml::from_reader(io::BufReader::new(fs.read(&path)?))
        .map_err(|e| invalid(format!("{path}: {e}")))?;

    let bit = match settings.get_child_element("spaceVisibilityFlags") {
        Some(flags) => flags.iter_children("type")
            .filter_map(|flag| flag.as_element())
            .find(|flag| flag.get_child_string("tag") == Some(gameplay))
            .and_then(|flag| flag.get_child_integer("bitIndex"))
            .ok_or_else(|| invalid(format!("{path}: no visibility bit for gameplay {gameplay}")))? as u32,
        None => GAMEPLAY_NAMES.iter().position(|name| *name == gameplay)
            .ok_or_else(|| invalid(format!("unknown gameplay {gameplay}")))? as u32,
    };

    if bit >= SERVER_VISIBILITY_BITS {
        return Err(invalid(format!("{path}: visibility bit {bit} of gameplay {gameplay} isn't the server's")));
    }
    Ok(1 << bit)

}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
