//! Elements waiting to be sent to clients, gathered in one bundle per client until the
//! app flushes them, rather than each sent alone in its own bundle as soon as written.
//!
//! An outbox also remembers which entity its client has selected, so that an element
//! addressed to the selected entity (a method call or a property change) is preceded by
//! a `SelectEntity` or `SelectPlayerEntity` only when that entity isn't already the one
//! selected. The client keeps its selection from one bundle to the next, but every flush
//! forgets it here, which costs a one-byte select at worst and keeps a bundle readable
//! on its own.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::io;

use crate::net::bundle::Bundle;
use crate::net::element::Element;
use crate::net::proto::Protocol;
use crate::net::socket::PacketSocket;

use super::client::element::{SelectEntity, SelectPlayerEntity};


/// How many packets a client's bundle may fill before it is sent without waiting for
/// the flush, so that a burst (entering a battle, say) goes out as a few bundles rather
/// than one of dozens of packets. Not taken from the real server, whose own limit is not
/// known: about 11 KB.
const FLUSH_PACKETS: usize = 8;

/// The outboxes of every client an app sends to.
#[derive(Debug, Default)]
pub struct Outbox {
    clients: HashMap<SocketAddr, ClientOutbox>,
}

impl Outbox {

    /// The outbox of the given client, created empty if it has none yet. What waits in it
    /// is sent first when it already fills [`FLUSH_PACKETS`] packets.
    pub fn client(&mut self, addr: SocketAddr, protocol: &mut Protocol, socket: &PacketSocket) -> io::Result<&mut ClientOutbox> {
        let client = self.clients.entry(addr).or_insert_with(ClientOutbox::new);
        if client.bundle.len() >= FLUSH_PACKETS {
            client.flush(addr, protocol, socket)?;
        }
        Ok(client)
    }

    /// Forget what waits for the given client, gone.
    pub fn remove(&mut self, addr: SocketAddr) {
        self.clients.remove(&addr);
    }

    /// Send what waits for the given client, as one reliable bundle.
    pub fn flush(&mut self, addr: SocketAddr, protocol: &mut Protocol, socket: &PacketSocket) -> io::Result<()> {
        match self.clients.get_mut(&addr) {
            Some(client) => client.flush(addr, protocol, socket),
            None => Ok(()),
        }
    }

    /// Send what waits for every client, returning the clients it failed for.
    pub fn flush_all(&mut self, protocol: &mut Protocol, socket: &PacketSocket) -> Vec<(SocketAddr, io::Error)> {
        let mut errors = Vec::new();
        for (&addr, client) in &mut self.clients {
            if let Err(error) = client.flush(addr, protocol, socket) {
                errors.push((addr, error));
            }
        }
        errors
    }

}

/// The bundle waiting to be sent to one client, and what it selects, see [`Outbox`].
#[derive(Debug)]
pub struct ClientOutbox {
    bundle: Bundle,
    /// The entity the client will have selected once it has read the bundle so far, when
    /// known: any element written with [`Self::write`] may select another.
    selected: Option<Selected>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Selected {
    Player,
    Entity(u32),
}

impl ClientOutbox {

    fn new() -> Self {
        Self { bundle: Bundle::new(), selected: None }
    }

    /// Select the client's player entity for the elements that follow.
    pub fn select_player(&mut self) -> io::Result<()> {
        if self.selected != Some(Selected::Player) {
            self.bundle.element_writer().write_simple(SelectPlayerEntity {})?;
            self.selected = Some(Selected::Player);
        }
        Ok(())
    }

    /// Select the given entity for the elements that follow.
    pub fn select_entity(&mut self, entity_id: u32) -> io::Result<()> {
        if self.selected != Some(Selected::Entity(entity_id)) {
            self.bundle.element_writer().write_simple(SelectEntity { entity_id })?;
            self.selected = Some(Selected::Entity(entity_id));
        }
        Ok(())
    }

    /// Write an element addressed to the selected entity, which leaves it selected.
    pub fn write_selected<E: Element<C>, C: ?Sized>(&mut self, element: E, config: &C) -> io::Result<()> {
        self.bundle.element_writer().write(element, config)
    }

    /// Write any other element. It may select another entity, so the next selection is
    /// written whatever it is.
    pub fn write<E: Element<C>, C: ?Sized>(&mut self, element: E, config: &C) -> io::Result<()> {
        self.bundle.element_writer().write(element, config)?;
        self.selected = None;
        Ok(())
    }

    /// Same as [`Self::write`], for an element without config.
    pub fn write_simple<E: Element<()>>(&mut self, element: E) -> io::Result<()> {
        self.write(element, &())
    }

    fn flush(&mut self, addr: SocketAddr, protocol: &mut Protocol, socket: &PacketSocket) -> io::Result<()> {
        if self.bundle.is_empty() {
            return Ok(());
        }
        self.selected = None;
        protocol.channel(addr, None).prepare(&mut self.bundle, true);
        let result = socket.send_bundle(&self.bundle, addr);
        self.bundle.clear();
        result.map(|_| ())
    }

}


#[cfg(test)]
mod tests {

    use crate::net::bundle::NextElementReader;

    use super::super::client::element::{id, TickSync};
    use super::*;

    /// The ids of the elements written to an outbox, in order.
    fn ids(client: &ClientOutbox) -> Vec<u8> {
        let mut ids = Vec::new();
        let mut reader = client.bundle.element_reader();
        while let Some(NextElementReader::Element(elt)) = reader.next() {
            let id = elt.id();
            ids.push(id);
            match id {
                id::SELECT_PLAYER_ENTITY => { elt.read_simple::<SelectPlayerEntity>().unwrap(); }
                id::SELECT_ENTITY => { elt.read_simple::<SelectEntity>().unwrap(); }
                id::TICK_SYNC => { elt.read_simple::<TickSync>().unwrap(); }
                id => panic!("unexpected element {id:#04X}"),
            }
        }
        ids
    }

    /// A selection is written only when it changes, and any other element forgets it.
    #[test]
    fn selection_is_written_once_until_another_element() {

        let mut client = ClientOutbox::new();
        client.select_player().unwrap();
        client.select_player().unwrap();
        client.select_entity(7).unwrap();
        client.select_entity(7).unwrap();
        client.write_simple(TickSync { tick: 0 }).unwrap();
        client.select_entity(7).unwrap();

        assert_eq!(ids(&client), [
            id::SELECT_PLAYER_ENTITY,
            id::SELECT_ENTITY,
            id::TICK_SYNC,
            id::SELECT_ENTITY,
        ]);

    }

}
