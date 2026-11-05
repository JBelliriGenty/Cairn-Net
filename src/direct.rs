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
    alerts: Vec<Value>,
}

pub fn read(api: &mut Api) -> Result<Live, String> {
    Ok(Live {
        servers: print(api, "/ip/dhcp-server", ".id,name,disabled,address-pool,interface,lease-time,dynamic", &[])?,
        networks: print(api, "/ip/dhcp-server/network", ".id,address,gateway,dns-server,domain,ntp-server,dhcp-option", &[])?,
        pools: print(api, "/ip/pool", ".id,name,ranges", &[])?,
        options: print(api, "/ip/dhcp-server/option", ".id,name,code,value", &[])?,
        leases: print(api, "/ip/dhcp-server/lease", ".id,address,mac-address,host-name,comment,server,dynamic", &[])?,
        addresses: print(api, "/ip/address", "address,interface,disabled", &[])?,
        alerts: print(api, "/ip/dhcp-server/alert", ".id,interface,valid-server,disabled", &[])?,
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

/// The other options of a network, in its order.
fn extra_options<'a>(l: &'a Live, net: Option<&Value>) -> Vec<&'a Value> {
    let names = net.map(|n| list(s(n, "dhcp-option"))).unwrap_or_default();
    names.iter().filter_map(|n| l.options.iter().find(|o| s(o, "name") == n)).filter(|o| s(o, "code") != "121").collect()
}

/// An option also used by a network other than `net`.
fn shared(l: &Live, net: Option<&Value>, name: &str) -> bool {
    l.networks.iter().filter(|n| Some(*n) != net).any(|n| list(s(n, "dhcp-option")).iter().any(|x| x == name))
}

fn remove(out: &mut Vec<Cmd>, path: &str, v: &Value) {
    out.push((format!("{path}/remove"), vec![(".id".into(), s(v, ".id").into())]));
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
        let alert = l.alerts.iter().find(|a| s(a, "interface") == s(v, "interface") && !yes(a, "disabled"));
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
            alert: alert.is_some(),
            alert_valid: alert.map(|a| list(s(a, "valid-server")).into_iter().map(|m| m.to_uppercase()).collect()).unwrap_or_default(),
            routes: routes_option(l, net).map(|o| routes_from_hex(s(o, "value"))).unwrap_or_default().into_iter().map(|r| (r, vec![])).collect(),
            options: extra_options(l, net).iter().map(|o| (format!("{},{}", s(o, "code"), s(o, "value")), vec![])).collect(),
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

/// The commands that turn `l` into `new` (dhcp.yml-shaped text). A server
/// gone from the text is removed with its network, pool and options (unless
/// something else uses them); one with a new name on the interface of a
/// vanished one is that server renamed. Static leases that vanished are removed.
pub fn plan(l: &Live, new: &str) -> Result<Vec<Cmd>, String> {
    let lines: Vec<String> = new.lines().map(String::from).collect();
    let want = servers::parse(&lines);
    let mut out = Vec::new();
    let live: Vec<&Value> = l.servers.iter().filter(|v| !yes(v, "dynamic")).collect();
    let curs: Vec<Option<&Value>> = want.iter().map(|w| {
        live.iter().find(|v| s(v, "name") == w.name)
            .or_else(|| live.iter().find(|v| s(v, "interface") == w.interface && !want.iter().any(|x| x.name == s(v, "name"))))
            .copied()
    }).collect();
    for (w, &cur) in want.iter().zip(&curs) {
        let cur_name = cur.map_or("", |c| s(c, "name"));
        let old_pool = cur.map(|c| s(c, "address-pool")).filter(|p| !p.is_empty() && *p != "static-only");
        let pool = l.pools.iter().find(|p| s(p, "name") == old_pool.unwrap_or(&w.name));
        // a pool named after its server follows a rename
        let pool_name = old_pool.filter(|p| *p != cur_name).unwrap_or(&w.name).to_string();
        upsert(&mut out, "/ip/pool", pool, &[("name", pool_name.clone()), ("ranges", w.range.clone())]);

        let net = cur.and_then(|c| network_of(l, c)).or_else(|| l.networks.iter().find(|n| s(n, "address") == w.network));
        let old_opt = routes_option(l, net);
        let opt_name = old_opt.map(|o| s(o, "name").to_string()).unwrap_or_else(|| format!("{}-routes", w.name));
        let routes: Vec<String> = w.routes.iter().map(|r| r.0.clone()).collect();
        let mut keep: Vec<String> = Vec::new(); // the network's options after the apply
        let mut gone: Vec<&Value> = Vec::new(); // options to remove once the network no longer lists them
        if !routes.is_empty() {
            let byname = l.options.iter().find(|o| s(o, "name") == opt_name);
            upsert(&mut out, "/ip/dhcp-server/option", byname, &[("name", opt_name.clone()), ("code", "121".into()), ("value", routes_hex(&routes))]);
            keep.push(opt_name.clone());
        } else if let Some(o) = old_opt.filter(|o| !shared(l, net, s(o, "name"))) {
            gone.push(o);
        }
        let extra = extra_options(l, net);
        for (o, _) in &w.options {
            let (code, val) = o.split_once(',').map(|(c, v)| (c.trim(), v.trim())).unwrap_or((o, ""));
            let fields = |n: &str| [("name", n.to_string()), ("code", code.to_string()), ("value", val.to_string())];
            match extra.iter().find(|x| s(x, "code") == code) {
                // the network's own option of that code: changed in place, unless another network shares it
                Some(x) if s(x, "value") == val || !shared(l, net, s(x, "name")) => {
                    upsert(&mut out, "/ip/dhcp-server/option", Some(x), &fields(s(x, "name")));
                    keep.push(s(x, "name").into());
                }
                _ => {
                    let n = format!("{}-opt{code}", w.name);
                    upsert(&mut out, "/ip/dhcp-server/option", l.options.iter().find(|x| s(x, "name") == n), &fields(&n));
                    keep.push(n);
                }
            }
        }
        gone.extend(extra.iter().filter(|x| !keep.iter().any(|k| k == s(x, "name")) && !shared(l, net, s(x, "name"))));
        // same order as now, new ones last: an unchanged list is no command
        let mut opts: Vec<String> = net.map(|n| list(s(n, "dhcp-option"))).unwrap_or_default().into_iter().filter(|o| keep.contains(o)).collect();
        opts.extend(keep.iter().filter(|k| !opts.contains(k)).cloned().collect::<Vec<_>>());
        upsert(&mut out, "/ip/dhcp-server/network", net, &[("address", w.network.clone()), ("gateway", w.gateway.clone()), ("dns-server", w.dns.join(",")),
            ("domain", w.domain.clone()), ("ntp-server", w.ntp.join(",")), ("dhcp-option", opts.join(","))]);
        for o in gone {
            remove(&mut out, "/ip/dhcp-server/option", o);
        }

        upsert(&mut out, "/ip/dhcp-server", cur, &[("name", w.name.clone()), ("interface", w.interface.clone()), ("address-pool", pool_name),
            ("lease-time", w.lease_time.clone()), ("disabled", if w.enabled { "false" } else { "true" }.into())]);

        // the alert follows the server's interface
        let iface = cur.map_or(w.interface.as_str(), |c| s(c, "interface"));
        let alert = l.alerts.iter().find(|a| s(a, "interface") == iface);
        if w.alert {
            upsert(&mut out, "/ip/dhcp-server/alert", alert, &[("interface", w.interface.clone()), ("valid-server", w.alert_valid.join(",")), ("disabled", "false".into())]);
        } else if let Some(a) = alert {
            remove(&mut out, "/ip/dhcp-server/alert", a);
        }
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

    // servers gone from the text, after their static leases
    for v in live.iter().filter(|v| !curs.contains(&Some(**v))) {
        let name = s(v, "name");
        for d in l.leases.iter().filter(|x| yes(x, "dynamic") && s(x, "server") == name) {
            remove(&mut out, "/ip/dhcp-server/lease", d);
        }
        remove(&mut out, "/ip/dhcp-server", v);
        for a in l.alerts.iter().filter(|a| s(a, "interface") == s(v, "interface")) {
            remove(&mut out, "/ip/dhcp-server/alert", a);
        }
        let net = network_of(l, v).filter(|n| !want.iter().any(|w| w.network == s(n, "address")));
        if let Some(n) = net {
            remove(&mut out, "/ip/dhcp-server/network", n);
            for o in list(s(n, "dhcp-option")).iter().filter(|o| !shared(l, net, o)).filter_map(|o| l.options.iter().find(|x| s(x, "name") == o)) {
                remove(&mut out, "/ip/dhcp-server/option", o);
            }
        }
        let pool = s(v, "address-pool");
        let used = live.iter().any(|x| x != v && s(x, "address-pool") == pool) || want.iter().any(|w| w.name == pool);
        if let Some(p) = l.pools.iter().find(|p| s(p, "name") == pool).filter(|_| !used) {
            remove(&mut out, "/ip/pool", p);
        }
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
            alerts: vec![],
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

    fn srv(t: &str) -> Value {
        servers::to_json(&servers::parse(&t.lines().map(String::from).collect::<Vec<_>>())[0])
    }

    #[test]
    fn options_become_commands() {
        let mut l = live();
        l.options = vec![json!({".id": "*6", "name": "tftp", "code": "66", "value": "s'old'"}), json!({".id": "*7", "name": "wpad", "code": "252", "value": "'x'"})];
        l.networks[0]["dhcp-option"] = json!("tftp,wpad");
        let t = to_text(&l);
        assert!(t.contains("    options:\n      - \"66,s'old'\"\n      - \"252,'x'\""), "{t}");
        assert!(plan(&l, &t).unwrap().is_empty());
        let mut v = srv(&t);
        v["options"] = json!(["66,s'new'", "67,s'boot.efi'"]);
        let new = dhcp::apply(&t, &[json!({"op": "srv_set", "name": "test", "server": v})]).unwrap();
        let p: Vec<String> = plan(&l, &new).unwrap().iter().map(show).collect();
        assert_eq!(p, vec![
            "/ip/dhcp-server/option/set .id=*6 value=s'new'",
            "/ip/dhcp-server/option/add name=test-opt67 code=67 value=s'boot.efi'",
            "/ip/dhcp-server/network/set .id=*2 dhcp-option=tftp,test-opt67",
            "/ip/dhcp-server/option/remove .id=*7",
        ], "{p:#?}");
    }

    #[test]
    fn alert_becomes_commands() {
        let mut l = live();
        let t = to_text(&l);
        let mut v = srv(&t);
        v["alert"] = json!(true);
        v["alert_valid"] = json!(["AA:BB:CC:DD:EE:FF"]);
        let new = dhcp::apply(&t, &[json!({"op": "srv_set", "name": "test", "server": v})]).unwrap();
        let p: Vec<String> = plan(&l, &new).unwrap().iter().map(show).collect();
        assert_eq!(p, vec!["/ip/dhcp-server/alert/add interface=test-lan valid-server=AA:BB:CC:DD:EE:FF disabled=false"], "{p:#?}");
        l.alerts = vec![json!({".id": "*9", "interface": "test-lan", "valid-server": "AA:BB:CC:DD:EE:FF", "disabled": "false"})];
        assert!(plan(&l, &new).unwrap().is_empty());
        assert!(to_text(&l).contains("    alert: true\n    alert_valid: [AA:BB:CC:DD:EE:FF]\n"));
        let p: Vec<String> = plan(&l, &t).unwrap().iter().map(show).collect();
        assert_eq!(p, vec!["/ip/dhcp-server/alert/remove .id=*9"]);
    }

    #[test]
    fn rename_and_delete() {
        let l = live();
        let t = to_text(&l);
        let mut v = srv(&t);
        v["name"] = json!("lab");
        let new = dhcp::apply(&t, &[json!({"op": "srv_set", "name": "test", "server": v})]).unwrap();
        let p: Vec<String> = plan(&l, &new).unwrap().iter().map(show).collect();
        // the static lease follows (its server field names the new one)
        assert_eq!(p, vec![
            "/ip/pool/set .id=*3 name=lab",
            "/ip/dhcp-server/set .id=*1 name=lab address-pool=lab",
            "/ip/dhcp-server/lease/set .id=*4 server=lab",
        ], "{p:#?}");
        // the reservation goes first (the page queues it), then the server
        let new = dhcp::apply(&t, &[json!({"op": "del", "mac": "AA:BB:CC:DD:EE:01"}), json!({"op": "srv_del", "name": "test"})]).unwrap();
        let p: Vec<String> = plan(&l, &new).unwrap().iter().map(show).collect();
        assert_eq!(p, vec![
            "/ip/dhcp-server/lease/remove .id=*4",
            "/ip/dhcp-server/lease/remove .id=*5",
            "/ip/dhcp-server/remove .id=*1",
            "/ip/dhcp-server/network/remove .id=*2",
            "/ip/pool/remove .id=*3",
        ], "{p:#?}");
    }
}
