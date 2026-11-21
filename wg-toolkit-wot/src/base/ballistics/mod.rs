//! What shells do: a gun's ammunition and reloads, a shell's flight through a map,
//! then what it does to what it meets, a vehicle's armour or the ground.
//!
//! ```text
//!   gun        its ammunition, magazine, bursts and reloads, and its dispersion
//!   shot       a shell's flight, step by step as the battle goes, until it lands
//!   speed      a shell's speed, and the armour it pierces at it
//!   impact     what a shell does to a vehicle: pierced, stopped, off it, its damage
//!   ricochet   the way and the speed a shell goes off armour or the ground at
//! ```

pub mod gun;
pub mod shot;
pub mod speed;
pub mod impact;
pub mod ricochet;

#[cfg(test)]
pub mod fixtures;
