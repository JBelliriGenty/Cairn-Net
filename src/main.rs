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
//! Firewall (src/firewall.rs): filter, NAT, mangle, raw, lists, read only.
//!
//! Without such a proxy, "login" in the configuration turns on the built-in
//! sign-in (src/login.rs, HTTPS required: "tls" with cert and key files).
//!
//! Usage: cairn-net /etc/cairn-net.json
//!        cairn-net hash-password   (reads a password on stdin, prints its hash)

mod dhcp;
mod direct;
mod dns;
mod firewall;
mod git;
mod infra;
mod login;
mod servers;
mod settings;
mod statics;
mod topo;

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
const TOPO_JS: &str = include_str!("../www/topo.js");
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
    notify: Option<Notify>,
    public: Value, // what /api/config returns
}

/// Where alerts go: an HTTP endpoint taking the message as body (ntfy topic
/// URL…), with optional headers read from a file (curl -H @file).
struct Notify {
    url: String,
    headers_file: String,
}

impl Notify {
    fn send(&self, title: &str, msg: &str) {
        let mut c = Command::new("curl");
        c.args(["-fsS", "-m", "10", "-o", "/dev/null", "-H", &format!("Title: {title}"), "--data-binary", msg]);
        if !self.headers_file.is_empty() {
            c.arg("-H").arg(format!("@{}", self.headers_file));
        }
        match c.arg("--").arg(&self.url).output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => eprintln!("notify: {}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => eprintln!("notify: curl: {e}"),
        }
    }
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
        topo::init(&data);
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
            notify: v["notify"]["url"].as_str().map(|u| Notify {
                url: u.to_string(),
                headers_file: v["notify"]["headers_file"].as_str().unwrap_or("").to_string(),
            }),
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

    /// Cairn Net's own address on this connection, as the switch sees it.
    fn local_ip(&self) -> Option<std::net::Ipv4Addr> {
        match self.0.get_ref().local_addr().ok()?.ip() {
            std::net::IpAddr::V4(a) => Some(a),
            std::net::IpAddr::V6(_) => None,
        }
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
        "neighbors": print(api, "/ip/neighbor", "interface,identity,address,mac-address,board,platform,interface-name,system-caps,system-caps-enabled", &[])?,
        "hosts": print(api, "/interface/bridge/host", "mac-address,on-interface,vid", &["?local=false"])?,
        "leases": print(api, "/ip/dhcp-server/lease", "address,mac-address,host-name,comment,status,server,dynamic,last-seen,expires-after", &[])?,
        "dhcp_servers": print(api, "/ip/dhcp-server", "name,disabled,invalid,address-pool,interface,lease-time,authoritative,comment", &[])?,
        "dhcp_networks": print(api, "/ip/dhcp-server/network", "address,gateway,dns-server,domain,ntp-server,dhcp-option,comment", &[])?,
        "dhcp_options": print(api, "/ip/dhcp-server/option", "name,code,value,comment", &[])?,
        "dhcp_alerts": print(api, "/ip/dhcp-server/alert", "interface,valid-server,unknown-server,disabled", &[])?,
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
        let names = names(&ips);
        let topology = topo::build(&switches, &arp, &names);
        json!({"switches": switches, "arp": arp, "names": names, "topology": topology}).to_string()
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
    if sw.cfg.mode == "git" && !sw.cfg.git_url.is_empty() && !sw.cfg.git_path.is_empty() {
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
            _ if c.mode == "direct" || (c.mode == "git" && !c.git_url.is_empty() && !c.git_path.is_empty() && cfg.dhcp.as_ref().is_none_or(|d| d.switch != c.name)) =>
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
        Src::Git(sw, _) => Ok((git::read(&sw.cfg, &sw.cfg.git_path)?, None)),
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
    let name = match src {
        Src::File(d) => d.switch.clone(),
        Src::Git(sw, _) | Src::Direct(sw) => sw.cfg.name.clone(),
    };
    let s: Value = serde_json::from_str(&slow()).unwrap_or(Value::Null);
    let router = s["switches"].as_array().into_iter().flatten().find(|x| x["name"] == name.as_str()).cloned().unwrap_or(Value::Null);
    // the names the DNS integration publishes (or, without one, those of the configured server)
    let dns_cfg = settings::get(&name).map(|sw| sw.cfg.clone()).filter(|c| c.dns != "none");
    let names: Vec<(String, String)> = match (&dns_cfg, src) {
        (Some(c), _) => dns::names(&router, &c.dns_servers).into_iter().map(|(ip, n, _)| (ip, n)).collect(),
        (None, Src::File(d) | Src::Git(_, Some(d))) => dhcp::names_file(router["leases"].as_array().map(Vec::as_slice).unwrap_or(&[]), &[d.server.clone()]),
        _ => vec![],
    };
    json!({
        "text": text,
        "entries": dhcp::parse(&text).iter().map(dhcp::to_json).collect::<Vec<_>>(),
        "problems": problems(src, &text),
        "servers": servers::parse(&lines).iter().map(servers::to_json).collect::<Vec<_>>(),
        "names": names.into_iter().map(|(ip, n)| json!([ip, n])).collect::<Vec<_>>(),
        "dns": dns_cfg.map(|c| json!({"kind": c.dns, "targets": c.dns_targets, "servers": c.dns_servers, "status": dns::status(&name)})),
        "router_error": err,
        "static": settings::get(&name).and_then(|sw| static_view(&sw.cfg)),
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
        let r = git::commit(&sw.cfg, &sw.cfg.git_path, author, &msg, |text| {
            let new = dhcp::apply(text, changes(a))?;
            let p = problems(src, &new);
            if p.is_empty() { Ok(new) } else { Err(p.join(" · ")) }
        });
        eprintln!("dhcp committed for {} by {user}: {}", sw.cfg.name, r.as_ref().map_or_else(|e| format!("error: {e}"), |c| c.clone().unwrap_or("nothing to change".into())));
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

/* ---------- Static DNS names (src/statics.rs) ---------- */

fn static_cfg(name: &str) -> Result<SwitchCfg, String> {
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    let c = sw.cfg.clone();
    if c.mode == "git" && !c.git_url.is_empty() && !c.git_dns_path.is_empty() { Ok(c) } else { Err(format!("{name}: no static DNS names file (git mode)")) }
}

/// Names a static entry must not take: the DHCP reservations of the switch's
/// file and the names its leases publish.
fn taken_names(c: &SwitchCfg) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if !c.git_path.is_empty() {
        let text = git::read(c, &c.git_path).unwrap_or_default();
        out.extend(dhcp::parse(&text).into_iter().map(|e| (e.name.to_lowercase(), format!("the DHCP reservation of {}", e.mac))));
    }
    let s: Value = serde_json::from_str(&slow()).unwrap_or(Value::Null);
    let router = s["switches"].as_array().into_iter().flatten().find(|x| x["name"] == c.name.as_str()).cloned().unwrap_or(Value::Null);
    let servers = if c.dns == "none" { vec![] } else { c.dns_servers.clone() };
    out.extend(dns::names(&router, &servers).into_iter().map(|(ip, n, _)| (n, format!("a lease ({ip})"))));
    out
}

fn static_view(c: &SwitchCfg) -> Option<Value> {
    let c = static_cfg(&c.name).ok()?;
    let (text, err) = match git::read(&c, &c.git_dns_path) {
        Ok(t) => (t, Value::Null),
        Err(e) => (String::new(), json!(e)),
    };
    let entries = statics::parse(&text);
    // what the DNS servers of the hosts integration answer (plain queries, no account)
    let servers: Vec<String> = if c.dns == "hosts" {
        c.dns_targets.iter().map(|t| {
            let h = t.rsplit_once('@').map_or(t.as_str(), |x| x.1);
            h.split(':').next().unwrap_or(h).to_string()
        }).collect()
    } else {
        vec![]
    };
    Some(json!({
        "path": c.git_dns_path, "error": err,
        "entries": entries.iter().map(statics::to_json).collect::<Vec<_>>(),
        "problems": statics::check(&entries, &taken_names(&c)),
        "served": statics::served_all(&servers, &entries),
    }))
}

fn static_preview(a: &Value) -> Result<Value, String> {
    let c = static_cfg(a["switch"].as_str().unwrap_or(""))?;
    let text = git::read(&c, &c.git_dns_path)?;
    let new = statics::apply(&text, changes(a))?;
    let problems = statics::check(&statics::parse(&new), &taken_names(&c));
    Ok(json!({"ok": true, "old": text, "new": new, "problems": problems}))
}

fn static_apply(a: &Value, user: &str, author: &str) -> Result<Value, String> {
    let c = static_cfg(a["switch"].as_str().unwrap_or(""))?;
    let taken = taken_names(&c);
    let n = changes(a).len();
    let msg = format!("dns ({}): {n} change{} from Cairn Net", c.git_dns_path, if n > 1 { "s" } else { "" });
    let r = git::commit(&c, &c.git_dns_path, author, &msg, |text| {
        let new = statics::apply(text, changes(a))?;
        let p = statics::check(&statics::parse(&new), &taken);
        if p.is_empty() { Ok(new) } else { Err(p.join(" · ")) }
    });
    eprintln!("dns names committed for {} by {user}: {}", c.name, r.as_ref().map_or_else(|e| format!("error: {e}"), |x| x.clone().unwrap_or("nothing to change".into())));
    r.map(|x| json!({"ok": true, "commit": x}))
}

/* ---------- Infrastructure (src/infra.rs) ---------- */

fn infra_view(name: &str) -> Value {
    let Some(sw) = settings::get(name) else { return json!({"error": format!("{name}: unknown switch")}) };
    let mut v = with_api(&sw, |api| {
        let mut v = infra::read(api)?;
        // the interface Cairn Net reaches the switch through: shown, never offered for editing
        if let Ok(p) = path_of(&sw.cfg, api) {
            v["mgmt_iface"] = json!(infra::mgmt_iface(&v, &p));
        }
        Ok(v)
    }).unwrap_or_else(|e| json!({"error": e}));
    if v["error"].is_null() {
        // outside with_api: not holding the switch's connection while fetching
        v["newest"] = json!(infra::newest(v["update"]["channel"].as_str().unwrap_or("stable")));
    }
    v["name"] = json!(sw.cfg.name);
    v["mode"] = json!(sw.cfg.mode);
    v["rollback_s"] = json!(infra::ROLLBACK_S);
    v
}

/// Firewall page: every switch, read account (src/firewall.rs).
fn firewall_view(name: &str) -> Value {
    let Some(sw) = settings::get(name) else { return json!({"error": format!("{name}: unknown switch")}) };
    let mut v = with_api(&sw, |api| {
        let mut v = firewall::read(api)?;
        v["rollback"] = json!(print(api, "/system/scheduler", "name,next-run", &[&format!("?name={}", infra::ROLLBACK)])?.pop());
        if let Ok(p) = path_of(&sw.cfg, api) {
            v["app_iface"] = json!(p.app.and_then(|a| firewall::app_iface(&v, a)));
            v["app"] = json!(p.app.map(|a| a.to_string()));
        }
        Ok(v)
    }).unwrap_or_else(|e| json!({"error": e}));
    if fw_git(&sw.cfg) {
        v["git"] = json!({"url": sw.cfg.git_url, "branch": sw.cfg.git_branch, "path": sw.cfg.git_fw_path});
        match fw_file(&sw.cfg) {
            Ok(f) => {
                if v["error"].is_null() {
                    v["drift"] = json!(firewall::drift(&v, &f));
                }
                v["file"] = json!(f);
            }
            Err(e) => v["file_error"] = json!(e),
        }
    }
    v["name"] = json!(sw.cfg.name);
    v["mode"] = json!(sw.cfg.mode);
    v["rollback_s"] = json!(infra::ROLLBACK_S);
    v
}

fn path_of(sw: &SwitchCfg, api: &Api) -> Result<infra::Path, String> {
    let switch = (sw.host.as_str(), sw.port).to_socket_addrs().map_err(|e| e.to_string())?
        .find_map(|a| match a.ip() { std::net::IpAddr::V4(v) => Some(v), _ => None }).ok_or("the switch has no IPv4 address")?;
    Ok(infra::Path { switch, app: api.local_ip() })
}

fn direct_switch(v: &Value) -> Result<Arc<Switch>, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    if sw.cfg.mode != "direct" {
        return Err(format!("{name} is not in direct mode: nothing is written to it"));
    }
    Ok(sw)
}

fn infra_preview(v: &Value) -> Result<Value, String> {
    let sw = direct_switch(v)?;
    let w = infra::Want::from_json(&v["want"]);
    let (cmds, undo) = with_api(&sw, |api| {
        let path = path_of(&sw.cfg, api)?;
        infra::plan(&infra::read(api)?, &w, &path)
    })?;
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S}))
}

fn rollback_left(api: &mut Api) -> Result<bool, String> {
    Ok(!print(api, "/system/scheduler", ".id", &[&format!("?name={}", infra::ROLLBACK)])?.is_empty())
}

/// Rollback armed, commands run, then a new connection reads the switch back;
/// if it reads as asked, the rollback countdown starts again from now (the
/// page has ROLLBACK_S seconds to confirm). Otherwise the rollback stays armed.
/// Shared by the Infrastructure and Firewall pages: one change waits at a time.
fn guarded_apply(c: &SwitchCfg, user: &str, what: &str, plan: impl FnOnce(&mut Api) -> Result<(Vec<direct::Cmd>, Vec<String>), String>,
    verify: impl FnOnce(&mut Api) -> Result<bool, String>) -> Result<Value, String> {
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    if rollback_left(&mut api)? {
        return Err("a change is still waiting for its confirmation: confirm it or wait for the rollback".into());
    }
    let (cmds, undo) = plan(&mut api)?;
    if cmds.is_empty() {
        return Ok(json!({"ok": true, "commands": [], "pending": false}));
    }
    let (done, err) = direct::run(&mut api, &[infra::arm(&undo)]);
    if done == 0 {
        return Err(format!("rollback not armed, nothing changed: {}", err.unwrap_or_default()));
    }
    let (done, err) = direct::run(&mut api, &cmds);
    drop(api);
    forget_cache();
    // a new connection: the switch must still be reachable the way Cairn Net reaches it
    let back = Api::connect(c, &c.write_user, &c.write_password).and_then(|mut a| {
        let ok = verify(&mut a)?;
        if ok {
            let clock = print(&mut a, "/system/clock", "date,time", &[])?.pop().unwrap_or_default();
            a.call(&["/system/scheduler/set", &format!("=numbers={}", infra::ROLLBACK), &format!("=start-date={}", clock["date"].as_str().unwrap_or("")),
                &format!("=start-time={}", clock["time"].as_str().unwrap_or(""))])?;
        }
        Ok(ok)
    });
    let verified = matches!(back, Ok(true));
    eprintln!("{what} applied on {} by {user}: {done}/{} commands{}, read back: {}", c.name, cmds.len(),
        err.as_ref().map_or(String::new(), |e| format!(", error: {e}")), back.as_ref().map_or_else(|e| format!("error: {e}"), |ok| if *ok { "as asked".into() } else { "differs".into() }));
    Ok(json!({"ok": err.is_none() && verified, "done": done, "commands": infra::show_all(&cmds), "error": err.or(back.err()),
        "verified": verified, "pending": true, "rollback_s": infra::ROLLBACK_S}))
}

fn infra_apply(v: &Value, user: &str) -> Result<Value, String> {
    let sw = direct_switch(v)?;
    let c = &sw.cfg;
    let w = infra::Want::from_json(&v["want"]);
    guarded_apply(c, user, "infra", |api| {
        let path = path_of(c, api)?;
        infra::plan(&infra::read(api)?, &w, &path)
    }, |api| Ok(infra::matches(&infra::read(api)?, &w)))
}

/// Git mode for the firewall: a repository and a firewall file in it.
fn fw_git(c: &SwitchCfg) -> bool {
    c.mode == "git" && !c.git_url.is_empty() && !c.git_fw_path.is_empty()
}

/// The firewall can be written: direct mode, or git mode with a firewall file.
fn fw_switch(v: &Value) -> Result<Arc<Switch>, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    if sw.cfg.mode != "direct" && !fw_git(&sw.cfg) {
        return Err(format!("{name}: the firewall is read-only (direct mode, or git mode with a firewall file, writes it)"));
    }
    Ok(sw)
}

/// The repository's firewall file (missing: empty, nothing managed yet).
fn fw_file(c: &SwitchCfg) -> Result<firewall::File, String> {
    let text = match git::read(c, &c.git_fw_path) {
        Err(e) if e.contains("No such file") => String::new(),
        r => r?,
    };
    firewall::parse_file(&text).map_err(|e| format!("{}: {e}", c.git_fw_path))
}

/// Commits `rows` as the table of the firewall file (None: already so).
fn fw_commit(c: &SwitchCfg, author: &str, table: &str, rows: Vec<firewall::Row>, msg: &str) -> Result<Option<String>, String> {
    git::commit(c, &c.git_fw_path, author, msg, |text| {
        let mut f = firewall::parse_file(text).map_err(|e| format!("{}: {e}", c.git_fw_path))?;
        if f.get(table) == Some(&rows) {
            return Ok(text.to_string());
        }
        f.insert(table.to_string(), rows);
        Ok(firewall::file_text(&f))
    })
}

// git mode: the table applied and waiting for "Keep", committed then
static FW_PENDING: Mutex<BTreeMap<String, (String, Vec<firewall::Row>)>> = Mutex::new(BTreeMap::new());

/// The firewall of one table as the page wants it (src/firewall.rs). In git
/// mode the page sends the file's rows: they take the ids of the same rows
/// on the switch.
fn firewall_plan(c: &SwitchCfg, api: &mut Api, w: &firewall::Want) -> Result<(Vec<direct::Cmd>, Vec<String>, bool), String> {
    let path = path_of(c, api)?;
    let live = firewall::read(api)?;
    if fw_git(c) {
        let rows: Vec<firewall::Row> = w.rows.iter().map(|(_, r)| r.clone()).collect();
        return firewall::plan(&live, &firewall::want_of(&live, &w.table, &rows, w.ack_conntrack), &path, c.port);
    }
    firewall::plan(&live, w, &path, c.port)
}

fn firewall_preview(v: &Value) -> Result<Value, String> {
    let sw = fw_switch(v)?;
    let w = firewall::Want::from_json(v)?;
    let (cmds, undo, conntrack) = with_api(&sw, |api| firewall_plan(&sw.cfg, api, &w))?;
    Ok(json!({"ok": true, "commands": infra::show_all(&cmds), "rollback": undo, "rollback_s": infra::ROLLBACK_S, "conntrack": conntrack}))
}

fn firewall_apply(v: &Value, user: &str, author: &str) -> Result<Value, String> {
    let sw = fw_switch(v)?;
    let c = &sw.cfg;
    let w = firewall::Want::from_json(v)?;
    if fw_git(c) && c.write_user.is_empty() {
        return Err("applying needs the write account (Settings)".into());
    }
    let mut out = guarded_apply(c, user, &format!("firewall {}", w.table), |api| {
        let (cmds, undo, conntrack) = firewall_plan(c, api, &w)?;
        if conntrack && !w.ack_conntrack {
            return Err("this change starts connection tracking: confirm it in the preview".into());
        }
        Ok((cmds, undo))
    }, |api| Ok(firewall::matches(&firewall::read(api)?, &w)))?;
    if fw_git(c) {
        let rows: Vec<firewall::Row> = w.rows.into_iter().map(|(_, r)| r).collect();
        if out["pending"] == true {
            FW_PENDING.lock().unwrap_or_else(|e| e.into_inner()).insert(c.name.clone(), (w.table, rows));
        } else {
            // the switch already runs it: only the file follows
            let msg = format!("firewall ({}): {} from Cairn Net", c.git_fw_path, w.table);
            out["commit"] = json!(fw_commit(c, author, &w.table, rows, &msg)?);
        }
    }
    Ok(out)
}

/// "Keep": the rollback goes; in git mode the kept table is then committed.
fn firewall_confirm(v: &Value, user: &str, author: &str) -> Result<Value, String> {
    let sw = fw_switch(v)?;
    let c = &sw.cfg;
    let pending = FW_PENDING.lock().unwrap_or_else(|e| e.into_inner()).remove(&c.name);
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    if !rollback_left(&mut api)? {
        return Err("no change waiting: the switch already put the old settings back".into());
    }
    api.call(&["/system/scheduler/remove", &format!("=numbers={}", infra::ROLLBACK)])?;
    eprintln!("firewall confirmed on {} by {user}", c.name);
    forget_cache();
    let mut out = json!({"ok": true});
    if let Some((table, rows)) = pending.filter(|_| fw_git(c)) {
        let msg = format!("firewall ({}): {table} from Cairn Net", c.git_fw_path);
        match fw_commit(c, author, &table, rows, &msg) {
            Ok(sha) => out["commit"] = json!(sha),
            Err(e) => {
                eprintln!("firewall commit for {} failed: {e}", c.name);
                out["commit_error"] = json!(e);
            }
        }
    }
    Ok(out)
}

/// Git mode: the file takes every table as the switch runs it (read account).
fn firewall_import(v: &Value, user: &str, author: &str) -> Result<Value, String> {
    let sw = fw_switch(v)?;
    let c = &sw.cfg;
    if !fw_git(c) {
        return Err(format!("{}: not in git mode with a firewall file", c.name));
    }
    let (live, waiting) = with_api(&sw, |api| Ok((firewall::read(api)?, rollback_left(api)?)))?;
    if waiting {
        return Err("a change is still waiting for its confirmation: import once it is kept or rolled back".into());
    }
    let f = firewall::to_file(&live);
    let msg = format!("firewall ({}): rules of {} imported by Cairn Net", c.git_fw_path, c.name);
    let sha = git::commit(c, &c.git_fw_path, author, &msg, |text| {
        Ok(if firewall::parse_file(text).ok().as_ref() == Some(&f) { text.to_string() } else { firewall::file_text(&f) })
    })?;
    eprintln!("firewall of {} imported by {user}: {}", c.name, sha.as_deref().unwrap_or("nothing to change"));
    Ok(json!({"ok": true, "commit": sha}))
}

/// Every 10 minutes: git-mode firewalls that differ from their file (not
/// while a change waits for its confirmation), notified when that changes.
fn drift_watch(notify: Option<&'static Notify>) {
    std::thread::spawn(move || {
        let mut last: BTreeMap<String, Vec<String>> = BTreeMap::new();
        loop {
            std::thread::sleep(Duration::from_secs(600));
            for sw in settings::all().into_iter().filter(|s| fw_git(&s.cfg)) {
                let name = sw.cfg.name.clone();
                let Ok((live, false)) = with_api(&sw, |api| Ok((firewall::read(api)?, rollback_left(api)?))) else { continue };
                let Ok(f) = fw_file(&sw.cfg) else { continue };
                let d = firewall::drift(&live, &f);
                if last.insert(name.clone(), d.clone()).unwrap_or_default() == d {
                    continue;
                }
                let msg = if d.is_empty() {
                    format!("{name}: the firewall is back in line with {}.", sw.cfg.git_fw_path)
                } else {
                    format!("{name}: the firewall differs from {} ({}). Apply the file or import the switch's rules (Cairn Net, Firewall).", sw.cfg.git_fw_path, d.join(", "))
                };
                eprintln!("drift: {msg}");
                if let Some(n) = notify {
                    n.send("Cairn Net: firewall drift", &msg);
                }
            }
        }
    });
}

/// The page still reaches the switch (new connection): the rollback goes.
fn infra_confirm(v: &Value, user: &str) -> Result<Value, String> {
    let sw = direct_switch(v)?;
    let c = &sw.cfg;
    let mut api = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))?;
    if !rollback_left(&mut api)? {
        return Err("no change waiting: the switch already put the old settings back".into());
    }
    api.call(&["/system/scheduler/remove", &format!("=numbers={}", infra::ROLLBACK)])?;
    eprintln!("infra confirmed on {} by {user}", c.name);
    forget_cache();
    Ok(json!({"ok": true}))
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
            (Method::Get, "/topo.js") => return respond(r, 200, TOPO_JS.into(), "text/javascript"),
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
        // commit author: what the authenticating proxy says (e.g. Authelia: Remote-Name,
        // Remote-Email); with the built-in sign-in these headers come from the browser
        let author = || {
            let proxied = |h: &str| if cfg.login.is_some() { String::new() } else { header(&r, h) };
            let name = Some(proxied("Remote-Name")).filter(|n| !n.is_empty() && !n.contains(['<', '>', '\n'])).unwrap_or_else(|| user.clone());
            let mail = Some(proxied("Remote-Email")).filter(|m| m.contains('@') && !m.contains(['<', '>', ' ', '\n'])).unwrap_or_else(|| format!("{user}@cairn-net"));
            format!("{name} <{mail}>")
        };
        let out = match path.as_str() {
            "/api/dhcp/preview" | "/api/dhcp/apply" => match source(cfg, v["switch"].as_str().unwrap_or("")) {
                Ok(src) if path == "/api/dhcp/apply" => dhcp_apply(&src, &v, &user, &author()),
                Ok(src) => dhcp_preview(&src, &v),
                Err(e) => json!({"ok": false, "error": e}),
            },
            "/api/dns-static/preview" => static_preview(&v).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/dns-static/apply" => static_apply(&v, &user, &author()).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/infra/preview" => infra_preview(&v).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/infra/apply" => infra_apply(&v, &user).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/firewall/preview" => firewall_preview(&v).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/firewall/apply" => firewall_apply(&v, &user, &author()).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/firewall/confirm" => firewall_confirm(&v, &user, &author()).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/firewall/import" => firewall_import(&v, &user, &author()).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/infra/confirm" => infra_confirm(&v, &user).unwrap_or_else(|e| json!({"ok": false, "error": e})),
            "/api/settings/probe" => probe(&v),
            // the app's own data, nothing is written to the switches
            "/api/kinds" => match topo::set_label(&v) {
                Ok(()) => {
                    forget_cache(); // the next /api/slow carries the new name and kind
                    json!({"ok": true, "kinds": topo::kinds()})
                }
                Err(e) => json!({"ok": false, "error": e}),
            },
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
        "/topo.js" => respond(r, 200, TOPO_JS.into(), "text/javascript"),
        "/api/kinds" => respond(r, 200, json!({"kinds": topo::kinds(), "all": topo::KINDS}).to_string(), "application/json"),
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
        "/api/infra" => {
            let q = r.url().split_once("?switch=").map_or(String::new(), |(_, n)| n.split('&').next().unwrap_or("").to_string());
            respond(r, 200, infra_view(&q).to_string(), "application/json")
        }
        "/api/firewall" => {
            let q = r.url().split_once("?switch=").map_or(String::new(), |(_, n)| n.split('&').next().unwrap_or("").to_string());
            respond(r, 200, firewall_view(&q).to_string(), "application/json")
        }
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
    drift_watch(cfg.notify.as_ref());
    // DNS integrations: every 2 minutes, from the shared read of the switches
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(20));
        dns::publish(&serde_json::from_str(&slow()).unwrap_or(Value::Null));
        std::thread::sleep(Duration::from_secs(100));
    });
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
