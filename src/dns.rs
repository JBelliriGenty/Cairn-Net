//! DNS integrations: the names of a switch's bound leases, published where a
//! resolver reads them, every 2 minutes (and at once when they change).
//! - hosts: "IP name" lines sent over SSH with the app's key; the target's
//!   authorized_keys forces the receiver (examples/cairn-net-hosts), so the
//!   key can do nothing else there.
//! - routeros: static DNS entries on the switch itself (comment "cairn-net"),
//!   written with its write account.
//! A switch that cannot be read publishes nothing: the last names stay.

use crate::direct::{run, Cmd};
use crate::{dhcp, print, Api};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const KINDS: [&str; 3] = ["none", "hosts", "routeros"];
const TAG: &str = "cairn-net";

/// user@host or user@host:port.
pub fn target_ok(t: &str) -> bool {
    let (dest, port) = t.rsplit_once(':').filter(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).unwrap_or((t, "22"));
    let Some((user, host)) = dest.split_once('@') else { return false };
    port.parse::<u16>().is_ok_and(|p| p > 0)
        && !user.is_empty() && user.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) && !user.starts_with('-')
        && !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c)) && !host.starts_with('-')
}

/// (IP, name) of the bound leases of `servers` (all when empty), with the
/// domain of their network for RouterOS (which has no search domain).
pub fn names(router: &Value, servers: &[String]) -> Vec<(String, String, String)> {
    let leases = router["leases"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let nets = router["dhcp_networks"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    dhcp::names_file(leases, servers, &crate::topo::manual_names(), &crate::mdns::names()).into_iter().map(|(ip, n)| {
        let dom = ip.parse().ok().and_then(|a| nets.iter().find(|x| dhcp::in_net(a, x["address"].as_str().unwrap_or(""))))
            .and_then(|x| x["domain"].as_str()).unwrap_or("").to_string();
        (ip, n, dom)
    }).collect()
}

pub fn hosts_text(names: &[(String, String, String)]) -> String {
    names.iter().map(|(ip, n, _)| format!("{ip} {n}\n")).collect()
}

/// Sends the lines; the receiver answers one line ("12 names" or "unchanged").
pub fn push_hosts(target: &str, text: &str) -> Result<String, String> {
    let (dest, port) = target.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).unwrap_or((target, "22"));
    let mut c = Command::new("ssh");
    c.args(crate::git::ssh_options()).args(["-o", "ConnectTimeout=10", "-p", port, "--", dest])
        .env("HOME", crate::git::data()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("ssh: {e}"))?;
    child.stdin.take().ok_or("ssh: no stdin")?.write_all(text.as_bytes()).map_err(|e| format!("ssh: {e}"))?;
    let o = child.wait_with_output().map_err(|e| format!("ssh: {e}"))?;
    let first = |b: &[u8]| String::from_utf8_lossy(b).lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added")).unwrap_or("").to_string();
    if o.status.success() { Ok(first(&o.stdout)) } else { Err(Some(first(&o.stderr)).filter(|e| !e.is_empty()).unwrap_or_else(|| "ssh failed".into())) }
}

/// The commands that make the switch's static entries tagged "cairn-net"
/// match `want` (name.domain when the network has a domain).
pub fn static_plan(live: &[Value], want: &[(String, String, String)]) -> Vec<Cmd> {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    let fq: Vec<(String, String)> = want.iter().map(|(ip, n, d)| (if d.is_empty() { n.clone() } else { format!("{n}.{d}") }, ip.clone())).collect();
    let mine: Vec<&Value> = live.iter().filter(|v| s(v, "comment") == TAG).collect();
    let mut out: Vec<Cmd> = mine.iter().filter(|v| !fq.iter().any(|(n, _)| *n == s(v, "name")))
        .map(|v| ("/ip/dns/static/remove".to_string(), vec![(".id".into(), s(v, ".id"))])).collect();
    for (n, ip) in &fq {
        match mine.iter().find(|v| s(v, "name") == *n) {
            Some(v) if s(v, "address") == *ip => {}
            Some(v) => out.push(("/ip/dns/static/set".into(), vec![(".id".into(), s(v, ".id")), ("address".into(), ip.clone())])),
            None => out.push(("/ip/dns/static/add".into(), vec![("name".into(), n.clone()), ("address".into(), ip.clone()), ("comment".into(), TAG.into())])),
        }
    }
    out
}

pub fn push_static(api: &mut Api, want: &[(String, String, String)]) -> Result<String, String> {
    let live = print(api, "/ip/dns/static", ".id,name,address,comment", &[])?;
    let cmds = static_plan(&live, want);
    match run(api, &cmds) {
        (n, None) if n == 0 => Ok("unchanged".into()),
        (n, None) => Ok(format!("{n} changes")),
        (_, Some(e)) => Err(e),
    }
}

/// Per switch: what was last sent, when, and how each target answered.
struct Sent {
    text: String,
    at: Instant,
    status: Value,
}
static SENT: Mutex<BTreeMap<String, Sent>> = Mutex::new(BTreeMap::new());

pub fn status(switch: &str) -> Value {
    SENT.lock().unwrap_or_else(|e| e.into_inner()).get(switch).map(|s| s.status.clone()).unwrap_or(Value::Null)
}

/// One pass over the switches (`slow` = the shared read of every switch).
pub fn publish(slow: &Value) {
    for sw in crate::settings::all() {
        let c = &sw.cfg;
        if c.dns.is_empty() || c.dns == "none" {
            continue;
        }
        let router = slow["switches"].as_array().into_iter().flatten().find(|x| x["name"] == c.name.as_str()).cloned().unwrap_or(Value::Null);
        let t = crate::now_ms();
        if router.is_null() || !router["error"].is_null() {
            let mut g = SENT.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = g.get_mut(&c.name) {
                s.status["skipped"] = json!({"t": t, "error": router["error"]});
            }
            continue;
        }
        crate::mdns::refresh(router["leases"].as_array().map(Vec::as_slice).unwrap_or(&[]), Duration::from_millis(1500));
        let n = names(&router, &c.dns_servers);
        let text = format!("{}|{}", c.dns, hosts_text(&n)) + &c.dns_targets.join(",");
        {
            let g = SENT.lock().unwrap_or_else(|e| e.into_inner());
            // unchanged: once every 30 min (a target may have been rebuilt)
            if g.get(&c.name).is_some_and(|s| s.text == text && s.at.elapsed() < Duration::from_secs(1800) && s.status["ok"] == true) {
                continue;
            }
        }
        let results: Vec<Value> = if c.dns == "hosts" {
            let body = hosts_text(&n);
            c.dns_targets.iter().map(|tg| match push_hosts(tg, &body) {
                Ok(m) => json!({"target": tg, "ok": true, "message": m}),
                Err(e) => json!({"target": tg, "ok": false, "message": e}),
            }).collect()
        } else {
            let r = Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}")).and_then(|mut a| push_static(&mut a, &n));
            vec![match r {
                Ok(m) => json!({"target": c.name, "ok": true, "message": m}),
                Err(e) => json!({"target": c.name, "ok": false, "message": e}),
            }]
        };
        let ok = results.iter().all(|r| r["ok"] == true);
        for r in results.iter().filter(|r| r["message"] != "unchanged") {
            eprintln!("dns {} -> {}: {}", c.name, r["target"].as_str().unwrap_or(""), r["message"].as_str().unwrap_or(""));
        }
        SENT.lock().unwrap_or_else(|e| e.into_inner()).insert(c.name.clone(), Sent { text, at: Instant::now(),
            status: json!({"t": t, "ok": ok, "kind": c.dns, "count": n.len(), "results": results}) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direct::show;

    fn show_plan(live: &[Value], want: &[(String, String, String)]) -> Vec<String> {
        static_plan(live, want).iter().map(show).collect()
    }

    #[test]
    fn targets() {
        assert!(target_ok("root@192.168.1.2") && target_ok("dns@pihole.lan:2222") && target_ok("a.b@h-1"));
        assert!(!target_ok("192.168.1.2") && !target_ok("-oProxyCommand=x@h") && !target_ok("u@-h") && !target_ok("u@h:0") && !target_ok("u@h;rm") && !target_ok("u @h"));
    }

    #[test]
    fn names_and_static_plan() {
        let router = json!({"leases": [
            {"address": "10.0.0.5", "host-name": "Phone", "server": "lan", "status": "bound"},
            {"address": "10.0.20.9", "host-name": "plug", "server": "iot", "status": "bound"},
            {"address": "10.0.0.6", "host-name": "gone", "server": "lan", "status": "waiting"}],
            "dhcp_networks": [{"address": "10.0.0.0/24", "domain": "home.example"}, {"address": "10.0.20.0/24", "domain": ""}]});
        let all = names(&router, &[]);
        assert_eq!(all, vec![("10.0.0.5".into(), "phone".into(), "home.example".into()), ("10.0.20.9".into(), "plug".into(), String::new())]);
        assert_eq!(names(&router, &["iot".into()]).len(), 1);
        assert_eq!(hosts_text(&all), "10.0.0.5 phone\n10.0.20.9 plug\n");
        let live = vec![json!({".id": "*1", "name": "phone.home.example", "address": "10.0.0.4", "comment": "cairn-net"}),
            json!({".id": "*2", "name": "old", "address": "10.0.0.7", "comment": "cairn-net"}),
            json!({".id": "*3", "name": "router", "address": "10.0.0.1", "comment": ""})];
        assert_eq!(show_plan(&live, &all), vec![
            "/ip/dns/static/remove .id=*2",
            "/ip/dns/static/set .id=*1 address=10.0.0.5",
            "/ip/dns/static/add name=plug address=10.0.20.9 comment=cairn-net",
        ]);
    }
}
