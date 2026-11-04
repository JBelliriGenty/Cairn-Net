//! Cairn Net (cairn-net): read-only web panel for a few MikroTik RouterOS
//! switches, with the look of Cairn (cairn-nas).
//!
//! Runs behind a reverse proxy that authenticates users (e.g. Traefik with
//! a forwardAuth portal): a request is served only if it comes from a trusted
//! proxy address AND carries the identity header (and a required group, if set).
//! Each switch is read through the RouterOS API over TLS (api-ssl, 8729) with a
//! read-only account, one persistent connection per switch (no login per poll,
//! so no login line per poll in the switch logs). The switches, their
//! certificate pins, accounts and management mode are set in the Settings
//! page (src/settings.rs).
//!
//! DHCP servers and reservations, per switch: git mode (src/git.rs), the
//! DHCP file of a repository, compared with the router and committed; direct
//! mode (src/direct.rs), the switch itself, written through the API with the
//! write account.
//!
//! Without such a proxy, "login" in the configuration turns on the built-in
//! sign-in (src/login.rs, HTTPS required: "tls" with cert and key files).
//!
//! Usage: cairn-net /etc/cairn-net.json
//!        cairn-net hash-password   (reads a password on stdin, prints its hash)

mod dhcp;
mod direct;
mod git;
mod login;
mod servers;
mod settings;

use openssl::hash::MessageDigest;
use openssl::ssl::{SslConnector, SslMethod, SslStream, SslVerifyMode};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use settings::{Switch, SwitchCfg};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const INDEX: &str = include_str!("../www/index.html");
const JS: &str = include_str!("../www/panel.js");
const CSS: &str = include_str!("../www/panel.css");
const ICON: &str = include_str!("../www/icon.svg");
const I18N: &str = include_str!("../www/i18n.js");

struct Config {
    listen: String,
    trusted_proxies: Vec<String>,
    user_header: String,
    groups_header: String,
    require_groups: Vec<String>,
    login: Option<login::Login>,
    tls: Option<(String, String)>, // PEM certificate and key files
    dhcp: Option<Dhcp>,
    public: Value, // what /api/config returns
}

/// Where the reservations live and which switch serves them.
struct Dhcp {
    file: String,    // copy of the repository's dhcp.yml
    switch: String,  // the router running the DHCP server
    server: String,  // its LAN DHCP server name
    network: String, // reservations must be inside
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
}

impl Config {
    fn load(path: &str) -> Config {
        let v: Value = serde_json::from_str(&fs::read_to_string(path).expect("config unreadable")).expect("config: invalid JSON");
        let s = |k: &str, d: &str| v[k].as_str().unwrap_or(d).to_string();
        // Switches: the Settings page owns them (data_dir/settings.json); the
        // configuration's list only seeds that file the first time.
        let password = v["password_file"].as_str().and_then(|f| fs::read_to_string(f).ok()).unwrap_or_default().trim().to_string();
        let seed: Vec<SwitchCfg> = v["switches"].as_array().into_iter().flatten().map(|w| {
            let mut c = SwitchCfg::from_json(w);
            if c.user.is_empty() {
                c.user = s("user", "monitor");
            }
            if c.password.is_empty() {
                c.password.clone_from(&password);
            }
            c
        }).collect();
        let data = s("data_dir", "/var/lib/cairn-net");
        settings::init(&format!("{data}/settings.json"), seed);
        git::init(&data);
        let mut public = json!({});
        for k in ["title", "subtitle", "lang"] {
            public[k] = v[k].clone();
        }
        let d = &v["dhcp"];
        let dhcp = d["file"].as_str().map(|f| Dhcp {
            file: f.to_string(),
            switch: d["switch"].as_str().expect("dhcp: switch").to_string(),
            server: d["server"].as_str().unwrap_or("lan").to_string(),
            network: d["network"].as_str().expect("dhcp: network").to_string(),
        });
        let l = &v["login"];
        let login = l.is_object().then(|| login::Login {
            users_file: l["users_file"].as_str().unwrap_or("/etc/cairn-net/users").to_string(),
            session_s: l["session_minutes"].as_u64().unwrap_or(30).max(1) * 60,
        });
        let tls = match (v["tls"]["cert"].as_str(), v["tls"]["key"].as_str()) {
            (Some(c), Some(k)) => Some((c.to_string(), k.to_string())),
            _ => None,
        };
        // the session cookie is Secure, and a password must not cross the network in clear
        assert!(login.is_none() || tls.is_some(), "config: \"login\" requires \"tls\" (cert and key)");
        public["login"] = json!(login.is_some());
        // optional words under the "names from the leases" table (where your DNS publishes them)
        public["names_note"] = d["names_note"].clone();
        public["dhcp"] = dhcp.as_ref().map_or(Value::Null, |d| json!({"switch": d.switch, "server": d.server, "network": d.network}));
        Config {
            listen: s("listen", "127.0.0.1:9093"),
            trusted_proxies: strings(&v, "trusted_proxies"),
            user_header: s("user_header", "Remote-User"),
            groups_header: s("groups_header", "Remote-Groups"),
            require_groups: strings(&v, "require_groups"),
            login,
            tls,
            dhcp,
            public,
        }
    }
}

/* ---------- RouterOS API (https://help.mikrotik.com/docs/display/ROS/API) ---------- */

pub struct Api(SslStream<TcpStream>);

fn put_len(n: usize, out: &mut Vec<u8>) {
    let n = n as u32;
    match n {
        0..=0x7f => out.push(n as u8),
        0x80..=0x3fff => out.extend_from_slice(&((n | 0x8000) as u16).to_be_bytes()),
        0x4000..=0x1f_ffff => out.extend_from_slice(&(n | 0xc0_0000).to_be_bytes()[1..]),
        0x20_0000..=0x0fff_ffff => out.extend_from_slice(&(n | 0xe000_0000).to_be_bytes()),
        _ => {
            out.push(0xf0);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}

fn encode(words: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for w in words {
        put_len(w.len(), &mut out);
        out.extend_from_slice(w.as_bytes());
    }
    out.push(0); // end of sentence
    out
}

/// Length prefix: the first byte says how many bytes follow.
fn get_len(first: u8, mut next: impl FnMut() -> std::io::Result<u8>) -> std::io::Result<u32> {
    let b = first as u32;
    let (mut n, extra) = match first {
        0x00..=0x7f => (b, 0),
        0x80..=0xbf => (b & 0x3f, 1),
        0xc0..=0xdf => (b & 0x1f, 2),
        0xe0..=0xef => (b & 0x0f, 3),
        _ => (0, 4),
    };
    for _ in 0..extra {
        n = n << 8 | next()? as u32;
    }
    Ok(n)
}

impl Api {
    /// TLS only, and the certificate's sha256 (lower-case hex): what the
    /// Settings page shows before anything is sent to the switch.
    fn tls(host: &str, port: u16) -> Result<(SslStream<TcpStream>, String), String> {
        let addr = (host, port).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        tcp.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
        tcp.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
        let mut b = SslConnector::builder(SslMethod::tls()).map_err(|e| e.to_string())?;
        b.set_verify(SslVerifyMode::NONE); // self-signed: checked below against the pin
        let tls = b.build().configure().map_err(|e| e.to_string())?.verify_hostname(false).use_server_name_indication(false)
            .connect(host, tcp).map_err(|e| e.to_string())?;
        let cert = tls.ssl().peer_certificate().ok_or("no certificate")?;
        let got: String = cert.digest(MessageDigest::sha256()).map_err(|e| e.to_string())?.iter().map(|b| format!("{b:02x}")).collect();
        Ok((tls, got))
    }

    fn connect(sw: &SwitchCfg, user: &str, password: &str) -> Result<Api, String> {
        let (tls, got) = Api::tls(&sw.host, sw.port)?;
        if got != sw.sha256 {
            return Err(format!("certificate mismatch (sha256 {})", settings::pretty(&got))); // never send the password then
        }
        let mut api = Api(tls);
        api.call(&["/login", &format!("=name={user}"), &format!("=password={password}")])?;
        Ok(api)
    }

    fn byte(&mut self) -> std::io::Result<u8> {
        let mut b = [0u8; 1];
        self.0.read_exact(&mut b)?;
        Ok(b[0])
    }

    fn word(&mut self) -> std::io::Result<String> {
        let first = self.byte()?;
        let len = get_len(first, || self.byte())?;
        let mut buf = vec![0u8; len as usize];
        self.0.read_exact(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn sentence(&mut self) -> std::io::Result<Vec<String>> {
        let mut words = Vec::new();
        loop {
            let w = self.word()?;
            if w.is_empty() {
                return Ok(words);
            }
            words.push(w);
        }
    }

    /// One command; the `!re` replies as objects ("=key=value" words).
    pub fn call(&mut self, words: &[&str]) -> Result<Vec<Value>, String> {
        self.0.write_all(&encode(words)).map_err(|e| e.to_string())?;
        let (mut rows, mut trap) = (Vec::new(), None);
        loop {
            let s = self.sentence().map_err(|e| e.to_string())?;
            let attrs: Map<String, Value> = s.iter().skip(1).filter_map(|w| w.strip_prefix('=')?.split_once('='))
                .map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect();
            match s.first().map(String::as_str) {
                Some("!re") => rows.push(Value::Object(attrs)),
                Some("!trap") => trap = Some(attrs.get("message").and_then(Value::as_str).unwrap_or("trap").to_string()),
                Some("!fatal") => return Err(s.get(1).cloned().unwrap_or_else(|| "fatal".into())),
                Some("!done") => return trap.map_or(Ok(rows), Err),
                _ => {} // !empty (RouterOS 7.18+), unknown
            }
        }
    }
}

/// Runs `f` on the switch's connection, opening it if needed; a stale
/// connection (switch rebooted, timeout) is reopened once.
fn with_api<T>(sw: &Switch, f: impl Fn(&mut Api) -> Result<T, String>) -> Result<T, String> {
    let c = &sw.cfg;
    let mut g = sw.conn.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = g.is_none();
    if fresh {
        *g = Some(Api::connect(c, &c.user, &c.password)?);
    }
    match f(g.as_mut().unwrap()) {
        Ok(v) => Ok(v),
        Err(e) if fresh => {
            *g = None;
            Err(e)
        }
        Err(_) => {
            *g = None; // stays closed if the reconnection fails
            *g = Some(Api::connect(c, &c.user, &c.password)?);
            f(g.as_mut().unwrap()).inspect_err(|_| *g = None)
        }
    }
}

pub fn print(api: &mut Api, path: &str, props: &str, query: &[&str]) -> Result<Vec<Value>, String> {
    let cmd = format!("{path}/print");
    let prop = format!("=.proplist={props}");
    let mut words = vec![cmd.as_str(), prop.as_str()];
    words.extend_from_slice(query);
    match api.call(&words) {
        Err(e) if e.starts_with("no such command") => Ok(vec![]),
        r => r,
    }
}

/* ---------- collectors ---------- */

fn fast_one(api: &mut Api) -> Result<Value, String> {
    Ok(json!({
        "resource": print(api, "/system/resource", "uptime,version,cpu-load,free-memory,total-memory,board-name", &[])?.pop(),
        "health": print(api, "/system/health", "name,value,type", &[])?,
        "ifaces": print(api, "/interface", "name,type,running,disabled,rx-byte,tx-byte,link-downs,comment,slave,last-link-up-time,actual-mtu", &[])?,
    }))
}

const MONITOR_KEYS: [&str; 10] = ["name", "status", "rate", "full-duplex", "sfp-module-present", "sfp-vendor-name",
    "sfp-vendor-part-number", "sfp-connector-type", "sfp-temperature", "sfp-rx-power"];

fn slow_one(api: &mut Api) -> Result<Value, String> {
    let eth: Vec<String> = print(api, "/interface/ethernet", "name", &[])?.iter().filter_map(|r| r["name"].as_str().map(String::from)).collect();
    let numbers = format!("=numbers={}", eth.join(","));
    let monitor: Vec<Value> = if eth.is_empty() { vec![] } else {
        api.call(&["/interface/ethernet/monitor", &numbers, "=once="])?.into_iter()
            .map(|r| Value::Object(MONITOR_KEYS.iter().filter_map(|k| Some((k.to_string(), r.get(*k)?.clone()))).collect())).collect()
    };
    Ok(json!({
        "routerboard": print(api, "/system/routerboard", "model,serial-number,current-firmware,upgrade-firmware", &[])?.pop(),
        "monitor": monitor,
        "ports": print(api, "/interface/bridge/port", "interface,pvid,status,role,edge-port,forwarding,comment", &[])?,
        "vlans": print(api, "/interface/bridge/vlan", "vlan-ids,current-tagged,current-untagged,comment,dynamic", &[])?,
        "bonds": print(api, "/interface/bonding", "name,slaves,mode,running,comment", &[])?,
        "neighbors": print(api, "/ip/neighbor", "interface,identity,address,mac-address,board,platform,interface-name", &[])?,
        "hosts": print(api, "/interface/bridge/host", "mac-address,on-interface,vid", &["?local=false"])?,
        "leases": print(api, "/ip/dhcp-server/lease", "address,mac-address,host-name,comment,status,server,dynamic,last-seen,expires-after", &[])?,
        "dhcp_servers": print(api, "/ip/dhcp-server", "name,disabled,invalid,address-pool,interface,lease-time,authoritative,comment", &[])?,
        "dhcp_networks": print(api, "/ip/dhcp-server/network", "address,gateway,dns-server,domain,ntp-server,dhcp-option,comment", &[])?,
        "dhcp_options": print(api, "/ip/dhcp-server/option", "name,code,value,comment", &[])?,
        "addresses": print(api, "/ip/address", "address,interface,disabled", &[])?,
        "vlan_ifaces": print(api, "/interface/vlan", "name,vlan-id,interface", &[])?,
        "pools": print(api, "/ip/pool", "name,ranges", &[])?,
        "arp": print(api, "/ip/arp", "address,mac-address,interface", &["?complete=true"])?,
    }))
}

/// This host's own ARP table (it talks to most of the LAN: monitoring pings).
fn local_arp() -> Vec<(String, String)> {
    fs::read_to_string("/proc/net/arp").unwrap_or_default().lines().skip(1).filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() >= 4 && f[3] != "00:00:00:00:00:00").then(|| (f[0].to_string(), f[3].to_uppercase()))
    }).collect()
}

/// Reverse names from the system resolver (the LAN DNS), one getent call.
fn names(ips: &BTreeSet<String>) -> BTreeMap<String, String> {
    if ips.is_empty() {
        return BTreeMap::new();
    }
    let out = Command::new("getent").arg("hosts").args(ips).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines().filter_map(|l| {
        let mut f = l.split_whitespace();
        Some((f.next()?.to_string(), f.next()?.split('.').next()?.to_string()))
    }).collect()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Every switch in parallel; a switch that fails gets {"error": …}.
fn collect(f: fn(&mut Api) -> Result<Value, String>) -> Vec<Value> {
    let handles: Vec<_> = settings::all().into_iter().map(|sw| std::thread::spawn(move || {
        let t = Instant::now();
        let mut v = match with_api(&sw, f) {
            Ok(v) => v,
            Err(e) => json!({"error": e}),
        };
        v["name"] = json!(sw.cfg.name);
        v["ms"] = json!(t.elapsed().as_millis() as u64);
        v
    })).collect();
    handles.into_iter().map(|h| h.join().unwrap_or_else(|_| json!({"error": "collector panicked"}))).collect()
}

static FAST: Mutex<Option<(Instant, String)>> = Mutex::new(None);
static SLOW: Mutex<Option<(Instant, String)>> = Mutex::new(None);

/// Several viewers share one read of the switches per period.
fn cached(slot: &Mutex<Option<(Instant, String)>>, ttl: Duration, build: impl FnOnce() -> String) -> String {
    let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, s)) = g.as_ref() {
        if t.elapsed() < ttl {
            return s.clone();
        }
    }
    let s = build();
    *g = Some((Instant::now(), s.clone()));
    s
}

fn fast() -> String {
    cached(&FAST, Duration::from_millis(1500), || json!({"t": now_ms(), "switches": collect(fast_one)}).to_string())
}

fn slow() -> String {
    cached(&SLOW, Duration::from_secs(10), || {
        let switches = collect(slow_one);
        let arp = local_arp();
        let mut ips: BTreeSet<String> = arp.iter().map(|(ip, _)| ip.clone()).collect();
        for s in &switches {
            for k in ["arp", "leases"] {
                ips.extend(s[k].as_array().into_iter().flatten().filter_map(|r| r["address"].as_str().map(String::from)));
            }
        }
        json!({"switches": switches, "arp": arp, "names": names(&ips)}).to_string()
    })
}

/* ---------- DHCP (lot 1) ---------- */

/// Where a DHCP view comes from: the repository file (git mode, `cfg.dhcp`)
/// or the switch itself (direct mode).
enum Src<'a> {
    File(&'a Dhcp),
    Git(Arc<Switch>, Option<&'a Dhcp>),
    Direct(Arc<Switch>),
}

fn source<'a>(cfg: &'a Config, name: &str) -> Result<Src<'a>, String> {
    let name = if name.is_empty() { cfg.dhcp.as_ref().map_or("", |d| d.switch.as_str()) } else { name };
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    if sw.cfg.mode == "direct" {
        return Ok(Src::Direct(sw));
    }
    let d = cfg.dhcp.as_ref().filter(|d| d.switch == name);
    if sw.cfg.mode == "git" && !sw.cfg.git_url.is_empty() {
        return Ok(Src::Git(sw, d));
    }
    match &cfg.dhcp {
        Some(d) if d.switch == name => Ok(Src::File(d)),
        _ => Err(format!("{name}: no DHCP to manage (read-only switch)")),
    }
}

/// Switches with a DHCP view, for the page's picker.
fn dhcp_targets(cfg: &Config) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for s in settings::all() {
        let c = &s.cfg;
        match &cfg.dhcp {
            _ if c.mode == "direct" || (c.mode == "git" && !c.git_url.is_empty() && cfg.dhcp.as_ref().is_none_or(|d| d.switch != c.name)) =>
                out.push(json!({"switch": c.name, "mode": c.mode, "server": "", "network": ""})),
            Some(d) if d.switch == c.name => out.push(json!({"switch": d.switch, "mode": c.mode, "server": d.server, "network": d.network})),
            _ => {}
        }
    }
    json!(out)
}

/// The current text, and what the switch runs (direct mode).
fn current(src: &Src) -> Result<(String, Option<direct::Live>), String> {
    match src {
        Src::File(d) => Ok((fs::read_to_string(&d.file).unwrap_or_default(), None)),
        Src::Git(sw, _) => Ok((git::read(&sw.cfg)?, None)),
        Src::Direct(sw) => {
            let l = with_api(sw, direct::read)?;
            Ok((direct::to_text(&l), Some(l)))
        }
    }
}

fn problems(src: &Src, text: &str) -> Vec<String> {
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let srvs = servers::parse(&lines);
    let nets = match src {
        Src::File(d) | Src::Git(_, Some(d)) => vec![d.network.clone()],
        Src::Git(_, None) | Src::Direct(_) => srvs.iter().map(|s| s.network.clone()).collect(),
    };
    let mut p = dhcp::check(&dhcp::parse(text), &nets);
    p.extend(servers::check(&srvs));
    p
}

fn changes(a: &Value) -> &[Value] {
    a["changes"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn dhcp_view(src: &Src) -> Value {
    let (text, err) = match current(src) {
        Ok((t, _)) => (t, Value::Null),
        Err(e) => (String::new(), json!(e)),
    };
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let names = match src {
        Src::File(d) | Src::Git(_, Some(d)) => {
            let s: Value = serde_json::from_str(&slow()).unwrap_or(Value::Null);
            let router = s["switches"].as_array().into_iter().flatten().find(|x| x["name"] == d.switch.as_str()).cloned().unwrap_or(Value::Null);
            dhcp::names_file(router["leases"].as_array().map(Vec::as_slice).unwrap_or(&[]), &d.server)
        }
        _ => vec![], // DNS integrations: later
    };
    json!({
        "text": text,
        "entries": dhcp::parse(&text).iter().map(dhcp::to_json).collect::<Vec<_>>(),
        "problems": problems(src, &text),
        "servers": servers::parse(&lines).iter().map(servers::to_json).collect::<Vec<_>>(),
        "names": names.into_iter().map(|(ip, n)| json!([ip, n])).collect::<Vec<_>>(),
        "router_error": err,
        // git mode: the commit needs the deploy key added with write access
        "can_apply": matches!(src, Src::Direct(_) | Src::Git(..)),
    })
}

fn dhcp_preview(src: &Src, a: &Value) -> Value {
    let (text, live) = match current(src) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    match dhcp::apply(&text, changes(a)) {
        Ok(new) => {
            let mut out = json!({"ok": true, "old": text, "new": new, "problems": problems(src, &new)});
            match live.as_ref().map(|l| direct::plan(l, &new)) {
                Some(Ok(c)) => out["commands"] = json!(c.iter().map(direct::show).collect::<Vec<_>>()),
                Some(Err(e)) => out["problems"].as_array_mut().unwrap().push(json!(e)),
                None => {}
            }
            out
        }
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// Direct mode: the same changes, from what the switch runs right now (read
/// with the write account), then the commands, then a read back.
fn dhcp_apply(src: &Src, a: &Value, user: &str, author: &str) -> Value {
    if let Src::Git(sw, _) = src {
        let n = changes(a).len();
        let msg = format!("dhcp ({}): {n} change{} from Cairn Net", sw.cfg.git_path, if n > 1 { "s" } else { "" });
        let r = git::commit(&sw.cfg, author, &msg, |text| {
            let new = dhcp::apply(text, changes(a))?;
            let p = problems(src, &new);
            if p.is_empty() { Ok(new) } else { Err(p.join(" · ")) }
        });
        eprintln!("dhcp committed for {} by {user}: {}", sw.cfg.name, r.as_ref().map_or_else(|e| format!("error: {e}"), |c| c.clone()));
        return match r {
            Ok(c) => json!({"ok": true, "commit": c}),
            Err(e) => json!({"ok": false, "error": e}),
        };
    }
    let Src::Direct(sw) = src else { return json!({"ok": false, "error": "this switch is not in direct or git mode"}) };
    let c = &sw.cfg;
    let mut api = match Api::connect(c, &c.write_user, &c.write_password) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "error": format!("write account: {e}")}),
    };
    let res = (|| -> Result<Value, String> {
        let live = direct::read(&mut api)?;
        let new = dhcp::apply(&direct::to_text(&live), changes(a))?;
        let p = problems(src, &new);
        if !p.is_empty() {
            return Err(p.join(" · "));
        }
        let cmds = direct::plan(&live, &new)?;
        let (done, err) = direct::run(&mut api, &cmds);
        let after = direct::to_text(&direct::read(&mut api)?);
        let sorted = |t: &str| {
            let mut l: Vec<&str> = t.lines().collect();
            l.sort();
            l.join("\n")
        };
        let verified = sorted(&after) == sorted(&direct::canonical(&new));
        eprintln!("dhcp applied on {} by {user}: {done}/{} commands{}{}", c.name, cmds.len(),
            err.as_ref().map_or(String::new(), |e| format!(", error: {e}")), if verified { "" } else { ", read back differs" });
        Ok(json!({"ok": err.is_none() && verified, "done": done, "commands": cmds.iter().map(direct::show).collect::<Vec<_>>(),
            "error": err, "verified": verified, "after": after}))
    })();
    forget_cache();
    res.unwrap_or_else(|e| json!({"ok": false, "error": e}))
}

/* ---------- Settings ---------- */

fn settings_view() -> Value {
    json!({"switches": settings::all().iter().map(|s| s.cfg.to_json(false)).collect::<Vec<_>>(), "modes": settings::MODES, "git_key": git::public_key()})
}

/// Connection test from the Settings page. Without `sha256` (or with another
/// one than the switch shows) only the certificate is read: no password is
/// sent before the user accepted the fingerprint. Empty passwords: the stored
/// ones of `orig`.
fn probe(body: &Value) -> Value {
    let mut c = SwitchCfg::from_json(&body["switch"]);
    if let Some(old) = body["orig"].as_str().and_then(settings::get) {
        c.keep_secrets(&old.cfg);
    }
    let got = match Api::tls(&c.host, c.port) {
        Ok((_, got)) => got,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let mut out = json!({"ok": true, "sha256": settings::pretty(&got)});
    if got != c.sha256 {
        return out; // the page asks the user to compare it first
    }
    let login = |user: &str, pw: &str| -> Result<Value, String> {
        let mut api = Api::connect(&c, user, pw)?;
        let id = print(&mut api, "/system/identity", "name", &[])?.pop().unwrap_or_default();
        let res = print(&mut api, "/system/resource", "version,board-name", &[])?.pop().unwrap_or_default();
        Ok(json!({"identity": id["name"], "version": res["version"], "board": res["board-name"]}))
    };
    out["read"] = login(&c.user, &c.password).map_or_else(|e| json!({"error": e}), |v| v);
    if !c.write_user.is_empty() {
        out["write"] = login(&c.write_user, &c.write_password).map_or_else(|e| json!({"error": e}), |v| v);
    }
    if c.mode == "git" && !c.git_url.is_empty() {
        out["git"] = git::probe(&c).map_or_else(|e| json!({"error": e}), |_| json!({}));
    }
    out
}

/// After a change of the switch list, the next read starts from scratch.
fn forget_cache() {
    for slot in [&FAST, &SLOW] {
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/* ---------- HTTP ---------- */

fn header(r: &Request, name: &str) -> String {
    r.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().trim().to_string()).unwrap_or_default()
}

fn cookie(r: &Request, name: &str) -> Option<String> {
    header(r, "Cookie").split(';').find_map(|c| c.trim().strip_prefix(&format!("{name}=")).map(String::from))
}

const SESSION: &str = "cairn_net_session";

fn user(cfg: &Config, r: &Request) -> Option<String> {
    if let Some(l) = &cfg.login {
        return login::session(l, &cookie(r, SESSION)?);
    }
    let ip = r.remote_addr()?.ip().to_string();
    if !cfg.trusted_proxies.contains(&ip) {
        return None;
    }
    let user = header(r, &cfg.user_header);
    let groups = header(r, &cfg.groups_header);
    let in_group = cfg.require_groups.is_empty() || groups.split(',').any(|g| cfg.require_groups.iter().any(|n| n == g.trim()));
    (!user.is_empty() && in_group).then_some(user)
}

fn respond(r: Request, code: u16, body: String, ctype: &str) {
    respond_cookie(r, code, body, ctype, None)
}

fn respond_cookie(r: Request, code: u16, body: String, ctype: &str, set_cookie: Option<String>) {
    let h = |k: &str, v: &str| Header::from_bytes(k, v).unwrap();
    let mut resp = Response::from_string(body)
        .with_status_code(code)
        .with_header(h("Content-Type", &format!("{ctype}; charset=utf-8")))
        .with_header(h("Cache-Control", "no-store"))
        .with_header(h("X-Content-Type-Options", "nosniff"))
        .with_header(h("Content-Security-Policy", "default-src 'self'; frame-ancestors 'none'"));
    if let Some(c) = set_cookie {
        resp = resp.with_header(h("Set-Cookie", &c));
    }
    let _ = r.respond(resp);
}

fn handle(cfg: &'static Config, mut r: Request) {
    let path = r.url().split('?').next().unwrap_or("").to_string();
    if path == "/healthz" {
        return respond(r, 200, "ok".into(), "text/plain");
    }
    if path == "/icon.svg" {
        return respond(r, 200, ICON.into(), "image/svg+xml");
    }
    if let Some(l) = &cfg.login {
        // signed out: the page itself loads and shows the sign-in form
        match (r.method().clone(), path.as_str()) {
            (Method::Get, "/") => return respond(r, 200, INDEX.into(), "text/html"),
            (Method::Get, "/panel.js") => return respond(r, 200, JS.into(), "text/javascript"),
            (Method::Get, "/panel.css") => return respond(r, 200, CSS.into(), "text/css"),
            (Method::Get, "/i18n.js") => return respond(r, 200, I18N.into(), "text/javascript"),
            (Method::Post, "/api/login") => {
                if !header(&r, "Content-Type").starts_with("application/json") {
                    return respond(r, 400, json!({"error": "invalid request"}).to_string(), "application/json");
                }
                let ip = r.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
                let mut body = String::new();
                let _ = r.as_reader().take(4096).read_to_string(&mut body);
                let a: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                return match login::sign_in(l, &ip, a["user"].as_str().unwrap_or(""), a["password"].as_str().unwrap_or("")) {
                    Ok(t) => respond_cookie(r, 200, json!({"ok": true}).to_string(), "application/json",
                        Some(format!("{SESSION}={t}; Path=/; HttpOnly; Secure; SameSite=Strict"))),
                    Err((code, e)) => respond(r, code, json!({"error": e}).to_string(), "application/json"),
                };
            }
            (Method::Post, "/api/logout") => {
                if let Some(t) = cookie(&r, SESSION) {
                    login::logout(&t);
                }
                return respond_cookie(r, 200, json!({"ok": true}).to_string(), "application/json",
                    Some(format!("{SESSION}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Strict")));
            }
            _ => {}
        }
    }
    let Some(user) = user(cfg, &r) else {
        return respond(r, if cfg.login.is_some() { 401 } else { 403 }, "access denied".into(), "text/plain");
    };
    // POSTs carry JSON only. CSRF: a cross-site form cannot send a JSON body
    // without a CORS preflight, which this server never answers.
    if r.method() == &Method::Post {
        if !header(&r, "Content-Type").starts_with("application/json") {
            return respond(r, 400, json!({"error": "invalid request"}).to_string(), "application/json");
        }
        let mut body = String::new();
        let _ = r.as_reader().take(65536).read_to_string(&mut body);
        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let out = match path.as_str() {
            "/api/dhcp/preview" | "/api/dhcp/apply" => match source(cfg, v["switch"].as_str().unwrap_or("")) {
                Ok(src) if path == "/api/dhcp/apply" => {
                    // commit author: what the authenticating proxy says (e.g. Authelia: Remote-Name,
                    // Remote-Email); with the built-in sign-in these headers come from the browser
                    let proxied = |h: &str| if cfg.login.is_some() { String::new() } else { header(&r, h) };
                    let name = Some(proxied("Remote-Name")).filter(|n| !n.is_empty() && !n.contains(['<', '>', '\n'])).unwrap_or_else(|| user.clone());
                    let mail = Some(proxied("Remote-Email")).filter(|m| m.contains('@') && !m.contains(['<', '>', ' ', '\n'])).unwrap_or_else(|| format!("{user}@cairn-net"));
                    dhcp_apply(&src, &v, &user, &format!("{name} <{mail}>"))
                }
                Ok(src) => dhcp_preview(&src, &v),
                Err(e) => json!({"ok": false, "error": e}),
            },
            "/api/settings/probe" => probe(&v),
            "/api/settings" => match settings::apply(&v) {
                Ok(()) => {
                    forget_cache();
                    eprintln!("settings changed by {user}: {} {}", v["op"].as_str().unwrap_or(""), v["switch"]["name"].as_str().or(v["name"].as_str()).unwrap_or(""));
                    json!({"ok": true, "settings": settings_view()})
                }
                Err(e) => json!({"ok": false, "error": e}),
            },
            _ => return respond(r, 404, "not found".into(), "text/plain"),
        };
        return respond(r, 200, out.to_string(), "application/json");
    }
    if r.method() != &Method::Get {
        return respond(r, 405, "read-only".into(), "text/plain");
    }
    match path.as_str() {
        "/" => respond(r, 200, INDEX.into(), "text/html"),
        "/panel.js" => respond(r, 200, JS.into(), "text/javascript"),
        "/panel.css" => respond(r, 200, CSS.into(), "text/css"),
        "/i18n.js" => respond(r, 200, I18N.into(), "text/javascript"),
        "/api/config" => {
            let mut c = cfg.public.clone();
            c["user"] = json!(user);
            // Per switch: how it is managed, health entries known to be harmless
            // (e.g. a second PSU left unplugged), its own web interface.
            c["dhcp_targets"] = dhcp_targets(cfg);
            c["switches"] = json!(settings::all().iter().map(|s| json!({"name": s.cfg.name, "mode": s.cfg.mode, "ignore": s.cfg.ignore, "url": s.cfg.url})).collect::<Vec<_>>());
            respond(r, 200, c.to_string(), "application/json")
        }
        "/api/fast" => respond(r, 200, fast(), "application/json"),
        "/api/slow" => respond(r, 200, slow(), "application/json"),
        "/api/settings" => respond(r, 200, settings_view().to_string(), "application/json"),
        "/api/dhcp" => {
            let q = r.url().split_once("?switch=").map_or(String::new(), |(_, n)| n.split('&').next().unwrap_or("").to_string());
            match source(cfg, &q) {
                Ok(src) => respond(r, 200, dhcp_view(&src).to_string(), "application/json"),
                Err(e) => respond(r, 404, e, "text/plain"),
            }
        }
        _ => respond(r, 404, "not found".into(), "text/plain"),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: cairn-net /etc/cairn-net.json | cairn-net hash-password");
    if path == "hash-password" {
        // e.g. docker run --rm -i cairn-net hash-password, then a line "name:<hash>" in the users file
        let mut pass = String::new();
        std::io::stdin().read_line(&mut pass).expect("stdin");
        let pass = pass.trim_end_matches(['\r', '\n']);
        assert!(!pass.is_empty(), "empty password");
        println!("{}", login::hash_password(pass));
        return;
    }
    let cfg: &'static Config = Box::leak(Box::new(Config::load(&path)));
    let server = match &cfg.tls {
        Some((cert, key)) => Server::https(&cfg.listen, tiny_http::SslConfig {
            certificate: fs::read(cert).expect("tls cert unreadable"),
            private_key: fs::read(key).expect("tls key unreadable"),
        }),
        None => Server::http(&cfg.listen),
    }
    .expect("cannot listen");
    eprintln!("cairn-net listening on {}", cfg.listen);
    for r in server.incoming_requests() {
        std::thread::spawn(move || handle(cfg, r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefix_round_trip() {
        for n in [0usize, 1, 0x7f, 0x80, 0x3fff, 0x4000, 0x1f_ffff, 0x20_0000, 0x0fff_ffff, 0x1000_0000] {
            let mut out = Vec::new();
            put_len(n, &mut out);
            let mut it = out[1..].iter().copied();
            assert_eq!(get_len(out[0], || Ok(it.next().unwrap())).unwrap() as usize, n, "n={n:#x}");
            assert!(it.next().is_none(), "n={n:#x}: extra bytes");
        }
    }

    #[test]
    fn sentence_encoding() {
        assert_eq!(encode(&["/login", "=name=a"]), b"\x06/login\x07=name=a\x00".to_vec());
    }
}
