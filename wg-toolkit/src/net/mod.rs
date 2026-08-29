//! BigWorld/Core network protocol and applications definition.
//! 
//! # Protocol
//! 
//! The network protocol is quite complex, the API proposed in this modules use following
//! terminology:
//! 
//! - Element: It's the smallest unit of the protocol, it has an ID and a data content
//!   that depends on the ID. A request ID can also be attached to an element, to be
//!   later answered using a (not so) special reply element.
//! 
//! - Bundle: An ordered sequence of elements that should be received in order on the 
//!   client side. A bundle automatically use multiple datagram packets if there are
//!   too much elements.
//! 
//! - Packet: Represents the payload sent in a single UDP datagram, it can be interpreted
//!   as-is because it contains all informations about itself, including its sequence 
//!   number if part of a bundle to be reconstructed.

pub mod codec;
pub mod element;

pub mod seq;
pub mod packet;
pub mod bundle;

pub mod filter;

pub mod socket;
pub mod proto;
