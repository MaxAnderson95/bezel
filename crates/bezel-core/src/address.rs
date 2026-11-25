//! What can be typed to reach a machine: `host`, `host:port` or an IP address.

use std::net::Ipv6Addr;

pub const DEFAULT_PORT: u16 = 3389;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("Type a host name to connect")]
    Empty,
    #[error("\"{0}\" is not a port")]
    Port(String),
    #[error("A host name can't contain spaces")]
    Space,
}

impl Address {
    pub fn parse(input: &str) -> Result<Self, AddressError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(AddressError::Empty);
        }
        if input.parse::<Ipv6Addr>().is_ok() {
            return Ok(Self {
                host: input.into(),
                port: DEFAULT_PORT,
            });
        }
        let (host, port) = match input.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
            Some((host, rest)) => (host, rest.strip_prefix(':')),
            None => match input.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (input, None),
            },
        };
        if host.is_empty() {
            return Err(AddressError::Empty);
        }
        if host.contains(char::is_whitespace) {
            return Err(AddressError::Space);
        }
        let port = match port {
            None => DEFAULT_PORT,
            Some(port) => port
                .parse()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| AddressError::Port(port.into()))?,
        };
        Ok(Self {
            host: host.into(),
            port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(host: &str, port: u16) -> Result<Address, AddressError> {
        Ok(Address {
            host: host.into(),
            port,
        })
    }

    #[test]
    fn reads_hosts_ports_and_ips() {
        assert_eq!(Address::parse("dc01.corp.lan"), address("dc01.corp.lan", 3389));
        assert_eq!(Address::parse(" dc01:3390 "), address("dc01", 3390));
        assert_eq!(Address::parse("10.0.0.5"), address("10.0.0.5", 3389));
        assert_eq!(Address::parse("10.0.0.5:3390"), address("10.0.0.5", 3390));
        assert_eq!(Address::parse("fe80::1"), address("fe80::1", 3389));
        assert_eq!(Address::parse("[fe80::1]:3390"), address("fe80::1", 3390));
        assert_eq!(Address::parse("[fe80::1]"), address("fe80::1", 3389));
    }

    #[test]
    fn refuses_what_cannot_be_reached() {
        assert_eq!(Address::parse("  "), Err(AddressError::Empty));
        assert_eq!(Address::parse(":3389"), Err(AddressError::Empty));
        assert_eq!(Address::parse("dc01:rdp"), Err(AddressError::Port("rdp".into())));
        assert_eq!(Address::parse("dc01:0"), Err(AddressError::Port("0".into())));
        assert_eq!(Address::parse("dc01:"), Err(AddressError::Port("".into())));
        assert_eq!(Address::parse("dc 01"), Err(AddressError::Space));
    }
}
