//! DHCP reservations: the source of truth is the
//! repository's `dhcp.yml`, a YAML list of "MAC,IP,name" strings, each entry
//! optionally preceded by its comment lines. The file is edited as text, line
//! by line, so comments and layout outside the touched entry stay as they are.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub mac: String,
    pub ip: String,
    pub name: String,
    pub comment: String, // the comment lines just above, without "# "
    pub line: usize,     // index of the "- "…"" line
    pub first: usize,    // index of its first comment line (= line when none)
}

fn entry_value(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("- \"")?.strip_suffix('"')
}

pub fn parse(text: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut in_list = false;
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("dhcp_reservations:") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        if !l.is_empty() && !l.starts_with(' ') && !l.starts_with('#') {
            break; // next top-level key (dhcp_servers…)
        }
        let Some(v) = entry_value(l) else { continue };
        let f: Vec<&str> = v.split(',').collect();
        let mut first = i;
        while first > 0 && lines[first - 1].trim_start().starts_with('#') && lines[first - 1].starts_with("  ") {
            first -= 1;
        }
        let comment = lines[first..i].iter().map(|c| c.trim_start().trim_start_matches('#').trim()).collect::<Vec<_>>().join(" ");
        out.push(Entry {
            mac: f.first().unwrap_or(&"").trim().to_uppercase(),
            ip: f.get(1).unwrap_or(&"").trim().to_string(),
            name: f.get(2..).map(|r| r.join(",")).unwrap_or_default().trim().to_string(),
            comment,
            line: i,
            first,
        });
    }
    out
}

pub fn is_mac(s: &str) -> bool {
    let p: Vec<&str> = s.split(':').collect();
    p.len() == 6 && p.iter().all(|x| x.len() == 2 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Host names as DNS labels: letters, digits, dashes (not at the ends), 1-63.
pub fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 63 && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-') && b[0] != b'-' && b[b.len() - 1] != b'-'
}

pub fn in_net(ip: Ipv4Addr, net: &str) -> bool {
    let Some((a, l)) = net.split_once('/') else { return false };
    let (Ok(a), Ok(l)) = (a.parse::<Ipv4Addr>(), l.parse::<u32>()) else { return false };
    let mask = if l == 0 { 0 } else { u32::MAX << (32 - l.min(32)) };
    u32::from(ip) & mask == u32::from(a) & mask
}

/// Problems that block an apply (message per entry).
pub fn check(entries: &[Entry], networks: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for e in entries {
        let who = if e.name.is_empty() { e.mac.as_str() } else { e.name.as_str() };
        if !is_mac(&e.mac) {
            out.push(format!("{who}: invalid MAC address {}", e.mac));
        }
        match e.ip.parse::<Ipv4Addr>() {
            Err(_) => out.push(format!("{who}: invalid IP address {}", e.ip)),
            Ok(ip) if !networks.iter().any(|n| in_net(ip, n)) => out.push(format!("{who}: {} is outside {}", e.ip, networks.join(", "))),
            _ => {}
        }
        if !is_name(&e.name) {
            out.push(format!("{who}: invalid name (letters, digits, dashes)"));
        }
        for (kind, k) in [("MAC", e.mac.clone()), ("IP", e.ip.clone()), ("name", e.name.to_lowercase())] {
            if let Some(other) = seen.insert(format!("{kind}|{k}"), who) {
                out.push(format!("{who}: same {kind} as {other}"));
            }
        }
    }
    out
}

/// One change from the page: add, set (by current MAC), del (by MAC).
pub fn apply(text: &str, changes: &[Value]) -> Result<String, String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for c in changes {
        if c["op"].as_str().is_some_and(|o| o.starts_with("srv_")) {
            lines = crate::servers::apply_one(&lines, c)?;
            continue;
        }
        let s = |k: &str| c[k].as_str().unwrap_or("").trim().to_string();
        let (mac, ip, name, comment) = (s("mac").to_uppercase(), s("ip"), s("name"), s("comment"));
        if [&mac, &ip, &name, &comment].iter().any(|v| v.contains('"') || v.contains('\n') || v.contains('\\')) {
            return Err("quotes, backslashes and line breaks are not allowed".into());
        }
        let text_now = lines.join("\n");
        let entries = parse(&text_now);
        let find = |m: &str| entries.iter().find(|e| e.mac == m.to_uppercase()).cloned();
        let line = format!("  - \"{mac},{ip},{name}\"");
        match c["op"].as_str() {
            Some("add") => {
                if find(&mac).is_some() {
                    return Err(format!("{mac} is already reserved"));
                }
                let at = entries.last().map(|e| e.line + 1)
                    .or_else(|| lines.iter().position(|l| l.starts_with("dhcp_reservations:")).map(|i| i + 1))
                    .ok_or("no dhcp_reservations list")?;
                let mut new = Vec::new();
                if !comment.is_empty() {
                    new.push(format!("  # {comment}"));
                }
                new.push(line);
                lines.splice(at..at, new);
            }
            Some("set") => {
                let e = find(&s("old_mac")).ok_or_else(|| format!("{} is not reserved", s("old_mac")))?;
                lines[e.line] = line;
            }
            Some("del") => {
                let e = find(&mac).ok_or_else(|| format!("{mac} is not reserved"))?;
                lines.drain(e.first..=e.line);
            }
            _ => return Err("unknown change".into()),
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// A name as a DNS label: lower case, common accents dropped, any other
/// character run becomes one dash ("Meross Smart Plug" -> meross-smart-plug);
/// None when nothing valid is left (a DNS server may drop names it already
/// has records for).
pub fn lease_name(host: &str) -> Option<String> {
    let mut n = String::new();
    for c in host.trim().to_lowercase().chars() {
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            n.push(c);
        } else if !n.is_empty() && !n.ends_with('-') {
            n.push('-');
        }
    }
    let n = n.trim_end_matches('-').to_string();
    is_name(&n).then_some(n)
}

/// "IP name" lines, sorted, one per name. Name of a lease: the one set by
/// hand for its MAC (`manual`, upper-case MAC -> name), else its reservation
/// ("ansible: NAME" comment), else its Bonjour name (`bonjour`, same keys),
/// else the client's host-name. A name taken by a lower IP gets -2, -3...
pub fn names_file(leases: &[Value], servers: &[String], manual: &BTreeMap<String, String>, bonjour: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let name = |l: &Value| {
        let mac = l["mac-address"].as_str().unwrap_or("").to_uppercase();
        let resa = l["comment"].as_str().and_then(|c| c.strip_prefix("ansible:"));
        [manual.get(&mac).map(String::as_str), resa, bonjour.get(&mac).map(String::as_str), l["host-name"].as_str()]
            .into_iter().flatten().find_map(lease_name)
    };
    let mut out: Vec<(Ipv4Addr, String)> = leases.iter()
        .filter(|l| (servers.is_empty() || servers.iter().any(|s| l["server"] == s.as_str())) && l["status"] == "bound")
        .filter_map(|l| Some((l["address"].as_str()?.parse().ok()?, name(l)?)))
        .collect();
    out.sort();
    let mut seen = std::collections::BTreeSet::new();
    out.into_iter().filter_map(|(ip, n)| {
        let n = (1..).map(|i| if i == 1 { n.clone() } else { format!("{n}-{i}") }).find(|c| !seen.contains(c))?;
        is_name(&n).then(|| {
            seen.insert(n.clone());
            (ip.to_string(), n)
        })
    }).collect()
}

pub fn to_json(e: &Entry) -> Value {
    json!({"mac": e.mac, "ip": e.ip, "name": e.name, "comment": e.comment})
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "---\n# header\ndhcp_reservations:\n  - \"02:00:00:00:00:21,192.168.1.121,PRINTER\"\n  # DESKTOP (office)\n  # second line\n  - \"BC:24:11:35:B4:8A,192.168.1.120,DESKTOP\"\n";

    #[test]
    fn parses_entries_and_comments() {
        let e = parse(FILE);
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].name.as_str(), e[0].comment.as_str()), ("PRINTER", ""));
        assert_eq!((e[1].ip.as_str(), e[1].comment.as_str(), e[1].first), ("192.168.1.120", "DESKTOP (office) second line", 4));
        let more = format!("{FILE}\ndhcp_servers:\n  - name: lan\n    routes:\n      - \"0.0.0.0/0,192.168.1.1\"\n");
        assert_eq!(parse(&more).len(), 2, "routes are not reservations");
        assert!(check(&e, &["192.168.1.0/24".into()]).is_empty());
    }

    #[test]
    fn checks() {
        let mut e = parse(FILE);
        e[1].ip = "192.168.1.121".into();
        e[0].mac = "zz".into();
        let p = check(&e, &["192.168.1.0/24".into()]);
        assert!(p.iter().any(|m| m.contains("same IP")), "{p:?}");
        assert!(p.iter().any(|m| m.contains("invalid MAC")), "{p:?}");
        e[1].ip = "10.0.0.1".into();
        assert!(check(&e, &["192.168.1.0/24".into()]).iter().any(|m| m.contains("outside")));
        assert!(!is_name("-a") && !is_name("a b") && is_name("BALISA-CLIENT"));
    }

    #[test]
    fn edits_keep_the_rest() {
        let add = apply(FILE, &[json!({"op": "add", "mac": "aa:bb:cc:dd:ee:ff", "ip": "192.168.1.7", "name": "X", "comment": "new one"})]).unwrap();
        assert!(add.ends_with("DESKTOP\"\n  # new one\n  - \"AA:BB:CC:DD:EE:FF,192.168.1.7,X\"\n"), "{add}");
        let set = apply(FILE, &[json!({"op": "set", "old_mac": "02:00:00:00:00:21", "mac": "02:00:00:00:00:21", "ip": "192.168.1.122", "name": "PRINTER"})]).unwrap();
        assert!(set.contains("192.168.1.122,PRINTER") && set.contains("# DESKTOP"));
        let del = apply(FILE, &[json!({"op": "del", "mac": "BC:24:11:35:B4:8A"})]).unwrap();
        assert_eq!(del, "---\n# header\ndhcp_reservations:\n  - \"02:00:00:00:00:21,192.168.1.121,PRINTER\"\n");
        assert!(apply(FILE, &[json!({"op": "add", "mac": "02:00:00:00:00:21", "ip": "192.168.1.9", "name": "Y"})]).is_err());
        assert!(apply(FILE, &[json!({"op": "add", "mac": "AA:BB:CC:DD:EE:FF", "ip": "1\"", "name": "Y"})]).is_err());
    }

    #[test]
    fn lease_names_follow_the_timer_rules() {
        let l = |ip: &str, h: &str, st: &str| json!({"server": "lan", "status": st, "address": ip, "host-name": h});
        let n = names_file(&[l("192.168.1.130", "Phone-of-Alex", "bound"), l("192.168.1.131", "Smart Plug_2", "bound"),
            l("192.168.1.132", "Old", "waiting"), l("192.168.1.133", "phone-of-alex", "bound"), l("192.168.1.134", "***", "bound")],
            &["lan".into()], &BTreeMap::new(), &BTreeMap::new());
        let p = |ip: &str, n: &str| (ip.to_string(), n.to_string());
        assert_eq!(n, vec![p("192.168.1.130", "phone-of-alex"), p("192.168.1.131", "smart-plug-2"), p("192.168.1.133", "phone-of-alex-2")]);
    }

    #[test]
    fn manual_then_reservation_then_bonjour_then_host_name() {
        let l = |ip: &str, mac: &str, h: &str, c: &str| json!({"server": "lan", "status": "bound", "address": ip, "mac-address": mac, "host-name": h, "comment": c});
        let manual = BTreeMap::from([("AA:BB:CC:DD:EE:01".to_string(), "Mac de Léa".to_string())]);
        let bonjour = BTreeMap::from([("AA:BB:CC:DD:EE:01".to_string(), "Ignored".to_string()), ("AA:BB:CC:DD:EE:04".to_string(), "MacBook-Pro-de-Leo".to_string())]);
        let n = names_file(&[l("10.0.0.1", "aa:bb:cc:dd:ee:01", "Mac", ""), l("10.0.0.2", "AA:BB:CC:DD:EE:02", "ESP_123456", "ansible: LIGHTS"),
            l("10.0.0.3", "AA:BB:CC:DD:EE:03", "", ""), l("10.0.0.4", "AA:BB:CC:DD:EE:04", "Mac", ""),
            l("10.0.0.10", "AA:BB:CC:DD:EE:10", "Mac", ""), l("10.0.0.9", "AA:BB:CC:DD:EE:09", "Mac", "")], &[], &manual, &bonjour);
        let p = |ip: &str, n: &str| (ip.to_string(), n.to_string());
        assert_eq!(n, vec![p("10.0.0.1", "mac-de-lea"), p("10.0.0.2", "lights"), p("10.0.0.4", "macbook-pro-de-leo"),
            p("10.0.0.9", "mac"), p("10.0.0.10", "mac-2")], "numbered in IP order, not text order");
    }
}
