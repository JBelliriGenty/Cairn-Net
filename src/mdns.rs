//! Bonjour names: a unicast mDNS reverse query (PTR <ip>.in-addr.arpa) sent
//! to each client's port 5353. Awake Apple devices (and many printers and
//! speakers) answer with their own name, such as "MacBook-Pro-de-Alex.local",
//! far better than the "Mac" they send in DHCP. Answers are kept per MAC in
//! data_dir/bonjour.json, so a device asleep keeps its name, restarts included.
//! Only clients on the app's own networks answer: responders ignore queries
//! from another subnet (RFC 6762), whatever the firewall says.

use serde_json::Value;
use std::collections::BTreeMap;
use std::net::UdpSocket;
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static NAMES: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
static FILE: OnceLock<String> = OnceLock::new();

pub fn init(data_dir: &str) {
    let path = format!("{data_dir}/bonjour.json");
    if let Some(m) = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        *NAMES.lock().unwrap_or_else(|e| e.into_inner()) = m;
    }
    let _ = FILE.set(path);
}

/// MAC (upper case) -> Bonjour name, without ".local".
pub fn names() -> BTreeMap<String, String> {
    NAMES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn query(ip: &str, id: u16) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    for part in ip.split('.').rev().chain(["in-addr", "arpa"]) {
        q.push(part.len() as u8);
        q.extend_from_slice(part.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 12, 0, 1]); // root, PTR, IN
    q
}

/// A name at `off` (labels and compression pointers), and the offset after it.
fn name_at(d: &[u8], mut off: usize) -> Option<(Vec<String>, usize)> {
    let mut out = Vec::new();
    let mut end = None;
    for _ in 0..32 {
        let n = *d.get(off)? as usize;
        if n == 0 {
            return Some((out, end.unwrap_or(off + 1)));
        }
        if n & 0xC0 == 0xC0 {
            end.get_or_insert(off + 2);
            off = ((n & 0x3F) << 8) | *d.get(off + 1)? as usize;
            continue;
        }
        out.push(String::from_utf8_lossy(d.get(off + 1..off + 1 + n)?).into_owned());
        off += 1 + n;
    }
    None // pointer loop
}

/// The first label of the first PTR answer: "MacBook-Pro-de-Alex".
fn ptr_answer(d: &[u8]) -> Option<String> {
    let count = |i: usize| Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]) as usize);
    let (qd, an) = (count(4)?, count(6)?);
    let mut off = 12;
    for _ in 0..qd {
        off = name_at(d, off)?.1 + 4;
    }
    for _ in 0..an {
        off = name_at(d, off)?.1;
        let (typ, len) = (count(off)?, count(off + 8)?);
        off += 10;
        if typ == 12 {
            return name_at(d, off)?.0.into_iter().next().filter(|n| !n.is_empty());
        }
        off += len;
    }
    None
}

/// Asks every bound lease at once and keeps the answers that come within `wait`.
pub fn refresh(leases: &[Value], wait: Duration) {
    let clients: Vec<(String, String)> = leases.iter().filter(|l| l["status"] == "bound")
        .filter_map(|l| Some((l["address"].as_str()?.to_string(), l["mac-address"].as_str()?.to_uppercase()))).collect();
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return };
    for (i, (ip, _)) in clients.iter().enumerate() {
        let _ = s.send_to(&query(ip, i as u16), (ip.as_str(), 5353));
    }
    let end = Instant::now() + wait;
    let mut buf = [0u8; 1500];
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        let _ = s.set_read_timeout(Some(left));
        let Ok((n, src)) = s.recv_from(&mut buf) else { break };
        let ip = src.ip().to_string();
        if let (Some((_, mac)), Some(name)) = (clients.iter().find(|(i, _)| *i == ip), ptr_answer(&buf[..n])) {
            let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
            if g.insert(mac.clone(), name.clone()).as_ref() != Some(&name) {
                if let (Some(p), Ok(t)) = (FILE.get(), serde_json::to_string_pretty(&*g)) {
                    let tmp = format!("{p}.tmp");
                    if fs::write(&tmp, t).and_then(|_| fs::rename(&tmp, p)).is_err() {
                        eprintln!("bonjour: cannot write {p}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_ptr_answer() {
        // answer to the query for 10.0.0.9, the PTR data compressed onto ".local"
        let mut a = query("10.0.0.9", 7);
        a[2] = 0x84;
        a[7] = 1; // one answer
        a.extend_from_slice(&[0xC0, 12, 0, 12, 0x80, 1, 0, 0, 0, 10]);
        let data: Vec<u8> = [&[13u8][..], b"MacBook-Pro-A", &[5], b"local", &[0]].concat();
        a.extend_from_slice(&(data.len() as u16).to_be_bytes());
        a.extend_from_slice(&data);
        assert_eq!(ptr_answer(&a).as_deref(), Some("MacBook-Pro-A"));
        assert_eq!(ptr_answer(&a[..a.len() - 3]), None, "truncated");
        assert_eq!(ptr_answer(&[0xC0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xC0, 12]), None, "pointer loop");
    }
}
