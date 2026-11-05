//! IP and routing page: addresses, routes and routing tables, ARP, pools,
//! DHCP client, IPv6 (addresses, routes, ND, settings) and /ip settings,
//! read with the read account.
//!
//! Direct mode edits one menu at a time, as the firewall page does: the page
//! sends the static rows it wants (a row with ".id" is an existing one);
//! `plan` turns the difference into remove / set / unset / add commands and a
//! rollback script that undoes them row by row. Only the properties of
//! `Menu::fields` are written. The change runs through main.rs guarded_apply
//! (rollback scheduler, read back through a new connection, confirmation).
//!
//! Moving the address Cairn Net connects to is the Infrastructure page's
//! job (main.rs move_apply): `mgmt_move` spots it and main.rs hands it over.
//! Anything else that could cut Cairn Net's path is refused: the interface
//! holding the switch's address, the network Cairn Net is on, the route the
//! switch answers Cairn Net with, its ARP entries, a DHCP client on that
//! path, a strict reverse-path filter that would drop Cairn Net's packets.

use crate::direct::Cmd;
use crate::firewall::{all, Row};
use crate::infra::{self, finish, q, Path};
use crate::Api;
use serde_json::{json, Value};
use std::net::Ipv4Addr;

pub struct Menu {
    pub key: &'static str,
    pub path: &'static str,
    /// written properties (everything else is read only here)
    pub fields: &'static [&'static str],
    /// properties every row needs
    pub need: &'static [&'static str],
    /// what finds a row just added, for its rollback
    pub keys: &'static [&'static str],
    /// one row, no add/remove (settings)
    pub single: bool,
    /// the menu has dynamic rows (read only)
    pub dynamic: bool,
}

const fn menu(key: &'static str, path: &'static str, fields: &'static [&'static str], need: &'static [&'static str], keys: &'static [&'static str]) -> Menu {
    Menu { key, path, fields, need, keys, single: false, dynamic: true }
}

pub const MENUS: &[Menu] = &[
    menu("address", "/ip/address", &["address", "interface", "comment", "disabled"], &["address", "interface"], &["address", "interface"]),
    menu("route", "/ip/route", &["dst-address", "gateway", "distance", "routing-table", "check-gateway", "scope", "target-scope", "comment", "disabled"],
        &["dst-address"], &["dst-address", "gateway", "routing-table"]),
    menu("table", "/routing/table", &["name", "fib", "comment", "disabled"], &["name"], &["name"]),
    menu("arp", "/ip/arp", &["address", "mac-address", "interface", "published", "comment", "disabled"], &["address", "mac-address", "interface"], &["address", "interface"]),
    Menu { dynamic: false, ..menu("pool", "/ip/pool", &["name", "ranges", "next-pool", "comment"], &["name", "ranges"], &["name"]) },
    menu("dhcp-client", "/ip/dhcp-client", &["interface", "add-default-route", "default-route-distance", "use-peer-dns", "use-peer-ntp", "comment", "disabled"],
        &["interface"], &["interface"]),
    menu("ipv6-address", "/ipv6/address", &["address", "from-pool", "interface", "advertise", "eui-64", "no-dad", "comment", "disabled"], &["interface"], &["address", "interface", "from-pool"]),
    menu("ipv6-route", "/ipv6/route", &["dst-address", "gateway", "distance", "routing-table", "check-gateway", "comment", "disabled"],
        &["dst-address"], &["dst-address", "gateway", "routing-table"]),
    menu("nd", "/ipv6/nd", &["interface", "ra-interval", "ra-lifetime", "ra-preference", "hop-limit", "mtu", "reachable-time", "retransmit-interval",
        "advertise-mac-address", "advertise-dns", "managed-address-configuration", "other-configuration", "disabled"], &["interface"], &["interface"]), // no comment: 7.25 takes it but never prints it
    Menu { single: true, dynamic: false, ..menu("ip-settings", "/ip/settings", &["ip-forward", "rp-filter", "send-redirects", "accept-redirects", "secure-redirects",
        "accept-source-route", "allow-fast-path", "tcp-syncookies", "arp-timeout", "icmp-rate-limit", "icmp-rate-mask", "max-neighbor-entries"], &[], &[]) },
    Menu { single: true, dynamic: false, ..menu("ipv6-settings", "/ipv6/settings", &["disable-ipv6", "forward", "accept-redirects", "accept-router-advertisements",
        "max-neighbor-entries", "min-neighbor-entries", "soft-max-neighbor-entries"], &[], &[]) },
];

pub fn menu_of(key: &str) -> Result<&'static Menu, String> {
    MENUS.iter().find(|m| m.key == key).ok_or_else(|| format!("{key}: unknown menu"))
}

/// Every menu, whole rows (a device without IPv6 reads it as empty), and
/// the DHCP servers' pools (those are edited on the DHCP page).
pub fn read(api: &mut Api) -> Result<Value, String> {
    let mut menus = serde_json::Map::new();
    for m in MENUS {
        menus.insert(m.key.into(), json!(all(api, m.path)?));
    }
    Ok(json!({"menus": menus, "dhcp_servers": crate::print(api, "/ip/dhcp-server", "name,address-pool,interface", &[])?}))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn yes(v: &str) -> bool {
    v == "true" || v == "yes"
}

/// Flags: on when present, whatever the value ("fib=").
const FLAGS: [&str; 1] = ["fib"];

/// The written properties of a row: no empty ones, `disabled` and flags only when on.
fn fields_of(m: &Menu, v: &Value) -> Row {
    m.fields.iter().filter_map(|k| Some((k.to_string(), v[*k].as_str()?.to_string())))
        .map(|(k, x)| if FLAGS.contains(&k.as_str()) && !matches!(x.as_str(), "false" | "no") { (k, "true".into()) } else { (k, x) })
        .filter(|(k, x)| !x.is_empty() && !(k == "disabled" && !yes(x)))
        .map(|(k, x)| if k == "disabled" { (k, "true".into()) } else { (k, x) })
        .collect()
}

/// Values as compared: RouterOS answers true/false and upper-case MACs
/// for what may be typed yes/no and lower case.
fn norm(k: &str, v: &str) -> String {
    let v = if k == "comment" || k == "name" { v.to_string() } else { v.to_lowercase() };
    match v.as_str() {
        "yes" => "true".into(),
        "no" => "false".into(),
        _ => v,
    }
}

fn same(k: &str, a: Option<&String>, b: Option<&String>) -> bool {
    a.map(|x| norm(k, x)) == b.map(|x| norm(k, x))
}

fn rows<'a>(live: &'a Value, m: &Menu) -> Vec<&'a Value> {
    live["menus"][m.key].as_array().into_iter().flatten().collect()
}

fn statics<'a>(live: &'a Value, m: &Menu) -> Vec<&'a Value> {
    rows(live, m).into_iter().filter(|r| !yes(s(r, "dynamic"))).collect()
}

/// What the page wants: one menu's static rows (settings: one row, only the
/// properties to change).
pub struct Want {
    pub menu: &'static Menu,
    pub rows: Vec<(Option<String>, Row)>,
}

impl Want {
    pub fn from_json(v: &Value) -> Result<Want, String> {
        let m = menu_of(s(v, "menu"))?;
        let rows = v["rows"].as_array().ok_or("no rows")?;
        if rows.len() > 2000 || m.single && rows.len() != 1 {
            return Err("invalid rows".into());
        }
        Ok(Want { menu: m, rows: rows.iter().map(|r| (r[".id"].as_str().map(String::from), fields_of(m, r))).collect() })
    }
}

/// "a.b.c.d/len" (len 0 to 32) or "a.b.c.d" -> (network, len).
fn net4(v: &str) -> Option<(u32, u32)> {
    let (a, l) = v.split_once('/').unwrap_or((v, "32"));
    let ip: Ipv4Addr = a.parse().ok()?;
    let len: u32 = l.parse().ok().filter(|l| *l <= 32)?;
    Some((u32::from(ip) & mask(len), len))
}

fn mask(len: u32) -> u32 {
    u32::MAX.checked_shl(32 - len).unwrap_or(0)
}

fn inside(net: (u32, u32), ip: Ipv4Addr) -> bool {
    u32::from(ip) & mask(net.1) == net.0
}

fn check_row(m: &Menu, r: &Row) -> Result<(), String> {
    if let Some((k, _)) = r.iter().find(|(_, v)| v.chars().any(char::is_control)) {
        return Err(format!("{k}: line breaks and control characters are not allowed"));
    }
    if let Some(k) = m.need.iter().find(|k| !r.contains_key(**k)) {
        return Err(format!("every row needs {k}"));
    }
    let v = |k: &str| r.get(k).map_or("", String::as_str);
    match m.key {
        "address" if infra::cidr(v("address")).is_none() => Err(format!("{}: invalid address (a.b.c.d/len)", v("address"))),
        "route" if net4(v("dst-address")).is_none() => Err(format!("{}: invalid destination (a.b.c.d/len)", v("dst-address"))),
        "arp" if v("address").parse::<Ipv4Addr>().is_err() => Err(format!("{}: invalid IP address", v("address"))),
        "arp" if !is_mac(v("mac-address")) => Err(format!("{}: invalid MAC address", v("mac-address"))),
        _ => Ok(()),
    }
}

fn is_mac(v: &str) -> bool {
    let p: Vec<&str> = v.split(':').collect();
    p.len() == 6 && p.iter().all(|x| x.len() == 2 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

/// "/ip/route" as a script path: "/ip route".
fn cli(path: &str) -> String {
    format!("/{}", path[1..].replace('/', " "))
}

/// A value on the API: disabled as yes/no, a flag as an empty word, the rest as given.
fn wire(k: &str, v: &str) -> (String, String) {
    match k {
        "disabled" => (k.into(), if yes(v) { "yes" } else { "no" }.into()),
        _ if FLAGS.contains(&k) => (k.into(), String::new()),
        _ => (k.into(), v.into()),
    }
}

/// A value in a rollback script: the script language wants yes/no for
/// booleans (a quoted "true" is refused) and a bare word for a flag.
fn script(k: &str, v: &str) -> String {
    match v {
        _ if FLAGS.contains(&k) => k.to_string(),
        "true" | "false" | "yes" | "no" if k != "comment" && k != "name" => format!("{k}={}", if yes(v) { "yes" } else { "no" }),
        _ => format!("{k}={}", q(v)),
    }
}

/// For an /ip/address set: RouterOS keeps the old `network` (seen on 7.25).
fn network(addr: &str) -> Option<String> {
    infra::cidr(addr).map(|(ip, n, l)| if l == 32 { ip } else { Ipv4Addr::from(n) }.to_string())
}

/// The commands, the rollback script, and warnings for the preview.
pub fn plan(live: &Value, w: &Want, path: &Path) -> Result<(Vec<Cmd>, Vec<String>, Vec<String>), String> {
    let m = w.menu;
    let base = m.path;
    let old = statics(live, m);
    let old_ids: Vec<&str> = old.iter().map(|r| s(r, ".id")).collect();
    let mut seen = std::collections::BTreeSet::new();
    for (id, r) in &w.rows {
        check_row(m, r)?;
        if let Some(id) = id {
            if !old_ids.contains(&id.as_str()) || !seen.insert(id.clone()) {
                return Err(format!("{id}: this row is gone from the switch: reload"));
            }
        }
    }
    let (mut cmds, mut undo) = (Vec::<Cmd>::new(), Vec::<String>::new());
    let kv = |p: &[(&str, &str)]| p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();

    if m.single {
        let cur = old.first().map(|r| fields_of(m, r)).unwrap_or_default();
        let raw = old.first().copied().cloned().unwrap_or(Value::Null);
        let mut set = vec![];
        for (k, v) in &w.rows[0].1 {
            if !same(k, cur.get(k), Some(v)) {
                set.push(wire(k, v));
                undo.push(format!("{} set {}", cli(base), script(k, s(&raw, k))));
            }
        }
        if !set.is_empty() {
            cmds.push((format!("{base}/set"), set));
        }
    } else {
        let kept: Vec<&str> = w.rows.iter().filter_map(|(id, _)| id.as_deref()).collect();
        for r in old.iter().filter(|r| !kept.contains(&s(r, ".id"))) {
            cmds.push((format!("{base}/remove"), kv(&[(".id", s(r, ".id"))])));
            let line: Vec<String> = fields_of(m, r).iter().map(|(k, v)| script(k, v)).collect();
            undo.push(format!(":do {{ {} add {} }} on-error={{ :log warning {} }}", cli(base), line.join(" "), q("cairn-net: a row could not be put back")));
        }
        for (id, r) in w.rows.iter().filter_map(|(id, r)| Some((id.as_ref()?, r))) {
            let o = old[old_ids.iter().position(|i| *i == id.as_str()).unwrap()];
            let cur = fields_of(m, o);
            let cur_net = Some(s(o, "network")).filter(|n| !n.is_empty()).map_or(String::new(), |n| format!(" network={}", q(n)));
            let mut set: Vec<(String, String)> = vec![];
            let at = format!("[find where .id={id}]");
            // an address goes back with its old network (RouterOS keeps it on a set)
            let old_net = |k: &str| if m.key == "address" && k == "address" { cur_net.clone() } else { String::new() };
            for (k, v) in r.iter().filter(|(k, v)| !same(k, cur.get(*k), Some(v))) {
                set.push(wire(k, v));
                undo.push(match cur.get(k) {
                    Some(o) => format!("{} set {at} {}{}", cli(base), script(k, o), old_net(k)),
                    None if k == "disabled" => format!("{} set {at} disabled=no", cli(base)),
                    None if k == "comment" => format!("{} set {at} comment=\"\"", cli(base)),
                    None => format!("{} unset {at} {k}", cli(base)),
                });
            }
            if m.key == "address" && set.iter().any(|(k, _)| k == "address") {
                if let Some(n) = r.get("address").and_then(|a| network(a)) {
                    set.push(("network".into(), n));
                }
            }
            for (k, o) in cur.iter().filter(|(k, _)| !r.contains_key(*k)) {
                match k.as_str() {
                    "disabled" => set.push(("disabled".into(), "no".into())),
                    "comment" => set.push(("comment".into(), String::new())),
                    _ => cmds.push((format!("{base}/unset"), kv(&[("numbers", id), ("value-name", k)]))),
                }
                undo.push(format!("{} set {at} {}", cli(base), script(k, o)));
            }
            if !set.is_empty() {
                set.insert(0, (".id".into(), id.clone()));
                cmds.push((format!("{base}/set"), set));
            }
        }
        for (_, r) in w.rows.iter().filter(|(id, _)| id.is_none()) {
            cmds.push((format!("{base}/add"), r.iter().map(|(k, v)| wire(k, v)).collect()));
            let mut find: Vec<String> = m.keys.iter().filter_map(|k| Some(format!("{k}={}", q(r.get(*k)?)))).collect();
            if m.dynamic {
                find.insert(0, "!dynamic".into());
            }
            undo.push(format!(":do {{ {} remove [find where {}] }} on-error={{}}", cli(base), find.join(" and ")));
        }
    }

    let warnings = guard(live, w, path)?;
    if !undo.is_empty() {
        undo.reverse();
        finish(&mut undo);
    }
    Ok((cmds, undo, warnings))
}

/// The address Cairn Net connects to changes, and nothing else: that is the
/// Infrastructure page's move (Cairn Net follows the switch). (.id, new address)
pub fn mgmt_move(live: &Value, w: &Want, path: &Path) -> Option<(String, String)> {
    if w.menu.key != "address" {
        return None;
    }
    let old = statics(live, w.menu);
    if w.rows.len() != old.len() || w.rows.iter().any(|(id, _)| id.is_none()) {
        return None;
    }
    let mut moved = None;
    for (id, r) in &w.rows {
        let o = old.iter().find(|x| Some(s(x, ".id")) == id.as_deref())?;
        let cur = fields_of(w.menu, o);
        if &cur == r {
            continue;
        }
        let only_addr = cur.iter().filter(|(k, _)| *k != "address").all(|(k, v)| r.get(k) == Some(v)) && r.keys().all(|k| k == "address" || cur.contains_key(k));
        let is_mgmt = infra::cidr(s(o, "address")).is_some_and(|(ip, _, _)| ip == path.switch);
        if !only_addr || !is_mgmt || moved.is_some() {
            return None;
        }
        moved = Some((id.clone()?, r.get("address")?.clone()));
    }
    moved
}

/* ---------- the management path ---------- */

/// How the switch reaches an address: the best route (longest prefix, then
/// distance) among its connected networks and its routes of table main.
#[derive(Debug, PartialEq, Clone)]
struct Hop {
    len: u32,
    distance: u32,
    connected: bool,
    blackhole: bool,
    gateway: String,
    iface: Option<String>,
}

/// (network, len, interface) of the enabled addresses.
fn connected(addrs: &[Value]) -> Vec<(u32, u32, String)> {
    addrs.iter().filter(|a| !yes(s(a, "disabled")) && !yes(s(a, "invalid")))
        .filter_map(|a| infra::cidr(s(a, "address")).map(|(ip, _, l)| (u32::from(ip) & mask(l), l, s(a, "interface").to_string()))).collect()
}

fn hop(addrs: &[Value], routes: &[Value], ip: Ipv4Addr) -> Option<Hop> {
    let nets = connected(addrs);
    let mut best: Option<Hop> = None;
    let mut offer = |h: Hop| {
        if best.as_ref().is_none_or(|b| h.len > b.len || h.len == b.len && h.distance < b.distance) {
            best = Some(h);
        }
    };
    for (n, l, i) in &nets {
        if inside((*n, *l), ip) {
            offer(Hop { len: *l, distance: 0, connected: true, blackhole: false, gateway: i.clone(), iface: Some(i.clone()) });
        }
    }
    for r in routes.iter().filter(|r| !yes(s(r, "disabled")) && s(r, "connect") != "true" && matches!(s(r, "routing-table"), "" | "main")) {
        let Some(dst) = net4(s(r, "dst-address")) else { continue };
        if !inside(dst, ip) {
            continue;
        }
        let gw = s(r, "gateway");
        let (gip, gif) = gw.split_once('%').map_or((gw, None), |(a, b)| (a, Some(b)));
        let iface = gif.map(String::from).or_else(|| match gip.parse::<Ipv4Addr>() {
            Ok(g) => nets.iter().find(|(n, l, _)| inside((*n, *l), g)).map(|x| x.2.clone()),
            Err(_) if !gip.is_empty() => Some(gip.to_string()), // an interface
            Err(_) => None,
        });
        offer(Hop { len: dst.1, distance: s(r, "distance").parse().unwrap_or(1), connected: false, blackhole: yes(s(r, "blackhole")) || s(r, "type") == "blackhole",
            gateway: gw.to_string(), iface });
    }
    best
}

/// The menu as it will be: dynamic rows as they are, the wanted rows over
/// the properties they had (read-only ones kept for the route choice).
fn after(live: &Value, w: &Want) -> Vec<Value> {
    let m = w.menu;
    let mut out: Vec<Value> = rows(live, m).into_iter().filter(|r| yes(s(r, "dynamic"))).cloned().collect();
    for (id, r) in &w.rows {
        let mut v = id.as_ref().and_then(|id| statics(live, m).into_iter().find(|x| s(x, ".id") == id)).cloned().unwrap_or_else(|| json!({}));
        for k in m.fields {
            v[*k] = json!(r.get(*k).map_or("", String::as_str));
        }
        out.push(v);
    }
    out
}

/// The rows a change touches: (old rows removed or edited, as they were;
/// edited and new rows, as they will be).
fn touched(live: &Value, w: &Want) -> (Vec<Row>, Vec<Row>) {
    let m = w.menu;
    let (mut gone, mut fresh) = (vec![], vec![]);
    for o in statics(live, m) {
        match w.rows.iter().find(|(id, _)| id.as_deref() == Some(s(o, ".id"))) {
            Some((_, r)) if &fields_of(m, o) == r => {}
            Some((_, r)) => {
                gone.push(fields_of(m, o));
                fresh.push(r.clone());
            }
            None => gone.push(fields_of(m, o)),
        }
    }
    fresh.extend(w.rows.iter().filter(|(id, _)| id.is_none()).map(|(_, r)| r.clone()));
    (gone, fresh)
}

fn arr(live: &Value, k: &str) -> Vec<Value> {
    live["menus"][k].as_array().cloned().unwrap_or_default()
}

/// Refuses what could cut Cairn Net's path; warnings for the preview.
fn guard(live: &Value, w: &Want, path: &Path) -> Result<Vec<String>, String> {
    let m = w.menu;
    let addrs = arr(live, "address");
    let routes = arr(live, "route");
    let mgmt = infra::mgmt_iface(&json!({"addresses": addrs}), path).map(String::from);
    let app_hop = path.app.and_then(|a| hop(&addrs, &routes, a));
    let mut warn = vec![];
    let v = |r: &Row, k: &str| r.get(k).cloned().unwrap_or_default();
    match m.key {
        "address" => {
            let (gone, fresh) = touched(live, w);
            for r in gone.iter().chain(&fresh) {
                let ifc = v(r, "interface");
                if mgmt.as_deref() == Some(ifc.as_str()) {
                    return Err(format!("{ifc}: Cairn Net reaches the switch through this interface, it is not edited from here"));
                }
            }
            for r in &gone {
                if let Some(app) = path.app.filter(|a| infra::contains(&v(r, "address"), *a)) {
                    return Err(format!("{}: Cairn Net ({app}) is on this network, the switch answers it through here", v(r, "interface")));
                }
            }
            for r in &fresh {
                let addr = v(r, "address");
                for end in std::iter::once(path.switch).chain(path.app) {
                    if infra::contains(&addr, end) {
                        return Err(format!("{addr} would take the network of the management path ({end})"));
                    }
                }
            }
            // overlaps between the enabled addresses after the change (new ones only)
            let new = after(live, w);
            let on: Vec<&Value> = new.iter().filter(|a| !yes(s(a, "disabled"))).collect();
            let is_fresh = |x: &Value| fresh.iter().any(|t| t.get("address").map(String::as_str) == Some(s(x, "address")));
            for (i, a) in on.iter().enumerate() {
                for b in &on[..i] {
                    if (is_fresh(a) || is_fresh(b)) && infra::overlap(s(a, "address"), s(b, "address")) {
                        return Err(format!("{} overlaps {}, already on the switch", s(a, "address"), s(b, "address")));
                    }
                }
            }
        }
        "route" => {
            if let Some(app) = path.app {
                let now = hop(&addrs, &after(live, w), app);
                match (&app_hop, &now) {
                    (_, None) => return Err(format!("no route would be left to Cairn Net ({app})")),
                    (_, Some(h)) if h.blackhole => return Err(format!("{app}: Cairn Net would be sent to a blackhole")),
                    (Some(b), Some(h)) if b.connected && b != h => return Err(format!("Cairn Net ({app}) is on a network of the switch: a route would take its replies elsewhere")),
                    (_, Some(h)) if h.iface.is_none() => return Err(format!("{}: the gateway towards Cairn Net ({app}) is on no network of the switch", h.gateway)),
                    (Some(b), Some(h)) if b != h => warn.push(format!("the switch answers Cairn Net ({app}) through {} after the change", h.gateway)),
                    _ => {}
                }
            }
        }
        "arp" => {
            let next = app_hop.as_ref().filter(|h| !h.connected).and_then(|h| h.gateway.split('%').next().map(String::from));
            let (gone, fresh) = touched(live, w);
            for r in gone.iter().chain(&fresh) {
                let a = v(r, "address");
                if path.app.map(|x| x.to_string()) == Some(a.clone()) || next.as_deref() == Some(a.as_str()) || a == path.switch.to_string() {
                    return Err(format!("{a}: on Cairn Net's path to the switch, its ARP entry is not edited from here"));
                }
            }
        }
        "dhcp-client" => {
            let path_ifs: Vec<String> = mgmt.iter().cloned().chain(app_hop.as_ref().and_then(|h| h.iface.clone())).collect();
            let (gone, fresh) = touched(live, w);
            for r in gone.iter().chain(&fresh) {
                let i = v(r, "interface");
                if path_ifs.contains(&i) {
                    return Err(format!("{i}: Cairn Net reaches the switch through this interface, its DHCP client is not edited from here"));
                }
            }
            for r in &fresh {
                let i = v(r, "interface");
                if !matches!(v(r, "add-default-route").as_str(), "no" | "false") && !app_hop.as_ref().is_some_and(|h| h.connected) {
                    warn.push(format!("{i}: a default route from this DHCP client may change how the switch answers Cairn Net"));
                }
            }
        }
        "pool" => {
            let srv = arr(live, "pool");
            let users = |name: &str| -> Option<String> {
                live["dhcp_servers"].as_array().into_iter().flatten().find(|d| s(d, "address-pool") == name).map(|d| format!("DHCP server {}", s(d, "name")))
                    .or_else(|| srv.iter().find(|p| s(p, "next-pool") == name).map(|p| format!("pool {}", s(p, "name"))))
            };
            for o in statics(live, m) {
                let kept = w.rows.iter().find(|(id, _)| id.as_deref() == Some(s(o, ".id")));
                if kept.is_none_or(|(_, r)| r != &fields_of(m, o)) {
                    if let Some(u) = users(s(o, "name")) {
                        return Err(format!("{}: used by {u}, edited on the DHCP page", s(o, "name")));
                    }
                }
            }
        }
        "ip-settings" => {
            let r = &w.rows[0].1;
            match r.get("rp-filter").map(String::as_str) {
                Some("strict") => {
                    let back = app_hop.as_ref().and_then(|h| h.iface.clone());
                    if path.app.is_none() || back.is_none() || back != mgmt {
                        return Err("rp-filter strict: Cairn Net's packets come in through another interface than the switch answers them by".into());
                    }
                }
                Some("loose") if path.app.is_some() && app_hop.is_none() => return Err("rp-filter loose: the switch has no route to Cairn Net".into()),
                _ => {}
            }
            if r.get("ip-forward").is_some_and(|x| !yes(x)) && live["menus"]["ip-settings"][0]["ip-forward"].as_str().is_some_and(yes) {
                warn.push("ip-forward off: the switch stops routing between its networks; devices reached through it may be cut".into());
            }
        }
        _ => {}
    }
    Ok(warn)
}

/// Whether the switch now runs what was asked: the same static rows (any
/// order), each with the asked properties.
pub fn matches(live: &Value, w: &Want) -> bool {
    let m = w.menu;
    let got: Vec<Row> = statics(live, m).into_iter().map(|r| fields_of(m, r)).collect();
    // a disabled route reads without its computed properties (routing-table, scope…), seen on 7.25
    let quiet = |g: &Row, k: &str, v: &str| !g.contains_key(k) && (g.contains_key("disabled") && k != "comment" || k == "routing-table" && v == "main");
    let fits = |g: &Row, r: &Row| r.iter().all(|(k, v)| same(k, g.get(k), Some(v)) || quiet(g, k, v)) && (m.single || g.contains_key("disabled") == r.contains_key("disabled"));
    if m.single {
        return got.first().is_some_and(|g| fits(g, &w.rows[0].1));
    }
    let mut used = vec![false; got.len()];
    got.len() == w.rows.len() && w.rows.iter().all(|(_, r)| match (0..got.len()).find(|i| !used[*i] && fits(&got[*i], r)) {
        Some(i) => {
            used[i] = true;
            true
        }
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::show_all;

    fn live() -> Value {
        json!({"menus": {
            "address": [
                {".id": "*2", "address": "10.99.0.1/24", "network": "10.99.0.0", "interface": "test-lan", "disabled": "false", "dynamic": "false"},
                {".id": "*3", "address": "192.168.1.86/24", "network": "192.168.1.0", "interface": "ether1", "disabled": "false", "dynamic": "false"},
                {".id": "*5", "address": "10.97.0.1/24", "network": "10.97.0.0", "interface": "test-ip", "disabled": "false", "dynamic": "false", "comment": "tests"},
            ],
            "route": [
                {".id": "*8", "dst-address": "0.0.0.0/0", "gateway": "192.168.1.1", "distance": "1", "routing-table": "main", "dynamic": "false", "static": "true", "active": "true"},
                {".id": "*D1", "dst-address": "192.168.1.0/24", "gateway": "ether1", "distance": "0", "routing-table": "main", "dynamic": "true", "connect": "true"},
            ],
            "arp": [{".id": "*1", "address": "192.168.1.69", "mac-address": "BC:24:11:7A:B8:93", "interface": "ether1", "dynamic": "true"}],
            "pool": [{".id": "*1", "name": "test", "ranges": "10.99.0.50-10.99.0.99"}, {".id": "*2", "name": "spare", "ranges": "10.97.0.50-10.97.0.60"}],
            "dhcp-client": [],
            "ip-settings": [{"ip-forward": "true", "rp-filter": "no", "arp-timeout": "30s"}],
        }, "dhcp_servers": [{"name": "test", "address-pool": "test", "interface": "test-lan"}]})
    }

    fn path() -> Path {
        Path { switch: "192.168.1.86".parse().unwrap(), app: Some("192.168.1.69".parse().unwrap()) }
    }

    fn want(menu: &str, rows: Value) -> Want {
        Want::from_json(&json!({"menu": menu, "rows": rows})).unwrap()
    }

    /// The page's rows: the static ones as read, `f` applied.
    fn edit(menu: &str, f: impl FnOnce(&mut Vec<Value>)) -> Want {
        let l = live();
        let mut r: Vec<Value> = l["menus"][menu].as_array().unwrap().iter().filter(|x| x["dynamic"] != "true").cloned().collect();
        f(&mut r);
        want(menu, json!(r))
    }

    #[test]
    fn unchanged_rows_mean_nothing_to_do() {
        for m in ["address", "route", "pool", "arp"] {
            let w = edit(m, |_| {});
            let (c, u, _) = plan(&live(), &w, &path()).unwrap();
            assert!(c.is_empty() && u.is_empty(), "{m}");
            assert!(matches(&live(), &w), "{m}");
        }
    }

    #[test]
    fn routes_add_edit_remove_and_undo() {
        let w = edit("route", |r| {
            r[0]["distance"] = json!("5");
            r[0]["check-gateway"] = json!("ping");
            r.push(json!({"dst-address": "10.50.0.0/16", "gateway": "10.97.0.254", "comment": "lab"}));
        });
        let (c, u, warn) = plan(&live(), &w, &path()).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/route/set .id=*8 check-gateway=ping distance=5", "/ip/route/add comment=lab dst-address=10.50.0.0/16 gateway=10.97.0.254"]);
        assert_eq!(&u[..3], [
            ":do { /ip route remove [find where !dynamic and dst-address=\"10.50.0.0/16\" and gateway=\"10.97.0.254\"] } on-error={}",
            "/ip route set [find where .id=*8] distance=\"1\"",
            "/ip route unset [find where .id=*8] check-gateway",
        ]);
        assert!(u.last().unwrap().contains("scheduler remove"));
        assert!(warn.is_empty());
        // a disabled route reads back without routing-table and scope: still as asked
        let mut l = live();
        l["menus"]["route"][0] = json!({".id": "*8", "dst-address": "0.0.0.0/0", "gateway": "192.168.1.1", "distance": "1", "disabled": "true", "static": "true"});
        assert!(matches(&l, &edit("route", |r| { r[0]["disabled"] = json!("true"); r[0]["scope"] = json!("30"); })));
        assert!(!matches(&l, &edit("route", |r| r[0]["distance"] = json!("2"))));
        // removed: added back whole
        let w = edit("route", |r| r.clear());
        let e = plan(&live(), &w, &path());
        // the app is on a connected network: removing the default route leaves its path alone
        let (c, u, _) = e.unwrap();
        assert_eq!(show_all(&c), vec!["/ip/route/remove .id=*8"]);
        assert!(u[0].starts_with(":do { /ip route add") && u[0].contains("gateway=\"192.168.1.1\""), "{u:?}");
    }

    #[test]
    fn routes_never_take_the_management_path() {
        let p = path();
        // a more specific route to the app, connected path: refused
        let w = edit("route", |r| r.push(json!({"dst-address": "192.168.1.69/32", "gateway": "10.97.0.254"})));
        assert!(plan(&live(), &w, &p).unwrap_err().contains("is on a network"));
        let w = edit("route", |r| r.push(json!({"dst-address": "192.168.1.0/25", "gateway": "10.97.0.254"})));
        assert!(plan(&live(), &w, &p).unwrap_err().contains("is on a network"));
        // routed app: the default route carries it
        let routed = Path { switch: "192.168.1.86".parse().unwrap(), app: Some("192.168.99.69".parse().unwrap()) };
        assert!(plan(&live(), &edit("route", |r| r.clear()), &routed).unwrap_err().contains("no route"));
        assert!(plan(&live(), &edit("route", |r| r[0]["gateway"] = json!("172.16.0.1")), &routed).unwrap_err().contains("on no network"));
        assert!(plan(&live(), &edit("route", |r| r[0]["disabled"] = json!("true")), &routed).unwrap_err().contains("no route"));
        let (_, _, warn) = plan(&live(), &edit("route", |r| r[0]["gateway"] = json!("10.97.0.254")), &routed).unwrap();
        assert!(warn[0].contains("through 10.97.0.254"), "{warn:?}");
        // a route elsewhere: fine; bad destination refused
        assert!(plan(&live(), &edit("route", |r| r.push(json!({"dst-address": "10.50.0.0/16", "gateway": "10.97.0.254"}))), &routed).is_ok());
        assert!(plan(&live(), &edit("route", |r| r.push(json!({"dst-address": "10.50.0/16", "gateway": "x"}))), &routed).unwrap_err().contains("invalid destination"));
    }

    #[test]
    fn addresses() {
        let p = path();
        // test interface: edit, the network goes with it
        let w = edit("address", |r| r[2]["address"] = json!("10.96.0.1/16"));
        let (c, u, _) = plan(&live(), &w, &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/address/set .id=*5 address=10.96.0.1/16 network=10.96.0.0"]);
        assert_eq!(u[0], "/ip address set [find where .id=*5] address=\"10.97.0.1/24\" network=\"10.97.0.0\"");
        assert!(!matches(&live(), &w));
        // new address, removal
        let (c, u, _) = plan(&live(), &edit("address", |r| { r.remove(0); r.push(json!({"address": "10.95.0.1/24", "interface": "test-ip"})); }), &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/address/remove .id=*2", "/ip/address/add address=10.95.0.1/24 interface=test-ip"]);
        assert!(u[0].contains("remove [find where !dynamic and address=\"10.95.0.1/24\" and interface=\"test-ip\"]"));
        assert!(u[1].contains("/ip address add address=\"10.99.0.1/24\" interface=\"test-lan\""));
        let e = |f: fn(&mut Vec<Value>)| plan(&live(), &edit("address", f), &p).unwrap_err();
        assert!(e(|r| { r.remove(1); }).contains("reaches the switch"));
        assert!(e(|r| r[1]["disabled"] = json!("true")).contains("reaches the switch"));
        assert!(e(|r| r.push(json!({"address": "172.20.0.1/24", "interface": "ether1"}))).contains("reaches the switch"));
        assert!(e(|r| r[2]["address"] = json!("192.168.0.1/16")).contains("management path"));
        assert!(e(|r| r[2]["address"] = json!("10.99.0.2/16")).contains("overlaps"));
        assert!(e(|r| r[2]["address"] = json!("10.97.0.1")).contains("invalid"));
        // the address Cairn Net connects to: handed over to the Infrastructure move
        let w = edit("address", |r| r[1]["address"] = json!("192.168.1.87/24"));
        assert_eq!(mgmt_move(&live(), &w, &p), Some(("*3".into(), "192.168.1.87/24".into())));
        assert_eq!(mgmt_move(&live(), &edit("address", |r| { r[1]["address"] = json!("192.168.1.87/24"); r[1]["comment"] = json!("x"); }), &p), None);
        assert_eq!(mgmt_move(&live(), &edit("address", |r| r[2]["address"] = json!("10.96.0.1/24")), &p), None);
    }

    #[test]
    fn arp_dhcp_client_pools_and_settings() {
        let p = path();
        let (c, _, _) = plan(&live(), &edit("arp", |r| r.push(json!({"address": "10.97.0.9", "mac-address": "aa:bb:cc:dd:ee:ff", "interface": "test-ip"}))), &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/arp/add address=10.97.0.9 interface=test-ip mac-address=aa:bb:cc:dd:ee:ff"]);
        // read back in upper case: still as asked
        let mut l = live();
        l["menus"]["arp"].as_array_mut().unwrap().push(json!({".id": "*9", "address": "10.97.0.9", "mac-address": "AA:BB:CC:DD:EE:FF", "interface": "test-ip", "dynamic": "false"}));
        assert!(matches(&l, &want("arp", json!([{"address": "10.97.0.9", "mac-address": "aa:bb:cc:dd:ee:ff", "interface": "test-ip"}]))));
        let e = |m: &str, r: Value| plan(&live(), &want(m, r), &p).unwrap_err();
        assert!(e("arp", json!([{"address": "192.168.1.69", "mac-address": "00:11:22:33:44:55", "interface": "ether1"}])).contains("ARP entry"));
        assert!(e("arp", json!([{"address": "10.97.0.9", "mac-address": "nope", "interface": "test-ip"}])).contains("MAC"));
        assert!(e("dhcp-client", json!([{"interface": "ether1"}])).contains("DHCP client"));
        let (_, _, warn) = plan(&live(), &want("dhcp-client", json!([{"interface": "test-ip", "add-default-route": "yes"}])), &path()).unwrap();
        assert!(warn.is_empty(), "connected app: a default route does not matter");
        // pools: the DHCP server's one stays, the spare one is free
        assert!(e("pool", json!([{".id": "*2", "name": "spare", "ranges": "10.97.0.50-10.97.0.60"}])).contains("DHCP page"));
        assert!(e("pool", json!([{".id": "*1", "name": "test", "ranges": "10.99.0.50-10.99.0.60"}, {".id": "*2", "name": "spare", "ranges": "10.97.0.50-10.97.0.60"}])).contains("DHCP page"));
        let (c, _, _) = plan(&live(), &want("pool", json!([{".id": "*1", "name": "test", "ranges": "10.99.0.50-10.99.0.99"}])), &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/pool/remove .id=*2"]);
        // settings: one set, undone value by value
        let w = want("ip-settings", json!([{"rp-filter": "loose", "arp-timeout": "30s"}]));
        let (c, u, _) = plan(&live(), &w, &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/settings/set rp-filter=loose"]);
        assert_eq!(u[0], "/ip settings set rp-filter=no");
        // booleans go back as yes/no (the script language refuses a quoted "true")
        assert_eq!(plan(&live(), &want("ip-settings", json!([{"ip-forward": "no"}])), &p).unwrap().1[0], "/ip settings set ip-forward=yes");
        // a flag: present = on, written as a bare word
        let mut l = live();
        l["menus"]["table"] = json!([{".id": "*0", "name": "main", "fib": "", "dynamic": "true"}, {".id": "*4", "name": "lab", "fib": ""}]);
        let (c, u, _) = plan(&l, &want("table", json!([{".id": "*4", "name": "lab"}, {"name": "vpn", "fib": "true"}])), &p).unwrap();
        assert_eq!(show_all(&c), vec!["/routing/table/unset numbers=*4 value-name=fib", "/routing/table/add fib=\"\" name=vpn"]);
        assert_eq!(&u[..2], [":do { /routing table remove [find where !dynamic and name=\"vpn\"] } on-error={}", "/routing table set [find where .id=*4] fib"]);
        assert!(matches(&l, &want("table", json!([{".id": "*4", "name": "lab", "fib": "true"}]))));
        let (_, _, warn) = plan(&live(), &want("ip-settings", json!([{"ip-forward": "no"}])), &p).unwrap();
        assert!(warn[0].contains("ip-forward"));
        assert!(matches(&live(), &want("ip-settings", json!([{"ip-forward": "yes"}]))));
        // strict: answered through ether1, came in through ether1: fine; routed through another interface: refused
        assert!(plan(&live(), &want("ip-settings", json!([{"rp-filter": "strict"}])), &p).is_ok());
        let mut l = live();
        l["menus"]["route"][0]["gateway"] = json!("10.97.0.254");
        let routed = Path { switch: "192.168.1.86".parse().unwrap(), app: Some("192.168.99.69".parse().unwrap()) };
        assert!(plan(&l, &want("ip-settings", json!([{"rp-filter": "strict"}])), &routed).unwrap_err().contains("strict"));
    }
}
