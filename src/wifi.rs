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
//!
//! Lot Wi-Fi 3: Cairn Net is the controller. The Wi-Fi state (data/wifi.json,
//! 0600: it holds the Wi-Fi passwords) has the SSIDs common to every access
//! point and the radio settings of each; `plan_state` brings one access point
//! to it (SSIDs created, changed, turned off or removed; VLANs on its uplink,
//! checked on the switch across), `watch` reports drift. A blank access point
//! gets a first-setup script without any secret (`bootstrap`), then `adopt`
//! logs in with the app's SSH key, gives the write account a password only
//! Cairn Net knows and puts the access point in direct mode.
//!
//! Lot Wi-Fi 4a: roaming settings per SSID in the state (802.11r fast
//! transition, 802.11k/v neighbour reports and transition requests, 2.4 GHz
//! probe delay to steer dual-band clients to 5 GHz); devices blocked on every
//! access point (access list rows of their own, blocked clients still
//! connected disconnected by the same apply); a client disconnected now
//! (`kick`); an SSID shared as a QR code with its password, or given a new
//! random one (`share`, op `rotate`).
//!
//! Lot Wi-Fi 4b: an SSID of the state may have a schedule {days, from, to},
//! by each access point's own clock (`/system/clock`). Outside it the SSID is
//! off (`effective`, so the card and drift agree), and `schedules` turns its
//! interfaces on or off every minute on access points in direct mode: their
//! `disabled` switch only, guarded apply kept by Cairn Net once read back.

use crate::direct::Cmd;
use crate::{infra, settings, Api};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::process::Command;
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
    profiles(api, &mut v)?; // the drift against the Wi-Fi state (lot Wi-Fi 3)
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
                "enabled": !yes(s(x, "disabled")), "ft": yes(s(x, "security.ft")), "kv": yes(s(x, "steering.rrm")) && yes(s(x, "steering.wnm")),
                "band_steering": yes(s(x, "steering.2g-probe-delay")),
                "clients": rows(ap, "reg").iter().filter(|c| s(c, "interface") == s(x, "name")).count(),
            }));
        }
    }
    let ssids: Vec<Value> = by_ssid.into_iter().map(|(ssid, on)| {
        let mut v = json!({"ssid": ssid, "clients": on.iter().filter_map(|e| e["clients"].as_u64()).sum::<u64>()});
        let mut drift = vec![];
        for f in ["vlan", "security", "isolation", "hidden", "enabled", "ft", "kv", "band_steering"] {
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
            "has_write": a["has_write"] == true, "plan": a["plan"], "plan_warn": a["plan_warn"], "plan_error": a["plan_error"],
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
                a["has_write"] = json!(!c.write_user.is_empty());
            }
        }
        // what applying the Wi-Fi state would change (passwords aside: the read account cannot see them)
        let st = state();
        for a in aps.iter_mut().filter(|a| a["error"].is_null()) {
            let (Some(st), name) = (st.as_ref(), s(a, "name").to_string()) else { continue };
            match plan_state(a, st, &name, uplink(&slow, &name).as_ref()) {
                Ok((c, _, w)) => {
                    a["plan"] = json!(infra::show_all(&c));
                    a["plan_warn"] = json!(w);
                }
                Err(e) => a["plan_error"] = json!(e),
            }
        }
        let leases: Vec<Value> = sws.iter().flat_map(|w| rows(w, "leases").iter().cloned()).collect();
        let mut v = build(&aps, rows(&slow["topology"], "devices"), &leases);
        v["t"] = json!(crate::now_ms());
        v["rollback_s"] = json!(infra::ROLLBACK_S);
        v["state"] = st.as_ref().map_or(Value::Null, public_state);
        v["app"] = json!(app_ip()); // the address an adopted access point lets in
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
        "radio": crate::print(api, "/interface/wifi/radio", "interface,bands,current-channels,current-max-reg-power", &[])?,
    });
    profiles(api, &mut v)?;
    v["reg"] = json!(crate::print(api, "/interface/wifi/registration-table", ".id,mac-address,interface", &[])?); // blocked clients still on
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

/// Profiles, bridge ports and bridge VLANs of an access point: where its
/// SSIDs' settings live and what its uplink carries.
fn profiles(api: &mut Api, v: &mut Value) -> Result<(), String> {
    v["cfg"] = json!(crate::chip::all(api, "/interface/wifi/configuration", &[])?);
    v["sec"] = json!(crate::print(api, "/interface/wifi/security", ".id,name,authentication-types,ft", &[])?);
    v["dp"] = json!(crate::chip::all(api, "/interface/wifi/datapath", &[])?);
    v["steer"] = json!(crate::chip::all(api, "/interface/wifi/steering", &[])?);
    v["acl"] = json!(crate::chip::all(api, "/interface/wifi/access-list", &[])?);
    // the access point's own clock (its time zone): SSID schedules follow it (lot Wi-Fi 4b)
    v["clock"] = crate::print(api, "/system/clock", "date,time", &[])?.pop().unwrap_or(Value::Null);
    v["ports"] = json!(crate::print(api, "/interface/bridge/port", "bridge,interface,pvid", &[])?);
    v["bvlan"] = json!(crate::print(api, "/interface/bridge/vlan", "bridge,vlan-ids,current-tagged,current-untagged", &[])?);
    Ok(())
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

/// SSID settings and where they live: field of `want`, profile kind and its
/// table in `conf`, property (on an interface: "<kind>.<property>").
const SSID_FIELDS: [(&str, &str, &str, &str); 8] = [
    ("isolation", "datapath", "dp", "client-isolation"),
    ("vlan", "datapath", "dp", "vlan-id"),
    ("hidden", "configuration", "cfg", "hide-ssid"),
    ("security", "security", "sec", "authentication-types"),
    // lot Wi-Fi 4a: roaming
    ("ft", "security", "sec", "ft"),
    ("rrm", "steering", "steer", "rrm"),
    ("wnm", "steering", "steer", "wnm"),
    ("band_steering", "steering", "steer", "2g-probe-delay"),
];
const FLAGS: [&str; 6] = ["isolation", "hidden", "ft", "rrm", "wnm", "band_steering"];

/// A value as compared and written: booleans yes/no, authentication types sorted.
fn norm(field: &str, v: &str) -> String {
    match field {
        f if FLAGS.contains(&f) => yn(yes(v)),
        "security" => {
            let mut a: Vec<&str> = v.split(',').map(str::trim).filter(|x| !x.is_empty()).collect();
            a.sort_unstable();
            a.join(",")
        }
        _ => v.trim().to_string(),
    }
}

/// The commands, the rollback lines (without the closing lines of
/// infra::finish) and the warnings of a change on one access point. `want`:
/// {"ifaces": [{name, frequency?, width?, tx_power?, disabled?}], "ssids":
/// [{ssid, isolation?, vlan?, hidden?, security?, passphrase?}]}; a field
/// left out is not changed, "" = automatic or none (the value is unset).
fn changes(live: &Value, want: &Value) -> Result<(Vec<Cmd>, Vec<String>, Vec<String>), String> {
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
        for (f, kind, table, prop) in SSID_FIELDS {
            let new = match w.get(f) {
                Some(Value::Bool(b)) => yn(*b),
                Some(Value::String(x)) => norm(f, x),
                _ => continue,
            };
            let on_iface = format!("{kind}.{prop}");
            if on.iter().all(|r| norm(f, s(r, &on_iface)) == new) {
                continue;
            }
            // a value back as a script writes it: bare yes/no and numbers, quoted strings
            let back = |old: &str| match f {
                "vlan" => old.to_string(),
                f if FLAGS.contains(&f) => yn(yes(old)),
                _ => infra::q(old),
            };
            for (p, ifs) in holders(live, &on, kind, table)? {
                let (path, menu, targets): (String, String, Vec<(&str, &str)>) = match p {
                    Some(p) => (format!("/interface/wifi/{kind}"), format!("/interface wifi {kind}"), vec![(s(p, ".id"), s(p, prop))]),
                    None => ("/interface/wifi".into(), "/interface wifi".into(), ifs.iter().map(|r| (s(r, ".id"), s(r, &on_iface))).collect()),
                };
                let key = if p.is_some() { prop.to_string() } else { on_iface.clone() };
                for (id, old) in targets {
                    cmds.push(if new.is_empty() {
                        (format!("{path}/unset"), vec![kv("numbers", id), kv("value-name", &key)])
                    } else {
                        (format!("{path}/set"), vec![kv(".id", id), kv(&key, &new)])
                    });
                    undo.push(guarded(if old.is_empty() {
                        format!("{menu} unset [find where .id={id}] value-name={key}")
                    } else {
                        format!("{menu} set [find where .id={id}] {key}={}", back(old))
                    }));
                }
            }
            match f {
                "isolation" if new == "no" => warn.push(format!("{ssid}: its clients see each other again")),
                "vlan" => warn.push(format!("{ssid}: its clients reconnect in another VLAN (new address)")),
                "security" => warn.push(format!("{ssid}: some devices must join it again")),
                "ft" if new == "yes" => warn.push(format!("{ssid}: some older devices cannot join with 802.11r: check them after applying")),
                _ => {}
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
    Ok((cmds, undo, warn))
}

/// `changes` with the closing rollback lines (lot Wi-Fi 2's engine, tested on its own).
#[cfg(test)]
fn plan(live: &Value, want: &Value) -> Result<(Vec<Cmd>, Vec<String>, Vec<String>), String> {
    let (cmds, mut undo, warn) = changes(live, want)?;
    if !cmds.is_empty() {
        infra::finish(&mut undo);
    }
    Ok((cmds, undo, warn))
}

/* ---------- the Wi-Fi state, common to every access point (lot Wi-Fi 3) ---------- */

const NO_STATE: &str = "no Wi-Fi state yet: take it from an access point first";
/// Comment of a bridge VLAN Cairn Net adds on an access point's uplink (then its id).
const VLAN_TAG: &str = "cairn-net: Wi-Fi VLAN";
const BANDS: [&str; 3] = ["2.4", "5", "6"];
/// The write account of an adopted access point.
const WRITE_USER: &str = "cairn-write";
static LOCK: Mutex<()> = Mutex::new(());

fn state_path() -> String {
    format!("{}/wifi.json", crate::git::data())
}

/// data/wifi.json: {"ssids": [{ssid, vlan, security, passphrase, isolation,
/// hidden, bands, ft, kv, band_steering, schedule {days, from, to}}],
/// "radios": {access point: {interface: {frequency, width,
/// tx_power}}}}; none until taken from an access point.
pub fn state() -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(state_path()).ok()?).ok()
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).collect()
}

fn check_state(v: &Value) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for x in rows(v, "ssids") {
        let n = s(x, "ssid");
        if n.is_empty() || n.len() > 32 || n.chars().any(char::is_control) {
            return Err(format!("SSID {n:?}: 1 to 32 bytes"));
        }
        if !seen.insert(n) {
            return Err(format!("{n}: twice in the list"));
        }
        let vlan = s(x, "vlan");
        if !vlan.is_empty() && int(vlan).is_none_or(|v| !(1..=4094).contains(&v)) {
            return Err(format!("{n}: VLAN 1 to 4094, or none"));
        }
        if !s(x, "security").chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ',') {
            return Err(format!("{n}: unknown security"));
        }
        if !s(x, "passphrase").is_empty() {
            passphrase_ok(s(x, "passphrase")).map_err(|e| format!("{n}: {e}"))?;
        }
        if strs(&x["bands"]).iter().any(|b| !BANDS.contains(b)) {
            return Err(format!("{n}: bands are 2.4, 5 and 6"));
        }
        if x["ft"] == true && s(x, "security").is_empty() {
            return Err(format!("{n}: 802.11r needs a password (WPA2 or WPA3)"));
        }
        let w = &x["schedule"];
        if !w.is_null() {
            let days = w["days"].as_array().map(|d| d.iter().map(Value::as_u64).collect::<Vec<_>>());
            if !w.is_object() || hm(s(w, "from")).is_none() || hm(s(w, "to")).is_none()
                || days.is_none_or(|d| d.is_empty() || d.iter().any(|d| d.is_none_or(|d| !(1..=7).contains(&d)))) {
                return Err(format!("{n}: a schedule is days 1 (Monday) to 7, from and to as HH:MM"));
            }
        }
    }
    let mut macs = BTreeSet::new();
    for b in rows(v, "blocked") {
        let m = s(b, "mac");
        if !is_mac(m) {
            return Err(format!("{m}: not a MAC address (AA:BB:CC:DD:EE:FF)"));
        }
        if !macs.insert(m) {
            return Err(format!("{m}: blocked twice"));
        }
    }
    Ok(())
}

/// "AA:BB:CC:DD:EE:FF", upper case: as RouterOS prints them, and safe bare in a script.
fn is_mac(m: &str) -> bool {
    m.len() == 17 && m.split(':').all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)))
}

fn write_state(v: &Value) -> Result<(), String> {
    check_state(v)?;
    settings::write_private(&state_path(), &serde_json::to_string_pretty(v).unwrap_or_default())
}

/// What the page sees of the state: whether an SSID has a password, never the password.
fn public_state(st: &Value) -> Value {
    let mut v = st.clone();
    for x in v["ssids"].as_array_mut().into_iter().flatten() {
        if let Some(o) = x.as_object_mut() {
            let has = o.remove("passphrase").is_some_and(|p| p.as_str().is_some_and(|p| !p.is_empty()));
            o.insert("has_passphrase".into(), json!(has));
        }
    }
    v
}

fn station(r: &Value) -> bool {
    s(r, "configuration.mode").starts_with("station")
}

/// The band of a radio: its `bands`, else its legal channels.
fn radio_band(live: &Value, iface: &str) -> &'static str {
    let r = rows(live, "radio").iter().find(|x| s(x, "interface") == iface);
    let b = r.map_or("", |x| s(x, "bands"));
    if let Some(i) = ["2ghz", "5ghz", "6ghz"].iter().position(|k| b.contains(k)) {
        return BANDS[i];
    }
    r.and_then(|x| channels(s(x, "current-channels")).keys().next().copied()).map_or("", band_of)
}

/// The radios of an access point (interfaces with a radio, stations aside) and their band.
fn radios(live: &Value) -> Vec<(&str, &'static str)> {
    rows(live, "wifi").iter().filter(|r| !s(r, "radio-mac").is_empty() && !station(r)).map(|r| (s(r, "name"), radio_band(live, s(r, "name")))).collect()
}

/// The state as one access point runs it (`conf`; its passwords with the
/// write account, else those `old` had): SSIDs of the radios' own interfaces
/// first, as they come first on a new access point, with the bands where
/// each is on; this access point's radio settings, the others' kept.
fn import(live: &Value, ap: &str, old: Option<&Value>) -> Value {
    let wifi = rows(live, "wifi");
    let mut on: Vec<&Value> = wifi.iter().filter(|r| !station(r) && !s(r, "configuration.ssid").is_empty()).collect();
    on.sort_by_key(|r| s(r, "radio-mac").is_empty());
    let mut ssids: Vec<Value> = vec![];
    for r in on {
        let n = s(r, "configuration.ssid");
        let i = match ssids.iter().position(|x| s(x, "ssid") == n) {
            Some(i) => i,
            None => {
                let sec = norm("security", s(r, "security.authentication-types"));
                let was = old.and_then(|o| rows(o, "ssids").iter().find(|x| s(x, "ssid") == n)).map_or("", |x| s(x, "passphrase"));
                let pw = r.get("secret").and_then(Value::as_str).filter(|p| !p.is_empty()).unwrap_or(was);
                ssids.push(json!({"ssid": n, "vlan": s(r, "datapath.vlan-id"), "security": sec, "passphrase": if sec.is_empty() { "" } else { pw },
                    "isolation": yes(s(r, "datapath.client-isolation")), "hidden": yes(s(r, "configuration.hide-ssid")), "bands": [],
                    "ft": !sec.is_empty() && yes(s(r, "security.ft")), "kv": yes(s(r, "steering.rrm")) && yes(s(r, "steering.wnm")),
                    "band_steering": yes(s(r, "steering.2g-probe-delay"))}));
                ssids.len() - 1
            }
        };
        let b = radio_band(live, &radio_of(wifi, s(r, "name")));
        let mut bands: Vec<String> = strs(&ssids[i]["bands"]).into_iter().map(String::from).collect();
        if !yes(s(r, "disabled")) && !b.is_empty() && !bands.iter().any(|x| x == b) {
            bands.push(b.into());
            bands.sort_by_key(|x| BANDS.iter().position(|y| y == x));
            ssids[i]["bands"] = json!(bands);
        }
    }
    // a schedule stays; an SSID off by its schedule now keeps its bands
    let now = clock_of(live);
    for x in &mut ssids {
        if let Some(o) = old.and_then(|o| rows(o, "ssids").iter().find(|o| s(o, "ssid") == s(x, "ssid") && o["schedule"].is_object())) {
            x["schedule"] = o["schedule"].clone();
            if !on_air(o, now) {
                x["bands"] = o["bands"].clone();
            }
        }
    }
    let mut radios = old.map(|o| o["radios"].clone()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    radios[ap] = Value::Object(wifi.iter().filter(|r| !s(r, "radio-mac").is_empty() && !station(r)).map(|r| (s(r, "name").to_string(),
        json!({"frequency": s(r, "channel.frequency"), "width": s(r, "channel.width"), "tx_power": s(r, "configuration.tx-power")}))).collect());
    // the rest of the old state (blocked devices) stays
    let mut st = old.cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    st["ssids"] = json!(ssids);
    st["radios"] = radios;
    st
}

/// What carries VLANs between an access point and the network: its uplink
/// port, and the port of the switch across with that switch's bridge VLANs.
#[derive(Default)]
pub struct Uplink {
    port: String,
    sw: String,
    sw_port: String,
    sw_vlans: Vec<Value>,
}

/// The uplink of an access point in /api/slow's topology (its first link).
fn uplink(slow: &Value, ap: &str) -> Option<Uplink> {
    let l = rows(&slow["topology"], "links").iter().find(|l| s(l, "a") == ap || s(l, "b") == ap)?;
    let (port, sw, sw_port) = if s(l, "a") == ap { (s(l, "a_via"), s(l, "b"), s(l, "b_via")) } else { (s(l, "b_via"), s(l, "a"), s(l, "a_via")) };
    let sw_vlans = rows(slow, "switches").iter().find(|w| s(w, "name") == sw).map_or(vec![], |w| rows(w, "vlans").to_vec());
    Some(Uplink { port: port.into(), sw: sw.into(), sw_port: sw_port.into(), sw_vlans })
}

/// "10,30,100-102" -> 10, 30, 100, 101, 102.
fn vlan_ids(v: &str) -> Vec<u32> {
    v.split(',').flat_map(|p| match p.trim().split_once('-') {
        Some((a, b)) => match (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
            (Ok(a), Ok(b)) if a <= b && b <= 4094 => (a..=b).collect(),
            _ => vec![],
        },
        None => p.trim().parse().ok().into_iter().collect(),
    }).collect()
}

/// Whether bridge VLAN rows carry `vlan` on `port`, tagged or not.
fn carries(vlans: &[Value], vlan: u32, port: &str) -> bool {
    vlans.iter().any(|r| vlan_ids(s(r, "vlan-ids")).contains(&vlan)
        && [s(r, "current-tagged"), s(r, "current-untagged")].iter().any(|l| l.split(',').any(|p| p.trim() == port)))
}

/// The port that carries an access point to the network: the topology's,
/// else its one bridge port that is no Wi-Fi interface.
fn uplink_port(live: &Value, up: Option<&Uplink>) -> Option<String> {
    if let Some(p) = up.map(|u| u.port.as_str()).filter(|p| !p.is_empty()) {
        return Some(p.into());
    }
    let wifi: BTreeSet<&str> = rows(live, "wifi").iter().map(|r| s(r, "name")).collect();
    let wired: Vec<&str> = rows(live, "ports").iter().map(|p| s(p, "interface")).filter(|i| !wifi.contains(i)).collect();
    (wired.len() == 1).then(|| wired[0].to_string())
}

/// The bridge new SSIDs join: the uplink's, else the one the others use.
fn bridge_of(live: &Value, port: Option<&str>) -> String {
    port.and_then(|p| rows(live, "ports").iter().find(|r| s(r, "interface") == p)).map(|r| s(r, "bridge")).filter(|b| !b.is_empty())
        .or_else(|| rows(live, "wifi").iter().map(|r| s(r, "datapath.bridge")).find(|b| !b.is_empty()))
        .unwrap_or("bridge").to_string()
}

/// An interface name from an SSID: "Guests (5G)" -> "guests-5g".
fn slug(ssid: &str) -> String {
    let mut o = String::new();
    for c in ssid.chars().flat_map(char::to_lowercase) {
        if o.len() >= 16 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.is_empty() && !o.ends_with('-') {
            o.push('-');
        }
    }
    match o.trim_end_matches('-') {
        "" => "ssid".into(),
        x => x.into(),
    }
}

/// A value in a rollback line: bare yes/no and numbers, quoted strings.
fn script_value(k: &str, v: &str) -> String {
    if ["isolation", "hide-ssid", ".ft", "rrm", "wnm", "2g-probe-delay"].iter().any(|x| k.ends_with(x)) || k == "disabled" {
        yn(yes(v))
    } else if k.ends_with("vlan-id") && v.chars().all(|c| c.is_ascii_digit()) {
        v.into()
    } else {
        infra::q(v)
    }
}

/// An SSID of the state where a radio lacks it: on the radio's own interface
/// when that one carries no SSID of the state (claimed once), else as a new
/// secondary interface; every setting written on the interface itself.
fn create(live: &Value, x: &Value, radio: &str, bridge: &str, known: &BTreeSet<&str>, taken: &mut BTreeSet<String>, claimed: &mut BTreeSet<String>) -> Result<(Cmd, Vec<String>), String> {
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    let m = rows(live, "wifi").iter().find(|r| s(r, "name") == radio).ok_or_else(|| format!("{radio}: no such Wi-Fi interface"))?;
    let (name, sec, pw, vlan) = (s(x, "ssid"), s(x, "security"), s(x, "passphrase"), s(x, "vlan"));
    if !sec.is_empty() && pw.is_empty() {
        return Err(format!("{name}: no password for it in the Wi-Fi state: set one"));
    }
    let mut props = vec![("configuration.ssid", name.to_string()), ("configuration.hide-ssid", yn(x["hidden"] == true)),
        ("datapath.bridge", bridge.to_string()), ("datapath.client-isolation", yn(x["isolation"] == true))];
    if !vlan.is_empty() {
        props.push(("datapath.vlan-id", vlan.to_string()));
    }
    if !sec.is_empty() {
        props.push(("security.authentication-types", sec.to_string()));
        props.push(("security.passphrase", pw.to_string()));
        if x["ft"] == true {
            props.push(("security.ft", "yes".into()));
        }
    }
    for (k, on) in [("steering.rrm", x["kv"] == true), ("steering.wnm", x["kv"] == true), ("steering.2g-probe-delay", x["band_steering"] == true)] {
        if on {
            props.push((k, "yes".into()));
        }
    }
    let mine = s(m, "configuration.ssid");
    if (mine.is_empty() || !known.contains(mine)) && claimed.insert(radio.to_string()) {
        let id = s(m, ".id");
        let mut set = vec![kv(".id", id)];
        let mut undo = vec![];
        for (k, v) in &props {
            set.push(kv(k, v));
            let old = if *k == "security.passphrase" { m.get("secret").and_then(Value::as_str).unwrap_or("") } else { s(m, k) };
            undo.push(guarded(if old.is_empty() {
                format!("/interface wifi unset [find where .id={id}] value-name={k}")
            } else {
                format!("/interface wifi set [find where .id={id}] {k}={}", script_value(k, old))
            }));
        }
        set.push(kv("disabled", "no"));
        undo.push(guarded(format!("/interface wifi set [find where .id={id}] disabled={}", yn(yes(s(m, "disabled"))))));
        return Ok((("/interface/wifi/set".into(), set), undo));
    }
    let base = format!("{radio}-{}", slug(name));
    let (mut n, mut i) = (base.clone(), 1);
    while taken.contains(&n) {
        i += 1;
        n = format!("{base}-{i}");
    }
    taken.insert(n.clone());
    let mut add = vec![kv("name", &n), kv("master-interface", radio)];
    if !s(m, "configuration.country").is_empty() {
        add.push(kv("configuration.country", s(m, "configuration.country")));
    }
    add.extend(props.iter().map(|(k, v)| kv(k, v)));
    add.push(kv("disabled", "no"));
    Ok((("/interface/wifi/add".into(), add), vec![guarded(format!("/interface wifi remove [find where name={}]", infra::q(&n)))]))
}

/// The rollback line that brings a removed secondary interface back.
fn readd(r: &Value) -> String {
    let mut w = vec![format!("name={}", infra::q(s(r, "name"))), format!("master-interface={}", infra::q(s(r, "master-interface"))),
        format!("disabled={}", yn(yes(s(r, "disabled"))))];
    if !s(r, "configuration").is_empty() {
        w.push(format!("configuration={}", infra::q(s(r, "configuration"))));
    } else {
        for k in ["configuration.ssid", "configuration.country", "configuration.hide-ssid", "security.authentication-types", "security.ft", "datapath.bridge", "datapath.vlan-id",
            "datapath.client-isolation", "steering.rrm", "steering.wnm", "steering.2g-probe-delay"] {
            if !s(r, k).is_empty() {
                w.push(format!("{k}={}", script_value(k, s(r, k))));
            }
        }
        if let Some(p) = r.get("secret").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            w.push(format!("security.passphrase={}", infra::q(p)));
        }
    }
    guarded(format!("/interface wifi add {}", w.join(" ")))
}

fn put(m: &mut BTreeMap<String, Value>, n: &str, k: &str, v: Value) {
    m.entry(n.to_string()).or_insert_with(|| json!({"name": n}))[k] = v;
}

/// What brings one access point to the Wi-Fi state: its radio settings;
/// every SSID of the state on the radios of its bands (created where
/// missing, turned off on the other bands) with the state's settings; SSIDs
/// the state does not have removed (a radio's own interface is turned off
/// instead); their VLANs on the access point's uplink, where a VLAN new to
/// it must already pass through the switch port across. Passwords are
/// compared only when `live` holds them (write account).
pub fn plan_state(live: &Value, st: &Value, ap: &str, up: Option<&Uplink>) -> Result<(Vec<Cmd>, Vec<String>, Vec<String>), String> {
    // an SSID outside its schedule is off there now: the card and drift do not fight the schedule
    let eff = effective(st, clock_of(live));
    let st = &eff;
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    let wifi = rows(live, "wifi");
    let ssids = rows(st, "ssids");
    if ssids.is_empty() {
        return Err("the Wi-Fi state has no SSID".into());
    }
    let secrets = wifi.iter().any(|r| r.get("secret").is_some());
    let radios = radios(live);
    let known: BTreeSet<&str> = ssids.iter().map(|x| s(x, "ssid")).collect();
    let port = uplink_port(live, up);
    let bridge = bridge_of(live, port.as_deref());
    let mut ifaces: BTreeMap<String, Value> = BTreeMap::new();
    if let Some(rs) = st["radios"][ap].as_object() {
        for (n, w) in rs.iter().filter(|(n, _)| radios.iter().any(|r| r.0 == n.as_str())) {
            for k in ["frequency", "width", "tx_power"] {
                if let Some(v) = w[k].as_str() {
                    put(&mut ifaces, n, k, json!(v));
                }
            }
        }
    }
    let (mut want, mut adds, mut add_undo, mut dels, mut del_undo, mut warn) = (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut taken: BTreeSet<String> = wifi.iter().map(|r| s(r, "name").to_string()).collect();
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    for x in ssids {
        let name = s(x, "ssid");
        let bands = strs(&x["bands"]);
        let on: Vec<&Value> = wifi.iter().filter(|r| s(r, "configuration.ssid") == name && !station(r)).collect();
        for (radio, band) in &radios {
            let wanted = bands.contains(band);
            let here: Vec<&&Value> = on.iter().filter(|r| radio_of(wifi, s(r, "name")) == *radio).collect();
            for r in &here {
                if yes(s(r, "disabled")) == wanted {
                    put(&mut ifaces, s(r, "name"), "disabled", json!(!wanted));
                }
            }
            if wanted && here.is_empty() {
                let (c, u) = create(live, x, radio, &bridge, &known, &mut taken, &mut claimed)?;
                adds.push(c);
                add_undo.extend(u);
            }
        }
        if !on.is_empty() {
            let kv = x["kv"] == true;
            let mut w = json!({"ssid": name, "isolation": x["isolation"] == true, "hidden": x["hidden"] == true, "vlan": s(x, "vlan"), "security": s(x, "security"),
                "rrm": kv, "wnm": kv, "band_steering": x["band_steering"] == true});
            if !s(x, "security").is_empty() {
                w["ft"] = json!(x["ft"] == true); // an open SSID has no 802.11r to compare
            }
            if secrets && !s(x, "security").is_empty() && !s(x, "passphrase").is_empty() {
                w["passphrase"] = json!(s(x, "passphrase"));
            }
            want.push(w);
        }
    }
    for r in wifi.iter().filter(|r| !station(r) && !s(r, "configuration.ssid").is_empty() && !known.contains(s(r, "configuration.ssid"))) {
        let n = s(r, "name");
        if !s(r, "radio-mac").is_empty() {
            if !claimed.contains(n) && !yes(s(r, "disabled")) {
                put(&mut ifaces, n, "disabled", json!(true));
            }
            continue;
        }
        dels.push(("/interface/wifi/remove".to_string(), vec![kv(".id", s(r, ".id"))]));
        del_undo.push(readd(r));
        warn.push(format!("{}: removed from {n}, its clients are cut", s(r, "configuration.ssid")));
    }

    // the VLANs of the SSIDs this access point carries, on its uplink
    let bands_here: BTreeSet<&str> = radios.iter().map(|r| r.1).collect();
    let used: BTreeSet<u32> = wifi.iter().filter_map(|r| s(r, "datapath.vlan-id").parse().ok()).collect();
    let vlans: BTreeSet<u32> = ssids.iter().filter(|x| strs(&x["bands"]).iter().any(|b| bands_here.contains(b))).filter_map(|x| s(x, "vlan").parse().ok()).collect();
    let (mut vcmds, mut vundo) = (vec![], vec![]);
    for v in vlans {
        let fresh = !used.contains(&v);
        match &port {
            Some(p) if !carries(rows(live, "bvlan"), v, p) => {
                let tag = format!("{VLAN_TAG} {v}");
                vcmds.push(("/interface/bridge/vlan/add".to_string(), vec![kv("bridge", &bridge), kv("vlan-ids", &v.to_string()), kv("tagged", p), kv("comment", &tag)]));
                vundo.push(guarded(format!("/interface bridge vlan remove [find where comment={}]", infra::q(&tag))));
            }
            Some(_) => {}
            None if fresh => return Err(format!("{ap}: no uplink found to carry VLAN {v}")),
            None => {}
        }
        if fresh {
            match up.filter(|u| !u.sw.is_empty()) {
                Some(u) if !carries(&u.sw_vlans, v, &u.sw_port) => {
                    return Err(format!("VLAN {v} does not pass through {} {}: add it there first", u.sw, u.sw_port));
                }
                Some(_) => {}
                None => warn.push(format!("{ap}: the switch across its uplink is unknown, VLAN {v} not checked there")),
            }
        }
    }

    let (cmds, undo, w) = changes(live, &json!({"ifaces": ifaces.into_values().collect::<Vec<_>>(), "ssids": want}))?;
    warn.extend(w);
    let (acmds, aundo) = blocks(live, st, &mut warn);
    let mut all = vcmds;
    all.extend(cmds);
    all.extend(dels);
    all.extend(adds);
    all.extend(acmds);
    let mut back = aundo;
    back.extend(add_undo);
    back.extend(del_undo);
    back.extend(undo);
    back.extend(vundo);
    if !all.is_empty() {
        infra::finish(&mut back);
    }
    Ok((all, back, warn))
}

/// Comment of an access list row Cairn Net adds for a blocked device.
const BLOCK_TAG: &str = "cairn-net: blocked";

/// The devices the state blocks, on one access point: a reject row of the
/// access list each (first in the list), rows of devices no longer blocked
/// removed, blocked devices still connected disconnected (not undone: they
/// come back with the rollback). Other rows of the list are left as they are.
fn blocks(live: &Value, st: &Value, warn: &mut Vec<String>) -> (Vec<Cmd>, Vec<String>) {
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    let blocked: Vec<&str> = rows(st, "blocked").iter().map(|b| s(b, "mac")).filter(|m| is_mac(m)).collect();
    let acl = rows(live, "acl");
    let ours = |r: &&Value| s(r, "comment") == BLOCK_TAG;
    let mac = |r: &Value| s(r, "mac-address").to_uppercase();
    let (mut cmds, mut undo) = (vec![], vec![]);
    for r in acl.iter().filter(ours).filter(|r| !blocked.contains(&mac(r).as_str())) {
        cmds.push(("/interface/wifi/access-list/remove".to_string(), vec![kv(".id", s(r, ".id"))]));
        undo.push(guarded(format!("/interface wifi access-list add mac-address={} action=reject comment={}", mac(r), infra::q(BLOCK_TAG))));
    }
    // ponytail: a row of ours edited on the device (disabled, accept) counts as there; Apply does not fix it
    let first = acl.first().map(|r| s(r, ".id"));
    for m in blocked.iter().filter(|m| !acl.iter().filter(ours).any(|r| mac(r) == **m)) {
        let mut add = vec![kv("mac-address", m), kv("action", "reject"), kv("comment", BLOCK_TAG)];
        if let Some(f) = first {
            add.push(kv("place-before", f));
        }
        cmds.push(("/interface/wifi/access-list/add".into(), add));
        undo.push(guarded(format!("/interface wifi access-list remove [find where comment={} and mac-address={m}]", infra::q(BLOCK_TAG))));
    }
    for c in rows(live, "reg").iter().filter(|c| blocked.contains(&s(c, "mac-address").to_uppercase().as_str())) {
        cmds.push(("/interface/wifi/registration-table/remove".into(), vec![kv(".id", s(c, ".id"))]));
        warn.push(format!("{}: blocked, disconnected from {}", s(c, "mac-address").to_uppercase(), s(c, "interface")));
    }
    (cmds, undo)
}

/* ---------- SSID schedules, run by Cairn Net (lot Wi-Fi 4b) ---------- */

/// "07:30" -> 450, minutes from midnight.
fn hm(v: &str) -> Option<u32> {
    let (h, m) = v.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (v.len() == 5 && h < 24 && m < 60).then_some(h * 60 + m)
}

/// Day of the week of a date, 1 = Monday to 7 = Sunday (days from 1970-01-01, a Thursday).
fn weekday(y: i64, m: u32, d: u32) -> u32 {
    let y = if m <= 2 { y - 1 } else { y };
    let (era, mp) = (y.div_euclid(400), (m as i64 + 9) % 12);
    let yoe = y - era * 400;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d as i64 - 1;
    ((era * 146_097 + doe - 719_468 + 3).rem_euclid(7) + 1) as u32
}

/// The access point's clock (`profiles`): day of the week and minute of the
/// day, in its own time zone. RouterOS 7.10+ prints 2026-11-20, older ones nov/20/2026.
fn clock_of(live: &Value) -> Option<(u32, u32)> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let parts: Vec<&str> = s(&live["clock"], "date").split(['-', '/']).collect();
    let (y, m, d): (i64, u32, u32) = match parts[..] {
        [y, m, d] if y.len() == 4 => (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?),
        [mon, d, y] => (y.parse().ok()?, MONTHS.iter().position(|x| x.eq_ignore_ascii_case(mon))? as u32 + 1, d.parse().ok()?),
        _ => return None,
    };
    Some((weekday(y, m, d), hm(s(&live["clock"], "time").get(..5)?)?))
}

/// Whether an SSID is on air at `now` by its schedule {days: [1..7], from,
/// to}: from <= time < to on those days; `to` before `from` runs past
/// midnight (from the day it starts); from = to, the whole day. No
/// schedule, or no clock: on, as the state says.
fn on_air(x: &Value, now: Option<(u32, u32)>) -> bool {
    let w = &x["schedule"];
    let (Some((day, min)), Some(from), Some(to)) = (now, hm(s(w, "from")), hm(s(w, "to"))) else { return true };
    let days: Vec<u64> = w["days"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    let on = |d: u32| days.contains(&(d as u64));
    match from.cmp(&to) {
        std::cmp::Ordering::Less => on(day) && (from..to).contains(&min),
        std::cmp::Ordering::Equal => on(day),
        std::cmp::Ordering::Greater => (on(day) && min >= from) || (on(if day == 1 { 7 } else { day - 1 }) && min < to),
    }
}

/// The state as it stands now on an access point: an SSID outside its schedule has no band.
fn effective(st: &Value, now: Option<(u32, u32)>) -> Value {
    let mut v = st.clone();
    for x in v["ssids"].as_array_mut().into_iter().flatten().filter(|x| !on_air(x, now)) {
        x["bands"] = json!([]);
    }
    v
}

/// What the schedules turn on or off on an access point now: the interfaces
/// of scheduled SSIDs ({name, disabled}, for `changes`): every one off outside
/// the schedule, those of the state's bands on inside it. Nothing else of the
/// state is applied without a person: an SSID missing there, or on a band the
/// state leaves out, waits for "Apply the state".
fn toggles(live: &Value, st: &Value) -> Vec<Value> {
    let (now, wifi) = (clock_of(live), rows(live, "wifi"));
    let mut out = vec![];
    for x in rows(st, "ssids").iter().filter(|x| x["schedule"].is_object()) {
        let (on, bands) = (on_air(x, now), strs(&x["bands"]));
        for r in wifi.iter().filter(|r| s(r, "configuration.ssid") == s(x, "ssid") && !station(r)) {
            if on && !bands.contains(&radio_band(live, &radio_of(wifi, s(r, "name")))) {
                continue;
            }
            let off = !on;
            if yes(s(r, "disabled")) != off {
                out.push(json!({"name": s(r, "name"), "disabled": off}));
            }
        }
    }
    out
}

/// An access point as the schedules leave it within a minute (`toggles`
/// done): what drift judges where Cairn Net runs them.
fn switched(live: &Value, st: &Value) -> Value {
    let mut v = live.clone();
    for t in toggles(live, st) {
        if let Some(r) = v["wifi"].as_array_mut().into_iter().flatten().find(|r| s(r, "name") == s(&t, "name")) {
            r["disabled"] = json!(yn(t["disabled"] == true));
        }
    }
    v
}

/// One access point's scheduled SSIDs turned on or off now (write account):
/// guarded apply, then kept by Cairn Net itself once a new connection reads
/// them back as asked; otherwise the access point undoes it. None: nothing to do,
/// or a person's change waits for "Keep" (next minute).
fn run_schedule(sw: &settings::Switch, st: &Value) -> Result<Option<String>, String> {
    let c = &sw.cfg;
    // looked at with the read account's kept connection: the write account logs in
    // (a log line each time) only when there is something to do
    let live = crate::with_api(sw, |api| conf(api, false))?;
    if !is_ap(&live) || toggles(&live, st).is_empty() {
        return Ok(None);
    }
    if clock_of(&live).is_none() {
        return Err("its clock cannot be read: the SSID schedules are not run there".into());
    }
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    if crate::rollback_left(&mut api)? {
        return Ok(None);
    }
    drop(api);
    let what = |t: &[Value]| t.iter().map(|x| format!("{} {}", s(x, "name"), if x["disabled"] == true { "off" } else { "on" })).collect::<Vec<_>>().join(", ");
    let mut done = String::new();
    let out = crate::guarded_apply(c, "schedule", "wifi schedule", |api| {
        let live = conf(api, false)?;
        let t = toggles(&live, st);
        done = what(&t);
        let (cmds, mut undo, _) = changes(&live, &json!({"ifaces": t}))?;
        if !cmds.is_empty() {
            infra::finish(&mut undo);
        }
        Ok((cmds, undo))
    }, |api| Ok(toggles(&conf(api, false)?, st).is_empty()))?;
    if out["pending"] != true {
        return Ok(None);
    }
    if out["verified"] != true {
        return Err(format!("{done}: not read back as asked ({}): the access point undoes it", s(&out, "error")));
    }
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    crate::rollback_disarm(&mut api)?;
    forget();
    Ok(Some(done))
}

/// Every minute, once an SSID of the state has a schedule: the access points
/// in direct mode with a write account turn those SSIDs on or off
/// (`run_schedule`). A failure is notified once, until it changes.
pub(crate) fn schedules(notify: Option<&'static crate::Notify>) {
    std::thread::spawn(move || {
        let mut last: BTreeMap<String, String> = BTreeMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(60));
            let Some(st) = state().filter(|st| rows(st, "ssids").iter().any(|x| x["schedule"].is_object())) else { continue };
            for sw in settings::all().iter().filter(|sw| sw.cfg.mode == "direct" && !sw.cfg.write_user.is_empty()) {
                let c = &sw.cfg;
                let err = match run_schedule(sw, &st) {
                    Ok(done) => {
                        if let Some(d) = done {
                            eprintln!("wifi schedule on {}: {d}", c.name);
                        }
                        String::new()
                    }
                    Err(e) => e,
                };
                if last.insert(c.name.clone(), err.clone()).unwrap_or_default() == err || err.is_empty() {
                    continue;
                }
                let msg = format!("{}: SSID schedule not run ({err}).", c.name);
                eprintln!("wifi schedule: {msg}");
                if let Some(n) = notify {
                    n.send("Cairn Net: Wi-Fi schedule", &msg);
                }
            }
        }
    });
}

fn slow() -> Value {
    serde_json::from_str(&crate::slow()).unwrap_or(Value::Null)
}

fn forget() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The write account's read of an access point, its passwords included.
fn conf_write(c: &settings::SwitchCfg) -> Result<(Api, Value), String> {
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let v = conf(&mut api, true)?;
    Ok((api, v))
}

/// POST /api/wifi/preview {switch}: what applying the Wi-Fi state there
/// would run; nothing is written, passwords masked.
pub fn preview(v: &Value) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let st = state().ok_or(NO_STATE)?;
    let up = uplink(&slow(), &sw.cfg.name);
    let (mut api, live) = conf_write(&sw.cfg)?;
    let (cmds, undo, warn) = plan_state(&live, &st, &sw.cfg.name, up.as_ref())?;
    let sched = crate::has_scheduler(&mut api);
    let undo: Vec<String> = undo.iter().filter(|l| sched || !l.starts_with("/system scheduler remove")).map(|l| crate::direct::redact(l)).collect();
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S, "warnings": warn}))
}

/// POST /api/wifi/apply {switch}: guarded apply of the Wi-Fi state (write
/// account), read back until nothing is left to change; "Keep" is
/// /api/infra/confirm, as on the other pages.
pub fn apply(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let st = state().ok_or(NO_STATE)?;
    let name = sw.cfg.name.clone();
    let up = uplink(&slow(), &name);
    let left = |api: &mut Api| -> Result<usize, String> { Ok(plan_state(&conf(api, true)?, &st, &name, up.as_ref())?.0.len()) };
    let out = crate::guarded_apply(&sw.cfg, user, "wifi", |api| {
        let (c, u, _) = plan_state(&conf(api, true)?, &st, &name, up.as_ref())?;
        Ok((c, u))
    }, |api| {
        // an interface just added can read incomplete for a moment
        for _ in 0..2 {
            if left(api)? == 0 {
                return Ok(true);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        Ok(left(api)? == 0)
    });
    forget();
    out
}

/// POST /api/wifi/state, nothing written to the access points:
/// {"op": "import", switch} takes the state from an access point (its
/// passwords too in direct mode); {"op": "ssids", ssids: [{ssid, orig, vlan,
/// security, passphrase, isolation, hidden, bands}]} replaces the SSIDs (an
/// empty password keeps the one of `orig`); {"op": "radios", switch,
/// radios: {interface: {frequency, width, tx_power}}} sets one access point's radios.
pub fn state_post(v: &Value, user: &str) -> Result<Value, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let old = state();
    let op = s(v, "op");
    let new = match op {
        "import" => {
            let name = s(v, "switch");
            let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
            // passwords with the write account; without one (not adopted yet), the read account
            let live = if sw.cfg.mode == "direct" && !sw.cfg.write_user.is_empty() { conf_write(&sw.cfg)?.1 } else { crate::with_api(&sw, |api| conf(api, false))? };
            if !is_ap(&live) {
                return Err(format!("{name}: no Wi-Fi radio"));
            }
            import(&live, name, old.as_ref())
        }
        "ssids" => {
            let old = old.ok_or(NO_STATE)?;
            let list: Vec<Value> = rows(v, "ssids").iter().map(|x| {
                let pw = match s(x, "passphrase") {
                    "" => rows(&old, "ssids").iter().find(|o| s(o, "ssid") == s(x, "orig")).map_or("", |o| s(o, "passphrase")),
                    p => p,
                };
                let sec = norm("security", s(x, "security"));
                let mut o = json!({"ssid": s(x, "ssid").trim(), "vlan": s(x, "vlan").trim(), "passphrase": if sec.is_empty() { "" } else { pw }, "security": sec,
                    "isolation": x["isolation"] == true, "hidden": x["hidden"] == true, "bands": strs(&x["bands"]),
                    "ft": x["ft"] == true, "kv": x["kv"] == true, "band_steering": x["band_steering"] == true});
                // lot Wi-Fi 4b: on air only {days, from, to}; none = always
                if let Some(w) = x["schedule"].as_object() {
                    o["schedule"] = json!({"days": w.get("days").cloned().unwrap_or_default(), "from": s(&x["schedule"], "from"), "to": s(&x["schedule"], "to")});
                }
                o
            }).collect();
            let mut st = old;
            st["ssids"] = json!(list);
            st
        }
        // lot Wi-Fi 4a: a device blocked on every access point, or no longer
        "block" | "unblock" => {
            let mut st = old.ok_or(NO_STATE)?;
            let mac = s(v, "mac").trim().to_uppercase();
            if !is_mac(&mac) {
                return Err(format!("{mac}: not a MAC address (AA:BB:CC:DD:EE:FF)"));
            }
            let mut list: Vec<Value> = rows(&st, "blocked").iter().filter(|b| s(b, "mac") != mac).cloned().collect();
            if op == "block" {
                list.push(json!({"mac": mac, "name": s(v, "name").trim().chars().take(64).collect::<String>()}));
            }
            st["blocked"] = json!(list);
            st
        }
        // a new random password for one SSID (shared as a QR code)
        "rotate" => {
            let mut st = old.ok_or(NO_STATE)?;
            let name = s(v, "ssid");
            let x = st["ssids"].as_array_mut().into_iter().flatten().find(|x| s(x, "ssid") == name).ok_or_else(|| format!("{name}: not in the Wi-Fi state"))?;
            if s(x, "security").is_empty() {
                return Err(format!("{name} is open (no security): no password to change"));
            }
            x["passphrase"] = json!(guest_password()?);
            st
        }
        "radios" => {
            let mut st = old.ok_or(NO_STATE)?;
            let name = s(v, "switch");
            settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
            let m: serde_json::Map<String, Value> = v["radios"].as_object().into_iter().flatten()
                .map(|(n, w)| (n.clone(), json!({"frequency": s(w, "frequency"), "width": s(w, "width"), "tx_power": s(w, "tx_power")}))).collect();
            st["radios"][name] = Value::Object(m);
            st
        }
        _ => return Err("unknown change".into()),
    };
    write_state(&new)?;
    eprintln!("wifi state: {op} by {user}");
    forget();
    Ok(json!({"ok": true}))
}

/// Every 10 minutes, once there is a Wi-Fi state: the access points that
/// differ from it (passwords compared in direct mode; not while a change
/// waits for its confirmation), notified when that changes.
pub(crate) fn watch(notify: Option<&'static crate::Notify>) {
    std::thread::spawn(move || {
        let mut last: BTreeMap<String, Vec<String>> = BTreeMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(600));
            let Some(st) = state() else { continue };
            let slow = slow();
            for sw in settings::all() {
                let c = &sw.cfg;
                if !rows(&slow, "switches").iter().any(|w| s(w, "name") == c.name && is_ap(w)) {
                    continue;
                }
                let live = if c.mode == "direct" && !c.write_user.is_empty() {
                    conf_write(c).and_then(|(mut api, live)| Ok(if crate::rollback_left(&mut api)? { None } else { Some(live) }))
                } else {
                    crate::with_api(&sw, |api| conf(api, false)).map(Some)
                };
                let Ok(Some(live)) = live else { continue }; // unreachable, or a change waiting: the page says so
                // where Cairn Net runs the schedules, a boundary just passed is no drift
                let live = if c.mode == "direct" && !c.write_user.is_empty() { switched(&live, &st) } else { live };
                let d = plan_state(&live, &st, &c.name, uplink(&slow, &c.name).as_ref()).map_or_else(|e| vec![e], |p| infra::show_all(&p.0));
                if last.insert(c.name.clone(), d.clone()).unwrap_or_default() == d {
                    continue;
                }
                let msg = if d.is_empty() {
                    format!("{}: in line with the Wi-Fi state again.", c.name)
                } else {
                    format!("{}: differs from the Wi-Fi state ({}). Apply the state there, or take the state from it (Cairn Net, Wi-Fi).", c.name, d.join("; "))
                };
                eprintln!("wifi drift: {msg}");
                if let Some(n) = notify {
                    n.send("Cairn Net: Wi-Fi drift", &msg);
                }
            }
        }
    });
}

/* ---------- a new access point: first setup and adoption (lot Wi-Fi 3) ---------- */

/// What a first-setup script needs (POST /api/wifi/bootstrap).
struct Boot {
    name: String,
    address: String, // a.b.c.d/prefix, in the management VLAN
    gateway: String,
    vlan: u32,         // management VLAN, tagged on the uplink
    uplink: String,    // the port towards the network
    pvid: Option<u32>, // the uplink's untagged VLAN
    dns: String,
    country: String,
    f24: Option<u32>, // fixed channels (MHz), else automatic
    f5: Option<u32>,
    read_user: String,
    app: String, // Cairn Net's address, the only one the accounts and api-ssl accept
    key: String, // Cairn Net's public SSH key
    extra: String,
}

fn boot_from(v: &Value) -> Result<Boot, String> {
    let t = |k: &str| s(v, k).trim();
    let ip = |x: &str| x.parse::<std::net::Ipv4Addr>().is_ok();
    let num = |k: &str, lo: u32, hi: u32| -> Result<Option<u32>, String> {
        match t(k) {
            "" => Ok(None),
            x => x.parse::<u32>().ok().filter(|n| (lo..=hi).contains(n)).map(Some).ok_or_else(|| format!("{k}: {lo} to {hi}")),
        }
    };
    if !crate::dhcp::is_name(t("name")) {
        return Err("name: letters, digits and dashes".into());
    }
    if !t("address").split_once('/').is_some_and(|(a, l)| ip(a) && l.parse::<u8>().is_ok_and(|l| (1..=32).contains(&l))) {
        return Err("address: a.b.c.d/prefix".into());
    }
    if !ip(t("gateway")) {
        return Err("gateway: an IPv4 address".into());
    }
    let vlan = num("vlan", 1, 4094)?.ok_or("vlan: the management VLAN, 1 to 4094")?;
    let pvid = num("pvid", 1, 4094)?;
    if pvid == Some(vlan) {
        return Err("pvid: the management VLAN is tagged, the untagged VLAN is another one".into());
    }
    let uplink = Some(t("uplink")).filter(|u| !u.is_empty()).unwrap_or("ether1");
    if !crate::dhcp::is_name(uplink) {
        return Err("uplink: an interface name".into());
    }
    let dns: Vec<&str> = t("dns").split([',', ' ']).filter(|x| !x.is_empty()).collect();
    if dns.is_empty() || !dns.iter().all(|d| ip(d)) {
        return Err("dns: IPv4 addresses".into());
    }
    let country = t("country");
    if country.is_empty() || country.len() > 40 || !country.chars().all(|c| c.is_ascii_alphabetic() || c == ' ') {
        return Err("country: its English name, as RouterOS spells it".into());
    }
    let read_user = match settings::get(t("like")) {
        Some(l) => l.cfg.user.clone(),
        None => Some(t("read_user")).filter(|u| !u.is_empty()).unwrap_or("cairn-read").to_string(),
    };
    if read_user == WRITE_USER || !read_user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("read account: letters, digits, - and _".into());
    }
    let app = match t("app") {
        "" => app_ip().ok_or("Cairn Net's own address: no switch answers, give it")?,
        a if ip(a) => a.to_string(),
        _ => return Err("Cairn Net's own address: an IPv4 address".into()),
    };
    let key = crate::git::public_key();
    if !key.starts_with("ssh-") {
        return Err("Cairn Net has no SSH key (data directory)".into());
    }
    Ok(Boot {
        name: t("name").into(), address: t("address").into(), gateway: t("gateway").into(), vlan, uplink: uplink.into(), pvid,
        dns: dns.join(","), country: country.into(), f24: num("ch24", 1, 13)?.map(|c| 2407 + 5 * c), f5: num("ch5", 32, 177)?.map(|c| 5000 + 5 * c),
        read_user, app, key, extra: fs::read_to_string(format!("{}/ap-extra.rsc", crate::git::data())).unwrap_or_default(),
    })
}

/// Cairn Net's address as its switches see it.
fn app_ip() -> Option<String> {
    settings::all().iter().find_map(|sw| crate::with_api(sw, |api| Ok(api.local_ip())).ok().flatten()).map(|a| a.to_string())
}

/// The first-setup script of a blank access point: bridge with its uplink,
/// management VLAN and address, radios off without SSID (the Wi-Fi state
/// brings them), accounts for Cairn Net (the write one with its SSH key),
/// services closed but api-ssl and SSH, the lines of data/ap-extra.rsc,
/// then VLAN filtering. No secret: passwords are random and nobody keeps them.
fn render(b: &Boot) -> String {
    let q = infra::q;
    let mut l: Vec<String> = vec![
        format!("# Cairn Net {}: first setup of the access point {} (RouterOS 7, wifi package).", env!("CARGO_PKG_VERSION"), b.name),
        "# Run it once on the blank device (reset without defaults), checked first, then as the script of a second reset:".into(),
        "#   /import file-name=<this file> verbose=yes dry-run".into(),
        "#   /system reset-configuration no-defaults=yes skip-backup=yes run-after-reset=<this file>".into(),
        "# then Cairn Net, Wi-Fi, Add an access point, Adopt. No secret inside: random passwords nobody keeps, Cairn Net's public key.".into(),
        "{".into(),
        ":local n 0".into(),
        format!(":while (([:len [/interface find where name={}]] = 0 || [:len [/interface wifi radio find]] = 0) && $n < 60) do={{ :delay 1s; :set n ($n + 1) }}", q(&b.uplink)),
        format!("/system identity set name={}", q(&b.name)),
        format!("/ip dns set servers={}", b.dns),
        "/interface bridge add name=bridge protocol-mode=rstp vlan-filtering=no comment=\"cairn-net\"".into(),
        format!("/interface bridge port add bridge=bridge interface={} pvid={} comment=\"uplink\"", b.uplink, b.pvid.unwrap_or(1)),
    ];
    if let Some(p) = b.pvid {
        l.push(format!("/interface bridge vlan add bridge=bridge vlan-ids={p} untagged={} comment=\"uplink\"", b.uplink));
    }
    let m = format!("mgmt{}", b.vlan);
    l.extend([
        format!("/interface vlan add name={m} interface=bridge vlan-id={} comment=\"management\"", b.vlan),
        format!("/interface bridge vlan add bridge=bridge vlan-ids={} tagged=bridge,{} comment=\"management\"", b.vlan, b.uplink),
        format!("/ip address add address={} interface={m} comment=\"management\"", b.address),
        format!("/ip route add dst-address=0.0.0.0/0 gateway={} comment=\"cairn-net\"", b.gateway),
        "# radios: off and without SSID until Cairn Net applies the Wi-Fi state".into(),
        ":foreach r in=[/interface wifi radio find] do={".into(),
        "  :local i [/interface wifi radio get $r interface]".into(),
        "  :local b [:tostr [/interface wifi radio get $r bands]]".into(),
        "  :do { /interface wifi unset [find where name=$i] mld-interface } on-error={}".into(),
        format!("  /interface wifi set [find where name=$i] configuration.country={} disabled=yes", q(&b.country)),
    ]);
    if let Some(f) = b.f24 {
        l.push(format!("  :if ($b ~ \"2ghz\") do={{ /interface wifi set [find where name=$i] channel.frequency={f} channel.width=20mhz }}"));
    }
    if let Some(f) = b.f5 {
        l.push(format!("  :if ($b ~ \"5ghz\") do={{ /interface wifi set [find where name=$i] channel.frequency={f} channel.width=20/40mhz }}"));
    }
    l.extend([
        "}".into(),
        "# accounts: random passwords; Cairn Net sets the read and write ones at adoption, logging in with its key".into(),
        "/user set [find name=admin] password=[:rndstr length=32]".into(),
        ":if ([:len [/user group find name=\"cairn-read\"]] = 0) do={ /user group add name=cairn-read policy=read,api,test comment=\"Cairn Net: reads\" }".into(),
        format!("/user add name={} group=cairn-read address={}/32 password=[:rndstr length=32] comment=\"Cairn Net: reads\"", q(&b.read_user), b.app),
        format!("/user add name={WRITE_USER} group=full address={}/32 password=[:rndstr length=32] comment=\"Cairn Net: writes\"", b.app),
        format!("/user ssh-keys add user={WRITE_USER} key={}", q(&b.key)),
        "# services: api-ssl for Cairn Net only, SSH; MAC-WinBox stays as a way in".into(),
        ":foreach s in={\"telnet\";\"ftp\";\"api\";\"www\";\"winbox\"} do={ /ip service set [find name=$s dynamic=no] disabled=yes }".into(),
        ":do { /ip service set [find name=\"reverse-proxy\" dynamic=no] disabled=yes } on-error={}".into(),
        format!("/certificate add name=api-ssl common-name={} key-size=2048 days-valid=3650", q(&b.name)),
        "/certificate sign api-ssl".into(),
        ":local k 0".into(),
        format!(":while ($k < 60) do={{ :do {{ /ip service set [find name=\"api-ssl\" dynamic=no] certificate=api-ssl address={}/32 disabled=no; :set k 99 }} on-error={{ :delay 1s; :set k ($k + 1) }} }}", b.app),
    ]);
    if !b.extra.trim().is_empty() {
        l.push("# local additions (data/ap-extra.rsc)".into());
        l.extend(b.extra.lines().map(|x| x.trim_end().to_string()));
    }
    l.extend([
        "# last: VLAN filtering (the uplink carries the management VLAN, then the SSIDs' VLANs Cairn Net adds)".into(),
        "/interface bridge set bridge vlan-filtering=yes".into(),
        format!(":log info {}", q(&format!("cairn-net: {} ready at {}, waiting for adoption", b.name, b.address))),
        "}".into(),
    ]);
    l.join("\n") + "\n"
}

/// POST /api/wifi/bootstrap: the first-setup script of a new access point.
pub fn bootstrap(v: &Value) -> Result<Value, String> {
    let b = boot_from(v)?;
    Ok(json!({"ok": true, "script": render(&b), "file": format!("{}.rsc", b.name.to_lowercase()), "app": b.app}))
}

/// `n` random characters of `alphabet`.
fn random(alphabet: &[u8], n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| format!("random: {e}"))?;
    // ponytail: modulo bias (256 % len), negligible at these lengths
    Ok(b.iter().map(|x| alphabet[*x as usize % alphabet.len()] as char).collect())
}

/// 32 random letters and digits, no look-alikes.
fn random_password() -> Result<String, String> {
    random(b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789", 32)
}

/// A password guests type: three groups of four lower-case letters and
/// digits without look-alikes ("k7mp-2xqa-9fhn", about 59 bits).
fn guest_password() -> Result<String, String> {
    let r = random(b"abcdefghjkmnpqrstuvwxyz23456789", 12)?;
    Ok([&r[..4], &r[4..8], &r[8..]].join("-"))
}

/// The text of a Wi-Fi QR code (the "WIFI:" scheme phones read).
fn wifi_qr_text(ssid: &str, security: &str, pw: &str, hidden: bool) -> String {
    let esc = |x: &str| x.chars().flat_map(|c| if "\\;,\":".contains(c) { vec!['\\', c] } else { vec![c] }).collect::<String>();
    let t = if security.is_empty() { "nopass" } else { "WPA" };
    format!("WIFI:T:{t};S:{};{}{};", esc(ssid), if security.is_empty() { String::new() } else { format!("P:{};", esc(pw)) }, if hidden { "H:true;" } else { "" })
}

/// A QR code as SVG: dark squares on white whatever the page theme, quiet zone of 4.
fn qr_svg(text: &str) -> Result<String, String> {
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M).map_err(|e| format!("QR code: {e}"))?;
    let w = code.width();
    let mut d = String::new();
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == qrcode::Color::Dark {
            d += &format!("M{} {}h1v1h-1z", i % w + 4, i / w + 4);
        }
    }
    let n = w + 8;
    Ok(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"wf-qr\" viewBox=\"0 0 {n} {n}\" shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"QR code\">\
        <rect width=\"{n}\" height=\"{n}\" fill=\"#fff\"/><path fill=\"#000\" d=\"{d}\"/></svg>"))
}

/// POST /api/wifi/share {ssid}: the SSID of the state with its password and
/// its QR code, for guests. Admins only, like every page; never cached.
pub fn share(v: &Value, user: &str) -> Result<Value, String> {
    let st = state().ok_or(NO_STATE)?;
    let name = s(v, "ssid");
    let x = rows(&st, "ssids").iter().find(|x| s(x, "ssid") == name).ok_or_else(|| format!("{name}: not in the Wi-Fi state"))?;
    let (sec, pw) = (s(x, "security"), s(x, "passphrase"));
    if !sec.is_empty() && pw.is_empty() {
        return Err(format!("{name}: no password for it in the Wi-Fi state: set one"));
    }
    eprintln!("wifi: {name} shared with {user}");
    Ok(json!({"ok": true, "ssid": name, "password": pw, "open": sec.is_empty(), "svg": qr_svg(&wifi_qr_text(name, sec, pw, x["hidden"] == true))?}))
}

/// POST /api/wifi/kick {switch, mac}: the client leaves that access point
/// now (write account, direct mode). Nothing to undo; it usually joins again
/// at once unless it is blocked.
pub fn kick(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let c = &sw.cfg;
    let mac = s(v, "mac").trim().to_uppercase();
    if !is_mac(&mac) {
        return Err(format!("{mac}: not a MAC address (AA:BB:CC:DD:EE:FF)"));
    }
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let reg = crate::print(&mut api, "/interface/wifi/registration-table", ".id,mac-address", &[&format!("?mac-address={mac}")])?;
    if reg.is_empty() {
        return Err(format!("{mac} is not connected to {}", c.name));
    }
    for r in &reg {
        api.call(&["/interface/wifi/registration-table/remove", &format!("=.id={}", s(r, ".id"))])?;
    }
    forget();
    eprintln!("wifi: {mac} disconnected from {} by {user}", c.name);
    Ok(json!({"ok": true}))
}

/// The RouterOS id of the radio to restart: a Wi-Fi interface with a radio
/// (not an SSID it carries) that is on (restarting would turn a radio on).
fn restart_target(rows: &[Value], iface: &str) -> Result<String, String> {
    let r = rows.iter().find(|r| s(r, "name") == iface).ok_or_else(|| format!("{iface}: no such Wi-Fi interface"))?;
    if s(r, "radio-mac").is_empty() {
        return Err(format!("{iface}: an SSID, not a radio: restart its radio"));
    }
    if yes(s(r, "disabled")) {
        return Err(format!("{iface}: disabled, nothing to restart"));
    }
    Ok(s(r, ".id").to_string())
}

/// POST /api/wifi/restart {switch, iface}: the radio stops three seconds and
/// starts again with its SSIDs (write account, direct mode); clients join
/// again by themselves. For a radio that stops taking new clients while the
/// connected ones stay (seen on RouterOS 7.25.1). One background job on the
/// access point does both, so the radio comes back even if this connection drops.
pub fn restart(v: &Value, user: &str) -> Result<Value, String> {
    let sw = crate::direct_switch(v)?;
    let c = &sw.cfg;
    let iface = s(v, "iface");
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let rows = crate::print(&mut api, "/interface/wifi", ".id,name,radio-mac,disabled", &[])?;
    let f = format!("[find where .id={}]", restart_target(&rows, iface)?);
    api.call(&["/execute", &format!("=script=/interface wifi disable {f}; :delay 3s; /interface wifi enable {f}")])?;
    forget();
    eprintln!("wifi: {iface} restarted on {} by {user}", c.name);
    Ok(json!({"ok": true}))
}

/// One RouterOS command line over SSH with Cairn Net's key; its output.
fn ssh_ros(user: &str, host: &str, script: &str) -> Result<String, String> {
    // ponytail: host key not checked (a blank device gets a new one at each reset): the
    // api-ssl certificate is matched against the one this session prints instead
    let dest = format!("{user}@{host}");
    let o = Command::new("ssh").args(["-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"])
        .args(crate::git::ssh_options()).args(["-o", "ConnectTimeout=10", "--", dest.as_str(), script])
        .env("HOME", crate::git::data()).output().map_err(|e| format!("ssh: {e}"))?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if o.status.success() {
        Ok(text(&o.stdout))
    } else {
        Err(format!("ssh {user}@{host}: {}", Some(text(&o.stderr)).filter(|e| !e.is_empty()).unwrap_or_else(|| "failed".into())))
    }
}

/// "Permission denied" from the access point: its write account does not
/// exist yet or lacks Cairn Net's key; say what to do instead of the raw ssh error.
fn ssh_refused(e: String, known: bool) -> String {
    match (e.contains("Permission denied"), known) {
        (false, _) => e,
        (true, true) => format!("{WRITE_USER} does not accept Cairn Net's key: run the two RouterOS lines first"),
        (true, false) => format!("{WRITE_USER} does not accept Cairn Net's key: the first-setup script did not run there"),
    }
}

/// POST /api/wifi/adopt {name, host?, like?}: the access point ran the
/// first-setup script (or its write account has Cairn Net's key). Over SSH
/// with that key, Cairn Net gives the write account a new password only it
/// knows (and the read account the password of `like`'s, or a new one),
/// reads the fingerprint of the api-ssl certificate and checks the API shows
/// that one, then saves the access point in Settings, direct mode. An access
/// point already in Settings keeps its address and read account.
pub fn adopt(v: &Value, user: &str) -> Result<Value, String> {
    let name = s(v, "name").trim();
    let old = settings::get(name).map(|x| x.cfg.clone());
    let host = match (s(v, "host").trim(), &old) {
        ("", Some(c)) => c.host.clone(),
        (h, _) => h.to_string(),
    };
    if !crate::dhcp::is_name(name) || host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) {
        return Err("the access point's name and address".into());
    }
    let (read_user, read_pw, set_read) = match (&old, settings::get(s(v, "like"))) {
        (Some(c), _) => (c.user.clone(), c.password.clone(), false),
        (None, Some(l)) => (l.cfg.user.clone(), l.cfg.password.clone(), true),
        (None, None) => (Some(s(v, "read_user").trim()).filter(|u| !u.is_empty()).unwrap_or("cairn-read").to_string(), random_password()?, true),
    };
    let write_pw = random_password()?;
    let q = infra::q;
    let mut script = format!("/user set [find name={}] password={}", q(WRITE_USER), q(&write_pw));
    if set_read {
        script += &format!("; /user set [find name={}] password={}", q(&read_user), q(&read_pw));
    }
    script += "; :put [/certificate get [/ip service get [find name=\"api-ssl\" dynamic=no] certificate] fingerprint]";
    let out = ssh_ros(WRITE_USER, &host, &script).map_err(|e| ssh_refused(e, old.is_some()))?;
    let shown = settings::pin(out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or(""));
    let (_, got) = Api::tls(&host, 8729)?;
    if shown != got {
        return Err(format!("the API certificate ({}) is not the one the access point shows over SSH: not adopted", settings::pretty(&got)));
    }
    if old.as_ref().is_some_and(|c| c.sha256 != got) {
        return Err("the API certificate is not the one Settings trust: not adopted".into());
    }
    let mut c = old.clone().unwrap_or_else(|| settings::SwitchCfg::from_json(&json!({"name": name, "host": host})));
    c.sha256 = got;
    c.user = read_user;
    c.password = read_pw;
    c.write_user = WRITE_USER.into();
    c.write_password = write_pw;
    c.mode = "direct".into();
    Api::connect(&c, &c.user, &c.password).map_err(|e| format!("read account: {e}"))?;
    Api::connect(&c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let mut list: Vec<settings::SwitchCfg> = settings::all().iter().map(|x| x.cfg.clone()).collect();
    match list.iter().position(|x| x.name == name) {
        Some(i) => list[i] = c,
        None => list.push(c),
    }
    settings::save(list)?;
    crate::forget_cache();
    forget();
    eprintln!("wifi: {name} adopted by {user} ({host})");
    Ok(json!({"ok": true, "name": name, "new": old.is_none()}))
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
    fn adopt_says_what_to_do_when_the_key_is_refused() {
        let e = || "ssh cairn-write@10.0.0.2: cairn-write@10.0.0.2: Permission denied (publickey,password).".to_string();
        assert_eq!(ssh_refused(e(), true), "cairn-write does not accept Cairn Net's key: run the two RouterOS lines first");
        assert_eq!(ssh_refused(e(), false), "cairn-write does not accept Cairn Net's key: the first-setup script did not run there");
        let other = "ssh cairn-write@10.0.0.2: Connection timed out";
        assert_eq!(ssh_refused(other.into(), true), other);
    }

    #[test]
    fn restart_only_a_radio_that_is_on() {
        let rows = vec![
            json!({".id": "*1", "name": "wifi1", "radio-mac": "02:00:00:00:00:01", "disabled": "false"}),
            json!({".id": "*5", "name": "wifi1-guest", "disabled": "false"}),
            json!({".id": "*2", "name": "wifi2", "radio-mac": "02:00:00:00:00:02", "disabled": "true"}),
        ];
        assert_eq!(restart_target(&rows, "wifi1"), Ok("*1".to_string()));
        assert_eq!(restart_target(&rows, "wifi1-guest"), Err("wifi1-guest: an SSID, not a radio: restart its radio".to_string()));
        assert_eq!(restart_target(&rows, "wifi2"), Err("wifi2: disabled, nothing to restart".to_string()));
        assert_eq!(restart_target(&rows, "wifi9"), Err("wifi9: no such Wi-Fi interface".to_string()));
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
    /// profiles of the setup script it was made with, the old
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
            // as on a real access point: ether1 = uplink, 20 untagged, 10, 30 and 99 tagged
            "ports": [{"bridge": "bridge", "interface": "ether1", "pvid": "20"}, {"bridge": "bridge", "interface": "wifi1", "pvid": "10"}],
            "bvlan": [{"bridge": "bridge", "vlan-ids": "99", "current-tagged": "bridge,ether1"}, {"bridge": "bridge", "vlan-ids": "10,30", "current-tagged": "ether1"},
                {"bridge": "bridge", "vlan-ids": "20", "current-untagged": "ether1"}, {"bridge": "bridge", "vlan-ids": "10", "current-tagged": "wifi1"}],
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

    /* ---------- the Wi-Fi state (lot Wi-Fi 3) ---------- */

    /// The switch across the fixture's uplink: 10, 20 (untagged), 30 and 40 pass, 50 does not.
    fn up() -> Uplink {
        Uplink { port: "ether1".into(), sw: "core".into(), sw_port: "sfp5".into(), sw_vlans: vec![
            json!({"vlan-ids": "10", "current-tagged": "sfp5"}), json!({"vlan-ids": "20", "current-untagged": "sfp5"}),
            json!({"vlan-ids": "30,40", "current-tagged": "bridge,sfp5"}), json!({"vlan-ids": "50", "current-tagged": "bridge"})] }
    }
    fn st_plan(st: &Value) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
        plan_state(&live(), st, "ap-hall", Some(&up())).map(|(c, u, w)| (infra::show_all(&c), u, w))
    }

    #[test]
    fn state_taken_from_an_access_point_changes_nothing_there() {
        let st = import(&live(), "ap-hall", None);
        assert_eq!(rows(&st, "ssids").iter().map(|x| s(x, "ssid")).collect::<Vec<_>>(), ["Home", "Guests", "IoT"], "the radios' own SSID first");
        let g = &st["ssids"][1];
        assert_eq!((s(g, "vlan"), s(g, "security"), s(g, "passphrase"), g["isolation"].as_bool(), strs(&g["bands"])),
            ("30", "wpa2-psk,wpa3-psk", "guest\"pass$1", Some(true), vec!["2.4", "5"]));
        assert_eq!(st["radios"]["ap-hall"]["wifi2"], json!({"frequency": "5180", "width": "20/40mhz", "tx_power": ""}));
        check_state(&st).unwrap();
        assert_eq!(st_plan(&st).unwrap(), (vec![], vec![], vec![]));
        // read without the passwords (read account): not compared, still nothing to do
        let mut l = live();
        for r in l["wifi"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("secret");
        }
        for r in l["sec"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("secret");
        }
        assert!(plan_state(&l, &st, "ap-hall", Some(&up())).unwrap().0.is_empty());
        // the page learns that there is a password, never which
        let p = public_state(&st).to_string();
        assert!(!p.contains("guest\\\"pass") && !p.contains("old home") && p.contains("\"has_passphrase\":true"), "{p}");
    }

    #[test]
    fn settings_of_the_state_where_they_live() {
        let mut st = import(&live(), "ap-hall", None);
        st["ssids"][0]["hidden"] = json!(true);
        st["ssids"][1]["isolation"] = json!(false);
        st["ssids"][2]["vlan"] = json!("40");
        st["radios"]["ap-hall"]["wifi1"]["tx_power"] = json!("18");
        let (c, u, w) = st_plan(&st).unwrap();
        assert_eq!(c, [
            "/interface/bridge/vlan/add bridge=bridge vlan-ids=40 tagged=ether1 comment=\"cairn-net: Wi-Fi VLAN 40\"",
            "/interface/wifi/set .id=*3 configuration.tx-power=18",
            "/interface/wifi/configuration/set .id=*1 hide-ssid=yes",
            "/interface/wifi/datapath/set .id=*3 client-isolation=no",
            "/interface/wifi/datapath/set .id=*2 vlan-id=40",
        ]);
        for l in [
            ":do { /interface wifi unset [find where .id=*3] value-name=configuration.tx-power } on-error={}",
            ":do { /interface wifi configuration unset [find where .id=*1] value-name=hide-ssid } on-error={}",
            ":do { /interface wifi datapath set [find where .id=*3] client-isolation=yes } on-error={}",
            ":do { /interface wifi datapath set [find where .id=*2] vlan-id=20 } on-error={}",
            ":do { /interface bridge vlan remove [find where comment=\"cairn-net: Wi-Fi VLAN 40\"] } on-error={}",
        ] {
            assert!(u.iter().any(|x| x == l), "rollback lacks {l}");
        }
        assert!(w.iter().any(|x| x.contains("IoT: its clients reconnect in another VLAN")) && w.iter().any(|x| x.contains("see each other")));
        // a VLAN the switch across does not carry: refused; switch unknown: a warning
        st["ssids"][2]["vlan"] = json!("50");
        let e = st_plan(&st).unwrap_err();
        assert_eq!(e, "VLAN 50 does not pass through core sfp5: add it there first");
        let (c, _, w) = plan_state(&live(), &st, "ap-hall", None).unwrap();
        assert!(w.iter().any(|x| x.contains("VLAN 50 not checked")));
        assert!(infra::show_all(&c)[0].contains("vlan-ids=50 tagged=ether1"), "no topology: the one wired bridge port");
    }

    #[test]
    fn ssids_added_moved_removed_and_renamed() {
        let mut st = import(&live(), "ap-hall", None);
        st["ssids"][1]["bands"] = json!(["5"]);
        let mut ssids = rows(&st, "ssids")[..2].to_vec(); // IoT goes
        ssids.push(json!({"ssid": "Lab net", "vlan": "30", "security": "wpa3-psk", "passphrase": "lab password", "isolation": false, "hidden": false, "bands": ["2.4", "5"]}));
        st["ssids"] = json!(ssids);
        let (c, u, w) = st_plan(&st).unwrap();
        let new = |r: &str| format!("/interface/wifi/add name={r}-lab-net master-interface={r} configuration.country=France configuration.ssid=\"Lab net\" \
            configuration.hide-ssid=no datapath.bridge=bridge datapath.client-isolation=no datapath.vlan-id=30 security.authentication-types=wpa3-psk \
            security.passphrase=\"•••\" disabled=no");
        assert_eq!(c, ["/interface/wifi/set .id=*C disabled=yes".to_string(), "/interface/wifi/remove .id=*B".into(), "/interface/wifi/remove .id=*D".into(),
            new("wifi1"), new("wifi2")]);
        assert_eq!(u[..3], [":do { /interface wifi remove [find where name=\"wifi1-lab-net\"] } on-error={}".to_string(),
            ":do { /interface wifi remove [find where name=\"wifi2-lab-net\"] } on-error={}".into(),
            ":do { /interface wifi add name=\"wifi1-iot\" master-interface=\"wifi1\" disabled=no configuration=\"cfg-iot\" } on-error={}".into()]);
        assert!(w.iter().any(|x| x == "IoT: removed from wifi1-iot, its clients are cut") && w.iter().any(|x| x == "wifi1-guest: off, its clients are cut"));
        assert!(st_plan(&json!({"ssids": [{"ssid": "Bare", "security": "wpa2-psk", "bands": ["5"]}]})).unwrap_err().contains("Bare: no password"));
        assert!(st_plan(&json!({"ssids": []})).is_err(), "an empty state is never applied");

        // the radios' own SSID renamed: their interfaces take the new one, settings written on them
        let mut st = import(&live(), "ap-hall", None);
        st["ssids"][0]["ssid"] = json!("Maison");
        let (c, u, _) = st_plan(&st).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c[0].starts_with("/interface/wifi/set .id=*3 configuration.ssid=Maison ") && c[0].ends_with("security.passphrase=\"•••\" disabled=no"), "{}", c[0]);
        assert!(u.iter().any(|l| l == ":do { /interface wifi set [find where .id=*3] configuration.ssid=\"Home\" } on-error={}"));
        assert!(u.iter().any(|l| l == ":do { /interface wifi set [find where .id=*4] datapath.vlan-id=10 } on-error={}"));
        assert!(u.iter().any(|l| l.contains("security.passphrase=\"old home pass\"")));
    }

    #[test]
    fn a_blank_access_point_gets_the_whole_state() {
        let st = import(&live(), "ap-hall", None);
        let mut blank = live();
        let wifi: Vec<Value> = rows(&blank, "wifi").iter().filter(|r| !s(r, "radio-mac").is_empty()).cloned().map(|mut r| {
            for k in ["configuration", "configuration.ssid", "security.authentication-types", "datapath.vlan-id", "datapath.bridge", "secret"] {
                r.as_object_mut().unwrap().remove(k);
            }
            r["disabled"] = json!("true");
            r
        }).collect();
        blank["wifi"] = json!(wifi);
        for k in ["cfg", "sec", "dp"] {
            blank[k] = json!([]);
        }
        blank["bvlan"] = json!([{"bridge": "bridge", "vlan-ids": "99", "current-tagged": "bridge,ether1"}, {"bridge": "bridge", "vlan-ids": "20", "current-untagged": "ether1"}]);
        let (c, u, _) = plan_state(&blank, &st, "ap-attic", Some(&up())).unwrap();
        let c = infra::show_all(&c);
        assert_eq!(c.len(), 8, "{c:#?}");
        assert!(c[0].contains("vlan-ids=10 tagged=ether1") && c[1].contains("vlan-ids=30 tagged=ether1"), "20 is already the uplink's untagged VLAN");
        assert!(c[2].starts_with("/interface/wifi/set .id=*3 configuration.ssid=Home ") && c[3].starts_with("/interface/wifi/set .id=*4 configuration.ssid=Home "));
        assert!(c[4].starts_with("/interface/wifi/add name=wifi1-guests master-interface=wifi1 ") && c[7].starts_with("/interface/wifi/add name=wifi2-iot "));
        assert!(c[2].ends_with("disabled=no") && c.iter().all(|l| !l.contains("old home pass")));
        assert!(u.iter().any(|l| l == ":do { /interface wifi set [find where .id=*3] disabled=yes } on-error={}"));
    }

    #[test]
    fn state_checks_vlan_lists_and_names() {
        check_state(&json!({"ssids": [{"ssid": "A", "vlan": "", "security": "", "passphrase": "", "bands": ["2.4"]}]})).unwrap();
        for (bad, why) in [
            (json!({"ssid": ""}), "1 to 32 bytes"),
            (json!({"ssid": "A", "vlan": "5000"}), "VLAN 1 to 4094"),
            (json!({"ssid": "A", "passphrase": "short"}), "8 to 63"),
            (json!({"ssid": "A", "bands": ["7"]}), "bands are"),
            (json!({"ssid": "A", "security": "wpa2 psk"}), "unknown security"),
        ] {
            let e = check_state(&json!({"ssids": [bad]})).unwrap_err();
            assert!(e.contains(why), "{e}");
        }
        assert!(check_state(&json!({"ssids": [{"ssid": "A"}, {"ssid": "A"}]})).unwrap_err().contains("twice"));
        assert_eq!(vlan_ids("10,30,100-102"), [10, 30, 100, 101, 102]);
        assert!(carries(&[json!({"vlan-ids": "20", "current-untagged": "ether1"})], 20, "ether1"));
        assert!(!carries(&[json!({"vlan-ids": "20", "current-tagged": "bridge"})], 20, "ether1"));
        assert_eq!([slug("Guests (5G)"), slug("Café"), slug("!!")], ["guests-5g", "caf", "ssid"]);
        let slow = json!({"topology": {"links": [{"a": "core", "a_via": "sfp5", "b": "ap-hall", "b_via": "ether1"}]}, "switches": [{"name": "core", "vlans": [{"vlan-ids": "10"}]}]});
        let u = uplink(&slow, "ap-hall").unwrap();
        assert_eq!((u.port.as_str(), u.sw.as_str(), u.sw_port.as_str(), u.sw_vlans.len()), ("ether1", "core", "sfp5", 1));
        assert!(uplink(&slow, "other").is_none());
    }

    #[test]
    fn first_setup_script() {
        let b = Boot { name: "ap-attic".into(), address: "10.0.99.15/24".into(), gateway: "10.0.99.1".into(), vlan: 99, uplink: "ether1".into(), pvid: Some(20),
            dns: "10.0.10.2,10.0.10.3".into(), country: "France".into(), f24: Some(2462), f5: Some(5220), read_user: "monitor".into(), app: "10.0.10.69".into(),
            key: "ssh-ed25519 AAAAC3Nza cairn-net".into(), extra: "/system clock set time-zone-name=Europe/Paris\n".into() };
        let t = render(&b);
        for l in [
            "/system identity set name=\"ap-attic\"",
            "/ip dns set servers=10.0.10.2,10.0.10.3",
            "/interface bridge port add bridge=bridge interface=ether1 pvid=20 comment=\"uplink\"",
            "/interface bridge vlan add bridge=bridge vlan-ids=20 untagged=ether1 comment=\"uplink\"",
            "/interface bridge vlan add bridge=bridge vlan-ids=99 tagged=bridge,ether1 comment=\"management\"",
            "/ip address add address=10.0.99.15/24 interface=mgmt99 comment=\"management\"",
            "  /interface wifi set [find where name=$i] configuration.country=\"France\" disabled=yes",
            "  :if ($b ~ \"2ghz\") do={ /interface wifi set [find where name=$i] channel.frequency=2462 channel.width=20mhz }",
            "/user add name=\"monitor\" group=cairn-read address=10.0.10.69/32 password=[:rndstr length=32] comment=\"Cairn Net: reads\"",
            "/user add name=cairn-write group=full address=10.0.10.69/32 password=[:rndstr length=32] comment=\"Cairn Net: writes\"",
            "/user ssh-keys add user=cairn-write key=\"ssh-ed25519 AAAAC3Nza cairn-net\"",
            "/system clock set time-zone-name=Europe/Paris",
        ] {
            assert!(t.lines().any(|x| x == l), "missing: {l}\n{t}");
        }
        assert_eq!(t.matches('{').count(), t.matches('}').count());
        assert!(t.find("time-zone-name").unwrap() < t.find("vlan-filtering=yes").unwrap(), "local additions before VLAN filtering");
        assert!(!t.contains("password=\""), "no password in the script");
        // the form is checked before anything else
        for (v, why) in [
            (json!({"name": "ap attic"}), "name"),
            (json!({"name": "ap", "address": "10.0.99.15"}), "address"),
            (json!({"name": "ap", "address": "10.0.99.15/24", "gateway": "10.0.99.1", "vlan": "99", "pvid": "99"}), "pvid"),
        ] {
            assert!(boot_from(&v).err().is_some_and(|e| e.starts_with(why)), "{v}");
        }
    }

    /* ---------- roaming, blocked devices, guests (lot Wi-Fi 4a) ---------- */

    #[test]
    fn roaming_settings_where_they_live() {
        let mut st = import(&live(), "ap-hall", None);
        let g = &mut st["ssids"][1];
        assert_eq!((g["ft"].as_bool(), g["kv"].as_bool(), g["band_steering"].as_bool()), (Some(false), Some(false), Some(false)), "none on the fixture");
        g["ft"] = json!(true);
        g["kv"] = json!(true);
        g["band_steering"] = json!(true);
        let (c, u, w) = st_plan(&st).unwrap();
        // 802.11r in the SSID's security profile; steering inline on its interfaces (no steering profile)
        assert_eq!(c[0], "/interface/wifi/security/set .id=*3 ft=yes");
        assert!(c.contains(&"/interface/wifi/set .id=*C steering.rrm=yes".to_string()) && c.contains(&"/interface/wifi/set .id=*E steering.2g-probe-delay=yes".to_string()), "{c:#?}");
        assert_eq!(c.len(), 7);
        assert!(u.iter().any(|l| l == ":do { /interface wifi security unset [find where .id=*3] value-name=ft } on-error={}"));
        assert!(u.iter().any(|l| l == ":do { /interface wifi unset [find where .id=*C] value-name=steering.wnm } on-error={}"));
        assert!(w.iter().any(|x| x == "Guests: some older devices cannot join with 802.11r: check them after applying"));
        // read back as applied: nothing left, and taken again from the device the state is the same
        let mut after = live();
        after["sec"][2]["ft"] = json!("yes");
        for r in after["wifi"].as_array_mut().unwrap().iter_mut().filter(|r| s(r, "configuration.ssid") == "Guests") {
            for k in ["security.ft", "steering.rrm", "steering.wnm", "steering.2g-probe-delay"] {
                r[k] = json!("yes");
            }
        }
        assert!(plan_state(&after, &st, "ap-hall", Some(&up())).unwrap().0.is_empty());
        assert_eq!(import(&after, "ap-hall", None)["ssids"][1], st["ssids"][1]);
        // a new SSID carries them inline; an open one cannot have 802.11r
        let mut st2 = st.clone();
        st2["ssids"][1]["ssid"] = json!("Lab");
        let c = st_plan(&st2).unwrap().0;
        assert!(c.iter().any(|l| l.starts_with("/interface/wifi/add name=wifi1-lab ") && l.contains("security.ft=yes steering.rrm=yes steering.wnm=yes steering.2g-probe-delay=yes")), "{c:#?}");
        st2["ssids"][1]["security"] = json!("");
        assert!(check_state(&st2).unwrap_err().contains("802.11r needs a password"));
    }

    #[test]
    fn devices_blocked_on_every_access_point() {
        let mut l = live();
        let reg = rows(&hall(), "reg").to_vec();
        let (on, id) = (s(&reg[0], "mac-address").to_uppercase(), s(&reg[0], ".id").to_string());
        l["reg"] = json!(reg);
        l["acl"] = json!([{".id": "*7", "mac-address": "02:00:00:00:00:99", "action": "accept", "comment": "printer"}]);
        let mut st = import(&l, "ap-hall", None);
        assert!(plan_state(&l, &st, "ap-hall", Some(&up())).unwrap().0.is_empty(), "nothing blocked yet");
        st["blocked"] = json!([{"mac": on, "name": "phone"}, {"mac": "02:00:00:00:00:42", "name": ""}]);
        check_state(&st).unwrap();
        let (c, u, w) = plan_state(&l, &st, "ap-hall", Some(&up())).unwrap();
        let c = infra::show_all(&c);
        assert_eq!(c, [
            format!("/interface/wifi/access-list/add mac-address={on} action=reject comment=\"cairn-net: blocked\" place-before=*7"),
            "/interface/wifi/access-list/add mac-address=02:00:00:00:00:42 action=reject comment=\"cairn-net: blocked\" place-before=*7".into(),
            format!("/interface/wifi/registration-table/remove .id={id}"),
        ]);
        assert_eq!(u[0], format!(":do {{ /interface wifi access-list remove [find where comment=\"cairn-net: blocked\" and mac-address={on}] }} on-error={{}}"));
        assert!(w.iter().any(|x| x.starts_with(&format!("{on}: blocked, disconnected from "))));
        // applied: the rows are there, the client is gone; unblocked: its row goes
        let mut l2 = l.clone();
        l2["reg"] = json!(reg[1..]);
        l2["acl"] = json!([{".id": "*8", "mac-address": on, "action": "reject", "comment": "cairn-net: blocked"},
            {".id": "*9", "mac-address": "02:00:00:00:00:42", "action": "reject", "comment": "cairn-net: blocked"}, l["acl"][0]]);
        assert!(plan_state(&l2, &st, "ap-hall", Some(&up())).unwrap().0.is_empty());
        st["blocked"] = json!([{"mac": "02:00:00:00:00:42", "name": ""}]);
        let (c, u, _) = plan_state(&l2, &st, "ap-hall", Some(&up())).unwrap();
        assert_eq!(infra::show_all(&c), ["/interface/wifi/access-list/remove .id=*8"]);
        assert_eq!(u[0], format!(":do {{ /interface wifi access-list add mac-address={on} action=reject comment=\"cairn-net: blocked\" }} on-error={{}}"));
        // the list is checked
        for (b, why) in [(json!([{"mac": "02:00:00:00:00:4"}]), "not a MAC"), (json!([{"mac": "02:00:00:00:00:4g"}]), "not a MAC"),
            (json!([{"mac": "02:00:00:00:00:42"}, {"mac": "02:00:00:00:00:42"}]), "twice")] {
            st["blocked"] = b;
            assert!(check_state(&st).unwrap_err().contains(why));
        }
    }

    #[test]
    fn guests_share_a_qr_code() {
        assert_eq!(wifi_qr_text("Guests (5G)", "wpa2-psk,wpa3-psk", "k7mp-2xqa-9fhn", false), "WIFI:T:WPA;S:Guests (5G);P:k7mp-2xqa-9fhn;;");
        assert_eq!(wifi_qr_text("a;b", "wpa2-psk", "p\\:\"x,", true), "WIFI:T:WPA;S:a\\;b;P:p\\\\\\:\\\"x\\,;H:true;;");
        assert_eq!(wifi_qr_text("Open", "", "", false), "WIFI:T:nopass;S:Open;;");
        let svg = qr_svg(&wifi_qr_text("Guests", "wpa2-psk", "k7mp-2xqa-9fhn", false)).unwrap();
        // version 3 (29 modules) and the quiet zone; the three finder patterns' corners
        assert!(svg.starts_with("<svg ") && svg.contains("viewBox=\"0 0 37 37\""), "{svg}");
        assert!(svg.contains("M4 4h1v1h-1z") && svg.contains("M32 4h1v1h-1z") && svg.contains("M4 32h1v1h-1z"));
        assert!(!svg.contains("style"), "CSP: no inline style");
        let p = guest_password().unwrap();
        assert_eq!((p.len(), p.matches('-').count()), (14, 2));
        assert!(p.chars().all(|c| c == '-' || "abcdefghjkmnpqrstuvwxyz23456789".contains(c)));
        passphrase_ok(&p).unwrap();
        assert_ne!(p, guest_password().unwrap());
    }

    #[test]
    fn read_back_and_rollback_job() {
        let mut st = import(&live(), "ap-hall", None);
        st["radios"]["ap-hall"]["wifi2"]["frequency"] = json!("5200");
        st["ssids"][1]["passphrase"] = json!("new guest pass");
        let (c, u, _) = plan_state(&live(), &st, "ap-hall", Some(&up())).unwrap();
        assert_eq!(c.len(), 2);
        let mut after = live();
        for r in after["wifi"].as_array_mut().unwrap() {
            if s(r, "name") == "wifi2" {
                r["channel.frequency"] = json!("5200");
            }
        }
        after["sec"][2]["secret"] = json!("new guest pass");
        assert!(plan_state(&after, &st, "ap-hall", Some(&up())).unwrap().0.is_empty(), "read back as asked: nothing left");

        // where there is no scheduler: the job, without the scheduler line; old password masked when shown
        let job = infra::arm_job(&u);
        assert_eq!(job.0, "/execute");
        assert!(job.1[0].1.contains("passphrase=\"guest\\\"pass\\$1\"") && !job.1[0].1.contains("scheduler"));
        let shown = crate::direct::show(&job);
        assert!(!shown.contains("guest") && shown.contains("passphrase=\"•••\""), "{shown}");
        assert!(infra::show_all(&c).iter().all(|l| !l.contains("new guest")));
    }

    /* ---------- SSID schedules (lot Wi-Fi 4b) ---------- */

    /// The fixture read at a given time of the access point's clock.
    fn at(date: &str, time: &str) -> Value {
        let mut l = live();
        l["clock"] = json!({"date": date, "time": time});
        l
    }
    fn guests_off(l: &mut Value) {
        for r in l["wifi"].as_array_mut().unwrap().iter_mut().filter(|r| s(r, "configuration.ssid") == "Guests") {
            r["disabled"] = json!("true");
        }
    }
    const DAYTIME: &str = "{\"days\": [1, 2, 3, 4, 5, 6, 7], \"from\": \"07:00\", \"to\": \"22:00\"}";

    #[test]
    fn days_and_the_access_points_clock() {
        assert_eq!([weekday(1970, 1, 1), weekday(2026, 11, 20), weekday(2000, 2, 29), weekday(2026, 3, 1)], [4, 5, 2, 7]);
        assert_eq!(clock_of(&at("2026-11-20", "10:22:33")), Some((5, 622)));
        assert_eq!(clock_of(&at("nov/22/2026", "23:59:00")), Some((7, 1439)), "RouterOS before 7.10");
        assert_eq!(clock_of(&live()), None, "no clock read");
        assert_eq!(clock_of(&at("2026-11-2x", "10:00:00")), None);
        assert_eq!([hm("07:30"), hm("7:30"), hm("24:00"), hm("")], [Some(450), None, None, None]);
    }

    #[test]
    fn on_air_by_its_schedule() {
        let x = |days: &[u32], from: &str, to: &str| json!({"schedule": {"days": days, "from": from, "to": to}});
        let week = x(&[1, 2, 3, 4, 5], "07:00", "22:00");
        assert!(on_air(&week, Some((5, 7 * 60))) && on_air(&week, Some((5, 21 * 60 + 59))));
        assert!(!on_air(&week, Some((5, 22 * 60))) && !on_air(&week, Some((5, 6 * 60 + 59))) && !on_air(&week, Some((6, 12 * 60))));
        // past midnight: from the evening of a day to the morning after
        let night = x(&[5, 7], "22:00", "07:00");
        assert!(on_air(&night, Some((5, 23 * 60))) && on_air(&night, Some((6, 6 * 60 + 59))) && on_air(&night, Some((1, 3 * 60))), "Sunday night into Monday");
        assert!(!on_air(&night, Some((6, 7 * 60))) && !on_air(&night, Some((5, 3 * 60))) && !on_air(&night, Some((6, 23 * 60))));
        // from = to: the whole day
        let sat = x(&[6], "00:00", "00:00");
        assert!(on_air(&sat, Some((6, 0))) && on_air(&sat, Some((6, 1439))) && !on_air(&sat, Some((7, 0))));
        // no schedule, or no clock: as the state says
        assert!(on_air(&json!({}), Some((1, 0))) && on_air(&week, None));
    }

    #[test]
    fn outside_its_schedule_an_ssid_is_off() {
        let mut st = import(&live(), "ap-hall", None);
        st["ssids"][1]["schedule"] = serde_json::from_str(DAYTIME).unwrap();
        check_state(&st).unwrap();
        let p = |l: &Value, st: &Value| plan_state(l, st, "ap-hall", Some(&up())).map(|(c, _, w)| (infra::show_all(&c), w)).unwrap();
        assert!(p(&at("2026-11-20", "12:00:00"), &st).0.is_empty(), "on air: as it is");
        assert_eq!(effective(&st, Some((5, 12 * 60))), st);
        // the card and drift want it off outside its schedule
        let night = at("2026-11-20", "23:30:00");
        let (c, w) = p(&night, &st);
        assert_eq!(c, ["/interface/wifi/set .id=*C disabled=yes", "/interface/wifi/set .id=*E disabled=yes"]);
        assert!(w.iter().any(|x| x == "wifi1-guest: off, its clients are cut"));
        assert_eq!(effective(&st, Some((5, 23 * 60)))["ssids"][1]["bands"], json!([]));
        // what the schedule runs by itself: the interfaces' switch only, undone by the rollback
        let t = toggles(&night, &st);
        assert_eq!(t, [json!({"name": "wifi1-guest", "disabled": true}), json!({"name": "wifi2-guest", "disabled": true})]);
        let (c, u, _) = changes(&night, &json!({"ifaces": t})).unwrap();
        assert_eq!(infra::show_all(&c), ["/interface/wifi/set .id=*C disabled=yes", "/interface/wifi/set .id=*E disabled=yes"]);
        assert!(u.contains(&":do { /interface wifi set [find where .id=*E] disabled=no } on-error={}".to_string()), "{u:?}");
        assert!(toggles(&at("2026-11-20", "12:00:00"), &st).is_empty());
        // read back once off: nothing left; 07:00 turns them on again
        let mut off = night.clone();
        guests_off(&mut off);
        assert!(toggles(&off, &st).is_empty() && p(&off, &st).0.is_empty());
        off["clock"]["time"] = json!("07:00:00");
        assert_eq!(toggles(&off, &st).len(), 2);
        // drift in the minute after a boundary: judged as switched, nothing to say
        assert_eq!(p(&off, &st).0.len(), 2);
        assert!(p(&switched(&off, &st), &st).0.is_empty() && p(&switched(&night, &st), &st).0.is_empty());
        // a band the state leaves out is not turned on by the schedule ("Apply the state" does that)
        st["ssids"][1]["bands"] = json!(["5"]);
        assert_eq!(toggles(&off, &st), [json!({"name": "wifi2-guest", "disabled": false})]);
        // SSIDs without a schedule are never touched
        st["ssids"][1].as_object_mut().unwrap().remove("schedule");
        assert!(toggles(&off, &st).is_empty() && toggles(&night, &st).is_empty());
    }

    #[test]
    fn schedules_checked_and_kept() {
        let one = |w: Value| check_state(&json!({"ssids": [{"ssid": "Guests", "bands": ["5"], "schedule": w}]}));
        one(json!({"days": [6, 7], "from": "22:00", "to": "07:00"})).unwrap();
        one(Value::Null).unwrap();
        for bad in [json!({"days": [], "from": "07:00", "to": "22:00"}), json!({"days": [0], "from": "07:00", "to": "22:00"}),
            json!({"days": [8], "from": "07:00", "to": "22:00"}), json!({"days": ["1"], "from": "07:00", "to": "22:00"}),
            json!({"days": [1], "from": "7:00", "to": "22:00"}), json!({"days": [1], "from": "07:00"}), json!("07:00-22:00")] {
            assert_eq!(one(bad.clone()).unwrap_err(), "Guests: a schedule is days 1 (Monday) to 7, from and to as HH:MM", "{bad}");
        }
        // taken again from an access point: the schedule stays, and an SSID off by it keeps its bands
        let mut old = import(&live(), "ap-hall", None);
        old["ssids"][1]["schedule"] = serde_json::from_str(DAYTIME).unwrap();
        let mut night = at("2026-11-20", "23:30:00");
        guests_off(&mut night);
        let st = import(&night, "ap-hall", Some(&old));
        assert_eq!(st["ssids"][1]["schedule"], old["ssids"][1]["schedule"]);
        assert_eq!(strs(&st["ssids"][1]["bands"]), ["2.4", "5"]);
        assert!(st["ssids"][0].get("schedule").is_none());
        // inside its schedule, the bands are read as they are
        night["clock"]["time"] = json!("12:00:00");
        assert_eq!(import(&night, "ap-hall", Some(&old))["ssids"][1]["bands"], json!([]));
    }
}
