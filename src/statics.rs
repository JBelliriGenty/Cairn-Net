//! Static DNS names (git mode): a YAML file in the repository holding one list
//! of "IP name" strings, each entry optionally preceded by its comment lines,
//! as a hosts file or a DNS server's local records take them. Edited line by
//! line like dhcp.yml; something else (Ansible, CI) publishes it. The names the
//! DNS servers really answer are read with plain DNS queries (no account).

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub ip: String,
    pub name: String,
    pub comment: String, // the comment lines just above, without "# "
    pub line: usize,
    pub first: usize, // first comment line (= line when none)
}

fn value(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix("- \"")?.strip_suffix('"')
}

fn top_key(l: &str) -> bool {
    !l.is_empty() && !l.starts_with([' ', '#', '-']) && l.trim_end().ends_with(':')
}

/// The entries of the first top-level list of the file.
pub fn parse(text: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut in_list = false;
    for (i, l) in lines.iter().enumerate() {
        if top_key(l) {
            if in_list {
                break;
            }
            in_list = true;
            continue;
        }
        let Some(v) = value(l).filter(|_| in_list) else { continue };
        let (ip, name) = v.split_once(' ').unwrap_or((v, ""));
        let mut first = i;
        while first > 0 && lines[first - 1].starts_with("  ") && lines[first - 1].trim_start().starts_with('#') {
            first -= 1;
        }
        let comment = lines[first..i].iter().map(|c| c.trim_start().trim_start_matches('#').trim()).collect::<Vec<_>>().join(" ");
        out.push(Entry { ip: ip.trim().into(), name: name.trim().into(), comment, line: i, first });
    }
    out
}

/// A host name: dot-separated DNS labels, 253 characters at most.
pub fn is_fqdn(s: &str) -> bool {
    s.len() <= 253 && s.split('.').all(crate::dhcp::is_name)
}

/// Problems that block a commit. `taken`: (lower-case name, where it is
/// already used) for the DHCP reservations and the lease names.
pub fn check(entries: &[Entry], taken: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeMap::new();
    for e in entries {
        let who = if e.name.is_empty() { e.ip.as_str() } else { e.name.as_str() };
        if e.ip.parse::<Ipv4Addr>().is_err() {
            out.push(format!("{who}: invalid IP address {}", e.ip));
        }
        if !is_fqdn(&e.name) {
            out.push(format!("{who}: invalid name (letters, digits, dashes, dots)"));
        }
        let n = e.name.to_lowercase();
        if seen.insert((n.clone(), e.ip.clone()), ()).is_some() {
            out.push(format!("{who}: listed twice with {}", e.ip));
        }
        if let Some((_, w)) = taken.iter().find(|(t, _)| *t == n) {
            out.push(format!("{who}: already the name of {w}"));
        }
    }
    out
}

/// Changes from the page, an entry being (ip, name): add {ip, name, comment},
/// set {old_ip, old_name, ip, name, comment}, del {ip, name}.
pub fn apply(text: &str, changes: &[Value]) -> Result<String, String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for c in changes {
        let s = |k: &str| c[k].as_str().unwrap_or("").trim().to_string();
        let (ip, name, comment) = (s("ip"), s("name"), s("comment"));
        if [&ip, &name, &comment].iter().any(|v| v.contains(['"', '\\', '\n', '\r'])) || ip.contains(' ') || name.contains(' ') {
            return Err("quotes, backslashes, spaces and line breaks are not allowed".into());
        }
        let entries = parse(&lines.join("\n"));
        let find = |ip: &str, name: &str| entries.iter().find(|e| e.ip == ip && e.name.eq_ignore_ascii_case(name)).cloned();
        let mut new = Vec::new();
        if !comment.is_empty() {
            new.push(format!("  # {comment}"));
        }
        new.push(format!("  - \"{ip} {name}\""));
        match c["op"].as_str() {
            Some("add") => {
                if find(&ip, &name).is_some() {
                    return Err(format!("{name} {ip} is already in the file"));
                }
                let at = entries.last().map(|e| e.line + 1)
                    .or_else(|| lines.iter().position(|l| top_key(l)).map(|i| i + 1))
                    .ok_or("no list in the file")?;
                lines.splice(at..at, new);
            }
            Some("set") => {
                let e = find(&s("old_ip"), &s("old_name")).ok_or_else(|| format!("{} {} is not in the file", s("old_name"), s("old_ip")))?;
                // comment unchanged: its lines stay as written (several lines, group comment)
                let from = if comment == e.comment { e.line } else { e.first };
                lines.splice(from..=e.line, new.split_off(if comment == e.comment { new.len() - 1 } else { 0 }));
            }
            Some("del") => {
                let e = find(&ip, &name).ok_or_else(|| format!("{name} {ip} is not in the file"))?;
                // a comment over a group of entries stays with the next one of the group
                let group = lines.get(e.line + 1).and_then(|l| value(l)).is_some();
                lines.drain(if group { e.line } else { e.first }..=e.line);
            }
            _ => return Err("unknown change".into()),
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

pub fn to_json(e: &Entry) -> Value {
    json!({"ip": e.ip, "name": e.name, "comment": e.comment})
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]); // recursion desired, one question
    for l in name.split('.') {
        q.push(l.len() as u8);
        q.extend(l.as_bytes());
    }
    q.extend([0, 0, 1, 0, 1]); // A, IN
    q
}

fn skip_name(b: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let n = *b.get(i)? as usize;
        if n == 0 {
            return Some(i + 1);
        }
        if n & 0xC0 == 0xC0 {
            return Some(i + 2);
        }
        i += n + 1;
    }
}

/// (id, A records) of an answer; empty for NXDOMAIN or no record.
fn answer(b: &[u8]) -> Option<(u16, Vec<String>)> {
    let id = u16::from_be_bytes([*b.first()?, *b.get(1)?]);
    let (qd, an) = (u16::from_be_bytes([*b.get(4)?, *b.get(5)?]), u16::from_be_bytes([*b.get(6)?, *b.get(7)?]));
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(b, i)? + 4;
    }
    let mut ips = Vec::new();
    for _ in 0..an {
        i = skip_name(b, i)?;
        let h = b.get(i..i + 10)?;
        let len = u16::from_be_bytes([h[8], h[9]]) as usize;
        let d = b.get(i + 10..i + 10 + len)?;
        if h[0..4] == [0, 1, 0, 1] && len == 4 {
            ips.push(Ipv4Addr::new(d[0], d[1], d[2], d[3]).to_string());
        }
        i += 10 + len;
    }
    Some((id, ips))
}

/// What `server` (IPv4, port 53) answers for each name: None when it gave no
/// answer within the deadline. All the queries leave at once.
pub fn served(server: &str, names: &[String]) -> Result<Vec<Option<Vec<String>>>, String> {
    let ip: Ipv4Addr = server.parse().map_err(|_| format!("{server}: not an IPv4 address"))?;
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    sock.connect((ip, 53)).map_err(|e| e.to_string())?;
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u16);
    for (i, n) in names.iter().enumerate() {
        sock.send(&query(salt.wrapping_add(i as u16), n)).map_err(|e| e.to_string())?;
    }
    let mut out = vec![None; names.len()];
    let mut left = names.len();
    let end = Instant::now() + Duration::from_millis(1500);
    let mut buf = [0u8; 1500];
    while left > 0 {
        let now = Instant::now();
        if now >= end {
            break;
        }
        sock.set_read_timeout(Some(end - now)).map_err(|e| e.to_string())?;
        let Ok(n) = sock.recv(&mut buf) else { break };
        if let Some((id, ips)) = answer(&buf[..n]) {
            let k = id.wrapping_sub(salt) as usize;
            if k < out.len() && out[k].is_none() {
                out[k] = Some(ips);
                left -= 1;
            }
        }
    }
    Ok(out)
}

/// Per DNS server: {server, answers: {name: [IP] | null}, error}.
pub fn served_all(servers: &[String], entries: &[Entry]) -> Vec<Value> {
    let mut names: Vec<String> = entries.iter().map(|e| e.name.clone()).filter(|n| is_fqdn(n)).collect();
    names.sort_unstable();
    names.dedup();
    let names = &names;
    std::thread::scope(|s| {
        let hs: Vec<_> = servers.iter().map(|srv| s.spawn(move || (srv, served(srv, names)))).collect();
        hs.into_iter().filter_map(|h| h.join().ok()).map(|(srv, r)| match r {
            Ok(a) => json!({"server": srv, "answers": names.iter().cloned().zip(a).collect::<HashMap<_, _>>()}),
            Err(e) => json!({"server": srv, "error": e}),
        }).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "---\n# header\nnames:\n  - \"192.168.1.1 router\"\n  # front door: one per route\n  - \"192.168.1.66 auth\"\n  - \"192.168.1.66 git\"\n  # alone\n  - \"192.168.1.65 loki\"\n\nother:\n  - \"10.0.0.1 nope\"\n";

    #[test]
    fn parses_the_first_list() {
        let e = parse(FILE);
        assert_eq!(e.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["router", "auth", "git", "loki"]);
        assert_eq!((e[1].comment.as_str(), e[1].first, e[2].comment.as_str()), ("front door: one per route", 4, ""));
        assert!(check(&e, &[]).is_empty());
    }

    #[test]
    fn checks() {
        let mut e = parse(FILE);
        e[2].name = "AUTH".into();
        e[2].ip = "192.168.1.66".into();
        e[0].ip = "192.168.1.300".into();
        e[3].name = "bad_name".into();
        let p = check(&e, &[("router".into(), "the reservation of 02:00:00:00:00:01".into())]);
        assert!(p.iter().any(|m| m == "AUTH: listed twice with 192.168.1.66"), "{p:?}");
        assert!(p.iter().any(|m| m.contains("invalid IP")) && p.iter().any(|m| m.contains("invalid name")), "{p:?}");
        assert!(p.iter().any(|m| m == "router: already the name of the reservation of 02:00:00:00:00:01"), "{p:?}");
        assert!(is_fqdn("cloud.example.org") && is_fqdn("traefik.front") && !is_fqdn("a..b") && !is_fqdn("-a"));
    }

    #[test]
    fn edits_keep_the_rest() {
        let add = apply(FILE, &[json!({"op": "add", "ip": "192.168.1.70", "name": "car", "comment": "new"})]).unwrap();
        assert!(add.contains("loki\"\n  # new\n  - \"192.168.1.70 car\"\n\nother:"), "{add}");
        let set = apply(FILE, &[json!({"op": "set", "old_ip": "192.168.1.65", "old_name": "loki", "ip": "192.168.1.65", "name": "logs", "comment": "renamed"})]).unwrap();
        assert!(set.contains("  # renamed\n  - \"192.168.1.65 logs\"\n") && !set.contains("alone"), "{set}");
        let set = apply(FILE, &[json!({"op": "set", "old_ip": "192.168.1.66", "old_name": "auth", "ip": "192.168.1.67", "name": "auth", "comment": "front door: one per route"})]).unwrap();
        assert!(set.contains("  # front door: one per route\n  - \"192.168.1.67 auth\"\n  - \"192.168.1.66 git\""), "{set}");
        // deleting the first of a group keeps the group's comment for the next one
        let del = apply(FILE, &[json!({"op": "del", "ip": "192.168.1.66", "name": "auth"})]).unwrap();
        assert!(del.contains("  # front door: one per route\n  - \"192.168.1.66 git\""), "{del}");
        let del = apply(FILE, &[json!({"op": "del", "ip": "192.168.1.65", "name": "loki"})]).unwrap();
        assert!(!del.contains("alone") && del.contains("other:\n  - \"10.0.0.1 nope\""), "{del}");
        assert!(apply(FILE, &[json!({"op": "add", "ip": "192.168.1.1", "name": "router"})]).is_err());
        assert!(apply(FILE, &[json!({"op": "add", "ip": "1.2.3.4", "name": "a\"b"})]).is_err());
        assert_eq!(apply("names:\n", &[json!({"op": "add", "ip": "1.2.3.4", "name": "a"})]).unwrap(), "names:\n  - \"1.2.3.4 a\"\n");
    }

    #[test]
    fn dns_answer() {
        let q = query(7, "loki");
        assert_eq!(&q[12..18], b"\x04loki\x00");
        // the query echoed back as an answer: header (1 question, 1 answer) + question + A record via a pointer
        let mut r = q.clone();
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 1;
        r.extend([0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 168, 1, 65]);
        assert_eq!(answer(&r), Some((7, vec!["192.168.1.65".to_string()])));
    }
}
