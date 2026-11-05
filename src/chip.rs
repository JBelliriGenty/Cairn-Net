//! Switch chip page: what the switch chip does in hardware (CRS3xx: port
//! mirror, rate limits, storm control, hardware ACL rules, port isolation,
//! counters, bridge offload), IGMP / MLD snooping, queues and PoE.
//!
//! Read for every switch; writes in direct mode only, as a list of row
//! operations (set / add / remove) on a few known tables and fields, through
//! the same guarded apply as the Infrastructure and Firewall pages: rollback
//! armed on the switch first, read back, confirmed by the page. A table the
//! switch does not have ("no such command": CHR, no PoE) reads empty and the
//! page hides it.
//!
//! Management path guard: the ports and interfaces towards Cairn Net (the
//! next hop's ARP entry, where the bridge learnt its MAC, the bond's members)
//! cannot become a mirror target, lose their PoE, get a rate limit, an ACL
//! rule, a port isolation override or a queue.

use crate::direct::Cmd;
use crate::infra::{self, q};
use crate::Api;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

pub struct Table {
    pub key: &'static str,
    pub path: &'static str,
    /// rows can be added and removed (else: set only)
    pub add: bool,
    /// what keeps a row out of the rollback's "remove what was added"
    pub keep: &'static str,
    pub fields: &'static [&'static str],
}

pub const TABLES: &[Table] = &[
    Table { key: "switch", path: "/interface/ethernet/switch", add: false, keep: "", fields: &["mirror-target"] },
    Table { key: "port", path: "/interface/ethernet/switch/port", add: false, keep: "",
        fields: &["mirror-ingress", "mirror-egress", "ingress-rate", "egress-rate", "storm-rate", "limit-broadcasts", "limit-unknown-multicasts", "limit-unknown-unicasts"] },
    Table { key: "isolation", path: "/interface/ethernet/switch/port-isolation", add: false, keep: "", fields: &["forwarding-override"] },
    Table { key: "rule", path: "/interface/ethernet/switch/rule", add: true, keep: "",
        fields: &["switch", "ports", "comment", "disabled", "mac-protocol", "src-mac-address", "dst-mac-address", "vlan-id", "vlan-priority", "vlan-header",
            "protocol", "src-address", "dst-address", "src-port", "dst-port", "dscp", "new-dst-ports", "new-vlan-id", "new-vlan-priority",
            "redirect-to-cpu", "copy-to-cpu", "mirror", "rate"] },
    Table { key: "bridge", path: "/interface/bridge", add: false, keep: "",
        fields: &["igmp-snooping", "igmp-version", "mld-version", "multicast-querier", "multicast-router", "querier-interval", "query-interval",
            "query-response-interval", "last-member-interval", "membership-interval", "startup-query-count"] },
    Table { key: "bport", path: "/interface/bridge/port", add: false, keep: "", fields: &["multicast-router", "fast-leave"] },
    Table { key: "simple", path: "/queue/simple", add: true, keep: "!dynamic",
        fields: &["name", "target", "dst", "max-limit", "limit-at", "burst-limit", "burst-threshold", "burst-time", "priority", "queue", "parent", "packet-marks", "comment", "disabled"] },
    Table { key: "tree", path: "/queue/tree", add: true, keep: "!dynamic",
        fields: &["name", "parent", "packet-mark", "max-limit", "limit-at", "burst-limit", "burst-threshold", "burst-time", "priority", "queue", "comment", "disabled"] },
    Table { key: "type", path: "/queue/type", add: true, keep: "!default",
        fields: &["name", "kind", "pfifo-limit", "bfifo-limit", "sfq-perturb", "sfq-allot", "pcq-rate", "pcq-limit", "pcq-classifier", "pcq-total-limit",
            "fq-codel-limit", "fq-codel-target", "fq-codel-interval", "cake-bandwidth", "cake-diffserv", "cake-flowmode"] },
    Table { key: "poe", path: "/interface/ethernet/poe", add: false, keep: "", fields: &["poe-out", "poe-priority"] },
];

/// Read only, for the page and the guard.
const EXTRA: &[(&str, &str)] = &[("mdb", "/interface/bridge/mdb"), ("arp", "/ip/arp"), ("hosts", "/interface/bridge/host"), ("bonds", "/interface/bonding")];

fn table(key: &str) -> Result<&'static Table, String> {
    TABLES.iter().find(|t| t.key == key).ok_or_else(|| format!("{key}: unknown table"))
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn rows<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn yes(v: &str) -> bool {
    v == "true" || v == "yes"
}

fn all(api: &mut Api, path: &str, query: &[&str]) -> Result<Vec<Value>, String> {
    let cmd = format!("{path}/print");
    let words: Vec<&str> = std::iter::once(cmd.as_str()).chain(query.iter().copied()).collect();
    match api.call(&words) {
        Err(e) if e.starts_with("no such command") => Ok(vec![]),
        r => r,
    }
}

pub fn read(api: &mut Api) -> Result<Value, String> {
    let mut v = Map::new();
    for t in TABLES {
        v.insert(t.key.into(), json!(all(api, t.path, &[])?));
    }
    for (k, p) in EXTRA {
        v.insert((*k).into(), json!(all(api, p, &[])?));
    }
    v.insert("routes".into(), json!(all(api, "/ip/route", &["?active=true"])?));
    // the queue each interface runs (only-hardware-queue: switched traffic never meets a queue)
    v.insert("qiface".into(), json!(all(api, "/queue/interface", &[])?));
    for r in v.get_mut("bport").and_then(Value::as_array_mut).into_iter().flatten() {
        if let Some(o) = r.as_object_mut() {
            o.remove("debug-info");
        }
    }
    Ok(Value::Object(v))
}

/// The ports and interfaces Cairn Net's packets cross on this switch.
pub fn mgmt(live: &Value, app: Option<Ipv4Addr>) -> BTreeSet<String> {
    let mut hops: BTreeSet<String> = rows(live, "routes").iter().map(|r| s(r, "gateway").to_string()).collect();
    hops.extend(app.map(|a| a.to_string()));
    let mut out = BTreeSet::new();
    for a in rows(live, "arp").iter().filter(|a| hops.contains(s(a, "address"))) {
        out.insert(s(a, "interface").to_string());
        for h in rows(live, "hosts").iter().filter(|h| s(h, "mac-address").eq_ignore_ascii_case(s(a, "mac-address"))) {
            out.insert(s(h, "on-interface").to_string());
            out.insert(s(h, "bridge").to_string());
        }
    }
    let slaves: Vec<String> = rows(live, "bonds").iter().filter(|b| out.contains(s(b, "name")))
        .flat_map(|b| s(b, "slaves").split(',').filter(|x| !x.is_empty()).map(String::from)).collect();
    out.extend(slaves);
    out.retain(|x| !x.is_empty());
    out
}

/// The switch chip a port sits on ("" for a bond, a VLAN, a CHR).
fn chip_of<'a>(live: &'a Value, iface: &str) -> &'a str {
    rows(live, "port").iter().find(|p| s(p, "name") == iface).map_or("", |p| s(p, "switch"))
}

/// Bridge ports alone on their own switch chip (CRS312 ether9: the 100M
/// management port on a small second chip): they can never be switched in
/// hardware with the others, the CPU forwards for them. Normal, not an alert.
pub fn cpu_ports(live: &Value) -> Vec<String> {
    let bports = rows(live, "bport");
    bports.iter().filter(|p| !yes(s(p, "hw-offload")))
        .map(|p| s(p, "interface"))
        .filter(|i| { let c = chip_of(live, i); !c.is_empty() && bports.iter().filter(|q| chip_of(live, s(q, "interface")) == c).count() == 1 })
        .map(String::from).collect()
}

/// Bridge ports asked to be offloaded (hw=yes) that are not, while they
/// could be: their traffic goes through the CPU. Only on switches with a
/// switch chip; ports alone on their chip (cpu_ports) are left out.
pub fn offload_off(live: &Value) -> Vec<String> {
    if rows(live, "switch").is_empty() {
        return vec![];
    }
    let cpu = cpu_ports(live);
    rows(live, "bport").iter()
        .filter(|p| yes(s(p, "hw")) && !yes(s(p, "hw-offload")) && !yes(s(p, "inactive")) && !yes(s(p, "disabled")))
        .map(|p| s(p, "interface").to_string()).filter(|i| !cpu.contains(i)).collect()
}

/// One change asked by the page.
#[derive(Debug, Clone)]
pub struct Op {
    pub table: String,
    pub act: String,
    pub id: String,
    pub values: Vec<(String, String)>,
}

impl Op {
    pub fn list(v: &Value) -> Result<Vec<Op>, String> {
        let ops = v.as_array().ok_or("no changes")?;
        ops.iter().map(|o| {
            let values = o["values"].as_object().into_iter().flatten()
                .map(|(k, x)| Ok((k.clone(), x.as_str().ok_or_else(|| format!("{k}: text expected"))?.trim().to_string())))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Op { table: s(o, "table").into(), act: s(o, "act").into(), id: s(o, "id").into(), values })
        }).collect()
    }
}

fn row<'a>(live: &'a Value, t: &Table, id: &str) -> Result<&'a Value, String> {
    rows(live, t.key).iter().find(|r| s(r, ".id") == id).ok_or_else(|| format!("{}: row {id} not found (reload)", t.key))
}

/// What a row is called in a message.
fn label(r: &Value) -> String {
    [s(r, "name"), s(r, "interface"), s(r, "comment"), s(r, ".id")].into_iter().find(|x| !x.is_empty()).unwrap_or("?").to_string()
}

fn guard(t: &Table, r: Option<&Value>, values: &[(String, String)], mgmt: &BTreeSet<String>, app: Option<Ipv4Addr>) -> Result<(), String> {
    let get = |k: &str| values.iter().find(|(f, _)| f == k).map(|(_, v)| v.as_str());
    let name = r.map(|r| if t.key == "bport" { s(r, "interface") } else { s(r, "name") }).unwrap_or("");
    let hits = |v: &str| v.split(',').map(str::trim).find(|x| mgmt.contains(*x) || app.is_some_and(|a| x.split('/').next() == Some(&a.to_string()))).map(String::from);
    let path = |what: &str, x: &str| Err(format!("{x} carries Cairn Net's connection to this switch: {what} refused"));
    match t.key {
        "switch" => if let Some(x) = get("mirror-target").and_then(hits) { return path("mirror target", &x) },
        "port" if mgmt.contains(name) && ["ingress-rate", "egress-rate"].iter().any(|k| get(k).is_some_and(|v| !v.is_empty() && v != "0")) =>
            return path("rate limit", name),
        "isolation" if mgmt.contains(name) && get("forwarding-override").is_some() => return path("port isolation", name),
        "rule" => {
            let ports = get("ports").map(String::from).or_else(|| r.map(|r| s(r, "ports").to_string())).unwrap_or_default();
            if let Some(x) = hits(&ports) {
                return path("hardware rule", &x);
            }
        }
        "simple" => for k in ["target", "dst"] {
            if let Some(x) = get(k).and_then(hits) {
                return path("queue", &x);
            }
        },
        "tree" => if let Some(x) = get("parent").and_then(hits) { return path("queue", &x) },
        "poe" if mgmt.contains(name) && get("poe-out").is_some_and(|v| v == "off") => return path("PoE off", name),
        _ => {}
    }
    Ok(())
}

/// A value in the rollback script: RouterOS prints booleans true/false but
/// some of them (disabled of a queue) only take yes/no back.
fn lit(v: &str) -> String {
    match v {
        "true" => "yes".into(),
        "false" => "no".into(),
        _ => q(v),
    }
}

/// Commands and rollback script of the operations, after the guard.
pub fn plan(live: &Value, ops: &[Op], app: Option<Ipv4Addr>) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let mgmt = mgmt(live, app);
    let (mut cmds, mut undo_sets, mut touched) = (Vec::new(), Vec::new(), BTreeSet::new());
    let mut ids = BTreeSet::new();
    for o in ops {
        let t = table(&o.table)?;
        if !o.id.is_empty() && !ids.insert((t.key, o.id.clone())) {
            return Err(format!("{}: one change per row", t.key));
        }
        for (k, v) in &o.values {
            if !t.fields.contains(&k.as_str()) {
                return Err(format!("{}: {k} cannot be changed here", t.key));
            }
            if v.chars().any(char::is_control) {
                return Err(format!("{k}: invalid value"));
            }
        }
        let cur = if o.act == "add" { None } else { Some(row(live, t, &o.id)?) };
        if let Some(r) = cur {
            if yes(s(r, "dynamic")) || yes(s(r, "default")) {
                return Err(format!("{} {}: built-in row, read only", t.key, label(r)));
            }
        }
        guard(t, cur, &o.values, &mgmt, app)?;
        let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
        let cli = format!("/{}", t.path[1..].replace('/', " "));
        match (o.act.as_str(), cur) {
            ("set", Some(r)) => {
                let changed: Vec<&(String, String)> = o.values.iter().filter(|(k, v)| s(r, k) != v.as_str()).collect();
                let set: Vec<(String, String)> = changed.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| kv(k, v)).collect();
                if !set.is_empty() {
                    cmds.push((format!("{}/set", t.path), std::iter::once(kv(".id", &o.id)).chain(set).collect()));
                }
                for (k, _) in changed.iter().filter(|(_, v)| v.is_empty()) {
                    cmds.push((format!("{}/unset", t.path), vec![kv("numbers", &o.id), kv("value-name", k)]));
                }
                for (k, _) in &changed {
                    let old = r.get(k.as_str()).and_then(Value::as_str);
                    undo_sets.push(match old {
                        Some(old) => format!("{cli} set [find where .id={}] {k}={}", o.id, lit(old)),
                        None => format!("{cli} unset [find where .id={}] value-name={k}", o.id),
                    });
                }
            }
            ("add", None) if t.add => {
                let v: Vec<(String, String)> = o.values.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| kv(k, v)).collect();
                if v.is_empty() {
                    return Err(format!("{}: empty row", t.key));
                }
                cmds.push((format!("{}/add", t.path), v));
                touched.insert(t.key);
            }
            ("remove", Some(_)) if t.add => {
                cmds.push((format!("{}/remove", t.path), vec![kv(".id", &o.id)]));
                touched.insert(t.key);
            }
            _ => return Err(format!("{}: {} not possible", t.key, o.act)),
        }
    }
    // rollback: remove what was added, put back what was removed (in place), old values back
    let mut undo = Vec::new();
    for t in TABLES.iter().filter(|t| touched.contains(t.key)) {
        let cli = format!("/{}", t.path[1..].replace('/', " "));
        let old: Vec<&Value> = rows(live, t.key).iter().filter(|r| !yes(s(r, "dynamic")) && !yes(s(r, "default"))).collect();
        let cond: Vec<String> = std::iter::once(t.keep.to_string()).filter(|k| !k.is_empty())
            .chain(old.iter().map(|r| format!(".id!={}", s(r, ".id")))).collect();
        undo.push(if cond.is_empty() { format!("{cli} remove [find]") } else { format!("{cli} remove [find where {}]", cond.join(" and ")) });
        let removed: BTreeSet<&str> = ops.iter().filter(|o| o.table == t.key && o.act == "remove").map(|o| o.id.as_str()).collect();
        for (i, r) in old.iter().enumerate().filter(|(_, r)| removed.contains(s(r, ".id"))) {
            let props: Vec<String> = t.fields.iter().filter_map(|k| Some((k, r.get(*k)?.as_str()?))).filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{k}={}", lit(v))).collect();
            // in place: before the next old row that stays (order matters for rules and simple queues)
            let next = old[i + 1..].iter().find(|n| !removed.contains(s(n, ".id"))).map(|n| s(n, ".id"));
            let before = next.filter(|_| matches!(t.key, "rule" | "simple")).map_or(String::new(), |n| format!(" place-before=[find where .id={n}]"));
            undo.push(format!(":do {{ {cli} add {}{before} }} on-error={{ :log warning {} }}", props.join(" "), q("cairn-net: a row could not be put back")));
        }
    }
    undo.extend(undo_sets);
    infra::finish(&mut undo);
    Ok((cmds, undo))
}

/// "10M" and "10000000", "yes" and "true", "" and absent read the same.
fn norm(v: &str) -> String {
    v.split('/').map(|p| {
        let p = p.trim().to_ascii_lowercase();
        let (n, m) = match p.chars().last() {
            Some('k') => (&p[..p.len() - 1], 1e3),
            Some('m') => (&p[..p.len() - 1], 1e6),
            Some('g') => (&p[..p.len() - 1], 1e9),
            _ => (p.as_str(), 1.0),
        };
        match (n.parse::<f64>(), p.as_str()) {
            (Ok(x), _) => format!("{}", (x * m).round()),
            (_, "yes") => "true".into(),
            (_, "no") => "false".into(),
            _ => p.clone(),
        }
    }).collect::<Vec<_>>().join("/")
}

/// Whether the switch now runs what was asked.
pub fn matches(before: &Value, after: &Value, ops: &[Op]) -> bool {
    ops.iter().all(|o| {
        let Ok(t) = table(&o.table) else { return false };
        let find = |id: &str| rows(after, t.key).iter().find(|r| s(r, ".id") == id);
        match o.act.as_str() {
            "set" => find(&o.id).is_some_and(|r| o.values.iter().all(|(k, v)| norm(s(r, k)) == norm(v) || v.is_empty() && matches!(s(r, k), "" | "none"))),
            "remove" => find(&o.id).is_none(),
            "add" => {
                let old: BTreeSet<&str> = rows(before, t.key).iter().map(|r| s(r, ".id")).collect();
                rows(after, t.key).iter().filter(|r| !old.contains(s(r, ".id")))
                    .any(|r| o.values.iter().filter(|(k, _)| k.as_str() != "disabled").all(|(k, v)| v.is_empty() || norm(s(r, k)) == norm(v)))
            }
            _ => false,
        }
    })
}

/* ---------- page glue (main.rs: routes) ---------- */

pub fn view(name: &str) -> Value {
    let Some(sw) = crate::settings::get(name) else { return json!({"error": format!("{name}: unknown switch")}) };
    let mut v = crate::with_api(&sw, |api| {
        let mut v = read(api)?;
        let app = crate::path_of(&sw.cfg, api).ok().and_then(|p| p.app);
        v["mgmt"] = json!(mgmt(&v, app));
        v["offload_off"] = json!(offload_off(&v));
        v["cpu_ports"] = json!(cpu_ports(&v));
        v["rollback"] = json!(crate::print(api, "/system/scheduler", "name,next-run", &[&format!("?name={}", infra::ROLLBACK)])?.pop());
        Ok(v)
    }).unwrap_or_else(|e| json!({"error": e}));
    v["name"] = json!(sw.cfg.name);
    v["mode"] = json!(sw.cfg.mode);
    v["rollback_s"] = json!(infra::ROLLBACK_S);
    v["fields"] = json!(TABLES.iter().map(|t| (t.key.to_string(), json!({"fields": t.fields, "add": t.add}))).collect::<Map<String, Value>>());
    v
}

fn app_of(sw: &crate::settings::SwitchCfg, api: &Api) -> Option<Ipv4Addr> {
    crate::path_of(sw, api).ok().and_then(|p| p.app)
}

pub fn preview(v: &Value) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let ops = Op::list(&v["ops"])?;
    let (cmds, undo) = crate::with_api(&sw, |api| plan(&read(api)?, &ops, app_of(&sw.cfg, api)))?;
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S}))
}

/// Guarded apply; confirmation is /api/infra/confirm (same rollback scheduler).
pub fn apply(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let ops = Op::list(&v["ops"])?;
    let before = RefCell::new(Value::Null);
    let lost = RefCell::new(Vec::new());
    let mut out = crate::guarded_apply(&sw.cfg, user, "chip", |api| {
        let live = read(api)?;
        let p = plan(&live, &ops, app_of(&sw.cfg, api))?;
        *before.borrow_mut() = live;
        Ok(p)
    }, |api| {
        let after = read(api)?;
        let was: BTreeSet<String> = offload_off(&before.borrow()).into_iter().collect();
        *lost.borrow_mut() = offload_off(&after).into_iter().filter(|p| !was.contains(p)).collect();
        Ok(matches(&before.borrow(), &after, &ops))
    })?;
    if !lost.borrow().is_empty() {
        eprintln!("chip on {} by {user}: hardware offload lost on {}", sw.cfg.name, lost.borrow().join(", "));
    }
    out["offload_lost"] = json!(*lost.borrow());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed read of a real CRS326 (switch chip, LACP uplink, reached through a gateway on mgmt VLAN).
    fn crs() -> Value {
        serde_json::from_str(include_str!("chip_fixture.json")).unwrap()
    }
    fn op(table: &str, act: &str, id: &str, values: &[(&str, &str)]) -> Op {
        Op { table: table.into(), act: act.into(), id: id.into(), values: values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }
    fn id_of(live: &Value, table: &str, name: &str) -> String {
        rows(live, table).iter().find(|r| s(r, "name") == name || s(r, "interface") == name).map(|r| s(r, ".id").to_string()).unwrap()
    }

    #[test]
    fn management_path_of_a_real_switch() {
        let m = mgmt(&crs(), Some("192.168.1.69".parse().unwrap()));
        for x in ["mgmt99", "bridge", "bond1", "sfp-sfpplus1", "sfp-sfpplus2"] {
            assert!(m.contains(x), "{x} missing from {m:?}");
        }
        assert!(!m.contains("ether1"));
    }

    #[test]
    fn offload_on_a_real_switch_and_its_loss() {
        let mut l = crs();
        assert!(offload_off(&l).is_empty());
        l["bport"][0]["hw-offload"] = json!("false");
        assert_eq!(offload_off(&l), vec![s(&l["bport"][0], "interface").to_string()]);
        // CHR: no switch chip, nothing to lose
        l["switch"] = json!([]);
        assert!(offload_off(&l).is_empty());
    }

    #[test]
    fn a_port_alone_on_its_own_chip_is_a_cpu_port() {
        // CRS312: ether9 (management, 100M) is the only bridge port on switch2
        let mut l = crs();
        l["port"].as_array_mut().unwrap().push(json!({".id": "*20", "name": "ether9", "switch": "switch2", "running": "true"}));
        l["bport"].as_array_mut().unwrap().push(json!({".id": "*30", "interface": "ether9", "bridge": "bridge", "hw": "true", "hw-offload": "false", "inactive": "false"}));
        assert_eq!(cpu_ports(&l), vec!["ether9".to_string()]);
        assert!(offload_off(&l).is_empty());
        // a port of the main chip losing its offload still counts
        l["bport"][0]["hw-offload"] = json!("false");
        assert_eq!(offload_off(&l), vec![s(&l["bport"][0], "interface").to_string()]);
        assert_eq!(cpu_ports(&l), vec!["ether9".to_string()]);
    }

    #[test]
    fn port_settings_and_their_rollback() {
        let l = crs();
        let e2 = id_of(&l, "port", "ether2");
        let (cmds, undo) = plan(&l, &[op("port", "set", &e2, &[("ingress-rate", "10M"), ("storm-rate", "100"), ("limit-broadcasts", "")])], None).unwrap();
        assert_eq!(infra::show_all(&cmds), vec![
            format!("/interface/ethernet/switch/port/set .id={e2} ingress-rate=10M"),
            format!("/interface/ethernet/switch/port/unset numbers={e2} value-name=limit-broadcasts"),
        ]);
        assert_eq!(undo[0], format!("/interface ethernet switch port unset [find where .id={e2}] value-name=ingress-rate"));
        assert_eq!(undo[1], format!("/interface ethernet switch port set [find where .id={e2}] limit-broadcasts=yes"));
        assert!(undo.last().unwrap().contains("scheduler remove"));
    }

    #[test]
    fn the_management_path_is_guarded() {
        let l = crs();
        let sw = s(&l["switch"][0], ".id").to_string();
        let sfp = id_of(&l, "port", "sfp-sfpplus1");
        let iso = id_of(&l, "isolation", "sfp-sfpplus2");
        for o in [op("switch", "set", &sw, &[("mirror-target", "sfp-sfpplus1")]), op("port", "set", &sfp, &[("egress-rate", "1G")]),
            op("isolation", "set", &iso, &[("forwarding-override", "ether1")]), op("rule", "add", "", &[("switch", "switch1"), ("ports", "ether2,bond1"), ("new-dst-ports", "")]),
            op("simple", "add", "", &[("name", "q"), ("target", "192.168.1.69/32"), ("max-limit", "1M/1M")]), op("tree", "add", "", &[("name", "t"), ("parent", "mgmt99")])] {
            let e = plan(&l, &[o.clone()], Some("192.168.1.69".parse().unwrap())).unwrap_err();
            assert!(e.contains("carries Cairn Net's connection"), "{o:?}: {e}");
        }
        // storm control and mirroring a source are fine on the uplink
        assert!(plan(&l, &[op("port", "set", &sfp, &[("storm-rate", "50"), ("mirror-ingress", "yes")])], None).is_ok());
        assert!(plan(&l, &[op("switch", "set", &sw, &[("mirror-target", "ether2")])], None).is_ok());
    }

    #[test]
    fn unknown_fields_and_built_in_rows_are_refused() {
        let l = crs();
        let e2 = id_of(&l, "port", "ether2");
        assert!(plan(&l, &[op("port", "set", &e2, &[("l3-hw-offloading", "no")])], None).unwrap_err().contains("cannot be changed"));
        let def = id_of(&l, "type", "default");
        assert!(plan(&l, &[op("type", "remove", &def, &[])], None).unwrap_err().contains("built-in"));
        assert!(plan(&l, &[op("port", "add", "", &[("storm-rate", "1")])], None).is_err());
        assert!(plan(&l, &[op("port", "set", &e2, &[("storm-rate", "1\n/system reset")])], None).unwrap_err().contains("invalid"));
    }

    #[test]
    fn rules_added_and_removed_roll_back_in_place() {
        let mut l = crs();
        l["rule"] = json!([{".id": "*1", "switch": "switch1", "ports": "ether2", "dst-port": "23", "protocol": "tcp", "new-dst-ports": ""},
            {".id": "*2", "switch": "switch1", "ports": "ether2", "mac-protocol": "ipv6", "new-dst-ports": ""}]);
        let (cmds, undo) = plan(&l, &[op("rule", "remove", "*1", &[]), op("rule", "add", "", &[("switch", "switch1"), ("ports", "ether2"), ("vlan-id", "30")])], None).unwrap();
        assert_eq!(infra::show_all(&cmds), vec!["/interface/ethernet/switch/rule/remove .id=*1", "/interface/ethernet/switch/rule/add switch=switch1 ports=ether2 vlan-id=30"]);
        assert_eq!(undo[0], "/interface ethernet switch rule remove [find where .id!=*1 and .id!=*2]");
        assert!(undo[1].contains("add switch=\"switch1\" ports=\"ether2\" protocol=\"tcp\" dst-port=\"23\" place-before=[find where .id=*2]"), "{}", undo[1]);
    }

    #[test]
    fn queues_on_a_router_without_switch_chip() {
        let mut l = crs();
        l["switch"] = json!([]);
        l["simple"] = json!([{".id": "*A", "name": "guests", "target": "10.30.0.0/24", "max-limit": "20M/20M", "dynamic": "false"}]);
        let (cmds, undo) = plan(&l, &[op("simple", "set", "*A", &[("max-limit", "50M/50M")]), op("type", "add", "", &[("name", "fq"), ("kind", "fq-codel")])], None).unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(undo[0], "/queue type remove [find where !default]");
        assert!(undo.iter().any(|u| u == "/queue simple set [find where .id=*A] max-limit=\"20M/20M\""));
    }

    #[test]
    fn read_back_tolerates_routeros_spelling() {
        let before = json!({"simple": [], "port": [{".id": "*2", "ingress-rate": "", "limit-broadcasts": "true"}]});
        let after = json!({"simple": [{".id": "*9", "name": "q", "max-limit": "10000000/5000000", "disabled": "false"}],
            "port": [{".id": "*2", "ingress-rate": "10000000", "limit-broadcasts": "true"}]});
        assert!(matches(&before, &after, &[op("port", "set", "*2", &[("ingress-rate", "10M"), ("limit-broadcasts", "yes")]),
            op("simple", "add", "", &[("name", "q"), ("max-limit", "10M/5M")])]));
        assert!(!matches(&before, &after, &[op("port", "set", "*2", &[("ingress-rate", "20M")])]));
        assert!(!matches(&before, &after, &[op("simple", "add", "", &[("name", "other")])]));
    }
}
