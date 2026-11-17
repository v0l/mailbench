use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

pub const TYPE_MX: u16 = 15;
pub const TYPE_SRV: u16 = 33;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mx {
    pub preference: u16,
    pub exchange: String,
}

pub fn encode_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.').filter(|l| !l.is_empty()) {
        out.push(label.len().min(63) as u8);
        out.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    out.push(0);
    out
}

pub fn encode_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&0x0100u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    out.extend(encode_name(name));
    out.extend_from_slice(&qtype.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out
}

fn u16_at(msg: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*msg.get(at)?, *msg.get(at + 1)?]))
}

fn read_name(msg: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut pos = start;
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            let target = ((len & 0x3F) << 8) | *msg.get(pos + 1)? as usize;
            if end.is_none() {
                end = Some(pos + 2);
            }
            jumps += 1;
            if jumps > 16 {
                return None;
            }
            pos = target;
            continue;
        }
        let label = msg.get(pos + 1..pos + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).into_owned());
        pos += 1 + len;
    }
    Some((labels.join("."), end.unwrap_or(pos)))
}

struct Record<'a> {
    rtype: u16,
    rdata_at: usize,
    rdata: &'a [u8],
}

fn records(msg: &[u8]) -> Option<Vec<Record<'_>>> {
    let flags = u16_at(msg, 2)?;
    if flags & 0x000F != 0 {
        return Some(Vec::new());
    }
    let questions = u16_at(msg, 4)? as usize;
    let answers = u16_at(msg, 6)? as usize;
    let mut pos = 12;
    for _ in 0..questions {
        let (_, next) = read_name(msg, pos)?;
        pos = next + 4;
    }
    let mut out = Vec::new();
    for _ in 0..answers {
        let (_, next) = read_name(msg, pos)?;
        let rtype = u16_at(msg, next)?;
        let len = u16_at(msg, next + 8)? as usize;
        let rdata_at = next + 10;
        let rdata = msg.get(rdata_at..rdata_at + len)?;
        out.push(Record { rtype, rdata_at, rdata });
        pos = rdata_at + len;
    }
    Some(out)
}

pub fn parse_srv(msg: &[u8]) -> Vec<Srv> {
    let mut out: Vec<Srv> = records(msg)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.rtype == TYPE_SRV && r.rdata.len() >= 7)
        .filter_map(|r| {
            let (target, _) = read_name(msg, r.rdata_at + 6)?;
            Some(Srv {
                priority: u16_at(r.rdata, 0)?,
                weight: u16_at(r.rdata, 2)?,
                port: u16_at(r.rdata, 4)?,
                target,
            })
        })
        .collect();
    out.sort_by_key(|s| (s.priority, std::cmp::Reverse(s.weight)));
    out
}

pub fn parse_mx(msg: &[u8]) -> Vec<Mx> {
    let mut out: Vec<Mx> = records(msg)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.rtype == TYPE_MX && r.rdata.len() >= 3)
        .filter_map(|r| {
            let (exchange, _) = read_name(msg, r.rdata_at + 2)?;
            Some(Mx { preference: u16_at(r.rdata, 0)?, exchange })
        })
        .collect();
    out.sort_by_key(|m| m.preference);
    out
}

pub fn nameservers() -> Vec<SocketAddr> {
    let mut found: Vec<SocketAddr> = std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .filter_map(|ip| ip.trim().parse::<std::net::IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, 53))
        .collect();
    if found.is_empty() {
        found = vec!["1.1.1.1:53".parse().unwrap(), "9.9.9.9:53".parse().unwrap()];
    }
    found
}

pub fn query(servers: &[SocketAddr], name: &str, qtype: u16) -> std::io::Result<Vec<u8>> {
    let id = rand::random::<u16>();
    let packet = encode_query(id, name, qtype);
    let mut last = std::io::Error::other("no nameserver");
    for server in servers {
        let bind = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let attempt = (|| {
            let socket = UdpSocket::bind(bind)?;
            socket.set_read_timeout(Some(Duration::from_secs(3)))?;
            socket.connect(server)?;
            socket.send(&packet)?;
            let mut buf = vec![0u8; 4096];
            loop {
                let n = socket.recv(&mut buf)?;
                if n >= 2 && u16_at(&buf, 0) == Some(id) {
                    buf.truncate(n);
                    return Ok(buf);
                }
            }
        })();
        match attempt {
            Ok(msg) => return Ok(msg),
            Err(e) => last = e,
        }
    }
    Err(last)
}
