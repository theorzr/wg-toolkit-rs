//! What of a space's static models is destructible, as the game's
//! `scripts/destructibles.xml` lists them by model, see the client's
//! `DestructiblesCache.py`: `trees`, `fallingAtoms` (lamp posts, poles) and `fragiles`
//! (benches, fences, barrels) are destructible whole, `structures` (houses) by modules,
//! each a body of their collision by its name, `n_wood0_1_5` say.
//!
//! # The file
//!
//! One file for every map, each entry naming its model. Only what the server needs to
//! break things is read here; the effects and the physics of a falling piece are the
//! client's.
//!
//! ```text
//!   <unitVehicleMass> 35000 </unitVehicleMass>      health is measured against it
//!   <fragiles>
//!     <entry>
//!       <filename> content/…/fence_01.model </filename>
//!       <health> 10 </health>
//!     </entry> …
//!   </fragiles>
//!   <fallingAtoms> … </fallingAtoms>                the same, lamp posts and poles
//!   <trees> … </trees>                              the same, speed trees
//!   <structures>
//!     <entry>
//!       <filename> content/…/house_03.model </filename>
//!       <modules>
//!         <module> <matName> n_wood0_1 </matName>   <health> 50 </health> </module>
//!         <module> <matName> n_wood0_2_1 </matName> <health> 35 </health> </module>
//!       </modules>
//!     </entry> …
//!   </structures>
//! ```
//!
//! A model is keyed by its path without extension, in lower case, see `stem`. Its
//! `.model` (as listed) and its `.havok` (as `space.bin` gives it) then share a key.
//!
//! # A structure's modules
//!
//! A module's `matName` is also the name of its collision body:
//! `<type>_<surface>_<id>[_<depends>…]`. In `n_wood0_2_1`, `n` is the type, `wood0`
//! the surface, `2` the id, and `1` the one module it lists. Its *material kind* is the
//! number the client knows it by, and the number its collision's bodies carry: 73 for
//! the module with the lowest id, then one more for each id in order, see
//! [`FIRST_MODULE_MATERIAL_KIND`]. Ids need not follow each other: with ids 1, 2 and 5,
//! the kinds are 73, 74 and 75.
//!
//! ```text
//!   matName        id   lists   material kind   breaking it also breaks
//!   n_wood0_1      1    –       73              –
//!   n_wood0_2_1    2    1       74              n_wood0_1 (73)
//! ```
//!
//! [`Module::depends`] is what the client's `destroyDepends` is: the modules a module
//! lists, and theirs, as far as the chain goes, each breaking with it. The client also
//! keeps the inverse, `inversedDestroyDepends`. No capture has shown yet which of the
//! two the real server applies when a module breaks.
//!
//! # Health
//!
//! A destructible's health is at a scale of 1. A model placed larger is stronger, by
//! the square of its scale, see [`scaled_health`]. A vehicle's ram damage is measured
//! in units of [`Destructibles::unit_vehicle_mass`], so a 35 t tank and a 70 t tank
//! break things in the ratio of their weights.

use std::collections::HashMap;
use std::io;

use crate::res::fs::ResFilesystem;
use crate::pxml::{self, Element};


/// Where the game lists its destructibles.
const DESTRUCTIBLES_PATH: &str = "scripts/destructibles.xml";

/// The material kind of a structure's first module, the next ones following in the
/// order of their ids, `DESTRUCTIBLE_MATKIND.NORMAL_MIN` in the client's `constants.py`:
/// as its collision's bodies have it, and as the server names a destroyed module.
pub const FIRST_MODULE_MATERIAL_KIND: u8 = 73;


/// The destructible models, by their path without extension and in lower case, as
/// resource paths are: the same for their `.model`, as listed, and their `.havok`
/// collision.
#[derive(Debug, Default)]
pub struct Destructibles {
    models: HashMap<String, Destructible>,
    /// The weight of a vehicle that the destructibles' health is measured against, in
    /// kilograms.
    pub unit_vehicle_mass: f32,
}

/// A destructible model, see [`Destructibles`].
#[derive(Debug, Clone)]
pub struct Destructible {
    pub kind: DestructibleKind,
    /// Its health at a scale of 1, whole, see [`scaled_health`]; a structure's is its
    /// modules'.
    pub health: f32,
    /// A structure's modules, by their body's name.
    pub modules: HashMap<String, Module>,
}

/// What a destructible does once destroyed, `DESTR_TYPE_*` in the client's
/// `DestructiblesCache.py`, its value the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DestructibleKind {
    /// It falls over, a speed tree.
    Tree = 0,
    /// It falls over, a lamp post or a pole.
    FallingAtom = 1,
    /// It breaks, a fence or a barrel.
    Fragile = 2,
    /// It breaks by modules, a house.
    Structure = 3,
}

/// A module of a structure, see [`Destructible::modules`].
#[derive(Debug, Clone)]
pub struct Module {
    /// Its material kind, from [`FIRST_MODULE_MATERIAL_KIND`] in the order of its
    /// structure's module ids.
    pub material_kind: u8,
    pub health: f32,
    /// The material kinds of the modules it brings down with it, as far as they go: the
    /// client's `destroyDepends`, see the module's docs.
    pub depends: Vec<u8>,
}

impl Destructibles {

    /// Read the game's destructibles.
    pub fn load(fs: &ResFilesystem) -> io::Result<Self> {

        let file = fs.read(DESTRUCTIBLES_PATH)
            .map_err(|e| io::Error::new(e.kind(), format!("{DESTRUCTIBLES_PATH}: {e}")))?;

        let root = pxml::from_reader(io::BufReader::new(file))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{DESTRUCTIBLES_PATH}: {e}")))?;

        let mut destructibles = Self {
            models: HashMap::new(),
            unit_vehicle_mass: number(&root, "unitVehicleMass").unwrap_or(35_000.0),
        };

        let whole = [
            ("trees", DestructibleKind::Tree),
            ("fallingAtoms", DestructibleKind::FallingAtom),
            ("fragiles", DestructibleKind::Fragile),
        ];

        for (section, kind) in whole {
            for entry in entries(&root, section) {
                let Some(model) = model(entry) else { continue };
                let health = number(entry, "health").unwrap_or(0.0);
                destructibles.models.insert(model, Destructible {
                    kind,
                    health,
                    modules: HashMap::new(),
                });
            }
        }

        for entry in entries(&root, "structures") {
            let Some(model) = model(entry) else { continue };
            destructibles.models.insert(model, Destructible {
                kind: DestructibleKind::Structure,
                health: 0.0,
                modules: modules(entry),
            });
        }

        Ok(destructibles)

    }

    /// A model's destructible, by any of its paths, its `.havok` say.
    pub fn get(&self, model: &str) -> Option<&Destructible> {
        self.models.get(&stem(model))
    }

    /// Whether a body of a model is destructible, the model by any of its paths, its
    /// `.havok` say, the body by its name and material kind: a structure's module is its
    /// `n_` body and the solid ones of the same material kind, a gun emplacement's
    /// `s_wall_1` with its `n_wood0_1` say.
    pub fn is_destructible(&self, model: &str, body: &str, material_kind: u32) -> bool {
        let Some(destructible) = self.get(model) else { return false };
        destructible.kind != DestructibleKind::Structure
            || destructible.modules.contains_key(body)
            || destructible.modules.values().any(|module| u32::from(module.material_kind) == material_kind)
    }

}

/// The health of a destructible placed at a scale, `scaledDestructibleHealth` in the
/// client's `DestructiblesCache.py`: rounded up, but a scale a hair over 1, as placed
/// transforms give, doesn't add a point.
pub fn scaled_health(scale: f32, health: f32) -> f32 {
    (scale * scale * health - 1e-3).ceil()
}

/// A structure's modules, see [`Destructible::modules`]: each named
/// `<type>_<surface>_<id>[_<depends>…]`, numbered in the order of their ids, those that
/// don't parse left out, as `_parseMaterialName` does.
fn modules(entry: &Element) -> HashMap<String, Module> {

    let mut parsed = Vec::new();
    for module in entry.get_child_element("modules").into_iter().flat_map(|modules| modules.iter_children("module")) {
        let Some(module) = module.as_element() else { continue };
        let Some(name) = module.get_child_string("matName") else { continue };
        let mut parts = name.split('_').skip(2);
        let Some(Ok(id)) = parts.next().map(str::parse::<u32>) else { continue };
        let Ok(depends) = parts.map(str::parse::<u32>).collect::<Result<Vec<_>, _>>() else { continue };
        parsed.push((name.to_string(), id, depends, number(module, "health").unwrap_or(0.0)));
    }

    let mut ids: Vec<u32> = parsed.iter().map(|&(_, id, _, _)| id).collect();
    ids.sort_unstable();

    let mut direct = HashMap::new();
    for (_, id, depends, _) in &parsed {
        let Some(kind) = material_kind(&ids, *id) else { continue };
        direct.insert(kind, depends.iter().filter_map(|&depend| material_kind(&ids, depend)).collect::<Vec<_>>());
    }

    let mut modules = HashMap::new();
    for (name, id, _, health) in parsed {
        let Some(kind) = material_kind(&ids, id) else { continue };
        modules.insert(name, Module {
            material_kind: kind,
            health,
            depends: reached(kind, &direct),
        });
    }
    
    modules

}

/// The material kind of a structure's module, by its id among its sorted `ids`.
fn material_kind(ids: &[u32], id: u32) -> Option<u8> {
    let rank = ids.iter().position(|&other| other == id)?;
    Some(FIRST_MODULE_MATERIAL_KIND + rank as u8)
}

/// The modules a module brings down, as far as their own depends go, without itself.
fn reached(root: u8, direct: &HashMap<u8, Vec<u8>>) -> Vec<u8> {
    let mut reached = Vec::new();
    let mut stack = vec![root];
    while let Some(kind) = stack.pop() {
        if reached.contains(&kind) {
            continue;
        }
        reached.push(kind);
        stack.extend(direct.get(&kind).into_iter().flatten());
    }
    reached.retain(|&kind| kind != root);
    reached
}

/// A child's number, written as an integer or a float.
fn number(element: &Element, key: &str) -> Option<f32> {
    element.get_child_integer(key).map(|value| value as f32).or_else(|| element.get_child_float(key))
}

/// The entries of a kind of destructibles, `fragiles` say.
fn entries<'a>(root: &'a Element, kind: &str) -> impl Iterator<Item = &'a Element> {
    root.get_child_element(kind).into_iter()
        .flat_map(|kind| kind.iter_children("entry"))
        .filter_map(|entry| entry.as_element())
}

/// An entry's model, as [`Destructibles`] keys it, see [`stem`].
fn model(entry: &Element) -> Option<String> {
    entry.get_child_string("filename").map(stem)
}

/// A model's path as [`Destructibles`] keys it: without its extension, in lower case.
fn stem(path: &str) -> String {
    path.rsplit_once('.').map_or(path, |(stem, _)| stem).to_lowercase()
}
