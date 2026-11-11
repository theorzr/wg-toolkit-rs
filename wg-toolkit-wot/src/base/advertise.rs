//! The address a client is told to reach the base app at, as it logs in and as it
//! switches into its battles and back: by rules of the clients' addresses, the most
//! specific one matching a client's address winning, else the base app's own address,
//! the one the client's traffic comes in on when the base app is bound to all.
//!
//! ```text
//!   --advertise 0.0.0.0/0=203.0.113.7:20017        anyone: the public address, the
//!                                                  router's port forwarded
//!   --advertise 192.168.1.0/24=192.168.1.10:20017  the LAN: the machine's address on it
//!   --advertise 127.0.0.0/8=127.0.0.1              the machine itself, the base app's port
//! ```

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::str::FromStr;


/// What address the clients of a network are told, see the module's documentation: as
/// `<network>/<prefix>=<ip>[:<port>]`, a network without a prefix being one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdvertiseRule {
    /// The network its clients are in, and how many of its leading bits they share.
    network: Ipv4Addr,
    prefix: u8,
    /// The address they are told, its port the base app's if none.
    ip: Ipv4Addr,
    port: Option<u16>,
}

impl AdvertiseRule {

    /// Whether a client at this address is in its network.
    fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
        u32::from(ip) & mask == u32::from(self.network) & mask
    }

}

impl FromStr for AdvertiseRule {

    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        
        let (network, addr) = value.split_once('=')
            .ok_or_else(|| format!("Expected <network>/<prefix>=<ip>[:<port>], got {value}"))?;
        
        let (network, prefix) = match network.split_once('/') {
            Some((network, prefix)) => (network, prefix.parse::<u8>().map_err(|e| format!("Invalid prefix {prefix}: {e}"))?),
            None => (network, 32),
        };
        
        if prefix > 32 {
            return Err(format!("Invalid prefix {prefix}: over 32"));
        }
        
        let network = network.parse::<Ipv4Addr>().map_err(|e| format!("Invalid network {network}: {e}"))?;
        let (ip, port) = match addr.parse::<SocketAddrV4>() {
            Ok(addr) => (*addr.ip(), Some(addr.port())),
            Err(_) => (addr.parse::<Ipv4Addr>().map_err(|e| format!("Invalid address {addr}: {e}"))?, None),
        };

        Ok(Self {
            network,
            prefix,
            ip,
            port,
        })

    }

}

/// The address the base app, bound at `bound`, tells a client at `client` to reach it at,
/// by these rules, see the module's documentation.
pub fn advertised(rules: &[AdvertiseRule], bound: SocketAddrV4, client: SocketAddr) -> SocketAddrV4 {

    let ip = match client.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped(),
    };

    let rule = ip.and_then(|ip| rules.iter()
        .filter(|rule| rule.contains(ip))
        .max_by_key(|rule| rule.prefix));
    
    match rule {
        Some(rule) => SocketAddrV4::new(rule.ip, rule.port.unwrap_or(bound.port())),
        None if bound.ip().is_unspecified() => SocketAddrV4::new(local_ip(client).unwrap_or(*bound.ip()), bound.port()),
        None => bound,
    }

}

/// The local address the system sends to a client from, which its traffic comes in on,
/// routes being the same both ways: a socket connected to it, which sends nothing, has
/// it as its own.
fn local_ip(client: SocketAddr) -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(client).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}


#[cfg(test)]
mod tests {

    use super::*;

    fn rules() -> Vec<AdvertiseRule> {
        ["0.0.0.0/0=203.0.113.7:30017", "192.168.1.0/24=192.168.1.10:20017", "127.0.0.0/8=127.0.0.1"]
            .into_iter()
            .map(|rule| rule.parse().unwrap())
            .collect()
    }

    fn addr(value: &str) -> SocketAddr {
        value.parse().unwrap()
    }

    #[test]
    fn tells_each_network_its_address() {
        let bound = "0.0.0.0:20017".parse().unwrap();
        let rules = rules();
        assert_eq!(advertised(&rules, bound, addr("198.51.100.4:5000")), "203.0.113.7:30017".parse().unwrap());
        assert_eq!(advertised(&rules, bound, addr("192.168.1.34:5000")), "192.168.1.10:20017".parse().unwrap());
        assert_eq!(advertised(&rules, bound, addr("192.168.2.34:5000")), "203.0.113.7:30017".parse().unwrap());
        assert_eq!(advertised(&rules, bound, addr("127.0.0.1:5000")), "127.0.0.1:20017".parse().unwrap());
    }

    #[test]
    fn tells_its_own_address_without_rules() {
        let bound = "10.0.0.5:20017".parse().unwrap();
        assert_eq!(advertised(&[], bound, addr("198.51.100.4:5000")), bound);
        // Bound to all, the address the client's traffic comes in on.
        let bound = "0.0.0.0:20017".parse().unwrap();
        assert_eq!(advertised(&[], bound, addr("127.0.0.1:5000")), "127.0.0.1:20017".parse().unwrap());
    }

    #[test]
    fn refuses_bad_rules() {
        for rule in ["192.168.1.0/24", "192.168.1.0/33=1.2.3.4", "192.168.1/24=1.2.3.4", "0.0.0.0/0=1.2.3.4:x"] {
            assert!(rule.parse::<AdvertiseRule>().is_err(), "{rule}");
        }
    }

}
