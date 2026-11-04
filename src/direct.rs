//! Direct mode: the switch is the source of truth.
//! What it runs is turned into the same text as dhcp.yml (servers, static
//! leases), so the page's editor, checks and diff work unchanged; applying
//! turns the difference between that text and the edited one into RouterOS
//! API commands, run with the write account, then the switch is read again.

use crate::dhcp::{self, in_net, Entry};
use crate::servers::{self, Server};
use crate::{print, Api};
use serde_json::Value;
use std::net::Ipv4Addr;

/// What the switch runs, as printed by the API (with .id).
pub struct Live {
    servers: Vec<Value>,
    networks: Vec<Value>,
    pools: Vec<Value>,
    options: Vec<Value>,
    leases: Vec<Value>,
    addresses: Vec<Value>,
}

pub fn read(api: &mut Api) -> Result<Live, String> {
    Ok(Live {
        servers: print(api, "/ip/dhcp-server", ".id,name,disabled,address-pool,interface,lease-time,dynamic", &[])?,
        networks: print(api, "/ip/dhcp-server/network", ".id,address,gateway,dns-server,domain,ntp-server,dhcp-option", &[])?,
        pools: print(api, "/ip/pool", ".id,name,ranges", &[])?,
        options: print(api, "/ip/dhcp-server/option", ".id,name,code,value", &[])?,
        leases: print(api, "/ip/dhcp-server/lease", ".id,address,mac-address,host-name,comment,server,dynamic", &[])?,
        addresses: print(api, "/ip/address", "address,interface,disabled", &[])?,
    })
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn list(v: &str) -> Vec<String> {
    v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// Option 121 (RFC 3442): prefix length, significant octets, gateway.
pub fn routes_hex(routes: &[String]) -> String {
    let mut hex = String::from("0x");
    for r in routes {
        let Some((dst, gw)) = r.split_once(',') else { continue };
        let (net, len) = dst.split_once('/').unwrap_or((dst, "32"));
        let len: usize = len.parse().unwrap_or(32);
        let o = net.parse::<Ipv4Addr>().map(|a| a.octets()).unwrap_or_default();
        let g = gw.parse::<Ipv4Addr>().map(|a| a.octets()).unwrap_or_default();
        hex += &format!("{len:02x}");
        for b in o[..len.div_ceil(8)].iter().chain(g.iter()) {
            hex += &format!("{b:02x}");
        }
    }
    hex
}

pub fn routes_from_hex(v: &str) -> Vec<String> {
    let h = v.trim_matches('\'').trim_start_matches("0x").trim_start_matches("0X");
    let b: Vec<u8> = (0..h.len() / 2).filter_map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        let len = b[i] as usize;
        let n = len.div_ceil(8);
        if len > 32 || i + 1 + n + 4 > b.len() {
            break;
        }
        let mut net = [0u8; 4];
        net[..n].copy_from_slice(&b[i + 1..i + 1 + n]);
        let g = &b[i + 1 + n..i + 5 + n];
        out.push(format!("{}/{len},{}", Ipv4Addr::from(net), Ipv4Addr::new(g[0], g[1], g[2], g[3])));
        i += 5 + n;
    }
    out
}

fn yes(v: &Value, k: &str) -> bool {
    s(v, k) == "true" || s(v, k) == "yes"
}

/// The network block a server hands out: the one holding the address of its
/// interface (as the page shows it).
fn network_of<'a>(l: &'a Live, srv: &Value) -> Option<&'a Value> {
    let ip: Ipv4Addr = l.addresses.iter().find(|a| s(a, "interface") == s(srv, "interface") && !yes(a, "disabled"))?
        .get("address")?.as_str()?.split('/').next()?.parse().ok()?;
    l.networks.iter().find(|n| in_net(ip, s(n, "address")))
}

fn routes_option<'a>(l: &'a Live, net: Option<&Value>) -> Option<&'a Value> {
    let names = list(s(net?, "dhcp-option"));
    l.options.iter().find(|o| s(o, "code") == "121" && names.iter().any(|n| n == s(o, "name")))
}

/// A static lease's name: its comment (without the "ansible: " prefix of the
/// Ansible roles that write the leases), else the client's host name, else built from the MAC.
fn lease_name(l: &Value) -> String {
    let c = s(l, "comment").trim_start_matches("ansible:").trim();
    if dhcp::is_name(c) {
        return c.to_string();
    }
    dhcp::lease_name(s(l, "host-name")).unwrap_or_else(|| format!("lease-{}", s(l, "mac-address").replace(':', "").to_lowercase()))
}

pub fn servers_of(l: &Live) -> Vec<Server> {
    l.servers.iter().filter(|v| !yes(v, "dynamic")).map(|v| {
        let net = network_of(l, v);
        let pool = l.pools.iter().find(|p| s(p, "name") == s(v, "address-pool"));
        let n = |k: &str| net.map(|x| s(x, k).to_string()).unwrap_or_default();
        Server {
            name: s(v, "name").into(),
            enabled: !yes(v, "disabled"),
            interface: s(v, "interface").into(),
            network: n("address"),
            range: pool.map(|p| s(p, "ranges").to_string()).unwrap_or_default(),
            gateway: n("gateway"),
            dns: list(&n("dns-server")),
            domain: n("domain"),
            ntp: list(&n("ntp-server")),
            lease_time: s(v, "lease-time").into(),
            routes: routes_option(l, net).map(|o| routes_from_hex(s(o, "value"))).unwrap_or_default().into_iter().map(|r| (r, vec![])).collect(),
            ..Default::default()
        }
    }).collect()
}

fn entries_of(l: &Live) -> Vec<(String, String, String)> {
    l.leases.iter().filter(|x| !yes(x, "dynamic")).map(|x| (s(x, "mac-address").to_uppercase(), s(x, "address").to_string(), lease_name(x))).collect()
}

/// The dhcp.yml-shaped text of what the switch runs.
pub fn to_text(l: &Live) -> String {
    text(&entries_of(l), &servers_of(l))
}

fn text(entries: &[(String, String, String)], srvs: &[Server]) -> String {
    let mut t = String::from("dhcp_reservations:\n");
    for (m, i, n) in entries {
        t += &format!("  - \"{m},{i},{n}\"\n");
    }
    t += "\ndhcp_servers:\n";
    for sv in srvs {
        for line in servers::render(sv) {
            t += &line;
            t += "\n";
        }
    }
    t
}

/// The edited text without its comments, in the order `to_text` writes it:
/// what the switch must give back after an apply.
pub fn canonical(new: &str) -> String {
    let lines: Vec<String> = new.lines().map(String::from).collect();
    let srvs: Vec<Server> = servers::parse(&lines).into_iter().map(|mut x| {
        x.comment.clear();
        x.routes.iter_mut().for_each(|r| r.1.clear());
        x
    }).collect();
    let e: Vec<_> = dhcp::parse(new).into_iter().map(|e: Entry| (e.mac, e.ip, e.name)).collect();
    text(&e, &srvs)
}

/// One API command: path and its "=key=value" words.
pub type Cmd = (String, Vec<(String, String)>);

pub fn show(c: &Cmd) -> String {
    let words: Vec<String> = c.1.iter().map(|(k, v)| format!("{k}={}", if v.is_empty() || v.contains(' ') { format!("\"{v}\"") } else { v.clone() })).collect();
    format!("{} {}", c.0, words.join(" "))
}

/// `set` of the fields that differ (none: no command), or `add`.
fn upsert(out: &mut Vec<Cmd>, path: &str, cur: Option<&Value>, want: &[(&str, String)]) {
    let kv = |f: &[(&str, String)]| f.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>();
    match cur {
        Some(c) => {
            let diff: Vec<(&str, String)> = want.iter().filter(|(k, v)| s(c, k) != v).cloned().collect();
            if !diff.is_empty() {
                let mut w = vec![(".id".to_string(), s(c, ".id").to_string())];
                w.extend(kv(&diff));
                out.push((format!("{path}/set"), w));
            }
        }
        None => out.push((format!("{path}/add"), kv(want))),
    }
}

/// The commands that turn `l` into `new` (dhcp.yml-shaped text). Servers that
/// vanished from the text are left alone (the editor has no server delete);
/// static leases that vanished are removed.
pub fn plan(l: &Live, new: &str) -> Result<Vec<Cmd>, String> {
    let lines: Vec<String> = new.lines().map(String::from).collect();
    let want = servers::parse(&lines);
    let mut out = Vec::new();
    for w in &want {
        let cur = l.servers.iter().find(|v| s(v, "name") == w.name && !yes(v, "dynamic"));
        let pool_name = cur.map(|c| s(c, "address-pool")).filter(|p| !p.is_empty() && *p != "static-only").unwrap_or(&w.name).to_string();
        let pool = l.pools.iter().find(|p| s(p, "name") == pool_name);
        upsert(&mut out, "/ip/pool", pool, &[("name", pool_name.clone()), ("ranges", w.range.clone())]);

        let net = cur.and_then(|c| network_of(l, c)).or_else(|| l.networks.iter().find(|n| s(n, "address") == w.network));
        let old_opt = routes_option(l, net);
        let opt_name = old_opt.map(|o| s(o, "name").to_string()).unwrap_or_else(|| format!("{}-routes", w.name));
        let routes: Vec<String> = w.routes.iter().map(|r| r.0.clone()).collect();
        if !routes.is_empty() {
            let byname = l.options.iter().find(|o| s(o, "name") == opt_name);
            upsert(&mut out, "/ip/dhcp-server/option", byname, &[("name", opt_name.clone()), ("code", "121".into()), ("value", routes_hex(&routes))]);
        }
        // other options of the network stay; only the routes one comes or goes
        let mut opts: Vec<String> = net.map(|n| list(s(n, "dhcp-option"))).unwrap_or_default().into_iter().filter(|o| *o != opt_name).collect();
        if !routes.is_empty() {
            opts.push(opt_name.clone());
        }
        upsert(&mut out, "/ip/dhcp-server/network", net, &[("address", w.network.clone()), ("gateway", w.gateway.clone()), ("dns-server", w.dns.join(",")),
            ("domain", w.domain.clone()), ("ntp-server", w.ntp.join(",")), ("dhcp-option", opts.join(","))]);
        // routes gone: their option goes too, unless another network uses it
        if let (true, Some(o)) = (routes.is_empty(), old_opt) {
            let elsewhere = l.networks.iter().filter(|n| Some(*n) != net).any(|n| list(s(n, "dhcp-option")).iter().any(|x| x == s(o, "name")));
            if !elsewhere {
                out.push(("/ip/dhcp-server/option/remove".into(), vec![(".id".into(), s(o, ".id").into())]));
            }
        }

        upsert(&mut out, "/ip/dhcp-server", cur, &[("name", w.name.clone()), ("interface", w.interface.clone()), ("address-pool", pool_name),
            ("lease-time", w.lease_time.clone()), ("disabled", if w.enabled { "false" } else { "true" }.into())]);
    }

    let entries = dhcp::parse(new);
    let statics: Vec<&Value> = l.leases.iter().filter(|x| !yes(x, "dynamic")).collect();
    for x in &statics {
        if !entries.iter().any(|e| e.mac == s(x, "mac-address").to_uppercase()) {
            out.push(("/ip/dhcp-server/lease/remove".into(), vec![(".id".into(), s(x, ".id").into())]));
        }
    }
    for e in &entries {
        let ip: Ipv4Addr = e.ip.parse().map_err(|_| format!("{}: invalid IP address {}", e.name, e.ip))?;
        let srv = want.iter().find(|w| in_net(ip, &w.network)).ok_or_else(|| format!("{}: {} is outside every server's network", e.name, e.ip))?;
        let cur = statics.iter().find(|x| s(x, "mac-address").to_uppercase() == e.mac).copied();
        if cur.is_none() {
            // a dynamic lease of the same client would refuse the static one
            for d in l.leases.iter().filter(|x| yes(x, "dynamic") && s(x, "mac-address").to_uppercase() == e.mac) {
                out.push(("/ip/dhcp-server/lease/remove".into(), vec![(".id".into(), s(d, ".id").into())]));
            }
        }
        let name_now = cur.map(lease_name);
        let mut fields = vec![("address", e.ip.clone()), ("server", srv.name.clone())];
        if name_now.as_deref() != Some(e.name.as_str()) {
            fields.push(("comment", e.name.clone()));
        }
        if cur.is_none() {
            fields.insert(0, ("mac-address", e.mac.clone()));
        }
        upsert(&mut out, "/ip/dhcp-server/lease", cur, &fields);
    }
    Ok(out)
}

/// Runs the commands in order; stops at the first error (what ran stays:
/// the answer says which). ponytail: no rollback, the re-read shows the state.
pub fn run(api: &mut Api, cmds: &[Cmd]) -> (usize, Option<String>) {
    for (i, (path, kv)) in cmds.iter().enumerate() {
        let words: Vec<String> = std::iter::once(path.clone()).chain(kv.iter().map(|(k, v)| format!("={k}={v}"))).collect();
        let w: Vec<&str> = words.iter().map(String::as_str).collect();
        if let Err(e) = api.call(&w) {
            return (i, Some(format!("{}: {e}", show(&cmds[i]))));
        }
    }
    (cmds.len(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn option_121_round_trip() {
        let r = vec!["192.168.99.0/24,192.168.1.11".to_string(), "192.168.20.0/24,192.168.1.11".into(), "0.0.0.0/0,192.168.1.1".into()];
        // a real value written by an Ansible role
        assert_eq!(routes_hex(&r), "0x18c0a863c0a8010b18c0a814c0a8010b00c0a80101");
        assert_eq!(routes_from_hex("0x18c0a863c0a8010b18c0a814c0a8010b00c0a80101"), r);
    }

    fn live() -> Live {
        Live {
            servers: vec![json!({".id": "*1", "name": "test", "disabled": "false", "address-pool": "test", "interface": "test-lan", "lease-time": "1d", "dynamic": "false"})],
            networks: vec![json!({".id": "*2", "address": "10.99.0.0/24", "gateway": "10.99.0.1", "dns-server": "", "domain": "", "ntp-server": "", "dhcp-option": ""})],
            pools: vec![json!({".id": "*3", "name": "test", "ranges": "10.99.0.100-10.99.0.199"})],
            options: vec![],
            leases: vec![json!({".id": "*4", "address": "10.99.0.5", "mac-address": "AA:BB:CC:DD:EE:01", "comment": "one", "server": "test", "dynamic": "false"}),
                json!({".id": "*5", "address": "10.99.0.150", "mac-address": "aa:bb:cc:dd:ee:02", "host-name": "Two", "server": "test", "dynamic": "true"})],
            addresses: vec![json!({"address": "10.99.0.1/24", "interface": "test-lan", "disabled": "false"})],
        }
    }

    #[test]
    fn unchanged_text_means_no_command() {
        let l = live();
        let t = to_text(&l);
        assert!(t.contains("  - \"AA:BB:CC:DD:EE:01,10.99.0.5,one\"") && t.contains("    range: 10.99.0.100-10.99.0.199"), "{t}");
        assert!(plan(&l, &t).unwrap().is_empty());
        assert_eq!(canonical(&t), t);
    }

    #[test]
    fn edits_become_commands() {
        let l = live();
        let t = to_text(&l);
        let new = dhcp::apply(&t, &[json!({"op": "add", "mac": "aa:bb:cc:dd:ee:02", "ip": "10.99.0.6", "name": "two"}),
            json!({"op": "del", "mac": "AA:BB:CC:DD:EE:01"})]).unwrap();
        let mut srv = servers::to_json(&servers::parse(&new.lines().map(String::from).collect::<Vec<_>>())[0]);
        srv["routes"] = json!(["0.0.0.0/0,10.99.0.1"]);
        srv["enabled"] = json!(false);
        let new = dhcp::apply(&new, &[json!({"op": "srv_set", "name": "test", "server": srv})]).unwrap();
        let p: Vec<String> = plan(&l, &new).unwrap().iter().map(show).collect();
        assert_eq!(p, vec![
            "/ip/dhcp-server/option/add name=test-routes code=121 value=0x000a630001",
            "/ip/dhcp-server/network/set .id=*2 dhcp-option=test-routes",
            "/ip/dhcp-server/set .id=*1 disabled=true",
            "/ip/dhcp-server/lease/remove .id=*4",
            "/ip/dhcp-server/lease/remove .id=*5",
            "/ip/dhcp-server/lease/add mac-address=AA:BB:CC:DD:EE:02 address=10.99.0.6 server=test comment=two",
        ], "{p:#?}");
    }
}
