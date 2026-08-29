//! This module defines applications and their elements, it provides abstraction over
//! the socket server for the different types of applications:
//! 
//! - Login app, the server-side application defining the protocol to establish an
//!   initial connection request from the client with initial RSA encryption and 
//!   then transfers the client to the base app once it's successful.
//! 
//! - Client app, the client-side application defining the protocol understood by 
//!   the client, where the client sends its entities and method calls to it.
//! 
//! - Base app, the server-side application defining the protocol understood by the 
//!   server, this is the application that is receiving the initial authentication
//!   key from client and then its method calls to entities.
//! 
//! - Cell app, the server-side application, not directly exposed to the client that
//!   is receiving requests from client when in-game.

pub mod proxy;

pub mod login;
pub mod login_proxy;

pub mod script;

pub mod client;
pub mod base;
pub mod cell;

use std::{fmt, io};


#[inline]
fn io_invalid_data(msg: fmt::Arguments<'_>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}
