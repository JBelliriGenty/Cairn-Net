//! Infrastructure page: what a switch is (identity, addresses, default
//! route, DNS, bridges, NAT, versions), as WebFig's Quick Set shows it, and
//! the low-risk changes of direct mode: identity, DNS servers, one address
//! and the default gateway.
//!
//! Every change is guarded by a rollback: before the first command, a
//! RouterOS scheduler is added that puts the old values back after
//! `ROLLBACK_S` seconds and removes itself. Cairn Net then reads the switch
//! again through a new connection; only a confirmation (a second new
//! connection, so the switch is still reachable) removes the scheduler.
//! The interface Cairn Net reaches the switch through is never edited, and no
//! new address may overlap the networks of either end of that path.

use crate::direct::{show, Cmd};
use crate::{print, Api};
use serde_json::{json, Value};
use std::net::Ipv4Addr;

pub const ROLLBACK: &str = "cairn-net-rollback";
pub const ROLLBACK_S: u64 = 60;

/// Everything the page shows, read with the read account.
/// The name a switch is asked to resolve: MikroTik's own update server.
pub const RESOLVE_NAME: &str = "upgrade.mikrotik.com";

/// Can the switch resolve a name itself? Its NTP servers by name and its
/// update check depend on it, the clients do not (they get their DNS by
/// DHCP), so a stale `/ip dns servers` goes unnoticed. Ok(address) or Err(why).
pub fn resolves(api: &mut Api) -> Result<String, String> {
    let script = format!(":do {{ :put [:resolve {RESOLVE_NAME}] }} on-error={{ :put \"no answer\" }}");
    let (_, done) = api.call_done(&["/execute", &format!("=script={script}"), "=as-string="])?;
    resolve_answer(done.get("ret").and_then(Value::as_str).unwrap_or(""))
}

fn resolve_answer(out: &str) -> Result<String, String> {
    let out = out.trim();
    if out.parse::<std::net::IpAddr>().is_ok() {
        Ok(out.to_string())
    } else {
        Err(if out.is_empty() { "no answer".into() } else { out.to_string() })
    }
}

pub fn read(api: &mut Api) -> Result<Value, String> {
    Ok(json!({
        "identity": print(api, "/system/identity", "name", &[])?.pop().map(|v| v["name"].clone()),
        "resource": print(api, "/system/resource", "uptime,version,board-name,architecture-name", &[])?.pop(),
        "routerboard": print(api, "/system/routerboard", "model,current-firmware,upgrade-firmware", &[])?.pop(),
        "update": print(api, "/system/package/update", "channel,installed-version,latest-version,status", &[])?.pop(),
        "addresses": print(api, "/ip/address", ".id,address,network,interface,disabled,dynamic,invalid,comment", &[])?,
        "dhcp_clients": print(api, "/ip/dhcp-client", "interface,status,address,gateway,disabled", &[])?,
        "routes": print(api, "/ip/route", ".id,dst-address,gateway,immediate-gw,distance,active,dynamic,disabled,routing-table,comment", &["?dst-address=0.0.0.0/0"])?,
        "dns": print(api, "/ip/dns", "servers,dynamic-servers,allow-remote-requests", &[])?.pop(),
        "ifaces": print(api, "/interface", "name,type,running,disabled,slave,mac-address,comment", &[])?,
        "vlan_ifaces": print(api, "/interface/vlan", "name,vlan-id,interface", &[])?,
        "bridges": print(api, "/interface/bridge", "name,vlan-filtering,protocol-mode,comment", &[])?,
        "ports": print(api, "/interface/bridge/port", ".id,interface,bridge,pvid,disabled", &[])?,
        "nat": print(api, "/ip/firewall/nat", "chain,action,src-address,dst-address,in-interface,out-interface,protocol,dst-port,to-addresses,to-ports,disabled,dynamic,comment", &[])?,
        "rollback": print(api, "/system/scheduler", "name,next-run", &[&format!("?name={ROLLBACK}")])?.pop(),
        // management services (dynamic rows are open connections); "address" before RouterOS 7.2x, "available-from" after
        "services": print(api, "/ip/service", ".id,name,port,address,available-from,disabled,dynamic,invalid", &[])?
            .into_iter().filter(|r| !yes(r, "dynamic")).collect::<Vec<_>>(),
        "mac_winbox": print(api, "/tool/mac-server/mac-winbox", "allowed-interface-list", &[])?.pop(),
        "iface_lists": print(api, "/interface/list", "name", &[])?,
    }))
}

/// The newest RouterOS of a channel, as MikroTik publishes it (the read
/// account may not run check-for-updates). Cached 6 h; None when unreachable.
pub fn newest(channel: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<HashMap<String, (Instant, Option<String>)>>> = Mutex::new(None);
    if !channel.chars().all(|c| c.is_ascii_alphanumeric()) || channel.is_empty() {
        return None;
    }
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some((t, v)) = m.get(channel) {
        if t.elapsed() < Duration::from_secs(if v.is_some() { 6 * 3600 } else { 600 }) {
            return v.clone();
        }
    }
    let host = "upgrade.mikrotik.com";
    let fetch = || -> Option<String> {
        let addr = std::net::ToSocketAddrs::to_socket_addrs(&(host, 443)).ok()?.next()?;
        let tcp = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)).ok()?;
        tcp.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let c = openssl::ssl::SslConnector::builder(openssl::ssl::SslMethod::tls()).ok()?.build();
        let mut tls = c.connect(host, tcp).ok()?; // verified against the system CAs
        write!(tls, "GET /routeros/NEWESTa7.{channel} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: cairn-net\r\n\r\n").ok()?;
        let mut body = String::new();
        let _ = tls.take(4096).read_to_string(&mut body);
        let (head, b) = body.split_once("\r\n\r\n")?;
        let v = b.split_whitespace().next()?;
        (head.starts_with("HTTP/1.") && head.split_whitespace().nth(1) == Some("200") && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')).then(|| v.to_string())
    };
    let v = fetch();
    m.insert(channel.to_string(), (Instant::now(), v.clone()));
    v
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn yes(v: &Value, k: &str) -> bool {
    s(v, k) == "true" || s(v, k) == "yes"
}

/// A RouterOS script string literal.
pub fn q(v: &str) -> String {
    let mut o = String::from("\"");
    for c in v.chars() {
        if matches!(c, '\\' | '"' | '$') {
            o.push('\\');
        }
        o.push(c);
    }
    o + "\""
}

/// "a.b.c.d/len" -> (address, network, len)
pub fn cidr(v: &str) -> Option<(Ipv4Addr, u32, u32)> {
    let (a, l) = v.split_once('/')?;
    let ip: Ipv4Addr = a.parse().ok()?;
    let len: u32 = l.parse().ok().filter(|l| (1..=32).contains(l))?;
    let mask = u32::MAX.checked_shl(32 - len).unwrap_or(0);
    Some((ip, u32::from(ip) & mask, len))
}

pub fn contains(net: &str, ip: Ipv4Addr) -> bool {
    cidr(net).is_some_and(|(_, n, l)| u32::from(ip) & u32::MAX.checked_shl(32 - l).unwrap_or(0) == n)
}

pub fn overlap(a: &str, b: &str) -> bool {
    match (cidr(a), cidr(b)) {
        (Some((_, na, la)), Some((_, nb, lb))) => {
            let l = la.min(lb);
            let m = u32::MAX.checked_shl(32 - l).unwrap_or(0);
            na & m == nb & m
        }
        _ => false,
    }
}

/// The two ends of the management path: the switch address Cairn Net
/// connects to, and Cairn Net's own address as the switch sees it.
pub struct Path {
    pub switch: Ipv4Addr,
    pub app: Option<Ipv4Addr>,
}

/// What the page asks for. A missing field is left as it is.
#[derive(Default)]
pub struct Want {
    pub identity: Option<String>,
    pub dns: Option<Vec<String>>,
    pub address: Option<(String, String)>, // .id, new "a.b.c.d/len"
    pub gateway: Option<String>,
    pub mode: Option<Mode>,
    pub services: Vec<(String, bool, String)>, // .id, disabled, "available-from" (comma list, empty = everyone)
    pub mac_winbox: Option<String>,           // allowed interface list
}

/// Router or bridge, Quick Set style, for one port: in router mode the port
/// leaves the bridge and traffic out of it is masqueraded; in bridge mode it
/// joins the bridge and loses its masquerade. Off unless the switch's
/// setting allows it (prototype).
#[derive(Clone, Debug)]
pub struct Mode {
    pub iface: String,
    pub bridge: String,
    pub router: bool,
}

impl Want {
    pub fn from_json(v: &Value) -> Want {
        let st = |k: &str| v[k].as_str().map(|x| x.trim().to_string());
        let m = &v["mode"];
        Want {
            identity: st("identity"),
            dns: v["dns"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()),
            address: v["address"]["id"].as_str().zip(v["address"]["address"].as_str()).map(|(i, a)| (i.to_string(), a.trim().to_string())),
            gateway: st("gateway"),
            mode: m["iface"].as_str().zip(m["bridge"].as_str()).map(|(i, b)| Mode { iface: i.into(), bridge: b.into(), router: m["router"].as_bool().unwrap_or(false) }),
            services: v["services"].as_array().into_iter().flatten().filter_map(|x| Some((x["id"].as_str()?.to_string(), x["disabled"].as_bool()?, x["from"].as_str().unwrap_or("").to_string()))).collect(),
            mac_winbox: st("mac_winbox"),
        }
    }
}

/// The new address of the switch when the change moves the one Cairn Net
/// connects to (the app then follows it, see main.rs move_apply).
pub fn moves(live: &Value, w: &Want, path: &Path) -> Option<Ipv4Addr> {
    let (id, new) = w.address.as_ref()?;
    let a = live["addresses"].as_array()?.iter().find(|a| s(a, ".id") == id)?;
    let (old, _, _) = cidr(s(a, "address"))?;
    let (ip, _, _) = cidr(new)?;
    (old == path.switch && ip != old).then_some(ip)
}

/// The interface holding the switch's management address.
pub fn mgmt_iface<'a>(live: &'a Value, path: &Path) -> Option<&'a str> {
    live["addresses"].as_array()?.iter().find(|a| cidr(s(a, "address")).is_some_and(|(ip, _, _)| ip == path.switch)).map(|a| s(a, "interface"))
}

fn static_default_routes(live: &Value) -> Vec<&Value> {
    live["routes"].as_array().into_iter().flatten()
        .filter(|r| s(r, "dst-address") == "0.0.0.0/0" && !yes(r, "dynamic") && matches!(s(r, "routing-table"), "" | "main")).collect()
}

/// The commands, and the RouterOS script that undoes them.
pub fn plan(live: &Value, w: &Want, path: &Path) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let (mut cmds, mut undo) = (Vec::<Cmd>::new(), Vec::<String>::new());
    let kv = |p: &[(&str, &str)]| p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    if w.mode.is_some() {
        return Err("router/bridge mode is changed on its own".into());
    }

    if let Some(id) = &w.identity {
        let old = live["identity"].as_str().unwrap_or("");
        if id.is_empty() || id.chars().count() > 64 || id.chars().any(char::is_control) {
            return Err("invalid identity (1 to 64 characters)".into());
        }
        if id != old {
            cmds.push(("/system/identity/set".into(), kv(&[("name", id)])));
            undo.push(format!("/system identity set name={}", q(old)));
        }
    }

    if let Some(dns) = &w.dns {
        if dns.len() > 4 || dns.iter().any(|d| d.parse::<Ipv4Addr>().is_err()) {
            return Err("DNS servers: up to 4 IPv4 addresses".into());
        }
        let old = s(&live["dns"], "servers");
        let new = dns.join(",");
        if new != old {
            cmds.push(("/ip/dns/set".into(), kv(&[("servers", &new)])));
            undo.push(format!("/ip dns set servers={}", q(old)));
        }
    }

    // networks the switch will have after the change (for the gateway check)
    let mut nets: Vec<String> = live["addresses"].as_array().into_iter().flatten().filter(|a| !yes(a, "disabled")).map(|a| s(a, "address").to_string()).collect();
    if let Some((id, new)) = &w.address {
        let a = live["addresses"].as_array().into_iter().flatten().find(|a| s(a, ".id") == id).ok_or("this address is gone from the switch: reload")?;
        let old = s(a, "address");
        if yes(a, "dynamic") {
            return Err(format!("{old} is dynamic (DHCP client): it cannot be edited"));
        }
        // the address Cairn Net connects to may move (the app follows it);
        // nothing else on that interface is edited
        let moving = cidr(old).is_some_and(|(ip, _, _)| ip == path.switch);
        if mgmt_iface(live, path) == Some(s(a, "interface")) && !moving {
            return Err(format!("{}: Cairn Net reaches the switch through this interface, it is not edited from here", s(a, "interface")));
        }
        let (ip, _, _) = cidr(new).ok_or_else(|| format!("{new}: invalid address (a.b.c.d/len)"))?;
        if moving {
            if Some(ip) == path.app || cidr(new).is_some_and(|(_, n, l)| l < 31 && (u32::from(ip) == n || u32::from(ip) == n | !u32::MAX.checked_shl(32 - l).unwrap_or(0))) {
                return Err(format!("{new}: not a usable address for the switch"));
            }
        } else {
            // the network Cairn Net's packets come from (replies leave by it)
            if let Some(app) = path.app.filter(|a| contains(old, *a)) {
                return Err(format!("{}: Cairn Net ({app}) is on this network, the switch answers it through here", s(a, "interface")));
            }
            for end in std::iter::once(path.switch).chain(path.app) {
                if contains(new, end) || ip == end {
                    return Err(format!("{new} would take the network of the management path ({end})"));
                }
            }
        }
        for n in &nets {
            if n != old && overlap(n, new) {
                return Err(format!("{new} overlaps {n}, already on the switch"));
            }
        }
        if new != old {
            // RouterOS keeps the old `network` on a set (seen on 7.25): send it too
            let net = cidr(new).map(|(ip, n, l)| if l == 32 { ip } else { Ipv4Addr::from(n) }).unwrap_or(ip).to_string();
            cmds.push(("/ip/address/set".into(), kv(&[(".id", id), ("address", new), ("network", &net)])));
            let old_net = Some(s(a, "network")).filter(|n| !n.is_empty()).map_or(String::new(), |n| format!(" network={}", q(n)));
            undo.push(format!("/ip address set [find where .id={id}] address={}{old_net}", q(old)));
            nets.retain(|n| n != old);
            nets.push(new.clone());
        }
    }

    if let Some(gw) = &w.gateway {
        let ip: Ipv4Addr = gw.parse().map_err(|_| format!("{gw}: invalid gateway"))?;
        if !nets.iter().any(|n| contains(n, ip)) {
            return Err(format!("{gw}: the gateway must be in one of the switch's networks"));
        }
        match static_default_routes(live).as_slice() {
            [] => {
                cmds.push(("/ip/route/add".into(), kv(&[("dst-address", "0.0.0.0/0"), ("gateway", gw)])));
                undo.push(format!("/ip route remove [find where dst-address=0.0.0.0/0 and gateway={} and !dynamic]", q(gw)));
            }
            [r] if s(r, "gateway") != gw => {
                cmds.push(("/ip/route/set".into(), kv(&[(".id", s(r, ".id")), ("gateway", gw)])));
                undo.push(format!("/ip route set [find where .id={}] gateway={}", s(r, ".id"), q(s(r, "gateway"))));
            }
            [_] => {}
            _ => return Err("several default routes: change them on the switch".into()),
        }
    }

    plan_services(live, w, path, &mut cmds, &mut undo)?;

    // moving the management address: last, its answer may never come back
    if moves(live, w, path).is_some() {
        if let Some(i) = cmds.iter().position(|c| c.0 == "/ip/address/set") {
            let c = cmds.remove(i);
            cmds.push(c);
        }
    }
    if !undo.is_empty() {
        undo.reverse(); // undo in reverse order
        finish(&mut undo);
    }
    Ok((cmds, undo))
}

/// The property holding a service's allowed addresses on this RouterOS.
fn from_key(r: &Value) -> &'static str {
    if r.get("available-from").is_some() || r.get("address").is_none() { "available-from" } else { "address" }
}

/// "a, b c" -> "a,b,c", every part an IPv4 address or network.
fn from_list(v: &str) -> Result<String, String> {
    let parts: Vec<&str> = v.split([',', ' ', ';', '\n']).map(str::trim).filter(|p| !p.is_empty()).collect();
    for p in &parts {
        if cidr(p).is_none() && p.parse::<Ipv4Addr>().is_err() {
            return Err(format!("{p}: invalid address (a.b.c.d or a.b.c.d/len)"));
        }
    }
    Ok(parts.join(","))
}

fn allows(from: &str, ip: Ipv4Addr) -> bool {
    from.is_empty() || from.split(',').any(|p| p.parse::<Ipv4Addr>() == Ok(ip) || contains(p, ip))
}

/// Management services (/ip service) and MAC-WinBox. api-ssl carries Cairn
/// Net: never disabled, never closed to Cairn Net's address. A way in
/// besides Cairn Net stays: SSH, WinBox or MAC-WinBox.
fn plan_services(live: &Value, w: &Want, path: &Path, cmds: &mut Vec<Cmd>, undo: &mut Vec<String>) -> Result<(), String> {
    let svcs: Vec<Value> = live["services"].as_array().cloned().unwrap_or_default();
    // state after the change: name -> (disabled, from)
    let mut after: Vec<(String, bool, String)> = svcs.iter().map(|r| (s(r, "name").to_string(), yes(r, "disabled"), s(r, from_key(r)).to_string())).collect();
    for (id, dis, from) in &w.services {
        let i = svcs.iter().position(|r| s(r, ".id") == id).ok_or("this service is gone from the switch: reload")?;
        let r = &svcs[i];
        let (name, key, from) = (s(r, "name"), from_key(r), from_list(from)?);
        if name == "api-ssl" {
            if *dis {
                return Err("api-ssl: Cairn Net reaches the switch through it, it stays on".into());
            }
            if let Some(app) = path.app.filter(|a| !allows(&from, *a)) {
                return Err(format!("api-ssl: {from} leaves out Cairn Net ({app})"));
            }
        }
        if *dis != yes(r, "disabled") || from != s(r, key) {
            let yn = |b: bool| if b { "yes" } else { "no" };
            cmds.push(("/ip/service/set".into(), vec![(".id".into(), id.clone()), ("disabled".into(), yn(*dis).into()), (key.into(), from.clone())]));
            undo.push(format!("/ip service set [find where .id={id}] disabled={} {key}={}", yn(yes(r, "disabled")), q(s(r, key))));
            after[i] = (name.to_string(), *dis, from);
        }
    }
    let old_mac = s(&live["mac_winbox"], "allowed-interface-list");
    let mut mac = old_mac.to_string();
    if let Some(m) = w.mac_winbox.as_ref().filter(|m| *m != old_mac) {
        let lists: Vec<&str> = live["iface_lists"].as_array().into_iter().flatten().map(|l| s(l, "name")).collect();
        if !lists.contains(&m.as_str()) && m != "all" && m != "none" {
            return Err(format!("{m}: no such interface list"));
        }
        cmds.push(("/tool/mac-server/mac-winbox/set".into(), vec![("allowed-interface-list".into(), m.clone())]));
        undo.push(format!("/tool mac-server mac-winbox set allowed-interface-list={}", q(old_mac)));
        mac.clone_from(m);
    }
    let open = |n: &str| after.iter().any(|(name, dis, _)| name == n && !dis);
    if (!w.services.is_empty() || w.mac_winbox.is_some()) && !open("ssh") && !open("winbox") && mac == "none" {
        return Err("keep a way in besides Cairn Net: SSH, WinBox or MAC-WinBox".into());
    }
    Ok(())
}

const ROUTER_NAT: &str = "cairn-net: router mode";

/// Router/bridge for one port (`m`), with the firewall as firewall::read
/// gives it (`fw`) for the NAT and conntrack checks.
pub fn plan_mode(live: &Value, fw: &Value, m: &Mode, path: &Path, port: u16) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let kv = |p: &[(&str, &str)]| p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
    let arr = |k: &str| live[k].as_array().cloned().unwrap_or_default();
    let ifc = arr("ifaces").into_iter().find(|i| s(i, "name") == m.iface).ok_or_else(|| format!("{}: no such interface", m.iface))?;
    if s(&ifc, "type") == "bridge" || yes(&ifc, "slave") && !arr("ports").iter().any(|p| s(p, "interface") == m.iface) {
        return Err(format!("{}: a bridge or a bond member cannot be switched", m.iface));
    }
    if !arr("bridges").iter().any(|b| s(b, "name") == m.bridge) {
        return Err(format!("{}: no such bridge", m.bridge));
    }
    // the management path: the interfaces holding either end, and the bridges under them (VLANs)
    let mut path_ifs: Vec<String> = mgmt_iface(live, path).into_iter().chain(path.app.and_then(|a| crate::firewall::app_iface(fw, a))).map(String::from).collect();
    for v in arr("vlan_ifaces") {
        if path_ifs.iter().any(|x| x == s(&v, "name")) {
            path_ifs.push(s(&v, "interface").to_string());
        }
    }
    if path.app.is_none() {
        return Err("Cairn Net's own address is unknown: no mode change".into());
    }
    for x in [&m.iface, &m.bridge] {
        if path_ifs.contains(x) {
            return Err(format!("{x} carries Cairn Net's path to the switch: it is not switched from here"));
        }
    }
    let port_row = arr("ports").into_iter().find(|p| s(p, "interface") == m.iface);
    if let Some(p) = &port_row {
        if s(p, "bridge") != m.bridge {
            return Err(format!("{} is a port of {}, not of {}", m.iface, s(p, "bridge"), m.bridge));
        }
    }
    let nat: Vec<Value> = fw["tables"]["nat"].as_array().cloned().unwrap_or_default();
    let masq = |r: &Value| !yes(r, "dynamic") && s(r, "chain") == "srcnat" && s(r, "action") == "masquerade" && s(r, "out-interface") == m.iface;
    let (mut cmds, mut undo, mut after_nat) = (Vec::<Cmd>::new(), Vec::<String>::new(), nat.clone());
    if m.router {
        if let Some(p) = &port_row {
            cmds.push(("/interface/bridge/port/remove".into(), kv(&[(".id", s(p, ".id"))])));
            let pvid = Some(s(p, "pvid")).filter(|v| !v.is_empty() && *v != "1").map_or(String::new(), |v| format!(" pvid={v}"));
            undo.push(format!("/interface bridge port add bridge={} interface={}{pvid}", q(&m.bridge), q(&m.iface)));
        }
        if !nat.iter().any(|r| masq(r) && !yes(r, "disabled")) {
            let row = json!({"chain": "srcnat", "action": "masquerade", "out-interface": m.iface, "comment": ROUTER_NAT});
            cmds.push(("/ip/firewall/nat/add".into(), kv(&[("chain", "srcnat"), ("action", "masquerade"), ("out-interface", &m.iface), ("comment", ROUTER_NAT)])));
            undo.push(format!("/ip firewall nat remove [find where comment={} and out-interface={}]", q(ROUTER_NAT), q(&m.iface)));
            after_nat.push(row);
            let t = &fw["tracking"];
            if s(t, "enabled") == "no" {
                return Err("connection tracking is disabled on the switch: NAT cannot work".into());
            }
            if !yes(t, "active") {
                return Err("connection tracking is off: the first NAT rule starts it and may cut the open connections, Cairn Net's own included. Start it on purpose first (Firewall page)".into());
            }
        }
    } else {
        // ponytail: removed masquerade rows come back at the end of the table, not at their old place
        for r in nat.iter().filter(|r| masq(r)) {
            cmds.push(("/ip/firewall/nat/remove".into(), kv(&[(".id", s(r, ".id"))])));
            undo.push(crate::firewall::add_line("/ip/firewall/nat", &crate::firewall::props(r)));
        }
        after_nat.retain(|r| !masq(r));
        if port_row.is_none() {
            if arr("addresses").iter().any(|a| s(a, "interface") == m.iface) {
                return Err(format!("{} has an IP address: move it first, a bridge port loses it", m.iface));
            }
            cmds.push(("/interface/bridge/port/add".into(), kv(&[("bridge", &m.bridge), ("interface", &m.iface)])));
            undo.push(format!("/interface bridge port remove [find where interface={} and bridge={}]", q(&m.iface), q(&m.bridge)));
        }
    }
    let mut after = fw.clone();
    after["tables"]["nat"] = json!(after_nat);
    crate::firewall::guard(&after, path, port)?;
    if !undo.is_empty() {
        undo.reverse();
        finish(&mut undo);
    }
    Ok((cmds, undo))
}

pub fn matches_mode(live: &Value, fw: &Value, m: &Mode) -> bool {
    let in_bridge = live["ports"].as_array().into_iter().flatten().any(|p| s(p, "interface") == m.iface && s(p, "bridge") == m.bridge);
    let masq = fw["tables"]["nat"].as_array().into_iter().flatten()
        .any(|r| !yes(r, "dynamic") && !yes(r, "disabled") && s(r, "chain") == "srcnat" && s(r, "action") == "masquerade" && s(r, "out-interface") == m.iface);
    in_bridge != m.router && masq == m.router
}

/// RouterOS uptime ("1w2d3h4m5s") in seconds.
pub fn uptime_s(v: &str) -> u64 {
    let (mut total, mut n) = (0, 0);
    for c in v.chars() {
        match c.to_digit(10) {
            Some(d) => n = n * 10 + d as u64,
            None => {
                total += n * match c { 'w' => 604800, 'd' => 86400, 'h' => 3600, 'm' => 60, _ => 1 };
                n = 0;
            }
        }
    }
    total + n
}

/// The end of every rollback script: a log line, then the scheduler goes.
pub fn finish(undo: &mut Vec<String>) {
    undo.push(format!(":log warning {}", q("cairn-net: change not confirmed, rolled back")));
    undo.push(format!("/system scheduler remove [find name={}]", q(ROLLBACK)));
}

/// The scheduler that runs `undo` after ROLLBACK_S seconds.
pub fn arm(undo: &[String]) -> Cmd {
    let kv = |k: &str, v: String| (k.to_string(), v);
    ("/system/scheduler/add".into(), vec![kv("name", ROLLBACK.into()), kv("interval", format!("{ROLLBACK_S}s")), kv("policy", "read,write".into()),
        kv("comment", "Cairn Net: puts the old settings back unless the change is confirmed".into()), kv("on-event", undo.join("\n"))])
}

/// Whether the switch now runs what was asked.
pub fn matches(live: &Value, w: &Want) -> bool {
    let id_ok = w.identity.as_ref().is_none_or(|i| live["identity"].as_str() == Some(i));
    let dns_ok = w.dns.as_ref().is_none_or(|d| s(&live["dns"], "servers") == d.join(","));
    let addr_ok = w.address.as_ref().is_none_or(|(id, a)| live["addresses"].as_array().into_iter().flatten().any(|x| s(x, ".id") == id && s(x, "address") == a));
    let gw_ok = w.gateway.as_ref().is_none_or(|g| static_default_routes(live).iter().any(|r| s(r, "gateway") == g));
    let svc_ok = w.services.iter().all(|(id, dis, from)| live["services"].as_array().into_iter().flatten()
        .any(|r| s(r, ".id") == id && yes(r, "disabled") == *dis && from_list(from).is_ok_and(|f| s(r, from_key(r)) == f)));
    let mac_ok = w.mac_winbox.as_ref().is_none_or(|m| s(&live["mac_winbox"], "allowed-interface-list") == m);
    id_ok && dns_ok && addr_ok && gw_ok && svc_ok && mac_ok
}

pub fn show_all(cmds: &[Cmd]) -> Vec<String> {
    cmds.iter().map(show).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_answers() {
        assert_eq!(resolve_answer("159.148.147.251
"), Ok("159.148.147.251".into()));
        assert_eq!(resolve_answer("no answer"), Err("no answer".into()));
        assert_eq!(resolve_answer(""), Err("no answer".into()));
    }

    fn live() -> Value {
        json!({
            "identity": "R1",
            "dns": {"servers": "192.168.1.2"},
            "addresses": [
                {".id": "*2", "address": "10.99.0.1/24", "interface": "test-lan", "disabled": "false", "dynamic": "false"},
                {".id": "*3", "address": "192.168.1.86/24", "network": "192.168.1.0", "interface": "ether1", "disabled": "false", "dynamic": "false"},
                {".id": "*4", "address": "172.20.0.1/24", "interface": "ether1", "disabled": "false", "dynamic": "false"},
            ],
            "routes": [
                {".id": "*8", "dst-address": "0.0.0.0/0", "gateway": "192.168.1.1", "dynamic": "false", "routing-table": "main"},
                {".id": "*9", "dst-address": "0.0.0.0/0", "gateway": "10.0.0.1", "dynamic": "true", "routing-table": "main"},
            ],
        })
    }

    fn path() -> Path {
        Path { switch: "192.168.1.86".parse().unwrap(), app: Some("192.168.99.69".parse().unwrap()) }
    }

    #[test]
    fn same_values_mean_nothing_to_do() {
        let w = Want { identity: Some("R1".into()), dns: Some(vec!["192.168.1.2".into()]), address: Some(("*2".into(), "10.99.0.1/24".into())), gateway: Some("192.168.1.1".into()), ..Default::default() };
        let (c, u) = plan(&live(), &w, &path()).unwrap();
        assert!(c.is_empty() && u.is_empty());
        assert!(matches(&live(), &w));
    }

    #[test]
    fn changes_and_their_rollback() {
        let w = Want { identity: Some("R\"2".into()), dns: Some(vec!["1.1.1.1".into(), "9.9.9.9".into()]), address: Some(("*2".into(), "10.98.0.1/16".into())), gateway: Some("10.98.0.254".into()), ..Default::default() };
        let (c, u) = plan(&live(), &w, &path()).unwrap();
        assert_eq!(show_all(&c), vec![
            "/system/identity/set name=R\"2",
            "/ip/dns/set servers=1.1.1.1,9.9.9.9",
            "/ip/address/set .id=*2 address=10.98.0.1/16 network=10.98.0.0",
            "/ip/route/set .id=*8 gateway=10.98.0.254",
        ]);
        assert_eq!(u, vec![
            "/ip route set [find where .id=*8] gateway=\"192.168.1.1\"",
            "/ip address set [find where .id=*2] address=\"10.99.0.1/24\"",
            "/ip dns set servers=\"192.168.1.2\"",
            "/system identity set name=\"R1\"",
            ":log warning \"cairn-net: change not confirmed, rolled back\"",
            "/system scheduler remove [find name=\"cairn-net-rollback\"]",
        ]);
        assert!(!matches(&live(), &w));
    }

    #[test]
    fn the_management_path_is_never_touched() {
        let p = path();
        let e = |w: Want| plan(&live(), &w, &p).unwrap_err();
        // the interface Cairn Net connects through
        assert!(e(Want { address: Some(("*4".into(), "172.20.0.2/24".into())), ..Default::default() }).contains("reaches the switch"));
        // another interface taking the switch's or the app's network
        assert!(e(Want { address: Some(("*2".into(), "192.168.1.200/24".into())), ..Default::default() }).contains("management path"));
        assert!(e(Want { address: Some(("*2".into(), "192.168.0.1/16".into())), ..Default::default() }).contains("management path"));
        assert!(e(Want { address: Some(("*2".into(), "192.168.99.1/24".into())), ..Default::default() }).contains("management path"));
        // gateway out of every network, bad input
        assert!(e(Want { gateway: Some("172.16.0.1".into()), ..Default::default() }).contains("networks"));
        assert!(e(Want { address: Some(("*2".into(), "10.99.0.1".into())), ..Default::default() }).contains("invalid"));
        assert!(e(Want { dns: Some(vec!["dns.example".into()]), ..Default::default() }).contains("IPv4"));
        // the network Cairn Net is on, held by another interface (routed management path)
        let routed = Path { switch: "192.168.1.86".parse().unwrap(), app: Some("10.99.0.50".parse().unwrap()) };
        assert!(plan(&live(), &Want { address: Some(("*2".into(), "10.98.0.1/24".into())), ..Default::default() }, &routed).unwrap_err().contains("is on this network"));
    }

    #[test]
    fn a_missing_default_route_is_added_and_undone() {
        let mut l = live();
        l["routes"] = json!([]);
        let (c, u) = plan(&l, &Want { gateway: Some("10.99.0.254".into()), ..Default::default() }, &path()).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/route/add dst-address=0.0.0.0/0 gateway=10.99.0.254"]);
        assert_eq!(u[0], "/ip route remove [find where dst-address=0.0.0.0/0 and gateway=\"10.99.0.254\" and !dynamic]");
    }

    #[test]
    fn the_management_address_moves_last() {
        let p = path();
        let w = Want { identity: Some("R2".into()), address: Some(("*3".into(), "192.168.1.87/24".into())), gateway: Some("192.168.1.254".into()), ..Default::default() };
        assert_eq!(moves(&live(), &w, &p), Some("192.168.1.87".parse().unwrap()));
        let (c, u) = plan(&live(), &w, &p).unwrap();
        assert_eq!(c.last().unwrap().0, "/ip/address/set");
        assert!(u.contains(&"/ip address set [find where .id=*3] address=\"192.168.1.86/24\" network=\"192.168.1.0\"".to_string()));
        // same address, other mask: no move
        assert_eq!(moves(&live(), &Want { address: Some(("*3".into(), "192.168.1.86/23".into())), ..Default::default() }, &p), None);
        let e = |a: &str| plan(&live(), &Want { address: Some(("*3".into(), a.into())), ..Default::default() }, &p).unwrap_err();
        assert!(e("192.168.99.69/24").contains("usable"), "the app's own address");
        assert!(e("192.168.1.0/24").contains("usable"), "network address");
        assert!(e("10.99.0.5/24").contains("overlaps"));
    }

    fn mode_live() -> (Value, Value) {
        let mut l = live();
        l["ifaces"] = json!([
            {"name": "ether1", "type": "ether", "slave": "false"}, {"name": "ether2", "type": "ether", "slave": "true"},
            {"name": "ether3", "type": "ether", "slave": "true"}, {"name": "lan", "type": "bridge"}, {"name": "test-lan", "type": "bridge"}]);
        l["bridges"] = json!([{"name": "lan"}, {"name": "test-lan"}]);
        l["ports"] = json!([{".id": "*A", "interface": "ether2", "bridge": "lan", "pvid": "20"}]);
        l["vlan_ifaces"] = json!([]);
        let fw = json!({"tables": {"nat": [], "filter": [], "raw": [], "mangle": []}, "addresses": l["addresses"], "tracking": {"enabled": "auto", "active": "true"}});
        (l, fw)
    }

    #[test]
    fn router_and_bridge_mode() {
        let (l, fw) = mode_live();
        let p = Path { switch: "192.168.1.86".parse().unwrap(), app: Some("192.168.1.69".parse().unwrap()) };
        let m = Mode { iface: "ether2".into(), bridge: "lan".into(), router: true };
        let (c, u) = plan_mode(&l, &fw, &m, &p, 8729).unwrap();
        assert_eq!(show_all(&c), vec!["/interface/bridge/port/remove .id=*A", "/ip/firewall/nat/add chain=srcnat action=masquerade out-interface=ether2 comment=\"cairn-net: router mode\""]);
        assert_eq!(&u[..2], ["/ip firewall nat remove [find where comment=\"cairn-net: router mode\" and out-interface=\"ether2\"]",
            "/interface bridge port add bridge=\"lan\" interface=\"ether2\" pvid=20"]);
        assert!(!matches_mode(&l, &fw, &m));
        // back to bridge mode: the masquerade goes, the port comes back
        let mut l2 = l.clone();
        l2["ports"] = json!([]);
        l2["ifaces"][1]["slave"] = json!("false"); // out of the bridge: no longer a slave
        let mut fw2 = fw.clone();
        fw2["tables"]["nat"] = json!([{".id": "*7", "chain": "srcnat", "action": "masquerade", "out-interface": "ether2", "comment": "x"}]);
        assert!(matches_mode(&l2, &fw2, &m));
        let b = Mode { router: false, ..m.clone() };
        let (c, u) = plan_mode(&l2, &fw2, &b, &p, 8729).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/firewall/nat/remove .id=*7", "/interface/bridge/port/add bridge=lan interface=ether2"]);
        assert!(u[1].contains("nat add") && u[1].contains("out-interface=\"ether2\""), "{u:?}");
        // conntrack not running: refused, it would start with the first NAT rule
        let mut off = fw.clone();
        off["tracking"]["active"] = json!("false");
        assert!(plan_mode(&l, &off, &m, &p, 8729).unwrap_err().contains("connection tracking is off"));
        // the management path, bad names, ports elsewhere, addressed ports
        let e = |m: Mode| plan_mode(&l, &fw, &m, &p, 8729).unwrap_err();
        assert!(e(Mode { iface: "ether1".into(), ..m.clone() }).contains("Cairn Net's path"));
        assert!(e(Mode { iface: "ether3".into(), ..m.clone() }).contains("bond member"));
        assert!(e(Mode { iface: "ether2".into(), bridge: "test-lan".into(), router: false }).contains("port of lan"));
        assert!(e(Mode { iface: "lan".into(), ..m.clone() }).contains("bridge"));
        assert!(e(Mode { iface: "test-lan".into(), bridge: "lan".into(), router: false }).contains("bridge"));
        let mut l3 = l.clone();
        l3["ifaces"].as_array_mut().unwrap().push(json!({"name": "ether4", "type": "ether", "slave": "false"}));
        l3["addresses"].as_array_mut().unwrap().push(json!({".id": "*9", "address": "10.5.0.1/24", "interface": "ether4"}));
        assert!(plan_mode(&l3, &fw, &Mode { iface: "ether4".into(), bridge: "lan".into(), router: false }, &p, 8729).unwrap_err().contains("IP address"));
        // a VLAN of the bridge carries the path: that bridge is not touched
        let mut l4 = l.clone();
        l4["vlan_ifaces"] = json!([{"name": "mgmt99", "interface": "lan", "vlan-id": "99"}]);
        l4["addresses"].as_array_mut().unwrap()[1]["interface"] = json!("mgmt99");
        let mut fw4 = fw.clone();
        fw4["addresses"] = l4["addresses"].clone();
        assert!(plan_mode(&l4, &fw4, &m, &p, 8729).unwrap_err().contains("lan carries"));
    }

    #[test]
    fn management_services() {
        let mut l = live();
        l["services"] = json!([
            {".id": "*4", "name": "ssh", "port": "22", "available-from": "", "disabled": "false"},
            {".id": "*8", "name": "winbox", "port": "8291", "available-from": "", "disabled": "false"},
            {".id": "*9", "name": "api-ssl", "port": "8729", "available-from": "192.168.99.69/32", "disabled": "false"}]);
        l["mac_winbox"] = json!({"allowed-interface-list": "all"});
        l["iface_lists"] = json!([{"name": "LAN"}]);
        let p = path();
        let w = Want { services: vec![("*8".into(), false, "192.168.1.10, 192.168.99.0/24".into())], mac_winbox: Some("LAN".into()), ..Default::default() };
        let (c, u) = plan(&l, &w, &p).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/service/set .id=*8 disabled=no available-from=192.168.1.10,192.168.99.0/24",
            "/tool/mac-server/mac-winbox/set allowed-interface-list=LAN"]);
        assert_eq!(&u[..2], ["/tool mac-server mac-winbox set allowed-interface-list=\"all\"", "/ip service set [find where .id=*8] disabled=no available-from=\"\""]);
        assert!(!matches(&l, &w));
        // older RouterOS: "address"
        let mut o = l.clone();
        o["services"][1] = json!({".id": "*8", "name": "winbox", "address": "", "disabled": "false"});
        assert!(show_all(&plan(&o, &w, &p).unwrap().0)[0].contains(" address=192.168.1.10"));
        let e = |w: Want| plan(&l, &w, &p).unwrap_err();
        assert!(e(Want { services: vec![("*9".into(), true, "".into())], ..Default::default() }).contains("stays on"));
        assert!(e(Want { services: vec![("*9".into(), false, "10.0.0.0/8".into())], ..Default::default() }).contains("leaves out Cairn Net"));
        assert!(e(Want { services: vec![("*8".into(), false, "nope".into())], ..Default::default() }).contains("invalid address"));
        assert!(e(Want { mac_winbox: Some("WAN".into()), ..Default::default() }).contains("no such interface list"));
        // no way in left besides Cairn Net
        let shut = Want { services: vec![("*4".into(), true, "".into()), ("*8".into(), true, "".into())], mac_winbox: Some("none".into()), ..Default::default() };
        assert!(e(shut).contains("way in"));
        // WinBox off, SSH kept: fine
        assert!(plan(&l, &Want { services: vec![("*8".into(), true, "".into())], mac_winbox: Some("none".into()), ..Default::default() }, &p).is_ok());
    }

    #[test]
    fn uptime() {
        assert_eq!(uptime_s("14h15m35s"), 14 * 3600 + 15 * 60 + 35);
        assert_eq!(uptime_s("1w2d"), 9 * 86400);
        assert_eq!(uptime_s("42s"), 42);
    }
}
