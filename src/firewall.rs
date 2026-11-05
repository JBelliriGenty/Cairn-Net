//! Firewall page (lot 2): the IPv4 firewall of a switch as RouterOS runs it,
//! read with the read account: filter, NAT, mangle and raw rules in order
//! with their counters, address lists, interface lists, connection tracking.
//! Rules are read whole (no property list): every matcher RouterOS knows is
//! shown, whatever the version.
//!
//! Direct mode edits one table at a time: the page sends the static rows it
//! wants, in order; `plan` turns the difference into API commands (remove,
//! set/unset, move, add with place-before) and a rollback script that puts
//! the whole table back as it was (remove the static rows, add the old ones
//! in order). Before anything, the wanted firewall is run against the packets
//! of the management path (Cairn Net -> API port of the switch and back):
//! a rule that could drop, redirect or re-route them refuses the change.
//! In "auto", RouterOS starts connection tracking with the first rule: the
//! page then asks for an explicit acknowledgement (open connections may drop).
//! Matchers the simulation does not know count as "could match".
//!
//! Git mode: the repository holds the firewall as a JSON file, one key per
//! table ("filter", "nat", "mangle", "raw", "address-list"), each a list of
//! rows in RouterOS properties, one row per line. A table missing from the
//! file is not managed. The page edits the file's rows; the app applies them
//! the same guarded way, then commits once the change is kept.

use crate::direct::Cmd;
use crate::infra::{cidr, finish, q, Path};
use crate::Api;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

pub const TABLES: [&str; 4] = ["filter", "nat", "mangle", "raw"];
pub const LISTS: &str = "address-list";
/// Read-only properties: never sent back.
const META: [&str; 7] = [".id", ".nextid", "bytes", "packets", "invalid", "dynamic", "creation-time"];

/// Every row of `path`, all properties; a menu the device lacks reads as empty.
fn all(api: &mut Api, path: &str) -> Result<Vec<Value>, String> {
    match api.call(&[&format!("{path}/print")]) {
        Err(e) if e.starts_with("no such command") => Ok(vec![]),
        r => r,
    }
}

pub fn read(api: &mut Api) -> Result<Value, String> {
    let mut tables = serde_json::Map::new();
    for t in TABLES {
        tables.insert(t.into(), json!(all(api, &format!("/ip/firewall/{t}"))?));
    }
    let tracking = all(api, "/ip/firewall/connection/tracking")?.pop();
    Ok(json!({
        "tables": tables,
        // ponytail: whole lists; a dynamic list of 100k entries would make a big page
        "address_lists": all(api, "/ip/firewall/address-list")?,
        "iface_lists": all(api, "/interface/list")?,
        "iface_members": all(api, "/interface/list/member")?,
        "addresses": crate::print(api, "/ip/address", "address,interface", &[])?,
        "tracking": tracking.map(|t| json!({"enabled": t["enabled"], "active": t["active-ipv4"], "entries": t["total-ip4-entries"], "max": t["max-entries"]})),
    }))
}

pub type Row = BTreeMap<String, String>;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn yes(v: &str) -> bool {
    v == "true" || v == "yes"
}

/// The properties of a row as they are written back: no read-only ones, no
/// empty ones, `disabled` only when true.
fn props(v: &Value) -> Row {
    v.as_object().into_iter().flatten()
        .filter(|(k, _)| !META.contains(&k.as_str()))
        .filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string())))
        .filter(|(k, x)| !x.is_empty() && !(k == "disabled" && !yes(x)))
        .map(|(k, x)| if k == "disabled" { (k, "true".into()) } else { (k, x) })
        .collect()
}

fn rows_of<'a>(live: &'a Value, table: &str) -> Vec<&'a Value> {
    let rows = if table == LISTS { &live["address_lists"] } else { &live["tables"][table] };
    rows.as_array().into_iter().flatten().collect()
}

fn statics<'a>(live: &'a Value, table: &str) -> Vec<&'a Value> {
    rows_of(live, table).into_iter().filter(|r| !yes(s(r, "dynamic"))).collect()
}

fn path_of(table: &str) -> Result<String, String> {
    if TABLES.contains(&table) || table == LISTS {
        Ok(format!("/ip/firewall/{table}"))
    } else {
        Err(format!("{table}: unknown table"))
    }
}

/// What the page wants: the static rows of one table, in order; a row with
/// ".id" is an existing one (edited or not), without it is a new one.
pub struct Want {
    pub table: String,
    pub rows: Vec<(Option<String>, Row)>,
    pub ack_conntrack: bool,
}

impl Want {
    pub fn from_json(v: &Value) -> Result<Want, String> {
        let rows = v["rows"].as_array().ok_or("no rows")?;
        if rows.len() > 2000 {
            return Err("too many rows".into());
        }
        Ok(Want {
            table: s(v, "table").to_string(),
            rows: rows.iter().map(|r| (r[".id"].as_str().map(String::from), props(r))).collect(),
            ack_conntrack: v["ack_conntrack"].as_bool().unwrap_or(false),
        })
    }
}

fn check_row(table: &str, r: &Row) -> Result<(), String> {
    for (k, v) in r {
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') || k == "place-before" {
            return Err(format!("{k}: invalid property name"));
        }
        if v.chars().any(char::is_control) {
            return Err(format!("{k}: line breaks and control characters are not allowed"));
        }
    }
    let need: &[&str] = if table == LISTS { &["list", "address"] } else { &["chain", "action"] };
    match need.iter().find(|k| !r.contains_key(**k)) {
        Some(k) => Err(format!("every row needs {k}")),
        None => Ok(()),
    }
}

/// One script line that adds a row back (a failing line does not stop the rest).
fn add_line(base: &str, r: &Row) -> String {
    let kv: Vec<String> = r.iter().map(|(k, v)| if k == "disabled" { "disabled=yes".into() } else { format!("{k}={}", q(v)) }).collect();
    format!(":do {{ {} add {} }} on-error={{ :log warning {} }}", cli(base), kv.join(" "), q("cairn-net: a firewall row could not be put back"))
}

/// "/ip/firewall/filter" as a script path: "/ip firewall filter".
fn cli(base: &str) -> String {
    format!("/{}", base[1..].replace('/', " "))
}

/// The commands, the rollback script, and whether connection tracking
/// would start (it cuts the connections already open once).
pub fn plan(live: &Value, w: &Want, path: &Path, port: u16) -> Result<(Vec<Cmd>, Vec<String>, bool), String> {
    let base = path_of(&w.table)?;
    let ordered = w.table != LISTS;
    let old = statics(live, &w.table);
    let old_ids: Vec<&str> = old.iter().map(|r| s(r, ".id")).collect();
    let mut seen = std::collections::BTreeSet::new();
    for (id, r) in &w.rows {
        check_row(&w.table, r)?;
        if let Some(id) = id {
            if !old_ids.contains(&id.as_str()) || !seen.insert(id.clone()) {
                return Err(format!("{id}: this row is gone from the switch: reload"));
            }
        }
    }

    // the wanted firewall must leave the management path alone
    let mut after = live.clone();
    let dynamic: Vec<Value> = rows_of(live, &w.table).into_iter().filter(|r| yes(s(r, "dynamic"))).cloned().collect();
    let wanted: Vec<Value> = dynamic.into_iter().chain(w.rows.iter().map(|(_, r)| json!(r))).collect();
    if w.table == LISTS { after["address_lists"] = json!(wanted) } else { after["tables"][&w.table] = json!(wanted) }
    guard(&after, path, port)?;
    let conntrack = !yes(s(&live["tracking"], "active")) && s(&live["tracking"], "enabled") == "auto" && uses_conntrack(&after);

    let kv = |p: &[(&str, &str)]| p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    let wire = |k: &str, v: &str| if k == "disabled" { (k.to_string(), if yes(v) { "yes" } else { "no" }.to_string()) } else { (k.to_string(), v.to_string()) };
    let mut cmds: Vec<Cmd> = vec![];
    // removed rows
    let kept: Vec<&str> = w.rows.iter().filter_map(|(id, _)| id.as_deref()).collect();
    for id in old_ids.iter().copied().filter(|i| !kept.contains(i)) {
        cmds.push((format!("{base}/remove"), kv(&[(".id", id)])));
    }
    // edited rows
    for (id, r) in w.rows.iter().filter_map(|(id, r)| Some((id.as_ref()?, r))) {
        let cur = props(old[old_ids.iter().position(|i| *i == id.as_str()).unwrap()]);
        let mut set: Vec<(String, String)> = vec![];
        for (k, v) in r {
            if cur.get(k) != Some(v) {
                set.push(wire(k, v));
            }
        }
        for k in cur.keys().filter(|k| !r.contains_key(*k)) {
            match k.as_str() {
                "disabled" => set.push(("disabled".into(), "no".into())),
                "comment" => set.push(("comment".into(), String::new())),
                _ => cmds.push((format!("{base}/unset"), kv(&[("numbers", id.as_str()), ("value-name", k.as_str())]))),
            }
        }
        if !set.is_empty() {
            set.insert(0, (".id".into(), id.clone()));
            cmds.push((format!("{base}/set"), set));
        }
    }
    // order of the kept rows, then the new ones before their next kept row
    if ordered {
        let mut cur: Vec<&str> = old_ids.iter().copied().filter(|i| kept.contains(i)).collect();
        for i in 0..kept.len() {
            if cur[i] != kept[i] {
                cmds.push((format!("{base}/move"), kv(&[("numbers", kept[i]), ("destination", cur[i])])));
                let j = cur.iter().position(|x| *x == kept[i]).unwrap();
                let m = cur.remove(j);
                cur.insert(i, m);
            }
        }
    }
    for (i, (_, r)) in w.rows.iter().enumerate().filter(|(_, (id, _))| id.is_none()) {
        let mut add: Vec<(String, String)> = r.iter().map(|(k, v)| wire(k, v)).collect();
        if let Some(next) = w.rows[i..].iter().find_map(|(id, _)| id.as_deref()).filter(|_| ordered) {
            add.push(("place-before".into(), next.into()));
        }
        cmds.push((format!("{base}/add"), add));
    }

    let mut undo = vec![];
    if !cmds.is_empty() {
        undo.push(format!("{} remove [find where !dynamic]", cli(&base)));
        undo.extend(old.iter().map(|r| add_line(&base, &props(r))));
        finish(&mut undo);
    }
    Ok((cmds, undo, conntrack))
}

/// Whether the switch now runs what was asked: the same static rows, in
/// order (lists: in any order), each with at least the asked properties.
pub fn matches(live: &Value, w: &Want) -> bool {
    let mut got: Vec<Row> = statics(live, &w.table).into_iter().map(props).collect();
    let mut want: Vec<Row> = w.rows.iter().map(|(_, r)| r.clone()).collect();
    if w.table == LISTS {
        let key = |r: &Row| (r.get("list").cloned(), r.get("address").cloned());
        got.sort_by_key(key);
        want.sort_by_key(key);
    }
    got.len() == want.len() && got.iter().zip(&want).all(|(g, w)| w.iter().all(|(k, v)| g.get(k) == Some(v)) && g.contains_key("disabled") == w.contains_key("disabled"))
}

/* ---------- the management path, packet by packet ---------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tri {
    Yes,
    No,
    Maybe,
}
use Tri::*;

impl Tri {
    fn not(self) -> Tri {
        match self { Yes => No, No => Yes, Maybe => Maybe }
    }
    fn of(b: bool) -> Tri {
        if b { Yes } else { No }
    }
}

/// The interface a packet crosses: none (input has no out-interface…),
/// known, or unknown (Cairn Net is on no network of the switch: routed).
#[derive(Clone, Copy)]
enum Ifc<'a> {
    None,
    Known(&'a str),
    Unknown,
}

struct Pkt<'a> {
    what: &'static str,
    src: Ipv4Addr,
    dst: Ipv4Addr,
    sport: Option<u16>,
    dport: Option<u16>,
    in_if: Ifc<'a>,
    out_if: Ifc<'a>,
    state: &'static str,
}

struct Ctx<'a> {
    lists: Vec<&'a Value>,
    members: Vec<&'a Value>,
    iface_lists: Vec<&'a Value>,
}

fn addr(v: &str, ip: Ipv4Addr) -> Tri {
    if let Some((a, b)) = v.split_once('-') {
        return match (a.parse::<Ipv4Addr>(), b.parse::<Ipv4Addr>()) {
            (Ok(a), Ok(b)) => Tri::of(a <= ip && ip <= b),
            _ => Maybe,
        };
    }
    if let Ok(a) = v.parse::<Ipv4Addr>() {
        return Tri::of(a == ip);
    }
    match cidr(v) {
        Some((_, n, l)) => Tri::of(u32::from(ip) & u32::MAX.checked_shl(32 - l).unwrap_or(0) == n),
        None => Maybe, // a DNS name in an address list, IPv6…
    }
}

fn ports(v: &str, p: Option<u16>) -> Tri {
    let Some(p) = p else { return Maybe };
    let mut r = No;
    for part in v.split(',') {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        match (a.trim().parse::<u16>(), b.trim().parse::<u16>()) {
            (Ok(a), Ok(b)) if a <= p && p <= b => return Yes,
            (Ok(_), Ok(_)) => {}
            _ => r = Maybe, // a service name
        }
    }
    r
}

fn in_list(ctx: &Ctx, list: &str, ip: Ipv4Addr) -> Tri {
    let mut r = No;
    for e in ctx.lists.iter().filter(|e| s(e, "list") == list && !yes(s(e, "disabled"))) {
        match addr(s(e, "address"), ip) {
            Yes => return Yes,
            Maybe => r = Maybe,
            No => {}
        }
    }
    r
}

fn iface_list(ctx: &Ctx, list: &str, iface: Ifc) -> Tri {
    let i = match iface {
        Ifc::None => return No,
        Ifc::Unknown => return if list == "none" { No } else { Maybe },
        Ifc::Known(i) => i,
    };
    match list {
        "all" => return Yes,
        "none" => return No,
        _ => {}
    }
    let def = ctx.iface_lists.iter().find(|l| s(l, "name") == list);
    if def.is_none_or(|d| yes(s(d, "builtin")) || !s(d, "include").is_empty() || !s(d, "exclude").is_empty()) {
        return Maybe; // static, dynamic, nested lists…
    }
    Tri::of(ctx.members.iter().any(|m| s(m, "list") == list && s(m, "interface") == i && !yes(s(m, "disabled"))))
}

/// Not matchers: bookkeeping, or the parameters of the action.
fn is_param(k: &str) -> bool {
    matches!(k, "chain" | "action" | "comment" | "disabled" | "log" | "log-prefix" | "jump-target" | "to-addresses" | "to-ports" | "passthrough"
        | "address-list" | "address-list-timeout" | "reject-with" | "route-dst") || k.starts_with("new-") || META.contains(&k)
}

fn matcher(ctx: &Ctx, k: &str, v: &str, p: &Pkt) -> Tri {
    let (neg, v) = v.strip_prefix('!').map_or((false, v), |x| (true, x));
    let r = match k {
        "src-address" => addr(v, p.src),
        "dst-address" => addr(v, p.dst),
        "protocol" => Tri::of(v == "tcp" || v == "6"),
        "src-port" => ports(v, p.sport),
        "dst-port" => ports(v, p.dport),
        "port" => match (ports(v, p.sport), ports(v, p.dport)) {
            (Yes, _) | (_, Yes) => Yes,
            (No, No) => No,
            _ => Maybe,
        },
        "in-interface" | "out-interface" => match if k == "in-interface" { p.in_if } else { p.out_if } {
            Ifc::None => No,
            Ifc::Unknown => Maybe,
            Ifc::Known(_) if v.starts_with("all-") || v.contains('*') => Maybe,
            Ifc::Known(i) => Tri::of(i == v),
        },
        "in-interface-list" => iface_list(ctx, v, p.in_if),
        "out-interface-list" => iface_list(ctx, v, p.out_if),
        "src-address-list" => in_list(ctx, v, p.src),
        "dst-address-list" => in_list(ctx, v, p.dst),
        "connection-state" => Tri::of(v.split(',').any(|x| x == p.state)),
        _ => Maybe,
    };
    if neg { r.not() } else { r }
}

fn rule_matches(ctx: &Ctx, r: &Value, p: &Pkt) -> Tri {
    let mut out = Yes;
    for (k, v) in r.as_object().into_iter().flatten() {
        let v = v.as_str().unwrap_or("");
        if is_param(k) || v.is_empty() {
            continue;
        }
        match matcher(ctx, k, v, p) {
            No => return No,
            Maybe => out = Maybe,
            Yes => {}
        }
    }
    out
}

enum Verdict {
    Accept,
    Return,
    End,
    Block(String),
}

/// What an action does to the management packets in a table.
fn harmful(table: &str, action: &str) -> bool {
    match table {
        "filter" => !matches!(action, "accept" | "jump" | "return" | "passthrough" | "log" | "add-src-to-address-list" | "add-dst-to-address-list" | "fasttrack-connection"),
        "raw" => !matches!(action, "accept" | "jump" | "return" | "passthrough" | "log" | "notrack" | "add-src-to-address-list" | "add-dst-to-address-list"),
        "nat" => matches!(action, "dst-nat" | "redirect" | "netmap" | "same" | "endpoint-independent-nat"),
        _ => matches!(action, "mark-routing" | "route"), // mangle
    }
}

fn walk(ctx: &Ctx, table: &str, rules: &[(String, &Value)], chain: &str, p: &Pkt, depth: u32) -> Verdict {
    if depth > 16 {
        return Block(format!("{table}: jumps nested too deep"));
    }
    for (label, r) in rules.iter().filter(|(_, r)| s(r, "chain") == chain && !yes(s(r, "disabled"))) {
        let m = rule_matches(ctx, r, p);
        if m == No {
            continue;
        }
        let action = s(r, "action");
        match action {
            "accept" if m == Yes => return Verdict::Accept,
            "return" if m == Yes => return Verdict::Return,
            "jump" => match walk(ctx, table, rules, s(r, "jump-target"), p, depth + 1) {
                Verdict::Block(e) => return Verdict::Block(e),
                Verdict::Accept if m == Yes => return Verdict::Accept,
                _ => {}
            },
            a if harmful(table, a) => return Verdict::Block(format!("{table} {label}")),
            _ => {}
        }
    }
    if depth == 0 { Verdict::End } else { Verdict::Return }
}
use Verdict::Block;

/// The interface Cairn Net's packets enter and leave by: the one holding a
/// network that contains the app's address (not always the one holding the
/// switch's address: a routed management path enters on another one).
pub fn app_iface(fw: &Value, app: Ipv4Addr) -> Option<&str> {
    fw["addresses"].as_array()?.iter()
        .find(|a| cidr(s(a, "address")).is_some_and(|(_, n, l)| u32::from(app) & u32::MAX.checked_shl(32 - l).unwrap_or(0) == n))
        .map(|a| s(a, "interface"))
}

/// The packets of the API connection, through every chain they cross.
pub fn guard(fw: &Value, path: &Path, port: u16) -> Result<(), String> {
    let app = path.app.ok_or("Cairn Net's own address is unknown: no firewall change")?;
    let mgmt = app_iface(fw, app).map_or(Ifc::Unknown, Ifc::Known);
    let ctx = Ctx {
        lists: fw["address_lists"].as_array().into_iter().flatten().collect(),
        members: fw["iface_members"].as_array().into_iter().flatten().collect(),
        iface_lists: fw["iface_lists"].as_array().into_iter().flatten().collect(),
    };
    let inbound = |state, what| Pkt { what, src: app, dst: path.switch, sport: None, dport: Some(port), in_if: mgmt, out_if: Ifc::None, state };
    let outbound = Pkt { what: "replies of the switch", src: path.switch, dst: app, sport: Some(port), dport: None, in_if: Ifc::None, out_if: mgmt, state: "established" };
    let new = inbound("new", "a new connection of Cairn Net");
    let est = inbound("established", "the open connection of Cairn Net");
    let checks: [(&str, &str, &Pkt); 9] = [
        ("raw", "prerouting", &new), ("raw", "prerouting", &est), ("raw", "output", &outbound),
        ("mangle", "prerouting", &new), ("mangle", "input", &new), ("mangle", "output", &outbound),
        ("nat", "dstnat", &new),
        ("filter", "input", &new), ("filter", "output", &outbound),
    ];
    for (table, chain, p) in checks.into_iter().chain([("filter", "input", &est)]) {
        let mut n = 0;
        let rules: Vec<(String, &Value)> = fw["tables"][table].as_array().into_iter().flatten().map(|r| {
            let label = if yes(s(r, "dynamic")) { "D".to_string() } else { n += 1; format!("#{n}") };
            let c = s(r, "comment");
            (if c.is_empty() { label } else { format!("{label} ({c})") }, r)
        }).collect();
        if let Block(rule) = walk(&ctx, table, &rules, chain, p, 0) {
            return Err(format!("{rule} could cut Cairn Net off the switch ({}, {chain}): refused", p.what));
        }
    }
    Ok(())
}

/// Whether RouterOS will track connections: in "auto", as soon as there is
/// one firewall rule (seen on RouterOS 7.25: a single filter drop starts it).
fn uses_conntrack(fw: &Value) -> bool {
    TABLES.iter().any(|t| fw["tables"][*t].as_array().into_iter().flatten().any(|r| !yes(s(r, "disabled"))))
}

/* ---------- Git mode: the firewall file ---------- */

pub type File = BTreeMap<String, Vec<Row>>;

pub fn parse_file(text: &str) -> Result<File, String> {
    if text.trim().is_empty() {
        return Ok(File::new());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let mut out = File::new();
    for (table, rows) in v.as_object().ok_or("the file must be a JSON object of tables")? {
        path_of(table)?;
        let rows = rows.as_array().ok_or_else(|| format!("{table}: a list of rows is expected"))?;
        let rows: Vec<Row> = rows.iter().map(|r| {
            let o = r.as_object().ok_or_else(|| format!("{table}: every row is an object"))?;
            let row: Row = o.iter().map(|(k, x)| x.as_str().map(|x| (k.clone(), x.to_string())).ok_or_else(|| format!("{table}: {k}: text expected")))
                .collect::<Result<_, String>>()?;
            let row = props(&json!(row));
            check_row(table, &row).map(|_| row)
        }).collect::<Result<_, String>>()?;
        out.insert(table.clone(), rows);
    }
    Ok(out)
}

/// One table per key, one row per line: readable diffs.
pub fn file_text(f: &File) -> String {
    let tables: Vec<String> = f.iter().map(|(t, rows)| {
        let rows: Vec<String> = rows.iter().map(|r| format!("    {}", json!(r))).collect();
        if rows.is_empty() { format!("  {}: []", json!(t)) } else { format!("  {}: [\n{}\n  ]", json!(t), rows.join(",\n")) }
    }).collect();
    format!("{{\n{}\n}}\n", tables.join(",\n"))
}

/// The static rows of every table as the switch runs them (import).
pub fn to_file(live: &Value) -> File {
    TABLES.iter().copied().chain([LISTS]).map(|t| (t.to_string(), statics(live, t).into_iter().map(props).collect())).collect()
}

/// The file's rows of one table as a Want: each row takes the id of a static
/// row of the switch with exactly the same properties (in order, once), the
/// others are new; switch rows left without a file row go.
pub fn want_of(live: &Value, table: &str, rows: &[Row], ack_conntrack: bool) -> Want {
    let old: Vec<(&str, Row)> = statics(live, table).into_iter().map(|r| (s(r, ".id"), props(r))).collect();
    let mut used = vec![false; old.len()];
    let rows = rows.iter().map(|r| {
        let id = old.iter().enumerate().position(|(i, (_, o))| !used[i] && o == r);
        id.map(|i| used[i] = true);
        (id.map(|i| old[i].0.to_string()), r.clone())
    }).collect();
    Want { table: table.to_string(), rows, ack_conntrack }
}

/// The tables of the file the switch does not run as written.
pub fn drift(live: &Value, f: &File) -> Vec<String> {
    f.iter().filter(|(t, rows)| !matches(live, &want_of(live, t, rows, false))).map(|(t, _)| t.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direct::show;

    // a core switch with an isolated IoT VLAN, reached on mgmt99
    fn live() -> Value {
        json!({
            "tables": {
                "filter": [
                    {".id": "*1", "chain": "input", "action": "accept", "connection-state": "established,related", "bytes": "1", "packets": "1", "invalid": "false", "dynamic": "false"},
                    {".id": "*2", "chain": "input", "action": "accept", "protocol": "udp", "in-interface-list": "IOT", "dst-port": "67", "invalid": "false", "dynamic": "false", "comment": "IoT: DHCP"},
                    {".id": "*5", "chain": "input", "action": "drop", "in-interface-list": "IOT", "invalid": "false", "dynamic": "false", "comment": "IoT: nothing else"},
                    {".id": "*6", "chain": "forward", "action": "drop", "in-interface-list": "IOT", "dst-address": "!192.168.20.0/24", "disabled": "true", "invalid": "false", "dynamic": "false"},
                ],
                "nat": [{".id": "*1", "chain": "srcnat", "action": "masquerade", "src-address": "192.168.20.0/24", "out-interface": "bridge", "dynamic": "false"}],
                "mangle": [], "raw": [],
            },
            "address_lists": [{".id": "*1", "list": "admin", "address": "192.168.99.0/24", "creation-time": "x", "dynamic": "false", "disabled": "false"}],
            "iface_lists": [{"name": "all", "builtin": "true"}, {"name": "IOT", "builtin": "false", "include": "", "exclude": ""}],
            "iface_members": [{"list": "IOT", "interface": "iot20", "disabled": "false"}],
            "addresses": [{"address": "192.168.1.11/24", "interface": "bridge"}, {"address": "192.168.99.11/24", "interface": "mgmt99"}, {"address": "192.168.20.1/24", "interface": "iot20"}],
            "tracking": {"enabled": "auto", "active": "true"},
        })
    }

    fn path() -> Path {
        Path { switch: "192.168.99.11".parse().unwrap(), app: Some("192.168.99.69".parse().unwrap()) }
    }

    fn rows(l: &Value, t: &str) -> Vec<Value> {
        statics(l, t).into_iter().cloned().collect()
    }

    fn want(table: &str, rows: Vec<Value>) -> Want {
        Want::from_json(&json!({"table": table, "rows": rows})).unwrap()
    }

    fn plan_of(w: &Want) -> Result<(Vec<String>, Vec<String>, bool), String> {
        plan(&live(), w, &path(), 8729).map(|(c, u, k)| (c.iter().map(show).collect(), u, k))
    }

    #[test]
    fn the_file_round_trips_and_drives_the_plan() {
        let f = to_file(&live());
        let text = file_text(&f);
        assert!(text.contains("\n    {\"action\":\"accept\",\"chain\":\"input\",\"comment\":\"IoT: DHCP\""));
        assert!(!text.contains("bytes") && !text.contains(".id"));
        assert_eq!(parse_file(&text).unwrap(), f);
        assert!(drift(&live(), &f).is_empty());
        // the file in line with the switch: nothing to do
        let (c, _, _) = plan_of(&want_of(&live(), "filter", &f["filter"], false)).unwrap();
        assert!(c.is_empty());
        // one row edited in the file: that row goes, its new version comes in its place
        let mut f2 = f.clone();
        f2.get_mut("filter").unwrap()[1].insert("dst-port".into(), "67-68".into());
        assert_eq!(drift(&live(), &f2), vec!["filter"]);
        let (c, _, _) = plan_of(&want_of(&live(), "filter", &f2["filter"], false)).unwrap();
        assert_eq!(c, vec!["/ip/firewall/filter/remove .id=*2", "/ip/firewall/filter/add action=accept chain=input comment=\"IoT: DHCP\" dst-port=67-68 in-interface-list=IOT protocol=udp place-before=*5"]);
        // a table missing from the file is not managed; bad files are refused
        f2.remove("nat");
        assert_eq!(drift(&live(), &f2), vec!["filter"]);
        assert!(parse_file("").unwrap().is_empty());
        assert!(parse_file("[]").is_err());
        assert!(parse_file("{\"routes\": []}").unwrap_err().contains("unknown table"));
        assert!(parse_file("{\"filter\": [{\"chain\": \"input\"}]}").unwrap_err().contains("action"));
        assert!(parse_file("{\"filter\": [{\"chain\": \"input\", \"action\": 1}]}").unwrap_err().contains("text"));
    }

    #[test]
    fn same_rows_mean_nothing_to_do() {
        let w = want("filter", rows(&live(), "filter"));
        let (c, u, k) = plan_of(&w).unwrap();
        assert!(c.is_empty() && u.is_empty() && !k);
        assert!(matches(&live(), &w));
    }

    #[test]
    fn edits_moves_adds_and_removes() {
        let mut r = rows(&live(), "filter");
        r[1]["dst-port"] = json!("67-68"); // edit
        r[1].as_object_mut().unwrap().remove("comment"); // cleared comment
        r[3].as_object_mut().unwrap().remove("disabled"); // enabled
        r[3].as_object_mut().unwrap().remove("dst-address"); // unset matcher
        r.remove(2); // *5 removed
        r.swap(0, 1); // *2 before *1
        r.insert(1, json!({"chain": "input", "action": "accept", "protocol": "icmp", "comment": "ping \"gw\""})); // new, before *1
        let (c, u, _) = plan_of(&want("filter", r)).unwrap();
        assert_eq!(c, vec![
            "/ip/firewall/filter/remove .id=*5",
            "/ip/firewall/filter/set .id=*2 dst-port=67-68 comment=\"\"",
            "/ip/firewall/filter/unset numbers=*6 value-name=dst-address",
            "/ip/firewall/filter/set .id=*6 disabled=no",
            "/ip/firewall/filter/move numbers=*2 destination=*1",
            "/ip/firewall/filter/add action=accept chain=input comment=\"ping \"gw\"\" protocol=icmp place-before=*1",
        ]);
        // the whole table goes back as it was, in order
        assert_eq!(u[0], "/ip firewall filter remove [find where !dynamic]");
        assert_eq!(u[1], ":do { /ip firewall filter add action=\"accept\" chain=\"input\" connection-state=\"established,related\" } on-error={ :log warning \"cairn-net: a firewall row could not be put back\" }");
        assert!(u[4].contains("disabled=yes") && u[4].contains("dst-address=\"!192.168.20.0/24\""));
        assert_eq!(u.len(), 4 + 1 + 2);
        assert!(u[6].starts_with("/system scheduler remove"));
    }

    #[test]
    fn address_lists_have_no_order() {
        let mut r = rows(&live(), LISTS);
        r.push(json!({"list": "admin", "address": "10.0.0.1", "comment": "x"}));
        let (c, u, _) = plan_of(&want(LISTS, r)).unwrap();
        assert_eq!(c, vec!["/ip/firewall/address-list/add address=10.0.0.1 comment=x list=admin"]);
        assert_eq!(u[0], "/ip firewall address-list remove [find where !dynamic]");
        assert!(!u[1].contains("creation-time"));
    }

    #[test]
    fn bad_rows_are_refused() {
        let e = |r: Value| plan_of(&want("filter", vec![r])).unwrap_err();
        assert!(e(json!({"chain": "input"})).contains("action"));
        assert!(e(json!({"chain": "input", "action": "accept", "Bad Key": "x"})).contains("invalid"));
        assert!(e(json!({"chain": "input", "action": "accept", "comment": "a\nb"})).contains("line breaks"));
        assert!(e(json!({".id": "*99", "chain": "input", "action": "accept"})).contains("gone"));
        assert!(Want::from_json(&json!({"table": "filter"})).is_err());
        assert!(plan_of(&want("routes", vec![])).unwrap_err().contains("unknown table"));
    }

    #[test]
    fn the_management_path_is_never_cut() {
        let refused = |table: &str, extra: Vec<Value>| {
            let mut r = rows(&live(), table);
            for (i, x) in extra.into_iter().enumerate() { r.insert(i, x); }
            plan_of(&want(table, r)).map(|_| ()).unwrap_err()
        };
        let ok = |table: &str, extra: Vec<Value>| {
            let mut r = rows(&live(), table);
            for (i, x) in extra.into_iter().enumerate() { r.insert(i, x); }
            plan_of(&want(table, r)).map(|_| ())
        };
        // everything in, at the top
        assert!(refused("filter", vec![json!({"chain": "input", "action": "drop"})]).contains("#1"));
        assert!(refused("filter", vec![json!({"chain": "input", "action": "reject", "in-interface": "mgmt99", "comment": "x"})]).contains("#1 (x)"));
        // the API port, a range holding it, the app's address, the switch's replies
        assert!(refused("filter", vec![json!({"chain": "input", "action": "drop", "protocol": "tcp", "dst-port": "8000-9000"})]).contains("new connection"));
        assert!(refused("filter", vec![json!({"chain": "input", "action": "drop", "src-address-list": "admin"})]).contains("input"));
        assert!(refused("filter", vec![json!({"chain": "output", "action": "drop", "dst-address": "192.168.99.69"})]).contains("replies"));
        // a matcher the simulation does not know: could match
        assert!(refused("filter", vec![json!({"chain": "input", "action": "drop", "tcp-flags": "syn"})]).contains("#1"));
        // through a jump
        assert!(refused("filter", vec![json!({"chain": "input", "action": "jump", "jump-target": "mine"}), json!({"chain": "mine", "action": "drop"})]).contains("#2"));
        // raw, dstnat, routing marks
        assert!(refused("raw", vec![json!({"chain": "prerouting", "action": "drop", "dst-port": "8729", "protocol": "tcp"})]).contains("raw"));
        assert!(refused("nat", vec![json!({"chain": "dstnat", "action": "dst-nat", "dst-port": "8729", "protocol": "tcp", "to-addresses": "10.0.0.1"})]).contains("nat"));
        assert!(refused("mangle", vec![json!({"chain": "prerouting", "action": "mark-routing", "new-routing-mark": "x"})]).contains("mangle"));
        // an address list edit that pulls the app into a drop rule
        let mut l = live();
        l["tables"]["filter"].as_array_mut().unwrap().insert(0, json!({".id": "*9", "chain": "input", "action": "drop", "src-address-list": "bad", "dynamic": "false"}));
        let w = want(LISTS, vec![json!({"list": "bad", "address": "192.168.99.0/24"})]);
        assert!(plan(&l, &w, &path(), 8729).unwrap_err().contains("#1"));
        // fine: other interfaces, other ports, accepted first, negations, disabled, other chains
        assert!(ok("filter", vec![json!({"chain": "input", "action": "drop", "in-interface": "test-fw"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "input", "action": "drop", "protocol": "tcp", "dst-port": "22,8291"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "input", "action": "accept", "src-address": "192.168.99.69", "protocol": "tcp", "dst-port": "8729"}), json!({"chain": "input", "action": "drop"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "input", "action": "drop", "src-address": "!192.168.99.0/24"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "input", "action": "drop", "disabled": "true"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "forward", "action": "drop"})]).is_ok());
        assert!(ok("filter", vec![json!({"chain": "input", "action": "drop", "connection-state": "invalid"})]).is_ok());
        // routed: the app (another network) enters on bridge, not on mgmt99
        let routed = |app: &str, extra: Value| {
            let mut r = rows(&live(), "filter");
            r.insert(0, extra);
            plan(&live(), &want("filter", r), &Path { switch: path().switch, app: Some(app.parse().unwrap()) }, 8729).map(|_| ())
        };
        assert!(routed("192.168.1.69", json!({"chain": "input", "action": "drop", "in-interface": "mgmt99"})).is_ok());
        assert!(routed("192.168.1.69", json!({"chain": "input", "action": "drop", "in-interface": "bridge"})).unwrap_err().contains("#1"));
        assert!(routed("192.168.1.69", json!({"chain": "output", "action": "drop", "out-interface": "bridge"})).unwrap_err().contains("replies"));
        // on no network of the switch: any interface could be the way in
        assert!(routed("10.1.2.3", json!({"chain": "input", "action": "drop", "in-interface": "iot20"})).unwrap_err().contains("#1"));
        assert!(routed("10.1.2.3", json!({"chain": "input", "action": "drop", "in-interface-list": "IOT"})).unwrap_err().contains("#1"));
        // without Cairn Net's address, no change at all
        let p = Path { switch: path().switch, app: None };
        assert!(plan(&live(), &want("filter", vec![]), &p, 8729).unwrap_err().contains("unknown"));
    }

    #[test]
    fn connection_tracking_starting_is_reported() {
        let mut l = live();
        l["tracking"]["active"] = json!("false");
        l["tables"]["nat"] = json!([]);
        l["tables"]["filter"] = json!([]);
        let p = |w: &Want| plan(&l, w, &path(), 8729).unwrap().2;
        assert!(!p(&want("filter", vec![json!({"chain": "forward", "action": "drop", "in-interface": "x", "disabled": "true"})])));
        assert!(p(&want("filter", vec![json!({"chain": "forward", "action": "drop", "in-interface": "x"})])));
        assert!(p(&want("nat", vec![json!({"chain": "srcnat", "action": "masquerade", "out-interface": "x"})])));
    }

    #[test]
    fn read_back_compares_the_asked_properties() {
        let mut after = live();
        // RouterOS adds counters and defaults; the order and asked values are what count
        after["tables"]["filter"][1]["log"] = json!("false");
        let w = want("filter", rows(&live(), "filter"));
        assert!(matches(&after, &w));
        after["tables"]["filter"][1]["dst-port"] = json!("68");
        assert!(!matches(&after, &w));
    }
}
