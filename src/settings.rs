//! Switches as the user set them in the Settings page: address, certificate
//! pin, accounts and how each switch is managed (mode). Kept in a JSON file
//! (mode 0600: it holds the RouterOS passwords) in the data directory; seeded
//! once from the configuration file when it does not exist yet.

use crate::Api;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex, OnceLock};

/// read: never written · direct: the app writes through the API · git: the
/// app edits a file in a repository, something else applies it.
pub const MODES: [&str; 3] = ["read", "direct", "git"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SwitchCfg {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub sha256: String, // lower-case hex, no colons: the certificate pin
    pub user: String,
    pub password: String,
    pub write_user: String,
    pub write_password: String,
    pub mode: String,
    pub ignore: Vec<String>,
    pub url: String,
    // git mode: repository, branch and path of the DHCP file in it
    pub git_url: String,
    pub git_branch: String,
    pub git_path: String,
}

pub fn pin(s: &str) -> String {
    s.replace(':', "").trim().to_lowercase()
}

/// AA:BB:… as RouterOS and openssl print it.
pub fn pretty(p: &str) -> String {
    p.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_uppercase()).collect::<Vec<_>>().join(":")
}

impl SwitchCfg {
    pub fn from_json(v: &Value) -> SwitchCfg {
        let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
        SwitchCfg {
            name: s("name"),
            host: s("host"),
            port: v["port"].as_u64().unwrap_or(8729) as u16,
            sha256: pin(&s("sha256")),
            user: s("user"),
            password: v["password"].as_str().unwrap_or("").to_string(),
            write_user: s("write_user"),
            write_password: v["write_password"].as_str().unwrap_or("").to_string(),
            mode: Some(s("mode")).filter(|m| !m.is_empty()).unwrap_or_else(|| "read".into()),
            ignore: v["ignore"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            url: s("url"),
            git_url: s("git_url"),
            git_branch: Some(s("git_branch")).filter(|b| !b.is_empty()).unwrap_or_else(|| "main".into()),
            git_path: s("git_path"),
        }
    }

    /// `secrets`: false for the page (it only learns whether a password is set).
    pub fn to_json(&self, secrets: bool) -> Value {
        let mut v = json!({"name": self.name, "host": self.host, "port": self.port, "sha256": pretty(&self.sha256), "user": self.user,
            "write_user": self.write_user, "mode": self.mode, "ignore": self.ignore, "url": self.url,
            "git_url": self.git_url, "git_branch": self.git_branch, "git_path": self.git_path});
        if secrets {
            v["password"] = json!(self.password);
            v["write_password"] = json!(self.write_password);
        } else {
            v["has_password"] = json!(!self.password.is_empty());
            v["has_write_password"] = json!(!self.write_password.is_empty());
        }
        v
    }

    /// The page sends empty passwords for "unchanged": take the stored ones
    /// (the write one only while the write account stays the same).
    pub fn keep_secrets(&mut self, old: &SwitchCfg) {
        if self.password.is_empty() {
            self.password.clone_from(&old.password);
        }
        if self.write_password.is_empty() && self.write_user == old.write_user {
            self.write_password.clone_from(&old.write_password);
        }
    }

    pub fn check(&self) -> Result<(), String> {
        let safe = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c));
        if !crate::dhcp::is_name(&self.name) {
            return Err("invalid switch name (letters, digits, dashes)".into());
        }
        if !safe(&self.host) {
            return Err("invalid address".into());
        }
        if self.port == 0 {
            return Err("invalid port".into());
        }
        if self.sha256.len() != 64 || !self.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("the certificate fingerprint is missing: test the connection first".into());
        }
        if !MODES.contains(&self.mode.as_str()) {
            return Err("unknown mode".into());
        }
        if self.user.is_empty() || self.password.is_empty() {
            return Err("the read account is missing".into());
        }
        if self.mode == "direct" && (self.write_user.is_empty() || self.write_password.is_empty()) {
            return Err("direct mode needs the write account".into());
        }
        // passed to git as arguments (never a shell): no option-looking or spaced values
        let plain = |v: &str| !v.starts_with('-') && !v.chars().any(|c| c.is_whitespace() || c.is_control());
        if !self.git_url.is_empty() {
            if !plain(&self.git_url) || !plain(&self.git_branch) || self.git_branch.is_empty() || !plain(&self.git_path) {
                return Err("invalid repository, branch or path".into());
            }
            if self.git_path.is_empty() || self.git_path.starts_with('/') || self.git_path.split('/').any(|p| p == ".." || p.is_empty()) {
                return Err("the file path must be relative to the repository (e.g. network/dhcp.yml)".into());
            }
        }
        if [&self.user, &self.write_user].iter().any(|u| u.contains(['=', '\n'])) || [&self.password, &self.write_password].iter().any(|p| p.contains('\n')) {
            return Err("invalid account".into());
        }
        Ok(())
    }
}

/// A switch and its persistent read connection.
pub struct Switch {
    pub cfg: SwitchCfg,
    pub conn: Mutex<Option<Api>>,
}

struct Store {
    path: String,
    list: Mutex<Vec<Arc<Switch>>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> &'static Store {
    STORE.get().expect("settings not loaded")
}

/// Loads the file, or creates it from `seed` (the configuration's switches).
pub fn init(path: &str, seed: Vec<SwitchCfg>) {
    let list = match fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<Value>(&t).expect("settings: invalid JSON")["switches"].as_array().cloned().unwrap_or_default()
            .iter().map(SwitchCfg::from_json).collect(),
        Err(_) => {
            write(path, &seed).expect("settings: cannot write the data directory");
            seed
        }
    };
    let _ = STORE.set(Store { path: path.into(), list: Mutex::new(list.into_iter().map(|cfg| Arc::new(Switch { cfg, conn: Mutex::new(None) })).collect()) });
}

fn write(path: &str, list: &[SwitchCfg]) -> Result<(), String> {
    let tmp = format!("{path}.tmp");
    let body = serde_json::to_string_pretty(&json!({"switches": list.iter().map(|s| s.to_json(true)).collect::<Vec<_>>()})).unwrap();
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn all() -> Vec<Arc<Switch>> {
    store().list.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn get(name: &str) -> Option<Arc<Switch>> {
    all().into_iter().find(|s| s.cfg.name == name)
}

/// Replaces the whole list; an unchanged switch keeps its open connection.
pub fn save(new: Vec<SwitchCfg>) -> Result<(), String> {
    for (i, s) in new.iter().enumerate() {
        s.check().map_err(|e| format!("{}: {e}", s.name))?;
        if new[..i].iter().any(|o| o.name == s.name) {
            return Err(format!("{}: same name as another switch", s.name));
        }
    }
    let st = store();
    let mut g = st.list.lock().unwrap_or_else(|e| e.into_inner());
    write(&st.path, &new)?;
    *g = new.into_iter().map(|cfg| g.iter().find(|o| o.cfg == cfg).cloned().unwrap_or_else(|| Arc::new(Switch { cfg, conn: Mutex::new(None) }))).collect();
    Ok(())
}

/// One edit from the page. `orig`: the name before the edit (none = new).
/// Empty passwords keep the stored ones.
pub fn apply(op: &Value) -> Result<(), String> {
    let mut list: Vec<SwitchCfg> = all().iter().map(|s| s.cfg.clone()).collect();
    match op["op"].as_str() {
        Some("save") => {
            let mut s = SwitchCfg::from_json(&op["switch"]);
            let orig = op["orig"].as_str().unwrap_or("");
            let old = list.iter().position(|x| !orig.is_empty() && x.name == orig);
            if let Some(i) = old {
                s.keep_secrets(&list[i]);
                list[i] = s;
            } else if !orig.is_empty() {
                return Err(format!("{orig}: unknown switch"));
            } else {
                list.push(s);
            }
        }
        Some("delete") => {
            let n = op["name"].as_str().unwrap_or("");
            list.retain(|s| s.name != n);
        }
        _ => return Err("unknown change".into()),
    }
    save(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_checks() {
        let v = json!({"name": "ROUTER", "host": "192.168.99.11", "sha256": "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF",
            "user": "monitor", "password": "x", "mode": "git"});
        let s = SwitchCfg::from_json(&v);
        assert_eq!((s.port, s.sha256.len()), (8729, 64));
        assert!(s.check().is_ok());
        assert_eq!(SwitchCfg::from_json(&s.to_json(true)), s);
        assert!(s.to_json(false).get("password").is_none());
        assert_eq!(pretty(&s.sha256), v["sha256"].as_str().unwrap());
        let mut d = s.clone();
        d.mode = "direct".into();
        assert!(d.check().is_err(), "direct needs a write account");
        d.sha256.clear();
        assert!(d.check().unwrap_err().contains("fingerprint"));
    }

    #[test]
    fn save_keeps_passwords_and_connections() {
        let dir = std::env::temp_dir().join(format!("cairn-net-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json").to_string_lossy().into_owned();
        let seed = SwitchCfg::from_json(&json!({"name": "A", "host": "10.0.0.1", "sha256": "ab".repeat(32), "user": "monitor", "password": "secret"}));
        init(&path, vec![seed.clone()]);
        let before = get("A").unwrap();
        apply(&json!({"op": "save", "orig": "", "switch": {"name": "B", "host": "10.0.0.2", "sha256": "cd".repeat(32), "user": "m", "password": "p"}})).unwrap();
        assert!(Arc::ptr_eq(&before, &get("A").unwrap()), "unchanged switch keeps its connection");
        apply(&json!({"op": "save", "orig": "A", "switch": {"name": "A", "host": "10.0.0.9", "sha256": "ab".repeat(32), "user": "monitor", "password": ""}})).unwrap();
        assert_eq!(get("A").unwrap().cfg.password, "secret");
        assert!(apply(&json!({"op": "save", "orig": "", "switch": {"name": "B", "host": "h", "sha256": "ef".repeat(32), "user": "u", "password": "p"}})).is_err());
        apply(&json!({"op": "delete", "name": "B"})).unwrap();
        assert_eq!(all().len(), 1);
        let disk: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(disk["switches"][0]["host"], "10.0.0.9");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }
}
