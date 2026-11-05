//! Tools page (WinBox's Tools menu), run from one switch: ping, traceroute,
//! torch, bandwidth test, IP scan and packet capture stream their results to
//! a run the page polls every second; netwatch probes are listed, added and
//! removed (direct mode); interface graphs are read.
//!
//! Each run opens its own API connection and closing it ends the command on
//! the switch (RouterOS stops a session's commands with the session). A run
//! ends at its maximum duration, on Stop, or once the page stops polling
//! (tab closed). The capture is the exception, the sniffer runs on its own:
//! a switch-side scheduler stops it even if Cairn Net goes away.
//! One heavy run (bandwidth test, capture) at a time per switch.

use crate::infra::q;
use crate::settings::{self, SwitchCfg};
use crate::{get_len, print, with_api, Api};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

const UNSEEN_S: u64 = 15; // the page stopped polling: its run is stopped
const KEEP_S: u64 = 600; // finished runs (and their capture file) kept that long
const ROWS: usize = 600;
const PER_SWITCH: usize = 4;
pub const SNIFFER_STOP: &str = "cairn-net-sniffer";

/// Rights (RouterOS group policies) per tool; `true` = writes to the switch
/// (write account and direct mode).
pub const TOOLS: [(&str, &[&str], bool); 8] = [
    ("ping", &["test"], false),
    ("traceroute", &["test"], false),
    ("torch", &["test"], false),
    ("btest", &["test"], false),
    ("scan", &["test"], false),
    ("sniffer", &["sniff", "write", "ftp"], true),
    ("netwatch", &["write"], true),
    ("graphing", &["read"], false),
];

struct Job {
    switch: String,
    tool: String,
    account: String,
    commands: Vec<String>,
    heavy: Vec<String>, // switches this run loads
    sections: bool,
    rows: Vec<Value>, // sectioned tools: the last complete section
    cur: Vec<Value>,
    section: String,
    done: bool,
    error: Option<String>,
    stopped: Option<&'static str>,
    started: Instant,
    seen: Instant,
    max: Duration,
    stop: Arc<AtomicBool>,
    sock: Option<TcpStream>,
    pcap: Option<Vec<u8>>,
}

static JOBS: Mutex<BTreeMap<u64, Job>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);
static REAPER: Once = Once::new();

fn jobs() -> std::sync::MutexGuard<'static, BTreeMap<u64, Job>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/* ---------- checks on what the page sends ---------- */

fn text<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("").trim()
}

fn num(a: &Value, k: &str, def: u64, min: u64, max: u64) -> Result<u64, String> {
    let n = match &a[k] {
        Value::Null => return Ok(def),
        Value::String(s) if s.trim().is_empty() => return Ok(def),
        v => v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).ok_or(format!("{k}: not a number"))?,
    };
    if n < min || n > max {
        return Err(format!("{k}: between {min} and {max}"));
    }
    Ok(n)
}

fn host_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 253 && !s.starts_with(['-', '.']) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn iface_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.chars().any(|c| c.is_control() || c == '=')
}

fn prefix(s: &str) -> Option<(Ipv4Addr, u32)> {
    let (ip, len) = s.split_once('/').unwrap_or((s, "32"));
    Some((ip.parse().ok()?, len.parse().ok().filter(|n| *n <= 32)?))
}

fn port_ok(s: &str) -> bool {
    s == "any" || s.parse::<u16>().is_ok_and(|p| p > 0)
}

fn proto_ok(s: &str) -> bool {
    s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Number of addresses of "a.b.c.d/n" or "a.b.c.d-e.f.g.h".
fn range_size(s: &str) -> Option<u64> {
    if let Some((a, b)) = s.split_once('-') {
        let (a, b) = (u32::from(a.trim().parse::<Ipv4Addr>().ok()?), u32::from(b.trim().parse::<Ipv4Addr>().ok()?));
        return (b >= a).then(|| (b - a) as u64 + 1);
    }
    s.contains('/').then(|| prefix(s)).flatten().map(|(_, n)| 1u64 << (32 - n))
}

/* ---------- what runs ---------- */

pub struct Target {
    pub ip: String,
    pub user: String,
    pub password: String,
    pub peer: Option<String>, // a switch Cairn Net manages: it carries the load too
    pub clean: Option<(SwitchCfg, String, String)>, // the server end, if Cairn Net manages it: account to clean it with
}

pub struct Run {
    pub words: Vec<String>,
    pub max_s: u64,
    pub sections: bool,
    pub heavy: bool,
}

/// The API command of a streaming tool, every value checked.
pub fn build(tool: &str, a: &Value, target: Option<&Target>) -> Result<Run, String> {
    let mut w: Vec<String> = Vec::new();
    let opt = |w: &mut Vec<String>, k: &str, v: &str, ok: bool| -> Result<(), String> {
        if v.is_empty() {
            return Ok(());
        }
        if !ok {
            return Err(format!("{k}: invalid value"));
        }
        w.push(format!("={k}={v}"));
        Ok(())
    };
    let address = text(a, "address");
    let iface = text(a, "interface");
    let run = |words: Vec<String>, max_s: u64, sections: bool, heavy: bool| -> Result<Run, String> { Ok(Run { words, max_s, sections, heavy }) };
    match tool {
        "ping" => {
            if !host_ok(address) {
                return Err("address: an IPv4 address or a name".into());
            }
            let count = num(a, "count", 5, 1, 100)?;
            w.push("/ping".into());
            w.push(format!("=address={address}"));
            w.push(format!("=count={count}"));
            w.push(format!("=size={}", num(a, "size", 56, 28, 10000)?));
            opt(&mut w, "interface", iface, iface_ok(iface))?;
            run(w, count + 5, false, false)
        }
        "traceroute" => {
            if !host_ok(address) {
                return Err("address: an IPv4 address or a name".into());
            }
            w.push("/tool/traceroute".into());
            w.push(format!("=address={address}"));
            w.push(format!("=count={}", num(a, "count", 3, 1, 10)?));
            w.push(format!("=max-hops={}", num(a, "max_hops", 30, 1, 64)?));
            opt(&mut w, "interface", iface, iface_ok(iface))?;
            run(w, 90, true, false)
        }
        "torch" => {
            if !iface_ok(iface) {
                return Err("interface: required".into());
            }
            let d = num(a, "duration", 30, 1, 300)?;
            w.push("/tool/torch".into());
            w.push(format!("=interface={iface}"));
            w.push(format!("=duration={d}s"));
            // a field given = traffic split (and filtered) by it, as in WinBox
            for k in ["src-address", "dst-address"] {
                let v = text(a, k);
                opt(&mut w, k, v, prefix(v).is_some())?;
            }
            let (port, proto) = (text(a, "port"), text(a, "ip-protocol"));
            opt(&mut w, "port", port, port_ok(port))?;
            opt(&mut w, "ip-protocol", proto, proto_ok(proto))?;
            run(w, d + 5, true, false)
        }
        "btest" => {
            let t = target.ok_or("no target")?;
            let d = num(a, "duration", 10, 1, 30)?;
            let speed = num(a, "speed", 100, 1, 1000)?;
            let proto = match text(a, "protocol") { "" | "tcp" => "tcp", "udp" => "udp", _ => return Err("protocol: tcp or udp".into()) };
            let dir = match text(a, "direction") { "" | "both" => "both", "transmit" => "transmit", "receive" => "receive", _ => return Err("direction: transmit, receive or both".into()) };
            w.push("/tool/bandwidth-test".into());
            w.push(format!("=address={}", t.ip));
            w.push(format!("=duration={d}s"));
            w.push(format!("=protocol={proto}"));
            w.push(format!("=direction={dir}"));
            w.push(format!("=local-tx-speed={speed}M"));
            w.push(format!("=remote-tx-speed={speed}M"));
            if !t.user.is_empty() {
                w.push(format!("=user={}", t.user));
                w.push(format!("=password={}", t.password));
            }
            run(w, d + 10, false, true)
        }
        "scan" => {
            let r = text(a, "range");
            match range_size(r) {
                Some(n) if n <= 4096 => {}
                Some(_) => return Err("range: 4096 addresses at most (a /20)".into()),
                None => return Err("range: a.b.c.d/n or a.b.c.d-e.f.g.h".into()),
            }
            let d = num(a, "duration", 20, 1, 120)?;
            w.push("/tool/ip-scan".into());
            w.push(format!("=address-range={r}"));
            w.push(format!("=duration={d}s"));
            opt(&mut w, "interface", iface, iface_ok(iface))?;
            run(w, d + 5, true, false)
        }
        _ => Err(format!("{tool}: unknown tool")),
    }
}

/// Words as shown on the page (password hidden).
fn show(words: &[String]) -> String {
    let mut out = words[0].clone();
    for x in &words[1..] {
        let (k, v) = x[1..].split_once('=').unwrap_or((x, ""));
        let v = if k == "password" { "********" } else { v };
        out += &format!(" {k}={}", if v.is_empty() || v.contains(' ') { format!("\"{v}\"") } else { v.to_string() });
    }
    out
}

/* ---------- rights ---------- */

/// The policies of an account's group (what RouterOS lets it do).
fn policies(api: &mut Api, user: &str) -> Result<Vec<String>, String> {
    let Some(u) = print(api, "/user", "group", &[&format!("?name={user}")])?.pop() else { return Ok(vec![]) };
    let g = u["group"].as_str().unwrap_or("");
    let p = print(api, "/user/group", "policy", &[&format!("?name={g}")])?.pop().unwrap_or_default();
    Ok(p["policy"].as_str().unwrap_or("").split(',').filter(|x| !x.is_empty() && !x.starts_with('!')).map(String::from).collect())
}

/// Which account runs a tool: the read account when it may, else the write
/// account; otherwise what each one lacks.
pub fn rights(read: &[String], write: Option<&[String]>, need: &[&str], writes: bool, direct: bool) -> Value {
    let lacks = |p: &[String]| need.iter().filter(|n| !p.iter().any(|x| x == *n)).copied().collect::<Vec<_>>();
    let (r, w) = (lacks(read), write.map(lacks));
    let account = if !writes && r.is_empty() {
        Some("read")
    } else if w.as_ref().is_some_and(Vec::is_empty) && (!writes || direct) {
        Some("write")
    } else {
        None
    };
    json!({"ok": account.is_some(), "account": account, "need": need, "read_lacks": if writes { json!(null) } else { json!(r) },
        "write_lacks": w, "direct_needed": writes && !direct})
}

fn tool_need(tool: &str) -> Option<(&'static [&'static str], bool)> {
    TOOLS.iter().find(|t| t.0 == tool).map(|t| (t.1, t.2))
}

/// Read and write accounts' policies and the run's account (user, password).
fn account(c: &SwitchCfg, sw: &settings::Switch, tool: &str) -> Result<(String, String, &'static str), String> {
    let (need, writes) = tool_need(tool).ok_or(format!("{tool}: unknown tool"))?;
    let (rp, wp) = with_api(sw, |api| Ok((policies(api, &c.user)?, if c.write_user.is_empty() { None } else { Some(policies(api, &c.write_user)?) })))?;
    let r = rights(&rp, wp.as_deref(), need, writes, c.mode == "direct");
    match r["account"].as_str() {
        Some("read") => Ok((c.user.clone(), c.password.clone(), "read")),
        Some(_) => Ok((c.write_user.clone(), c.write_password.clone(), "write")),
        None if r["direct_needed"] == true => Err(format!("{}: writes only in direct mode", c.name)),
        None => Err(format!("{tool} needs the right(s) {} (read account lacks {}, write account {})", need.join(", "),
            r["read_lacks"].as_array().map_or("-".into(), |v| v.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
            r["write_lacks"].as_array().map_or("absent".into(), |v| format!("lacks {}", v.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))))),
    }
}

/* ---------- page: what the switch offers ---------- */

pub fn view(name: &str) -> Value {
    let Some(sw) = settings::get(name) else { return json!({"error": format!("{name}: unknown switch")}) };
    let c = &sw.cfg;
    let mut v = with_api(&sw, |api| {
        let rp = policies(api, &c.user)?;
        let wp = if c.write_user.is_empty() { None } else { Some(policies(api, &c.write_user)?) };
        let r: serde_json::Map<String, Value> = TOOLS.iter().map(|(t, need, writes)| (t.to_string(), rights(&rp, wp.as_deref(), need, *writes, c.mode == "direct"))).collect();
        let www: Vec<Value> = print(api, "/ip/service", "name,port,disabled", &[])?.into_iter().filter(|s| matches!(s["name"].as_str(), Some("www" | "www-ssl"))).collect();
        Ok(json!({
            "read_policy": rp, "write_policy": wp, "rights": r,
            "interfaces": print(api, "/interface", "name,type,running,disabled", &[])?,
            "addresses": print(api, "/ip/address", "address,interface", &[])?,
            "netwatch": print(api, "/tool/netwatch", ".id,host,type,interval,port,status,since,comment,disabled,up-script,down-script", &[])?,
            "graphing": print(api, "/tool/graphing/interface", ".id,interface,allow-address,store-on-disk,disabled", &[])?,
            "graphing_resource": print(api, "/tool/graphing/resource", ".id,allow-address,store-on-disk,disabled", &[])?,
            "www": www,
        }))
    }).unwrap_or_else(|e| json!({"error": e}));
    v["name"] = json!(c.name);
    v["host"] = json!(c.host);
    v["mode"] = json!(c.mode);
    v["peers"] = json!(settings::all().iter().filter(|s| s.cfg.name != c.name).map(|s| s.cfg.name.clone()).collect::<Vec<_>>());
    v["runs"] = json!(jobs().iter().filter(|(_, j)| j.switch == c.name && (!j.done || j.pcap.is_some())).map(|(id, j)| json!({"id": id, "tool": j.tool, "done": j.done})).collect::<Vec<_>>());
    v
}

/* ---------- runs ---------- */

fn reaper() {
    REAPER.call_once(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(1));
            let mut g = jobs();
            for j in g.values_mut().filter(|j| !j.done) {
                if j.seen.elapsed() > Duration::from_secs(UNSEEN_S) {
                    halt(j, "page closed");
                } else if j.started.elapsed() > j.max + Duration::from_secs(10) {
                    halt(j, "maximum duration");
                }
            }
            g.retain(|_, j| !j.done || j.seen.elapsed() < Duration::from_secs(KEEP_S));
        });
    });
}

fn halt(j: &mut Job, why: &'static str) {
    if j.stopped.is_none() {
        j.stopped = Some(why);
    }
    j.stop.store(true, Ordering::SeqCst);
    if let Some(s) = &j.sock {
        let _ = s.shutdown(Shutdown::Both); // the reading thread wakes up, the switch ends the command
    }
}

fn add_row(id: u64, row: Value) {
    let mut g = jobs();
    let Some(j) = g.get_mut(&id) else { return };
    if j.sections {
        let s = row[".section"].as_str().unwrap_or("").to_string();
        if s != j.section && !j.cur.is_empty() {
            j.rows = std::mem::take(&mut j.cur);
        }
        j.section = s;
        j.cur.push(row);
    } else if j.rows.len() < ROWS {
        j.rows.push(row);
    }
}

fn finish(id: u64, r: Result<(), String>) {
    let mut g = jobs();
    let Some(j) = g.get_mut(&id) else { return };
    if j.sections && !j.cur.is_empty() && (j.rows.is_empty() || r.is_ok()) {
        j.rows = std::mem::take(&mut j.cur);
    }
    j.done = true;
    j.sock = None;
    if j.stopped.is_none() {
        j.error = r.err();
    }
    eprintln!("tool {} on {} ended{}{}", j.tool, j.switch, j.stopped.map_or(String::new(), |s| format!(" ({s})")), j.error.as_ref().map_or(String::new(), |e| format!(": {e}")));
}

/// A switch-side command whose `!re` replies are rows, until `!done`.
fn stream(id: u64, c: &SwitchCfg, user: &str, pass: &str, words: &[String]) -> Result<(), String> {
    let mut api = Api::connect(c, user, pass)?;
    {
        let mut g = jobs();
        let j = g.get_mut(&id).ok_or("gone")?;
        if j.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        j.sock = Some(api.0.get_ref().try_clone().map_err(|e| e.to_string())?);
    }
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    api.0.write_all(&crate::encode(&w)).map_err(|e| e.to_string())?;
    let mut trap = None;
    loop {
        let s = match api.sentence() {
            Ok(s) => s,
            Err(_) if jobs().get(&id).is_some_and(|j| j.stop.load(Ordering::SeqCst)) => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let row: serde_json::Map<String, Value> = s.iter().skip(1).filter_map(|w| w.strip_prefix('=')?.split_once('='))
            .map(|(k, v)| (k.to_string(), json!(v))).collect();
        match s.first().map(String::as_str) {
            Some("!re") => add_row(id, Value::Object(row)),
            Some("!trap") => trap = Some(row.get("message").and_then(Value::as_str).unwrap_or("trap").to_string()),
            Some("!fatal") => return Err(s.get(1).cloned().unwrap_or_else(|| "fatal".into())),
            Some("!done") => return trap.map_or(Ok(()), Err),
            _ => {}
        }
    }
}

fn register(c: &SwitchCfg, tool: &str, account: &str, commands: Vec<String>, heavy: Vec<String>, sections: bool, max_s: u64) -> Result<(u64, Arc<AtomicBool>), String> {
    reaper();
    let mut g = jobs();
    let live: Vec<&Job> = g.values().filter(|j| !j.done).collect();
    if let Some(j) = live.iter().find(|j| j.heavy.iter().any(|h| heavy.contains(h))) {
        return Err(format!("a {} already loads {}: one bandwidth test or capture at a time", if j.tool == "btest" { "bandwidth test" } else { "capture" },
            j.heavy.iter().find(|h| heavy.contains(h)).map_or("", String::as_str)));
    }
    if live.iter().filter(|j| j.switch == c.name).count() >= PER_SWITCH {
        return Err(format!("{PER_SWITCH} runs already on {}: stop one first", c.name));
    }
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    g.insert(id, Job {
        switch: c.name.clone(), tool: tool.into(), account: account.into(), commands, heavy, sections, rows: vec![], cur: vec![],
        section: String::new(), done: false, error: None, stopped: None, started: Instant::now(), seen: Instant::now(),
        max: Duration::from_secs(max_s), stop: stop.clone(), sock: None, pcap: None,
    });
    Ok((id, stop))
}

fn btest_target(c: &SwitchCfg, a: &Value, user: &str, pass: &str) -> Result<Target, String> {
    match text(a, "target") {
        "" | "self" => {
            let (u, p) = if c.write_user.is_empty() { (user, pass) } else { (c.write_user.as_str(), c.write_password.as_str()) };
            Ok(Target { ip: "127.0.0.1".into(), user: user.into(), password: pass.into(), peer: None, clean: Some((c.clone(), u.into(), p.into())) })
        }
        "switch" => {
            let p = settings::get(text(a, "peer")).ok_or("unknown switch")?;
            let ip = (p.cfg.host.as_str(), 0).to_socket_addrs().map_err(|e| e.to_string())?
                .find_map(|x| match x.ip() { std::net::IpAddr::V4(v) => Some(v), _ => None }).ok_or("the switch has no IPv4 address")?;
            if p.cfg.name == c.name {
                return Err("pick another switch, or the switch itself (local)".into());
            }
            let (u, pw) = if p.cfg.write_user.is_empty() { (&p.cfg.user, &p.cfg.password) } else { (&p.cfg.write_user, &p.cfg.write_password) };
            Ok(Target { ip: ip.to_string(), user: u.clone(), password: pw.clone(), peer: Some(p.cfg.name.clone()), clean: Some((p.cfg.clone(), u.clone(), pw.clone())) })
        }
        "address" => {
            let ip = text(a, "address");
            ip.parse::<Ipv4Addr>().map_err(|_| "address: an IPv4 address")?;
            let u = text(a, "user");
            if u.contains(['=', '\n']) {
                return Err("user: invalid".into());
            }
            Ok(Target { ip: ip.into(), user: u.into(), password: a["password"].as_str().unwrap_or("").into(), peer: None, clean: None })
        }
        _ => Err("target: self, switch or address".into()),
    }
}

pub fn start(v: &Value, user: &str) -> Result<Value, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    let c = sw.cfg.clone();
    let tool = v["tool"].as_str().unwrap_or("");
    let a = &v["args"];
    let (u, p, acct) = account(&c, &sw, tool)?;
    if tool == "sniffer" {
        let w = sniffer_want(a)?;
        let (id, stop) = register(&c, tool, acct, sniffer_show(&w), vec![c.name.clone()], false, w.duration + 30)?;
        eprintln!("tool sniffer started on {} by {user}", c.name);
        std::thread::spawn(move || {
            let r = sniff(id, &c, &w, &stop);
            finish(id, r);
        });
        return Ok(json!({"ok": true, "id": id, "account": acct}));
    }
    if matches!(tool, "netwatch" | "graphing") {
        return Err(format!("{tool} does not run"));
    }
    let target = if tool == "btest" { Some(btest_target(&c, a, &u, &p)?) } else { None };
    let run = build(tool, a, target.as_ref())?;
    let mut heavy = vec![];
    if run.heavy {
        let t = target.as_ref().unwrap();
        // leaving the switch: links get loaded, so the page asks twice and sends the name typed
        if t.ip != "127.0.0.1" && v["confirm"].as_str() != Some(c.name.as_str()) {
            return Err("type the switch's name to confirm the load".into());
        }
        heavy.push(c.name.clone());
        heavy.extend(t.peer.clone());
    }
    let (id, _) = register(&c, tool, acct, vec![show(&run.words)], heavy, run.sections, run.max_s)?;
    eprintln!("tool {tool} started on {} by {user}: {}", c.name, show(&run.words));
    std::thread::spawn(move || {
        let r = stream(id, &c, &u, &p, &run.words);
        if let Some(t) = &target {
            btest_cleanup(t);
        }
        finish(id, r);
    });
    Ok(json!({"ok": true, "id": id, "account": acct}))
}

/// RouterOS can leave the server end of a test sending once the client is
/// gone (seen with UDP towards the switch itself, CPU kept busy): the
/// server's sessions of the test's user are removed.
fn btest_cleanup(t: &Target) {
    let Some((c, user, pass)) = &t.clean else { return };
    std::thread::sleep(Duration::from_secs(2));
    let r = Api::connect(c, user, pass).and_then(|mut api| {
        let ids: Vec<String> = print(&mut api, "/tool/bandwidth-server/session", ".id,user", &[])?.into_iter()
            .filter(|s| s["user"] == t.user.as_str()).filter_map(|s| s[".id"].as_str().map(String::from)).collect();
        for id in &ids {
            api.call(&["/tool/bandwidth-server/session/remove", &format!("=.id={id}")])?;
        }
        Ok(ids.len())
    });
    match r {
        Ok(0) => {}
        Ok(n) => eprintln!("btest: {n} server session(s) left on {} removed", c.name),
        Err(e) => eprintln!("btest: server sessions on {} not checked: {e}", c.name),
    }
}

pub fn stop(v: &Value) -> Value {
    match v["id"].as_u64().and_then(|id| jobs().get_mut(&id).map(|j| halt(j, "stopped"))) {
        Some(()) => json!({"ok": true}),
        None => json!({"ok": false, "error": "no such run"}),
    }
}

/// One poll: also tells the server the page is still there.
pub fn poll(id: u64) -> Value {
    let mut g = jobs();
    let Some(j) = g.get_mut(&id) else { return json!({"error": "no such run (finished long ago, or Cairn Net restarted)"}) };
    j.seen = Instant::now();
    let rows = if j.sections && j.rows.is_empty() { &j.cur } else { &j.rows };
    json!({"id": id, "switch": j.switch, "tool": j.tool, "account": j.account, "commands": j.commands, "rows": rows, "done": j.done,
        "error": j.error, "stopped": j.stopped, "elapsed": j.started.elapsed().as_secs(), "max": j.max.as_secs(), "pcap": j.pcap.as_ref().map(Vec::len)})
}

/// The capture file and its name (switch, time).
pub fn pcap(id: u64) -> Option<(Vec<u8>, String)> {
    let g = jobs();
    let j = g.get(&id)?;
    Some((j.pcap.clone()?, format!("{}-{}.pcap", j.switch, crate::now_ms() / 1000)))
}

/* ---------- capture (/tool/sniffer) ---------- */

pub struct Sniff {
    pub duration: u64,
    pub limit_kib: u64,
    pub filters: Vec<(&'static str, String)>,
}

pub fn sniffer_want(a: &Value) -> Result<Sniff, String> {
    let mut filters = vec![];
    for (k, key, ok) in [
        ("interface", "filter-interface", iface_ok as fn(&str) -> bool),
        ("ip-address", "filter-ip-address", |s: &str| prefix(s).is_some()),
        ("port", "filter-port", |s: &str| s.parse::<u16>().is_ok_and(|p| p > 0)),
        ("ip-protocol", "filter-ip-protocol", proto_ok),
    ] {
        let v = text(a, k);
        if !v.is_empty() && !ok(v) {
            return Err(format!("{k}: invalid value"));
        }
        filters.push((key, v.to_string()));
    }
    Ok(Sniff { duration: num(a, "duration", 10, 1, 60)?, limit_kib: num(a, "limit", 1024, 16, 4096)?, filters })
}

fn sniffer_show(w: &Sniff) -> Vec<String> {
    let f: String = w.filters.iter().map(|(k, v)| format!(" {k}={}", if v.is_empty() { "\"\"".into() } else { v.clone() })).collect();
    vec![format!("/tool/sniffer/set file-name=cairn-net-<run>.pcap file-limit={}KiB{f}", w.limit_kib),
        format!("/system/scheduler/add name={SNIFFER_STOP} interval={}s (stops the capture if Cairn Net goes away)", w.duration + 20),
        "/tool/sniffer/start".into(), format!("… {} s …", w.duration), "/tool/sniffer/stop".into(),
        "/file/read (download), /file/remove, sniffer settings put back".into()]
}

fn call(api: &mut Api, w: &[String]) -> Result<Vec<Value>, String> {
    api.call(&w.iter().map(String::as_str).collect::<Vec<_>>())
}

/// A whole file through /file/read (32 KiB chunks, binary words).
fn read_file(api: &mut Api, name: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let w = ["/file/read".to_string(), format!("=file={name}"), format!("=offset={}", out.len()), "=chunk-size=32768".into()];
        api.0.write_all(&crate::encode(&w.iter().map(String::as_str).collect::<Vec<_>>())).map_err(|e| e.to_string())?;
        let (mut got, mut trap) = (0, None);
        loop {
            let mut words: Vec<Vec<u8>> = Vec::new();
            loop {
                let first = api.byte().map_err(|e| e.to_string())?;
                let len = get_len(first, || api.byte()).map_err(|e| e.to_string())? as usize;
                if len == 0 {
                    break;
                }
                let mut b = vec![0u8; len];
                api.0.read_exact(&mut b).map_err(|e| e.to_string())?;
                words.push(b);
            }
            match words.first().map(Vec::as_slice) {
                Some(b"!re") => {
                    for x in &words[1..] {
                        if let Some(d) = x.strip_prefix(b"=data=") {
                            got += d.len();
                            out.extend_from_slice(d);
                        }
                    }
                }
                Some(b"!trap") => trap = Some(words.iter().find_map(|x| x.strip_prefix(b"=message=")).map_or("trap".into(), |m| String::from_utf8_lossy(m).into_owned())),
                Some(b"!done") => break,
                Some(b"!fatal") => return Err("fatal".into()),
                _ => {}
            }
        }
        if let Some(t) = trap {
            return Err(t);
        }
        if got == 0 || out.len() > 8 << 20 {
            return Ok(out);
        }
    }
}

fn sniff(id: u64, c: &SwitchCfg, w: &Sniff, stop: &AtomicBool) -> Result<(), String> {
    let mut api = Api::connect(c, &c.write_user, &c.write_password)?;
    let old = api.call(&["/tool/sniffer/print"])?.pop().unwrap_or_default();
    if old["running"] == "true" {
        return Err("a capture is already running on the switch".into());
    }
    let file = format!("cairn-net-{id}.pcap");
    let mut set = vec!["/tool/sniffer/set".to_string(), format!("=file-name={file}"), format!("=file-limit={}", w.limit_kib)];
    set.extend(w.filters.iter().map(|(k, v)| format!("={k}={v}")));
    // put back afterwards: what the capture changed
    let keys: Vec<&str> = ["file-name", "file-limit"].into_iter().chain(w.filters.iter().map(|(k, _)| *k)).collect();
    let old_of = |k: &str| old[k].as_str().unwrap_or("").to_string();
    let mut back = vec!["/tool/sniffer/set".to_string()];
    back.extend(keys.iter().map(|k| format!("={k}={}", old_of(k))));
    let back_cli: String = keys.iter().map(|k| format!(" {k}={}", q(&old_of(k)))).collect();
    for s in api.call(&["/system/scheduler/print", "=.proplist=.id", &format!("?name={SNIFFER_STOP}")])? {
        api.call(&["/system/scheduler/remove", &format!("=.id={}", s[".id"].as_str().unwrap_or(""))])?;
    }
    call(&mut api, &["/system/scheduler/add".into(), format!("=name={SNIFFER_STOP}"), format!("=interval={}s", w.duration + 20),
        "=policy=read,write,test,sniff".into(), "=comment=Cairn Net: stops a packet capture left running".into(),
        format!("=on-event=/tool sniffer stop\n/tool sniffer set{back_cli}\n/file remove [find name={file}]\n/system scheduler remove [find name={SNIFFER_STOP}]")])?;
    call(&mut api, &set)?;
    let run = (|| -> Result<(), String> {
        api.call(&["/tool/sniffer/start"])?;
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(w.duration) && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1000));
            let size = api.call(&["/file/print", "=.proplist=size", &format!("?name={file}")])?.pop().and_then(|f| f["size"].as_str().map(String::from));
            let mut g = jobs();
            if let Some(j) = g.get_mut(&id) {
                j.rows = vec![json!({"elapsed": t0.elapsed().as_secs(), "size": size.unwrap_or_default()})];
            }
        }
        Ok(())
    })();
    // whatever happened: stop, clean, then fetch what was captured
    let clean = (|| -> Result<Vec<u8>, String> {
        api.call(&["/tool/sniffer/stop"])?;
        api.call(&["/system/scheduler/remove", &format!("=numbers={SNIFFER_STOP}")])?;
        call(&mut api, &back)?;
        std::thread::sleep(Duration::from_millis(500)); // file written on stop
        let data = read_file(&mut api, &file);
        api.call(&["/file/remove", &format!("=numbers={file}")])?;
        data
    })();
    run?;
    let data = clean?;
    if let Some(j) = jobs().get_mut(&id) {
        j.pcap = Some(data);
    }
    Ok(())
}

/* ---------- netwatch (direct mode) ---------- */

pub type Cmd = crate::direct::Cmd;

/// Add or remove one probe. `alert` (optional) = a URL the switch POSTs to
/// on down and up (e.g. a ntfy topic): a generic webhook, plain text body.
pub fn netwatch_cmds(switch: &str, a: &Value) -> Result<Vec<Cmd>, String> {
    let kv = |k: &str, v: String| (k.to_string(), v);
    match text(a, "op") {
        "remove" => {
            let id = text(a, "id");
            if !id.starts_with('*') || !id[1..].chars().all(|c| c.is_ascii_hexdigit()) || id.len() < 2 {
                return Err("id: invalid".into());
            }
            Ok(vec![("/tool/netwatch/remove".into(), vec![kv(".id", id.into())])])
        }
        "add" => {
            let host = text(a, "host");
            host.parse::<Ipv4Addr>().map_err(|_| "host: an IPv4 address")?;
            let typ = match text(a, "type") { "" | "simple" => "simple", "icmp" => "icmp", "tcp-conn" => "tcp-conn", _ => return Err("type: simple, icmp or tcp-conn".into()) };
            let mut f = vec![kv("host", host.into()), kv("type", typ.into()), kv("interval", format!("{}s", num(a, "interval", 30, 5, 3600)?)), kv("comment", "cairn-net".into())];
            if typ == "tcp-conn" {
                f.push(kv("port", num(a, "port", 0, 1, 65535)?.to_string()));
            }
            let url = text(a, "alert");
            if !url.is_empty() {
                if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 400 || url.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\\' | '$' | '`')) {
                    return Err("alert: an http(s) URL without spaces or quotes".into());
                }
                let who: String = switch.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();
                for (k, st) in [("down-script", "down"), ("up-script", "up")] {
                    f.push(kv(k, format!("/tool fetch url={} http-method=post http-data={} output=none", q(url), q(&format!("{who}: {host} {st}")))));
                }
            }
            Ok(vec![("/tool/netwatch/add".into(), f)])
        }
        _ => Err("op: add or remove".into()),
    }
}

pub fn netwatch(v: &Value, user: &str, apply: bool) -> Result<Value, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    let c = &sw.cfg;
    let cmds = netwatch_cmds(&c.name, v)?;
    // as typed in a terminal: values with spaces or quotes quoted and escaped
    let shown: Vec<String> = cmds.iter().map(|(p, kv)| kv.iter().fold(p.clone(), |o, (k, v)| o + &format!(" {k}={}", if v.is_empty() || v.contains([' ', '"']) { q(v) } else { v.clone() }))).collect();
    if !apply {
        return Ok(json!({"ok": true, "commands": shown}));
    }
    let (u, p, _) = account(c, &sw, "netwatch")?;
    let mut api = Api::connect(c, &u, &p).map_err(|e| format!("write account: {e}"))?;
    let (done, err) = crate::direct::run(&mut api, &cmds);
    // read back: the probe is there (add) or gone (remove)
    let host = text(v, "host");
    let rows = print(&mut api, "/tool/netwatch", ".id,host", &[])?;
    let verified = match text(v, "op") {
        "add" => rows.iter().any(|r| r["host"] == host),
        _ => !rows.iter().any(|r| r[".id"] == text(v, "id")),
    };
    eprintln!("netwatch {} on {} by {user}: {done}/{} commands, read back: {verified}", text(v, "op"), c.name, cmds.len());
    Ok(json!({"ok": err.is_none() && verified, "commands": shown, "error": err, "verified": verified}))
}

/// The read account's group with `test` added: (group, its row id, new policy),
/// or why it must not be changed (a built-in or shared group would grant
/// the right to other accounts too).
pub fn grant_plan(user: &str, users: &[Value], groups: &[Value]) -> Result<Option<(String, String, String)>, String> {
    let g = users.iter().find(|u| u["name"] == user).and_then(|u| u["group"].as_str()).ok_or(format!("{user}: account not found"))?;
    if matches!(g, "read" | "write" | "full") {
        return Err(format!("{user} is in the built-in group {g}: give it its own group first (Settings, preparation script)"));
    }
    let others: Vec<&str> = users.iter().filter(|u| u["group"] == g && u["name"] != user).filter_map(|u| u["name"].as_str()).collect();
    if !others.is_empty() {
        return Err(format!("the group {g} is shared with {}: they would get the right too", others.join(", ")));
    }
    let row = groups.iter().find(|r| r["name"] == g).ok_or(format!("{g}: group not found"))?;
    let mut p: Vec<&str> = row["policy"].as_str().unwrap_or("").split(',').filter(|x| !x.is_empty() && !x.starts_with('!')).collect();
    if p.contains(&"test") {
        return Ok(None);
    }
    p.push("test");
    Ok(Some((g.to_string(), row[".id"].as_str().unwrap_or("").to_string(), p.join(","))))
}

/// Gives the read account the `test` right (ping, traceroute, torch,
/// bandwidth test, IP scan). Applied with the write account when it holds
/// the `policy` right in direct mode; otherwise the command to paste.
pub fn grant(v: &Value, user: &str, apply: bool) -> Result<Value, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    let c = &sw.cfg;
    let (plan, wp) = with_api(&sw, |api| {
        let users = print(api, "/user", "name,group", &[])?;
        let groups = print(api, "/user/group", ".id,name,policy", &[])?;
        let wp = if c.write_user.is_empty() { None } else { Some(policies(api, &c.write_user)?) };
        Ok((grant_plan(&c.user, &users, &groups), wp))
    })?;
    let Some((group, id, policy)) = plan? else { return Ok(json!({"ok": true, "done": true})) };
    let shown = format!("/user group set {group} policy={policy}");
    let why = if c.mode != "direct" {
        Some("the switch is not in direct mode: paste the command in the switch's own terminal (SSH as admin); the Cairn Net Terminal page only writes in direct mode")
    } else if !wp.as_ref().is_some_and(|p| p.iter().any(|x| x == "policy")) {
        Some("the write account lacks the policy right: paste the command in the switch's own terminal (SSH as admin)")
    } else {
        None
    };
    if !apply || why.is_some() {
        return Ok(json!({"ok": true, "commands": [shown], "can_apply": why.is_none(), "why": why}));
    }
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    let (_, err) = crate::direct::run(&mut api, &[("/user/group/set".into(), vec![(".id".into(), id), ("policy".into(), policy)])]);
    let verified = policies(&mut api, &c.user)?.iter().any(|p| p == "test");
    eprintln!("grant test to {} on {} by {user}: read back {verified}", c.user, c.name);
    Ok(json!({"ok": err.is_none() && verified, "commands": [shown], "error": err, "verified": verified}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_only_own_group() {
        let u = |n: &str, g: &str| json!({"name": n, "group": g});
        let groups = [json!({".id": "*4", "name": "cairn-read", "policy": "read,api,!test"}), json!({".id": "*5", "name": "ops", "policy": "read,test"})];
        let p = grant_plan("cairn-read", &[u("cairn-read", "cairn-read")], &groups).unwrap().unwrap();
        assert_eq!(p, ("cairn-read".into(), "*4".into(), "read,api,test".into()));
        assert!(grant_plan("cairn-read", &[u("cairn-read", "read")], &groups).is_err());
        assert!(grant_plan("cairn-read", &[u("cairn-read", "cairn-read"), u("bob", "cairn-read")], &groups).is_err());
        assert_eq!(grant_plan("x", &[u("x", "ops")], &groups).unwrap(), None);
    }

    #[test]
    fn ping_words() {
        let r = build("ping", &json!({"address": "192.168.1.1", "count": "3", "size": 1400, "interface": "ether1"}), None).unwrap();
        assert_eq!(r.words, ["/ping", "=address=192.168.1.1", "=count=3", "=size=1400", "=interface=ether1"]);
        assert_eq!(r.max_s, 8);
        assert!(build("ping", &json!({"address": "1.1.1.1\n/system/reboot"}), None).is_err());
        assert!(build("ping", &json!({"address": "1.1.1.1", "count": 1000}), None).is_err());
    }

    #[test]
    fn torch_split_by_given_fields() {
        let r = build("torch", &json!({"interface": "sfp1", "src-address": "0.0.0.0/0", "port": "any", "dst-address": ""}), None).unwrap();
        assert_eq!(r.words, ["/tool/torch", "=interface=sfp1", "=duration=30s", "=src-address=0.0.0.0/0", "=port=any"]);
        assert!(build("torch", &json!({"interface": "sfp1", "port": "http"}), None).is_err());
        assert!(build("torch", &json!({}), None).is_err());
    }

    #[test]
    fn btest_capped_and_heavy() {
        let t = Target { ip: "127.0.0.1".into(), user: "u".into(), password: "p w".into(), peer: None, clean: None };
        let r = build("btest", &json!({"speed": 50, "duration": 5}), Some(&t)).unwrap();
        assert!(r.heavy);
        assert!(r.words.contains(&"=local-tx-speed=50M".to_string()) && r.words.contains(&"=duration=5s".to_string()));
        assert!(show(&r.words).contains("password=********") && !show(&r.words).contains("p w"));
        assert!(build("btest", &json!({"speed": 5000}), Some(&t)).is_err());
        assert!(build("btest", &json!({"duration": 600}), Some(&t)).is_err());
    }

    #[test]
    fn scan_range_limited() {
        assert_eq!(range_size("192.168.1.0/24"), Some(256));
        assert_eq!(range_size("10.0.0.10-10.0.0.20"), Some(11));
        assert_eq!(range_size("10.0.0.20-10.0.0.10"), None);
        assert!(build("scan", &json!({"range": "10.0.0.0/16"}), None).is_err());
        assert!(build("scan", &json!({"range": "10.0.0.0/20"}), None).is_ok());
    }

    #[test]
    fn rights_pick_an_account() {
        let p = |s: &str| s.split(',').map(String::from).collect::<Vec<_>>();
        let (r, w) = (p("read,api"), p("read,write,test,api"));
        assert_eq!(rights(&p("read,test"), Some(&w), &["test"], false, false)["account"], "read");
        assert_eq!(rights(&r, Some(&w), &["test"], false, false)["account"], "write");
        let none = rights(&r, None, &["test"], false, false);
        assert_eq!(none["ok"], false);
        assert_eq!(none["read_lacks"], json!(["test"]));
        // writes: write account and direct mode only
        assert_eq!(rights(&w, Some(&w), &["write"], true, false)["direct_needed"], true);
        assert_eq!(rights(&w, Some(&w), &["write"], true, true)["account"], "write");
        assert_eq!(rights(&w, Some(&w), &["sniff", "write"], true, true)["write_lacks"], json!(["sniff"]));
    }

    #[test]
    fn netwatch_alert_script_quoted() {
        let c = netwatch_cmds("SW 1", &json!({"op": "add", "host": "10.0.0.5", "type": "icmp", "alert": "https://ntfy.example/topic"})).unwrap();
        let f: BTreeMap<_, _> = c[0].1.iter().cloned().collect();
        assert_eq!(f["down-script"], r#"/tool fetch url="https://ntfy.example/topic" http-method=post http-data="SW1: 10.0.0.5 down" output=none"#);
        assert!(netwatch_cmds("s", &json!({"op": "add", "host": "10.0.0.5", "alert": "https://x/\"$(a)"})).is_err());
        assert!(netwatch_cmds("s", &json!({"op": "remove", "id": "*1A"})).is_ok());
        assert!(netwatch_cmds("s", &json!({"op": "remove", "id": "1; /x"})).is_err());
    }

    #[test]
    fn sniffer_filters_checked() {
        let w = sniffer_want(&json!({"interface": "ether1", "port": "443", "duration": 5})).unwrap();
        assert_eq!(w.filters[0], ("filter-interface", "ether1".to_string()));
        assert_eq!(w.filters[1], ("filter-ip-address", String::new())); // cleared: the old filter does not linger
        assert!(sniffer_want(&json!({"duration": 600})).is_err());
        assert!(sniffer_want(&json!({"ip-address": "x"})).is_err());
    }
}
