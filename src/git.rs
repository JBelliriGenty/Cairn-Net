//! Git mode: the DHCP and firewall files live in a repository; the
//! app keeps a shallow clone per switch in the data directory, edits the file
//! and pushes a commit under the user's name. Applying to the switch is left
//! to the user's own tool (Ansible, CI). One SSH deploy key for the whole app,
//! generated at first start; its public half is shown in Settings.

use crate::settings::SwitchCfg;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static DATA: OnceLock<String> = OnceLock::new();
// ponytail: one lock for every clone; per-switch locks if several repos get busy
static LOCK: Mutex<()> = Mutex::new(());
static SYNCED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

pub(crate) fn data() -> &'static str {
    DATA.get().map(String::as_str).unwrap_or("/var/lib/cairn-net")
}

fn key() -> PathBuf {
    Path::new(data()).join("git_key")
}

/// Creates the deploy key if missing (ssh-keygen, ed25519, no passphrase).
pub fn init(data_dir: &str) {
    let _ = DATA.set(data_dir.to_string());
    if !key().exists() {
        let ok = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", "cairn-net", "-f"]).arg(key()).status();
        if !ok.is_ok_and(|s| s.success()) {
            eprintln!("git: cannot create the deploy key (ssh-keygen)");
        }
    }
}

pub fn public_key() -> String {
    fs::read_to_string(key().with_extension("pub")).unwrap_or_default().trim().to_string()
}

fn dir(sw: &SwitchCfg) -> PathBuf {
    Path::new(data()).join("git").join(&sw.name)
}

/// ssh with the app's key (git and the DNS hosts targets): hosts accepted at first use.
pub(crate) fn ssh_options() -> Vec<String> {
    let k = key().display().to_string();
    let kh = format!("UserKnownHostsFile={}/known_hosts", data());
    ["-i", &k, "-o", "IdentitiesOnly=yes", "-o", &kh, "-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes"].map(String::from).to_vec()
}

fn git(cwd: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let ssh = format!("ssh {}", ssh_options().join(" "));
    let mut c = Command::new("git");
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let o = c.args(args).env("HOME", data()).env("GIT_SSH_COMMAND", ssh).env("GIT_TERMINAL_PROMPT", "0").output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&o.stderr);
        // the first meaningful line names the cause ("…: Deploy Key … not authorized to write",
        // " ! [rejected] …"); the last one is git's generic "make sure … the repository exists"
        let skip = |l: &str| l.trim().is_empty() || l.starts_with("Warning: Permanently added") || l.starts_with("To ") || l.starts_with("hint:");
        Err(err.lines().find(|l| !skip(l)).unwrap_or("git failed").trim().to_string())
    }
}

/// Can the app read the repository? (Settings test.)
pub fn probe(sw: &SwitchCfg) -> Result<(), String> {
    git(None, &["ls-remote", "--exit-code", "--heads", "--", &sw.git_url, &sw.git_branch]).map(|_| ())
}

/// The clone at the tip of the branch (at most one fetch per 30 s unless `force`).
fn sync(sw: &SwitchCfg, force: bool) -> Result<PathBuf, String> {
    let d = dir(sw);
    // callers hold LOCK and the clone is ours alone: an index.lock here was left by a run killed mid-git (restart)
    let _ = fs::remove_file(d.join(".git/index.lock"));
    let mut seen = SYNCED.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
    let fresh = seen.get(&sw.name).is_some_and(|t| t.elapsed() < Duration::from_secs(30));
    let origin = if d.join(".git").exists() { git(Some(&d), &["remote", "get-url", "origin"]).ok() } else { None };
    if origin.as_deref() != Some(sw.git_url.as_str()) {
        let _ = fs::remove_dir_all(&d); // settings changed: start over (our own clone, never the user's data)
        fs::create_dir_all(d.parent().unwrap()).map_err(|e| e.to_string())?;
        let ds = d.to_string_lossy().into_owned();
        git(None, &["clone", "--quiet", "--depth", "1", "--branch", &sw.git_branch, "--", &sw.git_url, &ds])?;
    } else if force || !fresh {
        git(Some(&d), &["fetch", "--quiet", "--depth", "1", "origin", &sw.git_branch])?;
        git(Some(&d), &["reset", "--quiet", "--hard", "FETCH_HEAD"])?;
    }
    seen.insert(sw.name.clone(), Instant::now());
    Ok(d)
}

/// `file` (sw.git_path or sw.git_fw_path) at the tip of the branch.
pub fn read(sw: &SwitchCfg, file: &str) -> Result<String, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = sync(sw, false)?;
    fs::read_to_string(d.join(file)).map_err(|e| format!("{file}: {e}"))
}

/// Rebuilds the file from the branch tip with `edit`, commits it as `author`
/// ("Name <mail>") and pushes. A push refused (someone pushed meanwhile)
/// leaves nothing behind: the clone is reset at the next read. A missing
/// file reads as empty (created by the commit). None: `edit` changed nothing.
pub fn commit(sw: &SwitchCfg, file: &str, author: &str, message: &str, edit: impl FnOnce(&str) -> Result<String, String>) -> Result<Option<String>, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = sync(sw, true)?;
    let f = d.join(file);
    let old = match fs::read_to_string(&f) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        r => r.map_err(|e| format!("{file}: {e}"))?,
    };
    let new = edit(&old)?;
    if new == old {
        return Ok(None);
    }
    if let Some(p) = f.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    fs::write(&f, new).map_err(|e| e.to_string())?;
    let who = format!("user.name={}", author.split(" <").next().unwrap_or(author));
    let mail = format!("user.email={}", author.split_once(" <").map_or("cairn-net@localhost", |(_, m)| m.trim_end_matches('>')));
    let r = (|| {
        git(Some(&d), &["add", "--", file])?;
        git(Some(&d), &["-c", &who, "-c", &mail, "commit", "--quiet", "-m", message, "--", file])?;
        git(Some(&d), &["push", "--quiet", "origin", &format!("HEAD:refs/heads/{}", sw.git_branch)])?;
        git(Some(&d), &["rev-parse", "--short", "HEAD"]).map(Some)
    })();
    if r.is_err() {
        // the next read fetches again and resets the clone to the branch tip
        if let Some(m) = SYNCED.get() {
            m.lock().unwrap_or_else(|e| e.into_inner()).remove(&sw.name);
        }
    }
    r
}
