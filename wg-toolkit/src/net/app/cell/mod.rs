//! Definition of elements related to cell application.
//!
//! The cell app is not directly exposed to the client in the same way the base app
//! is: it handles in-game entity simulation (movement, physics, AoI). No `App`/socket
//! server implementation exists yet for this application, only its element ids and
//! (partial) element codecs, reverse-engineered from the live client.

pub mod element;
