//! Wi-Fi page (lot Wi-Fi 1, read): every access point Cairn Net reads
//! (RouterOS 7 `wifi` package, /interface/wifi), seen as ONE network: the
//! radios of each access point (band, channel, width, power, state,
//! clients), the SSIDs merged by name with the differences between access
//! points, the clients of all access points with their name and IP (the
//! topology devices: DHCP leases, kinds.json, DNS), connections and roaming
//! read from the access points' logs, and the channel map.
//!
//! A switch is an access point when /interface/wifi has a radio (a row with
//! `radio-mac`). Only the fields listed in `build` leave this module: the
//! read account has no `sensitive` policy anyway (passphrases read "*****").
//!
//! Lot Wi-Fi 2: settings of ONE access point in direct mode (`plan`):
//! channel, width, power per radio; SSIDs on or off per radio; isolation
//! and password per SSID, in the profile that holds them. Guarded apply
//! (crate::guarded_apply: the scheduler, or the job where the device-mode
//! forbids it); old passwords are read by the write account for the
//! rollback and never leave the module unmasked.

use crate::direct::Cmd;
use crate::{infra, Api};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What /api/slow keeps of /interface/wifi (overview, topology).
pub const IFACE_PROPS: &str = "name,master-interface,radio-mac,running,disabled,configuration.ssid,datapath.vlan-id";

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn rows<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v[k].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn yes(v: &str) -> bool {
    v == "true" || v == "yes"
}
fn int(v: &str) -> Option<i64> {
    v.trim().parse().ok()
}

pub fn is_ap(v: &Value) -> bool {
    rows(v, "wifi").iter().any(|r| !s(r, "radio-mac").is_empty())
}

/// One access point: its Wi-Fi interfaces with their live state, radios,
/// registered clients and the Wi-Fi lines of its log. Not an access point:
/// {"wifi": []} after one call.
pub fn read(api: &mut Api) -> Result<Value, String> {
    let mut v = json!({"wifi": crate::chip::all(api, "/interface/wifi", &[])?});
    if !is_ap(&v) {
        return Ok(v);
    }
    let names: Vec<String> = rows(&v, "wifi").iter().filter(|r| !yes(s(r, "disabled"))).map(|r| s(r, "name").to_string()).collect();
    let mut monitor = vec![];
    for n in names {
        // one at a time: monitor answers without the interface's name
        if let Some(mut m) = api.call(&["/interface/wifi/monitor", &format!("=numbers={n}"), "=once="])?.pop() {
            m["name"] = json!(n);
            monitor.push(m);
        }
    }
    v["monitor"] = json!(monitor);
    v["radio"] = json!(crate::print(api, "/interface/wifi/radio", "interface,bands,current-max-reg-power,current-channels,hw-type", &[])?);
    // a running ":execute" of the write account = a rollback job waiting (lot Wi-Fi 2)
    v["jobs"] = json!(crate::print(api, "/system/script/job", "owner,type", &["?type=command"])?);
    v["rollback_sched"] = json!(!crate::print(api, "/system/scheduler", ".id", &[&format!("?name={}", infra::ROLLBACK)])?.is_empty());
    v["reg"] = json!(crate::chip::all(api, "/interface/wifi/registration-table", &[])?);
    // ponytail: the whole memory log (1000 lines at most) on every read; a remote log action if it ever weighs
    v["log"] = json!(crate::print(api, "/log", "time,topics,message", &[])?.into_iter().filter(|l| s(l, "topics").contains("wireless")).collect::<Vec<_>>());
    v["model"] = json!(crate::print(api, "/system/routerboard", "model", &[])?.pop().map_or(String::new(), |r| s(&r, "model").to_string()));
    Ok(v)
}

/// "5180/be/Ce/I" (monitor) or "2412" (configuration): control frequency,
/// low and high edges of the block in use, in MHz. C = control channel,
/// e = extension (20 MHz each): "Ce" is 40 MHz above, "eCee" 80 MHz.
fn block(spec: &str) -> Option<(u32, u32, u32)> {
    let mut it = spec.split('/');
    let f: u32 = it.next()?.trim().parse().ok()?;
    let ext = it.find(|p| !p.is_empty() && p.chars().all(|c| c == 'C' || c == 'e')).unwrap_or("C");
    let lo = f.checked_sub(10 + 20 * ext.find('C')? as u32)?;
    Some((f, lo, lo + 20 * ext.len() as u32))
}

fn band_of(f: u32) -> &'static str {
    if f < 3000 { "2.4" } else if f < 5925 { "5" } else { "6" }
}

fn channel_of(f: u32) -> u32 {
    match f {
        2484 => 14,
        f if f < 3000 => f.saturating_sub(2407) / 5,
        f if f < 5925 => f.saturating_sub(5000) / 5,
        f => f.saturating_sub(5950) / 5,
    }
}

/// The radio an interface runs on (a secondary SSID's master interface).
fn radio_of(wifi: &[Value], iface: &str) -> String {
    let m = wifi.iter().find(|r| s(r, "name") == iface).map_or("", |r| s(r, "master-interface"));
    if m.is_empty() { iface } else { m }.to_string()
}

/// One Wi-Fi log line: "<MAC>@<iface>(<SSID>) connected | disconnected, <why> |
/// roamed to <MAC>@<iface>(<SSID>) | associated | disassociated, <why>, signal strength -64, channel …".
fn event(ap: &str, l: &Value) -> Option<Value> {
    const VERBS: [&str; 5] = ["roamed to ", "disassociated", "associated", "disconnected", "connected"];
    let m = s(l, "message");
    let (mac, rest) = m.split_once('@')?;
    if mac.len() != 17 {
        return None;
    }
    let (iface, rest) = rest.split_once('(')?;
    // the SSID may hold anything: the verb after ") " tells where it ends
    let (at, verb) = VERBS.iter().filter_map(|v| rest.find(&format!(") {v}")).map(|i| (i, *v))).min_by_key(|x| x.0)?;
    let tail = &rest[at + 2 + verb.len()..];
    let signal = tail.split("signal strength ").nth(1).and_then(|x| int(x.split(',').next().unwrap_or("")));
    let mut e = json!({"t": s(l, "time"), "ap": ap, "mac": mac.to_uppercase(), "iface": iface, "ssid": &rest[..at],
        "what": verb.split(' ').next(), "signal": signal});
    if verb == "roamed to " {
        e["to"] = json!(tail.split_once('@')?.1.split('(').next());
    } else if let Some(why) = tail.strip_prefix(", ").map(|t| t.split(", signal strength").next().unwrap_or("")).filter(|w| !w.is_empty() && !w.starts_with("signal strength")) {
        e["why"] = json!(why);
    }
    Some(e)
}

/// The network view of the page from the reads of every access point
/// (`read` plus "name", or {"name", "error"}), the topology devices and the
/// DHCP leases of all switches (names and IPs).
pub fn build(aps: &[Value], devices: &[Value], leases: &[Value]) -> Value {
    let who = |mac: &str| -> (String, String, String) {
        if let Some(d) = devices.iter().find(|d| s(d, "mac").eq_ignore_ascii_case(mac)) {
            return (s(d, "name").into(), s(d, "ip").into(), s(d, "kind").into());
        }
        leases.iter().find(|l| s(l, "mac-address").eq_ignore_ascii_case(mac))
            .map_or_else(Default::default, |l| (s(l, "host-name").into(), s(l, "address").into(), String::new()))
    };
    let ok: Vec<&Value> = aps.iter().filter(|a| a["error"].is_null() && is_ap(a)).collect();

    // radios
    let mut radios: Vec<Value> = vec![];
    for ap in &ok {
        let wifi = rows(ap, "wifi");
        for r in wifi.iter().filter(|r| !s(r, "radio-mac").is_empty()) {
            let n = s(r, "name");
            let mon = rows(ap, "monitor").iter().find(|m| s(m, "name") == n);
            let conf = s(r, "channel.frequency").split(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
            let blk = mon.and_then(|m| block(s(m, "channel"))).or_else(|| block(conf));
            let rad = rows(ap, "radio").iter().find(|x| s(x, "interface") == n);
            let band = blk.map(|b| band_of(b.0)).or_else(|| {
                let b = rad.map_or("", |x| s(x, "bands"));
                ["2ghz", "5ghz", "6ghz"].iter().position(|k| b.contains(k)).map(|i| ["2.4", "5", "6"][i])
            });
            radios.push(json!({
                "ap": s(ap, "name"), "iface": n, "band": band,
                "freq": blk.map(|b| b.0), "channel": blk.map(|b| channel_of(b.0)), "lo": blk.map(|b| b.1), "hi": blk.map(|b| b.2),
                "width": blk.map(|b| b.2 - b.1), "width_cfg": s(r, "channel.width"), "auto": conf.is_empty(), "freq_cfg": s(r, "channel.frequency"),
                "tx_power": mon.and_then(|m| int(s(m, "tx-power"))), "max_power": rad.and_then(|x| int(s(x, "current-max-reg-power"))),
                "tx_cfg": s(r, "configuration.tx-power"),
                "channels": channels(rad.map_or("", |x| s(x, "current-channels"))).into_iter()
                    .map(|(f, ws)| json!({"f": f, "ch": channel_of(f), "w": ws.into_iter().map(|(w, dfs)| json!([w, dfs])).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "state": if yes(s(r, "disabled")) { "disabled" } else { mon.map_or("unknown", |m| s(m, "state")) },
                "clients": rows(ap, "reg").iter().filter(|c| radio_of(wifi, s(c, "interface")) == n).count(),
                "ssids": wifi.iter().filter(|x| radio_of(wifi, s(x, "name")) == n && !s(x, "configuration.ssid").is_empty())
                    .map(|x| s(x, "configuration.ssid")).collect::<Vec<_>>(),
            }));
        }
    }
    // channel map: blocks of different radios in the same band that overlap
    let spans: Vec<(String, String, Value, u64, u64, u64)> = radios.iter()
        .filter_map(|r| Some((s(r, "ap").to_string(), s(r, "iface").to_string(), r["band"].clone(), r["freq"].as_u64()?, r["lo"].as_u64()?, r["hi"].as_u64()?))).collect();
    for r in &mut radios {
        let (Some(f), Some(lo), Some(hi)) = (r["freq"].as_u64(), r["lo"].as_u64(), r["hi"].as_u64()) else { continue };
        let over: Vec<Value> = spans.iter()
            .filter(|o| (o.0 != s(r, "ap") || o.1 != s(r, "iface")) && o.2 == r["band"] && o.4 < hi && lo < o.5)
            .map(|o| json!({"ap": o.0, "iface": o.1, "same": o.3 == f})).collect();
        r["overlap"] = json!(over);
    }

    // SSIDs merged by name, one entry per interface carrying it
    let mut by_ssid: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for ap in &ok {
        let wifi = rows(ap, "wifi");
        for x in wifi.iter().filter(|x| !s(x, "configuration.ssid").is_empty()) {
            let radio = radio_of(wifi, s(x, "name"));
            let band = radios.iter().find(|r| s(r, "ap") == s(ap, "name") && s(r, "iface") == radio).map_or(Value::Null, |r| r["band"].clone());
            let mut sec: Vec<&str> = s(x, "security.authentication-types").split(',').filter(|a| !a.is_empty()).collect();
            sec.sort_unstable();
            by_ssid.entry(s(x, "configuration.ssid").to_string()).or_default().push(json!({
                "ap": s(ap, "name"), "iface": s(x, "name"), "radio": radio, "band": band,
                "vlan": s(x, "datapath.vlan-id"), "security": sec.join(","),
                "isolation": yes(s(x, "datapath.client-isolation")), "hidden": yes(s(x, "configuration.hide-ssid")),
                "enabled": !yes(s(x, "disabled")),
                "clients": rows(ap, "reg").iter().filter(|c| s(c, "interface") == s(x, "name")).count(),
            }));
        }
    }
    let ssids: Vec<Value> = by_ssid.into_iter().map(|(ssid, on)| {
        let mut v = json!({"ssid": ssid, "clients": on.iter().filter_map(|e| e["clients"].as_u64()).sum::<u64>()});
        let mut drift = vec![];
        for f in ["vlan", "security", "isolation", "hidden", "enabled"] {
            let mut by: BTreeMap<String, (Value, Vec<String>)> = BTreeMap::new();
            for e in &on {
                by.entry(e[f].to_string()).or_insert_with(|| (e[f].clone(), vec![])).1.push(format!("{} {}", s(e, "ap"), s(e, "iface")));
            }
            // the value most interfaces have stands for the SSID (a tie: the first in order)
            v[f] = by.values().rev().max_by_key(|x| x.1.len()).map_or(Value::Null, |x| x.0.clone());
            if by.len() > 1 {
                drift.push(json!({"what": f, "by": by.into_values().map(|(value, w)| json!({"value": value, "where": w})).collect::<Vec<_>>()}));
            }
        }
        // bands per access point (2.4 + 5 on one, 5 alone on another)
        let mut bands: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for e in on.iter().filter(|e| e["enabled"] == true) {
            bands.entry(s(e, "ap")).or_default().insert(s(e, "band").to_string());
        }
        let mut sets: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for (ap, b) in &bands {
            sets.entry(b.iter().cloned().collect::<Vec<_>>().join("+")).or_default().push(*ap);
        }
        if sets.len() > 1 {
            drift.push(json!({"what": "bands", "by": sets.into_iter().map(|(value, w)| json!({"value": value, "where": w})).collect::<Vec<_>>()}));
        }
        let missing: Vec<&str> = ok.iter().map(|a| s(a, "name")).filter(|n| !on.iter().any(|e| s(e, "ap") == *n)).collect();
        if !missing.is_empty() {
            drift.push(json!({"what": "missing", "where": missing}));
        }
        v["on"] = json!(on);
        v["drift"] = json!(drift);
        v
    }).collect();

    // log lines of all access points, oldest first; a client connecting to
    // another access point than its last one has roamed there
    let mut events: Vec<Value> = ok.iter().flat_map(|a| rows(a, "log").iter().filter_map(move |l| event(s(a, "name"), l))).collect();
    events.sort_by(|a, b| s(a, "t").cmp(s(b, "t")));
    let mut last: BTreeMap<String, String> = BTreeMap::new();
    let mut roams: BTreeMap<String, (u64, String)> = BTreeMap::new();
    for e in &mut events {
        let (mac, ap, t) = (s(e, "mac").to_string(), s(e, "ap").to_string(), s(e, "t").to_string());
        let what = s(e, "what").to_string();
        let roamed = match what.as_str() {
            "roamed" => true,
            "connected" => match last.get(&mac).filter(|p| **p != ap) {
                Some(p) => {
                    e["from"] = json!(p);
                    true
                }
                None => false,
            },
            _ => false,
        };
        if roamed {
            let r = roams.entry(mac.clone()).or_default();
            *r = (r.0 + 1, t);
        }
        if roamed || what == "connected" {
            last.insert(mac.clone(), ap);
        }
        e["name"] = json!(who(&mac).0);
    }
    events.reverse();
    events.truncate(200);

    let mut clients: Vec<Value> = vec![];
    for ap in &ok {
        let wifi = rows(ap, "wifi");
        for c in rows(ap, "reg") {
            let mac = s(c, "mac-address").to_uppercase();
            let (name, ip, kind) = who(&mac);
            let (tx_bytes, rx_bytes) = s(c, "bytes").split_once(',').map_or((None, None), |(a, b)| (int(a), int(b)));
            let (n, last_roam) = roams.get(&mac).cloned().unwrap_or_default();
            clients.push(json!({
                "mac": mac, "name": name, "ip": ip, "kind": kind,
                "ap": s(ap, "name"), "iface": s(c, "interface"), "radio": radio_of(wifi, s(c, "interface")),
                "ssid": s(c, "ssid"), "vlan": s(c, "vlan-id"), "band": s(c, "band"), "signal": int(s(c, "signal")),
                "tx_rate": int(s(c, "tx-rate")), "rx_rate": int(s(c, "rx-rate")),
                "tx_bps": int(s(c, "tx-bits-per-second")), "rx_bps": int(s(c, "rx-bits-per-second")),
                "tx_bytes": tx_bytes, "rx_bytes": rx_bytes,
                "uptime": s(c, "uptime"), "auth": s(c, "auth-type"), "authorized": yes(s(c, "authorized")),
                "roams": n, "last_roam": last_roam,
            }));
        }
    }
    // named clients first, by name; then by IP, then by MAC
    let key = |c: &Value| (s(c, "name").is_empty(), [s(c, "name"), s(c, "ip"), s(c, "mac")].into_iter().find(|x| !x.is_empty()).unwrap_or("").to_lowercase());
    clients.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then_with(|| crate::topo::natural(&ka.1, &kb.1))
    });

    let aps: Vec<Value> = aps.iter().map(|a| {
        let n = s(a, "name");
        json!({"name": n, "model": s(a, "model"), "error": a["error"], "clients": rows(a, "reg").len(), "mode": s(a, "mode"), "pending": a["pending"] == true,
            "radios": radios.iter().filter(|r| s(r, "ap") == n).cloned().collect::<Vec<_>>()})
    }).collect();
    json!({"aps": aps, "ssids": ssids, "clients": clients, "events": events})
}

static CACHE: Mutex<Option<(Instant, String)>> = Mutex::new(None);

/// GET /api/wifi: every access point read now (5 s shared by viewers). An
/// access point that stops answering stays listed, with its error, as long
/// as /api/slow last saw it with radios.
pub fn view() -> String {
    crate::cached(&CACHE, Duration::from_secs(5), || {
        let slow: Value = serde_json::from_str(&crate::slow()).unwrap_or(Value::Null);
        let sws = rows(&slow, "switches");
        let was_ap = |n: &str| sws.iter().any(|w| s(w, "name") == n && is_ap(w));
        let mut aps: Vec<Value> = crate::collect(read).into_iter().filter(|v| is_ap(v) || (!v["error"].is_null() && was_ap(s(v, "name")))).collect();
        for a in &mut aps {
            if let Some(sw) = crate::settings::get(s(a, "name")) {
                let c = &sw.cfg;
                // a change waiting for "Keep": the rollback scheduler, or the write account's job
                a["pending"] = json!(c.mode == "direct" && (a["rollback_sched"] == true || rows(a, "jobs").iter().any(|j| s(j, "owner") == c.write_user)));
                a["mode"] = json!(c.mode);
            }
        }
        let leases: Vec<Value> = sws.iter().flat_map(|w| rows(w, "leases").iter().cloned()).collect();
        let mut v = build(&aps, rows(&slow["topology"], "devices"), &leases);
        v["t"] = json!(crate::now_ms());
        v["rollback_s"] = json!(infra::ROLLBACK_S);
        v.to_string()
    })
}

/* ---------- settings of one access point (lot Wi-Fi 2, direct mode) ---------- */

/// Legal channels of a radio (`current-channels`, "5180/ac/Ceee/I" =
/// control 5180 MHz in an 80 MHz block; flag D = radar, DFS): control
/// frequency -> width (MHz) -> DFS.
fn channels(spec: &str) -> BTreeMap<u32, BTreeMap<u32, bool>> {
    let ce = |p: &str| !p.is_empty() && p.chars().all(|c| c == 'C' || c == 'e');
    let mut out: BTreeMap<u32, BTreeMap<u32, bool>> = BTreeMap::new();
    for c in spec.split([',', ';']) {
        let mut it = c.split('/');
        let Some(f) = it.next().and_then(|f| f.trim().parse::<u32>().ok()) else { continue };
        let rest: Vec<&str> = it.skip(1).collect(); // the mode (a, n, ac, ax, be) does not matter here
        let w = rest.iter().find(|p| ce(p)).map_or(1, |p| p.len()) as u32 * 20;
        let dfs = rest.iter().any(|p| !ce(p) && p.contains('D'));
        let e = out.entry(f).or_default().entry(w).or_insert(dfs);
        *e |= dfs;
    }
    out
}

/// "20/40/80mhz" -> 80; "" -> None.
fn width_mhz(v: &str) -> Option<u32> {
    v.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).max()
}
const WIDTHS: [&str; 5] = ["20mhz", "20/40mhz", "20/40/80mhz", "20/40/80/160mhz", "20/40/80/160/320mhz"];

fn yn(b: bool) -> String {
    if b { "yes" } else { "no" }.into()
}
/// A rollback line that never stops the rest of the rollback.
fn guarded(l: String) -> String {
    format!(":do {{ {l} }} on-error={{}}")
}

fn passphrase_ok(p: &str) -> Result<(), String> {
    let hex = p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit());
    if hex || ((8..=63).contains(&p.len()) && p.chars().all(|c| (' '..='~').contains(&c))) {
        Ok(())
    } else {
        Err("a Wi-Fi password is 8 to 63 printable ASCII characters".into())
    }
}

/// What the plan needs, read with the account given: interfaces (effective
/// values), configuration, security and datapath profiles, the radios'
/// legal channels; with `secrets` (write account, `sensitive`) the
/// passphrase of every security profile and interface in "secret", for the
/// rollback and the read back. They never leave this module.
pub fn conf(api: &mut Api, secrets: bool) -> Result<Value, String> {
    let mut v = json!({
        "wifi": crate::chip::all(api, "/interface/wifi", &[])?,
        "cfg": crate::chip::all(api, "/interface/wifi/configuration", &[])?,
        "sec": crate::print(api, "/interface/wifi/security", ".id,name,authentication-types", &[])?,
        "dp": crate::chip::all(api, "/interface/wifi/datapath", &[])?,
        "radio": crate::print(api, "/interface/wifi/radio", "interface,current-channels,current-max-reg-power", &[])?,
    });
    if secrets {
        for (k, path, prop) in [("sec", "/interface wifi security", "passphrase"), ("wifi", "/interface wifi", "security.passphrase")] {
            for r in v[k].as_array_mut().into_iter().flatten() {
                let script = format!(":put [{path} get [find where .id={}] {prop}]", s(r, ".id"));
                let (_, done) = api.call_done(&["/execute", &format!("=script={script}"), "=as-string="])?;
                let out = done.get("ret").and_then(Value::as_str).unwrap_or("");
                r["secret"] = json!(out.strip_suffix("\r\n").or(out.strip_suffix('\n')).unwrap_or(out));
            }
        }
    }
    Ok(v)
}

/// The profiles (`security` or `datapath`) that hold a setting for the
/// interfaces of one SSID: each profile with its interfaces, None for the
/// interfaces that use none (the setting is then written on them). A profile
/// another SSID also uses is refused: the change would reach that SSID too.
#[allow(clippy::type_complexity)]
fn holders<'a>(live: &'a Value, on: &[&'a Value], kind: &str, table: &str) -> Result<Vec<(Option<&'a Value>, Vec<&'a Value>)>, String> {
    let prof = |r: &Value| -> String {
        let own = s(r, kind);
        if !own.is_empty() {
            return own.into();
        }
        rows(live, "cfg").iter().find(|c| s(c, "name") == s(r, "configuration")).map_or(String::new(), |c| s(c, kind).to_string())
    };
    let ssid = s(on[0], "configuration.ssid");
    let mut out: Vec<(Option<&Value>, Vec<&Value>)> = vec![];
    for r in on {
        let name = prof(r);
        let p = match name.as_str() {
            "" => None,
            n => Some(rows(live, table).iter().find(|x| s(x, "name") == n).ok_or_else(|| format!("{kind} profile {n}: not found"))?),
        };
        if !name.is_empty() {
            if let Some(o) = rows(live, "wifi").iter().find(|x| s(x, "configuration.ssid") != ssid && prof(x) == name) {
                return Err(format!("{kind} profile {name} also serves {} ({}): change it on the device, or give {ssid} its own profile",
                    s(o, "configuration.ssid"), s(o, "name")));
            }
        }
        match out.iter_mut().find(|(q, _)| q.map(|q| s(q, "name")) == p.map(|p| s(p, "name"))) {
            Some(e) => e.1.push(r),
            None => out.push((p, vec![r])),
        }
    }
    Ok(out)
}

/// The commands, the rollback lines and the warnings of a change on one
/// access point. `want`: {"ifaces": [{name, frequency?, width?, tx_power?,
/// disabled?}], "ssids": [{ssid, isolation?, passphrase?}]}; a field left
/// out is not changed, "" = automatic (the value is unset).
pub fn plan(live: &Value, want: &Value) -> Result<(Vec<Cmd>, Vec<String>, Vec<String>), String> {
    let wifi = rows(live, "wifi");
    let row = |n: &str| wifi.iter().find(|r| s(r, "name") == n).ok_or_else(|| format!("{n}: no such Wi-Fi interface"));
    let station = |r: &Value| s(r, "configuration.mode").starts_with("station");
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    let (mut cmds, mut undo, mut warn): (Vec<Cmd>, Vec<String>, Vec<String>) = (vec![], vec![], vec![]);
    let mut off: BTreeMap<&str, bool> = wifi.iter().map(|r| (s(r, "name"), yes(s(r, "disabled")))).collect();
    let mut touched: BTreeSet<&str> = BTreeSet::new();

    for w in want["ifaces"].as_array().into_iter().flatten() {
        let n = s(w, "name");
        let r = row(n)?;
        if station(r) {
            return Err(format!("{n} is a station (an uplink?): not changed here"));
        }
        let id = s(r, ".id");
        let radio = rows(live, "radio").iter().find(|x| s(x, "interface") == n);
        let legal = channels(radio.map_or("", |x| s(x, "current-channels")));
        let (mut set, mut back, mut unset, mut unset_back) = (vec![kv(".id", id)], vec![], vec![], vec![]);
        let mut moved = false;
        for (k, prop) in [("frequency", "channel.frequency"), ("width", "channel.width"), ("tx_power", "configuration.tx-power")] {
            let Some(new) = w.get(k).and_then(Value::as_str) else { continue };
            let cur = s(r, prop);
            if new == cur {
                continue;
            }
            if s(r, "radio-mac").is_empty() {
                return Err(format!("{n} runs on {}'s radio: change its {k} there", s(r, "master-interface")));
            }
            if !new.is_empty() {
                match k {
                    "frequency" if new.parse().ok().is_none_or(|f: u32| !legal.contains_key(&f)) => return Err(format!("{n}: {new} MHz is not a legal channel of this radio here")),
                    "width" if !WIDTHS.contains(&new) || !legal.values().any(|ws| width_mhz(new).is_some_and(|m| ws.contains_key(&m))) =>
                        return Err(format!("{n}: width {new} not available on this radio")),
                    "tx_power" => {
                        let max = radio.and_then(|x| int(s(x, "current-max-reg-power"))).unwrap_or(30);
                        if int(new).is_none_or(|p| p < 1 || p > max) {
                            return Err(format!("{n}: transmit power 1 to {max} dBm (the legal maximum here)"));
                        }
                    }
                    _ => {}
                }
            }
            if new.is_empty() { unset.push(prop) } else { set.push(kv(prop, new)) }
            if cur.is_empty() { unset_back.push(prop) } else { back.push(format!("{prop}={}", infra::q(cur))) }
            touched.insert(n);
            moved |= k != "tx_power";
        }
        // the block the radio will use: a fixed channel at a fixed width tells whether it is a radar channel
        let get = |k: &str, prop: &str| w.get(k).and_then(Value::as_str).unwrap_or(s(r, prop)).to_string();
        let (f, wd) = (get("frequency", "channel.frequency"), get("width", "channel.width"));
        if moved {
            let dfs = f.parse().ok().and_then(|f: u32| legal.get(&f)).map(|ws| match width_mhz(&wd) {
                Some(m) => ws.get(&m).copied().unwrap_or(false),
                None => ws.values().any(|d| *d), // automatic width: the widest block may be one
            });
            if dfs == Some(true) {
                warn.push(format!("{n}: radar (DFS) channel: the radio listens 1 to 10 min before sending, its clients are cut meanwhile"));
            } else if f.is_empty() && legal.keys().any(|f| band_of(*f) == "5") {
                warn.push(format!("{n}: automatic channel: the radio may pick a radar (DFS) channel"));
            }
            warn.push(format!("{n}: its clients reconnect"));
        }
        if let Some(d) = w.get("disabled").and_then(Value::as_bool).filter(|d| *d != yes(s(r, "disabled"))) {
            set.push(kv("disabled", &yn(d)));
            back.push(format!("disabled={}", yn(!d)));
            off.insert(n, d);
            touched.insert(n);
            if d {
                warn.push(format!("{n}: off, its clients are cut"));
            }
        }
        if set.len() > 1 {
            cmds.push(("/interface/wifi/set".into(), set));
        }
        for p in unset {
            cmds.push(("/interface/wifi/unset".into(), vec![kv("numbers", id), kv("value-name", p)]));
        }
        if !back.is_empty() {
            undo.push(guarded(format!("/interface wifi set [find where .id={id}] {}", back.join(" "))));
        }
        for p in unset_back {
            undo.push(guarded(format!("/interface wifi unset [find where .id={id}] value-name={p}")));
        }
    }
    // a radio off stops its secondary SSIDs: both go off together
    for r in wifi {
        let (n, m) = (s(r, "name"), s(r, "master-interface"));
        if !m.is_empty() && (touched.contains(n) || touched.contains(m)) && off.get(m) == Some(&true) && off.get(n) == Some(&false) {
            return Err(format!("{m} off stops {n} ({}) too: turn it off as well, or leave {m} on", s(r, "configuration.ssid")));
        }
    }

    for w in want["ssids"].as_array().into_iter().flatten() {
        let ssid = s(w, "ssid");
        let on: Vec<&Value> = wifi.iter().filter(|r| s(r, "configuration.ssid") == ssid && !station(r)).collect();
        if on.is_empty() {
            return Err(format!("{ssid}: not on this access point"));
        }
        if let Some(b) = w.get("isolation").and_then(Value::as_bool).filter(|b| on.iter().any(|r| yes(s(r, "datapath.client-isolation")) != *b)) {
            for (p, ifs) in holders(live, &on, "datapath", "dp")? {
                match p {
                    Some(p) => {
                        cmds.push(("/interface/wifi/datapath/set".into(), vec![kv(".id", s(p, ".id")), kv("client-isolation", &yn(b))]));
                        let old = s(p, "client-isolation");
                        undo.push(guarded(if old.is_empty() {
                            format!("/interface wifi datapath unset [find where .id={}] value-name=client-isolation", s(p, ".id"))
                        } else {
                            format!("/interface wifi datapath set [find where .id={}] client-isolation={}", s(p, ".id"), yn(yes(old)))
                        }));
                    }
                    None => for r in ifs {
                        cmds.push(("/interface/wifi/set".into(), vec![kv(".id", s(r, ".id")), kv("datapath.client-isolation", &yn(b))]));
                        undo.push(guarded(format!("/interface wifi set [find where .id={}] datapath.client-isolation={}", s(r, ".id"), yn(yes(s(r, "datapath.client-isolation"))))));
                    },
                }
            }
            if !b {
                warn.push(format!("{ssid}: its clients see each other again"));
            }
        }
        if let Some(pw) = w.get("passphrase").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            passphrase_ok(pw)?;
            if on.iter().all(|r| s(r, "security.authentication-types").is_empty()) {
                return Err(format!("{ssid} is open (no security): no password to change"));
            }
            // the old passphrase comes from the write account; a preview shows it masked anyway
            let old = |r: &Value| r.get("secret").and_then(Value::as_str).map_or_else(|| "•••".to_string(), str::to_string);
            let mut changed = false;
            for (p, ifs) in holders(live, &on, "security", "sec")? {
                match p {
                    Some(p) if p.get("secret").and_then(Value::as_str) != Some(pw) => {
                        cmds.push(("/interface/wifi/security/set".into(), vec![kv(".id", s(p, ".id")), kv("passphrase", pw)]));
                        undo.push(guarded(format!("/interface wifi security set [find where .id={}] passphrase={}", s(p, ".id"), infra::q(&old(p)))));
                        changed = true;
                    }
                    Some(_) => {}
                    None => for r in ifs.into_iter().filter(|r| r.get("secret").and_then(Value::as_str) != Some(pw)) {
                        cmds.push(("/interface/wifi/set".into(), vec![kv(".id", s(r, ".id")), kv("security.passphrase", pw)]));
                        undo.push(guarded(format!("/interface wifi set [find where .id={}] security.passphrase={}", s(r, ".id"), infra::q(&old(r)))));
                        changed = true;
                    },
                }
            }
            if changed {
                warn.push(format!("{ssid}: every device on it needs the new password"));
            }
        }
    }
    if !cmds.is_empty() {
        infra::finish(&mut undo);
    }
    Ok((cmds, undo, warn))
}

/// Whether the access point now runs what was asked (`conf` with secrets).
pub fn matches(live: &Value, want: &Value) -> bool {
    let wifi = rows(live, "wifi");
    let ifaces = want["ifaces"].as_array().into_iter().flatten().all(|w| wifi.iter().find(|r| s(r, "name") == s(w, "name")).is_some_and(|r| {
        [("frequency", "channel.frequency"), ("width", "channel.width"), ("tx_power", "configuration.tx-power")].iter()
            .all(|(k, p)| w.get(*k).and_then(Value::as_str).is_none_or(|v| s(r, p) == v))
            && w.get("disabled").and_then(Value::as_bool).is_none_or(|d| yes(s(r, "disabled")) == d)
    }));
    let ssids = want["ssids"].as_array().into_iter().flatten().all(|w| {
        let on: Vec<&Value> = wifi.iter().filter(|r| s(r, "configuration.ssid") == s(w, "ssid")).collect();
        !on.is_empty()
            && w.get("isolation").and_then(Value::as_bool).is_none_or(|b| on.iter().all(|r| yes(s(r, "datapath.client-isolation")) == b))
            && w.get("passphrase").and_then(Value::as_str).filter(|p| !p.is_empty()).is_none_or(|p| on.iter().all(|r| r.get("secret").and_then(Value::as_str) == Some(p)))
    });
    ifaces && ssids
}

/// POST /api/wifi/preview: nothing is written; the rollback lines without the
/// old passwords.
pub fn preview(v: &Value) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let (cmds, undo, warn, sched) = crate::with_api(&sw, |api| {
        let (c, u, w) = plan(&conf(api, false)?, &v["want"])?;
        Ok((c, u, w, crate::has_scheduler(api)))
    })?;
    let undo: Vec<String> = undo.iter().filter(|l| sched || !l.starts_with("/system scheduler remove")).map(|l| crate::direct::redact(l)).collect();
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S, "warnings": warn}))
}

/// POST /api/wifi/apply: guarded apply (write account); "Keep" is
/// /api/infra/confirm, as on the other pages.
pub fn apply(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let want = &v["want"];
    let out = crate::guarded_apply(&sw.cfg, user, "wifi", |api| {
        let (c, u, _) = plan(&conf(api, true)?, want)?;
        Ok((c, u))
    }, |api| Ok(matches(&conf(api, true)?, want)));
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real read of a hAP be lite (RouterOS 7.25.1, wifi-mediatek): three
    /// SSIDs on both radios, 16 clients, its log since boot; MACs and names
    /// replaced.
    fn hall() -> Value {
        serde_json::from_str(include_str!("wifi_fixture.json")).unwrap()
    }
    fn find<'a>(v: &'a Value, k: &str, key: &str, val: &str) -> &'a Value {
        rows(v, k).iter().find(|x| s(x, key) == val).unwrap_or_else(|| panic!("{k}: no {key}={val}"))
    }

    #[test]
    fn blocks_and_channel_numbers() {
        assert_eq!(block("2412/be"), Some((2412, 2402, 2422)));
        assert_eq!(block("5180/be/Ce/I"), Some((5180, 5170, 5210)));
        assert_eq!(block("5200/ax/eCee/I"), Some((5200, 5170, 5250)));
        assert_eq!(block("5180/ac/Ceeeeeee/DI"), Some((5180, 5170, 5330)));
        assert_eq!(block("2412"), Some((2412, 2402, 2422)));
        assert_eq!(block(""), None);
        assert_eq!([2412, 2472, 2484, 5180, 5825, 5955].map(channel_of), [1, 13, 14, 36, 165, 1]);
        assert_eq!([2412, 5180, 5955].map(band_of), ["2.4", "5", "6"]);
    }

    #[test]
    fn log_lines() {
        let l = |m: &str| event("ap", &json!({"time": "2026-11-19 17:00:00", "topics": "wireless,info", "message": m}));
        let e = l("02:00:00:00:00:01@wifi2(My (home) net) connected, signal strength -61, channel 5180/be/Ce/I").unwrap();
        assert_eq!((s(&e, "what"), s(&e, "iface"), s(&e, "ssid"), e["signal"].as_i64()), ("connected", "wifi2", "My (home) net", Some(-61)));
        assert!(e["why"].is_null());
        let e = l("02:00:00:00:00:01@wifi1(Home) disconnected, connection lost, signal strength -76, channel 2412/be").unwrap();
        assert_eq!((s(&e, "what"), s(&e, "why")), ("disconnected", "connection lost"));
        let e = l("00:11:22:00:00:17@wifi1-iot(IoT) disassociated, key handshake timeout (wrong pass), signal strength -64, channel 2412/be").unwrap();
        assert_eq!((s(&e, "what"), s(&e, "why"), s(&e, "mac")), ("disassociated", "key handshake timeout (wrong pass)", "00:11:22:00:00:17"));
        let e = l("02:00:00:00:00:16@wifi1(Home) roamed to 02:00:00:00:00:16@wifi2(Home), signal strength -73, channel 5180/be/Ce/I").unwrap();
        assert_eq!((s(&e, "what"), s(&e, "iface"), s(&e, "to")), ("roamed", "wifi1", "wifi2"));
        assert!(l("user monitor logged in from 192.168.88.2 via api").is_none());
        assert!(l("x@y(z) something else").is_none());
    }

    #[test]
    fn one_access_point_from_a_real_read() {
        let v = build(&[hall()], &[], &[]);
        let ap = &v["aps"][0];
        assert_eq!((s(ap, "name"), s(ap, "model"), ap["clients"].as_u64()), ("ap-hall", "A42G-HbeP", Some(16)));
        let r1 = find(ap, "radios", "iface", "wifi1");
        assert_eq!((s(r1, "band"), r1["channel"].as_u64(), r1["width"].as_u64(), r1["tx_power"].as_i64(), r1["max_power"].as_i64()),
            ("2.4", Some(1), Some(20), Some(15), Some(20)));
        assert_eq!((s(r1, "state"), r1["auto"].as_bool()), ("running", Some(false)));
        assert_eq!(r1["ssids"], json!(["Home", "Guests", "IoT"]));
        let r2 = find(ap, "radios", "iface", "wifi2");
        assert_eq!((s(r2, "band"), r2["channel"].as_u64(), r2["lo"].as_u64(), r2["hi"].as_u64()), ("5", Some(36), Some(5170), Some(5210)));
        assert_eq!(r1["clients"].as_u64().unwrap() + r2["clients"].as_u64().unwrap(), 16);
        assert_eq!(rows(ap, "radios").len(), 2, "the disabled multi-link interface is no radio");
        assert!(rows(r1, "overlap").is_empty() && rows(r2, "overlap").is_empty());

        assert_eq!(rows(&v, "ssids").iter().map(|x| s(x, "ssid")).collect::<Vec<_>>(), ["Guests", "Home", "IoT"]);
        let guests = find(&v, "ssids", "ssid", "Guests");
        assert_eq!((s(guests, "vlan"), s(guests, "security"), guests["isolation"].as_bool()), ("30", "wpa2-psk,wpa3-psk", Some(true)));
        let iot = find(&v, "ssids", "ssid", "IoT");
        assert_eq!((s(iot, "vlan"), s(iot, "security"), rows(iot, "on").len()), ("20", "wpa2-psk", 2));
        assert!(rows(&v, "ssids").iter().all(|x| rows(x, "drift").is_empty()), "one access point, same settings on both radios");

        assert_eq!(rows(&v, "clients").len(), 16);
        assert!(rows(&v, "clients").iter().all(|c| !s(c, "ssid").is_empty() && c["signal"].as_i64().is_some()));
        let ev = rows(&v, "events");
        assert_eq!(s(&ev[0], "what"), "roamed", "newest first");
        assert!(ev.iter().any(|e| s(e, "why") == "key handshake timeout (wrong pass)"));
        let text = v.to_string();
        assert!(!text.contains("*****") && !text.contains("passphrase"));
    }

    #[test]
    fn names_from_devices_then_leases() {
        let fx = hall();
        let (a, b) = (s(&fx["reg"][0], "mac-address").to_string(), s(&fx["reg"][1], "mac-address").to_string());
        let devices = [json!({"mac": a, "name": "laptop", "ip": "10.0.10.5", "kind": "pc"})];
        let leases = [json!({"mac-address": b.to_lowercase(), "host-name": "phone", "address": "10.0.10.6"})];
        let v = build(&[fx], &devices, &leases);
        let ca = find(&v, "clients", "mac", &a);
        assert_eq!((s(ca, "name"), s(ca, "ip"), s(ca, "kind")), ("laptop", "10.0.10.5", "pc"));
        let cb = find(&v, "clients", "mac", &b);
        assert_eq!((s(cb, "name"), s(cb, "ip")), ("phone", "10.0.10.6"));
        assert_eq!(s(&v["clients"][0], "name"), "laptop", "named clients first, by name");
    }

    #[test]
    fn two_access_points_differences_overlaps_and_roaming() {
        let mut office = hall();
        office["name"] = json!("ap-office");
        office["reg"] = json!([]);
        // 2.4 GHz on the same channel, 5 GHz on the upper half of the same 40 MHz block
        for m in office["monitor"].as_array_mut().unwrap() {
            let ch = if s(m, "channel").starts_with("2412") { "2412/be" } else { "5200/be/eC/I" };
            m["channel"] = json!(ch);
        }
        // guests in another VLAN there, no IoT SSID
        let wifi: Vec<Value> = rows(&office, "wifi").iter().filter(|r| s(r, "configuration.ssid") != "IoT").cloned().map(|mut r| {
            if s(&r, "configuration.ssid") == "Guests" {
                r["datapath.vlan-id"] = json!("31");
            }
            r
        }).collect();
        office["wifi"] = json!(wifi);
        let mover = "02:00:00:00:00:16";
        office["log"] = json!([{"time": "2026-11-19 18:30:00", "topics": "wireless,info",
            "message": format!("{mover}@wifi2(Home) connected, signal strength -58, channel 5200/be/eC/I")}]);
        let v = build(&[hall(), office], &[], &[]);

        let o2 = find(&v["aps"][1], "radios", "iface", "wifi2");
        assert_eq!(o2["channel"].as_u64(), Some(40));
        assert_eq!(o2["overlap"], json!([{"ap": "ap-hall", "iface": "wifi2", "same": false}]));
        let o1 = find(&v["aps"][1], "radios", "iface", "wifi1");
        assert_eq!(o1["overlap"], json!([{"ap": "ap-hall", "iface": "wifi1", "same": true}]));

        let guests = find(&v, "ssids", "ssid", "Guests");
        let vlan = rows(guests, "drift").iter().find(|d| s(d, "what") == "vlan").expect("VLAN difference");
        assert_eq!(vlan["by"], json!([{"value": "30", "where": ["ap-hall wifi1-guest", "ap-hall wifi2-guest"]},
            {"value": "31", "where": ["ap-office wifi1-guest", "ap-office wifi2-guest"]}]));
        assert_eq!(s(guests, "vlan"), "30", "a tie keeps the first value");
        let iot = find(&v, "ssids", "ssid", "IoT");
        assert_eq!(rows(iot, "drift"), [json!({"what": "missing", "where": ["ap-office"]})]);
        assert!(rows(find(&v, "ssids", "ssid", "Home"), "drift").is_empty());

        let ev = &v["events"][0];
        assert_eq!((s(ev, "ap"), s(ev, "what"), s(ev, "from")), ("ap-office", "connected", "ap-hall"));
        let within = rows(&hall(), "log").iter().filter(|l| s(l, "message").starts_with(mover) && s(l, "message").contains("roamed to")).count();
        assert!(within > 0, "the fixture has a roam between the two radios");
        let c = find(&v, "clients", "mac", mover);
        assert_eq!(c["roams"].as_u64(), Some(within as u64 + 1));
        assert_eq!(s(c, "last_roam"), "2026-11-19 18:30:00");
    }

    #[test]
    fn an_access_point_that_does_not_answer() {
        let v = build(&[hall(), json!({"name": "ap-attic", "error": "timeout"})], &[], &[]);
        assert_eq!(s(&v["aps"][1], "error"), "timeout");
        assert!(rows(&v["aps"][1], "radios").is_empty());
        assert!(rows(&v, "ssids").iter().all(|x| rows(x, "drift").is_empty()), "not counted as missing its SSIDs");
    }

    /* ---------- settings (lot Wi-Fi 2) ---------- */

    /// `conf` of the fixture's access point: its interfaces and radios, the
    /// profiles of its setup script (mikrotik/hap-be-lite.rsc), the old
    /// passwords as the write account reads them.
    fn live() -> Value {
        let h = hall();
        let mut wifi = h["wifi"].clone();
        for r in wifi.as_array_mut().unwrap() {
            r["secret"] = json!(match s(r, "configuration.ssid") { "Home" => "old home pass", "IoT" => "old-iot-pass", "Guests" => "guest\"pass$1", _ => "" });
        }
        json!({
            "wifi": wifi,
            "cfg": [
                {".id": "*1", "name": "cfg-lan", "ssid": "Home", "security": "sec-lan", "datapath": "dp-lan"},
                {".id": "*2", "name": "cfg-iot", "ssid": "IoT", "security": "sec-iot", "datapath": "dp-iot"},
                {".id": "*3", "name": "cfg-guest", "ssid": "Guests", "security": "sec-guest", "datapath": "dp-guest"},
            ],
            "sec": [
                {".id": "*1", "name": "sec-lan", "authentication-types": "wpa2-psk,wpa3-psk", "secret": "old home pass"},
                {".id": "*2", "name": "sec-iot", "authentication-types": "wpa2-psk", "secret": "old-iot-pass"},
                {".id": "*3", "name": "sec-guest", "authentication-types": "wpa2-psk,wpa3-psk", "secret": "guest\"pass$1"},
            ],
            "dp": [
                {".id": "*1", "name": "dp-lan", "bridge": "bridge", "vlan-id": "10"},
                {".id": "*2", "name": "dp-iot", "bridge": "bridge", "vlan-id": "20"},
                {".id": "*3", "name": "dp-guest", "bridge": "bridge", "client-isolation": "true", "vlan-id": "30"},
            ],
            "radio": h["radio"],
        })
    }
    fn change(want: Value) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
        plan(&live(), &want).map(|(c, u, w)| (infra::show_all(&c), u, w))
    }

    #[test]
    fn legal_channels_of_a_real_radio() {
        let h = hall();
        let c5 = channels(s(&h["radio"][1], "current-channels"));
        assert_eq!(c5[&5180].keys().copied().collect::<Vec<_>>(), [20, 40, 80, 160]);
        assert!(!c5[&5180][&80] && c5[&5180][&160], "36 in 160 MHz reaches the radar channels");
        assert!(c5[&5260][&20], "52 is a radar channel");
        let c2 = channels(s(&h["radio"][0], "current-channels"));
        assert_eq!((c2.len(), c2[&2412].keys().copied().collect::<Vec<_>>()), (13, vec![20, 40]));
        assert_eq!((width_mhz("20/40/80mhz"), width_mhz("")), (Some(80), None));
        // the page gets them per radio
        let v = build(&[h], &[], &[]);
        let r = find(&v["aps"][0], "radios", "iface", "wifi2");
        assert_eq!((rows(r, "channels").len(), r["channels"][0]["ch"].as_u64(), s(r, "freq_cfg")), (29, Some(36), "5180"));
    }

    #[test]
    fn radio_settings_and_their_rollback() {
        let (c, u, w) = change(json!({"ifaces": [{"name": "wifi2", "frequency": "5200", "width": "20/40/80mhz", "tx_power": "20"}]})).unwrap();
        assert_eq!(c, ["/interface/wifi/set .id=*4 channel.frequency=5200 channel.width=20/40/80mhz configuration.tx-power=20"]);
        assert_eq!(u[0], ":do { /interface wifi set [find where .id=*4] channel.frequency=\"5180\" channel.width=\"20/40mhz\" } on-error={}");
        assert_eq!(u[1], ":do { /interface wifi unset [find where .id=*4] value-name=configuration.tx-power } on-error={}");
        assert!(u[2].starts_with(":log warning") && u[3].starts_with("/system scheduler remove"));
        assert_eq!(w, ["wifi2: its clients reconnect"]);
        // radar channel, automatic channel
        assert!(change(json!({"ifaces": [{"name": "wifi2", "frequency": "5260"}]})).unwrap().2[0].contains("radar (DFS) channel"));
        let (c, u, w) = change(json!({"ifaces": [{"name": "wifi2", "frequency": ""}]})).unwrap();
        assert_eq!(c, ["/interface/wifi/unset numbers=*4 value-name=channel.frequency"]);
        assert!(u[0].contains("channel.frequency=\"5180\"") && w[0].contains("automatic channel"));
        // unchanged: nothing at all
        assert_eq!(change(json!({"ifaces": [{"name": "wifi1", "frequency": "2412", "tx_power": ""}]})).unwrap(), (vec![], vec![], vec![]));
        // refused
        for (want, why) in [
            (json!({"name": "wifi2", "frequency": "5000"}), "not a legal channel"),
            (json!({"name": "wifi1", "width": "20/40/80mhz"}), "not available"),
            (json!({"name": "wifi1", "tx_power": "30"}), "1 to 20 dBm"),
            (json!({"name": "wifi1-iot", "frequency": "2437"}), "runs on wifi1's radio"),
            (json!({"name": "wifi9", "disabled": true}), "no such"),
        ] {
            let e = change(json!({"ifaces": [want]})).unwrap_err();
            assert!(e.contains(why), "{e}");
        }
    }

    #[test]
    fn ssids_on_and_off() {
        let e = change(json!({"ifaces": [{"name": "wifi1", "disabled": true}]})).unwrap_err();
        assert!(e.contains("wifi1 off stops wifi1-guest (Guests)"), "{e}");
        let (c, u, w) = change(json!({"ifaces": [{"name": "wifi2-guest", "disabled": true}]})).unwrap();
        assert_eq!(c, ["/interface/wifi/set .id=*E disabled=yes"]);
        assert_eq!(u[0], ":do { /interface wifi set [find where .id=*E] disabled=no } on-error={}");
        assert_eq!(w, ["wifi2-guest: off, its clients are cut"]);
        // the whole radio off: all its SSIDs together
        let all = ["wifi1", "wifi1-iot", "wifi1-guest"].map(|n| json!({"name": n, "disabled": true}));
        let (c, _, _) = change(json!({"ifaces": all})).unwrap();
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn isolation_and_password_where_they_live() {
        let (c, u, _) = change(json!({"ssids": [{"ssid": "Home", "isolation": true}]})).unwrap();
        assert_eq!(c, ["/interface/wifi/datapath/set .id=*1 client-isolation=yes"]);
        assert_eq!(u[0], ":do { /interface wifi datapath unset [find where .id=*1] value-name=client-isolation } on-error={}");
        let (c, u, w) = change(json!({"ssids": [{"ssid": "Guests", "isolation": false}]})).unwrap();
        assert_eq!((c[0].as_str(), u[0].as_str()), ("/interface/wifi/datapath/set .id=*3 client-isolation=no",
            ":do { /interface wifi datapath set [find where .id=*3] client-isolation=yes } on-error={}"));
        assert!(w[0].contains("see each other"));
        assert!(change(json!({"ssids": [{"ssid": "Guests", "isolation": true}]})).unwrap().0.is_empty());

        // the password goes to the one security profile; the rollback holds the old one, quoted
        let (c, u, w) = change(json!({"ssids": [{"ssid": "Guests", "passphrase": "new guest pass"}]})).unwrap();
        assert_eq!(c, ["/interface/wifi/security/set .id=*3 passphrase=\"•••\""]);
        assert_eq!(u[0], ":do { /interface wifi security set [find where .id=*3] passphrase=\"guest\\\"pass\\$1\" } on-error={}");
        assert_eq!(crate::direct::redact(&u[0]), ":do { /interface wifi security set [find where .id=*3] passphrase=\"•••\" } on-error={}");
        assert_eq!(w, ["Guests: every device on it needs the new password"]);
        assert!(change(json!({"ssids": [{"ssid": "Guests", "passphrase": "guest\"pass$1"}]})).unwrap().0.is_empty(), "same password");
        assert!(change(json!({"ssids": [{"ssid": "Guests", "passphrase": "short"}]})).unwrap_err().contains("8 to 63"));
        assert!(change(json!({"ssids": [{"ssid": "Nope", "isolation": true}]})).unwrap_err().contains("not on this access point"));

        // a profile two SSIDs share is not changed for one of them
        let mut l = live();
        l["cfg"][1]["security"] = json!("sec-lan");
        let e = plan(&l, &json!({"ssids": [{"ssid": "Home", "passphrase": "new home pass"}]})).unwrap_err();
        assert!(e.contains("sec-lan also serves IoT"), "{e}");
        // no profile: written on each interface of the SSID
        let mut l = live();
        l["cfg"][2].as_object_mut().unwrap().remove("security");
        let (c, u, _) = plan(&l, &json!({"ssids": [{"ssid": "Guests", "passphrase": "new guest pass"}]})).unwrap();
        assert_eq!(infra::show_all(&c), ["/interface/wifi/set .id=*C security.passphrase=\"•••\"", "/interface/wifi/set .id=*E security.passphrase=\"•••\""]);
        assert!(u[1].contains("[find where .id=*E] security.passphrase=\"guest\\\"pass\\$1\""));
        // an open network has no password
        let mut l = live();
        for r in l["wifi"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("security.authentication-types");
        }
        assert!(plan(&l, &json!({"ssids": [{"ssid": "IoT", "passphrase": "new iot pass"}]})).unwrap_err().contains("open"));
    }

    #[test]
    fn read_back_and_rollback_job() {
        let want = json!({"ifaces": [{"name": "wifi2", "frequency": "5200"}], "ssids": [{"ssid": "Guests", "passphrase": "new guest pass", "isolation": true}]});
        let mut after = live();
        assert!(!matches(&after, &want));
        for r in after["wifi"].as_array_mut().unwrap() {
            if s(r, "name") == "wifi2" {
                r["channel.frequency"] = json!("5200");
            }
            if s(r, "configuration.ssid") == "Guests" {
                r["secret"] = json!("new guest pass");
            }
        }
        assert!(matches(&after, &want));

        // where there is no scheduler: the job, without the scheduler line; old password masked when shown
        let (c, u, _) = plan(&live(), &want).unwrap();
        let job = infra::arm_job(&u);
        assert_eq!(job.0, "/execute");
        assert!(job.1[0].1.contains("passphrase=\"guest\\\"pass\\$1\"") && !job.1[0].1.contains("scheduler"));
        let shown = crate::direct::show(&job);
        assert!(!shown.contains("guest") && shown.contains("passphrase=\"•••\""), "{shown}");
        assert!(infra::show_all(&c).iter().all(|l| !l.contains("new guest")));
    }
}
