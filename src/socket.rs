use std::{
    net::{IpAddr, SocketAddr},
    sync::LazyLock,
};

use procfs_core::{
    ExplicitSystemInfo, FromBufRead, FromBufReadSI,
    net::{TcpNetEntries, TcpState, UdpNetEntries, UdpState, UnixNetEntries},
};

use crate::{Error, Host, Result};

#[derive(Debug, Clone, Copy)]
pub struct Socket<'a> {
    host: &'a Host,
    specification: &'a str,
}

#[derive(Debug)]
enum Query<'a> {
    Internet {
        tcp: bool,
        ip: Option<IpAddr>,
        port: u16,
    },
    Unix(&'a str),
}

impl Host {
    /// Inspect `tcp://[address:]port`, `udp://[address:]port`, or `unix:///path`.
    /// Bracket IPv6 addresses, and use `unix://@name` for abstract Unix sockets.
    pub fn socket<'a>(&'a self, specification: &'a str) -> Socket<'a> {
        Socket {
            host: self,
            specification,
        }
    }
}

impl Socket<'_> {
    /// TCP/Unix stream listeners, or bound unconnected UDP sockets.
    pub fn is_listening(&self) -> Result<bool> {
        match parse_query(self.specification)? {
            Query::Unix(path) => {
                unix_listening(&self.host.file("/proc/self/net/unix").content()?, path)
            }
            Query::Internet { tcp, ip, port } => {
                // EI_DATA in the executing target's ELF header identifies byte order.
                // Do not assume the test runner and Docker daemon share an architecture.
                let endian = self
                    .host
                    .check_output("od -An -tu1 -j5 -N1 /proc/self/exe")?;
                let little_endian = match endian.trim() {
                    "1" => true,
                    "2" => false,
                    _ => {
                        return Err(Error::Parse(
                            "target executable has no recognized ELF byte order".into(),
                        ));
                    }
                };
                for ipv6 in [false, true] {
                    if ip.is_some_and(|ip| ip.is_ipv6() != ipv6) {
                        continue;
                    }
                    let path = format!(
                        "/proc/self/net/{}{}",
                        if tcp { "tcp" } else { "udp" },
                        if ipv6 { "6" } else { "" }
                    );
                    // Linux kernels can omit IPv6 support entirely.
                    if ipv6 && !self.host.file(&path).exists()? {
                        continue;
                    }
                    let bytes = self.host.file(&path).content_bytes()?;
                    if internet_listeners(&bytes, tcp, little_endian)?
                        .iter()
                        .any(|addr| addr.port() == port && ip.is_none_or(|ip| ip == addr.ip()))
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }
}

fn parse_query(spec: &str) -> Result<Query<'_>> {
    let invalid = || Error::InvalidInput(format!("invalid socket specification: {spec:?}"));
    if spec.contains('\0') {
        return Err(invalid());
    }
    if let Some(path) = spec.strip_prefix("unix://") {
        if (path.starts_with('/') || (path.starts_with('@') && path.len() > 1))
            && !path.contains(['\n', '\r'])
        {
            return Ok(Query::Unix(path));
        }
        return Err(invalid());
    }
    let (tcp, endpoint) = if let Some(s) = spec.strip_prefix("tcp://") {
        (true, s)
    } else if let Some(s) = spec.strip_prefix("udp://") {
        (false, s)
    } else {
        return Err(invalid());
    };
    if let Ok(addr) = endpoint.parse::<SocketAddr>() {
        return Ok(Query::Internet {
            tcp,
            ip: Some(addr.ip()),
            port: addr.port(),
        });
    }
    let port = endpoint
        .strip_prefix("*:")
        .unwrap_or(endpoint)
        .parse::<u16>()
        .map_err(|_| invalid())?;
    Ok(Query::Internet {
        tcp,
        ip: None,
        port,
    })
}

fn internet_listeners(bytes: &[u8], tcp: bool, little_endian: bool) -> Result<Vec<SocketAddr>> {
    // Networking parsers only use byte order; timing/page fields are irrelevant.
    let info = ExplicitSystemInfo {
        boot_time_secs: 0,
        ticks_per_second: 0,
        page_size: 0,
        is_little_endian: little_endian,
    };
    if tcp {
        let entries = <TcpNetEntries as FromBufReadSI>::from_buf_read(bytes, &info)
            .map_err(|e| Error::Parse(e.to_string()))?;
        Ok(entries
            .0
            .into_iter()
            .filter(|entry| entry.state == TcpState::Listen)
            .map(|entry| entry.local_address)
            .collect())
    } else {
        let entries = <UdpNetEntries as FromBufReadSI>::from_buf_read(bytes, &info)
            .map_err(|e| Error::Parse(e.to_string()))?;
        Ok(entries
            .0
            .into_iter()
            .filter(|entry| entry.state == UdpState::Close && entry.local_address.port() != 0)
            .map(|entry| entry.local_address)
            .collect())
    }
}

fn unix_listening(text: &str, path: &str) -> Result<bool> {
    let entries =
        UnixNetEntries::from_buf_read(text.as_bytes()).map_err(|e| Error::Parse(e.to_string()))?;
    // procfs-core omits the Unix accept flag and truncates paths at whitespace.
    // Supplement those fields using regex, retaining its validation/type parsing.
    static FIELDS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r"^[[:xdigit:]]+:[ \t]+[[:xdigit:]]+[ \t]+[[:xdigit:]]+[ \t]+([[:xdigit:]]+)[ \t]+[[:xdigit:]]+[ \t]+[[:xdigit:]]+[ \t]+[0-9]+(?:[ \t]+(.*))?$"
    ).expect("valid Unix table field expression")
    });
    for (entry, line) in entries.0.iter().zip(text.lines().skip(1)) {
        let fields = FIELDS
            .captures(line)
            .ok_or_else(|| Error::Parse("invalid Unix socket record".into()))?;
        let flags = u32::from_str_radix(&fields[1], 16).map_err(|e| Error::Parse(e.to_string()))?;
        if entry.socket_type == 1
            && flags & 0x00010000 != 0
            && fields.get(2).is_some_and(|p| p.as_str() == path)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_endian_socket_addresses() -> Result<()> {
        for (address, little) in [("0100007F", true), ("7F000001", false)] {
            let row = format!(
                "header\n0: {address}:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 1234\n"
            );
            assert_eq!(
                internet_listeners(row.as_bytes(), true, little)?,
                vec!["127.0.0.1:8080".parse::<SocketAddr>().unwrap()]
            );
            assert!(
                internet_listeners(row.replace(" 0A ", " 01 ").as_bytes(), true, little)?
                    .is_empty()
            );
        }
        assert!(internet_listeners(b"header\nmalformed\n", true, true).is_err());
        Ok(())
    }
    #[test]
    fn unix_requires_listen_flag_and_exact_path() -> Result<()> {
        let record = "Num RefCount Protocol Flags Type St Inode Path\n00000000: 00000002 00000000 00010000 0001 01 1234 /tmp/my socket[*]\n";
        assert!(unix_listening(record, "/tmp/my socket[*]")?);
        assert!(!unix_listening(record, "/tmp/my socket")?);
        assert!(!unix_listening(
            &record.replace("00010000", "00000000"),
            "/tmp/my socket[*]"
        )?);
        Ok(())
    }
    #[test]
    fn query_rejects_ambiguous_addresses() {
        for value in [
            "tcp://localhost:80",
            "tcp://::1:80",
            "tcp://65536",
            "unix://relative",
            "tcp://80;id",
        ] {
            assert!(parse_query(value).is_err(), "{value}");
        }
        assert!(parse_query("tcp://[::1]:80").is_ok());
        assert!(parse_query("udp://*:53").is_ok());
    }
}
