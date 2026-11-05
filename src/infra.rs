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
        "ports": print(api, "/interface/bridge/port", "interface,bridge,pvid,disabled", &[])?,
        "nat": print(api, "/ip/firewall/nat", "chain,action,src-address,dst-address,in-interface,out-interface,protocol,dst-port,to-addresses,to-ports,disabled,dynamic,comment", &[])?,
        "rollback": print(api, "/system/scheduler", "name,next-run", &[&format!("?name={ROLLBACK}")])?.pop(),
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
fn q(v: &str) -> String {
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
fn cidr(v: &str) -> Option<(Ipv4Addr, u32, u32)> {
    let (a, l) = v.split_once('/')?;
    let ip: Ipv4Addr = a.parse().ok()?;
    let len: u32 = l.parse().ok().filter(|l| (1..=32).contains(l))?;
    let mask = u32::MAX.checked_shl(32 - len).unwrap_or(0);
    Some((ip, u32::from(ip) & mask, len))
}

fn contains(net: &str, ip: Ipv4Addr) -> bool {
    cidr(net).is_some_and(|(_, n, l)| u32::from(ip) & u32::MAX.checked_shl(32 - l).unwrap_or(0) == n)
}

fn overlap(a: &str, b: &str) -> bool {
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
}

impl Want {
    pub fn from_json(v: &Value) -> Want {
        let st = |k: &str| v[k].as_str().map(|x| x.trim().to_string());
        Want {
            identity: st("identity"),
            dns: v["dns"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()),
            address: v["address"]["id"].as_str().zip(v["address"]["address"].as_str()).map(|(i, a)| (i.to_string(), a.trim().to_string())),
            gateway: st("gateway"),
        }
    }
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
        if mgmt_iface(live, path) == Some(s(a, "interface")) {
            return Err(format!("{}: Cairn Net reaches the switch through this interface, it is not edited from here", s(a, "interface")));
        }
        let (ip, _, _) = cidr(new).ok_or_else(|| format!("{new}: invalid address (a.b.c.d/len)"))?;
        for end in std::iter::once(path.switch).chain(path.app) {
            if contains(new, end) || ip == end {
                return Err(format!("{new} would take the network of the management path ({end})"));
            }
        }
        for n in &nets {
            if n != old && overlap(n, new) {
                return Err(format!("{new} overlaps {n}, already on the switch"));
            }
        }
        if new != old {
            cmds.push(("/ip/address/set".into(), kv(&[(".id", id), ("address", new)])));
            undo.push(format!("/ip address set [find where .id={id}] address={}", q(old)));
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

    if !undo.is_empty() {
        // undo in reverse order, then the scheduler goes
        undo.reverse();
        undo.push(format!(":log warning {}", q("cairn-net: change not confirmed, rolled back")));
        undo.push(format!("/system scheduler remove [find name={}]", q(ROLLBACK)));
    }
    Ok((cmds, undo))
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
    id_ok && dns_ok && addr_ok && gw_ok
}

pub fn show_all(cmds: &[Cmd]) -> Vec<String> {
    cmds.iter().map(show).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Value {
        json!({
            "identity": "R1",
            "dns": {"servers": "192.168.1.2"},
            "addresses": [
                {".id": "*2", "address": "10.99.0.1/24", "interface": "test-lan", "disabled": "false", "dynamic": "false"},
                {".id": "*3", "address": "192.168.1.86/24", "interface": "ether1", "disabled": "false", "dynamic": "false"},
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
        let w = Want { identity: Some("R1".into()), dns: Some(vec!["192.168.1.2".into()]), address: Some(("*2".into(), "10.99.0.1/24".into())), gateway: Some("192.168.1.1".into()) };
        let (c, u) = plan(&live(), &w, &path()).unwrap();
        assert!(c.is_empty() && u.is_empty());
        assert!(matches(&live(), &w));
    }

    #[test]
    fn changes_and_their_rollback() {
        let w = Want { identity: Some("R\"2".into()), dns: Some(vec!["1.1.1.1".into(), "9.9.9.9".into()]), address: Some(("*2".into(), "10.98.0.1/16".into())), gateway: Some("10.98.0.254".into()) };
        let (c, u) = plan(&live(), &w, &path()).unwrap();
        assert_eq!(show_all(&c), vec![
            "/system/identity/set name=R\"2",
            "/ip/dns/set servers=1.1.1.1,9.9.9.9",
            "/ip/address/set .id=*2 address=10.98.0.1/16",
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
        assert!(e(Want { address: Some(("*3".into(), "192.168.1.87/24".into())), ..Default::default() }).contains("reaches the switch"));
        // another interface taking the switch's or the app's network
        assert!(e(Want { address: Some(("*2".into(), "192.168.1.200/24".into())), ..Default::default() }).contains("management path"));
        assert!(e(Want { address: Some(("*2".into(), "192.168.0.1/16".into())), ..Default::default() }).contains("management path"));
        assert!(e(Want { address: Some(("*2".into(), "192.168.99.1/24".into())), ..Default::default() }).contains("management path"));
        // gateway out of every network, bad input
        assert!(e(Want { gateway: Some("172.16.0.1".into()), ..Default::default() }).contains("networks"));
        assert!(e(Want { address: Some(("*2".into(), "10.99.0.1".into())), ..Default::default() }).contains("invalid"));
        assert!(e(Want { dns: Some(vec!["dns.example".into()]), ..Default::default() }).contains("IPv4"));
    }

    #[test]
    fn a_missing_default_route_is_added_and_undone() {
        let mut l = live();
        l["routes"] = json!([]);
        let (c, u) = plan(&l, &Want { gateway: Some("10.99.0.254".into()), ..Default::default() }, &path()).unwrap();
        assert_eq!(show_all(&c), vec!["/ip/route/add dst-address=0.0.0.0/0 gateway=10.99.0.254"]);
        assert_eq!(u[0], "/ip route remove [find where dst-address=0.0.0.0/0 and gateway=\"10.99.0.254\" and !dynamic]");
    }
}
