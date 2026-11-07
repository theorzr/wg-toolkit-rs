//! What of a space's static models is destructible, as the game's
//! `scripts/destructibles.xml` lists them by model, see the client's
//! `DestructiblesCache.py`: `trees`, `fallingAtoms` (lamp posts, poles) and `fragiles`
//! (benches, fences, barrels) are destructible whole, `structures` (houses) by modules,
//! each a body of their collision by its name, `n_wood0_1_5` say.

use std::collections::{HashMap, HashSet};
use std::io;

use crate::res::fs::ResFilesystem;
use crate::pxml;


/// Where the game lists its destructibles.
const DESTRUCTIBLES_PATH: &str = "scripts/destructibles.xml";


/// The destructible models, by their path without extension and in lower case, as
/// resource paths are: the same for their `.model`, as listed, and their `.havok`
/// collision.
#[derive(Debug, Default)]
pub struct Destructibles {
    whole: HashSet<String>,
    /// A structure's destructible modules, their body names.
    modules: HashMap<String, HashSet<String>>,
}

impl Destructibles {

    /// Read the game's destructibles.
    pub fn load(fs: &ResFilesystem) -> io::Result<Self> {

        let file = fs.read(DESTRUCTIBLES_PATH)
            .map_err(|e| io::Error::new(e.kind(), format!("{DESTRUCTIBLES_PATH}: {e}")))?;
        
        let root = pxml::from_reader(io::BufReader::new(file))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{DESTRUCTIBLES_PATH}: {e}")))?;

        let mut destructibles = Self::default();
        
        for kind in ["trees", "fallingAtoms", "fragiles"] {
            destructibles.whole.extend(entries(&root, kind).filter_map(model));
        }

        for entry in entries(&root, "structures") {
           
            let Some(model) = model(entry) else { continue };
            let modules = entry.get_child_element("modules").into_iter()
                .flat_map(|modules| modules.iter_children("module"))
                .filter_map(|module| module.as_element()?.get_child_string("matName"))
                .map(str::to_string)
                .collect();

            destructibles.modules.insert(model, modules);

        }

        Ok(destructibles)

    }

    /// Whether a body of a model is destructible, the model by any of its paths, its
    /// `.havok` say, the body by its name.
    pub fn is_destructible(&self, model: &str, body: &str) -> bool {
        let model = stem(model);
        self.whole.contains(&model) || self.modules.get(&model).is_some_and(|modules| modules.contains(body))
    }

}

/// The entries of a kind of destructibles, `fragiles` say.
fn entries<'a>(root: &'a pxml::Element, kind: &str) -> impl Iterator<Item = &'a pxml::Element> {
    root.get_child_element(kind).into_iter()
        .flat_map(|kind| kind.iter_children("entry"))
        .filter_map(|entry| entry.as_element())
}

/// An entry's model, as [`Destructibles`] keys it, see [`stem`].
fn model(entry: &pxml::Element) -> Option<String> {
    entry.get_child_string("filename").map(stem)
}

/// A model's path as [`Destructibles`] keys it: without its extension, in lower case.
fn stem(path: &str) -> String {
    path.rsplit_once('.').map_or(path, |(stem, _)| stem).to_lowercase()
}
