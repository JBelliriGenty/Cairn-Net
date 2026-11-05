//! Interfaces and bridge page: ethernet ports, bonds, bridges with their
//! ports and VLAN table, VLAN interfaces, the host table and the STP state
//! of every bridge port (read account, every switch).
//!
//! Direct mode writes one object at a time (add, set or remove), previewed
//! first and read back after. Anything but a comment can cut traffic, so it
//! goes through the rollback of the Infrastructure page (`guarded_apply`):
//! the switch puts the object back by itself unless the change is kept.
//! A change touching Cairn Net's path to the switch is refused: the
//! interfaces holding either end of the path, the VLAN and bridge under
//! them, the bridge port the app's packets arrive on (host table) and the
//! bond members behind it, and that VLAN's membership in the VLAN table.

use crate::direct::Cmd;
use crate::infra::{self, q, Path};
use crate::{print, Api};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn yes(v: &Value, k: &str) -> bool {
    matches!(s(v, k), "true" | "yes")
}

fn all(api: &mut Api, path: &str) -> Result<Vec<Value>, String> {
    match api.call(&[&format!("{path}/print")]) {
        Err(e) if e.starts_with("no such command") => Ok(vec![]),
        r => r,
    }
}

/// What the page shows, read with the read account.
pub fn read(api: &mut Api) -> Result<Value, String> {
    let ether = all(api, "/interface/ethernet")?;
    let names: Vec<&str> = ether.iter().map(|r| s(r, "name")).collect();
    let monitor = if names.is_empty() { vec![] } else {
        api.call(&["/interface/ethernet/monitor", &format!("=numbers={}", names.join(",")), "=once="]).unwrap_or_default()
    };
    Ok(json!({
        "ifaces": print(api, "/interface", ".id,name,type,running,disabled,slave,actual-mtu,mac-address,comment", &[])?,
        "ether": ether,
        "monitor": monitor,
        "bonds": all(api, "/interface/bonding")?,
        "bridges": all(api, "/interface/bridge")?,
        "bports": all(api, "/interface/bridge/port")?,
        "bvlans": all(api, "/interface/bridge/vlan")?,
        "vlans": all(api, "/interface/vlan")?,
        // ponytail: whole table; a big L2 network makes a big page (filtered in the page)
        "hosts": all(api, "/interface/bridge/host")?,
        "addresses": print(api, "/ip/address", "address,interface,disabled", &[])?,
        "routes": print(api, "/ip/route", "dst-address,gateway,active", &["?active=true"])?,
        "arp": print(api, "/ip/arp", "address,mac-address,interface", &[])?,
        "rollback": print(api, "/system/scheduler", "name,next-run", &[&format!("?name={}", infra::ROLLBACK)])?.pop(),
    }))
}

/* ---------- Cairn Net's path through the switch ---------- */

/// The objects Cairn Net's packets go through. `port: None` with a bridge:
/// the arrival port is unknown, so every port of that bridge counts.
#[derive(Debug, Default)]
pub struct Mgmt {
    pub ifaces: BTreeSet<String>,
    pub bridge: Option<String>,
    pub port: Option<String>,
    pub vid: Option<u32>,
    pub l3_on_bridge: bool, // the address sits on the bridge or on a VLAN of it: the bridge is a VLAN member
}

fn ip_in(net: &str, ip: std::net::Ipv4Addr) -> bool {
    net.ends_with("/0") || infra::cidr(net).is_some_and(|(_, n, l)| u32::from(ip) & u32::MAX.checked_shl(32 - l).unwrap_or(0) == n)
}

fn arr<'a>(live: &'a Value, k: &str) -> impl Iterator<Item = &'a Value> {
    live[k].as_array().into_iter().flatten()
}

/// The interface the app's packets enter through (on the app's network, or
/// towards the next hop of the route to it) is followed down to the bridge
/// port they arrive on and their VLAN; the interface holding the switch's
/// address (when another one) is protected too, without its VLAN.
pub fn mgmt(live: &Value, p: &Path) -> Mgmt {
    let mut m = Mgmt::default();
    let (mut entry, mut hop) = (None, None);
    if let Some(app) = p.app {
        hop = match arr(live, "addresses").find(|a| ip_in(s(a, "address"), app)) {
            Some(_) => Some(app),
            // routed: the next hop of the most specific active route towards the app
            None => arr(live, "routes").filter(|r| ip_in(s(r, "dst-address"), app))
                .max_by_key(|r| infra::cidr(s(r, "dst-address")).map_or(0, |c| c.2)) // 0.0.0.0/0 reads as 0
                .and_then(|r| s(r, "gateway").split('%').next()?.parse().ok()),
        };
        entry = hop.and_then(|h| arr(live, "addresses").find(|a| ip_in(s(a, "address"), h))).map(|a| s(a, "interface").to_string());
    }
    let hop_mac = hop.and_then(|h: std::net::Ipv4Addr| arr(live, "arp").find(|a| s(a, "address") == h.to_string()).map(|a| s(a, "mac-address").to_uppercase()));
    let slaves = |b: &str| arr(live, "bonds").find(|x| s(x, "name") == b).map(|x| s(x, "slaves").split(',').filter(|x| !x.is_empty()).map(String::from).collect::<Vec<_>>());
    let walks = entry.iter().map(|e| (e.clone(), true)).chain(infra::mgmt_iface(live, p).map(|x| (x.to_string(), false)));
    for (mut x, l2) in walks {
        let mut vid = None;
        loop {
            m.ifaces.insert(x.clone());
            if let Some(v) = arr(live, "vlans").find(|v| s(v, "name") == x) {
                vid = s(v, "vlan-id").parse().ok();
                x = s(v, "interface").to_string();
                continue;
            }
            if l2 && arr(live, "bridges").any(|b| s(b, "name") == x) {
                m.bridge = Some(x.clone());
                m.l3_on_bridge = true;
                let host = hop_mac.as_ref().and_then(|mac| arr(live, "hosts").find(|h| s(h, "mac-address").to_uppercase() == *mac && s(h, "bridge") == x));
                m.port = host.map(|h| s(h, "on-interface").to_string());
                // the VLAN the packets are learned in, else the VLAN interface's, else the port's PVID
                m.vid = host.and_then(|h| s(h, "vid").parse().ok()).or(vid)
                    .or_else(|| m.port.as_ref().and_then(|pt| arr(live, "bports").find(|b| s(b, "interface") == pt)?.get("pvid")?.as_str()?.parse().ok()));
                if let Some(pt) = m.port.clone() {
                    m.ifaces.insert(pt.clone());
                    m.ifaces.extend(slaves(&pt).unwrap_or_default());
                }
            }
            if let Some(sl) = slaves(&x) {
                m.ifaces.extend(sl);
            }
            break;
        }
    }
    m
}

impl Mgmt {
    /// Whether `name` may carry the path (unknown arrival port: every port of the bridge).
    fn carries(&self, live: &Value, name: &str) -> bool {
        if self.ifaces.contains(name) {
            return true;
        }
        let Some(b) = self.bridge.as_ref().filter(|_| self.port.is_none()) else { return false };
        arr(live, "bports").any(|p| s(p, "bridge") == b && (s(p, "interface") == name
            || arr(live, "bonds").any(|x| s(x, "name") == s(p, "interface") && s(x, "slaves").split(',').any(|y| y == name))))
    }

    /// The members whose VLAN membership the path needs.
    fn members(&self, live: &Value) -> Vec<String> {
        let Some(b) = &self.bridge else { return vec![] };
        let mut v: Vec<String> = match &self.port {
            Some(p) => vec![p.clone()],
            None => arr(live, "bports").filter(|p| s(p, "bridge") == b).map(|p| s(p, "interface").to_string()).collect(),
        };
        if self.l3_on_bridge {
            v.push(b.clone());
        }
        v
    }
}

/// "10,20,100-200" contains `vid`.
fn has_vid(ids: &str, vid: u32) -> bool {
    ids.split(',').any(|r| match r.split_once('-') {
        Some((a, b)) => a.trim().parse::<u32>().is_ok_and(|a| a <= vid) && b.trim().parse::<u32>().is_ok_and(|b| vid <= b),
        None => r.trim().parse() == Ok(vid),
    })
}

/// (tagged, untagged) of `who` in `vid` on `bridge`, over the static rows.
fn membership(rows: &[Value], bridge: &str, vid: u32, who: &str) -> (bool, bool) {
    let mut out = (false, false);
    for r in rows.iter().filter(|r| s(r, "bridge") == bridge && !yes(r, "disabled") && !yes(r, "dynamic") && has_vid(s(r, "vlan-ids"), vid)) {
        out.0 |= s(r, "tagged").split(',').any(|x| x == who);
        out.1 |= s(r, "untagged").split(',').any(|x| x == who);
    }
    out
}

/* ---------- one change ---------- */

/// Kinds of object, their API path, the properties the page may set, and the
/// properties that name a new object (rollback of an add).
static KINDS: [(&str, &str, &[&str], &[&str]); 6] = [
    ("ether", "/interface/ethernet", &["disabled", "comment", "mtu", "l2mtu", "speed", "auto-negotiation", "advertise", "rx-flow-control", "tx-flow-control"], &[]),
    ("bond", "/interface/bonding", &["name", "slaves", "mode", "transmit-hash-policy", "lacp-rate", "min-links", "mtu", "disabled", "comment"], &["name"]),
    ("bridge", "/interface/bridge", &["vlan-filtering", "protocol-mode", "priority", "comment"], &[]),
    ("bport", "/interface/bridge/port", &["bridge", "interface", "pvid", "frame-types", "ingress-filtering", "horizon", "disabled", "comment"], &["interface"]),
    ("bvlan", "/interface/bridge/vlan", &["bridge", "vlan-ids", "tagged", "untagged", "disabled", "comment"], &["bridge", "vlan-ids"]),
    ("vlan", "/interface/vlan", &["name", "vlan-id", "interface", "mtu", "disabled", "comment"], &["name"]),
];

fn kind(k: &str) -> Result<&'static (&'static str, &'static str, &'static [&'static str], &'static [&'static str]), String> {
    KINDS.iter().find(|x| x.0 == k).ok_or_else(|| format!("{k}: unknown kind"))
}

fn live_key(k: &str) -> &'static str {
    match k { "ether" => "ether", "bond" => "bonds", "bridge" => "bridges", "bport" => "bports", "bvlan" => "bvlans", _ => "vlans" }
}

/// What the page asks for: one object.
#[derive(Debug)]
pub struct Want {
    pub kind: String,
    pub op: String, // add, set, remove
    pub id: String,
    pub props: Vec<(String, String)>,
}

impl Want {
    pub fn from_json(v: &Value) -> Result<Want, String> {
        let w = Want {
            kind: s(v, "kind").into(),
            op: s(v, "op").into(),
            id: s(v, "id").into(),
            props: v["props"].as_object().into_iter().flatten()
                .map(|(k, x)| (k.clone(), x.as_str().map(|x| x.trim().to_string()).unwrap_or_else(|| x.to_string()))).collect(),
        };
        let (_, _, allowed, _) = kind(&w.kind)?;
        if let Some((k, _)) = w.props.iter().find(|(k, _)| !allowed.contains(&k.as_str())) {
            return Err(format!("{k}: not a property this page sets"));
        }
        if let Some((k, _)) = w.props.iter().find(|(_, x)| x.chars().any(char::is_control)) {
            return Err(format!("{k}: invalid value"));
        }
        Ok(w)
    }

    /// Only comments change: nothing can be cut, no rollback needed.
    pub fn harmless(&self) -> bool {
        self.op == "set" && !self.props.is_empty() && self.props.iter().all(|(k, _)| k == "comment")
    }
}

/// Values as RouterOS prints them: booleans, hex numbers, list order.
fn norm(v: &str) -> String {
    let v = v.trim();
    let v = match v { "yes" => "true", "no" => "false", x => x };
    if let Some(h) = v.strip_prefix("0x") {
        if let Ok(n) = u64::from_str_radix(h, 16) {
            return n.to_string();
        }
    }
    let mut l: Vec<&str> = v.split(',').map(str::trim).filter(|x| !x.is_empty()).collect();
    l.sort_unstable();
    l.join(",")
}

/// `key=value` for a script: RouterOS scripts take booleans as yes/no only
/// (the API prints true/false).
fn kv(k: &str, v: &str) -> String {
    match v {
        "true" | "false" if k != "comment" => format!("{k}={}", if v == "true" { "yes" } else { "no" }),
        _ => format!("{k}={}", q(v)),
    }
}

/// One rollback line that never stops the ones after it.
fn guarded(line: String) -> String {
    format!(":do {{ {line} }} on-error={{ :log warning {} }}", q("cairn-net: an interface change could not be put back"))
}

fn cli(base: &str) -> String {
    format!("/{}", base[1..].replace('/', " "))
}

/// The object of a change, as the switch has it.
fn target<'a>(live: &'a Value, w: &Want) -> Option<&'a Value> {
    arr(live, live_key(&w.kind)).find(|r| s(r, ".id") == w.id)
}

/// The object's state once the change is made (None: removed).
fn after(live: &Value, w: &Want) -> Result<Option<Map<String, Value>>, String> {
    match w.op.as_str() {
        "remove" => Ok(None),
        "add" => Ok(Some(w.props.iter().map(|(k, v)| (k.clone(), json!(v))).collect())),
        "set" => {
            let mut o = target(live, w).and_then(Value::as_object).cloned().ok_or("the object is gone: reload")?;
            for (k, v) in &w.props {
                o.insert(k.clone(), json!(v));
            }
            Ok(Some(o))
        }
        x => Err(format!("{x}: unknown operation")),
    }
}

/// The switch as it would be: `live` with the change made.
fn with_change(live: &Value, w: &Want) -> Result<Value, String> {
    let mut l = live.clone();
    let key = live_key(&w.kind);
    let rows = l[key].as_array_mut().ok_or("nothing read")?;
    rows.retain(|r| s(r, ".id") != w.id || w.op == "add");
    if let Some(o) = after(live, w)? {
        rows.push(Value::Object(o));
    }
    Ok(l)
}

/// Refusals: the path, then what RouterOS would take badly.
fn check(live: &Value, w: &Want, m: &Mgmt) -> Result<(), String> {
    let cur = target(live, w);
    if w.op != "add" && cur.is_none() {
        return Err("the object is gone: reload".into());
    }
    if w.op == "add" && matches!(w.kind.as_str(), "ether" | "bridge") || w.op == "remove" && matches!(w.kind.as_str(), "ether" | "bridge") {
        return Err(format!("{}: only edited here", w.kind));
    }
    if cur.is_some_and(|c| yes(c, "dynamic")) {
        return Err("a dynamic entry is made by RouterOS: it is not edited".into());
    }
    if w.harmless() {
        return Ok(());
    }
    let new = after(live, w)?;
    let both = |k: &str| [cur.map(|c| s(c, k).to_string()), new.as_ref().map(|n| n.get(k).and_then(Value::as_str).unwrap_or("").to_string())];
    let refuse = |what: &str, why: &str| -> Result<(), String> { Err(format!("{what} carries Cairn Net's path to the switch ({why}): refused, it could cut the app off. Change it from another way in (SSH, MAC-WinBox)")) };
    match w.kind.as_str() {
        "ether" | "bond" | "vlan" => {
            for n in both("name").into_iter().flatten() {
                if m.carries(live, &n) {
                    return refuse(&n, "an interface of the path");
                }
            }
            if w.kind == "bond" {
                for n in both("slaves").into_iter().flatten().flat_map(|l| l.split(',').map(String::from).collect::<Vec<_>>()) {
                    if m.carries(live, &n) {
                        return refuse(&n, "a member would move");
                    }
                }
                if w.op == "remove" {
                    let n = s(cur.unwrap(), "name");
                    if arr(live, "bports").any(|p| s(p, "interface") == n) || arr(live, "vlans").any(|v| s(v, "interface") == n) || arr(live, "addresses").any(|a| s(a, "interface") == n) {
                        return Err(format!("{n} is in use (bridge port, VLAN or address): remove that first"));
                    }
                }
            }
        }
        "bridge" => {
            if m.ifaces.contains(s(cur.unwrap(), "name")) {
                return refuse(s(cur.unwrap(), "name"), "the bridge of the path");
            }
        }
        "bport" => {
            for n in both("interface").into_iter().flatten() {
                if m.carries(live, &n) {
                    return refuse(&n, if m.port.as_deref() == Some(n.as_str()) { "the port Cairn Net's packets arrive on" } else { "an interface of the path" });
                }
            }
            if w.op == "add" {
                let n = new.as_ref().and_then(|n| n.get("interface")).and_then(Value::as_str).unwrap_or("");
                if arr(live, "addresses").any(|a| s(a, "interface") == n) {
                    return Err(format!("{n} has an IP address: a bridge port loses it, move it first"));
                }
            }
        }
        "bvlan" => {
            if let (Some(b), Some(vid)) = (&m.bridge, m.vid) {
                let before: Vec<Value> = arr(live, "bvlans").cloned().collect();
                let later: Vec<Value> = arr(&with_change(live, w)?, "bvlans").cloned().collect();
                for who in m.members(live) {
                    if membership(&before, b, vid, &who) != membership(&later, b, vid, &who) {
                        return refuse(&format!("VLAN {vid} of {who} on {b}"), "a member of the path would change");
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The command, and the RouterOS script that undoes it.
pub fn plan(live: &Value, w: &Want, p: &Path) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let (_, base, allowed, keys) = kind(&w.kind)?;
    let m = mgmt(live, p);
    if p.app.is_none() {
        return Err("Cairn Net's own address is unknown: no change".into());
    }
    check(live, w, &m)?;
    let cur = target(live, w);
    let mut undo = Vec::new();
    let cmd: Cmd = match w.op.as_str() {
        "set" => {
            let c = cur.unwrap();
            let diff: Vec<(String, String)> = w.props.iter().filter(|(k, v)| norm(s(c, k)) != norm(v)).cloned().collect();
            if diff.is_empty() {
                return Ok((vec![], vec![]));
            }
            let find = format!("[find where .id={}]", s(c, ".id"));
            let old: Vec<String> = diff.iter().map(|(k, _)| kv(k, s(c, k))).collect();
            undo.push(guarded(format!("{} set {find} {}", cli(base), old.join(" "))));
            (format!("{base}/set"), std::iter::once((".id".to_string(), s(c, ".id").to_string())).chain(diff).collect())
        }
        "add" => {
            let mut find: Vec<String> = Vec::new();
            for k in *keys {
                let v = w.props.iter().find(|(x, _)| x == k).map(|x| x.1.as_str()).filter(|v| !v.is_empty()).ok_or_else(|| format!("{k} is needed"))?;
                find.push(format!("{k}={}", q(v)));
            }
            undo.push(guarded(format!("{} remove [find where {}]", cli(base), find.join(" and "))));
            (format!("{base}/add"), w.props.iter().filter(|(_, v)| !v.is_empty()).cloned().collect())
        }
        "remove" => {
            let c = cur.unwrap();
            let back: Vec<String> = allowed.iter().filter(|k| !s(c, k).is_empty()).map(|k| kv(k, s(c, k))).collect();
            undo.push(guarded(format!("{} add {}", cli(base), back.join(" "))));
            (format!("{base}/remove"), vec![(".id".into(), s(c, ".id").into())])
        }
        x => return Err(format!("{x}: unknown operation")),
    };
    if w.harmless() {
        undo.clear();
    } else {
        infra::finish(&mut undo);
    }
    Ok((vec![cmd], undo))
}

/// Whether the switch now runs what was asked.
pub fn matches(live: &Value, w: &Want) -> bool {
    let rows: Vec<&Value> = arr(live, live_key(&w.kind)).collect();
    let same = |r: &Value| w.props.iter().all(|(k, v)| norm(s(r, k)) == norm(v));
    match w.op.as_str() {
        "remove" => !rows.iter().any(|r| s(r, ".id") == w.id),
        "set" => rows.iter().any(|r| s(r, ".id") == w.id && same(r)),
        _ => rows.iter().any(|r| same(r)),
    }
}

/* ---------- HTTP (main.rs routes) ---------- */

/// GET /api/ifaces?switch=: every switch, read account.
pub fn view(name: &str) -> Value {
    let Some(sw) = crate::settings::get(name) else { return json!({"error": format!("{name}: unknown switch")}) };
    let mut v = crate::with_api(&sw, |api| {
        let mut v = read(api)?;
        if let Ok(p) = crate::path_of(&sw.cfg, api) {
            let m = mgmt(&v, &p);
            v["mgmt"] = json!({"ifaces": m.ifaces, "bridge": m.bridge, "port": m.port, "vid": m.vid, "app": p.app.map(|a| a.to_string())});
        }
        Ok(v)
    }).unwrap_or_else(|e| json!({"error": e}));
    v["name"] = json!(sw.cfg.name);
    v["mode"] = json!(sw.cfg.mode);
    v["rollback_s"] = json!(infra::ROLLBACK_S);
    v
}

fn planned(c: &crate::settings::SwitchCfg, api: &mut Api, w: &Want) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let p = crate::path_of(c, api)?;
    plan(&read(api)?, w, &p)
}

/// POST /api/ifaces/preview: the command and its rollback, nothing written.
pub fn preview(v: &Value) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let w = Want::from_json(v)?;
    let (cmds, undo) = crate::with_api(&sw, |api| planned(&sw.cfg, api, &w))?;
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S, "guarded": !w.harmless()}))
}

/// POST /api/ifaces/apply: guarded like the Infrastructure page (confirmed
/// through /api/infra/confirm); a comment alone is written and read back.
pub fn apply(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let c = &sw.cfg;
    let w = Want::from_json(v)?;
    let what = format!("ifaces {} {}", w.kind, w.op);
    if !w.harmless() {
        return crate::guarded_apply(c, user, &what, |api| planned(c, api, &w), |api| Ok(matches(&read(api)?, &w)));
    }
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let (cmds, _) = planned(c, &mut api, &w)?;
    let (done, err) = crate::direct::run(&mut api, &cmds);
    drop(api);
    crate::forget_cache();
    let verified = crate::with_api(&sw, |api| Ok(matches(&read(api)?, &w))).unwrap_or(false);
    eprintln!("{what} applied on {} by {user}: {done}/{}, read back: {verified}", c.name, cmds.len());
    Ok(json!({"ok": err.is_none() && verified, "done": done, "commands": infra::show_all(&cmds), "error": err, "verified": verified, "pending": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    // the app (192.168.1.69) on the LAN bridge, switch address on VLAN 99 of the same bridge,
    // packets arrive on sfp2 (host table), a bond on sfp3+sfp4 carries VLAN 10 and 99 tagged
    fn live() -> Value {
        json!({
            "addresses": [{"address": "192.168.99.11/24", "interface": "mgmt99"}, {"address": "192.168.1.11/24", "interface": "bridge"}],
            "vlans": [{".id": "*20", "name": "mgmt99", "vlan-id": "99", "interface": "bridge"}, {".id": "*21", "name": "test", "vlan-id": "300", "interface": "bridge"}],
            "bridges": [{".id": "*1", "name": "bridge", "vlan-filtering": "true", "priority": "0x8000"}, {".id": "*2", "name": "lab", "vlan-filtering": "false"}],
            "bonds": [{".id": "*30", "name": "bond1", "slaves": "sfp3,sfp4", "mode": "802.3ad"}],
            "bports": [{".id": "*a", "bridge": "bridge", "interface": "sfp2", "pvid": "10"}, {".id": "*b", "bridge": "bridge", "interface": "bond1", "pvid": "10"},
                {".id": "*c", "bridge": "bridge", "interface": "ether5", "pvid": "20"}],
            "bvlans": [{".id": "*v1", "bridge": "bridge", "vlan-ids": "10", "tagged": "bridge,bond1", "untagged": "sfp2"},
                {".id": "*v2", "bridge": "bridge", "vlan-ids": "99", "tagged": "bridge,sfp2,bond1", "untagged": ""},
                {".id": "*v3", "bridge": "bridge", "vlan-ids": "20,300-310", "tagged": "bridge,bond1", "untagged": "ether5"}],
            "ether": [{".id": "*e2", "name": "sfp2", "mtu": "1500"}, {".id": "*e3", "name": "sfp3"}, {".id": "*e5", "name": "ether5", "mtu": "1500", "comment": ""}],
            "arp": [{"address": "192.168.1.69", "mac-address": "bc:24:11:00:00:69"}],
            "hosts": [{"mac-address": "BC:24:11:00:00:69", "on-interface": "sfp2", "bridge": "bridge"}],
            "routes": [],
        })
    }

    fn path() -> Path {
        Path { switch: "192.168.99.11".parse().unwrap(), app: Some("192.168.1.69".parse().unwrap()) }
    }

    fn want(kind: &str, op: &str, id: &str, props: &[(&str, &str)]) -> Want {
        Want { kind: kind.into(), op: op.into(), id: id.into(), props: props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn path_follows_vlan_bridge_and_host_table() {
        let m = mgmt(&live(), &path());
        assert_eq!(m.bridge.as_deref(), Some("bridge"));
        assert_eq!(m.port.as_deref(), Some("sfp2"));
        // the app is on the bridge's own network: its packets arrive untagged, VLAN 10 (sfp2's PVID), then are routed to mgmt99
        assert_eq!(m.vid, Some(10));
        assert!(m.ifaces.contains("mgmt99") && m.ifaces.contains("sfp2") && !m.ifaces.contains("ether5"));
    }

    #[test]
    fn routed_path_uses_the_gateway() {
        let mut l = live();
        l["addresses"] = json!([{"address": "192.168.99.12/24", "interface": "mgmt99"}]);
        l["routes"] = json!([{"dst-address": "0.0.0.0/0", "gateway": "192.168.99.11"}]);
        l["arp"] = json!([{"address": "192.168.99.11", "mac-address": "AA:00:00:00:00:01"}]);
        l["hosts"] = json!([{"mac-address": "AA:00:00:00:00:01", "on-interface": "bond1", "bridge": "bridge"}]);
        let p = Path { switch: "192.168.99.12".parse().unwrap(), app: path().app };
        let m = mgmt(&l, &p);
        assert_eq!(m.port.as_deref(), Some("bond1"));
        assert!(m.ifaces.contains("sfp3") && m.ifaces.contains("sfp4"));
        assert_eq!(m.vid, Some(99));
        // a member of the path's bond cannot be disabled
        assert!(plan(&l, &want("ether", "set", "*e3", &[("disabled", "true")]), &p).is_err());
    }

    #[test]
    fn unknown_arrival_port_protects_every_port_of_the_bridge() {
        let mut l = live();
        l["hosts"] = json!([]);
        assert!(plan(&l, &want("bport", "set", "*c", &[("pvid", "30")]), &path()).is_err());
        assert!(plan(&l, &want("ether", "set", "*e5", &[("mtu", "9000")]), &path()).is_err());
        // a comment stays possible
        assert!(plan(&l, &want("ether", "set", "*e5", &[("comment", "x")]), &path()).is_ok());
    }

    #[test]
    fn path_objects_are_refused() {
        let l = live();
        for w in [want("ether", "set", "*e2", &[("disabled", "true")]), want("bport", "set", "*a", &[("pvid", "99")]),
            want("bport", "remove", "*a", &[]), want("bridge", "set", "*1", &[("vlan-filtering", "false")]),
            want("vlan", "remove", "*20", &[]), want("bvlan", "set", "*v1", &[("untagged", "")]),
            want("bvlan", "remove", "*v1", &[]), want("bvlan", "add", "", &[("bridge", "bridge"), ("vlan-ids", "10"), ("tagged", "sfp2")])] {
            let e = plan(&l, &w, &path()).unwrap_err();
            assert!(e.contains("Cairn Net's path"), "{w:?}: {e}");
        }
    }

    #[test]
    fn other_changes_plan_a_command_and_its_rollback() {
        let l = live();
        let (c, u) = plan(&l, &want("bport", "set", "*c", &[("pvid", "30"), ("frame-types", "admit-only-untagged-and-priority-tagged")]), &path()).unwrap();
        assert_eq!(c[0].0, "/interface/bridge/port/set");
        assert!(u[0].starts_with(":do { /interface bridge port set [find where .id=*c] pvid=\"20\""), "{u:?}");
        // booleans go back as yes/no: a script refuses "false"
        let (_, u) = plan(&l, &want("bridge", "set", "*2", &[("vlan-filtering", "yes")]), &path()).unwrap();
        assert!(u[0].contains("vlan-filtering=no }"), "{u:?}");
        assert!(u.last().unwrap().contains("scheduler remove"));
        // rows of other VLANs may change: the path's ports keep VLAN 10
        assert!(plan(&l, &want("bvlan", "remove", "*v2", &[]), &path()).is_ok());
        assert!(plan(&l, &want("bvlan", "set", "*v3", &[("untagged", "")]), &path()).is_ok());
        let (_, u) = plan(&l, &want("bond", "add", "", &[("name", "bond2"), ("slaves", "ether6,ether7"), ("mode", "802.3ad")]), &path()).unwrap();
        assert!(u[0].contains("/interface bonding remove [find where name=\"bond2\"] }"), "{u:?}");
        let (_, u) = plan(&l, &want("vlan", "remove", "*21", &[]), &path()).unwrap();
        assert!(u[0].contains("/interface vlan add name=\"test\" vlan-id=\"300\" interface=\"bridge\" }"), "{u:?}");
        // a comment alone: no rollback
        let (_, u) = plan(&l, &want("ether", "set", "*e5", &[("comment", "uplink")]), &path()).unwrap();
        assert!(u.is_empty());
        // same value: nothing to do
        assert!(plan(&l, &want("bridge", "set", "*2", &[("vlan-filtering", "no")]), &path()).unwrap().0.is_empty());
    }

    #[test]
    fn bond_in_use_is_not_removed() {
        let mut l = live();
        l["hosts"] = json!([{"mac-address": "BC:24:11:00:00:69", "on-interface": "sfp2", "bridge": "bridge"}]);
        let e = plan(&l, &want("bond", "remove", "*30", &[]), &path()).unwrap_err();
        assert!(e.contains("in use"), "{e}");
    }

    #[test]
    fn read_back_normalizes() {
        let mut l = live();
        l["bridges"][0]["priority"] = json!("0x7000");
        assert!(matches(&l, &want("bridge", "set", "*1", &[("priority", "28672")])));
        assert!(matches(&l, &want("bvlan", "set", "*v1", &[("tagged", "bond1,bridge")])));
        assert!(matches(&l, &want("bvlan", "remove", "*zz", &[])));
        assert!(!matches(&l, &want("bvlan", "remove", "*v1", &[])));
        assert!(has_vid("20,300-310", 305) && !has_vid("20,300-310", 299));
    }
}
