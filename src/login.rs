//! Built-in sign-in, for a Cairn Net without an authenticating proxy in front
//! ("login" in the configuration, HTTPS required). Accounts live in a users
//! file, one "name:hash" per line, the hash made by `cairn-net hash-password`
//! (PBKDF2-SHA256). The file is read at every sign-in: editing it needs no
//! restart. Sessions are in memory (a restart signs everyone out), expire
//! after an idle time, and failures from one address double the wait.

use openssl::hash::MessageDigest;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Login {
    pub users_file: String,
    pub session_s: u64, // idle time before a session expires
}

const ITER: usize = 600_000; // OWASP 2023 figure for PBKDF2-HMAC-SHA256
// compared against when the user is unknown, so both cases cost the same time
const DUMMY: &str = "pbkdf2-sha256$600000$00000000000000000000000000000000$0000000000000000000000000000000000000000000000000000000000000000";

// token -> (user, last seen)
static SESSIONS: Mutex<BTreeMap<String, (String, Instant)>> = Mutex::new(BTreeMap::new());
// address -> (consecutive failures, blocked until)
static FAILS: Mutex<BTreeMap<String, (u32, Instant)>> = Mutex::new(BTreeMap::new());

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn derive(pass: &str, salt: &[u8], iter: usize) -> [u8; 32] {
    let mut k = [0u8; 32];
    openssl::pkcs5::pbkdf2_hmac(pass.as_bytes(), salt, iter, MessageDigest::sha256(), &mut k).expect("pbkdf2");
    k
}

pub fn hash_password(pass: &str) -> String {
    let mut salt = [0u8; 16];
    openssl::rand::rand_bytes(&mut salt).expect("random");
    format!("pbkdf2-sha256${ITER}${}${}", hex(&salt), hex(&derive(pass, &salt, ITER)))
}

fn verify(pass: &str, stored: &str) -> bool {
    let p: Vec<&str> = stored.split('$').collect();
    let ["pbkdf2-sha256", it, salt, hash] = p[..] else { return false };
    let (Ok(it), Some(salt), Some(hash)) = (it.parse::<usize>(), unhex(salt), unhex(hash)) else { return false };
    if !(1..=10_000_000).contains(&it) || hash.len() != 32 {
        return false;
    }
    openssl::memcmp::eq(&derive(pass, &salt, it), &hash)
}

fn check(l: &Login, name: &str, pass: &str) -> bool {
    if name.is_empty() || pass.is_empty() || pass.len() > 256 {
        return false;
    }
    let users = fs::read_to_string(&l.users_file).unwrap_or_default();
    let found = users.lines().filter_map(|x| x.trim().split_once(':')).find(|(n, _)| *n == name).map(|(_, h)| h.to_string());
    let ok = verify(pass, found.as_deref().unwrap_or(DUMMY));
    ok && found.is_some()
}

fn token() -> String {
    let mut b = [0u8; 32];
    openssl::rand::rand_bytes(&mut b).expect("random");
    hex(&b)
}

/// The user of a session cookie value, refreshing its idle timer.
pub fn session(l: &Login, tok: &str) -> Option<String> {
    let mut s = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    if s.get(tok).is_some_and(|(_, seen)| seen.elapsed().as_secs() > l.session_s) {
        s.remove(tok);
    }
    let (u, seen) = s.get_mut(tok)?;
    *seen = Instant::now();
    Some(u.clone())
}

pub fn logout(tok: &str) {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).remove(tok);
}

/// Seconds to wait before the next try from this address (rounded up).
fn blocked_for(ip: &str) -> u64 {
    let f = FAILS.lock().unwrap_or_else(|e| e.into_inner());
    f.get(ip).map_or(0, |(_, until)| (until.saturating_duration_since(Instant::now()).as_millis() as u64).div_ceil(1000))
}

/// 2^failures seconds, up to 5 min.
fn record_failure(ip: &str) {
    let mut f = FAILS.lock().unwrap_or_else(|e| e.into_inner());
    let n = f.get(ip).map_or(0, |e| e.0) + 1;
    f.insert(ip.to_string(), (n, Instant::now() + Duration::from_secs((1u64 << n.min(9)).min(300))));
}

/// Sign-in attempt: Ok(session token) or Err((status, message)).
pub fn sign_in(l: &Login, ip: &str, name: &str, pass: &str) -> Result<String, (u16, String)> {
    let wait = blocked_for(ip);
    if wait > 0 {
        return Err((429, format!("too many attempts: try again in {wait} s")));
    }
    if !check(l, name, pass) {
        record_failure(ip);
        eprintln!("login refused user={name} ip={ip}");
        std::thread::sleep(Duration::from_secs(1));
        return Err((401, "wrong user name or password".into()));
    }
    FAILS.lock().unwrap_or_else(|e| e.into_inner()).remove(ip);
    let t = token();
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).insert(t.clone(), (name.to_string(), Instant::now()));
    eprintln!("login user={name} ip={ip}");
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify() {
        let h = hash_password("s3cret");
        assert!(verify("s3cret", &h));
        assert!(!verify("s3cret ", &h));
        assert!(!verify("s3cret", "garbage"));
        assert!(!verify("x", DUMMY));
    }

    #[test]
    fn users_file_and_lockout() {
        let f = std::env::temp_dir().join(format!("cairn-net-users-{}", std::process::id()));
        fs::write(&f, format!("# comment\nalice:{}\n", hash_password("pw"))).unwrap();
        let l = Login { users_file: f.to_string_lossy().into(), session_s: 60 };
        assert!(check(&l, "alice", "pw"));
        assert!(!check(&l, "alice", "nope"));
        assert!(!check(&l, "bob", "pw"));
        assert!(!check(&l, "# comment", "pw"));
        let t = sign_in(&l, "10.0.0.9", "alice", "pw").unwrap();
        assert_eq!(session(&l, &t).as_deref(), Some("alice"));
        logout(&t);
        assert!(session(&l, &t).is_none());
        assert_eq!(sign_in(&l, "10.0.0.8", "alice", "nope").unwrap_err().0, 401);
        assert_eq!(sign_in(&l, "10.0.0.8", "alice", "pw").unwrap_err().0, 429); // blocked 2 s
        let _ = fs::remove_file(f);
    }
}
