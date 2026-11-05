//! DHCP servers of the router (`dhcp_servers` in the DHCP file). Same rule as the reservations: the file is edited as
//! text, one server block at a time; the comment lines above a server and
//! above each route are kept. Fixed layout, written by `render`:
//!
//!   - name: lan
//!     enabled: false
//!     interface: bridge
//!     ...
//!     alert: true          (only when on: watch for other DHCP servers)
//!     alert_valid: [MAC]   (only when some: DHCP servers that are expected)
//!     routes:
//!       # comment
//!       - "0.0.0.0/0,192.168.1.1"
//!     options:            (only when there are some)
//!       - "66,'tftp.example'"

use crate::dhcp::{in_net, is_name};
use serde_json::{json, Value};
use std::net::Ipv4Addr;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Server {
    pub name: String,
    pub enabled: bool,
    pub interface: String,
    pub network: String,
    pub range: String,
    pub gateway: String,
    pub dns: Vec<String>,
    pub domain: String,
    pub ntp: Vec<String>,
    pub lease_time: String,
    pub alert: bool,              // RouterOS rogue DHCP server alert on the interface
    pub alert_valid: Vec<String>, // MACs of the other DHCP servers that are expected
    pub routes: Vec<(String, Vec<String>)>, // "dst,gw" and the comment lines above it
    pub options: Vec<(String, Vec<String>)>, // "code,value" (RouterOS syntax), same
    pub comment: String,
    pub(crate) line: usize, // "  - name:" line
    pub(crate) end: usize,  // first line after the block
}

fn scalar(v: &str) -> String {
    v.trim().trim_matches('"').to_string()
}

fn flow(v: &str) -> Vec<String> {
    v.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(scalar).filter(|s| !s.is_empty()).collect()
}

fn block(lines: &[String]) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.starts_with("dhcp_servers:"))? + 1;
    let end = (start..lines.len()).find(|&i| !lines[i].is_empty() && !lines[i].starts_with(' ') && !lines[i].starts_with('#')).unwrap_or(lines.len());
    Some((start, end))
}

pub fn parse(lines: &[String]) -> Vec<Server> {
    let Some((start, end)) = block(lines) else { return vec![] };
    let mut out: Vec<Server> = Vec::new();
    let mut pending: Vec<String> = Vec::new(); // comment lines waiting for their item
    let mut in_list = ""; // "routes" or "options" while in their items
    for i in start..end {
        let l = &lines[i];
        let t = l.trim_start();
        if let Some(n) = l.strip_prefix("  - name:") {
            if let Some(p) = out.last_mut() {
                p.end = i - pending.len();
            }
            let comment = pending.iter().map(|c| c.trim_start().trim_start_matches('#').trim()).collect::<Vec<_>>().join(" ");
            out.push(Server { name: scalar(n), comment, line: i, end, ..Default::default() });
            pending.clear();
            in_list = "";
        } else if t.starts_with('#') {
            pending.push(l.clone());
        } else if let (Some(s), Some(v)) = (out.last_mut(), t.strip_prefix("- ").filter(|_| !in_list.is_empty())) {
            let item = (scalar(v), std::mem::take(&mut pending));
            if in_list == "routes" { s.routes.push(item) } else { s.options.push(item) }
        } else if let (Some(s), Some((k, v))) = (out.last_mut(), t.split_once(':')) {
            pending.clear();
            in_list = match k {
                "routes" if v.trim().is_empty() => "routes",
                "options" if v.trim().is_empty() => "options",
                _ => "",
            };
            match k {
                "enabled" => s.enabled = v.trim() == "true",
                "interface" => s.interface = scalar(v),
                "network" => s.network = scalar(v),
                "range" => s.range = scalar(v),
                "gateway" => s.gateway = scalar(v),
                "dns" => s.dns = flow(v),
                "domain" => s.domain = scalar(v),
                "ntp" => s.ntp = flow(v),
                "lease_time" => s.lease_time = scalar(v),
                "alert" => s.alert = v.trim() == "true",
                "alert_valid" => s.alert_valid = flow(v),
                _ => {}
            }
        }
    }
    // the last block stops at its last indented line (not the blank lines after)
    if let Some(s) = out.last_mut() {
        s.end = (s.line..s.end).rev().find(|&i| lines[i].starts_with("    ")).map_or(s.end, |i| i + 1);
    }
    out
}

pub fn render(s: &Server) -> Vec<String> {
    let list = |v: &[String]| format!("[{}]", v.join(", "));
    let mut out = vec![
        format!("  - name: {}", s.name),
        format!("    enabled: {}", s.enabled),
        format!("    interface: {}", s.interface),
        format!("    network: {}", s.network),
        format!("    range: {}", s.range),
        format!("    gateway: {}", s.gateway),
        format!("    dns: {}", list(&s.dns)),
        format!("    domain: {}", if s.domain.is_empty() { "\"\"".to_string() } else { s.domain.clone() }),
        format!("    ntp: {}", list(&s.ntp)),
        format!("    lease_time: {}", s.lease_time),
    ];
    // absent when off: files written before the alert existed stay as they are
    if s.alert {
        out.push("    alert: true".into());
        if !s.alert_valid.is_empty() {
            out.push(format!("    alert_valid: {}", list(&s.alert_valid)));
        }
    }
    let items = |out: &mut Vec<String>, key: &str, v: &[(String, Vec<String>)]| {
        out.push(format!("    {key}:"));
        for (r, c) in v {
            out.extend(c.iter().cloned());
            out.push(format!("      - \"{r}\""));
        }
    };
    if s.routes.is_empty() {
        out.push("    routes: []".into());
    } else {
        items(&mut out, "routes", &s.routes);
    }
    // absent when empty: files written before options existed stay as they are
    if !s.options.is_empty() {
        items(&mut out, "options", &s.options);
    }
    out
}

/// Only these characters reach the file: no quoting or YAML syntax to escape.
fn safe(v: &str) -> bool {
    v.chars().all(|c| c.is_ascii_alphanumeric() || ".:/-_,".contains(c))
}

fn from_json(c: &Value, old: Option<&Server>) -> Result<Server, String> {
    let s = |k: &str| c[k].as_str().unwrap_or("").trim().to_string();
    let l = |k: &str| c[k].as_array().map(|a| a.iter().filter_map(|x| Some(x.as_str()?.trim().to_string())).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let routes: Vec<String> = l("routes");
    let options: Vec<String> = l("options");
    // an item kept as it was keeps its comment
    let keep = |v: &[String], old: Option<&Vec<(String, Vec<String>)>>| -> Vec<(String, Vec<String>)> {
        v.iter().map(|r| (r.clone(), old.and_then(|o| o.iter().find(|(x, _)| x == r)).map(|(_, c)| c.clone()).unwrap_or_default())).collect()
    };
    let mut new = Server {
        // a new name in srv_set renames the server
        name: Some(s("name")).filter(|n| !n.is_empty()).or_else(|| old.map(|o| o.name.clone())).unwrap_or_default(),
        enabled: c["enabled"].as_bool().unwrap_or(false),
        interface: s("interface"),
        network: s("network"),
        range: s("range"),
        gateway: s("gateway"),
        dns: l("dns"),
        domain: s("domain"),
        ntp: l("ntp"),
        lease_time: s("lease_time"),
        alert: c["alert"].as_bool().unwrap_or(false),
        alert_valid: l("alert_valid").into_iter().map(|m| m.to_uppercase()).collect(),
        routes: keep(&routes, old.map(|o| &o.routes)),
        options: keep(&options, old.map(|o| &o.options)),
        ..Default::default()
    };
    let all = [&new.name, &new.interface, &new.network, &new.range, &new.gateway, &new.domain, &new.lease_time].into_iter().cloned()
        .chain(new.dns.iter().cloned()).chain(new.ntp.iter().cloned()).chain(new.alert_valid.iter().cloned()).chain(routes);
    if !all.into_iter().all(|v| safe(&v)) {
        return Err("only letters, digits and . : / - _ , are allowed".into());
    }
    // option values are RouterOS text ('quoted'): `check` judges them, only what would break the file is refused here
    if options.iter().any(|o| o.contains(['"', '\\', '\n'])) {
        return Err("quotes, backslashes and line breaks are not allowed".into());
    }
    if let Some(o) = old {
        new.comment = o.comment.clone();
    }
    Ok(new)
}

/// srv_add {server…, comment}, srv_set {name, server…} (server.name may
/// differ: rename) or srv_del {name} (with the comment lines above it).
pub fn apply_one(lines: &[String], c: &Value) -> Result<Vec<String>, String> {
    let mut lines = lines.to_vec();
    let servers = parse(&lines);
    match c["op"].as_str() {
        Some("srv_set") => {
            let name = c["name"].as_str().unwrap_or("");
            let o = servers.iter().find(|s| s.name == name).ok_or_else(|| format!("{name} is not a server"))?;
            let new = from_json(&c["server"], Some(o))?;
            if new.name != o.name && servers.iter().any(|s| s.name == new.name) {
                return Err(format!("{} is already a server", new.name));
            }
            lines.splice(o.line..o.end, render(&new));
        }
        Some("srv_del") => {
            let name = c["name"].as_str().unwrap_or("");
            let o = servers.iter().find(|s| s.name == name).ok_or_else(|| format!("{name} is not a server"))?;
            let first = (0..o.line).rev().take_while(|&i| lines[i].starts_with("  #")).last().unwrap_or(o.line);
            lines.drain(first..o.end);
        }
        Some("srv_add") => {
            let new = from_json(&c["server"], None)?;
            if servers.iter().any(|s| s.name == new.name) {
                return Err(format!("{} is already a server", new.name));
            }
            let comment = c["comment"].as_str().unwrap_or("").trim();
            if comment.contains(['\n', '\\', '"']) {
                return Err("quotes, backslashes and line breaks are not allowed".into());
            }
            let (_, end) = block(&lines).ok_or("no dhcp_servers list")?;
            // after the last server, before the blank lines that end the block
            let at = servers.last().map(|s| s.end).unwrap_or(end).min(end);
            let at = (0..at).rev().find(|&i| !lines[i].trim().is_empty()).map_or(at, |i| i + 1);
            let mut add = if comment.is_empty() { vec![] } else { vec![format!("  # {comment}")] };
            add.extend(render(&new));
            lines.splice(at..at, add);
        }
        _ => return Err("unknown change".into()),
    }
    Ok(lines)
}

fn ip(v: &str) -> Option<Ipv4Addr> {
    v.parse().ok()
}

fn net_ok(n: &str) -> bool {
    n.split_once('/').is_some_and(|(a, l)| ip(a).is_some() && l.parse::<u8>().is_ok_and(|l| l <= 32))
}

/// RouterOS option value: 0x… hex, a number, 'text' or s'text' (an address
/// in plain quotes is sent as 4 bytes by RouterOS, s'…' always as text).
pub fn option_value_ok(v: &str) -> bool {
    let txt = |t: &str| t.chars().all(|c| c.is_ascii_alphanumeric() || " .:/_-,@=?&%+~".contains(c));
    if let Some(h) = v.strip_prefix("0x") {
        return !h.is_empty() && h.len() % 2 == 0 && h.chars().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(t) = v.strip_prefix("s'").or_else(|| v.strip_prefix('\'')).and_then(|t| t.strip_suffix('\'')) {
        return txt(t);
    }
    !v.is_empty() && v.len() <= 10 && v.chars().all(|c| c.is_ascii_digit())
}

/// Codes the server sets from its own fields, or RouterOS itself.
const OWN_CODES: [u8; 9] = [1, 3, 6, 15, 42, 51, 53, 54, 121];

fn lease_ok(v: &str) -> bool {
    let units = !v.is_empty() && v.split_inclusive(['w', 'd', 'h', 'm', 's']).all(|p| p.len() > 1 && p[..p.len() - 1].chars().all(|c| c.is_ascii_digit()) && p.ends_with(['w', 'd', 'h', 'm', 's']));
    let clock = v.split(':').count() == 3 && v.split(':').all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()));
    units || clock
}

/// Problems that block an apply, "<server>: <problem>" (translated by the page).
pub fn check(servers: &[Server]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, s) in servers.iter().enumerate() {
        let who = &s.name;
        let mut p = |m: &str| out.push(format!("{who}: {m}"));
        if !is_name(&s.name) {
            p("invalid server name (letters, digits, dashes)");
        }
        let before = &servers[..i];
        if before.iter().any(|o| o.name == s.name) {
            p("same name as another server");
        }
        if !is_name(&s.interface.replace('_', "-")) {
            p("invalid interface");
        } else if before.iter().any(|o| o.interface == s.interface) {
            p("same interface as another server");
        }
        let inside = |v: &str| ip(v).is_some_and(|a| in_net(a, &s.network));
        if !net_ok(&s.network) {
            p("invalid network (address/length)");
        } else {
            if before.iter().any(|o| o.network == s.network) {
                p("same network as another server");
            }
            let range_ok = !s.range.is_empty() && s.range.split(',').all(|r| {
                let (a, b) = r.split_once('-').unwrap_or((r, r));
                inside(a) && inside(b) && ip(a) <= ip(b)
            });
            if !range_ok {
                p("invalid range (first-last, inside the network)");
            }
            if !inside(&s.gateway) {
                p("invalid gateway (an address of the network)");
            }
        }
        if s.dns.iter().any(|d| ip(d).is_none()) {
            p("invalid DNS server address");
        }
        if s.ntp.iter().any(|d| ip(d).is_none()) {
            p("invalid NTP server address");
        }
        if !s.domain.is_empty() && !s.domain.split('.').all(is_name) {
            p("invalid domain");
        }
        if !lease_ok(&s.lease_time) {
            p("invalid lease time (e.g. 1d, 12h, 30m)");
        }
        if s.routes.iter().any(|(r, _)| !r.split_once(',').is_some_and(|(d, g)| net_ok(d) && ip(g).is_some())) {
            p("invalid route (network/length,gateway)");
        } else if !s.routes.is_empty() && !s.routes.iter().any(|(r, _)| r.starts_with("0.0.0.0/0,")) {
            p("routes without a default route (0.0.0.0/0): clients would lose Internet");
        }
        if s.alert_valid.iter().any(|m| !crate::dhcp::is_mac(m)) {
            p("invalid MAC address of an expected DHCP server");
        }
        let mut codes = Vec::new();
        for (o, _) in &s.options {
            let (code, val) = o.split_once(',').unwrap_or((o, ""));
            match code.trim().parse::<u8>() {
                Ok(c) if c == 0 || c == 255 => p("invalid option code (1-254)"),
                Ok(c) if OWN_CODES.contains(&c) => p("option set by its own field or by RouterOS (1, 3, 6, 15, 42, 51, 53, 54, 121)"),
                Ok(c) if codes.contains(&c) => p("same option code twice"),
                Ok(c) => codes.push(c),
                Err(_) => p("invalid option code (1-254)"),
            }
            if !option_value_ok(val.trim()) {
                p("invalid option value (0x…, a number, 'text' or s'text')");
            }
        }
    }
    out
}

pub fn to_json(s: &Server) -> Value {
    json!({"name": s.name, "enabled": s.enabled, "interface": s.interface, "network": s.network, "range": s.range,
        "gateway": s.gateway, "dns": s.dns, "domain": s.domain, "ntp": s.ntp, "lease_time": s.lease_time,
        "alert": s.alert, "alert_valid": s.alert_valid,
        "routes": s.routes.iter().map(|(r, _)| r).collect::<Vec<_>>(),
        "options": s.options.iter().map(|(r, _)| r).collect::<Vec<_>>(), "comment": s.comment})
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "dhcp_reservations:\n  - \"02:00:00:00:00:21,192.168.1.121,PRINTER\"\n\n# servers\ndhcp_servers:\n  # LAN: off\n  - name: lan\n    enabled: false\n    interface: bridge\n    network: 192.168.1.0/24\n    range: 192.168.1.100-192.168.1.150\n    gateway: 192.168.1.1\n    dns: [192.168.1.2, 192.168.1.80]\n    domain: home.arpa\n    ntp: []\n    lease_time: 1d\n    routes:\n      # switches\n      - \"192.168.99.0/24,192.168.1.11\"\n      # Internet\n      - \"0.0.0.0/0,192.168.1.1\"\n  # IoT\n  - name: iot\n    enabled: true\n    interface: iot20\n    network: 192.168.20.0/24\n    range: 192.168.20.100-192.168.20.199\n    gateway: 192.168.20.1\n    dns: [192.168.1.2, 192.168.1.80]\n    domain: \"\"\n    ntp: [192.168.20.1]\n    lease_time: 1d\n    routes: []\n";

    fn lines(t: &str) -> Vec<String> {
        t.lines().map(String::from).collect()
    }

    #[test]
    fn parses_and_renders_back() {
        let l = lines(FILE);
        let s = parse(&l);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].name.as_str(), s[0].enabled, s[0].comment.as_str(), s[0].routes.len()), ("lan", false, "LAN: off", 2));
        assert_eq!((s[1].domain.as_str(), s[1].ntp.clone(), s[1].routes.len()), ("", vec!["192.168.20.1".to_string()], 0));
        assert!(check(&s).is_empty(), "{:?}", check(&s));
        // rendering a parsed server gives the same lines: edits only touch what changed
        assert_eq!(render(&s[0]), l[s[0].line..s[0].end].to_vec());
        assert_eq!(render(&s[1]), l[s[1].line..s[1].end].to_vec());
    }

    #[test]
    fn set_keeps_comments_and_add_appends() {
        let l = lines(FILE);
        let mut v = to_json(&parse(&l)[0]);
        v["enabled"] = json!(true);
        v["routes"] = json!(["0.0.0.0/0,192.168.1.1"]);
        let out = apply_one(&l, &json!({"op": "srv_set", "name": "lan", "server": v})).unwrap().join("\n");
        assert!(out.contains("  # LAN: off\n  - name: lan\n    enabled: true") && out.contains("      # Internet\n      - \"0.0.0.0/0") && !out.contains("# switches"), "{out}");
        assert!(out.contains("  # IoT\n  - name: iot"));
        let mut g = to_json(&parse(&l)[1]);
        g["name"] = json!("guest");
        g["interface"] = json!("guest30");
        g["network"] = json!("192.168.30.0/24");
        let out = apply_one(&l, &json!({"op": "srv_add", "server": g, "comment": "Guests"})).unwrap();
        let txt = out.join("\n");
        assert!(txt.contains("    routes: []\n  # Guests\n  - name: guest\n    enabled: true\n    interface: guest30") && txt.ends_with("routes: []"), "{txt}");
        let s = parse(&out);
        assert_eq!(s.len(), 3);
        let p = check(&s);
        assert!(p.iter().any(|m| m.starts_with("guest: invalid range")) && p.iter().any(|m| m.starts_with("guest: invalid gateway")), "{p:?}");
        assert!(apply_one(&l, &json!({"op": "srv_add", "server": {"name": "x\"y"}})).is_err());
    }

    #[test]
    fn options_rename_and_delete() {
        let l = lines(FILE);
        let mut v = to_json(&parse(&l)[1]);
        v["name"] = json!("things");
        v["options"] = json!(["66,s'tftp.example'", "252,'http://wpad.example/wpad.dat'"]);
        let out = apply_one(&l, &json!({"op": "srv_set", "name": "iot", "server": v})).unwrap();
        let txt = out.join("\n");
        assert!(txt.contains("  # IoT\n  - name: things\n") && txt.ends_with("    routes: []\n    options:\n      - \"66,s'tftp.example'\"\n      - \"252,'http://wpad.example/wpad.dat'\""), "{txt}");
        let s = parse(&out);
        assert_eq!(s[1].options.len(), 2);
        assert!(check(&s).is_empty(), "{:?}", check(&s));
        assert_eq!(render(&s[1]), out[s[1].line..s[1].end].to_vec());
        // renaming onto another server is refused
        v["name"] = json!("lan");
        assert!(apply_one(&l, &json!({"op": "srv_set", "name": "iot", "server": v})).is_err());
        // delete takes the comment above, and nothing else
        let out = apply_one(&l, &json!({"op": "srv_del", "name": "lan"})).unwrap().join("\n");
        assert!(out.contains("dhcp_servers:\n  # IoT\n  - name: iot") && !out.contains("LAN: off") && !out.contains("# switches"), "{out}");
        let out = apply_one(&l, &json!({"op": "srv_del", "name": "iot"})).unwrap().join("\n");
        assert!(out.ends_with("      - \"0.0.0.0/0,192.168.1.1\""), "{out}");
        let mut s = parse(&l);
        s[1].options = vec![("6,'1.1.1.1'".into(), vec![]), ("66,tftp".into(), vec![]), ("300,1".into(), vec![]), ("67,'a'".into(), vec![]), ("67,0x6162".into(), vec![])];
        let p = check(&s);
        for m in ["iot: option set by its own field", "iot: invalid option value", "iot: invalid option code", "iot: same option code twice"] {
            assert!(p.iter().any(|x| x.starts_with(m)), "{m} {p:?}");
        }
        // the alert: written only when on, read back the same
        let mut v = to_json(&parse(&l)[1]);
        v["alert"] = json!(true);
        v["alert_valid"] = json!(["aa:bb:cc:dd:ee:ff"]);
        let out = apply_one(&l, &json!({"op": "srv_set", "name": "iot", "server": v})).unwrap();
        assert!(out.join("\n").contains("    lease_time: 1d\n    alert: true\n    alert_valid: [AA:BB:CC:DD:EE:FF]\n    routes: []"));
        let s = parse(&out);
        assert!(s[1].alert && check(&s).is_empty());
        assert_eq!(render(&s[1]), out[s[1].line..s[1].end].to_vec());
        v["alert_valid"] = json!(["nope"]);
        let s = parse(&apply_one(&l, &json!({"op": "srv_set", "name": "iot", "server": v})).unwrap());
        assert!(check(&s).iter().any(|m| m.starts_with("iot: invalid MAC address of an expected")));
        assert!(option_value_ok("0x0a00") && option_value_ok("42") && option_value_ok("'10.0.0.1'") && !option_value_ok("0xabc") && !option_value_ok("'$x'") && !option_value_ok("s'a'b'"));
    }

    #[test]
    fn checks() {
        let mut s = parse(&lines(FILE));
        s[1].routes = vec![("192.168.99.0/24,192.168.20.1".into(), vec![])];
        s[1].lease_time = "1 day".into();
        s[1].interface = "bridge".into();
        let p = check(&s);
        for m in ["iot: routes without a default", "iot: invalid lease time", "iot: same interface"] {
            assert!(p.iter().any(|x| x.starts_with(m)), "{m} {p:?}");
        }
        assert!(lease_ok("1d") && lease_ok("1d12h") && lease_ok("00:10:00") && !lease_ok("d") && !lease_ok("10"));
    }
}
