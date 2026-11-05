//! System page (WinBox's System menu): clock and NTP, logs and logging
//! rules, users, scheduler and scripts, files, export and backup, packages,
//! certificates, SNMP and resources.
//!
//! Reads use the read account (every mode). Everything that needs the write
//! account (writes, but also file downloads, exports and the CPU profile,
//! which the read account may not run) is for direct mode only. A change is
//! one operation: previewed as RouterOS commands, run, then read back.
//! Nothing here can cut Cairn Net's way in (its accounts can't be deleted or
//! restricted, the rollback scheduler can't be touched), so no rollback; a
//! restore saves a backup first and restarts the switch.
//! Passwords (new users, backups) pass through and are masked everywhere.

use crate::direct::{show, Cmd};
use crate::settings::{Switch, SwitchCfg};
use crate::{infra, print, Api};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::Duration;

/// Tables: id absent = add, id = set, del = remove. Only these fields are sent.
const TABLES: &[(&str, &str, &[&str])] = &[
    ("log-rule", "/system/logging", &["topics", "action", "prefix", "regex", "disabled"]),
    ("log-action", "/system/logging/action", &["name", "target", "remote", "remote-port", "remote-protocol", "remote-log-format", "src-address",
        "memory-lines", "disk-file-name", "disk-lines-per-file", "disk-file-count"]),
    ("sched", "/system/scheduler", &["name", "start-date", "start-time", "interval", "on-event", "policy", "disabled", "comment"]),
    ("script", "/system/script", &["name", "source", "policy", "dont-require-permissions", "comment"]),
    ("user", "/user", &["name", "group", "password", "address", "comment"]),
    ("snmp-community", "/snmp/community", &["name", "addresses", "read-access", "write-access", "disabled"]),
];
/// Settings without rows.
const SINGLE: &[(&str, &str, &[&str])] = &[
    ("clock", "/system/clock", &["time-zone-name", "time-zone-autodetect"]),
    ("ntp", "/system/ntp/client", &["enabled", "mode", "servers"]),
    ("snmp", "/snmp", &["enabled", "contact", "location", "trap-target", "trap-community", "trap-version"]),
];
const MAX_FILE: usize = 32 << 20; // download size cap
/// RouterOS 7.25 answers "contents too long" to a /file/add above about 61 KB,
/// and never answers from 64 KB on: bigger files go over SFTP.
const MAX_UPLOAD: usize = 60_000;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

/// `print` of every property (no proplist); an unknown menu reads as empty.
fn all(api: &mut Api, path: &str) -> Result<Vec<Value>, String> {
    match api.call(&[&format!("{path}/print")]) {
        Err(e) if e.starts_with("no such command") => Ok(vec![]),
        r => r,
    }
}

/// Everything the page shows but the log, read account.
pub fn read(api: &mut Api) -> Result<Value, String> {
    Ok(json!({
        "clock": all(api, "/system/clock")?.pop(),
        "ntp": all(api, "/system/ntp/client")?.pop(),
        "resource": all(api, "/system/resource")?.pop(),
        "cpu": all(api, "/system/resource/cpu")?,
        "routerboard": print(api, "/system/routerboard", "model,current-firmware,upgrade-firmware,firmware-type", &[])?.pop(),
        "log_rules": all(api, "/system/logging")?,
        "log_actions": all(api, "/system/logging/action")?,
        "users": print(api, "/user", ".id,name,group,address,last-logged-in,disabled,expired,comment", &[])?,
        "groups": print(api, "/user/group", ".id,name,policy,comment", &[])?,
        "active": print(api, "/user/active", ".id,when,name,address,via,group", &[])?,
        "scheds": all(api, "/system/scheduler")?,
        "scripts": all(api, "/system/script")?,
        "files": print(api, "/file", ".id,name,type,size,last-modified", &[])?,
        "packages": print(api, "/system/package", ".id,name,version,disabled,scheduled,available,build-time", &[])?,
        "certs": print(api, "/certificate", ".id,name,common-name,issuer,invalid-before,invalid-after,expires-after,trusted,private-key,expired,revoked,fingerprint", &[])?,
        "snmp": print(api, "/snmp", "enabled,contact,location,trap-target,trap-community,trap-version", &[])?.pop(),
        "communities": print(api, "/snmp/community", ".id,name,addresses,security,read-access,write-access,disabled,default", &[])?,
    }))
}

/// The memory log (up to its memory-lines), oldest first.
pub fn log(api: &mut Api) -> Result<Vec<Value>, String> {
    print(api, "/log", ".id,time,topics,message", &[])
}

/// What the plan needs from the switch to refuse what would lock Cairn Net out.
pub struct Ctx<'a> {
    pub mine: [&'a str; 2], // Cairn Net's read and write accounts
    pub users: Vec<Value>,
    pub scheds: Vec<Value>,
}

impl Ctx<'_> {
    pub fn load<'a>(api: &mut Api, c: &'a SwitchCfg) -> Result<Ctx<'a>, String> {
        Ok(Ctx { mine: [c.user.as_str(), c.write_user.as_str()], users: print(api, "/user", ".id,name,group,disabled", &[])?,
            scheds: print(api, "/system/scheduler", ".id,name", &[])? })
    }
}

fn id_ok(id: &str) -> bool {
    id.len() > 1 && id.starts_with('*') && id[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// A file name RouterOS keeps as given: no way out of its file area.
fn name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 200 && !n.starts_with('/') && !n.split('/').any(|p| p.is_empty() || p == "..") && !n.chars().any(|c| c.is_control())
}

const PACKAGE_DO: [&str; 4] = ["enable", "disable", "uninstall", "unschedule"];

/// One operation of the page -> RouterOS commands. Ops:
/// {t: table, id?, set: {field: value}} | {t: table, id, del: true} | {t: "script", id, run: true}
/// | {t: single, set} | {t: "file", name, del: true} | {t: "backup", name, password?}
/// | {t: "pkg", name, do: enable|disable|uninstall|unschedule} | {t: "rb-upgrade"}.
pub fn plan(op: &Value, ctx: &Ctx) -> Result<Vec<Cmd>, String> {
    let t = s(op, "t");
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    let id = s(op, "id");
    if !id.is_empty() && !id_ok(id) {
        return Err("bad id".into());
    }
    let fields = |allowed: &[&str]| -> Result<Vec<(String, String)>, String> {
        let set = op["set"].as_object().ok_or("nothing to set")?;
        let mut out = Vec::new();
        for (k, v) in set {
            if !allowed.contains(&k.as_str()) {
                return Err(format!("{k}: not editable here"));
            }
            let v = match v { Value::String(x) => x.clone(), Value::Bool(b) => (if *b { "yes" } else { "no" }).into(), Value::Number(n) => n.to_string(), _ => return Err(format!("{k}: bad value")) };
            if v.contains(['\0']) {
                return Err(format!("{k}: bad value"));
            }
            out.push((k.clone(), v));
        }
        if out.is_empty() {
            return Err("nothing to set".into());
        }
        Ok(out)
    };
    if let Some((_, path, allowed)) = SINGLE.iter().find(|x| x.0 == t) {
        return Ok(vec![(format!("{path}/set"), fields(allowed)?)]);
    }
    match t {
        "file" => {
            let n = s(op, "name");
            if !name_ok(n) || !op["del"].as_bool().unwrap_or(false) {
                return Err("bad file operation".into());
            }
            Ok(vec![("/file/remove".into(), vec![kv("numbers", n)])])
        }
        "backup" => {
            let n = s(op, "name").trim_end_matches(".backup");
            if !name_ok(n) || n.contains('/') {
                return Err("bad backup name".into());
            }
            let pw = s(op, "password");
            Ok(vec![("/system/backup/save".into(), if pw.is_empty() { vec![kv("name", n), kv("dont-encrypt", "yes")] } else { vec![kv("name", n), kv("password", pw)] })])
        }
        "pkg" => {
            let (n, d) = (s(op, "name"), s(op, "do"));
            if !PACKAGE_DO.contains(&d) || n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("bad package operation".into());
            }
            if n == "routeros" && d != "unschedule" {
                return Err("the routeros package itself is left alone".into());
            }
            Ok(vec![(format!("/system/package/{d}"), vec![kv("numbers", n)])])
        }
        "rb-upgrade" => Ok(vec![("/system/routerboard/upgrade".into(), vec![])]),
        // downloads what enable asked for, then restarts to install all scheduled changes
        // called directly through the API it answers done and does nothing (its
        // confirmation prompt says no): run as a background script it restarts
        "pkg-apply" => Ok(vec![("/execute".into(), vec![kv("script", "/system package apply-changes")])]),
        _ => {
            let (_, path, allowed) = TABLES.iter().find(|x| x.0 == t).ok_or_else(|| format!("{t}: unknown"))?;
            let row = |rows: &[Value]| rows.iter().find(|r| s(r, ".id") == id).cloned();
            if t == "sched" && !id.is_empty() && row(&ctx.scheds).is_some_and(|r| s(&r, "name") == infra::ROLLBACK) {
                return Err("Cairn Net's rollback scheduler is not edited here".into());
            }
            if t == "user" {
                let u = row(&ctx.users);
                if !id.is_empty() {
                    let u = u.ok_or("unknown user")?;
                    let name = s(&u, "name");
                    if ctx.mine.contains(&name) {
                        return Err(format!("{name} is Cairn Net's own account: not deleted or restricted here"));
                    }
                    // a person's way in with every right: Cairn Net's own account does not count
                    let full = |r: &&Value| s(r, "group") == "full" && s(r, "disabled") != "true" && !ctx.mine.contains(&s(r, "name"));
                    if op["del"].as_bool() == Some(true) && full(&&u) && ctx.users.iter().filter(full).count() <= 1 {
                        return Err(format!("{name} is the last account with full rights"));
                    }
                }
            }
            if op["del"].as_bool() == Some(true) {
                return if id.is_empty() { Err("which row?".into()) } else { Ok(vec![(format!("{path}/remove"), vec![kv("numbers", id)])]) };
            }
            if op["run"].as_bool() == Some(true) {
                return if t == "script" && !id.is_empty() { Ok(vec![("/system/script/run".into(), vec![kv("number", id)])]) } else { Err("only a script runs".into()) };
            }
            let mut f = fields(allowed)?;
            if t == "user" {
                if !id.is_empty() && f.iter().any(|(k, _)| k != "address" && k != "comment") {
                    return Err("an existing user: only its allowed addresses and comment change here".into());
                }
                if id.is_empty() && ["name", "group", "password"].iter().any(|k| !f.iter().any(|(x, v)| x == k && !v.is_empty())) {
                    return Err("a new user needs a name, a group and a password".into());
                }
            }
            if t == "sched" && f.iter().any(|(k, v)| k == "name" && v == infra::ROLLBACK) {
                return Err(format!("{}: name reserved by Cairn Net", infra::ROLLBACK));
            }
            if id.is_empty() {
                Ok(vec![(format!("{path}/add"), f)])
            } else {
                f.insert(0, kv(".id", id));
                Ok(vec![(format!("{path}/set"), f)])
            }
        }
    }
}

/// A command as shown and logged: passwords never.
pub fn shown(c: &Cmd) -> String {
    let masked: Vec<(String, String)> = c.1.iter().map(|(k, v)| (k.clone(), if k == "password" { "*****".into() } else { v.clone() })).collect();
    show(&(c.0.clone(), masked))
}

/// Operations that need the switch's name typed (second confirmation).
pub fn needs_name(op: &Value) -> bool {
    matches!(s(op, "t"), "pkg" | "pkg-apply" | "rb-upgrade")
}

/// RouterOS takes yes/no and prints true/false.
fn norm(v: &str) -> &str {
    match v {
        "yes" => "true",
        "no" => "false",
        x => x,
    }
}

/// Read back after the commands: the row (or setting) carries what was sent,
/// a removed row is gone. Ok(None): nothing to read back (run, packages…).
fn verify(api: &mut Api, op: &Value, cmd: &Cmd) -> Result<Option<bool>, String> {
    let path = cmd.0.rsplit_once('/').map_or("", |x| x.0);
    let verb = cmd.0.rsplit('/').next().unwrap_or("");
    let rows = |api: &mut Api, id: &str| -> Result<Vec<Value>, String> { api.call(&[&format!("{path}/print"), &format!("?.id={id}")]) };
    let same = |r: &Value| cmd.1.iter().filter(|(k, _)| k != ".id" && k != "password").all(|(k, v)| norm(s(r, k)) == norm(v));
    Ok(match (s(op, "t"), verb) {
        ("file", _) => Some(print(api, "/file", ".id", &[&format!("?name={}", s(op, "name"))])?.is_empty()),
        ("backup", _) => Some(!print(api, "/file", ".id", &[&format!("?name={}.backup", s(op, "name").trim_end_matches(".backup"))])?.is_empty()),
        (t, "set") if SINGLE.iter().any(|x| x.0 == t) => Some(all(api, path)?.pop().is_some_and(|r| same(&r))),
        (_, "remove") => Some(rows(api, s(op, "id"))?.is_empty()),
        (_, "set") => Some(rows(api, s(op, "id"))?.pop().is_some_and(|r| same(&r))),
        // the API keeps the new id in !done, which `call` drops: any row as sent
        (_, "add") => Some(all(api, path)?.iter().any(same)),
        _ => None,
    })
}

pub fn preview(sw: &Switch, op: &Value) -> Result<Value, String> {
    let cmds = crate::with_api(sw, |api| plan(op, &Ctx::load(api, &sw.cfg)?))?;
    Ok(json!({"ok": true, "commands": cmds.iter().map(shown).collect::<Vec<_>>(), "confirm_name": needs_name(op)}))
}

fn write_api(c: &SwitchCfg) -> Result<Api, String> {
    Api::connect(c, &c.write_user, &c.write_password).map_err(|e| format!("write account: {e}"))
}

pub fn apply(sw: &Switch, v: &Value, user: &str) -> Result<Value, String> {
    let c = &sw.cfg;
    let op = &v["op"];
    if needs_name(op) && s(v, "confirm") != c.name {
        return Err("type the switch's name to confirm".into());
    }
    let mut api = write_api(c)?;
    let cmds = plan(op, &Ctx::load(&mut api, c)?)?;
    let shown_all: Vec<String> = cmds.iter().map(shown).collect();
    let restarts = s(op, "t") == "pkg-apply";
    if restarts {
        if !print(&mut api, "/system/package", "scheduled", &[])?.iter().any(|p| !s(p, "scheduled").is_empty()) {
            return Err("no package change is waiting".into());
        }
        // the Infrastructure page follows the restart (REBOOTS, main.rs)
        let from = print(&mut api, "/system/resource", "version", &[])?.pop().unwrap_or_default()["version"].clone();
        crate::REBOOTS.lock().unwrap_or_else(|e| e.into_inner())
            .insert(c.name.clone(), json!({"action": "reboot", "from": from, "to": Value::Null, "at": crate::now_ms()}));
    }
    let mut verified = Some(true);
    for (i, cmd) in cmds.iter().enumerate() {
        let words: Vec<String> = std::iter::once(cmd.0.clone()).chain(cmd.1.iter().map(|(k, v)| format!("={k}={v}"))).collect();
        let w: Vec<&str> = words.iter().map(String::as_str).collect();
        let res = api.call(&w).map_err(|e| format!("{}: {e}", shown_all[i]));
        match res {
            Ok(_) => {}
            // apply-changes restarts the switch: the connection goes with it
            Err(e) if restarts && !e.contains("permission") && !e.contains("failure") => {
                eprintln!("system on {} by {user}: {} (restarting)", c.name, shown_all[i]);
                crate::forget_cache();
                return Ok(json!({"ok": true, "done": cmds.len(), "commands": shown_all, "restarting": true}));
            }
            Err(e) => {
                if restarts {
                    crate::REBOOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&c.name);
                }
                eprintln!("system on {} by {user}: {e}", c.name);
                return Ok(json!({"ok": false, "done": i, "commands": shown_all, "error": e}));
            }
        };
        // a backup is written in the background: give it a moment to appear
        if s(op, "t") == "backup" {
            std::thread::sleep(Duration::from_secs(2));
        }
        verified = match (verified, verify(&mut api, op, cmd)?) {
            (Some(a), Some(b)) => Some(a && b),
            (_, None) => None,
            (None, _) => None,
        };
    }
    eprintln!("system on {} by {user}: {} (read back: {})", c.name, shown_all.join(" ; "), verified.map_or("n/a", |b| if b { "as asked" } else { "differs" }));
    crate::forget_cache();
    Ok(json!({"ok": verified != Some(false), "done": cmds.len(), "commands": shown_all, "verified": verified, "restarting": restarts}))
}

/* ---------- files: raw bytes through the API ---------- */

impl Api {
    fn raw_word(&mut self) -> std::io::Result<Vec<u8>> {
        let mut b = [0u8; 1];
        self.0.read_exact(&mut b)?;
        let len = crate::get_len(b[0], || {
            let mut b = [0u8; 1];
            self.0.read_exact(&mut b).map(|_| b[0])
        })?;
        let mut buf = vec![0u8; len as usize];
        self.0.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// `call` with byte words, for file contents: the value of `key` in the
    /// replies (several replies: joined).
    fn call_bytes(&mut self, words: &[&[u8]], key: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        for w in words {
            crate::put_len(w.len(), &mut out);
            out.extend_from_slice(w);
        }
        out.push(0);
        self.0.write_all(&out).map_err(|e| e.to_string())?;
        let prefix = format!("={key}=");
        let (mut val, mut trap) = (Vec::new(), None);
        loop {
            let mut sentence = Vec::new();
            loop {
                let w = self.raw_word().map_err(|e| e.to_string())?;
                if w.is_empty() {
                    break;
                }
                sentence.push(w);
            }
            match sentence.first().map(Vec::as_slice) {
                Some(b"!re") => {
                    for w in &sentence[1..] {
                        if let Some(x) = w.strip_prefix(prefix.as_bytes()) {
                            val.extend_from_slice(x);
                        }
                    }
                }
                Some(b"!trap") => trap = Some(sentence.iter().find_map(|w| w.strip_prefix(b"=message=")).map(|m| String::from_utf8_lossy(m).into_owned()).unwrap_or("trap".into())),
                Some(b"!fatal") => return Err("fatal".into()),
                Some(b"!done") => return trap.map_or(Ok(val), Err),
                _ => {}
            }
        }
    }
}

/// A whole file of the switch, in 32 KB chunks.
fn read_file(api: &mut Api, name: &str) -> Result<Vec<u8>, String> {
    let size: usize = print(api, "/file", "size", &[&format!("?name={name}")])?.pop().ok_or_else(|| format!("{name}: no such file"))?["size"]
        .as_str().and_then(|x| x.parse().ok()).unwrap_or(0);
    if size > MAX_FILE {
        return Err(format!("{name}: larger than {} MB", MAX_FILE >> 20));
    }
    let mut data = Vec::with_capacity(size);
    let file = format!("=file={name}");
    while data.len() < size {
        let off = format!("=offset={}", data.len());
        let chunk = api.call_bytes(&[b"/file/read", file.as_bytes(), b"=chunk-size=32768", off.as_bytes()], "data")?;
        if chunk.is_empty() {
            break;
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

pub fn download(c: &SwitchCfg, name: &str) -> Result<Vec<u8>, String> {
    if !name_ok(name) {
        return Err("bad file name".into());
    }
    read_file(&mut write_api(c)?, name)
}

pub fn upload(c: &SwitchCfg, v: &Value, user: &str) -> Result<Value, String> {
    let name = s(v, "name");
    if !name_ok(name) {
        return Err("bad file name".into());
    }
    let data = openssl::base64::decode_block(s(v, "data")).map_err(|_| "bad file data")?;
    if data.len() > MAX_UPLOAD {
        return Err(format!("larger than {} KB: RouterOS takes no bigger file through its API", MAX_UPLOAD / 1000));
    }
    let mut api = write_api(c)?;
    if !print(&mut api, "/file", ".id", &[&format!("?name={name}")])?.is_empty() {
        return Err(format!("{name} already exists: delete it first"));
    }
    let mut contents = b"=contents=".to_vec();
    contents.extend_from_slice(&data);
    api.call_bytes(&[b"/file/add", format!("=name={name}").as_bytes(), &contents], "ret")?;
    let size = print(&mut api, "/file", "size", &[&format!("?name={name}")])?.pop().map(|r| s(&r, "size").to_string());
    let ok = size.as_deref() == Some(&data.len().to_string());
    eprintln!("system on {}: {name} uploaded by {user} ({} bytes, read back {})", c.name, data.len(), size.as_deref().unwrap_or("none"));
    Ok(json!({"ok": ok, "size": data.len(), "verified": ok}))
}

/// `/export` to a temporary file, read, removed. sensitive = show-sensitive.
pub fn export(c: &SwitchCfg, sensitive: bool, user: &str) -> Result<Value, String> {
    let mut api = write_api(c)?;
    let base = format!("cairn-net-export-{}", crate::now_ms());
    let file = format!("=file={base}");
    let mut words = vec!["/export", file.as_str()];
    if sensitive {
        words.push("=show-sensitive=");
    }
    api.call(&words)?;
    let name = format!("{base}.rsc");
    // written in the background: wait until it is there and stops growing
    let mut last = String::new();
    for _ in 0..30 {
        let sz = print(&mut api, "/file", "size", &[&format!("?name={name}")])?.pop().map(|r| s(&r, "size").to_string()).unwrap_or_default();
        if !sz.is_empty() && sz == last {
            break;
        }
        last = sz;
        std::thread::sleep(Duration::from_millis(500));
    }
    let data = read_file(&mut api, &name);
    let _ = api.call(&["/file/remove", &format!("=numbers={name}")]);
    eprintln!("system on {}: export{} by {user}", c.name, if sensitive { " (show-sensitive)" } else { "" });
    Ok(json!({"ok": true, "text": String::from_utf8_lossy(&data?)}))
}

/// CPU profile over `secs` seconds: average usage (%) per process name.
pub fn profile(c: &SwitchCfg, secs: u64) -> Result<Value, String> {
    let mut api = write_api(c)?;
    let rows = api.call(&["/tool/profile", &format!("=duration={}s", secs.clamp(1, 10)), "=cpu=all"])
        .or_else(|_| api.call(&["/tool/profile", &format!("=duration={}s", secs.clamp(1, 10))]))?;
    Ok(json!(average(&rows)))
}

/// Profile rows (one section per second) -> [{name, cpu, usage}] by usage.
fn average(rows: &[Value]) -> Vec<Value> {
    let sections = rows.iter().map(|r| s(r, ".section")).collect::<std::collections::BTreeSet<_>>().len().max(1) as f64;
    let mut sum: std::collections::BTreeMap<(String, String), f64> = Default::default();
    for r in rows {
        *sum.entry((s(r, "name").to_string(), s(r, "cpu").to_string())).or_default() += s(r, "usage").trim_end_matches('%').parse::<f64>().unwrap_or(0.0);
    }
    let mut out: Vec<Value> = sum.into_iter().map(|((n, c), u)| json!({"name": n, "cpu": c, "usage": (u / sections * 10.0).round() / 10.0})).collect();
    out.sort_by(|a, b| b["usage"].as_f64().partial_cmp(&a["usage"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Restore a backup of the switch: a backup of the current state first
/// (cairn-net-before-restore), then /system/backup/load, which restarts it.
pub fn restore(c: &SwitchCfg, v: &Value, user: &str) -> Result<Value, String> {
    let name = s(v, "name");
    if s(v, "confirm") != c.name {
        return Err("type the switch's name to confirm".into());
    }
    if !name_ok(name) || !name.ends_with(".backup") {
        return Err("choose a .backup file".into());
    }
    let mut api = write_api(c)?;
    if !print(&mut api, "/system/scheduler", ".id", &[&format!("?name={}", infra::ROLLBACK)])?.is_empty() {
        return Err("a change is still waiting for its confirmation: confirm it or wait for the rollback".into());
    }
    if print(&mut api, "/file", ".id", &[&format!("?name={name}")])?.is_empty() {
        return Err(format!("{name}: no such file"));
    }
    let before = "cairn-net-before-restore";
    api.call(&["/system/backup/save", &format!("=name={before}"), "=dont-encrypt=yes"]).map_err(|e| format!("backup before restoring: {e}"))?;
    std::thread::sleep(Duration::from_secs(2));
    if print(&mut api, "/file", ".id", &[&format!("?name={before}.backup")])?.is_empty() {
        return Err("the backup before restoring did not appear: nothing restored".into());
    }
    let from = print(&mut api, "/system/resource", "version", &[])?.pop().unwrap_or_default()["version"].clone();
    crate::REBOOTS.lock().unwrap_or_else(|e| e.into_inner())
        .insert(c.name.clone(), json!({"action": "restore", "from": from, "to": Value::Null, "at": crate::now_ms()}));
    let (n, pw) = (format!("=name={name}"), format!("=password={}", s(v, "password")));
    // RouterOS wants the password word even empty (unencrypted backup)
    let words = ["/system/backup/load", n.as_str(), pw.as_str()];
    // the connection goes with the switch: only a refusal is an error
    if let Err(e) = api.call(&words) {
        if !e.contains("timed out") && !e.contains("reset") && !e.contains("eof") && !e.contains("failed to fill") {
            crate::REBOOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&c.name);
            return Err(format!("restore refused: {e}"));
        }
    }
    eprintln!("system on {}: restore of {name} by {user} ({before}.backup saved first)", c.name);
    crate::forget_cache();
    Ok(json!({"ok": true, "before": format!("{before}.backup")}))
}

/* ---------- HTTP ---------- */

/// %xx and + of a query string value.
fn unescape(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => match b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()) {
                Some(x) => {
                    out.push(x);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            x => out.push(x),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn query(url: &str, key: &str) -> String {
    url.split_once('?').map_or("", |x| x.1).split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')).map(unescape).unwrap_or_default()
}

/// GET /api/system?switch=, /api/system/log?switch=, /api/system/file?switch=&name=
pub fn get(url: &str) -> Result<(Vec<u8>, String, Option<String>), (u16, String)> {
    let path = url.split('?').next().unwrap_or("");
    let name = query(url, "switch");
    let sw = crate::settings::get(&name).ok_or((404, format!("{name}: unknown switch")))?;
    let c = &sw.cfg;
    let js = |v: Value| Ok((v.to_string().into_bytes(), "application/json".to_string(), None));
    match path {
        "/api/system" => {
            let mut v = crate::with_api(&sw, |api| {
                let mut v = read(api)?;
                if !c.write_user.is_empty() {
                    if let Some(u) = print(api, "/user", "group", &[&format!("?name={}", c.write_user)])?.pop() {
                        let g = s(&u, "group").to_string();
                        if let Some(p) = print(api, "/user/group", "policy", &[&format!("?name={g}")])?.pop() {
                            v["write_policy"] = json!(s(&p, "policy").split(',').filter(|x| !x.is_empty() && !x.starts_with('!')).collect::<Vec<_>>());
                        }
                    }
                }
                Ok(v)
            }).unwrap_or_else(|e| json!({"error": e}));
            v["name"] = json!(c.name);
            v["mode"] = json!(c.mode);
            v["mine"] = json!([c.user, c.write_user]);
            v["rollback_name"] = json!(infra::ROLLBACK);
            js(v)
        }
        "/api/system/log" => js(crate::with_api(&sw, log).map_or_else(|e| json!({"error": e}), |l| json!({"log": l}))),
        "/api/system/file" => {
            if c.mode != "direct" {
                return Err((403, format!("{} is not in direct mode", c.name)));
            }
            let f = query(url, "name");
            let data = download(c, &f).map_err(|e| (502, e))?;
            let base: String = f.rsplit('/').next().unwrap_or("file").chars().map(|ch| if ch.is_ascii_alphanumeric() || ".-_".contains(ch) { ch } else { '_' }).collect();
            Ok((data, "application/octet-stream".into(), Some(format!("attachment; filename=\"{}-{base}\"", c.name))))
        }
        _ => Err((404, "not found".into())),
    }
}

/// Answers a GET of get(): JSON, or a file as a download.
pub fn respond(r: tiny_http::Request) {
    let h = |k: &str, v: &str| tiny_http::Header::from_bytes(k, v).unwrap();
    let (code, data, ctype, disp) = match get(r.url()) {
        Ok((d, c, disp)) => (200, d, c, disp),
        Err((code, e)) => (code, e.into_bytes(), "text/plain".into(), None),
    };
    let mut resp = tiny_http::Response::from_data(data).with_status_code(code)
        .with_header(h("Content-Type", &format!("{ctype}; charset=utf-8")))
        .with_header(h("Cache-Control", "no-store"))
        .with_header(h("X-Content-Type-Options", "nosniff"))
        .with_header(h("Content-Security-Policy", "default-src 'self'; frame-ancestors 'none'"));
    if let Some(d) = disp {
        resp = resp.with_header(h("Content-Disposition", &d));
    }
    let _ = r.respond(resp);
}

/// POST /api/system/{preview,apply,upload,export,profile,restore}: direct mode only.
pub fn post(path: &str, v: &Value, user: &str) -> Value {
    let r = (|| {
        let sw = crate::direct_switch(v)?;
        let c = &sw.cfg;
        match path {
            "/api/system/preview" => preview(&sw, &v["op"]),
            "/api/system/apply" => apply(&sw, v, user),
            "/api/system/upload" => upload(c, v, user),
            "/api/system/export" => export(c, v["sensitive"].as_bool().unwrap_or(false), user),
            "/api/system/profile" => profile(c, v["seconds"].as_u64().unwrap_or(5)).map(|p| json!({"ok": true, "profile": p})),
            "/api/system/restore" => restore(c, v, user),
            _ => Err("not found".into()),
        }
    })();
    r.unwrap_or_else(|e| json!({"ok": false, "error": e}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx<'static> {
        Ctx {
            mine: ["cairn-read", "cairn-write"],
            users: vec![
                json!({".id": "*1", "name": "admin", "group": "full", "disabled": "false"}),
                json!({".id": "*2", "name": "cairn-read", "group": "cairn-read", "disabled": "false"}),
                json!({".id": "*3", "name": "cairn-write", "group": "full", "disabled": "false"}),
                json!({".id": "*4", "name": "old", "group": "full", "disabled": "true"}),
                json!({".id": "*5", "name": "bob", "group": "read", "disabled": "false"}),
            ],
            scheds: vec![json!({".id": "*A", "name": infra::ROLLBACK}), json!({".id": "*B", "name": "nightly"})],
        }
    }

    #[test]
    fn users_guarded() {
        let c = ctx();
        // Cairn Net's accounts: neither deleted nor restricted
        assert!(plan(&json!({"t": "user", "id": "*2", "del": true}), &c).is_err());
        assert!(plan(&json!({"t": "user", "id": "*3", "set": {"address": "10.0.0.1/32"}}), &c).is_err());
        // last enabled full account of a person (cairn-write, full too, does not count)
        assert!(plan(&json!({"t": "user", "id": "*1", "del": true}), &c).unwrap_err().contains("last account"));
        // the disabled full one can go, and so can bob
        assert!(plan(&json!({"t": "user", "id": "*4", "del": true}), &c).is_ok());
        assert_eq!(plan(&json!({"t": "user", "id": "*5", "set": {"address": "192.0.2.0/24"}}), &c).unwrap()[0].0, "/user/set");
        // existing user: no group or password change here
        assert!(plan(&json!({"t": "user", "id": "*5", "set": {"group": "full"}}), &c).is_err());
        // new user: name, group and password
        assert!(plan(&json!({"t": "user", "set": {"name": "x", "group": "read"}}), &c).is_err());
        let add = plan(&json!({"t": "user", "set": {"name": "x", "group": "read", "password": "s3cret"}}), &c).unwrap();
        assert!(!shown(&add[0]).contains("s3cret"), "password shown: {}", shown(&add[0]));
    }

    #[test]
    fn rollback_scheduler_untouched() {
        let c = ctx();
        assert!(plan(&json!({"t": "sched", "id": "*A", "del": true}), &c).is_err());
        assert!(plan(&json!({"t": "sched", "id": "*A", "set": {"interval": "1s"}}), &c).is_err());
        assert!(plan(&json!({"t": "sched", "set": {"name": infra::ROLLBACK, "on-event": "x"}}), &c).is_err());
        assert_eq!(plan(&json!({"t": "sched", "id": "*B", "del": true}), &c).unwrap()[0].0, "/system/scheduler/remove");
    }

    #[test]
    fn fields_and_ops() {
        let c = ctx();
        assert!(plan(&json!({"t": "ntp", "set": {"enabled": true, "servers": "pool.ntp.org"}}), &c).unwrap()[0].1.contains(&("enabled".into(), "yes".into())));
        assert!(plan(&json!({"t": "clock", "set": {"date": "x"}}), &c).is_err(), "field not allowed");
        assert!(plan(&json!({"t": "script", "id": "*1", "run": true}), &c).is_ok());
        assert!(plan(&json!({"t": "sched", "id": "*B", "run": true}), &c).is_err());
        assert!(plan(&json!({"t": "pkg", "name": "routeros", "do": "disable"}), &c).is_err());
        assert!(plan(&json!({"t": "pkg", "name": "ups", "do": "enable"}), &c).is_ok());
        assert!(needs_name(&json!({"t": "pkg-apply"})) && needs_name(&json!({"t": "rb-upgrade"})) && !needs_name(&json!({"t": "script"})));
        assert!(plan(&json!({"t": "user", "id": "1;x", "del": true}), &c).is_err(), "bad id");
        assert!(plan(&json!({"t": "file", "name": "../x", "del": true}), &c).is_err());
        assert!(plan(&json!({"t": "file", "name": "flash/a.rsc", "del": true}), &c).is_ok());
        let b = plan(&json!({"t": "backup", "name": "a", "password": "pw"}), &c).unwrap();
        assert!(shown(&b[0]).contains("password=*****"));
    }

    #[test]
    fn query_and_profile() {
        assert_eq!(query("/api/system/file?switch=SW%201&name=a+b%2Fc.rsc", "name"), "a b/c.rsc");
        assert_eq!(query("/x?switch=SW%201", "switch"), "SW 1");
        assert_eq!(unescape("100%"), "100%");
        let rows = vec![json!({"name": "total", "usage": "10", ".section": "0"}), json!({"name": "total", "usage": "20", ".section": "1"}),
            json!({"name": "dns", "usage": "2", ".section": "1"})];
        let a = average(&rows);
        assert_eq!(a[0]["name"], "total");
        assert_eq!(a[0]["usage"], 15.0);
        assert_eq!(a[1]["usage"], 1.0);
    }
}
