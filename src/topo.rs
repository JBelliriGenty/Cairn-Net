//! Topology, from what /api/slow already reads: which switch ports face each
//! other (LLDP / MNDP), where each device is plugged (bridge MAC tables), and
//! a first guess of what each device is (neighbour announcement, name, MAC
//! vendor). The page draws it; the user can correct a device's kind, kept in
//! data_dir/kinds.json (MAC -> kind).
//!
//! Neighbour discovery through a bridge: MNDP is a broadcast, so a switch
//! also hears the switches behind its neighbour, on the same port. An entry
//! "A sees B on port P, B's side being X" is kept when B, on X, sees A on P
//! (or B reports nothing on X), and dropped when another switch D heard on P
//! has its own two-sided link to B elsewhere than towards A: B is behind D.

use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{Mutex, OnceLock};

pub const KINDS: [&str; 11] = ["switch", "router", "ap", "server", "nas", "pc", "phone", "printer", "camera", "iot", "unknown"];

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn rows<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn list(v: &str) -> Vec<&str> {
    v.split(',').map(str::trim).filter(|x| !x.is_empty()).collect()
}

/// sfp-sfpplus2 before sfp-sfpplus10.
pub fn natural(a: &str, b: &str) -> Ordering {
    let split = |x: &str| {
        let i = x.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        (x[..i].to_string(), x[i..].parse::<u64>().unwrap_or(0))
    };
    split(a).cmp(&split(b)).then_with(|| a.cmp(b))
}

struct Sw<'a> {
    name: &'a str,
    v: &'a Value,
}

impl Sw<'_> {
    fn bond_of(&self, port: &str) -> Option<&str> {
        rows(self.v, "bonds").iter().find(|b| list(s(b, "slaves")).contains(&port)).map(|b| s(b, "name"))
    }
    /// A bond member counts as its bond.
    fn logical(&self, port: &str) -> String {
        self.bond_of(port).unwrap_or(port).to_string()
    }
    fn is_eth(&self, port: &str) -> bool {
        rows(self.v, "monitor").iter().any(|m| s(m, "name") == port)
    }
}

#[derive(Clone, PartialEq)]
struct Seen {
    a: usize,
    port: String,
    b: usize,
    rport: String,
}

/// Direct links between the managed switches: (a, port, b, rport), each once.
fn direct(sws: &[Sw]) -> Vec<Seen> {
    let idx: BTreeMap<&str, usize> = sws.iter().enumerate().map(|(i, w)| (w.name, i)).collect();
    let mut seen: Vec<Seen> = Vec::new();
    for (a, w) in sws.iter().enumerate() {
        for n in rows(w.v, "neighbors") {
            let Some(&b) = idx.get(s(n, "identity")) else { continue };
            let port = list(s(n, "interface")).first().copied().unwrap_or("");
            let rport = s(n, "interface-name").rsplit('/').next().unwrap_or("");
            if b == a || !w.is_eth(port) || rport.is_empty() {
                continue;
            }
            let e = Seen { a, port: port.into(), b, rport: rport.into() };
            if !seen.contains(&e) {
                seen.push(e);
            }
        }
    }
    let has_back = |c: &Seen| seen.iter().any(|x| x.a == c.b && x.port == c.rport);
    let two_sided = |c: &Seen| seen.iter().any(|x| x.a == c.b && x.port == c.rport && x.b == c.a && x.rport == c.port);
    let valid: Vec<&Seen> = seen.iter().filter(|c| !has_back(c) || two_sided(c)).collect();
    let behind = |c: &Seen| {
        valid.iter().any(|d| {
            d.a == c.a && d.b != c.b && sws[c.a].logical(&d.port) == sws[c.a].logical(&c.port)
                && valid.iter().any(|e| e.a == d.b && e.b == c.b && two_sided(*e) && sws[d.b].logical(&e.port) != sws[d.b].logical(&d.rport))
        })
    };
    let mut out: Vec<Seen> = Vec::new();
    for &c in &valid {
        if behind(c) {
            continue;
        }
        // both ends report the same cable: keep it once, from the first switch
        let e = if c.a <= c.b { c.clone() } else { Seen { a: c.b, port: c.rport.clone(), b: c.a, rport: c.port.clone() } };
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

/* ---------- what a device is ---------- */

const VIRTUAL: [&str; 6] = ["BC:24:11", "52:54:00", "00:50:56", "00:0C:29", "00:15:5D", "00:16:3E"];

// ponytail: a few common home vendors only; the user corrects the rest by hand.
const VENDORS: &[(&str, &str, &str)] = &[
    ("BC:24:11", "server", "Proxmox VM"), ("52:54:00", "server", "QEMU VM"), ("00:50:56", "server", "VMware VM"), ("00:0C:29", "server", "VMware VM"),
    ("00:15:5D", "server", "Hyper-V VM"), ("00:16:3E", "server", "Xen VM"),
    ("00:11:32", "nas", "Synology"), ("90:09:D0", "nas", "Synology"), ("00:08:9B", "nas", "QNAP"), ("24:5E:BE", "nas", "QNAP"),
    ("B8:27:EB", "server", "Raspberry Pi"), ("DC:A6:32", "server", "Raspberry Pi"), ("E4:5F:01", "server", "Raspberry Pi"),
    ("D8:3A:DD", "server", "Raspberry Pi"), ("2C:CF:67", "server", "Raspberry Pi"),
    ("00:17:88", "iot", "Philips Hue"), ("EC:B5:FA", "iot", "Philips Hue"), ("70:EE:50", "iot", "Netatmo"),
    ("24:0A:C4", "iot", "Espressif"), ("30:AE:A4", "iot", "Espressif"), ("A4:CF:12", "iot", "Espressif"), ("84:CC:A8", "iot", "Espressif"),
    ("EC:FA:BC", "iot", "Espressif"), ("3C:71:BF", "iot", "Espressif"), ("C8:2B:96", "iot", "Espressif"), ("8C:AA:B5", "iot", "Espressif"),
    ("24:62:AB", "iot", "Espressif"), ("7C:9E:BD", "iot", "Espressif"), ("48:3F:DA", "iot", "Espressif"), ("E8:DB:84", "iot", "Espressif"),
    ("00:0E:58", "iot", "Sonos"), ("48:A6:B8", "iot", "Sonos"), ("5C:AA:FD", "iot", "Sonos"), ("94:9F:3E", "iot", "Sonos"), ("B8:E9:37", "iot", "Sonos"),
    ("18:B4:30", "iot", "Nest"), ("64:16:66", "iot", "Nest"), ("44:65:0D", "iot", "Amazon"), ("68:54:FD", "iot", "Amazon"), ("FC:65:DE", "iot", "Amazon"),
    ("68:D7:9A", "ap", "Ubiquiti"), ("24:5A:4C", "ap", "Ubiquiti"), ("74:83:C2", "ap", "Ubiquiti"), ("78:8A:20", "ap", "Ubiquiti"),
    ("F4:92:BF", "ap", "Ubiquiti"), ("FC:EC:DA", "ap", "Ubiquiti"), ("80:2A:A8", "ap", "Ubiquiti"), ("E0:63:DA", "ap", "Ubiquiti"),
    ("18:E8:29", "ap", "Ubiquiti"), ("D8:B3:70", "ap", "Ubiquiti"), ("70:A7:41", "ap", "Ubiquiti"), ("9C:05:D6", "ap", "Ubiquiti"),
    ("00:04:0E", "router", "AVM"), ("3C:A6:2F", "router", "AVM"), ("98:9B:CB", "router", "AVM"), ("C8:0E:14", "router", "AVM"), ("2C:91:AB", "router", "AVM"),
    ("00:1E:8F", "printer", "Canon"), ("F4:81:39", "printer", "Canon"), ("00:26:AB", "printer", "Epson"), ("64:EB:8C", "printer", "Epson"),
    ("A4:EE:57", "printer", "Epson"), ("44:D2:44", "printer", "Epson"), ("00:1B:A9", "printer", "Brother"), ("30:05:5C", "printer", "Brother"), ("00:80:77", "printer", "Brother"),
    ("44:19:B6", "camera", "Hikvision"), ("C0:56:E3", "camera", "Hikvision"), ("4C:BD:8F", "camera", "Hikvision"), ("BC:AD:28", "camera", "Hikvision"),
    ("EC:71:DB", "camera", "Reolink"), ("3C:EF:8C", "camera", "Dahua"), ("E0:50:8B", "camera", "Dahua"), ("00:40:8C", "camera", "Axis"), ("AC:CC:8E", "camera", "Axis"),
    ("48:8F:5A", "router", "MikroTik"), ("64:D1:54", "router", "MikroTik"), ("CC:2D:E0", "router", "MikroTik"), ("4C:5E:0C", "router", "MikroTik"),
    ("6C:3B:6B", "router", "MikroTik"), ("B8:69:F4", "router", "MikroTik"), ("D4:CA:6D", "router", "MikroTik"), ("18:FD:74", "router", "MikroTik"),
    ("2C:C8:1B", "router", "MikroTik"), ("DC:2C:6E", "router", "MikroTik"), ("74:4D:28", "router", "MikroTik"), ("E4:8D:8C", "router", "MikroTik"),
];

// Words in a device name (lower case, trailing digits dropped: "cam2" -> "cam").
const WORDS: &[(&str, &[&str])] = &[
    ("router", &["router", "gateway", "gw", "box", "isp", "freebox", "fritzbox", "fritz", "bbox", "opnsense", "pfsense", "openwrt", "edgerouter"]),
    ("ap", &["ap", "uap", "unifi", "wifi", "wlan", "accesspoint", "nanohd"]),
    ("switch", &["switch", "sw"]),
    ("nas", &["nas", "qnap", "synology", "diskstation", "truenas", "unraid", "omv", "zimacube"]),
    ("server", &["server", "srv", "pve", "proxmox", "esxi", "hypervisor", "docker", "k8s", "vm"]),
    ("printer", &["printer", "print", "mfp", "laserjet", "officejet", "deskjet", "epson", "canon", "brother", "imprimante"]),
    ("camera", &["cam", "camera", "ipcam", "webcam", "reolink", "hikvision", "dahua", "doorbell", "nvr"]),
    ("phone", &["iphone", "ipad", "phone", "smartphone", "android", "pixel", "galaxy", "oneplus", "redmi", "telephone"]),
    ("pc", &["pc", "desktop", "laptop", "macbook", "imac", "macmini", "mac", "thinkpad", "workstation", "notebook", "surface", "windows", "win"]),
    ("iot", &["hue", "shelly", "tasmota", "esp", "esphome", "sonoff", "zigbee", "slzb", "zwave", "nuki", "tado", "netatmo", "plug", "bulb", "lamp",
        "sonos", "echo", "alexa", "chromecast", "nest", "homepod", "tv", "roku", "firetv", "appletv", "roomba", "thermostat", "sensor", "meross", "tuya", "wled", "knx"]),
];

fn by_name(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    for tok in lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()) {
        let base = tok.trim_end_matches(|c: char| c.is_ascii_digit());
        for (kind, words) in WORDS {
            // short words must be the whole word, long ones may start it ("printer01", "camerafront")
            if words.iter().any(|w| base == *w || (w.len() >= 4 && tok.starts_with(*w))) {
                return Some(*kind);
            }
        }
    }
    None
}

/// LLDP / MNDP: system capabilities first, then the board name.
fn by_announce(n: &Value) -> Option<&'static str> {
    // what is turned on; what it could do only when nothing says
    let caps = Some(s(n, "system-caps-enabled")).filter(|c| !c.is_empty()).unwrap_or_else(|| s(n, "system-caps"));
    let board = format!("{} {}", s(n, "board"), s(n, "platform")).trim().to_lowercase();
    let platform = s(n, "platform").to_lowercase();
    let has = |c: &str| caps.split(',').any(|x| x.trim() == c);
    Some(if has("wlan-ap") || ["uap", "u6", "u7", "cap", "wap", "nanohd"].iter().any(|p| board.starts_with(*p)) {
        "ap"
    } else if has("telephone") {
        "phone"
    } else if board.starts_with("crs") || board.starts_with("css") || (has("bridge") && !has("router")) {
        "switch"
    } else if has("router") || platform == "mikrotik" {
        "router"
    } else if has("station-only") {
        "pc"
    } else {
        return None;
    })
}

fn by_vendor(mac: &str) -> Option<(&'static str, &'static str)> {
    VENDORS.iter().find(|(p, ..)| mac.starts_with(*p)).map(|(_, k, v)| (*k, *v))
}

fn is_virtual(mac: &str) -> bool {
    VIRTUAL.iter().any(|p| mac.starts_with(*p))
}

/// Second hex digit 2, 6, A or E: a random (private) address, as phones, VMs and containers use.
fn is_random(mac: &str) -> bool {
    u8::from_str_radix(mac.get(0..2).unwrap_or("00"), 16).is_ok_and(|b| b & 2 != 0)
}

/// Guess, and why (for the page): {"by": announce|name|vendor|vms|random, "v": …}.
fn kind(mac: &str, name: &str, nb: Option<&Value>) -> (&'static str, Value) {
    if let Some(n) = nb {
        if let Some(k) = by_announce(n) {
            let what = [s(n, "board"), s(n, "platform")].into_iter().find(|x| !x.is_empty()).unwrap_or("LLDP");
            return (k, json!({"by": "announce", "v": what.trim_end_matches(',')}));
        }
    }
    if let Some(k) = by_name(name) {
        return (k, json!({"by": "name", "v": name}));
    }
    if let Some((k, v)) = by_vendor(mac) {
        return (k, json!({"by": "vendor", "v": v}));
    }
    ("unknown", if is_random(mac) { json!({"by": "random"}) } else { Value::Null })
}

/* ---------- the whole picture ---------- */

/// `switches`: the /api/slow rows; `arp`: this host's ARP table; `names`: IP -> DNS name.
pub fn build(switches: &[Value], arp: &[(String, String)], names: &BTreeMap<String, String>) -> Value {
    let sws: Vec<Sw> = switches.iter().filter(|v| v["error"].is_null()).map(|v| Sw { name: s(v, "name"), v }).collect();
    let up = |m: &str| m.to_uppercase();
    let lk = direct(&sws);

    // links: physical pairs grouped by the logical port at each end (LACP: the bond)
    let mut groups: BTreeMap<(usize, String, usize, String), Vec<(String, String)>> = BTreeMap::new();
    for e in &lk {
        groups.entry((e.a, sws[e.a].logical(&e.port), e.b, sws[e.b].logical(&e.rport))).or_default().push((e.port.clone(), e.rport.clone()));
    }
    let links: Vec<Value> = groups.into_iter().map(|((a, av, b, bv), mut pairs)| {
        pairs.sort_by(|x, y| natural(&x.0, &y.0));
        let lacp = sws[a].bond_of(&pairs[0].0).is_some() || sws[b].bond_of(&pairs[0].1).is_some();
        json!({"a": sws[a].name, "b": sws[b].name, "a_via": av, "b_via": bv, "pairs": pairs, "lacp": lacp})
    }).collect();

    // logical link ports of each switch, and who is across
    let mut across: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); sws.len()];
    for e in &lk {
        for (i, p, j) in [(e.a, &e.port, e.b), (e.b, &e.rport, e.a)] {
            across[i].insert(p.clone(), j);
            across[i].insert(sws[i].logical(p), j);
        }
    }

    // the switches' own MACs (their announcements) are not devices
    let names_sw: BTreeSet<&str> = sws.iter().map(|w| w.name).collect();
    let mut switch_macs: BTreeSet<String> = BTreeSet::new();
    let mut announced: BTreeMap<String, (usize, String, &Value)> = BTreeMap::new(); // other neighbours, by MAC
    for (i, w) in sws.iter().enumerate() {
        for n in rows(w.v, "neighbors") {
            let mac = up(s(n, "mac-address"));
            if mac.is_empty() {
                continue;
            }
            if names_sw.contains(s(n, "identity")) {
                switch_macs.insert(mac);
                continue;
            }
            if s(n, "identity").is_empty() {
                continue; // a network card's own agent says nothing useful
            }
            let port = list(s(n, "interface")).first().copied().unwrap_or("").to_string();
            if w.is_eth(&port) && !across[i].contains_key(&port) {
                announced.entry(mac).or_insert((i, w.logical(&port), n));
            }
        }
    }

    // MAC -> where seen (switch, interface, vid)
    let mut seen: BTreeMap<String, Vec<(usize, String, String)>> = BTreeMap::new();
    for (i, w) in sws.iter().enumerate() {
        for h in rows(w.v, "hosts") {
            let mac = up(s(h, "mac-address"));
            if !mac.is_empty() && !switch_macs.contains(&mac) {
                seen.entry(mac).or_default().push((i, s(h, "on-interface").to_string(), s(h, "vid").to_string()));
            }
        }
    }
    let knows = |j: usize, mac: &str| seen.get(mac).is_some_and(|v| v.iter().any(|(k, ..)| *k == j));
    let mut place: BTreeMap<String, (usize, String, Vec<String>)> = BTreeMap::new();
    for (mac, at) in &seen {
        // where it is plugged: a port that is not a link between switches; else a link
        // towards a switch that does not know it (a router without a MAC table, a VM host behind it)
        let mut cand: Vec<&(usize, String, String)> = at.iter().filter(|(i, p, _)| !across[*i].contains_key(p)).collect();
        if cand.is_empty() {
            cand = at.iter().filter(|(i, p, _)| !knows(across[*i][p], mac)).collect();
        }
        cand.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| natural(&x.1, &y.1)));
        let Some((i, p, _)) = cand.first() else { continue };
        let mut vids: Vec<String> = at.iter().filter(|(k, q, v)| k == i && q == p && !v.is_empty()).map(|x| x.2.clone()).collect();
        vids.dedup();
        place.insert(mac.clone(), (*i, p.clone(), vids));
    }
    for (mac, (i, p, _)) in &announced {
        place.entry(mac.clone()).or_insert((*i, p.clone(), vec![]));
    }

    // IP and name (as the Devices page did): leases, the switches' ARP, this host's ARP, DNS
    let mut ip_of: BTreeMap<String, String> = arp.iter().map(|(ip, mac)| (up(mac), ip.clone())).collect();
    let mut name_of: BTreeMap<String, String> = BTreeMap::new();
    for w in &sws {
        for a in rows(w.v, "arp") {
            ip_of.insert(up(s(a, "mac-address")), s(a, "address").to_string());
        }
    }
    for w in &sws {
        for l in rows(w.v, "leases") {
            let mac = up(s(l, "mac-address"));
            if mac.is_empty() {
                continue;
            }
            if s(l, "status") == "bound" || !ip_of.contains_key(&mac) {
                ip_of.insert(mac.clone(), s(l, "address").to_string());
            }
            let n = Some(s(l, "host-name")).filter(|x| !x.is_empty()).unwrap_or_else(|| s(l, "comment").trim_start_matches("ansible:").trim());
            if !n.is_empty() {
                name_of.insert(mac, n.to_string());
            }
        }
    }

    let mut devices: Vec<Value> = place.iter().map(|(mac, (i, port, vids))| {
        let nb = announced.get(mac).map(|x| x.2);
        let ip = ip_of.get(mac).cloned().unwrap_or_default();
        let name = name_of.get(mac).cloned().or_else(|| names.get(&ip).cloned())
            .or_else(|| nb.map(|n| s(n, "identity").to_string()).filter(|x| !x.is_empty())).unwrap_or_default();
        let (k, why) = kind(mac, &name, nb);
        json!({"mac": mac, "sw": sws[*i].name, "port": port, "vids": vids, "ip": ip, "name": name, "kind": k, "why": why,
            "announce": nb.map(|n| json!({"identity": s(n, "identity"), "platform": s(n, "platform"), "board": s(n, "board")}))})
    }).collect();

    // a port with virtual machines: the physical MAC among them is their host
    let mut vms: BTreeMap<(String, String), usize> = BTreeMap::new();
    for d in &devices {
        if is_virtual(s(d, "mac")) {
            *vms.entry((s(d, "sw").into(), s(d, "port").into())).or_default() += 1;
        }
    }
    for d in devices.iter_mut() {
        let n = vms.get(&(s(d, "sw").to_string(), s(d, "port").to_string())).copied().unwrap_or(0);
        if n > 0 && !is_virtual(s(d, "mac")) && !is_random(s(d, "mac")) && ["unknown", "pc"].contains(&s(d, "kind")) && d["why"]["by"] != "name" {
            d["kind"] = json!("server");
            d["why"] = json!({"by": "vms", "v": n});
        }
    }
    // set by hand (Devices page): the name, then the kind; the guess is kept for the page
    let manual = labels();
    for d in devices.iter_mut() {
        let Some(l) = manual.get(s(d, "mac")) else { continue };
        d["manual"] = l.clone();
        d["guess"] = json!({"kind": d["kind"], "why": d["why"], "name": d["name"]});
        if let Some(n) = l["name"].as_str() {
            d["name"] = json!(n);
            if let (Some(k), false) = (by_name(n), ["announce", "vms"].contains(&d["why"]["by"].as_str().unwrap_or(""))) {
                d["kind"] = json!(k);
                d["why"] = json!({"by": "name", "v": n});
            }
        }
        if let Some(k) = l["kind"].as_str() {
            d["kind"] = json!(k);
            d["why"] = json!({"by": "hand"});
        }
    }
    let order: BTreeMap<&str, usize> = sws.iter().enumerate().map(|(i, w)| (w.name, i)).collect();
    devices.sort_by(|a, b| order[s(a, "sw")].cmp(&order[s(b, "sw")]).then_with(|| natural(s(a, "port"), s(b, "port")))
        .then_with(|| s(a, "name").is_empty().cmp(&s(b, "name").is_empty())).then_with(|| s(a, "name").cmp(s(b, "name"))).then_with(|| s(a, "mac").cmp(s(b, "mac"))));
    json!({"links": links, "devices": devices})
}

/* ---------- names and kinds set by hand ---------- */

// data_dir/kinds.json: MAC -> {"kind", "name"} (a plain string = a kind, as first written)
static LABELS: OnceLock<(String, Mutex<BTreeMap<String, Value>>)> = OnceLock::new();

pub fn init(data_dir: &str) {
    let path = format!("{data_dir}/kinds.json");
    let raw: BTreeMap<String, Value> = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let map = raw.into_iter().map(|(m, v)| (m, if v.is_string() { json!({"kind": v}) } else { v })).collect();
    let _ = LABELS.set((path, Mutex::new(map)));
}

fn labels() -> BTreeMap<String, Value> {
    LABELS.get().map_or_else(BTreeMap::new, |(_, m)| m.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

pub fn kinds() -> Value {
    json!(labels())
}

/// MAC -> name set by hand, for the DNS names of the leases.
pub fn manual_names() -> BTreeMap<String, String> {
    labels().into_iter().filter_map(|(m, v)| Some((m, v["name"].as_str()?.to_string()))).collect()
}

fn is_mac(m: &str) -> bool {
    m.len() == 17 && m.split(':').count() == 6 && m.split(':').all(|x| x.len() == 2 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

/// {"mac", "kind", "name"}; empty values go back to the guess.
pub fn set_label(v: &Value) -> Result<(), String> {
    let mac = s(v, "mac").to_uppercase();
    let k = s(v, "kind");
    let name = s(v, "name").trim();
    if !is_mac(&mac) {
        return Err("invalid MAC address".into());
    }
    if !k.is_empty() && !KINDS.contains(&k) {
        return Err("unknown kind".into());
    }
    if name.chars().count() > 63 || name.chars().any(|c| c.is_control() || "<>\"".contains(c)) {
        return Err("invalid name (63 characters at most)".into());
    }
    let (path, m) = LABELS.get().ok_or("kinds not loaded")?;
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    let mut new = g.clone();
    let mut l = json!({});
    if !k.is_empty() {
        l["kind"] = json!(k);
    }
    if !name.is_empty() {
        l["name"] = json!(name);
    }
    if k.is_empty() && name.is_empty() {
        new.remove(&mac);
    } else {
        new.insert(mac, l);
    }
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(&new).unwrap()).and_then(|_| fs::rename(&tmp, path)).map_err(|e| e.to_string())?;
    *g = new;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nb(port: &str, who: &str, remote: &str, mac: &str) -> Value {
        json!({"interface": port, "identity": who, "interface-name": remote, "mac-address": mac, "platform": "MikroTik", "board": "CRS"})
    }
    fn mon(ports: &[&str]) -> Value {
        json!(ports.iter().map(|p| json!({"name": p})).collect::<Vec<_>>())
    }

    /// CORE has a 2-port LACP bond to EDGE, a bond to ACCESS, a single link to
    /// LAB (a router without MAC table, in a VM on the host behind core sfp9),
    /// plus redundant links blocked by STP, heard on one side only. MNDP
    /// crosses the bridges: everyone hears everyone on its uplink.
    fn fixture() -> Vec<Value> {
        vec![
            json!({"name": "CORE", "monitor": mon(&["ether1", "sfp1", "sfp2", "sfp3", "sfp4", "sfp5", "sfp9"]),
                "bonds": [{"name": "bond-edge", "slaves": "sfp1,sfp2"}, {"name": "bond-access", "slaves": "sfp3,sfp4"}],
                "neighbors": [nb("sfp1,bond-edge,bridge", "EDGE", "bridge/bond-core/combo1", "02:00:00:00:00:E1"),
                    nb("sfp2,bond-edge,bridge", "EDGE", "bridge/bond-core/combo3", "02:00:00:00:00:E1"),
                    nb("sfp3,bond-access,bridge", "ACCESS", "bridge/bond-core/sfp1", "02:00:00:00:00:A1"),
                    nb("sfp4,bond-access,bridge", "ACCESS", "bridge/bond-core/sfp2", "02:00:00:00:00:A1"),
                    nb("sfp9,bridge", "LAB", "ether1", "02:00:00:00:00:B1"),
                    json!({"interface": "sfp5,bridge", "identity": "AP1", "interface-name": "eth0", "mac-address": "02:00:00:00:00:F1", "platform": "", "board": "UAP-nanoHD,"}),
                    nb("mgmt99", "EDGE", "mgmt99", "02:00:00:00:00:E1"),
                    json!({"interface": "sfp9,bridge", "identity": "", "mac-address": "00:E0:4C:00:00:02", "system-caps-enabled": "bridge,station-only"})],
                "hosts": [{"mac-address": "02:00:00:00:00:F1", "on-interface": "sfp5", "vid": "10"},
                    {"mac-address": "AA:00:00:00:00:01", "on-interface": "sfp5", "vid": "10"},
                    {"mac-address": "00:11:32:00:00:01", "on-interface": "bond-access", "vid": "10"},
                    {"mac-address": "00:E0:4C:00:00:01", "on-interface": "sfp9", "vid": "10"},
                    {"mac-address": "BC:24:11:00:00:01", "on-interface": "sfp9", "vid": "10"},
                    {"mac-address": "BC:24:11:00:00:02", "on-interface": "sfp9", "vid": "20"},
                    {"mac-address": "02:00:00:00:00:E1", "on-interface": "bond-edge", "vid": "10"}],
                "leases": [{"mac-address": "00:11:32:00:00:01", "address": "10.0.0.5", "host-name": "", "comment": "ansible: storage", "status": "bound"}]}),
            json!({"name": "EDGE", "monitor": mon(&["combo1", "combo3", "ether9"]),
                "bonds": [{"name": "bond-core", "slaves": "combo1,combo3"}],
                "neighbors": [nb("combo1,bond-core,bridge", "CORE", "bridge/bond-edge/sfp1", "02:00:00:00:00:C1"),
                    nb("combo1,bond-core,bridge", "LAB", "ether1", "02:00:00:00:00:B1"),
                    nb("combo3,bond-core,bridge", "ACCESS", "bridge/bond-core/sfp2", "02:00:00:00:00:A1"),
                    nb("combo3,bond-core,bridge", "CORE", "bridge/bond-edge/sfp2", "02:00:00:00:00:C1")],
                "hosts": [{"mac-address": "00:11:32:00:00:01", "on-interface": "bond-core", "vid": "10"}]}),
            json!({"name": "ACCESS", "monitor": mon(&["sfp1", "sfp2", "ether17", "ether18", "ether19"]),
                "bonds": [{"name": "bond-core", "slaves": "sfp1,sfp2"}],
                "neighbors": [nb("ether17,bridge", "EDGE", "bridge/ether9", "02:00:00:00:00:E1"),
                    nb("ether18,bridge", "CORE", "bridge/ether1", "02:00:00:00:00:C1"),
                    nb("sfp1,bond-core,bridge", "CORE", "bridge/bond-access/sfp3", "02:00:00:00:00:C1"),
                    nb("sfp1,bond-core,bridge", "LAB", "ether1", "02:00:00:00:00:B1"),
                    nb("sfp2,bond-core,bridge", "EDGE", "bridge/bond-core/combo1", "02:00:00:00:00:E1"),
                    nb("sfp2,bond-core,bridge", "EDGE", "bridge/bond-core/combo3", "02:00:00:00:00:E1"),
                    nb("sfp2,bond-core,bridge", "CORE", "bridge/bond-access/sfp4", "02:00:00:00:00:C1")],
                "hosts": [{"mac-address": "00:11:32:00:00:01", "on-interface": "ether19", "vid": "10"},
                    {"mac-address": "BC:24:11:00:00:01", "on-interface": "bond-core", "vid": "10"}]}),
            json!({"name": "LAB", "monitor": mon(&["ether1"]), "bonds": [], "hosts": [],
                "neighbors": [nb("ether1", "EDGE", "bridge/bond-core/combo1", "02:00:00:00:00:E1"), nb("ether1", "EDGE", "bridge/bond-core/combo3", "02:00:00:00:00:E1"),
                    nb("ether1", "ACCESS", "bridge/bond-core/sfp1", "02:00:00:00:00:A1"), nb("ether1", "ACCESS", "bridge/bond-core/sfp2", "02:00:00:00:00:A1"),
                    nb("ether1", "CORE", "bridge/sfp9", "02:00:00:00:00:C1")]}),
        ]
    }

    #[test]
    fn links_through_bridges() {
        let t = build(&fixture(), &[], &BTreeMap::new());
        let got: Vec<String> = t["links"].as_array().unwrap().iter()
            .map(|l| format!("{} {} {} {} {}", l["a"].as_str().unwrap(), l["a_via"].as_str().unwrap(), l["b"].as_str().unwrap(), l["b_via"].as_str().unwrap(), l["pairs"])).collect();
        assert_eq!(got, [
            r#"CORE bond-access ACCESS bond-core [["sfp3","sfp1"],["sfp4","sfp2"]]"#,
            r#"CORE bond-edge EDGE bond-core [["sfp1","combo1"],["sfp2","combo3"]]"#,
            r#"CORE ether1 ACCESS ether18 [["ether1","ether18"]]"#,
            r#"CORE sfp9 LAB ether1 [["sfp9","ether1"]]"#,
            r#"EDGE ether9 ACCESS ether17 [["ether9","ether17"]]"#,
        ]);
        assert_eq!(t["links"][0]["lacp"], true);
        assert_eq!(t["links"][3]["lacp"], false);
    }

    #[test]
    fn devices_where_plugged() {
        let t = build(&fixture(), &[("10.0.0.9".into(), "00:e0:4c:00:00:01".into())], &BTreeMap::from([("10.0.0.9".into(), "host1".into())]));
        let got: Vec<String> = t["devices"].as_array().unwrap().iter()
            .map(|d| format!("{} {} {} {} {}", d["mac"].as_str().unwrap(), d["sw"].as_str().unwrap(), d["port"].as_str().unwrap(), d["kind"].as_str().unwrap(), d["name"].as_str().unwrap())).collect();
        assert_eq!(got, [
            "02:00:00:00:00:F1 CORE sfp5 ap AP1",          // announced by LLDP (UniFi board)
            "AA:00:00:00:00:01 CORE sfp5 unknown ",         // behind the AP
            "00:E0:4C:00:00:01 CORE sfp9 server host1",     // VM host: link to LAB, which has no MAC table
            "BC:24:11:00:00:01 CORE sfp9 server ",
            "BC:24:11:00:00:02 CORE sfp9 server ",
            "00:11:32:00:00:01 ACCESS ether19 nas storage", // on a link everywhere else
        ]);
        assert_eq!(t["devices"][2]["why"], json!({"by": "vms", "v": 2}));
        assert_eq!(t["devices"][1]["why"], json!({"by": "random"}));
    }

    #[test]
    fn names_and_announcements() {
        assert_eq!(by_name("Printer-Office"), Some("printer"));
        assert_eq!(by_name("cam2-garden"), Some("camera"));
        assert_eq!(by_name("iPhone-de-Bob"), Some("phone"));
        assert_eq!(by_name("esp32-kitchen"), Some("iot"));
        assert_eq!(by_name("campaign"), None, "short words match whole words only");
        assert_eq!(by_announce(&json!({"system-caps-enabled": "bridge,router", "platform": "MikroTik", "board": "CHR"})), Some("router"));
        assert_eq!(by_announce(&json!({"system-caps-enabled": "bridge", "board": "CRS326-24G-2S+"})), Some("switch"));
        assert_eq!(by_announce(&json!({"platform": "Linux"})), None);
        assert_eq!(by_announce(&json!({"platform": "UAP-nanoHD,", "system-caps": "bridge,wlan-ap,router,station-only", "system-caps-enabled": "bridge,wlan-ap,router"})), Some("ap"));
        assert!(natural("sfp9", "sfp10").is_lt());
    }

    #[test]
    fn names_and_kinds_by_hand() {
        let dir = std::env::temp_dir().join(format!("cairn-net-kinds-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("kinds.json"), r#"{"AA:BB:CC:DD:EE:01": "nas"}"#).unwrap(); // first format: kind only
        init(dir.to_str().unwrap());
        assert_eq!(kinds()["AA:BB:CC:DD:EE:01"], json!({"kind": "nas"}));
        set_label(&json!({"mac": "aa:bb:cc:dd:ee:ff", "kind": "", "name": " printer-hall "})).unwrap();
        let sw = json!({"name": "S", "monitor": [{"name": "ether1"}], "hosts": [{"mac-address": "AA:BB:CC:DD:EE:FF", "on-interface": "ether1"}]});
        let d = &build(&[sw.clone()], &[], &BTreeMap::new())["devices"][0];
        assert_eq!((d["name"].as_str(), d["kind"].as_str(), d["guess"]["name"].as_str()), (Some("printer-hall"), Some("printer"), Some("")));
        set_label(&json!({"mac": "AA:BB:CC:DD:EE:FF", "kind": "camera", "name": "printer-hall"})).unwrap();
        let d = &build(&[sw.clone()], &[], &BTreeMap::new())["devices"][0];
        assert_eq!((d["kind"].as_str(), &d["why"]), (Some("camera"), &json!({"by": "hand"})));
        assert!(set_label(&json!({"mac": "nope", "kind": "nas"})).is_err());
        assert!(set_label(&json!({"mac": "aa:bb:cc:dd:ee:ff", "kind": "toaster"})).is_err());
        assert!(set_label(&json!({"mac": "aa:bb:cc:dd:ee:ff", "name": "<b>"})).is_err());
        set_label(&json!({"mac": "AA:BB:CC:DD:EE:FF", "kind": "", "name": ""})).unwrap();
        set_label(&json!({"mac": "AA:BB:CC:DD:EE:01"})).unwrap();
        assert_eq!(kinds(), json!({}));
        fs::remove_dir_all(&dir).unwrap();
    }
}
