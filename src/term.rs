//! Terminal page: one RouterOS command line at a time, run through the API
//! (`/execute as-string`), its text output shown as the switch prints it.
//!
//! Commands that only read (print, monitor, export) run with the read
//! account on every switch; anything else needs direct mode and runs with
//! the write account, after a guard: /system reset-configuration is refused,
//! and /user, /password, /ip service, reboot/shutdown and anything naming
//! the management path (its interfaces, the switch's or the app's address)
//! wait until the switch's name is typed. Each command is logged
//! (data/terminal.log, one JSON line) with password-like values hidden.
//!
//! The guard reads words, not RouterOS syntax: it stops mistakes, not an
//! admin bent on getting round it (they hold the write account anyway).

use crate::settings;
use crate::Api;
use serde_json::{json, Value};
use std::io::Write;

const MAX_LEN: usize = 4096;
const READ_VERBS: [&str; 4] = ["print", "export", "monitor", "monitor-traffic"];
// verbs that cannot sit in the menu path of a read-only command
const WRITE_VERBS: [&str; 18] = ["add", "set", "remove", "unset", "enable", "disable", "move", "edit", "reset", "reboot", "shutdown",
    "run", "import", "execute", "password", "reset-configuration", "upgrade", "install"];
const SECRET_KEYS: [&str; 5] = ["password", "secret", "passphrase", "psk", "key"];

/// Only print, export or monitor, after a plain menu path, nothing written
/// to a file, no scripting.
pub fn read_only(cmd: &str) -> bool {
    if cmd.contains([';', '[', ']', '{', '}', '$', '\n', '\r']) {
        return false;
    }
    let toks: Vec<&str> = cmd.split(|c: char| c.is_whitespace() || c == '/').filter(|t| !t.is_empty()).collect();
    let Some(i) = toks.iter().position(|t| READ_VERBS.contains(t)) else { return false };
    let menu_ok = toks[..i].iter().all(|t| t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !WRITE_VERBS.contains(t));
    menu_ok && !toks[i + 1..].iter().any(|t| t.starts_with("file="))
}

/// Bare words and the values of key=value words (lists split, `!` dropped).
fn words(cmd: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut bare, mut vals) = (Vec::new(), Vec::new());
    for t in cmd.split(|c: char| c.is_whitespace() || "/;[]{}()\"".contains(c)).filter(|t| !t.is_empty()) {
        match t.split_once('=') {
            Some((_, v)) => vals.extend(v.split(',').map(|x| x.trim_start_matches('!')).filter(|x| !x.is_empty())),
            None => bare.push(t),
        }
    }
    (bare, vals)
}

/// Err = refused outright; Ok(reasons) = the switch's name must be typed
/// when not empty. `mgmt`: names and addresses on the management path.
pub fn guard(cmd: &str, mgmt: &[String]) -> Result<Vec<String>, String> {
    let (bare, vals) = words(cmd);
    if bare.contains(&"reset-configuration") {
        return Err("/system reset-configuration is refused: it would drop the app's accounts and addresses".into());
    }
    let mut why = Vec::new();
    if bare.iter().any(|w| *w == "user" || *w == "password") {
        why.push("accounts (/user, /password)".to_string());
    }
    if bare.windows(2).any(|w| w == ["ip", "service"]) {
        why.push("services (/ip service)".into());
    }
    if bare.iter().any(|w| *w == "reboot" || *w == "shutdown") {
        why.push("restart or shutdown".into());
    }
    for m in mgmt {
        if bare.contains(&m.as_str()) || vals.contains(&m.as_str()) {
            why.push(format!("management path ({m})"));
        }
    }
    Ok(why)
}

/// The command with password-like values replaced by ***.
pub fn redact(cmd: &str) -> String {
    let mut out = String::new();
    let mut rest = cmd;
    while let Some(eq) = rest.find('=') {
        let key_start = rest[..eq].rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
        let key = rest[key_start..eq].to_ascii_lowercase();
        out.push_str(&rest[..=eq]);
        rest = &rest[eq + 1..];
        if SECRET_KEYS.iter().any(|k| key.contains(k)) {
            let end = match rest.strip_prefix('"') {
                Some(r) => r.find('"').map_or(rest.len(), |i| i + 2),
                None => rest.find(char::is_whitespace).unwrap_or(rest.len()),
            };
            out.push_str("***");
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// RouterOS reports a script error inside the output: "... (line 1 column 10)".
fn is_error(out: &str) -> bool {
    let last = out.trim_end().rsplit('\n').next().unwrap_or("");
    last.ends_with(')') && last.rfind('(').is_some_and(|i| last[i..].contains("line "))
}

fn log(entry: Value) {
    eprintln!("terminal: {entry}");
    let path = std::path::Path::new(crate::git::data()).join("terminal.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{entry}");
    }
}

/// POST /api/term/run {switch, command, confirm?}: every outcome is logged,
/// refusals and pending confirmations too.
pub fn run(v: &Value, user: &str) -> Result<Value, String> {
    let name = v["switch"].as_str().unwrap_or("");
    let cmd = v["command"].as_str().unwrap_or("").trim();
    let mut entry = json!({"t": crate::now_ms(), "user": user, "switch": name, "command": redact(cmd)});
    let r = execute(name, cmd, v["confirm"].as_str());
    match &r {
        Ok(o) => {
            entry["account"] = o["account"].clone();
            entry["result"] = json!(if o["confirm"].is_array() { "confirm asked" } else if o["ok"] == true { "ok" } else { "error" });
            entry["error"] = if o["ok"] == true { Value::Null } else { o["error"].as_str().map_or_else(|| o["output"].as_str().map(str::trim).into(), Into::into) };
        }
        Err(e) => {
            entry["result"] = json!("refused");
            entry["error"] = json!(e);
        }
    }
    log(entry);
    r
}

fn execute(name: &str, cmd: &str, confirm: Option<&str>) -> Result<Value, String> {
    let sw = settings::get(name).ok_or_else(|| format!("{name}: unknown switch"))?;
    let c = &sw.cfg;
    if cmd.is_empty() || cmd.len() > MAX_LEN || cmd.chars().any(|ch| ch.is_control()) {
        return Err(format!("one line of at most {MAX_LEN} characters"));
    }
    let shown = redact(cmd);
    let ro = read_only(cmd);
    if !ro {
        if c.mode != "direct" || c.write_user.is_empty() {
            return Err(format!("{name} is read-only here (mode {}): print, export and monitor only", c.mode));
        }
        let mgmt = crate::with_api(&sw, |api| {
            let live = crate::ifaces::read(api)?;
            let p = crate::path_of(c, api)?;
            let mut m: Vec<String> = crate::ifaces::mgmt(&live, &p).ifaces.into_iter().collect();
            m.push(p.switch.to_string());
            m.extend(p.app.map(|a| a.to_string()));
            Ok(m)
        })?;
        let why = guard(cmd, &mgmt)?;
        if !why.is_empty() && confirm != Some(name) {
            return Ok(json!({"ok": false, "confirm": why, "command": shown, "account": "write"}));
        }
    }
    let (u, p, account) = if ro { (&c.user, &c.password, "read") } else { (&c.write_user, &c.write_password, "write") };
    // its own connection: closing it ends a command that never ends
    let res = Api::connect(c, u, p).and_then(|mut api| api.call_done(&["/execute", &format!("=script={cmd}"), "=as-string="]));
    let (ok, output, error) = match res {
        Ok((_, done)) => {
            let out = done.get("ret").and_then(Value::as_str).unwrap_or("").replace("\r\n", "\n");
            (!is_error(&out), out, None)
        }
        Err(e) if e.contains("timed out") || e.contains("temporarily unavailable") || e.contains("would block") =>
            (false, String::new(), Some("no end after 15 s, stopped: add once (monitor) or count= (ping)".to_string())),
        Err(e) => (false, String::new(), Some(e)),
    };
    Ok(json!({"ok": ok, "output": output, "error": error, "account": account, "command": shown}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_commands() {
        for c in ["/interface print", "/ip/address/print detail where interface=ether1", "ip address print", "/export", "/export terse",
            "/interface ethernet monitor ether1 once", "/interface monitor-traffic ether1 once", "/user print", "/ip route print where dst-address=0.0.0.0/0"] {
            assert!(read_only(c), "{c}");
        }
        for c in ["/ip address set 0 disabled=yes", "/system reboot", "/export file=x", "/ip address print; /system reboot", ":put [/user print]",
            "/system reboot print", "/ip address set 0 print", "/user/set admin password=x", "/interface print file=a", "$x print"] {
            assert!(!read_only(c), "{c}");
        }
    }

    #[test]
    fn guard_asks_or_refuses() {
        let m = vec!["ether1".to_string(), "10.0.0.2".to_string()];
        assert!(guard("/system reset-configuration no-defaults=yes", &m).is_err());
        assert!(guard("/system/reset-configuration", &m).is_err());
        assert_eq!(guard("/ip address add address=10.0.0.1/24 interface=ether2", &m).unwrap().len(), 0);
        assert_eq!(guard("/interface ethernet set ether10 mtu=1500", &m).unwrap().len(), 0);
        assert!(!guard("/interface disable ether1", &m).unwrap().is_empty());
        assert!(!guard("/interface bridge port add interface=!ether1,ether2 bridge=b", &m).unwrap().is_empty());
        assert!(!guard("/ip address set 1 address=10.0.0.2/24", &m).unwrap().is_empty());
        assert!(!guard("/user add name=x group=full", &m).unwrap().is_empty());
        assert!(!guard("/ip service disable api-ssl", &m).unwrap().is_empty());
        assert!(!guard("/ip/service/set api-ssl address=1.2.3.4", &m).unwrap().is_empty());
        assert!(!guard(":execute \"/system reboot\"", &m).unwrap().is_empty());
    }

    #[test]
    fn secrets_are_hidden() {
        assert_eq!(redact("/user add name=a password=s3cret group=read"), "/user add name=a password=*** group=read");
        assert_eq!(redact("/user set a password=\"two words\" comment=x"), "/user set a password=*** comment=x");
        assert_eq!(redact("/interface wireguard add private-key=abc"), "/interface wireguard add private-key=***");
        assert_eq!(redact("/password old-password=a new-password=b confirm-new-password=b"), "/password old-password=*** new-password=*** confirm-new-password=***");
        assert_eq!(redact("/ip address print where comment=key"), "/ip address print where comment=key");
    }

    #[test]
    fn errors_are_seen() {
        assert!(is_error("bad command name prnt (line 1 column 10)"));
        assert!(is_error("not enough permissions (9) (/system/identity/set; line 1)"));
        assert!(!is_error("  name: sw1"));
        assert!(!is_error("0 ether1 (line)"));
    }
}
